//! Symbolic differentiation as an AST transformation (design.md "계산 방식": the one
//! symbolic exception). Rules only; no expression value is ever computed. Integer
//! literals in coefficients and exponents are combined (`3*x^2` → `6*x`), which is
//! bookkeeping of the rule, not evaluation of the user's expression.
//!
//! Calls differentiate by the chain rule into named derivatives: `g(u)` → `g@1(u)*u'`,
//! `g@1(u)` → `g@2(u)*u'`, `~g(u)` → `u'/g@1(~g(u))`. Which of those Claude defined and
//! which the server fills is decided by the caller (`interp::deriv`).

use crate::ast::*;
use crate::span::Span;

/// Highest derivative order a name or a generated derivative may have.
pub const MAX_ORDER: u32 = 8;

/// Largest generated derivative (nodes), so repeated product rules cannot blow up.
pub const MAX_NODES: usize = 20_000;

/// Why a body has no derivative by the rules: the construct, as written in DSL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivError {
    pub what: String,
}

fn fail<T>(what: impl Into<String>) -> Result<T, DerivError> {
    Err(DerivError { what: what.into() })
}

/// d/d`input` of `e`. A result of structural zero is the literal `0` (for a vector /
/// matrix body: a list of zeros of the same size).
pub fn derive(e: &Expr, input: &str) -> Result<Expr, DerivError> {
    let d = Deriv { input, subject: None };
    let out = stacker::maybe_grow(crate::parser::STACK_RED_ZONE, crate::parser::STACK_GROW, || d.go(e))?;
    let mut out = out.unwrap_or_else(|| zero_like(&e.ty));
    out.assign_ids();
    if node_count(&out) > MAX_NODES {
        return fail(format!("미분 결과가 너무 큼 (노드 {MAX_NODES}개 초과)"));
    }
    Ok(out)
}

pub fn node_count(e: &Expr) -> usize {
    let mut n = 0;
    e.walk(&mut |_| n += 1);
    n
}

/// Name of the next derivative of a called function: `g` → `g@1`, `g@1` → `g@2`.
pub fn next_deriv(callee: &Ident) -> Ident {
    let mut id = callee.clone();
    id.inverse = false;
    id.deriv += 1;
    id.rekey();
    id
}

/// `f` of `f@2` / `~f`.
pub fn base_ident(id: &Ident) -> Ident {
    let mut b = id.clone();
    b.deriv = 0;
    b.inverse = false;
    b.rekey();
    b
}

struct Deriv<'a> {
    input: &'a str,
    /// Derivative of the enclosing split's subject (its name stands for the subject).
    subject: Option<Option<Expr>>,
}

fn mk(kind: ExprKind, ty: Ty) -> Expr {
    Expr { id: 0, ty, span: Span::default(), kind }
}

fn int(n: i64) -> Expr {
    let lit = mk(ExprKind::Num { text: n.unsigned_abs().to_string(), int: true }, Ty::Scalar);
    if n < 0 { mk(ExprKind::Neg { arg: Box::new(lit) }, Ty::Scalar) } else { lit }
}

fn name(key: &str) -> Ident {
    Ident::plain(key)
}

fn reserved(key: &str) -> Expr {
    mk(ExprKind::Name { name: name(key), kind: NameKind::Reserved }, Ty::Scalar)
}

fn builtin(key: &str, args: Vec<Expr>) -> Expr {
    mk(ExprKind::Call { callee: name(key), kind: NameKind::Builtin, args }, Ty::Scalar)
}

fn linalg(key: &str, args: Vec<Expr>, ty: Ty) -> Expr {
    mk(ExprKind::Call { callee: name(key), kind: NameKind::Builtin, args }, ty)
}

/// Integer literal value, through parentheses and minus signs.
fn int_lit(e: &Expr) -> Option<i64> {
    match &e.kind {
        ExprKind::Num { text, int: true } => text.parse().ok(),
        ExprKind::Paren { inner } => int_lit(inner),
        ExprKind::Neg { arg } => int_lit(arg).map(|v| -v),
        _ => None,
    }
}

/// `p/q` written with integer literals (`1/3`, `(-2)/3`, `-(1/2)`).
fn rational_lit(e: &Expr) -> Option<(i64, i64)> {
    match &e.kind {
        ExprKind::Paren { inner } => rational_lit(inner),
        ExprKind::Neg { arg } => rational_lit(arg).map(|(p, q)| (-p, q)),
        ExprKind::Div { num, den } => Some((int_lit(num)?, int_lit(den)?)).filter(|(_, q)| *q != 0),
        _ => None,
    }
}

fn is_int(e: &Expr, v: i64) -> bool {
    int_lit(e) == Some(v)
}

fn array_ty(tys: &[&Ty]) -> Ty {
    tys.iter().find(|t| t.is_array()).map_or(Ty::Scalar, |t| (*t).clone())
}

/// Zero of the type: `0`, or a list of zeros for a vector / matrix.
fn zero_like(ty: &Ty) -> Expr {
    match ty {
        Ty::Vector { n } => mk(ExprKind::List { items: (0..*n).map(|_| int(0)).collect() }, ty.clone()),
        Ty::Matrix { m, n } => {
            let row = |_| mk(ExprKind::List { items: (0..*n).map(|_| int(0)).collect() }, Ty::Vector { n: *n });
            mk(ExprKind::List { items: (0..*m).map(row).collect() }, ty.clone())
        }
        Ty::Tuple { n } => mk(ExprKind::Tuple { items: (0..*n).map(|_| int(0)).collect() }, ty.clone()),
        _ => int(0),
    }
}

/// Unary minus without double negation; a leading integer coefficient takes the sign
/// (`-(2*x)` → `-2*x`).
fn neg(e: Expr) -> Expr {
    let ty = e.ty.clone();
    match e.kind {
        ExprKind::Neg { arg } => *arg,
        ExprKind::Product { mut factors } if matches!(factors[0].kind, ExprKind::Num { int: true, .. }) => {
            let c = factors.remove(0);
            factors.insert(0, neg(c));
            mk(ExprKind::Product { factors }, ty)
        }
        kind => mk(ExprKind::Neg { arg: Box::new(mk(kind, ty.clone())) }, ty),
    }
}

/// n-ary product: nested products flattened, integer literals and signs gathered into
/// one leading coefficient. A literal 0 is never passed (zeros are `None` upstream).
fn mul(factors: Vec<Expr>) -> Expr {
    let mut coef: i64 = 1;
    let mut rest: Vec<Expr> = vec![];
    let mut stack: Vec<Expr> = factors.into_iter().rev().collect();
    while let Some(f) = stack.pop() {
        match f.kind {
            ExprKind::Product { factors } => stack.extend(factors.into_iter().rev()),
            ExprKind::Neg { arg } => {
                coef = -coef;
                stack.push(*arg);
            }
            ExprKind::Num { ref text, int: true } if text.parse::<i64>().ok().and_then(|v| coef.checked_mul(v)).is_some() => {
                coef *= text.parse::<i64>().unwrap();
            }
            _ => rest.push(f),
        }
    }
    let ty = array_ty(&rest.iter().map(|f| &f.ty).collect::<Vec<_>>());
    if coef == 0 {
        return zero_like(&ty);
    }
    let out = match (coef.abs(), rest.len()) {
        (_, 0) => return int(coef),
        (1, 1) => rest.pop().unwrap(),
        (1, _) => mk(ExprKind::Product { factors: rest }, ty.clone()),
        (c, _) => mk(ExprKind::Product { factors: std::iter::once(int(c)).chain(rest).collect() }, ty.clone()),
    };
    if coef < 0 { neg(out) } else { out }
}

/// Sum of signed terms: nested sums flattened, zeros already dropped (`None`).
fn add(terms: Vec<(Sign, Expr)>) -> Option<Expr> {
    let mut flat: Vec<Term> = vec![];
    for (sign, e) in terms {
        let (sign, e) = match e.kind {
            ExprKind::Neg { arg } => (flip(sign), *arg),
            kind => (sign, mk(kind, e.ty)),
        };
        match e.kind {
            ExprKind::Sum { terms } => {
                for t in terms {
                    flat.push(Term { id: 0, sign: if sign == Sign::Minus { flip(t.sign) } else { t.sign }, expr: t.expr });
                }
            }
            kind => flat.push(Term { id: 0, sign, expr: mk(kind, e.ty) }),
        }
    }
    match flat.len() {
        0 => None,
        1 => {
            let t = flat.pop().unwrap();
            Some(if t.sign == Sign::Minus { neg(t.expr) } else { t.expr })
        }
        _ => {
            // A leading minus prints as `-a + b`; keep it.
            let ty = array_ty(&flat.iter().map(|t| &t.expr.ty).collect::<Vec<_>>());
            Some(mk(ExprKind::Sum { terms: flat }, ty))
        }
    }
}

fn flip(s: Sign) -> Sign {
    match s {
        Sign::Plus => Sign::Minus,
        Sign::Minus => Sign::Plus,
    }
}

fn div(num: Expr, den: Expr) -> Expr {
    if is_int(&den, 1) {
        return num;
    }
    let ty = num.ty.clone();
    match num.kind {
        // `-(a)/b` prints better as `-(a/b)`.
        ExprKind::Neg { arg } => neg(mk(ExprKind::Div { num: arg, den: Box::new(den) }, ty)),
        kind => mk(ExprKind::Div { num: Box::new(mk(kind, ty.clone())), den: Box::new(den) }, ty),
    }
}

/// `b^n` for an integer n.
fn pow_int(base: Expr, n: i64) -> Expr {
    match n {
        0 => int(1),
        1 => base,
        _ => {
            let exp = if n < 0 { mk(ExprKind::Paren { inner: Box::new(int(n)) }, Ty::Scalar) } else { int(n) };
            mk(ExprKind::Pow { base: Box::new(base), exp: Box::new(exp) }, Ty::Scalar)
        }
    }
}

fn pow(base: Expr, exp: Expr) -> Expr {
    match int_lit(&exp) {
        Some(n) => pow_int(base, n),
        None => mk(ExprKind::Pow { base: Box::new(base), exp: Box::new(exp) }, Ty::Scalar),
    }
}

/// `p/q` literal exponent; a negative one in parentheses (`x^(-2/3)`).
fn rational(p: i64, q: i64) -> Expr {
    let (p, q) = if q < 0 { (-p, -q) } else { (p, q) };
    let frac = mk(ExprKind::Div { num: Box::new(int(p)), den: Box::new(int(q)) }, Ty::Scalar);
    if p < 0 { mk(ExprKind::Paren { inner: Box::new(frac) }, Ty::Scalar) } else { frac }
}

fn gcd(a: i64, b: i64) -> i64 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// `u' * g`, with u' = 1 dropped.
fn chain(du: Expr, outer: Expr) -> Expr {
    mul(vec![du, outer])
}

impl Deriv<'_> {
    fn depends(&self, e: &Expr) -> bool {
        let mut found = false;
        e.walk(&mut |n| match &n.kind {
            ExprKind::Name { name, kind: NameKind::Input } if name.key == self.input => found = true,
            ExprKind::Name { kind: NameKind::SplitName, .. } => found = true,
            _ => {}
        });
        found
    }

    /// `None` = structurally zero.
    fn go(&self, e: &Expr) -> Result<Option<Expr>, DerivError> {
        if !self.depends(e) {
            return Ok(None);
        }
        Ok(match &e.kind {
            ExprKind::Name { name, kind: NameKind::Input } if name.key == self.input => Some(int(1)),
            ExprKind::Name { kind: NameKind::SplitName, .. } => self.subject.clone().flatten(),
            ExprKind::Name { .. } | ExprKind::Num { .. } | ExprKind::Str { .. } => None,
            ExprKind::Paren { inner } => self.go(inner)?,
            ExprKind::Neg { arg } => self.go(arg)?.map(neg),
            ExprKind::Sum { terms } => {
                let mut out = vec![];
                for t in terms {
                    if let Some(d) = self.go(&t.expr)? {
                        out.push((t.sign, d));
                    }
                }
                add(out)
            }
            ExprKind::Product { factors } => {
                let mut out = vec![];
                for (i, f) in factors.iter().enumerate() {
                    if let Some(d) = self.go(f)? {
                        let fs = factors.iter().enumerate().map(|(j, g)| if i == j { d.clone() } else { g.clone() }).collect();
                        out.push((Sign::Plus, mul(fs)));
                    }
                }
                add(out)
            }
            ExprKind::Div { num, den } => {
                let dn = self.go(num)?;
                let dd = self.go(den)?;
                match (dn, dd) {
                    (dn, None) => dn.map(|dn| div(dn, (**den).clone())),
                    (None, Some(dd)) => Some(neg(div(mul(vec![(**num).clone(), dd]), pow_int((**den).clone(), 2)))),
                    (Some(dn), Some(dd)) => {
                        let top = add(vec![(Sign::Plus, mul(vec![dn, (**den).clone()])), (Sign::Minus, mul(vec![(**num).clone(), dd]))]);
                        top.map(|t| div(t, pow_int((**den).clone(), 2)))
                    }
                }
            }
            ExprKind::Pow { base, exp } => self.power(base, exp)?,
            ExprKind::Factorial { .. } => return fail("!"),
            ExprKind::Call { callee, kind: NameKind::Builtin, args } => self.builtin(&callee.key, args, &e.ty)?,
            ExprKind::Call { callee, kind: NameKind::Function, args } => {
                if args.len() != 1 {
                    return fail(format!("입력이 여럿인 함수 호출 {}(..)", callee.key));
                }
                let Some(du) = self.go(&args[0])? else { return Ok(None) };
                let u = args[0].clone();
                if callee.inverse {
                    // (g⁻¹)'(u) = 1 / g'(g⁻¹(u)).
                    let inner = mk(ExprKind::Call { callee: callee.clone(), kind: NameKind::Function, args: vec![u] }, Ty::Scalar);
                    let g1 = mk(ExprKind::Call { callee: next_deriv(&base_ident(callee)), kind: NameKind::Function, args: vec![inner] }, Ty::Scalar);
                    Some(div(du, g1))
                } else {
                    // A parametric call (whole body) evaluates to a point array.
                    let ty = match &e.ty {
                        Ty::Tuple { n } => Ty::Vector { n: *n },
                        t => t.clone(),
                    };
                    let call = mk(ExprKind::Call { callee: next_deriv(callee), kind: NameKind::Function, args: vec![u] }, ty);
                    Some(chain(du, call))
                }
            }
            ExprKind::Call { callee, .. } => return fail(callee.key.clone()),
            ExprKind::BigOp { op: BigOpKind::Sum, body, index, lo, lo_strict, hi, hi_strict } => {
                if self.depends(lo) || self.depends(hi) {
                    return fail("범위에 입력 변수가 있는 sum");
                }
                self.go(body)?.map(|b| {
                    mk(
                        ExprKind::BigOp { op: BigOpKind::Sum, body: Box::new(b), index: index.clone(), lo: lo.clone(), lo_strict: *lo_strict, hi: hi.clone(), hi_strict: *hi_strict },
                        Ty::Scalar,
                    )
                })
            }
            ExprKind::BigOp { op: BigOpKind::Prod, .. } => return fail("prod"),
            ExprKind::Tuple { items } => {
                let mut out = vec![];
                for it in items {
                    out.push(self.go(it)?.unwrap_or_else(|| int(0)));
                }
                Some(mk(ExprKind::Tuple { items: out }, e.ty.clone()))
            }
            ExprKind::List { items } => {
                let mut out = vec![];
                for it in items {
                    out.push(self.go(it)?.unwrap_or_else(|| zero_like(&it.ty)));
                }
                Some(mk(ExprKind::List { items: out }, e.ty.clone()))
            }
            ExprKind::Index { base, indices } => {
                if indices.iter().any(|i| self.depends(i)) {
                    return fail("원소 번호에 입력 변수");
                }
                self.go(base)?.map(|b| mk(ExprKind::Index { base: Box::new(b), indices: indices.clone() }, Ty::Scalar))
            }
            ExprKind::Split { name, subject, at, pieces } => {
                let ds = self.go(subject)?;
                let inner = Deriv { input: self.input, subject: Some(ds) };
                let mut out = vec![];
                for p in pieces {
                    let v = inner.go(&p.value)?.unwrap_or_else(|| int(0));
                    out.push(Piece { value: v, cond: p.cond.clone() });
                }
                Some(mk(ExprKind::Split { name: name.clone(), subject: subject.clone(), at: at.clone(), pieces: out }, e.ty.clone()))
            }
            ExprKind::Compare { .. } | ExprKind::Not { .. } | ExprKind::And { .. } | ExprKind::Or { .. } => return fail("부등식"),
            ExprKind::Address { .. } | ExprKind::Error {} => return fail("주소"),
            ExprKind::Feature { callee, .. } => return fail(callee.key.clone()),
        })
    }

    fn power(&self, base: &Expr, exp: &Expr) -> Result<Option<Expr>, DerivError> {
        let b = base.clone();
        if !self.depends(exp) {
            let Some(db) = self.go(base)? else { return Ok(None) };
            if let Some(n) = int_lit(exp) {
                // n u^(n-1) u'
                return Ok(Some(mul(vec![int(n), pow_int(b, n - 1), db])));
            }
            if let Some((p, q)) = rational_lit(exp) {
                let g = gcd(p, q).max(1);
                let (p, q) = (p / g, q / g);
                let (np, nq) = (p - q, q);
                let g2 = gcd(np, nq).max(1);
                let lower = if np == 0 { int(1) } else if np == nq { b } else { pow(b, rational(np / g2, nq / g2)) };
                let coef = rational(p, q);
                return Ok(Some(mul(vec![coef, lower, db])));
            }
            // v u^(v - 1) u'
            let lower = add(vec![(Sign::Plus, exp.clone()), (Sign::Minus, int(1))]).unwrap();
            let lower = pow(b, mk(ExprKind::Paren { inner: Box::new(lower) }, Ty::Scalar));
            return Ok(Some(mul(vec![exp.clone(), lower, db])));
        }
        let whole = mk(ExprKind::Pow { base: Box::new(base.clone()), exp: Box::new(exp.clone()) }, Ty::Scalar);
        let dv = self.go(exp)?.expect("exponent depends on the input");
        if !self.depends(base) {
            // a^v ln(a) v'
            let ln = if matches!(&base.kind, ExprKind::Name { name, kind: NameKind::Reserved } if name.key == "\\e") { None } else { Some(builtin("ln", vec![b])) };
            let mut fs = vec![whole];
            fs.extend(ln);
            fs.push(dv);
            return Ok(Some(mul(fs)));
        }
        // u^v (v' ln(u) + v u'/u)
        let du = self.go(base)?.expect("base depends on the input");
        let inner = add(vec![(Sign::Plus, mul(vec![dv, builtin("ln", vec![b.clone()])])), (Sign::Plus, div(mul(vec![exp.clone(), du]), b))]).unwrap();
        Ok(Some(mul(vec![whole, inner])))
    }

    fn builtin(&self, key: &str, args: &[Expr], ty: &Ty) -> Result<Option<Expr>, DerivError> {
        let u = || args[0].clone();
        let du = match self.go(&args[0])? {
            Some(d) => Some(d),
            None if args.len() == 1 => return Ok(None),
            None => None,
        };
        let one = |f: Expr| -> Option<Expr> { du.clone().map(|d| chain(d, f)) };
        let over = |den: Expr| -> Option<Expr> { du.clone().map(|d| div(d, den)) };
        Ok(match key {
            "sin" => one(builtin("cos", vec![u()])),
            "cos" => one(neg(builtin("sin", vec![u()]))),
            "tan" => over(pow_int(builtin("cos", vec![u()]), 2)),
            "asin" => over(builtin("sqrt", vec![add(vec![(Sign::Plus, int(1)), (Sign::Minus, pow_int(u(), 2))]).unwrap()])),
            "acos" => over(builtin("sqrt", vec![add(vec![(Sign::Plus, int(1)), (Sign::Minus, pow_int(u(), 2))]).unwrap()])).map(neg),
            "atan" => over(add(vec![(Sign::Plus, int(1)), (Sign::Plus, pow_int(u(), 2))]).unwrap()),
            "sinh" => one(builtin("cosh", vec![u()])),
            "cosh" => one(builtin("sinh", vec![u()])),
            "tanh" => over(pow_int(builtin("cosh", vec![u()]), 2)),
            "exp" => one(builtin("exp", vec![u()])),
            "ln" => over(u()),
            "log10" => over(mul(vec![u(), builtin("ln", vec![int(10)])])),
            "log2" => over(mul(vec![u(), builtin("ln", vec![int(2)])])),
            "log" => {
                // log(u, b) = ln(u) / ln(b)
                let q = mk(ExprKind::Div { num: Box::new(builtin("ln", vec![u()])), den: Box::new(builtin("ln", vec![args[1].clone()])) }, Ty::Scalar);
                return self.go(&q);
            }
            "sqrt" => over(mul(vec![int(2), builtin("sqrt", vec![u()])])),
            "abs" => one(builtin("sign", vec![u()])),
            "sign" | "floor" | "ceil" | "round" => None,
            "deg" => over(int(180)).map(|d| mul(vec![d, reserved("\\pi")])),
            "mod" => {
                if self.depends(&args[1]) {
                    return fail("mod의 둘째 인자에 입력 변수");
                }
                du
            }
            "transpose" => du.map(|d| linalg("transpose", vec![d], ty.clone())),
            "tr" => du.map(|d| linalg("tr", vec![d], Ty::Scalar)),
            "inv" => du.map(|d| {
                let inv = linalg("inv", vec![u()], ty.clone());
                neg(linalg("matmul", vec![linalg("matmul", vec![inv.clone(), d], ty.clone()), inv], ty.clone()))
            }),
            "matmul" => {
                let db = self.go(&args[1])?;
                let mut terms = vec![];
                if let Some(da) = du {
                    terms.push((Sign::Plus, linalg("matmul", vec![da, args[1].clone()], ty.clone())));
                }
                if let Some(db) = db {
                    terms.push((Sign::Plus, linalg("matmul", vec![u(), db], ty.clone())));
                }
                add(terms)
            }
            other => return fail(other.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::print::expr_dsl;

    /// Resolved body of `f(x) -> y: <src>` (plus helpers g, R, ~g, V).
    fn body(src: &str) -> Expr {
        let pre = "define:\n    g(x) -> y: x^2\n    ~g(x) -> y: sqrt(x)\n    R(t) -> matrix: [[cos(t), -sin(t)], [sin(t), cos(t)]]\n    V: [1, 2]\n";
        let state = crate::interp::apply(&Default::default(), pre, &Default::default(), 1).unwrap().state;
        let a = crate::interp::apply(&state, &format!("define:\n    f(x; a, b) -> y: {src}\n"), &Default::default(), 2).unwrap_or_else(|d| panic!("{d:?}"));
        a.state.functions["f"].body.clone()
    }

    fn d(src: &str) -> String {
        expr_dsl(&derive(&body(src), "x").unwrap())
    }

    #[test]
    fn polynomial() {
        assert_eq!(d("x^3 - 3*x"), "3*x^2 - 3");
        assert_eq!(d("a*x^2 + b*x + 1"), "2*a*x + b");
        assert_eq!(d("x^4"), "4*x^3");
        assert_eq!(d("5"), "0");
        assert_eq!(d("a*b"), "0");
        assert_eq!(d("x"), "1");
        assert_eq!(d("-x^2"), "-2*x");
    }

    #[test]
    fn second_derivative_of_a_cubic() {
        let d1 = derive(&body("x^3 - 3*x"), "x").unwrap();
        assert_eq!(expr_dsl(&derive(&d1, "x").unwrap()), "6*x");
        let d4 = (0..4).fold(body("x^4"), |e, _| derive(&e, "x").unwrap());
        assert_eq!(expr_dsl(&d4), "24");
    }

    #[test]
    fn quotient_and_powers() {
        assert_eq!(d("1/x"), "-(1/x^2)");
        assert_eq!(d("x/(x - 1)"), "((x - 1) - x)/(x - 1)^2");
        assert_eq!(d("x^(-1)"), "-x^(-2)");
        assert_eq!(d("x^(1/3)"), "1/3*x^(-2/3)");
        assert_eq!(d("x^(1/2)"), "1/2*x^(-1/2)");
        assert_eq!(d("x^a"), "a*x^(a - 1)");
        assert_eq!(d("2^x"), "2^x*ln(2)");
        assert_eq!(d("\\e^x"), "\\e^x");
        assert_eq!(d("x^x"), "x^x*(ln(x) + x/x)");
    }

    #[test]
    fn builtins_and_chain_rule() {
        assert_eq!(d("sin(3*x)"), "3*cos(3*x)");
        assert_eq!(d("cos(x)"), "-sin(x)");
        assert_eq!(d("exp(x^2)"), "2*x*exp(x^2)");
        assert_eq!(d("ln(x)"), "1/x");
        assert_eq!(d("sqrt(x)"), "1/(2*sqrt(x))");
        assert_eq!(d("abs(x)"), "sign(x)");
        assert_eq!(d("floor(x)"), "0");
        assert_eq!(d("tan(x)"), "1/cos(x)^2");
        assert_eq!(d("log(x, 2)"), "1/x/ln(2)");
    }

    #[test]
    fn calls_become_named_derivatives() {
        assert_eq!(d("g(2*x)"), "2*g@1(2*x)");
        assert_eq!(d("g(x)^2"), "2*g(x)*g@1(x)");
        assert_eq!(d("~g(x)"), "1/g@1(~g(x))");
        let e = derive(&body("g(x)"), "x").unwrap();
        let e2 = derive(&e, "x").unwrap();
        assert_eq!(expr_dsl(&e2), "g@2(x)");
    }

    #[test]
    fn linear_algebra_and_sums() {
        assert_eq!(d("matmul(R(x), V)_[1]"), "matmul(R@1(x), V)_[1]");
        assert_eq!(d("sum(x^k/k; 1 <= k <= 3)"), "sum(k*x^(k - 1)/k; 1 <= k <= 3)");
    }

    #[test]
    fn split_differentiates_each_line() {
        let pre = "define:\n    f(x) -> y:\n        split x at 0\n        -x, x < 0\n        x^2, x >= 0\n";
        let s = crate::interp::apply(&Default::default(), pre, &Default::default(), 1).unwrap().state;
        let out = derive(&s.functions["f"].body, "x").unwrap();
        assert_eq!(expr_dsl(&out), "split x at 0\n-1, x < 0\n2*x, x >= 0");
    }

    #[test]
    fn unsupported_constructs_are_named() {
        for (src, what) in [("gamma(x)", "gamma"), ("x!", "!"), ("prod(x + k; 1 <= k <= 3)", "prod")] {
            assert_eq!(derive(&body(src), "x").unwrap_err().what, what, "{src}");
        }
        // Constant parts never fail.
        assert_eq!(d("gamma(a) + x"), "1");
    }
}
