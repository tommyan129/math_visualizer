//! req change status, computed by scanning the request log after the req's step
//! (design.md "요청사항": 그대로 / 수정됨 (단계, 현재 식) / 삭제됨 (단계), with the
//! indirect cause when a referenced function changed or a cascade deleted it).

use std::collections::BTreeSet;

use dsl::interp::{Note, address_resolves};
use dsl::state::{Change, ReqTarget, Request, State};
use model::protocol::{ReqStatus, ReqView};

use crate::store::{Actor, StoredEntry};

/// `entries` must cover every log row from the req's own step on.
pub fn view(req: &Request, entries: &[StoredEntry], state: &State) -> ReqView {
    let by_user = entries.iter().find(|e| e.seq == req.seq).is_some_and(|e| e.actor == Actor::User);
    ReqView { number: req.number, src: req.src.clone(), seq: req.seq, by_user, done: req.done, status: status(req, entries, state) }
}

fn status(req: &Request, entries: &[StoredEntry], state: &State) -> ReqStatus {
    let (key, is_function) = match &req.target {
        ReqTarget::Function { function } | ReqTarget::Sub { function, .. } => (function.as_str(), true),
        ReqTarget::Highlight { highlight } => (highlight.as_str(), false),
        ReqTarget::Scene { scene, .. } => return scene_status(req, scene, entries, state),
    };
    let mut deps: BTreeSet<String> = req.deps.iter().cloned().collect();
    let mut modified: Option<(u64, Option<String>)> = None;
    let mut deleted: Option<(u64, Option<String>)> = None;
    for e in entries.iter().filter(|e| e.seq > req.seq) {
        for c in &e.result.changes {
            match c {
                Change::PutFunction { function } if deps.contains(&function.key) || (is_function && function.key == key) => {
                    let via = (function.key != key).then(|| function.key.clone());
                    modified = Some((e.seq, via));
                    deps.insert(function.key.clone());
                    deps.extend(function.calls.iter().cloned());
                }
                Change::PutHighlight { highlight } if !is_function && highlight.key == key => {
                    modified = Some((e.seq, None));
                    deps.extend(highlight.deps.iter().cloned());
                }
                Change::RemoveFunction { key: k } if is_function && k == key => deleted = Some((e.seq, cause(&e.result.notes, k))),
                Change::RemoveHighlight { key: k } if !is_function && k == key => deleted = Some((e.seq, cause(&e.result.notes, k))),
                _ => {}
            }
        }
    }
    let exists = if is_function { state.functions.contains_key(key) } else { state.highlights.contains_key(key) };
    if !exists {
        let (seq, cause) = deleted.unwrap_or((req.seq, None));
        return ReqStatus::Deleted { seq, cause };
    }
    if let ReqTarget::Sub { function, address } = &req.target
        && !address_resolves(state, address)
    {
        let seq = modified.as_ref().map_or(req.seq, |m| m.0);
        return ReqStatus::Deleted { seq, cause: Some(format!("{function} 변경으로 부분식이 사라짐")) };
    }
    match modified {
        // A deletion followed by a new definition counts as a modification.
        Some((seq, via)) => ReqStatus::Modified { seq, current: current(req, state), via },
        None if deleted.is_some() => ReqStatus::Modified { seq: deleted.unwrap().0, current: current(req, state), via: None },
        None => ReqStatus::Unchanged {},
    }
}

/// A scene changes when its definition changes (`defined` is that row's sequence);
/// user toggles and navigation do not count.
fn scene_status(req: &Request, key: &str, entries: &[StoredEntry], state: &State) -> ReqStatus {
    let mut modified = None;
    let mut deleted = None;
    for e in entries.iter().filter(|e| e.seq > req.seq) {
        for c in &e.result.changes {
            match c {
                Change::PutScene { scene } if scene.key == key && scene.defined == e.seq => modified = Some(e.seq),
                Change::RemoveScene { key: k } if k == key => {
                    let cause = e.result.notes.iter().any(|n| matches!(n, Note::DefaultSceneRemoved { key: k } if k == key)).then(|| "그 좌표계의 함수가 모두 지워져".to_string());
                    deleted = Some((e.seq, cause));
                }
                _ => {}
            }
        }
    }
    let Some(sc) = state.scenes.get(key) else {
        let (seq, cause) = deleted.unwrap_or((req.seq, None));
        return ReqStatus::Deleted { seq, cause };
    };
    let current = if sc.src.is_empty() { sc.label() } else { sc.src.clone() };
    match (modified, deleted) {
        (Some(seq), _) | (None, Some((seq, _))) => ReqStatus::Modified { seq, current, via: None },
        (None, None) => ReqStatus::Unchanged {},
    }
}

fn current(req: &Request, state: &State) -> String {
    match &req.target {
        ReqTarget::Scene { scene, .. } => state.scenes.get(scene).map(|s| s.src.clone()).unwrap_or_default(),
        ReqTarget::Function { function } | ReqTarget::Sub { function, .. } => state.functions.get(function).map(|f| f.source()).unwrap_or_default(),
        ReqTarget::Highlight { highlight } => state.highlights.get(highlight).map(|h| format!("{} = {}", h.key, h.src)).unwrap_or_default(),
    }
}

/// "g 삭제로" when a cascade removed `key`; `None` when it was named in `del`.
fn cause(notes: &[Note], key: &str) -> Option<String> {
    notes.iter().find_map(|n| match n {
        Note::FunctionDeleted { key: k, via } if k == key => Some(via.as_ref().map(|v| format!("{v} 삭제로"))),
        Note::ObjectDeleted { key: k, via, reason, .. } if k == key => Some(match (via, reason) {
            (Some(v), _) => Some(format!("{v} 삭제로")),
            (None, Some(r)) => Some(r.clone()),
            (None, None) => None,
        }),
        _ => None,
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::AppliedResult;
    use dsl::interp::{Limits, apply};

    /// Applies requests 1, 2, ... and keeps the log rows.
    fn history(reqs: &[&str]) -> (State, Vec<StoredEntry>) {
        let mut s = State::default();
        let mut log = vec![];
        for (i, r) in reqs.iter().enumerate() {
            let seq = i as u64 + 1;
            let a = apply(&s, r, &Limits::default(), seq).unwrap_or_else(|d| panic!("{r}: {d:?}"));
            s = a.state;
            log.push(StoredEntry { seq, actor: if r.starts_with("req") { Actor::User } else { Actor::Claude }, result: AppliedResult { changes: a.changes, notes: a.notes } });
        }
        (s, log)
    }

    fn statuses(reqs: &[&str]) -> Vec<ReqStatus> {
        let (s, log) = history(reqs);
        s.requests.values().map(|r| view(r, &log, &s).status).collect()
    }

    const FG: &str = "define:\n    f(x; a) -> y: a*x\n    g(x; a) -> y: f(x) + 1\n";

    #[test]
    fn scene_targets() {
        const SC: &str = "scene(1):\n    space: 2D(x, y)\n    show: f\n    step:\n        show: +g\n";
        assert_eq!(statuses(&[FG, SC, "req(scene(1).step(1))\n"]), [ReqStatus::Unchanged {}]);
        let changed = statuses(&[FG, SC, "req(scene(1).step(1))\n", &SC.replace("+g", "-f")]);
        assert!(matches!(&changed[0], ReqStatus::Modified { seq: 4, current, .. } if current.contains("-f")), "{changed:?}");
        // A cascade inside the scene counts as a change; the scene stays.
        assert!(matches!(statuses(&[FG, SC, "req(scene(1))\n", "del g\n"])[0], ReqStatus::Modified { seq: 4, .. }));
        assert_eq!(statuses(&[FG, SC, "req(scene(1))\n", "del scene(1)\n"]), [ReqStatus::Deleted { seq: 4, cause: None }]);
        assert_eq!(
            statuses(&[FG, "req(scene(2D(x, y)))\n", "del f\n"]),
            [ReqStatus::Deleted { seq: 3, cause: Some("그 좌표계의 함수가 모두 지워져".into()) }]
        );
    }

    #[test]
    fn unchanged_modified_deleted() {
        assert_eq!(statuses(&[FG, "req(g)\n"]), [ReqStatus::Unchanged {}]);
        assert_eq!(
            statuses(&[FG, "req(g)\n", "define:\n    g(x; a) -> y: f(x) + 2\n"]),
            [ReqStatus::Modified { seq: 3, current: "g(x; a) -> y: f(x) + 2".into(), via: None }]
        );
        assert_eq!(statuses(&[FG, "req(g)\n", "del g\n"]), [ReqStatus::Deleted { seq: 3, cause: None }]);
    }

    #[test]
    fn indirect_changes_name_their_cause() {
        assert_eq!(
            statuses(&[FG, "req(g)\n", "define:\n    f(x; a) -> y: a*x^2\n"]),
            [ReqStatus::Modified { seq: 3, current: "g(x; a) -> y: f(x) + 1".into(), via: Some("f".into()) }]
        );
        assert_eq!(statuses(&[FG, "req(g)\n", "del f\n"]), [ReqStatus::Deleted { seq: 3, cause: Some("f 삭제로".into()) }]);
    }

    #[test]
    fn highlight_and_subexpression_targets() {
        let base = "define:\n    f(x; a, b) -> y: a*x^2 + b*x + 1\n";
        assert_eq!(
            statuses(&[base, "H1 = highlight(f; 0 <= x <= 2)\n", "req(H1)\n", "del f\n"]),
            [ReqStatus::Deleted { seq: 4, cause: Some("f 삭제로".into()) }]
        );
        assert_eq!(
            statuses(&[base, "req(f.(b*x))\n", "define:\n    f(x; a) -> y: a*x^2 + 1\n"]),
            [ReqStatus::Deleted { seq: 3, cause: Some("f 변경으로 부분식이 사라짐".into()) }]
        );
        let (s, log) = history(&[base, "req(f.(b*x);; \"부호\")\n"]);
        let v = view(&s.requests[&1], &log, &s);
        assert!(v.by_user);
        assert_eq!(v.status, ReqStatus::Unchanged {});
    }

    #[test]
    fn recreated_after_delete_counts_as_modified() {
        assert_eq!(
            statuses(&[FG, "req(f)\n", "del f\n", "define:\n    f(x) -> y: x\n"]),
            [ReqStatus::Modified { seq: 4, current: "f(x) -> y: x".into(), via: None }]
        );
    }
}
