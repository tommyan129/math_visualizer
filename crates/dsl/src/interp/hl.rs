//! Stage 2 statements: `constant:`, `style`, user variables, `highlight`, `req`,
//! subexpression addresses, and the highlight cascade (re-resolution on every request).

use std::collections::BTreeSet;

use super::*;
use crate::print::{expr_dsl, term_dsl};

/// One step of a subexpression address.
enum Step<'e> {
    Term(&'e Expr),
    Part(&'e str, Span),
}

/// Where an address currently points.
#[derive(Clone)]
enum Cursor<'f> {
    Node(&'f Expr),
    Term(&'f Term),
    /// `.coef` / `.var` over several factors.
    Many(Vec<&'f Expr>),
}

impl<'f> Cursor<'f> {
    fn ids(&self) -> Vec<NodeId> {
        match self {
            Cursor::Node(e) => vec![e.id],
            Cursor::Term(t) => vec![t.id],
            Cursor::Many(v) => v.iter().map(|e| e.id).collect(),
        }
    }
}

/// Resolved highlight slots.
struct HlResolved {
    target: HlTarget,
    range: Option<Expr>,
    style: HlStyle,
    style_name: Option<String>,
    memo: Option<String>,
    consts: Vec<String>,
    deps: Vec<String>,
    axes: Vec<String>,
}

const PARTS: &str = ".coef .var .num .den .exp .base .arg";

pub(super) fn unparen(e: &Expr) -> &Expr {
    match &e.kind {
        ExprKind::Paren { inner } => unparen(inner),
        _ => e,
    }
}

/// Structural equality ignoring ids, types, spans and Paren nodes.
fn same_shape(a: &Expr, b: &Expr) -> bool {
    let (a, b) = (unparen(a), unparen(b));
    match (&a.kind, &b.kind) {
        (ExprKind::Num { text: x, .. }, ExprKind::Num { text: y, .. }) => x == y,
        (ExprKind::Name { name: x, .. }, ExprKind::Name { name: y, .. }) => x.key == y.key,
        (ExprKind::Str { raw: x }, ExprKind::Str { raw: y }) => x == y,
        (ExprKind::Sum { terms: x }, ExprKind::Sum { terms: y }) => {
            x.len() == y.len()
                && x.iter().zip(y).all(|(p, q)| {
                    let (ps, pe) = signed(p.sign, &p.expr);
                    let (qs, qe) = signed(q.sign, &q.expr);
                    ps == qs && same_shape(&pe, &qe)
                })
        }
        (ExprKind::Call { callee: f, args: x, .. }, ExprKind::Call { callee: g, args: y, .. }) => f.key == g.key && same_list(x, y),
        (ExprKind::BigOp { op: o1, index: i1, lo_strict: l1, hi_strict: h1, .. }, ExprKind::BigOp { op: o2, index: i2, lo_strict: l2, hi_strict: h2, .. }) => {
            o1 == o2 && i1.key == i2.key && l1 == l2 && h1 == h2 && same_children(a, b)
        }
        (ExprKind::Compare { rest: r1, .. }, ExprKind::Compare { rest: r2, .. }) => {
            r1.len() == r2.len() && r1.iter().zip(r2).all(|(p, q)| p.op == q.op) && same_children(a, b)
        }
        (x, y) if std::mem::discriminant(x) == std::mem::discriminant(y) => same_children(a, b),
        _ => false,
    }
}

fn same_children(a: &Expr, b: &Expr) -> bool {
    let (x, y) = (a.children(), b.children());
    x.len() == y.len() && x.iter().zip(&y).all(|(p, q)| same_shape(p, q))
}

fn same_list(x: &[Expr], y: &[Expr]) -> bool {
    x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_shape(p, q))
}

/// A term with the sign pulled out of a leading unary minus: `+(-b)*x` → `-(b*x)`.
fn signed(sign: Sign, e: &Expr) -> (Sign, Expr) {
    let flip = |s: Sign| if s == Sign::Plus { Sign::Minus } else { Sign::Plus };
    let e = unparen(e);
    match &e.kind {
        ExprKind::Neg { arg } => {
            let (s, inner) = signed(sign, arg);
            (flip(s), inner)
        }
        ExprKind::Product { factors } if matches!(unparen(&factors[0]).kind, ExprKind::Neg { .. }) => {
            let ExprKind::Neg { arg } = &unparen(&factors[0]).kind else { unreachable!() };
            let mut f = factors.clone();
            f[0] = (**arg).clone();
            (flip(sign), Expr { kind: ExprKind::Product { factors: f }, ..e.clone() })
        }
        _ => (sign, e.clone()),
    }
}

fn contains_input(e: &Expr) -> bool {
    let mut found = false;
    e.walk(&mut |n| found |= matches!(n.kind, ExprKind::Name { kind: NameKind::Input, .. }));
    found
}

/// Every node an address term can match, in pre-order: Sum terms (with their sign)
/// and every other node that is not itself the expression of a Sum term.
fn candidates<'f>(e: &'f Expr, as_term_expr: bool, out: &mut Vec<Cursor<'f>>) {
    if !as_term_expr && !matches!(e.kind, ExprKind::Paren { .. }) {
        out.push(Cursor::Node(e));
    }
    if let ExprKind::Sum { terms } = &e.kind {
        for t in terms {
            out.push(Cursor::Term(t));
            candidates(&t.expr, true, out);
        }
        return;
    }
    for c in e.children() {
        candidates(c, false, out);
    }
}

pub(super) fn set_spans(e: &mut Expr, span: Span) {
    e.span = span;
    for c in e.children_mut() {
        set_spans(c, span);
    }
}

/// Applies `\"` and `\\` (design.md "문자열 이스케이프").
pub fn unescape(raw: &str) -> String {
    let mut out = String::new();
    let mut it = raw.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' && matches!(it.peek(), Some('"') | Some('\\')) {
            out.push(it.next().unwrap());
        } else {
            out.push(c);
        }
    }
    out
}

/// A number literal, optionally negated or parenthesized. The server never evaluates expressions.
pub(super) fn literal(e: &Expr) -> Option<f64> {
    match &e.kind {
        ExprKind::Num { text, .. } => text.parse().ok(),
        ExprKind::Neg { arg } => literal(arg).map(|v| -v),
        ExprKind::Paren { inner } => literal(inner),
        _ => None,
    }
}

/// Default slider step: a power of ten giving about 100 steps over the range.
fn nice_step(min: f64, max: f64) -> f64 {
    10f64.powf(((max - min) / 100.0).log10().floor())
}

fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

impl<'s> Cx<'s> {
    pub(super) fn text(&self, span: Span) -> &'s str {
        &self.src[span.range()]
    }

    /// Name checks shared by constants, styles, variables and highlights.
    /// `own`: the kind word this name may already have (it is then a modification).
    fn check_new_name(&mut self, name: &Ident, span: Span, own: &str) -> bool {
        let key = &name.key;
        let msg = if name.deriv > 0 || name.inverse {
            Some("이 이름 형태는 함수에만 씀".to_string())
        } else if names::is_builtin(key) || is_stage2_word(key) {
            Some(format!("'{key}'는 기본 이름이라 쓸 수 없음"))
        } else if is_reserved(name) {
            Some(format!("'{key}'는 예약어라 이름으로 쓸 수 없음"))
        } else if matches!(key.as_str(), "x" | "y") || self.axis_names().contains(key) {
            Some(format!("'{key}'는 좌표축 이름이라 쓸 수 없음"))
        } else if self.failed.contains(key) {
            return false;
        } else {
            match self.state.kind_of(key) {
                Some(k) if k != own => Some(format!("'{key}'는 이미 {k} 이름임")),
                _ => None,
            }
        };
        match msg {
            Some(m) => {
                self.err_s(span, code::NAME_CONFLICT, m, format!("다른 이름, 또는 del {key} 후 다시"));
                false
            }
            None => true,
        }
    }

    fn next_order(&mut self) -> u64 {
        let o = self.state.counters.next_order;
        self.state.counters.next_order += 1;
        o
    }

    // ----- constant: -----------------------------------------------------

    pub(super) fn declare_constant(&mut self, e: &ConstEntry) {
        let before = self.diags.len();
        let key = e.name.key.clone();
        if !self.check_new_name(&e.name, e.name_span, "상수") {
            return;
        }
        let existing = self.state.constants.get(&key).cloned();
        let integer = match &e.kind {
            Some((k, _)) if k == "integer" => true,
            Some((k, _)) if k == "real" => false,
            Some((k, span)) => {
                self.err_s(*span, code::CONST_KIND, format!("상수 종류 '{k}'는 없음"), "real 또는 integer");
                return;
            }
            None => existing.as_ref().is_some_and(|c| c.integer),
        };
        let (mut range, mut default, mut step) = (None, None, None);
        let mut seen: Vec<&str> = vec![];
        for p in &e.props {
            if seen.contains(&p.key.as_str()) {
                self.err(p.key_span, code::DUPLICATE, format!("'{}'를 두 번 적음", p.key));
                continue;
            }
            seen.push(&p.key);
            let num = |cx: &mut Self, v: &Expr| -> Option<f64> {
                let r = literal(v);
                if r.is_none() {
                    cx.err_s(v.span, code::CONST_PROP, "숫자만 쓸 수 있음 (서버는 식의 값을 계산하지 않음)", "6.2832 같은 숫자");
                }
                r
            };
            match p.key.as_str() {
                "range" => match &p.value.kind {
                    ExprKind::Tuple { items } if items.len() == 2 => {
                        if let (Some(a), Some(b)) = (num(self, &items[0]), num(self, &items[1])) {
                            range = Some((a, b));
                        }
                    }
                    _ => self.err_s(p.value.span, code::CONST_PROP, "range는 (최솟값, 최댓값) 모양", "range=(-5, 5)"),
                },
                "default" => default = num(self, &p.value),
                "step" => step = num(self, &p.value),
                other => self.err_s(p.key_span, code::CONST_PROP, format!("상수 속성 '{other}'는 없음"), "range=(최솟값, 최댓값), default=값, step=간격"),
            }
        }
        if self.diags.len() > before {
            return;
        }
        let (min, max) = range.unwrap_or(if integer { (0.0, 20.0) } else { (-10.0, 10.0) });
        let step = step.unwrap_or(if integer { 1.0 } else { nice_step(min, max) });
        let value = default.unwrap_or_else(|| {
            let v = existing.as_ref().map_or(if integer { 5.0 } else { 1.0 }, |c| c.value);
            if min < max { v.clamp(min, max) } else { v }
        });
        let at = e.span;
        if min >= max {
            self.err(at, code::CONST_RANGE, format!("최솟값({})이 최댓값({})보다 작아야 함", fmt_num(min), fmt_num(max)));
        } else if step <= 0.0 || step > max - min {
            self.err(at, code::CONST_RANGE, format!("step({})은 0보다 크고 범위 폭 이하여야 함", fmt_num(step)));
        } else if !(min..=max).contains(&value) {
            self.err(at, code::CONST_RANGE, format!("default({})가 범위 {}..{} 밖임", fmt_num(value), fmt_num(min), fmt_num(max)));
        } else if integer && [min, max, step, value].iter().any(|v| v.fract() != 0.0) {
            let why = if e.kind.is_some() { "integer 상수" } else { "sum/prod 범위에 쓰인 정수 상수" };
            self.err(at, code::CONST_KIND, format!("'{key}'는 {why}라 범위·default·step이 모두 정수여야 함"));
        }
        if self.diags.len() > before {
            return;
        }
        let order = match &existing {
            Some(c) => c.order,
            None => self.next_order(),
        };
        let c = Constant {
            key: key.clone(),
            name: e.name.clone(),
            value,
            min,
            max,
            step,
            integer,
            declared: true,
            kind_fixed: e.kind.is_some(),
            order,
            version: existing.as_ref().map_or(0, |c| c.version),
        };
        if existing.as_ref() != Some(&c) {
            // Auto-created earlier in this request: the declaration replaces that note.
            self.notes.retain(|n| !matches!(n, Note::ConstantCreated { key: k } if *k == key));
            self.notes.push(Note::ConstantDeclared { key: key.clone() });
        }
        self.state.constants.insert(key.clone(), c);
        self.declared_now.insert(key);
    }

    // ----- style ---------------------------------------------------------

    pub(super) fn style(&mut self, name: &Ident, name_span: Span, props: &[StyleProp], span: Span) {
        let before = self.diags.len();
        let key = name.key.clone();
        if !self.check_new_name(name, name_span, "style") {
            return;
        }
        let mut def = StyleDef { key: key.clone(), name: name.clone(), color: None, fill: None, opacity: None, width: None, src: self.text(span).to_string(), order: 0, version: 0 };
        let mut seen: Vec<&str> = vec![];
        for p in props {
            if seen.contains(&p.key.as_str()) {
                self.err(p.key_span, code::DUPLICATE, format!("'{}'를 두 번 적음", p.key));
                continue;
            }
            seen.push(&p.key);
            let ident = match &p.value.kind {
                ExprKind::Name { name, .. } => Some(name.key.as_str()),
                _ => None,
            };
            match p.key.as_str() {
                "color" => match (&p.value.kind, ident) {
                    (_, Some(c)) if base_style_color(c).is_some() => def.color = base_style_color(c).map(String::from),
                    (ExprKind::Str { raw }, _) if is_hex_color(raw) => def.color = Some(raw.to_lowercase()),
                    _ => self.err_s(p.value.span, code::STYLE, "색은 기본 색 이름이나 \"#rrggbb\"", base_colors()),
                },
                "fill" => match ident.and_then(Fill::parse) {
                    Some(f) => def.fill = Some(f),
                    None => self.err_s(p.value.span, code::STYLE, "fill은 axis, above, below, region 중 하나", "axis"),
                },
                "opacity" => match literal(&p.value).filter(|v| (0.0..=1.0).contains(v)) {
                    Some(v) => def.opacity = Some(v),
                    None => self.err_s(p.value.span, code::STYLE, "opacity는 0에서 1 사이 숫자", "0.25"),
                },
                "width" => match literal(&p.value).filter(|v| (0.0..=20.0).contains(v)) {
                    Some(v) => def.width = Some(v),
                    None => self.err_s(p.value.span, code::STYLE, "width는 0에서 20 사이 숫자 (px)", "2"),
                },
                other => self.err_s(p.key_span, code::STYLE, format!("style 속성 '{other}'는 없음"), "color, fill, opacity, width"),
            }
        }
        if self.diags.len() > before {
            self.failed.insert(key);
            return;
        }
        let old = self.state.styles.get(&key).cloned();
        def.order = match &old {
            Some(o) => o.order,
            None => self.next_order(),
        };
        def.version = old.as_ref().map_or(0, |o| o.version);
        self.note_put(ObjKind::Style, &key, old.as_ref().map(|o| o == &def));
        self.state.styles.insert(key, def);
    }

    fn note_put(&mut self, kind: ObjKind, key: &str, same: Option<bool>) {
        let key = key.to_string();
        self.notes.push(match same {
            None => Note::ObjectCreated { kind, key },
            Some(true) => Note::ObjectUnchanged { kind, key },
            Some(false) => Note::ObjectModified { kind, key },
        });
    }

    // ----- assignment ----------------------------------------------------

    pub(super) fn assign(&mut self, name: &Ident, name_span: Span, value: &AssignValue, span: Span) {
        match value {
            AssignValue::Highlight(slots) => self.highlight_stmt(name, name_span, slots),
            AssignValue::Expr(e) => self.var(name, name_span, e, span),
        }
    }

    fn var(&mut self, name: &Ident, name_span: Span, value: &Expr, _span: Span) {
        let before = self.diags.len();
        let key = name.key.clone();
        if !self.check_new_name(name, name_span, "변수") {
            return;
        }
        // Names are resolved where the variable is used; check here that each one exists somewhere.
        self.loose_check(value);
        // Self-reference through other variables.
        let mut probe = value.clone();
        let mut deps = BTreeSet::new();
        self.expand(&mut probe, &mut deps, &mut vec![key.clone()], false);
        if self.diags.len() > before {
            self.failed.insert(key);
            return;
        }
        let mut stored = value.clone();
        stored.clear_spans();
        let old = self.state.vars.get(&key).cloned();
        let order = match &old {
            Some(o) => o.order,
            None => self.next_order(),
        };
        let v = UserVar { key: key.clone(), name: name.clone(), value: stored, src: self.text(value.span).to_string(), order, version: old.as_ref().map_or(0, |o| o.version) };
        self.note_put(ObjKind::Var, &key, old.as_ref().map(|o| o == &v));
        self.state.vars.insert(key, v);
    }

    /// Every name in a variable value must exist as an axis, constant, variable, function or builtin.
    fn loose_check(&mut self, e: &Expr) {
        let axes = self.axis_names();
        let mut bad: Vec<(String, Span, bool)> = vec![];
        e.walk(&mut |n| match &n.kind {
            ExprKind::Name { name, .. } => {
                let k = &name.key;
                let known = matches!(k.as_str(), "x" | "y") || axes.contains(k) || is_reserved(name) || self.state.constants.contains_key(k) || self.state.vars.contains_key(k);
                if !known && !is_bad_tex(name) && !bad.iter().any(|(b, _, _)| b == k) {
                    bad.push((k.clone(), n.span, false));
                }
            }
            ExprKind::Call { callee, .. } => {
                let k = &callee.key;
                let known = k == "between" || names::builtin_arity(k).is_some() || self.is_function_name(k);
                if !known && !bad.iter().any(|(b, _, _)| b == k) {
                    bad.push((k.clone(), n.span, true));
                }
            }
            ExprKind::Address { .. } => bad.push((String::new(), n.span, false)),
            _ => {}
        });
        for (k, span, call) in bad {
            if k.is_empty() {
                self.err(span, code::TYPE, "부분식 주소는 highlight·req의 대상 칸에만 쓸 수 있음");
            } else if call {
                self.err(span, code::UNDECLARED, format!("함수 '{k}'가 정의되지 않음"));
            } else {
                self.err(span, code::UNDECLARED, format!("'{k}'가 선언되지 않음 (변수 값에는 좌표축 이름, 상수, 변수, 함수 호출을 씀)"));
            }
        }
    }

    /// Substitutes user variables (by reference: their current value) and expands
    /// `between(f, g)`. Spans of substituted nodes point at the use site.
    pub(super) fn expand(&mut self, e: &mut Expr, deps: &mut BTreeSet<String>, stack: &mut Vec<String>, grid: bool) {
        if !self.tick() {
            return;
        }
        let span = e.span;
        match &e.kind {
            ExprKind::Name { name, .. } if self.state.vars.contains_key(&name.key) => {
                let key = name.key.clone();
                if stack.contains(&key) {
                    self.err(span, code::CYCLE, format!("변수 '{key}'가 자기 자신을 참조함"));
                    return;
                }
                deps.insert(key.clone());
                let mut v = self.state.vars[&key].value.clone();
                set_spans(&mut v, span);
                stack.push(key);
                self.expand(&mut v, deps, stack, grid);
                stack.pop();
                // Keep the substituted value grouped.
                *e = Expr { id: 0, ty: Ty::Unknown, span, kind: ExprKind::Paren { inner: Box::new(v) } };
                return;
            }
            ExprKind::Call { callee, args, .. } if callee.key == "between" => {
                let fs: Vec<Option<Ident>> = args
                    .iter()
                    .map(|a| match &a.kind {
                        ExprKind::Name { name, .. } if self.state.functions.contains_key(&name.key) => Some(name.clone()),
                        _ => None,
                    })
                    .collect();
                if args.len() != 2 || fs.iter().any(|f| f.is_none()) {
                    self.err_s(span, code::TYPE, "between의 인자는 함수 이름 두 개", "between(f, g)");
                    return;
                }
                if !grid {
                    self.err_s(span, code::RANGE, "between은 대상 없는 highlight의 범위 칸에서만 쓸 수 있음", "H1 = highlight(; between(f, g))");
                    return;
                }
                let (f, g) = (fs[0].clone().unwrap(), fs[1].clone().unwrap());
                let (sf, sg) = (self.state.function_system(&f.key), self.state.function_system(&g.key));
                if sf != sg {
                    self.err_s(span, code::PLACEMENT, format!("between의 두 함수가 다른 좌표계임 ({}, {})", sf.unwrap_or_default(), sg.unwrap_or_default()), "같은 입력·출력 축의 함수 두 개");
                    return;
                }
                let func = &self.state.functions[&f.key];
                let (input, output) = (func.inputs[0].clone(), func.output.clone());
                let node = |kind| Expr { id: 0, ty: Ty::Unknown, span, kind };
                let name = |k: &Ident| node(ExprKind::Name { name: k.clone(), kind: NameKind::Unresolved });
                let call = |f: &Ident| node(ExprKind::Call { callee: f.clone(), kind: NameKind::Unresolved, args: vec![name(&input)] });
                let chain = |a: &Ident, b: &Ident| {
                    node(ExprKind::Compare { first: Box::new(call(a)), rest: vec![CompareLink { op: CmpOp::Lt, rhs: name(&output) }, CompareLink { op: CmpOp::Lt, rhs: call(b) }] })
                };
                *e = node(ExprKind::Paren { inner: Box::new(node(ExprKind::Or { args: vec![chain(&f, &g), chain(&g, &f)] })) });
                return;
            }
            _ => {}
        }
        for c in e.children_mut() {
            self.expand(c, deps, stack, grid);
        }
    }

    /// Expands and resolves a range slot over `axes`. Returns the resolved region and what it used.
    pub(super) fn resolve_range(&mut self, e: &Expr, axes: &[&str], grid: bool, deps: &mut BTreeSet<String>, consts: &mut BTreeSet<String>) -> Option<Expr> {
        let before = self.diags.len();
        let mut r = e.clone();
        self.expand(&mut r, deps, &mut vec![], grid);
        if self.diags.len() > before {
            return None;
        }
        let mut sc = Scope::range(axes);
        let mut info = DefInfo::default();
        let ty = self.resolve(&mut r, &mut sc, &mut info);
        if self.diags.len() > before {
            return None;
        }
        if ty != Ty::Region {
            let what = match ty {
                Ty::Str => "문자열",
                _ => "숫자 식",
            };
            self.err_s(e.span, code::RANGE, format!("범위 칸에는 부등식이 와야 하는데 {what}이 옴"), "0 <= x <= 2");
            return None;
        }
        consts.extend(info.used_consts);
        for f in &info.callees {
            deps.insert(f.clone());
            if let Some(func) = self.state.functions.get(f) {
                consts.extend(func.consts.iter().cloned());
            }
        }
        r.assign_ids();
        r.clear_spans();
        Some(r)
    }

    /// String literal or a variable holding one.
    fn resolve_memo(&mut self, e: &Expr, deps: &mut BTreeSet<String>) -> Option<String> {
        match &e.kind {
            ExprKind::Str { raw } => Some(unescape(raw)),
            ExprKind::Name { name, .. } => match self.state.vars.get(&name.key).map(|v| v.value.kind.clone()) {
                Some(ExprKind::Str { raw }) => {
                    deps.insert(name.key.clone());
                    Some(unescape(&raw))
                }
                Some(_) => {
                    self.err(e.span, code::MEMO, format!("변수 '{}'의 값이 문자열이 아님", name.key));
                    None
                }
                None => {
                    self.err_s(e.span, code::MEMO, format!("'{}'라는 변수가 없음", name.key), format!("\"...\" 또는 {} = \"...\"를 먼저", name.key));
                    None
                }
            },
            _ => {
                self.err_s(e.span, code::MEMO, "memo 칸에는 문자열이나 문자열 변수", "\"설명\"");
                None
            }
        }
    }

    // ----- addresses -----------------------------------------------------

    /// `f.(b*x).coef` → function key, node ids, canonical address text.
    fn resolve_address(&mut self, e: &Expr) -> Option<(String, Vec<NodeId>, String)> {
        let mut steps = vec![];
        let mut cur = e;
        while let ExprKind::Address { base, step } = &cur.kind {
            steps.push(match step {
                AddressStep::Term { pattern } => Step::Term(pattern),
                AddressStep::Part { part } => Step::Part(part, Span::new(base.span.end as usize, cur.span.end as usize)),
            });
            cur = base;
        }
        steps.reverse();
        let fkey = match &cur.kind {
            ExprKind::Name { name, .. } if self.state.functions.contains_key(&name.key) => name.key.clone(),
            ExprKind::Name { name, .. } if self.failed.contains(&name.key) => return None,
            ExprKind::Name { name, .. } => {
                let what = self.state.kind_of(&name.key).map_or("정의가 없음".to_string(), |k| format!("{k}라 주소를 쓸 수 없음 (함수만)"));
                self.err(cur.span, code::TARGET, format!("'{}'는 {what}", name.key));
                return None;
            }
            _ => {
                self.err_s(cur.span, code::TARGET, "주소는 함수 이름으로 시작해야 함", "f.(2*x)");
                return None;
            }
        };
        let f = self.state.functions[&fkey].clone();
        let mut at = Cursor::Node(unparen(&f.body));
        let mut text = fkey.clone();
        for step in &steps {
            match step {
                Step::Term(pattern) => {
                    let (psign, pexpr) = signed(Sign::Plus, pattern);
                    text = format!("{text}.({})", term_dsl(psign, &pexpr));
                    let root = match &at {
                        Cursor::Node(n) => *n,
                        Cursor::Term(t) => &t.expr,
                        Cursor::Many(_) => {
                            self.err(pattern.span, code::ADDRESS_PART, "여러 인수를 가리킨 뒤에는 더 내려갈 수 없음");
                            return None;
                        }
                    };
                    let mut all = vec![];
                    candidates(root, false, &mut all);
                    let mut exact = vec![];
                    let mut flipped = vec![];
                    for c in all {
                        let (s, ce) = match &c {
                            Cursor::Term(t) => signed(t.sign, &t.expr),
                            Cursor::Node(n) => signed(Sign::Plus, n),
                            Cursor::Many(_) => unreachable!(),
                        };
                        if same_shape(&ce, &pexpr) {
                            if s == psign { exact.push(c) } else { flipped.push((c, s)) }
                        }
                    }
                    match exact.len() {
                        1 => at = exact.pop().unwrap(),
                        0 if !flipped.is_empty() => {
                            let fixed = term_dsl(flipped[0].1, &pexpr);
                            self.err_s(pattern.span, code::ADDRESS_SIGN, format!("{fkey}의 항은 부호까지 써야 함"), format!("{}.({fixed})", text.rsplit_once(".(").map_or(fkey.as_str(), |(b, _)| b)));
                            return None;
                        }
                        0 => {
                            let list = match &unparen(root).kind {
                                ExprKind::Sum { terms } => terms.iter().map(|t| { let (s, e) = signed(t.sign, &t.expr); term_dsl(s, &e) }).collect::<Vec<_>>().join(", "),
                                _ => expr_dsl(root),
                            };
                            self.err_s(pattern.span, code::ADDRESS_MISSING, format!("{fkey}에 ({}) 모양이 없음", term_dsl(psign, &pexpr)), format!("있는 항: {list}"));
                            return None;
                        }
                        n => {
                            self.err_s(pattern.span, code::ADDRESS_AMBIGUOUS, format!("({}) 모양이 {fkey}에 {n}곳 있음", term_dsl(psign, &pexpr)), "그것을 포함한 더 큰 항으로 가리킨 뒤 .var, .arg 등으로 내려가기");
                            return None;
                        }
                    }
                }
                Step::Part(part, span) => {
                    text = format!("{text}.{part}");
                    let node: &Expr = match &at {
                        Cursor::Node(n) => n,
                        Cursor::Term(t) => &t.expr,
                        Cursor::Many(_) => {
                            self.err(*span, code::ADDRESS_PART, "여러 인수를 가리킨 뒤에는 더 내려갈 수 없음");
                            return None;
                        }
                    };
                    let node = unparen(node);
                    let fail = |cx: &mut Self, msg: String| {
                        cx.err_s(*span, code::ADDRESS_PART, msg, format!("이 자리의 식: {}", expr_dsl(node)));
                    };
                    at = match (*part, &node.kind) {
                        ("coef" | "var", _) => {
                            let factors: Vec<&Expr> = match &node.kind {
                                ExprKind::Product { factors } => factors.iter().collect(),
                                ExprKind::Neg { arg } => vec![unparen(arg)],
                                _ => vec![node],
                            };
                            let want_var = *part == "var";
                            let picked: Vec<&Expr> = factors.into_iter().filter(|f| contains_input(f) == want_var).collect();
                            match picked.len() {
                                0 => {
                                    fail(self, format!("({}) 안에 {}가 없음", expr_dsl(node), if want_var { "입력 변수가 든 인수" } else { "계수" }));
                                    return None;
                                }
                                1 => Cursor::Node(picked[0]),
                                _ => Cursor::Many(picked),
                            }
                        }
                        ("num", ExprKind::Div { num, .. }) => Cursor::Node(unparen(num)),
                        ("den", ExprKind::Div { den, .. }) => Cursor::Node(unparen(den)),
                        ("num" | "den", _) => {
                            fail(self, format!(".{part}는 분수에만 씀"));
                            return None;
                        }
                        ("base", ExprKind::Pow { base, .. }) => Cursor::Node(unparen(base)),
                        ("exp", ExprKind::Pow { exp, .. }) => Cursor::Node(unparen(exp)),
                        ("base" | "exp", _) => {
                            fail(self, format!(".{part}는 거듭제곱에만 씀"));
                            return None;
                        }
                        ("arg", ExprKind::Call { args, .. }) if args.len() == 1 => Cursor::Node(unparen(&args[0])),
                        ("arg", ExprKind::Call { .. }) => {
                            fail(self, "인자가 여럿이라 .arg로 가리킬 수 없음".into());
                            return None;
                        }
                        ("arg", _) => {
                            fail(self, ".arg는 함수 호출에만 씀".into());
                            return None;
                        }
                        (other, _) => {
                            self.err_s(*span, code::ADDRESS_PART, format!("부분 이름 '.{other}'는 없음"), PARTS);
                            return None;
                        }
                    };
                }
            }
        }
        if steps.is_empty() {
            return Some((fkey, vec![], text));
        }
        Some((fkey, at.ids(), text))
    }

    // ----- highlight -----------------------------------------------------

    fn highlight_stmt(&mut self, name: &Ident, name_span: Span, slots: &Slots) {
        let before = self.diags.len();
        let key = name.key.clone();
        if !self.check_new_name(name, name_span, "강조") {
            return;
        }
        if slots.items.len() > 4 {
            self.err_s(slots.spans[4], code::SLOTS, "highlight 칸은 넷까지 (대상; 범위; style; memo)", "H1 = highlight(f; 0 <= x <= 2; blue; \"설명\")");
            self.failed.insert(key);
            return;
        }
        let mut source = HlSource { target: slots.get(0).cloned(), range: slots.get(1).cloned(), style: slots.get(2).cloned(), memo: slots.get(3).cloned() };
        let old = self.state.highlights.get(&key).cloned();
        let auto_color = match &old {
            Some(o) => o.auto_color.clone(),
            None => AUTO_COLORS[self.state.counters.next_hl_color as usize % AUTO_COLORS.len()].to_string(),
        };
        let Some(r) = self.resolve_highlight(&source, &auto_color, slots.span) else {
            self.failed.insert(key);
            return;
        };
        if self.diags.len() > before {
            self.failed.insert(key);
            return;
        }
        for e in [&mut source.target, &mut source.range, &mut source.style, &mut source.memo].into_iter().flatten() {
            e.clear_spans();
        }
        let order = match &old {
            Some(o) => o.order,
            None => {
                self.state.counters.next_hl_color += 1;
                self.next_order()
            }
        };
        let h = Highlight {
            key: key.clone(),
            name: name.clone(),
            target: r.target,
            range: r.range,
            style: r.style,
            style_name: r.style_name,
            memo: r.memo,
            consts: r.consts,
            deps: r.deps,
            axes: r.axes,
            source,
            src: self.text(slots.span).to_string(),
            auto_color,
            order,
            version: old.as_ref().map_or(0, |o| o.version),
        };
        let same = old.as_ref().map(|o| {
            let mut a = o.clone();
            a.version = 0;
            let mut b = h.clone();
            b.version = 0;
            a == b
        });
        self.note_put(ObjKind::Highlight, &key, same);
        self.state.highlights.insert(key, h);
    }

    fn resolve_highlight(&mut self, src: &HlSource, auto_color: &str, at: Span) -> Option<HlResolved> {
        let before = self.diags.len();
        let mut deps = BTreeSet::new();
        let mut consts = BTreeSet::new();
        let target = match &src.target {
            None => HlTarget::Grid {},
            Some(e) => match &e.kind {
                ExprKind::Name { name, .. } if self.state.functions.contains_key(&name.key) => HlTarget::Function { function: name.key.clone() },
                ExprKind::Name { name, .. } if self.failed.contains(&name.key) => return None,
                ExprKind::Address { .. } => {
                    let (function, nodes, address) = self.resolve_address(e)?;
                    if nodes.is_empty() { HlTarget::Function { function } } else { HlTarget::Sub { function, nodes, address } }
                }
                ExprKind::Name { name, .. } => {
                    match self.state.kind_of(&name.key) {
                        Some(k) => self.err(e.span, code::TARGET, format!("{k} '{}'는 강조 대상이 될 수 없음 (함수나 부분식)", name.key)),
                        None => self.err(e.span, code::UNDECLARED, format!("'{}'라는 함수가 없음", name.key)),
                    }
                    return None;
                }
                _ => {
                    self.err_s(e.span, code::TARGET, "대상 칸에는 함수 이름이나 부분식 주소", "f 또는 f.(2*x)");
                    return None;
                }
            },
        };
        let range = match (&target, &src.range) {
            (HlTarget::Grid {}, None) => {
                self.err_s(at, code::SLOTS, "대상이 없으면 범위 칸에 칠할 영역이 필요함", "H1 = highlight(; between(f, g) and 0 <= x <= 2)");
                return None;
            }
            (HlTarget::Sub { .. }, Some(r)) => {
                self.err(r.span, code::RANGE, "부분식 강조에는 범위를 쓸 수 없음 (식 안의 색칠만)");
                return None;
            }
            (_, None) => None,
            (HlTarget::Grid {}, Some(r)) => {
                let all = self.axis_names();
                let all: Vec<&str> = all.iter().map(|s| s.as_str()).collect();
                Some(self.resolve_range(r, &all, true, &mut deps, &mut consts)?)
            }
            (HlTarget::Function { function }, Some(r)) => {
                let input = self.state.functions[function].inputs[0].key.clone();
                Some(self.resolve_range(r, &[input.as_str()], false, &mut deps, &mut consts)?)
            }
        };
        let axes = match &target {
            HlTarget::Function { function } | HlTarget::Sub { function, .. } => {
                deps.insert(function.clone());
                let f = &self.state.functions[function];
                consts.extend(f.consts.iter().cloned());
                vec![f.inputs[0].key.clone(), f.output.key.clone()]
            }
            HlTarget::Grid {} => {
                let mut used = vec![];
                if let Some(r) = &range {
                    r.walk(&mut |n| {
                        if let ExprKind::Name { name, kind: NameKind::Input } = &n.kind
                            && !used.contains(&name.key)
                        {
                            used.push(name.key.clone());
                        }
                    });
                }
                used
            }
        };
        let fits_some = || {
            let mut systems: Vec<[String; 2]> = vec![["x".into(), "y".into()]];
            systems.extend(self.state.functions.values().map(|f| [f.inputs[0].key.clone(), f.output.key.clone()]));
            systems.iter().any(|s| axes.iter().all(|a| s.contains(a)))
        };
        if axes.len() > 2 || !fits_some() {
            self.err_s(src.range.as_ref().map_or(at, |r| r.span), code::PLACEMENT, format!("범위에 쓴 좌표축 {}이 한 좌표계에 속하지 않음", axes.join(", ")), "x, y만 쓰거나 t, y만 쓰기");
            return None;
        }
        let default_fill = if matches!(target, HlTarget::Grid {}) { Fill::Region } else { Fill::Axis };
        let mut style = HlStyle { color: auto_color.to_string(), fill: default_fill, opacity: DEFAULT_OPACITY, width: DEFAULT_WIDTH };
        let mut style_name = None;
        if let Some(e) = &src.style {
            let ExprKind::Name { name, .. } = &e.kind else {
                self.err_s(e.span, code::STYLE, "style 칸에는 style 이름", base_colors());
                return None;
            };
            let k = &name.key;
            if let Some(c) = base_style_color(k) {
                style.color = c.to_string();
            } else if let Some(d) = self.state.styles.get(k) {
                if let Some(c) = &d.color {
                    style.color = c.clone();
                }
                style.fill = d.fill.unwrap_or(default_fill);
                style.opacity = d.opacity.unwrap_or(DEFAULT_OPACITY);
                style.width = d.width.unwrap_or(DEFAULT_WIDTH);
                deps.insert(k.clone());
            } else if self.failed.contains(k) {
                return None;
            } else {
                self.err_s(e.span, code::STYLE, format!("'{k}'라는 style이 없음"), base_colors());
                return None;
            }
            style_name = Some(k.clone());
        }
        match (&target, style.fill) {
            // The region is the fill; a fill rule does not apply.
            (HlTarget::Grid {}, _) => style.fill = Fill::Region,
            (HlTarget::Function { .. }, Fill::Region) => {
                let span = src.style.as_ref().map_or(at, |e| e.span);
                self.err_s(span, code::STYLE, "fill: region은 대상 없는 highlight에만 씀", "fill: axis, above, below");
                return None;
            }
            _ => {}
        }
        let memo = match &src.memo {
            None => None,
            Some(e) => Some(self.resolve_memo(e, &mut deps)?),
        };
        if self.diags.len() > before {
            return None;
        }
        Some(HlResolved { target, range, style, style_name, memo, consts: consts.into_iter().collect(), deps: deps.into_iter().collect(), axes })
    }

    /// Re-resolves every highlight against the state after this request. One that no
    /// longer resolves (deleted function, vanished subexpression, changed variable) is
    /// deleted and reported (cascade). Errors here are not request errors.
    pub(super) fn refresh_highlights(&mut self) {
        let mut hs: Vec<Highlight> = self.state.highlights.values().cloned().collect();
        hs.sort_by_key(|h| h.order);
        for h in hs {
            let before = self.diags.len();
            let r = self.resolve_highlight(&h.source, &h.auto_color, Span::new(0, 0));
            let errs: Vec<Diagnostic> = self.diags.drain(before..).collect();
            if self.stopped {
                // A resource limit is a real error.
                self.diags.extend(errs.into_iter().filter(|d| d.code == code::LIMIT));
                return;
            }
            match r {
                Some(r) if errs.is_empty() => {
                    let e = self.state.highlights.get_mut(&h.key).unwrap();
                    e.target = r.target;
                    e.range = r.range;
                    e.style = r.style;
                    e.style_name = r.style_name;
                    e.memo = r.memo;
                    e.consts = r.consts;
                    e.deps = r.deps;
                    e.axes = r.axes;
                }
                _ => {
                    self.state.highlights.remove(&h.key);
                    let via = h.deps.iter().find(|d| self.state.kind_of(d).is_none()).cloned();
                    let reason = match &via {
                        Some(_) => None,
                        None => Some(match (&h.target, errs.first()) {
                            (HlTarget::Sub { function, address, .. }, Some(d)) if d.code.starts_with("E-address") => format!("{function} 변경으로 {address}가 가리키던 부분이 사라짐"),
                            (_, Some(d)) => d.message.clone(),
                            (_, None) => "다시 해석할 수 없음".into(),
                        }),
                    };
                    self.notes.push(Note::ObjectDeleted { kind: ObjKind::Highlight, key: h.key.clone(), via, reason });
                }
            }
        }
    }

    // ----- req -----------------------------------------------------------

    pub(super) fn req(&mut self, slots: &Slots, scene: Option<&ReqScene>, span: Span) {
        let before = self.diags.len();
        if slots.items.len() > 3 {
            self.err_s(slots.spans[3], code::SLOTS, "req 칸은 셋까지 (대상; 범위; memo)", "req(f; 0.9 < x < 1.1; \"설명\")");
            return;
        }
        let mut deps = BTreeSet::new();
        if let Some((r, step, at)) = scene {
            return self.req_scene(r, *step, *at, slots, span);
        }
        let Some(t) = slots.get(0) else {
            self.err_s(slots.spans[0], code::SLOTS, "req에는 대상이 필요함", "req(f)");
            return;
        };
        let (target, target_src, function) = match &t.kind {
            ExprKind::Name { name, .. } if self.state.functions.contains_key(&name.key) => {
                let f = &self.state.functions[&name.key];
                (ReqTarget::Function { function: f.key.clone() }, f.source(), Some(f.key.clone()))
            }
            ExprKind::Name { name, .. } if self.state.highlights.contains_key(&name.key) => {
                let h = &self.state.highlights[&name.key];
                deps.extend(h.deps.iter().filter(|d| self.state.functions.contains_key(*d)).cloned());
                (ReqTarget::Highlight { highlight: h.key.clone() }, format!("{} = {}", h.key, h.src), h.deps.iter().find(|d| self.state.functions.contains_key(*d)).cloned())
            }
            ExprKind::Address { .. } => {
                let Some((function, _, address)) = self.resolve_address(t) else { return };
                let src = self.state.functions[&function].source();
                (ReqTarget::Sub { function: function.clone(), address }, src, Some(function))
            }
            ExprKind::Name { name, .. } => {
                match self.state.kind_of(&name.key) {
                    Some(k) => self.err(t.span, code::TARGET, format!("{k} '{}'는 req 대상이 될 수 없음 (함수, 강조, 부분식)", name.key)),
                    None => self.err(t.span, code::UNDECLARED, format!("'{}'라는 정의가 없음", name.key)),
                }
                return;
            }
            _ => {
                self.err_s(t.span, code::TARGET, "대상 칸에는 함수·강조 이름이나 부분식 주소", "req(f)");
                return;
            }
        };
        if let Some(f) = &function {
            // The target and everything it calls: an indirect change shows as "수정됨 (g 변경으로)".
            let mut stack = vec![f.clone()];
            while let Some(k) = stack.pop() {
                if deps.insert(k.clone())
                    && let Some(func) = self.state.functions.get(&k)
                {
                    stack.extend(func.calls.iter().cloned());
                }
            }
        }
        let fa = function.as_ref().and_then(|f| self.state.functions.get(f)).map(|f| [f.inputs[0].key.clone(), f.output.key.clone()]);
        let fa = fa.unwrap_or_else(|| ["x".to_string(), "y".to_string()]);
        let axes: &[&str] = &[fa[0].as_str(), fa[1].as_str()];
        let range = match slots.get(1) {
            None => None,
            Some(r) => {
                let mut ignored = BTreeSet::new();
                let mut text_src = r.clone();
                self.expand(&mut text_src, &mut ignored, &mut vec![], true);
                if self.resolve_range(r, axes, true, &mut ignored, &mut BTreeSet::new()).is_none() {
                    return;
                }
                Some(expr_dsl(&text_src))
            }
        };
        let memo = match slots.get(2) {
            None => None,
            Some(m) => match self.resolve_memo(m, &mut BTreeSet::new()) {
                Some(t) => Some(t),
                None => return,
            },
        };
        if self.diags.len() > before {
            return;
        }
        self.push_request(target, range, memo, target_src, deps.into_iter().collect(), span);
    }

    fn push_request(&mut self, target: ReqTarget, range: Option<String>, memo: Option<String>, target_src: String, deps: Vec<String>, span: Span) {
        self.state.counters.next_request += 1;
        let number = self.state.counters.next_request;
        let request = Request { number, target, range, memo, src: self.text(span).to_string(), target_src, deps, seq: self.seq, done: false, version: 0 };
        self.state.requests.insert(number, request);
        self.notes.push(Note::RequestAdded { number });
    }

    /// `req(scene(1).step(3);; memo)`: no range on a scene target.
    fn req_scene(&mut self, r: &SceneRef, step: Option<(u32, Span)>, at: Span, slots: &Slots, span: Span) {
        let key = match r {
            SceneRef::Number(n) => n.to_string(),
            SceneRef::System(sp) => sp.key(),
        };
        let Some(sc) = self.state.scenes.get(&key) else {
            self.err(at, code::UNDECLARED, format!("scene({key})이 없음"));
            return;
        };
        if let Some((n, s)) = step
            && n as usize > sc.step_count()
        {
            let count = sc.step_count();
            self.err_s(s, code::SCENE, format!("scene({key})에는 step이 {count}개뿐임"), if count > 0 { format!("scene({key}).step({count})") } else { format!("scene({key})") });
            return;
        }
        let target_src = if sc.src.is_empty() { sc.label() } else { sc.src.clone() };
        if let Some(r) = slots.get(1) {
            self.err_s(r.span, code::RANGE, "scene을 가리키는 req에는 범위를 쓰지 않음", "req(scene(1).step(2);; \"설명\")");
            return;
        }
        let memo = match slots.get(2) {
            None => None,
            Some(m) => match self.resolve_memo(m, &mut BTreeSet::new()) {
                Some(t) => Some(t),
                None => return,
            },
        };
        self.push_request(ReqTarget::Scene { scene: key, step: step.map(|s| s.0) }, None, memo, target_src, vec![], span);
    }

    /// Axis names of every coordinate system in use, plus `x`, `y`.
    pub(super) fn axis_names(&self) -> Vec<String> {
        let mut v = vec!["x".to_string(), "y".to_string()];
        for f in self.state.functions.values() {
            for a in [&f.inputs[0].key, &f.output.key] {
                if !v.contains(a) {
                    v.push(a.clone());
                }
            }
        }
        v
    }
}

/// Does `address` (`f.(b*x).coef`) still point at something in `state`? (req change status)
pub fn address_resolves(state: &State, address: &str) -> bool {
    let (Some(e), diags) = crate::parser::parse_expr(address) else { return false };
    if !diags.is_empty() {
        return false;
    }
    let limits = Limits::default();
    let mut cx = Cx {
        src: address,
        state: state.clone(),
        notes: vec![],
        diags: vec![],
        failed: HashSet::new(),
        fuel_left: limits.fuel,
        fuel_limit: limits.fuel,
        deadline: Instant::now() + limits.wall_clock,
        wall_clock: limits.wall_clock,
        stopped: false,
        seq: 0,
        declared_now: BTreeSet::new(),
    };
    cx.resolve_address(&e).is_some() && cx.diags.is_empty()
}

fn base_colors() -> String {
    BASE_STYLES.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
}
