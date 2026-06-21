//! `ptc-analyze` — RunRecord JSONL에서 통과율 표·실패 분포(M3-T06)와
//! 모드 비교·통계([`stats`], M4)를 낸다.
//!
//! 분석은 하네스의 **하류 소비자**다. 그래서 RunRecord 타입에 직접 의존하지 않고,
//! JSONL을 외부 인터페이스로 보고 필요한 필드만 담은 [`RunRow`]로 역직렬화한다.
//! 덕분에 `ptc-harness`가 거꾸로 이 크레이트에 의존(게이트의 표 재현)해도 순환이 없다.
//!
//! **측정의 정직성(설계 5.3절):** 점추정만 내지 않는다. 통과율 표는 분자/분모를
//! 함께 들고, 실패는 6분류 분포로 쪼개 "우리 탓"과 "모델 탓"을 가른다.

pub mod stats;

use serde::Deserialize;
use std::collections::BTreeMap;

/// 분석에 필요한 RunRecord의 부분 뷰. 그 밖의 필드는 serde가 무시한다.
#[derive(Debug, Clone, Deserialize)]
pub struct RunRow {
    #[serde(default)]
    pub task_id: String,
    pub provider: String,
    pub tier: String,
    /// 실행 모드(`"ptc"` | `"baseline_1_0"`) — M4 비교 집계용.
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub grade: Option<GradeView>,
    /// 실패 분류 라벨(통과면 `None`).
    #[serde(default)]
    pub failure: Option<String>,
    #[serde(default)]
    pub metrics: MetricsView,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GradeView {
    pub pass: bool,
}

/// 성능 지표 뷰(M4 비교용). 누락 필드는 0으로 본다.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MetricsView {
    #[serde(default)]
    pub llm_calls: u32,
    #[serde(default)]
    pub tool_calls: u32,
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub latency_ms: u64,
}

impl MetricsView {
    /// 입력+출력 토큰 합.
    pub fn total_tokens(&self) -> u32 {
        self.input_tokens + self.output_tokens
    }
}

impl RunRow {
    /// 채점을 통과했는가(채점 없음 = 미통과).
    pub fn passed(&self) -> bool {
        self.grade.as_ref().is_some_and(|g| g.pass)
    }
}

/// 실패 분류 6라벨(설계 4.4절). `ptc-harness`의 taxonomy와 같은 문자열 계약이며,
/// 순환 의존을 피하려 분석 쪽에 사본으로 둔다.
pub const FAILURE_LABELS: [&str; 6] = [
    "EXTRACTION_FAIL",
    "PARSE_ERROR",
    "VALIDATION_REJECT",
    "RUNTIME_ERROR",
    "WRONG_ANSWER",
    "HARNESS_BUG",
];

/// JSONL 텍스트를 행들로 파싱한다(빈 줄 무시). 한 줄이라도 깨지면 줄 번호와 함께 실패.
pub fn parse_jsonl(text: &str) -> Result<Vec<RunRow>, String> {
    let mut rows = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: RunRow = serde_json::from_str(line)
            .map_err(|e| format!("JSONL {}번째 줄 파싱 실패: {e}", lineno + 1))?;
        rows.push(row);
    }
    Ok(rows)
}

/// 한 (provider, tier) 칸의 통과/전체.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub passed: usize,
    pub total: usize,
}

impl Cell {
    pub fn rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.passed as f64 / self.total as f64
        }
    }
}

/// provider×tier 통과율 표. 같은 입력이면 같은 표가 되도록 키를 정렬해 보관한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassRateTable {
    cells: BTreeMap<(String, String), Cell>,
    providers: Vec<String>,
    tiers: Vec<String>,
}

impl PassRateTable {
    pub fn providers(&self) -> &[String] {
        &self.providers
    }

    pub fn tiers(&self) -> &[String] {
        &self.tiers
    }

    /// (provider, tier) 칸. 조합이 없으면 `None`.
    pub fn cell(&self, provider: &str, tier: &str) -> Option<Cell> {
        self.cells
            .get(&(provider.to_string(), tier.to_string()))
            .copied()
    }

    /// 사람이 읽는 표(각 칸은 `통과/전체` 비율).
    pub fn render(&self) -> String {
        let mut out = String::from("[통과율 표 — provider × tier]\n");
        out.push_str(&format!("{:<16}", "provider"));
        for tier in &self.tiers {
            out.push_str(&format!("{tier:>14}"));
        }
        out.push('\n');
        for provider in &self.providers {
            out.push_str(&format!("{provider:<16}"));
            for tier in &self.tiers {
                match self.cell(provider, tier) {
                    Some(cell) => out.push_str(&format!(
                        "{:>14}",
                        format!("{:.2}({}/{})", cell.rate(), cell.passed, cell.total)
                    )),
                    None => out.push_str(&format!("{:>14}", "-")),
                }
            }
            out.push('\n');
        }
        out
    }
}

/// 통과율 표를 집계한다.
pub fn pass_rate_table(rows: &[RunRow]) -> PassRateTable {
    let mut cells: BTreeMap<(String, String), Cell> = BTreeMap::new();
    for row in rows {
        let cell = cells
            .entry((row.provider.clone(), row.tier.clone()))
            .or_insert(Cell {
                passed: 0,
                total: 0,
            });
        cell.total += 1;
        if row.passed() {
            cell.passed += 1;
        }
    }
    let mut providers: Vec<String> = rows.iter().map(|r| r.provider.clone()).collect();
    providers.sort_unstable();
    providers.dedup();
    let mut tiers: Vec<String> = rows.iter().map(|r| r.tier.clone()).collect();
    tiers.sort_unstable_by_key(|tier| (tier_rank(tier), tier.clone()));
    tiers.dedup();
    PassRateTable {
        cells,
        providers,
        tiers,
    }
}

/// 난이도 정렬 순위(easy→medium→hard, 그 밖은 뒤로).
fn tier_rank(tier: &str) -> u8 {
    match tier {
        "easy" => 0,
        "medium" => 1,
        "hard" => 2,
        _ => 3,
    }
}

/// 실패 6분류 분포(모든 라벨을 0 포함해 정해진 순서로 돌려준다).
pub fn failure_distribution(rows: &[RunRow]) -> Vec<(&'static str, usize)> {
    FAILURE_LABELS
        .iter()
        .map(|&label| {
            let count = rows
                .iter()
                .filter(|r| r.failure.as_deref() == Some(label))
                .count();
            (label, count)
        })
        .collect()
}

/// PARSE_ERROR가 전체 실행에서 차지하는 비율(문법 확장 트리거 — 설계 M3 게이트).
pub fn parse_error_rate(rows: &[RunRow]) -> f64 {
    if rows.is_empty() {
        return 0.0;
    }
    let parse_errors = rows
        .iter()
        .filter(|r| r.failure.as_deref() == Some("PARSE_ERROR"))
        .count();
    parse_errors as f64 / rows.len() as f64
}

// ── 모드 비교 (M4) ──

/// 한 (task, mode)의 반복 평균 집계.
#[derive(Debug, Clone, PartialEq)]
pub struct ModeAgg {
    pub mean_llm_calls: f64,
    pub mean_total_tokens: f64,
    /// 반복 다수결 통과(반복의 과반이 통과).
    pub pass: bool,
    pub repeats: usize,
}

/// 주어진 모드의 행들을 task별로 평균낸다.
fn aggregate_mode(rows: &[RunRow], mode: &str) -> BTreeMap<String, ModeAgg> {
    let mut by_task: BTreeMap<String, Vec<&RunRow>> = BTreeMap::new();
    for row in rows.iter().filter(|r| r.mode == mode) {
        by_task.entry(row.task_id.clone()).or_default().push(row);
    }
    by_task
        .into_iter()
        .map(|(task, runs)| {
            let n = runs.len() as f64;
            let llm: f64 = runs.iter().map(|r| r.metrics.llm_calls as f64).sum();
            let tokens: f64 = runs.iter().map(|r| r.metrics.total_tokens() as f64).sum();
            let passed = runs.iter().filter(|r| r.passed()).count();
            (
                task,
                ModeAgg {
                    mean_llm_calls: llm / n,
                    mean_total_tokens: tokens / n,
                    pass: passed * 2 >= runs.len(),
                    repeats: runs.len(),
                },
            )
        })
        .collect()
}

/// PTC vs baseline의 task별 짝지은 비교(공통 task만, task_id 정렬).
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub tasks: Vec<String>,
    /// (ptc 통과, baseline 통과) — McNemar 입력.
    pub pass_pairs: Vec<(bool, bool)>,
    pub ptc_llm_calls: Vec<f64>,
    pub baseline_llm_calls: Vec<f64>,
    pub ptc_tokens: Vec<f64>,
    pub baseline_tokens: Vec<f64>,
}

/// 두 모드를 task별로 짝지어 비교 자료를 만든다.
pub fn compare_modes(rows: &[RunRow], ptc_mode: &str, baseline_mode: &str) -> Comparison {
    let ptc = aggregate_mode(rows, ptc_mode);
    let baseline = aggregate_mode(rows, baseline_mode);
    let mut comparison = Comparison {
        tasks: Vec::new(),
        pass_pairs: Vec::new(),
        ptc_llm_calls: Vec::new(),
        baseline_llm_calls: Vec::new(),
        ptc_tokens: Vec::new(),
        baseline_tokens: Vec::new(),
    };
    // ptc는 BTreeMap이라 task_id 정렬 순서로 순회된다(결정론).
    for (task, p) in &ptc {
        let Some(b) = baseline.get(task) else {
            continue;
        };
        comparison.tasks.push(task.clone());
        comparison.pass_pairs.push((p.pass, b.pass));
        comparison.ptc_llm_calls.push(p.mean_llm_calls);
        comparison.baseline_llm_calls.push(b.mean_llm_calls);
        comparison.ptc_tokens.push(p.mean_total_tokens);
        comparison.baseline_tokens.push(b.mean_total_tokens);
    }
    comparison
}

/// 실패 분포를 사람이 읽는 문자열로(0인 분류는 생략).
pub fn render_failure_distribution(rows: &[RunRow]) -> String {
    let mut out = String::from("[실패 분류 분포]\n");
    let dist = failure_distribution(rows);
    if dist.iter().all(|(_, count)| *count == 0) {
        out.push_str("  (없음)\n");
        return out;
    }
    for (label, count) in dist {
        if count > 0 {
            out.push_str(&format!("  {label}: {count}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jsonl(rows: &[(&str, &str, bool, Option<&str>)]) -> String {
        rows.iter()
            .map(|(provider, tier, pass, failure)| {
                let grade = if *pass {
                    "\"grade\":{\"level\":\"L1\",\"pass\":true}"
                } else {
                    "\"grade\":{\"level\":\"L1\",\"pass\":false}"
                };
                let fail = match failure {
                    Some(label) => format!(",\"failure\":\"{label}\""),
                    None => String::new(),
                };
                format!("{{\"provider\":\"{provider}\",\"tier\":\"{tier}\",{grade}{fail}}}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn parses_jsonl_skipping_blank_lines() {
        let text = format!("{}\n\n", jsonl(&[("mock", "easy", true, None)]));
        let rows = parse_jsonl(&text).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].passed());
    }

    #[test]
    fn malformed_line_reports_line_number() {
        let err = parse_jsonl("{not json}").unwrap_err();
        assert!(err.contains("1번째 줄"));
    }

    #[test]
    fn pass_rate_table_aggregates_by_provider_and_tier() {
        let text = jsonl(&[
            ("mock-a", "easy", true, None),
            ("mock-a", "easy", false, Some("WRONG_ANSWER")),
            ("mock-a", "hard", true, None),
            ("mock-b", "easy", true, None),
        ]);
        let rows = parse_jsonl(&text).unwrap();
        let table = pass_rate_table(&rows);

        assert_eq!(table.providers(), &["mock-a", "mock-b"]);
        // tier는 easy→hard 순(난이도 정렬).
        assert_eq!(table.tiers(), &["easy", "hard"]);
        assert_eq!(table.cell("mock-a", "easy").unwrap().rate(), 0.5);
        assert_eq!(table.cell("mock-a", "hard").unwrap().rate(), 1.0);
        assert_eq!(table.cell("mock-b", "hard"), None);
    }

    #[test]
    fn tiers_are_ordered_easy_medium_hard() {
        let text = jsonl(&[
            ("m", "hard", true, None),
            ("m", "easy", true, None),
            ("m", "medium", true, None),
        ]);
        let table = pass_rate_table(&parse_jsonl(&text).unwrap());
        assert_eq!(table.tiers(), &["easy", "medium", "hard"]);
    }

    #[test]
    fn identical_input_yields_identical_table() {
        let text = jsonl(&[
            ("mock-a", "easy", true, None),
            ("mock-b", "hard", false, Some("PARSE_ERROR")),
        ]);
        let rows = parse_jsonl(&text).unwrap();
        assert_eq!(pass_rate_table(&rows), pass_rate_table(&rows));
    }

    #[test]
    fn failure_distribution_enumerates_all_six_labels() {
        let text = jsonl(&[
            ("m", "easy", false, Some("PARSE_ERROR")),
            ("m", "easy", false, Some("PARSE_ERROR")),
            ("m", "easy", false, Some("WRONG_ANSWER")),
            ("m", "easy", true, None),
        ]);
        let rows = parse_jsonl(&text).unwrap();
        let dist = failure_distribution(&rows);
        assert_eq!(dist.len(), 6);
        let parse = dist.iter().find(|(l, _)| *l == "PARSE_ERROR").unwrap().1;
        assert_eq!(parse, 2);
        let harness = dist.iter().find(|(l, _)| *l == "HARNESS_BUG").unwrap().1;
        assert_eq!(harness, 0);
    }

    #[test]
    fn compare_modes_pairs_metrics_by_task() {
        // task t1: ptc llm=1 tokens=10 pass; baseline llm=6 tokens=60 pass.
        let line = |task: &str, mode: &str, llm: u32, tokens: u32, pass: bool| {
            format!(
                "{{\"task_id\":\"{task}\",\"provider\":\"m\",\"tier\":\"easy\",\"mode\":\"{mode}\",\"grade\":{{\"level\":\"L1\",\"pass\":{pass}}},\"metrics\":{{\"llm_calls\":{llm},\"input_tokens\":{tokens},\"output_tokens\":0}}}}"
            )
        };
        let text = [
            line("t1", "ptc", 1, 10, true),
            line("t1", "baseline_1_0", 6, 60, true),
            line("t2", "ptc", 1, 20, true),
            line("t2", "baseline_1_0", 9, 90, true),
        ]
        .join("\n");
        let rows = parse_jsonl(&text).unwrap();
        let cmp = compare_modes(&rows, "ptc", "baseline_1_0");

        assert_eq!(cmp.tasks, vec!["t1", "t2"]);
        assert_eq!(cmp.pass_pairs, vec![(true, true), (true, true)]);
        assert_eq!(cmp.ptc_llm_calls, vec![1.0, 1.0]);
        assert_eq!(cmp.baseline_llm_calls, vec![6.0, 9.0]);
        assert_eq!(cmp.ptc_tokens, vec![10.0, 20.0]);
        assert_eq!(cmp.baseline_tokens, vec![60.0, 90.0]);
    }

    #[test]
    fn parse_error_rate_is_fraction_of_all_runs() {
        let text = jsonl(&[
            ("m", "easy", false, Some("PARSE_ERROR")),
            ("m", "easy", true, None),
            ("m", "easy", true, None),
            ("m", "easy", true, None),
        ]);
        let rows = parse_jsonl(&text).unwrap();
        assert_eq!(parse_error_rate(&rows), 0.25);
        assert_eq!(parse_error_rate(&[]), 0.0);
    }
}
