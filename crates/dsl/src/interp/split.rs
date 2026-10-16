//! Stage 5 `split` bodies (design.md "구간별 함수 (split)"): name resolution, boundary
//! order, and the overlap check without evaluation.
//!
//! Boundary points b₁ < … < bₘ cut the subject's values into 2m + 1 pieces
//! `(-∞, b₁)`, `{b₁}`, `(b₁, b₂)`, … numbered 0..=2m: open intervals even, points odd.
//! On those ordinals (subject = piece number p, bⱼ = 2j - 1) every condition written with
//! the subject and the boundary points has one truth value per piece, so the pieces each
//! line claims are known from the structure alone.

use super::*;
use crate::print::expr_dsl;

/// Most boundary points in one split.
const MAX_BOUNDARIES: usize = 32;

#[derive(Clone, Copy)]
enum Pos {
    Subject,
    Bound(usize),
}

/// Number literal (`2`, `-1.5`, `(3)`), for the order check of numeric boundaries.
fn literal(e: &Expr) -> Option<f64> {
    match &e.kind {
        ExprKind::Num { text, .. } => text.parse().ok(),
        ExprKind::Neg { arg } => literal(arg).map(|v| -v),
        ExprKind::Paren { inner } => literal(inner),
        _ => None,
    }
}

fn strip_parens(e: &Expr) -> &Expr {
    match &e.kind {
        ExprKind::Paren { inner } => strip_parens(inner),
        _ => e,
    }
}

fn uses_kind(e: &Expr, pred: &dyn Fn(NameKind) -> bool) -> bool {
    let mut found = false;
    e.walk(&mut |n| {
        if let ExprKind::Name { kind, .. } = &n.kind {
            found |= pred(*kind);
        }
    });
    found
}

impl<'s> Cx<'s> {
    pub(super) fn resolve_split(&mut self, body: &mut Expr, sc: &mut Scope, info: &mut DefInfo) -> Ty {
        let span = body.span;
        let ExprKind::Split { name, subject, at, pieces } = &mut body.kind else { unreachable!() };
        let mut ok = true;

        let st = self.resolve(subject, sc, info);
        self.expect_scalar(&st, subject.span);
        ok &= st == Ty::Scalar;

        if let Some(n) = name.as_ref() {
            ok &= self.check_split_name(n, span, sc);
        }

        // Boundary points: constant expressions in the written (ascending) order.
        if at.len() > MAX_BOUNDARIES {
            self.err(at[MAX_BOUNDARIES].span, code::LIMIT, format!("경계점이 너무 많음 ({}개, 상한 {MAX_BOUNDARIES})", at.len()));
            return Ty::Unknown;
        }
        for b in at.iter_mut() {
            let t = self.resolve(b, sc, info);
            self.expect_scalar(&t, b.span);
            ok &= t == Ty::Scalar;
            if uses_kind(b, &|k| k == NameKind::Input) {
                self.err_s(b.span, code::SPLIT_AT, "경계점에는 입력 변수를 쓸 수 없음 (숫자와 상수만)", "경계가 식이면 split 식으로 옮김: split x - a at 0");
                ok = false;
            }
        }
        let texts: Vec<String> = at.iter().map(|b| expr_dsl(strip_parens(b))).collect();
        for i in 1..at.len() {
            if let Some(j) = texts[..i].iter().position(|t| *t == texts[i]) {
                self.err_s(at[i].span, code::SPLIT_AT, format!("경계점 {}를 두 번 적음 ({}번째와 {}번째)", texts[i], j + 1, i + 1), "같은 점이 필요하면 경계점 하나짜리 함수를 따로 정의");
                ok = false;
                continue;
            }
            let (a, b) = (&at[i - 1], &at[i]);
            match (literal(a), literal(b)) {
                (Some(x), Some(y)) if x >= y => {
                    self.err_s(a.span.to(b.span), code::SPLIT_AT, format!("경계점은 오름차순으로 씀 ({} 다음에 {})", texts[i - 1], texts[i]), format!("at {}", ascending(&texts, at)));
                    ok = false;
                }
                _ => {
                    let has_const = |e: &Expr| uses_kind(e, &|k| k == NameKind::Constant);
                    if has_const(a) || has_const(b) {
                        let (mut lo, mut hi) = (a.clone(), b.clone());
                        lo.assign_ids();
                        lo.clear_spans();
                        hi.assign_ids();
                        hi.clear_spans();
                        info.constraints.push(OrderConstraint { src: format!("{} < {}", texts[i - 1], texts[i]), lo, hi });
                    }
                }
            }
        }

        // Pieces: the name stands for the subject in values and conditions.
        sc.split_name = name.as_ref().map(|n| n.key.clone());
        let subject_text = expr_dsl(strip_parens(subject));
        let shown = name.as_ref().map_or(subject_text.clone(), |n| n.key.clone());
        let atoms = 2 * at.len() + 1;
        let mut owner: Vec<Option<usize>> = vec![None; atoms];
        for (li, p) in pieces.iter_mut().enumerate() {
            let vt = self.resolve(&mut p.value, sc, info);
            match vt {
                Ty::Scalar | Ty::Unknown => {}
                t => self.err(p.value.span, code::TYPE, format!("split 줄의 값은 숫자여야 함 (이 식은 {})", t.describe())),
            }
            let ct = self.resolve(&mut p.cond, sc, info);
            if !matches!(ct, Ty::Region | Ty::Unknown) {
                self.err_s(p.cond.span, code::SPLIT_COND, "split 줄의 두 번째 칸은 구간(부등식)이어야 함", format!("{}, {shown} < {}", expr_dsl(&p.value), texts[0]));
                ok = false;
                continue;
            }
            if ct == Ty::Unknown || !ok {
                ok = false;
                continue;
            }
            let Some(claim) = self.claim(&p.cond, &subject_text, name.as_ref().map(|n| n.key.as_str()), &texts, &shown) else {
                ok = false;
                continue;
            };
            let mut clash: Vec<(usize, usize)> = vec![];
            for (a, c) in claim.iter().enumerate() {
                if !c {
                    continue;
                }
                match owner[a] {
                    Some(other) => clash.push((a, other)),
                    None => owner[a] = Some(li),
                }
            }
            if !clash.is_empty() {
                let lines: BTreeSet<usize> = clash.iter().map(|(_, o)| o + 1).collect();
                let lines = lines.iter().map(|l| format!("{l}번째")).collect::<Vec<_>>().join(", ");
                let parts = clash.iter().map(|(a, _)| piece_label(*a, &shown, &texts)).collect::<Vec<_>>().join(", ");
                self.err_s(p.cond.span, code::SPLIT_OVERLAP, format!("{}번째 줄의 구간이 {lines} 줄과 겹침 ({parts})", li + 1), "한 조각은 한 줄만 차지하도록 < 와 <= 를 나눔");
                ok = false;
            }
        }
        sc.split_name = None;
        let ty = if ok { Ty::Scalar } else { Ty::Unknown };
        body.ty = ty.clone();
        ty
    }

    /// `r` in `split r = ... at`: same naming rules as a constant, never a slider.
    fn check_split_name(&mut self, n: &Ident, span: Span, sc: &Scope) -> bool {
        let k = n.key.as_str();
        let conflict = if sc.inputs.iter().any(|i| i == k) || sc.outputs.iter().any(|o| o == k) {
            Some("축 이름")
        } else if sc.consts.iter().any(|c| c == k) || self.state.constants.contains_key(k) {
            Some("상수 이름")
        } else if k == sc.def_key || self.is_function_name(k) {
            Some("정의 이름")
        } else if names::is_builtin(k) {
            Some("기본 함수 이름")
        } else if is_reserved(n) || n.deriv > 0 || n.inverse {
            Some("예약어")
        } else {
            self.state.kind_of(k).map(|_| "다른 이름")
        };
        if let Some(what) = conflict {
            self.err(span, code::NAME_CONFLICT, format!("split 식 이름 '{k}'는 이미 {what}임"));
            return false;
        }
        true
    }

    /// Pieces a condition claims, or None after reporting an operand that is neither the
    /// subject nor a boundary point.
    fn claim(&mut self, cond: &Expr, subject: &str, name: Option<&str>, bounds: &[String], shown: &str) -> Option<Vec<bool>> {
        let atoms = 2 * bounds.len() + 1;
        match &cond.kind {
            ExprKind::Paren { inner } => self.claim(inner, subject, name, bounds, shown),
            ExprKind::Not { arg } => Some(self.claim(arg, subject, name, bounds, shown)?.into_iter().map(|b| !b).collect()),
            ExprKind::And { args } | ExprKind::Or { args } => {
                let and = matches!(cond.kind, ExprKind::And { .. });
                let mut acc = vec![and; atoms];
                let mut ok = true;
                for a in args {
                    match self.claim(a, subject, name, bounds, shown) {
                        Some(v) => acc.iter_mut().zip(v).for_each(|(x, y)| *x = if and { *x && y } else { *x || y }),
                        None => ok = false,
                    }
                }
                ok.then_some(acc)
            }
            ExprKind::Compare { first, rest } => {
                let mut prev = self.position(first, subject, name, bounds, shown)?;
                let mut acc = vec![true; atoms];
                for l in rest {
                    let next = self.position(&l.rhs, subject, name, bounds, shown)?;
                    for (p, x) in acc.iter_mut().enumerate() {
                        let ord = |q: Pos| match q {
                            Pos::Subject => p,
                            Pos::Bound(j) => 2 * j + 1,
                        };
                        let (a, b) = (ord(prev), ord(next));
                        *x &= match l.op {
                            CmpOp::Lt => a < b,
                            CmpOp::Le => a <= b,
                            CmpOp::Gt => a > b,
                            CmpOp::Ge => a >= b,
                            CmpOp::Eq => a == b,
                        };
                    }
                    prev = next;
                }
                Some(acc)
            }
            _ => {
                self.err_s(cond.span, code::SPLIT_COND, "split 줄의 구간은 부등식으로 씀", format!("{shown} < {}", bounds[0]));
                None
            }
        }
    }

    fn position(&mut self, e: &Expr, subject: &str, name: Option<&str>, bounds: &[String], shown: &str) -> Option<Pos> {
        let e = strip_parens(e);
        if let ExprKind::Name { kind: NameKind::SplitName, .. } = e.kind {
            return Some(Pos::Subject);
        }
        let text = expr_dsl(e);
        if text == subject {
            return Some(Pos::Subject);
        }
        if let Some(j) = bounds.iter().position(|b| *b == text) {
            return Some(Pos::Bound(j));
        }
        let what = match name {
            Some(n) => format!("split 식 이름 {n}"),
            None => format!("split 식 {subject}"),
        };
        self.err_s(
            e.span,
            code::SPLIT_COND,
            format!("split 구간에는 {what}과 경계점({})만 씀 ('{text}'는 둘 다 아님)", bounds.join(", ")),
            format!("{shown} < {}", bounds[0]),
        );
        None
    }
}

/// `x < 0`, `x == 0`, `0 < x < 1`, `x > 1` for piece number `a`.
fn piece_label(a: usize, s: &str, b: &[String]) -> String {
    let j = a / 2;
    if a % 2 == 1 {
        format!("{s} == {}", b[j])
    } else if j == 0 {
        format!("{s} < {}", b[0])
    } else if j == b.len() {
        format!("{s} > {}", b[j - 1])
    } else {
        format!("{} < {s} < {}", b[j - 1], b[j])
    }
}

/// Boundary list sorted when all are number literals (suggestion only).
fn ascending(texts: &[String], at: &[Expr]) -> String {
    let mut v: Vec<(f64, &String)> = at.iter().zip(texts).filter_map(|(e, t)| literal(e).map(|x| (x, t))).collect();
    if v.len() != texts.len() {
        return texts.join(", ");
    }
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v.into_iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(", ")
}
