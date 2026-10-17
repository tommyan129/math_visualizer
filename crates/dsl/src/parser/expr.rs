//! Expression parser (design.md "연산", "연산자 우선순위"). Recursive descent.
//!
//! Precedence, tightest first:
//!  1 name notation `f@2` `~f` `x_{1}` `x^[*]` `\bar{x}`   (part of the name)
//!  2 call `f(…)`, element `A_[i, j]`, address `.(…)` `.coef`
//!  3 `!`   4 `^` (right)   5 unary `-`   6 `*` `/`   7 `+` `-`
//!  8 comparison chain   9 `not`   10 `and`   11 `or`

use super::{PResult, Parser};
use crate::ast::*;
use crate::diag::{Diagnostic, code};
use crate::lexer::{Keyword, Tok};
use crate::names;
use crate::span::Span;

impl<'a> Parser<'a> {
    pub(crate) fn expr(&mut self) -> PResult<Expr> {
        self.nested(|p| p.or_expr())
    }

    fn or_expr(&mut self) -> PResult<Expr> {
        let first = self.and_expr()?;
        if !self.at(Tok::Kw(Keyword::Or)) {
            return Ok(first);
        }
        let mut args = vec![first];
        while self.eat(Tok::Kw(Keyword::Or)) {
            args.push(self.and_expr()?);
        }
        let span = args[0].span.to(args.last().unwrap().span);
        Ok(Expr::new(ExprKind::Or { args }, span))
    }

    fn and_expr(&mut self) -> PResult<Expr> {
        let first = self.not_expr()?;
        if !self.at(Tok::Kw(Keyword::And)) {
            return Ok(first);
        }
        let mut args = vec![first];
        while self.eat(Tok::Kw(Keyword::And)) {
            args.push(self.not_expr()?);
        }
        let span = args[0].span.to(args.last().unwrap().span);
        Ok(Expr::new(ExprKind::And { args }, span))
    }

    fn not_expr(&mut self) -> PResult<Expr> {
        if self.at(Tok::Kw(Keyword::Not)) {
            let start = self.bump().span;
            let arg = self.nested(|p| p.not_expr())?;
            let span = start.to(arg.span);
            return Ok(Expr::new(ExprKind::Not { arg: Box::new(arg) }, span));
        }
        self.compare()
    }

    fn cmp_op(&self) -> Option<CmpOp> {
        Some(match self.peek() {
            Tok::Lt => CmpOp::Lt,
            Tok::Le => CmpOp::Le,
            Tok::Gt => CmpOp::Gt,
            Tok::Ge => CmpOp::Ge,
            Tok::EqEq => CmpOp::Eq,
            _ => return None,
        })
    }

    fn compare(&mut self) -> PResult<Expr> {
        let first = self.additive()?;
        let mut rest = vec![];
        let mut op_spans = vec![];
        while let Some(op) = self.cmp_op() {
            op_spans.push(self.bump().span);
            rest.push(CompareLink { op, rhs: self.additive()? });
        }
        if rest.is_empty() {
            return Ok(first);
        }
        let span = first.span.to(rest.last().unwrap().rhs.span);
        if rest.len() > 1 {
            let dir = |op: CmpOp| match op {
                CmpOp::Lt | CmpOp::Le => 1,
                CmpOp::Gt | CmpOp::Ge => -1,
                CmpOp::Eq => 0,
            };
            if let Some(i) = rest.iter().position(|l| l.op == CmpOp::Eq) {
                self.diag(
                    Diagnostic::new(op_spans[i], code::COMPARE_EQ_CHAIN, "'=='는 이어 쓸 수 없음")
                        .suggest("a == b and b == c"),
                );
            } else if rest.iter().any(|l| dir(l.op) != dir(rest[0].op)) {
                let i = rest.iter().position(|l| dir(l.op) != dir(rest[0].op)).unwrap();
                self.diag(
                    Diagnostic::new(op_spans[i], code::COMPARE_MIXED, "비교 방향이 섞여 있음")
                        .suggest("같은 방향으로 (0 <= x < 5) 또는 and로 나누기"),
                );
            }
        }
        Ok(Expr::new(ExprKind::Compare { first: Box::new(first), rest }, span))
    }

    fn additive(&mut self) -> PResult<Expr> {
        let first = self.multiplicative()?;
        if !matches!(self.peek(), Tok::Plus | Tok::Minus) {
            return Ok(first);
        }
        let mut terms = vec![Term { id: 0, sign: Sign::Plus, expr: first }];
        loop {
            let sign = match self.peek() {
                Tok::Plus => Sign::Plus,
                Tok::Minus => Sign::Minus,
                _ => break,
            };
            self.bump();
            terms.push(Term { id: 0, sign, expr: self.multiplicative()? });
        }
        let span = terms[0].expr.span.to(terms.last().unwrap().expr.span);
        Ok(Expr::new(ExprKind::Sum { terms }, span))
    }

    /// Can this token begin an operand? Used to detect implicit multiplication.
    fn starts_operand(&self) -> bool {
        matches!(
            self.peek(),
            Tok::Int | Tok::Decimal | Tok::Ident | Tok::TexName | Tok::LParen | Tok::LBrace | Tok::LBracket | Tok::Tilde | Tok::Str
        )
    }

    fn multiplicative(&mut self) -> PResult<Expr> {
        let mut cur = self.unary()?;
        // True while `cur` is a Product built by this loop (a Paren never is).
        let mut open_product = false;
        loop {
            let rhs = match self.peek() {
                Tok::Star => {
                    self.bump();
                    self.unary()?
                }
                Tok::Slash => {
                    self.bump();
                    let den = self.unary()?;
                    let span = cur.span.to(den.span);
                    cur = Expr::new(ExprKind::Div { num: Box::new(cur), den: Box::new(den) }, span);
                    open_product = false;
                    continue;
                }
                // Already reported by the lexer (`a | b`); keep going as if it were `*`.
                Tok::Invalid => {
                    self.bump();
                    if !self.starts_operand() && !matches!(self.peek(), Tok::Minus | Tok::Invalid) {
                        continue;
                    }
                    self.unary()?
                }
                _ if self.starts_operand() => {
                    let rhs = self.unary()?;
                    let l = &self.src[cur.span.range()];
                    let r = &self.src[rhs.span.range()];
                    self.diag(
                        Diagnostic::new(cur.span.to(rhs.span), code::IMPLICIT_MUL, "곱셈 기호 '*'가 빠짐")
                            .suggest(format!("{l}*{r}")),
                    );
                    rhs
                }
                _ => break,
            };
            let span = cur.span.to(rhs.span);
            if open_product {
                if let ExprKind::Product { factors } = &mut cur.kind {
                    factors.push(rhs);
                }
                cur.span = span;
            } else {
                cur = Expr::new(ExprKind::Product { factors: vec![cur, rhs] }, span);
                open_product = true;
            }
        }
        Ok(cur)
    }

    fn unary(&mut self) -> PResult<Expr> {
        match self.peek() {
            Tok::Minus => {
                let start = self.bump().span;
                let arg = self.nested(|p| p.unary())?;
                let span = start.to(arg.span);
                Ok(Expr::new(ExprKind::Neg { arg: Box::new(arg) }, span))
            }
            Tok::Plus => {
                let span = self.bump().span;
                self.diag(Diagnostic::new(span, code::UNARY_PLUS, "단항 '+'는 없음").suggest("'+' 빼기"));
                self.unary()
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> PResult<Expr> {
        let base = self.factorial()?;
        if !self.at(Tok::Caret) {
            return Ok(base);
        }
        self.bump();
        let exp = self.exponent()?;
        // `sin^2(x)`, `f^2(x)`: a bare function name followed by `^`.
        if matches!(base.kind, ExprKind::Name { .. }) && self.at(Tok::LParen) {
            let ExprKind::Name { name, .. } = &base.kind else { unreachable!() };
            let lparen = self.peek_span();
            let (args, end) = self.call_args()?;
            let call_span = base.span.to(end);
            let args_text = &self.src[lparen.start as usize..end.end as usize];
            let exp_text = &self.src[exp.span.range()];
            self.diag(
                Diagnostic::new(base.span.to(end), code::FUNC_POW, "함수 이름 바로 뒤에 '^'를 쓸 수 없음")
                    .suggest(format!("{}{args_text}^{exp_text}", name.key)),
            );
            let call = Expr::new(ExprKind::Call { callee: name.clone(), kind: NameKind::Unresolved, args }, call_span);
            let span = call.span.to(exp.span);
            return Ok(Expr::new(ExprKind::Pow { base: Box::new(call), exp: Box::new(exp) }, span));
        }
        let span = base.span.to(exp.span);
        Ok(Expr::new(ExprKind::Pow { base: Box::new(base), exp: Box::new(exp) }, span))
    }

    /// Right side of `^`: one number, one name, or a bracketed group; may chain (`2^3^2`).
    fn exponent(&mut self) -> PResult<Expr> {
        let atom = match self.peek() {
            Tok::Minus | Tok::Plus => {
                let start = self.peek_span();
                let e = self.unary()?;
                let text = &self.src[e.span.range()];
                self.diag(
                    Diagnostic::new(start.to(e.span), code::POW_RHS, "'^' 오른쪽에 부호가 오면 괄호가 필요함")
                        .suggest(format!("^({text})")),
                );
                return Ok(e);
            }
            Tok::Int | Tok::Decimal => self.number(),
            Tok::Ident | Tok::TexName | Tok::Tilde => {
                let name = self.name()?;
                if self.at(Tok::LParen) {
                    let (args, end) = self.call_args()?;
                    let span = name.1.to(end);
                    let text = &self.src[span.range()];
                    self.diag(
                        Diagnostic::new(span, code::POW_RHS, "'^' 오른쪽에 함수 호출은 괄호가 필요함").suggest(format!("^({text})")),
                    );
                    Expr::new(ExprKind::Call { callee: name.0, kind: NameKind::Unresolved, args }, span)
                } else {
                    Expr::new(ExprKind::Name { name: name.0, kind: NameKind::Unresolved }, name.1)
                }
            }
            Tok::LParen | Tok::LBrace => self.primary()?,
            Tok::Invalid => {
                let span = self.bump().span;
                Expr::new(ExprKind::Error {}, span)
            }
            _ => return self.unexpected("'^' 오른쪽에 숫자, 이름, 괄호 묶음"),
        };
        let atom = if self.at(Tok::Caret) {
            self.bump();
            let rhs = self.nested(|p| p.exponent())?;
            let span = atom.span.to(rhs.span);
            Expr::new(ExprKind::Pow { base: Box::new(atom), exp: Box::new(rhs) }, span)
        } else {
            atom
        };
        if self.at(Tok::Bang) {
            let bang = self.bump().span;
            let text = &self.src[atom.span.range()];
            self.diag(
                Diagnostic::new(bang, code::POW_RHS, "'^' 오른쪽의 '!'는 읽는 방법이 둘로 갈림")
                    .suggest(format!("^({text}!) 또는 (밑^{text})!")),
            );
            let span = atom.span.to(bang);
            return Ok(Expr::new(ExprKind::Factorial { arg: Box::new(atom) }, span));
        }
        Ok(atom)
    }

    fn factorial(&mut self) -> PResult<Expr> {
        let mut e = self.postfix()?;
        while self.at(Tok::Bang) {
            let end = self.bump().span;
            let span = e.span.to(end);
            e = Expr::new(ExprKind::Factorial { arg: Box::new(e) }, span);
        }
        Ok(e)
    }

    fn postfix(&mut self) -> PResult<Expr> {
        let mut e = self.primary()?;
        loop {
            match (self.peek(), self.peek_at(1)) {
                (Tok::Underscore, Tok::LBracket) => {
                    self.bump();
                    self.bump();
                    let (indices, end) = self.list_until(Tok::RBracket, "']'")?;
                    let span = e.span.to(end);
                    e = Expr::new(ExprKind::Index { base: Box::new(e), indices }, span);
                }
                (Tok::Dot, Tok::LParen) => {
                    self.bump();
                    self.bump();
                    let pattern = self.expr()?;
                    let end = self.expect(Tok::RParen, "')'")?;
                    let span = e.span.to(end);
                    e = Expr::new(ExprKind::Address { base: Box::new(e), step: AddressStep::Term { pattern: Box::new(pattern) } }, span);
                }
                (Tok::Dot, Tok::Ident) => {
                    self.bump();
                    let t = self.bump();
                    let part = self.src[t.span.range()].to_string();
                    let span = e.span.to(t.span);
                    e = Expr::new(ExprKind::Address { base: Box::new(e), step: AddressStep::Part { part } }, span);
                }
                (Tok::Dot, _) => {
                    return self.error_here(code::SYNTAX, "'.' 뒤에는 '(항)' 또는 .coef 같은 부분 이름이 와야 함", Some("f.(2*x) 또는 f.den"));
                }
                _ => break,
            }
        }
        Ok(e)
    }

    pub(super) fn primary(&mut self) -> PResult<Expr> {
        match self.peek() {
            Tok::Int | Tok::Decimal => Ok(self.number()),
            Tok::Ident if names::BIG_OPS.contains(&self.peek_text()) && self.peek_at(1) == Tok::LParen => self.big_op(),
            Tok::Ident if self.peek_text() == "arrow" && self.peek_at(1) == Tok::LParen => self.arrow(),
            Tok::Ident if (names::feature(self.peek_text()).is_some() || self.peek_text() == names::CALC) && self.peek_at(1) == Tok::LParen => self.feature_call(),
            Tok::Ident | Tok::TexName | Tok::Tilde => {
                let (name, span) = self.name()?;
                if self.at(Tok::LParen) {
                    let (args, end) = self.call_args()?;
                    let span = span.to(end);
                    return Ok(Expr::new(ExprKind::Call { callee: name, kind: NameKind::Unresolved, args }, span));
                }
                Ok(Expr::new(ExprKind::Name { name, kind: NameKind::Unresolved }, span))
            }
            Tok::LParen => {
                let start = self.bump().span;
                let first = self.expr()?;
                if self.at(Tok::Comma) {
                    let mut items = vec![first];
                    while self.eat(Tok::Comma) {
                        items.push(self.expr()?);
                    }
                    let end = self.expect(Tok::RParen, "')'")?;
                    return Ok(Expr::new(ExprKind::Tuple { items }, start.to(end)));
                }
                let end = self.expect(Tok::RParen, "')'")?;
                Ok(Expr::new(ExprKind::Paren { inner: Box::new(first) }, start.to(end)))
            }
            // `{}` groups without creating a node.
            Tok::LBrace => {
                self.bump();
                let inner = self.expr()?;
                self.expect(Tok::RBrace, "'}'")?;
                Ok(inner)
            }
            Tok::LBracket => {
                let start = self.bump().span;
                let (items, end) = self.list_until(Tok::RBracket, "']'")?;
                Ok(Expr::new(ExprKind::List { items }, start.to(end)))
            }
            Tok::Str => {
                let t = self.bump();
                let text = &self.src[t.span.range()];
                let inner = &text[1..];
                // An unterminated string was already reported by the lexer.
                let raw = inner.strip_suffix('"').unwrap_or(inner);
                Ok(Expr::new(ExprKind::Str { raw: raw.to_string() }, t.span))
            }
            Tok::Invalid => {
                let span = self.bump().span;
                Ok(Expr::new(ExprKind::Error {}, span))
            }
            _ => self.unexpected("식"),
        }
    }

    fn number(&mut self) -> Expr {
        let t = self.bump();
        let text = self.src[t.span.range()].to_string();
        Expr::new(ExprKind::Num { text, int: t.tok == Tok::Int }, t.span)
    }

    /// `( args )` after a callee; returns args and the closing span.
    fn call_args(&mut self) -> PResult<(Vec<Expr>, Span)> {
        self.expect(Tok::LParen, "'('")?;
        self.list_until(Tok::RParen, "')'")
    }

    /// Comma-separated expressions up to `close` (already past the opener).
    fn list_until(&mut self, close: Tok, what: &str) -> PResult<(Vec<Expr>, Span)> {
        let mut items = vec![];
        if !self.at(close) {
            loop {
                items.push(self.expr()?);
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
        }
        if self.at(Tok::Semi) {
            return self.error_here(code::SYNTAX, "여기서는 ';' 칸 구분을 쓸 수 없음", Some(","));
        }
        let end = self.expect(close, what)?;
        Ok((items, end))
    }

    /// `arrow(v)` / `arrow(v; P)`: the start point goes in its own `;` slot. A Call node
    /// with one or two arguments.
    fn arrow(&mut self) -> PResult<Expr> {
        let (callee, head) = self.name()?;
        self.expect(Tok::LParen, "'('")?;
        let mut args = vec![self.expr()?];
        if self.at(Tok::Comma) {
            let v = self.src[args[0].span.range()].to_string();
            return self.error_here(code::SYNTAX, "arrow의 시작점은 ';' 뒤에 씀", Some(&format!("arrow({v}; P)")));
        }
        if self.eat(Tok::Semi) {
            args.push(self.expr()?);
        }
        let end = self.expect(Tok::RParen, "')'")?;
        Ok(Expr::new(ExprKind::Call { callee, kind: NameKind::Unresolved, args }, head.to(end)))
    }

    /// `extrema(f)`, `roots(f; -1 < x < 1)`, `extrema(f;; max)`, `intersect(f, y = x)`:
    /// targets; range; filter word. Empty slots allowed; at most three. `calc(f, g, h;
    /// x > 0; integral)` (stage 5c) uses the same slots: steps; range; kind word.
    fn feature_call(&mut self) -> PResult<Expr> {
        let (callee, head) = self.name()?;
        self.expect(Tok::LParen, "'('")?;
        let mut args = vec![];
        if !matches!(self.peek(), Tok::Semi | Tok::RParen) {
            loop {
                let e = self.expr()?;
                // `y = x`: the other curve written as an equation of the target's axes.
                let e = if self.eat(Tok::Eq) {
                    let rhs = self.expr()?;
                    let span = e.span.to(rhs.span);
                    Expr::new(ExprKind::Compare { first: Box::new(e), rest: vec![CompareLink { op: CmpOp::Eq, rhs }] }, span)
                } else {
                    e
                };
                args.push(e);
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
        }
        let mut range = None;
        let mut filter = None;
        if self.eat(Tok::Semi) {
            if !matches!(self.peek(), Tok::Semi | Tok::RParen) {
                range = Some(Box::new(self.expr()?));
            }
            if self.eat(Tok::Semi) && !self.at(Tok::RParen) {
                filter = Some(self.name()?.0.key);
            }
        }
        let calc = callee.key == names::CALC;
        if self.at(Tok::Semi) {
            if calc {
                return self.error_here(code::SYNTAX, "calc 칸은 세 개까지 (단계; 범위; 종류)", Some("calc(f, g; x > 0; integral)"));
            }
            let form = format!("{}(f; -1 < x < 1; 종류)", callee.key);
            return self.error_here(code::SYNTAX, "특징점 칸은 세 개까지 (대상; 범위; 종류)", Some(&form));
        }
        let end = self.expect(Tok::RParen, "')'")?;
        if calc {
            return Ok(Expr::new(ExprKind::Calc { steps: args, range, kind: filter }, head.to(end)));
        }
        Ok(Expr::new(ExprKind::Feature { callee, args, range, filter }, head.to(end)))
    }

    /// `sum(body; lo <= k <= hi)` / `prod(...)`.
    fn big_op(&mut self) -> PResult<Expr> {
        let head = self.bump();
        let op = if &self.src[head.span.range()] == "sum" { BigOpKind::Sum } else { BigOpKind::Prod };
        let form = format!("{}(식; 1 <= k <= n)", &self.src[head.span.range()]);
        self.expect(Tok::LParen, "'('")?;
        let body = self.expr()?;
        if !self.eat(Tok::Semi) {
            return self.error_here(code::BIG_OP_RANGE, "범위 칸이 없음 (식 뒤에 ';')", Some(&form));
        }
        let range = self.expr()?;
        let end = self.expect(Tok::RParen, "')'")?;
        let span = head.span.to(end);
        match self.index_range(&range, &form) {
            Some((index, lo, lo_strict, hi, hi_strict)) => {
                Ok(Expr::new(ExprKind::BigOp { op, body: Box::new(body), index, lo: Box::new(lo), lo_strict, hi: Box::new(hi), hi_strict }, span))
            }
            None => Ok(Expr::new(ExprKind::Error {}, span)),
        }
    }

    /// `lo <= k <= hi` (or from the top) of `sum` / `prod` / `each`: (index, lo, lo strict,
    /// hi, hi strict). Reports and returns None for any other shape.
    pub(super) fn index_range(&mut self, range: &Expr, form: &str) -> Option<(Ident, Expr, bool, Expr, bool)> {
        let parsed = (|| {
            let ExprKind::Compare { first, rest } = range.kind.clone() else { return None };
            if rest.len() != 2 || rest.iter().any(|l| l.op == CmpOp::Eq) || (matches!(rest[0].op, CmpOp::Lt | CmpOp::Le) != matches!(rest[1].op, CmpOp::Lt | CmpOp::Le)) {
                return None;
            }
            let [l0, l1] = <[CompareLink; 2]>::try_from(rest).unwrap();
            let ExprKind::Name { name: index, .. } = l0.rhs.kind else { return None };
            Some(if matches!(l0.op, CmpOp::Lt | CmpOp::Le) {
                (index, *first, l0.op == CmpOp::Lt, l1.rhs, l1.op == CmpOp::Lt)
            } else {
                (index, l1.rhs, l1.op == CmpOp::Gt, *first, l0.op == CmpOp::Gt)
            })
        })();
        if parsed.is_none() {
            self.diag(Diagnostic::new(range.span, code::BIG_OP_RANGE, "범위는 '하한 <= 인덱스 <= 상한' 모양이어야 함").suggest(form.to_string()));
        }
        parsed
    }

    /// A name with its notation parts: `~f`, `\bar{x}`, `x_{1}`, `x^[*]`, `f@2`.
    pub(super) fn name(&mut self) -> PResult<(Ident, Span)> {
        let start = self.peek_span();
        let inverse = self.eat(Tok::Tilde);
        let (base, accent) = self.name_base()?;
        let mut id = Ident { key: String::new(), base, accent, sub: None, sup: None, deriv: 0, inverse };
        let mut end = self.prev_span();

        if self.at(Tok::Underscore) && self.peek_at(1) != Tok::LBracket {
            let us = self.bump().span;
            if self.eat(Tok::LBrace) {
                let mut sub = String::new();
                while matches!(self.peek(), Tok::Ident | Tok::Int | Tok::TexName) {
                    let t = self.bump();
                    let text = &self.src[t.span.range()];
                    if t.tok == Tok::TexName {
                        self.check_tex_name(&text[1..], t.span);
                        sub.push_str(text);
                        // `x_{\alpha i}` needs a separator after a command name.
                        if matches!(self.peek(), Tok::Ident) {
                            sub.push(' ');
                        }
                    } else {
                        sub.push_str(text);
                    }
                }
                if sub.is_empty() {
                    return self.error_here(code::SUBSCRIPT, "아래첨자 안에는 이름이나 숫자가 와야 함", Some("x_{1}"));
                }
                end = self.expect(Tok::RBrace, "'}'")?;
                id.sub = Some(sub);
            } else {
                // `x_1`: braces are required.
                let shown = if matches!(self.peek(), Tok::Ident | Tok::Int | Tok::TexName) {
                    let t = self.bump();
                    end = t.span;
                    let s = self.src[t.span.range()].to_string();
                    id.sub = Some(s.clone());
                    s
                } else {
                    "1".to_string()
                };
                self.diag(
                    Diagnostic::new(us.to(end), code::SUBSCRIPT, "아래첨자는 중괄호가 필요함 ('_'는 이름 글자로 못 씀)")
                        .suggest(format!("{}_{{{shown}}}", id.base)),
                );
            }
        }

        if self.at(Tok::Caret) && self.peek_at(1) == Tok::LBracket {
            self.bump();
            let open = self.bump().span;
            let mut depth = 0;
            while !(self.at(Tok::RBracket) && depth == 0) {
                match self.peek() {
                    Tok::LParen => depth += 1,
                    Tok::RParen => depth -= 1,
                    Tok::Newline | Tok::Eof => return self.error_here(code::SYNTAX, "윗첨자 이름이 닫히지 않음", Some("x^[*]")),
                    _ => {}
                }
                self.bump();
            }
            let close = self.bump().span;
            let text = self.src[open.end as usize..close.start as usize].trim().to_string();
            if text.is_empty() {
                self.diag(Diagnostic::new(open.to(close), code::SYNTAX, "윗첨자 이름이 비어 있음").suggest("x^[*]"));
            }
            id.sup = Some(text);
            end = close;
        }

        if self.at(Tok::At) {
            self.bump();
            if self.at(Tok::Int) {
                let t = self.bump();
                id.deriv = self.src[t.span.range()].parse().unwrap_or(u32::MAX);
                end = t.span;
                if id.deriv == 0 {
                    self.diag(Diagnostic::new(t.span, code::SYNTAX, "미분 횟수는 1 이상").suggest(format!("{}@1", id.base)));
                    id.deriv = 1;
                }
            } else {
                return self.error_here(code::SYNTAX, "'@' 뒤에는 미분 횟수가 와야 함", Some("f@2"));
            }
        }

        id.rekey();
        Ok((id, start.to(end)))
    }

    fn name_base(&mut self) -> PResult<(String, Option<String>)> {
        match self.peek() {
            Tok::Ident => {
                let t = self.bump();
                Ok((self.src[t.span.range()].to_string(), None))
            }
            Tok::TexName => {
                let t = self.bump();
                let cmd = &self.src[t.span.start as usize + 1..t.span.end as usize];
                if names::ACCENTS.contains(&cmd) {
                    let accent = cmd.to_string();
                    let form = format!("\\{accent}{{x}}");
                    if !self.eat(Tok::LBrace) {
                        return self.error_here(code::TEX, "장식 뒤에는 {글자}가 와야 함", Some(&form));
                    }
                    let base = match self.peek() {
                        Tok::Ident => {
                            let b = self.bump();
                            self.src[b.span.range()].to_string()
                        }
                        Tok::TexName => {
                            let b = self.bump();
                            let text = self.src[b.span.range()].to_string();
                            self.check_tex_name(&text[1..], b.span);
                            text
                        }
                        _ => return self.error_here(code::TEX, "장식 안에는 글자 하나가 와야 함", Some(&form)),
                    };
                    self.expect(Tok::RBrace, "'}'")?;
                    return Ok((base, Some(accent)));
                }
                self.check_tex_name(cmd, t.span);
                Ok((format!("\\{cmd}"), None))
            }
            _ => self.unexpected("이름"),
        }
    }

    /// Reports `\cmd` that is neither a Greek letter nor a reserved word.
    fn check_tex_name(&mut self, cmd: &str, span: Span) {
        if names::GREEK.contains(&cmd) || names::RESERVED.contains(&cmd) {
            return;
        }
        let d = match names::tex_command_suggestion(cmd) {
            Some(s) => Diagnostic::new(span, code::TEX, format!("LaTeX 명령 '\\{cmd}'는 받지 않음")).suggest(s),
            None => Diagnostic::new(span, code::TEX, format!("모르는 LaTeX 이름 '\\{cmd}'")),
        };
        self.diag(d);
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse_expr;
    use crate::ast::*;
    use crate::diag::code;

    /// Compact s-expression rendering for structural assertions.
    fn sx(e: &Expr) -> String {
        match &e.kind {
            ExprKind::Num { text, .. } => text.clone(),
            ExprKind::Name { name, .. } => name.key.clone(),
            ExprKind::Sum { terms } => format!(
                "(+ {})",
                terms.iter().map(|t| format!("{}{}", if t.sign == Sign::Minus { "-" } else { "" }, sx(&t.expr))).collect::<Vec<_>>().join(" ")
            ),
            ExprKind::Product { factors } => format!("(* {})", factors.iter().map(sx).collect::<Vec<_>>().join(" ")),
            ExprKind::Div { num, den } => format!("(/ {} {})", sx(num), sx(den)),
            ExprKind::Pow { base, exp } => format!("(^ {} {})", sx(base), sx(exp)),
            ExprKind::Neg { arg } => format!("(neg {})", sx(arg)),
            ExprKind::Factorial { arg } => format!("(! {})", sx(arg)),
            ExprKind::Paren { inner } => format!("[{}]", sx(inner)),
            ExprKind::Call { callee, args, .. } => format!("({} {})", callee.key, args.iter().map(sx).collect::<Vec<_>>().join(" ")),
            ExprKind::BigOp { op, body, index, lo, lo_strict, hi, hi_strict } => format!(
                "({:?} {} {} {}{} {}{})",
                op,
                sx(body),
                index.key,
                if *lo_strict { "<" } else { "<=" },
                sx(lo),
                if *hi_strict { "<" } else { "<=" },
                sx(hi)
            ),
            ExprKind::Compare { first, rest } => {
                let mut s = format!("(cmp {}", sx(first));
                for l in rest {
                    s.push_str(&format!(" {:?} {}", l.op, sx(&l.rhs)));
                }
                s + ")"
            }
            ExprKind::Not { arg } => format!("(not {})", sx(arg)),
            ExprKind::And { args } => format!("(and {})", args.iter().map(sx).collect::<Vec<_>>().join(" ")),
            ExprKind::Or { args } => format!("(or {})", args.iter().map(sx).collect::<Vec<_>>().join(" ")),
            ExprKind::Str { raw } => format!("{raw:?}"),
            ExprKind::Tuple { items } => format!("(tuple {})", items.iter().map(sx).collect::<Vec<_>>().join(" ")),
            ExprKind::List { items } => format!("(list {})", items.iter().map(sx).collect::<Vec<_>>().join(" ")),
            ExprKind::Index { base, indices } => format!("(idx {} {})", sx(base), indices.iter().map(sx).collect::<Vec<_>>().join(" ")),
            ExprKind::Address { base, step } => match step {
                AddressStep::Term { pattern } => format!("(addr {} {})", sx(base), sx(pattern)),
                AddressStep::Part { part } => format!("(addr {} .{part})", sx(base)),
            },
            ExprKind::Split { subject, at, pieces, .. } => format!(
                "(split {} at {}{})",
                sx(subject),
                at.iter().map(sx).collect::<Vec<_>>().join(" "),
                pieces.iter().map(|p| format!(" [{} {}]", sx(&p.value), sx(&p.cond))).collect::<String>()
            ),
            ExprKind::Feature { callee, args, range, filter } => format!(
                "({} {}; {}; {})",
                callee.key,
                args.iter().map(sx).collect::<Vec<_>>().join(" "),
                range.as_ref().map_or(String::new(), |r| sx(r)),
                filter.as_deref().unwrap_or("")
            ),
            ExprKind::Calc { steps, range, kind } => format!(
                "(calc {}; {}; {})",
                steps.iter().map(sx).collect::<Vec<_>>().join(" "),
                range.as_ref().map_or(String::new(), |r| sx(r)),
                kind.as_deref().unwrap_or("")
            ),
            ExprKind::Error {} => "ERR".into(),
        }
    }

    fn ok(src: &str) -> String {
        let (e, d) = parse_expr(src);
        assert!(d.is_empty(), "{src}: unexpected diagnostics {d:?}");
        sx(&e.unwrap())
    }

    fn errs(src: &str) -> Vec<&'static str> {
        parse_expr(src).1.into_iter().map(|d| d.code).collect()
    }

    fn suggestion(src: &str) -> String {
        parse_expr(src).1[0].suggestion.clone().unwrap_or_default()
    }

    #[test]
    fn sum_is_nary_with_signs() {
        assert_eq!(ok("a*x^2 + b*x + 1"), "(+ (* a (^ x 2)) (* b x) 1)");
        assert_eq!(ok("a - b*x"), "(+ a -(* b x))");
    }

    #[test]
    fn product_and_division() {
        assert_eq!(ok("a*b*c"), "(* a b c)");
        assert_eq!(ok("a/b*c"), "(* (/ a b) c)");
        assert_eq!(ok("a*b/c"), "(/ (* a b) c)");
        assert_eq!(ok("a/b/c"), "(/ (/ a b) c)");
    }

    #[test]
    fn unary_minus_and_power() {
        assert_eq!(ok("-x^2"), "(neg (^ x 2))");
        assert_eq!(ok("-2^2"), "(neg (^ 2 2))");
        assert_eq!(ok("2^3^2"), "(^ 2 (^ 3 2))");
        assert_eq!(ok("-a*b"), "(* (neg a) b)");
        assert_eq!(ok("a*-b"), "(* a (neg b))");
    }

    #[test]
    fn power_right_side_forms() {
        assert_eq!(ok("x^n"), "(^ x n)");
        assert_eq!(ok("x^(n+1)"), "(^ x [(+ n 1)])");
        assert_eq!(ok("x^{1/3}"), "(^ x (/ 1 3))");
        assert_eq!(ok("x^2.5"), "(^ x 2.5)");
        assert_eq!(ok("x^\\pi"), "(^ x \\pi)");
    }

    #[test]
    fn power_right_side_errors() {
        assert_eq!(errs("2^-1"), vec![code::POW_RHS]);
        assert_eq!(suggestion("2^-1"), "^(-1)");
        assert_eq!(errs("2^f(x)"), vec![code::POW_RHS]);
        assert_eq!(suggestion("2^f(x)"), "^(f(x))");
        assert_eq!(errs("x^n!"), vec![code::POW_RHS]);
    }

    #[test]
    fn factorial_binds_tighter_than_power() {
        assert_eq!(ok("n!"), "(! n)");
        assert_eq!(ok("2*n!"), "(* 2 (! n))");
        assert_eq!(ok("n!^2"), "(^ (! n) 2)");
    }

    #[test]
    fn function_name_followed_by_caret() {
        assert_eq!(errs("sin^2(x)"), vec![code::FUNC_POW]);
        assert_eq!(suggestion("sin^2(x)"), "sin(x)^2");
        assert_eq!(errs("f^{2}(x + 1)"), vec![code::FUNC_POW]);
    }

    #[test]
    fn implicit_multiplication() {
        assert_eq!(errs("2x"), vec![code::IMPLICIT_MUL]);
        assert_eq!(suggestion("2x"), "2*x");
        assert_eq!(suggestion("2(x + 1)"), "2*(x + 1)");
        assert_eq!(errs("(a)(b)"), vec![code::IMPLICIT_MUL]);
        assert_eq!(errs("x y"), vec![code::IMPLICIT_MUL]);
    }

    #[test]
    fn unary_plus_rejected() {
        assert_eq!(errs("+x"), vec![code::UNARY_PLUS]);
    }

    #[test]
    fn parens_and_braces() {
        assert_eq!(ok("(a + b)*c"), "(* [(+ a b)] c)");
        // `{}` creates no node.
        assert_eq!(ok("{a + b}*c"), "(* (+ a b) c)");
        assert_eq!(ok("((a))"), "[[a]]");
    }

    #[test]
    fn calls_and_builtins() {
        assert_eq!(ok("sin(x)"), "(sin x)");
        assert_eq!(ok("log(x, 2)"), "(log x 2)");
        assert_eq!(ok("g(f(x))"), "(g (f x))");
        assert_eq!(ok("sin(x)^2"), "(^ (sin x) 2)");
    }

    #[test]
    fn comparisons() {
        assert_eq!(ok("0 <= x < 5"), "(cmp 0 Le x Lt 5)");
        assert_eq!(ok("5 > x >= 0"), "(cmp 5 Gt x Ge 0)");
        assert_eq!(ok("x == 1"), "(cmp x Eq 1)");
        assert_eq!(errs("a < b > c"), vec![code::COMPARE_MIXED]);
        assert_eq!(errs("a == b == c"), vec![code::COMPARE_EQ_CHAIN]);
    }

    #[test]
    fn logic_precedence() {
        assert_eq!(ok("not x == 1"), "(not (cmp x Eq 1))");
        assert_eq!(ok("a < 1 or b < 2 and c < 3"), "(or (cmp a Lt 1) (and (cmp b Lt 2) (cmp c Lt 3)))");
    }

    #[test]
    fn name_notation() {
        assert_eq!(ok("x_{1}"), "x_{1}");
        assert_eq!(ok("x_{\\alpha}"), "x_{\\alpha}");
        assert_eq!(ok("\\bar{x}"), "\\bar{x}");
        assert_eq!(ok("\\hat{\\theta}"), "\\hat{\\theta}");
        assert_eq!(ok("x^[*]"), "x^[*]");
        assert_eq!(ok("x^[(k)]"), "x^[(k)]");
        assert_eq!(ok("f@2(x)"), "(f@2 x)");
        assert_eq!(ok("~f(x)"), "(~f x)");
        assert_eq!(ok("x^[*]^2"), "(^ x^[*] 2)");
        assert_eq!(ok("~g(f@1(x))"), "(~g (f@1 x))");
    }

    #[test]
    fn feature_calls() {
        assert_eq!(ok("extrema(f)"), "(extrema f; ; )");
        assert_eq!(ok("roots(f; -1 < x < 1)"), "(roots f; (cmp (neg 1) Lt x Lt 1); )");
        assert_eq!(ok("extrema(f;; max)"), "(extrema f; ; max)");
        assert_eq!(ok("breaks(g; x > 0; hole)"), "(breaks g; (cmp x Gt 0); hole)");
        assert_eq!(ok("intersect(f, y = x)"), "(intersect f (cmp y Eq x); ; )");
        assert_eq!(ok("intersect(f, g; 0 < x < 2)"), "(intersect f g; (cmp 0 Lt x Lt 2); )");
        // Other names keep the ordinary call syntax.
        assert_eq!(ok("extremum(f)"), "(extremum f)");
    }

    #[test]
    fn feature_call_errors() {
        assert_eq!(errs("extrema(f; x > 0; max; 1)"), vec![code::SYNTAX]);
        assert_eq!(errs("roots(f x)"), vec![code::IMPLICIT_MUL]);
        assert_eq!(errs("roots(f;"), vec![code::SYNTAX]);
    }

    #[test]
    fn calc_calls() {
        assert_eq!(ok("calc(f, g, h)"), "(calc f g h; ; )");
        assert_eq!(ok("calc(f, F;; integral)"), "(calc f F; ; integral)");
        assert_eq!(ok("calc(f, g; x > 0)"), "(calc f g; (cmp x Gt 0); )");
        assert_eq!(ok("calc(f, F; 0 < x < 3; integral)"), "(calc f F; (cmp 0 Lt x Lt 3); integral)");
        assert_eq!(ok("calc(f@1, g)"), "(calc f@1 g; ; )");
    }

    #[test]
    fn calc_call_errors() {
        assert_eq!(errs("calc(f, g; x > 0; integral; 1)"), vec![code::SYNTAX]);
        assert_eq!(suggestion("calc(f, g; x > 0; integral; 1)"), "calc(f, g; x > 0; integral)");
        assert_eq!(errs("calc(f, g"), vec![code::SYNTAX]);
    }

    #[test]
    fn derivative_name_errors() {
        assert_eq!(errs("f@0(x)"), vec![code::SYNTAX]);
        assert_eq!(errs("f@(x)"), vec![code::SYNTAX]);
    }

    #[test]
    fn subscript_needs_braces() {
        assert_eq!(errs("x_1"), vec![code::SUBSCRIPT]);
        assert_eq!(suggestion("x_1"), "x_{1}");
        assert_eq!(errs("my_var"), vec![code::SUBSCRIPT]);
    }

    #[test]
    fn tex_names() {
        assert_eq!(ok("\\theta + \\pi"), "(+ \\theta \\pi)");
        assert_eq!(errs("\\frac"), vec![code::TEX]);
        assert_eq!(suggestion("\\cdot"), "*");
        assert_eq!(errs("\\foo"), vec![code::TEX]);
    }

    #[test]
    fn later_stage_forms_parse() {
        assert_eq!(ok("A_[1, 2]"), "(idx A 1 2)");
        assert_eq!(ok("[[1, 2], [3, 4]]"), "(list (list 1 2) (list 3 4))");
        assert_eq!(ok("f.(2*x).coef"), "(addr (addr f (* 2 x)) .coef)");
        assert_eq!(ok("(cos(t), sin(t))"), "(tuple (cos t) (sin t))");
        assert_eq!(ok("\"memo\""), "\"memo\"");
    }

    #[test]
    fn matrices_vectors_and_arrow() {
        assert_eq!(ok("[[1, 2],\n [3, 4]]"), "(list (list 1 2) (list 3 4))");
        assert_eq!(ok("matmul(R(t), [1, 0])_[2]"), "(idx (matmul (R t) (list 1 0)) 2)");
        assert_eq!(ok("2*A - B"), "(+ (* 2 A) -B)");
        assert_eq!(ok("arrow(v)"), "(arrow v)");
        assert_eq!(ok("arrow([1, f(c)]; [c, f(c)])"), "(arrow (list 1 (f c)) (list c (f c)))");
        assert_eq!(ok("span(v, w)"), "(span v w)");
    }

    #[test]
    fn arrow_errors() {
        assert_eq!(errs("arrow(v, P)"), vec![code::SYNTAX]);
        assert_eq!(suggestion("arrow(v, P)"), "arrow(v; P)");
        assert_eq!(errs("arrow(v; P; Q)"), vec![code::SYNTAX]);
        assert_eq!(errs("arrow()"), vec![code::SYNTAX]);
    }

    #[test]
    fn big_ops() {
        assert_eq!(ok("sum(k^2; 1 <= k <= n)"), "(Sum (^ k 2) k <=1 <=n)");
        assert_eq!(ok("prod(k; 1 < k <= n)"), "(Prod k k <1 <=n)");
        // Descending form is normalized.
        assert_eq!(ok("sum(k; n >= k >= 1)"), "(Sum k k <=1 <=n)");
        assert_eq!(errs("sum(k^2)"), vec![code::BIG_OP_RANGE]);
        assert_eq!(errs("sum(k; k <= n)"), vec![code::BIG_OP_RANGE]);
        assert_eq!(errs("sum(k; 1 <= k + 1 <= n)"), vec![code::BIG_OP_RANGE]);
    }

    #[test]
    fn lexer_errors_do_not_cascade() {
        // One unicode error, no extra implicit-multiplication / syntax errors.
        assert_eq!(errs("sin(θ) + 1"), vec![code::UNICODE]);
        assert_eq!(errs("a | b"), vec![code::CHAR]);
    }

    #[test]
    fn syntax_errors() {
        assert_eq!(errs("(a + b"), vec![code::SYNTAX]);
        assert_eq!(errs("a +"), vec![code::SYNTAX]);
        assert_eq!(errs("a )"), vec![code::SYNTAX]);
    }

    /// At least one error path per node kind not covered by the tests above.
    #[test]
    fn error_paths_for_remaining_node_kinds() {
        assert_eq!(errs("[1, 2"), vec![code::SYNTAX]); // List
        assert_eq!(errs("A_[1; 2]"), vec![code::SYNTAX]); // Index
        assert_eq!(errs("f.+1"), vec![code::SYNTAX]); // Address
        assert_eq!(errs("(a, )"), vec![code::SYNTAX]); // Tuple
        assert_eq!(errs("not"), vec![code::SYNTAX]); // Not
        assert_eq!(errs("!x"), vec![code::SYNTAX]); // Factorial needs an operand before it
        assert_eq!(errs("f(1; 2)"), vec![code::SYNTAX]); // Call
        assert_eq!(errs("a and"), vec![code::SYNTAX]); // And
        assert_eq!(errs("a or"), vec![code::SYNTAX]); // Or
        assert_eq!(errs("a / "), vec![code::SYNTAX]); // Div
        assert_eq!(errs("x_{}"), vec![code::SUBSCRIPT]); // Name
        assert_eq!(errs("\\bar x"), vec![code::TEX]); // Name (accent)
    }

    #[test]
    fn deep_nesting_is_a_limit_error_not_a_stack_overflow() {
        let src = format!("{}x{}", "(".repeat(5000), ")".repeat(5000));
        assert_eq!(errs(&src), vec![code::LIMIT]);
        assert_eq!(errs(&"-".repeat(5000)), vec![code::LIMIT]);
    }

    #[test]
    fn node_ids_are_preorder_with_terms() {
        let (e, _) = parse_expr("a*x + 1");
        let mut e = e.unwrap();
        e.assign_ids();
        let ExprKind::Sum { terms } = &e.kind else { panic!() };
        assert_eq!(e.id, 0);
        assert_eq!(terms[0].id, 1);
        assert_eq!(terms[0].expr.id, 2); // product
        let ExprKind::Product { factors } = &terms[0].expr.kind else { panic!() };
        assert_eq!((factors[0].id, factors[1].id), (3, 4));
        assert_eq!((terms[1].id, terms[1].expr.id), (5, 6));
    }

    #[test]
    fn ast_json_is_internally_tagged_without_span() {
        let (e, _) = parse_expr("-x");
        let json = serde_json::to_value(e.unwrap()).unwrap();
        assert_eq!(json["type"], "Neg");
        assert_eq!(json["arg"]["type"], "Name");
        assert_eq!(json["arg"]["name"]["key"], "x");
        assert!(json.get("span").is_none());
        let back: Expr = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(back).unwrap(), json);
    }
}
