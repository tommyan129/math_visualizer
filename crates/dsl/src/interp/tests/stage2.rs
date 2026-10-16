//! Stage 2: constant:, style, user variables, highlight, subexpression addresses, req.

use super::*;

const F: &str = "define:\n    f(x; a, b) -> y: a*x^2 + b*x + 1\n";

fn notes(state: &State, src: &str) -> Vec<Note> {
    run(state, src).unwrap_or_else(|d| panic!("{src}: {d:?}")).notes
}

fn apply_ok(state: &State, src: &str) -> State {
    run(state, src).unwrap_or_else(|d| panic!("{src}: {d:?}")).state
}

/// Text of the node with id `id` in `e` (terms print with their sign).
fn node_text(e: &Expr, id: NodeId) -> Option<String> {
    if e.id == id {
        return Some(crate::print::expr_dsl(e));
    }
    if let ExprKind::Sum { terms } = &e.kind {
        for t in terms {
            if t.id == id {
                return Some(crate::print::term_dsl(t.sign, &t.expr));
            }
        }
    }
    e.children().into_iter().find_map(|c| node_text(c, id))
}

fn sub_texts(s: &State, h: &str) -> Vec<String> {
    let HlTarget::Sub { function, nodes, .. } = &s.highlights[h].target else { panic!("{:?}", s.highlights[h].target) };
    nodes.iter().map(|n| node_text(&s.functions[function].body, *n).unwrap()).collect()
}

// ----- constant: ---------------------------------------------------------

#[test]
fn constant_block_declares_slider_conditions() {
    let s = build(&["constant:\n    a: range=(-5, 5), default=2, step=0.5\n    n: integer, range=(1, 50), default=4\ndefine:\n    f(x; a, n) -> y: a*sum(x^k; 1 <= k <= n)\n"]);
    let a = &s.constants["a"];
    assert_eq!((a.value, a.min, a.max, a.step, a.integer, a.declared), (2.0, -5.0, 5.0, 0.5, false, true));
    let n = &s.constants["n"];
    assert_eq!((n.value, n.min, n.max, n.step, n.integer, n.kind_fixed), (4.0, 1.0, 50.0, 1.0, true, true));
    // After the function: updates the auto-created constant; the step follows the range.
    let s = build(&[F, "constant:\n    b: range=(0, 1000)\n"]);
    let b = &s.constants["b"];
    assert_eq!((b.value, b.min, b.max, b.step), (1.0, 0.0, 1000.0, 10.0));
    let ns = notes(&build(&[F]), "constant:\n    b: range=(0, 1000)\n");
    assert_eq!(ns, vec![Note::ConstantDeclared { key: "b".into() }]);
    // Declared after `define:` in the same request: reported as declared, not auto-created.
    let ns = notes(&State::default(), &format!("{F}constant:\n    a: range=(-2, 2)\n"));
    assert!(ns.contains(&Note::ConstantDeclared { key: "a".into() }));
    assert!(!ns.contains(&Note::ConstantCreated { key: "a".into() }));
    assert!(ns.contains(&Note::ConstantCreated { key: "b".into() }));
    // Re-declaring the same conditions changes nothing.
    assert!(notes(&s, "constant:\n    b: range=(0, 1000), default=1\n").is_empty());
}

#[test]
fn constant_block_errors() {
    let s = build(&[F]);
    assert_eq!(codes(&s, "constant:\n    a: range=(5, 1)\n"), vec![code::CONST_RANGE]);
    assert_eq!(codes(&s, "constant:\n    a: range=(0, 1), default=3\n"), vec![code::CONST_RANGE]);
    assert_eq!(codes(&s, "constant:\n    a: range=(0, 1), step=0\n"), vec![code::CONST_RANGE]);
    assert_eq!(codes(&s, "constant:\n    a: integer, range=(0, 10), step=0.5\n"), vec![code::CONST_KIND]);
    assert_eq!(codes(&s, "constant:\n    a: rational, range=(0, 1)\n"), vec![code::CONST_KIND]);
    assert_eq!(codes(&s, "constant:\n    a: color=(0, 1)\n"), vec![code::CONST_PROP]);
    assert_eq!(codes(&s, "constant:\n    a: range=(0, 2*\\pi)\n"), vec![code::CONST_PROP]);
    assert_eq!(codes(&s, "constant:\n    a: range=(0, 1), range=(0, 2)\n"), vec![code::DUPLICATE]);
    assert_eq!(codes(&s, "constant:\n    q: range=(0, 1)\n"), vec![code::UNUSED_CONST]);
    assert_eq!(codes(&s, "constant:\n    f: range=(0, 1)\n"), vec![code::NAME_CONFLICT]);
    // A constant declared real cannot become a sum bound.
    let r = build(&["constant:\n    n: real, range=(0, 10)\ndefine:\n    g(x; n) -> y: n*x\n"]);
    assert_eq!(codes(&r, "define:\n    g(x; n) -> y: sum(x^k; 1 <= k <= n)\n"), vec![code::CONST_KIND]);
    // Errors in several entries are all reported.
    assert_eq!(codes(&s, "constant:\n    a: range=(5, 1)\n    b: default=x\n"), vec![code::CONST_RANGE, code::CONST_PROP]);
}

#[test]
fn integer_constant_inferred_from_sum_keeps_integer_rules() {
    let s = build(&["define:\n    g(x; n) -> y: sum(x^k; 1 <= k <= n)\n"]);
    assert_eq!(codes(&s, "constant:\n    n: range=(0, 10), step=0.5\n"), vec![code::CONST_KIND]);
    let s = apply_ok(&s, "constant:\n    n: range=(0, 10)\n");
    assert_eq!((s.constants["n"].integer, s.constants["n"].step), (true, 1.0));
}

// ----- style ---------------------------------------------------------------

#[test]
fn style_block() {
    let s = build(&["style up:\n    color: orange\n    fill: above\n    opacity: 0.25\n    width: 2\n"]);
    let st = &s.styles["up"];
    assert_eq!((st.color.as_deref(), st.fill, st.opacity, st.width), (Some("#ffa057"), Some(Fill::Above), Some(0.25), Some(2.0)));
    let s = build(&["style hex:\n    color: \"#FF8800\"\n"]);
    assert_eq!(s.styles["hex"].color.as_deref(), Some("#ff8800"));
    let e = State::default();
    assert_eq!(codes(&e, "style s:\n    color: pink\n"), vec![code::STYLE]);
    assert_eq!(codes(&e, "style s:\n    fill: left\n"), vec![code::STYLE]);
    assert_eq!(codes(&e, "style s:\n    opacity: 2\n"), vec![code::STYLE]);
    assert_eq!(codes(&e, "style s:\n    size: 2\n"), vec![code::STYLE]);
    assert_eq!(codes(&e, "style s:\n    color: red\n    color: blue\n"), vec![code::DUPLICATE]);
    assert_eq!(codes(&e, "style blue:\n    fill: above\n"), vec![code::NAME_CONFLICT]);
}

// ----- user variables --------------------------------------------------------

#[test]
fn user_variables() {
    let s = build(&[F, "R1 = 0.9 < x < 1.1\nmemo1 = \"x = 1 \\\"근처\\\"\"\n"]);
    assert_eq!(s.vars["R1"].src, "0.9 < x < 1.1");
    assert!(matches!(s.vars["memo1"].value.kind, ExprKind::Str { .. }));
    assert_eq!(codes(&s, "R2 = 0 < q\n"), vec![code::UNDECLARED]);
    assert_eq!(codes(&s, "R2 = g(x) < y\n"), vec![code::UNDECLARED]);
    assert_eq!(codes(&s, "x = 1\n"), vec![code::NAME_CONFLICT]);
    assert_eq!(codes(&s, "f = 1\n"), vec![code::NAME_CONFLICT]);
    assert_eq!(codes(&s, "a = 1\n"), vec![code::NAME_CONFLICT]);
    // Self reference through another variable.
    let c = apply_ok(&s, "A = 0 < x\nB = A and x < 1\n");
    assert_eq!(codes(&c, "A = B\n"), vec![code::CYCLE]);
    // A variable cannot be used inside a function body.
    assert_eq!(codes(&s, "define:\n    g(x) -> y: R1*x\n"), vec![code::TYPE]);
}

// ----- highlight -------------------------------------------------------------

#[test]
fn highlight_on_function_with_range() {
    let s = build(&[F]);
    let a = run(&s, "H1 = highlight(f; 0 <= x <= 2; blue; \"정적분 넓이\")\n").unwrap();
    assert_eq!(a.notes, vec![Note::ObjectCreated { kind: ObjKind::Highlight, key: "H1".into() }]);
    let h = &a.state.highlights["H1"];
    assert_eq!(h.target, HlTarget::Function { function: "f".into() });
    assert_eq!(h.style, HlStyle { color: "#6ea8ff".into(), fill: Fill::Axis, opacity: DEFAULT_OPACITY, width: DEFAULT_WIDTH });
    assert_eq!(h.memo.as_deref(), Some("정적분 넓이"));
    assert_eq!(h.consts, ["a", "b"]);
    assert_eq!(h.deps, ["f"]);
    assert_eq!(h.src, "highlight(f; 0 <= x <= 2; blue; \"정적분 넓이\")");
    let range = h.range.as_ref().unwrap();
    let mut kinds = vec![];
    range.walk(&mut |e| {
        if let ExprKind::Name { name, kind } = &e.kind {
            kinds.push((name.key.clone(), *kind));
        }
    });
    assert_eq!(kinds, [("x".to_string(), NameKind::Input)]);
    assert!(a.changes.iter().any(|c| matches!(c, Change::PutHighlight { highlight } if highlight.key == "H1" && highlight.version == 1)));

    // Modify, then unchanged, then delete.
    let m = run(&a.state, "H1 = highlight(f; 0 <= x <= 3; blue; \"정적분 넓이\")\n").unwrap();
    assert_eq!(m.notes, vec![Note::ObjectModified { kind: ObjKind::Highlight, key: "H1".into() }]);
    assert_eq!(m.state.highlights["H1"].order, h.order, "order and auto color are kept");
    let u = run(&m.state, "H1 = highlight(f; 0 <= x <= 3; blue; \"정적분 넓이\")\n").unwrap();
    assert_eq!(u.notes, vec![Note::ObjectUnchanged { kind: ObjKind::Highlight, key: "H1".into() }]);
    let d = run(&m.state, "del H1\n").unwrap();
    assert!(d.state.highlights.is_empty());
    assert!(d.changes.contains(&Change::RemoveHighlight { key: "H1".into() }));
}

#[test]
fn auto_color_is_fixed_at_creation() {
    let s = build(&[F, "H1 = highlight(f)\nH2 = highlight(f)\n"]);
    let (c1, c2) = (s.highlights["H1"].style.color.clone(), s.highlights["H2"].style.color.clone());
    assert_ne!(c1, c2);
    // Deleting H1 does not recolor H2.
    let s = apply_ok(&s, "del H1\nH3 = highlight(f)\n");
    assert_eq!(s.highlights["H2"].style.color, c2);
    assert_ne!(s.highlights["H3"].style.color, c2);
}

#[test]
fn targetless_between_expands_to_both_orders() {
    let s = build(&["define:\n    f(x) -> y: x\n    g(x) -> y: x^2\n", "H2 = highlight(; between(f, g) and 0 <= x <= 2; green)\n"]);
    let h = &s.highlights["H2"];
    assert_eq!(h.target, HlTarget::Grid {});
    assert_eq!(h.style.fill, Fill::Region);
    assert_eq!(h.deps, ["f", "g"]);
    let text = crate::print::expr_dsl(h.range.as_ref().unwrap());
    assert_eq!(text, "(f(x) < y < g(x) or g(x) < y < f(x)) and 0 <= x <= 2");
    // Only functions; only in targetless highlights.
    assert_eq!(codes(&s, "H3 = highlight(; between(f, 2))\n"), vec![code::TYPE]);
    assert_eq!(codes(&s, "H3 = highlight(f; between(f, g))\n"), vec![code::RANGE]);
    assert_eq!(codes(&s, "define:\n    h(x) -> y: between(f, g)\n"), vec![code::TYPE]);
}

#[test]
fn ranges_follow_variables_by_reference() {
    let s = build(&[F, "R1 = 0 <= x <= 1\nm = \"처음\"\nH1 = highlight(f; R1 and x > a;; m)\n"]);
    let h = &s.highlights["H1"];
    assert_eq!(h.deps, ["R1", "f", "m"]);
    assert_eq!(h.memo.as_deref(), Some("처음"));
    assert_eq!(crate::print::expr_dsl(h.range.as_ref().unwrap()), "(0 <= x <= 1) and x > a");
    // Reassigning the variable changes the highlight in the same request's result.
    let a = run(&s, "R1 = 0 <= x <= 5\nm = \"나중\"\n").unwrap();
    let h = &a.state.highlights["H1"];
    assert_eq!(crate::print::expr_dsl(h.range.as_ref().unwrap()), "(0 <= x <= 5) and x > a");
    assert_eq!(h.memo.as_deref(), Some("나중"));
    assert!(a.changes.iter().any(|c| matches!(c, Change::PutHighlight { .. })));
    // A variable that stops being a region drops the highlight (cascade), and says why.
    let a = run(&s, "R1 = 3\n").unwrap();
    assert!(a.state.highlights.is_empty());
    assert!(matches!(&a.notes[1], Note::ObjectDeleted { kind: ObjKind::Highlight, key, via: None, reason: Some(_) } if key == "H1"), "{:?}", a.notes);
    // Deleting the variable cascades with `via`.
    let a = run(&s, "del R1\n").unwrap();
    assert!(a.notes.contains(&Note::ObjectDeleted { kind: ObjKind::Highlight, key: "H1".into(), via: Some("R1".into()), reason: None }));
}

#[test]
fn highlight_cascade_on_function_and_style_delete() {
    let s = build(&[
        "define:\n    f(x) -> y: x\n    g(x) -> y: f(x) + 1\n",
        "style up:\n    fill: above\nH1 = highlight(g;; up)\nH2 = highlight(; y < f(x))\nH3 = highlight(f)\n",
    ]);
    let a = run(&s, "del f\n").unwrap();
    assert!(a.state.highlights.is_empty());
    let deleted: Vec<_> = a.notes.iter().filter_map(|n| match n { Note::ObjectDeleted { key, via, .. } => Some((key.as_str(), via.as_deref())), _ => None }).collect();
    assert_eq!(deleted, [("H1", Some("g")), ("H2", Some("f")), ("H3", Some("f"))]);
    let a = run(&s, "del up\n").unwrap();
    assert_eq!(a.state.highlights.keys().collect::<Vec<_>>(), ["H2", "H3"]);
    // Toggle entries of deleted highlights go too.
    let t = apply_user_op(&s, &UserOp::Toggle { scene: DEFAULT_SCENE.into(), target: "H3".into(), visible: false }, 3).unwrap().state;
    assert_eq!(t.scenes[DEFAULT_SCENE].hidden, ["H3"]);
    assert!(apply_ok(&t, "del H3\n").scenes[DEFAULT_SCENE].hidden.is_empty());
}

#[test]
fn highlight_with_user_style_and_fill_rules() {
    let s = build(&[F, "style up:\n    color: red\n    fill: above\n    opacity: 0.3\nstyle area:\n    fill: region\n"]);
    let s2 = apply_ok(&s, "H1 = highlight(f;; up)\nH2 = highlight(; y > 0; up)\n");
    assert_eq!(s2.highlights["H1"].style, HlStyle { color: "#ff6b6b".into(), fill: Fill::Above, opacity: 0.3, width: DEFAULT_WIDTH });
    assert_eq!(s2.highlights["H2"].style.fill, Fill::Region, "targetless always fills the region");
    assert_eq!(codes(&s, "H1 = highlight(f;; area)\n"), vec![code::STYLE]);
    // Changing the style updates highlights using it.
    let s3 = apply_ok(&s2, "style up:\n    color: green\n    fill: below\n");
    assert_eq!((s3.highlights["H1"].style.color.as_str(), s3.highlights["H1"].style.fill), ("#5fd38a", Fill::Below));
}

#[test]
fn highlight_errors() {
    let s = build(&[F, "R1 = 0 < x\nH1 = highlight(f)\n"]);
    assert_eq!(codes(&s, "H2 = highlight()\n"), vec![code::SLOTS]);
    assert_eq!(codes(&s, "H2 = highlight(f;;;;)\n"), vec![code::SLOTS]);
    assert_eq!(codes(&s, "H2 = highlight(q)\n"), vec![code::UNDECLARED]);
    assert_eq!(codes(&s, "H2 = highlight(H1)\n"), vec![code::TARGET]);
    assert_eq!(codes(&s, "H2 = highlight(a)\n"), vec![code::TARGET]);
    assert_eq!(codes(&s, "H2 = highlight(2*x)\n"), vec![code::TARGET]);
    assert_eq!(codes(&s, "H2 = highlight(f; x)\n"), vec![code::RANGE]);
    assert_eq!(codes(&s, "H2 = highlight(f; y > 0)\n"), vec![code::UNDECLARED]);
    assert_eq!(codes(&s, "H2 = highlight(f; 0 < q)\n"), vec![code::UNDECLARED]);
    assert_eq!(codes(&s, "H2 = highlight(f.(b*x); 0 < x)\n"), vec![code::RANGE]);
    assert_eq!(codes(&s, "H2 = highlight(f;; pink)\n"), vec![code::STYLE]);
    assert_eq!(codes(&s, "H2 = highlight(f;; \"blue\")\n"), vec![code::STYLE]);
    assert_eq!(codes(&s, "H2 = highlight(f;;; R1)\n"), vec![code::MEMO]);
    assert_eq!(codes(&s, "H2 = highlight(f;;; 3)\n"), vec![code::MEMO]);
    assert_eq!(codes(&s, "R1 = highlight(f)\n"), vec![code::NAME_CONFLICT]);
    assert_eq!(codes(&s, "H1 = 3\n"), vec![code::NAME_CONFLICT]);
    assert_eq!(codes(&s, "define:\n    H1(x) -> y: x\n"), vec![code::NAME_CONFLICT]);
    // Independent errors in one request are all reported, nothing applied.
    assert_eq!(codes(&s, "H2 = highlight(q)\nH3 = highlight(f;; pink)\n"), vec![code::UNDECLARED, code::STYLE]);
}

// ----- subexpression addresses --------------------------------------------------

#[test]
fn address_coef_var_and_terms() {
    let s = build(&[F, "H4 = highlight(f.(b*x).coef;; orange)\nH5 = highlight(f.(a*x^2).var;; blue)\nH6 = highlight(f.(1))\n"]);
    assert_eq!(sub_texts(&s, "H4"), ["b"]);
    assert_eq!(sub_texts(&s, "H5"), ["x^2"]);
    assert_eq!(sub_texts(&s, "H6"), ["1"]);
    let HlTarget::Sub { address, .. } = &s.highlights["H4"].target else { panic!() };
    assert_eq!(address, "f.(b*x).coef");
    assert_eq!(s.highlights["H4"].style.color, "#ffa057");
}

#[test]
fn address_parts() {
    let s = build(&["define:\n    g(x) -> y: x^2/(x - 1)\n    h(x; c) -> y: sin(3*x) + c*\\e^(2*x) - 3*c*x\n"]);
    let s = apply_ok(&s, "A1 = highlight(g.den)\nA2 = highlight(g.num.exp)\nA3 = highlight(h.(sin(3*x)).arg.coef)\nA4 = highlight(h.(c*\\e^(2*x)).var.exp)\nA5 = highlight(h.(-3*c*x).coef)\n");
    assert_eq!(sub_texts(&s, "A1"), ["x - 1"], "the Paren around the denominator is skipped");
    assert_eq!(sub_texts(&s, "A2"), ["2"]);
    assert_eq!(sub_texts(&s, "A3"), ["3"]);
    assert_eq!(sub_texts(&s, "A4"), ["2*x"]);
    assert_eq!(sub_texts(&s, "A5"), ["3", "c"], "several coefficient factors");
    assert_eq!(codes(&s, "B = highlight(g.exp)\n"), vec![code::ADDRESS_PART]);
    assert_eq!(codes(&s, "B = highlight(h.(-3*c*x).var.exp)\n"), vec![code::ADDRESS_PART]);
    assert_eq!(codes(&s, "B = highlight(g.top)\n"), vec![code::ADDRESS_PART]);
    assert_eq!(codes(&s, "B = highlight(h.(-3*c*x).coef.exp)\n"), vec![code::ADDRESS_PART]);
    assert_eq!(codes(&s, "B = highlight(A1.den)\n"), vec![code::TARGET]);
}

#[test]
fn address_sign_and_shape_errors() {
    let s = build(&["define:\n    f(x; a, b) -> y: a - b*x + x*x\n    p(x) -> y: x + sin(x)\n    n(x) -> y: -x^3/6 + x\n"]);
    let d = first_err(&s, "H = highlight(f.(b*x))\n");
    assert_eq!((d.code, d.suggestion.as_deref()), (code::ADDRESS_SIGN, Some("f.(-b*x)")));
    let s2 = apply_ok(&s, "H = highlight(f.(-b*x))\nT = highlight(n.(-x^3/6))\n");
    assert_eq!(sub_texts(&s2, "H"), ["-b*x"], "the term with its sign");
    assert_eq!(sub_texts(&s2, "T"), ["-x^3/6"], "a leading minus is the term's sign");
    let d = first_err(&s, "H = highlight(f.(c*x))\n");
    assert_eq!((d.code, d.suggestion.as_deref()), (code::ADDRESS_MISSING, Some("있는 항: a, -b*x, x*x")));
    // `x` appears twice in p (a term and inside sin).
    assert_eq!(codes(&s, "H = highlight(p.(x))\n"), vec![code::ADDRESS_AMBIGUOUS]);
    assert_eq!(sub_texts(&apply_ok(&s, "H = highlight(p.(sin(x)).arg)\n"), "H"), ["x"]);
    // Parentheses in the pattern or the body do not matter.
    let q = build(&["define:\n    q(x) -> y: 3*((x + 1))^2\n", "H = highlight(q.((x+1)^2).base)\n"]);
    assert_eq!(sub_texts(&q, "H"), ["x + 1"]);
}

#[test]
fn address_follows_shape_and_cascades_when_the_part_vanishes() {
    let s = build(&[F, "H4 = highlight(f.(b*x).coef;; orange)\n"]);
    let id_before = match &s.highlights["H4"].target { HlTarget::Sub { nodes, .. } => nodes.clone(), _ => panic!() };
    // A new term in front shifts node ids; the address follows the shape.
    let a = run(&s, "define:\n    f(x; a, b) -> y: x^3 + a*x^2 + b*x + 1\n").unwrap();
    assert_eq!(sub_texts(&a.state, "H4"), ["b"]);
    let HlTarget::Sub { nodes, .. } = &a.state.highlights["H4"].target else { panic!() };
    assert_ne!(nodes, &id_before);
    assert!(a.changes.iter().any(|c| matches!(c, Change::PutHighlight { .. })));
    // The term disappears: the highlight is deleted and the response says why.
    let a = run(&s, "define:\n    f(x; a, b) -> y: a*x^2 + 1\n").unwrap();
    assert!(a.state.highlights.is_empty());
    let n = a.notes.iter().find(|n| matches!(n, Note::ObjectDeleted { .. })).unwrap();
    assert_eq!(n, &Note::ObjectDeleted { kind: ObjKind::Highlight, key: "H4".into(), via: None, reason: Some("f 변경으로 f.(b*x).coef가 가리키던 부분이 사라짐".into()) });
}

// ----- req -----------------------------------------------------------------------

#[test]
fn req_is_numbered_and_fixed_to_its_step() {
    let s = build(&["define:\n    f(x; a) -> y: a*x\n    g(x; a) -> y: f(x) + 1\n", "memo1 = \"끊겨 보임\"\nR1 = 0.9 < x < 1.1\nH1 = highlight(f)\n"]);
    let a = apply(&s, "req(g; R1; memo1)\nreq(f.(a*x).coef;; \"부호\")\nreq(H1)\nreq(f)\n", &Limits::default(), 7).unwrap();
    assert_eq!(a.notes, (1..=4).map(|number| Note::RequestAdded { number }).collect::<Vec<_>>());
    let r = &a.state.requests[&1];
    assert_eq!(r.target, ReqTarget::Function { function: "g".into() });
    assert_eq!((r.range.as_deref(), r.memo.as_deref(), r.seq, r.done), (Some("(0.9 < x < 1.1)"), Some("끊겨 보임"), 7, false));
    assert_eq!(r.target_src, "g(x; a) -> y: f(x) + 1");
    assert_eq!(r.deps, ["f", "g"]);
    assert_eq!(r.src, "req(g; R1; memo1)");
    assert_eq!(a.state.requests[&2].target, ReqTarget::Sub { function: "f".into(), address: "f.(a*x).coef".into() });
    assert_eq!(a.state.requests[&3].target, ReqTarget::Highlight { highlight: "H1".into() });
    assert_eq!(a.state.requests[&3].deps, ["f"]);
    // Not cascaded: deleting the target keeps the req.
    let d = apply_ok(&a.state, "del f\n");
    assert_eq!(d.requests.len(), 4);
    // The memo was read at the request's step; changing the variable later does not change it.
    let m = apply_ok(&a.state, "memo1 = \"다른 말\"\n");
    assert_eq!(m.requests[&1].memo.as_deref(), Some("끊겨 보임"));
    // Marking done is a user operation.
    let done = apply_user_op(&a.state, &UserOp::SetRequestDone { number: 2, done: true }, 8).unwrap();
    assert!(done.state.requests[&2].done);
    assert!(apply_user_op(&a.state, &UserOp::SetRequestDone { number: 9, done: true }, 8).is_err());
}

#[test]
fn req_errors() {
    let s = build(&[F]);
    assert_eq!(codes(&s, "req()\n"), vec![code::SLOTS]);
    assert_eq!(codes(&s, "req(;0 < x)\n"), vec![code::SLOTS]);
    assert_eq!(codes(&s, "req(f;;;)\n"), vec![code::SLOTS]);
    assert_eq!(codes(&s, "req(q)\n"), vec![code::UNDECLARED]);
    assert_eq!(codes(&s, "req(a)\n"), vec![code::TARGET]);
    assert_eq!(codes(&s, "req(f; 3)\n"), vec![code::RANGE]);
    assert_eq!(codes(&s, "req(f;; 3)\n"), vec![code::MEMO]);
    assert_eq!(codes(&s, "req(f.(c*x))\n"), vec![code::ADDRESS_MISSING]);
}
