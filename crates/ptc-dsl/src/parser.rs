//! Parser — 토큰 열을 AST로 바꾼다.
//!
//! 재귀 하강 + 표현식의 이항 연산자는 precedence-climbing으로 처리한다.
//! 후위 연산자(멤버 `a.b`, 인덱스 `a[b]`, 호출 `f(x)`)는 좌결합 체인이므로
//! 별도 루프로 감는다.
//!
//! 이 티켓(M1-T04)은 **표현식**만 다룬다. 문장·블록 파싱(M1-T05)이
//! 같은 `Parser` 골격 위에 얹힌다.

use crate::ast::{Arg, BinOp, Expr, ExprKind, Stmt};
use crate::error::ParseError;
use crate::span::Span;
use crate::token::{Token, TokenKind};

/// 표현식 재귀 중첩의 상한. 깊은 식(`((((…))))`, `a[a[a[…]]]`)이 파서·인터프리터
/// 재귀를 폭주시켜 스택 오버플로로 프로세스를 죽이는 것을 파싱 단계에서 막는다.
/// 정상 코드의 식 깊이는 한 자릿수라 충분히 여유 있고, 스택 한계보다는 한참 낮다.
pub(crate) const MAX_EXPR_DEPTH: usize = 128;

/// 토큰 열 위를 커서로 훑는 파서.
pub(crate) struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// 현재 표현식 재귀 깊이(`parse_binary` 진입마다 증가, 반환 시 감소).
    depth: usize,
}

impl Parser {
    pub(crate) fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            pos: 0,
            depth: 0,
        }
    }

    // ── 표현식 ──

    /// 표현식 하나를 파싱한다.
    pub(crate) fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary(0)
    }

    /// precedence-climbing: `min_prec` 이상으로 결합하는 이항 연산만 흡수한다.
    /// 표현식 재귀의 단일 관문이므로 여기서 깊이를 세어 [`MAX_EXPR_DEPTH`]를 강제한다
    /// (괄호·인덱스·인자 안의 식은 모두 `parse_expr`→여기로 들어온다).
    fn parse_binary(&mut self, min_prec: u8) -> Result<Expr, ParseError> {
        self.depth += 1;
        if self.depth > MAX_EXPR_DEPTH {
            self.depth -= 1;
            return Err(ParseError::ExpressionTooDeep {
                depth: MAX_EXPR_DEPTH + 1,
                max: MAX_EXPR_DEPTH,
                span: self.peek_span(),
            });
        }
        let result = self.parse_binary_body(min_prec);
        self.depth -= 1;
        result
    }

    fn parse_binary_body(&mut self, min_prec: u8) -> Result<Expr, ParseError> {
        let mut left = self.parse_postfix()?;
        while let Some(op) = as_binop(self.peek_kind()) {
            let prec = precedence(op);
            if prec < min_prec {
                break;
            }
            // 비교 연산자는 비결합: `a < b < c`처럼 비교 결과를 다시 비교하면 거부한다.
            // (Python의 연쇄 비교를 흉내 내지 않으므로, 혼란스러운 런타임 TypeMismatch
            //  대신 파싱 시점에 명확히 실패시킨다.)
            if is_comparison(op) {
                if let ExprKind::Binary { op: left_op, .. } = &left.kind {
                    if is_comparison(*left_op) {
                        return Err(ParseError::ChainedComparison {
                            span: self.peek_span(),
                        });
                    }
                }
            }
            self.advance();
            let right = self.parse_binary(prec + 1)?; // 좌결합: 같은 레벨은 왼쪽이 먼저
            let span = left.span;
            left = Expr::new(
                ExprKind::Binary {
                    op,
                    lhs: Box::new(left),
                    rhs: Box::new(right),
                },
                span,
            );
        }
        Ok(left)
    }

    /// 기본 표현식에 후위 연산자(`.` `[]` `()`)를 좌결합으로 이어 붙인다.
    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;
        loop {
            expr = match self.peek_kind() {
                TokenKind::Dot => self.parse_member(expr)?,
                TokenKind::LParen => self.parse_call(expr)?,
                TokenKind::LBracket => self.parse_index(expr)?,
                _ => break,
            };
        }
        Ok(expr)
    }

    fn parse_member(&mut self, base: Expr) -> Result<Expr, ParseError> {
        let span = base.span;
        self.advance(); // '.'
        let (field, _) = self.expect_ident()?;
        Ok(Expr::new(
            ExprKind::Member {
                base: Box::new(base),
                field,
            },
            span,
        ))
    }

    fn parse_call(&mut self, callee: Expr) -> Result<Expr, ParseError> {
        let span = callee.span;
        self.advance(); // '('
        let args = self.parse_args()?;
        self.expect(TokenKind::RParen)?;
        Ok(Expr::new(
            ExprKind::Call {
                callee: Box::new(callee),
                args,
            },
            span,
        ))
    }

    fn parse_index(&mut self, base: Expr) -> Result<Expr, ParseError> {
        let span = base.span;
        self.advance(); // '['
        let idx = self.parse_expr()?;
        self.expect(TokenKind::RBracket)?;
        Ok(Expr::new(
            ExprKind::Index {
                base: Box::new(base),
                idx: Box::new(idx),
            },
            span,
        ))
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let token = self.peek().clone();
        let span = token.span;
        let kind = match token.kind {
            TokenKind::Num(n) => ExprKind::Num(n),
            TokenKind::Str(s) => ExprKind::Str(s),
            TokenKind::True => ExprKind::Bool(true),
            TokenKind::False => ExprKind::Bool(false),
            TokenKind::None => ExprKind::None,
            TokenKind::Ident(name) => ExprKind::Var(name),
            TokenKind::LBracket => return self.parse_list(),
            TokenKind::LParen => return self.parse_group(),
            _ => return Err(self.unexpected("expression")),
        };
        self.advance();
        Ok(Expr::new(kind, span))
    }

    /// 괄호 묶음: 그룹은 안쪽 표현식을 그대로 돌려준다(노드를 만들지 않음).
    fn parse_group(&mut self) -> Result<Expr, ParseError> {
        self.expect(TokenKind::LParen)?;
        let inner = self.parse_expr()?;
        self.expect(TokenKind::RParen)?;
        Ok(inner)
    }

    fn parse_list(&mut self) -> Result<Expr, ParseError> {
        let span = self.peek_span();
        self.expect(TokenKind::LBracket)?;
        let items = self.parse_comma_separated(TokenKind::RBracket, Self::parse_expr)?;
        self.expect(TokenKind::RBracket)?;
        Ok(Expr::new(ExprKind::List(items), span))
    }

    /// 호출 인자: 위치 인자 또는 `name=value` 키워드 인자.
    fn parse_args(&mut self) -> Result<Vec<Arg>, ParseError> {
        self.parse_comma_separated(TokenKind::RParen, Self::parse_arg)
    }

    fn parse_arg(&mut self) -> Result<Arg, ParseError> {
        if self.is_kwarg_ahead() {
            let (name, _) = self.expect_ident()?;
            self.expect(TokenKind::Assign)?;
            let value = self.parse_expr()?;
            Ok(Arg::Kw(name, value))
        } else {
            Ok(Arg::Pos(self.parse_expr()?))
        }
    }

    /// `Ident "="` 형태가 앞에 오면 키워드 인자다.
    fn is_kwarg_ahead(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Ident(_))
            && matches!(self.peek_kind_at(1), TokenKind::Assign)
    }

    /// `end` 토큰 전까지 콤마로 구분된 항목을 파싱한다(후행 콤마 허용).
    fn parse_comma_separated<T>(
        &mut self,
        end: TokenKind,
        parse_item: fn(&mut Self) -> Result<T, ParseError>,
    ) -> Result<Vec<T>, ParseError> {
        let mut items = Vec::new();
        if self.check(&end) {
            return Ok(items);
        }
        loop {
            items.push(parse_item(self)?);
            if !self.check(&TokenKind::Comma) {
                break;
            }
            self.advance(); // ','
            if self.check(&end) {
                break; // 후행 콤마
            }
        }
        Ok(items)
    }

    // ── 커서 헬퍼 ──

    pub(crate) fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    pub(crate) fn peek_kind(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn peek_kind_at(&self, offset: usize) -> &TokenKind {
        let idx = (self.pos + offset).min(self.tokens.len() - 1);
        &self.tokens[idx].kind
    }

    pub(crate) fn peek_span(&self) -> Span {
        self.tokens[self.pos].span
    }

    /// 현재 토큰을 반환하고 커서를 전진시킨다(`Eof`에서는 멈춘다).
    pub(crate) fn advance(&mut self) -> Token {
        let token = self.tokens[self.pos].clone();
        if !matches!(token.kind, TokenKind::Eof) {
            self.pos += 1;
        }
        token
    }

    pub(crate) fn check(&self, kind: &TokenKind) -> bool {
        self.peek_kind() == kind
    }

    /// 기대한 토큰이면 소비하고, 아니면 위치를 담은 에러를 낸다.
    pub(crate) fn expect(&mut self, want: TokenKind) -> Result<Token, ParseError> {
        if self.check(&want) {
            Ok(self.advance())
        } else {
            Err(self.unexpected(&want.to_string()))
        }
    }

    fn expect_ident(&mut self) -> Result<(String, Span), ParseError> {
        let token = self.peek().clone();
        if let TokenKind::Ident(name) = token.kind {
            self.advance();
            Ok((name, token.span))
        } else {
            Err(self.unexpected("identifier"))
        }
    }

    pub(crate) fn unexpected(&self, expected: &str) -> ParseError {
        let token = self.peek();
        if matches!(token.kind, TokenKind::Eof) {
            ParseError::UnexpectedEof { span: token.span }
        } else {
            ParseError::UnexpectedToken {
                expected: expected.to_string(),
                found: token.kind.to_string(),
                span: token.span,
            }
        }
    }
}

impl Parser {
    // ── 문장과 블록 ──

    /// 프로그램 전체(문장의 나열)를 파싱한다.
    pub(crate) fn parse_program(&mut self) -> Result<Vec<Stmt>, ParseError> {
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.at_eof() {
            stmts.push(self.parse_statement()?);
            self.skip_newlines();
        }
        Ok(stmts)
    }

    fn parse_statement(&mut self) -> Result<Stmt, ParseError> {
        match self.peek_kind() {
            TokenKind::For => self.parse_for(),
            TokenKind::If => self.parse_if(),
            TokenKind::Emit => self.parse_emit(),
            TokenKind::Ident(_) if self.is_assign_ahead() => self.parse_assign(),
            _ => self.parse_expr_stmt(),
        }
    }

    fn parse_assign(&mut self) -> Result<Stmt, ParseError> {
        let (name, span) = self.expect_ident()?;
        self.expect(TokenKind::Assign)?;
        let value = self.parse_expr()?;
        self.end_statement()?;
        Ok(Stmt::Assign { name, value, span })
    }

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek_span();
        self.expect(TokenKind::For)?;
        let (var, _) = self.expect_ident()?;
        self.expect(TokenKind::In)?;
        let iter = self.parse_expr()?;
        self.expect(TokenKind::Colon)?;
        let body = self.parse_block()?;
        Ok(Stmt::For {
            var,
            iter,
            body,
            span,
        })
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek_span();
        self.expect(TokenKind::If)?;
        let cond = self.parse_expr()?;
        self.expect(TokenKind::Colon)?;
        let then = self.parse_block()?;
        let els = if self.check(&TokenKind::Else) {
            self.advance(); // 'else'
            self.expect(TokenKind::Colon)?;
            self.parse_block()?
        } else {
            Vec::new()
        };
        Ok(Stmt::If {
            cond,
            then,
            els,
            span,
        })
    }

    fn parse_emit(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek_span();
        self.expect(TokenKind::Emit)?;
        self.expect(TokenKind::LParen)?;
        let value = self.parse_expr()?;
        self.expect(TokenKind::RParen)?;
        self.end_statement()?;
        Ok(Stmt::Emit { value, span })
    }

    fn parse_expr_stmt(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek_span();
        let expr = self.parse_expr()?;
        self.end_statement()?;
        Ok(Stmt::Expr { expr, span })
    }

    /// `block = INDENT statement+ DEDENT`. 콜론 줄을 끝내는 줄바꿈을 먼저 소비한다.
    fn parse_block(&mut self) -> Result<Vec<Stmt>, ParseError> {
        self.expect(TokenKind::Newline)?;
        self.expect(TokenKind::Indent)?;
        let mut stmts = Vec::new();
        while !matches!(self.peek_kind(), TokenKind::Dedent | TokenKind::Eof) {
            stmts.push(self.parse_statement()?);
        }
        if stmts.is_empty() {
            return Err(self.unexpected("statement"));
        }
        self.expect(TokenKind::Dedent)?;
        Ok(stmts)
    }

    /// 단순 문장은 줄바꿈으로 끝난다.
    fn end_statement(&mut self) -> Result<(), ParseError> {
        match self.peek_kind() {
            TokenKind::Newline => {
                self.advance();
                Ok(())
            }
            TokenKind::Eof => Ok(()),
            _ => Err(self.unexpected("newline")),
        }
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Eof)
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek_kind(), TokenKind::Newline) {
            self.advance();
        }
    }

    /// `IDENT "="` 형태가 앞에 오면 대입문이다(그 외 식별자 시작은 표현식 문장).
    fn is_assign_ahead(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Ident(_))
            && matches!(self.peek_kind_at(1), TokenKind::Assign)
    }
}

/// 토큰 열을 프로그램(문장 벡터)으로 파싱한다.
pub fn parse(tokens: Vec<Token>) -> Result<Vec<Stmt>, ParseError> {
    Parser::new(tokens).parse_program()
}

/// 토큰을 이항 연산자로 해석한다(연산자가 아니면 `None`).
fn as_binop(kind: &TokenKind) -> Option<BinOp> {
    Some(match kind {
        TokenKind::Plus => BinOp::Add,
        TokenKind::Minus => BinOp::Sub,
        TokenKind::Star => BinOp::Mul,
        TokenKind::Slash => BinOp::Div,
        TokenKind::Gt => BinOp::Gt,
        TokenKind::Lt => BinOp::Lt,
        TokenKind::Ge => BinOp::Ge,
        TokenKind::Le => BinOp::Le,
        TokenKind::EqEq => BinOp::Eq,
        TokenKind::Ne => BinOp::Ne,
        TokenKind::And => BinOp::And,
        TokenKind::Or => BinOp::Or,
        _ => return None,
    })
}

/// 비교 연산자인가(비결합 처리 대상).
fn is_comparison(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt | BinOp::Ge | BinOp::Le
    )
}

/// 결합력(클수록 강하게 묶는다).
fn precedence(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt | BinOp::Ge | BinOp::Le => 3,
        BinOp::Add | BinOp::Sub => 4,
        BinOp::Mul | BinOp::Div => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;

    fn parse(src: &str) -> Expr {
        let tokens = tokenize(src).expect("lexes");
        Parser::new(tokens).parse_expr().expect("parses")
    }

    fn parse_err(src: &str) -> ParseError {
        let tokens = tokenize(src).expect("lexes");
        Parser::new(tokens)
            .parse_expr()
            .expect_err("should fail to parse")
    }

    #[test]
    fn multiplication_binds_tighter_than_addition() {
        // 1 + 2 * 3  ==  1 + (2 * 3)
        match parse("1 + 2 * 3").kind {
            ExprKind::Binary {
                op: BinOp::Add,
                lhs,
                rhs,
            } => {
                assert_eq!(lhs.kind, ExprKind::Num(1.0));
                assert!(matches!(rhs.kind, ExprKind::Binary { op: BinOp::Mul, .. }));
            }
            other => panic!("expected Add at root, got {other:?}"),
        }
    }

    #[test]
    fn subtraction_is_left_associative() {
        // 1 - 2 - 3  ==  (1 - 2) - 3
        match parse("1 - 2 - 3").kind {
            ExprKind::Binary {
                op: BinOp::Sub,
                lhs,
                rhs,
            } => {
                assert!(matches!(lhs.kind, ExprKind::Binary { op: BinOp::Sub, .. }));
                assert_eq!(rhs.kind, ExprKind::Num(3.0));
            }
            other => panic!("expected Sub at root, got {other:?}"),
        }
    }

    #[test]
    fn comparison_binds_tighter_than_logical_and() {
        // a < b and c  ==  (a < b) and c
        match parse("a < b and c").kind {
            ExprKind::Binary {
                op: BinOp::And,
                lhs,
                ..
            } => assert!(matches!(lhs.kind, ExprKind::Binary { op: BinOp::Lt, .. })),
            other => panic!("expected And at root, got {other:?}"),
        }
    }

    #[test]
    fn chained_comparison_is_rejected() {
        assert!(matches!(
            parse_err("1 < 2 < 3"),
            ParseError::ChainedComparison { .. }
        ));
        assert!(matches!(
            parse_err("a == b == c"),
            ParseError::ChainedComparison { .. }
        ));
    }

    #[test]
    fn separate_comparisons_joined_by_logical_op_parse() {
        // a < b and c < d 는 연쇄가 아니라 두 비교를 and로 묶은 것 → 정상.
        match parse("a < b and c < d").kind {
            ExprKind::Binary {
                op: BinOp::And,
                lhs,
                rhs,
            } => {
                assert!(matches!(lhs.kind, ExprKind::Binary { op: BinOp::Lt, .. }));
                assert!(matches!(rhs.kind, ExprKind::Binary { op: BinOp::Lt, .. }));
            }
            other => panic!("expected And at root, got {other:?}"),
        }
    }

    #[test]
    fn deeply_nested_expression_is_rejected_without_panic() {
        // 깊은 중첩 괄호가 스택 오버플로 대신 ExpressionTooDeep로 거부되는지.
        let deep = format!("{}1{}", "(".repeat(500), ")".repeat(500));
        assert!(matches!(
            parse_err(&deep),
            ParseError::ExpressionTooDeep { .. }
        ));
        // 깊은 인덱스 체인도 마찬가지.
        let idx = format!("a{}{}", "[a".repeat(300), "]".repeat(300));
        assert!(matches!(
            parse_err(&idx),
            ParseError::ExpressionTooDeep { .. }
        ));
    }

    #[test]
    fn moderately_nested_expression_still_parses() {
        // 정상적인 깊이(한 자릿수~수십)의 식은 영향 없이 파싱된다.
        let ok = format!("{}1{}", "(".repeat(20), ")".repeat(20));
        let tokens = tokenize(&ok).expect("lexes");
        assert!(Parser::new(tokens).parse_expr().is_ok());
    }

    #[test]
    fn grouping_overrides_precedence() {
        // (1 + 2) * 3  ==  Mul(Add(..), 3)
        match parse("(1 + 2) * 3").kind {
            ExprKind::Binary {
                op: BinOp::Mul,
                lhs,
                ..
            } => assert!(matches!(lhs.kind, ExprKind::Binary { op: BinOp::Add, .. })),
            other => panic!("expected Mul at root, got {other:?}"),
        }
    }

    #[test]
    fn postfix_chain_indexes_then_accesses_member() {
        // team[0].id  ==  Member(Index(Var, 0), "id")
        match parse("team[0].id").kind {
            ExprKind::Member { base, field } => {
                assert_eq!(field, "id");
                assert!(matches!(base.kind, ExprKind::Index { .. }));
            }
            other => panic!("expected Member at root, got {other:?}"),
        }
    }

    #[test]
    fn nested_call_parses_inner_call_as_argument() {
        // notify(format(x))
        match parse("notify(format(x))").kind {
            ExprKind::Call { callee, args } => {
                assert!(matches!(callee.kind, ExprKind::Var(ref n) if n == "notify"));
                assert_eq!(args.len(), 1);
                match &args[0] {
                    Arg::Pos(inner) => {
                        assert!(matches!(inner.kind, ExprKind::Call { .. }))
                    }
                    other => panic!("expected positional inner call, got {other:?}"),
                }
            }
            other => panic!("expected Call at root, got {other:?}"),
        }
    }

    #[test]
    fn call_mixes_positional_and_keyword_arguments() {
        // get_expenses(m.id, quarter="Q3")
        match parse(r#"get_expenses(m.id, quarter="Q3")"#).kind {
            ExprKind::Call { args, .. } => {
                assert_eq!(args.len(), 2);
                assert!(matches!(args[0], Arg::Pos(_)));
                match &args[1] {
                    Arg::Kw(name, value) => {
                        assert_eq!(name, "quarter");
                        assert_eq!(value.kind, ExprKind::Str("Q3".into()));
                    }
                    other => panic!("expected kwarg, got {other:?}"),
                }
            }
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn empty_call_has_no_arguments() {
        match parse("f()").kind {
            ExprKind::Call { args, .. } => assert!(args.is_empty()),
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn list_literal_collects_elements() {
        match parse("[1, 2, 3]").kind {
            ExprKind::List(items) => assert_eq!(items.len(), 3),
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn literals_parse_to_their_kinds() {
        assert_eq!(parse("True").kind, ExprKind::Bool(true));
        assert_eq!(parse("False").kind, ExprKind::Bool(false));
        assert_eq!(parse("None").kind, ExprKind::None);
        assert_eq!(parse(r#""hi""#).kind, ExprKind::Str("hi".into()));
    }

    // ── 음성 테스트 ──

    #[test]
    fn missing_closing_paren_is_rejected() {
        assert!(matches!(
            parse_err("(1 + 2"),
            ParseError::UnexpectedEof { .. } | ParseError::UnexpectedToken { .. }
        ));
    }

    #[test]
    fn dangling_operator_is_rejected() {
        assert!(matches!(
            parse_err("1 +"),
            ParseError::UnexpectedEof { .. } | ParseError::UnexpectedToken { .. }
        ));
    }

    #[test]
    fn leading_operator_is_rejected_with_location() {
        match parse_err("* 3") {
            ParseError::UnexpectedToken { found, span, .. } => {
                assert_eq!(found, "*");
                assert_eq!(span, Span::new(1, 1));
            }
            other => panic!("expected UnexpectedToken, got {other:?}"),
        }
    }

    // ── 문장·블록 (M1-T05) ──

    fn program(src: &str) -> Vec<Stmt> {
        super::parse(tokenize(src).expect("lexes")).expect("parses")
    }

    fn program_err(src: &str) -> ParseError {
        super::parse(tokenize(src).expect("lexes")).expect_err("should fail")
    }

    #[test]
    fn assignment_statement_binds_name() {
        match &program("x = 1 + 2")[..] {
            [Stmt::Assign { name, .. }] => assert_eq!(name, "x"),
            other => panic!("expected one Assign, got {other:?}"),
        }
    }

    #[test]
    fn bare_call_is_an_expression_statement() {
        match &program("notify(x)")[..] {
            [Stmt::Expr { expr, .. }] => {
                assert!(matches!(expr.kind, ExprKind::Call { .. }))
            }
            other => panic!("expected one Expr stmt, got {other:?}"),
        }
    }

    #[test]
    fn emit_statement_wraps_value() {
        match &program("emit(total)")[..] {
            [Stmt::Emit { value, .. }] => {
                assert!(matches!(value.kind, ExprKind::Var(ref n) if n == "total"))
            }
            other => panic!("expected one Emit, got {other:?}"),
        }
    }

    #[test]
    fn for_loop_collects_body_statements() {
        let src = "for m in team:\n    emit(m)";
        match &program(src)[..] {
            [Stmt::For { var, body, .. }] => {
                assert_eq!(var, "m");
                assert_eq!(body.len(), 1);
                assert!(matches!(body[0], Stmt::Emit { .. }));
            }
            other => panic!("expected one For, got {other:?}"),
        }
    }

    #[test]
    fn if_else_fills_both_branches() {
        let src = "if x:\n    emit(1)\nelse:\n    emit(2)";
        match &program(src)[..] {
            [Stmt::If { then, els, .. }] => {
                assert_eq!(then.len(), 1);
                assert_eq!(els.len(), 1);
            }
            other => panic!("expected one If, got {other:?}"),
        }
    }

    #[test]
    fn if_without_else_has_empty_else_branch() {
        let src = "if x:\n    emit(1)";
        match &program(src)[..] {
            [Stmt::If { els, .. }] => assert!(els.is_empty()),
            other => panic!("expected one If, got {other:?}"),
        }
    }

    #[test]
    fn nested_blocks_and_dedents_are_balanced() {
        let src = "if a:\n    for m in t:\n        emit(m)\n    emit(done)";
        match &program(src)[..] {
            [Stmt::If { then, .. }] => {
                assert_eq!(then.len(), 2);
                assert!(matches!(then[0], Stmt::For { .. }));
                assert!(matches!(then[1], Stmt::Emit { .. }));
            }
            other => panic!("expected one If, got {other:?}"),
        }
    }

    #[test]
    fn multiple_top_level_statements_parse_in_order() {
        let src = "team = list_team(\"eng\")\nfor m in team:\n    emit(m)";
        let stmts = program(src);
        assert_eq!(stmts.len(), 2);
        assert!(matches!(stmts[0], Stmt::Assign { .. }));
        assert!(matches!(stmts[1], Stmt::For { .. }));
    }

    // ── 음성 테스트 ──

    #[test]
    fn for_without_colon_is_rejected() {
        assert!(matches!(
            program_err("for m in team\n    emit(m)"),
            ParseError::UnexpectedToken { .. } | ParseError::UnexpectedEof { .. }
        ));
    }

    #[test]
    fn block_without_indent_is_rejected() {
        assert!(matches!(
            program_err("if x:\nemit(1)"),
            ParseError::UnexpectedToken { .. } | ParseError::UnexpectedEof { .. }
        ));
    }

    #[test]
    fn emit_without_parentheses_is_rejected() {
        assert!(matches!(
            program_err("emit x"),
            ParseError::UnexpectedToken { .. } | ParseError::UnexpectedEof { .. }
        ));
    }
}
