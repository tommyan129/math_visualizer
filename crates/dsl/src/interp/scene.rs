//! Stage 3: scenes and steps (design.md "scene", "step").
//!
//! - `scene(n):` is validated with spans, then stored as an unresolved `SceneSource`.
//! - Every request re-resolves each Claude scene from its source. A reference that no
//!   longer resolves is removed from the source and reported; the scene itself stays
//!   (cascade exception), even when it becomes empty.
//! - Default scenes follow the coordinate systems of the functions: created with the
//!   first function of a system, removed with the last.
//! - Each scene caches the full state of every step plus what the step changed.

use std::collections::BTreeSet;

use super::hl::{literal, set_spans, unparen};
use super::*;
use crate::parser::parse_expr;
use crate::print::expr_dsl;

/// What one note string resolved to.
struct NoteOut {
    segs: Vec<NoteSeg>,
    /// Inner text of `{...}` segments that failed (cascade: they become plain text).
    failed: Vec<String>,
}

/// One scene resolved from its source.
struct Built {
    clip: Option<Clip>,
    view: Option<ViewSpec>,
    steps: Vec<StepState>,
    deps: Vec<String>,
}

fn shift_spans(e: &mut Expr, by: u32) {
    e.span = Span { start: e.span.start + by, end: e.span.end + by };
    for c in e.children_mut() {
        shift_spans(c, by);
    }
}

fn cleared(e: &Expr) -> Expr {
    let mut e = e.clone();
    e.clear_spans();
    e
}

/// `2D(x, y)` of the given axes.
fn key_of(axes: &[Ident]) -> String {
    system_key(&axes.iter().map(|a| a.key.as_str()).collect::<Vec<_>>())
}

/// Literal number bounds of an axis interval in a clip or view: `lo <= a <= hi`.
fn literal_intervals(e: &Expr) -> Vec<(String, f64, f64)> {
    let parts: Vec<&Expr> = match &unparen(e).kind {
        ExprKind::And { args } => args.iter().collect(),
        _ => vec![e],
    };
    let mut out = vec![];
    for p in parts {
        if let ExprKind::Compare { first, rest } = &unparen(p).kind
            && rest.len() == 2
            && let ExprKind::Name { name, .. } = &unparen(&rest[0].rhs).kind
            && let (Some(a), Some(b)) = (literal(first), literal(&rest[1].rhs))
        {
            let up = matches!(rest[0].op, CmpOp::Lt | CmpOp::Le) && matches!(rest[1].op, CmpOp::Lt | CmpOp::Le);
            let down = matches!(rest[0].op, CmpOp::Gt | CmpOp::Ge) && matches!(rest[1].op, CmpOp::Gt | CmpOp::Ge);
            if up {
                out.push((name.key.clone(), a, b));
            } else if down {
                out.push((name.key.clone(), b, a));
            }
        }
    }
    out
}

impl<'s> Cx<'s> {
    /// Runs `f` and takes the diagnostics it produced out of the request's list
    /// (cascade re-resolution: failures are not request errors, except resource limits).
    fn quiet<T>(&mut self, f: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        let before = self.diags.len();
        let r = f(self);
        let errs: Vec<Diagnostic> = self.diags.drain(before..).collect();
        if self.stopped {
            self.diags.extend(errs.into_iter().filter(|d| d.code == code::LIMIT));
            return None;
        }
        if errs.is_empty() { r } else { None }
    }

    // ----- placement -------------------------------------------------------

    /// Can `key` (a function or highlight) be shown in a scene over `axes`?
    fn placement(&self, key: &str, axes: &[String]) -> Result<(), String> {
        let sys = system_key(&axes.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        if let Some(own) = self.state.function_system(key) {
            return if own == sys { Ok(()) } else { Err(format!("'{key}'는 {own} 좌표계 함수라 {sys} scene에 넣을 수 없음")) };
        }
        if let Some(f) = self.state.functions.get(key) {
            return match f.vector_dim() {
                Some(n) if n as usize == axes.len() => Ok(()),
                Some(n) => Err(format!("{} '{key}'의 벡터 길이가 {n}이라 축 {}개인 {sys} scene에 넣을 수 없음", f.kind.word(), axes.len())),
                None => Err(format!("{} '{key}'는 그리는 정의가 아님", f.kind.word())),
            };
        }
        if let Some(h) = self.state.highlights.get(key) {
            if let HlTarget::Function { function } | HlTarget::Sub { function, .. } = &h.target {
                return self.placement(function, axes).map_err(|_| format!("'{key}'는 {} 좌표계 함수 {function}의 강조라 {sys} scene에 넣을 수 없음", self.state.function_system(function).unwrap_or_default()));
            }
            return match h.axes.iter().find(|a| !axes.contains(a)) {
                None => Ok(()),
                Some(a) => Err(format!("'{key}'의 범위에 축 {a}가 있어 {sys} scene에 넣을 수 없음")),
            };
        }
        Err(format!("'{key}'라는 함수나 강조가 없음"))
    }

    /// A `show` / `hide` / step-show name, with spans: reports and returns false when it cannot be placed.
    fn check_target(&mut self, name: &Ident, span: Span, axes: &[String]) -> bool {
        let k = &name.key;
        if self.failed.contains(k) {
            return false;
        }
        if !self.state.functions.contains_key(k) && !self.state.highlights.contains_key(k) {
            match self.state.kind_of(k) {
                Some(kind) => self.err(span, code::TARGET, format!("{kind} '{k}'는 scene에 넣을 수 없음 (함수, 강조)")),
                None => self.err(span, code::UNDECLARED, format!("'{k}'라는 함수나 강조가 없음")),
            }
            return false;
        }
        if let Some(f) = self.state.functions.get(k).filter(|f| !f.kind.is_drawn()) {
            let word = f.kind.word();
            self.err(span, code::TARGET, format!("{word} '{k}'는 그리는 정의가 아니라 scene에 넣을 수 없음 (함수, span, arrow, 강조)"));
            return false;
        }
        match self.placement(k, axes) {
            Ok(()) => true,
            Err(m) => {
                let sug = match (self.state.function_system(k), self.state.functions.get(k).and_then(|f| f.vector_dim())) {
                    (Some(s), _) => format!("space: {s}"),
                    (None, Some(3)) => "3D scene (6단계)".to_string(),
                    _ => "그 축을 쓰는 scene".to_string(),
                };
                self.err_s(span, code::PLACEMENT, m, sug);
                false
            }
        }
    }

    /// `set` / `animate` value: a number literal inside the constant's range.
    fn check_value(&mut self, name: &Ident, name_span: Span, value: &Expr) -> Option<f64> {
        let Some(c) = self.state.constants.get(&name.key).cloned() else {
            if !self.failed.contains(&name.key) {
                let what = self.state.kind_of(&name.key).map_or("없음".to_string(), |k| format!("{k}임"));
                self.err(name_span, code::UNDECLARED, format!("상수 '{}'가 {what}", name.key));
            }
            return None;
        };
        let Some(v) = literal(value) else {
            self.err_s(value.span, code::STEP, "숫자만 쓸 수 있음 (서버는 식의 값을 계산하지 않음)", "a = 2");
            return None;
        };
        if v < c.min || v > c.max {
            self.err_s(value.span, code::STEP, format!("{}={}은 슬라이더 범위 {}..{} 밖임", c.key, fmt_num(v), fmt_num(c.min), fmt_num(c.max)), format!("constant:\n    {}: range=(..., ...)로 범위를 넓히기", c.key));
            return None;
        }
        if c.integer && v.fract() != 0.0 {
            self.err(value.span, code::STEP, format!("'{}'는 정수 상수라 값도 정수여야 함", c.key));
            return None;
        }
        Some(v)
    }

    // ----- clip, view, note --------------------------------------------------

    fn resolve_clip(&mut self, e: &Expr, axes: &[String], deps: &mut BTreeSet<String>) -> Option<Clip> {
        let ax: Vec<&str> = axes.iter().map(|s| s.as_str()).collect();
        let mut consts = BTreeSet::new();
        let r = self.resolve_range(e, &ax, true, deps, &mut consts)?;
        deps.extend(consts.iter().cloned());
        Some(Clip { src: expr_dsl(e), expr: r, consts: consts.into_iter().collect() })
    }

    /// `-5 <= x <= 5 and -3 <= y <= 3`: one interval per axis, constant-free bounds.
    fn resolve_view(&mut self, e: &Expr, axes: &[String]) -> Option<ViewSpec> {
        let shape = |cx: &mut Self, at: Span| {
            cx.err_s(at, code::VIEW, "view는 축별 구간 (카메라 창은 직사각형)", format!("-5 <= {} <= 5 and -3 <= {} <= 3", axes[0], axes[1]));
        };
        let parts: Vec<&Expr> = match &unparen(e).kind {
            ExprKind::And { args } => args.iter().collect(),
            ExprKind::Compare { .. } => vec![e],
            _ => {
                shape(self, e.span);
                return None;
            }
        };
        let mut spec = ViewSpec { h: None, v: None, src: expr_dsl(e) };
        for p in parts {
            let ExprKind::Compare { first, rest } = &unparen(p).kind else {
                shape(self, p.span);
                return None;
            };
            let axis = match rest.as_slice() {
                [a, _] => match &unparen(&a.rhs).kind {
                    ExprKind::Name { name, .. } if axes.contains(&name.key) => Some(name.key.clone()),
                    _ => None,
                },
                _ => None,
            };
            let up = rest.iter().all(|l| matches!(l.op, CmpOp::Lt | CmpOp::Le));
            let down = rest.iter().all(|l| matches!(l.op, CmpOp::Gt | CmpOp::Ge));
            let Some(axis) = axis.filter(|_| up || down) else {
                shape(self, p.span);
                return None;
            };
            let (mut lo, mut hi) = ((**first).clone(), rest[1].rhs.clone());
            if down {
                std::mem::swap(&mut lo, &mut hi);
            }
            for b in [&mut lo, &mut hi] {
                let mut bad = None;
                b.walk(&mut |n| match &n.kind {
                    ExprKind::Name { name, .. } if !is_reserved(name) => bad = bad.clone().or(Some((name.key.clone(), n.span))),
                    ExprKind::Call { callee, .. } if names::builtin_arity(&callee.key).is_none() => bad = bad.clone().or(Some((callee.key.clone(), n.span))),
                    _ => {}
                });
                if let Some((k, at)) = bad {
                    self.err_s(at, code::VIEW, format!("view 경계에는 숫자만 씀 ('{k}' 불가: 카메라는 처음 한 번만 맞춤)"), "-2*\\pi 같은 숫자 식");
                    return None;
                }
                let mut sc = Scope::range(&[]);
                let mut info = DefInfo::default();
                let before = self.diags.len();
                let ty = self.resolve(b, &mut sc, &mut info);
                if self.diags.len() > before {
                    return None;
                }
                if !matches!(ty, Ty::Scalar) {
                    self.err(b.span, code::VIEW, "view 경계는 숫자여야 함");
                    return None;
                }
                b.assign_ids();
                b.clear_spans();
            }
            if let (Some(a), Some(z)) = (literal(&lo), literal(&hi))
                && a >= z
            {
                self.err(p.span, code::VIEW, format!("view 구간 {}..{}이 비어 있음", fmt_num(a), fmt_num(z)));
                return None;
            }
            let slot = if axis == axes[0] { &mut spec.h } else { &mut spec.v };
            if slot.is_some() {
                self.err(p.span, code::VIEW, format!("축 {axis}의 구간을 두 번 적음"));
                return None;
            }
            *slot = Some(ViewRange { lo, hi });
        }
        Some(spec)
    }

    /// Note string (or a string variable): `{expr}` is DSL, `$...$` LaTeX, `{{` `}}` `$$` literal.
    /// `strict`: errors are reported (scene definition); otherwise failed `{...}` parts are
    /// returned in `failed` (cascade).
    fn resolve_note(&mut self, value: &Expr, axes: &[String], strict: bool, deps: &mut BTreeSet<String>) -> Option<NoteOut> {
        let at = value.span;
        let (raw, base) = match &value.kind {
            ExprKind::Str { raw } => (raw.clone(), strict.then_some(at.start + 1)),
            ExprKind::Name { name, .. } => match self.state.vars.get(&name.key).map(|v| v.value.kind.clone()) {
                Some(ExprKind::Str { raw }) => {
                    deps.insert(name.key.clone());
                    (raw, None)
                }
                Some(_) => {
                    self.err(at, code::NOTE, format!("변수 '{}'의 값이 문자열이 아님", name.key));
                    return None;
                }
                None => {
                    self.err_s(at, code::NOTE, format!("'{}'라는 변수가 없음", name.key), format!("note: \"...\" 또는 {} = \"...\"를 먼저", name.key));
                    return None;
                }
            },
            _ => {
                self.err_s(at, code::NOTE, "note에는 문자열이나 문자열 변수", "note: \"{f}의 접선\"");
                return None;
            }
        };
        let pos = |i: usize| match base {
            Some(b) => Span::new(b as usize + i, b as usize + i + 1),
            None => at,
        };
        let b = raw.as_bytes();
        let mut out = NoteOut { segs: vec![], failed: vec![] };
        let mut text = String::new();
        let mut i = 0;
        let flush = |text: &mut String, segs: &mut Vec<NoteSeg>| {
            if !text.is_empty() {
                segs.push(NoteSeg::Text { text: std::mem::take(text) });
            }
        };
        while i < b.len() {
            match b[i] {
                b'\\' if matches!(b.get(i + 1), Some(b'"') | Some(b'\\')) => {
                    text.push(b[i + 1] as char);
                    i += 2;
                }
                b'{' if b.get(i + 1) == Some(&b'{') => {
                    text.push('{');
                    i += 2;
                }
                b'}' if b.get(i + 1) == Some(&b'}') => {
                    text.push('}');
                    i += 2;
                }
                b'$' if b.get(i + 1) == Some(&b'$') => {
                    text.push('$');
                    i += 2;
                }
                b'{' => {
                    let mut depth = 0;
                    let mut j = i;
                    while j < b.len() {
                        match b[j] {
                            b'{' => depth += 1,
                            b'}' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    if j >= b.len() {
                        self.err_s(pos(i), code::NOTE, "note의 '{'가 닫히지 않음", "{식}, 글자 그대로는 {{");
                        return None;
                    }
                    let inner = &raw[i + 1..j];
                    let off = base.map(|b| b + i as u32 + 1);
                    let seg = if strict { self.note_math(inner, off, at, axes, deps) } else { self.quiet(|cx| cx.note_math(inner, off, at, axes, deps)) };
                    match seg {
                        Some(seg) => {
                            flush(&mut text, &mut out.segs);
                            out.segs.push(seg);
                        }
                        None if strict => return None,
                        None => {
                            out.failed.push(inner.to_string());
                            text.push_str(&unescape(inner));
                        }
                    }
                    i = j + 1;
                }
                b'$' => {
                    let Some(j) = raw[i + 1..].find('$').map(|j| i + 1 + j) else {
                        self.err_s(pos(i), code::NOTE, "note의 '$'가 닫히지 않음", "$LaTeX$, 글자 그대로는 $$");
                        return None;
                    };
                    flush(&mut text, &mut out.segs);
                    out.segs.push(NoteSeg::Tex { tex: unescape(&raw[i + 1..j]) });
                    i = j + 1;
                }
                _ => {
                    let c = raw[i..].chars().next().unwrap();
                    text.push(c);
                    i += c.len_utf8();
                }
            }
        }
        flush(&mut text, &mut out.segs);
        Some(out)
    }

    /// `{expr}` inside a note. `base`: byte offset of `inner` in the request (literal notes).
    fn note_math(&mut self, inner: &str, base: Option<u32>, at: Span, axes: &[String], deps: &mut BTreeSet<String>) -> Option<NoteSeg> {
        let src = unescape(inner);
        let place = |d: &mut Diagnostic| {
            d.span = match base {
                Some(b) => Span { start: d.span.start + b, end: d.span.end + b },
                None => at,
            };
        };
        let (e, diags) = parse_expr(&src);
        if !diags.is_empty() || e.is_none() {
            for mut d in diags {
                place(&mut d);
                d.message = format!("note의 {{{inner}}}: {}", d.message);
                self.diags.push(d);
            }
            return None;
        }
        let mut e = e.unwrap();
        match base {
            Some(b) => shift_spans(&mut e, b),
            None => set_spans(&mut e, at),
        }
        if let ExprKind::Name { name, .. } = &e.kind
            && self.state.functions.contains_key(&name.key)
        {
            deps.insert(name.key.clone());
            return Some(NoteSeg::Def { function: name.key.clone() });
        }
        let before = self.diags.len();
        self.expand(&mut e, deps, &mut vec![], false);
        let ax: Vec<&str> = axes.iter().map(|s| s.as_str()).collect();
        let mut sc = Scope::range(&ax);
        let mut info = DefInfo::default();
        let ty = self.resolve(&mut e, &mut sc, &mut info);
        if ty == Ty::Str {
            self.err(e.span, code::NOTE, "note의 {...} 안에는 식을 씀 (문자열은 바깥에)");
        }
        if self.diags.len() > before {
            for d in &mut self.diags[before..] {
                d.message = format!("note의 {{{inner}}}: {}", d.message);
            }
            return None;
        }
        deps.extend(info.callees.iter().cloned());
        deps.extend(info.used_consts.iter().cloned());
        e.assign_ids();
        e.clear_spans();
        Some(NoteSeg::Math { expr: e })
    }

    // ----- scene(n): ----------------------------------------------------------

    pub(super) fn scene(&mut self, def: &SceneDef) {
        let before = self.diags.len();
        let n = match &def.id {
            SceneRef::Number(n) => *n,
            SceneRef::System(sp) => {
                self.err_s(def.id_span, code::SCENE, format!("기본 scene({})은 서버가 관리해서 정의할 수 없음", sp.key()), "scene(1):처럼 번호를 붙인 scene");
                return;
            }
        };
        let key = n.to_string();
        let (mut space, mut show, mut hide, mut clip, mut view) = (None, None, None, None, None);
        let mut seen: Vec<&str> = vec![];
        for f in &def.fields {
            if seen.contains(&f.key.as_str()) {
                self.err(f.key_span, code::DUPLICATE, format!("'{}' 칸을 두 번 적음", f.key));
                continue;
            }
            seen.push(&f.key);
            match (f.key.as_str(), &f.value) {
                ("space", FieldValue::Space(sp)) => space = Some(sp),
                ("show", FieldValue::Names(v)) => show = Some(v),
                ("hide", FieldValue::Names(v)) => hide = Some(v),
                ("clip", FieldValue::Expr(e)) => clip = Some(e),
                ("view", FieldValue::Expr(e)) => view = Some(e),
                _ => {}
            }
        }
        let axes: Vec<Ident> = match space {
            Some(sp) => {
                match sp.kind.as_str() {
                    "2D" => {}
                    "3D" => self.unsupported(sp.span, "3D 좌표계", 6),
                    "polar" => self.unsupported(sp.span, "극좌표계", 5),
                    k => self.err_s(sp.span, code::SCENE, format!("좌표계 '{k}'는 없음"), "2D(x, y)"),
                }
                if sp.axes.len() != 2 && sp.kind == "2D" {
                    self.err_s(sp.span, code::SCENE, format!("2D 좌표계에는 축 이름 두 개가 필요함 ({}개 줌)", sp.axes.len()), "2D(x, y)");
                } else if sp.axes.len() == 2 && sp.axes[0].0.key == sp.axes[1].0.key {
                    self.err_s(sp.span, code::SCENE, "두 축의 이름이 같음", "2D(x, y)");
                } else if let Some((a, s)) = sp.axes.iter().find(|(a, _)| a.deriv > 0 || a.inverse || is_reserved(a)) {
                    self.err(*s, code::SCENE, format!("'{}'는 축 이름으로 쓸 수 없음", a.key));
                }
                sp.axes.iter().map(|(a, _)| a.clone()).collect()
            }
            None => {
                // One coordinate system among the `show` targets: take it.
                let systems: BTreeSet<String> = show.into_iter().flatten().filter_map(|(n, _)| self.state.function_system(&n.key)).collect();
                let flat_2d = show.into_iter().flatten().any(|(n, _)| self.state.functions.get(&n.key).and_then(|f| f.vector_dim()) == Some(2));
                match (systems.len(), show.into_iter().flatten().find_map(|(n, _)| self.state.functions.get(&n.key).and_then(|f| f.curve_axes()))) {
                    (1, Some((i, o))) => vec![i.clone(), o.clone()],
                    // Only spans / arrows of length-2 vectors: the plain plane.
                    (0, _) if flat_2d => vec![Ident::plain("x"), Ident::plain("y")],
                    _ => {
                        self.err_s(def.id_span, code::SCENE, "scene의 좌표계(space)가 필요함", format!("scene({key}):\n    space: 2D(x, y)"));
                        vec![]
                    }
                }
            }
        };
        if self.diags.len() > before {
            return;
        }
        let ak: Vec<String> = axes.iter().map(|a| a.key.clone()).collect();
        let mut src = SceneSource { axes: axes.clone(), ..Default::default() };
        let mut deps = BTreeSet::new();
        for (name, span) in show.into_iter().flatten() {
            if self.check_target(name, *span, &ak) && !src.show.iter().any(|n| n.key == name.key) {
                src.show.push(name.clone());
            }
        }
        for (name, span) in hide.into_iter().flatten() {
            if !self.check_target(name, *span, &ak) {
                continue;
            }
            if src.show.iter().any(|n| n.key == name.key) {
                self.err_s(*span, code::SCENE, format!("'{}'가 show와 hide에 같이 있음", name.key), "hide는 나중 step에서 +로 공개할 대상");
            } else if !src.hide.iter().any(|n| n.key == name.key) {
                src.hide.push(name.clone());
            }
        }
        if let Some(e) = clip
            && self.resolve_clip(e, &ak, &mut deps).is_some()
        {
            src.clip = Some(cleared(e));
        }
        let mut views: Vec<(&Expr, Span)> = vec![];
        if let Some(e) = view
            && self.resolve_view(e, &ak).is_some()
        {
            src.view = Some(cleared(e));
            views.push((e, e.span));
        }
        for st in &def.steps {
            let mut ss = StepSource::default();
            let mut seen: Vec<&str> = vec![];
            for f in &st.fields {
                if seen.contains(&f.key.as_str()) {
                    self.err(f.key_span, code::DUPLICATE, format!("step 안에 '{}' 칸을 두 번 적음", f.key));
                    continue;
                }
                seen.push(&f.key);
                match &f.value {
                    FieldValue::Toggles(v) => {
                        for (on, name, span) in v {
                            if self.check_target(name, *span, &ak) {
                                ss.show.retain(|t| t.name.key != name.key);
                                ss.show.push(StepToggle { on: *on, name: name.clone() });
                            }
                        }
                    }
                    FieldValue::Set(items) => {
                        for it in items {
                            if ss.set.iter().any(|s| s.name.key == it.name.key) {
                                self.err(it.name_span, code::DUPLICATE, format!("'{}'를 두 번 적음", it.name.key));
                            } else if let Some(v) = self.check_value(&it.name, it.name_span, &it.value) {
                                ss.set.push(SetSource { name: it.name.clone(), value: v });
                            }
                        }
                    }
                    FieldValue::Animate(a) => {
                        let from = self.check_value(&a.name, a.name_span, &a.from);
                        let to = self.check_value(&a.name, a.name_span, &a.to);
                        let duration = match &a.duration {
                            None => Some(ANIMATE_SECONDS),
                            Some((t, span)) => match t.parse::<f64>() {
                                Ok(d) if d > 0.0 && d <= 60.0 => Some(d),
                                _ => {
                                    self.err_s(*span, code::STEP, "재생 시간은 0초보다 길고 60초 이하", "2s");
                                    None
                                }
                            },
                        };
                        if ss.set.iter().any(|s| s.name.key == a.name.key) {
                            self.err(a.name_span, code::STEP, format!("'{}'를 같은 step의 set과 animate에 같이 씀", a.name.key));
                        } else if let (Some(from), Some(to), Some(duration)) = (from, to, duration) {
                            ss.animate = Some(AnimateSource { name: a.name.clone(), from, to, duration, looped: a.looped });
                        }
                    }
                    FieldValue::Expr(e) => {
                        if self.resolve_view(e, &ak).is_some() {
                            ss.view = Some(cleared(e));
                            views.push((e, e.span));
                        }
                    }
                    FieldValue::Note { append, value } if self.resolve_note(value, &ak, true, &mut deps).is_some() => {
                        ss.note = Some(NoteSource { append: *append, value: cleared(value) });
                    }
                    _ => {}
                }
            }
            src.steps.push(ss);
        }
        // view and clip that can never overlap (both literal per-axis intervals).
        if let Some(c) = clip {
            let ci = literal_intervals(c);
            for (v, span) in &views {
                for (axis, a, b) in literal_intervals(v) {
                    if let Some((_, c0, c1)) = ci.iter().find(|(k, _, _)| *k == axis)
                        && (b < *c0 || a > *c1)
                    {
                        self.err_s(*span, code::VIEW, format!("view의 {axis} 구간 {}..{}이 clip {}..{}과 전혀 겹치지 않아 아무것도 안 보임", fmt_num(a), fmt_num(b), fmt_num(*c0), fmt_num(*c1)), "view나 clip 범위를 겹치게");
                    }
                }
            }
        }
        if self.diags.len() > before {
            return;
        }
        let old = self.state.scenes.get(&key).cloned();
        let built = self.build(&key, &mut src, false);
        let mut sc = Scene {
            key: key.clone(),
            number: Some(n),
            axes,
            clip: built.clip,
            view: built.view,
            steps: built.steps,
            src: self.text(def.span).to_string(),
            deps: built.deps,
            ..Default::default()
        };
        match &old {
            Some(o) => {
                sc.order = o.order;
                sc.version = o.version;
                sc.toggles = o.toggles.clone();
                // The user's clip stays only while Claude's clip is the same.
                sc.user_clip = if o.clip.as_ref().map(|c| &c.src) == sc.clip.as_ref().map(|c| &c.src) { o.user_clip.clone() } else { None };
                let same = o.source.as_ref() == Some(&src) && o.axes == sc.axes;
                sc.defined = if same { o.defined } else { self.seq };
                self.notes.push(if same { Note::SceneUnchanged { key: key.clone() } } else { Note::SceneModified { key: key.clone() } });
            }
            None => {
                sc.order = self.next_scene_order();
                sc.defined = self.seq;
                self.notes.push(Note::SceneCreated { key: key.clone() });
            }
        }
        sc.source = Some(src);
        self.state.scenes.insert(key, sc);
    }

    fn next_scene_order(&mut self) -> u64 {
        let o = self.state.counters.next_order;
        self.state.counters.next_order += 1;
        o
    }

    pub(super) fn del_scene(&mut self, r: &SceneRef, span: Span) {
        match r {
            SceneRef::Number(n) => {
                let key = n.to_string();
                if self.state.scenes.remove(&key).is_some() {
                    self.notes.push(Note::SceneDeleted { key });
                } else {
                    self.err(span, code::UNDECLARED, format!("scene({key})이 없음"));
                }
            }
            SceneRef::System(sp) => {
                self.err(span, code::SCENE, format!("기본 scene({})은 지울 수 없음 (그 좌표계의 함수가 모두 지워지면 자동으로 사라짐)", sp.key()));
            }
        }
    }

    /// Resolves a scene source against the current state. `prune`: references that no
    /// longer resolve are removed from `src` and reported (cascade); otherwise the source
    /// was just validated and everything resolves.
    fn build(&mut self, key: &str, src: &mut SceneSource, prune: bool) -> Built {
        let ak: Vec<String> = src.axes.iter().map(|a| a.key.clone()).collect();
        let mut deps: BTreeSet<String> = BTreeSet::new();
        let mut removed: Vec<(String, String)> = vec![];
        let place_ok = |cx: &Self, n: &Ident| cx.placement(&n.key, &ak).is_ok();
        for (list, place) in [(&mut src.show, "show"), (&mut src.hide, "hide")] {
            list.retain(|n| {
                let ok = place_ok(self, n);
                if !ok {
                    removed.push((n.key.clone(), place.to_string()));
                }
                ok
            });
        }
        deps.extend(src.show.iter().chain(&src.hide).map(|n| n.key.clone()));
        let clip = match src.clip.clone() {
            None => None,
            Some(e) => {
                let mut d = BTreeSet::new();
                match self.quiet(|cx| cx.resolve_clip(&e, &ak, &mut d)) {
                    Some(c) => {
                        deps.extend(d);
                        Some(c)
                    }
                    None => {
                        removed.push((expr_dsl(&e), "clip".into()));
                        src.clip = None;
                        None
                    }
                }
            }
        };
        let view = src.view.clone().and_then(|e| self.quiet(|cx| cx.resolve_view(&e, &ak)));

        let mut cur = StepState::default();
        for n in &src.show {
            cur.visible.insert(n.key.clone(), true);
        }
        for n in &src.hide {
            cur.visible.insert(n.key.clone(), false);
            cur.concealed.push(n.key.clone());
        }
        cur.view = view.clone();
        let mut steps = vec![cur.clone()];
        for (i, st) in src.steps.iter_mut().enumerate() {
            let place = |what: &str| format!("step {} {what}", i + 1);
            let mut next = StepState { changed: vec![], changed_values: vec![], changed_view: false, changed_note: false, animate: vec![], ..cur.clone() };
            st.show.retain(|t| {
                let ok = place_ok(self, &t.name);
                if !ok {
                    removed.push((t.name.key.clone(), place("show")));
                }
                ok
            });
            for t in &st.show {
                next.visible.insert(t.name.key.clone(), t.on);
                next.changed.push(t.name.key.clone());
                if t.on {
                    next.concealed.retain(|k| *k != t.name.key);
                }
                deps.insert(t.name.key.clone());
            }
            let constants = &self.state.constants;
            let fits = |k: &str, v: f64| constants.get(k).is_some_and(|c| (c.min..=c.max).contains(&v) && (!c.integer || v.fract() == 0.0));
            st.set.retain(|s| {
                let ok = fits(&s.name.key, s.value);
                if !ok {
                    removed.push((s.name.key.clone(), place("set")));
                }
                ok
            });
            for s in &st.set {
                next.values.insert(s.name.key.clone(), s.value);
                next.changed_values.push(s.name.key.clone());
                deps.insert(s.name.key.clone());
            }
            if let Some(a) = &st.animate {
                if fits(&a.name.key, a.from) && fits(&a.name.key, a.to) {
                    next.values.insert(a.name.key.clone(), a.to);
                    next.changed_values.push(a.name.key.clone());
                    next.animate.push(Animate { constant: a.name.key.clone(), from: a.from, to: a.to, duration: a.duration, looped: a.looped });
                    deps.insert(a.name.key.clone());
                } else {
                    removed.push((a.name.key.clone(), place("animate")));
                    st.animate = None;
                }
            }
            if let Some(e) = st.view.clone()
                && let Some(v) = self.quiet(|cx| cx.resolve_view(&e, &ak))
            {
                next.view = Some(v);
                next.changed_view = true;
            }
            if let Some(n) = st.note.clone() {
                let mut d = BTreeSet::new();
                match self.quiet(|cx| cx.resolve_note(&n.value, &ak, false, &mut d)) {
                    Some(out) => {
                        deps.extend(d);
                        if !out.failed.is_empty()
                            && let ExprKind::Str { raw } = &n.value.kind
                        {
                            // `{f(2)}` whose `f` is gone stays as the plain text `f(2)`.
                            let mut raw = raw.clone();
                            for inner in &out.failed {
                                raw = raw.replacen(&format!("{{{inner}}}"), inner, 1);
                                removed.push((inner.clone(), place("note")));
                            }
                            st.note = Some(NoteSource { append: n.append, value: Expr { kind: ExprKind::Str { raw }, ..n.value.clone() } });
                        }
                        let text = match &st.note.as_ref().unwrap().value.kind {
                            ExprKind::Str { raw } => unescape(raw),
                            _ => out.segs.iter().map(|s| match s { NoteSeg::Text { text } => text.clone(), _ => String::new() }).collect(),
                        };
                        let block = NoteBlock { segs: out.segs, text };
                        if n.append {
                            next.note.push(block);
                        } else {
                            next.note = vec![block];
                        }
                        next.changed_note = true;
                    }
                    None => {
                        let what = match &n.value.kind {
                            ExprKind::Name { name, .. } => name.key.clone(),
                            _ => "note".into(),
                        };
                        removed.push((what, place("note")));
                        st.note = None;
                    }
                }
            }
            steps.push(next.clone());
            cur = next;
        }
        if prune {
            for (target, place) in removed {
                self.notes.push(Note::SceneRefRemoved { scene: key.to_string(), target, place });
            }
        }
        Built { clip, view, steps, deps: deps.into_iter().collect() }
    }

    /// Re-resolves every Claude scene from its source (cascade without deleting the scene).
    pub(super) fn refresh_scenes(&mut self) {
        let mut keys: Vec<(u64, String)> = self.state.scenes.values().filter(|s| !s.is_default()).map(|s| (s.order, s.key.clone())).collect();
        keys.sort();
        for (_, key) in keys {
            let Some(mut src) = self.state.scenes[&key].source.clone() else { continue };
            let built = self.build(&key, &mut src, true);
            if self.stopped {
                return;
            }
            let s = self.state.scenes.get_mut(&key).unwrap();
            if s.source.as_ref() != Some(&src) {
                s.defined = self.seq;
            }
            s.clip = built.clip;
            s.view = built.view;
            s.steps = built.steps;
            s.deps = built.deps;
            s.source = Some(src);
            // A user clip that no longer resolves goes back to Claude's.
            if let Some(uc) = s.user_clip.clone() {
                let ak: Vec<String> = s.axes.iter().map(|a| a.key.clone()).collect();
                let e = uc.expr.clone();
                if self.quiet(|cx| cx.resolve_clip(&e, &ak, &mut BTreeSet::new())).is_none() {
                    self.state.scenes.get_mut(&key).unwrap().user_clip = None;
                }
            }
        }
    }

    /// One default scene per coordinate system in use; members and toggles of every scene.
    pub(super) fn sync_default_scenes(&mut self) {
        let mut systems: Vec<(u64, String, Vec<Ident>)> = vec![];
        let mut fns: Vec<&Function> = self.state.functions.values().filter(|f| f.kind.is_drawn()).collect();
        fns.sort_by_key(|f| f.order);
        for f in fns {
            // Spans and arrows of length-2 vectors go in the default `2D(x, y)`; length 3
            // waits for 3D (stage 6).
            let axes = match (f.curve_axes(), f.vector_dim()) {
                (Some((i, o)), _) => vec![i.clone(), o.clone()],
                (None, Some(2)) => vec![Ident::plain("x"), Ident::plain("y")],
                _ => continue,
            };
            let k = key_of(&axes);
            if !systems.iter().any(|(_, s, _)| *s == k) {
                systems.push((f.order, k, axes));
            }
        }
        let gone: Vec<String> = self.state.scenes.values().filter(|s| s.is_default() && !systems.iter().any(|(_, k, _)| *k == s.key)).map(|s| s.key.clone()).collect();
        for k in gone {
            self.state.scenes.remove(&k);
            self.notes.push(Note::DefaultSceneRemoved { key: k });
        }
        for (_, k, axes) in systems {
            if !self.state.scenes.contains_key(&k) {
                let order = self.next_scene_order();
                self.state.scenes.insert(k.clone(), Scene { key: k.clone(), axes, steps: vec![StepState::default()], defined: self.seq, order, ..Default::default() });
                self.notes.push(Note::DefaultSceneCreated { key: k });
            }
        }
        // Members in creation order (functions and highlights share the order counter).
        let mut items: Vec<(u64, String)> = self.state.functions.values().map(|f| (f.order, f.key.clone())).chain(self.state.highlights.values().map(|h| (h.order, h.key.clone()))).collect();
        items.sort();
        let keys: Vec<String> = self.state.scenes.keys().cloned().collect();
        for k in keys {
            let ak: Vec<String> = self.state.scenes[&k].axes.iter().map(|a| a.key.clone()).collect();
            let members: Vec<String> = items.iter().filter(|(_, i)| self.placement(i, &ak).is_ok()).map(|(_, i)| i.clone()).collect();
            let fnset: Vec<String> = members.iter().filter(|m| self.state.functions.contains_key(*m)).cloned().collect();
            let s = self.state.scenes.get_mut(&k).unwrap();
            if s.is_default() {
                // Functions start on, highlights off (design.md "기본 scene").
                if s.steps.is_empty() {
                    s.steps.push(StepState::default());
                }
                s.steps[0].visible = fnset.into_iter().map(|f| (f, true)).collect();
            }
            s.toggles.retain(|t, _| members.contains(t));
            s.members = members;
        }
    }
}

/// A detached interpreter context for one expression (user clip).
fn detached<'s>(src: &'s str, state: &State) -> Cx<'s> {
    let limits = Limits::default();
    Cx {
        src,
        state: state.clone(),
        notes: vec![],
        diags: vec![],
        failed: HashSet::new(),
        fuel_left: limits.fuel,
        fuel_limit: limits.fuel,
        deadline: Instant::now() + limits.wall_clock,
        wall_clock: limits.wall_clock,
        stopped: false,
        seq: 0,
        declared_now: BTreeSet::new(),
    }
}

/// The user's clip text for `scene`, resolved over its axes.
pub(super) fn user_clip(state: &State, scene: &str, text: &str) -> Result<Clip, String> {
    let s = state.scenes.get(scene).ok_or_else(|| format!("scene({scene})이 없음"))?;
    let ak: Vec<String> = s.axes.iter().map(|a| a.key.clone()).collect();
    let (e, diags) = parse_expr(text);
    let idx = crate::span::LineIndex::new(text);
    if let Some(d) = diags.first() {
        return Err(d.render(&idx));
    }
    let e = e.ok_or("clip이 비어 있음")?;
    let mut cx = detached(text, state);
    match cx.resolve_clip(&e, &ak, &mut BTreeSet::new()) {
        Some(c) if cx.diags.is_empty() => Ok(c),
        _ => Err(cx.diags.first().map_or("clip을 해석할 수 없음".into(), |d| d.render(&idx))),
    }
}

/// Moves `scene` from step `from` to step `to` (one step at a time). Forward into step k
/// and back out of step k both clear the user overrides of the targets step k mentions;
/// constants take the values step k sets (forward) or the earlier step's value (back;
/// unchanged when no earlier step set them). Returns the applied result (possibly empty).
pub fn navigate(state: &State, scene: &str, from: usize, to: usize, seq: u64) -> Result<Applied, String> {
    let mut new = state.clone();
    let s = new.scenes.get(scene).ok_or_else(|| format!("scene({scene})이 없음"))?.clone();
    let last = s.step_count();
    let (from, to) = (from.min(last), to.min(last));
    let mut values: Vec<(String, f64)> = vec![];
    let mut cleared: Vec<String> = vec![];
    if to > from {
        for k in from + 1..=to {
            cleared.extend(s.steps[k].changed.iter().cloned());
            values.extend(s.steps[k].changed_values.iter().filter_map(|c| s.steps[k].values.get(c).map(|v| (c.clone(), *v))));
        }
    } else {
        for k in (to + 1..=from).rev() {
            cleared.extend(s.steps[k].changed.iter().cloned());
            values.extend(s.steps[k].changed_values.iter().filter_map(|c| s.steps[k - 1].values.get(c).map(|v| (c.clone(), *v))));
        }
    }
    let sc = new.scenes.get_mut(scene).unwrap();
    for t in &cleared {
        sc.toggles.remove(t);
    }
    for (c, v) in values {
        if let Some(k) = new.constants.get_mut(&c) {
            k.value = v.clamp(k.min, k.max);
        }
    }
    let changes = new.diff_from(state, seq);
    Ok(Applied { state: new, changes, notes: vec![] })
}
