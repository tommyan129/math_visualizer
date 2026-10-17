//! Resolved state model of one workspace. This is what the browser receives
//! (never raw statements), and what the request log stores as applied results.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ast::{Expr, ExprKind, Ident, Ty};

/// Key of the default scene of the `2D(x, y)` system.
pub const DEFAULT_SCENE: &str = "2D(x, y)";

/// What a `define:` entry is. Everything lives in `State::functions` so references,
/// cascade, constants and req work the same for all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum DefKind {
    /// `f(x) -> y`: drawn as a curve. With two inputs, `g(x, y) -> z` is a surface in
    /// `3D(x, y, z)` (stage 6).
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
    /// `p(t) -> (x, y): (cos(t), sin(t)), 0 <= t <= 2*\pi`: drawn as a curve over its range.
    /// `q(t) -> (x, y, z)` is a space curve, `s(u, v) -> (x, y, z)` a surface (stage 6).
    Parametric,
    /// `h(\theta) -> polar(r): 1 + cos(\theta)`: drawn in a `polar(r, \theta)` scene.
    Polar,
    /// `C(x, y; r): x^2 + y^2 = r^2`: implicit curve, no value (cannot be called).
    Equation,
    /// `E: extrema(f)`: feature points of a curve, found by the browser (stage 5b). Used in
    /// an expression it is the point `[x, y]` when there is exactly one.
    Feature,
    /// `C: calc(f, g, h)`: neighbouring steps of one calculation (stage 5c). Not drawn on
    /// the canvas; the panel shows the steps with the changed parts colored, and the
    /// browser verifies neighbours numerically.
    Calc,
}

impl DefKind {
    /// Shown in a scene (toggle list) at all.
    pub fn is_drawn(self) -> bool {
        !matches!(self, DefKind::Matrix | DefKind::Value)
    }

    pub fn word(self) -> &'static str {
        match self {
            DefKind::Curve => "함수",
            DefKind::Matrix => "행렬 값 함수",
            DefKind::Value => "벡터·행렬",
            DefKind::Span => "span",
            DefKind::Arrow => "arrow",
            DefKind::Parametric => "매개변수 함수",
            DefKind::Polar => "극좌표 함수",
            DefKind::Equation => "방정식",
            DefKind::Feature => "특징점",
            DefKind::Calc => "계산 과정",
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
    /// Output axis of a curve (`y`) or polar function (`r`); `None` for every other kind.
    pub output: Option<Ident>,
    /// Output axes of a parametric function (`x`, `y`); empty for every other kind.
    #[serde(default)]
    pub outputs: Vec<Ident>,
    /// Parameter range of a parametric function (required) or polar function (optional;
    /// `None` = 0 <= θ <= 2π). For a parametric surface, the first parameter's.
    #[serde(default)]
    pub range: Option<ParamRange>,
    /// Range of the second parameter of a parametric surface `s(u, v) -> (x, y, z)`.
    #[serde(default)]
    pub range2: Option<ParamRange>,
    /// `family(...; 0 <= k <= n)`: one copy per integer k (stage 5). Body and range use k.
    #[serde(default)]
    pub family: Option<FamilyIndex>,
    /// Feature-point definitions (stage 5b): what to find, resolved for the browser.
    #[serde(default)]
    pub feature: Option<FeatureSpec>,
    /// Calculation-steps definitions (stage 5c).
    #[serde(default)]
    pub calc: Option<CalcSpec>,
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
    /// `lo < hi` between neighboring split boundary points that use constants (design.md
    /// "경계점 순서가 곧 상수 조건"). The browser keeps sliders from breaking them.
    #[serde(default)]
    pub constraints: Vec<OrderConstraint>,
    /// Server-generated derivatives of the body, order 1, 2, … (index 0 = order 1), only
    /// up to the highest order something needs (design.md "f@n"). Not names: a Claude
    /// `f@n` definition takes precedence for its order.
    #[serde(default)]
    pub derivs: Vec<Expr>,
    /// Data color index, fixed at creation.
    pub color: u32,
    /// Creation order, for listing.
    #[ts(type = "number")]
    pub order: u64,
    /// Log sequence of the last change; the browser recomputes only changed objects.
    #[ts(type = "number")]
    pub version: u64,
}

/// `lo <= t <= hi` of a parametric / polar function. Bounds use constants (and function
/// calls), never the parameter; strict and non-strict draw the same.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ParamRange {
    pub lo: Expr,
    pub hi: Expr,
    /// As written.
    pub src: String,
}

/// Index range of a repeated definition. Bounds are integers (constants in them are
/// integer sliders, as in `sum`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FamilyIndex {
    pub index: Ident,
    pub lo: Expr,
    pub lo_strict: bool,
    pub hi: Expr,
    pub hi_strict: bool,
    /// `0 <= k <= n` as written.
    pub src: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum FeatureKind {
    Roots,
    Intersect,
    Yint,
    Extrema,
    Inflections,
    Argmax,
    Argmin,
    Breaks,
    Tangents,
}

impl FeatureKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "roots" => Self::Roots,
            "intersect" => Self::Intersect,
            "yint" => Self::Yint,
            "extrema" => Self::Extrema,
            "inflections" => Self::Inflections,
            "argmax" => Self::Argmax,
            "argmin" => Self::Argmin,
            "breaks" => Self::Breaks,
            "tangents" => Self::Tangents,
            _ => return None,
        })
    }

    /// Derivative order of the target the browser needs (generated when Claude has not
    /// defined `f@n`): sign changes of f' for extrema and interval max / min, of f'' for
    /// inflection points, x'(t) / y'(t) (or r'(θ)) for tangents. Roots, intersections and
    /// breaks are found from values.
    pub fn deriv_order(self) -> u32 {
        match self {
            Self::Extrema | Self::Argmax | Self::Argmin | Self::Tangents => 1,
            Self::Inflections => 2,
            Self::Roots | Self::Intersect | Self::Yint | Self::Breaks => 0,
        }
    }
}

/// What a feature-point definition finds (`E: extrema(f; -1 < x < 1; max)`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FeatureSpec {
    pub kind: FeatureKind,
    /// Target curve (`tangents`: a parametric or polar function).
    pub target: String,
    /// `intersect`: the other curve as an expression in the target's input (`g(x)`, `2*x`).
    pub other: Option<Expr>,
    /// Range slot: a region over the target's input; every point is checked against it.
    pub range: Option<Expr>,
    /// Search interval from the range (constant expressions); `None` = the computed area
    /// on that side.
    pub lo: Option<Expr>,
    pub hi: Option<Expr>,
    /// Third slot (`max`, `hole`, `vertical`, …).
    pub filter: Option<String>,
    /// Coordinate system the points are drawn in (the target's): `polar(r, \theta)` or 2D.
    pub polar: bool,
    pub axes: Vec<Ident>,
    /// `breaks`: expressions in the target's input whose zeros may be breaks, from the
    /// target's AST (design.md "식이 깨지는 점"). Refreshed when the target changes.
    pub candidates: Vec<BreakCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BreakCandidate {
    pub cause: BreakCause,
    pub expr: Expr,
}

/// Where a break candidate came from; the browser classifies by the values on both sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum BreakCause {
    /// Denominator = 0 (or `tan`, a negative power): hole or vertical asymptote.
    Denominator,
    /// `sqrt` / `log` argument = 0 (`asin` / `acos` argument = ±1): domain endpoint.
    Domain,
    /// `abs` argument = 0: cusp candidate.
    Abs,
    /// split boundary (or `sign` argument = 0): jump candidate.
    Boundary,
    /// `floor` / `ceil` / `mod` argument (`round` argument − 1/2) at an integer: jump candidates.
    Step,
}

/// What the neighbouring steps of a `calc` claim about each other (third slot).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum CalcKind {
    /// Empty slot: every step is the same function rewritten (neighbours equal).
    Transform,
    /// `integral`: the second step is an antiderivative of the first (its derivative is
    /// the first; a constant difference is allowed); later steps rewrite the second.
    Integral,
    /// `limit`: not verified numerically (marked "검증 안 됨").
    Limit,
}

impl CalcKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "integral" => Self::Integral,
            "limit" => Self::Limit,
            _ => return None,
        })
    }
}

/// `C: calc(f, g, h; x > 0; integral)` resolved for the browser (stage 5c).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CalcSpec {
    pub kind: CalcKind,
    /// Step definitions in order (two or more, distinct).
    pub steps: Vec<String>,
    /// The steps' shared input (`x`, `\theta`).
    pub input: Ident,
    /// Range slot: a region over the input; verification points are taken inside it.
    pub range: Option<Expr>,
    /// Verification interval from the range (constant expressions); `None` = the default
    /// on that side.
    pub lo: Option<Expr>,
    pub hi: Option<Expr>,
    /// Coordinate system of the steps (the first step's).
    pub polar: bool,
    pub axes: Vec<Ident>,
    /// Changed parts between neighbours (`diffs[i]`: steps i and i + 1), from comparing
    /// their ASTs. Refreshed when a step changes.
    pub diffs: Vec<StepDiff>,
    /// Break candidates of every step (verification points keep away from them).
    pub candidates: Vec<BreakCandidate>,
}

/// Node ids (Sum term ids included) that differ between two neighbouring step bodies.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StepDiff {
    /// In the earlier step: parts that were replaced or removed.
    pub before: Vec<u32>,
    /// In the later step: parts that are new.
    pub after: Vec<u32>,
}

/// `lo < hi`, both constant expressions (numbers and constants only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OrderConstraint {
    pub lo: Expr,
    pub hi: Expr,
    /// `a < b` in DSL notation.
    pub src: String,
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
            (DefKind::Polar, Some(o)) => format!("{}({inputs}{consts}) -> polar({})", self.key, o.key),
            (DefKind::Parametric, _) => {
                format!("{}({inputs}{consts}) -> ({})", self.key, self.outputs.iter().map(|o| o.key.as_str()).collect::<Vec<_>>().join(", "))
            }
            (DefKind::Equation, _) => format!("{}({inputs}{consts})", self.key),
            (DefKind::Matrix, _) => format!("{}({inputs}{consts}) -> matrix", self.key),
            _ if self.consts.is_empty() => self.key.clone(),
            _ => format!("{}({inputs}{consts})", self.key),
        }
    }

    /// `(input, output)` of a one-input curve (not a surface).
    pub fn curve_axes(&self) -> Option<(&Ident, &Ident)> {
        match (self.kind, self.inputs.as_slice(), &self.output) {
            (DefKind::Curve, [i], Some(o)) => Some((i, o)),
            _ => None,
        }
    }

    /// `g(x, y) -> z` (stage 6).
    pub fn is_surface(&self) -> bool {
        self.kind == DefKind::Curve && self.inputs.len() == 2
    }

    /// Drawn in a 3D system: a surface, a space curve or a parametric surface.
    pub fn is_3d(&self) -> bool {
        self.system().is_some_and(|(_, ax)| ax.len() == 3)
    }

    /// Coordinate system it is drawn in: `(polar, axes)` — `2D(x, y)` for curves
    /// (input, output), parametric functions (outputs) and equations (inputs);
    /// `3D(x, y, z)` for surfaces (inputs, output) and parametric functions with three
    /// outputs; `polar(r, \theta)` for polar functions. `None` for spans, arrows and
    /// undrawn kinds.
    pub fn system(&self) -> Option<(bool, Vec<&Ident>)> {
        match self.kind {
            DefKind::Curve if self.is_surface() => Some((false, vec![&self.inputs[0], &self.inputs[1], self.output.as_ref()?])),
            DefKind::Curve => self.curve_axes().map(|(i, o)| (false, vec![i, o])),
            DefKind::Parametric => Some((false, self.outputs.iter().collect())),
            DefKind::Equation => Some((false, self.inputs.iter().collect())),
            DefKind::Polar => Some((true, vec![self.output.as_ref()?, self.inputs.first()?])),
            DefKind::Feature => self.feature.as_ref().map(|f| (f.polar, f.axes.iter().collect())),
            DefKind::Calc => self.calc.as_ref().map(|c| (c.polar, c.axes.iter().collect())),
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

    /// Full definition in DSL notation. A split body goes on indented lines under the head.
    pub fn source(&self) -> String {
        if matches!(self.body.kind, ExprKind::Split { .. }) {
            let lines: Vec<String> = self.body_src.lines().enumerate().map(|(i, l)| format!("{}{l}", if i == 0 { "    " } else { "        " })).collect();
            return format!("{}:\n{}", self.signature(), lines.join("\n"));
        }
        let body = match (&self.range, &self.range2) {
            (Some(r), Some(r2)) => format!("{}, {}, {}", self.body_src, r.src, r2.src),
            (Some(r), None) => format!("{}, {}", self.body_src, r.src),
            _ => self.body_src.clone(),
        };
        match &self.family {
            Some(e) => format!("{}: family({body}; {})", self.signature(), e.src),
            None => format!("{}: {body}", self.signature()),
        }
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
    /// Horizontal and vertical axis; for a polar scene the radius and the angle.
    pub axes: Vec<Ident>,
    /// `polar(r, \theta)` system (otherwise `2D`).
    pub polar: bool,
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
    /// The user turned Claude's clip off (cleared the clip field); `user_clip` is then `None`.
    #[serde(default)]
    pub clip_off: bool,
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

/// `2D(x, y)` or, with three axes, `3D(x, y, z)`.
pub fn system_key(axes: &[&str]) -> String {
    format!("{}D({})", if axes.len() == 3 { 3 } else { 2 }, axes.join(", "))
}

/// `2D(x, y)` or `polar(r, \theta)`.
pub fn any_system_key(polar: bool, axes: &[&str]) -> String {
    if polar { format!("polar({})", axes.join(", ")) } else { system_key(axes) }
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

/// Camera range per axis; a missing axis keeps 1:1 scale around 0 (2D) or takes
/// -5..5 (3D box).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ViewSpec {
    pub h: Option<ViewRange>,
    pub v: Option<ViewRange>,
    /// Third axis of a 3D scene (the box height).
    #[serde(default)]
    pub d: Option<ViewRange>,
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
    #[serde(default)]
    pub polar: bool,
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

    /// Coordinate system key of a drawn definition (`Function::system`). Spans and arrows
    /// fit any 2D system with as many axes as their vectors have; other kinds are not drawn.
    pub fn function_system(&self, key: &str) -> Option<String> {
        let (polar, axes) = self.functions.get(key)?.system()?;
        Some(any_system_key(polar, &axes.iter().map(|a| a.key.as_str()).collect::<Vec<_>>()))
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
