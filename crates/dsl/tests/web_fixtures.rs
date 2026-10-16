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
];

#[test]
fn write_web_fixtures() {
    let src = format!("define:\n{}", DEFS.iter().map(|d| format!("    {d}\n")).collect::<String>());
    let state = apply(&State::default(), &src, &Limits::default(), 1).unwrap_or_else(|d| panic!("{d:?}")).state;
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/test/fixtures/state.json");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    std::fs::write(out, serde_json::to_string_pretty(&state).unwrap()).unwrap();
}
