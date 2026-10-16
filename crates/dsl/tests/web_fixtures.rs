//! Writes `web/test/fixtures/state.json`: a resolved state used by the browser tests,
//! so codegen and sampling are tested against the AST the server really produces.

use dsl::interp::{Limits, apply};
use dsl::state::State;

const DEFS: &[&str] = &[
    "inv(x) -> y: 1/x",
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

#[test]
fn write_web_fixtures() {
    let src = format!("define:\n{}", DEFS.iter().map(|d| format!("    {d}\n")).collect::<String>());
    let state = apply(&State::default(), &src, &Limits::default(), 1).unwrap_or_else(|d| panic!("{d:?}")).state;
    let state = apply(&state, HIGHLIGHTS, &Limits::default(), 2).unwrap_or_else(|d| panic!("{d:?}")).state;
    let state = apply(&state, SCENES, &Limits::default(), 3).unwrap_or_else(|d| panic!("{d:?}")).state;
    let state = dsl::interp::apply_user_op(&state, &dsl::interp::UserOp::Toggle { scene: "2D(x, y)".into(), target: "lin".into(), visible: false }, 4).unwrap().state;
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/test/fixtures/state.json");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    std::fs::write(out, serde_json::to_string_pretty(&state).unwrap()).unwrap();
}
