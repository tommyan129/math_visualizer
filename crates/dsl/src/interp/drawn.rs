//! Stage 5 drawn definitions (design.md "정의 블록과 함수", "매개변수 함수"):
//! parametric `p(t) -> (x, y): (..), range`, polar `h(\theta) -> polar(r)`, and
//! equations `C(x, y; a): lhs = rhs`. Head checks, output slots, parameter ranges.

use super::*;

impl<'s> Cx<'s> {
    /// Head of a parametric / polar function, an equation, or a body with a range.
    /// `None` = none of these (the stage 1–4 shapes).
    pub(super) fn check_shape_drawn<'d>(&mut self, def: &'d Def) -> Option<Option<Shape<'d>>> {
        let inputs: &[(Ident, Span)] = def.params.as_ref().map_or(&[], |p| &p.inputs);
        let consts: &[(Ident, Span)] = def.params.as_ref().map_or(&[], |p| &p.consts);
        let head = def.params.as_ref().map_or(def.name_span, |p| def.name_span.to(p.span));
        let head_src = self.src[head.range()].to_string();
        let t = inputs.first().map_or("t", |(i, _)| i.key.as_str());
        let (expr, range) = match &def.body {
            DefBody::Expr(e) => (Some(e), None),
            DefBody::WithRange { expr, range } => (Some(expr), Some(range)),
            DefBody::Equation { .. } => (None, None),
        };
        let fail = Some(None);
        let (kind, output, outputs) = match (&def.output, &def.body) {
            (None, DefBody::Equation { .. }) => (DefKind::Equation, None, vec![]),
            (Some(o), DefBody::Equation { lhs, rhs }) => {
                let eq = format!("{} = {}", &self.src[lhs.span.range()], &self.src[rhs.span.range()]);
                self.err_s(o.span(), code::ARROW, "방정식에는 '->' 출력 축이 없음 (입력 축이 곧 좌표축)", format!("{head_src}: {eq}"));
                return fail;
            }
            (Some(Output::Tuple(axes, span)), _) => {
                if axes.len() == 3 {
                    self.unsupported(*span, "3D 매개변수 곡선", 6);
                    return fail;
                }
                if axes.len() != 2 {
                    self.err_s(*span, code::AXES, format!("출력 축은 두 개 (준 개수 {}개)", axes.len()), "-> (x, y)");
                    return fail;
                }
                if let (None, Some(expr)) = (range, expr) {
                    let body = &self.src[expr.span.range()];
                    self.err_s(expr.span, code::RANGE, "매개변수 함수에는 매개변수 범위가 필요함 (축이 없어 어디까지 그릴지 정할 수 없음)", format!("{head_src} -> ({}): {body}, 0 <= {t} <= 2*\\pi", axes.iter().map(|(a, _)| a.key.as_str()).collect::<Vec<_>>().join(", ")));
                    return fail;
                }
                (DefKind::Parametric, None, axes.iter().map(|(a, _)| a.clone()).collect())
            }
            (Some(Output::Coord { system, axes, span }), _) => {
                if system.key != "polar" {
                    self.err_s(*span, code::AXES, format!("'{}(..)' 출력은 없음", system.key), "-> polar(r)");
                    return fail;
                }
                if axes.len() != 1 {
                    self.err_s(*span, code::AXES, format!("극좌표 출력 축은 하나 (준 개수 {}개)", axes.len()), "-> polar(r)");
                    return fail;
                }
                (DefKind::Polar, Some(&axes[0].0), vec![])
            }
            (_, DefBody::WithRange { range, .. }) => {
                self.err_s(range.span, code::RANGE, "범위는 매개변수·극좌표 함수에만 붙음", format!("{head_src} -> (x, y): (t, f(t)), 0 <= t <= 1"));
                return fail;
            }
            _ => return None,
        };

        // Inputs: one parameter, or two axes for an equation.
        let want = if kind == DefKind::Equation { 2 } else { 1 };
        if inputs.len() != want {
            match (kind, inputs.len()) {
                (DefKind::Equation, 3) => self.unsupported(head, "3D 음함수 곡면", 6),
                (DefKind::Parametric, 2) => self.unsupported(head, "매개변수 곡면", 6),
                (DefKind::Equation, n) => self.err_s(head, code::AXES, format!("방정식의 입력은 좌표축 두 개 (준 개수 {n}개)"), format!("{}(x, y)", def.name.key)),
                (_, 0) => self.err_s(head, code::SYNTAX, "입력 변수가 없음", format!("{}(t)", def.name.key)),
                (k, n) => self.err(head, code::AXES, format!("{}의 입력은 하나 (준 개수 {n}개)", k.word())),
            }
            return fail;
        }
        // Axis names: distinct, usable as axes.
        let mut all: Vec<(&Ident, Span)> = inputs.iter().map(|(i, s)| (i, *s)).collect();
        let out_span = def.output.as_ref().map_or(def.name_span, |o| o.span());
        all.extend(output.map(|o| (o, out_span)));
        all.extend(outputs.iter().map(|o| (o, out_span)));
        for (i, (a, sp)) in all.iter().enumerate() {
            if a.deriv > 0 || a.inverse || is_reserved(a) {
                self.err(*sp, code::AXES, format!("'{}'는 축 이름으로 쓸 수 없음", a.key));
                return fail;
            }
            if all[..i].iter().any(|(b, _)| b.key == a.key) {
                self.err(*sp, code::AXES, format!("축 이름 '{}'를 두 번 씀", a.key));
                return fail;
            }
        }

        let body = match &def.body {
            DefBody::Equation { lhs, rhs } => Expr::new(
                ExprKind::Compare { first: Box::new(lhs.clone()), rest: vec![CompareLink { op: CmpOp::Eq, rhs: rhs.clone() }] },
                lhs.span.to(rhs.span),
            ),
            _ => expr.expect("not an equation").clone(),
        };
        Some(Some(Shape { kind, inputs, consts, output, outputs, body, range }))
    }

    /// `(f(t), g(t))` with one number per output axis, or a vector expression of that
    /// length (`matmul(R(t), [1, 0])`), or a whole parametric call (`p(2*t)`).
    pub(super) fn resolve_parametric(&mut self, body: &mut Expr, n: u32, sc: &mut Scope, info: &mut DefInfo) -> Ty {
        if let ExprKind::Tuple { items } = &mut body.kind {
            if items.len() as u32 != n {
                self.err(body.span, code::SHAPE, format!("출력 축은 {n}개인데 출력 칸은 {}개", items.len()));
                return Ty::Unknown;
            }
            let mut ok = true;
            sc.in_slot = true;
            for it in items.iter_mut() {
                let t = self.resolve(it, sc, info);
                match t {
                    Ty::Scalar => {}
                    Ty::Unknown => ok = false,
                    Ty::Vector { .. } => {
                        let v = &self.src[it.span.range()];
                        self.err_s(it.span, code::TYPE, format!("출력 칸 하나에는 숫자를 넣음 (이 식은 {})", t.describe()), format!("벡터 하나를 출력 전체로: {v}"));
                        ok = false;
                    }
                    t => {
                        self.expect_scalar(&t, it.span);
                        ok = false;
                    }
                }
            }
            sc.in_slot = false;
            let ty = if ok { Ty::Tuple { n } } else { Ty::Unknown };
            body.ty = ty.clone();
            return ty;
        }
        sc.tuple_ok = true;
        let t = self.resolve(body, sc, info);
        sc.tuple_ok = false;
        match t {
            Ty::Unknown => Ty::Unknown,
            Ty::Vector { n: m } | Ty::Tuple { n: m } if m == n => t,
            Ty::Vector { n: m } | Ty::Tuple { n: m } => {
                self.err(body.span, code::SHAPE, format!("출력 축은 {n}개인데 이 식은 {}", if matches!(t, Ty::Vector { .. }) { format!("길이 {m} 벡터") } else { format!("출력 {m}개") }));
                Ty::Unknown
            }
            t => {
                let b = &self.src[body.span.range()];
                self.err_s(body.span, code::TYPE, format!("출력 축이 {n}개라 출력도 {n}개여야 함 (이 식은 {})", t.describe()), format!("({b}, ...) 또는 길이 {n} 벡터"));
                Ty::Unknown
            }
        }
    }

    /// `lhs = rhs`, both numbers.
    pub(super) fn resolve_equation(&mut self, body: &mut Expr, sc: &mut Scope, info: &mut DefInfo) -> Ty {
        let ExprKind::Compare { first, rest } = &mut body.kind else { unreachable!() };
        let mut ok = true;
        for e in std::iter::once(&mut **first).chain(rest.iter_mut().map(|l| &mut l.rhs)) {
            let t = self.resolve(e, sc, info);
            match t {
                Ty::Scalar => {}
                Ty::Unknown => ok = false,
                Ty::Region => {
                    self.err(e.span, code::TYPE, "방정식의 양쪽은 숫자 식이어야 함 (부등식은 대상 없는 highlight로)");
                    ok = false;
                }
                t => {
                    self.expect_scalar(&t, e.span);
                    ok = false;
                }
            }
        }
        let ty = if ok { Ty::Region } else { Ty::Unknown };
        body.ty = ty.clone();
        ty
    }

    /// `lo <= t <= hi` (`<` also accepted, or written from the top with `>=` / `>`).
    /// The bounds may use constants and calls, never the parameter.
    pub(super) fn resolve_param_range(&mut self, range: &Expr, sc: &mut Scope, info: &mut DefInfo) -> Option<ParamRange> {
        let t = sc.inputs[0].clone();
        let src = self.src[range.span.range()].to_string();
        let sug = format!("0 <= {t} <= 2*\\pi");
        let shape = match &range.kind {
            ExprKind::Compare { first, rest } if rest.len() == 2 => {
                let is_t = matches!(&rest[0].rhs.kind, ExprKind::Name { name, .. } if name.key == t);
                let up = rest.iter().all(|l| matches!(l.op, CmpOp::Lt | CmpOp::Le));
                let down = rest.iter().all(|l| matches!(l.op, CmpOp::Gt | CmpOp::Ge));
                match (is_t, up, down) {
                    (true, true, _) => Some(((**first).clone(), rest[1].rhs.clone())),
                    (true, _, true) => Some((rest[1].rhs.clone(), (**first).clone())),
                    _ => None,
                }
            }
            _ => None,
        };
        let Some((mut lo, mut hi)) = shape else {
            self.err_s(range.span, code::RANGE, format!("매개변수 범위는 '아래 <= {t} <= 위' 모양"), sug);
            return None;
        };
        let mut ok = true;
        for b in [&mut lo, &mut hi] {
            let ty = self.resolve(b, sc, info);
            self.expect_scalar(&ty, b.span);
            ok &= ty == Ty::Scalar;
            let mut uses_t = false;
            b.walk(&mut |n| uses_t |= matches!(&n.kind, ExprKind::Name { kind: NameKind::Input, .. }));
            if uses_t {
                self.err_s(b.span, code::RANGE, format!("범위 끝에는 매개변수 {t}를 쓸 수 없음 (상수와 숫자로)"), sug.clone());
                ok = false;
            }
        }
        if !ok {
            return None;
        }
        for b in [&mut lo, &mut hi] {
            b.assign_ids();
            b.clear_spans();
        }
        Some(ParamRange { lo, hi, src })
    }

    /// `family(...; lo <= k <= hi)` head: allowed kind, index name. Puts the index in scope
    /// (the caller pops it after the body and range). False when the index name clashes.
    /// The index is pushed in every case.
    pub(super) fn open_family(&mut self, h: &FamilyHead, kind: DefKind, sc: &mut Scope) -> bool {
        if !matches!(kind, DefKind::Curve | DefKind::Parametric | DefKind::Polar | DefKind::Equation) {
            // Reported once; the index still goes in scope so the body adds no errors.
            self.err(h.range_span, code::TYPE, format!("family는 그리는 함수(곡선, 매개변수, 극좌표, 방정식)에만 씀 ({}에는 쓸 수 없음)", kind.word()));
            sc.indices.push(h.index.key.clone());
            return true;
        }
        let k = &h.index.key;
        let clash = sc.inputs.contains(k)
            || sc.consts.contains(k)
            || sc.outputs.contains(k)
            || *k == sc.def_key
            || self.is_function_name(k)
            || self.state.constants.contains_key(k)
            || names::is_builtin(k)
            || is_reserved(&h.index);
        if clash {
            self.err(h.range_span, code::NAME_CONFLICT, format!("인덱스 '{k}'가 다른 이름과 겹침"));
        }
        // In scope either way, so the body adds no errors of its own.
        sc.indices.push(k.clone());
        !clash
    }

    /// Bounds of the index: integer-valued, constants and numbers only.
    pub(super) fn resolve_family(&mut self, h: &FamilyHead, sc: &mut Scope, info: &mut DefInfo) -> Option<FamilyIndex> {
        let (mut lo, mut hi) = (h.lo.clone(), h.hi.clone());
        let mut ok = true;
        info.in_bound += 1;
        for b in [&mut lo, &mut hi] {
            let t = self.resolve(b, sc, info);
            self.expect_scalar(&t, b.span);
            ok &= t == Ty::Scalar;
            let mut uses_input = false;
            b.walk(&mut |n| uses_input |= matches!(&n.kind, ExprKind::Name { kind: NameKind::Input, .. }));
            if uses_input {
                self.err_s(b.span, code::BIG_OP_RANGE, "family 범위에는 입력 변수를 쓸 수 없음 (숫자와 상수만)", "family(식; 0 <= k <= n)");
                ok = false;
            }
        }
        info.in_bound -= 1;
        if !ok {
            return None;
        }
        for b in [&mut lo, &mut hi] {
            b.assign_ids();
            b.clear_spans();
        }
        Some(FamilyIndex { index: h.index.clone(), lo, lo_strict: h.lo_strict, hi, hi_strict: h.hi_strict, src: self.src[h.range_span.range()].to_string() })
    }
}
