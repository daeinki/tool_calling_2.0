//! OpenAIProvider — OpenAI Chat Completions API용 [`LlmProvider`] 구현 (M3-T07).
//!
//! [`AnthropicProvider`](crate::AnthropicProvider)와 같은 패턴을 따른다: HTTP·JSON
//! 세부는 이 모듈 안에 갇히고, 바깥에는 [`LlmProvider`] trait만 보인다(경계, §7).
//! 라이브 호출은 키·네트워크가 필요하므로 `#[ignore]` 테스트·바이너리로 분리하고,
//! 본 모듈은 요청 본문 구성·응답 파싱만 네트워크 없이 단위 검증한다.

use crate::{CompletionReq, CompletionResp, LlmError, LlmProvider};
use std::time::Instant;

const OPENAI_BASE_URL: &str = "https://api.openai.com";
/// 비용을 고려한 기본 모델. 더 강한 모델은 [`OpenAiProvider::with_model`]로 교체.
const DEFAULT_MODEL: &str = "gpt-4o-mini";

/// OpenAI Chat Completions provider.
pub struct OpenAiProvider {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::blocking::Client,
}

impl OpenAiProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: DEFAULT_MODEL.to_string(),
            base_url: OPENAI_BASE_URL.to_string(),
            client: reqwest::blocking::Client::new(),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// 테스트·프록시용 베이스 URL 교체.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// `OPENAI_API_KEY` 환경변수에서 키를 읽어 생성한다.
    pub fn from_env() -> Result<Self, String> {
        let key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| "OPENAI_API_KEY 환경변수가 설정되지 않음".to_string())?;
        Ok(Self::new(key))
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Chat Completions 요청 본문. system·user를 메시지 2개로 싣고, seed를 지원한다.
    fn build_body(&self, req: &CompletionReq) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
            "messages": [
                { "role": "system", "content": req.system },
                { "role": "user", "content": req.user },
            ],
        });
        if let Some(seed) = req.seed {
            body["seed"] = serde_json::json!(seed);
        }
        body
    }
}

impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn complete(&self, req: CompletionReq) -> Result<CompletionResp, LlmError> {
        let body = self.build_body(&req);
        let started = Instant::now();
        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .map_err(|e| LlmError::Transport(e.to_string()))?;
        let latency_ms = started.elapsed().as_millis() as u64;

        let status = response.status();
        let text = response
            .text()
            .map_err(|e| LlmError::Transport(e.to_string()))?;
        if !status.is_success() {
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: text,
            });
        }

        let json: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| LlmError::Decode(e.to_string()))?;
        let parsed = parse_response(&json)?;

        Ok(CompletionResp {
            text: parsed.text,
            input_tokens: parsed.input_tokens,
            output_tokens: parsed.output_tokens,
            stop_reason: parsed.stop_reason,
            latency_ms,
        })
    }
}

struct ParsedResponse {
    text: String,
    input_tokens: u32,
    output_tokens: u32,
    stop_reason: String,
}

/// Chat Completions 응답 JSON에서 코드 텍스트와 토큰 회계를 뽑는다.
fn parse_response(json: &serde_json::Value) -> Result<ParsedResponse, LlmError> {
    let choice = json
        .get("choices")
        .and_then(|choices| choices.get(0))
        .ok_or_else(|| LlmError::Decode("응답에 choices[0]이 없음".to_string()))?;
    let text = choice
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .ok_or_else(|| LlmError::Decode("응답에 choices[0].message.content가 없음".to_string()))?
        .to_string();

    let usage = json.get("usage");
    let token = |field: &str| {
        usage
            .and_then(|u| u.get(field))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32
    };

    Ok(ParsedResponse {
        text,
        input_tokens: token("prompt_tokens"),
        output_tokens: token("completion_tokens"),
        stop_reason: choice
            .get("finish_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_carries_model_system_and_user_messages() {
        let provider = OpenAiProvider::new("key").with_model("gpt-4o");
        let mut req = CompletionReq::new("be a coder", "2+2=?");
        req.seed = Some(7);
        let body = provider.build_body(&req);
        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "be a coder");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "2+2=?");
        assert_eq!(body["seed"], 7);
    }

    #[test]
    fn body_omits_seed_when_absent() {
        let body = OpenAiProvider::new("key").build_body(&CompletionReq::new("s", "u"));
        assert!(body.get("seed").is_none());
    }

    #[test]
    fn parses_a_well_formed_response() {
        let json = serde_json::json!({
            "choices": [{
                "message": { "role": "assistant", "content": "emit(4)" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 12, "completion_tokens": 3 }
        });
        let parsed = parse_response(&json).unwrap();
        assert_eq!(parsed.text, "emit(4)");
        assert_eq!(parsed.input_tokens, 12);
        assert_eq!(parsed.output_tokens, 3);
        assert_eq!(parsed.stop_reason, "stop");
    }

    #[test]
    fn missing_choices_is_a_decode_error() {
        let json = serde_json::json!({ "usage": { "prompt_tokens": 1 } });
        assert!(matches!(parse_response(&json), Err(LlmError::Decode(_))));
    }

    #[test]
    fn name_is_openai() {
        assert_eq!(OpenAiProvider::new("k").name(), "openai");
    }

    #[test]
    fn from_env_errors_without_key() {
        if std::env::var("OPENAI_API_KEY").is_err() {
            assert!(OpenAiProvider::from_env().is_err());
        }
    }

    #[test]
    #[ignore = "실제 OpenAI API 키와 네트워크 필요: cargo test -- --ignored"]
    fn live_completion_returns_nonempty_text() {
        let provider = OpenAiProvider::from_env().expect("OPENAI_API_KEY 필요");
        let resp = provider
            .complete(CompletionReq::new(
                "DSL 코드만 출력해. 마지막은 emit(...).",
                "4를 emit하는 코드를 작성해.",
            ))
            .expect("완성 성공");
        assert!(!resp.text.is_empty());
    }
}
