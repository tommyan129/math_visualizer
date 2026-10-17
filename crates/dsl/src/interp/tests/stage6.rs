//! Stage 6a: surfaces, space curves, parametric surfaces, 3D scenes.

use super::*;

const G: &str = "g(x, y) -> z: x^2 - y^2";

fn keys(s: &Scene) -> Vec<&str> {
    s.axes.iter().map(|a| a.key.as_str()).collect()
}

#[test]
fn surface_makes_the_3d_default_scene() {
    let s = build(&[&def(&[G])]);
    let g = &s.functions["g"];
    assert_eq!(g.kind, DefKind::Curve);
    assert!(g.is_surface() && g.is_3d());
    assert_eq!(g.curve_axes(), None);
    assert_eq!(g.signature(), "g(x, y) -> z");
    assert_eq!(s.function_system("g").as_deref(), Some("3D(x, y, z)"));
    let sc = &s.scenes["3D(x, y, z)"];
    assert_eq!(keys(sc), ["x", "y", "z"]);
    assert_eq!(sc.members, ["g"]);
    assert!(!s.scenes.contains_key(DEFAULT_SCENE));
}

#[test]
fn surface_is_callable_as_a_number() {
    let s = build(&[&def(&[G, "h(x) -> y: g(x, 1)"])]);
    assert_eq!(s.functions["h"].calls, ["g"]);
    assert!(s.scenes.contains_key(DEFAULT_SCENE) && s.scenes.contains_key("3D(x, y, z)"));
    assert_eq!(codes(&s, &def(&["k(x) -> y: g(x)"])), vec![code::ARITY]);
    // Cascade: deleting g removes h.
    let t = run(&s, "del g\n").unwrap().state;
    assert!(t.functions.is_empty() && !t.scenes.contains_key("3D(x, y, z)"));
}

#[test]
fn surface_head_errors() {
    let s = State::default();
    assert_eq!(codes(&s, &def(&["g(x, y) -> x: y"])), vec![code::AXES]);
    assert_eq!(first_err(&s, &def(&["g(x, y) -> x: y"])).suggestion.as_deref(), Some("g(x, y) -> z"));
    assert_eq!(codes(&s, &def(&["g(x, x) -> z: x"])), vec![code::DUPLICATE]);
    assert_eq!(codes(&s, &def(&["g(x, y) -> z: x, 0 <= x <= 1"])), vec![code::RANGE]);
    assert_eq!(codes(&s, &def(&["g(x, y, w) -> z: x"])), vec![code::AXES]);
}

#[test]
fn space_curve() {
    let s = build(&[&def(&["q(t; a) -> (x, y, z): (cos(t), sin(t), a*t), 0 <= t <= 4*\\pi"])]);
    let q = &s.functions["q"];
    assert_eq!(q.kind, DefKind::Parametric);
    assert!(q.is_3d() && q.range2.is_none());
    assert_eq!(s.function_system("q").as_deref(), Some("3D(x, y, z)"));
    assert_eq!(q.source(), "q(t; a) -> (x, y, z): (cos(t), sin(t), a*t), 0 <= t <= 4*\\pi");
    // A length-3 vector fills all three slots.
    build(&[&def(&["v(; a): [1, a, 0]", "q(t; a) -> (x, y, z): t*v, 0 <= t <= 1"])]);
    assert_eq!(codes(&State::default(), &def(&["q(t) -> (x, y, z): (t, t), 0 <= t <= 1"])), vec![code::SHAPE]);
}

#[test]
fn parametric_surface_ranges_by_name() {
    let src = "s(u, v) -> (x, y, z): (cos(u)*sin(v), sin(u)*sin(v), cos(v)), 0 <= v <= \\pi, 0 <= u <= 2*\\pi";
    let s = build(&[&def(&[src])]);
    let f = &s.functions["s"];
    assert!(f.is_3d());
    assert_eq!(f.range.as_ref().unwrap().src, "0 <= u <= 2*\\pi");
    assert_eq!(f.range2.as_ref().unwrap().src, "0 <= v <= \\pi");
    assert_eq!(s.function_system("s").as_deref(), Some("3D(x, y, z)"));
    // Re-printed in parameter order and re-applied unchanged.
    let again = run(&s, &def(&[&f.source()["".len()..]])).unwrap();
    assert_eq!(again.state.functions["s"], *f);
}

#[test]
fn parametric_surface_errors() {
    let s = State::default();
    let body = "(u, v, u*v)";
    // Missing the second range.
    let d = first_err(&s, &def(&[&format!("s(u, v) -> (x, y, z): {body}, 0 <= u <= 1")]));
    assert_eq!(d.code, code::RANGE);
    assert!(d.message.contains("v의 범위"), "{}", d.message);
    // Same parameter twice.
    assert_eq!(codes(&s, &def(&[&format!("s(u, v) -> (x, y, z): {body}, 0 <= u <= 1, 0 <= u <= 2")])), vec![code::RANGE]);
    // A bound using a parameter.
    assert_eq!(codes(&s, &def(&[&format!("s(u, v) -> (x, y, z): {body}, 0 <= u <= v, 0 <= v <= 1")])), vec![code::RANGE]);
    // Two ranges on a curve.
    assert_eq!(codes(&s, &def(&["q(t) -> (x, y, z): (t, t, t), 0 <= t <= 1, 0 <= t <= 2"])), vec![code::RANGE]);
    // A two-parameter function needs three outputs.
    assert_eq!(codes(&s, &def(&["s(u, v) -> (x, y): (u, v), 0 <= u <= 1, 0 <= v <= 1"])), vec![code::AXES]);
    // No ranges at all.
    let d = first_err(&s, &def(&[&format!("s(u, v) -> (x, y, z): {body}")]));
    assert_eq!(d.code, code::RANGE);
    assert!(d.suggestion.as_deref().unwrap().ends_with("0 <= u <= 2*\\pi, 0 <= v <= 2*\\pi"), "{:?}", d.suggestion);
}

#[test]
fn scene_3d_space_and_view_box() {
    let s = build(&[&def(&[G])]);
    let t = run(&s, "scene(1):\n    space: 3D(x, y, z)\n    show: g\n    view: -3 <= x <= 3 and -2 <= z <= 2\n").unwrap().state;
    let sc = &t.scenes["1"];
    assert_eq!(keys(sc), ["x", "y", "z"]);
    let v = sc.view.as_ref().unwrap();
    assert!(v.h.is_some() && v.v.is_none() && v.d.is_some());
    // Inferred from show.
    let t = run(&s, "scene(1):\n    show: g\n").unwrap().state;
    assert_eq!(keys(&t.scenes["1"]), ["x", "y", "z"]);
    // Clip over all three axes.
    run(&s, "scene(1):\n    show: g\n    clip: x^2 + y^2 + z^2 <= 4\n").unwrap();
    assert_eq!(codes(&s, "scene(1):\n    space: 3D(x, y, z)\n    view: -3 <= w <= 3\n"), vec![code::VIEW]);
    assert_eq!(codes(&s, "scene(1):\n    space: 3D(x, y)\n"), vec![code::SCENE]);
    assert_eq!(codes(&s, "scene(1):\n    space: 3D(x, y, x)\n"), vec![code::SCENE]);
    assert_eq!(codes(&s, "scene(1):\n    space: 2D(x, y)\n    show: g\n"), vec![code::PLACEMENT]);
    let d = first_err(&s, "scene(1):\n    space: 2D(x, y)\n    show: g\n");
    assert_eq!(d.suggestion.as_deref(), Some("space: 3D(x, y, z)"));
}

#[test]
fn spans_and_arrows_of_length_3() {
    let s = build(&[&def(&["S: span([1, 0, 0], [0, 1, 1])", "a: arrow([1, 2, 3])"])]);
    assert_eq!(s.scenes["3D(x, y, z)"].members, ["S", "a"]);
    assert!(!s.scenes.contains_key(DEFAULT_SCENE));
    let t = run(&s, "scene(1):\n    show: S, a\n").unwrap().state;
    assert_eq!(keys(&t.scenes["1"]), ["x", "y", "z"]);
    let t = run(&s, &def(&["q(t) -> (x, y, z): (t, t, t), 0 <= t <= 1"])).unwrap().state;
    assert_eq!(t.scenes["3D(x, y, z)"].members, ["S", "a", "q"]);
}

#[test]
fn surface_highlights() {
    let s = build(&[&def(&[G])]);
    let t = run(&s, "H1 = highlight(g; x^2 + y^2 < 1)\n").unwrap().state;
    let h = &t.highlights["H1"];
    assert_eq!(h.style.fill, Fill::Region);
    assert_eq!(h.axes, ["x", "y", "z"]);
    assert!(t.scenes["3D(x, y, z)"].members.contains(&"H1".to_string()));
    // The range is over the inputs.
    assert_eq!(codes(&s, "H1 = highlight(g; z < 1)\n"), vec![code::UNDECLARED]);
    // Sub-expression coloring works as for curves.
    run(&s, "H1 = highlight(g.(-y^2))\n").unwrap_or_else(|d| panic!("{d:?}"));
    // Targetless regions are not placed in a 3D scene.
    let t = run(&s, "H2 = highlight(; x < 1)\n").unwrap().state;
    assert!(!t.scenes["3D(x, y, z)"].members.contains(&"H2".to_string()));
    let d = first_err(&s, "H2 = highlight(; x < 1)\nscene(1):\n    show: g, H2\n");
    assert_eq!(d.code, code::PLACEMENT);
}

#[test]
fn one_variable_features_reject_surfaces() {
    let s = build(&[&def(&[G, "f(x) -> y: x"])]);
    assert_eq!(codes(&s, &def(&["E: extrema(g)"])), vec![code::UNSUPPORTED]);
    assert_eq!(codes(&s, &def(&["h(x, y) -> z: x", "C: calc(g, h)"])), vec![code::TYPE, code::TYPE]);
    assert_eq!(codes(&s, &def(&["g@1(x, y) -> z: 2*x"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["~g(x, y) -> z: x"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["k(x) -> y: g@1(x, 1)"])), vec![code::TYPE]);
    assert_eq!(codes(&s, "H1 = highlight(; between(g, f))\n"), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["E: intersect(f, g)"])), vec![code::TYPE]);
}

#[test]
fn family_of_surfaces() {
    let s = build(&[&def(&["g(x, y; n) -> z: family(k*x + y; 0 <= k <= n)"])]);
    assert!(s.functions["g"].is_3d());
}

#[test]
fn split_surface_and_req() {
    let src = "define:\n    g(x, y) -> z:\n        split x*y at 0\n        0, x*y < 0\n        x*y, x*y >= 0\n";
    let s = build(&[src]);
    assert!(s.functions["g"].is_3d());
    let t = run(&s, "req(g; x^2 + y^2 < 1; \"saddle?\")\n").unwrap().state;
    assert_eq!(t.requests.len(), 1);
}
