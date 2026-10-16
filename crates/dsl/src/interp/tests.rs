use super::*;
use crate::span::LineIndex;

mod stage2;
mod stage3;
mod stage4;

fn run(state: &State, src: &str) -> Result<Applied, Vec<Diagnostic>> {
    apply(state, src, &Limits::default(), 1)
}

/// Applies a sequence of requests, each must succeed; seq numbers 1, 2, ...
fn build(reqs: &[&str]) -> State {
    let mut s = State::default();
    for (i, r) in reqs.iter().enumerate() {
        s = apply(&s, r, &Limits::default(), i as u64 + 1).unwrap_or_else(|d| panic!("{r}: {d:?}")).state;
    }
    s
}

fn codes(state: &State, src: &str) -> Vec<&'static str> {
    run(state, src).unwrap_err().into_iter().map(|d| d.code).collect()
}

fn first_err(state: &State, src: &str) -> Diagnostic {
    run(state, src).unwrap_err().remove(0)
}

fn def(lines: &[&str]) -> String {
    let mut s = "define:\n".to_string();
    for l in lines {
        s.push_str("    ");
        s.push_str(l);
        s.push('\n');
    }
    s
}

#[test]
fn define_creates_function_and_auto_constant() {
    let a = run(&State::default(), &def(&["f(x; a) -> y: a*x^2"])).unwrap();
    let f = &a.state.functions["f"];
    assert_eq!(f.consts, ["a"]);
    assert_eq!(f.body_src, "a*x^2");
    assert_eq!(f.source(), "f(x; a) -> y: a*x^2");
    let c = &a.state.constants["a"];
    assert_eq!((c.value, c.min, c.max, c.integer), (1.0, -10.0, 10.0, false));
    assert_eq!(a.notes, vec![Note::FunctionCreated { key: "f".into() }, Note::ConstantCreated { key: "a".into() }, Note::DefaultSceneCreated { key: "2D(x, y)".into() }]);
    assert!(a.changes.iter().any(|c| matches!(c, Change::PutFunction { function } if function.key == "f" && function.version == 1)));
    assert!(a.changes.iter().any(|c| matches!(c, Change::PutConstant { constant } if constant.key == "a")));
}

#[test]
fn names_are_resolved_and_typed() {
    let s = build(&[&def(&["f(x; a) -> y: a*sin(x) + \\pi"])]);
    let body = &s.functions["f"].body;
    let mut kinds = vec![];
    body.walk(&mut |e| {
        assert_ne!(e.ty, Ty::Unknown);
        assert_eq!(e.span, Span::default(), "spans must not be stored");
        match &e.kind {
            ExprKind::Name { name, kind } => kinds.push((name.key.clone(), *kind)),
            ExprKind::Call { callee, kind, .. } => kinds.push((callee.key.clone(), *kind)),
            _ => {}
        }
    });
    assert_eq!(
        kinds,
        vec![("a".into(), NameKind::Constant), ("sin".into(), NameKind::Builtin), ("x".into(), NameKind::Input), ("\\pi".into(), NameKind::Reserved)]
    );
    assert_eq!(body.id, 0);
}

#[test]
fn reassignment_modifies_and_keeps_color() {
    let s1 = build(&[&def(&["f(x) -> y: x", "g(x) -> y: 2*x"])]);
    let a = apply(&s1, &def(&["f(x) -> y: x^2"]), &Limits::default(), 2).unwrap();
    assert_eq!(a.notes, vec![Note::FunctionModified { key: "f".into() }]);
    assert_eq!(a.state.functions["f"].color, s1.functions["f"].color);
    assert_eq!(a.state.functions["f"].version, 2);
    assert_eq!(a.state.functions["g"].version, 1, "untouched objects keep their version");
    assert_eq!(a.changes.len(), 1);
    // Same text again: unchanged, no changes.
    let b = apply(&a.state, &def(&["f(x) -> y: x^2"]), &Limits::default(), 3).unwrap();
    assert_eq!(b.notes, vec![Note::FunctionUnchanged { key: "f".into() }]);
    assert!(b.changes.is_empty());
}

#[test]
fn colors_are_assigned_at_creation() {
    let s = build(&[&def(&["f(x) -> y: x", "g(x) -> y: x"]), "del f\n", &def(&["h(x) -> y: x"])]);
    assert_eq!(s.functions["g"].color, 1);
    assert_eq!(s.functions["h"].color, 2, "deleting f must not shift colors");
}

#[test]
fn undeclared_letter_suggests_signature() {
    let d = first_err(&State::default(), &def(&["f(x) -> y: a*x"]));
    assert_eq!(d.code, code::UNDECLARED);
    assert_eq!(d.suggestion.as_deref(), Some("f(x; a)"));
    let d = first_err(&State::default(), &def(&["f(x; b) -> y: b*x + c"]));
    assert_eq!(d.suggestion.as_deref(), Some("f(x; b, c)"));
}

#[test]
fn juxtaposed_known_letters_suggest_product() {
    let d = first_err(&State::default(), &def(&["f(x; a) -> y: ax"]));
    assert_eq!(d.code, code::UNDECLARED);
    assert_eq!(d.suggestion.as_deref(), Some("a*x"));
}

#[test]
fn undeclared_name_reported_once_per_definition() {
    assert_eq!(codes(&State::default(), &def(&["f(x) -> y: a*x + a"])), vec![code::UNDECLARED]);
}

#[test]
fn referenced_function_constants_must_be_declared() {
    let s = build(&[&def(&["f(x; a) -> y: a*x"])]);
    let d = first_err(&s, &def(&["g(x) -> y: f(x) + 1"]));
    assert_eq!(d.code, code::MISSING_CONST);
    assert_eq!(d.suggestion.as_deref(), Some("g(x; a)"));
    // Transitive: h calls g which calls f.
    let s = build(&[&def(&["f(x; a) -> y: a*x", "g(x; a) -> y: f(x)"])]);
    assert_eq!(first_err(&s, &def(&["h(x) -> y: g(x)"])).suggestion.as_deref(), Some("h(x; a)"));
}

#[test]
fn constant_added_to_callee_propagates_to_callers() {
    let s = build(&[&def(&["f(x; a) -> y: a*x", "g(x; a) -> y: f(x)", "h(x; a) -> y: g(x) + 1"])]);
    let a = apply(&s, &def(&["f(x; a, b) -> y: a*x + b"]), &Limits::default(), 2).unwrap();
    assert_eq!(a.state.functions["g"].consts, ["a", "b"]);
    assert_eq!(a.state.functions["h"].consts, ["a", "b"]);
    assert!(a.notes.contains(&Note::SignatureConstAdded { function: "g".into(), constant: "b".into(), via: "f".into() }));
    assert!(a.notes.contains(&Note::SignatureConstAdded { function: "h".into(), constant: "b".into(), via: "f".into() }));
    assert!(a.changes.iter().any(|c| matches!(c, Change::PutFunction { function } if function.key == "h")));
}

#[test]
fn constant_removed_from_callee_is_removed_only_where_unused() {
    let s = build(&[&def(&["f(x; a, b) -> y: a*x + b", "g(x; a, b) -> y: f(x) + a", "k(x; a, b) -> y: f(x) + b"])]);
    let a = apply(&s, &def(&["f(x; a) -> y: a*x"]), &Limits::default(), 2).unwrap();
    // g does not use b itself → removed. k uses b directly → kept.
    assert_eq!(a.state.functions["g"].consts, ["a"]);
    assert_eq!(a.state.functions["k"].consts, ["a", "b"]);
    assert!(a.notes.contains(&Note::SignatureConstRemoved { function: "g".into(), constant: "b".into(), via: "f".into() }));
    assert!(a.state.constants.contains_key("b"));
}

#[test]
fn del_cascades_through_function_references() {
    let s = build(&[&def(&["f(x; a) -> y: a*x", "g(x; a) -> y: f(x)", "h(x; a) -> y: g(x)", "u(x) -> y: x"])]);
    let a = apply(&s, "del f\n", &Limits::default(), 2).unwrap();
    assert_eq!(a.state.functions.keys().collect::<Vec<_>>(), ["u"]);
    assert_eq!(
        a.notes,
        vec![
            Note::FunctionDeleted { key: "f".into(), via: None },
            Note::FunctionDeleted { key: "g".into(), via: Some("f".into()) },
            Note::FunctionDeleted { key: "h".into(), via: Some("g".into()) },
            Note::ConstantRemoved { key: "a".into() },
        ]
    );
    // The applied result represents every deletion, cascaded ones included.
    let removed: Vec<_> = a.changes.iter().filter_map(|c| if let Change::RemoveFunction { key } = c { Some(key.as_str()) } else { None }).collect();
    assert_eq!(removed, ["f", "g", "h"]);
    assert!(a.changes.contains(&Change::RemoveConstant { key: "a".into() }));
}

#[test]
fn applied_result_replays_to_the_same_state() {
    let mut log = vec![];
    let mut live = State::default();
    let reqs = [
        def(&["f(x; a) -> y: a*x", "g(x; a) -> y: f(x)"]),
        def(&["f(x; a, b) -> y: a*x + b"]),
        "del f\n".to_string(),
        def(&["f(x; n) -> y: sum(k*x; 1 <= k <= n)"]),
    ];
    for (i, r) in reqs.iter().enumerate() {
        let a = apply(&live, r, &Limits::default(), i as u64 + 1).unwrap();
        // Stored as JSON in the log.
        log.push(serde_json::to_string(&a.changes).unwrap());
        live = a.state;
    }
    let mut replayed = State::default();
    for entry in &log {
        let changes: Vec<Change> = serde_json::from_str(entry).unwrap();
        replayed.apply_changes(&changes);
    }
    assert_eq!(replayed, live);
}

#[test]
fn del_errors() {
    let s = build(&[&def(&["f(x; a) -> y: a*x"])]);
    assert_eq!(codes(&s, "del q\n"), vec![code::UNDECLARED]);
    assert_eq!(codes(&s, "del a\n"), vec![code::DEL_CONSTANT]);
}

#[test]
fn del_then_redefine_in_one_request() {
    let s = build(&[&def(&["f(x) -> y: x", "g(x) -> y: f(x)"])]);
    let a = run(&s, &format!("del f\n{}", def(&["f(x) -> y: 2*x"]))).unwrap();
    assert!(a.state.functions.contains_key("f"));
    assert!(!a.state.functions.contains_key("g"));
}

#[test]
fn request_is_atomic() {
    let s = build(&[&def(&["f(x) -> y: x"])]);
    let r = run(&s, &def(&["g(x) -> y: x", "h(x) -> y: q*x"]));
    assert!(r.is_err());
    // Caller keeps `s`; nothing to roll back because apply never mutates the input.
    assert_eq!(s.functions.len(), 1);
}

#[test]
fn all_independent_errors_in_one_response() {
    let src = "define:\n    f(x) -> y: 2x\n    g(x) -> y: a*x\n    h(t) -> t: t\n    k(x) -> y: sin(x, 1)\ndel zz\n";
    let d = run(&State::default(), src).unwrap_err();
    assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), vec![code::IMPLICIT_MUL, code::UNDECLARED, code::AXES, code::ARITY, code::UNDECLARED]);
    let idx = LineIndex::new(src);
    assert_eq!(d[1].render(&idx), "3:16 [E-undeclared] 'a'가 선언되지 않음 → g(x; a)");
}

#[test]
fn failed_definition_does_not_cascade_errors() {
    // f fails to parse; g referring to f gets no extra error.
    assert_eq!(codes(&State::default(), &def(&["f(x) -> y: x +", "g(x) -> y: f(x)"])), vec![code::SYNTAX]);
    // f fails to resolve; same.
    assert_eq!(codes(&State::default(), &def(&["f(x) -> y: q*x", "g(x) -> y: f(x)"])), vec![code::UNDECLARED]);
}

#[test]
fn cycles_are_rejected() {
    assert_eq!(codes(&State::default(), &def(&["f(x) -> y: f(x - 1)"])), vec![code::CYCLE]);
    let s = build(&[&def(&["f(x) -> y: x", "g(x) -> y: f(x)"])]);
    assert_eq!(codes(&s, &def(&["f(x) -> y: g(x)"])), vec![code::CYCLE]);
}

#[test]
fn stage_shape_errors() {
    let s = State::default();
    assert_eq!(codes(&s, &def(&["f(t) -> t: t"])), vec![code::AXES]);
    assert_eq!(codes(&s, &def(&["f(\\pi) -> y: 1"])), vec![code::AXES]);
    assert_eq!(codes(&s, &def(&["g(x, y) -> z: x"])), vec![code::UNSUPPORTED]);
    assert_eq!(codes(&s, &def(&["p(t) -> (x, y): (cos(t), sin(t)), 0 <= t <= 1"])), vec![code::UNSUPPORTED]);
    assert_eq!(codes(&s, &def(&["C(x, y): x^2 + y^2 = 1"])), vec![code::UNSUPPORTED]);
    assert_eq!(codes(&s, &def(&["f@2(x) -> y: 2"])), vec![code::UNSUPPORTED]);
    assert_eq!(codes(&s, &def(&["f(x) -> y: [1, 2]"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["f(x) -> y: \"text\""])), vec![code::TYPE]);
    let d = first_err(&s, &def(&["f(x): x^2"]));
    assert_eq!(d.code, code::ARROW);
    assert_eq!(d.suggestion.as_deref(), Some("f(x) -> y: x^2"));
}

#[test]
fn name_conflicts() {
    let s = build(&[&def(&["f(x; a) -> y: a*x"])]);
    assert_eq!(codes(&s, &def(&["sin(x) -> y: x"])), vec![code::NAME_CONFLICT]);
    assert_eq!(codes(&s, &def(&["a(x) -> y: x"])), vec![code::NAME_CONFLICT]);
    assert_eq!(codes(&s, &def(&["g(x; f) -> y: f"])), vec![code::NAME_CONFLICT]);
    assert_eq!(codes(&s, &def(&["g(x; y) -> y: x"])), vec![code::NAME_CONFLICT]);
    assert_eq!(codes(&s, &def(&["g(x; b, b) -> y: b"])), vec![code::DUPLICATE]);
    assert_eq!(codes(&s, &def(&["g(x; \\pi) -> y: x"])), vec![code::NAME_CONFLICT]);
}

#[test]
fn sum_bounds_make_integer_constants() {
    let s = build(&[&def(&["f(x; n) -> y: sum(x^k/k!; 0 <= k <= n)"])]);
    let n = &s.constants["n"];
    assert!(n.integer);
    assert_eq!((n.value, n.step), (5.0, 1.0));
    let mut index_seen = false;
    s.functions["f"].body.walk(&mut |e| {
        if let ExprKind::Name { name, kind: NameKind::Index } = &e.kind {
            index_seen |= name.key == "k";
        }
    });
    assert!(index_seen);
    // An existing real constant becomes integer when used in a bound.
    let s = build(&[&def(&["f(x; n) -> y: n*x"]), &def(&["g(x; n) -> y: sum(k; 1 <= k <= n)"])]);
    assert!(s.constants["n"].integer);
}

#[test]
fn sum_bound_may_use_the_input_variable() {
    build(&[&def(&["f(x) -> y: sum(1/k; 1 <= k <= x)"])]);
}

#[test]
fn sum_index_must_not_clash() {
    assert_eq!(codes(&State::default(), &def(&["f(x; a) -> y: sum(a; 1 <= a <= 3)"])), vec![code::NAME_CONFLICT]);
}

#[test]
fn type_errors() {
    let s = build(&[&def(&["f(x) -> y: x"])]);
    assert_eq!(codes(&s, &def(&["g(x) -> y: x < 1"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["g(x) -> y: (x < 1) + 1"])), vec![code::TYPE]);
    let d = first_err(&s, &def(&["g(x) -> y: f + 1"]));
    assert_eq!((d.code, d.suggestion.as_deref()), (code::NOT_VALUE, Some("f(x)")));
    let d = first_err(&s, &def(&["g(x; a) -> y: a(x + 1)"]));
    assert_eq!((d.code, d.suggestion.as_deref()), (code::NOT_FUNCTION, Some("a*(x + 1)")));
    assert_eq!(codes(&s, &def(&["g(x) -> y: q(x)"])), vec![code::UNDECLARED]);
    assert_eq!(codes(&s, &def(&["g(x) -> y: f(x, 1)"])), vec![code::ARITY]);
    let d = first_err(&s, &def(&["g(x) -> y: log(x)"]));
    assert_eq!(d.code, code::ARITY);
    assert_eq!(codes(&s, &def(&["g(x) -> y: x + y"])), vec![code::UNDECLARED]);
}

#[test]
fn reserved_values() {
    let s = State::default();
    build(&[&def(&["f(x) -> y: \\e^x + \\pi"])]);
    assert_eq!(codes(&s, &def(&["f(x) -> y: \\i*x"])), vec![code::UNSUPPORTED]);
    assert_eq!(codes(&s, &def(&["f(x) -> y: \\infty"])), vec![code::TYPE]);
    // A bad LaTeX name is reported once (by the parser), not again as undeclared.
    assert_eq!(codes(&s, &def(&["f(x) -> y: \\frac"])), vec![code::TEX]);
}

#[test]
fn greek_and_hangul_names() {
    let s = build(&[&def(&["넓이(x; \\alpha, a_{1}) -> y: \\alpha*x + a_{1}"])]);
    assert!(s.constants.contains_key("\\alpha"));
    assert!(s.constants.contains_key("a_{1}"));
    assert_eq!(s.functions["넓이"].signature(), "넓이(x; \\alpha, a_{1}) -> y");
}

#[test]
fn limits() {
    let tiny = Limits { fuel: 3, ..Limits::default() };
    let d = apply(&State::default(), &def(&["f(x; a) -> y: a*x^2 + 1"]), &tiny, 1).unwrap_err();
    assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), vec![code::LIMIT]);
    let small = Limits { max_source_bytes: 10, ..Limits::default() };
    assert_eq!(apply(&State::default(), &def(&["f(x) -> y: x"]), &small, 1).unwrap_err()[0].code, code::LIMIT);
    let few = Limits { max_functions: 1, ..Limits::default() };
    assert_eq!(apply(&State::default(), &def(&["f(x) -> y: x", "g(x) -> y: x"]), &few, 1).unwrap_err()[0].code, code::LIMIT);
}

#[test]
fn user_ops() {
    let s = build(&[&def(&["f(x; a, n) -> y: a*sum(k; 1 <= k <= n)"])]);
    let v = apply_user_op(&s, &UserOp::SetValue { constant: "a".into(), value: 99.0 }, 2).unwrap();
    assert_eq!(v.state.constants["a"].value, 10.0, "clamped to range");
    assert_eq!(v.changes.len(), 1);
    let v = apply_user_op(&s, &UserOp::SetValue { constant: "n".into(), value: 3.6 }, 2).unwrap();
    assert_eq!(v.state.constants["n"].value, 4.0, "integer constant rounds");
    assert!(apply_user_op(&s, &UserOp::SetRange { constant: "n".into(), min: 0.5, max: 10.0 }, 2).is_err());
    assert!(apply_user_op(&s, &UserOp::SetRange { constant: "a".into(), min: 2.0, max: 2.0 }, 2).is_err());
    let r = apply_user_op(&s, &UserOp::SetRange { constant: "a".into(), min: 2.0, max: 3.0 }, 2).unwrap();
    assert_eq!(r.state.constants["a"].value, 2.0, "value moves into the new range");
    let t = apply_user_op(&s, &UserOp::Toggle { scene: DEFAULT_SCENE.into(), target: "f".into(), visible: false }, 2).unwrap();
    assert_eq!(t.state.scenes[DEFAULT_SCENE].toggles, [("f".to_string(), false)].into());
    assert!(!t.state.scenes[DEFAULT_SCENE].visible("f", 0));
    // Deleting the last function of the system removes its default scene with the toggles.
    let d = apply(&t.state, "del f\n", &Limits::default(), 3).unwrap();
    assert!(d.state.scenes.is_empty());
    assert!(d.notes.contains(&Note::DefaultSceneRemoved { key: DEFAULT_SCENE.into() }));
    assert!(apply_user_op(&s, &UserOp::Toggle { scene: DEFAULT_SCENE.into(), target: "q".into(), visible: false }, 2).is_err());
}

#[test]
fn bare_reserved_and_greek_names_suggest_latex() {
    // docs/improvements.md: `pi` used to suggest adding a constant named pi.
    let d = first_err(&State::default(), &def(&["sq(x; n) -> y: 4/pi*sum(x^k; 1 <= k <= n)"]));
    assert_eq!((d.code, d.suggestion.as_deref()), (code::UNDECLARED, Some("\\pi")));
    let d = first_err(&State::default(), &def(&["f(x) -> y: sin(theta*x)"]));
    assert_eq!(d.suggestion.as_deref(), Some("\\theta"));
    let d = first_err(&State::default(), &def(&["f(x) -> y: e^x"]));
    assert_eq!(d.suggestion.as_deref(), Some("\\e"));
    // Declared as a constant it is just a constant.
    build(&[&def(&["f(x; alpha) -> y: alpha*x"])]);
}
