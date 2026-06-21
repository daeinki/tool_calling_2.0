//! E2E 러너 — 한 태스크를 LLM→추출→검증→실행→채점으로 잇는다 (M2-T08).
//!
//! **SRP:** 러너는 오케스트레이션만 한다. 각 단계는 이미 만든 컴포넌트
//! (provider · [`extract_code`] · 파서/검증기/인터프리터 · [`Grader`])에 위임하고,
//! 실패는 단계에 따라 [`FailureCategory`]로 매긴 뒤 [`RunRecord`]로 적재한다.

use crate::extract::{extract_code, Extraction};
use crate::grader::{ExactMatch, Execution, Grader, TraceMatch};
use crate::record::{record_trace, value_to_json, Grade, Metrics, Mode, RunRecord};
use crate::task::Task;
use crate::taxonomy::FailureCategory;
use ptc_dsl::{parse, tokenize, validate, Interpreter, ToolCatalog};
use ptc_llm::{CompletionReq, LlmProvider};
use ptc_tools::MockToolServer;

/// 시스템 프롬프트 v1(버전 관리 대상). 컴파일 시 임베드한다.
pub const SYS_V1: &str = include_str!("../../../prompts/sys-v1.md");
/// 시스템 프롬프트 버전 식별자.
pub const SYS_V1_VERSION: &str = "sys-v1";

/// 실행 한 건의 결과: 적재용 레코드 + 실패 분류(통과면 `None`).
pub struct RunResult {
    pub record: RunRecord,
    pub category: Option<FailureCategory>,
}

impl RunResult {
    /// 채점에서 통과했는가.
    pub fn passed(&self) -> bool {
        self.record.grade.as_ref().is_some_and(|grade| grade.pass)
    }
}

/// 한 실행의 식별 정보. 인수 수를 줄이려 한데 묶는다(clean-code §2).
struct RunMeta<'a> {
    run_id: String,
    task: &'a Task,
    provider_name: &'a str,
    seed: Option<u64>,
    temperature: f32,
    repeat_idx: u32,
}

/// 러너 설정. 시스템 프롬프트·도구 카탈로그·모드를 묶는다.
pub struct Runner<'a> {
    pub system_prompt: &'a str,
    pub prompt_version: &'a str,
    pub catalog: &'a ToolCatalog,
    pub mode: Mode,
}

impl<'a> Runner<'a> {
    /// sys-v1 프롬프트 + PTC 모드의 기본 러너.
    pub fn new(catalog: &'a ToolCatalog) -> Self {
        Self {
            system_prompt: SYS_V1,
            prompt_version: SYS_V1_VERSION,
            catalog,
            mode: Mode::Ptc,
        }
    }

    /// 한 태스크를 R회 반복 실행한다(temperature 0.0 고정).
    pub fn run_repeated(
        &self,
        task: &Task,
        provider: &dyn LlmProvider,
        repeats: u32,
        seed: Option<u64>,
    ) -> Vec<RunResult> {
        (0..repeats)
            .map(|repeat_idx| self.run_once(task, provider, repeat_idx, seed, 0.0))
            .collect()
    }

    /// 한 태스크를 한 번 실행한다.
    pub fn run_once(
        &self,
        task: &Task,
        provider: &dyn LlmProvider,
        repeat_idx: u32,
        seed: Option<u64>,
        temperature: f32,
    ) -> RunResult {
        let meta = RunMeta {
            run_id: format!("{}-{}-{}", task.id, provider.name(), repeat_idx),
            task,
            provider_name: provider.name(),
            seed,
            temperature,
            repeat_idx,
        };
        let mut req = CompletionReq::new(self.system_prompt, build_user_prompt(task));
        req.temperature = temperature;
        req.seed = seed;

        let resp = match provider.complete(req) {
            Ok(resp) => resp,
            Err(err) => {
                // provider/네트워크 실패는 코드 taxonomy에 속하지 않으므로 기록만 한다.
                let mut record = self.skeleton(&meta);
                record.error = Some(format!("provider error: {err}"));
                return RunResult {
                    record,
                    category: None,
                };
            }
        };

        let mut record = self.skeleton(&meta);
        record.generated_code = resp.text.clone();
        record.metrics.input_tokens = resp.input_tokens;
        record.metrics.output_tokens = resp.output_tokens;
        record.metrics.latency_ms = resp.latency_ms;

        let extraction = extract_code(&resp.text);
        record.extraction = extraction.record_label().to_string();
        let code = match extraction.code() {
            Some(code) => code.to_string(),
            None => {
                return fail(
                    record,
                    FailureCategory::ExtractionFail,
                    "추출할 코드가 없음",
                )
            }
        };

        let tokens = match tokenize(&code) {
            Ok(tokens) => tokens,
            Err(err) => return fail(record, parse_failure(&extraction), &err.to_string()),
        };
        let program = match parse(tokens) {
            Ok(program) => program,
            Err(err) => return fail(record, parse_failure(&extraction), &err.to_string()),
        };

        if let Err(err) = validate(&program, self.catalog) {
            record.validation = format!("reject:{err}");
            return fail(record, FailureCategory::Validation, &err.to_string());
        }
        record.validation = "pass".to_string();

        let grader = match build_grader(task) {
            Some(grader) => grader,
            None => return fail(record, FailureCategory::HarnessBug, "알 수 없는 채점 레벨"),
        };

        let mut server = MockToolServer::new();
        let outcome = Interpreter::new(&mut server).run(&program);
        record.tool_trace = record_trace(server.trace());
        record.metrics.tool_calls = server.trace().len() as u32;

        let output = match outcome {
            Ok(output) => output,
            Err(err) => return fail(record, FailureCategory::Runtime, &err.to_string()),
        };
        record.final_output = output.as_ref().map(value_to_json);

        let execution = Execution {
            output,
            trace: server.trace().to_vec(),
        };
        let pass = grader.grade(&execution);
        record.grade = Some(Grade {
            level: grader.level().to_string(),
            pass,
        });

        if pass {
            RunResult {
                record,
                category: None,
            }
        } else {
            fail(record, FailureCategory::WrongAnswer, "기대값과 다른 결과")
        }
    }

    /// 실행 식별 정보로 빈 레코드 골격을 만든다. 호출자가 코드·지표·결과를 채운다.
    fn skeleton(&self, meta: &RunMeta) -> RunRecord {
        RunRecord {
            run_id: meta.run_id.clone(),
            task_id: meta.task.id.clone(),
            tier: meta.task.tier.clone(),
            mode: self.mode,
            provider: meta.provider_name.to_string(),
            model: meta.provider_name.to_string(),
            prompt_version: self.prompt_version.to_string(),
            seed: meta.seed,
            temperature: meta.temperature,
            repeat_idx: meta.repeat_idx,
            generated_code: String::new(),
            extraction: "n/a".to_string(),
            validation: "n/a".to_string(),
            tool_trace: Vec::new(),
            final_output: None,
            grade: None,
            failure: None,
            metrics: Metrics {
                llm_calls: 1, // PTC 모드: LLM 호출은 항상 1회.
                tool_calls: 0,
                input_tokens: 0,
                output_tokens: 0,
                latency_ms: 0,
            },
            error: None,
        }
    }
}

/// 통과율 = 통과 수 / 전체.
pub fn pass_rate(results: &[RunResult]) -> f64 {
    if results.is_empty() {
        return 0.0;
    }
    let passed = results.iter().filter(|r| r.passed()).count();
    passed as f64 / results.len() as f64
}

/// M2는 user 프롬프트에 질문만 싣는다(도구 설명은 시스템 프롬프트에 있음).
fn build_user_prompt(task: &Task) -> String {
    task.question.clone()
}

/// 파스 실패의 분류. 펜스 없는(Whole) 코드의 파스 실패는 추출 실패로 본다(3.2절 규칙 2).
fn parse_failure(extraction: &Extraction) -> FailureCategory {
    match extraction {
        Extraction::Whole(_) => FailureCategory::ExtractionFail,
        _ => FailureCategory::Parse,
    }
}

/// 태스크의 채점 레벨에 맞는 채점기를 만든다. PTC·baseline 양 모드가 공유한다(DRY).
pub(crate) fn build_grader(task: &Task) -> Option<Box<dyn Grader>> {
    match task.grader.as_str() {
        "L1" => Some(Box::new(ExactMatch::new(task.expected_value()))),
        "L2" => Some(Box::new(TraceMatch::new(task.expected_tool_calls.clone()))),
        _ => None,
    }
}

fn fail(mut record: RunRecord, category: FailureCategory, message: &str) -> RunResult {
    record.error = Some(message.to_string());
    // 실패 분류를 레코드에 박아 JSONL만으로 분포를 집계할 수 있게 한다(M3-T06).
    record.failure = Some(category.label().to_string());
    RunResult {
        record,
        category: Some(category),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::parse_task;
    use ptc_llm::MockProvider;

    fn task() -> Task {
        parse_task(
            "id=\"easy_first_member\"\ntier=\"easy\"\nquestion=\"엔지니어링 팀 첫 구성원?\"\ngrader=\"L1\"\nexpected_output=\"Alice\"",
        )
        .unwrap()
    }

    fn run(provider: &dyn LlmProvider, task: &Task) -> RunResult {
        let catalog = ToolCatalog::new(ptc_tools::tool_names());
        Runner::new(&catalog).run_once(task, provider, 0, Some(42), 0.0)
    }

    #[test]
    fn happy_path_passes_and_fills_record() {
        let provider =
            MockProvider::new().default_response("team = list_team(\"eng\")\nemit(team[0].name)");
        let result = run(&provider, &task());

        assert!(result.passed());
        assert_eq!(result.category, None);
        let record = &result.record;
        assert_eq!(record.validation, "pass");
        assert!(record.grade.as_ref().unwrap().pass);
        assert_eq!(record.final_output, Some(serde_json::json!("Alice")));
        assert_eq!(record.metrics.llm_calls, 1);
        assert_eq!(record.metrics.tool_calls, 1);
        assert!(record.metrics.input_tokens > 0);
    }

    #[test]
    fn fenced_response_is_extracted_and_passes() {
        let provider = MockProvider::new()
            .default_response("```dsl\nteam = list_team(\"eng\")\nemit(team[0].name)\n```");
        let result = run(&provider, &task());
        assert!(result.passed());
        assert_eq!(result.record.extraction, "fenced");
    }

    #[test]
    fn wrong_answer_is_classified() {
        let provider = MockProvider::new().default_response("emit(\"Bob\")");
        let result = run(&provider, &task());
        assert!(!result.passed());
        assert_eq!(result.category, Some(FailureCategory::WrongAnswer));
    }

    #[test]
    fn empty_response_is_extraction_fail() {
        let provider = MockProvider::new().default_response("");
        let result = run(&provider, &task());
        assert_eq!(result.category, Some(FailureCategory::ExtractionFail));
    }

    #[test]
    fn fenced_syntax_error_is_parse_error() {
        let provider = MockProvider::new().default_response("```\nfor m in\n```");
        let result = run(&provider, &task());
        assert_eq!(result.category, Some(FailureCategory::Parse));
    }

    #[test]
    fn unknown_tool_is_validation_reject() {
        let provider = MockProvider::new().default_response("emit(frobnicate(1))");
        let result = run(&provider, &task());
        assert_eq!(result.category, Some(FailureCategory::Validation));
        assert!(result.record.validation.starts_with("reject:"));
    }

    #[test]
    fn runtime_error_is_classified() {
        let provider = MockProvider::new().default_response("emit(missing)");
        let result = run(&provider, &task());
        assert_eq!(result.category, Some(FailureCategory::Runtime));
    }

    #[test]
    fn unknown_grader_level_is_harness_bug() {
        let mut task = task();
        task.grader = "L9".to_string();
        let provider = MockProvider::new().default_response("emit(1)");
        let result = run(&provider, &task);
        assert_eq!(result.category, Some(FailureCategory::HarnessBug));
    }

    #[test]
    fn repeated_runs_yield_full_pass_rate_on_deterministic_provider() {
        let provider =
            MockProvider::new().default_response("team = list_team(\"eng\")\nemit(team[0].name)");
        let catalog = ToolCatalog::new(ptc_tools::tool_names());
        let results = Runner::new(&catalog).run_repeated(&task(), &provider, 5, Some(42));
        assert_eq!(results.len(), 5);
        assert_eq!(pass_rate(&results), 1.0);
    }
}
