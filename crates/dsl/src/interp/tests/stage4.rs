use super::*;

fn ty_of(s: &State, key: &str) -> Ty {
    s.functions[key].ty.clone()
}

fn la() -> State {
    build(&[&def(&[
        "A: [[1, 2],",
        "    [3, 4]]",
        "B: [[1, 0, 2], [0, 1, 3]]",
        "v(; a): [1, a]",
        "w: [0, 1]",
        "u: [1, 2, 0]",
        "R(\\theta) -> matrix: [[cos(\\theta), -sin(\\theta)], [sin(\\theta), cos(\\theta)]]",
    ])])
}

#[test]
fn literals_and_values_are_typed_by_shape() {
    let s = la();
    assert_eq!(ty_of(&s, "A"), Ty::Matrix { m: 2, n: 2 });
    assert_eq!(ty_of(&s, "B"), Ty::Matrix { m: 2, n: 3 });
    assert_eq!(ty_of(&s, "v"), Ty::Vector { n: 2 });
    assert_eq!(ty_of(&s, "u"), Ty::Vector { n: 3 });
    assert_eq!(ty_of(&s, "R"), Ty::Matrix { m: 2, n: 2 });
    assert_eq!(s.functions["v"].kind, DefKind::Value);
    assert_eq!(s.functions["v"].consts, ["a"]);
    assert_eq!(s.functions["v"].source(), "v(; a): [1, a]");
    assert_eq!(s.functions["R"].kind, DefKind::Matrix);
    assert_eq!(s.functions["R"].signature(), "R(\\theta) -> matrix");
    assert!(s.constants.contains_key("a"));
    // Not drawn: no default scene from these alone, no scene members.
    assert!(s.scenes.is_empty());
}

#[test]
fn operations_follow_sizes() {
    let s = build(&[&def(&[
        "A: [[1, 2], [3, 4]]",
        "B: [[1, 0, 2], [0, 1, 3]]",
        "v: [1, 2]",
        "r: [[1, 2, 3]]",
        "C: matmul(A, B)",
        "p: matmul(A, v)",
        "q: matmul(transpose(v), v)",
        "T: transpose(B)",
        "t: transpose(r)",
        "D: 2*A - inv(A)/3 + pow(A, 2)",
        "n: -v + v",
    ])]);
    assert_eq!(ty_of(&s, "r"), Ty::Matrix { m: 1, n: 3 });
    assert_eq!(ty_of(&s, "C"), Ty::Matrix { m: 2, n: 3 });
    assert_eq!(ty_of(&s, "p"), Ty::Vector { n: 2 }, "m×1 is a vector");
    assert_eq!(ty_of(&s, "q"), Ty::Matrix { m: 1, n: 1 }, "1×1 stays a matrix");
    assert_eq!(ty_of(&s, "T"), Ty::Matrix { m: 3, n: 2 });
    assert_eq!(ty_of(&s, "t"), Ty::Vector { n: 3 });
    assert_eq!(ty_of(&s, "D"), Ty::Matrix { m: 2, n: 2 });
    assert_eq!(ty_of(&s, "n"), Ty::Vector { n: 2 });
    // Every node of a stored body is typed (the browser picks matrix code from it).
    s.functions["D"].body.walk(&mut |e| assert_ne!(e.ty, Ty::Unknown));
}

#[test]
fn static_shape_errors_with_suggestions() {
    let s = la();
    // Ragged rows.
    let d = first_err(&s, &def(&["M: [[1, 2], [3]]"]));
    assert_eq!(d.code, code::SHAPE);
    // matmul size mismatch: 2×3 · 2×2.
    let d = first_err(&s, &def(&["M: matmul(B, A)"]));
    assert_eq!(d.code, code::SHAPE);
    assert!(d.message.contains("2×3") && d.message.contains("2×2"), "{}", d.message);
    // `A^2` → pow; `A*B` → matmul.
    let d = first_err(&s, &def(&["M: A^2"]));
    assert_eq!((d.code, d.suggestion.as_deref()), (code::MATRIX_OP, Some("pow(A, 2)")));
    let d = first_err(&s, &def(&["M: A*B"]));
    assert_eq!((d.code, d.suggestion.as_deref()), (code::MATRIX_OP, Some("matmul(A, B)")));
    let d = first_err(&s, &def(&["M(; a): v*v"]));
    assert_eq!(d.code, code::MATRIX_OP);
    // Sum of different sizes, matrix + number.
    assert_eq!(codes(&s, &def(&["M: A + B"])), vec![code::SHAPE]);
    assert_eq!(codes(&s, &def(&["M: A + 1"])), vec![code::SHAPE]);
    // Divide by a matrix, square-only functions.
    assert_eq!(codes(&s, &def(&["M: A/A"])), vec![code::MATRIX_OP]);
    assert_eq!(codes(&s, &def(&["M: inv(B)"])), vec![code::SHAPE]);
    assert_eq!(codes(&s, &def(&["M: det(w)"])), vec![code::SHAPE]);
    assert_eq!(codes(&s, &def(&["M: pow(A, 1.5)"])), vec![code::TYPE]);
    assert_eq!(first_err(&s, &def(&["M: matmul(2, A)"])).suggestion.as_deref(), Some("2*A"));
    // Mixed / nested literals, vectors of vectors, empty.
    assert_eq!(codes(&s, &def(&["M: [[1, 2], 3]"])), vec![code::SHAPE]);
    assert_eq!(codes(&s, &def(&["M: [[[1]]]"])), vec![code::SHAPE]);
    assert_eq!(codes(&s, &def(&["M: [w, w]"])), vec![code::SHAPE]);
    assert_eq!(codes(&s, &def(&["M: []"])), vec![code::SHAPE]);
    // Arity of a matrix builtin.
    assert_eq!(codes(&s, &def(&["M: matmul(A)"])), vec![code::ARITY]);
}

#[test]
fn element_access() {
    let s = build(&[&def(&[
        "A: [[1, 2], [3, 4]]",
        "v: [5, 6]",
        "R(t) -> matrix: [[cos(t), -sin(t)], [sin(t), cos(t)]]",
        "f(x) -> y: A_[1, 2]*x + v_[2]",
        "g(x; k) -> y: matmul(R(x), [1, 0])_[2] + v_[k]",
    ])]);
    assert_eq!(s.functions["f"].calls, ["A", "v"]);
    assert_eq!(s.functions["g"].calls, ["R", "v"]);
    assert_eq!(codes(&s, &def(&["h(x) -> y: v_[3]"])), vec![code::INDEX]);
    assert_eq!(codes(&s, &def(&["h(x) -> y: v_[0]"])), vec![code::INDEX]);
    assert_eq!(codes(&s, &def(&["h(x) -> y: A_[1]"])), vec![code::INDEX]);
    assert_eq!(codes(&s, &def(&["h(x) -> y: v_[1, 1]"])), vec![code::INDEX]);
    assert_eq!(codes(&s, &def(&["h(x) -> y: A_[1.5, 1]"])), vec![code::INDEX]);
    assert_eq!(codes(&s, &def(&["h(x) -> y: x_[1]"])), vec![code::TYPE]);
}

#[test]
fn definition_kinds_and_their_errors() {
    let s = la();
    // A curve must be a number; a matrix function must be a matrix; a value must be an array.
    let d = first_err(&s, &def(&["f(x) -> y: [x, 1]"]));
    assert_eq!(d.code, code::TYPE);
    assert_eq!(d.suggestion.as_deref(), Some("f(x) -> matrix: [x, 1]"));
    assert_eq!(codes(&s, &def(&["M(x) -> matrix: x^2"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["c: 2"])), vec![code::TYPE]);
    // Constant-only parentheses are for vectors; a function needs inputs.
    assert_eq!(codes(&s, &def(&["f(; a) -> y: a"])), vec![code::SYNTAX]);
    assert_eq!(codes(&s, &def(&["M(; a) -> matrix: [[a]]"])), vec![code::SYNTAX]);
    // A vector with inputs, or an undeclared constant in a value.
    assert_eq!(codes(&s, &def(&["m(x): [x, 1]"])), vec![code::SYNTAX]);
    let d = first_err(&s, &def(&["m: [1, b]"]));
    assert_eq!((d.code, d.suggestion.as_deref()), (code::UNDECLARED, Some("m(; b)")));
    // Referencing `v` needs its constant `a`.
    let d = first_err(&s, &def(&["m: 2*v"]));
    assert_eq!((d.code, d.suggestion.as_deref()), (code::MISSING_CONST, Some("m(; a)")));
    // A value is not called; a function is not a value; self-reference.
    assert_eq!(codes(&s, &def(&["f(x) -> y: w(x)_[1]"])), vec![code::NOT_FUNCTION]);
    assert_eq!(codes(&s, &def(&["M: R"])), vec![code::NOT_VALUE]);
    assert_eq!(codes(&s, &def(&["M: 2*M"])), vec![code::CYCLE]);
    // Matrix builtin names are reserved.
    assert_eq!(codes(&s, &def(&["det(x) -> y: x"])), vec![code::NAME_CONFLICT]);
}

#[test]
fn constants_propagate_through_values() {
    let s = build(&[&def(&["v(; a): [1, a]", "f(x; a) -> y: v_[2]*x"])]);
    // Adding a constant to `v` adds it to `f`.
    let a = run(&s, &def(&["v(; a, b): [b, a]"])).unwrap();
    assert_eq!(a.state.functions["f"].consts, ["a", "b"]);
    // Deleting `v` cascades to `f`.
    let a = run(&s, "del v\n").unwrap();
    assert!(a.state.functions.is_empty());
    assert!(a.notes.contains(&Note::FunctionDeleted { key: "f".into(), via: Some("v".into()) }));
}

#[test]
fn span_and_arrow_definitions() {
    let s = build(&[&def(&[
        "u: [1, 2, 0]",
        "w3: [0, 1, 1]",
        "v(; a): [1, a]",
        "w: [0, 1]",
        "S(; a): span(v, w)",
        "T: span(u, w3)",
        "p(; a): arrow(v)",
        "q(; a, c): arrow([1, c]; v)",
    ])]);
    assert_eq!(s.functions["S"].kind, DefKind::Span);
    assert_eq!(s.functions["S"].vector_dim(), Some(2));
    assert_eq!(s.functions["T"].vector_dim(), Some(3));
    assert_eq!(s.functions["q"].kind, DefKind::Arrow);
    assert_eq!(s.functions["q"].source(), "q(; a, c): arrow([1, c]; v)");
    // Length-2 spans and arrows get the default 2D(x, y) scene, length 3 does not fit it.
    let sc = &s.scenes["2D(x, y)"];
    assert_eq!(sc.members, ["S", "p", "q"]);
    assert!(sc.visible("S", 0));
    // Errors: lengths, matrices, numbers, nesting, inputs, use in an expression.
    assert_eq!(codes(&s, &def(&["X(; a): span(v, u)"])), vec![code::SHAPE]);
    assert_eq!(codes(&s, &def(&["X: span([1, 2, 3, 4])"])), vec![code::SHAPE]);
    assert_eq!(codes(&s, &def(&["X: span([[1, 2], [3, 4]])"])), vec![code::SHAPE]);
    assert_eq!(codes(&s, &def(&["X: arrow(w; 1)"])), vec![code::SHAPE]);
    assert_eq!(codes(&s, &def(&["X: arrow(w; u)"])), vec![code::SHAPE]);
    assert_eq!(codes(&s, &def(&["X: span()"])), vec![code::ARITY]);
    assert_eq!(codes(&s, &def(&["X: 2*span(w)"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["X(x): span(w)"])), vec![code::SYNTAX]);
    assert_eq!(codes(&s, &def(&["f(x) -> y: S_[1]"])), vec![code::TYPE]);
    assert_eq!(codes(&s, &def(&["f(x) -> y: span(w)"])), vec![code::TYPE]);
    // Toggle, cascade, req.
    let a = run(&s, "del w\n").unwrap();
    assert!(!a.state.functions.contains_key("S"));
    let a = run(&s, "req(S;; \"평면인지\")\n").unwrap();
    assert!(matches!(&a.state.requests[&1].target, ReqTarget::Function { function } if function == "S"));
    // Not a highlight target.
    assert_eq!(codes(&s, "H1 = highlight(S)\n"), vec![code::TARGET]);
}

#[test]
fn span_placement_by_vector_length() {
    let s = build(&[&def(&["u: [1, 2, 0]", "w3: [0, 1, 1]", "S: span(u, w3)", "v(; a): [1, a]", "w: [0, 1]", "P(; a): span(v, w)", "g(t) -> y: t"])]);
    // Length-3 span in a 2D scene: rejected.
    let d = first_err(&s, "scene(1):\n    space: 2D(x, y)\n    show: S\n");
    assert_eq!(d.code, code::PLACEMENT);
    // Length 2 fits any 2D system, and alone infers 2D(x, y).
    let s2 = run(&s, "scene(1):\n    show: P\n").unwrap().state;
    assert_eq!(s2.scenes["1"].axes.iter().map(|a| a.key.as_str()).collect::<Vec<_>>(), ["x", "y"]);
    let s3 = run(&s, "scene(2):\n    space: 2D(t, y)\n    show: g, P\n").unwrap().state;
    assert_eq!(s3.scenes["2"].members, ["P", "g"]);
    // A vector value is not drawn.
    assert_eq!(codes(&s, "scene(1):\n    space: 2D(x, y)\n    show: v\n"), vec![code::TARGET]);
}

#[test]
fn matrix_function_inside_a_curve() {
    let s = build(&[&def(&[
        "R(\\theta) -> matrix: [[cos(\\theta), -sin(\\theta)], [sin(\\theta), cos(\\theta)]]",
        "f(x; b) -> y: matmul(R(b), [x, 1])_[2]",
    ])]);
    let f = &s.functions["f"];
    assert_eq!(f.calls, ["R"]);
    let mut seen = vec![];
    f.body.walk(&mut |e| {
        if let ExprKind::Call { callee, kind, .. } = &e.kind {
            seen.push((callee.key.clone(), *kind, e.ty.clone()));
        }
    });
    assert_eq!(
        seen,
        vec![
            ("matmul".into(), NameKind::Builtin, Ty::Vector { n: 2 }),
            ("R".into(), NameKind::Function, Ty::Matrix { m: 2, n: 2 }),
        ]
    );
    // Two inputs on a matrix function are fine (not drawn).
    build(&[&def(&["M(s, t) -> matrix: [[s, t], [t, s]]", "f(x) -> y: det(M(x, 1))"])]);
}
