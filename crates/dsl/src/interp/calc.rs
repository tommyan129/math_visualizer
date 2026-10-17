//! Calculation-steps definitions (stage 5c decisions): `C: calc(f, g, h)`,
//! `calc(f, g; x > 0)`, `calc(f, F;; integral)`. The server checks the steps, compares
//! neighbouring ASTs for the changed parts, and collects break candidates; the browser
//! verifies neighbours numerically.

use super::feature::{bounds, uses_only_input};
use super::*;
use crate::diff::diff;

impl<'s> Cx<'s> {
    /// Body `calc(steps; range; kind)`. Leaves the spec in `pending_calc`.
    pub(super) fn resolve_calc(&mut self, body: &mut Expr, sc: &mut Scope, info: &mut DefInfo) -> Ty {
        let span = body.span;
        let ExprKind::Calc { steps, range, kind } = &mut body.kind else { unreachable!("routed by check_shape") };
        if steps.len() < 2 {
            let one = steps.first().map_or("f".to_string(), |s| self.src[s.span.range()].to_string());
            self.err_s(span, code::ARITY, format!("calc에는 단계가 둘 이상 필요함 ({}개 줌)", steps.len()), format!("calc({one}, g)"));
            return Ty::Unknown;
        }

        // Steps: distinct curve (or polar) definitions on the same axes.
        let mut keys: Vec<String> = vec![];
        let mut first: Option<Function> = None;
        let mut ok = true;
        for st in steps.iter_mut() {
            let sspan = st.span;
            let ExprKind::Name { name, kind: nk } = &mut st.kind else {
                self.err_s(sspan, code::TARGET, "calc의 단계는 정의 이름 (식은 정의로 먼저 선언)", "g(x) -> y: ...");
                ok = false;
                continue;
            };
            let key = name.key.clone();
            let Some(f) = self.state.functions.get(&key).cloned() else {
                if !self.failed.contains(&key) {
                    self.err(sspan, code::UNDECLARED, format!("함수 '{key}'가 정의되지 않음"));
                }
                ok = false;
                continue;
            };
            if f.family.is_some() || !matches!(f.kind, DefKind::Curve | DefKind::Polar) {
                let w = if f.family.is_some() { "반복 정의" } else { f.kind.word() };
                self.err(sspan, code::TYPE, format!("calc의 단계는 일반 함수나 극좌표 함수 ('{key}'는 {w})"));
                ok = false;
                continue;
            }
            if keys.contains(&key) {
                self.err(sspan, code::DUPLICATE, format!("calc에 '{key}'를 두 번 적음"));
                ok = false;
                continue;
            }
            match &first {
                None => first = Some(f.clone()),
                Some(f0) if same_axes(f0, &f) => {}
                Some(f0) => {
                    self.err(sspan, code::AXES, format!("calc의 단계는 같은 축의 함수 ('{key}'는 {}, '{}'는 {})", f.signature(), f0.key, f0.signature()));
                    ok = false;
                    continue;
                }
            }
            *nk = NameKind::Function;
            st.ty = f.ty.clone();
            info.callees.insert(key.clone());
            keys.push(key);
        }
        if !ok {
            return Ty::Unknown;
        }
        let f0 = first.expect("two or more steps");
        let input = f0.inputs[0].clone();
        let (polar, axes) = f0.system().map(|(p, a)| (p, a.into_iter().cloned().collect::<Vec<_>>())).unwrap();

        // Range over the steps' input.
        let mut region = None;
        let (mut lo, mut hi) = (None, None);
        if let Some(r) = range {
            let rspan = r.span;
            let mut rs = Scope::range(&[&input.key]);
            rs.consts = sc.consts.clone();
            match self.resolve(r, &mut rs, info) {
                Ty::Region => {}
                Ty::Unknown => return Ty::Unknown,
                _ => {
                    self.err_s(rspan, code::RANGE, format!("범위 칸에는 {} 부등식이 와야 함", input.key), format!("{} > 0", input.key));
                    return Ty::Unknown;
                }
            }
            (lo, hi) = bounds(r, &input.key);
            region = Some((**r).clone());
        }

        let calc_kind = match kind {
            None => CalcKind::Transform,
            Some(w) => match CalcKind::parse(w) {
                Some(k) => k,
                None => {
                    let list = keys.join(", ");
                    self.err_s(span, code::CALC, format!("calc의 종류는 {} 중 하나 (비우면 식 변형; '{w}'는 없음)", names::CALC_KINDS.join(", ")), format!("calc({list};; integral)"));
                    return Ty::Unknown;
                }
            },
        };

        let missing: Vec<String> = info.used_consts.iter().filter(|c| !sc.consts.contains(c)).cloned().collect();
        if !missing.is_empty() {
            self.err_s(span, code::MISSING_CONST, format!("범위 칸에 쓴 상수를 적지 않음 ({})", missing.join(", ")), sc.signature_with(&missing));
            return Ty::Unknown;
        }

        let clean = |e: Option<Expr>| {
            e.map(|mut e| {
                e.assign_ids();
                e.clear_spans();
                e
            })
        };
        self.pending_calc = Some(CalcSpec {
            kind: calc_kind,
            steps: keys,
            input,
            range: clean(region),
            lo: clean(lo),
            hi: clean(hi),
            polar,
            axes,
            diffs: vec![],
            candidates: vec![],
        });
        body.ty = Ty::Scalar;
        Ty::Scalar
    }

    /// After every definition is in place: the steps may have changed in this request, so
    /// their axes are checked again and the diffs and break candidates recomputed.
    pub(super) fn refresh_calcs(&mut self) {
        let keys: Vec<String> = self.state.functions.values().filter(|f| f.calc.is_some()).map(|f| f.key.clone()).collect();
        for k in keys {
            let spec = self.state.functions[&k].calc.clone().unwrap();
            let steps: Vec<&Function> = spec.steps.iter().filter_map(|s| self.state.functions.get(s)).collect();
            if steps.len() != spec.steps.len() {
                continue;
            }
            let fits = |f: &Function| {
                f.family.is_none() && matches!(f.kind, DefKind::Curve | DefKind::Polar) && same_axes(steps[0], f) && spec.range.as_ref().is_none_or(|r| uses_only_input(r, &f.inputs[0].key))
            };
            if let Some(bad) = steps.iter().find(|f| !fits(f)) {
                self.err(Span::new(0, 0), code::TYPE, format!("계산 과정 '{k}'의 단계 '{}'가 맞지 않게 바뀜 ({}) — {k}도 함께 고치거나 del {k}", bad.key, bad.signature()));
                continue;
            }
            let diffs: Vec<StepDiff> = steps.windows(2).map(|w| diff(&w[0].body, &w[1].body)).collect();
            let (polar, axes) = steps[0].system().map(|(p, a)| (p, a.into_iter().cloned().collect::<Vec<_>>())).unwrap();
            let input = steps[0].inputs[0].clone();
            let mut candidates: Vec<BreakCandidate> = vec![];
            for s in &spec.steps {
                for c in self.break_candidates(s) {
                    if !candidates.contains(&c) {
                        candidates.push(c);
                    }
                }
            }
            let c = self.state.functions.get_mut(&k).unwrap().calc.as_mut().unwrap();
            c.diffs = diffs;
            c.polar = polar;
            c.axes = axes;
            c.input = input;
            c.candidates = candidates;
        }
    }
}

/// Same kind, same input, and (curves) the same output axis.
fn same_axes(a: &Function, b: &Function) -> bool {
    a.kind == b.kind && a.inputs.first().map(|i| &i.key) == b.inputs.first().map(|i| &i.key) && a.output.as_ref().map(|o| &o.key) == b.output.as_ref().map(|o| &o.key)
}
