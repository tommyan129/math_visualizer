//! AST → DSL text, for responses (addresses, term lists, current expressions).
//! Parentheses are added only where precedence needs them; a Paren node prints as written.

use crate::ast::*;

/// Binding strength, loosest first (design.md "연산자 우선순위", reversed).
fn prec(e: &Expr) -> u8 {
    match &e.kind {
        ExprKind::Or { .. } => 1,
        ExprKind::And { .. } => 2,
        ExprKind::Not { .. } => 3,
        ExprKind::Compare { .. } => 4,
        ExprKind::Sum { .. } => 5,
        ExprKind::Product { .. } | ExprKind::Div { .. } => 6,
        ExprKind::Neg { .. } => 7,
        ExprKind::Pow { .. } => 8,
        ExprKind::Factorial { .. } => 9,
        _ => 10,
    }
}

fn wrap(e: &Expr, min: u8) -> String {
    let s = expr_dsl(e);
    if prec(e) < min { format!("({s})") } else { s }
}

fn cmp(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
        CmpOp::Eq => "==",
    }
}

pub fn expr_dsl(e: &Expr) -> String {
    stacker::maybe_grow(crate::parser::STACK_RED_ZONE, crate::parser::STACK_GROW, || print(e))
}

fn print(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Num { text, .. } => text.clone(),
        ExprKind::Name { name, .. } => name.key.clone(),
        ExprKind::Str { raw } => format!("\"{raw}\""),
        ExprKind::Error {} => "?".into(),
        ExprKind::Sum { terms } => {
            let mut out = String::new();
            for (i, t) in terms.iter().enumerate() {
                let s = if i > 0 && matches!(t.expr.kind, ExprKind::Neg { .. }) { format!("({})", expr_dsl(&t.expr)) } else { wrap(&t.expr, 6) };
                match (i, t.sign) {
                    (0, Sign::Plus) => out.push_str(&s),
                    (0, Sign::Minus) => out.push_str(&format!("-{s}")),
                    (_, Sign::Plus) => out.push_str(&format!(" + {s}")),
                    (_, Sign::Minus) => out.push_str(&format!(" - {s}")),
                }
            }
            out
        }
        ExprKind::Product { factors } => factors
            .iter()
            .enumerate()
            .map(|(i, f)| if i > 0 && matches!(f.kind, ExprKind::Neg { .. }) { format!("({})", expr_dsl(f)) } else { wrap(f, 6) })
            .collect::<Vec<_>>()
            .join("*"),
        ExprKind::Div { num, den } => format!("{}/{}", wrap(num, 6), wrap(den, 7)),
        ExprKind::Pow { base, exp } => {
            let b = wrap(base, 9);
            let x = match exp.kind {
                ExprKind::Num { .. } | ExprKind::Name { .. } | ExprKind::Paren { .. } => expr_dsl(exp),
                ExprKind::Pow { .. } => expr_dsl(exp),
                _ => format!("({})", expr_dsl(exp)),
            };
            format!("{b}^{x}")
        }
        ExprKind::Neg { arg } => format!("-{}", wrap(arg, 8)),
        ExprKind::Factorial { arg } => format!("{}!", wrap(arg, 10)),
        ExprKind::Paren { inner } => format!("({})", expr_dsl(inner)),
        ExprKind::Call { callee, args, .. } if callee.key == "arrow" => format!("arrow({})", args.iter().map(expr_dsl).collect::<Vec<_>>().join("; ")),
        ExprKind::Call { callee, args, .. } => format!("{}({})", callee.key, args.iter().map(expr_dsl).collect::<Vec<_>>().join(", ")),
        ExprKind::BigOp { op, body, index, lo, lo_strict, hi, hi_strict } => {
            let head = match op {
                BigOpKind::Sum => "sum",
                BigOpKind::Prod => "prod",
            };
            let l = if *lo_strict { "<" } else { "<=" };
            let h = if *hi_strict { "<" } else { "<=" };
            format!("{head}({}; {} {l} {} {h} {})", expr_dsl(body), wrap(lo, 5), index.key, wrap(hi, 5))
        }
        ExprKind::Compare { first, rest } => {
            let mut s = wrap(first, 5);
            for l in rest {
                s.push_str(&format!(" {} {}", cmp(l.op), wrap(&l.rhs, 5)));
            }
            s
        }
        ExprKind::Not { arg } => format!("not {}", wrap(arg, 3)),
        ExprKind::And { args } => args.iter().map(|a| wrap(a, 3)).collect::<Vec<_>>().join(" and "),
        ExprKind::Or { args } => args.iter().map(|a| wrap(a, 2)).collect::<Vec<_>>().join(" or "),
        ExprKind::Tuple { items } => format!("({})", items.iter().map(expr_dsl).collect::<Vec<_>>().join(", ")),
        ExprKind::List { items } => format!("[{}]", items.iter().map(expr_dsl).collect::<Vec<_>>().join(", ")),
        ExprKind::Index { base, indices } => format!("{}_[{}]", wrap(base, 10), indices.iter().map(expr_dsl).collect::<Vec<_>>().join(", ")),
        ExprKind::Address { base, step } => match step {
            AddressStep::Term { pattern } => format!("{}.({})", wrap(base, 10), expr_dsl(pattern)),
            AddressStep::Part { part } => format!("{}.{part}", wrap(base, 10)),
        },
        ExprKind::Feature { callee, args, range, filter } => {
            // Empty slots print as `;;` (`extrema(f;; max)`).
            let mut s = args.iter().map(feature_arg).collect::<Vec<_>>().join(", ");
            match (range, filter) {
                (Some(r), Some(f)) => s.push_str(&format!("; {}; {f}", expr_dsl(r))),
                (Some(r), None) => s.push_str(&format!("; {}", expr_dsl(r))),
                (None, Some(f)) => s.push_str(&format!(";; {f}")),
                (None, None) => {}
            }
            format!("{}({s})", callee.key)
        }
        // One line per piece, unindented (`Function::source` indents).
        ExprKind::Split { name, subject, at, pieces } => {
            let name = name.as_ref().map_or(String::new(), |n| format!("{} = ", n.key));
            let at = at.iter().map(expr_dsl).collect::<Vec<_>>().join(", ");
            let mut s = format!("split {name}{} at {at}", expr_dsl(subject));
            for p in pieces {
                s.push_str(&format!("\n{}, {}", expr_dsl(&p.value), expr_dsl(&p.cond)));
            }
            s
        }
    }
}

/// A feature target: a name, or `y = x` (stored as an `==` comparison).
fn feature_arg(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Compare { first, rest } if rest.len() == 1 && rest[0].op == CmpOp::Eq => format!("{} = {}", expr_dsl(first), expr_dsl(&rest[0].rhs)),
        _ => expr_dsl(e),
    }
}

/// A Sum term with its sign, as written in an address: `-b*x`.
pub fn term_dsl(sign: Sign, e: &Expr) -> String {
    let s = if matches!(e.kind, ExprKind::Sum { .. }) { format!("({})", expr_dsl(e)) } else { expr_dsl(e) };
    match sign {
        Sign::Plus => s,
        Sign::Minus => format!("-{s}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_expr;

    fn round(src: &str) -> String {
        let (e, d) = parse_expr(src);
        assert!(d.is_empty(), "{d:?}");
        expr_dsl(&e.unwrap())
    }

    #[test]
    fn prints_back_as_written() {
        for s in [
            "a*x^2 + b*x + 1",
            "-x^2 + 1",
            "x/(x - 1)",
            "(a + b)*c",
            "a/b*c",
            "sin(3*x)^2",
            "x^(n + 1)",
            "2^3^2",
            "-(a*b)",
            "sum(x^k/k!; 1 <= k <= n)",
            "0.9 < x < 1.1",
            "f(x) < y < g(x) or g(x) < y < f(x)",
            "not x == 1 and y > 0",
            "\"메모\"",
            "f.(b*x).coef",
            "a - (-b)",
            "[[1, 2], [3, 4]]",
            "matmul(R(t), [1, 0])_[2]",
            "A_[1, 2] + 2*transpose(v)_[1, 1]",
            "arrow(v; [1, f(c)])",
            "extrema(f)",
            "roots(f; -1 < x < 1)",
            "extrema(f;; max)",
            "breaks(f; x > 0; hole)",
            "intersect(f, y = 2*x)",
        ] {
            assert_eq!(round(s), s);
        }
    }

    #[test]
    fn braces_print_as_parentheses_where_needed() {
        assert_eq!(round("{a + b}*c"), "(a + b)*c");
        assert_eq!(round("x^{1/3}"), "x^(1/3)");
    }
}
