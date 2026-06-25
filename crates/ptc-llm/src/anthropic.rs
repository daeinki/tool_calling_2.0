//! AnthropicProvider — Anthropic Messages API용 [`LlmProvider`] 구현 (M2-T10).
//!
//! 실제 LLM 호출은 네트워크·API 키가 필요하므로, 라이브 호출 자체는 키가 있을 때만
//! 도는 `#[ignore]` 통합 테스트와 수동 실행 바이너리로 분리한다. 본 모듈은 컴파일·
//! 단위 테스트(요청 본문 구성·응답 파싱)는 네트워크 없이 검증한다.
//!
//! **경계(clean-code §7):** HTTP·JSON 세부는 이 모듈 안에 갇히고, 바깥에는
//! [`LlmProvider`] trait만 보인다.

use crate::{CompletionReq, CompletionResp, LlmError, LlmProvider};
use std::time::Instant;

const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// 비용을 고려한 기본 모델. 더 강한 모델은 [`AnthropicProvider::with_model`]로 교체.
const DEFAULT_MODEL: &str = "claude-sonnet-4-6";

/// Anthropic Messages API provider.
pub struct AnthropicProvider {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::blocking::Client,
}

impl AnthropicProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: DEFAULT_MODEL.to_string(),
            base_url: ANTHROPIC_BASE_URL.to_string(),
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

    /// `ANTHROPIC_API_KEY` 환경변수에서 키를 읽어 생성한다.
    pub fn from_env() -> Result<Self, String> {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| "ANTHROPIC_API_KEY 환경변수가 설정되지 않음".to_string())?;
        Ok(Self::new(key))
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Messages API 요청 본문(seed는 API 미지원이라 싣지 않는다).
    fn build_body(&self, req: &CompletionReq) -> serde_json::Value {
        serde_json::json!({
            "model": self.model,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
            "system": req.system,
            "messages": [{ "role": "user", "content": req.user }],
        })
    }
}

impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn complete(&self, req: CompletionReq) -> Result<CompletionResp, LlmError> {
        let body = self.build_body(&req);
        let started = Instant::now();
        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
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

/// Messages API 응답 JSON에서 코드 텍스트와 토큰 회계를 뽑는다.
fn parse_response(json: &serde_json::Value) -> Result<ParsedResponse, LlmError> {
    // content는 블록 배열이며 첫 블록이 항상 text는 아니다(extended thinking이 켜지면
    // thinking 블록이, 도구 사용 시 tool_use 블록이 앞설 수 있다). 첫 text 블록을 찾는다.
    let text = json
        .get("content")
        .and_then(|content| content.as_array())
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|block| block.get("type").and_then(|t| t.as_str()) == Some("text"))
                .and_then(|block| block.get("text"))
                .and_then(|t| t.as_str())
        })
        .ok_or_else(|| LlmError::Decode("응답에 text 블록이 없음".to_string()))?
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
        input_tokens: token("input_tokens"),
        output_tokens: token("output_tokens"),
        stop_reason: json
            .get("stop_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_carries_model_and_user_message() {
        let provider = AnthropicProvider::new("key").with_model("claude-opus-4-8");
        let body = provider.build_body(&CompletionReq::new("be a coder", "2+2=?"));
        assert_eq!(body["model"], "claude-opus-4-8");
        assert_eq!(body["system"], "be a coder");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "2+2=?");
    }

    #[test]
    fn parses_a_well_formed_response() {
        let json = serde_json::json!({
            "content": [{ "type": "text", "text": "emit(4)" }],
            "usage": { "input_tokens": 12, "output_tokens": 3 },
            "stop_reason": "end_turn"
        });
        let parsed = parse_response(&json).unwrap();
        assert_eq!(parsed.text, "emit(4)");
        assert_eq!(parsed.input_tokens, 12);
        assert_eq!(parsed.output_tokens, 3);
        assert_eq!(parsed.stop_reason, "end_turn");
    }

    #[test]
    fn missing_content_is_a_decode_error() {
        let json = serde_json::json!({ "usage": { "input_tokens": 1 } });
        assert!(matches!(parse_response(&json), Err(LlmError::Decode(_))));
    }

    #[test]
    fn picks_text_block_after_a_leading_non_text_block() {
        // extended thinking이 켜지면 thinking 블록이 text 블록 앞에 온다.
        let json = serde_json::json!({
            "content": [
                { "type": "thinking", "thinking": "...추론..." },
                { "type": "text", "text": "emit(4)" }
            ],
            "usage": { "input_tokens": 10, "output_tokens": 2 },
            "stop_reason": "end_turn"
        });
        let parsed = parse_response(&json).unwrap();
        assert_eq!(parsed.text, "emit(4)");
    }

    #[test]
    fn response_without_any_text_block_is_a_decode_error() {
        let json = serde_json::json!({
            "content": [{ "type": "tool_use", "name": "x", "input": {} }],
            "stop_reason": "tool_use"
        });
        assert!(matches!(parse_response(&json), Err(LlmError::Decode(_))));
    }

    #[test]
    fn from_env_errors_without_key() {
        // 키가 설정돼 있으면 이 단언은 건너뛴다(환경 의존).
        if std::env::var("ANTHROPIC_API_KEY").is_err() {
            assert!(AnthropicProvider::from_env().is_err());
        }
    }

    #[test]
    #[ignore = "실제 Anthropic API 키와 네트워크 필요: cargo test -- --ignored"]
    fn live_completion_returns_nonempty_text() {
        let provider = AnthropicProvider::from_env().expect("ANTHROPIC_API_KEY 필요");
        let resp = provider
            .complete(CompletionReq::new(
                "DSL 코드만 출력해. 마지막은 emit(...).",
                "4를 emit하는 코드를 작성해.",
            ))
            .expect("완성 성공");
        assert!(!resp.text.is_empty());
        assert!(resp.input_tokens > 0);
    }
}
