//! Lexer — 소스 텍스트를 토큰 열로 바꾼다 (M1-T03).
//!
//! Python 스타일 들여쓰기 스택으로 블록 구조를 `Indent`/`Dedent` 토큰으로
//! 환원한다. 빈 줄과 `#` 줄 주석은 건너뛴다(LLM이 자주 생성하므로 lexer에서
//! 흡수해 불필요한 `PARSE_ERROR`를 막는다).
//!
//! SRP: lexer는 토큰만 만든다. 문법 구조 판단은 parser(T04/T05)의 몫이다.

use crate::error::LexError;
use crate::span::Span;
use crate::token::{Token, TokenKind};

/// 소스를 토큰 열로 변환한다. 마지막은 항상 `Eof`다.
pub fn tokenize(src: &str) -> Result<Vec<Token>, LexError> {
    let mut out = Vec::new();
    let mut indent_stack = vec![0usize];
    let mut last_line = 1u32;

    for (idx, raw_line) in src.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        last_line = line_no;
        let chars: Vec<char> = raw_line.chars().collect();
        let indent = measure_indent(&chars, line_no)?;

        if is_blank_or_comment(&chars, indent) {
            continue;
        }

        emit_indentation(&mut out, &mut indent_stack, indent, line_no)?;
        lex_content(&chars, indent, line_no, &mut out)?;
        out.push(Token::new(
            TokenKind::Newline,
            Span::new(line_no, (chars.len() + 1) as u32),
        ));
    }

    close_open_blocks(&mut out, &mut indent_stack, last_line);
    out.push(Token::new(TokenKind::Eof, Span::new(last_line, 1)));
    Ok(out)
}

/// 선행 공백(스페이스) 개수를 센다. 들여쓰기에 탭이 섞이면 거부한다.
fn measure_indent(chars: &[char], line_no: u32) -> Result<usize, LexError> {
    let mut indent = 0;
    while indent < chars.len() && chars[indent] == ' ' {
        indent += 1;
    }
    if chars.get(indent) == Some(&'\t') {
        return Err(LexError::InconsistentIndent {
            span: Span::new(line_no, (indent + 1) as u32),
        });
    }
    Ok(indent)
}

/// 들여쓰기 이후가 비어 있거나 주석으로 시작하면 그 줄은 무시 대상이다.
fn is_blank_or_comment(chars: &[char], indent: usize) -> bool {
    match chars.get(indent) {
        None => true,
        Some('#') => true,
        Some(_) => false,
    }
}

/// 현재 들여쓰기를 스택과 비교해 `Indent`/`Dedent` 토큰을 적절히 낸다.
fn emit_indentation(
    out: &mut Vec<Token>,
    stack: &mut Vec<usize>,
    indent: usize,
    line_no: u32,
) -> Result<(), LexError> {
    let current = *stack.last().expect("indent stack never empties");
    if indent > current {
        stack.push(indent);
        out.push(Token::new(TokenKind::Indent, Span::new(line_no, 1)));
    } else if indent < current {
        while *stack.last().expect("indent stack never empties") > indent {
            stack.pop();
            out.push(Token::new(TokenKind::Dedent, Span::new(line_no, 1)));
        }
        if *stack.last().expect("indent stack never empties") != indent {
            return Err(LexError::InconsistentIndent {
                span: Span::new(line_no, 1),
            });
        }
    }
    Ok(())
}

/// 파일 끝에서 열린 블록을 모두 닫는 `Dedent`를 낸다.
fn close_open_blocks(out: &mut Vec<Token>, stack: &mut Vec<usize>, last_line: u32) {
    while stack.len() > 1 {
        stack.pop();
        out.push(Token::new(TokenKind::Dedent, Span::new(last_line, 1)));
    }
}

/// 한 줄의 실제 내용(들여쓰기 이후)을 토큰화한다.
fn lex_content(
    chars: &[char],
    start: usize,
    line_no: u32,
    out: &mut Vec<Token>,
) -> Result<(), LexError> {
    let mut i = start;
    while i < chars.len() {
        let c = chars[i];
        if c == ' ' || c == '\t' {
            i += 1;
            continue;
        }
        if c == '#' {
            break;
        }
        let span = Span::new(line_no, (i + 1) as u32);
        let (kind, next) = if c.is_ascii_digit() {
            lex_number(chars, i)
        } else if c == '"' {
            lex_string(chars, i, span)?
        } else if c.is_alphabetic() || c == '_' {
            lex_word(chars, i)
        } else {
            lex_symbol(chars, i, span)?
        };
        out.push(Token::new(kind, span));
        i = next;
    }
    Ok(())
}

/// 정수 또는 소수. 소수점은 뒤에 숫자가 와야만 소수의 일부로 본다
/// (그래야 `team[0].id`의 `.`이 멤버 접근으로 남는다).
fn lex_number(chars: &[char], start: usize) -> (TokenKind, usize) {
    let mut i = start;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
        i += 1;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
    }
    let text: String = chars[start..i].iter().collect();
    let value = text.parse().expect("scanned digits form a valid f64");
    (TokenKind::Num(value), i)
}

/// 큰따옴표 문자열. `\n \t \\ \"` 이스케이프를 지원한다.
fn lex_string(chars: &[char], start: usize, span: Span) -> Result<(TokenKind, usize), LexError> {
    let mut buf = String::new();
    let mut i = start + 1;
    loop {
        let ch = chars
            .get(i)
            .copied()
            .ok_or(LexError::UnterminatedString { span })?;
        match ch {
            '"' => return Ok((TokenKind::Str(buf), i + 1)),
            '\\' => {
                let escaped = chars
                    .get(i + 1)
                    .copied()
                    .ok_or(LexError::UnterminatedString { span })?;
                buf.push(unescape(escaped));
                i += 2;
            }
            _ => {
                buf.push(ch);
                i += 1;
            }
        }
    }
}

fn unescape(ch: char) -> char {
    match ch {
        'n' => '\n',
        't' => '\t',
        other => other,
    }
}

/// 식별자 또는 키워드.
fn lex_word(chars: &[char], start: usize) -> (TokenKind, usize) {
    let mut i = start;
    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
        i += 1;
    }
    let word: String = chars[start..i].iter().collect();
    let kind = match word.as_str() {
        "for" => TokenKind::For,
        "in" => TokenKind::In,
        "if" => TokenKind::If,
        "else" => TokenKind::Else,
        "emit" => TokenKind::Emit,
        "and" => TokenKind::And,
        "or" => TokenKind::Or,
        "True" => TokenKind::True,
        "False" => TokenKind::False,
        "None" => TokenKind::None,
        _ => TokenKind::Ident(word),
    };
    (kind, i)
}

/// 연산자·구두점. 미등록 기호는 거부한다.
fn lex_symbol(chars: &[char], start: usize, span: Span) -> Result<(TokenKind, usize), LexError> {
    let c = chars[start];
    let peek = chars.get(start + 1).copied();
    let (kind, len) = match c {
        '+' => (TokenKind::Plus, 1),
        '-' => (TokenKind::Minus, 1),
        '*' => (TokenKind::Star, 1),
        '/' => (TokenKind::Slash, 1),
        '(' => (TokenKind::LParen, 1),
        ')' => (TokenKind::RParen, 1),
        '[' => (TokenKind::LBracket, 1),
        ']' => (TokenKind::RBracket, 1),
        ',' => (TokenKind::Comma, 1),
        ':' => (TokenKind::Colon, 1),
        '.' => (TokenKind::Dot, 1),
        '=' if peek == Some('=') => (TokenKind::EqEq, 2),
        '=' => (TokenKind::Assign, 1),
        '>' if peek == Some('=') => (TokenKind::Ge, 2),
        '>' => (TokenKind::Gt, 1),
        '<' if peek == Some('=') => (TokenKind::Le, 2),
        '<' => (TokenKind::Lt, 1),
        '!' if peek == Some('=') => (TokenKind::Ne, 2),
        _ => return Err(LexError::UnexpectedChar { ch: c, span }),
    };
    Ok((kind, start + len))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 토큰 종류만 뽑아 비교를 간결하게 한다.
    fn kinds(src: &str) -> Vec<TokenKind> {
        tokenize(src)
            .expect("should tokenize")
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn assignment_yields_expected_tokens() {
        assert_eq!(
            kinds("x = 1"),
            vec![
                TokenKind::Ident("x".into()),
                TokenKind::Assign,
                TokenKind::Num(1.0),
                TokenKind::Newline,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn for_block_brackets_body_with_indent_and_dedent() {
        let src = "for m in team:\n    emit(m)";
        assert_eq!(
            kinds(src),
            vec![
                TokenKind::For,
                TokenKind::Ident("m".into()),
                TokenKind::In,
                TokenKind::Ident("team".into()),
                TokenKind::Colon,
                TokenKind::Newline,
                TokenKind::Indent,
                TokenKind::Emit,
                TokenKind::LParen,
                TokenKind::Ident("m".into()),
                TokenKind::RParen,
                TokenKind::Newline,
                TokenKind::Dedent,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn nested_blocks_balance_indents_and_dedents() {
        let src = "if a:\n    for b in c:\n        emit(b)";
        let result = kinds(src);
        let indents = result.iter().filter(|k| **k == TokenKind::Indent).count();
        let dedents = result.iter().filter(|k| **k == TokenKind::Dedent).count();
        assert_eq!(indents, 2);
        assert_eq!(dedents, 2);
    }

    #[test]
    fn comparison_operators_lex_as_two_char_tokens() {
        assert_eq!(
            kinds("a >= b != c"),
            vec![
                TokenKind::Ident("a".into()),
                TokenKind::Ge,
                TokenKind::Ident("b".into()),
                TokenKind::Ne,
                TokenKind::Ident("c".into()),
                TokenKind::Newline,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn dot_after_integer_is_member_access_not_decimal() {
        assert_eq!(
            kinds("team[0].id"),
            vec![
                TokenKind::Ident("team".into()),
                TokenKind::LBracket,
                TokenKind::Num(0.0),
                TokenKind::RBracket,
                TokenKind::Dot,
                TokenKind::Ident("id".into()),
                TokenKind::Newline,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn decimal_number_is_single_token() {
        assert_eq!(kinds("2.5")[0], TokenKind::Num(2.5));
    }

    #[test]
    fn string_handles_escapes() {
        assert_eq!(kinds(r#""a\nb""#)[0], TokenKind::Str("a\nb".to_string()));
    }

    #[test]
    fn keywords_and_literals_are_recognized() {
        assert_eq!(
            kinds("if True and None"),
            vec![
                TokenKind::If,
                TokenKind::True,
                TokenKind::And,
                TokenKind::None,
                TokenKind::Newline,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn blank_lines_and_comments_are_skipped() {
        let src = "x = 1\n\n# a comment\ny = 2";
        assert_eq!(
            kinds(src),
            vec![
                TokenKind::Ident("x".into()),
                TokenKind::Assign,
                TokenKind::Num(1.0),
                TokenKind::Newline,
                TokenKind::Ident("y".into()),
                TokenKind::Assign,
                TokenKind::Num(2.0),
                TokenKind::Newline,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn empty_source_yields_only_eof() {
        assert_eq!(kinds(""), vec![TokenKind::Eof]);
    }

    // ── 음성 테스트: 거부되어야 할 입력 ──

    #[test]
    fn dedent_to_unmatched_level_is_rejected() {
        // 둘째 줄은 4칸, 셋째 줄은 2칸 — 어떤 열린 레벨과도 맞지 않는다.
        let src = "if a:\n    emit(1)\n  emit(2)";
        let err = tokenize(src).expect_err("should reject");
        assert!(matches!(err, LexError::InconsistentIndent { .. }));
    }

    #[test]
    fn tab_indentation_is_rejected() {
        let err = tokenize("if a:\n\temit(1)").expect_err("should reject");
        assert!(matches!(err, LexError::InconsistentIndent { .. }));
    }

    #[test]
    fn unterminated_string_is_rejected() {
        let err = tokenize("x = \"oops").expect_err("should reject");
        assert!(matches!(err, LexError::UnterminatedString { .. }));
    }

    #[test]
    fn unexpected_char_is_rejected_with_location() {
        let err = tokenize("x = @").expect_err("should reject");
        match err {
            LexError::UnexpectedChar { ch, span } => {
                assert_eq!(ch, '@');
                assert_eq!(span, Span::new(1, 5));
            }
            other => panic!("expected UnexpectedChar, got {other:?}"),
        }
    }
}
