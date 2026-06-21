//! 소스 위치 정보. 모든 진단(에러)이 줄·열을 담아 실패 분류를 가능하게 한다.

use std::fmt;

/// 소스 코드의 한 지점. 줄과 열은 1부터 시작한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: u32,
    pub col: u32,
}

impl Span {
    pub fn new(line: u32, col: u32) -> Self {
        Self { line, col }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_as_line_colon_col() {
        assert_eq!(Span::new(3, 7).to_string(), "3:7");
    }

    #[test]
    fn equal_spans_compare_equal() {
        assert_eq!(Span::new(1, 1), Span::new(1, 1));
        assert_ne!(Span::new(1, 1), Span::new(1, 2));
    }
}
