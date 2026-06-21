//! `tasks/*.toml` 생성기 (M3-T03 보조).
//!
//! [`suite::SUITE`]의 레퍼런스 해법을 실행해 기대 출력·기대 호출을 도출하고,
//! 선언적 태스크 파일을 써 낸다. 기대값을 손으로 적지 않으므로 산수 실수가 없다.
//! 한 번 실행해 산출물을 커밋한다: `cargo run -p ptc-harness --bin gen-tasks`.

use ptc_dsl::Value;
use ptc_harness::suite::{run_solution, SuiteEntry, SUITE};
use std::collections::BTreeMap;
use std::process::ExitCode;

fn main() -> ExitCode {
    if let Err(err) = std::fs::create_dir_all("tasks") {
        eprintln!("tasks/ 생성 실패: {err}");
        return ExitCode::FAILURE;
    }
    for entry in SUITE {
        let run = match run_solution(entry.solution) {
            Ok(run) => run,
            Err(err) => {
                eprintln!("[{}] 해법 실행 실패: {err}", entry.id);
                return ExitCode::FAILURE;
            }
        };
        let toml = render_task(entry, &run.output, count_calls(&run.trace));
        let path = format!("tasks/{}.toml", entry.id);
        if let Err(err) = std::fs::write(&path, toml) {
            eprintln!("{path} 쓰기 실패: {err}");
            return ExitCode::FAILURE;
        }
        println!("wrote {path}");
    }
    println!("총 {}개 태스크 생성", SUITE.len());
    ExitCode::SUCCESS
}

/// 트레이스를 도구별 호출 횟수로 집계한다(도구명 사전순).
fn count_calls(trace: &[ptc_tools::ToolCall]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for call in trace {
        *counts.entry(call.tool.clone()).or_insert(0) += 1;
    }
    counts
}

/// 한 태스크의 TOML 문서를 만든다. 기대 출력은 L1에서만 싣고, 기대 호출은 항상 싣는다.
fn render_task(
    entry: &SuiteEntry,
    output: &Option<Value>,
    calls: BTreeMap<String, usize>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# 골든 태스크 (M3) — 자동 생성(gen-tasks). 기대값은 레퍼런스 해법 실행에서 도출.\n# tier={} · domains={} · grader={}\n",
        entry.tier,
        entry.domains.join(","),
        entry.grader,
    ));
    out.push_str(&format!("id = {}\n", toml_str(entry.id)));
    out.push_str(&format!("tier = {}\n", toml_str(entry.tier)));
    out.push_str(&format!("domains = {}\n", toml_str_array(entry.domains)));
    out.push_str(&format!("question = {}\n", toml_str(entry.question)));
    out.push_str(&format!("grader = {}\n", toml_str(entry.grader)));

    if entry.grader == "L1" {
        let value = output
            .as_ref()
            .expect("L1 태스크는 emit 출력이 있어야 한다(suite 테스트가 보장)");
        out.push_str(&format!("expected_output = {}\n", value_to_toml(value)));
    }

    for (tool, count) in calls {
        out.push_str("\n[[expected_tool_calls]]\n");
        out.push_str(&format!("tool = {}\n", toml_str(&tool)));
        out.push_str(&format!("count = {count}\n"));
    }
    out
}

/// 인터프리터 [`Value`]를 TOML 리터럴로 렌더한다. 기대 출력에 나타날 수 있는
/// 종류(수·문자열·불·리스트)만 지원하고, 그 밖은 스위트 결함이므로 패닉.
fn value_to_toml(value: &Value) -> String {
    match value {
        // f64는 항상 소수점을 포함해 TOML float로 파싱되게 한다(`13000.0`).
        Value::Num(n) => format!("{n:?}"),
        Value::Str(s) => toml_str(s),
        Value::Bool(b) => b.to_string(),
        Value::List(items) => {
            let rendered: Vec<String> = items.iter().map(value_to_toml).collect();
            format!("[{}]", rendered.join(", "))
        }
        other => panic!("기대 출력으로 부적절한 값: {other:?}"),
    }
}

/// TOML 기본 문자열 리터럴(따옴표·역슬래시 이스케이프).
fn toml_str(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn toml_str_array(items: &[&str]) -> String {
    let rendered: Vec<String> = items.iter().map(|s| toml_str(s)).collect();
    format!("[{}]", rendered.join(", "))
}
