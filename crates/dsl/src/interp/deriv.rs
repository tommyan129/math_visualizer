//! Named derivatives `f@n` and inverses `~f` (design.md "f@n", stage-5b decisions):
//! head checks, calls, and the server-generated derivatives (`Function::derivs`).

use std::collections::BTreeMap;

use super::*;
use crate::deriv::{self as d, MAX_ORDER, base_ident};

/// `f@2` → ("f", 2); `~f` → ("f", 0); `None` for a plain name.
pub(super) fn related(id: &Ident) -> Option<(String, u32)> {
    (id.deriv > 0 || id.inverse).then(|| (base_ident(id).key, id.deriv))
}

/// `f@n` written in DSL (`f` for order 0).
fn deriv_key(base: &str, n: u32) -> String {
    if n == 0 { base.to_string() } else { format!("{base}@{n}") }
}

/// Kinds that get a derivative by the rules: one input and a body that is a value.
fn derivable(f: &Function) -> bool {
    f.family.is_none() && f.inputs.len() == 1 && matches!(f.kind, DefKind::Curve | DefKind::Polar | DefKind::Parametric | DefKind::Matrix)
}

impl<'s> Cx<'s> {
    /// Head of `f@n(x) -> y` / `~f(x) -> y`: the base `f` must be a curve (decision 5b).
    /// The axes are compared once the shape is known (`check_related_axes`).
    pub(super) fn check_related_head(&mut self, def: &Def) -> bool {
        let id = &def.name;
        let base = base_ident(id).key;
        if id.deriv > 0 && id.inverse {
            self.err(def.name_span, code::NAME_CONFLICT, format!("'{}'처럼 역함수와 도함수를 함께 쓴 이름은 없음 (역함수의 도함수는 다른 이름으로 정의)", id.key));
            return false;
        }
        if id.deriv > MAX_ORDER {
            self.err(def.name_span, code::LIMIT, format!("미분 횟수가 너무 큼 ({}, 상한 {MAX_ORDER})", id.deriv));
            return false;
        }
        let what = if id.inverse { "역함수" } else { "도함수" };
        match self.state.functions.get(&base) {
            None if self.failed.contains(&base) => false,
            None => {
                self.err_s(def.name_span, code::UNDECLARED, format!("{what} '{}'의 원래 함수 '{base}'가 없음", id.key), format!("{base}(x) -> y: ... 를 먼저 정의"));
                false
            }
            Some(f) if f.kind != DefKind::Curve || f.family.is_some() || f.is_surface() => {
                let w = if f.family.is_some() { "반복 정의" } else if f.is_surface() { "곡면" } else { f.kind.word() };
                self.err(def.name_span, code::TYPE, format!("{what} 이름은 일반 함수(f(x) -> y)에만 붙음 ('{base}'는 {w})"));
                false
            }
            Some(_) => true,
        }
    }

    /// `f@n` / `~f` are drawn on f's axes: same input and output.
    pub(super) fn check_related_axes(&mut self, def: &Def, shape: &Shape) -> bool {
        let base = base_ident(&def.name).key;
        let f = &self.state.functions[&base];
        let (i, o) = f.curve_axes().expect("checked to be a curve");
        let head = def.params.as_ref().map_or(def.name_span, |p| def.name_span.to(p.span));
        let head = def.output.as_ref().map_or(head, |o| head.to(o.span()));
        let sig = format!("{}({}{}) -> {}", def.name.key, i.key, if shape.consts.is_empty() { String::new() } else { format!("; {}", shape.consts.iter().map(|(c, _)| c.key.as_str()).collect::<Vec<_>>().join(", ")) }, o.key);
        let same = shape.kind == DefKind::Curve && shape.inputs.len() == 1 && shape.inputs[0].0.key == i.key && shape.output.is_some_and(|out| out.key == o.key);
        if !same {
            self.err_s(head, code::AXES, format!("'{}'는 '{base}'와 같은 축으로 씀 ({} -> {})", def.name.key, i.key, o.key), sig);
        }
        same
    }

    /// A call of `f@n` / `~f`. Claude's definition when there is one (handled by the
    /// caller); otherwise `f@n` is the generated derivative of `f` (decision 5b) and `~f`
    /// is an error. Returns None when `callee` is a plain name.
    pub(super) fn resolve_related_call(&mut self, callee: &Ident, argc: usize, span: Span, sc: &Scope, info: &mut DefInfo) -> Option<(NameKind, Ty)> {
        let (base, n) = related(callee)?;
        if self.state.functions.contains_key(&callee.key) {
            return None;
        }
        let key = &callee.key;
        if callee.inverse {
            if callee.deriv > 0 {
                self.err(span, code::NAME_CONFLICT, format!("'{key}'처럼 역함수와 도함수를 함께 쓴 이름은 없음"));
            } else if !self.failed.contains(key) {
                self.err_s(span, code::UNDECLARED, format!("역함수 '{key}'가 정의되지 않음 (서버는 역함수를 만들지 않음)"), format!("{key}(x) -> y: ... 를 정의"));
            }
            return Some((NameKind::Unresolved, Ty::Unknown));
        }
        if n > MAX_ORDER {
            self.err(span, code::LIMIT, format!("미분 횟수가 너무 큼 ({n}, 상한 {MAX_ORDER})"));
            return Some((NameKind::Unresolved, Ty::Unknown));
        }
        if !sc.def_key.is_empty() && base == sc.def_key {
            self.err(span, code::CYCLE, format!("'{base}'가 자기 도함수 '{key}'를 부름 (도함수가 다시 '{base}'의 식을 따라감)"));
            return Some((NameKind::Unresolved, Ty::Unknown));
        }
        let Some(f) = self.state.functions.get(&base).cloned() else {
            if !self.failed.contains(&base) && info.reported.insert(key.clone()) {
                self.err(span, code::UNDECLARED, format!("함수 '{base}'가 정의되지 않음 ('{key}'는 {base}의 도함수)"));
            }
            return Some((NameKind::Unresolved, Ty::Unknown));
        };
        if f.kind == DefKind::Parametric || !derivable(&f) || !matches!(f.kind, DefKind::Curve | DefKind::Polar | DefKind::Matrix) {
            let w = if f.family.is_some() { "반복 정의" } else { f.kind.word() };
            self.err(span, code::TYPE, format!("'{key}'를 부를 수 없음 ('{base}'는 {w}라 식에서 쓰는 도함수가 없음)"));
            return Some((NameKind::Unresolved, Ty::Unknown));
        }
        if f.inputs.len() != argc {
            self.err(span, code::ARITY, format!("{key}는 인자 {}개를 받음 ({argc}개 줌)", f.inputs.len()));
        }
        // Depends on f (cascade, constants); the derivative itself is filled in `finish`.
        info.callees.insert(base.clone());
        self.deriv_sites.entry(base).or_insert(span);
        Some((NameKind::Function, f.ty.clone()))
    }

    /// After `f` changed: `f@n` / `~f` still need a curve f on the same axes.
    pub(super) fn check_related_defs(&mut self) {
        let mut errs = vec![];
        for g in self.state.functions.values() {
            let Some((base, _)) = related(&g.name) else { continue };
            let ok = match self.state.functions.get(&base) {
                Some(f) => f.kind == DefKind::Curve && f.family.is_none() && f.curve_axes().is_some() && f.curve_axes().map(|(i, o)| (&i.key, &o.key)) == g.curve_axes().map(|(i, o)| (&i.key, &o.key)),
                None => false,
            };
            if !ok {
                errs.push(format!("'{}'가 '{base}'와 맞지 않음 ('{base}'가 같은 축의 일반 함수여야 함; del {} 하거나 함께 고침)", g.key, g.key));
            }
        }
        for e in errs {
            self.err(Span::new(0, 0), code::TYPE, e);
        }
    }

    /// Every expression in the state that may call a generated derivative.
    fn stored_exprs(&self) -> Vec<&Expr> {
        let mut v: Vec<&Expr> = vec![];
        for f in self.state.functions.values() {
            v.push(&f.body);
            if let Some(r) = &f.range {
                v.extend([&r.lo, &r.hi]);
            }
            if let Some(fam) = &f.family {
                v.extend([&fam.lo, &fam.hi]);
            }
        }
        v.extend(self.state.highlights.values().filter_map(|h| h.range.as_ref()));
        v.extend(self.state.scenes.values().flat_map(|s| s.clip.iter().chain(s.user_clip.iter())).map(|c| &c.expr));
        v
    }

    /// Generated derivatives for everything that needs one: calls of undefined `f@n` and
    /// feature points (`Function::feature` needs). Derivatives of derivatives follow the
    /// chain rule into callees (`g@1`), so needs are closed over them.
    pub(super) fn fill_derivs(&mut self) {
        let defined = |k: &str, fs: &BTreeMap<String, Function>| fs.contains_key(k);
        let mut need: BTreeMap<String, u32> = BTreeMap::new();
        let add_calls = |e: &Expr, need: &mut BTreeMap<String, u32>, fs: &BTreeMap<String, Function>| {
            e.walk(&mut |n| {
                if let ExprKind::Call { callee, kind: NameKind::Function, .. } = &n.kind
                    && let Some((base, k)) = related(callee)
                    && k > 0
                    && !defined(&callee.key, fs)
                {
                    let e = need.entry(base).or_default();
                    *e = (*e).max(k);
                }
            });
        };
        for e in self.stored_exprs() {
            add_calls(e, &mut need, &self.state.functions);
        }
        for (base, k) in self.feature_needs() {
            let e = need.entry(base).or_default();
            *e = (*e).max(k);
        }
        let mut out: BTreeMap<String, Vec<Expr>> = BTreeMap::new();
        let mut failed: BTreeMap<String, String> = BTreeMap::new();
        loop {
            let pending: Vec<(String, u32)> = need.iter().filter(|(b, n)| out.get(*b).map_or(0, |v| v.len() as u32) < **n && !failed.contains_key(*b)).map(|(b, n)| (b.clone(), *n)).collect();
            if pending.is_empty() {
                break;
            }
            for (base, n) in pending {
                if !self.tick() {
                    return;
                }
                let Some(f) = self.state.functions.get(&base) else {
                    failed.insert(base, "정의가 없음".into());
                    continue;
                };
                if !derivable(f) {
                    failed.insert(base, format!("{}에는 도함수가 없음", f.kind.word()));
                    continue;
                }
                if n > MAX_ORDER {
                    failed.insert(base, format!("미분 횟수 {n}이 상한 {MAX_ORDER}을 넘음"));
                    continue;
                }
                let input = f.inputs[0].key.clone();
                let mut chain = out.remove(&base).unwrap_or_default();
                let mut cur = chain.last().cloned().unwrap_or_else(|| f.body.clone());
                let mut err = None;
                while (chain.len() as u32) < n {
                    match d::derive(&cur, &input) {
                        Ok(e) => {
                            add_calls(&e, &mut need, &self.state.functions);
                            chain.push(e.clone());
                            cur = e;
                        }
                        Err(e) => {
                            err = Some(e.what);
                            break;
                        }
                    }
                }
                if let Some(w) = err {
                    failed.insert(base.clone(), w);
                }
                out.insert(base, chain);
            }
        }
        for (base, what) in &failed {
            let n = need[base];
            let span = self.deriv_sites.get(base).copied().unwrap_or(Span::new(0, 0));
            let order = out.get(base).map_or(0, |v| v.len() as u32) + 1;
            let key = deriv_key(base, order.min(n));
            self.err_s(span, code::DERIV, format!("'{base}'의 도함수를 만들 수 없음 ({what}) — {key}이(가) 필요함"), format!("{key}({}) -> ...: 직접 정의", self.state.functions.get(base).and_then(|f| f.inputs.first()).map_or("x", |i| i.key.as_str())));
        }
        if !failed.is_empty() {
            return;
        }
        for f in self.state.functions.values_mut() {
            f.derivs = out.remove(&f.key).unwrap_or_default();
        }
    }
}

/// `apply` response: generated derivatives that are new or changed and not shadowed by a
/// Claude definition of the same order.
pub(super) fn derivative_notes(old: &State, new: &State) -> Vec<Note> {
    let mut notes = vec![];
    for f in new.functions.values() {
        let before = old.functions.get(&f.key).map_or(&[][..], |o| &o.derivs[..]);
        for (i, e) in f.derivs.iter().enumerate() {
            let n = i as u32 + 1;
            if new.functions.contains_key(&deriv_key(&f.key, n)) || before.get(i) == Some(e) {
                continue;
            }
            notes.push(Note::DerivativeGenerated { function: f.key.clone(), order: n, src: crate::print::expr_dsl(e) });
        }
    }
    notes
}

