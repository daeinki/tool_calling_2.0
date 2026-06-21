//! M3 게이트 CI 진입점. 골든 스위트를 Mock 3종으로 실행해 통과율 표를 재현·판정하고,
//! RunRecord를 results/m3.jsonl로 적재한 뒤 실패 시 비정상 종료한다.

use std::process::ExitCode;

fn main() -> ExitCode {
    let outcome = ptc_harness::gate::run_m3_gate();
    print!("{}", outcome.report());

    if outcome.error.is_none() {
        let _ = std::fs::create_dir_all("results");
        if let Err(err) = std::fs::write("results/m3.jsonl", &outcome.jsonl) {
            eprintln!("RunRecord 적재 실패: {err}");
        }
    }

    if outcome.passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
