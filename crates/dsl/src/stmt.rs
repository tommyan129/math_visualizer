//! Statement AST. Server-only (history and resolution); the browser never sees statements.

use crate::ast::{Expr, Ident};
use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `define:` block. Each entry is its own unit for error recovery and application.
    Define { defs: Vec<Def>, span: Span },
    /// `del f, g`.
    Del { names: Vec<(Ident, Span)>, span: Span },
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
