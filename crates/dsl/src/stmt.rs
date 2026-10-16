//! Statement AST. Server-only (history and resolution); the browser never sees statements.

use crate::ast::{Expr, Ident};
use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `define:` block. Each entry is its own unit for error recovery and application.
    Define { defs: Vec<Def>, span: Span },
    /// `del f, g`.
    Del { names: Vec<(Ident, Span)>, span: Span },
    /// `constant:` block: slider conditions.
    Constant { entries: Vec<ConstEntry>, span: Span },
    /// `style name:` block with indented `prop: value` lines.
    Style { name: Ident, name_span: Span, props: Vec<StyleProp>, span: Span },
    /// `name = value`: a user variable, or a highlight when the value is `highlight(...)`.
    Assign { name: Ident, name_span: Span, value: AssignValue, span: Span },
    /// `req(target; range; memo)`.
    Req { slots: Slots, span: Span },
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
    /// The whole entry, for source text in history and responses.
    pub span: Span,
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
    /// Indented block under the head (`split`, stage 5). Contents are skipped.
    Block(Span),
}
