//! M4 게이트 CI 진입점. PTC 2.0과 baseline 1.0(ReAct)을 골든 스위트에서 비교해
//! 정확성 비열등(McNemar)과 토큰·호출 절감(부트스트랩 CI)을 판정하고, RunRecord를
//! results/m4.jsonl로 적재한 뒤 실패 시 비정상 종료한다.

use std::process::ExitCode;

fn main() -> ExitCode {
    let outcome = ptc_harness::gate::run_m4_gate();
    print!("{}", outcome.report());

    if outcome.error.is_none() {
        let _ = std::fs::create_dir_all("results");
        if let Err(err) = std::fs::write("results/m4.jsonl", &outcome.jsonl) {
            eprintln!("RunRecord 적재 실패: {err}");
        }
    }

    if outcome.passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
