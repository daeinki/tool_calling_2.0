//! M1 게이트 CI 진입점. 요약을 출력하고, 실패 시 비정상 종료한다.

use std::process::ExitCode;

fn main() -> ExitCode {
    let outcome = ptc_harness::gate::run_gate();
    print!("{}", outcome.report());
    if outcome.passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
