//! Baseline 1.0 (ReAct) 러너 — 도구 결과마다 LLM을 재호출한다 (M4-T02).
//!
//! PTC가 한 번의 LLM 호출로 프로그램을 받아 실행하는 것과 달리, baseline은 매 턴
//! 한 행동([`Action`])만 받아 도구를 부르고 관측을 대화에 누적한 뒤 다시 LLM을
//! 부른다. 그래서 LLM 호출 수 = 도구 호출 수 + 1이고, 대화가 커지며 입력 토큰도
//! 늘어난다 — 이것이 PTC 대비 비용 차이의 근원이다.
//!
//! **공정성·DRY(설계 M4):** 도구 실행([`MockToolServer`])과 채점([`build_grader`])은
//! PTC와 **똑같은 컴포넌트**를 쓴다. 두 모드의 유일한 차이는 LLM 호출 방식뿐이다.

use crate::grader::Execution;
use crate::react::{parse_action, Action};
use crate::record::{record_trace, value_to_json, Grade, Mode, RunRecord};
use crate::runner::{build_grader, fail, skeleton, RunMeta, RunResult};
use crate::task::Task;
use crate::taxonomy::FailureCategory;
use ptc_dsl::{ToolCatalog, ToolSink};
use ptc_llm::{CompletionReq, LlmProvider};
use ptc_tools::MockToolServer;

/// baseline 시스템 프롬프트 v1(버전 관리). 컴파일 시 임베드한다.
pub const SYS_BASELINE_V1: &str = include_str!("../../../prompts/sys-baseline-v1.md");
/// baseline 시스템 프롬프트 버전 식별자.
pub const SYS_BASELINE_V1_VERSION: &str = "sys-baseline-v1";
/// ReAct 루프가 한 태스크에 허용하는 최대 턴 수(무한 루프 방지).
pub const MAX_TURNS: u32 = 32;

/// baseline ReAct 러너. 시스템 프롬프트·도구 카탈로그·최대 턴을 묶는다.
pub struct BaselineRunner<'a> {
    pub system_prompt: &'a str,
    pub prompt_version: &'a str,
    pub catalog: &'a ToolCatalog,
    pub max_turns: u32,
}

impl<'a> BaselineRunner<'a> {
    /// sys-baseline-v1 프롬프트 기본 러너.
    pub fn new(catalog: &'a ToolCatalog) -> Self {
        Self {
            system_prompt: SYS_BASELINE_V1,
            prompt_version: SYS_BASELINE_V1_VERSION,
            catalog,
            max_turns: MAX_TURNS,
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

    /// 한 태스크를 ReAct 루프로 한 번 실행한다.
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
        // ReAct: llm_calls는 턴마다 1씩 누적하므로 0에서 시작한다.
        let mut record = skeleton(&meta, Mode::Baseline1_0, self.prompt_version, 0);
        let mut server = MockToolServer::new();
        let mut observations: Vec<String> = Vec::new();

        for _turn in 0..self.max_turns {
            let mut req = CompletionReq::new(
                self.system_prompt,
                build_user(&task.question, &observations),
            );
            req.temperature = temperature;
            req.seed = seed;

            let resp = match provider.complete(req) {
                Ok(resp) => resp,
                Err(err) => {
                    record.error = Some(format!("provider error: {err}"));
                    return RunResult {
                        record,
                        category: None,
                    };
                }
            };
            record.metrics.llm_calls += 1;
            record.metrics.input_tokens += resp.input_tokens;
            record.metrics.output_tokens += resp.output_tokens;
            record.metrics.latency_ms += resp.latency_ms;
            push_line(&mut record.generated_code, &resp.text);

            match parse_action(&resp.text) {
                Err(message) => return fail(record, FailureCategory::Parse, &message),
                Ok(Action::Call { tool, args }) => {
                    if !self.catalog.contains(&tool) {
                        record.validation = format!("reject:{tool}");
                        return fail(
                            record,
                            FailureCategory::Validation,
                            &format!("미등록 도구: {tool}"),
                        );
                    }
                    match server.call(&tool, args) {
                        Ok(result) => observations.push(value_to_json(&result).to_string()),
                        Err(err) => {
                            finalize_tools(&mut record, &server);
                            return fail(record, FailureCategory::Runtime, &err.to_string());
                        }
                    }
                }
                Ok(Action::Final { value }) => {
                    record.validation = "pass".to_string();
                    finalize_tools(&mut record, &server);
                    record.final_output = Some(value_to_json(&value));
                    return self.grade(record, task, value, &server);
                }
            }
        }

        finalize_tools(&mut record, &server);
        fail(
            record,
            FailureCategory::Runtime,
            &format!("최대 턴({}) 초과 — 수렴 실패", self.max_turns),
        )
    }

    /// 최종값을 PTC와 같은 채점기로 판정한다.
    fn grade(
        &self,
        mut record: RunRecord,
        task: &Task,
        value: ptc_dsl::Value,
        server: &MockToolServer,
    ) -> RunResult {
        let grader = match build_grader(task) {
            Some(grader) => grader,
            None => return fail(record, FailureCategory::HarnessBug, "알 수 없는 채점 레벨"),
        };
        let execution = Execution {
            output: Some(value),
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
}

/// 도구 트레이스·호출 수를 레코드에 박는다(여러 종료점에서 공통).
fn finalize_tools(record: &mut RunRecord, server: &MockToolServer) {
    record.tool_trace = record_trace(server.trace());
    record.metrics.tool_calls = server.trace().len() as u32;
}

/// 질문과 누적 관측으로 이번 턴의 user 프롬프트를 만든다.
/// 관측 수가 곧 진행 단계이며, mock은 `OBSERVATION ` 표식을 세어 단계를 안다.
fn build_user(question: &str, observations: &[String]) -> String {
    let mut user = format!("질문: {question}\n");
    for (index, observation) in observations.iter().enumerate() {
        user.push_str(&format!("\nOBSERVATION {}: {}", index + 1, observation));
    }
    user
}

fn push_line(buffer: &mut String, line: &str) {
    if !buffer.is_empty() {
        buffer.push('\n');
    }
    buffer.push_str(line);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::pass_rate;
    use crate::suite::step_response;
    use crate::task::parse_task;
    use ptc_llm::{CompletionResp, LlmError};
    use ptc_tools::tool_names;

    /// 관측 수로 단계를 골라 스크립트된 행동을 돌려주는 테스트용 ReAct provider.
    /// 스텝 선택·토큰 회계는 suite의 [`step_response`]를 공유한다(DRY).
    struct ScriptProvider {
        steps: Vec<String>,
    }

    impl LlmProvider for ScriptProvider {
        fn name(&self) -> &str {
            "script"
        }

        fn complete(&self, req: CompletionReq) -> Result<CompletionResp, LlmError> {
            Ok(step_response(&req, &self.steps))
        }
    }

    fn first_member_task() -> Task {
        parse_task("id=\"t\"\ntier=\"easy\"\nquestion=\"eng 첫 구성원?\"\ngrader=\"L1\"\nexpected_output=\"Alice\"").unwrap()
    }

    fn run(provider: &dyn LlmProvider, task: &Task) -> RunResult {
        let catalog = ToolCatalog::new(tool_names());
        BaselineRunner::new(&catalog).run_once(task, provider, 0, Some(42), 0.0)
    }

    #[test]
    fn react_loop_calls_tool_then_finalizes_and_passes() {
        let provider = ScriptProvider {
            steps: vec![
                "CALL list_team {\"arg0\":\"eng\"}".to_string(),
                "FINAL \"Alice\"".to_string(),
            ],
        };
        let result = run(&provider, &first_member_task());
        assert!(result.passed());
        let record = &result.record;
        // ReAct: 도구 호출 1회 + 최종 1회 = LLM 호출 2회.
        assert_eq!(record.metrics.llm_calls, 2);
        assert_eq!(record.metrics.tool_calls, 1);
        assert_eq!(record.mode, Mode::Baseline1_0);
        assert!(record.metrics.input_tokens > 0);
    }

    #[test]
    fn unknown_tool_in_a_turn_is_validation_reject() {
        let provider = ScriptProvider {
            steps: vec!["CALL frobnicate {\"arg0\":1}".to_string()],
        };
        let result = run(&provider, &first_member_task());
        assert_eq!(result.category, Some(FailureCategory::Validation));
        assert!(result.record.validation.starts_with("reject:"));
    }

    #[test]
    fn malformed_action_is_parse_error() {
        let provider = ScriptProvider {
            steps: vec!["I will think about it".to_string()],
        };
        let result = run(&provider, &first_member_task());
        assert_eq!(result.category, Some(FailureCategory::Parse));
    }

    #[test]
    fn wrong_final_answer_is_classified() {
        let provider = ScriptProvider {
            steps: vec![
                "CALL list_team {\"arg0\":\"eng\"}".to_string(),
                "FINAL \"Bob\"".to_string(),
            ],
        };
        let result = run(&provider, &first_member_task());
        assert!(!result.passed());
        assert_eq!(result.category, Some(FailureCategory::WrongAnswer));
    }

    #[test]
    fn never_finalizing_hits_max_turns() {
        // 항상 같은 CALL만 반복 → 최대 턴 초과로 종료.
        let provider = ScriptProvider {
            steps: vec!["CALL get_budget {\"arg0\":\"Q3\"}".to_string()],
        };
        let catalog = ToolCatalog::new(tool_names());
        let runner = BaselineRunner {
            max_turns: 4,
            ..BaselineRunner::new(&catalog)
        };
        let result = runner.run_once(&first_member_task(), &provider, 0, Some(42), 0.0);
        assert_eq!(result.category, Some(FailureCategory::Runtime));
        assert_eq!(result.record.metrics.llm_calls, 4);
    }

    #[test]
    fn react_consumes_more_llm_calls_than_a_single_ptc_call() {
        // list_team + get_expenses×? 가 아니라, 여기선 단순히 호출 누적을 확인.
        let provider = ScriptProvider {
            steps: vec![
                "CALL list_team {\"arg0\":\"eng\"}".to_string(),
                "CALL get_expenses {\"arg0\":1,\"arg1\":\"Q3\"}".to_string(),
                "FINAL 1750.0".to_string(),
            ],
        };
        let task = parse_task(
            "id=\"t\"\ntier=\"easy\"\nquestion=\"q\"\ngrader=\"L1\"\nexpected_output=1750.0",
        )
        .unwrap();
        let results = {
            let catalog = ToolCatalog::new(tool_names());
            BaselineRunner::new(&catalog).run_repeated(&task, &provider, 3, Some(42))
        };
        assert_eq!(pass_rate(&results), 1.0);
        // 매 실행 LLM 호출 3회(도구 2 + 최종 1) ≫ PTC의 1회.
        assert!(results.iter().all(|r| r.record.metrics.llm_calls == 3));
    }
}
