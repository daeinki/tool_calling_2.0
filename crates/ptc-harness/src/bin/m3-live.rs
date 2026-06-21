//! M3 라이브 비교 — 실제 provider들로 골든 스위트를 실행하고 통과율 표를 낸다.
//!
//! 수동/시크릿 전용. 사용 가능한 provider만 자동 선택한다:
//! - Anthropic: `ANTHROPIC_API_KEY` 있을 때
//! - OpenAI: `OPENAI_API_KEY` 있을 때
//! - Ollama(로컬): `OLLAMA_HOST` 설정 시(예: `OLLAMA_HOST=http://localhost:11434`)
//!
//! 사용: `ANTHROPIC_API_KEY=... cargo run -p ptc-harness --bin m3-live`

use ptc_analyze::{parse_jsonl, pass_rate_table, render_failure_distribution};
use ptc_harness::batch::{run_batch, BatchConfig};
use ptc_harness::record::to_jsonl;
use ptc_harness::task::load_tasks_dir;
use ptc_llm::{AnthropicProvider, LlmProvider, OllamaProvider, OpenAiProvider};
use ptc_tools::tool_names;
use std::path::Path;
use std::process::ExitCode;

/// 라이브는 비용이 있으므로 반복은 작게(필요 시 코드에서 조정).
const LIVE_REPEATS: u32 = 3;

fn main() -> ExitCode {
    let providers = available_providers();
    if providers.is_empty() {
        eprintln!(
            "사용 가능한 provider 없음. ANTHROPIC_API_KEY / OPENAI_API_KEY / OLLAMA_HOST 중 하나 이상 설정."
        );
        return ExitCode::FAILURE;
    }
    eprintln!(
        "provider: {}",
        providers
            .iter()
            .map(|p| p.name())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let tasks = match load_tasks_dir(Path::new("tasks")) {
        Ok(tasks) => tasks,
        Err(err) => {
            eprintln!("태스크 로드 실패: {err}");
            return ExitCode::FAILURE;
        }
    };

    let refs: Vec<&dyn LlmProvider> = providers.iter().map(|p| p.as_ref()).collect();
    let catalog = ptc_dsl::ToolCatalog::new(tool_names());
    let outcome = run_batch(
        &refs,
        &tasks,
        &catalog,
        &BatchConfig {
            repeats: LIVE_REPEATS,
            seed: Some(42),
        },
    );

    let jsonl = to_jsonl(&outcome.records());
    let _ = std::fs::create_dir_all("results");
    if let Err(err) = std::fs::write("results/m3-live.jsonl", &jsonl) {
        eprintln!("RunRecord 적재 실패: {err}");
    }

    let rows = match parse_jsonl(&jsonl) {
        Ok(rows) => rows,
        Err(err) => {
            eprintln!("자체 JSONL 파싱 실패: {err}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "== M3 라이브 비교 ==\n\n태스크: {} · provider: {} · 반복: {}\n",
        tasks.len(),
        providers.len(),
        LIVE_REPEATS,
    );
    print!("{}", pass_rate_table(&rows).render());
    println!();
    print!("{}", render_failure_distribution(&rows));

    ExitCode::SUCCESS
}

/// 환경에서 구성 가능한 provider만 모은다.
fn available_providers() -> Vec<Box<dyn LlmProvider>> {
    let mut providers: Vec<Box<dyn LlmProvider>> = Vec::new();
    if let Ok(provider) = AnthropicProvider::from_env() {
        providers.push(Box::new(provider));
    }
    if let Ok(provider) = OpenAiProvider::from_env() {
        providers.push(Box::new(provider));
    }
    if std::env::var("OLLAMA_HOST").is_ok() {
        providers.push(Box::new(OllamaProvider::from_env()));
    }
    providers
}
