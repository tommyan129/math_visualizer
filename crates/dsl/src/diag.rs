//! Diagnostics: one line `line:col [E-code] message → suggestion`.
//! Codes are fixed strings; tests assert on codes, not message text.

use crate::span::{LineIndex, Span};

/// Error codes. Never rename one once it ships; tests and Claude rely on them.
pub mod code {
    // lexer
    pub const UNICODE: &str = "E-unicode";
    pub const CHAR: &str = "E-char";
    pub const TAB: &str = "E-tab";
    pub const INDENT: &str = "E-indent";
    pub const DEDENT: &str = "E-dedent";
    pub const NUMBER: &str = "E-number";
    pub const STRING: &str = "E-string";
    // parser
    pub const SYNTAX: &str = "E-syntax";
    pub const IMPLICIT_MUL: &str = "E-implicit-mul";
    pub const UNARY_PLUS: &str = "E-unary-plus";
    pub const POW_RHS: &str = "E-pow-rhs";
    pub const FUNC_POW: &str = "E-func-pow";
    pub const COMPARE_MIXED: &str = "E-compare-mixed";
    pub const COMPARE_EQ_CHAIN: &str = "E-compare-eq-chain";
    pub const SUBSCRIPT: &str = "E-subscript";
    pub const TEX: &str = "E-tex";
    pub const BIG_OP_RANGE: &str = "E-bigop-range";
    /// Resource limit (depth, size, fuel, time).
    pub const LIMIT: &str = "E-limit";
    // resolution
    pub const UNDECLARED: &str = "E-undeclared";
    pub const NOT_VALUE: &str = "E-not-value";
    pub const NOT_FUNCTION: &str = "E-not-function";
    pub const ARITY: &str = "E-arity";
    pub const TYPE: &str = "E-type";
    pub const CYCLE: &str = "E-cycle";
    pub const MISSING_CONST: &str = "E-missing-const";
    pub const NAME_CONFLICT: &str = "E-name-conflict";
    pub const DUPLICATE: &str = "E-duplicate";
    pub const ARROW: &str = "E-arrow";
    pub const AXES: &str = "E-axes";
    pub const DEL_CONSTANT: &str = "E-del-constant";
    // stage 2
    pub const HIGHLIGHT_NAME: &str = "E-highlight-name";
    pub const SLOTS: &str = "E-slots";
    pub const CONST_PROP: &str = "E-const-prop";
    pub const CONST_RANGE: &str = "E-const-range";
    pub const CONST_KIND: &str = "E-const-kind";
    pub const UNUSED_CONST: &str = "E-unused-const";
    pub const STYLE: &str = "E-style";
    pub const TARGET: &str = "E-target";
    pub const RANGE: &str = "E-range";
    pub const MEMO: &str = "E-memo";
    pub const ADDRESS_MISSING: &str = "E-address-missing";
    pub const ADDRESS_AMBIGUOUS: &str = "E-address-ambiguous";
    pub const ADDRESS_SIGN: &str = "E-address-sign";
    pub const ADDRESS_PART: &str = "E-address-part";
    // stage 3
    /// Scene block structure (missing space, step outside a scene, default scene by name).
    pub const SCENE: &str = "E-scene";
    pub const SCENE_FIELD: &str = "E-scene-field";
    /// A definition whose axes do not match the scene's coordinate system.
    pub const PLACEMENT: &str = "E-placement";
    /// `set` / `animate` values.
    pub const STEP: &str = "E-step";
    pub const NOTE: &str = "E-note";
    /// `view` shape, or view and clip that never overlap.
    pub const VIEW: &str = "E-view";
    /// Syntax of a later stage.
    pub const UNSUPPORTED: &str = "E-unsupported";
    // stage 4
    /// Matrix / vector sizes: ragged rows, operand sizes that do not match.
    pub const SHAPE: &str = "E-shape";
    /// An operator on matrices that must be written as a function (`A^2`, `A*B`).
    pub const MATRIX_OP: &str = "E-matrix-op";
    /// Element number outside the vector / matrix (`v_[3]` on a length-2 vector).
    pub const INDEX: &str = "E-index";
    // stage 5
    /// A split condition written with something other than the split expression and its boundary points.
    pub const SPLIT_COND: &str = "E-split-cond";
    /// Two split lines claim the same piece.
    pub const SPLIT_OVERLAP: &str = "E-split-overlap";
    /// Split boundary points out of order, repeated, or using an input variable.
    pub const SPLIT_AT: &str = "E-split-at";
    /// A needed derivative cannot be generated (no rule for a construct in the body).
    pub const DERIV: &str = "E-deriv";
    /// Feature-point slots: a filter word the kind does not have.
    pub const FEATURE: &str = "E-feature";
    /// `calc` slots: a kind word that does not exist.
    pub const CALC: &str = "E-calc";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: Span,
    pub code: &'static str,
    pub message: String,
    pub suggestion: Option<String>,
}

impl Diagnostic {
    pub fn new(span: Span, code: &'static str, message: impl Into<String>) -> Self {
        Self { span, code, message: message.into(), suggestion: None }
    }

    pub fn suggest(mut self, s: impl Into<String>) -> Self {
        self.suggestion = Some(s.into());
        self
    }

    pub fn render(&self, index: &LineIndex) -> String {
        let (line, col) = index.line_col(self.span.start);
        match &self.suggestion {
            Some(s) => format!("{line}:{col} [{}] {} → {s}", self.code, self.message),
            None => format!("{line}:{col} [{}] {}", self.code, self.message),
        }
    }
}
