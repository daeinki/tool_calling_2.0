//! 배치 러너 — providers × tasks × R회 실행을 순회·집계한다 (M3-T05).
//!
//! **SRP:** 배치 러너는 순회와 결과 모음만 책임진다. 한 실행의 LLM→추출→검증→
//! 실행→채점은 전적으로 [`Runner`]에 위임한다(M2 러너 재사용 = DRY). 순회 순서는
//! providers→tasks→repeat로 고정되고, tasks가 정렬돼 들어오면 결과도 결정론적이다.

use crate::baseline::BaselineRunner;
use crate::record::RunRecord;
use crate::runner::{RunResult, Runner};
use crate::task::Task;
use ptc_dsl::ToolCatalog;
use ptc_llm::LlmProvider;

/// 배치 실행 설정. 반복 횟수와 시드를 묶는다.
pub struct BatchConfig {
    pub repeats: u32,
    pub seed: Option<u64>,
}

/// 배치 실행의 전체 결과(모든 (provider, task, repeat) 조합).
pub struct BatchOutcome {
    pub results: Vec<RunResult>,
}

impl BatchOutcome {
    /// 적재용 RunRecord 모음.
    pub fn records(&self) -> Vec<RunRecord> {
        self.results.iter().map(|r| r.record.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.results.len()
    }

    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }
}

/// 모든 provider로 모든 태스크를 R회 PTC 모드로 실행한다. 실행은 [`Runner`]에 위임.
pub fn run_batch(
    providers: &[&dyn LlmProvider],
    tasks: &[Task],
    catalog: &ToolCatalog,
    config: &BatchConfig,
) -> BatchOutcome {
    let runner = Runner::new(catalog);
    run_each(providers, tasks, config, |task, provider| {
        runner.run_repeated(task, provider, config.repeats, config.seed)
    })
}

/// 모든 provider로 모든 태스크를 R회 baseline(ReAct) 모드로 실행한다(M4).
pub fn run_baseline_batch(
    providers: &[&dyn LlmProvider],
    tasks: &[Task],
    catalog: &ToolCatalog,
    config: &BatchConfig,
) -> BatchOutcome {
    let runner = BaselineRunner::new(catalog);
    run_each(providers, tasks, config, |task, provider| {
        runner.run_repeated(task, provider, config.repeats, config.seed)
    })
}

/// providers × tasks 순회 골격. 한 (task, provider)의 R회 실행은 호출자가 넘긴다.
/// PTC·baseline 배치가 순회 로직을 공유한다(DRY) — 차이는 어느 러너를 쓰느냐뿐.
fn run_each<F>(
    providers: &[&dyn LlmProvider],
    tasks: &[Task],
    config: &BatchConfig,
    run_repeated: F,
) -> BatchOutcome
where
    F: Fn(&Task, &dyn LlmProvider) -> Vec<RunResult>,
{
    let mut results = Vec::with_capacity(providers.len() * tasks.len() * config.repeats as usize);
    for provider in providers {
        for task in tasks {
            results.extend(run_repeated(task, *provider));
        }
    }
    BatchOutcome { results }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::pass_rate;
    use crate::task::parse_task;
    use ptc_llm::MockProvider;
    use ptc_tools::tool_names;

    fn task(id: &str) -> Task {
        // emit(0)으로 통과하는 최소 L1 태스크.
        parse_task(&format!(
            "id=\"{id}\"\ntier=\"easy\"\nquestion=\"q {id}\"\ngrader=\"L1\"\nexpected_output=0.0"
        ))
        .unwrap()
    }

    #[test]
    fn batch_covers_every_provider_task_repeat_combination() {
        let a = MockProvider::new()
            .with_name("mock-a")
            .default_response("emit(0)");
        let b = MockProvider::new()
            .with_name("mock-b")
            .default_response("emit(0)");
        let providers: [&dyn LlmProvider; 2] = [&a, &b];
        let tasks = vec![task("t1"), task("t2"), task("t3")];
        let catalog = ToolCatalog::new(tool_names());

        let outcome = run_batch(
            &providers,
            &tasks,
            &catalog,
            &BatchConfig {
                repeats: 4,
                seed: Some(42),
            },
        );

        assert_eq!(outcome.len(), 2 * 3 * 4);
        assert_eq!(pass_rate(&outcome.results), 1.0);
        // 두 provider가 모두 등장한다.
        let providers_seen: std::collections::BTreeSet<&str> = outcome
            .results
            .iter()
            .map(|r| r.record.provider.as_str())
            .collect();
        assert_eq!(
            providers_seen,
            std::collections::BTreeSet::from(["mock-a", "mock-b"])
        );
        // tier가 레코드에 박혀 분석에서 집계 가능하다.
        assert!(outcome.results.iter().all(|r| r.record.tier == "easy"));
    }

    #[test]
    fn batch_is_deterministic_across_runs() {
        let p = MockProvider::new()
            .with_name("mock-a")
            .default_response("emit(0)");
        let providers: [&dyn LlmProvider; 1] = [&p];
        let tasks = vec![task("t1"), task("t2")];
        let catalog = ToolCatalog::new(tool_names());
        let config = BatchConfig {
            repeats: 3,
            seed: Some(7),
        };

        let first = run_batch(&providers, &tasks, &catalog, &config);
        let second = run_batch(&providers, &tasks, &catalog, &config);
        assert_eq!(first.records(), second.records());
    }
}
