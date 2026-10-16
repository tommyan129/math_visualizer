//! Parser: tokens → statement / expression AST.
//! Errors are collected; an `Err(())` means "already reported, resynchronize".

mod expr;
mod stmt;

use crate::ast::Expr;
use crate::diag::{Diagnostic, code};
use crate::lexer::{Tok, Token, lex};
use crate::span::Span;
use crate::stmt::Stmt;

/// `Err(())` = already reported; the caller resynchronizes.
#[allow(clippy::result_unit_err)]
pub type PResult<T> = Result<T, ()>;

/// Nesting limit for expressions. Keeps every recursive pass (parse, resolve,
/// serde, drop) far from stack limits, including 2 MB test threads in debug builds.
pub const MAX_PARSE_DEPTH: usize = 64;

/// `stacker` parameters shared by every recursive pass.
pub const STACK_RED_ZONE: usize = 128 * 1024;
pub const STACK_GROW: usize = 2 * 1024 * 1024;

pub struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    pos: usize,
    diags: Vec<Diagnostic>,
    depth: usize,
    depth_reported: bool,
    /// Keys of definitions that failed to parse, so later references are not reported again.
    failed_defs: Vec<String>,
}

impl<'a> Parser<'a> {
    pub fn new(src: &'a str) -> Self {
        let (toks, diags) = lex(src);
        Parser { src, toks, pos: 0, diags, depth: 0, depth_reported: false, failed_defs: vec![] }
    }

    fn peek(&self) -> Tok {
        self.peek_at(0)
    }

    fn peek_at(&self, n: usize) -> Tok {
        self.toks.get(self.pos + n).map_or(Tok::Eof, |t| t.tok)
    }

    fn peek_span(&self) -> Span {
        self.toks.get(self.pos).map_or_else(|| Span::new(self.src.len(), self.src.len()), |t| t.span)
    }

    fn peek_text(&self) -> &'a str {
        &self.src[self.peek_span().range()]
    }

    fn prev_span(&self) -> Span {
        self.pos.checked_sub(1).map_or(Span::default(), |i| self.toks[i].span)
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos.min(self.toks.len() - 1)];
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn at(&self, t: Tok) -> bool {
        self.peek() == t
    }

    fn eat(&mut self, t: Tok) -> bool {
        if self.at(t) {
            self.bump();
            true
        } else {
            false
        }
    }

    /// Runs `f` one nesting level deeper; fails once `MAX_PARSE_DEPTH` is exceeded.
    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> PResult<T>) -> PResult<T> {
        if self.depth >= MAX_PARSE_DEPTH {
            if !self.depth_reported {
                self.depth_reported = true;
                let span = self.peek_span();
                self.diag(Diagnostic::new(span, code::LIMIT, format!("식이 너무 깊게 중첩됨 (상한 {MAX_PARSE_DEPTH})")));
            }
            return Err(());
        }
        self.depth += 1;
        // Debug-build frames are large (~40 KB per level); grow the stack on demand.
        let r = stacker::maybe_grow(STACK_RED_ZONE, STACK_GROW, || f(self));
        self.depth -= 1;
        r
    }

    fn diag(&mut self, d: Diagnostic) {
        self.diags.push(d);
    }

    fn describe_current(&self) -> String {
        match self.peek() {
            Tok::Newline => "줄 끝".into(),
            Tok::Eof => "입력 끝".into(),
            Tok::Indent => "들여쓰기".into(),
            Tok::Dedent => "내어쓰기".into(),
            _ => format!("'{}'", self.peek_text()),
        }
    }

    /// Reports at the current token and fails.
    fn error_here<T>(&mut self, code: &'static str, msg: &str, suggestion: Option<&str>) -> PResult<T> {
        let mut d = Diagnostic::new(self.peek_span(), code, msg);
        if let Some(s) = suggestion {
            d = d.suggest(s);
        }
        self.diag(d);
        Err(())
    }

    /// Reports "expected X, found Y" and fails.
    fn unexpected<T>(&mut self, expected: &str) -> PResult<T> {
        self.report_unexpected(expected);
        Err(())
    }

    fn report_unexpected(&mut self, expected: &str) {
        let found = self.describe_current();
        self.diag(Diagnostic::new(self.peek_span(), code::SYNTAX, format!("{expected}이(가) 와야 하는데 {found}이(가) 옴")));
    }

    fn expect(&mut self, t: Tok, what: &str) -> PResult<Span> {
        if self.at(t) {
            Ok(self.bump().span)
        } else {
            self.unexpected(what)
        }
    }
}

/// Parses a single expression line (tests and the browser-facing helpers).
pub fn parse_expr(src: &str) -> (Option<Expr>, Vec<Diagnostic>) {
    let mut p = Parser::new(src);
    let e = p.expr().ok();
    if e.is_some() && !matches!(p.peek(), Tok::Newline | Tok::Eof) {
        p.report_unexpected("줄 끝");
    }
    (e, p.diags)
}

pub struct Parsed {
    pub stmts: Vec<Stmt>,
    pub diags: Vec<Diagnostic>,
    pub failed_defs: Vec<String>,
}

/// Parses a whole request.
pub fn parse_request(src: &str) -> Parsed {
    let mut p = Parser::new(src);
    let stmts = p.program();
    Parsed { stmts, diags: p.diags, failed_defs: p.failed_defs }
}

#[cfg(test)]
pub fn parse_program(src: &str) -> (Vec<Stmt>, Vec<Diagnostic>) {
    let p = parse_request(src);
    (p.stmts, p.diags)
}
