//! 골든 픽스처 — 손으로 작성한 DSL 스크립트와 그 기대 결과 (M1-T10).
//!
//! LLM 없이 인터프리터의 정확성을 고정하기 위한 데이터다. 각 픽스처는
//! **스크립트(데이터)**와 **사전 계산된 기대 트레이스·출력(데이터)**으로만 이뤄지며,
//! 실행·비교 로직([`run_fixture`])은 이들과 분리된다(데이터/로직 분리).
//!
//! 기대값은 [`ptc_tools::MockToolServer`]의 고정 데이터로부터 손으로 계산했다.
//! (분기 지출 = id*1000 + 분기지수*250 + 250, 분기 예산 = 1500 + 분기지수*500.)

use ptc_dsl::{parse, tokenize, Interpreter, ToolCatalog, Value};
use ptc_tools::{tool_names, MockToolServer, ToolCall};
use std::collections::BTreeMap;

/// 난이도 계층.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Easy,
    Medium,
    Hard,
}

/// 도구 한 종에 대한 기대 호출 횟수.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedCall {
    pub tool: &'static str,
    pub count: usize,
}

/// 스크립트 하나와 그 기대 결과(트레이스 요약 + 최종 출력).
pub struct Fixture {
    pub name: &'static str,
    pub tier: Tier,
    pub source: &'static str,
    /// `emit`된 최종값. `emit`이 없으면 `None`.
    pub expected_output: Option<Value>,
    /// 도구별 기대 호출 횟수(여기 없는 도구는 0회여야 한다).
    pub expected_calls: Vec<ExpectedCall>,
}

/// 픽스처 한 건을 실행한 결과.
#[derive(Debug)]
pub struct FixtureReport {
    pub name: String,
    pub output_ok: bool,
    pub calls_ok: bool,
    pub actual_output: Option<Value>,
    pub actual_calls: BTreeMap<String, usize>,
    /// 파이프라인이 예기치 않게 실패하면 그 메시지(정상 픽스처는 항상 `None`).
    pub error: Option<String>,
}

impl FixtureReport {
    pub fn passed(&self) -> bool {
        self.error.is_none() && self.output_ok && self.calls_ok
    }
}

/// 픽스처를 lex → parse → validate → interpret 전 경로로 실행하고 기대값과 비교한다.
pub fn run_fixture(fixture: &Fixture) -> FixtureReport {
    let mut report = FixtureReport {
        name: fixture.name.to_string(),
        output_ok: false,
        calls_ok: false,
        actual_output: None,
        actual_calls: BTreeMap::new(),
        error: None,
    };

    let program = match compile(fixture.source) {
        Ok(program) => program,
        Err(message) => {
            report.error = Some(message);
            return report;
        }
    };

    let mut server = MockToolServer::new();
    let output = match Interpreter::new(&mut server).run(&program) {
        Ok(output) => output,
        Err(err) => {
            report.error = Some(err.to_string());
            return report;
        }
    };

    report.actual_calls = count_calls(server.trace());
    report.calls_ok = report.actual_calls == expected_map(&fixture.expected_calls);
    report.output_ok = output == fixture.expected_output;
    report.actual_output = output;
    report
}

/// lex → parse → validate. 실패 메시지를 문자열로 합친다.
fn compile(source: &str) -> Result<Vec<ptc_dsl::Stmt>, String> {
    let tokens = tokenize(source).map_err(|e| e.to_string())?;
    let program = parse(tokens).map_err(|e| e.to_string())?;
    let catalog = ToolCatalog::new(tool_names());
    ptc_dsl::validate(&program, &catalog).map_err(|e| e.to_string())?;
    Ok(program)
}

fn count_calls(trace: &[ToolCall]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for call in trace {
        *counts.entry(call.tool.clone()).or_insert(0) += 1;
    }
    counts
}

fn expected_map(expected: &[ExpectedCall]) -> BTreeMap<String, usize> {
    expected
        .iter()
        .map(|c| (c.tool.to_string(), c.count))
        .collect()
}

/// 10개의 골든 픽스처(easy 3 · medium 4 · hard 3).
pub fn fixtures() -> Vec<Fixture> {
    vec![
        // ── Easy ──
        Fixture {
            name: "emit_literal",
            tier: Tier::Easy,
            source: "emit(42)",
            expected_output: Some(Value::Num(42.0)),
            expected_calls: calls(&[]),
        },
        Fixture {
            name: "first_member_name",
            tier: Tier::Easy,
            source: "team = list_team(\"eng\")\nemit(team[0].name)",
            expected_output: Some(text("Alice")),
            expected_calls: calls(&[("list_team", 1)]),
        },
        Fixture {
            name: "budget_lookup",
            tier: Tier::Easy,
            source: "emit(get_budget(\"Q3\"))",
            expected_output: Some(Value::Num(2500.0)),
            expected_calls: calls(&[("get_budget", 1)]),
        },
        // ── Medium ──
        Fixture {
            name: "branch_on_budget",
            tier: Tier::Medium,
            source: "budget = get_budget(\"Q1\")\nspent = get_expenses(2, \"Q1\")\nif spent > budget:\n    emit(\"over\")\nelse:\n    emit(\"under\")",
            expected_output: Some(text("over")),
            expected_calls: calls(&[("get_budget", 1), ("get_expenses", 1)]),
        },
        Fixture {
            name: "sum_q3_expenses_eng",
            tier: Tier::Medium,
            source: "team = list_team(\"eng\")\ntotal = 0\nfor m in team:\n    total = total + get_expenses(m.id, \"Q3\")\nemit(total)",
            expected_output: Some(Value::Num(13000.0)),
            expected_calls: calls(&[("list_team", 1), ("get_expenses", 4)]),
        },
        Fixture {
            name: "count_over_budget_eng",
            tier: Tier::Medium,
            source: "team = list_team(\"eng\")\nbudget = get_budget(\"Q3\")\ncount = 0\nfor m in team:\n    spent = get_expenses(m.id, \"Q3\")\n    if spent > budget:\n        count = count + 1\nemit(count)",
            expected_output: Some(Value::Num(3.0)),
            expected_calls: calls(&[("list_team", 1), ("get_budget", 1), ("get_expenses", 4)]),
        },
        Fixture {
            name: "email_over_budget_no_emit",
            tier: Tier::Medium,
            source: "team = list_team(\"eng\")\nbudget = get_budget(\"Q3\")\nfor m in team:\n    spent = get_expenses(m.id, \"Q3\")\n    if spent > budget:\n        send_email(to=m.name, body=\"over budget\")",
            expected_output: None,
            expected_calls: calls(&[
                ("list_team", 1),
                ("get_budget", 1),
                ("get_expenses", 4),
                ("send_email", 3),
            ]),
        },
        // ── Hard ──
        Fixture {
            name: "nested_call_argument",
            tier: Tier::Hard,
            source: "emit(get_expenses(list_team(\"eng\")[0].id, \"Q3\"))",
            expected_output: Some(Value::Num(1750.0)),
            expected_calls: calls(&[("list_team", 1), ("get_expenses", 1)]),
        },
        Fixture {
            name: "cross_reference_two_depts",
            tier: Tier::Hard,
            source: "eng = list_team(\"eng\")\nsales = list_team(\"sales\")\ntotal = 0\nfor m in eng:\n    total = total + get_expenses(m.id, \"Q1\")\nfor m in sales:\n    total = total + get_expenses(m.id, \"Q1\")\nemit(total)",
            expected_output: Some(Value::Num(22500.0)),
            expected_calls: calls(&[("list_team", 2), ("get_expenses", 6)]),
        },
        Fixture {
            name: "nested_loops_over_depts",
            tier: Tier::Hard,
            source: "total = 0\nfor d in [\"eng\", \"sales\"]:\n    for m in list_team(d):\n        total = total + get_expenses(m.id, \"Q2\")\nemit(total)",
            expected_output: Some(Value::Num(24000.0)),
            expected_calls: calls(&[("list_team", 2), ("get_expenses", 6)]),
        },
    ]
}

fn calls(pairs: &[(&'static str, usize)]) -> Vec<ExpectedCall> {
    pairs
        .iter()
        .map(|(tool, count)| ExpectedCall {
            tool,
            count: *count,
        })
        .collect()
}

fn text(s: &str) -> Value {
    Value::Str(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_ten_fixtures_spanning_all_tiers() {
        let all = fixtures();
        assert_eq!(all.len(), 10);
        for tier in [Tier::Easy, Tier::Medium, Tier::Hard] {
            assert!(
                all.iter().any(|f| f.tier == tier),
                "no fixture for {tier:?}"
            );
        }
    }

    #[test]
    fn fixture_names_are_unique() {
        let all = fixtures();
        let mut names: Vec<&str> = all.iter().map(|f| f.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all.len());
    }

    #[test]
    fn every_fixture_matches_its_expected_output_and_trace() {
        for fixture in fixtures() {
            let report = run_fixture(&fixture);
            assert!(
                report.passed(),
                "fixture '{}' failed: error={:?} output_ok={} (got {:?}) calls_ok={} (got {:?})",
                report.name,
                report.error,
                report.output_ok,
                report.actual_output,
                report.calls_ok,
                report.actual_calls,
            );
        }
    }
}
