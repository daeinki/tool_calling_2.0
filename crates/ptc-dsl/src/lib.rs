//! `ptc-dsl` — Programmatic Tool Calling용 커스텀 DSL.
//!
//! 파이프라인: lex → parse → validate → interpret.
//! 인터프리터는 외부(MCP/HTTP)를 모르며, 등록된 도구 호출만이 유일한 외부 효과다.
//!
//! 현재 구현 범위: M1-T01(공통 타입 — [`Span`]과 단계별 에러 골격),
//! M1-T02(AST 정의), M1-T03(lexer + 토큰), M1-T04/T05(parser — 표현식·문장),
//! M1-T06(validator — 정적 검증), M1-T07(interpreter — 값·ToolSink·제어흐름).

pub mod ast;
pub mod error;
pub mod interp;
pub mod lexer;
pub mod parser;
pub mod span;
pub mod token;
pub mod validator;

pub use ast::{Arg, BinOp, Expr, ExprKind, Stmt};
pub use error::{LexError, ParseError, RuntimeError, ToolError, ValidationError};
pub use interp::{Interpreter, ToolSink, Value};
pub use lexer::tokenize;
pub use parser::parse;
pub use span::Span;
pub use token::{Token, TokenKind};
pub use validator::{validate, ToolCatalog, MAX_NESTING_DEPTH};
