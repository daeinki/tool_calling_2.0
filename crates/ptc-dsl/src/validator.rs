//! Validator — 실행 전 AST를 훑어 거부할 프로그램을 걸러낸다 (M1-T06).
//!
//! 세 가지를 거부한다:
//! 1. **미등록 도구 호출** — 호출 대상 이름이 주입된 [`ToolCatalog`]에 없음.
//! 2. **중첩 깊이 초과** — 제어흐름(for/if) 블록이 [`MAX_NESTING_DEPTH`]보다 깊음.
//! 3. **금지 노드** — 호출 대상이 이름으로 해석되지 않음(`f()()`, `a[0]()` 등).
//!
//! DI: 어떤 이름이 합법인지는 호출자가 [`ToolCatalog`]로 주입한다. 검증기는
//! 카탈로그가 어떻게 채워지는지(mock인지 실제 MCP인지) 알지 못한다.

use crate::ast::{callee_name, Arg, Expr, ExprKind, Stmt};
use crate::error::ValidationError;
use crate::span::Span;
use std::collections::BTreeSet;

/// 제어흐름 블록의 최대 중첩 깊이. LLM이 생성한 과도하게 깊은(따라서
/// 다루기 어려운) 코드를 실행 전에 거부한다.
pub const MAX_NESTING_DEPTH: usize = 8;

/// 호출 가능한 이름의 등록부. `namespace.tool` 형태도 하나의 이름으로 본다.
#[derive(Debug, Clone, Default)]
pub struct ToolCatalog {
    names: BTreeSet<String>,
}

impl ToolCatalog {
    pub fn new<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            names: names.into_iter().map(Into::into).collect(),
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }
}

/// 프로그램이 정적 규칙을 모두 만족하면 `Ok(())`, 아니면 첫 위반을 돌려준다.
pub fn validate(program: &[Stmt], catalog: &ToolCatalog) -> Result<(), ValidationError> {
    let validator = Validator { catalog };
    validator.check_block(program, 0)
}

struct Validator<'a> {
    catalog: &'a ToolCatalog,
}

impl Validator<'_> {
    fn check_block(&self, stmts: &[Stmt], depth: usize) -> Result<(), ValidationError> {
        for stmt in stmts {
            self.check_stmt(stmt, depth)?;
        }
        Ok(())
    }

    fn check_stmt(&self, stmt: &Stmt, depth: usize) -> Result<(), ValidationError> {
        match stmt {
            Stmt::Assign { value, .. } => self.check_expr(value),
            Stmt::Emit { value, .. } => self.check_expr(value),
            Stmt::Expr { expr, .. } => self.check_expr(expr),
            Stmt::For {
                iter, body, span, ..
            } => {
                self.check_expr(iter)?;
                self.descend(body, depth, *span)
            }
            Stmt::If {
                cond,
                then,
                els,
                span,
            } => {
                self.check_expr(cond)?;
                self.descend(then, depth, *span)?;
                self.descend(els, depth, *span)
            }
        }
    }

    /// 블록 한 단계 안으로 들어간다. 새 깊이가 한계를 넘으면 거부한다.
    fn descend(&self, body: &[Stmt], depth: usize, span: Span) -> Result<(), ValidationError> {
        let inner = depth + 1;
        if inner > MAX_NESTING_DEPTH {
            return Err(ValidationError::NestingTooDeep {
                depth: inner,
                max: MAX_NESTING_DEPTH,
                span,
            });
        }
        self.check_block(body, inner)
    }

    fn check_expr(&self, expr: &Expr) -> Result<(), ValidationError> {
        match &expr.kind {
            ExprKind::Num(_)
            | ExprKind::Str(_)
            | ExprKind::Bool(_)
            | ExprKind::None
            | ExprKind::Var(_) => Ok(()),
            ExprKind::List(items) => {
                for item in items {
                    self.check_expr(item)?;
                }
                Ok(())
            }
            ExprKind::Member { base, .. } => self.check_expr(base),
            ExprKind::Index { base, idx } => {
                self.check_expr(base)?;
                self.check_expr(idx)
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.check_expr(lhs)?;
                self.check_expr(rhs)
            }
            ExprKind::Call { callee, args } => self.check_call(callee, args, expr.span),
        }
    }

    fn check_call(&self, callee: &Expr, args: &[Arg], span: Span) -> Result<(), ValidationError> {
        match callee_name(callee) {
            Some(name) if self.catalog.contains(&name) => {}
            Some(name) => return Err(ValidationError::UnknownTool { name, span }),
            None => {
                return Err(ValidationError::ForbiddenNode {
                    node: "call target".into(),
                    span,
                })
            }
        }
        for arg in args {
            self.check_expr(arg_expr(arg))?;
        }
        Ok(())
    }
}

fn arg_expr(arg: &Arg) -> &Expr {
    match arg {
        Arg::Pos(expr) => expr,
        Arg::Kw(_, expr) => expr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;
    use crate::parser::parse;

    fn catalog() -> ToolCatalog {
        ToolCatalog::new(["list_team", "get_expenses", "get_budget", "hr.list_team"])
    }

    fn check(src: &str) -> Result<(), ValidationError> {
        let program = parse(tokenize(src).expect("lexes")).expect("parses");
        validate(&program, &catalog())
    }

    #[test]
    fn program_calling_only_registered_tools_passes() {
        let src = "team = list_team(\"eng\")\nfor m in team:\n    x = get_expenses(m.id)";
        assert!(check(src).is_ok());
    }

    #[test]
    fn namespaced_tool_call_passes_when_registered() {
        assert!(check("team = hr.list_team(\"eng\")").is_ok());
    }

    #[test]
    fn member_access_on_a_value_is_not_a_tool_call() {
        // m.id 는 값 접근이지 호출이 아니므로 카탈로그 검사를 받지 않는다.
        assert!(check("for m in list_team(\"eng\"):\n    emit(m.id)").is_ok());
    }

    // ── 음성 테스트 ──

    #[test]
    fn unregistered_tool_is_rejected_with_its_name() {
        match check("x = frobnicate(1)") {
            Err(ValidationError::UnknownTool { name, .. }) => assert_eq!(name, "frobnicate"),
            other => panic!("expected UnknownTool, got {other:?}"),
        }
    }

    #[test]
    fn unregistered_namespaced_tool_is_rejected() {
        match check("x = hr.unknown(1)") {
            Err(ValidationError::UnknownTool { name, .. }) => assert_eq!(name, "hr.unknown"),
            other => panic!("expected UnknownTool, got {other:?}"),
        }
    }

    #[test]
    fn unregistered_tool_nested_in_arguments_is_rejected() {
        // 바깥 호출은 등록돼 있지만 인자 안의 호출은 미등록.
        match check("x = get_expenses(frobnicate(1))") {
            Err(ValidationError::UnknownTool { name, .. }) => assert_eq!(name, "frobnicate"),
            other => panic!("expected UnknownTool, got {other:?}"),
        }
    }

    #[test]
    fn call_target_that_is_not_a_name_is_forbidden() {
        match check("x = team[0]()") {
            Err(ValidationError::ForbiddenNode { .. }) => {}
            other => panic!("expected ForbiddenNode, got {other:?}"),
        }
    }

    #[test]
    fn nesting_within_limit_passes() {
        assert!(validate(&nested_for_program(MAX_NESTING_DEPTH), &catalog()).is_ok());
    }

    #[test]
    fn nesting_beyond_limit_is_rejected() {
        match validate(&nested_for_program(MAX_NESTING_DEPTH + 1), &catalog()) {
            Err(ValidationError::NestingTooDeep { depth, max, .. }) => {
                assert_eq!(max, MAX_NESTING_DEPTH);
                assert!(depth > max);
            }
            other => panic!("expected NestingTooDeep, got {other:?}"),
        }
    }

    /// `levels`겹 중첩된 for 루프 프로그램을 만든다(가장 안쪽 본문 깊이 = levels).
    fn nested_for_program(levels: usize) -> Vec<Stmt> {
        let mut src = String::new();
        for level in 0..levels {
            let indent = "    ".repeat(level);
            src.push_str(&format!("{indent}for x in items:\n"));
        }
        let indent = "    ".repeat(levels);
        src.push_str(&format!("{indent}emit(1)"));
        parse(tokenize(&src).expect("lexes")).expect("parses")
    }
}
