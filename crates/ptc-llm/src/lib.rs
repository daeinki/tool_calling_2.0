//! `ptc-llm` — LLM provider 추상화 (M2-T01).
//!
//! 프롬프트를 코드 텍스트로 바꾸는 한 가지 일을 [`LlmProvider`] trait 뒤에 둔다.
//! Anthropic·Ollama·OpenAI 등 구현체가 바뀌어도 러너 코드는 영향받지 않는다.
//!
//! **경계 관리(clean-code §7):** HTTP·JSON 직렬화 같은 외부 세부사항은 구현체
//! 안에 갇히고, 도메인에는 [`CompletionReq`]/[`CompletionResp`]/[`LlmError`]만
//! 노출된다.
//!
//! **동기 trait:** 설계 문서의 async 스케치와 달리 v0는 동기로 둔다. M2는
//! 태스크를 R회 순차 실행할 뿐 동시성이 필요 없고, 동기 쪽이 런타임·`async-trait`
//! 의존성 없이 테스트가 단순하다. `Send + Sync` 바운드를 유지하므로 이후
//! 스레드 기반 병렬이 필요해도 trait 시그니처를 바꾸지 않는다.
//!
//! 현재 구현 범위: M2-T01(trait + req/resp/error), M2-T02([`MockProvider`]),
//! M2-T10([`AnthropicProvider`] — 라이브), M3-T07([`OpenAiProvider`]·[`OllamaProvider`]).

pub mod anthropic;
pub mod mock;
pub mod ollama;
pub mod openai;

pub use anthropic::AnthropicProvider;
pub use mock::{estimate_tokens, MockProvider};
pub use ollama::OllamaProvider;
pub use openai::OpenAiProvider;

use std::time::Instant;
use thiserror::Error;

/// 프롬프트를 코드 텍스트로 완성하는 LLM 백엔드.
pub trait LlmProvider: Send + Sync {
    /// 로깅·RunRecord에 기록할 provider 식별자(예: `"anthropic"`).
    fn name(&self) -> &str;

    /// 프롬프트를 한 번 완성한다. 토큰 회계를 응답에 함께 싣는다.
    fn complete(&self, req: CompletionReq) -> Result<CompletionResp, LlmError>;
}

/// 완성 요청. 재현성을 위해 입력 조건을 모두 명시한다.
#[derive(Debug, Clone)]
pub struct CompletionReq {
    /// 버전 관리되는 시스템 프롬프트.
    pub system: String,
    /// 태스크 질문 + 도구 스텁.
    pub user: String,
    /// 재현성: 0.0 권장.
    pub temperature: f32,
    /// provider가 지원하면 고정한다.
    pub seed: Option<u64>,
    pub max_tokens: u32,
}

impl CompletionReq {
    /// 재현성 우선 기본값(temperature 0.0, seed 없음)으로 요청을 만든다.
    pub fn new(system: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            system: system.into(),
            user: user.into(),
            temperature: 0.0,
            seed: None,
            max_tokens: 1024,
        }
    }
}

/// 완성 응답. 성능 비교의 1차 지표인 토큰 회계를 반드시 포함한다.
#[derive(Debug, Clone)]
pub struct CompletionResp {
    /// 생성된 코드(추출 전 원문).
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub stop_reason: String,
    pub latency_ms: u64,
}

/// provider 호출 실패. 외부 라이브러리 타입을 노출하지 않고 문자열로 감싼다.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LlmError {
    #[error("전송 실패: {0}")]
    Transport(String),

    #[error("provider 오류 응답 (status {status}): {message}")]
    Api { status: u16, message: String },

    #[error("응답 디코딩 실패: {0}")]
    Decode(String),
}

/// provider 응답 파서가 돌려주는 공통 추출 결과(어댑터 3종이 공유한다).
/// `CompletionResp`에서 latency를 뺀 부분으로, 전송 측이 latency를 채워 완성한다.
pub(crate) struct ParsedResponse {
    pub(crate) text: String,
    pub(crate) input_tokens: u32,
    pub(crate) output_tokens: u32,
    pub(crate) stop_reason: String,
}

/// JSON 객체에서 u32 토큰 필드를 읽는다(객체가 없거나 필드가 없으면 0).
/// 토큰 회계의 '누락=0' 규칙을 한 곳에 둔다(어댑터 3종 공유).
pub(crate) fn u32_field(obj: Option<&serde_json::Value>, field: &str) -> u32 {
    obj.and_then(|o| o.get(field))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32
}

/// HTTP 요청을 보내고, 지연을 재고, 상태·본문을 검사한 뒤 provider별 `parse`로
/// 디코딩해 [`CompletionResp`]를 만든다. 어댑터 3종이 공유하던 전송·디코딩 플러밍을
/// 한 곳에 모았다(각 어댑터는 URL·헤더·본문과 parse 함수만 제공한다).
pub(crate) fn send_and_decode(
    request: reqwest::blocking::RequestBuilder,
    parse: impl FnOnce(&serde_json::Value) -> Result<ParsedResponse, LlmError>,
) -> Result<CompletionResp, LlmError> {
    let started = Instant::now();
    let response = request
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
    let parsed = parse(&json)?;

    Ok(CompletionResp {
        text: parsed.text,
        input_tokens: parsed.input_tokens,
        output_tokens: parsed.output_tokens,
        stop_reason: parsed.stop_reason,
        latency_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_request_defaults_to_deterministic_settings() {
        let req = CompletionReq::new("sys", "user");
        assert_eq!(req.temperature, 0.0);
        assert_eq!(req.seed, None);
        assert!(req.max_tokens > 0);
    }

    /// trait가 객체 안전하며 토큰 회계를 실어 나르는지 확인하는 테스트 더블.
    struct StubProvider;

    impl LlmProvider for StubProvider {
        fn name(&self) -> &str {
            "stub"
        }

        fn complete(&self, _req: CompletionReq) -> Result<CompletionResp, LlmError> {
            Ok(CompletionResp {
                text: "emit(1)".to_string(),
                input_tokens: 10,
                output_tokens: 3,
                stop_reason: "end_turn".to_string(),
                latency_ms: 5,
            })
        }
    }

    #[test]
    fn provider_is_object_safe_and_carries_token_accounting() {
        let provider: &dyn LlmProvider = &StubProvider;
        let resp = provider
            .complete(CompletionReq::new("sys", "2+2=?"))
            .unwrap();
        assert_eq!(provider.name(), "stub");
        assert_eq!(resp.input_tokens, 10);
        assert_eq!(resp.output_tokens, 3);
    }

    #[test]
    fn errors_render_with_context() {
        let err = LlmError::Api {
            status: 429,
            message: "rate limited".into(),
        };
        assert_eq!(
            err.to_string(),
            "provider 오류 응답 (status 429): rate limited"
        );
    }
}
