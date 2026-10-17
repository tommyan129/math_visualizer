//! Feature-point definitions (stage 5b decisions): `E: extrema(f)`, `roots(f; -1 < x < 1)`,
//! `intersect(f, g)`, `intersect(f, y = x)`, `breaks(f;; hole)`, `tangents(p;; vertical)`.
//! The server resolves the slots and extracts break candidates from the target's AST; the
//! browser finds the points.

use std::collections::HashMap;

use super::*;

/// Most break candidates kept per definition.
const MAX_CANDIDATES: usize = 64;
/// Call depth followed into callees for break candidates.
const MAX_INLINE: u32 = 8;

impl<'s> Cx<'s> {
    /// Body `kind(targets; range; filter)`. Leaves the spec in `pending_feature`.
    pub(super) fn resolve_feature(&mut self, body: &mut Expr, sc: &mut Scope, info: &mut DefInfo) -> Ty {
        let span = body.span;
        let ExprKind::Feature { callee, args, range, filter } = &mut body.kind else { unreachable!("routed by check_shape") };
        let key = callee.key.clone();
        let kind = FeatureKind::parse(&key).expect("parser checks the name");
        let (want, filters) = names::feature(&key).unwrap();
        let form = match kind {
            FeatureKind::Intersect => "intersect(f, g)".to_string(),
            FeatureKind::Tangents => "tangents(p)".to_string(),
            FeatureKind::Argmax | FeatureKind::Argmin => format!("{key}(f; 0 <= x <= 2)"),
            _ => format!("{key}(f)"),
        };
        if args.len() != want {
            self.err_s(span, code::ARITY, format!("{key}는 대상 {want}개를 받음 ({}개 줌)", args.len()), form);
            return Ty::Unknown;
        }
        // Target.
        let tspan = args[0].span;
        let ExprKind::Name { name: tname, kind: tk } = &mut args[0].kind else {
            self.err_s(tspan, code::TARGET, format!("{key}의 첫 칸은 함수 이름"), form);
            return Ty::Unknown;
        };
        let target = tname.key.clone();
        let Some(f) = self.state.functions.get(&target).cloned() else {
            if !self.failed.contains(&target) {
                self.err(tspan, code::UNDECLARED, format!("함수 '{target}'가 정의되지 않음"));
            }
            return Ty::Unknown;
        };
        let ok_kind = match kind {
            FeatureKind::Tangents => matches!(f.kind, DefKind::Parametric | DefKind::Polar),
            _ => f.kind == DefKind::Curve,
        };
        if f.is_surface() {
            self.unsupported(tspan, &format!("곡면 '{target}'의 특징점"), 6);
            return Ty::Unknown;
        }
        if f.family.is_some() || !ok_kind {
            let want_word = if kind == FeatureKind::Tangents { "매개변수·극좌표 함수" } else { "일반 함수(f(x) -> y)" };
            let w = if f.family.is_some() { "반복 정의" } else { f.kind.word() };
            let sug = if kind != FeatureKind::Tangents && matches!(f.kind, DefKind::Parametric | DefKind::Polar) { format!("tangents({target})") } else { form };
            self.err_s(tspan, code::TYPE, format!("{key}의 대상은 {want_word} ('{target}'는 {w})"), sug);
            return Ty::Unknown;
        }
        *tk = NameKind::Function;
        args[0].ty = f.ty.clone();
        info.callees.insert(target.clone());
        let input = f.inputs[0].clone();
        let (polar, axes) = f.system().map(|(p, a)| (p, a.into_iter().cloned().collect::<Vec<_>>())).unwrap();

        // Second curve of `intersect`: a curve on the same axes, or `y = <expr in x>`.
        let mut other = None;
        if kind == FeatureKind::Intersect {
            let (oi, oo) = f.curve_axes().map(|(i, o)| (i.key.clone(), o.key.clone())).unwrap();
            let a = &mut args[1];
            let aspan = a.span;
            match &mut a.kind {
                ExprKind::Name { name, kind: nk } if self.state.functions.contains_key(&name.key) => {
                    let g = self.state.functions[&name.key].clone();
                    if g.kind != DefKind::Curve || g.family.is_some() || g.is_surface() {
                        self.err(aspan, code::TYPE, format!("intersect의 둘째 칸은 일반 함수나 '{oo} = 식' ('{}'는 {})", name.key, g.kind.word()));
                        return Ty::Unknown;
                    }
                    if g.curve_axes().map(|(i, o)| (i.key.clone(), o.key.clone())) != Some((oi.clone(), oo.clone())) {
                        let (gi, go) = g.curve_axes().map(|(i, o)| (i.key.clone(), o.key.clone())).unwrap();
                        self.err(aspan, code::AXES, format!("'{}'({gi} -> {go})는 '{target}'({oi} -> {oo})와 축이 다름", name.key));
                        return Ty::Unknown;
                    }
                    *nk = NameKind::Function;
                    info.callees.insert(name.key.clone());
                    let x = Expr { id: 0, ty: Ty::Scalar, span: Span::default(), kind: ExprKind::Name { name: input.clone(), kind: NameKind::Input } };
                    other = Some(Expr { id: 0, ty: Ty::Scalar, span: Span::default(), kind: ExprKind::Call { callee: name.clone(), kind: NameKind::Function, args: vec![x] } });
                }
                ExprKind::Compare { first, rest } if rest.len() == 1 && rest[0].op == CmpOp::Eq => {
                    let lhs_ok = matches!(&first.kind, ExprKind::Name { name, .. } if name.key == oo);
                    if !lhs_ok {
                        let rhs = self.src[rest[0].rhs.span.range()].to_string();
                        self.err_s(first.span, code::AXES, format!("'=' 왼쪽은 '{target}'의 출력 축 {oo}"), format!("intersect({target}, {oo} = {rhs})"));
                        return Ty::Unknown;
                    }
                    let mut rhs = rest[0].rhs.clone();
                    let t = self.resolve_feature_expr(&mut rhs, &oi, sc, info);
                    if t != Ty::Scalar {
                        if t != Ty::Unknown {
                            self.err(rhs.span, code::TYPE, format!("'{oo} = ' 오른쪽은 {oi}의 식 (이 식은 {})", t.describe()));
                        }
                        return Ty::Unknown;
                    }
                    rest[0].rhs = rhs.clone();
                    other = Some(rhs);
                }
                ExprKind::Name { name, .. } if !self.failed.contains(&name.key) => {
                    self.err(aspan, code::UNDECLARED, format!("함수 '{}'가 정의되지 않음", name.key));
                    return Ty::Unknown;
                }
                ExprKind::Name { .. } => return Ty::Unknown,
                _ => {
                    self.err_s(aspan, code::TARGET, format!("intersect의 둘째 칸은 함수 이름이나 '{oo} = 식'"), format!("intersect({target}, {oo} = {oi})"));
                    return Ty::Unknown;
                }
            }
        }

        // Range over the target's input.
        let mut region = None;
        let (mut lo, mut hi) = (None, None);
        if let Some(r) = range {
            let rspan = r.span;
            let t = self.resolve_feature_expr(r, &input.key, sc, info);
            match t {
                Ty::Region => {}
                Ty::Unknown => return Ty::Unknown,
                _ => {
                    self.err_s(rspan, code::RANGE, format!("범위 칸에는 {} 부등식이 와야 함", input.key), format!("-1 < {} < 1", input.key));
                    return Ty::Unknown;
                }
            }
            (lo, hi) = bounds(r, &input.key);
            region = Some((**r).clone());
        }
        if matches!(kind, FeatureKind::Argmax | FeatureKind::Argmin) && (lo.is_none() || hi.is_none()) {
            let at = range.as_ref().map_or(span, |r| r.span);
            self.err_s(at, code::RANGE, format!("{key}에는 양 끝이 있는 구간이 필요함 (구간 위 최댓값·최솟값)"), format!("{key}({target}; 0 <= {} <= 2)", input.key));
            return Ty::Unknown;
        }

        // Filter word.
        let filter = match filter {
            None => None,
            Some(w) if filters.contains(&w.as_str()) => Some(w.clone()),
            Some(w) => {
                let msg = if filters.is_empty() { format!("{key}에는 종류 칸이 없음") } else { format!("{key}의 종류는 {} 중 하나 ('{w}'는 없음)", filters.join(", ")) };
                let sug = if filters.is_empty() { format!("{key}({target})") } else { format!("{key}({target};; {})", filters[0]) };
                self.err_s(span, code::FEATURE, msg, sug);
                return Ty::Unknown;
            }
        };

        // Constants written in the slots must be in the head (`E(; a)`); the target's are
        // checked by the referenced-definition rule.
        let missing: Vec<String> = info.used_consts.iter().filter(|c| !sc.consts.contains(c)).cloned().collect();
        if !missing.is_empty() {
            self.err_s(span, code::MISSING_CONST, format!("범위·식 칸에 쓴 상수를 적지 않음 ({})", missing.join(", ")), sc.signature_with(&missing));
            return Ty::Unknown;
        }

        let clean = |e: Option<Expr>| {
            e.map(|mut e| {
                e.assign_ids();
                e.clear_spans();
                e
            })
        };
        self.pending_feature = Some(FeatureSpec {
            kind,
            target,
            other: clean(other),
            range: clean(region),
            lo: clean(lo),
            hi: clean(hi),
            filter,
            polar,
            axes,
            candidates: vec![],
        });
        let ty = Ty::Vector { n: 2 };
        body.ty = ty.clone();
        ty
    }

    /// An expression in the target's input (range slot, `y = ...`). Any existing constant
    /// resolves; the caller checks that the head lists it.
    fn resolve_feature_expr(&mut self, e: &mut Expr, input: &str, sc: &Scope, info: &mut DefInfo) -> Ty {
        let mut rs = Scope::range(&[input]);
        rs.consts = sc.consts.clone();
        self.resolve(e, &mut rs, info)
    }

    /// After every definition is in place: the target's coordinate system and break
    /// candidates follow the current target (it may have changed in this request).
    pub(super) fn refresh_features(&mut self) {
        let keys: Vec<String> = self.state.functions.values().filter(|f| f.feature.is_some()).map(|f| f.key.clone()).collect();
        for k in keys {
            let spec = self.state.functions[&k].feature.clone().unwrap();
            let Some(t) = self.state.functions.get(&spec.target) else { continue };
            let ok = match spec.kind {
                FeatureKind::Tangents => matches!(t.kind, DefKind::Parametric | DefKind::Polar),
                _ => t.kind == DefKind::Curve,
            } && t.family.is_none()
                && t.inputs.first().is_some_and(|i| spec.range.is_none() || uses_only_input(spec.range.as_ref().unwrap(), &i.key));
            if !ok {
                self.err(Span::new(0, 0), code::TYPE, format!("특징점 '{k}'의 대상 '{}'가 맞지 않게 바뀜 ({}) — {k}도 함께 고치거나 del {k}", spec.target, t.kind.word()));
                continue;
            }
            let (polar, axes) = t.system().map(|(p, a)| (p, a.into_iter().cloned().collect::<Vec<_>>())).unwrap();
            // argmax / argmin check breaks inside the interval (no max past an asymptote).
            let candidates = if matches!(spec.kind, FeatureKind::Breaks | FeatureKind::Argmax | FeatureKind::Argmin) { self.break_candidates(&spec.target) } else { vec![] };
            let f = self.state.functions.get_mut(&k).unwrap();
            let s = f.feature.as_mut().unwrap();
            s.polar = polar;
            s.axes = axes;
            s.candidates = candidates;
        }
    }

    /// Derivative orders feature points need, per target (`FeatureKind::deriv_order`),
    /// unless Claude defined every one of them.
    pub(super) fn feature_needs(&self) -> Vec<(String, u32)> {
        let mut out = vec![];
        for f in self.state.functions.values() {
            let Some(s) = &f.feature else { continue };
            let n = s.kind.deriv_order();
            if n > 0 && (1..=n).any(|k| !self.state.functions.contains_key(&format!("{}@{k}", s.target))) {
                out.push((s.target.clone(), n));
            }
        }
        out
    }

    /// Candidate expressions from the target's AST, callees inlined (their input replaced
    /// by the argument), each in the target's input.
    pub(super) fn break_candidates(&self, target: &str) -> Vec<BreakCandidate> {
        let f = &self.state.functions[target];
        let mut out: Vec<BreakCandidate> = vec![];
        let mut seen: Vec<String> = vec![];
        self.collect_breaks(&f.body, &HashMap::new(), 0, &mut out, &mut seen);
        for c in &mut out {
            c.expr.assign_ids();
        }
        out
    }

    fn collect_breaks(&self, e: &Expr, subst: &HashMap<String, Expr>, depth: u32, out: &mut Vec<BreakCandidate>, seen: &mut Vec<String>) {
        if out.len() >= MAX_CANDIDATES {
            return;
        }
        let mut push = |cause: BreakCause, x: &Expr, out: &mut Vec<BreakCandidate>| {
            let mut x = substitute(x, subst);
            while let ExprKind::Paren { inner } = x.kind {
                x = *inner;
            }
            if !mentions_input(&x) {
                return;
            }
            let text = format!("{cause:?} {}", crate::print::expr_dsl(&x));
            if out.len() < MAX_CANDIDATES && !seen.contains(&text) {
                seen.push(text);
                out.push(BreakCandidate { cause, expr: x });
            }
        };
        let num = |v: i64| Expr { id: 0, ty: Ty::Scalar, span: Span::default(), kind: ExprKind::Num { text: v.to_string(), int: true } };
        let minus = |a: &Expr, b: Expr| Expr { id: 0, ty: Ty::Scalar, span: Span::default(), kind: ExprKind::Sum { terms: vec![Term { id: 0, sign: Sign::Plus, expr: a.clone() }, Term { id: 0, sign: Sign::Minus, expr: b }] } };
        let plus = |a: &Expr, b: Expr| Expr { id: 0, ty: Ty::Scalar, span: Span::default(), kind: ExprKind::Sum { terms: vec![Term { id: 0, sign: Sign::Plus, expr: a.clone() }, Term { id: 0, sign: Sign::Plus, expr: b }] } };
        match &e.kind {
            ExprKind::Div { den, .. } if !den.ty.is_array() => push(BreakCause::Denominator, den, out),
            ExprKind::Pow { base, exp } => {
                if let Some((p, q)) = literal_exponent(exp) {
                    if p < 0 {
                        push(BreakCause::Denominator, base, out);
                    } else if q % 2 == 0 {
                        push(BreakCause::Domain, base, out);
                    }
                }
            }
            ExprKind::Call { callee, kind: NameKind::Builtin, args } if args.len() <= 2 && !args.is_empty() => match callee.key.as_str() {
                "sqrt" | "ln" | "log10" | "log2" | "log" => push(BreakCause::Domain, &args[0], out),
                "abs" => push(BreakCause::Abs, &args[0], out),
                "sign" => push(BreakCause::Boundary, &args[0], out),
                // Steps where the argument crosses an integer (round: a half-integer;
                // mod(u, m): u/m crosses an integer).
                "floor" | "ceil" => push(BreakCause::Step, &args[0], out),
                "round" => {
                    let half = Expr { id: 0, ty: Ty::Scalar, span: Span::default(), kind: ExprKind::Num { text: "0.5".into(), int: false } };
                    push(BreakCause::Step, &minus(&args[0], half), out);
                }
                "mod" if args.len() == 2 => {
                    let q = Expr { id: 0, ty: Ty::Scalar, span: Span::default(), kind: ExprKind::Div { num: Box::new(args[0].clone()), den: Box::new(args[1].clone()) } };
                    push(BreakCause::Step, &q, out);
                }
                "tan" => {
                    let c = Expr { id: 0, ty: Ty::Scalar, span: Span::default(), kind: ExprKind::Call { callee: Ident::plain("cos"), kind: NameKind::Builtin, args: vec![args[0].clone()] } };
                    push(BreakCause::Denominator, &c, out);
                }
                "asin" | "acos" => {
                    push(BreakCause::Domain, &minus(&args[0], num(1)), out);
                    push(BreakCause::Domain, &plus(&args[0], num(1)), out);
                }
                _ => {}
            },
            ExprKind::Call { callee, kind: NameKind::Function, args } if args.len() == 1 && depth < MAX_INLINE => {
                if let Some(g) = self.state.functions.get(&callee.key).filter(|g| matches!(g.kind, DefKind::Curve | DefKind::Polar) && g.family.is_none() && g.inputs.len() == 1) {
                    let mut inner = HashMap::new();
                    inner.insert(g.inputs[0].key.clone(), substitute(&args[0], subst));
                    self.collect_breaks(&g.body, &inner, depth + 1, out, seen);
                }
            }
            ExprKind::Split { subject, at, pieces, .. } => {
                for b in at {
                    push(BreakCause::Boundary, &minus(subject, b.clone()), out);
                }
                // Inside the pieces the split's name stands for the subject.
                let mut inner = subst.clone();
                inner.insert(SPLIT_NAME.into(), substitute(subject, subst));
                self.collect_breaks(subject, subst, depth, out, seen);
                for p in pieces {
                    self.collect_breaks(&p.value, &inner, depth, out, seen);
                }
                return;
            }
            // Index-dependent terms are not expressions in the input alone.
            ExprKind::BigOp { .. } => return,
            _ => {}
        }
        for c in e.children() {
            self.collect_breaks(c, subst, depth, out, seen);
        }
    }
}

/// Key under which `substitute` replaces split-name references.
const SPLIT_NAME: &str = "\u{0}split";

/// Replaces input names (and split names, under `SPLIT_NAME`) by expressions.
fn substitute(e: &Expr, subst: &HashMap<String, Expr>) -> Expr {
    let mut out = e.clone();
    if subst.is_empty() {
        return out;
    }
    fn go(e: &mut Expr, subst: &HashMap<String, Expr>) {
        match &e.kind {
            ExprKind::Name { name, kind: NameKind::Input } => {
                if let Some(r) = subst.get(&name.key) {
                    *e = paren(r.clone());
                }
                return;
            }
            ExprKind::Name { kind: NameKind::SplitName, .. } => {
                if let Some(r) = subst.get(SPLIT_NAME) {
                    *e = paren(r.clone());
                }
                return;
            }
            _ => {}
        }
        for c in e.children_mut() {
            go(c, subst);
        }
    }
    go(&mut out, subst);
    out
}

/// Parentheses around a compound replacement so printing keeps its grouping.
fn paren(e: Expr) -> Expr {
    match e.kind {
        ExprKind::Num { .. } | ExprKind::Name { .. } | ExprKind::Call { .. } | ExprKind::Paren { .. } => e,
        _ => Expr { id: 0, ty: e.ty.clone(), span: Span::default(), kind: ExprKind::Paren { inner: Box::new(e) } },
    }
}

fn mentions_input(e: &Expr) -> bool {
    let mut found = false;
    e.walk(&mut |n| found |= matches!(n.kind, ExprKind::Name { kind: NameKind::Input | NameKind::SplitName, .. }));
    found
}

pub(super) fn uses_only_input(e: &Expr, input: &str) -> bool {
    let mut ok = true;
    e.walk(&mut |n| {
        if let ExprKind::Name { name, kind: NameKind::Input } = &n.kind {
            ok &= name.key == input;
        }
    });
    ok
}

/// `p/q` (or an integer p, q = 1) written as literals.
fn literal_exponent(e: &Expr) -> Option<(i64, i64)> {
    fn int(e: &Expr) -> Option<i64> {
        match &e.kind {
            ExprKind::Num { text, int: true } => text.parse().ok(),
            ExprKind::Paren { inner } => int(inner),
            ExprKind::Neg { arg } => int(arg).map(|v| -v),
            _ => None,
        }
    }
    match &e.kind {
        ExprKind::Paren { inner } => literal_exponent(inner),
        ExprKind::Neg { arg } => literal_exponent(arg).map(|(p, q)| (-p, q)),
        ExprKind::Div { num, den } => Some((int(num)?, int(den)?)).filter(|(_, q)| *q != 0).map(|(p, q)| if q < 0 { (-p, -q) } else { (p, q) }),
        _ => int(e).map(|p| (p, 1)),
    }
}

/// Search interval from a range: `lo < x < hi`, `x > lo`, `and` of those. Bounds are the
/// sides that do not mention the input. Anything else (`or`, `not`) gives no bounds.
pub(super) fn bounds(r: &Expr, input: &str) -> (Option<Expr>, Option<Expr>) {
    let is_x = |e: &Expr| matches!(&e.kind, ExprKind::Name { name, kind: NameKind::Input } if name.key == input);
    let free = |e: &Expr| !mentions_input(e);
    match &r.kind {
        ExprKind::Compare { first, rest } => {
            let mut items: Vec<&Expr> = vec![first];
            items.extend(rest.iter().map(|l| &l.rhs));
            let mut lo = None;
            let mut hi = None;
            for (i, l) in rest.iter().enumerate() {
                let (a, b) = (items[i], items[i + 1]);
                let up = matches!(l.op, CmpOp::Lt | CmpOp::Le);
                let down = matches!(l.op, CmpOp::Gt | CmpOp::Ge);
                // a < x: lower bound a; x < b: upper bound b (and mirrored for >).
                if is_x(b) && free(a) {
                    if up {
                        lo = Some(a.clone());
                    } else if down {
                        hi = Some(a.clone());
                    }
                }
                if is_x(a) && free(b) {
                    if up {
                        hi = Some(b.clone());
                    } else if down {
                        lo = Some(b.clone());
                    }
                }
                if l.op == CmpOp::Eq && (is_x(a) && free(b) || is_x(b) && free(a)) {
                    let v = if is_x(a) { b.clone() } else { a.clone() };
                    lo = Some(v.clone());
                    hi = Some(v);
                }
            }
            (lo, hi)
        }
        ExprKind::And { args } => {
            let mut lo = None;
            let mut hi = None;
            for a in args {
                let (l, h) = bounds(a, input);
                lo = lo.or(l);
                hi = hi.or(h);
            }
            (lo, hi)
        }
        ExprKind::Paren { inner } => bounds(inner, input),
        _ => (None, None),
    }
}
