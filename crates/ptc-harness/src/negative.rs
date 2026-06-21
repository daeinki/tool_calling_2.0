//! 음성 테스트 스위트 — 거부되어야 할 입력과 그 실패 분류 (M1-T11).
//!
//! 각 입력이 파이프라인의 **어느 단계에서** 거부되는지를 실패 분류(taxonomy)와
//! 매핑한다. 이로써 "인터프리터 버그"와 "잘못된 입력"을 정직하게 가른다.
//!
//! 여기서 도달 가능한 분류는 파이프라인 3단계뿐이다(lex/parse → PARSE_ERROR,
//! validate → VALIDATION_REJECT, interpret → RUNTIME_ERROR). 나머지 taxonomy
//! 항목(EXTRACTION_FAIL·WRONG_ANSWER·HARNESS_BUG)은 LLM·채점 계층이 생기는
//! M2 이후에 다룬다.

use crate::taxonomy::FailureCategory;
use ptc_dsl::{parse, tokenize, Interpreter, ToolCatalog, MAX_NESTING_DEPTH};
use ptc_tools::{tool_names, MockToolServer};

// 파이프라인이 도달 가능한 분류는 Parse/Validation/Runtime 셋뿐이다.
// 분류 정의는 `taxonomy` 모듈 하나에 있고 여기서 재사용한다(DRY).

/// 거부되어야 할 스크립트 하나와 그 기대 분류.
pub struct NegativeCase {
    pub name: &'static str,
    pub source: String,
    pub expected: FailureCategory,
}

/// 음성 케이스 한 건의 실행 결과.
#[derive(Debug)]
pub struct NegativeReport {
    pub name: String,
    pub expected: FailureCategory,
    /// 실제 거부 분류. `None`이면 거부되지 않음(= 음성 테스트 실패).
    pub actual: Option<FailureCategory>,
}

impl NegativeReport {
    pub fn rejected_as_expected(&self) -> bool {
        self.actual == Some(self.expected)
    }
}

/// 스크립트를 파이프라인에 흘려 거부 단계를 분류한다. 끝까지 성공하면 `None`.
pub fn classify(source: &str) -> Option<FailureCategory> {
    let tokens = match tokenize(source) {
        Ok(tokens) => tokens,
        Err(_) => return Some(FailureCategory::Parse),
    };
    let program = match parse(tokens) {
        Ok(program) => program,
        Err(_) => return Some(FailureCategory::Parse),
    };
    let catalog = ToolCatalog::new(tool_names());
    if ptc_dsl::validate(&program, &catalog).is_err() {
        return Some(FailureCategory::Validation);
    }
    let mut server = MockToolServer::new();
    match Interpreter::new(&mut server).run(&program) {
        Ok(_) => None,
        Err(_) => Some(FailureCategory::Runtime),
    }
}

pub fn run_negative(case: &NegativeCase) -> NegativeReport {
    NegativeReport {
        name: case.name.to_string(),
        expected: case.expected,
        actual: classify(&case.source),
    }
}

/// 10개의 음성 케이스(parse 4 · validation 3 · runtime 3).
pub fn cases() -> Vec<NegativeCase> {
    use FailureCategory::{Parse, Runtime, Validation};
    vec![
        // ── PARSE_ERROR (어휘·구문) ──
        case("lex_unexpected_char", "x = @", Parse),
        case("lex_tab_indent", "if True:\n\temit(1)", Parse),
        case("parse_missing_colon", "for m in team\n    emit(m)", Parse),
        case("parse_unclosed_paren", "emit(1", Parse),
        // ── VALIDATION_REJECT (정적 검증) ──
        case("validation_unknown_tool", "x = frobnicate(1)", Validation),
        case(
            "validation_forbidden_call_target",
            "team = list_team(\"eng\")\nx = team[0]()",
            Validation,
        ),
        NegativeCase {
            name: "validation_nesting_too_deep",
            source: nested_for_source(MAX_NESTING_DEPTH + 1),
            expected: Validation,
        },
        // ── RUNTIME_ERROR (실행) ──
        case("runtime_undefined_variable", "emit(missing)", Runtime),
        case(
            "runtime_type_mismatch_iter",
            "for x in 5:\n    emit(x)",
            Runtime,
        ),
        case(
            "runtime_key_not_found",
            "team = list_team(\"eng\")\nemit(team[0].salary)",
            Runtime,
        ),
    ]
}

fn case(name: &'static str, source: &str, expected: FailureCategory) -> NegativeCase {
    NegativeCase {
        name,
        source: source.to_string(),
        expected,
    }
}

/// `levels`겹 중첩된 for 루프(검증기의 중첩 한계 초과 유도용).
fn nested_for_source(levels: usize) -> String {
    let mut src = String::new();
    for level in 0..levels {
        src.push_str(&"    ".repeat(level));
        src.push_str("for x in items:\n");
    }
    src.push_str(&"    ".repeat(levels));
    src.push_str("emit(1)");
    src
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_ten_cases_spanning_all_categories() {
        let all = cases();
        assert_eq!(all.len(), 10);
        for category in [
            FailureCategory::Parse,
            FailureCategory::Validation,
            FailureCategory::Runtime,
        ] {
            assert!(
                all.iter().any(|c| c.expected == category),
                "no case for {category}"
            );
        }
    }

    #[test]
    fn every_case_is_rejected_at_its_expected_stage() {
        for case in cases() {
            let report = run_negative(&case);
            assert!(
                report.rejected_as_expected(),
                "case '{}' expected {} but got {:?}",
                report.name,
                report.expected,
                report.actual,
            );
        }
    }

    #[test]
    fn a_valid_program_is_not_classified_as_a_failure() {
        // 분류기가 정상 프로그램을 거부로 오인하지 않는지 확인(거짓 양성 방지).
        assert_eq!(classify("emit(1)"), None);
    }
}
