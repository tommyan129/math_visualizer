//! Statement AST. Server-only (history and resolution); the browser never sees statements.

use crate::ast::{Expr, Ident};
use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `define:` block. Each entry is its own unit for error recovery and application.
    Define { defs: Vec<Def>, span: Span },
    /// `del f, g, scene(2)`.
    Del { names: Vec<(Ident, Span)>, scenes: Vec<(SceneRef, Span)>, span: Span },
    /// `constant:` block: slider conditions.
    Constant { entries: Vec<ConstEntry>, span: Span },
    /// `style name:` block with indented `prop: value` lines.
    Style { name: Ident, name_span: Span, props: Vec<StyleProp>, span: Span },
    /// `name = value`: a user variable, or a highlight when the value is `highlight(...)`.
    Assign { name: Ident, name_span: Span, value: AssignValue, span: Span },
    /// `req(target; range; memo)`. A `scene(1)` / `scene(1).step(3)` target is parsed
    /// separately (`scene`, `step` are reserved words, not expressions).
    Req { slots: Slots, scene: Option<ReqScene>, span: Span },
    /// `scene(1):` block (stage 3).
    Scene(SceneDef),
}

/// `scene(1).step(3)` as a req target: the scene, the step number and its span, the whole span.
pub type ReqScene = (SceneRef, Option<(u32, Span)>, Span);

/// `scene(1)` or `scene(2D(x, y))` (a default scene; never defined or deleted by name).
#[derive(Debug, Clone, PartialEq)]
pub enum SceneRef {
    Number(u32),
    System(Space),
}

/// Coordinate system notation `2D(x, y)` (`3D`, `polar` parse for later stages).
#[derive(Debug, Clone, PartialEq)]
pub struct Space {
    /// `2D`, `3D`, `polar`.
    pub kind: String,
    pub axes: Vec<(Ident, Span)>,
    pub span: Span,
}

impl Space {
    /// `2D(x, y)`: the key of the coordinate system and of its default scene.
    pub fn key(&self) -> String {
        format!("{}({})", self.kind, self.axes.iter().map(|(a, _)| a.key.as_str()).collect::<Vec<_>>().join(", "))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneDef {
    pub id: SceneRef,
    /// `scene(1)` head.
    pub id_span: Span,
    /// `space`, `show`, `hide`, `clip`, `view` in written order.
    pub fields: Vec<Field>,
    pub steps: Vec<StepDef>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepDef {
    /// `show`, `set`, `animate`, `view`, `note`.
    pub fields: Vec<Field>,
    /// `step:` keyword.
    pub head: Span,
    pub span: Span,
}

/// `name: value` inside a scene or step block. Field names are recognized only at the
/// start of a line inside the block (design.md "칸 이름"), so they are not reserved words.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub key: String,
    pub key_span: Span,
    pub value: FieldValue,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum FieldValue {
    /// `space: 2D(x, y)`
    Space(Space),
    /// Scene `show` / `hide`: `f, H1`.
    Names(Vec<(Ident, Span)>),
    /// Step `show`: `+T1, -H1` (`true` = on).
    Toggles(Vec<(bool, Ident, Span)>),
    /// `clip`, `view`: an inequality.
    Expr(Expr),
    /// `set: a = 2, b = -1`
    Set(Vec<SetItem>),
    /// `animate: n = 4 -> 64, 2s, loop`
    Animate(AnimateItem),
    /// `note: "..."` or `note: + "..."` (append). The value is a string or a variable.
    Note { append: bool, value: Expr },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetItem {
    pub name: Ident,
    pub name_span: Span,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimateItem {
    pub name: Ident,
    pub name_span: Span,
    pub from: Expr,
    pub to: Expr,
    /// Seconds as written (`2s`, `0.5s`).
    pub duration: Option<(String, Span)>,
    pub looped: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AssignValue {
    Expr(Expr),
    /// `highlight(target; range; style; memo)`.
    Highlight(Slots),
}

/// `;`-separated argument slots of `highlight(...)` / `req(...)`; an empty slot is `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct Slots {
    pub items: Vec<Option<Expr>>,
    /// Span of each slot (an empty slot gets the position of its separator).
    pub spans: Vec<Span>,
    /// `highlight(...)` including the head.
    pub span: Span,
}

impl Slots {
    pub fn get(&self, i: usize) -> Option<&Expr> {
        self.items.get(i).and_then(|e| e.as_ref())
    }
}

/// `a: integer, range=(-5, 5), default=1, step=0.1`
#[derive(Debug, Clone, PartialEq)]
pub struct ConstEntry {
    pub name: Ident,
    pub name_span: Span,
    /// `real` / `integer` written first.
    pub kind: Option<(String, Span)>,
    pub props: Vec<Prop>,
    pub span: Span,
}

/// `key=value` in a constant entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Prop {
    pub key: String,
    pub key_span: Span,
    pub value: Expr,
}

/// `color: orange` inside a style block.
#[derive(Debug, Clone, PartialEq)]
pub struct StyleProp {
    pub key: String,
    pub key_span: Span,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Def {
    pub name: Ident,
    pub name_span: Span,
    /// `None` when written without parentheses (`A: [[1, 2]]`, stage 4).
    pub params: Option<Params>,
    pub output: Option<Output>,
    pub body: DefBody,
    /// `family(body; 0 <= k <= n)`: the body is repeated over the integer index (stage 5).
    pub family: Option<FamilyHead>,
    /// The whole entry, for source text in history and responses.
    pub span: Span,
}

/// Index range of `family(...; lo <= k <= hi)`.
#[derive(Debug, Clone, PartialEq)]
pub struct FamilyHead {
    pub index: Ident,
    pub lo: Expr,
    pub lo_strict: bool,
    pub hi: Expr,
    pub hi_strict: bool,
    /// `lo <= k <= hi` as written.
    pub range_span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    pub inputs: Vec<(Ident, Span)>,
    /// After `;`.
    pub consts: Vec<(Ident, Span)>,
    /// `(...)` including the parentheses.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    /// `-> y`
    Axis(Ident, Span),
    /// `-> (x, y)` (stage 5)
    Tuple(Vec<(Ident, Span)>, Span),
    /// `-> polar(r)` (stage 5)
    Coord { system: Ident, axes: Vec<(Ident, Span)>, span: Span },
    /// `-> matrix` (stage 4)
    Matrix(Span),
}

impl Output {
    pub fn span(&self) -> Span {
        match self {
            Output::Axis(_, s) | Output::Tuple(_, s) | Output::Matrix(s) => *s,
            Output::Coord { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DefBody {
    Expr(Expr),
    /// `lhs = rhs` (implicit curve, stage 5).
    Equation { lhs: Expr, rhs: Expr },
    /// `expr, range` (parametric, stage 5).
    WithRange { expr: Expr, range: Expr },
}
