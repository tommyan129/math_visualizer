//! The engine owns every workspace and the store, on one dedicated thread.
//! All requests queue here, so at most one interpretation runs at a time.

use std::path::Path;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use dsl::diag::Diagnostic;
use dsl::interp::{Applied, Limits, Note, UserOp, apply, apply_user_op};
use dsl::span::LineIndex;
use dsl::state::State;
use model::protocol::{ReqView, Snapshot, WorkspaceInfo};
use tokio::sync::{broadcast, oneshot};

use crate::keys;
use crate::store::{Actor, AppliedResult, LogEntry, Store};

/// Thread stack for interpretation; recursive passes also grow on demand via `stacker`.
const ENGINE_STACK: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub enum Event {
    Updated(Arc<Snapshot>),
    Workspaces(Vec<WorkspaceInfo>),
}

#[derive(Debug)]
pub enum KeyError {
    Invalid,
    RateLimited,
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
    List { reply: Reply<Vec<WorkspaceInfo>> },
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
    state: State,
    revision: u64,
    updated_ms: i64,
    last_notes: Vec<Note>,
}

impl Workspace {
    fn info(&self) -> WorkspaceInfo {
        WorkspaceInfo { id: self.id.clone(), name: self.name.clone(), has_key: self.key_hash.is_some(), revision: self.revision, updated_at: self.updated_ms }
    }

    fn snapshot(&self) -> Arc<Snapshot> {
        Arc::new(Snapshot { workspace: self.id.clone(), revision: self.revision, state: self.state.clone(), notes: self.last_notes.clone() })
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
        .map(|l| Workspace {
            id: l.row.id,
            name: l.row.name,
            key_hash: l.row.key_hash,
            state: l.state,
            revision: l.revision,
            updated_ms: l.updated_ms,
            last_notes: l.last_notes,
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
                    WorkspaceView { info: w.info(), state: w.state.clone() }
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
        }
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
        w.last_notes = result.notes;
        let _ = self.events.send(Event::Updated(w.snapshot()));
        let _ = self.events.send(Event::Workspaces(self.list()));
        Ok(seq)
    }

    /// Key (Claude over MCP) or workspace id (the user's browser, private listener).
    fn locate(&mut self, at: &WsRef) -> Result<(usize, Actor), ApplyError> {
        match at {
            WsRef::Key(k) => self.find_by_key(k).map(|i| (i, Actor::Claude)).map_err(ApplyError::Key),
            WsRef::Id(id) => self.index(id).map(|i| (i, Actor::User)).ok_or_else(|| ApplyError::Internal("작업 공간이 없음".into())),
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
            state: State::default(),
            revision: 0,
            updated_ms: now,
            last_notes: vec![],
        });
        let info = self.workspaces.last().unwrap().info();
        let _ = self.events.send(Event::Workspaces(self.list()));
        Ok(info)
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
