//! Statements. Recovery: a failed statement is skipped to the end of its line
//! (plus any block nested under it) and parsing resumes at the next line at the
//! same indentation, so independent errors are all reported at once.

use super::{PResult, Parser};
use crate::ast::{Expr, ExprKind, Ident, Piece};
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
    pub(super) fn sync_line(&mut self) {
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
    pub(super) fn skip_block(&mut self) -> Span {
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

    pub(super) fn expect_line_end(&mut self) -> PResult<()> {
        match self.peek() {
            Tok::Newline => {
                self.bump();
                Ok(())
            }
            Tok::Eof | Tok::Dedent => Ok(()),
            _ => self.unexpected("줄 끝"),
        }
    }

    fn statement(&mut self) -> PResult<Stmt> {
        let span = self.peek_span();
        match self.peek() {
            Tok::Kw(Keyword::Define) => self.define(),
            Tok::Kw(Keyword::Del) => self.del(),
            Tok::Kw(Keyword::Constant) => self.constant(),
            Tok::Kw(Keyword::Style) => self.style(),
            Tok::Kw(Keyword::Scene) => self.scene(),
            Tok::Kw(Keyword::Step) => {
                self.diag(Diagnostic::new(span, code::SCENE, "step은 scene 블록 안에만 씀").suggest("scene(1):\n    space: 2D(x, y)\n    step:\n        show: +f"));
                Err(())
            }
            _ if self.line_has_assignment() => self.assign(),
            Tok::Ident if self.peek_text() == "req" && self.peek_at(1) == Tok::LParen => {
                let (slots, scene) = self.slots_with_scene()?;
                self.expect_line_end()?;
                Ok(Stmt::Req { span: slots.span, slots, scene })
            }
            Tok::Ident if self.peek_text() == "highlight" && self.peek_at(1) == Tok::LParen => {
                let slots = self.slots()?;
                let text = &self.src[slots.span.range()];
                self.diag(Diagnostic::new(slots.span, code::HIGHLIGHT_NAME, "highlight에는 이름이 필요함").suggest(format!("H1 = {text}")));
                Err(())
            }
            _ => {
                // A definition written outside `define:`?
                let looks_like_def = self.toks[self.pos..]
                    .iter()
                    .take_while(|t| !matches!(t.tok, Tok::Newline | Tok::Eof))
                    .any(|t| matches!(t.tok, Tok::Arrow | Tok::Colon));
                if looks_like_def {
                    self.diag(Diagnostic::new(span, code::SYNTAX, "정의는 define: 블록 안에 써야 함").suggest("define:\n    f(x) -> y: ..."));
                } else {
                    self.report_unexpected("문장(define:, constant:, style, del, 이름 = 값, req)");
                }
                Err(())
            }
        }
    }

    /// Does the current line contain `=` outside brackets (an assignment)?
    fn line_has_assignment(&self) -> bool {
        let mut depth = 0i32;
        for t in &self.toks[self.pos..] {
            match t.tok {
                Tok::Newline | Tok::Eof => return false,
                Tok::LParen | Tok::LBracket | Tok::LBrace => depth += 1,
                Tok::RParen | Tok::RBracket | Tok::RBrace => depth -= 1,
                Tok::Colon | Tok::Arrow if depth == 0 => return false,
                Tok::Eq if depth == 0 => return true,
                _ => {}
            }
        }
        false
    }

    fn assign(&mut self) -> PResult<Stmt> {
        let start = self.peek_span();
        let (name, name_span) = self.name()?;
        self.expect(Tok::Eq, "'='")?;
        let value = if self.at(Tok::Ident) && self.peek_at(1) == Tok::LParen && self.peek_text() == "highlight" {
            AssignValue::Highlight(self.slots()?)
        } else if self.at(Tok::Ident) && self.peek_at(1) == Tok::LParen && self.peek_text() == "req" {
            let slots = self.slots()?;
            let text = &self.src[slots.span.range()];
            self.diag(Diagnostic::new(start.to(slots.span), code::SYNTAX, "req에는 이름을 붙이지 않음 (번호가 자동으로 붙음)").suggest(text.to_string()));
            return Err(());
        } else {
            AssignValue::Expr(self.expr()?)
        };
        let end = self.prev_span();
        self.expect_line_end()?;
        Ok(Stmt::Assign { name, name_span, value, span: start.to(end) })
    }

    /// `head(slot; slot; ...)`: `;`-separated, empty slots allowed.
    fn slots(&mut self) -> PResult<Slots> {
        let (slots, scene) = self.slots_inner(false)?;
        debug_assert!(scene.is_none());
        Ok(slots)
    }

    /// `req(...)`: the first slot may be `scene(1)` or `scene(1).step(3)` (left `None` in `items`).
    fn slots_with_scene(&mut self) -> PResult<(Slots, Option<ReqScene>)> {
        self.slots_inner(true)
    }

    fn slots_inner(&mut self, scene_first: bool) -> PResult<(Slots, Option<ReqScene>)> {
        let head = self.bump().span;
        self.expect(Tok::LParen, "'('")?;
        let mut items = vec![];
        let mut spans = vec![];
        let mut scene = None;
        loop {
            if scene_first && items.is_empty() && self.at(Tok::Kw(Keyword::Scene)) {
                let (r, span) = self.scene_ref()?;
                let mut end = span;
                let step = if self.at(Tok::Dot) && self.peek_at(1) == Tok::Kw(Keyword::Step) {
                    self.bump();
                    self.bump();
                    self.expect(Tok::LParen, "'('")?;
                    let t = self.peek_span();
                    let n = self.positive_int("step 번호")?;
                    end = self.expect(Tok::RParen, "')'")?;
                    Some((n, t))
                } else {
                    None
                };
                spans.push(span.to(end));
                items.push(None);
                scene = Some((r, step, span.to(end)));
            } else if matches!(self.peek(), Tok::Semi | Tok::RParen) {
                spans.push(self.peek_span());
                items.push(None);
            } else {
                let e = self.expr()?;
                spans.push(e.span);
                items.push(Some(e));
            }
            if self.at(Tok::Comma) {
                return self.error_here(code::SYNTAX, "칸 구분은 ','가 아니라 ';'", Some(";"));
            }
            if !self.eat(Tok::Semi) {
                break;
            }
        }
        let end = self.expect(Tok::RParen, "';' 또는 ')'")?;
        Ok((Slots { items, spans, span: head.to(end) }, scene))
    }

    /// `constant:` with one indented entry per line.
    fn constant(&mut self) -> PResult<Stmt> {
        let start = self.bump().span;
        self.expect(Tok::Colon, "'constant' 뒤에 ':'")?;
        if !self.at(Tok::Newline) || self.peek_at(1) != Tok::Indent {
            self.diag(Diagnostic::new(start, code::SYNTAX, "constant: 아래에 들여쓴 상수가 와야 함").suggest("constant:\n    a: range=(-5, 5), default=1"));
            return Err(());
        }
        self.bump();
        self.bump();
        let mut entries = vec![];
        self.block_lines(|p| {
            entries.push(p.const_entry()?);
            Ok(())
        });
        Ok(Stmt::Constant { entries, span: start.to(self.prev_span()) })
    }

    /// Runs `line` for each line of an indented block (already past `Indent`), with recovery.
    pub(super) fn block_lines(&mut self, mut line: impl FnMut(&mut Self) -> PResult<()>) {
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
                    self.diag(Diagnostic::new(span, code::INDENT, "여기서는 더 들여쓸 수 없음").suggest("앞 줄과 같은 들여쓰기"));
                    self.skip_block();
                }
                _ => {
                    if line(self).is_err() {
                        self.sync_line();
                    }
                }
            }
        }
    }

    fn const_entry(&mut self) -> PResult<ConstEntry> {
        let start = self.peek_span();
        let (name, name_span) = self.name()?;
        self.expect(Tok::Colon, "':'")?;
        let mut kind = None;
        let mut props = vec![];
        loop {
            let key_tok = self.peek();
            let is_key = matches!(key_tok, Tok::Ident | Tok::Kw(Keyword::Step)) && self.peek_at(1) == Tok::Eq;
            if is_key {
                let t = self.bump();
                let key = self.src[t.span.range()].to_string();
                self.bump();
                let value = self.expr()?;
                props.push(Prop { key, key_span: t.span, value });
            } else if self.at(Tok::Ident) && kind.is_none() && props.is_empty() {
                let t = self.bump();
                kind = Some((self.src[t.span.range()].to_string(), t.span));
            } else {
                return self.unexpected("'range=', 'default=', 'step=' 또는 종류(real, integer)");
            }
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        let end = self.prev_span();
        self.expect_line_end()?;
        Ok(ConstEntry { name, name_span, kind, props, span: start.to(end) })
    }

    /// `style name:` with indented `prop: value` lines.
    fn style(&mut self) -> PResult<Stmt> {
        let start = self.bump().span;
        let (name, name_span) = self.name()?;
        self.expect(Tok::Colon, "':'")?;
        if !self.at(Tok::Newline) || self.peek_at(1) != Tok::Indent {
            self.diag(Diagnostic::new(start, code::SYNTAX, "style 아래에 들여쓴 속성이 와야 함").suggest(format!("style {}:\n    color: orange\n    fill: above", name.key)));
            return Err(());
        }
        self.bump();
        self.bump();
        let mut props = vec![];
        self.block_lines(|p| {
            let t = p.peek_span();
            if !p.at(Tok::Ident) {
                return p.unexpected("속성 이름(color, fill, opacity, width)");
            }
            p.bump();
            let key = p.src[t.range()].to_string();
            p.expect(Tok::Colon, "':'")?;
            let value = p.expr()?;
            let span = t.to(value.span);
            p.expect_line_end()?;
            props.push(StyleProp { key, key_span: t, value, span });
            Ok(())
        });
        Ok(Stmt::Style { name, name_span, props, span: start.to(self.prev_span()) })
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

    pub(super) fn name_list(&mut self) -> PResult<Vec<(Ident, Span)>> {
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
        if r.as_ref().map_or(true, |d| matches!(&d.body, DefBody::Expr(e) if matches!(e.kind, ExprKind::Error {}))) {
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
            // `v(; a)`: a vector, matrix, span or arrow with constants has no inputs
            // (checked against the body by the interpreter).
            if inputs.is_empty() && consts.is_empty() {
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

        let mut family = None;
        let mut family_end = None;
        let body = if self.at(Tok::Newline) && self.peek_at(1) == Tok::Indent {
            self.bump();
            DefBody::Expr(self.split_block())
        } else if self.at(Tok::Ident) && self.peek_text() == "family" && self.peek_at(1) == Tok::LParen {
            // `family(body; 0 <= k <= n)`: the body is any one-line definition body.
            self.bump();
            self.bump();
            let form = "family(식; 0 <= k <= n)";
            let e = self.expr()?;
            let body = if self.eat(Tok::Eq) {
                DefBody::Equation { lhs: e, rhs: self.expr()? }
            } else if self.eat(Tok::Comma) {
                DefBody::WithRange { expr: e, range: self.expr()? }
            } else {
                DefBody::Expr(e)
            };
            if !self.eat(Tok::Semi) {
                return self.error_here(code::BIG_OP_RANGE, "family에는 반복 범위 칸이 필요함 (본문 뒤에 ';')", Some(form));
            }
            let range = self.expr()?;
            family_end = Some(self.expect(Tok::RParen, "')'")?);
            let (index, lo, lo_strict, hi, hi_strict) = self.index_range(&range, form).ok_or(())?;
            family = Some(FamilyHead { index, lo, lo_strict, hi, hi_strict, range_span: range.span });
            self.expect_line_end()?;
            body
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
            DefBody::Expr(e) | DefBody::Equation { rhs: e, .. } | DefBody::WithRange { range: e, .. } => e.span,
        };
        let end = family_end.unwrap_or(end);
        Ok(Def { name, name_span, params, output, body, family, span: start.to(end) })
    }

    /// Indented block under a definition head: `split [name =] expr at b1, b2, ...` and
    /// then one `value, condition` line per piece (design.md "구간별 함수"). On error the
    /// rest of the block is skipped and an `Error` node stands for the body (the caller
    /// marks the definition failed; the parser already resynchronized).
    fn split_block(&mut self) -> Expr {
        let start = self.bump().span; // Indent
        match self.split_lines() {
            Ok(e) => e,
            Err(()) => {
                let mut depth = 1;
                while depth > 0 && !self.at(Tok::Eof) {
                    match self.bump().tok {
                        Tok::Indent => depth += 1,
                        Tok::Dedent => depth -= 1,
                        _ => {}
                    }
                }
                Expr::new(ExprKind::Error {}, start.to(self.prev_span()))
            }
        }
    }

    fn split_lines(&mut self) -> PResult<Expr> {
        let start = self.peek_span();
        if !self.at(Tok::Kw(Keyword::Split)) {
            return self.error_here(code::SYNTAX, "정의 아래 블록은 split으로 시작해야 함", Some("f(x) -> y:\n    split x at 0\n    -x, x < 0\n    x, x >= 0"));
        }
        self.bump();
        let name = if matches!(self.peek(), Tok::Ident | Tok::TexName) && self.peek_at(1) == Tok::Eq {
            let (n, _) = self.name()?;
            self.bump();
            Some(n)
        } else {
            None
        };
        let subject = self.expr()?;
        if !self.eat(Tok::Kw(Keyword::At)) {
            return self.unexpected("'at'");
        }
        let mut at = vec![self.expr()?];
        while self.eat(Tok::Comma) {
            at.push(self.expr()?);
        }
        self.expect_line_end()?;
        let mut pieces = vec![];
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
                Tok::Indent => return self.error_here(code::INDENT, "split 줄은 같은 들여쓰기로 씀", None),
                _ => {
                    let value = self.expr()?;
                    if !self.eat(Tok::Comma) {
                        return self.error_here(code::SYNTAX, "split 줄은 '식, 구간' 모양이어야 함", Some("x^2, x < 0"));
                    }
                    let cond = self.expr()?;
                    self.expect_line_end()?;
                    pieces.push(Piece { value, cond });
                }
            }
        }
        if pieces.is_empty() {
            // The block is already closed: report without skipping further.
            self.diag(Diagnostic::new(start, code::SYNTAX, "split 아래에 '식, 구간' 줄이 없음").suggest("split x at 0\n    -x, x < 0\n    x, x >= 0"));
            return Ok(Expr::new(ExprKind::Error {}, start));
        }
        let end = pieces.last().map_or(start, |p| p.cond.span);
        Ok(Expr::new(ExprKind::Split { name, subject: Box::new(subject), at, pieces }, start.to(end)))
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
        let mut names = vec![];
        let mut scenes = vec![];
        loop {
            if self.at(Tok::Kw(Keyword::Scene)) {
                scenes.push(self.scene_ref()?);
            } else {
                names.push(self.name()?);
            }
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        let span = start.to(self.prev_span());
        self.expect_line_end()?;
        Ok(Stmt::Del { names, scenes, span })
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
        assert!(matches!(&d[5].body, DefBody::Expr(e) if matches!(&e.kind, ExprKind::Split { pieces, .. } if pieces.len() == 1)));
    }

    #[test]
    fn split_block_parses() {
        let d = defs("define:\n    f(x) -> y:\n        split r = x^2 at 0, 1\n        x, r < 0\n\n        2, 0 <= r <= 1\n        3, r > 1\n    g(x) -> y: x\n");
        assert_eq!(d.len(), 2);
        let DefBody::Expr(e) = &d[0].body else { panic!() };
        let ExprKind::Split { name, at, pieces, .. } = &e.kind else { panic!() };
        assert_eq!(name.as_ref().unwrap().key, "r");
        assert_eq!((at.len(), pieces.len()), (2, 3));
        assert!(matches!(pieces[1].cond.kind, ExprKind::Compare { .. }));
    }

    #[test]
    fn split_block_errors() {
        // Not a split, missing `at`, missing `, condition`, no lines: each reported, the next definition still parses.
        for (src, code) in [
            ("define:\n    f(x) -> y:\n        x^2\n    g(x) -> y: x\n", code::SYNTAX),
            ("define:\n    f(x) -> y:\n        split x\n        1, x < 0\n    g(x) -> y: x\n", code::SYNTAX),
            ("define:\n    f(x) -> y:\n        split x at 0\n        1 at x < 0\n    g(x) -> y: x\n", code::SYNTAX),
            ("define:\n    f(x) -> y:\n        split x at 0\n    g(x) -> y: x\n", code::SYNTAX),
        ] {
            let (s, d) = parse_program(src);
            assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), vec![code], "{src}");
            let Stmt::Define { defs, .. } = &s[0] else { panic!() };
            assert_eq!(defs.last().unwrap().name.key, "g", "{src}");
        }
    }

    #[test]
    fn family_body_parses() {
        let d = defs("define:\n    D(t; n) -> (x, y): family((k, t), 0 <= t <= k; 0 <= k <= n)\n    O(x, y; n): family(x^2 + y^2 = k^2; n >= k > 0)\n");
        let h = d[0].family.as_ref().unwrap();
        assert_eq!(h.index.key, "k");
        assert!(matches!(d[0].body, DefBody::WithRange { .. }));
        let h = d[1].family.as_ref().unwrap();
        assert!(matches!(d[1].body, DefBody::Equation { .. }));
        // Written from the top: lo = 0 (strict), hi = n.
        assert!(h.lo_strict && !h.hi_strict && matches!(h.lo.kind, ExprKind::Num { .. }));
    }

    #[test]
    fn family_body_errors() {
        assert_eq!(errs("define:\n    F(x) -> y: family(x + k)\n"), vec![code::BIG_OP_RANGE]);
        assert_eq!(errs("define:\n    F(x) -> y: family(x + k; 3)\n"), vec![code::BIG_OP_RANGE]);
        assert_eq!(errs("define:\n    F(x) -> y: family(x + k; 0 <= k <= 3\n"), vec![code::SYNTAX]);
    }

    #[test]
    fn stage4_definition_shapes() {
        let d = defs("define:\n    v(; a): [1, a]\n    A: [[1, 2],\n        [3, 4]]\n    S(; a): span(v, [0, 1])\n    p: arrow(v; [1, 2])\n");
        assert_eq!(d.len(), 4);
        let p = d[0].params.as_ref().unwrap();
        assert!(p.inputs.is_empty() && p.consts.len() == 1);
        assert!(d[1].params.is_none() && matches!(&d[1].body, DefBody::Expr(e) if matches!(e.kind, ExprKind::List { .. })));
        assert!(matches!(&d[3].body, DefBody::Expr(e) if matches!(&e.kind, ExprKind::Call { callee, args, .. } if callee.key == "arrow" && args.len() == 2)));
        // `v()` has neither inputs nor constants.
        assert_eq!(errs("define:\n    v(): [1, 2]\n"), vec![code::SYNTAX]);
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
    fn constant_block() {
        let s = ok("constant:\n    a: range=(-5, 5), default=1, step=0.1\n    n: integer, range=(1, 50)\n");
        let Stmt::Constant { entries, .. } = &s[0] else { panic!() };
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name.key, "a");
        assert!(entries[0].kind.is_none());
        assert_eq!(entries[0].props.iter().map(|p| p.key.as_str()).collect::<Vec<_>>(), ["range", "default", "step"]);
        assert!(matches!(entries[0].props[0].value.kind, ExprKind::Tuple { .. }));
        assert_eq!(entries[1].kind.as_ref().unwrap().0, "integer");
    }

    #[test]
    fn constant_block_errors() {
        assert_eq!(errs("constant:\n    a range=(1, 2)\n"), vec![code::SYNTAX]);
        assert_eq!(errs("constant: a: range=(1, 2)\n"), vec![code::SYNTAX]);
        // The kind word comes first; one bad entry does not hide the next.
        assert_eq!(errs("constant:\n    a: range=(1, 2), integer\n    b: default=+\n"), vec![code::SYNTAX, code::UNARY_PLUS, code::SYNTAX]);
    }

    #[test]
    fn style_block() {
        let s = ok("style up:\n    color: orange\n    fill: above\n    opacity: 0.25\n");
        let Stmt::Style { name, props, .. } = &s[0] else { panic!() };
        assert_eq!(name.key, "up");
        assert_eq!(props.iter().map(|p| p.key.as_str()).collect::<Vec<_>>(), ["color", "fill", "opacity"]);
        assert_eq!(errs("style s:\n    color orange\n"), vec![code::SYNTAX]);
        assert_eq!(errs("style s: color: orange\n"), vec![code::SYNTAX]);
    }

    #[test]
    fn assignment_and_highlight() {
        let s = ok("R1 = 0.9 < x < 1.1\nm = \"메모\"\nH1 = highlight(f; R1; blue; m)\nH2 = highlight(; between(f, g) and 0 <= x <= 2)\nH3 = highlight(f.(b*x).coef;; orange)\n");
        assert_eq!(s.len(), 5);
        assert!(matches!(&s[0], Stmt::Assign { name, value: AssignValue::Expr(e), .. } if name.key == "R1" && matches!(e.kind, ExprKind::Compare { .. })));
        assert!(matches!(&s[1], Stmt::Assign { value: AssignValue::Expr(e), .. } if matches!(&e.kind, ExprKind::Str { raw } if raw == "메모")));
        let Stmt::Assign { value: AssignValue::Highlight(h), .. } = &s[2] else { panic!() };
        assert_eq!(h.items.len(), 4);
        let Stmt::Assign { value: AssignValue::Highlight(h), .. } = &s[3] else { panic!() };
        assert!(h.items[0].is_none() && matches!(h.get(1).unwrap().kind, ExprKind::And { .. }));
        let Stmt::Assign { value: AssignValue::Highlight(h), .. } = &s[4] else { panic!() };
        assert_eq!(h.items.len(), 3);
        assert!(h.items[1].is_none());
        assert!(matches!(h.get(0).unwrap().kind, ExprKind::Address { .. }));
    }

    #[test]
    fn req_statement() {
        let s = ok("req(f; 0.9 < x < 1.1; \"끊겨 보임\")\nreq(g;; \"부호\")\nreq(h)\nreq(g.den; ; m)\n");
        assert_eq!(s.len(), 4);
        let Stmt::Req { slots, .. } = &s[2] else { panic!() };
        assert_eq!(slots.items.len(), 1);
        let Stmt::Req { slots, .. } = &s[3] else { panic!() };
        assert_eq!(slots.items.len(), 3);
        assert!(slots.items[1].is_none());
    }

    #[test]
    fn highlight_and_req_errors() {
        assert_eq!(errs("highlight(f;;)\n"), vec![code::HIGHLIGHT_NAME]);
        assert_eq!(errs("H1 = highlight(f, 0 < x;;)\n"), vec![code::SYNTAX]);
        assert_eq!(errs("R = req(f)\n"), vec![code::SYNTAX]);
        assert_eq!(errs("H1 = highlight(f; 0 < x\n"), vec![code::SYNTAX]);
        assert_eq!(errs("R1 = \n"), vec![code::SYNTAX]);
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
