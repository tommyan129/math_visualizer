use super::*;

fn apply_ok(s: &State, src: &str) -> State {
    run(s, src).unwrap_or_else(|d| panic!("{src}: {d:?}")).state
}

fn base() -> State {
    build(&[
        &def(&["f(x; a) -> y: a*x^2", "g(t) -> y: sin(t)", "T(x; a) -> y: 2*a*x - a"]),
        "H1 = highlight(f; 0 <= x <= 1)\nH2 = highlight(f; 1 <= x <= 2)\nm = \"{f}의 넓이\"\n",
    ])
}

const SCENE: &str = "scene(1):\n    space: 2D(x, y)\n    show: f, H1\n    clip: -10 <= x <= 10\n    view: -5 <= x <= 5 and -3 <= y <= 3\n    step:\n        show: +T, -H1\n        set: a = 2\n        note: \"{x == 2}에서의 접선 $\\alpha$\"\n    step:\n        show: +H2\n        animate: a = 1 -> 3, 0.5s, loop\n        view: -1 <= x <= 1\n        note: + m\n";

#[test]
fn scene_with_steps_caches_every_step() {
    let a = run(&base(), SCENE).unwrap();
    assert!(a.notes.contains(&Note::SceneCreated { key: "1".into() }));
    let sc = &a.state.scenes["1"];
    assert_eq!(sc.step_count(), 2);
    assert_eq!(sc.members, ["f", "T", "H1", "H2"], "g is on 2D(t, y)");
    assert_eq!(sc.clip.as_ref().unwrap().src, "-10 <= x <= 10");
    let v = sc.view.as_ref().unwrap();
    assert!(v.h.is_some() && v.v.is_some());
    // Step 0: show list.
    let s0 = &sc.steps[0];
    assert_eq!(s0.visible, [("f".to_string(), true), ("H1".to_string(), true)].into());
    assert!(s0.note.is_empty());
    // Step 1: diff applied, note replaced.
    let s1 = &sc.steps[1];
    assert_eq!(s1.changed, ["T", "H1"]);
    assert_eq!((s1.visible["T"], s1.visible["H1"], s1.visible["f"]), (true, false, true));
    assert_eq!(s1.values, [("a".to_string(), 2.0)].into());
    let segs = &s1.note[0].segs;
    assert!(matches!(&segs[0], NoteSeg::Math { expr } if matches!(expr.kind, ExprKind::Compare { .. })));
    assert!(matches!(&segs[1], NoteSeg::Text { text } if text == "에서의 접선 "));
    assert!(matches!(&segs[2], NoteSeg::Tex { tex } if tex == "\\alpha"));
    // Step 2: animate end value is the step's value; note appended from a variable.
    let s2 = &sc.steps[2];
    assert_eq!(s2.animate, vec![Animate { constant: "a".into(), from: 1.0, to: 3.0, duration: 0.5, looped: true }]);
    assert_eq!(s2.values["a"], 3.0);
    assert!(s2.changed_view && !s1.changed_view);
    assert_eq!(s2.note.len(), 2);
    assert!(matches!(&s2.note[1].segs[0], NoteSeg::Def { function } if function == "f"));
    assert_eq!(s2.view.as_ref().unwrap().src, "-1 <= x <= 1");
}

#[test]
fn default_scenes_follow_coordinate_systems() {
    let s = base();
    assert_eq!(s.scenes.keys().collect::<Vec<_>>(), ["2D(t, y)", "2D(x, y)"]);
    let d = &s.scenes["2D(x, y)"];
    assert!(d.visible("f", 0) && !d.visible("H1", 0), "functions on, highlights off");
    assert_eq!(s.scenes["2D(t, y)"].members, ["g"]);
    // The last function of 2D(t, y) goes: its default scene goes, and the response says so.
    let a = run(&s, "del g\n").unwrap();
    assert!(!a.state.scenes.contains_key("2D(t, y)"));
    assert!(a.notes.contains(&Note::DefaultSceneRemoved { key: "2D(t, y)".into() }));
    // Defined again: a fresh default scene.
    let t = apply_user_op(&s, &UserOp::Toggle { scene: "2D(t, y)".into(), target: "g".into(), visible: false }, 9).unwrap().state;
    let t = apply_ok(&apply_ok(&t, "del g\n"), &def(&["g(t) -> y: t"]));
    assert!(t.scenes["2D(t, y)"].toggles.is_empty());
    assert!(codes(&s, "del scene(2D(x, y))\n") == vec![code::SCENE]);
    assert!(codes(&s, "scene(2D(x, y)):\n    show: f\n") == vec![code::SCENE]);
}

#[test]
fn placement_is_checked_by_axes() {
    let s = base();
    let d = first_err(&s, "scene(1):\n    space: 2D(x, y)\n    show: g\n");
    assert_eq!((d.code, d.suggestion.as_deref()), (code::PLACEMENT, Some("space: 2D(t, y)")));
    assert_eq!(codes(&s, "scene(1):\n    space: 2D(x, y)\n    step:\n        show: +g\n"), vec![code::PLACEMENT]);
    // space omitted: taken from the show targets when they agree.
    let t = apply_ok(&s, "scene(2):\n    show: g\n");
    assert_eq!(key_axes(&t.scenes["2"]), ["t", "y"]);
    assert_eq!(codes(&s, "scene(2):\n    show: f, g\n"), vec![code::SCENE]);
    // A targetless highlight fits where its axes are.
    let t = apply_ok(&s, "H3 = highlight(; 0 < t < 1)\nscene(3):\n    space: 2D(t, y)\n    show: H3\n");
    assert!(t.scenes["3"].members.contains(&"H3".to_string()));
    assert!(!t.scenes["2D(x, y)"].members.contains(&"H3".to_string()));
    assert_eq!(codes(&s, "H3 = highlight(; 0 < t < x)\n"), vec![code::PLACEMENT]);
    assert_eq!(codes(&s, "H3 = highlight(; between(f, g))\n"), vec![code::PLACEMENT]);
    assert_eq!(codes(&s, "scene(1):\n    space: 3D(x, y, z)\n"), vec![code::UNSUPPORTED]);
    assert_eq!(codes(&s, "scene(1):\n    space: 2D(x, x)\n"), vec![code::SCENE]);
}

fn key_axes(s: &Scene) -> Vec<&str> {
    s.axes.iter().map(|a| a.key.as_str()).collect()
}

#[test]
fn scene_field_errors() {
    let s = base();
    let c = |src: &str| codes(&s, &format!("scene(1):\n    space: 2D(x, y)\n{src}"));
    assert_eq!(c("    show: f, nope\n"), vec![code::UNDECLARED]);
    assert_eq!(c("    show: a\n"), vec![code::TARGET]);
    assert_eq!(c("    show: f\n    hide: f\n"), vec![code::SCENE]);
    assert_eq!(c("    show: f\n    show: T\n"), vec![code::DUPLICATE]);
    assert_eq!(c("    view: x < 1\n"), vec![code::VIEW]);
    assert_eq!(c("    view: -a <= x <= 1\n"), vec![code::VIEW]);
    assert_eq!(c("    view: 1 <= x <= -1\n"), vec![code::VIEW]);
    assert_eq!(c("    view: 0 <= x <= 1 and 2 <= x <= 3\n"), vec![code::VIEW]);
    assert_eq!(c("    clip: x + 1\n"), vec![code::RANGE]);
    // view and clip that never overlap (both literal): error; touching or partial: fine.
    assert_eq!(c("    clip: 0 <= x <= 1\n    view: 2 <= x <= 3\n"), vec![code::VIEW]);
    assert_eq!(c("    clip: 0 <= x <= 1\n    step:\n        view: 5 <= x <= 6\n"), vec![code::VIEW]);
    run(&s, "scene(1):\n    space: 2D(x, y)\n    clip: 0 <= x <= 1\n    view: 0.5 <= x <= 3\n").unwrap();
    run(&s, "scene(1):\n    space: 2D(x, y)\n    clip: 0 <= x <= a\n    view: 2 <= x <= 3\n").unwrap();
    // Step values: literal, in range, integer for integer constants.
    assert_eq!(c("    step:\n        set: a = 2*1\n"), vec![code::STEP]);
    assert_eq!(c("    step:\n        set: a = 99\n"), vec![code::STEP]);
    assert_eq!(c("    step:\n        set: zz = 1\n"), vec![code::UNDECLARED]);
    assert_eq!(c("    step:\n        animate: a = 0 -> 1, 0s\n"), vec![code::STEP]);
    assert_eq!(c("    step:\n        set: a = 1\n        animate: a = 0 -> 1\n"), vec![code::STEP]);
    assert_eq!(c("    step:\n        note: \"a\"\n        note: \"b\"\n"), vec![code::DUPLICATE]);
}

#[test]
fn note_parsing() {
    let s = base();
    let c = |note: &str| codes(&s, &format!("scene(1):\n    space: 2D(x, y)\n    step:\n        note: {note}\n"));
    // A typo inside {...} is an apply error, positioned inside the string.
    let src = "scene(1):\n    space: 2D(x, y)\n    step:\n        note: \"보기 {fx}\"\n";
    let d = first_err(&s, src);
    assert_eq!(d.code, code::UNDECLARED);
    assert_eq!(crate::span::LineIndex::new(src).line_col(d.span.start), (4, 20));
    assert_eq!(c("\"{2x}\""), vec![code::IMPLICIT_MUL]);
    assert_eq!(c("\"{f(x\""), vec![code::NOTE]);
    assert_eq!(c("\"$\\\\alpha\""), vec![code::NOTE]);
    assert_eq!(c("zz"), vec![code::NOTE]);
    assert_eq!(c("1 + 2"), vec![code::NOTE]);
    // Literal braces and dollars.
    let t = apply_ok(&s, "scene(1):\n    space: 2D(x, y)\n    step:\n        note: \"{{a} costs $$5 \\\"q\\\" {{b}}\"\n");
    assert_eq!(t.scenes["1"].steps[1].note[0].segs, vec![NoteSeg::Text { text: "{a} costs $5 \"q\" {b}".into() }]);
    // Nested braces (subscripts) inside {...}; a string variable is parsed where it is placed.
    let t = apply_ok(&s, "define:\n    h(x; a_{1}) -> y: a_{1}*x\nscene(1):\n    space: 2D(x, y)\n    step:\n        note: \"{h(a_{1})}\"\n");
    assert!(matches!(&t.scenes["1"].steps[1].note[0].segs[0], NoteSeg::Math { .. }));
    assert_eq!(codes(&s, "n = \"{nope}\"\nscene(1):\n    space: 2D(x, y)\n    step:\n        note: n\n"), vec![code::UNDECLARED]);
}

#[test]
fn notes_follow_definitions_and_variables() {
    let s = apply_ok(&base(), SCENE);
    // `{f}` refers to f: changing f changes nothing in the scene source, the browser renders the new f.
    let t = apply_ok(&s, &def(&["f(x; a) -> y: a*x^3"]));
    assert!(matches!(&t.scenes["1"].steps[2].note[1].segs[0], NoteSeg::Def { function } if function == "f"));
    // A changed variable changes the note (reference semantics).
    let t = apply_ok(&s, "m = \"바뀐 {T}\"\n");
    assert!(matches!(&t.scenes["1"].steps[2].note[1].segs[1], NoteSeg::Def { function } if function == "T"));
}

#[test]
fn claude_scene_survives_cascade_with_references_removed() {
    let s = apply_ok(&base(), "scene(1):\n    space: 2D(x, y)\n    show: T\n    clip: y < T(x)\n    step:\n        show: +T\n        note: \"{T(1)} 그리고 {T}\"\n");
    let a = run(&s, "del T\n").unwrap();
    let sc = &a.state.scenes["1"];
    let removed: Vec<(&str, &str)> = a.notes.iter().filter_map(|n| match n { Note::SceneRefRemoved { target, place, .. } => Some((target.as_str(), place.as_str())), _ => None }).collect();
    assert_eq!(removed, [("T", "show"), ("y < T(x)", "clip"), ("T", "step 1 show"), ("T(1)", "step 1 note"), ("T", "step 1 note")]);
    assert!(sc.clip.is_none() && sc.steps[0].visible.is_empty());
    assert_eq!(sc.steps[1].note[0].segs, vec![NoteSeg::Text { text: "T(1) 그리고 T".into() }]);
    assert_eq!(sc.step_count(), 1, "the empty step stays");
    assert!(!a.notes.iter().any(|n| matches!(n, Note::SceneDeleted { .. })));
    // Everything gone: the scene stays, empty.
    let t = apply_ok(&a.state, "del f\n");
    assert!(t.scenes.contains_key("1"));
    assert!(t.scenes["1"].members.is_empty());
}

#[test]
fn hide_conceals_until_revealed() {
    let s = apply_ok(&base(), "scene(1):\n    space: 2D(x, y)\n    show: f\n    hide: H2\n    step:\n        show: +H1\n    step:\n        show: +H2\n");
    let sc = &s.scenes["1"];
    assert_eq!(sc.steps[0].concealed, ["H2"]);
    assert_eq!(sc.steps[1].concealed, ["H2"]);
    assert!(sc.steps[2].concealed.is_empty());
    assert!(!sc.visible("H2", 1) && sc.visible("H2", 2));
}

#[test]
fn navigation_overwrites_only_what_a_step_mentions() {
    let s = apply_ok(&base(), SCENE);
    // The user turns f off and T on at step 0.
    let s = apply_user_op(&s, &UserOp::Toggle { scene: "1".into(), target: "f".into(), visible: false }, 10).unwrap().state;
    let s = apply_user_op(&s, &UserOp::Toggle { scene: "1".into(), target: "T".into(), visible: false }, 11).unwrap().state;
    let s = apply_user_op(&s, &UserOp::SetValue { constant: "a".into(), value: 5.0 }, 12).unwrap().state;
    // Forward into step 1: T (mentioned) follows the step, f (not mentioned) keeps the user's choice.
    let n = navigate(&s, "1", 0, 1, 13).unwrap();
    let sc = &n.state.scenes["1"];
    assert!(sc.visible("T", 1) && !sc.visible("f", 1) && !sc.visible("H1", 1));
    assert_eq!(n.state.constants["a"].value, 2.0);
    // Forward into step 2: animate sets the end value.
    let n2 = navigate(&n.state, "1", 1, 2, 14).unwrap();
    assert_eq!(n2.state.constants["a"].value, 3.0);
    // Back to step 1: only what step 2 changed (H2, a) reverts.
    let b = navigate(&n2.state, "1", 2, 1, 15).unwrap();
    assert!(!b.state.scenes["1"].visible("H2", 1));
    assert_eq!(b.state.constants["a"].value, 2.0);
    // Back to step 0: no earlier step set a, so it stays.
    let b0 = navigate(&b.state, "1", 1, 0, 16).unwrap();
    assert_eq!(b0.state.constants["a"].value, 2.0);
    assert!(b0.state.scenes["1"].visible("H1", 0) && !b0.state.scenes["1"].visible("T", 0));
    assert!(!b0.state.scenes["1"].visible("f", 0), "the user's f toggle survives");
    // Nothing to do: no changes.
    assert!(navigate(&b0.state, "1", 0, 0, 17).unwrap().changes.is_empty());
    assert!(navigate(&s, "9", 0, 1, 18).is_err());
}

#[test]
fn modifying_a_scene_keeps_user_state() {
    let s = apply_ok(&base(), SCENE);
    let s = apply_user_op(&s, &UserOp::Toggle { scene: "1".into(), target: "T".into(), visible: true }, 10).unwrap().state;
    let s = apply_user_op(&s, &UserOp::SetClip { scene: "1".into(), src: Some("-2 <= x <= 2".into()) }, 11).unwrap().state;
    assert_eq!(s.scenes["1"].user_clip.as_ref().unwrap().src, "-2 <= x <= 2");
    assert!(apply_user_op(&s, &UserOp::SetClip { scene: "1".into(), src: Some("-2 <= q".into()) }, 12).is_err());
    assert!(apply_user_op(&s, &UserOp::Toggle { scene: "1".into(), target: "g".into(), visible: true }, 12).is_err());
    // Same block again: unchanged.
    let a = run(&s, SCENE).unwrap();
    assert!(a.notes.contains(&Note::SceneUnchanged { key: "1".into() }));
    // A changed clip resets the user's clip; toggles stay.
    let a = run(&s, &SCENE.replace("-10 <= x <= 10", "-9 <= x <= 9")).unwrap();
    assert!(a.notes.contains(&Note::SceneModified { key: "1".into() }));
    assert!(a.state.scenes["1"].user_clip.is_none());
    assert!(a.state.scenes["1"].toggles["T"]);
    // del scene(1).
    let a = run(&s, "del scene(1)\n").unwrap();
    assert!(a.notes.contains(&Note::SceneDeleted { key: "1".into() }));
    assert_eq!(codes(&s, "del scene(7)\n"), vec![code::UNDECLARED]);
}

#[test]
fn req_points_at_a_scene_step() {
    let s = apply_ok(&base(), SCENE);
    let a = run(&s, "req(scene(1).step(2);; \"애니메이션이 빠름\")\nreq(scene(1))\n").unwrap();
    let r = &a.state.requests[&1];
    assert_eq!(r.target, ReqTarget::Scene { scene: "1".into(), step: Some(2) });
    assert_eq!(r.memo.as_deref(), Some("애니메이션이 빠름"));
    assert!(r.target_src.starts_with("scene(1):"));
    assert_eq!(codes(&s, "req(scene(1).step(3))\n"), vec![code::SCENE]);
    assert_eq!(codes(&s, "req(scene(5))\n"), vec![code::UNDECLARED]);
    assert_eq!(codes(&s, "req(scene(1); 0 < x < 1)\n"), vec![code::RANGE]);
}

#[test]
fn functions_on_other_axes() {
    let s = base();
    // Ranges on a t-function use t; req ranges use the target's axes.
    apply_ok(&s, "H9 = highlight(g; 0 <= t <= 1)\nreq(g; 0 < t < 1)\n");
    assert_eq!(codes(&s, "H9 = highlight(g; 0 <= x <= 1)\n"), vec![code::UNDECLARED]);
    // An axis name cannot become a constant or variable name.
    assert_eq!(codes(&s, "t = 1\n"), vec![code::NAME_CONFLICT]);
}
