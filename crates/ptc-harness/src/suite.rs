//! 골든 태스크 스위트 — tier×domain 계층화된 22개 과제의 단일 출처 (M3-T03).
//!
//! 각 항목은 질문과 **레퍼런스 해법(canned DSL)**을 담는다. 기대 출력·기대 호출은
//! 손으로 적지 않고 레퍼런스 해법을 mock 위에서 실행해 도출한다([`run_solution`]).
//! 그래서 "정답"은 곧 "레퍼런스 해법이 고정 데이터에서 계산한 값"으로 정의되며,
//! 산수 실수로 기대값이 어긋날 여지가 없다(정답의 근거 = 검토 가능한 짧은 코드).
//!
//! 이 테이블은 두 곳에 쓰인다:
//! - `gen-tasks` 바이너리: 여기서 `tasks/*.toml`(선언적 데이터)을 생성한다.
//! - M3 게이트: 레퍼런스 해법을 [`MockProvider`]로 주입해 통과율 표를 재현한다.

use crate::react::Action;
use ptc_dsl::{parse, tokenize, validate, Interpreter, ToolCatalog, Value};
use ptc_llm::{
    estimate_tokens, CompletionReq, CompletionResp, LlmError, LlmProvider, MockProvider,
};
use ptc_tools::{tool_names, MockToolServer, ToolCall};

/// 스위트 한 항목. 기대값은 담지 않는다(해법 실행으로 도출).
pub struct SuiteEntry {
    pub id: &'static str,
    pub tier: &'static str,
    pub domains: &'static [&'static str],
    pub question: &'static str,
    /// 채점 레벨: `"L1"`(출력) 또는 `"L2"`(절차/트레이스).
    pub grader: &'static str,
    /// 레퍼런스 해법 DSL. 정답을 정의하고 게이트에서 mock 응답으로 쓰인다.
    pub solution: &'static str,
}

/// 22개 골든 태스크. easy 7 · medium 8 · hard 7, hr/finance/schedule 도메인.
pub const SUITE: &[SuiteEntry] = &[
    // ── HR ──
    SuiteEntry {
        id: "easy_first_member",
        tier: "easy",
        domains: &["hr"],
        question: "엔지니어링(eng) 팀의 첫 번째 구성원 이름을 알려줘.",
        grader: "L1",
        solution: "team = list_team(\"eng\")
emit(team[0].name)",
    },
    SuiteEntry {
        id: "easy_eng_headcount",
        tier: "easy",
        domains: &["hr"],
        question: "엔지니어링(eng) 팀의 인원수는 몇 명이야?",
        grader: "L1",
        solution: "team = list_team(\"eng\")
n = 0
for m in team:
    n = n + 1
emit(n)",
    },
    SuiteEntry {
        id: "medium_last_eng_member",
        tier: "medium",
        domains: &["hr"],
        question: "엔지니어링(eng) 팀 명단의 마지막 구성원 이름은?",
        grader: "L1",
        solution: "team = list_team(\"eng\")
name = \"\"
for m in team:
    name = m.name
emit(name)",
    },
    // ── Finance ──
    SuiteEntry {
        id: "easy_q3_budget",
        tier: "easy",
        domains: &["finance"],
        question: "Q3 분기의 1인 출장 예산은 얼마야?",
        grader: "L1",
        solution: "emit(get_budget(\"Q3\"))",
    },
    SuiteEntry {
        id: "easy_alice_q3_expense",
        tier: "easy",
        domains: &["hr", "finance"],
        question: "엔지니어링 팀 첫 구성원의 Q3 출장 지출은?",
        grader: "L1",
        solution: "team = list_team(\"eng\")
emit(get_expenses(team[0].id, \"Q3\"))",
    },
    SuiteEntry {
        id: "easy_dave_q1_expense",
        tier: "easy",
        domains: &["hr", "finance"],
        question: "엔지니어링 팀 네 번째 구성원의 Q1 출장 지출은?",
        grader: "L1",
        solution: "team = list_team(\"eng\")
emit(get_expenses(team[3].id, \"Q1\"))",
    },
    SuiteEntry {
        id: "medium_eng_q3_total",
        tier: "medium",
        domains: &["hr", "finance"],
        question: "엔지니어링 팀 전원의 Q3 출장 지출 합계는?",
        grader: "L1",
        solution: "team = list_team(\"eng\")
total = 0
for m in team:
    total = total + get_expenses(m.id, \"Q3\")
emit(total)",
    },
    SuiteEntry {
        id: "medium_bob_annual_expense",
        tier: "medium",
        domains: &["hr", "finance"],
        question: "엔지니어링 팀 두 번째 구성원의 연간 출장 지출 합계는?",
        grader: "L1",
        solution: "team = list_team(\"eng\")
emit(get_expenses(team[1].id))",
    },
    SuiteEntry {
        id: "medium_sales_q3_total",
        tier: "medium",
        domains: &["hr", "finance"],
        question: "세일즈(sales) 팀 전원의 Q3 출장 지출 합계는?",
        grader: "L1",
        solution: "team = list_team(\"sales\")
total = 0
for m in team:
    total = total + get_expenses(m.id, \"Q3\")
emit(total)",
    },
    SuiteEntry {
        id: "medium_alice_vs_bob_q3",
        tier: "medium",
        domains: &["hr", "finance"],
        question: "엔지니어링 팀 첫 두 구성원의 Q3 출장 지출 합은?",
        grader: "L1",
        solution: "team = list_team(\"eng\")
emit(get_expenses(team[0].id, \"Q3\") + get_expenses(team[1].id, \"Q3\"))",
    },
    SuiteEntry {
        id: "hard_eng_over_q3_budget_count",
        tier: "hard",
        domains: &["hr", "finance"],
        question: "엔지니어링 팀에서 Q3 출장 지출이 Q3 예산을 초과한 사람은 몇 명이야?",
        grader: "L1",
        solution: "team = list_team(\"eng\")
budget = get_budget(\"Q3\")
n = 0
for m in team:
    spent = get_expenses(m.id, \"Q3\")
    if spent > budget:
        n = n + 1
emit(n)",
    },
    SuiteEntry {
        id: "hard_eng_q3_overage_total",
        tier: "hard",
        domains: &["hr", "finance"],
        question: "엔지니어링 팀에서 Q3 예산을 초과한 사람들의 초과액 총합은?",
        grader: "L1",
        solution: "team = list_team(\"eng\")
budget = get_budget(\"Q3\")
total = 0
for m in team:
    spent = get_expenses(m.id, \"Q3\")
    if spent > budget:
        total = total + spent - budget
emit(total)",
    },
    SuiteEntry {
        id: "hard_eng_annual_total",
        tier: "hard",
        domains: &["hr", "finance"],
        question: "엔지니어링 팀 전원의 연간 출장 지출 총합은?",
        grader: "L1",
        solution: "team = list_team(\"eng\")
total = 0
for m in team:
    total = total + get_expenses(m.id)
emit(total)",
    },
    // ── Schedule ──
    SuiteEntry {
        id: "easy_alice_meeting_hours",
        tier: "easy",
        domains: &["schedule"],
        question: "Alice의 회의 시간 총합은 몇 시간이야?",
        grader: "L1",
        solution: "events = list_events(\"Alice\")
total = 0
for e in events:
    total = total + e.hours
emit(total)",
    },
    SuiteEntry {
        id: "easy_carol_meeting_count",
        tier: "easy",
        domains: &["schedule"],
        question: "Carol의 회의는 몇 개야?",
        grader: "L1",
        solution: "events = list_events(\"Carol\")
n = 0
for e in events:
    n = n + 1
emit(n)",
    },
    SuiteEntry {
        id: "medium_first_event_title",
        tier: "medium",
        domains: &["schedule"],
        question: "Alice의 첫 번째 회의 제목은 무엇이야?",
        grader: "L1",
        solution: "events = list_events(\"Alice\")
emit(events[0].title)",
    },
    SuiteEntry {
        id: "medium_frank_meeting_hours",
        tier: "medium",
        domains: &["schedule"],
        question: "Frank의 회의 시간 총합은?",
        grader: "L1",
        solution: "events = list_events(\"Frank\")
total = 0
for e in events:
    total = total + e.hours
emit(total)",
    },
    SuiteEntry {
        id: "medium_eng_total_meeting_hours",
        tier: "medium",
        domains: &["hr", "schedule"],
        question: "엔지니어링 팀 전원의 회의 시간 총합은?",
        grader: "L1",
        solution: "team = list_team(\"eng\")
total = 0
for m in team:
    events = list_events(m.name)
    for e in events:
        total = total + e.hours
emit(total)",
    },
    SuiteEntry {
        id: "hard_busiest_eng_member",
        tier: "hard",
        domains: &["hr", "schedule"],
        question: "엔지니어링 팀에서 회의 시간이 가장 많은 사람의 이름은?",
        grader: "L1",
        solution: "team = list_team(\"eng\")
best = \"\"
best_hours = 0
for m in team:
    events = list_events(m.name)
    hours = 0
    for e in events:
        hours = hours + e.hours
    if hours > best_hours:
        best = m.name
        best_hours = hours
emit(best)",
    },
    SuiteEntry {
        id: "hard_eng_multi_event_count",
        tier: "hard",
        domains: &["hr", "schedule"],
        question: "엔지니어링 팀에서 회의가 2개 이상인 사람은 몇 명이야?",
        grader: "L1",
        solution: "team = list_team(\"eng\")
n = 0
for m in team:
    events = list_events(m.name)
    c = 0
    for e in events:
        c = c + 1
    if c >= 2:
        n = n + 1
emit(n)",
    },
    // ── L2 (절차/트레이스 채점) ──
    SuiteEntry {
        id: "hard_email_over_budget",
        tier: "hard",
        domains: &["hr", "finance"],
        question: "엔지니어링 팀에서 Q3 예산을 초과한 사람 각각에게 이메일을 보내줘.",
        grader: "L2",
        solution: "team = list_team(\"eng\")
budget = get_budget(\"Q3\")
for m in team:
    spent = get_expenses(m.id, \"Q3\")
    if spent > budget:
        send_email(m.name, \"over budget\")
emit(0)",
    },
    SuiteEntry {
        id: "hard_email_each_eng",
        tier: "hard",
        domains: &["hr"],
        question: "엔지니어링 팀 전원에게 회의 요약 이메일을 보내줘.",
        grader: "L2",
        solution: "team = list_team(\"eng\")
for m in team:
    send_email(m.name, \"summary\")
emit(0)",
    },
];

/// 레퍼런스 해법 실행 결과: 최종 출력과 도구 호출 트레이스.
pub struct SolutionRun {
    pub output: Option<Value>,
    pub trace: Vec<ToolCall>,
}

/// 레퍼런스 해법을 mock 위에서 실행한다. 실패(파싱/검증/런타임)는 곧 스위트 결함이다.
pub fn run_solution(solution: &str) -> Result<SolutionRun, String> {
    let tokens = tokenize(solution).map_err(|e| format!("lex: {e}"))?;
    let program = parse(tokens).map_err(|e| format!("parse: {e}"))?;
    let catalog = ToolCatalog::new(tool_names());
    validate(&program, &catalog).map_err(|e| format!("validate: {e}"))?;
    let mut server = MockToolServer::new();
    let output = Interpreter::new(&mut server)
        .run(&program)
        .map_err(|e| format!("runtime: {e}"))?;
    Ok(SolutionRun {
        output,
        trace: server.trace().to_vec(),
    })
}

/// 태스크 id로 레퍼런스 해법을 찾는다(게이트의 mock 주입용).
pub fn solution_for(id: &str) -> Option<&'static str> {
    SUITE
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.solution)
}

/// 레퍼런스 해법을 응답으로 내는 결정론적 mock provider(M3 게이트용).
///
/// 러너는 user 프롬프트로 질문을 싣고, 이 provider는 질문으로 해법을 라우팅한다.
/// 한 질문이 다른 질문의 부분문자열일 때 오라우팅을 막으려 **긴 질문부터** 등록한다
/// (MockProvider는 등록 순서 우선·부분문자열 매칭).
pub fn reference_provider(name: &str) -> MockProvider {
    let mut entries: Vec<&SuiteEntry> = SUITE.iter().collect();
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.question.len()));
    let mut provider = MockProvider::new().with_name(name);
    for entry in entries {
        provider = provider.respond_to(entry.question, entry.solution);
    }
    provider
}

/// baseline 1.0(ReAct)을 결정론적으로 구동하는 mock provider(M4 게이트용).
///
/// 각 태스크의 레퍼런스 해법 트레이스에서 ReAct 스크립트를 도출한다:
/// 트레이스의 도구 호출 하나하나가 `CALL`이 되고, 마지막에 레퍼런스 출력으로 `FINAL`.
/// 그래서 baseline은 PTC와 **같은 도구 호출 순서**를 재현하되, 호출마다 LLM을 다시
/// 부른다(LLM 호출 = 도구 호출 + 1). 질문으로 태스크를, 관측 수로 단계를 식별한다.
pub struct ScriptedBaselineProvider {
    name: String,
    /// (질문, 렌더된 행동들). 질문 부분문자열 오라우팅을 막으려 긴 질문부터 둔다.
    scripts: Vec<(String, Vec<String>)>,
}

impl ScriptedBaselineProvider {
    fn script_for(&self, user: &str) -> Option<&[String]> {
        self.scripts
            .iter()
            .find(|(question, _)| user.contains(question))
            .map(|(_, script)| script.as_slice())
    }
}

impl LlmProvider for ScriptedBaselineProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn complete(&self, req: CompletionReq) -> Result<CompletionResp, LlmError> {
        let script = self.script_for(&req.user).ok_or_else(|| {
            LlmError::Transport("baseline: 질문에 맞는 스크립트 없음".to_string())
        })?;
        let step = req.user.matches("OBSERVATION ").count();
        let text = script
            .get(step)
            .or_else(|| script.last())
            .cloned()
            .unwrap_or_default();
        Ok(CompletionResp {
            input_tokens: estimate_tokens(&req.system) + estimate_tokens(&req.user),
            output_tokens: estimate_tokens(&text),
            text,
            stop_reason: "end_turn".to_string(),
            latency_ms: 0,
        })
    }
}

/// 레퍼런스 트레이스로부터 baseline ReAct provider를 만든다.
pub fn baseline_provider(name: &str) -> ScriptedBaselineProvider {
    let mut scripts: Vec<(String, Vec<String>)> = SUITE
        .iter()
        .map(|entry| {
            (
                entry.question.to_string(),
                script_for_solution(entry.solution),
            )
        })
        .collect();
    scripts.sort_by_key(|(question, _)| std::cmp::Reverse(question.len()));
    ScriptedBaselineProvider {
        name: name.to_string(),
        scripts,
    }
}

/// 한 해법의 트레이스를 ReAct 행동 시퀀스(CALL... 다음 FINAL)로 렌더한다.
fn script_for_solution(solution: &str) -> Vec<String> {
    let run = run_solution(solution).expect("스위트 해법은 실행 가능(suite 테스트가 보장)");
    let mut actions: Vec<String> = run
        .trace
        .iter()
        .map(|call| {
            Action::Call {
                tool: call.tool.clone(),
                args: call.args.clone(),
            }
            .render()
        })
        .collect();
    actions.push(
        Action::Final {
            value: run.output.unwrap_or(Value::Null),
        }
        .render(),
    );
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn suite_has_at_least_twenty_tasks() {
        assert!(SUITE.len() >= 20, "스위트가 20개 미만: {}", SUITE.len());
    }

    #[test]
    fn task_ids_are_unique() {
        let ids: BTreeSet<&str> = SUITE.iter().map(|e| e.id).collect();
        assert_eq!(ids.len(), SUITE.len(), "중복 id 존재");
    }

    #[test]
    fn every_tier_and_domain_is_represented() {
        let tiers: BTreeSet<&str> = SUITE.iter().map(|e| e.tier).collect();
        assert_eq!(tiers, BTreeSet::from(["easy", "medium", "hard"]));
        let domains: BTreeSet<&str> = SUITE
            .iter()
            .flat_map(|e| e.domains.iter().copied())
            .collect();
        assert!(domains.contains("hr"));
        assert!(domains.contains("finance"));
        assert!(domains.contains("schedule"));
    }

    #[test]
    fn reference_provider_routes_each_question_to_its_solution() {
        use ptc_llm::{CompletionReq, LlmProvider};
        let provider = reference_provider("mock-test");
        assert_eq!(provider.name(), "mock-test");
        for entry in SUITE {
            let resp = provider
                .complete(CompletionReq::new("sys", entry.question))
                .unwrap();
            assert_eq!(
                resp.text, entry.solution,
                "[{}] 질문이 해법으로 라우팅되지 않음",
                entry.id
            );
        }
    }

    #[test]
    fn baseline_provider_drives_react_to_correct_answer() {
        use crate::baseline::BaselineRunner;
        use crate::task::load_tasks_dir;
        use std::path::Path;

        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tasks");
        let tasks = load_tasks_dir(&dir).unwrap();
        let task = tasks
            .iter()
            .find(|t| t.id == "medium_eng_q3_total")
            .expect("태스크 존재");

        let provider = baseline_provider("mock-baseline");
        let catalog = ToolCatalog::new(tool_names());
        let result = BaselineRunner::new(&catalog).run_once(task, &provider, 0, Some(42), 0.0);

        assert!(result.passed(), "baseline이 정답에 도달해야 함");
        // list_team×1 + get_expenses×4 = 5 도구 호출 → LLM 호출 6회(PTC는 1회).
        assert_eq!(result.record.metrics.tool_calls, 5);
        assert_eq!(result.record.metrics.llm_calls, 6);
    }

    #[test]
    fn every_solution_runs_cleanly() {
        for entry in SUITE {
            let run = run_solution(entry.solution)
                .unwrap_or_else(|e| panic!("[{}] 해법 실행 실패: {e}", entry.id));
            // L1은 출력이, L2는 트레이스가 있어야 의미가 있다.
            match entry.grader {
                "L1" => assert!(run.output.is_some(), "[{}] L1인데 emit 없음", entry.id),
                "L2" => assert!(!run.trace.is_empty(), "[{}] L2인데 호출 없음", entry.id),
                other => panic!("[{}] 알 수 없는 grader {other}", entry.id),
            }
        }
    }
}
