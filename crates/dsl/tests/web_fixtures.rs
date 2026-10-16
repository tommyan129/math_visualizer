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
}
