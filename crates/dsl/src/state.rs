//! Resolved state model of one workspace. This is what the browser receives
//! (never raw statements), and what the request log stores as applied results.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ast::{Expr, Ident, Ty};

/// Key of the default scene of the `2D(x, y)` system.
pub const DEFAULT_SCENE: &str = "2D(x, y)";

/// What a `define:` entry is. Everything lives in `State::functions` so references,
/// cascade, constants and req work the same for all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum DefKind {
    /// `f(x) -> y`: drawn as a curve.
    #[default]
    Curve,
    /// `R(\theta) -> matrix`: vector- or matrix-valued, not drawn.
    Matrix,
    /// `A: [[1, 2], [3, 4]]`, `v(; a): [1, a]`: a named vector or matrix, not drawn.
    Value,
    /// `S: span(v, w)`: drawn as a point, line or plane through the origin.
    Span,
    /// `a: arrow(v)`, `arrow(v; P)`: drawn as an arrow.
    Arrow,
}

impl DefKind {
    /// Shown in a scene (toggle list) at all.
    pub fn is_drawn(self) -> bool {
        matches!(self, DefKind::Curve | DefKind::Span | DefKind::Arrow)
    }

    pub fn word(self) -> &'static str {
        match self {
            DefKind::Curve => "함수",
            DefKind::Matrix => "행렬 값 함수",
            DefKind::Value => "벡터·행렬",
            DefKind::Span => "span",
            DefKind::Arrow => "arrow",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Function {
    pub key: String,
    pub name: Ident,
    #[serde(default)]
    pub kind: DefKind,
    /// Empty for `Value`, `Span`, `Arrow`.
    pub inputs: Vec<Ident>,
    /// Output axis of a curve; `None` for every other kind.
    pub output: Option<Ident>,
    /// Value type of the body: scalar for a curve, vector / matrix for `Matrix` and
    /// `Value`; for `Span` / `Arrow` the vector type of their arguments.
    #[serde(default = "scalar_ty")]
    pub ty: Ty,
    /// Constant keys in signature order (after `;`). Complete: includes every
    /// constant used by functions this one calls (design.md "참조한 함수의 상수").
    pub consts: Vec<String>,
    pub body: Expr,
    /// Body as written, for DSL-notation responses.
    pub body_src: String,
    /// Direct references to other definitions: called functions and named vectors / matrices.
    pub calls: Vec<String>,
    /// Data color index, fixed at creation.
    pub color: u32,
    /// Creation order, for listing.
    #[ts(type = "number")]
    pub order: u64,
    /// Log sequence of the last change; the browser recomputes only changed objects.
    #[ts(type = "number")]
    pub version: u64,
}

fn scalar_ty() -> Ty {
    Ty::Scalar
}

impl Function {
    /// `f(x; a, b) -> y`, `R(\theta) -> matrix`, `v(; a)`, `S`.
    pub fn signature(&self) -> String {
        let inputs = self.inputs.iter().map(|i| i.key.as_str()).collect::<Vec<_>>().join(", ");
        let consts = if self.consts.is_empty() { String::new() } else { format!("; {}", self.consts.join(", ")) };
        match (&self.kind, &self.output) {
            (DefKind::Curve, Some(o)) => format!("{}({inputs}{consts}) -> {}", self.key, o.key),
            (DefKind::Matrix, _) => format!("{}({inputs}{consts}) -> matrix", self.key),
            _ if self.consts.is_empty() => self.key.clone(),
            _ => format!("{}({inputs}{consts})", self.key),
        }
    }

    /// `(input, output)` of a curve.
    pub fn curve_axes(&self) -> Option<(&Ident, &Ident)> {
        match (self.kind, self.inputs.first(), &self.output) {
            (DefKind::Curve, Some(i), Some(o)) => Some((i, o)),
            _ => None,
        }
    }

    /// Number of axes a span / arrow needs (its vector length).
    pub fn vector_dim(&self) -> Option<u32> {
        match (self.kind, &self.ty) {
            (DefKind::Span | DefKind::Arrow, Ty::Vector { n }) => Some(*n),
            _ => None,
        }
    }

    /// Full definition line in DSL notation.
    pub fn source(&self) -> String {
        format!("{}: {}", self.signature(), self.body_src)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Constant {
    pub key: String,
    pub name: Ident,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    /// Integer-only (declared `integer`, or used in a `sum`/`prod` bound).
    pub integer: bool,
    /// Declared in a `constant:` block (otherwise auto-created).
    #[serde(default)]
    pub declared: bool,
    /// The kind word (`real` / `integer`) was written; a `sum` bound cannot change it.
    #[serde(default)]
    pub kind_fixed: bool,
    #[ts(type = "number")]
    pub order: u64,
    #[ts(type = "number")]
    pub version: u64,
}

impl Constant {
    pub fn auto(name: Ident, integer: bool, order: u64) -> Self {
        let (value, min, max, step) = if integer { (5.0, 0.0, 20.0, 1.0) } else { (1.0, -10.0, 10.0, 0.1) };
        Constant { key: name.key.clone(), name, value, min, max, step, integer, declared: false, kind_fixed: false, order, version: 0 }
    }

    pub fn make_integer(&mut self) {
        self.integer = true;
        self.step = 1.0;
        self.min = self.min.round();
        self.max = self.max.round().max(self.min + 1.0);
        self.value = self.value.round().clamp(self.min, self.max);
    }
}

/// A scene: a view preset over one coordinate system (design.md "scene").
/// Claude scenes are keyed by number (`"1"`), default scenes by their system (`"2D(x, y)"`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(default)]
pub struct Scene {
    pub key: String,
    /// `None` = default scene (one per coordinate system, managed by the server).
    pub number: Option<u32>,
    /// Horizontal and vertical axis.
    pub axes: Vec<Ident>,
    /// Functions and highlights that fit the coordinate system, in creation order: the
    /// toggle list. Computed by the server; the browser does not apply placement rules.
    pub members: Vec<String>,
    /// Claude's clip (always applied unless the user adjusted it).
    pub clip: Option<Clip>,
    /// Camera on first entry.
    pub view: Option<ViewSpec>,
    /// Step 0 (scene `show` / `hide` / `view`) followed by one entry per `step:`.
    pub steps: Vec<StepState>,
    /// Claude scene fields as written (unresolved), re-resolved on every request.
    pub source: Option<SceneSource>,
    /// Block as written; empty for default scenes.
    pub src: String,
    /// Functions, highlights, constants and variables referred to.
    pub deps: Vec<String>,
    /// Log sequence of the last definition change (not user state).
    #[ts(type = "number")]
    pub defined: u64,
    /// User overrides of visibility (target → on). A step clears the ones it mentions.
    pub toggles: BTreeMap<String, bool>,
    /// The user's clip adjustment; `None` = Claude's clip.
    pub user_clip: Option<Clip>,
    #[ts(type = "number")]
    pub order: u64,
    #[ts(type = "number")]
    pub version: u64,
    /// Stage 1-2 shape: targets the user turned off. Migrated into `toggles` on load.
    #[serde(skip_serializing)]
    #[ts(skip)]
    pub hidden: Vec<String>,
}

impl Scene {
    pub fn is_default(&self) -> bool {
        self.number.is_none()
    }

    /// `scene(1)` / `scene(2D(x, y))`
    pub fn label(&self) -> String {
        format!("scene({})", self.key)
    }

    /// Number of `step:` blocks (step 0 not counted).
    pub fn step_count(&self) -> usize {
        self.steps.len().saturating_sub(1)
    }

    /// Visibility at `step`: the user's override, else the step's cached state.
    pub fn visible(&self, target: &str, step: usize) -> bool {
        if let Some(v) = self.toggles.get(target) {
            return *v;
        }
        self.steps.get(step.min(self.step_count())).and_then(|s| s.visible.get(target)).copied().unwrap_or(false)
    }
}

/// `2D(x, y)` with the given axis keys.
pub fn system_key(axes: &[&str]) -> String {
    format!("2D({})", axes.join(", "))
}

/// A resolved clip region over the scene axes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Clip {
    pub expr: Expr,
    /// As written (DSL notation).
    pub src: String,
    /// Constants used (recompute when they change).
    pub consts: Vec<String>,
}

/// Camera range per axis; a missing axis keeps 1:1 scale around 0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ViewSpec {
    pub h: Option<ViewRange>,
    pub v: Option<ViewRange>,
    pub src: String,
}

/// Bounds are constant-free expressions (`-2*\pi`), evaluated by the browser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ViewRange {
    pub lo: Expr,
    pub hi: Expr,
}

/// Full state at one step, cached by the server, plus what the step changed.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StepState {
    /// Visibility of every target the scene mentions so far.
    pub visible: BTreeMap<String, bool>,
    /// Predict-then-reveal: kept out of the toggle list until a step turns them on.
    pub concealed: Vec<String>,
    /// Constant values set at or before this step (last wins).
    pub values: BTreeMap<String, f64>,
    /// Camera at this step (the last view set at or before it; step 0 = scene view).
    pub view: Option<ViewSpec>,
    /// Note blocks shown at this step (replaced, or appended with `note: + ...`).
    pub note: Vec<NoteBlock>,
    /// Targets whose visibility this step sets.
    pub changed: Vec<String>,
    /// Constants this step sets (`set`, `animate` end values).
    pub changed_values: Vec<String>,
    pub changed_view: bool,
    pub changed_note: bool,
    /// Played on entry (forward only).
    pub animate: Vec<Animate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Animate {
    pub constant: String,
    pub from: f64,
    pub to: f64,
    /// Seconds.
    pub duration: f64,
    /// Back and forth until the user grabs the slider or leaves the step.
    pub looped: bool,
}

/// Default `animate` duration in seconds (stage-3 decision).
pub const ANIMATE_SECONDS: f64 = 2.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NoteBlock {
    pub segs: Vec<NoteSeg>,
    /// String as written (escapes applied).
    pub text: String,
}

/// Part of a note string (design.md "note 안의 수식").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum NoteSeg {
    Text { text: String },
    /// `$...$`: unchecked LaTeX.
    Tex { tex: String },
    /// `{expr}`: a resolved DSL expression.
    Math { expr: Expr },
    /// `{f}`: the whole definition of `f`, rendered from the current state.
    Def { function: String },
}

/// Scene fields as written (spans cleared), for re-resolution and cascade.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SceneSource {
    pub axes: Vec<Ident>,
    pub show: Vec<Ident>,
    pub hide: Vec<Ident>,
    pub clip: Option<Expr>,
    pub view: Option<Expr>,
    pub steps: Vec<StepSource>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StepSource {
    pub show: Vec<StepToggle>,
    pub set: Vec<SetSource>,
    pub animate: Option<AnimateSource>,
    pub view: Option<Expr>,
    pub note: Option<NoteSource>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StepToggle {
    pub on: bool,
    pub name: Ident,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SetSource {
    pub name: Ident,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AnimateSource {
    pub name: Ident,
    pub from: f64,
    pub to: f64,
    pub duration: f64,
    pub looped: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NoteSource {
    pub append: bool,
    /// A string literal or a variable name.
    pub value: Expr,
}

/// How a highlight fills (design.md "style" fill rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Fill {
    /// Between the curve and the x axis (default).
    Axis,
    /// y > f(x).
    Above,
    /// y < f(x).
    Below,
    /// Inside an inequality region (targetless highlights).
    Region,
}

impl Fill {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "axis" => Fill::Axis,
            "above" => Fill::Above,
            "below" => Fill::Below,
            "region" => Fill::Region,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Fill::Axis => "axis",
            Fill::Above => "above",
            Fill::Below => "below",
            Fill::Region => "region",
        }
    }
}

/// Base styles: one per color, fill to axis (stage-2 decision).
pub const BASE_STYLES: &[(&str, &str)] = &[
    ("blue", "#6ea8ff"),
    ("orange", "#ffa057"),
    ("green", "#5fd38a"),
    ("red", "#ff6b6b"),
    ("purple", "#c08cff"),
    ("yellow", "#f2cc60"),
    ("gray", "#a3a6af"),
];

/// Auto colors for highlights written without a style, in creation order.
pub const AUTO_COLORS: &[&str] = &["#5fd38a", "#ffa057", "#c08cff", "#6ea8ff", "#f2cc60", "#ff6b6b"];

pub const DEFAULT_OPACITY: f64 = 0.16;
pub const DEFAULT_WIDTH: f64 = 1.5;

pub fn base_style_color(name: &str) -> Option<&'static str> {
    BASE_STYLES.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

/// Resolved style of one highlight, as the browser draws it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HlStyle {
    /// `#rrggbb`.
    pub color: String,
    pub fill: Fill,
    /// Fill opacity 0..1.
    pub opacity: f64,
    /// Boundary line width, px (0 = none).
    pub width: f64,
}

/// User style (`style name:` block). Unset properties take the defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StyleDef {
    pub key: String,
    pub name: Ident,
    pub color: Option<String>,
    pub fill: Option<Fill>,
    pub opacity: Option<f64>,
    pub width: Option<f64>,
    /// Block source as written.
    pub src: String,
    #[ts(type = "number")]
    pub order: u64,
    #[ts(type = "number")]
    pub version: u64,
}

/// User variable: an untyped value kept by reference (design.md "사용자 변수").
/// The value stays unresolved; names are resolved where it is used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UserVar {
    pub key: String,
    pub name: Ident,
    pub value: Expr,
    /// Value as written.
    pub src: String,
    #[ts(type = "number")]
    pub order: u64,
    #[ts(type = "number")]
    pub version: u64,
}

/// What a highlight points at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum HlTarget {
    /// Targetless: the range is a region on the coordinate grid.
    Grid {},
    Function { function: String },
    /// Part of a function's expression. `nodes` are node ids in the current body (Sum
    /// term ids included, which cover the sign); several for `.coef` / `.var` over more
    /// than one factor.
    Sub { function: String, nodes: Vec<u32>, address: String },
}

/// Slot ASTs as written (unresolved, spans cleared): re-resolved whenever a
/// dependency changes, so addresses follow the expression's shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HlSource {
    pub target: Option<Expr>,
    pub range: Option<Expr>,
    pub style: Option<Expr>,
    pub memo: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Highlight {
    pub key: String,
    pub name: Ident,
    pub target: HlTarget,
    /// Resolved range for the browser: variables and `between` expanded. `None` = whole target.
    pub range: Option<Expr>,
    pub style: HlStyle,
    /// Style name used (`blue`, a user style), `None` = auto color.
    pub style_name: Option<String>,
    pub memo: Option<String>,
    /// Constants the drawing depends on (range and target function), for recompute.
    pub consts: Vec<String>,
    /// Functions, variables and styles this highlight refers to (cascade).
    pub deps: Vec<String>,
    /// Axis names the drawing uses: the target function's axes, or the axes a targetless
    /// range mentions (scene placement).
    #[serde(default)]
    pub axes: Vec<String>,
    pub source: HlSource,
    /// `highlight(...)` as written.
    pub src: String,
    /// Auto color fixed at creation (used when the style gives no color).
    pub auto_color: String,
    #[ts(type = "number")]
    pub order: u64,
    #[ts(type = "number")]
    pub version: u64,
}

/// What a req points at, fixed at its history step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum ReqTarget {
    Function { function: String },
    Highlight { highlight: String },
    /// `f.(b*x)`: re-checked against the current expression for the change status.
    Sub { function: String, address: String },
    /// `scene(1)` or `scene(1).step(3)` (step number as of the req).
    Scene { scene: String, step: Option<u32> },
}

/// `req(target; range; memo)` (user → Claude feedback). Fixed to its history step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Request {
    #[ts(type = "number")]
    pub number: u64,
    pub target: ReqTarget,
    /// Range in DSL notation with variables expanded, as of the request.
    pub range: Option<String>,
    pub memo: Option<String>,
    /// `req(...)` as written.
    pub src: String,
    /// Target definition at that step, in DSL notation ("what it pointed at").
    pub target_src: String,
    /// Functions the target depended on at that step (itself and its callees), for indirect changes.
    pub deps: Vec<String>,
    /// History step (log sequence) of the request.
    #[ts(type = "number")]
    pub seq: u64,
    pub done: bool,
    #[ts(type = "number")]
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct State {
    pub functions: BTreeMap<String, Function>,
    pub constants: BTreeMap<String, Constant>,
    pub scenes: BTreeMap<String, Scene>,
    #[serde(default)]
    pub highlights: BTreeMap<String, Highlight>,
    #[serde(default)]
    pub styles: BTreeMap<String, StyleDef>,
    #[serde(default)]
    pub vars: BTreeMap<String, UserVar>,
    /// Keyed by request number.
    #[serde(default)]
    #[ts(as = "BTreeMap<u32, Request>")]
    pub requests: BTreeMap<u64, Request>,
    pub counters: Counters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Counters {
    #[ts(type = "number")]
    pub next_order: u64,
    pub next_color: u32,
    #[serde(default)]
    pub next_hl_color: u32,
    #[serde(default)]
    #[ts(type = "number")]
    pub next_request: u64,
}

impl State {
    /// Kind of the object named `key`, for namespace checks and messages.
    pub fn kind_of(&self, key: &str) -> Option<&'static str> {
        if self.functions.contains_key(key) {
            Some("함수")
        } else if self.constants.contains_key(key) {
            Some("상수")
        } else if self.highlights.contains_key(key) {
            Some("강조")
        } else if self.styles.contains_key(key) {
            Some("style")
        } else if self.vars.contains_key(key) {
            Some("변수")
        } else {
            None
        }
    }

    /// Coordinate system of a curve: `2D(input, output)`. Spans and arrows fit any
    /// system with as many axes as their vectors have; other kinds are not drawn.
    pub fn function_system(&self, key: &str) -> Option<String> {
        let (i, o) = self.functions.get(key)?.curve_axes()?;
        Some(system_key(&[i.key.as_str(), o.key.as_str()]))
    }

    /// Stage 1-2 states: the always-present default scene with `hidden` lists. Moves
    /// `hidden` into `toggles`; default scenes are then kept in sync by the interpreter.
    pub fn migrate(&mut self) {
        for s in self.scenes.values_mut() {
            if s.axes.is_empty() && s.key == DEFAULT_SCENE {
                s.axes = vec![Ident::plain("x"), Ident::plain("y")];
                if s.steps.is_empty() {
                    s.steps = vec![StepState::default()];
                }
            }
            for k in std::mem::take(&mut s.hidden) {
                s.toggles.insert(k, false);
            }
        }
    }
}

/// One element of an applied result: the resolved state of a changed object.
/// Replaying changes from a checkpoint reproduces the state without re-parsing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
#[allow(clippy::large_enum_variant)]
pub enum Change {
    PutFunction { function: Function },
    RemoveFunction { key: String },
    PutConstant { constant: Constant },
    RemoveConstant { key: String },
    PutScene { scene: Scene },
    RemoveScene { key: String },
    SetCounters { counters: Counters },
    PutHighlight { highlight: Highlight },
    RemoveHighlight { key: String },
    PutStyle { style: StyleDef },
    RemoveStyle { key: String },
    PutVar { var: UserVar },
    RemoveVar { key: String },
    PutRequest { request: Request },
}

impl State {
    pub fn apply_changes(&mut self, changes: &[Change]) {
        for c in changes {
            match c {
                Change::PutFunction { function } => {
                    self.functions.insert(function.key.clone(), function.clone());
                }
                Change::RemoveFunction { key } => {
                    self.functions.remove(key);
                }
                Change::PutConstant { constant } => {
                    self.constants.insert(constant.key.clone(), constant.clone());
                }
                Change::RemoveConstant { key } => {
                    self.constants.remove(key);
                }
                Change::PutScene { scene } => {
                    self.scenes.insert(scene.key.clone(), scene.clone());
                }
                Change::RemoveScene { key } => {
                    self.scenes.remove(key);
                }
                Change::SetCounters { counters } => self.counters = *counters,
                Change::PutHighlight { highlight } => {
                    self.highlights.insert(highlight.key.clone(), highlight.clone());
                }
                Change::RemoveHighlight { key } => {
                    self.highlights.remove(key);
                }
                Change::PutStyle { style } => {
                    self.styles.insert(style.key.clone(), style.clone());
                }
                Change::RemoveStyle { key } => {
                    self.styles.remove(key);
                }
                Change::PutVar { var } => {
                    self.vars.insert(var.key.clone(), var.clone());
                }
                Change::RemoveVar { key } => {
                    self.vars.remove(key);
                }
                Change::PutRequest { request } => {
                    self.requests.insert(request.number, request.clone());
                }
            }
        }
    }

    /// Changes turning `old` into `self`; stamps `seq` as the version of every changed object.
    pub fn diff_from(&mut self, old: &State, seq: u64) -> Vec<Change> {
        let mut out = vec![];
        diff_map(&mut self.functions, &old.functions, seq, |f| &mut f.version, |f| Change::PutFunction { function: f }, |k| Change::RemoveFunction { key: k }, &mut out);
        diff_map(&mut self.constants, &old.constants, seq, |c| &mut c.version, |c| Change::PutConstant { constant: c }, |k| Change::RemoveConstant { key: k }, &mut out);
        diff_map(&mut self.scenes, &old.scenes, seq, |s| &mut s.version, |s| Change::PutScene { scene: s }, |k| Change::RemoveScene { key: k }, &mut out);
        diff_map(&mut self.styles, &old.styles, seq, |s| &mut s.version, |s| Change::PutStyle { style: s }, |k| Change::RemoveStyle { key: k }, &mut out);
        diff_map(&mut self.vars, &old.vars, seq, |v| &mut v.version, |v| Change::PutVar { var: v }, |k| Change::RemoveVar { key: k }, &mut out);
        diff_map(&mut self.highlights, &old.highlights, seq, |h| &mut h.version, |h| Change::PutHighlight { highlight: h }, |k| Change::RemoveHighlight { key: k }, &mut out);
        diff_map(&mut self.requests, &old.requests, seq, |r| &mut r.version, |r| Change::PutRequest { request: r }, |_| unreachable!("requests are never removed"), &mut out);
        if self.counters != old.counters {
            out.push(Change::SetCounters { counters: self.counters });
        }
        out
    }
}

#[allow(clippy::too_many_arguments)]
fn diff_map<K: Ord + Clone, T: Clone + PartialEq>(
    new: &mut BTreeMap<K, T>,
    old: &BTreeMap<K, T>,
    seq: u64,
    version: impl Fn(&mut T) -> &mut u64 + Copy,
    put: impl Fn(T) -> Change,
    remove: impl Fn(K) -> Change,
    out: &mut Vec<Change>,
) {
    for (k, v) in new.iter_mut() {
        match old.get(k) {
            Some(o) if same_ignoring_version(o, v, version) => *version(v) = *version(&mut o.clone()),
            _ => {
                *version(v) = seq;
                out.push(put(v.clone()));
            }
        }
    }
    for k in old.keys().filter(|k| !new.contains_key(*k)) {
        out.push(remove(k.clone()));
    }
}

fn same_ignoring_version<T: Clone + PartialEq>(old: &T, new: &T, version: impl Fn(&mut T) -> &mut u64) -> bool {
    let mut a = old.clone();
    *version(&mut a) = 0;
    let mut b = new.clone();
    *version(&mut b) = 0;
    a == b
}
