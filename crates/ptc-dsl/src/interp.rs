//! Interpreter — 검증된 AST를 실행한다 (M1-T07: 값 모델·ToolSink·제어흐름).
//!
//! 인터프리터는 외부를 모른다. 도구 호출은 [`ToolSink`] trait 하나로만 외부와
//! 만나며, MCP·JSON-RPC·HTTP를 전혀 알지 못한다. 검증에서는 mock이, 프로덕션에서는
//! 실제 MCP 클라이언트가 이 trait를 구현한다.
//!
//! 이 티켓은 **제어흐름**(assign/for/if/emit)과 그것을 구동하는 데 필요한
//! 표현식(리터럴·변수·리스트)만 평가한다. 계산식(binary/member/index)과
//! 도구 호출(call)은 M1-T08이 채운다.

use crate::ast::{callee_name, Arg, BinOp, Expr, ExprKind, Stmt};
use crate::error::{RuntimeError, ToolError};
use crate::span::Span;
use std::collections::BTreeMap;

/// 인터프리터의 런타임 값. JSON과 1:1로 매핑되어 도구 결과를 그대로 받는다.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Str(String),
    Bool(bool),
    Null,
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
}

impl Value {
    /// 에러 메시지용 타입 이름.
    fn type_name(&self) -> &'static str {
        match self {
            Value::Num(_) => "num",
            Value::Str(_) => "str",
            Value::Bool(_) => "bool",
            Value::Null => "null",
            Value::List(_) => "list",
            Value::Map(_) => "map",
        }
    }

    /// 조건식의 참 여부(Python스러운 규칙).
    fn is_truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Null => false,
            Value::Num(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::List(items) => !items.is_empty(),
            Value::Map(m) => !m.is_empty(),
        }
    }
}

/// 도구 호출이 외부와 만나는 유일한 경계. 인터프리터는 이 trait 뒤만 본다.
pub trait ToolSink {
    fn call(&mut self, tool: &str, args: BTreeMap<String, Value>) -> Result<Value, ToolError>;
}

/// 문장 실행의 흐름 제어. `emit`은 즉시 최종값을 들고 위로 빠져나간다.
enum Flow {
    Normal,
    Emit(Value),
}

/// 트리-워킹 인터프리터. 변수는 단일 평탄 환경에 둔다(Python처럼 블록 스코프 없음).
pub struct Interpreter<'a> {
    sink: &'a mut dyn ToolSink,
    env: BTreeMap<String, Value>,
}

impl<'a> Interpreter<'a> {
    pub fn new(sink: &'a mut dyn ToolSink) -> Self {
        Self {
            sink,
            env: BTreeMap::new(),
        }
    }

    /// 프로그램을 실행하고, `emit`된 최종값이 있으면 돌려준다.
    pub fn run(&mut self, program: &[Stmt]) -> Result<Option<Value>, RuntimeError> {
        match self.exec_block(program)? {
            Flow::Emit(value) => Ok(Some(value)),
            Flow::Normal => Ok(None),
        }
    }

    fn exec_block(&mut self, stmts: &[Stmt]) -> Result<Flow, RuntimeError> {
        for stmt in stmts {
            if let Flow::Emit(value) = self.exec_stmt(stmt)? {
                return Ok(Flow::Emit(value));
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> Result<Flow, RuntimeError> {
        match stmt {
            Stmt::Assign { name, value, .. } => {
                let value = self.eval_expr(value)?;
                self.env.insert(name.clone(), value);
                Ok(Flow::Normal)
            }
            Stmt::Emit { value, .. } => {
                let value = self.eval_expr(value)?;
                Ok(Flow::Emit(value))
            }
            Stmt::Expr { expr, .. } => {
                self.eval_expr(expr)?;
                Ok(Flow::Normal)
            }
            Stmt::For {
                var, iter, body, ..
            } => self.exec_for(var, iter, body),
            Stmt::If {
                cond, then, els, ..
            } => self.exec_if(cond, then, els),
        }
    }

    fn exec_for(&mut self, var: &str, iter: &Expr, body: &[Stmt]) -> Result<Flow, RuntimeError> {
        let iterable = self.eval_expr(iter)?;
        let items = match iterable {
            Value::List(items) => items,
            other => {
                return Err(RuntimeError::TypeMismatch {
                    expected: "list".into(),
                    found: other.type_name().into(),
                    span: iter.span,
                })
            }
        };
        for item in items {
            self.env.insert(var.to_string(), item);
            if let Flow::Emit(value) = self.exec_block(body)? {
                return Ok(Flow::Emit(value));
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_if(&mut self, cond: &Expr, then: &[Stmt], els: &[Stmt]) -> Result<Flow, RuntimeError> {
        if self.eval_expr(cond)?.is_truthy() {
            self.exec_block(then)
        } else {
            self.exec_block(els)
        }
    }

    fn eval_expr(&mut self, expr: &Expr) -> Result<Value, RuntimeError> {
        match &expr.kind {
            ExprKind::Num(n) => Ok(Value::Num(*n)),
            ExprKind::Str(s) => Ok(Value::Str(s.clone())),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::None => Ok(Value::Null),
            ExprKind::Var(name) => self.eval_var(name, expr),
            ExprKind::List(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.eval_expr(item)?);
                }
                Ok(Value::List(values))
            }
            ExprKind::Binary { op, lhs, rhs } => self.eval_binary(*op, lhs, rhs),
            ExprKind::Member { base, field } => self.eval_member(base, field, expr.span),
            ExprKind::Index { base, idx } => self.eval_index(base, idx, expr.span),
            ExprKind::Call { callee, args } => self.eval_call(callee, args, expr.span),
        }
    }

    fn eval_var(&self, name: &str, expr: &Expr) -> Result<Value, RuntimeError> {
        self.env
            .get(name)
            .cloned()
            .ok_or_else(|| RuntimeError::UndefinedVariable {
                name: name.to_string(),
                span: expr.span,
            })
    }

    /// 이항 연산. `and`/`or`는 단락 평가한다.
    fn eval_binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> Result<Value, RuntimeError> {
        match op {
            BinOp::And => {
                let left = self.eval_expr(lhs)?;
                if !left.is_truthy() {
                    return Ok(Value::Bool(false));
                }
                return Ok(Value::Bool(self.eval_expr(rhs)?.is_truthy()));
            }
            BinOp::Or => {
                let left = self.eval_expr(lhs)?;
                if left.is_truthy() {
                    return Ok(Value::Bool(true));
                }
                return Ok(Value::Bool(self.eval_expr(rhs)?.is_truthy()));
            }
            _ => {}
        }
        let left = self.eval_expr(lhs)?;
        let right = self.eval_expr(rhs)?;
        let span = rhs.span;
        match op {
            BinOp::Eq => Ok(Value::Bool(left == right)),
            BinOp::Ne => Ok(Value::Bool(left != right)),
            BinOp::Add => eval_add(left, right, span),
            BinOp::Sub => Ok(Value::Num(
                expect_num(&left, span)? - expect_num(&right, span)?,
            )),
            BinOp::Mul => Ok(Value::Num(
                expect_num(&left, span)? * expect_num(&right, span)?,
            )),
            BinOp::Div => Ok(Value::Num(
                expect_num(&left, span)? / expect_num(&right, span)?,
            )),
            BinOp::Gt => Ok(Value::Bool(
                expect_num(&left, span)? > expect_num(&right, span)?,
            )),
            BinOp::Lt => Ok(Value::Bool(
                expect_num(&left, span)? < expect_num(&right, span)?,
            )),
            BinOp::Ge => Ok(Value::Bool(
                expect_num(&left, span)? >= expect_num(&right, span)?,
            )),
            BinOp::Le => Ok(Value::Bool(
                expect_num(&left, span)? <= expect_num(&right, span)?,
            )),
            BinOp::And | BinOp::Or => unreachable!("단락 연산은 위에서 처리됨"),
        }
    }

    /// `base.field` — base는 Map이어야 하고 field가 있어야 한다.
    fn eval_member(&mut self, base: &Expr, field: &str, span: Span) -> Result<Value, RuntimeError> {
        match self.eval_expr(base)? {
            Value::Map(map) => map
                .get(field)
                .cloned()
                .ok_or_else(|| RuntimeError::KeyNotFound {
                    key: field.to_string(),
                    span,
                }),
            other => Err(RuntimeError::TypeMismatch {
                expected: "map".into(),
                found: other.type_name().into(),
                span,
            }),
        }
    }

    /// `base[idx]` — 리스트는 정수 인덱스로, 맵은 문자열 키로 접근한다.
    fn eval_index(&mut self, base: &Expr, idx: &Expr, span: Span) -> Result<Value, RuntimeError> {
        let collection = self.eval_expr(base)?;
        let key = self.eval_expr(idx)?;
        match (collection, key) {
            (Value::List(items), Value::Num(n)) => {
                let pos = list_index(n, span)?;
                items
                    .get(pos)
                    .cloned()
                    .ok_or_else(|| RuntimeError::KeyNotFound {
                        key: pos.to_string(),
                        span,
                    })
            }
            (Value::Map(map), Value::Str(key)) => map
                .get(&key)
                .cloned()
                .ok_or(RuntimeError::KeyNotFound { key, span }),
            (collection, _) => Err(RuntimeError::TypeMismatch {
                expected: "list or map".into(),
                found: collection.type_name().into(),
                span,
            }),
        }
    }

    /// 도구 호출. 인자를 먼저 평가하므로(중첩 호출은 안쪽이 먼저) sink 호출 순서가 보장된다.
    fn eval_call(
        &mut self,
        callee: &Expr,
        args: &[Arg],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let tool = callee_name(callee).ok_or_else(|| RuntimeError::TypeMismatch {
            expected: "tool name".into(),
            found: "expression".into(),
            span,
        })?;
        let bound = self.eval_args(args)?;
        self.sink
            .call(&tool, bound)
            .map_err(|err| RuntimeError::ToolFailed {
                tool,
                reason: err.to_string(),
                span,
            })
    }

    /// 인자를 sink가 받는 맵으로 바인딩한다.
    /// 위치 인자는 `arg0`, `arg1`, ... 키로, 키워드 인자는 그 이름으로 넣는다.
    fn eval_args(&mut self, args: &[Arg]) -> Result<BTreeMap<String, Value>, RuntimeError> {
        let mut bound = BTreeMap::new();
        let mut positional = 0;
        for arg in args {
            match arg {
                Arg::Pos(expr) => {
                    let value = self.eval_expr(expr)?;
                    bound.insert(format!("arg{positional}"), value);
                    positional += 1;
                }
                Arg::Kw(name, expr) => {
                    let value = self.eval_expr(expr)?;
                    bound.insert(name.clone(), value);
                }
            }
        }
        Ok(bound)
    }
}

/// `+`는 수 덧셈과 문자열 이어붙이기를 모두 지원한다.
fn eval_add(left: Value, right: Value, span: Span) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Num(a), Value::Num(b)) => Ok(Value::Num(a + b)),
        (Value::Str(a), Value::Str(b)) => Ok(Value::Str(a + &b)),
        (left, _) => Err(RuntimeError::TypeMismatch {
            expected: "num or str".into(),
            found: left.type_name().into(),
            span,
        }),
    }
}

fn expect_num(value: &Value, span: Span) -> Result<f64, RuntimeError> {
    match value {
        Value::Num(n) => Ok(*n),
        other => Err(RuntimeError::TypeMismatch {
            expected: "num".into(),
            found: other.type_name().into(),
            span,
        }),
    }
}

/// 리스트 인덱스는 음이 아닌 정수여야 한다.
fn list_index(n: f64, span: Span) -> Result<usize, RuntimeError> {
    if n.fract() != 0.0 || n < 0.0 {
        return Err(RuntimeError::TypeMismatch {
            expected: "non-negative integer index".into(),
            found: "num".into(),
            span,
        });
    }
    Ok(n as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;
    use crate::parser::parse;

    /// 도구를 호출하지 않는 테스트용 sink. T07에서는 call이 일어나지 않는다.
    struct NoTools;

    impl ToolSink for NoTools {
        fn call(&mut self, tool: &str, _args: BTreeMap<String, Value>) -> Result<Value, ToolError> {
            Err(ToolError::Unknown(tool.to_string()))
        }
    }

    fn run(src: &str) -> Result<Option<Value>, RuntimeError> {
        let program = parse(tokenize(src).expect("lexes")).expect("parses");
        let mut sink = NoTools;
        Interpreter::new(&mut sink).run(&program)
    }

    #[test]
    fn assign_then_emit_returns_value() {
        assert_eq!(run("x = 5\nemit(x)").unwrap(), Some(Value::Num(5.0)));
    }

    #[test]
    fn program_without_emit_returns_none() {
        assert_eq!(run("x = 1").unwrap(), None);
    }

    #[test]
    fn emit_short_circuits_remaining_statements() {
        assert_eq!(run("emit(1)\nemit(2)").unwrap(), Some(Value::Num(1.0)));
    }

    #[test]
    fn if_takes_then_branch_when_truthy() {
        let src = "if True:\n    emit(1)\nelse:\n    emit(2)";
        assert_eq!(run(src).unwrap(), Some(Value::Num(1.0)));
    }

    #[test]
    fn if_takes_else_branch_when_falsy() {
        let src = "if False:\n    emit(1)\nelse:\n    emit(2)";
        assert_eq!(run(src).unwrap(), Some(Value::Num(2.0)));
    }

    #[test]
    fn if_without_else_skips_when_falsy() {
        assert_eq!(run("if False:\n    emit(1)").unwrap(), None);
    }

    #[test]
    fn empty_collection_is_falsy() {
        assert_eq!(run("if []:\n    emit(1)").unwrap(), None);
    }

    #[test]
    fn for_loop_binds_variable_each_iteration() {
        // 루프가 모든 원소를 돌고 변수를 바인딩했음을 마지막 값으로 확인한다.
        let src = "for x in [1, 2, 3]:\n    last = x\nemit(last)";
        assert_eq!(run(src).unwrap(), Some(Value::Num(3.0)));
    }

    #[test]
    fn emit_inside_loop_stops_at_first_iteration() {
        let src = "for x in [10, 20, 30]:\n    emit(x)";
        assert_eq!(run(src).unwrap(), Some(Value::Num(10.0)));
    }

    // ── 런타임 에러 ──

    #[test]
    fn iterating_a_non_list_is_a_type_mismatch() {
        match run("for x in 5:\n    emit(x)") {
            Err(RuntimeError::TypeMismatch { found, .. }) => assert_eq!(found, "num"),
            other => panic!("expected TypeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn referencing_an_undefined_variable_is_an_error() {
        match run("emit(missing)") {
            Err(RuntimeError::UndefinedVariable { name, .. }) => assert_eq!(name, "missing"),
            other => panic!("expected UndefinedVariable, got {other:?}"),
        }
    }

    // ── 계산식 (M1-T08) ──

    #[test]
    fn arithmetic_respects_precedence() {
        assert_eq!(run("emit(1 + 2 * 3)").unwrap(), Some(Value::Num(7.0)));
    }

    #[test]
    fn string_concatenation_uses_plus() {
        assert_eq!(
            run("emit(\"a\" + \"b\")").unwrap(),
            Some(Value::Str("ab".into()))
        );
    }

    #[test]
    fn comparison_yields_bool() {
        assert_eq!(run("emit(3 > 2)").unwrap(), Some(Value::Bool(true)));
        assert_eq!(run("emit(2 >= 3)").unwrap(), Some(Value::Bool(false)));
    }

    #[test]
    fn logical_and_short_circuits_on_false() {
        // 오른쪽이 미정의 변수라도 왼쪽이 거짓이면 평가하지 않는다.
        assert_eq!(
            run("emit(False and missing)").unwrap(),
            Some(Value::Bool(false))
        );
    }

    #[test]
    fn logical_or_short_circuits_on_true() {
        assert_eq!(
            run("emit(True or missing)").unwrap(),
            Some(Value::Bool(true))
        );
    }

    #[test]
    fn adding_incompatible_types_is_a_type_mismatch() {
        assert!(matches!(
            run("emit(1 + \"x\")"),
            Err(RuntimeError::TypeMismatch { .. })
        ));
    }

    // ── 도구 호출과 eval_args (M1-T08) ──

    /// 호출을 순서대로 기록하고, 도구별로 정해진 값을 돌려주는 테스트용 sink.
    #[derive(Default)]
    struct RecordingSink {
        calls: Vec<String>,
        last_args: BTreeMap<String, Value>,
    }

    fn member_map(name: &str, id: f64) -> Value {
        let mut map = BTreeMap::new();
        map.insert("name".to_string(), Value::Str(name.to_string()));
        map.insert("id".to_string(), Value::Num(id));
        Value::Map(map)
    }

    impl ToolSink for RecordingSink {
        fn call(&mut self, tool: &str, args: BTreeMap<String, Value>) -> Result<Value, ToolError> {
            self.calls.push(tool.to_string());
            self.last_args = args;
            match tool {
                "list_team" => Ok(Value::List(vec![
                    member_map("Alice", 1.0),
                    member_map("Bob", 2.0),
                ])),
                "get_member" => Ok(member_map("Alice", 1.0)),
                "format" => Ok(Value::Str("F".into())),
                "probe" | "notify" => Ok(Value::Null),
                "fail" => Err(ToolError::Failed("boom".into())),
                other => Err(ToolError::Unknown(other.to_string())),
            }
        }
    }

    fn run_recording(src: &str, sink: &mut RecordingSink) -> Result<Option<Value>, RuntimeError> {
        let program = parse(tokenize(src).expect("lexes")).expect("parses");
        Interpreter::new(sink).run(&program)
    }

    #[test]
    fn member_access_reads_a_map_field() {
        let mut sink = RecordingSink::default();
        let result = run_recording("m = get_member()\nemit(m.name)", &mut sink);
        assert_eq!(result.unwrap(), Some(Value::Str("Alice".into())));
    }

    #[test]
    fn index_reads_list_element_and_map_key() {
        let mut sink = RecordingSink::default();
        let list = run_recording("t = list_team()\nemit(t[1].name)", &mut sink);
        assert_eq!(list.unwrap(), Some(Value::Str("Bob".into())));
    }

    #[test]
    fn eval_args_covers_all_four_cases() {
        // 리터럴 / 변수 / 필드 접근 / 중첩 호출 네 경우를 한 호출에 모두 담는다.
        let mut sink = RecordingSink::default();
        let src = "x = 7\nm = get_member()\nprobe(1, x, m.name, format(x))";
        run_recording(src, &mut sink).unwrap();

        assert_eq!(sink.last_args["arg0"], Value::Num(1.0)); // 리터럴
        assert_eq!(sink.last_args["arg1"], Value::Num(7.0)); // 변수
        assert_eq!(sink.last_args["arg2"], Value::Str("Alice".into())); // 필드
        assert_eq!(sink.last_args["arg3"], Value::Str("F".into())); // 중첩 호출 결과
    }

    #[test]
    fn keyword_argument_binds_by_name() {
        let mut sink = RecordingSink::default();
        run_recording("probe(42, quarter=\"Q3\")", &mut sink).unwrap();
        assert_eq!(sink.last_args["arg0"], Value::Num(42.0));
        assert_eq!(sink.last_args["quarter"], Value::Str("Q3".into()));
    }

    #[test]
    fn nested_call_invokes_inner_tool_before_outer() {
        let mut sink = RecordingSink::default();
        run_recording("x = 1\nnotify(format(x))", &mut sink).unwrap();
        assert_eq!(sink.calls, vec!["format".to_string(), "notify".to_string()]);
    }

    #[test]
    fn loop_calls_a_tool_once_per_element() {
        let mut sink = RecordingSink::default();
        run_recording("for m in list_team():\n    notify(m.name)", &mut sink).unwrap();
        // list_team 1회 + 멤버 2명에 대한 notify 2회.
        assert_eq!(sink.calls, vec!["list_team", "notify", "notify"]);
    }

    #[test]
    fn tool_failure_propagates_as_runtime_error() {
        let mut sink = RecordingSink::default();
        match run_recording("emit(fail())", &mut sink) {
            Err(RuntimeError::ToolFailed { tool, .. }) => assert_eq!(tool, "fail"),
            other => panic!("expected ToolFailed, got {other:?}"),
        }
    }

    #[test]
    fn member_access_on_non_map_is_a_type_mismatch() {
        let mut sink = RecordingSink::default();
        assert!(matches!(
            run_recording("emit((5).name)", &mut sink),
            Err(RuntimeError::TypeMismatch { .. })
        ));
    }

    #[test]
    fn missing_map_key_is_key_not_found() {
        let mut sink = RecordingSink::default();
        match run_recording("m = get_member()\nemit(m.salary)", &mut sink) {
            Err(RuntimeError::KeyNotFound { key, .. }) => assert_eq!(key, "salary"),
            other => panic!("expected KeyNotFound, got {other:?}"),
        }
    }

    #[test]
    fn list_index_out_of_bounds_is_key_not_found() {
        let mut sink = RecordingSink::default();
        assert!(matches!(
            run_recording("t = list_team()\nemit(t[9])", &mut sink),
            Err(RuntimeError::KeyNotFound { .. })
        ));
    }
}
