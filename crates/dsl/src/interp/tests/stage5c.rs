//! Stage 5c: calculation steps `calc(...)`.

use super::*;
use crate::print::expr_dsl;

const STEPS: [&str; 3] = ["f(x) -> y: (x+1)^2 - (x-1)^2", "g(x) -> y: (x^2 + 2*x + 1) - (x^2 - 2*x + 1)", "h(x) -> y: 4*x"];

fn with_steps(extra: &[&str]) -> Vec<String> {
    STEPS.iter().chain(extra).map(|s| s.to_string()).collect()
}

fn defs(lines: &[String]) -> String {
    def(&lines.iter().map(|s| s.as_str()).collect::<Vec<_>>())
}

#[test]
fn calc_definition() {
    let s = build(&[&defs(&with_steps(&["C: calc(f, g, h)"]))]);
    let c = &s.functions["C"];
    assert_eq!(c.kind, DefKind::Calc);
    assert_eq!(c.calls, ["f", "g", "h"]);
    let spec = c.calc.as_ref().unwrap();
    assert_eq!(spec.kind, CalcKind::Transform);
    assert_eq!(spec.steps, ["f", "g", "h"]);
    assert_eq!(spec.input.key, "x");
    assert_eq!(spec.axes.iter().map(|a| a.key.as_str()).collect::<Vec<_>>(), ["x", "y"]);
    // One diff per neighbouring pair; f → g changes both squares, g → h everything.
    assert_eq!(spec.diffs.len(), 2);
    assert_eq!(spec.diffs[0].before.len(), 2);
    assert_eq!(spec.diffs[0].after.len(), 2);
    assert_eq!(spec.diffs[1].after, [s.functions["h"].body.id]);
    assert_eq!(c.source(), "C: calc(f, g, h)");
    // Shown with its steps in the default scene.
    assert!(s.scenes[DEFAULT_SCENE].members.contains(&"C".to_string()));
}

#[test]
fn calc_slots() {
    let s = build(&[&def(&["f(x) -> y: sqrt(x^2)", "g(x) -> y: x", "F(x) -> y: x^2/2", "C: calc(f, g; x > 0)", "I: calc(g, F;; integral)", "L: calc(f, g; 0 < x < 3; limit)"])]);
    let c = s.functions["C"].calc.as_ref().unwrap();
    assert_eq!(expr_dsl(c.range.as_ref().unwrap()), "x > 0");
    assert_eq!(c.lo.as_ref().map(expr_dsl).as_deref(), Some("0"));
    assert!(c.hi.is_none());
    // sqrt's argument is a domain break candidate.
    assert!(c.candidates.iter().any(|b| b.cause == BreakCause::Domain));
    assert_eq!(s.functions["I"].calc.as_ref().unwrap().kind, CalcKind::Integral);
    let l = s.functions["L"].calc.as_ref().unwrap();
    assert_eq!(l.kind, CalcKind::Limit);
    assert_eq!(l.hi.as_ref().map(expr_dsl).as_deref(), Some("3"));
    assert_eq!(s.functions["I"].source(), "I: calc(g, F;; integral)");
}

#[test]
fn calc_errors() {
    let s = build(&[&def(&["f(x) -> y: x", "g(x) -> y: x", "u(t) -> y: t", "p(t) -> (x, y): (t, t), 0 <= t <= 1", "v: [1, 2]", "k(x; a) -> y: a*x"])]);
    let one = first_err(&s, &def(&["C: calc(f)"]));
    assert_eq!(one.code, code::ARITY);
    assert_eq!(one.suggestion.as_deref(), Some("calc(f, g)"));
    assert_eq!(codes(&s, &def(&["C: calc(f, 2*x)"])), vec![code::TARGET]);
    assert_eq!(codes(&s, &def(&["C: calc(f, q)"])), vec![code::UNDECLARED]);
    assert_eq!(codes(&s, &def(&["C: calc(f, p)"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["C: calc(f, v)"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["C: calc(f, f)"])), vec![code::DUPLICATE]);
    assert_eq!(codes(&s, &def(&["C: calc(f, u)"])), vec![code::AXES]);
    let k = first_err(&s, &def(&["C: calc(f, g;; derivative)"]));
    assert_eq!(k.code, code::CALC);
    assert_eq!(k.suggestion.as_deref(), Some("calc(f, g;; integral)"));
    assert_eq!(codes(&s, &def(&["C: calc(f, g; x + 1)"])), vec![code::RANGE]);
    assert_eq!(codes(&s, &def(&["C: calc(f, g; x > a)"])), vec![code::MISSING_CONST]);
    build_on(&s, &def(&["C(; a): calc(f, g; x > a)"]));
    // Not a value: cannot be used in an expression, called, or written inside one.
    let s = build_on(&s, &def(&["C: calc(f, g)"]));
    assert_eq!(codes(&s, &def(&["m(x) -> y: C"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["m(x) -> y: C(x)"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["m(x) -> y: calc(f, g)"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["calc(x) -> y: x"])), vec![code::NAME_CONFLICT]);
}

#[test]
fn calc_constant_rule() {
    // A step's constants must be in the calc head (referenced-definition rule).
    let s = build(&[&def(&["f(x; a) -> y: a*(x + 1)", "g(x; a) -> y: a*x + a"])]);
    assert_eq!(codes(&s, &def(&["C: calc(f, g)"])), vec![code::MISSING_CONST]);
    let s = build_on(&s, &def(&["C(; a): calc(f, g)"]));
    assert_eq!(s.functions["C"].consts, ["a"]);
}

#[test]
fn calc_follows_its_steps() {
    let s = build(&[&defs(&with_steps(&["C: calc(f, g, h)"]))]);
    // Changing a step recomputes the diff.
    let s2 = build_on(&s, &def(&["h(x) -> y: 4*x + 0"]));
    let d = &s2.functions["C"].calc.as_ref().unwrap().diffs[1];
    assert_ne!(d, &s.functions["C"].calc.as_ref().unwrap().diffs[1]);
    // A step changed to other axes is rejected; deleting one removes C.
    assert_eq!(codes(&s, &def(&["h(t) -> y: 4*t"])), vec![code::TYPE]);
    let a = run(&s, "del g").unwrap();
    assert!(!a.state.functions.contains_key("C"));
    assert!(a.notes.contains(&Note::FunctionDeleted { key: "C".into(), via: Some("g".into()) }));
}

#[test]
fn calc_with_named_derivatives_and_polar() {
    build(&[&def(&["f(x) -> y: x^3", "f@1(x) -> y: 3*x^2", "g(x) -> y: 3*x*x", "C: calc(f@1, g)"])]);
    let s = build(&[&def(&["p(\\theta) -> polar(r): 2*sin(\\theta)*cos(\\theta)", "q(\\theta) -> polar(r): sin(2*\\theta)", "C: calc(p, q)"])]);
    assert!(s.functions["C"].calc.as_ref().unwrap().polar);
}

fn build_on(state: &State, src: &str) -> State {
    run(state, src).unwrap_or_else(|d| panic!("{src}: {d:?}")).state
}
