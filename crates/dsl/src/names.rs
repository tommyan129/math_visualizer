//! Whitelists for names (stage-1 decisions; start short and grow).

/// Greek letters accepted as `\name`.
pub const GREEK: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "kappa", "lambda", "mu", "nu", "xi", "rho", "sigma", "tau",
    "phi", "chi", "psi", "omega", "Gamma", "Delta", "Theta", "Lambda", "Sigma", "Phi", "Psi", "Omega",
];

/// `\bar{x}` and friends: a distinct name from the plain letter.
pub const ACCENTS: &[&str] = &["bar", "hat", "vec", "tilde", "dot"];

/// LaTeX-form reserved values.
pub const RESERVED: &[&str] = &["pi", "e", "i", "infty"];

/// Builtin functions and their accepted argument counts.
pub const BUILTINS: &[(&str, &[usize])] = &[
    ("sin", &[1]),
    ("cos", &[1]),
    ("tan", &[1]),
    ("asin", &[1]),
    ("acos", &[1]),
    ("atan", &[1]),
    ("sinh", &[1]),
    ("cosh", &[1]),
    ("tanh", &[1]),
    ("exp", &[1]),
    ("ln", &[1]),
    ("log10", &[1]),
    ("log2", &[1]),
    ("log", &[2]),
    ("sqrt", &[1]),
    ("abs", &[1]),
    ("sign", &[1]),
    ("floor", &[1]),
    ("ceil", &[1]),
    ("round", &[1]),
    ("mod", &[2]),
    ("deg", &[1]),
    ("gamma", &[1]),
];

/// Linear-algebra builtins (stage 4), typed by shape. `span` / `arrow` are only the
/// body of a definition (`S: span(v, w)`, `p: arrow(v; P)`).
pub const LINALG: &[(&str, &[usize])] = &[
    ("matmul", &[2]),
    ("pow", &[2]),
    ("inv", &[1]),
    ("transpose", &[1]),
    ("det", &[1]),
    ("tr", &[1]),
    ("span", &[]),
    ("arrow", &[1, 2]),
];

/// Largest number of rows or columns in a matrix literal (resource bound).
pub const MAX_DIM: u32 = 32;

pub fn is_linalg(name: &str) -> bool {
    LINALG.iter().any(|(n, _)| *n == name)
}

/// Handled by the parser as `sum(body; range)` / `prod(body; range)`.
pub const BIG_OPS: &[&str] = &["sum", "prod"];

pub fn builtin_arity(name: &str) -> Option<&'static [usize]> {
    BUILTINS.iter().find(|(n, _)| *n == name).map(|(_, a)| *a)
}

pub fn is_builtin(name: &str) -> bool {
    builtin_arity(name).is_some() || BIG_OPS.contains(&name) || is_linalg(name)
}

/// Replacement for a LaTeX command that is not a name (structure commands, operators).
pub fn tex_command_suggestion(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "frac" => "a / b",
        "sqrt" => "sqrt(x)",
        "cdot" | "times" => "*",
        "div" => "/",
        "le" | "leq" => "<=",
        "ge" | "geq" => ">=",
        "neq" | "ne" => "not (a == b)",
        "left" | "right" => "( )",
        "sum" => "sum(식; 1 <= k <= n)",
        "prod" => "prod(식; 1 <= k <= n)",
        "sin" => "sin(x)",
        "cos" => "cos(x)",
        "tan" => "tan(x)",
        "ln" => "ln(x)",
        "log" => "log(x, b)",
        "exp" => "exp(x)",
        "infin" | "inf" => "\\infty",
        "varepsilon" => "\\epsilon",
        "vartheta" => "\\theta",
        "varphi" => "\\phi",
        _ => return None,
    })
}
