//! 코드 추출 — LLM 응답 텍스트에서 코드만 안정적으로 뽑는다 (M2-T03).
//!
//! 추출 규칙(설계 3.2절, 버전 관리 대상):
//! 1. 코드 펜스(```` ``` ````)가 있으면 **첫 번째 펜스 블록 내용만** 취한다.
//! 2. 펜스가 없으면 전체 텍스트를 코드로 간주한다.
//! 3. 언어 태그(```` ```python ````, ```` ```dsl ````)는 무시한다 — 우리 파서가
//!    실제 문법을 검증하므로 태그를 신뢰하지 않는다.
//!
//! 추출 자체는 파싱하지 않는다(SRP). 추출 결과의 분류는 [`Extraction::record_label`]로
//! 노출하고, "펜스 없는 코드가 파스 실패 시 extraction_fail" 같은 판정은 러너의 몫이다.

/// 추출 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Extraction {
    /// 펜스 블록에서 추출(언어 태그 제거됨).
    Fenced(String),
    /// 펜스가 없어 전체 텍스트를 코드로 간주.
    Whole(String),
    /// 추출할 코드가 없음(빈 응답 또는 빈 펜스) → extraction_fail.
    Empty,
}

impl Extraction {
    /// 추출된 코드(있으면).
    pub fn code(&self) -> Option<&str> {
        match self {
            Extraction::Fenced(code) | Extraction::Whole(code) => Some(code),
            Extraction::Empty => None,
        }
    }

    /// RunRecord의 `extraction` 필드 값.
    pub fn record_label(&self) -> &'static str {
        match self {
            Extraction::Fenced(_) => "fenced",
            Extraction::Whole(_) => "ok",
            Extraction::Empty => "extraction_fail",
        }
    }
}

/// 응답 텍스트에서 코드를 추출한다.
pub fn extract_code(text: &str) -> Extraction {
    match first_fenced_block(text) {
        Some(code) if !code.trim().is_empty() => Extraction::Fenced(code),
        Some(_) => Extraction::Empty, // 빈 펜스
        None => {
            let whole = text.trim();
            if whole.is_empty() {
                Extraction::Empty
            } else {
                Extraction::Whole(whole.to_string())
            }
        }
    }
}

/// 첫 펜스 블록의 내용을 돌려준다. 펜스가 없으면 `None`.
/// 여는 펜스 줄(태그 포함)은 건너뛰고, 닫는 펜스가 없으면 끝까지 취한다(잘린 응답 대비).
fn first_fenced_block(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let open = lines.iter().position(|line| is_fence(line))?;
    let rest = &lines[open + 1..];
    let body = match rest.iter().position(|line| is_fence(line)) {
        Some(close) => &rest[..close],
        None => rest,
    };
    Some(body.join("\n"))
}

fn is_fence(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fenced_block_is_extracted_and_language_tag_ignored() {
        let text = "여기 코드입니다:\n```dsl\nemit(1)\n```\n끝.";
        assert_eq!(
            extract_code(text),
            Extraction::Fenced("emit(1)".to_string())
        );
    }

    #[test]
    fn multiline_code_inside_fence_is_preserved() {
        let text = "```python\nteam = list_team(\"eng\")\nemit(team[0].name)\n```";
        assert_eq!(
            extract_code(text),
            Extraction::Fenced("team = list_team(\"eng\")\nemit(team[0].name)".to_string())
        );
    }

    #[test]
    fn first_fence_block_wins_when_multiple() {
        let text = "```\nemit(1)\n```\nsome prose\n```\nemit(2)\n```";
        assert_eq!(
            extract_code(text),
            Extraction::Fenced("emit(1)".to_string())
        );
    }

    #[test]
    fn unclosed_fence_takes_rest_of_text() {
        let text = "```dsl\nemit(1)\nemit(2)";
        assert_eq!(
            extract_code(text),
            Extraction::Fenced("emit(1)\nemit(2)".to_string())
        );
    }

    #[test]
    fn text_without_fence_is_treated_as_whole_code() {
        assert_eq!(
            extract_code("  emit(1)  "),
            Extraction::Whole("emit(1)".to_string())
        );
    }

    #[test]
    fn empty_text_is_extraction_failure() {
        assert_eq!(extract_code("   \n  "), Extraction::Empty);
    }

    #[test]
    fn empty_fence_is_extraction_failure() {
        assert_eq!(extract_code("```dsl\n```"), Extraction::Empty);
    }

    #[test]
    fn record_labels_match_taxonomy() {
        assert_eq!(extract_code("```\nemit(1)\n```").record_label(), "fenced");
        assert_eq!(extract_code("emit(1)").record_label(), "ok");
        assert_eq!(extract_code("").record_label(), "extraction_fail");
    }

    #[test]
    fn code_accessor_returns_none_only_when_empty() {
        assert!(extract_code("emit(1)").code().is_some());
        assert!(extract_code("").code().is_none());
    }
}
