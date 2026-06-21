//! DSL 파이프라인 각 단계의 도메인 에러.
//!
//! 단계마다 별도 에러 타입을 두어(lex → parse → validate → run) 책임을 분리한다.
//! 모든 에러는 [`Span`]을 담아, 실패가 어디서 났는지(줄·열)와
//! 어떤 종류인지를 함께 전달한다. 이는 하네스의 실패 분류(taxonomy)와
//! 줄·열이 찍힌 진단 메시지의 토대다.
//!
//! 여기서는 각 단계의 대표 변형(variant)만 정의하는 **골격**이다.
//! 후속 티켓(T03 lexer, T04/T05 parser, T06 validator, T08 interpreter)이
//! 필요한 변형을 같은 패턴으로 채워 넣는다.

use crate::span::Span;
use thiserror::Error;

/// 토큰화(lexing) 단계 에러. (T03에서 확장)
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LexError {
    #[error("[{span}] 예상치 못한 문자 '{ch}'")]
    UnexpectedChar { ch: char, span: Span },

    #[error("[{span}] 들여쓰기가 일관되지 않음")]
    InconsistentIndent { span: Span },

    #[error("[{span}] 문자열이 닫히지 않음")]
    UnterminatedString { span: Span },
}

impl LexError {
    pub fn span(&self) -> Span {
        match self {
            LexError::UnexpectedChar { span, .. }
            | LexError::InconsistentIndent { span }
            | LexError::UnterminatedString { span } => *span,
        }
    }
}

/// 구문 분석(parsing) 단계 에러. (T04/T05에서 확장)
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    #[error("[{span}] '{expected}'을(를) 기대했으나 '{found}'을(를) 만남")]
    UnexpectedToken {
        expected: String,
        found: String,
        span: Span,
    },

    #[error("[{span}] 입력이 예기치 않게 끝남")]
    UnexpectedEof { span: Span },
}

impl ParseError {
    pub fn span(&self) -> Span {
        match self {
            ParseError::UnexpectedToken { span, .. } | ParseError::UnexpectedEof { span } => *span,
        }
    }
}

/// 정적 검증(validation) 단계 에러. (T06에서 확장)
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("[{span}] 등록되지 않은 도구 '{name}'")]
    UnknownTool { name: String, span: Span },

    #[error("[{span}] 중첩 깊이 {depth}이(가) 허용치 {max}을(를) 초과")]
    NestingTooDeep {
        depth: usize,
        max: usize,
        span: Span,
    },

    #[error("[{span}] 허용되지 않은 노드 '{node}'")]
    ForbiddenNode { node: String, span: Span },
}

impl ValidationError {
    pub fn span(&self) -> Span {
        match self {
            ValidationError::UnknownTool { span, .. }
            | ValidationError::NestingTooDeep { span, .. }
            | ValidationError::ForbiddenNode { span, .. } => *span,
        }
    }
}

/// 실행(interpretation) 단계 에러. (T08에서 확장)
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuntimeError {
    #[error("[{span}] 타입 불일치: '{expected}'을(를) 기대했으나 '{found}'")]
    TypeMismatch {
        expected: String,
        found: String,
        span: Span,
    },

    #[error("[{span}] 정의되지 않은 변수 '{name}'")]
    UndefinedVariable { name: String, span: Span },

    #[error("[{span}] 키 '{key}'을(를) 찾을 수 없음")]
    KeyNotFound { key: String, span: Span },

    #[error("[{span}] 도구 '{tool}' 호출 실패: {reason}")]
    ToolFailed {
        tool: String,
        reason: String,
        span: Span,
    },
}

impl RuntimeError {
    pub fn span(&self) -> Span {
        match self {
            RuntimeError::TypeMismatch { span, .. }
            | RuntimeError::UndefinedVariable { span, .. }
            | RuntimeError::KeyNotFound { span, .. }
            | RuntimeError::ToolFailed { span, .. } => *span,
        }
    }
}

/// 도구 호출 단계 에러. 위치(span)는 호출 측(인터프리터)이 보유하므로 담지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ToolError {
    #[error("등록되지 않은 도구 '{0}'")]
    Unknown(String),

    #[error("도구 실행 실패: {0}")]
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lex_error_displays_span_and_cause() {
        let err = LexError::UnexpectedChar {
            ch: '$',
            span: Span::new(2, 5),
        };
        assert_eq!(err.to_string(), "[2:5] 예상치 못한 문자 '$'");
    }

    #[test]
    fn parse_error_displays_span_and_cause() {
        let err = ParseError::UnexpectedToken {
            expected: ":".into(),
            found: "for".into(),
            span: Span::new(4, 1),
        };
        assert_eq!(
            err.to_string(),
            "[4:1] ':'을(를) 기대했으나 'for'을(를) 만남"
        );
    }

    #[test]
    fn validation_error_displays_span_and_cause() {
        let err = ValidationError::NestingTooDeep {
            depth: 9,
            max: 8,
            span: Span::new(12, 3),
        };
        assert_eq!(
            err.to_string(),
            "[12:3] 중첩 깊이 9이(가) 허용치 8을(를) 초과"
        );
    }

    #[test]
    fn runtime_error_displays_span_and_cause() {
        let err = RuntimeError::UndefinedVariable {
            name: "team".into(),
            span: Span::new(7, 10),
        };
        assert_eq!(err.to_string(), "[7:10] 정의되지 않은 변수 'team'");
    }

    #[test]
    fn span_accessor_returns_location_for_every_phase() {
        let s = Span::new(1, 2);
        assert_eq!(LexError::InconsistentIndent { span: s }.span(), s);
        assert_eq!(ParseError::UnexpectedEof { span: s }.span(), s);
        assert_eq!(
            ValidationError::UnknownTool {
                name: "x".into(),
                span: s
            }
            .span(),
            s
        );
        assert_eq!(
            RuntimeError::KeyNotFound {
                key: "k".into(),
                span: s
            }
            .span(),
            s
        );
    }
}
