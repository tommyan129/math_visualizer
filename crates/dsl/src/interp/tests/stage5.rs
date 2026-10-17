//! Stage 5a: split, parametric, polar, equations.

use super::*;

fn build_on(state: &State, src: &str) -> State {
    run(state, src).unwrap_or_else(|d| panic!("{src}: {d:?}")).state
}

fn split_def(head: &str, lines: &[&str]) -> String {
    let mut s = format!("define:\n    {head}:\n");
    for l in lines {
        s.push_str("        ");
        s.push_str(l);
        s.push('\n');
    }
    s
}

#[test]
fn split_basic() {
    let s = build(&[&split_def("f(x) -> y", &["split x at 0", "-x, x < 0", "x, x >= 0"])]);
    let f = &s.functions["f"];
    assert_eq!(f.ty, Ty::Scalar);
    assert!(matches!(&f.body.kind, ExprKind::Split { pieces, .. } if pieces.len() == 2));
    assert_eq!(f.source(), "f(x) -> y:\n    split x at 0\n        -x, x < 0\n        x, x >= 0");
    assert!(f.constraints.is_empty());
}

#[test]
fn split_named_subject_usable_in_pieces() {
    let s = build(&[&split_def("f(x) -> y", &["split r = x^2 at 1", "r, r < 1", "1/r, r >= 1"])]);
    let mut kinds = vec![];
    s.functions["f"].body.walk(&mut |e| {
        if let ExprKind::Name { name, kind } = &e.kind
            && name.key == "r"
        {
            kinds.push(*kind);
        }
    });
    assert!(kinds.len() == 4 && kinds.iter().all(|k| *k == NameKind::SplitName));
}

#[test]
fn split_condition_written_with_the_expression() {
    // Unnamed subject: conditions repeat the expression itself; parentheses do not matter.
    build(&[&split_def("f(x) -> y", &["split x - 1 at 0", "0, (x - 1) < 0", "1, x - 1 >= 0"])]);
}

#[test]
fn split_overlap_rejected() {
    let src = split_def("f(x) -> y", &["split x at 0, 1", "x, x <= 0", "1, 0 <= x <= 1", "2, x > 1"]);
    let d = first_err(&State::default(), &src);
    assert_eq!(d.code, code::SPLIT_OVERLAP);
    assert!(d.message.contains("x == 0"), "{}", d.message);
}

#[test]
fn split_unclaimed_piece_is_undefined_not_error() {
    // `x == 0` is claimed by no line: allowed (undefined there).
    build(&[&split_def("f(x) -> y", &["split x at 0", "-1, x < 0", "1, x > 0"])]);
}

#[test]
fn split_compound_conditions() {
    // `or`, `not` and chains combine on the pieces; together they cover each piece once.
    build(&[&split_def("f(x) -> y", &["split x at 0, 1", "0, x < 0 or x > 1", "1, not (x < 0 or x > 1)"])]);
    let src = split_def("f(x) -> y", &["split x at 0, 1", "0, x < 0 or x > 1", "1, x >= 1"]);
    assert_eq!(codes(&State::default(), &src), vec![code::SPLIT_OVERLAP]);
}

#[test]
fn split_condition_must_use_subject_and_boundaries() {
    let src = split_def("f(x) -> y", &["split x at 0", "-x, x < 5", "x, x >= 0"]);
    assert_eq!(codes(&State::default(), &src), vec![code::SPLIT_COND]);
    // A plain expression is not a condition.
    let src = split_def("f(x) -> y", &["split x at 0", "-x, x", "x, x >= 0"]);
    assert_eq!(codes(&State::default(), &src), vec![code::SPLIT_COND]);
}

#[test]
fn split_boundary_order_and_constraints() {
    let s = build(&[&split_def("f(x; a, b) -> y", &["split x at a, b", "0, x < a", "x, a <= x <= b", "1, x > b"])]);
    let c = &s.functions["f"].constraints;
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].src, "a < b");
    // Mixed: number and constant → constraint; two numbers out of order → error.
    let s = build(&[&split_def("g(x; a) -> y", &["split x at 0, a", "0, x < 0", "1, x >= 0"])]);
    assert_eq!(s.functions["g"].constraints[0].src, "0 < a");
    let src = split_def("f(x) -> y", &["split x at 1, 0", "0, x < 1", "1, x >= 1"]);
    let d = first_err(&State::default(), &src);
    assert_eq!(d.code, code::SPLIT_AT);
    assert_eq!(d.suggestion.as_deref(), Some("at 0, 1"));
}

#[test]
fn split_boundary_errors() {
    let src = split_def("f(x; a) -> y", &["split x at a, a", "0, x < a", "1, x >= a"]);
    assert_eq!(codes(&State::default(), &src), vec![code::SPLIT_AT]);
    let src = split_def("f(x) -> y", &["split x at x", "0, x < x", "1, x >= x"]);
    assert!(codes(&State::default(), &src).contains(&code::SPLIT_AT));
    // Boundary constants follow the referenced-constant rule like any other constant.
    let src = split_def("f(x) -> y", &["split x at a", "0, x < a", "1, x >= a"]);
    assert_eq!(codes(&State::default(), &src), vec![code::UNDECLARED]);
}

#[test]
fn split_name_conflicts() {
    let src = split_def("f(x; a) -> y", &["split a = x at 0", "0, a < 0", "1, a >= 0"]);
    assert_eq!(codes(&State::default(), &src), vec![code::NAME_CONFLICT]);
    let src = split_def("f(x) -> y", &["split x = x^2 at 0", "0, x < 0", "1, x >= 0"]);
    assert!(codes(&State::default(), &src).contains(&code::NAME_CONFLICT));
}

#[test]
fn split_only_in_curves() {
    let src = split_def("R(x) -> matrix", &["split x at 0", "[1, 0], x < 0", "[0, 1], x >= 0"]);
    assert!(codes(&State::default(), &src).contains(&code::TYPE));
}

#[test]
fn split_function_called_by_another() {
    let s = build(&[
        &split_def("f(x; a) -> y", &["split x at a", "0, x < a", "1, x >= a"]),
        &def(&["g(x; a) -> y: 2*f(x)"]),
    ]);
    assert_eq!(s.functions["g"].consts, ["a"]);
}

// ----- parametric / polar / equation ---------------------------------------

#[test]
fn parametric_definition() {
    let a = run(&State::default(), &def(&["p(t) -> (x, y): (cos(t), sin(t)), 0 <= t <= 2*\\pi"])).unwrap();
    let p = &a.state.functions["p"];
    assert_eq!(p.kind, DefKind::Parametric);
    assert_eq!(p.ty, Ty::Tuple { n: 2 });
    assert_eq!(p.outputs.iter().map(|o| o.key.as_str()).collect::<Vec<_>>(), ["x", "y"]);
    assert_eq!(p.range.as_ref().unwrap().src, "0 <= t <= 2*\\pi");
    assert_eq!(p.source(), "p(t) -> (x, y): (cos(t), sin(t)), 0 <= t <= 2*\\pi");
    // Goes into the default 2D(x, y) scene by its outputs.
    assert!(a.state.scenes["2D(x, y)"].members.contains(&"p".to_string()));
}

#[test]
fn parametric_range_required_and_shaped() {
    let d = first_err(&State::default(), &def(&["p(t) -> (x, y): (cos(t), sin(t))"]));
    assert_eq!(d.code, code::RANGE);
    assert_eq!(d.suggestion.as_deref(), Some("p(t) -> (x, y): (cos(t), sin(t)), 0 <= t <= 2*\\pi"));
    assert_eq!(codes(&State::default(), &def(&["p(t) -> (x, y): (t, t), t < 1"])), vec![code::RANGE]);
    assert_eq!(codes(&State::default(), &def(&["p(t) -> (x, y): (t, t), 0 <= t <= t^2"])), vec![code::RANGE]);
    // Written from the top, with a constant and a call in the bound.
    let s = build(&[&def(&["f(x) -> y: x^2", "p(t; k) -> (x, y): (k, t), f(k) >= t >= 0"])]);
    let r = s.functions["p"].range.as_ref().unwrap();
    assert!(matches!(r.lo.kind, ExprKind::Num { .. }));
    assert!(matches!(r.hi.kind, ExprKind::Call { .. }));
    assert_eq!(s.functions["p"].calls, ["f"]);
    // A range on an ordinary curve is not a domain restriction.
    assert_eq!(codes(&State::default(), &def(&["f(x) -> y: x, 0 <= x <= 1"])), vec![code::RANGE]);
}

#[test]
fn parametric_output_slots() {
    let s = build(&[&def(&["p(t) -> (x, y): (cos(t), sin(t)), 0 <= t <= 1", "R(\\theta) -> matrix: [[cos(\\theta), -sin(\\theta)], [sin(\\theta), cos(\\theta)]]"])]);
    // A vector of the right length fills both slots; a whole parametric call too.
    build_on(&s, &def(&["q(t) -> (x, y): matmul(R(t), [1, 0]), 0 <= t <= 1"]));
    build_on(&s, &def(&["q(t) -> (x, y): p(2*t), 0 <= t <= 1"]));
    // Multi-output function in one slot, vector in one slot, wrong vector length, wrong slot count.
    assert_eq!(codes(&s, &def(&["q(t) -> (x, y): (p(t), t), 0 <= t <= 1"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["q(t) -> (x, y): ([t, 1], t), 0 <= t <= 1"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["q(t) -> (x, y): [t, t, t], 0 <= t <= 1"])), vec![code::SHAPE]);
    assert_eq!(codes(&s, &def(&["q(t) -> (x, y): (t, t, t), 0 <= t <= 1"])), vec![code::SHAPE]);
    assert_eq!(codes(&s, &def(&["q(t) -> (x, y): t, 0 <= t <= 1"])), vec![code::TYPE]);
    // A parametric function cannot be called inside an expression.
    assert_eq!(codes(&s, &def(&["f(x) -> y: p(x) + 1"])), vec![code::TYPE]);
    // Output axes are not values in the body.
    assert_eq!(codes(&s, &def(&["q(t) -> (x, y): (x, t), 0 <= t <= 1"])), vec![code::UNDECLARED]);
}

#[test]
fn polar_definition_and_scene() {
    let a = run(&State::default(), &def(&["h(\\theta) -> polar(r): 1 + cos(\\theta)"])).unwrap();
    let h = &a.state.functions["h"];
    assert_eq!((h.kind, h.range.is_none()), (DefKind::Polar, true));
    assert_eq!(h.source(), "h(\\theta) -> polar(r): 1 + cos(\\theta)");
    let sc = &a.state.scenes["polar(r, \\theta)"];
    assert!(sc.polar && sc.members == ["h"]);
    assert!(!a.state.scenes.contains_key("2D(x, y)"));
    // Optional range; callable as a number; split body.
    let s = build(&[&def(&["h(\\theta) -> polar(r): \\theta, 0 <= \\theta <= 4*\\pi", "f(x) -> y: h(x)"])]);
    assert!(s.functions["h"].range.is_some());
    build(&[&split_def("h(\\theta) -> polar(r)", &["split \\theta at \\pi", "1, \\theta < \\pi", "2, \\theta >= \\pi"])]);
    assert_eq!(codes(&State::default(), &def(&["h(\\theta) -> polar(r, s): 1"])), vec![code::AXES]);
    assert_eq!(codes(&State::default(), &def(&["h(\\theta) -> polar(r): r"])), vec![code::UNDECLARED]);
}

#[test]
fn polar_claude_scene() {
    let s = build(&[&def(&["h(\\theta) -> polar(r): 1 + cos(\\theta)", "f(x) -> y: x"])]);
    let a = run(&s, "scene(1):\n    space: polar(r, \\theta)\n    show: h\n    view: r <= 3\n    clip: r <= 2 and 0 <= \\theta <= \\pi\n").unwrap();
    let sc = &a.state.scenes["1"];
    assert!(sc.polar);
    let v = sc.view.as_ref().unwrap();
    assert!(matches!(v.h.as_ref().unwrap().lo.kind, ExprKind::Neg { .. }));
    // A curve of 2D(x, y) does not fit; neither do views over θ.
    assert_eq!(codes(&s, "scene(1):\n    space: polar(r, \\theta)\n    show: f\n"), vec![code::PLACEMENT]);
    assert_eq!(codes(&s, "scene(1):\n    space: polar(r, \\theta)\n    view: 0 <= \\theta <= 1\n"), vec![code::VIEW]);
    // Without `space`, the system comes from the shown function.
    let a = run(&s, "scene(2):\n    show: h\n").unwrap();
    assert!(a.state.scenes["2"].polar);
}

#[test]
fn equation_definition() {
    let a = run(&State::default(), &def(&["C(x, y; a): x^2 + y^2 = a^2"])).unwrap();
    let c = &a.state.functions["C"];
    assert_eq!((c.kind, c.ty.clone()), (DefKind::Equation, Ty::Region));
    assert_eq!(c.source(), "C(x, y; a): x^2 + y^2 = a^2");
    assert!(a.state.scenes["2D(x, y)"].members.contains(&"C".to_string()));
    // Calling an equation, an output arrow, one input, an inequality side.
    let s = a.state;
    assert_eq!(codes(&s, &def(&["f(x; a) -> y: C(x, 1)"])), vec![code::TYPE]);
    assert_eq!(codes(&State::default(), &def(&["C(x, y) -> z: x = y"])), vec![code::ARROW]);
    assert_eq!(codes(&State::default(), &def(&["C(x): x = 1"])), vec![code::AXES]);
    assert_eq!(codes(&State::default(), &def(&["C(x, y): (x < 1) = y"])), vec![code::TYPE]);
}

#[test]
fn new_kinds_cannot_be_highlighted_yet() {
    let s = build(&[&def(&["p(t) -> (x, y): (t, t), 0 <= t <= 1", "C(x, y): x = y"])]);
    assert_eq!(codes(&s, "H1 = highlight(p)\n"), vec![code::UNSUPPORTED]);
    assert_eq!(codes(&s, "H1 = highlight(C)\n"), vec![code::UNSUPPORTED]);
}

// ----- family ----------------------------------------------------------------

#[test]
fn family_riemann_dividers() {
    let src = def(&["f(x) -> y: x^2", "D(t; a, h, n) -> (x, y): family((a + k*h, t), 0 <= t <= f(a + k*h); 0 <= k <= n)"]);
    let a = run(&State::default(), &src).unwrap();
    let d = &a.state.functions["D"];
    let e = d.family.as_ref().unwrap();
    assert_eq!((e.index.key.as_str(), e.src.as_str()), ("k", "0 <= k <= n"));
    assert_eq!(d.source(), "D(t; a, h, n) -> (x, y): family((a + k*h, t), 0 <= t <= f(a + k*h); 0 <= k <= n)");
    // k is the index in the body and the range; n (used in the bound) becomes an integer slider.
    let mut kinds = vec![];
    d.body.walk(&mut |x| {
        if let ExprKind::Name { name, kind } = &x.kind
            && name.key == "k"
        {
            kinds.push(*kind);
        }
    });
    assert!(!kinds.is_empty() && kinds.iter().all(|k| *k == NameKind::Index));
    assert!(a.state.constants["n"].integer);
    assert!(!a.state.constants["a"].integer);
}

#[test]
fn family_other_kinds() {
    build(&[&def(&["O(x, y; n): family(x^2 + y^2 = k^2; 1 <= k <= n)", "F(x) -> y: family(sin(x + k*\\pi/4); 0 <= k <= 3)", "h(\\theta) -> polar(r): family(k*\\theta; 1 <= k <= 3)"])]);
}

#[test]
fn family_arrows() {
    // A vector field on a grid: column mod(k, N), row floor(k/N).
    let src = def(&["F(x, y) -> matrix: [-y, x]", "V(; N): family(arrow(F(mod(k, N) - N/2, floor(k/N) - N/2)/N; [mod(k, N) - N/2, floor(k/N) - N/2]); 0 <= k <= N^2 - 1)"]);
    let a = run(&State::default(), &src).unwrap_or_else(|d| panic!("{d:?}"));
    let v = &a.state.functions["V"];
    assert_eq!(v.kind, DefKind::Arrow);
    assert_eq!(v.vector_dim(), Some(2));
    assert!(v.family.is_some());
    assert!(a.state.constants["N"].integer);
    assert_eq!(a.state.scenes[DEFAULT_SCENE].members, ["V"]);
    // Length-3 arrows go to the 3D scene; the index may sit in the start point only.
    let s = build(&[&def(&["W: family(arrow([0, 0, 1]; [k, 0, 0]); 0 <= k <= 3)"])]);
    assert_eq!(s.scenes["3D(x, y, z)"].members, ["W"]);
    assert_eq!(s.functions["W"].source(), "W: family(arrow([0, 0, 1]; [k, 0, 0]); 0 <= k <= 3)");
    // The vector and the start point still need the same length.
    assert_eq!(codes(&State::default(), &def(&["W: family(arrow([k, 1]; [0, 0, 0]); 0 <= k <= 3)"])), vec![code::SHAPE]);
    // Still a drawn definition: not a value.
    assert_eq!(codes(&s, &def(&["g(x) -> y: W_[1]"])), vec![code::TYPE]);
}

#[test]
fn family_errors() {
    let s = State::default();
    // Missing index range, bad range, index clashing with an input, input in the bound.
    assert_eq!(codes(&s, &def(&["F(x) -> y: family(x + k)"])), vec![code::BIG_OP_RANGE]);
    assert_eq!(codes(&s, &def(&["F(x) -> y: family(x + k; k < 3)"])), vec![code::BIG_OP_RANGE]);
    assert_eq!(codes(&s, &def(&["F(x) -> y: family(x; 0 <= x <= 3)"])), vec![code::NAME_CONFLICT]);
    assert!(codes(&s, &def(&["F(x) -> y: family(x + k; 0 <= k <= x)"])).contains(&code::BIG_OP_RANGE));
    // Only drawn functions repeat.
    assert_eq!(codes(&s, &def(&["v: family([k, 1]; 0 <= k <= 2)"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["S: family(span([k, 1]); 0 <= k <= 2)"])), vec![code::TYPE]);
    // A repeated definition is not a value.
    let s = build(&[&def(&["F(x) -> y: family(x + k; 0 <= k <= 3)"])]);
    assert_eq!(codes(&s, &def(&["g(x) -> y: F(x)"])), vec![code::TYPE]);
    assert_eq!(codes(&s, "H1 = highlight(F)\n"), vec![code::UNSUPPORTED]);
    // `family` is the repetition word, not a definition name.
    assert_eq!(codes(&State::default(), &def(&["family(x) -> y: x"])), vec![code::NAME_CONFLICT]);
}
