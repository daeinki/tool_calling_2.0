//! 채점기 — 실행 결과를 pass/fail로 판정한다 (M2-T05).
//!
//! 채점 레벨은 **trait 다형성**으로 표현한다(boolean 플래그 인수 금지, clean-code §2).
//! 각 채점기는 자신의 기준(기대값)을 캡슐화하고, 공용 [`Execution`]을 입력으로 본다.
//! 그래서 L2(TraceMatch)·L3(Semantic)를 trait 시그니처 변경 없이 더할 수 있다.
//!
//! - **L1 [`ExactMatch`]**: emit된 최종값이 기대값과 정확히 일치(정답이 유일한 태스크).
//! - **L2 [`TraceMatch`]**: 도구 호출 트레이스가 기대 호출 개수를 만족(절차 정확성).
//! - L3 Semantic(M3): LLM 채점관.

use crate::task::ExpectedToolCall;
use ptc_dsl::Value;
use ptc_tools::{base_tool, ToolCall};

/// 한 번 실행한 결과. 채점기가 들여다보는 모든 것.
#[derive(Debug, Clone)]
pub struct Execution {
    /// emit된 최종값(없으면 `None`).
    pub output: Option<Value>,
    /// 순서대로의 도구 호출(L2 채점용).
    pub trace: Vec<ToolCall>,
}

/// 실행 결과를 판정하는 채점기.
pub trait Grader {
    /// 채점 레벨 식별자(RunRecord에 기록).
    fn level(&self) -> &'static str;

    /// 통과 여부.
    fn grade(&self, execution: &Execution) -> bool;
}

/// L1 — emit된 최종값이 기대값과 정확히 일치하는지 본다.
pub struct ExactMatch {
    expected: Option<Value>,
}

impl ExactMatch {
    pub fn new(expected: Option<Value>) -> Self {
        Self { expected }
    }
}

impl Grader for ExactMatch {
    fn level(&self) -> &'static str {
        "L1"
    }

    fn grade(&self, execution: &Execution) -> bool {
        execution.output == self.expected
    }
}

/// L2 — 도구 호출 트레이스가 기대 호출 집합을 만족하는지 본다(절차 정확성).
///
/// 순서가 아니라 **도구별 호출 횟수**를 검증한다. LLM이 같은 도구를 다른 순서로
/// 부르더라도 절차가 옳으면 통과시키기 위함이다. bare·`domain.tool` 호출은
/// [`base_tool`]로 정규화해 같은 도구로 센다.
pub struct TraceMatch {
    expected: Vec<ExpectedToolCall>,
}

impl TraceMatch {
    pub fn new(expected: Vec<ExpectedToolCall>) -> Self {
        Self { expected }
    }

    /// 기대 제약이 비어 있는가 — 비면 어떤 트레이스든 통과하므로 측정상 무의미하다.
    /// 로더가 L2 태스크에서 이를 거부하도록 노출한다(측정의 정직성).
    pub fn is_vacuous(&self) -> bool {
        self.expected.is_empty()
    }
}

impl Grader for TraceMatch {
    fn level(&self) -> &'static str {
        "L2"
    }

    fn grade(&self, execution: &Execution) -> bool {
        self.expected
            .iter()
            .all(|exp| expectation_holds(exp, observed_count(&execution.trace, &exp.tool)))
    }
}

/// 트레이스에서 기대 도구와 같은 도구(네임스페이스 무관)의 호출 횟수.
fn observed_count(trace: &[ToolCall], expected_tool: &str) -> usize {
    let target = base_tool(expected_tool);
    trace
        .iter()
        .filter(|call| base_tool(&call.tool) == target)
        .count()
}

/// 한 도구의 기대 제약(count 정확·count_min 하한)이 관측 횟수에서 성립하는가.
/// 둘 다 비면 "최소 1회 호출"을 요구해, 빈 기대가 조용히 통과하지 않게 한다.
fn expectation_holds(expected: &ExpectedToolCall, observed: usize) -> bool {
    let exact_ok = expected.count.is_none_or(|n| observed == n);
    let min_ok = expected.count_min.is_none_or(|n| observed >= n);
    let presence_ok = expected.count.is_some() || expected.count_min.is_some() || observed >= 1;
    exact_ok && min_ok && presence_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn execution(output: Option<Value>) -> Execution {
        Execution {
            output,
            trace: Vec::new(),
        }
    }

    fn call(tool: &str) -> ToolCall {
        ToolCall {
            tool: tool.into(),
            args: Default::default(),
        }
    }

    fn traced(tools: &[&str]) -> Execution {
        Execution {
            output: None,
            trace: tools.iter().map(|t| call(t)).collect(),
        }
    }

    fn expect(tool: &str, count: Option<usize>, count_min: Option<usize>) -> ExpectedToolCall {
        ExpectedToolCall {
            tool: tool.into(),
            count,
            count_min,
        }
    }

    #[test]
    fn exact_match_passes_on_equal_value() {
        let grader = ExactMatch::new(Some(Value::Num(13000.0)));
        assert!(grader.grade(&execution(Some(Value::Num(13000.0)))));
    }

    #[test]
    fn exact_match_fails_on_different_value() {
        let grader = ExactMatch::new(Some(Value::Num(13000.0)));
        assert!(!grader.grade(&execution(Some(Value::Num(42.0)))));
    }

    #[test]
    fn exact_match_handles_no_emit_expectation() {
        let grader = ExactMatch::new(None);
        assert!(grader.grade(&execution(None)));
        assert!(!grader.grade(&execution(Some(Value::Null))));
    }

    #[test]
    fn level_is_l1() {
        assert_eq!(ExactMatch::new(None).level(), "L1");
    }

    #[test]
    fn usable_as_trait_object() {
        // 채점 레벨을 boolean 분기가 아닌 다형성으로 고르는지 확인.
        let grader: Box<dyn Grader> = Box::new(ExactMatch::new(Some(Value::Str("over".into()))));
        assert!(grader.grade(&execution(Some(Value::Str("over".into())))));
        assert_eq!(grader.level(), "L1");
    }

    #[test]
    fn l1_ignores_the_trace() {
        let grader = ExactMatch::new(Some(Value::Num(1.0)));
        let with_trace = Execution {
            output: Some(Value::Num(1.0)),
            trace: vec![ToolCall {
                tool: "list_team".into(),
                args: Default::default(),
            }],
        };
        assert!(grader.grade(&with_trace));
    }

    // ── L2 TraceMatch (M3-T02) ──

    #[test]
    fn trace_match_exact_count_passes_and_fails() {
        let grader = TraceMatch::new(vec![expect("get_expenses", Some(4), None)]);
        assert!(grader.grade(&traced(&[
            "list_team",
            "get_expenses",
            "get_expenses",
            "get_expenses",
            "get_expenses"
        ])));
        // 3회뿐이면 정확 개수 불일치로 실패.
        assert!(!grader.grade(&traced(&["get_expenses", "get_expenses", "get_expenses"])));
    }

    #[test]
    fn trace_match_count_min_is_a_lower_bound() {
        let grader = TraceMatch::new(vec![expect("get_expenses", None, Some(2))]);
        assert!(grader.grade(&traced(&["get_expenses", "get_expenses", "get_expenses"])));
        assert!(!grader.grade(&traced(&["get_expenses"])));
    }

    #[test]
    fn trace_match_without_counts_requires_presence() {
        let grader = TraceMatch::new(vec![expect("send_email", None, None)]);
        assert!(grader.grade(&traced(&["list_team", "send_email"])));
        assert!(!grader.grade(&traced(&["list_team"])));
    }

    #[test]
    fn trace_match_normalizes_namespaced_calls() {
        // 기대는 bare, 실제 호출은 namespaced여도 같은 도구로 센다.
        let grader = TraceMatch::new(vec![expect("list_events", Some(1), None)]);
        assert!(grader.grade(&traced(&["schedule.list_events"])));
    }

    #[test]
    fn trace_match_checks_every_expectation() {
        let grader = TraceMatch::new(vec![
            expect("list_team", Some(1), None),
            expect("get_expenses", None, Some(4)),
        ]);
        let ok = traced(&[
            "list_team",
            "get_expenses",
            "get_expenses",
            "get_expenses",
            "get_expenses",
        ]);
        assert!(grader.grade(&ok));
        // list_team이 빠지면 첫 기대가 깨져 실패.
        let missing_team = traced(&[
            "get_expenses",
            "get_expenses",
            "get_expenses",
            "get_expenses",
        ]);
        assert!(!grader.grade(&missing_team));
    }

    #[test]
    fn trace_match_level_and_vacuity() {
        let grader = TraceMatch::new(Vec::new());
        assert_eq!(grader.level(), "L2");
        assert!(grader.is_vacuous());
        // 빈 기대는 측정상 무의미: 어떤 트레이스든 통과해 버린다.
        assert!(grader.grade(&traced(&["anything"])));
        assert!(!TraceMatch::new(vec![expect("x", None, None)]).is_vacuous());
    }

    #[test]
    fn trace_match_usable_as_trait_object() {
        let grader: Box<dyn Grader> =
            Box::new(TraceMatch::new(vec![expect("list_team", Some(1), None)]));
        assert_eq!(grader.level(), "L2");
        assert!(grader.grade(&traced(&["list_team"])));
    }
}
