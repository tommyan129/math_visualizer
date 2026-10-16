//! Resolved state model of one workspace. This is what the browser receives
//! (never raw statements), and what the request log stores as applied results.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ast::{Expr, Ident};

/// Identifier of the stage-1 default scene.
pub const DEFAULT_SCENE: &str = "2D(x, y)";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Function {
    pub key: String,
    pub name: Ident,
    pub inputs: Vec<Ident>,
    pub output: Ident,
    /// Constant keys in signature order (after `;`). Complete: includes every
    /// constant used by functions this one calls (design.md "참조한 함수의 상수").
    pub consts: Vec<String>,
    pub body: Expr,
    /// Body as written, for DSL-notation responses.
    pub body_src: String,
    /// Direct callees (user functions only).
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

impl Function {
    /// `f(x; a, b) -> y`
    pub fn signature(&self) -> String {
        let inputs = self.inputs.iter().map(|i| i.key.as_str()).collect::<Vec<_>>().join(", ");
        let consts = if self.consts.is_empty() { String::new() } else { format!("; {}", self.consts.join(", ")) };
        format!("{}({inputs}{consts}) -> {}", self.key, self.output.key)
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

/// Per-scene user state. Stage 2 has only the default scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SceneState {
    pub key: String,
    /// Functions and highlights the user turned off.
    pub hidden: Vec<String>,
    #[ts(type = "number")]
    pub version: u64,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct State {
    pub functions: BTreeMap<String, Function>,
    pub constants: BTreeMap<String, Constant>,
    pub scenes: BTreeMap<String, SceneState>,
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

impl Default for State {
    fn default() -> Self {
        let mut scenes = BTreeMap::new();
        scenes.insert(DEFAULT_SCENE.to_string(), SceneState { key: DEFAULT_SCENE.to_string(), hidden: vec![], version: 0 });
        State {
            functions: BTreeMap::new(),
            constants: BTreeMap::new(),
            scenes,
            highlights: BTreeMap::new(),
            styles: BTreeMap::new(),
            vars: BTreeMap::new(),
            requests: BTreeMap::new(),
            counters: Counters::default(),
        }
    }
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
    PutScene { scene: SceneState },
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
        diff_map(&mut self.scenes, &old.scenes, seq, |s| &mut s.version, |s| Change::PutScene { scene: s }, |_| unreachable!("scenes are not removed in stage 2"), &mut out);
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
