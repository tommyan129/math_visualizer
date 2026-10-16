//! MCP response text: short fixed sections, DSL notation, only changes and what
//! the server did on its own; never an echo of the request (design.md "MCP tool").

use dsl::interp::{Note, ObjKind, fmt_num};
use dsl::state::{BASE_STYLES, Constant, DEFAULT_SCENE, Highlight, Request, State};
use model::protocol::{ReqStatus, ReqView, ViewBox, Warning};

use crate::engine::{ApplyError, ApplyOk, KeyError, RequestsOut, WorkspaceView};

/// What the browser said (or did not say) about a workspace.
#[derive(Debug, Clone)]
pub enum Browser {
    NotConnected,
    /// The active tab shows another workspace.
    OtherWorkspace,
    /// Connected but no answer in time.
    Timeout,
    Rendered { warnings: Vec<Warning> },
    View { scene: String, view: ViewBox, warnings: Vec<Warning> },
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

pub fn browser_line(b: &Browser) -> String {
    match b {
        Browser::NotConnected => "브라우저: 미연결 (화면 확인은 연결 후 get_state로)".into(),
        Browser::OtherWorkspace => "브라우저: 다른 작업 공간을 보고 있음 (화면 확인은 get_state로)".into(),
        Browser::Timeout => "브라우저: 응답 없음 (렌더 결과는 get_state로)".into(),
        Browser::Rendered { warnings } | Browser::View { warnings, .. } => format!("브라우저: {}", warnings_text(warnings)),
    }
}

pub fn apply_ok(ok: &ApplyOk, browser: &Browser) -> String {
    let mut lines = apply_lines(ok);
    lines.push(browser_line(browser));
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
        });
    }
    lines
}

fn view_text(v: &ViewBox) -> String {
    format!("{} <= x <= {} and {} <= y <= {}", fmt_num(v.x_min), fmt_num(v.x_max), fmt_num(v.y_min), fmt_num(v.y_max))
}

pub fn state_text(w: &WorkspaceView, browser: &Browser) -> String {
    let s: &State = &w.state;
    let mut lines = vec![format!("작업 공간: {} · rev {}", w.info.name, w.info.revision)];
    match browser {
        Browser::View { scene, view, .. } => {
            lines.push(format!("scene: {scene}"));
            lines.push(format!("view: {}", view_text(view)));
        }
        _ => lines.push(format!("scene: {DEFAULT_SCENE} (기본)")),
    }
    let hidden = s.scenes.get(DEFAULT_SCENE).map(|sc| sc.hidden.clone()).unwrap_or_default();
    let mut fs: Vec<_> = s.functions.values().collect();
    fs.sort_by_key(|f| f.order);
    if fs.is_empty() {
        lines.push("정의: 없음".into());
    } else {
        lines.push("정의:".into());
        for f in fs {
            let off = if hidden.contains(&f.key) { " [꺼짐]" } else { "" };
            lines.push(format!("  {}{off}", f.source()));
        }
    }
    let mut hs: Vec<_> = s.highlights.values().collect();
    hs.sort_by_key(|h| h.order);
    if !hs.is_empty() {
        lines.push("강조:".into());
        for h in hs {
            let off = if hidden.contains(&h.key) { " [꺼짐]" } else { "" };
            lines.push(format!("  {}{off}", highlight_text(h)));
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
        let text = apply_ok(&ok, &Browser::Rendered { warnings: vec![] });
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

    #[test]
    fn state_text_uses_dsl_notation() {
        let (state, _) = applied(&["define:\n    f(x; n) -> y: sum(x^k; 0 <= k <= n)\n"]);
        let w = WorkspaceView { info: WorkspaceInfo { id: "w1".into(), name: "calc".into(), has_key: true, revision: 1, updated_at: 0 }, state };
        let b = Browser::View {
            scene: DEFAULT_SCENE.into(),
            view: ViewBox { x_min: -10.0, x_max: 10.0, y_min: -6.25, y_max: 6.25 },
            warnings: vec![Warning { source: Some("f".into()), code: "W-sum-terms".into(), message: "항이 너무 많은 점은 그리지 않음".into() }],
        };
        assert_eq!(
            state_text(&w, &b),
            "작업 공간: calc · rev 1\nscene: 2D(x, y)\nview: -10 <= x <= 10 and -6.25 <= y <= 6.25\n정의:\n  f(x; n) -> y: sum(x^k; 0 <= k <= n)\n상수: n = 5 (정수, 0..20)\n브라우저: 경고 1개\n  f: 항이 너무 많은 점은 그리지 않음 [W-sum-terms]"
        );
    }
}
