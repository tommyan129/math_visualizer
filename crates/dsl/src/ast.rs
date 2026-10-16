//! Expression AST (design.md "AST 구성").
//!
//! - serde internally tagged (`{"type": "Sum", ...}`), named fields everywhere.
//! - `span` is server-only (`#[serde(skip)]`, excluded from ts-rs).
//! - Sum / Product are n-ary; subtraction is a sign on a Sum term; Div is binary.
//! - `()` is a Paren node; `{}` creates no node.
//! - Node ids are pre-order within one definition, reassigned on every parse.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::span::Span;

pub type NodeId = u32;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Expr {
    pub id: NodeId,
    /// Static type from the type checker; `Unknown` until resolution.
    pub ty: Ty,
    #[serde(skip)]
    #[ts(skip)]
    pub span: Span,
    #[serde(flatten)]
    #[ts(flatten)]
    pub kind: ExprKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum ExprKind {
    /// Literal kept as written (`0.50` displays as `0.50`); `int` = digits only.
    Num { text: String, int: bool },
    Name { name: Ident, kind: NameKind },
    Sum { terms: Vec<Term> },
    Product { factors: Vec<Expr> },
    Div { num: Box<Expr>, den: Box<Expr> },
    Pow { base: Box<Expr>, exp: Box<Expr> },
    /// Unary minus (precedence 5). Binary minus is a `Term` sign.
    Neg { arg: Box<Expr> },
    Factorial { arg: Box<Expr> },
    /// Visible parentheses.
    Paren { inner: Box<Expr> },
    Call { callee: Ident, kind: NameKind, args: Vec<Expr> },
    /// `sum(body; lo <= k <= hi)` / `prod(...)`. Index is integer-valued.
    BigOp { op: BigOpKind, body: Box<Expr>, index: Ident, lo: Box<Expr>, lo_strict: bool, hi: Box<Expr>, hi_strict: bool },
    /// Same-direction chain: `0 <= x < 5`. `==` never chains.
    Compare { first: Box<Expr>, rest: Vec<CompareLink> },
    Not { arg: Box<Expr> },
    And { args: Vec<Expr> },
    Or { args: Vec<Expr> },
    /// `"..."`, raw text between the quotes (escapes not yet applied).
    Str { raw: String },
    /// `(a, b)` — output slot of a parametric function (stage 5).
    Tuple { items: Vec<Expr> },
    /// `[a, b]` / `[[..], [..]]` (stage 4).
    List { items: Vec<Expr> },
    /// `A_[i, j]` (stage 4).
    Index { base: Box<Expr>, indices: Vec<Expr> },
    /// `f.(2*x).coef` (stage 2). Kept so the full precedence table parses.
    Address { base: Box<Expr>, step: AddressStep },
    /// Placeholder after a reported error; never leaves the server.
    Error {},
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Term {
    /// Terms are addressable (`f.(-b*x)` includes the sign), so they carry an id.
    pub id: NodeId,
    pub sign: Sign,
    pub expr: Expr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Sign {
    Plus,
    Minus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum BigOpKind {
    Sum,
    Prod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum CmpOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CompareLink {
    pub op: CmpOp,
    pub rhs: Expr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum AddressStep {
    Term { pattern: Box<Expr> },
    Part { part: String },
}

/// A name with its notation parts (precedence 1: they are part of the name).
/// `key` is the canonical spelling used for lookup and in responses.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Ident {
    pub key: String,
    /// `x`, `\theta`, `넓이`.
    pub base: String,
    /// `bar` for `\bar{x}`.
    pub accent: Option<String>,
    /// Canonical text inside `_{...}`.
    pub sub: Option<String>,
    /// Text inside `^[...]`.
    pub sup: Option<String>,
    /// `f@n` (stage 5); 0 when absent.
    pub deriv: u32,
    /// `~f` (stage 5).
    pub inverse: bool,
}

impl Ident {
    pub fn plain(base: &str) -> Self {
        let mut id = Ident { key: String::new(), base: base.to_string(), accent: None, sub: None, sup: None, deriv: 0, inverse: false };
        id.rekey();
        id
    }

    pub fn rekey(&mut self) {
        let mut k = String::new();
        if self.inverse {
            k.push('~');
        }
        match &self.accent {
            Some(a) => k.push_str(&format!("\\{a}{{{}}}", self.base)),
            None => k.push_str(&self.base),
        }
        if let Some(s) = &self.sub {
            k.push_str(&format!("_{{{s}}}"));
        }
        if let Some(s) = &self.sup {
            k.push_str(&format!("^[{s}]"));
        }
        if self.deriv > 0 {
            k.push_str(&format!("@{}", self.deriv));
        }
        self.key = k;
    }
}

/// Filled by name resolution (design.md "이름은 서버가 종류까지 해석").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum NameKind {
    Unresolved,
    /// Input variable of the enclosing definition.
    Input,
    Constant,
    /// User-defined function.
    Function,
    /// Whitelisted builtin function.
    Builtin,
    /// `\pi`, `\e`, `\i`, `\infty`.
    Reserved,
    UserVar,
    /// Index of an enclosing `sum` / `prod`.
    Index,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum Ty {
    Unknown,
    Scalar,
    /// Inequality region (comparison / and / or / not).
    Region,
    Str,
    Vector { n: u32 },
    Matrix { m: u32, n: u32 },
    Tuple { n: u32 },
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Expr { id: 0, ty: Ty::Unknown, span, kind }
    }

    /// Pre-order numbering (parent first). Terms take an id before their expression.
    pub fn assign_ids(&mut self) {
        let mut next = 0;
        self.number(&mut next);
    }

    fn number(&mut self, next: &mut NodeId) {
        self.id = *next;
        *next += 1;
        match &mut self.kind {
            ExprKind::Num { .. } | ExprKind::Name { .. } | ExprKind::Str { .. } | ExprKind::Error {} => {}
            ExprKind::Sum { terms } => {
                for t in terms {
                    t.id = *next;
                    *next += 1;
                    t.expr.number(next);
                }
            }
            ExprKind::Product { factors: v } | ExprKind::And { args: v } | ExprKind::Or { args: v } | ExprKind::Tuple { items: v } | ExprKind::List { items: v } | ExprKind::Call { args: v, .. } => {
                v.iter_mut().for_each(|e| e.number(next))
            }
            ExprKind::Div { num: a, den: b } | ExprKind::Pow { base: a, exp: b } => {
                a.number(next);
                b.number(next);
            }
            ExprKind::Neg { arg } | ExprKind::Factorial { arg } | ExprKind::Not { arg } | ExprKind::Paren { inner: arg } => arg.number(next),
            ExprKind::BigOp { body, lo, hi, .. } => {
                body.number(next);
                lo.number(next);
                hi.number(next);
            }
            ExprKind::Compare { first, rest } => {
                first.number(next);
                rest.iter_mut().for_each(|l| l.rhs.number(next));
            }
            ExprKind::Index { base, indices } => {
                base.number(next);
                indices.iter_mut().for_each(|e| e.number(next));
            }
            ExprKind::Address { base, step } => {
                base.number(next);
                if let AddressStep::Term { pattern } = step {
                    pattern.number(next);
                }
            }
        }
    }

    /// Direct children in source order (Sum terms yield their expressions).
    pub fn children(&self) -> Vec<&Expr> {
        match &self.kind {
            ExprKind::Num { .. } | ExprKind::Name { .. } | ExprKind::Str { .. } | ExprKind::Error {} => vec![],
            ExprKind::Sum { terms } => terms.iter().map(|t| &t.expr).collect(),
            ExprKind::Product { factors: v } | ExprKind::And { args: v } | ExprKind::Or { args: v } | ExprKind::Tuple { items: v } | ExprKind::List { items: v } | ExprKind::Call { args: v, .. } => {
                v.iter().collect()
            }
            ExprKind::Div { num: a, den: b } | ExprKind::Pow { base: a, exp: b } => vec![a, b],
            ExprKind::Neg { arg } | ExprKind::Factorial { arg } | ExprKind::Not { arg } | ExprKind::Paren { inner: arg } => vec![arg],
            ExprKind::BigOp { body, lo, hi, .. } => vec![body, lo, hi],
            ExprKind::Compare { first, rest } => std::iter::once(&**first).chain(rest.iter().map(|l| &l.rhs)).collect(),
            ExprKind::Index { base, indices } => std::iter::once(&**base).chain(indices.iter()).collect(),
            ExprKind::Address { base, step } => match step {
                AddressStep::Term { pattern } => vec![base, pattern],
                AddressStep::Part { .. } => vec![base],
            },
        }
    }

    pub fn children_mut(&mut self) -> Vec<&mut Expr> {
        match &mut self.kind {
            ExprKind::Num { .. } | ExprKind::Name { .. } | ExprKind::Str { .. } | ExprKind::Error {} => vec![],
            ExprKind::Sum { terms } => terms.iter_mut().map(|t| &mut t.expr).collect(),
            ExprKind::Product { factors: v } | ExprKind::And { args: v } | ExprKind::Or { args: v } | ExprKind::Tuple { items: v } | ExprKind::List { items: v } | ExprKind::Call { args: v, .. } => {
                v.iter_mut().collect()
            }
            ExprKind::Div { num: a, den: b } | ExprKind::Pow { base: a, exp: b } => vec![a, b],
            ExprKind::Neg { arg } | ExprKind::Factorial { arg } | ExprKind::Not { arg } | ExprKind::Paren { inner: arg } => vec![arg],
            ExprKind::BigOp { body, lo, hi, .. } => vec![body, lo, hi],
            ExprKind::Compare { first, rest } => std::iter::once(&mut **first).chain(rest.iter_mut().map(|l| &mut l.rhs)).collect(),
            ExprKind::Index { base, indices } => std::iter::once(&mut **base).chain(indices.iter_mut()).collect(),
            ExprKind::Address { base, step } => match step {
                AddressStep::Term { pattern } => vec![base, pattern],
                AddressStep::Part { .. } => vec![base],
            },
        }
    }

    /// Read-only pre-order walk.
    pub fn walk(&self, f: &mut dyn FnMut(&Expr)) {
        f(self);
        for c in self.children() {
            c.walk(f);
        }
    }

    /// Spans point into one request's source; stored state must not keep them.
    pub fn clear_spans(&mut self) {
        self.span = Span::default();
        for c in self.children_mut() {
            c.clear_spans();
        }
    }

    /// Nesting depth (a leaf is 1).
    pub fn depth(&self) -> usize {
        1 + self.children().iter().map(|c| c.depth()).max().unwrap_or(0)
    }
}
