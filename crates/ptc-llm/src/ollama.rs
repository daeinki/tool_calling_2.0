//! OllamaProvider — 로컬 Ollama Chat API용 [`LlmProvider`] 구현 (M3-T07).
//!
//! 로컬 실행이라 비용 0·키 불필요인 비교 경로를 제공한다(설계 리스크 완화). 기본
//! 엔드포인트는 `http://localhost:11434`이며 `OLLAMA_HOST`로 바꿀 수 있다. HTTP·JSON
//! 세부는 이 모듈에 갇히고 바깥에는 [`LlmProvider`] trait만 보인다(경계, §7).

use crate::{
    send_and_decode, u32_field, CompletionReq, CompletionResp, LlmError, LlmProvider,
    ParsedResponse,
};

const OLLAMA_BASE_URL: &str = "http://localhost:11434";
const DEFAULT_MODEL: &str = "llama3.2";

/// 로컬 Ollama provider.
pub struct OllamaProvider {
    model: String,
    base_url: String,
    client: reqwest::blocking::Client,
}

impl OllamaProvider {
    pub fn new() -> Self {
        Self {
            model: DEFAULT_MODEL.to_string(),
            base_url: OLLAMA_BASE_URL.to_string(),
            client: reqwest::blocking::Client::new(),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// `OLLAMA_HOST`가 있으면 그 베이스 URL을, 없으면 localhost 기본값을 쓴다.
    pub fn from_env() -> Self {
        match std::env::var("OLLAMA_HOST") {
            Ok(host) if !host.is_empty() => Self::new().with_base_url(host),
            _ => Self::new(),
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Chat 요청 본문. 스트리밍을 끄고 한 번에 받으며, temperature·seed·최대 토큰은
    /// options에 싣는다. `num_predict`(=max_tokens)를 빼면 Anthropic/OpenAI와 토큰·지연
    /// 비교가 불공정해지고 폭주 생성이 막히지 않으므로 함께 전달한다.
    fn build_body(&self, req: &CompletionReq) -> serde_json::Value {
        let mut options = serde_json::json!({
            "temperature": req.temperature,
            "num_predict": req.max_tokens,
        });
        if let Some(seed) = req.seed {
            options["seed"] = serde_json::json!(seed);
        }
        serde_json::json!({
            "model": self.model,
            "stream": false,
            "options": options,
            "messages": [
                { "role": "system", "content": req.system },
                { "role": "user", "content": req.user },
            ],
        })
    }
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    fn complete(&self, req: CompletionReq) -> Result<CompletionResp, LlmError> {
        let body = self.build_body(&req);
        let request = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body);
        send_and_decode(request, parse_response)
    }
}

/// 200 본문에 담긴 `error` 메시지(있으면). Ollama는 모델 없음 등을 이 형태로 신호한다.
fn error_in_body(json: &serde_json::Value) -> Option<&str> {
    json.get("error").and_then(|e| e.as_str())
}

/// Ollama chat 응답 JSON에서 코드 텍스트와 토큰 회계를 뽑는다.
fn parse_response(json: &serde_json::Value) -> Result<ParsedResponse, LlmError> {
    // Ollama는 일부 실패(예: 모델 없음)를 200 본문의 `error` 필드로 신호한다.
    // 그대로 두면 'message.content 없음' Decode로 원인이 가려지므로 먼저 Api 에러로 바꾼다.
    // (전송 측이 2xx일 때만 여기 도달하므로 status는 200으로 보고한다.)
    if let Some(error) = error_in_body(json) {
        return Err(LlmError::Api {
            status: 200,
            message: error.to_string(),
        });
    }
    let text = json
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .ok_or_else(|| LlmError::Decode("응답에 message.content가 없음".to_string()))?
        .to_string();

    Ok(ParsedResponse {
        text,
        input_tokens: u32_field(Some(json), "prompt_eval_count"),
        output_tokens: u32_field(Some(json), "eval_count"),
        stop_reason: json
            .get("done_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_disables_streaming_and_carries_messages() {
        let provider = OllamaProvider::new().with_model("llama3.1");
        let mut req = CompletionReq::new("be a coder", "2+2=?");
        req.seed = Some(7);
        let body = provider.build_body(&req);
        assert_eq!(body["model"], "llama3.1");
        assert_eq!(body["stream"], false);
        assert_eq!(body["messages"][0]["content"], "be a coder");
        assert_eq!(body["messages"][1]["content"], "2+2=?");
        assert_eq!(body["options"]["seed"], 7);
        // max_tokens는 num_predict로 전달되어 출력 길이가 상한된다.
        assert_eq!(body["options"]["num_predict"], req.max_tokens);
    }

    #[test]
    fn error_field_in_body_is_detected() {
        // Ollama는 모델 없음 등을 200 + {"error":...}로 신호 — Decode로 가리지 않는다.
        let json = serde_json::json!({ "error": "model 'llama3.2' not found" });
        assert_eq!(error_in_body(&json), Some("model 'llama3.2' not found"));
        // 정상 응답에는 error 필드가 없다.
        let ok = serde_json::json!({ "message": { "content": "emit(1)" } });
        assert_eq!(error_in_body(&ok), None);
    }

    #[test]
    fn parses_a_well_formed_response() {
        let json = serde_json::json!({
            "message": { "role": "assistant", "content": "emit(4)" },
            "prompt_eval_count": 12,
            "eval_count": 3,
            "done_reason": "stop"
        });
        let parsed = parse_response(&json).unwrap();
        assert_eq!(parsed.text, "emit(4)");
        assert_eq!(parsed.input_tokens, 12);
        assert_eq!(parsed.output_tokens, 3);
        assert_eq!(parsed.stop_reason, "stop");
    }

    #[test]
    fn missing_message_is_a_decode_error() {
        let json = serde_json::json!({ "eval_count": 1 });
        assert!(matches!(parse_response(&json), Err(LlmError::Decode(_))));
    }

    #[test]
    fn name_is_ollama() {
        assert_eq!(OllamaProvider::new().name(), "ollama");
    }

    #[test]
    #[ignore = "로컬 Ollama 서버 필요(ollama serve): cargo test -- --ignored"]
    fn live_completion_returns_nonempty_text() {
        let provider = OllamaProvider::from_env();
        let resp = provider
            .complete(CompletionReq::new(
                "DSL 코드만 출력해. 마지막은 emit(...).",
                "4를 emit하는 코드를 작성해.",
            ))
            .expect("완성 성공");
        assert!(!resp.text.is_empty());
    }
}
