//! M2 라이브 게이트 — 실제 Anthropic provider로 골든 태스크를 R회 실행한다.
//!
//! 수동/CI 시크릿 전용. `ANTHROPIC_API_KEY`가 필요하며, 라이브 통과 기준은
//! 통과율 ≥ 0.8 + HARNESS_BUG 0건이다(설계 M4 게이트의 M2 버전).
//!
//! 사용: `ANTHROPIC_API_KEY=... cargo run -p ptc-harness --bin m2-live`

use std::process::ExitCode;

const LIVE_PASS_THRESHOLD: f64 = 0.8;

fn main() -> ExitCode {
    let provider = match ptc_llm::AnthropicProvider::from_env() {
        Ok(provider) => provider,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("model = {}", provider.model());

    let outcome = ptc_harness::gate::run_m2_gate_with(&provider);
    print!("{}", outcome.report());

    if outcome.error.is_none() {
        let _ = std::fs::create_dir_all("results");
        let jsonl = ptc_harness::record::to_jsonl(&outcome.records());
        if let Err(err) = std::fs::write("results/m2-live.jsonl", jsonl) {
            eprintln!("RunRecord 적재 실패: {err}");
        }
    }

    if outcome.passed_at(LIVE_PASS_THRESHOLD) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
