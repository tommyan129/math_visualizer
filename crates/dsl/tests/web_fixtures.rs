//! Writes `web/test/fixtures/state.json`: a resolved state used by the browser tests,
//! so codegen and sampling are tested against the AST the server really produces.

use dsl::interp::{Limits, apply};
use dsl::state::State;

const DEFS: &[&str] = &[
    "recip(x) -> y: 1/x",
    "cbrt(x) -> y: x^(1/3)",
    "cbrt2(x) -> y: x^{2/3}",
    "half(x) -> y: x^(1/2)",
    "dec(x) -> y: x^0.5",
    "negsq(x) -> y: -2^2 + 0*x",
    "taylor(x; n) -> y: sum(x^k/k!; 0 <= k <= n)",
    "harm(x) -> y: sum(1/k; 1 <= k <= x)",
    "usesharm(x) -> y: harm(x) + 1",
    "rnd(x) -> y: round(x)",
    "md(x) -> y: mod(x, 3)",
    "fct(x) -> y: x!",
    "gm(x) -> y: gamma(x)",
    "par(x; a) -> y: a*x^2",
    "callpar(x; a) -> y: par(x + 1) - par(x)",
    "tn(x) -> y: tan(x)",
    "sq(x) -> y: sqrt(x)",
    "steep(x) -> y: 1000*x",
    "lg(x) -> y: log(x, 2) + ln(\\e) + deg(180) - \\pi",
    "fl(x) -> y: floor(x)",
    "big(x) -> y: sum(1; 1 <= k <= 1000000)",
    "lin(x) -> y: x",
    "quad(x) -> y: x^2",
    "poly(x; a, b) -> y: a*x^2 - b*x + 1",
    "wave(t; a) -> y: a*sin(t)",
    // Stage 4: vectors, matrices, matrix functions, span, arrow.
    "M2: [[1, 2], [3, 4]]",
    "vv(; a): [1, a]",
    "Rot(\\theta) -> matrix: [[cos(\\theta), -sin(\\theta)], [sin(\\theta), cos(\\theta)]]",
    "rotx(x; b) -> y: matmul(Rot(b), [x, 1])_[2]",
    "linalg(x) -> y: det(M2)*x + tr(inv(M2)) + pow(M2, 2)_[1, 2] + transpose(M2)_[1, 2] + pow(M2, -1)_[1, 1]",
    "outer(x; k) -> y: matmul([1, x], transpose([2, 3]))_[2, 2] + [1, 2, 3]_[k]",
    "Sp(; a): span(vv, [0, 1])",
    "Sl: span([1, 2], [2, 4])",
    "Sz: span([0, 0])",
    "Ar(; a): arrow(vv; [1, 1])",
];

/// Stage 2: highlights over the functions above.
const HIGHLIGHTS: &str = "\
R1 = 0 <= x <= 2
H1 = highlight(quad; R1; blue; \"넓이\")
H2 = highlight(; between(lin, quad) and 0 <= x <= 2; green)
H3 = highlight(poly.(-b*x).coef;; orange)
H4 = highlight(poly.(-b*x);; red)
H5 = highlight(lin; x < -1 or x > 1)
H6 = highlight(; y < par(x) and not (x > a))
";

/// Stage 3: a scene on another coordinate system with clip, view, steps and notes.
const SCENES: &str = "\
H7 = highlight(; 0 < t < 1 and y < wave(t))
scene(1):
    space: 2D(t, y)
    show: wave
    hide: H7
    clip: y > -0.5
    view: -2*\\pi <= t <= 2*\\pi and -2 <= y <= 2
    step:
        show: +H7
        set: a = 2
        note: \"{wave} 아래 $\\int$ {{넓이}}\"
    step:
        animate: a = 1 -> 3, 0.5s, loop
        view: -1 <= t <= 1
        note: + \"{wave(1) > 0}\"
";

/// Stage 5: split bodies (block definitions).
const SPLITS: &str = "\
define:
    absx(x) -> y:
        split x at 0
        -x, x < 0
        x, x >= 0
    stp(x; a, b) -> y:
        split x at a, b
        0, x < a
        1, a <= x <= b
        2, x > b
    sq2(x) -> y:
        split r = x^2 at 1
        r, r < 1
        1/r, r >= 1
    gap(x) -> y:
        split x at 0
        -1, x < 0
        1, x > 0
    pc(t) -> (x, y): (cos(t), sin(t)), 0 <= t <= 2*\\pi
    pv(t; a) -> (x, y): matmul(Rot(t), [a, 0]), 0 <= t <= \\pi
    hc(\\theta) -> polar(r): 1 + cos(\\theta)
    Ci(x, y): x^2 + y^2 = 1
    Hy(x, y): x*y = 1
    Dv(t; n) -> (x, y): family((j, t), 0 <= t <= quad(j); 0 <= j <= n)
    Oc(x, y): family(x^2 + y^2 = j^2; 1 <= j <= 3)
    sad(x, y) -> z: x^2 - y^2
    usesad(x) -> y: sad(x, 1)
    hx(t; a) -> (x, y, z): (cos(t), sin(t), a*t), 0 <= t <= 4*\\pi
    sph(u, v) -> (x, y, z): (cos(u)*sin(v), sin(u)*sin(v), cos(v)), 0 <= v <= \\pi, 0 <= u <= 2*\\pi
    pole(x, y) -> z: 1/x
    S3: span([1, 0, 0], [0, 1, 1])
    A3: arrow([1, 2, 3]; [0, 0, 1])
H8 = highlight(sad; x^2 + y^2 < 1)
";

/// Stage 5b: generated derivatives and feature points (`web/test/fixtures/features.json`).
const FEATURES: &str = "\
define:
    cub(x) -> y: x^3 - 3*x
    E: extrema(cub)
    I: inflections(cub)
    R: roots(cub)
    q4(x) -> y: x^4
    I4: inflections(q4)
    E4: extrema(q4)
    R4: roots(q4)
    dbl(x) -> y: (x - 1)^2*(x + 2)
    R2: roots(dbl)
    rat(x) -> y: (x^2 - 1)/(x - 1) + 1/(x + 2)
    B: breaks(rat)
    Bh: breaks(rat;; hole)
    ab(x) -> y: abs(x - 1)
    Bc: breaks(ab)
    sq(x) -> y: sqrt(x + 3)
    Bs: breaks(sq)
    st(x) -> y:
        split x at 0
        -1, x < 0
        1, x >= 0
    Bj: breaks(st)
    lin2(x) -> y: x
    P: intersect(cub, lin2)
    Q: intersect(cub, y = 2)
    Y: yint(cub)
    M: argmax(cub; -3 <= x <= 3)
    Mm: argmin(cub; -2 <= x <= 2)
    rec(x) -> y: 1/(x + 3)
    Mi: argmax(rec; -5 <= x <= 0)
    Mi2: argmin(rec; -5 <= x <= 0)
    Mi3: argmax(rec; -2 <= x <= 0)
    hp(x) -> y: 1 - (x - 1)^3/(x - 1)
    Mh: argmax(hp; 0 <= x <= 2)
    Mj: argmax(st; -1 <= x <= 1)
    lg(x) -> y: ln(abs(x + 3))
    Ml: argmin(lg; -5 <= x <= 0)
    Bl: breaks(lg)
    rs(x) -> y: 1/sqrt(abs(x))
    Mr: argmax(rs; -1 <= x <= 1)
    Br: breaks(rs)
    fl(x) -> y: floor(x)
    Bf: breaks(fl; -2.5 <= x <= 2.5)
    fd(x) -> y: floor(100*x)
    Bd: breaks(fd; 0.005 <= x <= 0.035)
    rd(x) -> y: round(x)
    Bo: breaks(rd; 0 <= x <= 2)
    md(x) -> y: mod(x, 2)
    Bm: breaks(md; -1 <= x <= 3)
    Pm: extrema(cub;; min)
    Rr(; c): roots(cub; 0 < x < c)
    T(x) -> y: cub@1(Pm_[1])*(x - Pm_[1]) + Pm_[2]
    U(x) -> y: x - P_[1]
    circ(t) -> (x, y): (cos(t), sin(t)), 0 <= t <= 2*\\pi
    Tc: tangents(circ)
    card(\\theta) -> polar(r): 1 + cos(\\theta)
    Tp: tangents(card;; horizontal)
";

/// Stage 5c: calculation steps and Claude's derivatives / inverses to verify
/// (`web/test/fixtures/verify.json`).
const VERIFY: &str = "\
define:
    f(x) -> y: (x+1)^2 - (x-1)^2
    g(x) -> y: (x^2 + 2*x + 1) - (x^2 - 2*x + 1)
    h(x) -> y: 4*x
    C: calc(f, g, h)
    hb(x) -> y: 4*x + 1
    Cb: calc(f, hb)
    s1(x) -> y: sqrt(x^2)
    s2(x) -> y: x
    Cs: calc(s1, s2; x > 0)
    Cn: calc(s1, s2)
    l1(x) -> y: ln(x^2)
    l2(x) -> y: 2*ln(x)
    Cl: calc(l1, l2)
    p(x) -> y: x^2
    P1(x) -> y: x^3/3 + 7
    Ci: calc(p, P1;; integral)
    P2(x) -> y: x^3/2
    Cw: calc(p, P2;; integral)
    Cm: calc(f, h;; limit)
    cu(x; a) -> y: a*x^3
    cu@1(x; a) -> y: 3*a*x^2
    cu@2(x; a) -> y: 6*x
    w(x) -> y: sin(x)
    w@1(x) -> y: sin(x)
    e(x) -> y: exp(x)
    ~e(x) -> y: ln(x)
    q(x) -> y: x^2
    ~q(x) -> y: sqrt(x)
    qb(x) -> y: x^2
    ~qb(x) -> y: x/2
    pp(\\theta) -> polar(r): 2*sin(\\theta)*cos(\\theta)
    pq(\\theta) -> polar(r): sin(2*\\theta)
    Cp: calc(pp, pq)
    du(x) -> y: x^4
    du@3(x) -> y: 24*x
    nf(x) -> y: 1/x
    nn(x) -> y: sqrt(-1 - x^2)
    Cu: calc(nf, nn)
";

#[test]
fn write_web_fixtures() {
    let src = format!("define:\n{}", DEFS.iter().map(|d| format!("    {d}\n")).collect::<String>());
    let state = apply(&State::default(), &src, &Limits::default(), 1).unwrap_or_else(|d| panic!("{d:?}")).state;
    let state = apply(&state, HIGHLIGHTS, &Limits::default(), 2).unwrap_or_else(|d| panic!("{d:?}")).state;
    let state = apply(&state, SCENES, &Limits::default(), 3).unwrap_or_else(|d| panic!("{d:?}")).state;
    let state = dsl::interp::apply_user_op(&state, &dsl::interp::UserOp::Toggle { scene: "2D(x, y)".into(), target: "lin".into(), visible: false }, 4).unwrap().state;
    let state = apply(&state, SPLITS, &Limits::default(), 5).unwrap_or_else(|d| panic!("{d:?}")).state;
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/test/fixtures/state.json");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    std::fs::write(out, serde_json::to_string_pretty(&state).unwrap()).unwrap();

    let features = apply(&State::default(), FEATURES, &Limits::default(), 1).unwrap_or_else(|d| panic!("{d:?}")).state;
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/test/fixtures/features.json");
    std::fs::write(out, serde_json::to_string_pretty(&features).unwrap()).unwrap();

    let verify = apply(&State::default(), VERIFY, &Limits::default(), 1).unwrap_or_else(|d| panic!("{d:?}")).state;
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/test/fixtures/verify.json");
    std::fs::write(out, serde_json::to_string_pretty(&verify).unwrap()).unwrap();
}
