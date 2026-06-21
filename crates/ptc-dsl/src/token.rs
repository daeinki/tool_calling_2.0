//! 토큰 — lexer(T03)가 생성하고 parser(T04/T05)가 소비하는 순수 자료구조.
//!
//! `TokenKind`는 토큰의 종류, `Token`은 거기에 [`Span`]을 더한 것이다.
//! (AST의 `ExprKind`/`Expr`와 같은 분리 패턴.)

use crate::span::Span;
use std::fmt;

/// 토큰의 종류.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // 리터럴
    Num(f64),
    Str(String),
    Ident(String),
    True,
    False,
    None,
    // 키워드
    For,
    In,
    If,
    Else,
    Emit,
    // 논리 연산자(키워드형)
    And,
    Or,
    // 산술·비교 연산자
    Plus,
    Minus,
    Star,
    Slash,
    Gt,
    Lt,
    Ge,
    Le,
    EqEq,
    Ne,
    Assign,
    // 구두점
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Colon,
    Dot,
    // 레이아웃
    Newline,
    Indent,
    Dedent,
    Eof,
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            TokenKind::Num(n) => return write!(f, "{n}"),
            TokenKind::Str(s) => return write!(f, "\"{s}\""),
            TokenKind::Ident(name) => return write!(f, "{name}"),
            TokenKind::True => "True",
            TokenKind::False => "False",
            TokenKind::None => "None",
            TokenKind::For => "for",
            TokenKind::In => "in",
            TokenKind::If => "if",
            TokenKind::Else => "else",
            TokenKind::Emit => "emit",
            TokenKind::And => "and",
            TokenKind::Or => "or",
            TokenKind::Plus => "+",
            TokenKind::Minus => "-",
            TokenKind::Star => "*",
            TokenKind::Slash => "/",
            TokenKind::Gt => ">",
            TokenKind::Lt => "<",
            TokenKind::Ge => ">=",
            TokenKind::Le => "<=",
            TokenKind::EqEq => "==",
            TokenKind::Ne => "!=",
            TokenKind::Assign => "=",
            TokenKind::LParen => "(",
            TokenKind::RParen => ")",
            TokenKind::LBracket => "[",
            TokenKind::RBracket => "]",
            TokenKind::Comma => ",",
            TokenKind::Colon => ":",
            TokenKind::Dot => ".",
            TokenKind::Newline => "<newline>",
            TokenKind::Indent => "<indent>",
            TokenKind::Dedent => "<dedent>",
            TokenKind::Eof => "<eof>",
        };
        write!(f, "{s}")
    }
}

/// 토큰 = 종류 + 소스 위치.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}
