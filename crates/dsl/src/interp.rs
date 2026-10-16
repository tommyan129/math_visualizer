//! Interpreter: request source + current state → new state + applied result,
//! or every independent error. Atomic: on any error nothing is applied.
//!
//! The server never evaluates expression values. "Interpreting" here is name
//! resolution, type checking, the referenced-function constant rule, cascade,
//! and constant bookkeeping. Fuel counts AST nodes visited during that work.

use std::collections::{BTreeSet, HashSet};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ast::*;
use crate::diag::{Diagnostic, code};
use crate::names;
use crate::parser::{STACK_GROW, STACK_RED_ZONE, parse_request};
use crate::span::Span;
use crate::state::*;
use crate::stmt::*;

/// Starting values; measure and adjust (design.md "자원 상한").
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_source_bytes: usize,
    pub fuel: u64,
    pub wall_clock: Duration,
    /// Scene budget: definitions per workspace ("Layer 수 수백").
    pub max_functions: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_source_bytes: 64 * 1024, fuel: 1_000_000, wall_clock: Duration::from_secs(2), max_functions: 300 }
    }
}

/// What the server did, for the `apply` response and the history view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum Note {
    FunctionCreated { key: String },
    FunctionModified { key: String },
    FunctionUnchanged { key: String },
    /// `via`: the deleted function whose removal cascaded here; `None` = named in `del`.
    FunctionDeleted { key: String, via: Option<String> },
    ConstantCreated { key: String },
    ConstantMadeInteger { key: String },
    ConstantRemoved { key: String },
    SignatureConstAdded { function: String, constant: String, via: String },
    SignatureConstRemoved { function: String, constant: String, via: String },
}

#[derive(Debug, Clone)]
pub struct Applied {
    pub state: State,
    pub changes: Vec<Change>,
    pub notes: Vec<Note>,
}

pub fn apply(state: &State, src: &str, limits: &Limits, seq: u64) -> Result<Applied, Vec<Diagnostic>> {
    if src.len() > limits.max_source_bytes {
        return Err(vec![Diagnostic::new(
            Span::new(0, 0),
            code::LIMIT,
            format!("요청이 너무 큼 ({}바이트, 상한 {}바이트)", src.len(), limits.max_source_bytes),
        )]);
    }
    let parsed = parse_request(src);
    let mut cx = Cx {
        src,
        state: state.clone(),
        notes: vec![],
        diags: parsed.diags,
        failed: parsed.failed_defs.into_iter().collect(),
        fuel_left: limits.fuel,
        fuel_limit: limits.fuel,
        deadline: Instant::now() + limits.wall_clock,
        wall_clock: limits.wall_clock,
        stopped: false,
    };
    for stmt in &parsed.stmts {
        if cx.stopped {
            break;
        }
        match stmt {
            Stmt::Define { defs, .. } => defs.iter().for_each(|d| cx.define(d)),
            Stmt::Del { names, .. } => cx.del(names),
        }
    }
    cx.finish();
    if cx.state.functions.len() > limits.max_functions {
        cx.diags.push(Diagnostic::new(
            Span::new(0, 0),
            code::LIMIT,
            format!("정의가 너무 많음 ({}개, 상한 {}개)", cx.state.functions.len(), limits.max_functions),
        ));
    }
    if !cx.diags.is_empty() {
        let mut d = cx.diags;
        d.sort_by_key(|d| d.span.start);
        return Err(d);
    }
    let mut new = cx.state;
    let changes = new.diff_from(state, seq);
    Ok(Applied { state: new, changes, notes: cx.notes })
}

struct Cx<'s> {
    src: &'s str,
    state: State,
    notes: Vec<Note>,
    diags: Vec<Diagnostic>,
    /// Names whose definition failed in this request; references to them are not re-reported.
    failed: HashSet<String>,
    fuel_left: u64,
    fuel_limit: u64,
    deadline: Instant,
    wall_clock: Duration,
    stopped: bool,
}

/// Name scope inside one definition body.
struct Scope {
    def_key: String,
    input: String,
    output: String,
    consts: Vec<String>,
    /// Enclosing `sum`/`prod` indices, innermost last.
    indices: Vec<String>,
}

impl Scope {
    fn signature_with(&self, extra: &[String]) -> String {
        let mut consts = self.consts.clone();
        for e in extra {
            if !consts.contains(e) {
                consts.push(e.clone());
            }
        }
        if consts.is_empty() { format!("{}({})", self.def_key, self.input) } else { format!("{}({}; {})", self.def_key, self.input, consts.join(", ")) }
    }
}

#[derive(Default)]
struct DefInfo {
    callees: BTreeSet<String>,
    /// Constants used inside a `sum`/`prod` bound (must be integers).
    int_consts: BTreeSet<String>,
    /// Depth inside `sum`/`prod` bounds.
    in_bound: u32,
    /// Names already reported in this definition (one error per name).
    reported: HashSet<String>,
}

impl<'s> Cx<'s> {
    fn err(&mut self, span: Span, code: &'static str, msg: impl Into<String>) {
        self.diags.push(Diagnostic::new(span, code, msg));
    }

    fn err_s(&mut self, span: Span, code: &'static str, msg: impl Into<String>, suggestion: impl Into<String>) {
        self.diags.push(Diagnostic::new(span, code, msg).suggest(suggestion));
    }

    fn unsupported(&mut self, span: Span, what: &str, stage: u8) {
        self.err(span, code::UNSUPPORTED, format!("{what}은(는) 아직 지원하지 않음 ({stage}단계 기능)"));
    }

    /// One unit of fuel. Returns false once a limit is hit (reported once).
    fn tick(&mut self) -> bool {
        if self.stopped {
            return false;
        }
        if self.fuel_left == 0 {
            self.stopped = true;
            self.err(Span::new(0, 0), code::LIMIT, format!("해석 비용이 상한을 넘음 (상한 {} 노드)", self.fuel_limit));
            return false;
        }
        self.fuel_left -= 1;
        if self.fuel_left.is_multiple_of(1024) && Instant::now() > self.deadline {
            self.stopped = true;
            self.err(Span::new(0, 0), code::LIMIT, format!("해석 시간이 상한을 넘음 (상한 {:?})", self.wall_clock));
            return false;
        }
        true
    }

    fn is_function_name(&self, key: &str) -> bool {
        self.state.functions.contains_key(key) || self.failed.contains(key)
    }

    // ----- define -------------------------------------------------------

    /// Checks the stage-1 shape `f(x; a, ...) -> y: expr`. Reports and returns None otherwise.
    fn check_shape<'d>(&mut self, def: &'d Def) -> Option<(&'d Params, &'d Ident, &'d Expr)> {
        if def.name.deriv > 0 {
            self.unsupported(def.name_span, "도함수 이름 (f@n)", 5);
            return None;
        }
        if def.name.inverse {
            self.unsupported(def.name_span, "역함수 이름 (~f)", 5);
            return None;
        }
        let Some(params) = &def.params else {
            self.unsupported(def.span, "괄호 없는 정의 (상수 행렬·벡터, span)", 4);
            return None;
        };
        let body = match &def.body {
            DefBody::Expr(e) => e,
            DefBody::Equation { .. } => {
                self.unsupported(def.span, "방정식 정의 (음함수)", 5);
                return None;
            }
            DefBody::WithRange { range, .. } => {
                self.unsupported(range.span, "매개변수 범위", 5);
                return None;
            }
            DefBody::Block(span) => {
                self.unsupported(*span, "정의 아래 블록 (split)", 5);
                return None;
            }
        };
        let output = match &def.output {
            Some(Output::Axis(o, _)) => o,
            Some(Output::Tuple(_, span)) => {
                self.unsupported(*span, "매개변수 함수 출력 (x, y)", 5);
                return None;
            }
            Some(Output::Coord { span, .. }) => {
                self.unsupported(*span, "극좌표 함수", 5);
                return None;
            }
            Some(Output::Matrix(span)) => {
                self.unsupported(*span, "행렬 값 함수", 4);
                return None;
            }
            None => {
                let inputs = params.inputs.iter().map(|(i, _)| i.key.as_str()).collect::<Vec<_>>().join(", ");
                let body_src = &self.src[body.span.range()];
                self.err_s(
                    def.name_span.to(params.span),
                    code::ARROW,
                    "'->' 출력 축이 필요함",
                    format!("{}({inputs}) -> y: {body_src}", def.name.key),
                );
                return None;
            }
        };
        if params.inputs.len() > 1 {
            self.unsupported(params.span, "다변수 함수", 6);
            return None;
        }
        let input = &params.inputs.first()?.0;
        if input.key != "x" || output.key != "y" {
            self.err_s(
                def.name_span.to(def.output.as_ref().unwrap().span()),
                code::AXES,
                format!("1단계는 2D(x, y) 좌표계만 지원함 ({} -> {})", input.key, output.key),
                format!("{}(x) -> y", def.name.key),
            );
            return None;
        }
        Some((params, output, body))
    }

    fn check_names(&mut self, def: &Def, params: &Params, output: &Ident) {
        let key = &def.name.key;
        let input = &params.inputs[0].0.key;
        if names::is_builtin(key) {
            self.err_s(def.name_span, code::NAME_CONFLICT, format!("'{key}'는 기본 함수 이름이라 쓸 수 없음"), format!("{key}_{{1}} 같은 다른 이름"));
        } else if is_reserved(&def.name) {
            self.err(def.name_span, code::NAME_CONFLICT, format!("'{key}'는 예약어라 이름으로 쓸 수 없음"));
        } else if self.state.constants.contains_key(key) {
            self.err(def.name_span, code::NAME_CONFLICT, format!("'{key}'는 이미 상수 이름임"));
        } else if key == input || *key == output.key {
            self.err(def.name_span, code::NAME_CONFLICT, format!("'{key}'는 축 이름이라 함수 이름으로 쓸 수 없음"));
        }
        let mut seen: Vec<&str> = vec![];
        for (c, span) in &params.consts {
            let ck = c.key.as_str();
            if seen.contains(&ck) {
                self.err(*span, code::DUPLICATE, format!("상수 '{ck}'를 두 번 적음"));
                continue;
            }
            seen.push(ck);
            if c.deriv > 0 || c.inverse {
                self.err(*span, code::NAME_CONFLICT, format!("'{ck}'는 상수 이름으로 쓸 수 없음"));
            } else if names::is_builtin(ck) {
                self.err(*span, code::NAME_CONFLICT, format!("'{ck}'는 기본 함수 이름이라 상수로 쓸 수 없음"));
            } else if is_reserved(c) {
                self.err(*span, code::NAME_CONFLICT, format!("'{ck}'는 예약어라 상수로 쓸 수 없음"));
            } else if ck == input || ck == output.key {
                self.err(*span, code::NAME_CONFLICT, format!("'{ck}'는 축 이름이라 상수로 쓸 수 없음"));
            } else if ck == key || self.is_function_name(ck) {
                self.err(*span, code::NAME_CONFLICT, format!("'{ck}'는 이미 함수 이름임"));
            }
        }
    }

    fn define(&mut self, def: &Def) {
        let before = self.diags.len();
        let key = def.name.key.clone();
        let Some((params, output, body)) = self.check_shape(def) else {
            self.failed.insert(key);
            return;
        };
        self.check_names(def, params, output);

        let mut scope = Scope {
            def_key: key.clone(),
            input: params.inputs[0].0.key.clone(),
            output: output.key.clone(),
            consts: params.consts.iter().map(|(c, _)| c.key.clone()).collect(),
            indices: vec![],
        };
        let mut info = DefInfo::default();
        let mut body = body.clone();
        let body_src = self.src[body.span.range()].to_string();
        match self.resolve(&mut body, &mut scope, &mut info) {
            Ty::Region => self.err(body.span, code::TYPE, "부등식은 함수 값이 될 수 없음"),
            Ty::Scalar | Ty::Unknown => {}
            _ => self.err(body.span, code::TYPE, "함수 값은 숫자여야 함"),
        }

        // Cycles through other functions (direct self-calls are reported during resolution).
        for c in &info.callees {
            if c != &key && self.reaches(c, &key) {
                self.err(def.name_span, code::CYCLE, format!("'{key}'와 '{c}'가 서로를 부름 (순환 참조)"));
            }
        }

        // Referenced-function constants: every constant a callee uses must be declared here.
        let mut missing: Vec<(String, String)> = vec![];
        for c in &info.callees {
            if let Some(f) = self.state.functions.get(c) {
                for k in &f.consts {
                    if !scope.consts.contains(k) && !missing.iter().any(|(m, _)| m == k) {
                        missing.push((k.clone(), c.clone()));
                    }
                }
            }
        }
        if !missing.is_empty() {
            let list = missing.iter().map(|(k, via)| format!("{via}의 {k}")).collect::<Vec<_>>().join(", ");
            let extra: Vec<String> = missing.iter().map(|(k, _)| k.clone()).collect();
            self.err_s(def.name_span, code::MISSING_CONST, format!("부르는 함수가 쓰는 상수를 적지 않음 ({list})"), scope.signature_with(&extra));
        }

        if self.diags.len() > before {
            self.failed.insert(key);
            return;
        }

        body.assign_ids();
        body.clear_spans();
        let old = self.state.functions.get(&key).cloned();
        let (color, order) = match &old {
            Some(o) => (o.color, o.order),
            None => {
                let c = self.state.counters;
                self.state.counters.next_color += 1;
                self.state.counters.next_order += 1;
                (c.next_color, c.next_order)
            }
        };
        let function = Function {
            key: key.clone(),
            name: def.name.clone(),
            inputs: vec![params.inputs[0].0.clone()],
            output: output.clone(),
            consts: scope.consts.clone(),
            body,
            body_src,
            calls: info.callees.iter().cloned().collect(),
            color,
            order,
            version: old.as_ref().map_or(0, |o| o.version),
        };
        self.notes.push(match &old {
            None => Note::FunctionCreated { key: key.clone() },
            Some(o) if *o == function => Note::FunctionUnchanged { key: key.clone() },
            Some(_) => Note::FunctionModified { key: key.clone() },
        });

        for (c, _) in &params.consts {
            let integer = info.int_consts.contains(&c.key);
            match self.state.constants.get_mut(&c.key) {
                None => {
                    let order = self.state.counters.next_order;
                    self.state.counters.next_order += 1;
                    self.state.constants.insert(c.key.clone(), Constant::auto(c.clone(), integer, order));
                    self.notes.push(Note::ConstantCreated { key: c.key.clone() });
                }
                Some(k) if integer && !k.integer => {
                    k.make_integer();
                    self.notes.push(Note::ConstantMadeInteger { key: c.key.clone() });
                }
                Some(_) => {}
            }
        }

        let old_consts = old.map(|o| o.consts).unwrap_or_default();
        self.state.functions.insert(key.clone(), function);
        self.propagate_consts(&key, &old_consts);
    }

    /// Does `from` (transitively) call `target`?
    fn reaches(&self, from: &str, target: &str) -> bool {
        let mut stack = vec![from.to_string()];
        let mut seen = HashSet::new();
        while let Some(k) = stack.pop() {
            if k == target {
                return true;
            }
            if !seen.insert(k.clone()) {
                continue;
            }
            if let Some(f) = self.state.functions.get(&k) {
                stack.extend(f.calls.iter().cloned());
            }
        }
        false
    }

    /// After `key` changed its constants, add/remove them on every (transitive) caller.
    fn propagate_consts(&mut self, key: &str, old_consts: &[String]) {
        let new_consts = self.state.functions[key].consts.clone();
        let added: Vec<String> = new_consts.iter().filter(|c| !old_consts.contains(c)).cloned().collect();
        let removed: Vec<String> = old_consts.iter().filter(|c| !new_consts.contains(c)).cloned().collect();
        if added.is_empty() && removed.is_empty() {
            return;
        }
        let mut callers: Vec<(u64, String)> = self
            .state
            .functions
            .values()
            .filter(|h| h.key != key && self.reaches(&h.key, key))
            .map(|h| (h.order, h.key.clone()))
            .collect();
        callers.sort();
        // Iterate to a fixpoint: a caller's requirement depends on its callees' updated constants.
        loop {
            let mut changed = false;
            for (_, h) in &callers {
                let f = &self.state.functions[h];
                let required: BTreeSet<String> =
                    f.calls.iter().filter_map(|c| self.state.functions.get(c)).flat_map(|c| c.consts.iter().cloned()).collect();
                let mut consts = f.consts.clone();
                let mut notes = vec![];
                for c in &added {
                    if required.contains(c) && !consts.contains(c) {
                        consts.push(c.clone());
                        notes.push(Note::SignatureConstAdded { function: h.clone(), constant: c.clone(), via: key.to_string() });
                    }
                }
                for c in &removed {
                    if consts.contains(c) && !required.contains(c) && !uses_constant(&f.body, c) {
                        consts.retain(|k| k != c);
                        notes.push(Note::SignatureConstRemoved { function: h.clone(), constant: c.clone(), via: key.to_string() });
                    }
                }
                if !notes.is_empty() {
                    self.state.functions.get_mut(h).unwrap().consts = consts;
                    self.notes.extend(notes);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    // ----- del ----------------------------------------------------------

    fn del(&mut self, names: &[(Ident, Span)]) {
        for (name, span) in names {
            let key = &name.key;
            if self.failed.contains(key) {
                continue;
            }
            if self.state.functions.contains_key(key) {
                let mut queue = vec![(key.clone(), None::<String>)];
                while let Some((k, via)) = queue.pop() {
                    if self.state.functions.remove(&k).is_none() {
                        continue;
                    }
                    self.notes.push(Note::FunctionDeleted { key: k.clone(), via });
                    let mut callers: Vec<(u64, String)> =
                        self.state.functions.values().filter(|h| h.calls.contains(&k)).map(|h| (h.order, h.key.clone())).collect();
                    callers.sort();
                    // Reverse so the earliest-created caller is deleted (and listed) first.
                    queue.extend(callers.into_iter().rev().map(|(_, h)| (h, Some(k.clone()))));
                }
            } else if self.state.constants.contains_key(key) {
                self.err(*span, code::DEL_CONSTANT, format!("상수 '{key}'는 직접 지울 수 없음 (쓰는 함수가 없어지면 자동으로 지워짐)"));
            } else {
                self.err(*span, code::UNDECLARED, format!("'{key}'라는 정의가 없음"));
            }
        }
    }

    /// Drops constants no function uses and toggle entries for deleted functions.
    fn finish(&mut self) {
        let used: HashSet<&String> = self.state.functions.values().flat_map(|f| f.consts.iter()).collect();
        let orphans: Vec<String> = self.state.constants.keys().filter(|k| !used.contains(k)).cloned().collect();
        for k in orphans {
            self.state.constants.remove(&k);
            self.notes.push(Note::ConstantRemoved { key: k });
        }
        let functions = &self.state.functions;
        for s in self.state.scenes.values_mut() {
            s.hidden.retain(|k| functions.contains_key(k));
        }
    }

    // ----- resolution ---------------------------------------------------

    fn resolve(&mut self, e: &mut Expr, sc: &mut Scope, info: &mut DefInfo) -> Ty {
        stacker::maybe_grow(STACK_RED_ZONE, STACK_GROW, || self.resolve_inner(e, sc, info))
    }

    fn expect_scalar(&mut self, ty: &Ty, span: Span) {
        match ty {
            Ty::Scalar | Ty::Unknown => {}
            Ty::Region => self.err(span, code::TYPE, "부등식에는 산술 연산을 할 수 없음"),
            _ => self.err(span, code::TYPE, "숫자가 와야 함"),
        }
    }

    fn resolve_inner(&mut self, e: &mut Expr, sc: &mut Scope, info: &mut DefInfo) -> Ty {
        if !self.tick() {
            return Ty::Unknown;
        }
        let span = e.span;
        let ty = match &mut e.kind {
            ExprKind::Num { .. } => Ty::Scalar,
            ExprKind::Error {} => Ty::Unknown,
            ExprKind::Name { name, kind } => {
                let (k, t) = self.resolve_name(name, span, sc, info);
                *kind = k;
                t
            }
            ExprKind::Call { callee, kind, args } => {
                let mut arg_tys = vec![];
                for a in args.iter_mut() {
                    let t = self.resolve(a, sc, info);
                    arg_tys.push((t, a.span));
                }
                for (t, s) in &arg_tys {
                    self.expect_scalar(t, *s);
                }
                let (k, t) = self.resolve_call(callee, args.len(), span, sc, info);
                *kind = k;
                t
            }
            ExprKind::Sum { terms } => {
                for t in terms.iter_mut() {
                    let ty = self.resolve(&mut t.expr, sc, info);
                    self.expect_scalar(&ty, t.expr.span);
                }
                Ty::Scalar
            }
            ExprKind::Product { .. } | ExprKind::Div { .. } | ExprKind::Pow { .. } | ExprKind::Neg { .. } | ExprKind::Factorial { .. } | ExprKind::Paren { .. } => {
                for c in e.children_mut() {
                    let ty = self.resolve(c, sc, info);
                    self.expect_scalar(&ty, c.span);
                }
                Ty::Scalar
            }
            ExprKind::BigOp { body, index, lo, hi, .. } => {
                let ik = index.key.clone();
                let clash = ik == sc.input
                    || sc.consts.contains(&ik)
                    || sc.indices.contains(&ik)
                    || ik == sc.def_key
                    || self.is_function_name(&ik)
                    || self.state.constants.contains_key(&ik)
                    || names::is_builtin(&ik)
                    || is_reserved(index);
                if clash {
                    self.err(span, code::NAME_CONFLICT, format!("인덱스 '{ik}'가 다른 이름과 겹침"));
                }
                sc.indices.push(ik);
                let bt = self.resolve(body, sc, info);
                sc.indices.pop();
                self.expect_scalar(&bt, body.span);
                info.in_bound += 1;
                for b in [lo, hi] {
                    let t = self.resolve(b, sc, info);
                    self.expect_scalar(&t, b.span);
                }
                info.in_bound -= 1;
                Ty::Scalar
            }
            ExprKind::Compare { .. } => {
                for c in e.children_mut() {
                    let ty = self.resolve(c, sc, info);
                    self.expect_scalar(&ty, c.span);
                }
                Ty::Region
            }
            ExprKind::Not { .. } | ExprKind::And { .. } | ExprKind::Or { .. } => {
                for c in e.children_mut() {
                    let ty = self.resolve(c, sc, info);
                    if !matches!(ty, Ty::Region | Ty::Unknown) {
                        self.err(c.span, code::TYPE, "and / or / not은 부등식에만 쓸 수 있음");
                    }
                }
                Ty::Region
            }
            ExprKind::Str { .. } => {
                self.unsupported(span, "식 안의 문자열", 2);
                Ty::Unknown
            }
            ExprKind::Tuple { .. } => {
                self.unsupported(span, "튜플 (매개변수 함수 출력)", 5);
                Ty::Unknown
            }
            ExprKind::List { .. } => {
                self.unsupported(span, "행렬·벡터 값", 4);
                Ty::Unknown
            }
            ExprKind::Index { .. } => {
                self.unsupported(span, "원소 꺼내기 A_[i, j]", 4);
                Ty::Unknown
            }
            ExprKind::Address { .. } => {
                self.unsupported(span, "부분식 주소", 2);
                Ty::Unknown
            }
        };
        e.ty = ty.clone();
        ty
    }

    fn resolve_name(&mut self, name: &Ident, span: Span, sc: &Scope, info: &mut DefInfo) -> (NameKind, Ty) {
        let key = &name.key;
        if is_bad_tex(name) {
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if name.deriv > 0 || name.inverse {
            self.unsupported(span, "도함수·역함수 이름", 5);
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if sc.indices.contains(key) {
            return (NameKind::Index, Ty::Scalar);
        }
        if *key == sc.input {
            return (NameKind::Input, Ty::Scalar);
        }
        if sc.consts.contains(key) {
            if info.in_bound > 0 {
                info.int_consts.insert(key.clone());
            }
            return (NameKind::Constant, Ty::Scalar);
        }
        if is_reserved(name) {
            return match key.as_str() {
                "\\pi" | "\\e" => (NameKind::Reserved, Ty::Scalar),
                "\\i" => {
                    self.err(span, code::UNSUPPORTED, "복소수 \\i는 아직 지원하지 않음 (실수 모드)");
                    (NameKind::Reserved, Ty::Unknown)
                }
                _ => {
                    self.err(span, code::TYPE, format!("{key}는 값으로 쓸 수 없음"));
                    (NameKind::Reserved, Ty::Unknown)
                }
            };
        }
        if !info.reported.insert(key.clone()) {
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if *key == sc.def_key || self.is_function_name(key) || names::is_builtin(key) {
            self.err_s(span, code::NOT_VALUE, format!("함수 '{key}'는 불러서 써야 함"), format!("{key}({})", sc.input));
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if *key == sc.output {
            self.err(span, code::UNDECLARED, format!("출력 축 '{key}'는 식에 쓸 수 없음"));
            return (NameKind::Unresolved, Ty::Unknown);
        }
        // `pi`, `theta`: the LaTeX name was meant, not a new slider.
        if let Some(tex) = latex_name_for(name) {
            self.err_s(span, code::UNDECLARED, format!("'{key}'가 선언되지 않음 (원래 이름은 {tex})"), tex);
        } else if let Some(split) = split_known(key, sc) {
            // `ax` where `a` and `x` are known: probably a missing `*`.
            self.err_s(span, code::UNDECLARED, format!("'{key}'가 선언되지 않음"), split);
        } else {
            self.err_s(span, code::UNDECLARED, format!("'{key}'가 선언되지 않음"), sc.signature_with(std::slice::from_ref(key)));
        }
        (NameKind::Unresolved, Ty::Unknown)
    }

    fn resolve_call(&mut self, callee: &Ident, argc: usize, span: Span, sc: &Scope, info: &mut DefInfo) -> (NameKind, Ty) {
        let key = &callee.key;
        if is_bad_tex(callee) {
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if callee.deriv > 0 || callee.inverse {
            self.unsupported(span, "도함수·역함수 호출", 5);
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if let Some(arity) = names::builtin_arity(key) {
            if !arity.contains(&argc) {
                if key == "log" && argc == 1 {
                    self.err_s(span, code::ARITY, "밑 없는 log는 쓸 수 없음", "ln(x), log10(x), log2(x), log(x, b)");
                } else {
                    self.err(span, code::ARITY, format!("{key}는 인자 {}개를 받음 ({argc}개 줌)", arity[0]));
                }
            }
            return (NameKind::Builtin, Ty::Scalar);
        }
        if *key == sc.def_key {
            self.err(span, code::CYCLE, format!("'{key}'가 자기 자신을 부름 (재귀는 쓸 수 없음)"));
            return (NameKind::Function, Ty::Unknown);
        }
        if let Some(f) = self.state.functions.get(key) {
            if f.inputs.len() != argc {
                let n = f.inputs.len();
                self.err(span, code::ARITY, format!("{key}는 인자 {n}개를 받음 ({argc}개 줌)"));
            }
            info.callees.insert(key.clone());
            return (NameKind::Function, Ty::Scalar);
        }
        if self.failed.contains(key) {
            return (NameKind::Function, Ty::Unknown);
        }
        if *key == sc.input || sc.consts.contains(key) || sc.indices.contains(key) {
            let args = &self.src[span.range()][key.len()..];
            self.err_s(span, code::NOT_FUNCTION, format!("'{key}'는 함수가 아님"), format!("{key}*{args}"));
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if info.reported.insert(key.clone()) {
            self.err(span, code::UNDECLARED, format!("함수 '{key}'가 정의되지 않음"));
        }
        (NameKind::Unresolved, Ty::Unknown)
    }
}

fn is_reserved(name: &Ident) -> bool {
    name.accent.is_none()
        && name.sub.is_none()
        && name.sup.is_none()
        && name.base.strip_prefix('\\').is_some_and(|b| names::RESERVED.contains(&b))
}

/// `\frac`, `\foo`: already reported by the parser.
fn is_bad_tex(name: &Ident) -> bool {
    name.base.strip_prefix('\\').is_some_and(|b| !names::GREEK.contains(&b) && !names::RESERVED.contains(&b))
}

fn uses_constant(body: &Expr, key: &str) -> bool {
    let mut found = false;
    body.walk(&mut |e| {
        if let ExprKind::Name { name, kind: NameKind::Constant } = &e.kind {
            found |= name.key == key;
        }
    });
    found
}

/// `pi` → `\pi`, `alpha` → `\alpha`. Not `i` / `infty`: they are not values in stage 1.
fn latex_name_for(name: &Ident) -> Option<String> {
    let plain = name.accent.is_none() && name.sub.is_none() && name.sup.is_none() && !name.base.starts_with('\\');
    let b = name.base.as_str();
    (plain && (matches!(b, "pi" | "e") || names::GREEK.contains(&b))).then(|| format!("\\{b}"))
}

/// `ax` → `a*x` when every letter is a name in scope.
fn split_known(key: &str, sc: &Scope) -> Option<String> {
    if key.chars().count() < 2 || !key.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let known = |s: &str| s == sc.input || sc.consts.iter().any(|c| c == s) || sc.indices.iter().any(|c| c == s);
    let parts: Vec<String> = key.chars().map(|c| c.to_string()).collect();
    parts.iter().all(|p| known(p)).then(|| parts.join("*"))
}

// ----- user operations --------------------------------------------------

/// Browser-side operations, logged in the same sequence as requests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum UserOp {
    Toggle { scene: String, function: String, visible: bool },
    /// Slider released / value typed.
    SetValue { constant: String, value: f64 },
    SetRange { constant: String, min: f64, max: f64 },
}

impl UserOp {
    /// Short DSL-like text for the history view and responses.
    pub fn describe(&self) -> String {
        match self {
            UserOp::Toggle { function, visible, .. } => format!("{} {function}", if *visible { "켜기" } else { "끄기" }),
            UserOp::SetValue { constant, value } => format!("slider {constant} = {}", fmt_num(*value)),
            UserOp::SetRange { constant, min, max } => format!("range {constant}: {} .. {}", fmt_num(*min), fmt_num(*max)),
        }
    }
}

pub fn fmt_num(v: f64) -> String {
    let s = format!("{:.6}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

pub fn apply_user_op(state: &State, op: &UserOp, seq: u64) -> Result<Applied, String> {
    let mut new = state.clone();
    match op {
        UserOp::Toggle { scene, function, visible } => {
            if !new.functions.contains_key(function) {
                return Err(format!("'{function}'라는 정의가 없음"));
            }
            let s = new.scenes.get_mut(scene).ok_or_else(|| format!("scene '{scene}'이 없음"))?;
            s.hidden.retain(|k| k != function);
            if !visible {
                s.hidden.push(function.clone());
            }
        }
        UserOp::SetValue { constant, value } => {
            let c = new.constants.get_mut(constant).ok_or_else(|| format!("상수 '{constant}'가 없음"))?;
            if !value.is_finite() {
                return Err("값이 숫자가 아님".into());
            }
            let v = if c.integer { value.round() } else { *value };
            c.value = v.clamp(c.min, c.max);
        }
        UserOp::SetRange { constant, min, max } => {
            let c = new.constants.get_mut(constant).ok_or_else(|| format!("상수 '{constant}'가 없음"))?;
            if !min.is_finite() || !max.is_finite() {
                return Err("범위 끝이 숫자가 아님".into());
            }
            if min >= max {
                return Err(format!("최솟값({})이 최댓값({})보다 작아야 함", fmt_num(*min), fmt_num(*max)));
            }
            if c.integer && (min.fract() != 0.0 || max.fract() != 0.0) {
                return Err(format!("'{constant}'는 정수 상수라 범위 끝도 정수여야 함"));
            }
            c.min = *min;
            c.max = *max;
            c.value = c.value.clamp(*min, *max);
        }
    }
    let changes = new.diff_from(state, seq);
    Ok(Applied { state: new, changes, notes: vec![] })
}

#[cfg(test)]
mod tests;
