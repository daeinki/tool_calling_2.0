//! M2 게이트 CI 진입점. Mock 경로로 골든 태스크를 실행해 통과율·HARNESS_BUG를
//! 판정하고, RunRecord를 results/m2.jsonl로 적재한 뒤 실패 시 비정상 종료한다.

use std::process::ExitCode;

fn main() -> ExitCode {
    let outcome = ptc_harness::gate::run_m2_gate();
    print!("{}", outcome.report());

    if outcome.error.is_none() {
        let _ = std::fs::create_dir_all("results");
        let jsonl = ptc_harness::record::to_jsonl(&outcome.records());
        if let Err(err) = std::fs::write("results/m2.jsonl", jsonl) {
            eprintln!("RunRecord 적재 실패: {err}");
        }
    }

    if outcome.passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
