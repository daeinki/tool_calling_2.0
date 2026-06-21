//! 추상 구문 트리(AST). 파서(T04/T05)가 생성하고, 검증기(T06)와
//! 인터프리터(T07/T08)가 소비하는 순수 자료구조다.
//!
//! 설계 원칙(clean-code §5): AST는 **데이터만 노출하고 동작은 갖지 않는다**.
//! 유일한 메서드는 위치 정보를 돌려주는 `span` 접근자뿐이며, 평가·검증 같은
//! 동작은 인터프리터·검증기 쪽에 둔다.
//!
//! 모든 노드는 [`Span`]을 담아, 검증·실행 에러가 줄·열을 가리킬 수 있게 한다.
//! `Stmt`는 설계 문서대로 변형마다 `span` 필드를 두고, `Expr`는 재귀가 깊어
//! 변형이 많으므로 `kind`(종류)와 `span`을 분리한 래퍼로 둔다.

use crate::span::Span;

/// 문장. 프로그램은 문장의 나열이다.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `name = value`
    Assign {
        name: String,
        value: Expr,
        span: Span,
    },
    /// `for var in iter: body`
    For {
        var: String,
        iter: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
    /// `if cond: then else: els` (`els`는 else 절이 없으면 빈 벡터)
    If {
        cond: Expr,
        then: Vec<Stmt>,
        els: Vec<Stmt>,
        span: Span,
    },
    /// `emit(value)` — 최종 결과 반환
    Emit { value: Expr, span: Span },
    /// 표현식 문장 (예: 결과를 버리는 도구 호출)
    Expr { expr: Expr, span: Span },
}

impl Stmt {
    /// 이 문장의 소스 위치.
    pub fn span(&self) -> Span {
        match self {
            Stmt::Assign { span, .. }
            | Stmt::For { span, .. }
            | Stmt::If { span, .. }
            | Stmt::Emit { span, .. }
            | Stmt::Expr { span, .. } => *span,
        }
    }
}

/// 표현식 노드 = 종류([`ExprKind`]) + 소스 위치([`Span`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// 표현식의 종류. 재귀 노드는 [`Box`]로 감싼다.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Num(f64),
    Str(String),
    Bool(bool),
    None,
    Var(String),
    List(Vec<Expr>),
    /// `base.field`
    Member {
        base: Box<Expr>,
        field: String,
    },
    /// `base[idx]`
    Index {
        base: Box<Expr>,
        idx: Box<Expr>,
    },
    /// `callee(args)`
    Call {
        callee: Box<Expr>,
        args: Vec<Arg>,
    },
    /// `lhs op rhs`
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
}

/// 호출 인자: 위치 인자 또는 키워드 인자.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Pos(Expr),
    Kw(String, Expr),
}

/// 이항 연산자. 문법의 `OP`에 대응한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
    And,
    Or,
}

/// 호출 대상을 도구 이름으로 해석한다. 단순 이름과 `namespace.tool`만 허용하며,
/// 그 외(계산식을 호출하는 등)는 `None`이다. 검증기와 인터프리터가 공유한다.
pub fn callee_name(callee: &Expr) -> Option<String> {
    match &callee.kind {
        ExprKind::Var(name) => Some(name.clone()),
        ExprKind::Member { base, field } => match &base.kind {
            ExprKind::Var(namespace) => Some(format!("{namespace}.{field}")),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 다음 프로그램에 해당하는 AST를 손으로 구성한다:
    ///
    /// ```text
    /// team = list_team("eng")
    /// for m in team:
    ///     emit(m.name)
    /// ```
    fn sample_program() -> Vec<Stmt> {
        let sp = Span::new(1, 1);

        let call_list_team = Expr::new(
            ExprKind::Call {
                callee: Box::new(Expr::new(ExprKind::Var("list_team".into()), sp)),
                args: vec![Arg::Pos(Expr::new(ExprKind::Str("eng".into()), sp))],
            },
            sp,
        );

        let member_name = Expr::new(
            ExprKind::Member {
                base: Box::new(Expr::new(ExprKind::Var("m".into()), sp)),
                field: "name".into(),
            },
            sp,
        );

        vec![
            Stmt::Assign {
                name: "team".into(),
                value: call_list_team,
                span: sp,
            },
            Stmt::For {
                var: "m".into(),
                iter: Expr::new(ExprKind::Var("team".into()), sp),
                body: vec![Stmt::Emit {
                    value: member_name,
                    span: sp,
                }],
                span: sp,
            },
        ]
    }

    #[test]
    fn hand_built_ast_clones_equal() {
        let program = sample_program();
        let cloned = program.clone();
        assert_eq!(program, cloned);
    }

    #[test]
    fn debug_renders_non_empty() {
        let program = sample_program();
        assert!(!format!("{program:?}").is_empty());
    }

    #[test]
    fn stmt_span_accessor_returns_location() {
        let stmt = Stmt::Emit {
            value: Expr::new(ExprKind::None, Span::new(5, 9)),
            span: Span::new(5, 1),
        };
        assert_eq!(stmt.span(), Span::new(5, 1));
    }

    #[test]
    fn expr_carries_its_own_span() {
        let expr = Expr::new(ExprKind::Var("x".into()), Span::new(3, 4));
        assert_eq!(expr.span, Span::new(3, 4));
    }
}
