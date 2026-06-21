//! 실패 분류(taxonomy) — 설계 4.4절의 6분류 (M2-T07).
//!
//! 실패를 단일 'fail'로 뭉뚱그리지 않고 원인별로 분류해, **인터프리터 버그와
//! LLM 약점을 분리**한다. 각 분류는 책임 소재([`FailureCategory::responsibility`])를
//! 함께 가진다.
//!
//! M1(T11)은 파이프라인에서 도달 가능한 3분류(Parse/Validation/Runtime)만 썼고,
//! M2에서 LLM·채점 계층이 생기며 나머지(ExtractionFail/WrongAnswer/HarnessBug)가
//! 더해진다. 정의는 여기 하나뿐이며 음성 스위트가 이를 재사용한다(DRY).

use std::fmt;

/// 실행 한 건의 실패 원인 분류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureCategory {
    /// 응답에서 코드를 추출하지 못함.
    ExtractionFail,
    /// 추출된 코드가 DSL 문법 위반.
    Parse,
    /// 미등록 도구·깊이 초과 등 정적 검증 거부.
    Validation,
    /// 실행 중 타입 오류·키 없음 등.
    Runtime,
    /// 실행됐으나 답이 틀림.
    WrongAnswer,
    /// 채점기·mock 자체 오류(하네스 책임).
    HarnessBug,
}

impl FailureCategory {
    /// 모든 분류(보고서의 분포 집계용).
    pub const ALL: [FailureCategory; 6] = [
        FailureCategory::ExtractionFail,
        FailureCategory::Parse,
        FailureCategory::Validation,
        FailureCategory::Runtime,
        FailureCategory::WrongAnswer,
        FailureCategory::HarnessBug,
    ];

    /// RunRecord·보고서에 쓰는 분류 문자열.
    pub fn label(&self) -> &'static str {
        match self {
            FailureCategory::ExtractionFail => "EXTRACTION_FAIL",
            FailureCategory::Parse => "PARSE_ERROR",
            FailureCategory::Validation => "VALIDATION_REJECT",
            FailureCategory::Runtime => "RUNTIME_ERROR",
            FailureCategory::WrongAnswer => "WRONG_ANSWER",
            FailureCategory::HarnessBug => "HARNESS_BUG",
        }
    }

    /// 책임 소재 — "우리 탓"과 "모델 탓"을 정직하게 가른다.
    pub fn responsibility(&self) -> &'static str {
        match self {
            FailureCategory::ExtractionFail => "프롬프트 / 추출 규칙",
            FailureCategory::Parse => "LLM (잘못된 문법 생성)",
            FailureCategory::Validation => "LLM (없는 도구 호출 등)",
            FailureCategory::Runtime => "LLM 로직 / 인터프리터",
            FailureCategory::WrongAnswer => "LLM 로직",
            FailureCategory::HarnessBug => "하네스 (우리 책임)",
        }
    }

    /// 하네스 자체 결함인가(게이트를 막는다 — 4.4절).
    pub fn is_harness_fault(&self) -> bool {
        matches!(self, FailureCategory::HarnessBug)
    }
}

impl fmt::Display for FailureCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_six_categories_are_enumerated() {
        assert_eq!(FailureCategory::ALL.len(), 6);
    }

    #[test]
    fn labels_match_design_taxonomy() {
        assert_eq!(FailureCategory::ExtractionFail.label(), "EXTRACTION_FAIL");
        assert_eq!(FailureCategory::Parse.to_string(), "PARSE_ERROR");
        assert_eq!(FailureCategory::HarnessBug.label(), "HARNESS_BUG");
    }

    #[test]
    fn labels_are_unique() {
        let mut labels: Vec<&str> = FailureCategory::ALL.iter().map(|c| c.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), 6);
    }

    #[test]
    fn only_harness_bug_is_our_fault() {
        for category in FailureCategory::ALL {
            assert_eq!(
                category.is_harness_fault(),
                category == FailureCategory::HarnessBug
            );
            assert!(!category.responsibility().is_empty());
        }
    }
}
