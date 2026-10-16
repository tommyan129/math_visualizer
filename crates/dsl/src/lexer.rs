//! Lexer (design.md "어휘 규칙").
//!
//! - Indentation: spaces only, 4 per level; emits `Indent` / `Dedent` like Python.
//! - Newlines inside `()`, `[]`, `{}` are ignored. Statements end at a newline.
//! - Strings are cut as one raw token; `{}` and `$` inside are not interpreted here.
//! - Errors are collected and lexing continues. Characters that cannot start a token
//!   become `Invalid` so the parser can treat them as an operand without cascading.

use crate::diag::{Diagnostic, code};
use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Define,
    Constant,
    Scene,
    Step,
    Split,
    At,
    And,
    Or,
    Not,
    Del,
}

impl Keyword {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "define" => Self::Define,
            "constant" => Self::Constant,
            "scene" => Self::Scene,
            "step" => Self::Step,
            "split" => Self::Split,
            "at" => Self::At,
            "and" => Self::And,
            "or" => Self::Or,
            "not" => Self::Not,
            "del" => Self::Del,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    /// ASCII letters / Hangul, then letters or digits.
    Ident,
    /// `\name`; the span includes the backslash.
    TexName,
    /// Integer literal (digits only).
    Int,
    /// Decimal literal (`1.5`, `1e-3`).
    Decimal,
    /// `"..."`, span includes the quotes.
    Str,
    Kw(Keyword),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Bang,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Semi,
    Colon,
    Arrow,
    Lt,
    Le,
    Gt,
    Ge,
    EqEq,
    Eq,
    At,
    Tilde,
    Dot,
    Underscore,
    /// Already reported; parse as an operand.
    Invalid,
    Newline,
    Indent,
    Dedent,
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

pub const INDENT_WIDTH: usize = 4;

pub fn lex(src: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut lx = Lexer { src, bytes: src.as_bytes(), pos: 0, depth: 0, open_indent: 0, indents: vec![0], toks: vec![], diags: vec![] };
    lx.run();
    (lx.toks, lx.diags)
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    /// Bracket nesting; newlines and indentation are ignored while > 0.
    depth: u32,
    /// Indentation of the line that opened the outermost bracket.
    open_indent: usize,
    indents: Vec<usize>,
    toks: Vec<Token>,
    diags: Vec<Diagnostic>,
}

pub fn is_hangul(c: char) -> bool {
    matches!(c, '\u{AC00}'..='\u{D7A3}' | '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}')
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || is_hangul(c)
}

fn is_name_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || is_hangul(c)
}

impl<'a> Lexer<'a> {
    fn peek_char(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn byte(&self, at: usize) -> Option<u8> {
        self.bytes.get(at).copied()
    }

    fn push(&mut self, tok: Tok, start: usize) {
        self.toks.push(Token { tok, span: Span::new(start, self.pos) });
    }

    fn err(&mut self, d: Diagnostic) {
        self.diags.push(d);
    }

    fn run(&mut self) {
        let mut at_line_start = true;
        loop {
            if at_line_start && self.depth == 0 {
                at_line_start = false;
                if !self.indentation() {
                    break;
                }
            }
            let Some(c) = self.peek_char() else { break };
            let start = self.pos;
            match c {
                ' ' | '\r' => self.pos += 1,
                '\t' => {
                    self.pos += 1;
                    self.err(Diagnostic::new(Span::new(start, self.pos), code::TAB, "탭은 쓸 수 없음").suggest("스페이스"));
                }
                '#' => self.skip_comment(),
                '\n' => {
                    self.pos += 1;
                    // An unclosed bracket must not swallow the rest of the input: a
                    // continuation line has to be indented deeper than the opening line.
                    if self.depth > 0 && !self.next_line_continues() {
                        self.depth = 0;
                    }
                    if self.depth == 0 {
                        self.push_newline(start);
                        at_line_start = true;
                    }
                }
                '"' => self.string(),
                '\\' => self.tex_name(),
                '0'..='9' => self.number(),
                '.' if self.byte(self.pos + 1).is_some_and(|b| b.is_ascii_digit()) => self.number(),
                c if is_name_start(c) => self.ident(),
                c if c.is_ascii() => self.punct(c),
                c => {
                    self.pos += c.len_utf8();
                    let d = Diagnostic::new(Span::new(start, self.pos), code::UNICODE, format!("'{c}'는 쓸 수 없음"));
                    self.err(match unicode_suggestion(c) {
                        Some(s) => d.suggest(s),
                        None => d,
                    });
                    self.push(Tok::Invalid, start);
                }
            }
        }
        let end = self.pos;
        self.push_newline(end);
        while self.indents.len() > 1 {
            self.indents.pop();
            self.toks.push(Token { tok: Tok::Dedent, span: Span::new(end, end) });
        }
        self.toks.push(Token { tok: Tok::Eof, span: Span::new(end, end) });
    }

    /// Inside brackets: is the next non-blank line a continuation (deeper than the
    /// opening line, or starting with a closing bracket)?
    fn next_line_continues(&self) -> bool {
        for line in self.src[self.pos..].split('\n') {
            let content = line.trim_start_matches([' ', '\t', '\r']);
            if content.is_empty() || content.starts_with('#') {
                continue;
            }
            let width = line.len() - content.len();
            return width > self.open_indent || content.starts_with([')', ']', '}']);
        }
        true
    }

    /// One `Newline` per logical line; none for empty lines.
    fn push_newline(&mut self, at: usize) {
        if matches!(self.toks.last().map(|t| t.tok), Some(t) if t != Tok::Newline && t != Tok::Indent && t != Tok::Dedent) {
            self.toks.push(Token { tok: Tok::Newline, span: Span::new(at, at) });
        }
    }

    /// Measures leading spaces of a line, skipping blank / comment-only lines.
    /// Returns false at end of input.
    fn indentation(&mut self) -> bool {
        loop {
            let line_start = self.pos;
            let mut width = 0;
            while let Some(b) = self.byte(self.pos) {
                match b {
                    b' ' => width += 1,
                    b'\t' => {
                        self.err(
                            Diagnostic::new(Span::new(self.pos, self.pos + 1), code::TAB, "들여쓰기에 탭은 쓸 수 없음")
                                .suggest("스페이스 4칸"),
                        );
                        width += INDENT_WIDTH;
                    }
                    b'\r' => {}
                    _ => break,
                }
                self.pos += 1;
            }
            match self.byte(self.pos) {
                None => return false,
                Some(b'\n') => {
                    self.pos += 1;
                    continue;
                }
                Some(b'#') => {
                    self.skip_comment();
                    continue;
                }
                _ => {}
            }
            let span = Span::new(line_start, self.pos);
            let cur = *self.indents.last().unwrap();
            if width > cur {
                if width - cur != INDENT_WIDTH {
                    self.err(
                        Diagnostic::new(span, code::INDENT, format!("{}칸 들어감, {INDENT_WIDTH}칸 필요", width - cur))
                            .suggest(format!("{}칸 들여쓰기", cur + INDENT_WIDTH)),
                    );
                }
                self.indents.push(width);
                self.toks.push(Token { tok: Tok::Indent, span });
            } else if width < cur {
                while *self.indents.last().unwrap() > width {
                    self.indents.pop();
                    self.toks.push(Token { tok: Tok::Dedent, span });
                }
                if *self.indents.last().unwrap() != width {
                    let expect = *self.indents.last().unwrap();
                    self.err(
                        Diagnostic::new(span, code::DEDENT, format!("{width}칸 들여쓰기는 바깥 블록과 맞지 않음"))
                            .suggest(format!("{expect}칸 들여쓰기")),
                    );
                    // Treat as a new level so the rest of the block still parses.
                    self.indents.push(width);
                    self.toks.push(Token { tok: Tok::Indent, span });
                }
            }
            return true;
        }
    }

    fn skip_comment(&mut self) {
        while let Some(b) = self.byte(self.pos) {
            if b == b'\n' {
                break;
            }
            self.pos += 1;
        }
    }

    fn string(&mut self) {
        let start = self.pos;
        self.pos += 1;
        loop {
            match self.byte(self.pos) {
                None | Some(b'\n') => {
                    self.err(Diagnostic::new(Span::new(start, self.pos), code::STRING, "문자열이 닫히지 않음").suggest("\"로 닫기"));
                    break;
                }
                Some(b'\\') if matches!(self.byte(self.pos + 1), Some(b'"') | Some(b'\\')) => self.pos += 2,
                Some(b'"') => {
                    self.pos += 1;
                    break;
                }
                Some(_) => self.pos += self.peek_char().unwrap().len_utf8(),
            }
        }
        self.push(Tok::Str, start);
    }

    fn tex_name(&mut self) {
        let start = self.pos;
        self.pos += 1;
        while self.byte(self.pos).is_some_and(|b| b.is_ascii_alphabetic()) {
            self.pos += 1;
        }
        if self.pos == start + 1 {
            self.err(Diagnostic::new(Span::new(start, self.pos), code::CHAR, "'\\' 뒤에 LaTeX 이름이 없음").suggest("\\theta 같은 이름"));
            self.push(Tok::Invalid, start);
        } else {
            self.push(Tok::TexName, start);
        }
    }

    fn ident(&mut self) {
        let start = self.pos;
        while let Some(c) = self.peek_char() {
            if !is_name_continue(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
        let tok = match Keyword::parse(&self.src[start..self.pos]) {
            Some(k) => Tok::Kw(k),
            None => Tok::Ident,
        };
        self.push(tok, start);
    }

    fn digits(&mut self) {
        while self.byte(self.pos).is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
    }

    fn number(&mut self) {
        let start = self.pos;
        let leading_dot = self.byte(self.pos) == Some(b'.');
        let mut decimal = leading_dot;
        if leading_dot {
            self.pos += 1;
            self.digits();
            let text = &self.src[start..self.pos];
            self.err(
                Diagnostic::new(Span::new(start, self.pos), code::NUMBER, format!("'{text}'처럼 점 앞에 숫자가 없음"))
                    .suggest(format!("0{text}")),
            );
        } else {
            self.digits();
            if self.byte(self.pos) == Some(b'.') {
                if self.byte(self.pos + 1).is_some_and(|b| b.is_ascii_digit()) {
                    decimal = true;
                    self.pos += 1;
                    self.digits();
                } else {
                    // `5.` — a trailing dot is never an address on a number.
                    self.pos += 1;
                    let text = &self.src[start..self.pos];
                    let int = &text[..text.len() - 1];
                    self.err(
                        Diagnostic::new(Span::new(start, self.pos), code::NUMBER, format!("'{text}'처럼 점 뒤에 숫자가 없음"))
                            .suggest(format!("{int} 또는 {int}.0")),
                    );
                    decimal = true;
                }
            }
        }
        // Exponent only when followed by digits, so `2e` stays `2` `e` (caught as implicit multiplication).
        if matches!(self.byte(self.pos), Some(b'e' | b'E')) {
            let mut p = self.pos + 1;
            if matches!(self.byte(p), Some(b'+' | b'-')) {
                p += 1;
            }
            if self.byte(p).is_some_and(|b| b.is_ascii_digit()) {
                self.pos = p;
                self.digits();
                decimal = true;
            }
        }
        if self.byte(self.pos) == Some(b'_') && self.byte(self.pos + 1).is_some_and(|b| b.is_ascii_digit()) {
            let s = self.pos;
            self.pos += 1;
            self.digits();
            let text = &self.src[start..self.pos];
            self.err(
                Diagnostic::new(Span::new(s, self.pos), code::NUMBER, "숫자 자릿수 구분 '_'는 쓸 수 없음 ('_'는 첨자 전용)")
                    .suggest(text.replace('_', "")),
            );
        }
        self.push(if decimal { Tok::Decimal } else { Tok::Int }, start);
    }

    fn punct(&mut self, c: char) {
        let start = self.pos;
        let next = self.byte(self.pos + 1);
        let (tok, len) = match (c, next) {
            ('-', Some(b'>')) => (Tok::Arrow, 2),
            ('<', Some(b'=')) => (Tok::Le, 2),
            ('>', Some(b'=')) => (Tok::Ge, 2),
            ('=', Some(b'=')) => (Tok::EqEq, 2),
            ('+', _) => (Tok::Plus, 1),
            ('-', _) => (Tok::Minus, 1),
            ('*', Some(b'*')) => return self.bad(2, "'**'는 거듭제곱이 아님", Some("^")),
            ('*', _) => (Tok::Star, 1),
            ('/', _) => (Tok::Slash, 1),
            ('^', _) => (Tok::Caret, 1),
            ('!', Some(b'=')) => return self.bad(2, "'!='는 쓸 수 없음", Some("not (a == b)")),
            ('!', _) => (Tok::Bang, 1),
            ('(', _) => (Tok::LParen, 1),
            (')', _) => (Tok::RParen, 1),
            ('[', _) => (Tok::LBracket, 1),
            (']', _) => (Tok::RBracket, 1),
            ('{', _) => (Tok::LBrace, 1),
            ('}', _) => (Tok::RBrace, 1),
            (',', _) => (Tok::Comma, 1),
            (';', _) => (Tok::Semi, 1),
            (':', _) => (Tok::Colon, 1),
            ('<', _) => (Tok::Lt, 1),
            ('>', _) => (Tok::Gt, 1),
            ('=', _) => (Tok::Eq, 1),
            ('@', _) => (Tok::At, 1),
            ('~', _) => (Tok::Tilde, 1),
            ('.', _) => (Tok::Dot, 1),
            ('_', _) => (Tok::Underscore, 1),
            ('|', _) => return self.bad(1, "'|'는 쓸 수 없음", Some("abs(x)")),
            ('%', _) => return self.bad(1, "'%'는 쓸 수 없음", Some("mod(a, m)")),
            ('\'', _) => return self.bad(1, "'''(프라임)은 쓸 수 없음", Some("f@1")),
            ('&', _) => return self.bad(1, "'&'는 쓸 수 없음", Some("and")),
            _ => return self.bad(1, &format!("'{c}'는 쓸 수 없음"), None),
        };
        match tok {
            Tok::LParen | Tok::LBracket | Tok::LBrace => {
                if self.depth == 0 {
                    self.open_indent = *self.indents.last().unwrap();
                }
                self.depth += 1
            }
            Tok::RParen | Tok::RBracket | Tok::RBrace => self.depth = self.depth.saturating_sub(1),
            _ => {}
        }
        self.pos += len;
        self.push(tok, start);
    }

    fn bad(&mut self, len: usize, msg: &str, suggestion: Option<&str>) {
        let start = self.pos;
        self.pos += len;
        let d = Diagnostic::new(Span::new(start, self.pos), code::CHAR, msg);
        self.err(match suggestion {
            Some(s) => d.suggest(s),
            None => d,
        });
        self.push(Tok::Invalid, start);
    }
}

/// LaTeX / ASCII replacement for a rejected non-ASCII character.
pub fn unicode_suggestion(c: char) -> Option<&'static str> {
    Some(match c {
        'α' => "\\alpha",
        'β' => "\\beta",
        'γ' => "\\gamma",
        'δ' => "\\delta",
        'ε' | 'ϵ' => "\\epsilon",
        'ζ' => "\\zeta",
        'η' => "\\eta",
        'θ' | 'ϑ' => "\\theta",
        'κ' => "\\kappa",
        'λ' => "\\lambda",
        'μ' => "\\mu",
        'ν' => "\\nu",
        'ξ' => "\\xi",
        'π' => "\\pi",
        'ρ' => "\\rho",
        'σ' => "\\sigma",
        'τ' => "\\tau",
        'φ' | 'ϕ' => "\\phi",
        'χ' => "\\chi",
        'ψ' => "\\psi",
        'ω' => "\\omega",
        'Γ' => "\\Gamma",
        'Δ' => "\\Delta",
        'Θ' => "\\Theta",
        'Λ' => "\\Lambda",
        'Σ' | '∑' => "sum(식; 1 <= k <= n)",
        'Π' | '∏' => "prod(식; 1 <= k <= n)",
        'Φ' => "\\Phi",
        'Ψ' => "\\Psi",
        'Ω' => "\\Omega",
        '∞' => "\\infty",
        '≤' | '⩽' => "<=",
        '≥' | '⩾' => ">=",
        '≠' => "not (a == b)",
        '×' | '·' | '⋅' | '∙' => "*",
        '÷' => "/",
        '−' | '–' | '—' => "-",
        '√' => "sqrt(x)",
        '°' => "deg(x)",
        '′' => "f@1",
        '″' => "f@2",
        '²' => "^2",
        '³' => "^3",
        '“' | '”' => "\"",
        '‘' | '’' => "'",
        '（' => "(",
        '）' => ")",
        '，' => ",",
        '　' => "스페이스",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use Tok::*;

    fn toks(src: &str) -> Vec<Tok> {
        let (t, d) = lex(src);
        assert!(d.is_empty(), "unexpected diagnostics: {d:?}");
        t.into_iter().map(|t| t.tok).collect()
    }

    fn codes(src: &str) -> Vec<&'static str> {
        lex(src).1.into_iter().map(|d| d.code).collect()
    }

    fn text(src: &str) -> Vec<String> {
        lex(src).0.iter().filter(|t| !matches!(t.tok, Newline | Eof)).map(|t| src[t.span.range()].to_string()).collect()
    }

    #[test]
    fn operators_and_names() {
        assert_eq!(
            toks("f(x; a) -> y: a*x^2 <= 1 == b"),
            vec![Ident, LParen, Ident, Semi, Ident, RParen, Arrow, Ident, Colon, Ident, Star, Ident, Caret, Int, Le, Int, EqEq, Ident, Newline, Eof]
        );
        assert_eq!(text("x_{1} f@2 ~f \\theta x^[*]"), vec!["x", "_", "{", "1", "}", "f", "@", "2", "~", "f", "\\theta", "x", "^", "[", "*", "]"]);
    }

    #[test]
    fn keywords() {
        assert_eq!(toks("del f"), vec![Kw(Keyword::Del), Ident, Newline, Eof]);
        assert_eq!(toks("not a and b or c"), vec![Kw(Keyword::Not), Ident, Kw(Keyword::And), Ident, Kw(Keyword::Or), Ident, Newline, Eof]);
        // Field names are not keywords.
        assert_eq!(toks("show"), vec![Ident, Newline, Eof]);
    }

    #[test]
    fn numbers() {
        assert_eq!(toks("1.5 0.5 1e-3 2E+4 7"), vec![Decimal, Decimal, Decimal, Decimal, Int, Newline, Eof]);
        // `2x` and `2e` lex as number + name; the parser reports implicit multiplication.
        assert_eq!(toks("2x 2e"), vec![Int, Ident, Int, Ident, Newline, Eof]);
    }

    #[test]
    fn number_errors() {
        assert_eq!(codes(".5"), vec![code::NUMBER]);
        assert_eq!(codes("5."), vec![code::NUMBER]);
        assert_eq!(codes("1_000"), vec![code::NUMBER]);
        let (_, d) = lex(".5");
        assert_eq!(d[0].suggestion.as_deref(), Some("0.5"));
    }

    #[test]
    fn address_dot_is_not_a_number() {
        assert_eq!(toks("f.(2*x).coef"), vec![Ident, Dot, LParen, Int, Star, Ident, RParen, Dot, Ident, Newline, Eof]);
    }

    #[test]
    fn hangul_names_allowed() {
        assert_eq!(toks("넓이1 = 3"), vec![Ident, Eq, Int, Newline, Eof]);
    }

    #[test]
    fn unicode_rejected_with_latex_suggestion() {
        let (t, d) = lex("sin(θ)");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, code::UNICODE);
        assert_eq!(d[0].suggestion.as_deref(), Some("\\theta"));
        assert_eq!(t[2].tok, Invalid);
    }

    #[test]
    fn unicode_allowed_inside_strings() {
        assert_eq!(toks("\"θ ≤ π {f} $x$\""), vec![Str, Newline, Eof]);
    }

    #[test]
    fn string_escapes_and_errors() {
        assert_eq!(text(r#""a \" b \\""#), vec![r#""a \" b \\""#]);
        // `\n` is not an escape, so `\nabla` survives.
        assert_eq!(toks(r#""\nabla""#), vec![Str, Newline, Eof]);
        assert_eq!(codes("\"abc"), vec![code::STRING]);
    }

    #[test]
    fn comments_and_blank_lines() {
        assert_eq!(toks("# c\n\nx # tail\n\n# end"), vec![Ident, Newline, Eof]);
    }

    #[test]
    fn indentation_blocks() {
        assert_eq!(
            toks("define:\n    f(x) -> y: x\n    g(x) -> y: x\nh"),
            vec![
                Kw(Keyword::Define), Colon, Newline, Indent,
                Ident, LParen, Ident, RParen, Arrow, Ident, Colon, Ident, Newline,
                Ident, LParen, Ident, RParen, Arrow, Ident, Colon, Ident, Newline,
                Dedent, Ident, Newline, Eof
            ]
        );
        // Dedents are closed at end of input.
        assert_eq!(toks("a:\n    b:\n        c"), vec![Ident, Colon, Newline, Indent, Ident, Colon, Newline, Indent, Ident, Newline, Dedent, Dedent, Eof]);
    }

    #[test]
    fn indentation_errors() {
        assert_eq!(codes("a:\n  b"), vec![code::INDENT]);
        assert_eq!(codes("a:\n\tb"), vec![code::TAB]);
        assert_eq!(codes("a:\n    b:\n        c\n  d"), vec![code::DEDENT]);
        let (_, d) = lex("a:\n  b");
        assert_eq!(d[0].message, "2칸 들어감, 4칸 필요");
    }

    #[test]
    fn newlines_ignored_inside_brackets() {
        assert_eq!(toks("A: [[1, 2],\n      [3, 4]]"), vec![
            Ident, Colon, LBracket, LBracket, Int, Comma, Int, RBracket, Comma, LBracket, Int, Comma, Int, RBracket, RBracket, Newline, Eof
        ]);
    }

    #[test]
    fn unclosed_bracket_ends_at_next_shallower_line() {
        assert_eq!(toks("f(x\ng"), vec![Ident, LParen, Ident, Newline, Ident, Newline, Eof]);
        // Deeper continuation lines and a leading closer still continue the bracket.
        assert_eq!(toks("f(x,\n    y\n)"), vec![Ident, LParen, Ident, Comma, Ident, RParen, Newline, Eof]);
    }

    #[test]
    fn rejected_ascii() {
        assert_eq!(codes("a | b"), vec![code::CHAR]);
        assert_eq!(codes("a != b"), vec![code::CHAR]);
        assert_eq!(codes("a ** b"), vec![code::CHAR]);
        assert_eq!(codes("f'(x)"), vec![code::CHAR]);
        assert_eq!(codes("\\ x"), vec![code::CHAR]);
    }

    #[test]
    fn several_errors_reported_at_once() {
        assert_eq!(codes("θ + .5 | 5."), vec![code::UNICODE, code::NUMBER, code::CHAR, code::NUMBER]);
    }
}
