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
    /// `constant:` entry created or changed the constant.
    ConstantDeclared { key: String },
    ObjectCreated { kind: ObjKind, key: String },
    ObjectModified { kind: ObjKind, key: String },
    ObjectUnchanged { kind: ObjKind, key: String },
    /// `via`: deleted object this depended on; `reason`: why it could not be kept otherwise.
    /// Both `None` = named in `del`.
    ObjectDeleted { kind: ObjKind, key: String, via: Option<String>, reason: Option<String> },
    RequestAdded {
        #[ts(type = "number")]
        number: u64,
    },
    /// Claude scenes (`scene(n):` blocks). `key` is the number as text.
    SceneCreated { key: String },
    SceneModified { key: String },
    SceneUnchanged { key: String },
    SceneDeleted { key: String },
    /// Default scene of a coordinate system created / removed with its functions.
    DefaultSceneCreated { key: String },
    DefaultSceneRemoved { key: String },
    /// Server-generated derivative `f@n` (new or changed), in DSL notation (stage 5b).
    DerivativeGenerated { function: String, order: u32, src: String },
    /// Cascade inside a Claude scene: the scene stays, the reference goes.
    /// `place`: `show`, `clip`, `step 2 show`, `step 1 note` …
    SceneRefRemoved { scene: String, target: String, place: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ObjKind {
    Highlight,
    Style,
    Var,
}

impl ObjKind {
    pub fn word(self) -> &'static str {
        match self {
            ObjKind::Highlight => "강조",
            ObjKind::Style => "style",
            ObjKind::Var => "변수",
        }
    }
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
        seq,
        declared_now: BTreeSet::new(),
        deriv_sites: Default::default(),
        pending_feature: None,
        pending_calc: None,
    };
    for stmt in &parsed.stmts {
        if cx.stopped {
            break;
        }
        match stmt {
            Stmt::Define { defs, .. } => defs.iter().for_each(|d| cx.define(d)),
            Stmt::Del { names, scenes, .. } => cx.del(names, scenes),
            Stmt::Constant { entries, .. } => entries.iter().for_each(|e| cx.declare_constant(e)),
            Stmt::Style { name, name_span, props, span } => cx.style(name, *name_span, props, *span),
            Stmt::Assign { name, name_span, value, span } => cx.assign(name, *name_span, value, *span),
            Stmt::Req { slots, scene, span } => cx.req(slots, scene.as_ref(), *span),
            Stmt::Scene(def) => cx.scene(def),
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
    let mut notes = cx.notes;
    notes.extend(deriv::derivative_notes(state, &new));
    let changes = new.diff_from(state, seq);
    Ok(Applied { state: new, changes, notes })
}

/// Brings a restored state up to the current model: stage 1-2 scene shapes, default
/// scenes for every coordinate system in use. Not logged (deterministic from the log).
pub fn normalize(state: &State, seq: u64) -> State {
    let mut s = state.clone();
    s.migrate();
    match apply(&s, "", &Limits::default(), seq) {
        Ok(a) => a.state,
        Err(_) => s,
    }
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
    /// Log sequence this request will get (history step of a `req`).
    seq: u64,
    /// Constants declared in this request's `constant:` blocks.
    declared_now: BTreeSet<String>,
    /// First place in this request that needs a generated derivative of a function (error spans).
    deriv_sites: std::collections::HashMap<String, Span>,
    /// Spec of the feature-point definition being resolved (`resolve_feature`).
    pending_feature: Option<FeatureSpec>,
    pending_calc: Option<CalcSpec>,
}

/// Name scope inside one definition body, or one range slot.
struct Scope {
    /// Definition key; empty for a range slot.
    def_key: String,
    inputs: Vec<String>,
    /// Output axes (`y`; `x, y` of a parametric function; `r` of a polar function).
    outputs: Vec<String>,
    consts: Vec<String>,
    /// Range slots may use any existing constant without listing it.
    free_consts: bool,
    /// Enclosing `sum`/`prod` indices, innermost last.
    indices: Vec<String>,
    /// Name of the enclosing split's subject (`split r = ... at`).
    split_name: Option<String>,
    /// The next node is a parametric body as a whole, where a parametric call fits.
    tuple_ok: bool,
    /// Resolving one output slot of a parametric body (for messages).
    in_slot: bool,
}

impl Scope {
    fn signature_with(&self, extra: &[String]) -> String {
        let mut consts = self.consts.clone();
        for e in extra {
            if !consts.contains(e) {
                consts.push(e.clone());
            }
        }
        let inputs = self.inputs.join(", ");
        if consts.is_empty() { format!("{}({inputs})", self.def_key) } else { format!("{}({inputs}; {})", self.def_key, consts.join(", ")) }
    }

    fn input(&self) -> &str {
        self.inputs.first().map_or("x", |s| s.as_str())
    }

    /// Scope of a range slot: the given axes, any existing constant.
    fn range(inputs: &[&str]) -> Self {
        Scope { def_key: String::new(), inputs: inputs.iter().map(|s| s.to_string()).collect(), outputs: vec![], consts: vec![], free_consts: true, indices: vec![], split_name: None, tuple_ok: false, in_slot: false }
    }
}

/// A definition head after `check_shape`.
struct Shape<'d> {
    kind: DefKind,
    inputs: &'d [(Ident, Span)],
    consts: &'d [(Ident, Span)],
    /// Output axis of a curve (`y`) or polar function (`r`).
    output: Option<&'d Ident>,
    /// Output axes of a parametric function.
    outputs: Vec<Ident>,
    body: Expr,
    /// `0 <= t <= 2*\pi` after the body (parametric, polar).
    range: Option<&'d Expr>,
}

#[derive(Default)]
struct DefInfo {
    callees: BTreeSet<String>,
    /// Constants used directly (range slots).
    used_consts: BTreeSet<String>,
    /// Constants used inside a `sum`/`prod` bound (must be integers).
    int_consts: BTreeSet<String>,
    /// Depth inside `sum`/`prod` bounds.
    in_bound: u32,
    /// Order of split boundary points that use constants.
    constraints: Vec<OrderConstraint>,
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

    /// Classifies a definition and checks its head (design.md "정의", "행렬·벡터"):
    /// `f(x; a) -> y`, `R(\theta) -> matrix`, `A: [[..]]` / `v(; a): [..]`,
    /// `S: span(..)`, `p: arrow(..)`. Reports and returns None otherwise.
    fn check_shape<'d>(&mut self, def: &'d Def) -> Option<Shape<'d>> {
        if deriv::related(&def.name).is_some() && !self.check_related_head(def) {
            return None;
        }
        if let Some(r) = self.check_shape_drawn(def) {
            return r;
        }
        let DefBody::Expr(body) = &def.body else { unreachable!("routed by check_shape_drawn") };
        let inputs: &[(Ident, Span)] = def.params.as_ref().map_or(&[], |p| &p.inputs);
        let consts: &[(Ident, Span)] = def.params.as_ref().map_or(&[], |p| &p.consts);
        let head = def.params.as_ref().map_or(def.name_span, |p| def.name_span.to(p.span));
        let body_src = &self.src[body.span.range()];
        let drawn_call = match &body.kind {
            ExprKind::Call { callee, .. } if callee.key == "span" => Some(DefKind::Span),
            ExprKind::Call { callee, .. } if callee.key == "arrow" => Some(DefKind::Arrow),
            ExprKind::Feature { .. } => Some(DefKind::Feature),
            ExprKind::Calc { .. } => Some(DefKind::Calc),
            _ => None,
        };
        let const_sig = |key: &str| if consts.is_empty() { key.to_string() } else { format!("{key}(; {})", consts.iter().map(|(c, _)| c.key.as_str()).collect::<Vec<_>>().join(", ")) };

        let (kind, output) = match &def.output {
            Some(Output::Axis(o, _)) => (DefKind::Curve, Some(o)),
            Some(Output::Matrix(_)) => (DefKind::Matrix, None),
            Some(Output::Tuple(..) | Output::Coord { .. }) => unreachable!("routed by check_shape_drawn"),
            None => {
                let kind = drawn_call.unwrap_or(DefKind::Value);
                if !inputs.is_empty() {
                    let list = inputs.iter().map(|(i, _)| i.key.as_str()).collect::<Vec<_>>().join(", ");
                    if kind == DefKind::Value && !matches!(body.kind, ExprKind::List { .. }) {
                        // A function written without its output axis.
                        let all = def.params.as_ref().map_or(String::new(), |p| self.src[p.span.range()].to_string());
                        self.err_s(head, code::ARROW, "'->' 출력 축이 필요함", format!("{}{all} -> y: {body_src}", def.name.key));
                    } else {
                        self.err_s(
                            head,
                            code::SYNTAX,
                            format!("{}에는 입력 변수({list})가 없음 (상수는 ';' 뒤에)", kind.word()),
                            format!("{}: {body_src}", const_sig(&def.name.key)),
                        );
                    }
                    return None;
                }
                (kind, None)
            }
        };
        if matches!(kind, DefKind::Curve | DefKind::Matrix) && inputs.is_empty() {
            let out = def.output.as_ref().map_or(String::new(), |o| format!(" -> {}", &self.src[o.span().range()]));
            let sug = if kind == DefKind::Matrix { format!("{}: {body_src}", const_sig(&def.name.key)) } else { format!("{}(x{}){out}: {body_src}", def.name.key, consts.iter().map(|(c, _)| format!("; {}", c.key)).collect::<String>()) };
            let msg = if kind == DefKind::Matrix { "입력 변수가 없음 (상수 행렬·벡터는 '-> matrix' 없이 씀)" } else { "입력 변수가 없음" };
            self.err_s(head, code::SYNTAX, msg, sug);
            return None;
        }
        if let (Some(k), DefKind::Curve | DefKind::Matrix) = (drawn_call, kind) {
            self.err_s(body.span, code::TYPE, format!("{}은(는) 함수 값이 될 수 없음 (그리는 정의)", k.word()), format!("{}: {body_src}", const_sig(&def.name.key)));
            return None;
        }
        if kind == DefKind::Curve {
            if inputs.len() > 1 {
                self.unsupported(def.params.as_ref().unwrap().span, "다변수 함수", 6);
                return None;
            }
            let input = &inputs[0].0;
            let output = output.unwrap();
            if input.key == output.key {
                self.err_s(
                    def.name_span.to(def.output.as_ref().unwrap().span()),
                    code::AXES,
                    format!("입력 축과 출력 축이 같음 ({} -> {})", input.key, output.key),
                    format!("{}({}) -> y", def.name.key, input.key),
                );
                return None;
            }
            if output.deriv > 0 || output.inverse || is_reserved(output) {
                self.err(def.name_span.to(def.output.as_ref().unwrap().span()), code::AXES, "좌표축 이름으로 쓸 수 없는 이름");
                return None;
            }
        }
        if let Some((i, s)) = inputs.iter().find(|(i, _)| i.deriv > 0 || i.inverse || is_reserved(i)) {
            let what = if kind == DefKind::Curve { "좌표축" } else { "입력 변수" };
            self.err(*s, code::AXES, format!("'{}'는 {what} 이름으로 쓸 수 없음", i.key));
            return None;
        }
        Some(Shape { kind, inputs, consts, output, outputs: vec![], body: body.clone(), range: None })
    }

    fn check_names(&mut self, def: &Def, shape: &Shape) {
        let key = &def.name.key;
        let is_input = |k: &str| shape.inputs.iter().any(|(i, _)| i.key == k);
        let is_output = |k: &str| shape.output.is_some_and(|o| o.key == k) || shape.outputs.iter().any(|o| o.key == k);
        if let Some(kind) = self.state.kind_of(key).filter(|k| !matches!(*k, "함수" | "상수")) {
            self.err_s(def.name_span, code::NAME_CONFLICT, format!("'{key}'는 이미 {kind} 이름임"), format!("del {key} 후 정의하거나 다른 이름"));
        } else if names::is_builtin(key) || is_stage2_word(key) {
            self.err_s(def.name_span, code::NAME_CONFLICT, format!("'{key}'는 기본 함수 이름이라 쓸 수 없음"), format!("{key}_{{1}} 같은 다른 이름"));
        } else if is_reserved(&def.name) {
            self.err(def.name_span, code::NAME_CONFLICT, format!("'{key}'는 예약어라 이름으로 쓸 수 없음"));
        } else if self.state.constants.contains_key(key) {
            self.err(def.name_span, code::NAME_CONFLICT, format!("'{key}'는 이미 상수 이름임"));
        } else if is_input(key) || is_output(key) {
            self.err(def.name_span, code::NAME_CONFLICT, format!("'{key}'는 축 이름이라 정의 이름으로 쓸 수 없음"));
        }
        let mut seen: Vec<&str> = vec![];
        for (i, span) in shape.inputs {
            if seen.contains(&i.key.as_str()) {
                self.err(*span, code::DUPLICATE, format!("입력 변수 '{}'를 두 번 적음", i.key));
            }
            seen.push(&i.key);
        }
        let mut seen: Vec<&str> = vec![];
        for (c, span) in shape.consts {
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
            } else if is_input(ck) || is_output(ck) {
                self.err(*span, code::NAME_CONFLICT, format!("'{ck}'는 축 이름이라 상수로 쓸 수 없음"));
            } else if ck == key || self.is_function_name(ck) {
                self.err(*span, code::NAME_CONFLICT, format!("'{ck}'는 이미 정의 이름임"));
            } else if let Some(kind) = self.state.kind_of(ck).filter(|k| *k != "상수") {
                self.err(*span, code::NAME_CONFLICT, format!("'{ck}'는 이미 {kind} 이름이라 상수로 쓸 수 없음"));
            }
        }
    }

    fn define(&mut self, def: &Def) {
        let before = self.diags.len();
        let key = def.name.key.clone();
        if matches!(&def.body, DefBody::Expr(e) if matches!(e.kind, ExprKind::Error {})) {
            // A split block that failed to parse (already reported).
            self.failed.insert(key);
            return;
        }
        let Some(shape) = self.check_shape(def) else {
            self.failed.insert(key);
            return;
        };
        if deriv::related(&def.name).is_some() && !self.check_related_axes(def, &shape) {
            self.failed.insert(key);
            return;
        }
        self.check_names(def, &shape);

        let mut scope = Scope {
            def_key: key.clone(),
            inputs: shape.inputs.iter().map(|(i, _)| i.key.clone()).collect(),
            outputs: shape.output.iter().map(|o| o.key.clone()).chain(shape.outputs.iter().map(|o| o.key.clone())).collect(),
            consts: shape.consts.iter().map(|(c, _)| c.key.clone()).collect(),
            free_consts: false,
            indices: vec![],
            split_name: None,
            tuple_ok: false,
            in_slot: false,
        };
        let mut info = DefInfo::default();
        let mut body = shape.body.clone();
        let body_src = if matches!(body.kind, ExprKind::Split { .. }) { crate::print::expr_dsl(&body) } else { self.src[body.span.range()].to_string() };
        // `family`: the index is in scope for the body and the parameter range.
        let family_ok = match &def.family {
            Some(h) => self.open_family(h, shape.kind, &mut scope),
            None => false,
        };
        let ty = match shape.kind {
            DefKind::Span | DefKind::Arrow => self.resolve_drawn(&mut body, shape.kind, &mut scope, &mut info),
            DefKind::Curve | DefKind::Polar if matches!(body.kind, ExprKind::Split { .. }) => self.resolve_split(&mut body, &mut scope, &mut info),
            DefKind::Parametric => self.resolve_parametric(&mut body, shape.outputs.len() as u32, &mut scope, &mut info),
            DefKind::Equation => self.resolve_equation(&mut body, &mut scope, &mut info),
            DefKind::Feature => self.resolve_feature(&mut body, &mut scope, &mut info),
            DefKind::Calc => self.resolve_calc(&mut body, &mut scope, &mut info),
            _ if matches!(body.kind, ExprKind::Split { .. }) => {
                self.err(body.span, code::TYPE, format!("split은 그리는 함수(f(x) -> y)에만 씀 ({}에는 쓸 수 없음)", shape.kind.word()));
                Ty::Unknown
            }
            _ => self.resolve(&mut body, &mut scope, &mut info),
        };
        let range = shape.range.and_then(|r| self.resolve_param_range(r, &mut scope, &mut info));
        let family = match (&def.family, family_ok) {
            (Some(h), true) => {
                scope.indices.pop();
                self.resolve_family(h, &mut scope, &mut info)
            }
            (Some(_), false) => {
                scope.indices.pop();
                None
            }
            _ => None,
        };
        match (shape.kind, &ty) {
            (_, Ty::Unknown) | (DefKind::Span | DefKind::Arrow | DefKind::Parametric | DefKind::Equation | DefKind::Feature | DefKind::Calc, _) => {}
            (DefKind::Polar, Ty::Scalar) => {}
            (DefKind::Polar, t) => self.err(body.span, code::TYPE, format!("극좌표 함수의 값(r)은 숫자여야 함 (이 식은 {})", t.describe())),
            (_, Ty::Region) => self.err(body.span, code::TYPE, "부등식은 정의 값이 될 수 없음"),
            (DefKind::Curve, Ty::Scalar) => {}
            (DefKind::Curve, t) if t.is_array() => {
                self.err_s(body.span, code::TYPE, format!("함수 값은 숫자여야 함 (이 식은 {})", t.describe()), format!("{}({}) -> matrix: {body_src}", key, scope.inputs.join(", ")))
            }
            (DefKind::Matrix | DefKind::Value, t) if t.is_array() => {}
            (DefKind::Matrix, Ty::Scalar) => {
                let sig = scope.signature_with(&[]);
                self.err_s(body.span, code::TYPE, "'-> matrix' 정의의 값은 벡터·행렬이어야 함", format!("{sig} -> y: {body_src}"))
            }
            (DefKind::Value, Ty::Scalar) => {
                // A plain number fits a slider constant (its default takes numbers only);
                // anything else (a feature coordinate, a call) becomes a function of x with
                // the constants its callees use.
                let literal = match &body.kind {
                    ExprKind::Num { .. } => true,
                    ExprKind::Neg { arg } => matches!(arg.kind, ExprKind::Num { .. }),
                    _ => false,
                };
                let suggestion = if literal {
                    format!("constant:\n    {key}: default={body_src}")
                } else {
                    let mut consts = scope.consts.clone();
                    for c in &info.callees {
                        for k in self.state.functions.get(c).map(|f| f.consts.as_slice()).unwrap_or_default() {
                            if !consts.contains(k) {
                                consts.push(k.clone());
                            }
                        }
                    }
                    let tail = if consts.is_empty() { String::new() } else { format!("; {}", consts.join(", ")) };
                    format!("{key}(x{tail}) -> y: {body_src}")
                };
                self.err_s(body.span, code::TYPE, "괄호 없는 정의는 벡터·행렬·span·arrow만 씀 (숫자 하나는 슬라이더 상수나 함수로)", suggestion)
            }
            _ => self.err(body.span, code::TYPE, "정의 값은 숫자, 벡터, 행렬이어야 함"),
        }

        // Cycles through other definitions (direct self-references are reported during resolution).
        for c in &info.callees {
            if c != &key && self.reaches(c, &key) {
                self.err(def.name_span, code::CYCLE, format!("'{key}'와 '{c}'가 서로를 부름 (순환 참조)"));
            }
        }

        // Referenced-definition constants: every constant a callee uses must be declared here.
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
            self.err_s(def.name_span, code::MISSING_CONST, format!("참조하는 정의가 쓰는 상수를 적지 않음 ({list})"), scope.signature_with(&extra));
        }

        for c in &info.int_consts {
            if let Some(k) = self.state.constants.get(c)
                && k.kind_fixed
                && !k.integer
            {
                self.err_s(def.name_span, code::CONST_KIND, format!("상수 '{c}'는 real로 선언됐는데 sum/prod 범위에 쓰임 (범위는 정수만)"), format!("constant:\n    {c}: integer, ..."));
            }
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
            kind: shape.kind,
            inputs: shape.inputs.iter().map(|(i, _)| i.clone()).collect(),
            output: shape.output.cloned(),
            outputs: shape.outputs.clone(),
            range,
            family,
            feature: self.pending_feature.take(),
            calc: self.pending_calc.take(),
            ty,
            consts: scope.consts.clone(),
            body,
            body_src,
            calls: info.callees.iter().cloned().collect(),
            constraints: std::mem::take(&mut info.constraints),
            derivs: old.as_ref().map_or(vec![], |o| o.derivs.clone()),
            color,
            order,
            version: old.as_ref().map_or(0, |o| o.version),
        };
        self.notes.push(match &old {
            None => Note::FunctionCreated { key: key.clone() },
            Some(o) if *o == function => Note::FunctionUnchanged { key: key.clone() },
            Some(_) => Note::FunctionModified { key: key.clone() },
        });

        for (c, _) in shape.consts {
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

    fn del(&mut self, names: &[(Ident, Span)], scenes: &[(SceneRef, Span)]) {
        for (r, span) in scenes {
            self.del_scene(r, *span);
        }
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
                    // Callers, and `k@n` / `~k`, which belong to k by name.
                    let mut callers: Vec<(u64, String)> = self
                        .state
                        .functions
                        .values()
                        .filter(|h| h.calls.contains(&k) || deriv::related(&h.name).is_some_and(|(b, _)| b == k))
                        .map(|h| (h.order, h.key.clone()))
                        .collect();
                    callers.sort();
                    // Reverse so the earliest-created caller is deleted (and listed) first.
                    queue.extend(callers.into_iter().rev().map(|(_, h)| (h, Some(k.clone()))));
                }
            } else if self.state.highlights.remove(key).is_some() {
                self.notes.push(Note::ObjectDeleted { kind: ObjKind::Highlight, key: key.clone(), via: None, reason: None });
            } else if self.state.styles.remove(key).is_some() {
                self.notes.push(Note::ObjectDeleted { kind: ObjKind::Style, key: key.clone(), via: None, reason: None });
            } else if self.state.vars.remove(key).is_some() {
                self.notes.push(Note::ObjectDeleted { kind: ObjKind::Var, key: key.clone(), via: None, reason: None });
            } else if self.state.constants.contains_key(key) {
                self.err(*span, code::DEL_CONSTANT, format!("상수 '{key}'는 직접 지울 수 없음 (쓰는 함수가 없어지면 자동으로 지워짐)"));
            } else {
                self.err(*span, code::UNDECLARED, format!("'{key}'라는 정의가 없음"));
            }
        }
    }

    /// Drops constants no function uses, re-resolves highlights and scenes (cascade),
    /// keeps default scenes in sync with the coordinate systems in use.
    fn finish(&mut self) {
        let used: HashSet<String> = self.state.functions.values().flat_map(|f| f.consts.iter().cloned()).collect();
        let orphans: Vec<String> = self.state.constants.keys().filter(|k| !used.contains(*k)).cloned().collect();
        for k in orphans {
            if self.declared_now.contains(&k) {
                self.err_s(Span::new(0, 0), code::UNUSED_CONST, format!("상수 '{k}'를 쓰는 함수가 없음"), format!("f(x; {k}) -> y: ...처럼 함수 괄호의 ';' 뒤에 적기"));
                continue;
            }
            self.state.constants.remove(&k);
            self.notes.push(Note::ConstantRemoved { key: k });
        }
        if self.diags.is_empty() {
            self.check_related_defs();
            self.refresh_features();
            self.refresh_calcs();
        }
        if self.diags.is_empty() {
            self.refresh_highlights();
            self.refresh_scenes();
            self.sync_default_scenes();
            self.fill_derivs();
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
            t => self.err(span, code::TYPE, format!("숫자가 와야 함 (이 식은 {})", t.describe())),
        }
    }

    /// Sum terms: all numbers, or all vectors / matrices of one size.
    fn same_shape(&mut self, tys: &[(Ty, Span)], verb: &str) -> Ty {
        let first = tys.iter().find(|(t, _)| t.is_array()).cloned();
        let Some((want, wspan)) = first else {
            for (t, s) in tys {
                self.expect_scalar(t, *s);
            }
            return if tys.iter().any(|(t, _)| *t == Ty::Unknown) { Ty::Unknown } else { Ty::Scalar };
        };
        let mut ok = true;
        for (t, s) in tys {
            match t {
                Ty::Unknown => ok = false,
                t if *t == want => {}
                Ty::Scalar => {
                    self.err(*s, code::SHAPE, format!("{}와 숫자는 {verb} 수 없음", want.describe()));
                    ok = false;
                }
                t if t.is_array() => {
                    self.err(wspan.to(*s), code::SHAPE, format!("크기가 다름: {}와 {}", want.describe(), t.describe()));
                    ok = false;
                }
                t => {
                    self.expect_scalar(t, *s);
                    ok = false;
                }
            }
        }
        if ok { want } else { Ty::Unknown }
    }

    /// `[1, 2]` → vector(2); `[[1, 2], [3, 4]]` → 2×2 (rows of one length).
    fn list_ty(&mut self, items: &[Expr], tys: &[Ty], span: Span) -> Ty {
        if items.is_empty() {
            self.err(span, code::SHAPE, "빈 벡터는 쓸 수 없음");
            return Ty::Unknown;
        }
        let is_row = |e: &Expr| matches!(e.kind, ExprKind::List { .. });
        let rows = items.iter().filter(|e| is_row(e)).count();
        if rows > 0 && rows < items.len() {
            self.err_s(span, code::SHAPE, "숫자와 [..] 행이 섞여 있음", "벡터 [1, 2] 또는 행렬 [[1, 2], [3, 4]]");
            return Ty::Unknown;
        }
        if rows == 0 {
            let mut ok = true;
            for (e, t) in items.iter().zip(tys) {
                match t {
                    Ty::Scalar => {}
                    Ty::Unknown => ok = false,
                    t if t.is_array() => {
                        self.err_s(e.span, code::SHAPE, format!("벡터의 원소는 숫자여야 함 (이 식은 {})", t.describe()), "행렬은 [[1, 2], [3, 4]]처럼 행을 [..]로 씀");
                        return Ty::Unknown;
                    }
                    t => {
                        self.expect_scalar(t, e.span);
                        ok = false;
                    }
                }
            }
            if items.len() as u32 > names::MAX_DIM {
                self.err(span, code::LIMIT, format!("벡터가 너무 김 ({}개, 상한 {})", items.len(), names::MAX_DIM));
                return Ty::Unknown;
            }
            return if ok { Ty::Vector { n: items.len() as u32 } } else { Ty::Unknown };
        }
        // Rows: each resolved as a vector (a row of length 1 is a 1×1 matrix `[5]`... a vector of length 1).
        let mut lens = vec![];
        for (e, t) in items.iter().zip(tys) {
            match t {
                Ty::Vector { n } => lens.push(*n),
                Ty::Unknown => return Ty::Unknown,
                _ => {
                    self.err(e.span, code::SHAPE, "행렬은 두 겹까지만 씀 ([[1, 2], [3, 4]])");
                    return Ty::Unknown;
                }
            }
        }
        if let Some(i) = lens.iter().position(|n| *n != lens[0]) {
            self.err_s(
                items[i].span,
                code::SHAPE,
                format!("행 길이가 다름 (1행 {}개, {}행 {}개)", lens[0], i + 1, lens[i]),
                "모든 행의 원소 개수를 같게",
            );
            return Ty::Unknown;
        }
        if items.len() as u32 > names::MAX_DIM {
            self.err(span, code::LIMIT, format!("행렬 행이 너무 많음 ({}개, 상한 {})", items.len(), names::MAX_DIM));
            return Ty::Unknown;
        }
        Ty::of_dims(items.len() as u32, lens[0])
    }

    /// `v_[i]` / `A_[i, j]`: number of indices and literal indices checked statically.
    fn index_ty(&mut self, base: &Expr, bt: &Ty, indices: &[Expr], span: Span) {
        let b = self.src[base.span.range()].to_string();
        let sizes: Vec<u32> = match bt {
            Ty::Unknown => return,
            Ty::Vector { n } => vec![*n],
            Ty::Matrix { m, n } => vec![*m, *n],
            t => {
                self.err(span, code::TYPE, format!("원소 번호는 벡터·행렬에만 씀 (이 식은 {})", t.describe()));
                return;
            }
        };
        if indices.len() != sizes.len() {
            let sug = if sizes.len() == 1 { format!("{b}_[1]") } else { format!("{b}_[1, 2]") };
            self.err_s(span, code::INDEX, format!("{}의 원소 번호는 {}개 (준 개수 {}개)", bt.describe(), sizes.len(), indices.len()), sug);
            return;
        }
        for (k, (i, n)) in indices.iter().zip(&sizes).enumerate() {
            let what = if sizes.len() == 1 { "번호" } else if k == 0 { "행 번호" } else { "열 번호" };
            match &i.kind {
                ExprKind::Num { text, int: true } => {
                    let v: u64 = text.parse().unwrap_or(u64::MAX);
                    if v == 0 || v > *n as u64 {
                        self.err(i.span, code::INDEX, format!("{what} {text}는 범위 밖 ({}, 번호는 1부터 {n})", bt.describe()));
                    }
                }
                ExprKind::Num { text, .. } => self.err(i.span, code::INDEX, format!("{what} {text}는 정수가 아님")),
                _ => {}
            }
        }
    }

    /// `matmul`, `pow`, `inv`, `transpose`, `det`, `tr` by operand sizes. `span` / `arrow`
    /// here are nested in an expression, which is not allowed.
    fn linalg_call(&mut self, key: &str, tys: &[(Ty, Span)], args: &[Expr], span: Span) -> Ty {
        let text = |e: &Expr| self.src[e.span.range()].to_string();
        if matches!(key, "span" | "arrow") {
            let call = &self.src[span.range()];
            let name = if key == "span" { "S" } else { "p" };
            self.err_s(span, code::TYPE, format!("{key}은(는) 식 안에 쓸 수 없음 (정의 하나로 씀)"), format!("define:\n    {name}: {call}"));
            return Ty::Unknown;
        }
        let arity = names::LINALG.iter().find(|(n, _)| *n == key).unwrap().1;
        if !arity.contains(&tys.len()) {
            self.err(span, code::ARITY, format!("{key}는 인자 {}개를 받음 ({}개 줌)", arity[0], tys.len()));
            return Ty::Unknown;
        }
        if tys.iter().any(|(t, _)| *t == Ty::Unknown) {
            return Ty::Unknown;
        }
        let (a, aspan) = &tys[0];
        let square = |t: &Ty| matches!(t, Ty::Matrix { m, n } if m == n);
        match key {
            "matmul" => {
                let (b, bspan) = &tys[1];
                for (t, s, e) in [(a, *aspan, &args[0]), (b, *bspan, &args[1])] {
                    if !t.is_array() {
                        let other = if std::ptr::eq(e, &args[0]) { text(&args[1]) } else { text(&args[0]) };
                        self.err_s(s, code::TYPE, format!("matmul은 행렬·벡터끼리 곱함 (이 식은 {}; 숫자 곱은 '*')", t.describe()), format!("{}*{other}", text(e)));
                        return Ty::Unknown;
                    }
                }
                let ((m, k), (k2, n)) = (a.dims().unwrap(), b.dims().unwrap());
                if k != k2 {
                    self.err(span, code::SHAPE, format!("matmul 크기가 맞지 않음: {} · {} (왼쪽 열 {k} ≠ 오른쪽 행 {k2})", a.describe(), b.describe()));
                    return Ty::Unknown;
                }
                Ty::of_dims(m, n)
            }
            "transpose" => match a.dims() {
                Some((m, n)) => Ty::of_dims(n, m),
                None => {
                    self.err(*aspan, code::TYPE, format!("transpose는 행렬·벡터에 씀 (이 식은 {})", a.describe()));
                    Ty::Unknown
                }
            },
            "pow" | "inv" | "det" | "tr" => {
                if !square(a) {
                    if *a == Ty::Scalar {
                        let sug = match key {
                            "pow" => format!("{}^{}", text(&args[0]), text(&args[1])),
                            "inv" => format!("1/{}", text(&args[0])),
                            _ => text(&args[0]),
                        };
                        self.err_s(*aspan, code::TYPE, format!("{key}는 정사각 행렬에 씀 (이 식은 숫자)"), sug);
                    } else {
                        self.err(*aspan, code::SHAPE, format!("{key}는 정사각 행렬에만 씀 (이 식은 {})", a.describe()));
                    }
                    return Ty::Unknown;
                }
                if key == "pow" {
                    let (et, es) = &tys[1];
                    self.expect_scalar(et, *es);
                    if let ExprKind::Num { text, int: false } = &args[1].kind {
                        self.err(*es, code::TYPE, format!("pow의 지수는 정수여야 함 ({text})"));
                    }
                }
                if matches!(key, "det" | "tr") { Ty::Scalar } else { a.clone() }
            }
            _ => unreachable!(),
        }
    }

    /// Body of `S: span(v, w, ...)` / `p: arrow(v)` / `arrow(v; P)`: vectors of one length
    /// with 2 or 3 entries (the axis count of the scenes it can go in). Returns that vector type.
    fn resolve_drawn(&mut self, body: &mut Expr, kind: DefKind, sc: &mut Scope, info: &mut DefInfo) -> Ty {
        let span = body.span;
        let ExprKind::Call { callee, kind: nk, args } = &mut body.kind else { unreachable!() };
        *nk = NameKind::Builtin;
        let name = callee.key.clone();
        if kind == DefKind::Span && args.is_empty() {
            self.err_s(span, code::ARITY, "span에는 벡터가 하나 이상 필요함", "span(v, w)");
            return Ty::Unknown;
        }
        let mut tys = vec![];
        for a in args.iter_mut() {
            let t = self.resolve(a, sc, info);
            tys.push((t, a.span));
        }
        let mut n_seen: Option<(u32, Span)> = None;
        let mut ok = true;
        for (i, (t, s)) in tys.iter().enumerate() {
            let what = if kind == DefKind::Arrow && i == 1 { "시작점" } else { "벡터" };
            match t {
                Ty::Unknown => ok = false,
                Ty::Vector { n } => match n_seen {
                    None => n_seen = Some((*n, *s)),
                    Some((m, ms)) if m != *n => {
                        self.err(ms.to(*s), code::SHAPE, format!("{name}의 벡터 길이가 다름 ({m}과 {n})"));
                        ok = false;
                    }
                    _ => {}
                },
                t => {
                    let sug = if *t == Ty::Scalar { "[x, y] 모양의 벡터" } else { "열벡터 [1, 2]" };
                    self.err_s(*s, code::SHAPE, format!("{name}의 {what}는 벡터여야 함 (이 식은 {})", t.describe()), sug);
                    ok = false;
                }
            }
        }
        let ty = match n_seen {
            Some((n, s)) if ok => {
                if !(2..=3).contains(&n) {
                    self.err(s, code::SHAPE, format!("{name}의 벡터 길이는 2 또는 3 (좌표축 개수) — 이 벡터는 {n}"));
                    Ty::Unknown
                } else {
                    Ty::Vector { n }
                }
            }
            _ => Ty::Unknown,
        };
        body.ty = ty.clone();
        ty
    }

    fn resolve_inner(&mut self, e: &mut Expr, sc: &mut Scope, info: &mut DefInfo) -> Ty {
        if !self.tick() {
            return Ty::Unknown;
        }
        // Only the node itself may be a parametric call, never its children.
        let tuple_ok = std::mem::take(&mut sc.tuple_ok);
        let span = e.span;
        let ty = match &mut e.kind {
            ExprKind::Num { .. } => Ty::Scalar,
            ExprKind::Error {} => Ty::Unknown,
            ExprKind::Name { name, kind } => {
                let (k, t) = self.resolve_name(name, span, sc, info);
                *kind = k;
                t
            }
            ExprKind::Call { callee, .. } if callee.key == "between" => {
                self.err_s(span, code::TYPE, "between은 대상 없는 highlight의 범위 칸에서만 쓸 수 있음", "H1 = highlight(; between(f, g))");
                Ty::Unknown
            }
            ExprKind::Call { callee, kind, args } => {
                let mut arg_tys = vec![];
                for a in args.iter_mut() {
                    let t = self.resolve(a, sc, info);
                    arg_tys.push((t, a.span));
                }
                if names::is_linalg(&callee.key) {
                    *kind = NameKind::Builtin;
                    self.linalg_call(&callee.key, &arg_tys, args, span)
                } else {
                    for (t, s) in &arg_tys {
                        self.expect_scalar(t, *s);
                    }
                    let (k, t) = self.resolve_call(callee, args.len(), span, sc, info, tuple_ok);
                    *kind = k;
                    t
                }
            }
            ExprKind::Sum { terms } => {
                let mut tys = vec![];
                for t in terms.iter_mut() {
                    let ty = self.resolve(&mut t.expr, sc, info);
                    tys.push((ty, t.expr.span));
                }
                self.same_shape(&tys, "더할")
            }
            ExprKind::Paren { inner } => self.resolve(inner, sc, info),
            ExprKind::Neg { arg } => {
                let t = self.resolve(arg, sc, info);
                if !t.is_array() {
                    self.expect_scalar(&t, arg.span);
                }
                if t.is_array() || t == Ty::Unknown { t } else { Ty::Scalar }
            }
            ExprKind::Product { factors } => {
                let mut arrays: Vec<(Ty, Span)> = vec![];
                let mut unknown = false;
                for f in factors.iter_mut() {
                    let t = self.resolve(f, sc, info);
                    if t.is_array() {
                        arrays.push((t, f.span));
                    } else {
                        unknown |= t == Ty::Unknown;
                        self.expect_scalar(&t, f.span);
                    }
                }
                match arrays.len() {
                    _ if unknown && arrays.len() < 2 => Ty::Unknown,
                    0 => Ty::Scalar,
                    1 => arrays.pop().unwrap().0,
                    _ => {
                        let (a, b) = (&self.src[arrays[0].1.range()], &self.src[arrays[1].1.range()]);
                        let sug = format!("matmul({a}, {b})");
                        self.err_s(arrays[0].1.to(arrays[1].1), code::MATRIX_OP, "행렬·벡터끼리 '*'는 쓸 수 없음 (원소끼리 곱인지 행렬 곱인지 모호함)", sug);
                        Ty::Unknown
                    }
                }
            }
            ExprKind::Div { num, den } => {
                let nt = self.resolve(num, sc, info);
                let dt = self.resolve(den, sc, info);
                if !nt.is_array() {
                    self.expect_scalar(&nt, num.span);
                }
                if dt.is_array() {
                    let (a, b) = (&self.src[num.span.range()], &self.src[den.span.range()]);
                    self.err_s(den.span, code::MATRIX_OP, "행렬·벡터로 나눌 수 없음", format!("matmul({a}, inv({b}))"));
                    Ty::Unknown
                } else {
                    self.expect_scalar(&dt, den.span);
                    match (nt, dt) {
                        (Ty::Unknown, _) | (_, Ty::Unknown) => Ty::Unknown,
                        (nt, _) if nt.is_array() => nt,
                        _ => Ty::Scalar,
                    }
                }
            }
            ExprKind::Pow { base, exp } => {
                let bt = self.resolve(base, sc, info);
                let et = self.resolve(exp, sc, info);
                self.expect_scalar(&et, exp.span);
                if bt.is_array() {
                    let (b, x) = (&self.src[base.span.range()], &self.src[exp.span.range()]);
                    let x = x.trim_start_matches('{').trim_end_matches('}');
                    let x = x.strip_prefix('(').and_then(|x| x.strip_suffix(')')).unwrap_or(x);
                    match bt {
                        Ty::Matrix { m, n } if m == n => self.err_s(span, code::MATRIX_OP, "행렬에 '^'는 쓸 수 없음 (거듭제곱은 pow)", format!("pow({b}, {x})")),
                        _ => self.err(span, code::MATRIX_OP, format!("{}에 '^'는 쓸 수 없음 (pow는 정사각 행렬만)", bt.describe())),
                    }
                    Ty::Unknown
                } else {
                    self.expect_scalar(&bt, base.span);
                    Ty::Scalar
                }
            }
            ExprKind::Factorial { arg } => {
                let t = self.resolve(arg, sc, info);
                self.expect_scalar(&t, arg.span);
                Ty::Scalar
            }
            ExprKind::BigOp { body, index, lo, hi, .. } => {
                let ik = index.key.clone();
                let clash = sc.inputs.contains(&ik)
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
                self.err(span, code::TYPE, "문자열은 memo 칸이나 변수 값으로만 쓸 수 있음");
                Ty::Unknown
            }
            ExprKind::Tuple { .. } => {
                self.err_s(span, code::TYPE, "튜플 (a, b)는 매개변수 함수의 출력 칸에만 씀", "p(t) -> (x, y): (cos(t), sin(t)), 0 <= t <= 2*\\pi");
                Ty::Unknown
            }
            ExprKind::List { items } => {
                let mut tys = vec![];
                for i in items.iter_mut() {
                    let t = self.resolve(i, sc, info);
                    tys.push(t);
                }
                self.list_ty(items, &tys, span)
            }
            ExprKind::Index { base, indices } => {
                let bt = self.resolve(base, sc, info);
                for i in indices.iter_mut() {
                    let t = self.resolve(i, sc, info);
                    self.expect_scalar(&t, i.span);
                }
                self.index_ty(base, &bt, indices, span);
                Ty::Scalar
            }
            ExprKind::Address { .. } => {
                self.err(span, code::TYPE, "부분식 주소는 highlight·req의 대상 칸에만 쓸 수 있음");
                Ty::Unknown
            }
            ExprKind::Calc { .. } => {
                let call = self.src[span.range()].to_string();
                self.err_s(span, code::TYPE, "calc는 식 안에 쓸 수 없음 (정의 하나로 씀)", format!("define:\n    C: {call}"));
                Ty::Unknown
            }
            ExprKind::Feature { callee, .. } => {
                let call = self.src[span.range()].to_string();
                self.err_s(span, code::TYPE, format!("{}은(는) 식 안에 쓸 수 없음 (정의 하나로 씀)", callee.key), format!("define:\n    E: {call}"));
                Ty::Unknown
            }
            // Only ever a whole body (parser); `define` routes it to `resolve_split`.
            ExprKind::Split { .. } => {
                self.err(span, code::TYPE, "split은 정의 본문 전체로만 씀");
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
        if let Some((base, _)) = deriv::related(name)
            && !self.state.functions.contains_key(key)
        {
            if self.state.functions.contains_key(&base) && name.deriv > 0 {
                self.err_s(span, code::NOT_VALUE, format!("함수 '{key}'는 불러서 써야 함"), format!("{key}({})", sc.input()));
            } else {
                self.err(span, code::UNDECLARED, format!("'{key}'가 정의되지 않음"));
            }
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if sc.indices.contains(key) {
            return (NameKind::Index, Ty::Scalar);
        }
        if sc.split_name.as_ref() == Some(key) {
            return (NameKind::SplitName, Ty::Scalar);
        }
        if sc.inputs.contains(key) {
            return (NameKind::Input, Ty::Scalar);
        }
        if sc.consts.contains(key) || (sc.free_consts && self.state.constants.contains_key(key)) {
            if info.in_bound > 0 {
                info.int_consts.insert(key.clone());
            }
            info.used_consts.insert(key.clone());
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
        if let Some(f) = self.state.functions.get(key).filter(|f| matches!(f.kind, DefKind::Value | DefKind::Feature))
            && *key != sc.def_key
        {
            info.callees.insert(key.clone());
            return (NameKind::Value, f.ty.clone());
        }
        if self.failed.contains(key) && !self.state.functions.contains_key(key) {
            // Its definition failed in this request (already reported).
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if !info.reported.insert(key.clone()) {
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if !sc.def_key.is_empty() && *key == sc.def_key {
            self.err(span, code::CYCLE, format!("'{key}'가 자기 자신을 참조함"));
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if let Some(f) = self.state.functions.get(key).filter(|f| matches!(f.kind, DefKind::Span | DefKind::Arrow)) {
            self.err(span, code::TYPE, format!("{} '{key}'는 그리는 정의라 식에 쓸 수 없음", f.kind.word()));
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if self.state.functions.get(key).is_some_and(|f| f.kind == DefKind::Calc) {
            self.err(span, code::TYPE, format!("계산 과정 '{key}'는 값이 없어 식에 쓸 수 없음 (단계 정의를 직접 씀)"));
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if self.is_function_name(key) || names::is_builtin(key) {
            self.err_s(span, code::NOT_VALUE, format!("함수 '{key}'는 불러서 써야 함"), format!("{key}({})", sc.input()));
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if let Some(kind) = self.state.kind_of(key).filter(|k| *k != "상수") {
            let place = if sc.def_key.is_empty() { "범위 칸" } else { "함수 식" };
            self.err(span, code::TYPE, format!("{kind} '{key}'는 {place}에 쓸 수 없음"));
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if sc.def_key.is_empty() {
            let axes = sc.inputs.join(", ");
            self.err(span, code::UNDECLARED, format!("'{key}'가 선언되지 않음 (여기에는 축 {axes}, 상수, 변수, 함수 호출을 씀)"));
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if sc.outputs.contains(key) {
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

    fn resolve_call(&mut self, callee: &Ident, argc: usize, span: Span, sc: &Scope, info: &mut DefInfo, tuple_ok: bool) -> (NameKind, Ty) {
        let key = &callee.key;
        if is_bad_tex(callee) {
            return (NameKind::Unresolved, Ty::Unknown);
        }
        if let Some(r) = self.resolve_related_call(callee, argc, span, sc, info) {
            return r;
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
        if !sc.def_key.is_empty() && *key == sc.def_key {
            self.err(span, code::CYCLE, format!("'{key}'가 자기 자신을 부름 (재귀는 쓸 수 없음)"));
            return (NameKind::Function, Ty::Unknown);
        }
        if let Some(f) = self.state.functions.get(key).cloned() {
            match f.kind {
                _ if f.family.is_some() => {
                    self.err(span, code::TYPE, format!("반복 정의 '{key}'는 값이 여럿이라 식 안에서 부를 수 없음"));
                    return (NameKind::Unresolved, Ty::Unknown);
                }
                DefKind::Curve | DefKind::Matrix | DefKind::Polar => {}
                DefKind::Parametric if tuple_ok => {}
                DefKind::Parametric => {
                    let msg = if sc.in_slot { format!("매개변수 함수 '{key}'는 출력이 여럿이라 출력 칸 하나에 넣을 수 없음") } else { format!("매개변수 함수 '{key}'는 출력이 여럿이라 식 안에서 부를 수 없음") };
                    self.err(span, code::TYPE, msg);
                    return (NameKind::Unresolved, Ty::Unknown);
                }
                DefKind::Equation => {
                    self.err(span, code::TYPE, format!("방정식 '{key}'는 값이 없어 부를 수 없음"));
                    return (NameKind::Unresolved, Ty::Unknown);
                }
                DefKind::Value => {
                    self.err_s(span, code::NOT_FUNCTION, format!("'{key}'는 {}라 부를 수 없음", f.ty.describe()), key.clone());
                    return (NameKind::Unresolved, Ty::Unknown);
                }
                DefKind::Span | DefKind::Arrow => {
                    self.err(span, code::TYPE, format!("{} '{key}'는 그리는 정의라 식에 쓸 수 없음", f.kind.word()));
                    return (NameKind::Unresolved, Ty::Unknown);
                }
                DefKind::Calc => {
                    self.err(span, code::TYPE, format!("계산 과정 '{key}'는 값이 없어 부를 수 없음 (단계 정의를 직접 부름)"));
                    return (NameKind::Unresolved, Ty::Unknown);
                }
                DefKind::Feature => {
                    self.err_s(span, code::NOT_FUNCTION, format!("특징점 '{key}'는 부를 수 없음 (점 하나일 때 좌표는 {key}_[1], {key}_[2])"), format!("{key}_[1]"));
                    return (NameKind::Unresolved, Ty::Unknown);
                }
            }
            if f.inputs.len() != argc {
                let n = f.inputs.len();
                self.err(span, code::ARITY, format!("{key}는 인자 {n}개를 받음 ({argc}개 줌)"));
            }
            info.callees.insert(key.clone());
            return (NameKind::Function, f.ty.clone());
        }
        if self.failed.contains(key) {
            return (NameKind::Function, Ty::Unknown);
        }
        if sc.inputs.contains(key) || sc.consts.contains(key) || sc.indices.contains(key) {
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

/// Words with a meaning of their own (stage 2, `family` of stage 5) that cannot be names.
fn is_stage2_word(key: &str) -> bool {
    matches!(key, "highlight" | "req" | "between" | "family") || base_style_color(key).is_some()
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
    let known = |s: &str| sc.inputs.iter().any(|i| i == s) || sc.consts.iter().any(|c| c == s) || sc.indices.iter().any(|c| c == s);
    let parts: Vec<String> = key.chars().map(|c| c.to_string()).collect();
    parts.iter().all(|p| known(p)).then(|| parts.join("*"))
}

// ----- user operations --------------------------------------------------

/// Browser-side operations, logged in the same sequence as requests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum UserOp {
    /// Show / hide a function or highlight in a scene.
    Toggle {
        scene: String,
        target: String,
        visible: bool,
    },
    /// Slider released / value typed.
    SetValue { constant: String, value: f64 },
    SetRange { constant: String, min: f64, max: f64 },
    /// Mark a req done / pending (user in the browser, or Claude via `get_requests`).
    SetRequestDone {
        #[ts(type = "number")]
        number: u64,
        done: bool,
    },
    /// The user's clip for a scene as DSL text; `None` = back to Claude's clip.
    SetClip { scene: String, src: Option<String> },
}

impl UserOp {
    /// Short DSL-like text for the history view and responses.
    pub fn describe(&self) -> String {
        match self {
            UserOp::Toggle { target, visible, .. } => format!("{} {target}", if *visible { "켜기" } else { "끄기" }),
            UserOp::SetValue { constant, value } => format!("slider {constant} = {}", fmt_num(*value)),
            UserOp::SetRange { constant, min, max } => format!("range {constant}: {} .. {}", fmt_num(*min), fmt_num(*max)),
            UserOp::SetRequestDone { number, done } => format!("req #{number} {}", if *done { "처리됨" } else { "대기" }),
            UserOp::SetClip { scene, src } => format!("clip scene({scene}): {}", src.as_deref().unwrap_or("처음 값")),
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
        UserOp::Toggle { scene, target, visible } => {
            if !new.functions.contains_key(target) && !new.highlights.contains_key(target) {
                return Err(format!("'{target}'라는 정의가 없음"));
            }
            let s = new.scenes.get_mut(scene).ok_or_else(|| format!("scene({scene})이 없음"))?;
            if !s.members.contains(target) {
                return Err(format!("'{target}'는 {}의 좌표계에 맞지 않음", s.label()));
            }
            s.toggles.insert(target.clone(), *visible);
        }
        UserOp::SetClip { scene, src } => {
            let clip = match src {
                None => None,
                Some(text) => Some(scene::user_clip(&new, scene, text)?),
            };
            let s = new.scenes.get_mut(scene).ok_or_else(|| format!("scene({scene})이 없음"))?;
            s.user_clip = clip;
        }
        UserOp::SetRequestDone { number, done } => {
            let r = new.requests.get_mut(number).ok_or_else(|| format!("요청사항 #{number}이 없음"))?;
            r.done = *done;
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

mod deriv;
mod calc;
mod feature;
mod hl;
mod scene;
mod drawn;
mod split;
pub use hl::{address_resolves, unescape};
pub use scene::navigate;
#[cfg(test)]
mod tests;
