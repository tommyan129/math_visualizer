//! MCP response text: short fixed sections, DSL notation, only changes and what
//! the server did on its own; never an echo of the request (design.md "MCP tool").

use dsl::interp::{Note, ObjKind, fmt_num};
use dsl::state::{BASE_STYLES, Constant, Highlight, NoteSeg, Request, Scene, State};
use model::protocol::{FeatureClass, FeaturePoints, ReqStatus, ReqView, SpanDim, ViewBox, Warning};

use crate::engine::{ApplyError, ApplyOk, KeyError, RequestsOut, WorkspaceView};

/// What the browser said (or did not say) about a workspace.
#[derive(Debug, Clone)]
pub enum Browser {
    NotConnected,
    /// The active tab shows another workspace.
    OtherWorkspace,
    /// Connected but no answer in time.
    Timeout,
    Rendered { warnings: Vec<Warning>, spans: Vec<SpanDim>, features: Vec<FeaturePoints> },
    View { view: ViewBox, warnings: Vec<Warning>, spans: Vec<SpanDim>, features: Vec<FeaturePoints> },
}

pub fn key_error(e: &KeyError) -> String {
    match e {
        KeyError::Invalid => "[E-key] key가 교체됐거나 없음 → 사용자에게 이 작업 공간의 현재 key를 물어볼 것".into(),
        KeyError::RateLimited => "[E-key-limit] 틀린 key 시도가 많아 1분 동안 key 확인을 막음 → 잠시 뒤 다시".into(),
    }
}

pub fn apply_err(e: &ApplyError) -> String {
    match e {
        ApplyError::Key(k) => key_error(k),
        ApplyError::Rejected(lines) => format!("적용 안 됨 (오류 {}개, 아무것도 바뀌지 않음)\n{}", lines.len(), lines.join("\n")),
        ApplyError::Internal(m) => format!("[E-internal] {m}"),
    }
}

fn constant_text(c: &Constant) -> String {
    let kind = if c.integer { "정수, " } else { "" };
    let step = if c.declared && !c.integer { format!(", 간격 {}", fmt_num(c.step)) } else { String::new() };
    format!("{} = {} ({kind}{}..{}{step})", c.key, fmt_num(c.value), fmt_num(c.min), fmt_num(c.max))
}

/// `blue` for a base color, else the hex value.
fn color_name(hex: &str) -> String {
    BASE_STYLES.iter().find(|(_, c)| *c == hex).map_or(hex.to_string(), |(n, _)| n.to_string())
}

fn highlight_text(h: &Highlight) -> String {
    let auto = if h.style_name.is_none() { format!(" · 자동 색 {}", color_name(&h.style.color)) } else { String::new() };
    format!("{} = {}{auto}", h.key, h.src)
}

/// `직선 (1차원)` for a span drawn as a line.
fn span_dim_word(dim: u32) -> &'static str {
    match dim {
        0 => "점 (0차원, 원점)",
        1 => "직선 (1차원)",
        2 => "평면 (2차원)",
        _ => "공간 (3차원)",
    }
}

fn warnings_text(w: &[Warning]) -> String {
    if w.is_empty() {
        return "경고 없음".into();
    }
    let items: Vec<String> = w
        .iter()
        .map(|w| match &w.source {
            Some(s) => format!("{s}: {} [{}]", w.message, w.code),
            None => format!("{} [{}]", w.message, w.code),
        })
        .collect();
    format!("경고 {}개\n  {}", w.len(), items.join("\n  "))
}

/// Most points listed per feature-point definition.
const FEATURE_LIST_MAX: usize = 12;

/// `극대 (-1, 2), 극소 (1, -2)`; a vertical asymptote as `수직 점근선 x = 1`.
fn feature_text(s: &State, fp: &FeaturePoints) -> String {
    if fp.points.is_empty() {
        return "찾은 점 없음".into();
    }
    let axis = s.functions.get(&fp.key).and_then(|f| f.feature.as_ref()).and_then(|sp| sp.axes.first()).map_or("x".to_string(), |a| a.key.clone());
    let mut items: Vec<String> = fp
        .points
        .iter()
        .take(FEATURE_LIST_MAX)
        .map(|p| match (p.class, p.y) {
            (FeatureClass::Asymptote, _) | (_, None) => format!("{} {axis} = {}", p.class.word(), fmt_num(p.x)),
            (c, Some(y)) => format!("{} ({}, {})", c.word(), fmt_num(p.x), fmt_num(y)),
        })
        .collect();
    if fp.points.len() > FEATURE_LIST_MAX || fp.capped {
        items.push(format!("… (모두 {}개{})", fp.points.len(), if fp.capped { " 이상" } else { "" }));
    }
    format!("{}개: {}", fp.points.len(), items.join(", "))
}

pub fn browser_line(b: &Browser) -> String {
    match b {
        Browser::NotConnected => "브라우저: 미연결 (화면 확인은 연결 후 get_state로)".into(),
        Browser::OtherWorkspace => "브라우저: 다른 작업 공간을 보고 있음 (화면 확인은 get_state로)".into(),
        Browser::Timeout => "브라우저: 응답 없음 (렌더 결과는 get_state로)".into(),
        // `get_state` marks each span in the definition list instead.
        Browser::View { warnings, .. } => format!("브라우저: {}", warnings_text(warnings)),
        Browser::Rendered { warnings, spans, .. } => {
            let mut t = format!("브라우저: {}", warnings_text(warnings));
            if !spans.is_empty() {
                t.push_str(&format!("\nspan: {}", spans.iter().map(|d| format!("{} {}", d.key, span_dim_word(d.dim))).collect::<Vec<_>>().join(", ")));
            }
            t
        }
    }
}

/// `특징점: E 2개: 극대 (-1, 2), ...` lines after an `apply` that drew some.
fn feature_lines(s: &State, b: &Browser) -> Vec<String> {
    match b {
        Browser::Rendered { features, .. } => features.iter().map(|fp| format!("특징점 {}: {}", fp.key, feature_text(s, fp))).collect(),
        _ => vec![],
    }
}

pub fn apply_ok(ok: &ApplyOk, browser: &Browser) -> String {
    let mut lines = apply_lines(ok);
    lines.push(browser_line(browser));
    lines.extend(feature_lines(&ok.state, browser));
    lines.join("\n")
}

/// `적용됨 · rev N` and one line per change / automatic action.
pub fn apply_lines(ok: &ApplyOk) -> Vec<String> {
    let mut lines = vec![format!("적용됨 · rev {}", ok.revision)];
    let s = &ok.state;
    let sig = |k: &str| s.functions.get(k).map_or_else(|| k.to_string(), |f| f.signature());
    for n in &ok.notes {
        lines.push(match n {
            Note::FunctionCreated { key } => format!("추가: {}", sig(key)),
            Note::FunctionModified { key } => format!("수정: {}", sig(key)),
            Note::FunctionUnchanged { key } => format!("변경 없음: {key}"),
            Note::FunctionDeleted { key, via: None } => format!("삭제: {key}"),
            Note::FunctionDeleted { key, via: Some(v) } => format!("삭제: {key} ({v} 삭제로 함께)"),
            Note::ConstantCreated { key } => match s.constants.get(key) {
                Some(c) => format!("자동: 상수 생성 {}", constant_text(c)),
                None => format!("자동: 상수 생성 {key}"),
            },
            Note::ConstantMadeInteger { key } => format!("자동: 상수 {key}를 정수로 바꿈 (sum/prod 범위에 쓰임)"),
            Note::ConstantRemoved { key } => format!("자동: 상수 {key} 제거 (쓰는 함수 없음)"),
            Note::SignatureConstAdded { function, constant, via } => {
                format!("자동: {} ← 상수 {constant} 추가 ({via}가 {constant}를 씀)", sig(function))
            }
            Note::SignatureConstRemoved { function, constant, via } => {
                format!("자동: {} ← 상수 {constant} 뺌 ({via}가 더 이상 안 씀)", sig(function))
            }
            Note::ConstantDeclared { key } => match s.constants.get(key) {
                Some(c) => format!("상수: {}", constant_text(c)),
                None => format!("상수: {key}"),
            },
            Note::ObjectCreated { kind, key } | Note::ObjectModified { kind, key } => {
                let verb = if matches!(n, Note::ObjectCreated { .. }) { "추가" } else { "수정" };
                match kind {
                    ObjKind::Highlight => format!("{verb}: {}", s.highlights.get(key).map_or_else(|| key.clone(), highlight_text)),
                    ObjKind::Style => format!("{verb}: style {key}"),
                    ObjKind::Var => format!("{verb}: 변수 {key} = {}", s.vars.get(key).map_or("", |v| v.src.as_str())),
                }
            }
            Note::ObjectUnchanged { key, .. } => format!("변경 없음: {key}"),
            Note::ObjectDeleted { kind, key, via, reason } => {
                let what = if *kind == ObjKind::Highlight { key.clone() } else { format!("{} {key}", kind.word()) };
                match (via, reason) {
                    (Some(v), _) => format!("삭제: {what} ({v} 삭제로 함께)"),
                    (None, Some(r)) => format!("삭제: {what} ({r})"),
                    (None, None) => format!("삭제: {what}"),
                }
            }
            Note::RequestAdded { number } => format!("요청사항 #{number} 추가"),
            Note::SceneCreated { key } => format!("추가: {}", scene_line(s, key)),
            Note::SceneModified { key } => format!("수정: {}", scene_line(s, key)),
            Note::SceneUnchanged { key } => format!("변경 없음: scene({key})"),
            Note::SceneDeleted { key } => format!("삭제: scene({key})"),
            Note::DefaultSceneCreated { key } => format!("자동: 기본 scene({key}) 생성"),
            Note::DefaultSceneRemoved { key } => format!("자동: 기본 scene({key}) 제거 (그 좌표계의 함수가 없음)"),
            Note::SceneRefRemoved { scene, target, place } => format!("자동: scene({scene})의 {place}에서 {target} 뺌 (scene은 남음)"),
            Note::DerivativeGenerated { function, order, src } => {
                let x = s.functions.get(function).and_then(|f| f.inputs.first()).map_or("x", |i| i.key.as_str());
                format!("자동: 도함수 {function}@{order}({x}) = {src} (서버 기호 미분; {function}@{order}을 정의하면 그걸 씀)")
            }
        });
    }
    lines
}

/// `scene(1) · 2D(x, y) · step 2개`
fn scene_line(s: &State, key: &str) -> String {
    match s.scenes.get(key) {
        Some(sc) => {
            let system = dsl::state::any_system_key(sc.polar, &sc.axes.iter().map(|a| a.key.as_str()).collect::<Vec<_>>());
            format!("scene({key}) · {system} · step {}개", sc.step_count())
        }
        None => format!("scene({key})"),
    }
}

fn view_text(v: &ViewBox, axes: (&str, &str)) -> String {
    format!("{} <= {} <= {} and {} <= {} <= {}", fmt_num(v.x_min), axes.0, fmt_num(v.x_max), fmt_num(v.y_min), axes.1, fmt_num(v.y_max))
}

/// Note blocks of a step as text: `{f}` as its definition, math in DSL notation.
fn note_text(s: &State, sc: &Scene, step: usize) -> Option<String> {
    let st = sc.steps.get(step)?;
    if st.note.is_empty() {
        return None;
    }
    let blocks: Vec<String> = st
        .note
        .iter()
        .map(|b| {
            b.segs
                .iter()
                .map(|g| match g {
                    NoteSeg::Text { text } => text.clone(),
                    NoteSeg::Tex { tex } => format!("${tex}$"),
                    NoteSeg::Math { expr } => format!("{{{}}}", dsl::print::expr_dsl(expr)),
                    NoteSeg::Def { function } => format!("{{{}}}", s.functions.get(function).map_or(function.clone(), |f| f.source())),
                })
                .collect()
        })
        .collect();
    Some(blocks.join(" / "))
}

pub fn state_text(w: &WorkspaceView, browser: &Browser) -> String {
    let s: &State = &w.state;
    let mut lines = vec![format!("작업 공간: {} · rev {}", w.info.name, w.info.revision)];
    let step = w.nav.step as usize;
    let cur = w.nav.scene.as_ref().and_then(|k| s.scenes.get(k));
    match cur {
        None => lines.push("scene: 없음 (정의가 생기면 기본 scene이 만들어짐)".into()),
        Some(sc) if sc.is_default() => lines.push(format!("scene: {} (기본)", sc.label())),
        Some(sc) => lines.push(format!("scene: {} · step {step}/{}", sc.label(), sc.step_count())),
    }
    if let (Browser::View { view, .. }, Some(sc)) = (browser, cur)
        && sc.axes.len() == 2
    {
        if sc.polar {
            // Polar scenes: the largest radius fully on screen, plus the screen rectangle.
            let r = [-view.x_min, view.x_max, -view.y_min, view.y_max].into_iter().fold(f64::INFINITY, f64::min).max(0.0);
            lines.push(format!("view: {} <= {} (화면 {})", sc.axes[0].key, fmt_num(r), view_text(view, ("x", "y"))));
        } else {
            lines.push(format!("view: {}", view_text(view, (&sc.axes[0].key, &sc.axes[1].key))));
        }
    }
    if let Some(sc) = cur {
        match (&sc.user_clip, &sc.clip) {
            (Some(u), _) => lines.push(format!("clip: {} (사용자 조정, Claude 값 {})", u.src, sc.clip.as_ref().map_or("없음", |c| c.src.as_str()))),
            (None, Some(c)) => lines.push(format!("clip: {}", c.src)),
            _ => {}
        }
        if let Some(n) = note_text(s, sc, step) {
            lines.push(format!("note: {n}"));
        }
    }
    let mut others: Vec<&Scene> = s.scenes.values().filter(|o| Some(&o.key) != w.nav.scene.as_ref()).collect();
    others.sort_by_key(|o| (o.number.is_some(), o.number, o.order));
    if !others.is_empty() {
        let items: Vec<String> = others
            .iter()
            .map(|o| {
                let mut t = if o.is_default() { format!("{} (기본)", o.label()) } else { format!("{} (step {}개)", o.label(), o.step_count()) };
                if w.nav.updated.contains(&o.key) {
                    t.push_str(" 갱신됨");
                }
                t
            })
            .collect();
        lines.push(format!("다른 scene: {}", items.join(", ")));
    }
    let spans: &[SpanDim] = match browser {
        Browser::View { spans, .. } | Browser::Rendered { spans, .. } => spans,
        _ => &[],
    };
    let features: &[FeaturePoints] = match browser {
        Browser::View { features, .. } | Browser::Rendered { features, .. } => features,
        _ => &[],
    };
    let mark = |key: &str| -> String {
        let Some(sc) = cur else { return String::new() };
        let f = s.functions.get(key);
        if f.is_some_and(|f| !f.kind.is_drawn()) {
            // Vectors, matrices, matrix functions: used by others, never drawn.
            return String::new();
        }
        if !sc.members.iter().any(|m| m == key) {
            let system = match (s.function_system(key), s.highlights.get(key), f.and_then(|f| f.vector_dim())) {
                (Some(sys), _, _) => sys,
                (None, Some(h), _) => h.axes.join(", "),
                (None, None, Some(n)) => format!("축 {n}개"),
                _ => String::new(),
            };
            return format!(" [다른 좌표계 {system}]");
        }
        if let Some(d) = spans.iter().find(|d| d.key == key)
            && sc.visible(key, step)
        {
            return format!(" [{}]", span_dim_word(d.dim));
        }
        if let Some(fp) = features.iter().find(|d| d.key == key)
            && sc.visible(key, step)
        {
            return format!(" → {}", feature_text(s, fp));
        }
        if sc.steps.get(step).is_some_and(|st| st.concealed.iter().any(|c| c == key)) {
            return " [공개 전]".into();
        }
        if sc.visible(key, step) { String::new() } else { " [꺼짐]".into() }
    };
    let mut fs: Vec<_> = s.functions.values().collect();
    fs.sort_by_key(|f| f.order);
    if fs.is_empty() {
        lines.push("정의: 없음".into());
    } else {
        lines.push("정의:".into());
        for f in fs {
            lines.push(format!("  {}{}", f.source(), mark(&f.key)));
            // Generated derivatives in use that Claude has not defined.
            let x = f.inputs.first().map_or("x", |i| i.key.as_str());
            for (i, e) in f.derivs.iter().enumerate() {
                let k = format!("{}@{}", f.key, i + 1);
                if !s.functions.contains_key(&k) {
                    lines.push(format!("    자동 도함수 {k}({x}) = {}", dsl::print::expr_dsl(e)));
                }
            }
        }
    }
    let mut hs: Vec<_> = s.highlights.values().collect();
    hs.sort_by_key(|h| h.order);
    if !hs.is_empty() {
        lines.push("강조:".into());
        for h in hs {
            lines.push(format!("  {}{}", highlight_text(h), mark(&h.key)));
        }
    }
    let mut cs: Vec<_> = s.constants.values().collect();
    cs.sort_by_key(|c| c.order);
    if !cs.is_empty() {
        lines.push(format!("상수: {}", cs.iter().map(|c| constant_text(c)).collect::<Vec<_>>().join(", ")));
    }
    let mut st: Vec<_> = s.styles.values().collect();
    st.sort_by_key(|x| x.order);
    for x in st {
        lines.push(x.src.clone());
    }
    let mut vs: Vec<_> = s.vars.values().collect();
    vs.sort_by_key(|v| v.order);
    if !vs.is_empty() {
        lines.push(format!("변수: {}", vs.iter().map(|v| format!("{} = {}", v.key, v.src)).collect::<Vec<_>>().join(", ")));
    }
    let pending = s.requests.values().filter(|r| !r.done).count();
    if pending > 0 {
        lines.push(format!("요청사항: 대기 {pending}개 (get_requests로 확인)"));
    }
    lines.push(browser_line(browser));
    lines.join("\n")
}

fn status_text(st: &ReqStatus) -> String {
    match st {
        ReqStatus::Unchanged {} => "그대로".into(),
        ReqStatus::Modified { seq, current, via: None } => format!("수정됨 (단계 {seq}) → 현재 {current}"),
        ReqStatus::Modified { seq, current, via: Some(v) } => format!("수정됨 (단계 {seq}, {v} 변경으로) → 현재 {current}"),
        ReqStatus::Deleted { seq, cause: None } => format!("삭제됨 (단계 {seq})"),
        ReqStatus::Deleted { seq, cause: Some(c) } => format!("삭제됨 (단계 {seq}, {c})"),
    }
}

fn request_text(v: &ReqView, r: &Request) -> String {
    let who = if v.by_user { "사용자" } else { "Claude" };
    let mark = if v.done { "처리됨" } else { "대기" };
    let mut lines = vec![format!("#{} [{mark}] {who} · 단계 {} · {}", v.number, v.seq, v.src), format!("  대상(단계 {}): {}", v.seq, r.target_src)];
    if let Some(range) = &r.range {
        lines.push(format!("  범위: {range}"));
    }
    if let Some(m) = &r.memo {
        lines.push(format!("  memo: {m}"));
    }
    lines.push(format!("  상태: {}", status_text(&v.status)));
    lines.join("\n")
}

/// `get_requests`: pending ones (or all), with what they point at and their change status.
pub fn requests_text(out: &RequestsOut, all: bool, marked: &[u64]) -> String {
    let state = &out.state;
    let pending = out.items.iter().filter(|v| !v.done).count();
    let done = out.items.len() - pending;
    let mut lines = vec![];
    for e in &out.mark_errors {
        lines.push(format!("[E-req] {e}"));
    }
    let ok: Vec<String> = marked.iter().filter(|n| out.items.iter().any(|v| v.number == **n && v.done)).map(|n| format!("#{n}")).collect();
    if !ok.is_empty() {
        lines.push(format!("처리됨으로 표시: {}", ok.join(", ")));
    }
    let hidden = if all || done == 0 { String::new() } else { format!(" (처리됨 {done}개 숨김 → all=true)") };
    lines.push(format!("요청사항 · {} · 대기 {pending}개{hidden}", out.info.name));
    for v in out.items.iter().filter(|v| all || !v.done) {
        if let Some(r) = state.requests.get(&v.number) {
            lines.push(request_text(v, r));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dsl::interp::{Limits, apply};
    use model::protocol::WorkspaceInfo;

    fn applied(reqs: &[&str]) -> (State, Vec<Note>) {
        let mut s = State::default();
        let mut notes = vec![];
        for (i, r) in reqs.iter().enumerate() {
            let a = apply(&s, r, &Limits::default(), i as u64 + 1).unwrap();
            s = a.state;
            notes = a.notes;
        }
        (s, notes)
    }

    #[test]
    fn apply_text_lists_changes_and_automatic_actions_only() {
        let (state, notes) = applied(&["define:\n    f(x; a) -> y: a*x\n    g(x; a) -> y: f(x)\n", "define:\n    f(x; a, b) -> y: a*x + b\n"]);
        let ok = ApplyOk { workspace: "w1".into(), revision: 2, notes, state };
        let text = apply_ok(&ok, &Browser::Rendered { warnings: vec![], spans: vec![], features: vec![] });
        assert_eq!(
            text,
            "적용됨 · rev 2\n수정: f(x; a, b) -> y\n자동: 상수 생성 b = 1 (-10..10)\n자동: g(x; a, b) -> y ← 상수 b 추가 (f가 b를 씀)\n브라우저: 경고 없음"
        );
        assert!(!text.contains("a*x + b"), "the request is not echoed");
    }

    #[test]
    fn cascade_text() {
        let (state, notes) = applied(&["define:\n    f(x; a) -> y: a*x\n    g(x; a) -> y: f(x)\n", "del f\n"]);
        let ok = ApplyOk { workspace: "w1".into(), revision: 2, notes, state };
        let text = apply_ok(&ok, &Browser::NotConnected);
        assert!(text.contains("삭제: f\n삭제: g (f 삭제로 함께)\n자동: 상수 a 제거 (쓰는 함수 없음)"), "{text}");
    }

    fn view_of(state: State, scene: &str, step: u32) -> WorkspaceView {
        WorkspaceView {
            info: WorkspaceInfo { id: "w1".into(), name: "calc".into(), has_key: true, revision: 1, updated_at: 0 },
            state,
            nav: model::protocol::Nav { scene: Some(scene.into()), step, updated: vec![] },
        }
    }

    #[test]
    fn state_text_uses_dsl_notation() {
        let (state, _) = applied(&["define:\n    f(x; n) -> y: sum(x^k; 0 <= k <= n)\n"]);
        let w = view_of(state, "2D(x, y)", 0);
        let b = Browser::View {
            view: ViewBox { x_min: -10.0, x_max: 10.0, y_min: -6.25, y_max: 6.25 },
            warnings: vec![Warning { source: Some("f".into()), code: "W-sum-terms".into(), message: "항이 너무 많은 점은 그리지 않음".into() }],
            spans: vec![],
            features: vec![],
        };
        assert_eq!(
            state_text(&w, &b),
            "작업 공간: calc · rev 1\nscene: scene(2D(x, y)) (기본)\nview: -10 <= x <= 10 and -6.25 <= y <= 6.25\n정의:\n  f(x; n) -> y: sum(x^k; 0 <= k <= n)\n상수: n = 5 (정수, 0..20)\n브라우저: 경고 1개\n  f: 항이 너무 많은 점은 그리지 않음 [W-sum-terms]"
        );
    }

    #[test]
    fn span_dimension_and_linear_algebra_text() {
        let (state, notes) = applied(&["define:\n    v(; a): [1, a]\n    w: [0, 1]\n    u: [1, 2, 3]\n    S(; a): span(v, w)\n    T: span(u)\n    p(; a): arrow(v; w)\n    R(t) -> matrix: [[cos(t), 0], [0, 1]]\n"]);
        let ok = ApplyOk { workspace: "w1".into(), revision: 1, notes, state: state.clone() };
        let lines = apply_lines(&ok);
        assert!(lines.contains(&"추가: v(; a)".to_string()), "{lines:?}");
        assert!(lines.contains(&"추가: R(t) -> matrix".to_string()), "{lines:?}");
        let r = apply_ok(&ok, &Browser::Rendered { warnings: vec![], spans: vec![SpanDim { key: "S".into(), dim: 1 }], features: vec![] });
        assert!(r.ends_with("브라우저: 경고 없음\nspan: S 직선 (1차원)"), "{r}");
        let w = view_of(state, "2D(x, y)", 0);
        let b = Browser::View { view: ViewBox { x_min: -1.0, x_max: 1.0, y_min: -1.0, y_max: 1.0 }, warnings: vec![], spans: vec![SpanDim { key: "S".into(), dim: 2 }], features: vec![] };
        let t = state_text(&w, &b);
        assert!(t.contains("  v(; a): [1, a]\n  w: [0, 1]\n  u: [1, 2, 3]\n  S(; a): span(v, w) [평면 (2차원)]\n  T: span(u) [다른 좌표계 축 3개]\n  p(; a): arrow(v; w)\n  R(t) -> matrix: [[cos(t), 0], [0, 1]]"), "{t}");
    }

    #[test]
    fn derivative_and_feature_text() {
        use model::protocol::FeaturePoint;
        let (state, notes) = applied(&["define:\n    f(x) -> y: x^3 - 3*x\n    E: extrema(f)\n    B: breaks(f)\n"]);
        let ok = ApplyOk { workspace: "w1".into(), revision: 1, notes, state: state.clone() };
        let lines = apply_lines(&ok);
        assert!(lines.contains(&"자동: 도함수 f@1(x) = 3*x^2 - 3 (서버 기호 미분; f@1을 정의하면 그걸 씀)".to_string()), "{lines:?}");
        let pts = vec![
            FeaturePoints { key: "E".into(), points: vec![FeaturePoint { x: -1.0, y: Some(2.0), class: FeatureClass::Max }, FeaturePoint { x: 1.0, y: Some(-2.0), class: FeatureClass::Min }], capped: false },
            FeaturePoints { key: "B".into(), points: vec![], capped: false },
        ];
        let r = apply_ok(&ok, &Browser::Rendered { warnings: vec![], spans: vec![], features: pts.clone() });
        assert!(r.ends_with("특징점 E: 2개: 극대 (-1, 2), 극소 (1, -2)\n특징점 B: 찾은 점 없음"), "{r}");
        let w = view_of(state, "2D(x, y)", 0);
        let b = Browser::View { view: ViewBox { x_min: -3.0, x_max: 3.0, y_min: -3.0, y_max: 3.0 }, warnings: vec![], spans: vec![], features: pts };
        let t = state_text(&w, &b);
        assert!(t.contains("  f(x) -> y: x^3 - 3*x\n    자동 도함수 f@1(x) = 3*x^2 - 3\n  E: extrema(f) → 2개: 극대 (-1, 2), 극소 (1, -2)\n  B: breaks(f) → 찾은 점 없음"), "{t}");
    }

    #[test]
    fn scene_text() {
        let (state, notes) = applied(&[
            "define:\n    f(x; a) -> y: a*x\n    g(t) -> y: t\nH1 = highlight(f; 0 <= x <= 1)\n",
            "scene(1):\n    space: 2D(x, y)\n    show: f\n    hide: H1\n    clip: -2 <= x <= 2\n    step:\n        show: +H1\n        note: \"{f} 아래 넓이 $\\int$\"\n",
        ]);
        let ok = ApplyOk { workspace: "w1".into(), revision: 2, notes, state: state.clone() };
        assert_eq!(apply_lines(&ok)[1], "추가: scene(1) · 2D(x, y) · step 1개");
        let w = view_of(state.clone(), "1", 0);
        let t = state_text(&w, &Browser::NotConnected);
        assert!(t.contains("scene: scene(1) · step 0/1\nclip: -2 <= x <= 2\n다른 scene: scene(2D(x, y)) (기본), scene(2D(t, y)) (기본)"), "{t}");
        assert!(t.contains("  g(t) -> y: t [다른 좌표계 2D(t, y)]"), "{t}");
        assert!(t.contains("H1 = highlight(f; 0 <= x <= 1) · 자동 색 green [공개 전]"), "{t}");
        let t = state_text(&view_of(state.clone(), "1", 1), &Browser::NotConnected);
        assert!(t.contains("note: {f(x; a) -> y: a*x} 아래 넓이 $\\int$"), "{t}");
        let (_, notes) = applied(&["define:\n    g(t) -> y: t\n", "del g\n"]);
        let ok = ApplyOk { workspace: "w1".into(), revision: 2, notes, state };
        assert!(apply_lines(&ok).contains(&"자동: 기본 scene(2D(t, y)) 제거 (그 좌표계의 함수가 없음)".to_string()));
    }
}
