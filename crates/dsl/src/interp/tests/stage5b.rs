//! Stage 5b: named derivatives `f@n`, inverses `~f`, generated derivatives.

use super::*;
use crate::print::expr_dsl;

fn build_on(state: &State, src: &str) -> State {
    run(state, src).unwrap_or_else(|d| panic!("{src}: {d:?}")).state
}

#[test]
fn derivative_and_inverse_definitions() {
    let s = build(&[&def(&["f(x; a) -> y: a*x^3", "f@1(x; a) -> y: 3*a*x^2", "f@2(x; a) -> y: 6*a*x", "~f(x; a) -> y: (x/a)^(1/3)"])]);
    for k in ["f@1", "f@2", "~f"] {
        assert_eq!(s.functions[k].kind, DefKind::Curve);
        assert_eq!(s.functions[k].calls, Vec::<String>::new(), "{k} does not depend on f by reference");
    }
    assert_eq!(s.functions["f@2"].source(), "f@2(x; a) -> y: 6*a*x");
    // Nothing calls an undefined derivative: none generated.
    assert!(s.functions["f"].derivs.is_empty());
}

#[test]
fn derivative_definition_errors() {
    let s = build(&[&def(&["f(x) -> y: x^2", "p(t) -> (x, y): (t, t), 0 <= t <= 1"])]);
    assert_eq!(codes(&State::default(), &def(&["~f(x) -> y: x"])), vec![code::UNDECLARED]);
    let d = first_err(&s, &def(&["f@1(t) -> y: 2*t"]));
    assert_eq!(d.code, code::AXES);
    assert_eq!(d.suggestion.as_deref(), Some("f@1(x) -> y"));
    assert_eq!(codes(&s, &def(&["f@1(x) -> z: 2*x"])), vec![code::AXES]);
    assert_eq!(codes(&s, &def(&["p@1(t) -> (x, y): (1, 1), 0 <= t <= 1"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["~f@1(x) -> y: 1"])), vec![code::NAME_CONFLICT]);
    assert_eq!(codes(&s, &def(&["f@9(x) -> y: 0"])), vec![code::LIMIT]);
    // f defined in the same request first is fine.
    build(&[&def(&["g(x) -> y: x", "g@1(x) -> y: 1"])]);
}

#[test]
fn del_base_cascades_to_named_derivatives() {
    let s = build(&[&def(&["f(x) -> y: x^2", "f@1(x) -> y: 2*x", "~f(x) -> y: sqrt(x)", "g(x) -> y: x"])]);
    let a = run(&s, "del f").unwrap();
    assert!(!a.state.functions.contains_key("f@1") && !a.state.functions.contains_key("~f"));
    assert!(a.state.functions.contains_key("g"));
    assert!(a.notes.contains(&Note::FunctionDeleted { key: "f@1".into(), via: Some("f".into()) }));
    // Deleting only the derivative keeps f.
    let a = run(&s, "del f@1").unwrap();
    assert!(a.state.functions.contains_key("f"));
}

#[test]
fn base_changed_to_other_axes_rejected() {
    let s = build(&[&def(&["f(x) -> y: x^2", "f@1(x) -> y: 2*x"])]);
    assert_eq!(codes(&s, &def(&["f(t) -> y: t^2"])), vec![code::TYPE]);
}

#[test]
fn undefined_derivative_call_is_generated() {
    let a = run(&State::default(), &def(&["f(x) -> y: x^3 - 3*x", "T(x; c) -> y: f@1(c)*(x - c) + f(c)"])).unwrap();
    let f = &a.state.functions["f"];
    assert_eq!(f.derivs.iter().map(expr_dsl).collect::<Vec<_>>(), ["3*x^2 - 3"]);
    assert_eq!(a.state.functions["T"].calls, ["f"]);
    assert!(a.notes.contains(&Note::DerivativeGenerated { function: "f".into(), order: 1, src: "3*x^2 - 3".into() }));
    // Generated nodes are typed and numbered like parsed ones.
    assert_eq!(f.derivs[0].id, 0);
    f.derivs[0].walk(&mut |e| assert_ne!(e.ty, Ty::Unknown));

    // Same request again: no new note.
    let b = apply(&a.state, &def(&["T(x; c) -> y: f@1(c)*(x - c) + f(c)"]), &Limits::default(), 2).unwrap();
    assert!(!b.notes.iter().any(|n| matches!(n, Note::DerivativeGenerated { .. })));

    // Claude's f@1 replaces it: nothing left to generate.
    let c = apply(&a.state, &def(&["f@1(x) -> y: 3*x^2 - 3"]), &Limits::default(), 3).unwrap();
    assert!(c.state.functions["f"].derivs.is_empty());
    // f changes: the generated derivative follows.
    let d = apply(&a.state, &def(&["f(x) -> y: x^2"]), &Limits::default(), 4).unwrap();
    assert_eq!(expr_dsl(&d.state.functions["f"].derivs[0]), "2*x");
    assert!(d.notes.contains(&Note::DerivativeGenerated { function: "f".into(), order: 1, src: "2*x".into() }));
}

#[test]
fn generated_derivatives_close_over_callees() {
    let s = build(&[&def(&["g(x) -> y: sin(x)", "f(x) -> y: g(2*x)", "T(x) -> y: f@2(x)"])]);
    assert_eq!(s.functions["f"].derivs.iter().map(expr_dsl).collect::<Vec<_>>(), ["2*g@1(2*x)", "4*g@2(2*x)"]);
    assert_eq!(s.functions["g"].derivs.iter().map(expr_dsl).collect::<Vec<_>>(), ["cos(x)", "-sin(x)"]);
    // Claude's g@1 is used for order 1; order 2 is still generated from g.
    let s2 = build_on(&s, &def(&["g@1(x) -> y: cos(x)"]));
    assert_eq!(s2.functions["g"].derivs.len(), 2);
}

#[test]
fn inverse_calls() {
    let s = State::default();
    assert_eq!(codes(&s, &def(&["f(x) -> y: x^2", "g(x) -> y: ~f(x)"])), vec![code::UNDECLARED]);
    let s = build(&[&def(&["f(x) -> y: x^3", "~f(x) -> y: x^(1/3)", "g(x) -> y: ~f(x) + 1", "T(x) -> y: g@1(x)"])]);
    assert_eq!(expr_dsl(&s.functions["g"].derivs[0]), "1/f@1(~f(x))");
    assert_eq!(expr_dsl(&s.functions["f"].derivs[0]), "3*x^2");
    assert_eq!(s.functions["g"].calls, ["~f"]);
}

#[test]
fn derivative_call_errors() {
    let s = build(&[&def(&["f(x; a) -> y: a*x^2", "p(t) -> (x, y): (t, t), 0 <= t <= 1", "G(x) -> y: gamma(x)"])]);
    assert_eq!(codes(&s, &def(&["T(x) -> y: f@1(x)"])), vec![code::MISSING_CONST]);
    build_on(&s, &def(&["T(x; a) -> y: f@1(x)"]));
    assert_eq!(codes(&s, &def(&["h(x) -> y: h@1(x)"])), vec![code::CYCLE]);
    assert_eq!(codes(&s, &def(&["T(x) -> y: q@1(x)"])), vec![code::UNDECLARED]);
    assert_eq!(codes(&s, &def(&["T(x) -> y: p@1(x)"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["T(x; a) -> y: f@1(x, x)"])), vec![code::ARITY]);
    assert_eq!(codes(&s, &def(&["T(x; a) -> y: f@1"])), vec![code::NOT_VALUE]);
    let src = def(&["T(x) -> y: G@1(x) + 1"]);
    let d = first_err(&s, &src);
    assert_eq!(d.code, code::DERIV);
    assert!(d.message.contains("gamma"), "{}", d.message);
    let col = LineIndex::new(&src).line_col(d.span.start).1;
    assert_eq!(col, 16, "points at the call");
    // Claude's own G@1 needs nothing generated.
    build_on(&s, &def(&["G@1(x) -> y: 1", "T(x) -> y: G@1(x) + 1"]));
}

#[test]
fn derivative_in_highlight_range() {
    let s = build(&[&def(&["f(x) -> y: x^3 - 3*x"]), "H1 = highlight(f; f@1(x) < 0)"]);
    assert_eq!(s.functions["f"].derivs.len(), 1);
}

// ----- feature points ---------------------------------------------------

fn spec(s: &State, k: &str) -> FeatureSpec {
    s.functions[k].feature.clone().unwrap_or_else(|| panic!("{k} has no feature spec"))
}

#[test]
fn feature_definitions_resolve() {
    let s = build(&[&def(&[
        "f(x) -> y: x^3 - 3*x",
        "g(x) -> y: x",
        "E: extrema(f)",
        "M: extrema(f;; max)",
        "R: roots(f; -1 < x < 2)",
        "P: intersect(f, g; 0 < x < 3)",
        "Q: intersect(f, y = 2*x)",
        "I: inflections(f)",
        "Y: yint(f)",
        "A: argmax(f; -2 <= x <= 2)",
    ])]);
    let e = &s.functions["E"];
    assert_eq!((e.kind, &e.ty, &e.calls), (DefKind::Feature, &Ty::Vector { n: 2 }, &vec!["f".to_string()]));
    assert_eq!(e.source(), "E: extrema(f)");
    assert_eq!(spec(&s, "E").kind, FeatureKind::Extrema);
    assert_eq!(spec(&s, "M").filter.as_deref(), Some("max"));
    let r = spec(&s, "R");
    assert_eq!((r.lo.as_ref().map(expr_dsl), r.hi.as_ref().map(expr_dsl)), (Some("-1".into()), Some("2".into())));
    assert_eq!(expr_dsl(r.range.as_ref().unwrap()), "-1 < x < 2");
    assert_eq!(spec(&s, "P").other.as_ref().map(expr_dsl).as_deref(), Some("g(x)"));
    assert_eq!(s.functions["P"].calls, ["f", "g"]);
    assert_eq!(spec(&s, "Q").other.as_ref().map(expr_dsl).as_deref(), Some("2*x"));
    assert_eq!(spec(&s, "Q").axes.iter().map(|a| a.key.as_str()).collect::<Vec<_>>(), ["x", "y"]);
    // Placed in the target's scene like the target.
    assert!(s.scenes[DEFAULT_SCENE].members.contains(&"E".to_string()));
    // Derivatives generated for the kinds that need them: f'' for inflection points.
    assert_eq!(s.functions["f"].derivs.iter().map(expr_dsl).collect::<Vec<_>>(), ["3*x^2 - 3", "6*x"]);
}

#[test]
fn feature_needs_only_missing_orders() {
    let s = build(&[&def(&["f(x) -> y: x^4", "f@1(x) -> y: 4*x^3", "E: extrema(f)"])]);
    assert!(s.functions["f"].derivs.is_empty(), "Claude's f@1 covers extrema");
    let s = build_on(&s, &def(&["I: inflections(f)"]));
    assert_eq!(s.functions["f"].derivs.len(), 2);
    // Roots and breaks need no derivative.
    let s = build(&[&def(&["f(x) -> y: x^4", "R: roots(f)", "B: breaks(f)"])]);
    assert!(s.functions["f"].derivs.is_empty());
}

#[test]
fn feature_constants_follow_the_rule() {
    let s = build(&[&def(&["f(x; a) -> y: a*x^2 - 1"])]);
    let d = first_err(&s, &def(&["E: extrema(f)"]));
    assert_eq!(d.code, code::MISSING_CONST);
    assert_eq!(d.suggestion.as_deref(), Some("E(; a)"));
    build_on(&s, &def(&["E(; a): extrema(f)"]));
    // Constants in the range slot too.
    let s = build(&[&def(&["g(x) -> y: x^2 - 1", "h(x; c) -> y: c*x"])]);
    let d = first_err(&s, &def(&["R: roots(g; -c < x < c)"]));
    assert_eq!((d.code, d.suggestion.as_deref()), (code::MISSING_CONST, Some("R(; c)")));
    build_on(&s, &def(&["R(; c): roots(g; -c < x < c)", "Q(; c): intersect(g, y = c*x)"]));
}

#[test]
fn feature_errors() {
    let s = build(&[&def(&["f(x) -> y: x^2", "u(t) -> y: t", "p(t) -> (x, y): (t, t^2), 0 <= t <= 1", "C(x, y): x^2 + y^2 = 1", "h(\\theta) -> polar(r): 1"])]);
    let c = |src: &str| codes(&s, &def(&[src]));
    assert_eq!(c("E: extrema(q)"), vec![code::UNDECLARED]);
    assert_eq!(c("E: extrema(p)"), vec![code::TYPE]);
    assert_eq!(first_err(&s, &def(&["E: extrema(p)"])).suggestion.as_deref(), Some("tangents(p)"));
    assert_eq!(c("E: roots(C)"), vec![code::TYPE]);
    assert_eq!(c("E: tangents(f)"), vec![code::TYPE]);
    assert_eq!(c("E: extrema(f, f)"), vec![code::ARITY]);
    assert_eq!(c("E: intersect(f)"), vec![code::ARITY]);
    assert_eq!(c("E: intersect(f, u)"), vec![code::AXES]);
    assert_eq!(c("E: intersect(f, z = x)"), vec![code::AXES]);
    assert_eq!(c("E: intersect(f, 2*x)"), vec![code::TARGET]);
    assert_eq!(c("E: extrema(2*f)"), vec![code::TARGET]);
    assert_eq!(c("E: extrema(f;; hole)"), vec![code::FEATURE]);
    assert_eq!(c("E: roots(f;; max)"), vec![code::FEATURE]);
    assert_eq!(c("E: roots(f; 2*x)"), vec![code::RANGE]);
    assert_eq!(c("E: roots(f; y > 0)"), vec![code::UNDECLARED]);
    assert_eq!(c("E: argmax(f)"), vec![code::RANGE]);
    assert_eq!(c("E: argmax(f; x > 0)"), vec![code::RANGE]);
    assert_eq!(c("E(x) -> y: extrema(f)"), vec![code::TYPE]);
    assert_eq!(c("g(x) -> y: extrema(f) + 1"), vec![code::TYPE]);
    assert_eq!(c("roots(x) -> y: x"), vec![code::NAME_CONFLICT]);
    // tangents takes a parametric or polar function.
    build_on(&s, &def(&["T: tangents(p)", "H: tangents(h;; horizontal)"]));
}

#[test]
fn scalar_value_definition_suggestion() {
    let s = build(&[&def(&["g(x; a) -> y: a*sin(x)", "A(; a): argmax(g; 0 <= x <= 3)"])]);
    // A feature coordinate is not a slider default (numbers only): suggest a function of x
    // with the constants its callees use.
    let errs = run(&s, &def(&["V: A_[2] + 1"])).unwrap_err();
    let d = errs.iter().find(|d| d.code == code::TYPE).unwrap();
    assert_eq!(d.suggestion.as_deref(), Some("V(x; a) -> y: A_[2] + 1"));
    let d = first_err(&s, &def(&["V(; a): A_[1]"]));
    assert_eq!((d.code, d.suggestion.as_deref()), (code::TYPE, Some("V(x; a) -> y: A_[1]")));
    // A plain number still points to a constant.
    let d = first_err(&s, &def(&["V: -2"]));
    assert_eq!((d.code, d.suggestion.as_deref()), (code::TYPE, Some("constant:\n    V: default=-2")));
}

#[test]
fn feature_point_as_value() {
    let s = build(&[&def(&["f(x) -> y: x^3 - 3*x", "P: extrema(f;; min)", "T(x) -> y: f@1(P_[1])*(x - P_[1]) + P_[2]", "V: arrow([1, 0]; P)"])]);
    let t = &s.functions["T"];
    assert_eq!(t.calls, ["P", "f"]);
    let mut kinds = vec![];
    t.body.walk(&mut |e| {
        if let ExprKind::Name { name, kind } = &e.kind
            && name.key == "P"
        {
            kinds.push((*kind, e.ty.clone()));
        }
    });
    assert!(kinds.iter().all(|k| *k == (NameKind::Value, Ty::Vector { n: 2 })), "{kinds:?}");
    // Calling it, or a cycle through it, is an error.
    assert_eq!(codes(&s, &def(&["g(x) -> y: P(x)"])), vec![code::NOT_FUNCTION]);
    assert_eq!(codes(&s, &def(&["f(x) -> y: x - P_[1]"])), vec![code::CYCLE]);
    // del f takes P and everything using P.
    let a = run(&s, "del f").unwrap();
    assert!(a.state.functions.is_empty());
}

#[test]
fn break_candidates_from_the_ast() {
    let s = build(&[&def(&[
        "g(x) -> y: 1/(x - 2)",
        "f(x) -> y: x/(x^2 - 1) + sqrt(x + 3) + abs(x) + g(2*x) + tan(x) + x^(-2)",
        "B: breaks(f)",
    ])]);
    let c: Vec<(BreakCause, String)> = spec(&s, "B").candidates.iter().map(|c| (c.cause, expr_dsl(&c.expr))).collect();
    assert_eq!(
        c,
        vec![
            (BreakCause::Denominator, "x^2 - 1".into()),
            (BreakCause::Domain, "x + 3".into()),
            (BreakCause::Abs, "x".into()),
            (BreakCause::Denominator, "(2*x) - 2".into()),
            (BreakCause::Denominator, "cos(x)".into()),
            (BreakCause::Denominator, "x".into()),
        ]
    );
    // split boundaries; the split name stands for its subject.
    let src = "define:\n    h(x) -> y:\n        split r = x - 1 at 0, 2\n        1/r, r < 0\n        r, 0 <= r < 2\n        0, r >= 2\n    B: breaks(h)\n";
    let s = build(&[src]);
    let c: Vec<(BreakCause, String)> = spec(&s, "B").candidates.iter().map(|c| (c.cause, expr_dsl(&c.expr))).collect();
    assert_eq!(c, vec![(BreakCause::Boundary, "(x - 1) - 0".into()), (BreakCause::Boundary, "(x - 1) - 2".into()), (BreakCause::Denominator, "x - 1".into())]);
    // The target changes: candidates follow.
    let s2 = build_on(&s, &def(&["h(x) -> y: ln(x)"]));
    assert_eq!(spec(&s2, "B").candidates.iter().map(|c| expr_dsl(&c.expr)).collect::<Vec<_>>(), ["x"]);
}

#[test]
fn feature_target_changed_kind_is_rejected() {
    let s = build(&[&def(&["f(x) -> y: x^2", "E: extrema(f)"])]);
    assert_eq!(codes(&s, &def(&["f(t) -> (x, y): (t, t), 0 <= t <= 1"])), vec![code::TYPE]);
}
