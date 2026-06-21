//! M1 게이트 러너 — 골든 픽스처(T10)와 음성 스위트(T11)를 한 번에 판정한다 (M1-T12).
//!
//! 게이트 통과 조건(자동 판정):
//! 1. 골든 픽스처 10개가 100% 기대 트레이스·출력과 일치.
//! 2. 음성 케이스가 모두 기대 단계에서 거부됨.
//!
//! (커버리지 ≥80%는 `cargo llvm-cov`로 별도 측정한다 — 코드가 아닌 도구의 몫.)

use crate::batch::{run_baseline_batch, run_batch, BatchConfig, BatchOutcome};
use crate::fixtures::{self, FixtureReport};
use crate::negative::{self, NegativeReport};
use crate::record::{to_jsonl, RunRecord};
use crate::runner::{pass_rate, RunResult, Runner};
use crate::suite::{baseline_provider, reference_provider};
use crate::task::{load_task, load_tasks_dir};
use crate::taxonomy::FailureCategory;
use ptc_analyze::stats::{bootstrap_ratio_ci, mcnemar, Ci, McNemar};
use ptc_analyze::{
    compare_modes, parse_error_rate, parse_jsonl, pass_rate_table, render_failure_distribution,
    PassRateTable,
};
use ptc_dsl::ToolCatalog;
use ptc_llm::{LlmProvider, MockProvider};
use ptc_tools::tool_names;
use std::path::{Path, PathBuf};

/// 게이트 실행 결과.
pub struct GateOutcome {
    pub fixtures: Vec<FixtureReport>,
    pub negatives: Vec<NegativeReport>,
}

impl GateOutcome {
    pub fn fixtures_passed(&self) -> usize {
        self.fixtures.iter().filter(|r| r.passed()).count()
    }

    pub fn negatives_passed(&self) -> usize {
        self.negatives
            .iter()
            .filter(|r| r.rejected_as_expected())
            .count()
    }

    /// 모든 픽스처가 일치하고 모든 음성 케이스가 거부되면 게이트 통과.
    pub fn passed(&self) -> bool {
        self.fixtures_passed() == self.fixtures.len()
            && self.negatives_passed() == self.negatives.len()
    }

    /// 사람이 읽을 요약(CI 바이너리에서 출력).
    pub fn report(&self) -> String {
        let mut out = String::new();
        out.push_str("== M1 게이트 ==\n\n[골든 픽스처]\n");
        for report in &self.fixtures {
            out.push_str(&format!("  {} {}\n", mark(report.passed()), report.name));
        }
        out.push_str("\n[음성 케이스]\n");
        for report in &self.negatives {
            out.push_str(&format!(
                "  {} {} (기대 {})\n",
                mark(report.rejected_as_expected()),
                report.name,
                report.expected
            ));
        }
        out.push_str(&format!(
            "\n픽스처 {}/{} · 음성 {}/{} → {}\n",
            self.fixtures_passed(),
            self.fixtures.len(),
            self.negatives_passed(),
            self.negatives.len(),
            if self.passed() { "통과" } else { "실패" },
        ));
        out
    }
}

fn mark(ok: bool) -> char {
    if ok {
        '✓'
    } else {
        '✗'
    }
}

/// 전체 스위트를 실행해 게이트 결과를 만든다.
pub fn run_gate() -> GateOutcome {
    GateOutcome {
        fixtures: fixtures::fixtures()
            .iter()
            .map(fixtures::run_fixture)
            .collect(),
        negatives: negative::cases()
            .iter()
            .map(negative::run_negative)
            .collect(),
    }
}

// ── M2 게이트 (Mock 경로) ──

/// M2 게이트 반복 횟수.
const M2_REPEATS: u32 = 5;

/// M2 게이트 결과.
pub struct M2GateOutcome {
    pub task_id: String,
    pub results: Vec<RunResult>,
    /// 게이트를 돌릴 수 없게 한 설정 오류(태스크 로드 실패 등).
    pub error: Option<String>,
}

impl M2GateOutcome {
    fn setup_error(message: String) -> Self {
        Self {
            task_id: String::new(),
            results: Vec::new(),
            error: Some(message),
        }
    }

    pub fn pass_rate(&self) -> f64 {
        pass_rate(&self.results)
    }

    /// HARNESS_BUG 건수. 1건이라도 있으면 게이트 미통과(설계 4.4절).
    pub fn harness_bugs(&self) -> usize {
        self.results
            .iter()
            .filter(|r| r.category == Some(FailureCategory::HarnessBug))
            .count()
    }

    /// 통과율 1.0 + HARNESS_BUG 0건이면 게이트 통과(Mock 경로 기준).
    pub fn passed(&self) -> bool {
        self.passed_at(1.0)
    }

    /// 주어진 통과율 임계 이상 + HARNESS_BUG 0건이면 통과(라이브는 0.8 사용).
    pub fn passed_at(&self, threshold: f64) -> bool {
        self.error.is_none()
            && !self.results.is_empty()
            && self.pass_rate() >= threshold
            && self.harness_bugs() == 0
    }

    /// 적재용 RunRecord 모음.
    pub fn records(&self) -> Vec<RunRecord> {
        self.results.iter().map(|r| r.record.clone()).collect()
    }

    pub fn report(&self) -> String {
        if let Some(error) = &self.error {
            return format!("== M2 게이트 ==\n\n설정 오류: {error}\n");
        }
        let mut out = format!(
            "== M2 게이트 (Mock) ==\n\n태스크: {}\n반복: {}\n통과율: {:.2}\nHARNESS_BUG: {}\n",
            self.task_id,
            self.results.len(),
            self.pass_rate(),
            self.harness_bugs(),
        );
        out.push_str("\n[실패 분류 분포]\n");
        for category in FailureCategory::ALL {
            let count = self
                .results
                .iter()
                .filter(|r| r.category == Some(category))
                .count();
            if count > 0 {
                out.push_str(&format!("  {} {}\n", category, count));
            }
        }
        out.push_str(&format!(
            "\n→ {}\n",
            if self.passed() { "통과" } else { "실패" }
        ));
        out
    }
}

/// 골든 태스크별 알려진 정답 코드(LLM이 생성했어야 할 결과).
fn canned_solution(task_id: &str) -> Option<&'static str> {
    match task_id {
        "easy_first_member" => Some("team = list_team(\"eng\")\nemit(team[0].name)"),
        _ => None,
    }
}

fn golden_task() -> Result<crate::task::Task, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tasks/easy_first_member.toml");
    load_task(&path).map_err(|err| format!("태스크 로드 실패: {err}"))
}

fn run_repeated_gate(task: crate::task::Task, provider: &dyn LlmProvider) -> M2GateOutcome {
    let catalog = ToolCatalog::new(tool_names());
    let results = Runner::new(&catalog).run_repeated(&task, provider, M2_REPEATS, Some(42));
    M2GateOutcome {
        task_id: task.id,
        results,
        error: None,
    }
}

/// 골든 태스크를 MockProvider(정답 코드 주입)로 R회 실행해 게이트를 판정한다.
pub fn run_m2_gate() -> M2GateOutcome {
    let task = match golden_task() {
        Ok(task) => task,
        Err(message) => return M2GateOutcome::setup_error(message),
    };
    let Some(code) = canned_solution(&task.id) else {
        return M2GateOutcome::setup_error(format!("canned 해법 없음: {}", task.id));
    };
    let provider = MockProvider::new().with_name("mock").default_response(code);
    run_repeated_gate(task, &provider)
}

/// 골든 태스크를 주어진 provider(실제 LLM 등)로 R회 실행한다(라이브 경로).
pub fn run_m2_gate_with(provider: &dyn LlmProvider) -> M2GateOutcome {
    match golden_task() {
        Ok(task) => run_repeated_gate(task, provider),
        Err(message) => M2GateOutcome::setup_error(message),
    }
}

// ── M3 게이트 (스위트 · Mock 3종) ──

/// M3 게이트 반복 횟수.
const M3_REPEATS: u32 = 5;
/// 결정론적 mock 3종(이름만 다름) — provider 비교의 자동 판정용 대역.
const M3_PROVIDERS: [&str; 3] = ["mock-anthropic", "mock-openai", "mock-ollama"];
/// 문법 부족으로 인한 PARSE_ERROR 허용 상한(설계 M3 게이트). 초과 시 문법 확장 트리거.
const PARSE_ERROR_THRESHOLD: f64 = 0.10;

/// M3 게이트 결과.
pub struct M3GateOutcome {
    pub task_count: usize,
    pub jsonl: String,
    pub table: PassRateTable,
    /// 같은 시드로 두 번 돌린 표가 동일한가(재현성).
    pub reproduced: bool,
    pub parse_error_rate: f64,
    pub harness_bugs: usize,
    pub overall_pass_rate: f64,
    /// 게이트를 돌릴 수 없게 한 설정 오류(태스크 로드 실패 등).
    pub error: Option<String>,
}

impl M3GateOutcome {
    fn setup_error(message: String) -> Self {
        Self {
            task_count: 0,
            jsonl: String::new(),
            table: pass_rate_table(&[]),
            reproduced: false,
            parse_error_rate: 0.0,
            harness_bugs: 0,
            overall_pass_rate: 0.0,
            error: Some(message),
        }
    }

    /// 통과 조건(전부 충족):
    /// 재현됨 · PARSE_ERROR < 10% · HARNESS_BUG 0건 · 레퍼런스 해법 전부 통과(1.0).
    pub fn passed(&self) -> bool {
        self.error.is_none()
            && self.reproduced
            && self.parse_error_rate < PARSE_ERROR_THRESHOLD
            && self.harness_bugs == 0
            && self.overall_pass_rate == 1.0
    }

    pub fn report(&self) -> String {
        if let Some(error) = &self.error {
            return format!("== M3 게이트 ==\n\n설정 오류: {error}\n");
        }
        let rows = parse_jsonl(&self.jsonl).unwrap_or_default();
        let mut out = format!(
            "== M3 게이트 (Mock 3종) ==\n\n태스크: {} · provider: {} · 반복: {}\n\n",
            self.task_count,
            M3_PROVIDERS.len(),
            M3_REPEATS,
        );
        out.push_str(&self.table.render());
        out.push('\n');
        out.push_str(&render_failure_distribution(&rows));
        out.push_str(&format!(
            "\n재현성: {} · PARSE_ERROR: {:.1}% (< {:.0}%) · HARNESS_BUG: {} · 전체 통과율: {:.2}\n",
            if self.reproduced { "일치" } else { "불일치" },
            self.parse_error_rate * 100.0,
            PARSE_ERROR_THRESHOLD * 100.0,
            self.harness_bugs,
            self.overall_pass_rate,
        ));
        out.push_str(&format!(
            "\n→ {}\n",
            if self.passed() { "통과" } else { "실패" }
        ));
        out
    }
}

fn tasks_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tasks")
}

/// 골든 스위트를 Mock 3종으로 R회 실행하고 통과율 표를 재현·검증한다(M3-T08).
pub fn run_m3_gate() -> M3GateOutcome {
    let tasks = match load_tasks_dir(&tasks_dir()) {
        Ok(tasks) => tasks,
        Err(err) => return M3GateOutcome::setup_error(format!("태스크 로드 실패: {err}")),
    };
    let providers: Vec<MockProvider> = M3_PROVIDERS.iter().map(|n| reference_provider(n)).collect();
    let provider_refs: Vec<&dyn LlmProvider> =
        providers.iter().map(|p| p as &dyn LlmProvider).collect();
    let config = BatchConfig {
        repeats: M3_REPEATS,
        seed: Some(42),
    };

    // 같은 시드로 두 번 실행해 통과율 표가 재현되는지 본다.
    let first = run_batch(&provider_refs, &tasks, &catalog(), &config);
    let second = run_batch(&provider_refs, &tasks, &catalog(), &config);
    summarize_m3(tasks.len(), first, second)
}

fn catalog() -> ToolCatalog {
    ToolCatalog::new(tool_names())
}

/// 두 배치 실행을 표·분포로 집계한다(JSONL을 거쳐 분석 — 영속 데이터로 판정).
fn summarize_m3(task_count: usize, first: BatchOutcome, second: BatchOutcome) -> M3GateOutcome {
    let jsonl = to_jsonl(&first.records());
    let rows = match parse_jsonl(&jsonl) {
        Ok(rows) => rows,
        Err(err) => return M3GateOutcome::setup_error(format!("자체 JSONL 파싱 실패: {err}")),
    };
    let table = pass_rate_table(&rows);

    let second_jsonl = to_jsonl(&second.records());
    let second_table = match parse_jsonl(&second_jsonl) {
        Ok(rows) => pass_rate_table(&rows),
        Err(err) => return M3GateOutcome::setup_error(format!("재현 JSONL 파싱 실패: {err}")),
    };

    M3GateOutcome {
        task_count,
        reproduced: table == second_table,
        parse_error_rate: parse_error_rate(&rows),
        harness_bugs: count_harness_bugs(&first),
        overall_pass_rate: pass_rate(&first.results),
        table,
        jsonl,
        error: None,
    }
}

fn count_harness_bugs(outcome: &BatchOutcome) -> usize {
    outcome
        .results
        .iter()
        .filter(|r| r.category == Some(FailureCategory::HarnessBug))
        .count()
}

// ── M4 게이트 (PTC vs baseline 1.0 비교) ──

/// M4 반복 횟수(통계적 의미를 위해 R≥10).
const M4_REPEATS: u32 = 10;
/// 부트스트랩 재표집 횟수.
const M4_BOOTSTRAP: usize = 10_000;
/// 유의수준.
const M4_ALPHA: f64 = 0.05;

/// M4 게이트 결과 — 정확성(McNemar)과 절감(부트스트랩 CI)을 함께 담는다.
pub struct M4GateOutcome {
    pub task_count: usize,
    pub repeats: u32,
    pub jsonl: String,
    pub mcnemar: McNemar,
    /// PTC/baseline LLM 호출 비율의 95% CI(상한 < 1 이면 유의한 절감).
    pub llm_call_ratio: Ci,
    /// PTC/baseline 토큰 비율의 95% CI.
    pub token_ratio: Ci,
    pub ptc_pass_rate: f64,
    pub baseline_pass_rate: f64,
    pub harness_bugs: usize,
    pub error: Option<String>,
}

impl M4GateOutcome {
    fn setup_error(message: String) -> Self {
        Self {
            task_count: 0,
            repeats: 0,
            jsonl: String::new(),
            mcnemar: mcnemar(&[]),
            llm_call_ratio: Ci {
                point: 0.0,
                lower: 0.0,
                upper: 0.0,
            },
            token_ratio: Ci {
                point: 0.0,
                lower: 0.0,
                upper: 0.0,
            },
            ptc_pass_rate: 0.0,
            baseline_pass_rate: 0.0,
            harness_bugs: 0,
            error: Some(message),
        }
    }

    /// 통과 조건(전부 충족):
    /// - R≥10, HARNESS_BUG 0, PTC가 전 태스크 통과(1.0)
    /// - Q1: McNemar로 PTC가 유의하게 악화되지 않음(비열등)
    /// - Q2: LLM 호출·토큰 절감의 95% CI 상한이 모두 < 1.0(유의한 절감)
    pub fn passed(&self) -> bool {
        self.error.is_none()
            && self.repeats >= 10
            && self.harness_bugs == 0
            && self.ptc_pass_rate == 1.0
            && !self.mcnemar.degraded(M4_ALPHA)
            && self.llm_call_ratio.upper < 1.0
            && self.token_ratio.upper < 1.0
    }

    pub fn report(&self) -> String {
        if let Some(error) = &self.error {
            return format!("== M4 게이트 ==\n\n설정 오류: {error}\n");
        }
        let savings = |ci: &Ci| (1.0 - ci.point) * 100.0;
        format!(
            "== M4 게이트 (PTC 2.0 vs baseline 1.0) ==\n\n\
             태스크: {} · 반복: {} · 부트스트랩: {}\n\n\
             [Q1 정확성 — McNemar]\n  \
             PTC만 통과: {} · baseline만 통과: {} · p={:.4} ({})\n  \
             PTC 통과율: {:.2} · baseline 통과율: {:.2}\n  \
             → PTC 비열등: {}\n\n\
             [Q2 절감 — 부트스트랩 95% CI]\n  \
             LLM 호출 비율(PTC/baseline): {:.3} [{:.3}, {:.3}] → 절감 {:.1}%\n  \
             토큰 비율(PTC/baseline):     {:.3} [{:.3}, {:.3}] → 절감 {:.1}%\n\n\
             HARNESS_BUG: {}\n\n→ {}\n",
            self.task_count,
            self.repeats,
            M4_BOOTSTRAP,
            self.mcnemar.ptc_only,
            self.mcnemar.baseline_only,
            self.mcnemar.p_value,
            if self.mcnemar.exact {
                "정확검정"
            } else {
                "카이제곱"
            },
            self.ptc_pass_rate,
            self.baseline_pass_rate,
            if self.mcnemar.degraded(M4_ALPHA) {
                "아니오"
            } else {
                "예"
            },
            self.llm_call_ratio.point,
            self.llm_call_ratio.lower,
            self.llm_call_ratio.upper,
            savings(&self.llm_call_ratio),
            self.token_ratio.point,
            self.token_ratio.lower,
            self.token_ratio.upper,
            savings(&self.token_ratio),
            self.harness_bugs,
            if self.passed() { "통과" } else { "실패" },
        )
    }
}

/// 골든 스위트를 PTC와 baseline 1.0으로 R회씩 돌려 두 모드를 통계적으로 비교한다(M4).
pub fn run_m4_gate() -> M4GateOutcome {
    let tasks = match load_tasks_dir(&tasks_dir()) {
        Ok(tasks) => tasks,
        Err(err) => return M4GateOutcome::setup_error(format!("태스크 로드 실패: {err}")),
    };
    let catalog = catalog();
    let config = BatchConfig {
        repeats: M4_REPEATS,
        seed: Some(42),
    };

    // 두 모드는 같은 도구·채점기를 쓰고, LLM 호출 방식만 다르다(공정성).
    let ptc_provider = reference_provider("mock");
    let base_provider = baseline_provider("mock");
    let ptc = run_batch(&[&ptc_provider], &tasks, &catalog, &config);
    let baseline = run_baseline_batch(&[&base_provider], &tasks, &catalog, &config);

    let mut records = ptc.records();
    records.extend(baseline.records());
    let jsonl = to_jsonl(&records);
    let rows = match parse_jsonl(&jsonl) {
        Ok(rows) => rows,
        Err(err) => return M4GateOutcome::setup_error(format!("자체 JSONL 파싱 실패: {err}")),
    };

    let comparison = compare_modes(&rows, "ptc", "baseline_1_0");
    M4GateOutcome {
        task_count: tasks.len(),
        repeats: M4_REPEATS,
        mcnemar: mcnemar(&comparison.pass_pairs),
        // 호출·토큰에 다른 시드를 줘 부트스트랩 표집을 독립시킨다.
        llm_call_ratio: bootstrap_ratio_ci(
            &comparison.ptc_llm_calls,
            &comparison.baseline_llm_calls,
            M4_BOOTSTRAP,
            42,
        ),
        token_ratio: bootstrap_ratio_ci(
            &comparison.ptc_tokens,
            &comparison.baseline_tokens,
            M4_BOOTSTRAP,
            43,
        ),
        ptc_pass_rate: pass_rate(&ptc.results),
        baseline_pass_rate: pass_rate(&baseline.results),
        harness_bugs: count_harness_bugs(&ptc) + count_harness_bugs(&baseline),
        jsonl,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m1_gate_passes() {
        let outcome = run_gate();
        assert!(outcome.passed(), "M1 게이트 실패\n{}", outcome.report());
        assert_eq!(outcome.fixtures_passed(), 10);
        assert_eq!(outcome.negatives_passed(), 10);
    }

    #[test]
    fn m2_gate_passes_on_mock_path() {
        let outcome = run_m2_gate();
        assert!(outcome.passed(), "M2 게이트 실패\n{}", outcome.report());
        assert_eq!(outcome.pass_rate(), 1.0);
        assert_eq!(outcome.harness_bugs(), 0);
        assert_eq!(outcome.results.len(), 5);
    }

    #[test]
    fn m3_gate_passes_on_mock_suite() {
        let outcome = run_m3_gate();
        assert!(outcome.passed(), "M3 게이트 실패\n{}", outcome.report());
        assert!(outcome.task_count >= 20, "스위트 20개 미만");
        assert!(outcome.reproduced, "통과율 표가 재현되지 않음");
        assert_eq!(outcome.harness_bugs, 0);
        assert_eq!(outcome.overall_pass_rate, 1.0);
        assert!(outcome.parse_error_rate < PARSE_ERROR_THRESHOLD);
    }

    #[test]
    fn m3_table_covers_three_providers_and_three_tiers() {
        let outcome = run_m3_gate();
        assert_eq!(outcome.table.providers().len(), 3);
        assert_eq!(outcome.table.tiers(), &["easy", "medium", "hard"]);
        // 모든 칸이 100% 통과(레퍼런스 해법).
        for provider in outcome.table.providers() {
            for tier in outcome.table.tiers() {
                let cell = outcome.table.cell(provider, tier).expect("칸 존재");
                assert_eq!(cell.rate(), 1.0, "{provider}×{tier} 통과율 < 1.0");
            }
        }
    }

    #[test]
    fn m4_gate_proves_ptc_advantage() {
        let outcome = run_m4_gate();
        assert!(outcome.passed(), "M4 게이트 실패\n{}", outcome.report());
        // Q1: 둘 다 전 태스크 통과 → 비열등(악화 없음).
        assert_eq!(outcome.ptc_pass_rate, 1.0);
        assert_eq!(outcome.baseline_pass_rate, 1.0);
        assert!(!outcome.mcnemar.degraded(M4_ALPHA));
        // Q2: PTC가 LLM 호출·토큰 모두 유의하게 적다(CI 상한 < 1).
        assert!(outcome.llm_call_ratio.upper < 1.0);
        assert!(outcome.token_ratio.upper < 1.0);
        assert_eq!(outcome.harness_bugs, 0);
        assert!(outcome.repeats >= 10);
    }

    #[test]
    fn m4_ptc_uses_strictly_fewer_llm_calls() {
        // baseline은 도구 호출마다 LLM을 다시 부르므로 호출 비율이 1보다 한참 작다.
        let outcome = run_m4_gate();
        assert!(
            outcome.llm_call_ratio.point < 0.5,
            "LLM 호출 비율 {} 이 0.5 미만이어야",
            outcome.llm_call_ratio.point
        );
    }
}
