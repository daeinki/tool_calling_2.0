//! MockProvider — 네트워크 없이 동작하는 결정론적 [`LlmProvider`] (M2-T02).
//!
//! 러너와 CI가 실제 LLM 없이 전체 파이프라인을 검증하도록, 프롬프트에 대해
//! 미리 정해둔 코드 텍스트를 돌려준다. 같은 입력 → 항상 같은 응답이므로
//! "러너·채점 로직이 옳은지"를 "LLM이 코드를 잘 생성하는지"와 분리해 검증한다.
//!
//! user 프롬프트에 등록된 부분 문자열(needle)이 포함되면 그 응답을 고른다.
//! 어느 것도 맞지 않으면 default 응답을, 그것도 없으면 오류를 돌려준다.

use crate::{CompletionReq, CompletionResp, LlmError, LlmProvider};

/// needle → 코드 텍스트 라우팅 규칙.
struct Route {
    needle: String,
    code: String,
}

/// 결정론적 mock provider.
pub struct MockProvider {
    name: String,
    routes: Vec<Route>,
    default: Option<String>,
}

impl MockProvider {
    pub fn new() -> Self {
        Self {
            name: "mock".to_string(),
            routes: Vec::new(),
            default: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// user 프롬프트에 `needle`이 들어 있으면 `code`로 응답한다(등록 순서 우선).
    pub fn respond_to(mut self, needle: impl Into<String>, code: impl Into<String>) -> Self {
        self.routes.push(Route {
            needle: needle.into(),
            code: code.into(),
        });
        self
    }

    /// 어떤 라우트도 맞지 않을 때의 기본 응답.
    pub fn default_response(mut self, code: impl Into<String>) -> Self {
        self.default = Some(code.into());
        self
    }

    fn resolve(&self, user: &str) -> Option<&str> {
        self.routes
            .iter()
            .find(|route| user.contains(&route.needle))
            .map(|route| route.code.as_str())
            .or(self.default.as_deref())
    }
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn complete(&self, req: CompletionReq) -> Result<CompletionResp, LlmError> {
        let code = self.resolve(&req.user).ok_or_else(|| {
            LlmError::Transport("mock: 프롬프트에 맞는 응답이 등록되지 않음".to_string())
        })?;
        Ok(CompletionResp {
            text: code.to_string(),
            input_tokens: estimate_tokens(&req.system) + estimate_tokens(&req.user),
            output_tokens: estimate_tokens(code),
            stop_reason: "end_turn".to_string(),
            // 결정론을 위해 측정하지 않는다(실제 지연은 라이브 provider에서만 의미).
            latency_ms: 0,
        })
    }
}

/// 문자 수 기반의 결정론적 토큰 추정(대략 4자=1토큰, 최소 1).
///
/// 결정론적 mock들이 토큰 회계를 **같은 방식으로** 채우게 공유한다(PTC mock과
/// baseline ReAct mock의 토큰 비교가 공정하려면 추정식이 하나여야 한다 — DRY).
pub fn estimate_tokens(text: &str) -> u32 {
    (text.chars().count() / 4).max(1) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_response_is_returned_and_deterministic() {
        let provider = MockProvider::new().default_response("emit(1)");
        let first = provider
            .complete(CompletionReq::new("sys", "anything"))
            .unwrap();
        let second = provider
            .complete(CompletionReq::new("sys", "anything"))
            .unwrap();
        assert_eq!(first.text, "emit(1)");
        assert_eq!(first.text, second.text);
        assert_eq!(first.output_tokens, second.output_tokens);
    }

    #[test]
    fn routes_match_by_needle_in_user_prompt() {
        let provider = MockProvider::new()
            .respond_to("expense", "emit(13000)")
            .respond_to("budget", "emit(2500)")
            .default_response("emit(0)");
        assert_eq!(
            provider
                .complete(CompletionReq::new("sys", "sum the expense totals"))
                .unwrap()
                .text,
            "emit(13000)"
        );
        assert_eq!(
            provider
                .complete(CompletionReq::new("sys", "what is the budget?"))
                .unwrap()
                .text,
            "emit(2500)"
        );
    }

    #[test]
    fn first_matching_route_wins() {
        let provider = MockProvider::new()
            .respond_to("q", "first")
            .respond_to("q", "second");
        assert_eq!(
            provider
                .complete(CompletionReq::new("s", "q"))
                .unwrap()
                .text,
            "first"
        );
    }

    #[test]
    fn unmatched_prompt_without_default_errors() {
        let provider = MockProvider::new().respond_to("x", "emit(1)");
        assert!(matches!(
            provider.complete(CompletionReq::new("s", "y")),
            Err(LlmError::Transport(_))
        ));
    }

    #[test]
    fn token_accounting_is_populated_and_deterministic() {
        let provider = MockProvider::new().default_response("emit(42)");
        let resp = provider
            .complete(CompletionReq::new("system prompt", "user question"))
            .unwrap();
        assert!(resp.input_tokens > 0);
        assert!(resp.output_tokens > 0);
        assert_eq!(resp.stop_reason, "end_turn");
    }

    #[test]
    fn usable_through_trait_object_with_custom_name() {
        let provider = MockProvider::new()
            .with_name("mock-anthropic")
            .default_response("emit(1)");
        let dynamic: &dyn LlmProvider = &provider;
        assert_eq!(dynamic.name(), "mock-anthropic");
    }
}
