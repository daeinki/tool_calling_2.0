//! 골든 태스크 — 선언적으로 정의한 검증 과제와 그 로더 (M2-T06).
//!
//! 태스크는 **코드가 아니라 데이터(TOML)**다. 질문·기대 출력·채점 레벨을 담으며,
//! 기대값은 [`ptc_tools::MockToolServer`]의 고정 데이터로부터 사전 계산한다(L1 채점 가능).

use ptc_dsl::Value;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// 태스크 한 건.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub tier: String,
    #[serde(default)]
    pub domains: Vec<String>,
    pub question: String,
    /// 채점 레벨 식별자(예: `"L1"`).
    pub grader: String,
    /// 사전 계산된 기대 최종값. L1 채점에 쓰며, L2(절차) 태스크에선 생략 가능.
    #[serde(default)]
    pub expected_output: Option<TaskValue>,
    /// 선택: 절차 정확성 검증용 기대 호출(L2에서 사용).
    #[serde(default)]
    pub expected_tool_calls: Vec<ExpectedToolCall>,
}

impl Task {
    /// 기대 출력을 인터프리터 [`Value`]로 변환한다(없으면 `None`).
    pub fn expected_value(&self) -> Option<Value> {
        self.expected_output.as_ref().map(TaskValue::to_value)
    }
}

/// 도구 한 종에 대한 기대 호출 횟수.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ExpectedToolCall {
    pub tool: String,
    #[serde(default)]
    pub count: Option<usize>,
    #[serde(default)]
    pub count_min: Option<usize>,
}

/// TOML로 표현 가능한 기대값. [`Value`]의 부분집합.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum TaskValue {
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<TaskValue>),
}

impl TaskValue {
    fn to_value(&self) -> Value {
        match self {
            TaskValue::Bool(b) => Value::Bool(*b),
            TaskValue::Num(n) => Value::Num(*n),
            TaskValue::Str(s) => Value::Str(s.clone()),
            TaskValue::List(items) => Value::List(items.iter().map(TaskValue::to_value).collect()),
        }
    }
}

/// 태스크 로딩·파싱 실패.
#[derive(Debug, Error)]
pub enum TaskError {
    #[error("태스크 파일 읽기 실패 ({path}): {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("태스크 파싱 실패: {0}")]
    Parse(String),
    #[error("태스크 id 중복: {0}")]
    DuplicateId(String),
    #[error("태스크 [{id}] 유효성 위반: {reason}")]
    Invalid { id: String, reason: String },
}

/// TOML 문자열을 태스크로 파싱한다.
pub fn parse_task(toml_str: &str) -> Result<Task, TaskError> {
    toml::from_str(toml_str).map_err(|e| TaskError::Parse(e.to_string()))
}

/// TOML 파일에서 태스크를 읽는다.
pub fn load_task(path: &Path) -> Result<Task, TaskError> {
    let text = std::fs::read_to_string(path).map_err(|source| TaskError::Io {
        path: path.display().to_string(),
        source,
    })?;
    parse_task(&text)
}

/// 디렉터리의 모든 `*.toml` 태스크를 로드한다(M3-T04).
///
/// 파일 순서에 무관하게 같은 결과를 내도록 경로를 정렬해 결정론을 보장한다.
/// id가 유일하고 각 태스크가 채점 레벨과 정합함을 함께 검증한다(측정의 정직성).
/// 스캔(IO)과 파싱·검증을 분리한다(명령/조회 분리).
pub fn load_tasks_dir(dir: &Path) -> Result<Vec<Task>, TaskError> {
    let paths = toml_paths(dir)?;
    let mut tasks = Vec::with_capacity(paths.len());
    let mut seen = BTreeSet::new();
    for path in paths {
        let task = load_task(&path)?;
        validate_task(&task)?;
        if !seen.insert(task.id.clone()) {
            return Err(TaskError::DuplicateId(task.id));
        }
        tasks.push(task);
    }
    Ok(tasks)
}

/// 디렉터리의 `*.toml` 경로를 정렬해 모은다(순수 IO 조회).
fn toml_paths(dir: &Path) -> Result<Vec<PathBuf>, TaskError> {
    let entries = std::fs::read_dir(dir).map_err(|source| TaskError::Io {
        path: dir.display().to_string(),
        source,
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|source| TaskError::Io {
                path: dir.display().to_string(),
                source,
            })?
            .path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

/// 태스크가 선언한 채점 레벨과 데이터가 정합한지 검증한다.
/// L1은 기대 출력이, L2는 비지 않은 기대 호출이 있어야 한다(빈 L2는 측정상 무의미).
fn validate_task(task: &Task) -> Result<(), TaskError> {
    let invalid = |reason: &str| TaskError::Invalid {
        id: task.id.clone(),
        reason: reason.to_string(),
    };
    match task.grader.as_str() {
        "L1" if task.expected_output.is_none() => Err(invalid("L1인데 expected_output 없음")),
        "L2" if task.expected_tool_calls.is_empty() => {
            Err(invalid("L2인데 expected_tool_calls 비어 있음"))
        }
        "L1" | "L2" => Ok(()),
        other => Err(invalid(&format!("알 수 없는 grader: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_task_with_scalar_expected_output() {
        let toml = r#"
            id = "easy_first_member"
            tier = "easy"
            domains = ["hr"]
            question = "엔지니어링 팀의 첫 구성원?"
            grader = "L1"
            expected_output = "Alice"
            [[expected_tool_calls]]
            tool = "list_team"
            count = 1
        "#;
        let task = parse_task(toml).unwrap();
        assert_eq!(task.id, "easy_first_member");
        assert_eq!(task.grader, "L1");
        assert_eq!(task.expected_value(), Some(Value::Str("Alice".into())));
        assert_eq!(task.expected_tool_calls.len(), 1);
        assert_eq!(task.expected_tool_calls[0].count, Some(1));
    }

    #[test]
    fn untagged_expected_output_handles_number_and_list() {
        let number = parse_task(
            "id=\"a\"\ntier=\"easy\"\nquestion=\"q\"\ngrader=\"L1\"\nexpected_output=13000.0",
        )
        .unwrap();
        assert_eq!(number.expected_value(), Some(Value::Num(13000.0)));

        let list = parse_task("id=\"b\"\ntier=\"medium\"\nquestion=\"q\"\ngrader=\"L1\"\nexpected_output=[\"Bob\", \"Dana\"]").unwrap();
        assert_eq!(
            list.expected_value(),
            Some(Value::List(vec![
                Value::Str("Bob".into()),
                Value::Str("Dana".into())
            ]))
        );
    }

    #[test]
    fn expected_output_is_optional_for_procedural_tasks() {
        // L2(절차) 태스크는 기대 출력 없이 기대 호출만 둘 수 있다.
        let toml = "id=\"a\"\ntier=\"hard\"\nquestion=\"q\"\ngrader=\"L2\"\n[[expected_tool_calls]]\ntool=\"send_email\"\ncount_min=1";
        let task = parse_task(toml).unwrap();
        assert_eq!(task.expected_value(), None);
        assert_eq!(task.expected_tool_calls[0].count_min, Some(1));
    }

    #[test]
    fn missing_required_field_is_a_parse_error() {
        // question 누락.
        let result = parse_task("id=\"a\"\ntier=\"easy\"\ngrader=\"L1\"\nexpected_output=1.0");
        assert!(matches!(result, Err(TaskError::Parse(_))));
    }

    #[test]
    fn loads_the_golden_task_file_from_disk() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tasks/easy_first_member.toml");
        let task = load_task(&path).unwrap();
        assert_eq!(task.id, "easy_first_member");
        assert_eq!(task.tier, "easy");
        assert_eq!(task.expected_value(), Some(Value::Str("Alice".into())));
    }

    #[test]
    fn missing_file_is_an_io_error() {
        let result = load_task(Path::new("/no/such/task.toml"));
        assert!(matches!(result, Err(TaskError::Io { .. })));
    }

    // ── 디렉터리 로더 (M3-T04) ──

    fn tasks_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tasks")
    }

    #[test]
    fn loads_the_whole_golden_suite_with_unique_ids() {
        let tasks = load_tasks_dir(&tasks_dir()).unwrap();
        assert!(tasks.len() >= 20, "스위트가 20개 미만: {}", tasks.len());
        let ids: BTreeSet<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids.len(), tasks.len(), "중복 id 존재");
    }

    #[test]
    fn directory_load_is_sorted_and_deterministic() {
        let first = load_tasks_dir(&tasks_dir()).unwrap();
        let second = load_tasks_dir(&tasks_dir()).unwrap();
        let ids: Vec<&str> = first.iter().map(|t| t.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "id가 정렬 순서로 로드되어야 한다");
        assert_eq!(
            ids,
            second.iter().map(|t| t.id.as_str()).collect::<Vec<_>>()
        );
    }

    /// 격리된 임시 디렉터리(테스트명으로 유일).
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ptc_t04_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn duplicate_ids_across_files_are_rejected() {
        let dir = temp_dir("dup");
        let body = "tier=\"easy\"\nquestion=\"q\"\ngrader=\"L1\"\nexpected_output=1.0";
        std::fs::write(dir.join("a.toml"), format!("id=\"same\"\n{body}")).unwrap();
        std::fs::write(dir.join("b.toml"), format!("id=\"same\"\n{body}")).unwrap();
        assert!(matches!(
            load_tasks_dir(&dir),
            Err(TaskError::DuplicateId(id)) if id == "same"
        ));
    }

    #[test]
    fn vacuous_l2_task_is_rejected() {
        let task = parse_task("id=\"x\"\ntier=\"hard\"\nquestion=\"q\"\ngrader=\"L2\"").unwrap();
        assert!(matches!(
            validate_task(&task),
            Err(TaskError::Invalid { .. })
        ));
    }

    #[test]
    fn l1_without_expected_output_is_rejected() {
        let task = parse_task("id=\"x\"\ntier=\"easy\"\nquestion=\"q\"\ngrader=\"L1\"").unwrap();
        assert!(matches!(
            validate_task(&task),
            Err(TaskError::Invalid { .. })
        ));
    }

    #[test]
    fn unknown_grader_is_rejected() {
        let task = parse_task(
            "id=\"x\"\ntier=\"easy\"\nquestion=\"q\"\ngrader=\"L9\"\nexpected_output=1.0",
        )
        .unwrap();
        assert!(matches!(
            validate_task(&task),
            Err(TaskError::Invalid { .. })
        ));
    }
}
