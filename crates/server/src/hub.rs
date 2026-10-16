//! Browser connections. Exactly one tab is active: a new `Hello` takes over and the
//! previous tab is told `TakenOver`. Only the active tab receives snapshots.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use model::protocol::*;
use tokio::sync::{mpsc, oneshot, watch};

use crate::engine::{ApplyError, EngineHandle, Event, WsRef};
use crate::report::{self, Browser};

/// How long `apply` waits for the browser to report render warnings.
pub const APPLY_WAIT: Duration = Duration::from_secs(3);
/// How long `get_state` waits for the browser's view.
pub const VIEW_WAIT: Duration = Duration::from_secs(2);
/// How long `snapshot` waits for the PNG.
pub const SNAPSHOT_WAIT: Duration = Duration::from_secs(5);
/// Long side of the `snapshot` PNG (stage-2 decision).
pub const SNAPSHOT_MAX_SIDE: u32 = 1600;

#[derive(Debug, Clone)]
struct Rendered {
    workspace: String,
    revision: u64,
    warnings: Vec<Warning>,
    spans: Vec<SpanDim>,
}

struct Active {
    conn: u64,
    tx: mpsc::UnboundedSender<ServerMsg>,
    workspace: Option<String>,
    rendered: watch::Sender<Option<Rendered>>,
}

#[derive(Default)]
struct Inner {
    next_conn: u64,
    next_request: u64,
    active: Option<Active>,
    pending: HashMap<u64, oneshot::Sender<BrowserReply>>,
}

#[derive(Clone)]
pub struct Hub {
    inner: Arc<Mutex<Inner>>,
    engine: EngineHandle,
}

pub struct Connection {
    pub id: u64,
    pub rx: mpsc::UnboundedReceiver<ServerMsg>,
    tx: mpsc::UnboundedSender<ServerMsg>,
}

impl Hub {
    pub fn new(engine: EngineHandle) -> Self {
        let hub = Hub { inner: Arc::new(Mutex::new(Inner::default())), engine };
        let h = hub.clone();
        let mut events = h.engine.subscribe();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(Event::Updated(snapshot)) => {
                        let inner = h.inner.lock().unwrap();
                        if let Some(a) = &inner.active
                            && a.workspace.as_deref() == Some(snapshot.workspace.as_str())
                        {
                            let _ = a.tx.send(ServerMsg::Snapshot { snapshot: (*snapshot).clone() });
                        }
                    }
                    Ok(Event::Workspaces(list)) => {
                        if let Some(a) = &h.inner.lock().unwrap().active {
                            let _ = a.tx.send(ServerMsg::Workspaces { list });
                        }
                    }
                    Ok(Event::Navigate(workspace)) => {
                        let inner = h.inner.lock().unwrap();
                        if let Some(a) = &inner.active
                            && a.workspace.as_deref() != Some(workspace.as_str())
                        {
                            let _ = a.tx.send(ServerMsg::Navigate { workspace });
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
        hub
    }

    pub fn connect(&self) -> Connection {
        let mut inner = self.inner.lock().unwrap();
        inner.next_conn += 1;
        let (tx, rx) = mpsc::unbounded_channel();
        Connection { id: inner.next_conn, rx, tx }
    }

    pub fn disconnect(&self, conn: u64) {
        let mut inner = self.inner.lock().unwrap();
        if inner.active.as_ref().is_some_and(|a| a.conn == conn) {
            inner.active = None;
        }
    }

    fn is_active(&self, conn: u64) -> bool {
        self.inner.lock().unwrap().active.as_ref().is_some_and(|a| a.conn == conn)
    }

    pub async fn on_message(&self, c: &Connection, msg: ClientMsg) {
        match msg {
            ClientMsg::Hello { protocol } => {
                if protocol != PROTOCOL_VERSION {
                    let _ = c.tx.send(ServerMsg::ReloadRequired { protocol: PROTOCOL_VERSION });
                    return;
                }
                {
                    let mut inner = self.inner.lock().unwrap();
                    if let Some(old) = inner.active.take()
                        && old.conn != c.id
                    {
                        let _ = old.tx.send(ServerMsg::TakenOver {});
                    }
                    let (rendered, _) = watch::channel(None);
                    inner.active = Some(Active { conn: c.id, tx: c.tx.clone(), workspace: None, rendered });
                }
                let _ = c.tx.send(ServerMsg::Welcome { protocol: PROTOCOL_VERSION });
                let list = self.engine.list().await;
                let _ = c.tx.send(ServerMsg::Workspaces { list });
            }
            ClientMsg::Request { id, request } => {
                let response = if self.is_active(c.id) {
                    self.request(c, request).await
                } else {
                    ServerResponse::Error { message: "다른 탭에서 열려 있음".into() }
                };
                let _ = c.tx.send(ServerMsg::Response { id, response });
            }
            ClientMsg::Reply { id, reply } => {
                if let Some(tx) = self.inner.lock().unwrap().pending.remove(&id) {
                    let _ = tx.send(reply);
                }
            }
            ClientMsg::Rendered { workspace, revision, warnings, spans } => {
                let inner = self.inner.lock().unwrap();
                if let Some(a) = inner.active.as_ref().filter(|a| a.conn == c.id) {
                    a.rendered.send_replace(Some(Rendered { workspace, revision, warnings, spans }));
                }
            }
        }
    }

    async fn request(&self, c: &Connection, request: ClientRequest) -> ServerResponse {
        match request {
            ClientRequest::CreateWorkspace { name } => match self.engine.create(name).await {
                Ok(workspace) => ServerResponse::Created { workspace },
                Err(message) => ServerResponse::Error { message },
            },
            ClientRequest::OpenWorkspace { workspace } => {
                let Some(snapshot) = self.engine.snapshot(workspace.clone()).await else {
                    return ServerResponse::Error { message: "작업 공간이 없음".into() };
                };
                let mut inner = self.inner.lock().unwrap();
                if let Some(a) = inner.active.as_mut().filter(|a| a.conn == c.id) {
                    a.workspace = Some(workspace);
                    a.rendered.send_replace(None);
                }
                let _ = c.tx.send(ServerMsg::Snapshot { snapshot: (*snapshot).clone() });
                ServerResponse::Ok {}
            }
            ClientRequest::ReplaceKey { workspace } => match self.engine.replace_key(workspace.clone()).await {
                Ok(key) => ServerResponse::Key { workspace, key },
                Err(message) => ServerResponse::Error { message },
            },
            ClientRequest::UserOp { workspace, op } => match self.engine.user_op(workspace, op).await {
                Ok(_) => ServerResponse::Ok {},
                Err(message) => ServerResponse::Error { message },
            },
            ClientRequest::Apply { workspace, source } => match self.engine.apply(WsRef::Id(workspace), source).await {
                Ok(ok) => ServerResponse::Applied { lines: report::apply_lines(&ok) },
                Err(ApplyError::Rejected(lines)) => ServerResponse::Rejected { lines },
                Err(e) => ServerResponse::Error { message: report::apply_err(&e) },
            },
            ClientRequest::GetRequests { workspace } => match self.engine.requests(WsRef::Id(workspace), vec![]).await {
                Ok(out) => ServerResponse::Requests { items: out.items },
                Err(e) => ServerResponse::Error { message: report::apply_err(&e) },
            },
            ClientRequest::Navigate { workspace, scene, step } => match self.engine.navigate(workspace, scene, step).await {
                Ok(()) => ServerResponse::Ok {},
                Err(message) => ServerResponse::Error { message },
            },
        }
    }

    /// Waits until the active tab has drawn `revision` of `workspace`.
    pub async fn wait_rendered(&self, workspace: &str, revision: u64) -> Browser {
        let mut rx = {
            let inner = self.inner.lock().unwrap();
            match &inner.active {
                None => return Browser::NotConnected,
                Some(a) if a.workspace.as_deref() != Some(workspace) => return Browser::OtherWorkspace,
                Some(a) => a.rendered.subscribe(),
            }
        };
        let done = tokio::time::timeout(APPLY_WAIT, rx.wait_for(|r| r.as_ref().is_some_and(|r| r.workspace == workspace && r.revision >= revision))).await;
        match done {
            Ok(Ok(r)) => Browser::Rendered {
                warnings: r.as_ref().map(|r| r.warnings.clone()).unwrap_or_default(),
                spans: r.as_ref().map(|r| r.spans.clone()).unwrap_or_default(),
            },
            // Tab closed or taken over while waiting.
            Ok(Err(_)) => Browser::NotConnected,
            Err(_) => Browser::Timeout,
        }
    }

    /// Sends `request` to the active tab if it shows `workspace`, and waits for the reply.
    async fn ask(&self, workspace: &str, request: ServerRequest, wait: Duration) -> Result<BrowserReply, Browser> {
        let (id, rx) = {
            let mut inner = self.inner.lock().unwrap();
            let tx = match &inner.active {
                None => return Err(Browser::NotConnected),
                Some(a) if a.workspace.as_deref() != Some(workspace) => return Err(Browser::OtherWorkspace),
                Some(a) => a.tx.clone(),
            };
            inner.next_request += 1;
            let id = inner.next_request;
            let (otx, orx) = oneshot::channel();
            inner.pending.insert(id, otx);
            let _ = tx.send(ServerMsg::Request { id, request });
            (id, orx)
        };
        let r = tokio::time::timeout(wait, rx).await;
        self.inner.lock().unwrap().pending.remove(&id);
        match r {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(_)) => Err(Browser::NotConnected),
            Err(_) => Err(Browser::Timeout),
        }
    }

    /// Asks the active tab for its current scene, view and warnings.
    pub async fn get_view(&self, workspace: &str) -> Browser {
        match self.ask(workspace, ServerRequest::GetView {}, VIEW_WAIT).await {
            Ok(BrowserReply::View { workspace: w, view, warnings, spans, .. }) if w == workspace => Browser::View { view, warnings, spans },
            Ok(_) => Browser::OtherWorkspace,
            Err(b) => b,
        }
    }

    /// PNG of the active tab's canvas: `Ok(base64)` or why there is none.
    pub async fn snapshot(&self, workspace: &str) -> Result<(String, u32, u32), String> {
        match self.ask(workspace, ServerRequest::Snapshot { max_side: SNAPSHOT_MAX_SIDE }, SNAPSHOT_WAIT).await {
            Ok(BrowserReply::Png { workspace: w, data, width, height }) if w == workspace => Ok((data, width, height)),
            Ok(BrowserReply::Error { message }) => Err(format!("브라우저: {message}")),
            Ok(_) => Err(report::browser_line(&Browser::OtherWorkspace)),
            Err(b) => Err(report::browser_line(&b)),
        }
    }
}
