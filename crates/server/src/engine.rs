//! The engine owns every workspace and the store, on one dedicated thread.
//! All requests queue here, so at most one interpretation runs at a time.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use dsl::diag::Diagnostic;
use dsl::interp::{Applied, Limits, Note, UserOp, apply, apply_user_op, navigate, normalize};
use dsl::span::LineIndex;
use dsl::state::State;
use model::protocol::{Nav, ReqView, Snapshot, WorkspaceInfo};
use tokio::sync::{broadcast, oneshot};

use crate::keys;
use crate::store::{Actor, AppliedResult, LogEntry, Store};

/// Thread stack for interpretation; recursive passes also grow on demand via `stacker`.
const ENGINE_STACK: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub enum Event {
    Updated(Arc<Snapshot>),
    Workspaces(Vec<WorkspaceInfo>),
    /// Claude made a new scene here: the browser opens this workspace.
    Navigate(String),
}

#[derive(Debug)]
pub enum KeyError {
    Invalid,
    RateLimited,
    /// The key's workspace is in the trash.
    Trashed,
}

#[derive(Debug)]
pub enum ApplyError {
    Key(KeyError),
    /// Rendered `line:col [E-code] message → suggestion` lines.
    Rejected(Vec<String>),
    Internal(String),
}

#[derive(Debug, Clone)]
pub struct ApplyOk {
    pub workspace: String,
    pub revision: u64,
    pub notes: Vec<Note>,
    pub state: State,
}

#[derive(Debug, Clone)]
pub struct WorkspaceView {
    pub info: WorkspaceInfo,
    pub state: State,
    pub nav: Nav,
}

/// Which workspace a command addresses: by key (MCP) or by id (private listener).
#[derive(Debug, Clone)]
pub enum WsRef {
    Key(String),
    Id(String),
}

#[derive(Debug, Clone)]
pub struct RequestsOut {
    pub info: WorkspaceInfo,
    pub items: Vec<ReqView>,
    /// Problems with the requested `done` numbers.
    pub mark_errors: Vec<String>,
    pub state: State,
}

type Reply<T> = oneshot::Sender<T>;

enum Cmd {
    Apply { at: WsRef, src: String, reply: Reply<Result<ApplyOk, ApplyError>> },
    Requests { at: WsRef, done: Vec<u64>, reply: Reply<Result<RequestsOut, ApplyError>> },
    Resolve { key: String, reply: Reply<Result<WorkspaceView, KeyError>> },
    UserOp { workspace: String, op: UserOp, reply: Reply<Result<u64, String>> },
    Create { name: String, reply: Reply<Result<WorkspaceInfo, String>> },
    ReplaceKey { workspace: String, reply: Reply<Result<String, String>> },
    Snapshot { workspace: String, reply: Reply<Option<Arc<Snapshot>>> },
    Navigate { workspace: String, scene: String, step: Option<u32>, reply: Reply<Result<(), String>> },
    List { reply: Reply<Vec<WorkspaceInfo>> },
    /// `trashed`: move to / out of the trash.
    Trash { workspace: String, trashed: bool, reply: Reply<Result<(), String>> },
    Delete { workspace: String, reply: Reply<Result<(), String>> },
}

#[derive(Clone)]
pub struct EngineHandle {
    tx: std::sync::mpsc::Sender<Cmd>,
    events: broadcast::Sender<Event>,
}

struct Workspace {
    id: String,
    name: String,
    key_hash: Option<String>,
    trashed_ms: Option<i64>,
    state: State,
    revision: u64,
    updated_ms: i64,
    last_notes: Vec<Note>,
    nav: NavState,
}

/// Current scene and step per workspace (design.md: held by the server, not logged).
#[derive(Debug, Clone, Default)]
struct NavState {
    scene: Option<String>,
    /// Step each scene was left at.
    steps: BTreeMap<String, u32>,
    updated: BTreeSet<String>,
}

impl NavState {
    fn step(&self) -> u32 {
        self.scene.as_ref().and_then(|s| self.steps.get(s)).copied().unwrap_or(0)
    }

    fn view(&self) -> Nav {
        Nav { scene: self.scene.clone(), step: self.step(), updated: self.updated.iter().cloned().collect() }
    }

    /// Navigation rules after a change (design.md "Claude가 장면을 바꿨을 때 화면 이동").
    /// Returns true when a new Claude scene was made (the browser should come here).
    fn follow(&mut self, notes: &[Note], state: &State) -> bool {
        let mut jump = false;
        for n in notes {
            match n {
                Note::SceneCreated { key } => {
                    self.scene = Some(key.clone());
                    self.steps.insert(key.clone(), 0);
                    self.updated.remove(key);
                    jump = true;
                }
                Note::SceneModified { key } | Note::SceneRefRemoved { scene: key, .. } | Note::DefaultSceneCreated { key } => {
                    if self.scene.as_ref() != Some(key) {
                        self.updated.insert(key.clone());
                    }
                }
                Note::SceneDeleted { key } | Note::DefaultSceneRemoved { key } => {
                    self.steps.remove(key);
                    self.updated.remove(key);
                }
                _ => {}
            }
        }
        // A step count that shrank: stay at the last step.
        for (k, st) in self.steps.iter_mut() {
            if let Some(s) = state.scenes.get(k) {
                *st = (*st).min(s.step_count() as u32);
            }
        }
        self.steps.retain(|k, _| state.scenes.contains_key(k));
        self.updated.retain(|k| state.scenes.contains_key(k));
        if self.scene.as_ref().is_none_or(|s| !state.scenes.contains_key(s)) {
            // The most recently changed scene.
            // The most recent change; within one request, the first scene it made.
            self.scene = state.scenes.values().max_by_key(|s| (s.defined, std::cmp::Reverse(s.order))).map(|s| s.key.clone());
            if let Some(s) = &self.scene {
                self.updated.remove(s);
            }
        }
        jump
    }
}

impl Workspace {
    fn info(&self) -> WorkspaceInfo {
        WorkspaceInfo { id: self.id.clone(), name: self.name.clone(), has_key: self.key_hash.is_some(), revision: self.revision, updated_at: self.updated_ms, trashed_at: self.trashed_ms }
    }

    fn snapshot(&self) -> Arc<Snapshot> {
        Arc::new(Snapshot { workspace: self.id.clone(), revision: self.revision, state: self.state.clone(), notes: self.last_notes.clone(), nav: self.nav.view() })
    }
}

struct Engine {
    store: Store,
    workspaces: Vec<Workspace>,
    limiter: keys::Limiter,
    limits: Limits,
    events: broadcast::Sender<Event>,
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

pub fn render_diags(src: &str, diags: &[Diagnostic]) -> Vec<String> {
    let idx = LineIndex::new(src);
    diags.iter().map(|d| d.render(&idx)).collect()
}

/// Opens the store, restores every workspace, and starts the engine thread.
pub fn start(db: &Path, limits: Limits) -> Result<EngineHandle, String> {
    let store = Store::open(db).map_err(|e| format!("db {}: {e}", db.display()))?;
    let loaded = store.load_all()?;
    let workspaces = loaded
        .into_iter()
        .map(|l| {
            let state = normalize(&l.state, l.revision);
            let mut nav = NavState::default();
            nav.follow(&[], &state);
            Workspace { id: l.row.id, name: l.row.name, key_hash: l.row.key_hash, trashed_ms: l.row.trashed_ms, state, revision: l.revision, updated_ms: l.updated_ms, last_notes: l.last_notes, nav }
        })
        .collect();
    let (events, _) = broadcast::channel(64);
    let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
    let mut engine = Engine { store, workspaces, limiter: keys::Limiter::default(), limits, events: events.clone() };
    std::thread::Builder::new()
        .name("engine".into())
        .stack_size(ENGINE_STACK)
        .spawn(move || {
            while let Ok(cmd) = rx.recv() {
                engine.handle(cmd);
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(EngineHandle { tx, events })
}

impl Engine {
    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Apply { at, src, reply } => {
                let _ = reply.send(self.apply(&at, &src));
            }
            Cmd::Requests { at, done, reply } => {
                let _ = reply.send(self.requests(&at, &done));
            }
            Cmd::Resolve { key, reply } => {
                let r = self.find_by_key(&key).map(|i| {
                    let w = &self.workspaces[i];
                    WorkspaceView { info: w.info(), state: w.state.clone(), nav: w.nav.view() }
                });
                let _ = reply.send(r);
            }
            Cmd::UserOp { workspace, op, reply } => {
                let _ = reply.send(self.user_op(&workspace, &op));
            }
            Cmd::Create { name, reply } => {
                let _ = reply.send(self.create(&name));
            }
            Cmd::ReplaceKey { workspace, reply } => {
                let _ = reply.send(self.replace_key(&workspace));
            }
            Cmd::Snapshot { workspace, reply } => {
                let _ = reply.send(self.index(&workspace).map(|i| self.workspaces[i].snapshot()));
            }
            Cmd::List { reply } => {
                let _ = reply.send(self.list());
            }
            Cmd::Navigate { workspace, scene, step, reply } => {
                let _ = reply.send(self.navigate(&workspace, &scene, step));
            }
            Cmd::Trash { workspace, trashed, reply } => {
                let _ = reply.send(self.trash(&workspace, trashed));
            }
            Cmd::Delete { workspace, reply } => {
                let _ = reply.send(self.delete(&workspace));
            }
        }
    }

    /// Shows `scene`; on the current scene, walks to `step` and logs what that changed.
    fn navigate(&mut self, workspace: &str, scene: &str, step: Option<u32>) -> Result<(), String> {
        let i = self.index(workspace).ok_or("작업 공간이 없음")?;
        let w = &mut self.workspaces[i];
        let sc = w.state.scenes.get(scene).ok_or_else(|| format!("scene({scene})이 없음"))?;
        let last = sc.step_count() as u32;
        let same = w.nav.scene.as_deref() == Some(scene);
        let from = w.nav.steps.get(scene).copied().unwrap_or(0).min(last);
        let to = step.unwrap_or(from).min(last);
        w.nav.scene = Some(scene.to_string());
        w.nav.steps.insert(scene.to_string(), to);
        w.nav.updated.remove(scene);
        if same && from != to {
            let seq = w.revision + 1;
            let applied = navigate(&w.state, scene, from as usize, to as usize, seq)?;
            if !applied.changes.is_empty() {
                let label = format!("scene({scene}) step {to}");
                self.commit(i, Actor::User, "op", &label, applied)?;
                return Ok(());
            }
        }
        let _ = self.events.send(Event::Updated(self.workspaces[i].snapshot()));
        Ok(())
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.workspaces.iter().position(|w| w.id == id)
    }

    fn list(&self) -> Vec<WorkspaceInfo> {
        self.workspaces.iter().map(|w| w.info()).collect()
    }

    fn find_by_key(&mut self, key: &str) -> Result<usize, KeyError> {
        let now = Instant::now();
        if self.limiter.blocked(now) {
            return Err(KeyError::RateLimited);
        }
        let h = keys::hash(key.trim());
        match self.workspaces.iter().position(|w| w.key_hash.as_deref() == Some(h.as_str())) {
            Some(i) if self.workspaces[i].trashed_ms.is_some() => Err(KeyError::Trashed),
            Some(i) => Ok(i),
            None => {
                self.limiter.fail(now);
                Err(KeyError::Invalid)
            }
        }
    }

    fn commit(&mut self, i: usize, actor: Actor, kind: &str, source: &str, applied: Applied) -> Result<u64, String> {
        let seq = self.workspaces[i].revision + 1;
        let time_ms = now_ms();
        let result = AppliedResult { changes: applied.changes, notes: applied.notes };
        let entry = LogEntry { workspace: &self.workspaces[i].id, seq, time_ms, actor, kind, source, result: &result };
        self.store.append(&entry, &applied.state).map_err(|e| format!("기록 실패: {e}"))?;
        let w = &mut self.workspaces[i];
        w.state = applied.state;
        w.revision = seq;
        w.updated_ms = time_ms;
        let jump = w.nav.follow(&result.notes, &w.state);
        w.last_notes = result.notes;
        let _ = self.events.send(Event::Updated(w.snapshot()));
        if jump {
            let _ = self.events.send(Event::Navigate(w.id.clone()));
        }
        let _ = self.events.send(Event::Workspaces(self.list()));
        Ok(seq)
    }

    /// Key (Claude over MCP) or workspace id (the user's browser, private listener).
    fn locate(&mut self, at: &WsRef) -> Result<(usize, Actor), ApplyError> {
        match at {
            WsRef::Key(k) => self.find_by_key(k).map(|i| (i, Actor::Claude)).map_err(ApplyError::Key),
            WsRef::Id(id) => match self.index(id) {
                Some(i) if self.workspaces[i].trashed_ms.is_some() => Err(ApplyError::Internal("작업 공간이 휴지통에 있음 (복원 후 사용)".into())),
                Some(i) => Ok((i, Actor::User)),
                None => Err(ApplyError::Internal("작업 공간이 없음".into())),
            },
        }
    }

    fn apply(&mut self, at: &WsRef, src: &str) -> Result<ApplyOk, ApplyError> {
        let (i, actor) = self.locate(at)?;
        let seq = self.workspaces[i].revision + 1;
        let applied = apply(&self.workspaces[i].state, src, &self.limits, seq).map_err(|d| ApplyError::Rejected(render_diags(src, &d)))?;
        let notes = applied.notes.clone();
        let revision = self.commit(i, actor, "dsl", src, applied).map_err(ApplyError::Internal)?;
        let w = &self.workspaces[i];
        Ok(ApplyOk { workspace: w.id.clone(), revision, notes, state: w.state.clone() })
    }

    /// Marks `done` (logged as an operation), then lists every req with its change status.
    fn requests(&mut self, at: &WsRef, done: &[u64]) -> Result<RequestsOut, ApplyError> {
        let (i, actor) = self.locate(at)?;
        let mut mark_errors = vec![];
        for n in done {
            let op = UserOp::SetRequestDone { number: *n, done: true };
            let seq = self.workspaces[i].revision + 1;
            match apply_user_op(&self.workspaces[i].state, &op, seq) {
                Ok(a) if a.changes.is_empty() => {}
                Ok(a) => {
                    self.commit(i, actor, "op", &op.describe(), a).map_err(ApplyError::Internal)?;
                }
                Err(e) => mark_errors.push(e),
            }
        }
        let w = &self.workspaces[i];
        let from = w.state.requests.values().map(|r| r.seq).min().unwrap_or(u64::MAX);
        let entries = if from == u64::MAX { vec![] } else { self.store.entries_from(&w.id, from).map_err(ApplyError::Internal)? };
        let items = w.state.requests.values().map(|r| crate::requests::view(r, &entries, &w.state)).collect();
        Ok(RequestsOut { info: w.info(), items, mark_errors, state: w.state.clone() })
    }

    fn user_op(&mut self, workspace: &str, op: &UserOp) -> Result<u64, String> {
        let i = self.index(workspace).ok_or("작업 공간이 없음")?;
        let seq = self.workspaces[i].revision + 1;
        let applied = apply_user_op(&self.workspaces[i].state, op, seq)?;
        if applied.changes.is_empty() {
            return Ok(self.workspaces[i].revision);
        }
        self.commit(i, Actor::User, "op", &op.describe(), applied)
    }

    fn create(&mut self, name: &str) -> Result<WorkspaceInfo, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("이름이 비어 있음".into());
        }
        if name.chars().count() > 80 {
            return Err("이름이 너무 김 (80자까지)".into());
        }
        let mut n = self.workspaces.len() + 1;
        while self.index(&format!("w{n}")).is_some() {
            n += 1;
        }
        let id = format!("w{n}");
        let now = now_ms();
        self.store.create_workspace(&id, name, now).map_err(|e| format!("기록 실패: {e}"))?;
        self.workspaces.push(Workspace {
            id,
            name: name.to_string(),
            key_hash: None,
            trashed_ms: None,
            state: State::default(),
            revision: 0,
            updated_ms: now,
            last_notes: vec![],
            nav: NavState::default(),
        });
        let info = self.workspaces.last().unwrap().info();
        let _ = self.events.send(Event::Workspaces(self.list()));
        Ok(info)
    }

    fn trash(&mut self, workspace: &str, trashed: bool) -> Result<(), String> {
        let i = self.index(workspace).ok_or("작업 공간이 없음")?;
        if self.workspaces[i].trashed_ms.is_some() == trashed {
            return Ok(());
        }
        let now = now_ms();
        self.store.set_trashed(workspace, trashed, now).map_err(|e| format!("기록 실패: {e}"))?;
        self.workspaces[i].trashed_ms = trashed.then_some(now);
        let _ = self.events.send(Event::Workspaces(self.list()));
        Ok(())
    }

    /// Deletes for good; only a workspace in the trash.
    fn delete(&mut self, workspace: &str) -> Result<(), String> {
        let i = self.index(workspace).ok_or("작업 공간이 없음")?;
        if self.workspaces[i].trashed_ms.is_none() {
            return Err("휴지통에 있는 작업 공간만 영구 삭제할 수 있음".into());
        }
        self.store.delete_workspace(workspace).map_err(|e| format!("삭제 실패: {e}"))?;
        self.workspaces.remove(i);
        let _ = self.events.send(Event::Workspaces(self.list()));
        Ok(())
    }

    fn replace_key(&mut self, workspace: &str) -> Result<String, String> {
        let i = self.index(workspace).ok_or("작업 공간이 없음")?;
        let key = keys::generate(&self.workspaces[i].name);
        let h = keys::hash(&key);
        self.store.set_key_hash(workspace, &h, now_ms()).map_err(|e| format!("기록 실패: {e}"))?;
        self.workspaces[i].key_hash = Some(h);
        let _ = self.events.send(Event::Workspaces(self.list()));
        Ok(key)
    }
}

impl EngineHandle {
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    async fn call<T>(&self, make: impl FnOnce(Reply<T>) -> Cmd) -> T {
        let (tx, rx) = oneshot::channel();
        self.tx.send(make(tx)).expect("engine thread alive");
        rx.await.expect("engine replies")
    }

    pub async fn apply(&self, at: WsRef, src: String) -> Result<ApplyOk, ApplyError> {
        self.call(|reply| Cmd::Apply { at, src, reply }).await
    }

    pub async fn requests(&self, at: WsRef, done: Vec<u64>) -> Result<RequestsOut, ApplyError> {
        self.call(|reply| Cmd::Requests { at, done, reply }).await
    }

    pub async fn resolve(&self, key: String) -> Result<WorkspaceView, KeyError> {
        self.call(|reply| Cmd::Resolve { key, reply }).await
    }

    pub async fn user_op(&self, workspace: String, op: UserOp) -> Result<u64, String> {
        self.call(|reply| Cmd::UserOp { workspace, op, reply }).await
    }

    pub async fn create(&self, name: String) -> Result<WorkspaceInfo, String> {
        self.call(|reply| Cmd::Create { name, reply }).await
    }

    pub async fn replace_key(&self, workspace: String) -> Result<String, String> {
        self.call(|reply| Cmd::ReplaceKey { workspace, reply }).await
    }

    pub async fn snapshot(&self, workspace: String) -> Option<Arc<Snapshot>> {
        self.call(|reply| Cmd::Snapshot { workspace, reply }).await
    }

    pub async fn list(&self) -> Vec<WorkspaceInfo> {
        self.call(|reply| Cmd::List { reply }).await
    }

    pub async fn navigate(&self, workspace: String, scene: String, step: Option<u32>) -> Result<(), String> {
        self.call(|reply| Cmd::Navigate { workspace, scene, step, reply }).await
    }

    pub async fn trash(&self, workspace: String, trashed: bool) -> Result<(), String> {
        self.call(|reply| Cmd::Trash { workspace, trashed, reply }).await
    }

    pub async fn delete(&self, workspace: String) -> Result<(), String> {
        self.call(|reply| Cmd::Delete { workspace, reply }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(body: &str) -> String {
        format!("define:\n    {body}\n")
    }

    #[tokio::test]
    async fn apply_needs_a_valid_key_and_old_key_dies_on_replace() {
        let dir = tempfile::tempdir().unwrap();
        let e = start(&dir.path().join("t.db"), Limits::default()).unwrap();
        let w = e.create("calc".into()).await.unwrap();
        assert!(matches!(e.apply(WsRef::Key("nope".into()), def("f(x) -> y: x")).await, Err(ApplyError::Key(KeyError::Invalid))));
        let k1 = e.replace_key(w.id.clone()).await.unwrap();
        let ok = e.apply(WsRef::Key(k1.clone()), def("f(x; a) -> y: a*x")).await.unwrap();
        assert_eq!(ok.revision, 1);
        let k2 = e.replace_key(w.id.clone()).await.unwrap();
        assert!(matches!(e.apply(WsRef::Key(k1), def("g(x) -> y: x")).await, Err(ApplyError::Key(KeyError::Invalid))));
        assert!(e.apply(WsRef::Key(k2), def("g(x) -> y: x")).await.is_ok());
    }

    #[tokio::test]
    async fn trash_restore_and_delete_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let (id, key, other) = {
            let e = start(&db, Limits::default()).unwrap();
            let w = e.create("calc".into()).await.unwrap();
            let o = e.create("other".into()).await.unwrap();
            let k = e.replace_key(w.id.clone()).await.unwrap();
            e.apply(WsRef::Key(k.clone()), def("f(x) -> y: x")).await.unwrap();
            // Only a workspace in the trash can be deleted.
            assert!(e.delete(w.id.clone()).await.is_err());
            e.trash(w.id.clone(), true).await.unwrap();
            assert!(matches!(e.apply(WsRef::Key(k.clone()), def("g(x) -> y: x")).await, Err(ApplyError::Key(KeyError::Trashed))));
            assert!(e.apply(WsRef::Id(w.id.clone()), def("g(x) -> y: x")).await.is_err());
            e.trash(o.id.clone(), true).await.unwrap();
            (w.id, k, o.id)
        };
        let e = start(&db, Limits::default()).unwrap();
        let list = e.list().await;
        assert!(list.iter().all(|w| w.trashed_at.is_some()));
        e.trash(id.clone(), false).await.unwrap();
        assert!(e.apply(WsRef::Key(key), def("g(x) -> y: x")).await.is_ok());
        e.delete(other.clone()).await.unwrap();
        assert!(e.snapshot(other).await.is_none());
        let e = start(&db, Limits::default()).unwrap();
        let list = e.list().await;
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].id.as_str(), list[0].trashed_at, list[0].revision), (id.as_str(), None, 2));
    }

    #[tokio::test]
    async fn rejected_request_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let e = start(&dir.path().join("t.db"), Limits::default()).unwrap();
        let w = e.create("calc".into()).await.unwrap();
        let k = e.replace_key(w.id.clone()).await.unwrap();
        let err = e.apply(WsRef::Key(k), "define:\n    f(x) -> y: 2x\n    g(x) -> y: a*x\n".into()).await.unwrap_err();
        let ApplyError::Rejected(lines) = err else { panic!("{err:?}") };
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("2:16 [E-implicit-mul]"), "{}", lines[0]);
        assert_eq!(e.snapshot(w.id).await.unwrap().revision, 0);
    }

    #[tokio::test]
    async fn restart_restores_state_from_sqlite() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let (id, key, before) = {
            let e = start(&db, Limits::default()).unwrap();
            let w = e.create("calc".into()).await.unwrap();
            let k = e.replace_key(w.id.clone()).await.unwrap();
            e.apply(WsRef::Key(k.clone()), def("f(x; a) -> y: a*x^2")).await.unwrap();
            e.apply(WsRef::Key(k.clone()), def("g(x; a) -> y: f(x) + ((((((x))))))")).await.unwrap();
            e.user_op(w.id.clone(), UserOp::SetValue { constant: "a".into(), value: 0.5 }).await.unwrap();
            e.apply(WsRef::Key(k.clone()), "del f\n".into()).await.unwrap();
            e.apply(WsRef::Key(k.clone()), def("f(x; b) -> y: sum(x^k; 0 <= k <= b)")).await.unwrap();
            (w.id.clone(), k, e.snapshot(w.id).await.unwrap())
        };
        let e = start(&db, Limits::default()).unwrap();
        let after = e.snapshot(id).await.unwrap();
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.state, before.state);
        // The key survives a restart.
        assert!(e.apply(WsRef::Key(key), "del f\n".into()).await.is_ok());
    }

    #[tokio::test]
    async fn checkpoint_then_replay() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let (id, before) = {
            let e = start(&db, Limits::default()).unwrap();
            let w = e.create("calc".into()).await.unwrap();
            let k = e.replace_key(w.id.clone()).await.unwrap();
            e.apply(WsRef::Key(k.clone()), def("f(x; a) -> y: a*x")).await.unwrap();
            for i in 0..(crate::store::CHECKPOINT_EVERY + 5) {
                e.user_op(w.id.clone(), UserOp::SetValue { constant: "a".into(), value: (i % 7) as f64 }).await.unwrap();
            }
            (w.id.clone(), e.snapshot(w.id).await.unwrap())
        };
        let e = start(&db, Limits::default()).unwrap();
        let after = e.snapshot(id).await.unwrap();
        assert_eq!((after.revision, &after.state), (before.revision, &before.state));
    }

    #[tokio::test]
    async fn navigation_follows_claude_and_logs_only_effects() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let e = start(&db, Limits::default()).unwrap();
        let w = e.create("calc".into()).await.unwrap();
        let k = e.replace_key(w.id.clone()).await.unwrap();
        let mut events = e.subscribe();
        e.apply(WsRef::Key(k.clone()), def("f(x; a) -> y: a*x")).await.unwrap();
        // The first function makes the default scene, and nothing was shown: go there.
        assert_eq!(e.snapshot(w.id.clone()).await.unwrap().nav.scene.as_deref(), Some("2D(x, y)"));
        let sc = "scene(1):\n    space: 2D(x, y)\n    show: f\n    step:\n        set: a = 3\n    step:\n        note: \"끝\"\n";
        e.apply(WsRef::Key(k.clone()), sc.into()).await.unwrap();
        let snap = e.snapshot(w.id.clone()).await.unwrap();
        assert_eq!((snap.nav.scene.as_deref(), snap.nav.step), (Some("1"), 0));
        let mut jumped = false;
        while let Ok(ev) = events.try_recv() {
            jumped |= matches!(ev, Event::Navigate(id) if id == w.id);
        }
        assert!(jumped, "a new Claude scene moves the browser to its workspace");
        // Step 1 sets a: logged. Step 2 changes nothing logged: no row.
        let rev = snap.revision;
        e.navigate(w.id.clone(), "1".into(), Some(1)).await.unwrap();
        let s1 = e.snapshot(w.id.clone()).await.unwrap();
        assert_eq!((s1.revision, s1.state.constants["a"].value, s1.nav.step), (rev + 1, 3.0, 1));
        e.navigate(w.id.clone(), "1".into(), Some(2)).await.unwrap();
        let s2 = e.snapshot(w.id.clone()).await.unwrap();
        assert_eq!((s2.revision, s2.nav.step), (rev + 1, 2));
        // Editing the current scene with fewer steps: stay at the last one.
        e.apply(WsRef::Key(k.clone()), "scene(1):\n    space: 2D(x, y)\n    show: f\n    step:\n        set: a = 3\n".into()).await.unwrap();
        assert_eq!(e.snapshot(w.id.clone()).await.unwrap().nav.step, 1);
        // Another scene changed: only marked updated.
        e.apply(WsRef::Key(k.clone()), def("g(t) -> y: t")).await.unwrap();
        let s3 = e.snapshot(w.id.clone()).await.unwrap();
        assert_eq!((s3.nav.scene.as_deref(), s3.nav.updated.clone()), (Some("1"), vec!["2D(t, y)".to_string()]));
        e.navigate(w.id.clone(), "2D(t, y)".into(), None).await.unwrap();
        assert!(e.snapshot(w.id.clone()).await.unwrap().nav.updated.is_empty());
        // The current scene goes: move to the most recently changed one.
        e.apply(WsRef::Key(k.clone()), "del g\n".into()).await.unwrap();
        assert_eq!(e.snapshot(w.id.clone()).await.unwrap().nav.scene.as_deref(), Some("1"));
        assert!(e.navigate(w.id.clone(), "9".into(), None).await.is_err());
        // Restart: state survives (navigation itself is not stored).
        let before = e.snapshot(w.id.clone()).await.unwrap();
        drop(e);
        let e = start(&db, Limits::default()).unwrap();
        let after = e.snapshot(w.id.clone()).await.unwrap();
        assert_eq!(after.state, before.state);
        assert!(after.nav.scene.is_some());
    }

    #[tokio::test]
    async fn deep_expression_survives_storage() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        // Near the nesting limit: deeper than serde_json's default recursion limit once encoded.
        let body = format!("{}x{}", "(x + ".repeat(60), ")".repeat(60));
        let id = {
            let e = start(&db, Limits::default()).unwrap();
            let w = e.create("deep".into()).await.unwrap();
            let k = e.replace_key(w.id.clone()).await.unwrap();
            e.apply(WsRef::Key(k), def(&format!("f(x) -> y: {body}"))).await.unwrap();
            w.id
        };
        let e = start(&db, Limits::default()).unwrap();
        assert!(e.snapshot(id).await.unwrap().state.functions.contains_key("f"));
    }
}
