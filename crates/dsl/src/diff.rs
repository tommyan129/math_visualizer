//! Changed parts between neighbouring calculation steps (design.md "계산 과정의 각 단계",
//! stage 5c): a structural AST comparison, no evaluation. Sum terms and Product factors
//! are matched as lists (longest common subsequence), so a rewritten term colors only
//! itself; other nodes with the same head compare child by child.

use crate::ast::{Expr, ExprKind};
use crate::print::expr_dsl;
use crate::state::StepDiff;

/// Most list items compared by LCS (resource bound; longer lists count as replaced).
const MAX_ITEMS: usize = 200;

/// Node ids that differ between `before` and `after` (ids as stored in each body).
pub fn diff(before: &Expr, after: &Expr) -> StepDiff {
    let mut d = StepDiff::default();
    walk(before, after, &mut d);
    d
}

fn walk(a: &Expr, b: &Expr, d: &mut StepDiff) {
    if expr_dsl(a) == expr_dsl(b) {
        return;
    }
    use ExprKind as K;
    match (&a.kind, &b.kind) {
        (K::Sum { terms: ta }, K::Sum { terms: tb }) => {
            let ka: Vec<String> = ta.iter().map(|t| format!("{:?} {}", t.sign, expr_dsl(&t.expr))).collect();
            let kb: Vec<String> = tb.iter().map(|t| format!("{:?} {}", t.sign, expr_dsl(&t.expr))).collect();
            lists(&ka, &kb, d, |i, j, d| {
                // A term rewritten in place (same sign): only its inside changed.
                if ta[i].sign == tb[j].sign {
                    walk(&ta[i].expr, &tb[j].expr, d)
                } else {
                    d.before.push(ta[i].id);
                    d.after.push(tb[j].id);
                }
            }, |i, d| d.before.push(ta[i].id), |j, d| d.after.push(tb[j].id));
        }
        (K::Product { factors: fa }, K::Product { factors: fb }) => {
            let ka: Vec<String> = fa.iter().map(expr_dsl).collect();
            let kb: Vec<String> = fb.iter().map(expr_dsl).collect();
            lists(&ka, &kb, d, |i, j, d| walk(&fa[i], &fb[j], d), |i, d| d.before.push(fa[i].id), |j, d| d.after.push(fb[j].id));
        }
        _ if same_head(a, b) => {
            let (ca, cb) = (a.children(), b.children());
            for (x, y) in ca.iter().zip(&cb) {
                walk(x, y, d);
            }
        }
        _ => whole(a, b, d),
    }
}

fn whole(a: &Expr, b: &Expr, d: &mut StepDiff) {
    d.before.push(a.id);
    d.after.push(b.id);
}

/// Same node kind, same non-child fields and the same number of children: compare the
/// children pairwise.
fn same_head(a: &Expr, b: &Expr) -> bool {
    use ExprKind as K;
    let heads = match (&a.kind, &b.kind) {
        (K::Div { .. }, K::Div { .. })
        | (K::Pow { .. }, K::Pow { .. })
        | (K::Neg { .. }, K::Neg { .. })
        | (K::Factorial { .. }, K::Factorial { .. })
        | (K::Paren { .. }, K::Paren { .. })
        | (K::Not { .. }, K::Not { .. })
        | (K::And { .. }, K::And { .. })
        | (K::Or { .. }, K::Or { .. })
        | (K::Tuple { .. }, K::Tuple { .. })
        | (K::List { .. }, K::List { .. })
        | (K::Index { .. }, K::Index { .. }) => true,
        (K::Call { callee: x, .. }, K::Call { callee: y, .. }) => x == y,
        (K::BigOp { op: x, index: i, lo_strict: ls, hi_strict: hs, .. }, K::BigOp { op: y, index: j, lo_strict: ls2, hi_strict: hs2, .. }) => x == y && i == j && ls == ls2 && hs == hs2,
        (K::Compare { rest: x, .. }, K::Compare { rest: y, .. }) => x.iter().map(|l| l.op).eq(y.iter().map(|l| l.op)),
        (K::Split { name: x, at: ax, pieces: px, .. }, K::Split { name: y, at: ay, pieces: py, .. }) => x == y && ax.len() == ay.len() && px.len() == py.len(),
        _ => false,
    };
    heads && a.children().len() == b.children().len()
}

/// Matches two lists by their keys (LCS). Inside each gap between matched items, equal
/// counts pair up in order (`pair`: an item rewritten in place); otherwise the gap's items
/// are removed (`gone`) and added (`new`).
fn lists(
    ka: &[String],
    kb: &[String],
    d: &mut StepDiff,
    mut pair: impl FnMut(usize, usize, &mut StepDiff),
    mut gone: impl FnMut(usize, &mut StepDiff),
    mut new: impl FnMut(usize, &mut StepDiff),
) {
    let (n, m) = (ka.len(), kb.len());
    let anchors: Vec<(usize, usize)> = if n > MAX_ITEMS || m > MAX_ITEMS { vec![] } else { lcs(ka, kb) };
    let (mut i0, mut j0) = (0, 0);
    for (i1, j1) in anchors.into_iter().chain(std::iter::once((n, m))) {
        let (gi, gj) = (i1 - i0, j1 - j0);
        if gi == gj {
            for k in 0..gi {
                pair(i0 + k, j0 + k, d);
            }
        } else {
            (i0..i1).for_each(|i| gone(i, d));
            (j0..j1).for_each(|j| new(j, d));
        }
        (i0, j0) = (i1 + 1, j1 + 1);
    }
}

/// Index pairs of a longest common subsequence, in order.
fn lcs(a: &[String], b: &[String]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let mut t = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i][j] = if a[i] == b[j] { t[i + 1][j + 1] + 1 } else { t[i + 1][j].max(t[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, vec![]);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if t[i + 1][j] >= t[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::NodeId;
    use crate::parser::parse_expr;

    /// Every id a diff colors is a node of its body (tests).
    fn ids(e: &Expr) -> Vec<NodeId> {
        let mut v = vec![];
        collect(e, &mut v);
        v
    }
    
    fn collect(e: &Expr, v: &mut Vec<NodeId>) {
        v.push(e.id);
        if let ExprKind::Sum { terms } = &e.kind {
            v.extend(terms.iter().map(|t| t.id));
        }
        for c in e.children() {
            collect(c, v);
        }
    }

    fn p(src: &str) -> Expr {
        let (e, d) = parse_expr(src);
        assert!(d.is_empty(), "{src}: {d:?}");
        let mut e = e.unwrap();
        e.assign_ids();
        e
    }

    /// The changed parts in DSL notation: (before, after).
    fn changed(a: &str, b: &str) -> (Vec<String>, Vec<String>) {
        let (ea, eb) = (p(a), p(b));
        let d = diff(&ea, &eb);
        assert!(d.before.iter().all(|i| ids(&ea).contains(i)) && d.after.iter().all(|i| ids(&eb).contains(i)));
        (texts(&ea, &d.before), texts(&eb, &d.after))
    }

    fn texts(e: &Expr, want: &[NodeId]) -> Vec<String> {
        let mut out = vec![];
        find(e, want, &mut out);
        out
    }

    fn find(e: &Expr, want: &[NodeId], out: &mut Vec<String>) {
        if want.contains(&e.id) {
            out.push(expr_dsl(e));
        }
        if let ExprKind::Sum { terms } = &e.kind {
            for t in terms.iter().filter(|t| want.contains(&t.id)) {
                out.push(format!("{}{}", if t.sign == crate::ast::Sign::Minus { "-" } else { "+" }, expr_dsl(&t.expr)));
            }
        }
        for c in e.children() {
            find(c, want, out);
        }
    }

    #[test]
    fn same_expression_has_no_diff() {
        assert_eq!(changed("x^2 + 2*x + 1", "x^2 + 2*x + 1"), (vec![], vec![]));
    }

    #[test]
    fn expanded_terms_are_colored_alone() {
        // (x+1)^2 - (x-1)^2 → (x^2+2x+1) - (x^2-2x+1): each square becomes a parenthesized sum.
        let (b, a) = changed("(x+1)^2 - (x-1)^2", "(x^2 + 2*x + 1) - (x^2 - 2*x + 1)");
        assert_eq!(b, vec!["(x + 1)^2", "(x - 1)^2"]);
        assert_eq!(a, vec!["(x^2 + 2*x + 1)", "(x^2 - 2*x + 1)"]);
    }

    #[test]
    fn unchanged_terms_stay_plain() {
        let (b, a) = changed("x^2 + 2*x + 1 + 3", "x^2 + 2*x + 4");
        assert_eq!(b, vec!["+1", "+3"]);
        assert_eq!(a, vec!["+4"]);
    }

    #[test]
    fn rewritten_term_compares_inside() {
        // Same position, same sign: only the exponent changed.
        let (b, a) = changed("3*x^2 + 1", "3*x^3 + 1");
        assert_eq!(b, vec!["2"]);
        assert_eq!(a, vec!["3"]);
    }

    #[test]
    fn sign_change_colors_the_term() {
        let (b, a) = changed("x + y", "x - y");
        assert_eq!(b, vec!["+y"]);
        assert_eq!(a, vec!["-y"]);
    }

    #[test]
    fn different_heads_replace_the_node() {
        let (b, a) = changed("sin(x)^2 + cos(x)^2", "1");
        assert_eq!(b, vec!["sin(x)^2 + cos(x)^2"]);
        assert_eq!(a, vec!["1"]);
        let (b, a) = changed("2*sin(x)*cos(x)", "sin(2*x)");
        assert_eq!((b.len(), a.len()), (1, 1));
    }

    #[test]
    fn factors_match_as_a_list() {
        let (b, a) = changed("2*a*x", "2*x");
        assert_eq!(b, vec!["a"]);
        assert!(a.is_empty());
    }
}
