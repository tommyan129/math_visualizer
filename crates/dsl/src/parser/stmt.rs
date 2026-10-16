//! Statements. Recovery: a failed statement is skipped to the end of its line
//! (plus any block nested under it) and parsing resumes at the next line at the
//! same indentation, so independent errors are all reported at once.

use super::{PResult, Parser};
use crate::ast::Ident;
use crate::diag::{Diagnostic, code};
use crate::lexer::{Keyword, Tok};
use crate::span::Span;
use crate::stmt::*;

impl<'a> Parser<'a> {
    pub(crate) fn program(&mut self) -> Vec<Stmt> {
        let mut out = vec![];
        loop {
            match self.peek() {
                Tok::Eof => break,
                Tok::Newline | Tok::Dedent => {
                    self.bump();
                }
                Tok::Indent => {
                    let span = self.peek_span();
                    self.diag(Diagnostic::new(span, code::INDENT, "여기서는 들여쓸 수 없음").suggest("들여쓰기 없이"));
                    self.skip_block();
                }
                _ => match self.statement() {
                    Ok(s) => out.push(s),
                    Err(()) => self.sync_line(),
                },
            }
        }
        out
    }

    /// Skips to the start of the next line at the current indentation.
    fn sync_line(&mut self) {
        loop {
            match self.peek() {
                Tok::Eof | Tok::Dedent => return,
                Tok::Newline => {
                    self.bump();
                    break;
                }
                _ => {
                    self.bump();
                }
            }
        }
        if self.at(Tok::Indent) {
            self.skip_block();
        }
    }

    /// At `Indent`: skips through the matching `Dedent`. Returns the block span.
    fn skip_block(&mut self) -> Span {
        let start = self.peek_span();
        let mut depth = 0;
        loop {
            let t = self.bump();
            match t.tok {
                Tok::Indent => depth += 1,
                Tok::Dedent => {
                    depth -= 1;
                    if depth == 0 {
                        return start.to(self.prev_span());
                    }
                }
                Tok::Eof => return start.to(t.span),
                _ => {}
            }
        }
    }

    fn expect_line_end(&mut self) -> PResult<()> {
        match self.peek() {
            Tok::Newline => {
                self.bump();
                Ok(())
            }
            Tok::Eof | Tok::Dedent => Ok(()),
            _ => self.unexpected("줄 끝"),
        }
    }

    fn unsupported<T>(&mut self, span: Span, what: &str, stage: u8) -> PResult<T> {
        self.diag(Diagnostic::new(span, code::UNSUPPORTED, format!("{what}은(는) 아직 지원하지 않음 ({stage}단계 기능)")));
        Err(())
    }

    fn statement(&mut self) -> PResult<Stmt> {
        let span = self.peek_span();
        match self.peek() {
            Tok::Kw(Keyword::Define) => self.define(),
            Tok::Kw(Keyword::Del) => self.del(),
            Tok::Kw(Keyword::Constant) => self.unsupported(span, "constant: 블록", 2),
            Tok::Kw(Keyword::Scene) => self.unsupported(span, "scene", 3),
            Tok::Kw(Keyword::Step) => self.unsupported(span, "step", 3),
            Tok::Ident if self.peek_at(1) == Tok::Eq => self.unsupported(span, "변수 대입 (highlight 포함)", 2),
            Tok::Ident if matches!(self.peek_text(), "req" | "highlight") => self.unsupported(span, self.peek_text(), 2),
            _ => {
                // A definition written outside `define:`?
                let looks_like_def = self.toks[self.pos..]
                    .iter()
                    .take_while(|t| !matches!(t.tok, Tok::Newline | Tok::Eof))
                    .any(|t| matches!(t.tok, Tok::Arrow | Tok::Colon));
                if looks_like_def {
                    self.diag(Diagnostic::new(span, code::SYNTAX, "정의는 define: 블록 안에 써야 함").suggest("define:\n    f(x) -> y: ..."));
                } else {
                    self.report_unexpected("문장(define:, del)");
                }
                Err(())
            }
        }
    }

    fn define(&mut self) -> PResult<Stmt> {
        let start = self.bump().span;
        self.expect(Tok::Colon, "'define' 뒤에 ':'")?;
        if !self.at(Tok::Newline) || self.peek_at(1) != Tok::Indent {
            self.diag(Diagnostic::new(start, code::SYNTAX, "define: 아래에 들여쓴 정의가 와야 함").suggest("define:\n    f(x) -> y: ..."));
            return Err(());
        }
        self.bump();
        self.bump();
        let mut defs = vec![];
        loop {
            match self.peek() {
                Tok::Dedent => {
                    self.bump();
                    break;
                }
                Tok::Eof => break,
                Tok::Newline => {
                    self.bump();
                }
                Tok::Indent => {
                    let span = self.peek_span();
                    self.diag(Diagnostic::new(span, code::INDENT, "정의 사이에서는 더 들여쓸 수 없음").suggest("앞 정의와 같은 들여쓰기"));
                    self.skip_block();
                }
                _ => match self.def() {
                    Ok(d) => defs.push(d),
                    Err(()) => self.sync_line(),
                },
            }
        }
        let span = start.to(self.prev_span());
        Ok(Stmt::Define { defs, span })
    }

    fn name_list(&mut self) -> PResult<Vec<(Ident, Span)>> {
        let mut v = vec![self.name()?];
        while self.eat(Tok::Comma) {
            v.push(self.name()?);
        }
        Ok(v)
    }

    fn def(&mut self) -> PResult<Def> {
        let start = self.peek_span();
        let (name, name_span) = self.name()?;
        let r = self.def_rest(start, name.clone(), name_span);
        if r.is_err() {
            self.failed_defs.push(name.key);
        }
        r
    }

    fn def_rest(&mut self, start: Span, name: Ident, name_span: Span) -> PResult<Def> {

        let params = if self.at(Tok::LParen) {
            let open = self.bump().span;
            let inputs = if matches!(self.peek(), Tok::Semi | Tok::RParen) { vec![] } else { self.name_list()? };
            let consts = if self.eat(Tok::Semi) { self.name_list()? } else { vec![] };
            let close = self.expect(Tok::RParen, "')'")?;
            if inputs.is_empty() {
                self.diag(Diagnostic::new(open.to(close), code::SYNTAX, "입력 변수가 없음").suggest(format!("{}(x)", name.key)));
            }
            Some(Params { inputs, consts, span: open.to(close) })
        } else {
            None
        };

        let output = if self.at(Tok::Arrow) {
            self.bump();
            Some(self.output()?)
        } else {
            None
        };

        self.expect(Tok::Colon, "':'")?;

        let body = if self.at(Tok::Newline) && self.peek_at(1) == Tok::Indent {
            self.bump();
            DefBody::Block(self.skip_block())
        } else {
            let e = self.expr()?;
            let body = if self.eat(Tok::Eq) {
                DefBody::Equation { lhs: e, rhs: self.expr()? }
            } else if self.eat(Tok::Comma) {
                DefBody::WithRange { expr: e, range: self.expr()? }
            } else {
                DefBody::Expr(e)
            };
            self.expect_line_end()?;
            body
        };
        let end = match &body {
            DefBody::Block(s) => *s,
            DefBody::Expr(e) | DefBody::Equation { rhs: e, .. } | DefBody::WithRange { range: e, .. } => e.span,
        };
        Ok(Def { name, name_span, params, output, body, span: start.to(end) })
    }

    fn output(&mut self) -> PResult<Output> {
        let start = self.peek_span();
        if self.at(Tok::LParen) {
            self.bump();
            let axes = self.name_list()?;
            let end = self.expect(Tok::RParen, "')'")?;
            return Ok(Output::Tuple(axes, start.to(end)));
        }
        if self.at(Tok::Ident) && self.peek_text() == "matrix" {
            return Ok(Output::Matrix(self.bump().span));
        }
        let (name, span) = self.name()?;
        if self.at(Tok::LParen) {
            self.bump();
            let axes = self.name_list()?;
            let end = self.expect(Tok::RParen, "')'")?;
            return Ok(Output::Coord { system: name, axes, span: start.to(end) });
        }
        Ok(Output::Axis(name, span))
    }

    fn del(&mut self) -> PResult<Stmt> {
        let start = self.bump().span;
        let names = self.name_list()?;
        let span = start.to(self.prev_span());
        self.expect_line_end()?;
        Ok(Stmt::Del { names, span })
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse_program;
    use crate::ast::ExprKind;
    use crate::diag::code;
    use crate::stmt::*;

    fn ok(src: &str) -> Vec<Stmt> {
        let (s, d) = parse_program(src);
        assert!(d.is_empty(), "unexpected diagnostics: {d:?}");
        s
    }

    fn errs(src: &str) -> Vec<&'static str> {
        parse_program(src).1.into_iter().map(|d| d.code).collect()
    }

    fn defs(src: &str) -> Vec<Def> {
        match ok(src).into_iter().next().unwrap() {
            Stmt::Define { defs, .. } => defs,
            s => panic!("not a define: {s:?}"),
        }
    }

    #[test]
    fn define_plain_function() {
        let d = defs("define:\n    f(x; a, b) -> y: a*x^2 + b*x + 1\n");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].name.key, "f");
        let p = d[0].params.as_ref().unwrap();
        assert_eq!(p.inputs.iter().map(|(n, _)| n.key.as_str()).collect::<Vec<_>>(), ["x"]);
        assert_eq!(p.consts.iter().map(|(n, _)| n.key.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert!(matches!(&d[0].output, Some(Output::Axis(n, _)) if n.key == "y"));
        assert!(matches!(&d[0].body, DefBody::Expr(e) if matches!(e.kind, ExprKind::Sum { .. })));
    }

    #[test]
    fn define_several_and_source_span() {
        let src = "define:\n    f(x) -> y: x\n    g(x) -> y: f(x) + 1\n";
        let d = defs(src);
        assert_eq!(d.len(), 2);
        assert_eq!(&src[d[1].span.range()], "g(x) -> y: f(x) + 1");
    }

    #[test]
    fn later_stage_definition_shapes_parse() {
        let d = defs(
            "define:\n    C(x, y; r): x^2 + y^2 = r^2\n    p(t) -> (x, y): (cos(t), sin(t)), 0 <= t <= 2*\\pi\n    h(\\theta) -> polar(r): 1 + cos(\\theta)\n    R(\\theta) -> matrix: [[1, 0], [0, 1]]\n    A: [[1, 2], [3, 4]]\n    g(x, y) -> z:\n        split x at 0\n        x, x < 0\n",
        );
        assert!(matches!(d[0].body, DefBody::Equation { .. }));
        assert!(matches!(d[1].output, Some(Output::Tuple(..))) && matches!(d[1].body, DefBody::WithRange { .. }));
        assert!(matches!(&d[2].output, Some(Output::Coord { system, .. }) if system.key == "polar"));
        assert!(matches!(d[3].output, Some(Output::Matrix(_))));
        assert!(d[4].params.is_none());
        assert!(matches!(d[5].body, DefBody::Block(_)));
    }

    #[test]
    fn del_statement() {
        let s = ok("del f, g\n");
        assert!(matches!(&s[0], Stmt::Del { names, .. } if names.len() == 2 && names[1].0.key == "g"));
    }

    #[test]
    fn statements_in_sequence() {
        let s = ok("define:\n    f(x) -> y: x\ndel f\ndefine:\n    f(x) -> y: 2*x\n");
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn all_independent_errors_reported() {
        // Two broken definitions and one broken top-level statement; the good definition survives.
        let src = "define:\n    f(x) -> y: 2x\n    g(x) -> y: x +\n    h(x) -> y: x\ndel\n";
        let (s, d) = parse_program(src);
        assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), vec![code::IMPLICIT_MUL, code::SYNTAX, code::SYNTAX]);
        let Stmt::Define { defs, .. } = &s[0] else { panic!() };
        // f recovers inline (implicit multiplication), g is skipped, h parses.
        assert_eq!(defs.iter().map(|d| d.name.key.as_str()).collect::<Vec<_>>(), ["f", "h"]);
    }

    #[test]
    fn recovery_resumes_at_same_indentation() {
        // The broken split block is skipped as a whole; the next sibling definition still parses.
        let src = "define:\n    f(x -> y: x\n    g(x) -> y: x\nh(x) -> y: x\n";
        let (s, d) = parse_program(src);
        assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), vec![code::SYNTAX, code::SYNTAX]);
        let Stmt::Define { defs, .. } = &s[0] else { panic!() };
        assert_eq!(defs.len(), 1);
        assert_eq!(d[1].suggestion.as_deref(), Some("define:\n    f(x) -> y: ..."));
    }

    #[test]
    fn unsupported_statements() {
        assert_eq!(errs("constant:\n    a: range=(-5, 5)\n"), vec![code::UNSUPPORTED]);
        assert_eq!(errs("scene(1):\n    space: 2D(x, y)\n"), vec![code::UNSUPPORTED]);
        assert_eq!(errs("H1 = highlight(f;;)\n"), vec![code::UNSUPPORTED]);
        assert_eq!(errs("req(f)\n"), vec![code::UNSUPPORTED]);
    }

    #[test]
    fn structural_errors() {
        assert_eq!(errs("define:\n"), vec![code::SYNTAX]);
        assert_eq!(errs("define: f(x) -> y: x\n"), vec![code::SYNTAX]);
        assert_eq!(errs("define:\n    f(x) -> y x\n"), vec![code::SYNTAX]);
        assert_eq!(errs("    del f\n"), vec![code::INDENT]);
        assert_eq!(errs("define:\n    f(x) -> y: x\n        g(x) -> y: x\n"), vec![code::INDENT]);
        assert_eq!(errs("define:\n    f() -> y: 1\n"), vec![code::SYNTAX]);
        assert_eq!(errs("x + 1\n"), vec![code::SYNTAX]);
    }
}
