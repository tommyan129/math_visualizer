//! Server <-> browser messages over one WebSocket (design.md "서버↔브라우저 통신").
//! JSON, internally tagged envelopes; request/response pairs share an `id`.

use dsl::interp::{Note, UserOp};
use dsl::state::State;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Bumped whenever a message or state-model shape changes incompatibly.
/// A browser with a different version is told to reload.
pub const PROTOCOL_VERSION: u32 = 6;

/// Browser -> server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum ClientMsg {
    /// First message. Makes this tab the active one (the previous tab gets `TakenOver`).
    Hello { protocol: u32 },
    /// Browser-initiated request; answered by `ServerMsg::Response` with the same id.
    Request {
        #[ts(type = "number")]
        id: u64,
        request: ClientRequest,
    },
    /// Answer to a `ServerMsg::Request`.
    Reply {
        #[ts(type = "number")]
        id: u64,
        reply: BrowserReply,
    },
    /// Sent after the browser has drawn a snapshot; `apply` waits briefly for this.
    Rendered {
        workspace: String,
        #[ts(type = "number")]
        revision: u64,
        warnings: Vec<Warning>,
        /// Dimension each shown span was drawn with (stage 4).
        #[serde(default)]
        spans: Vec<SpanDim>,
        /// Points each shown feature-point definition found (stage 5b).
        #[serde(default)]
        features: Vec<FeaturePoints>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum ClientRequest {
    CreateWorkspace { name: String },
    /// Start receiving snapshots of this workspace.
    OpenWorkspace { workspace: String },
    /// Generate a new key; the old one stops working immediately.
    ReplaceKey { workspace: String },
    UserOp { workspace: String, op: UserOp },
    /// DSL written by the user; same path as Claude's `apply` (no key: private listener).
    Apply { workspace: String, source: String },
    /// Requests with their change status (requests tab).
    GetRequests { workspace: String },
    /// Show `scene`. Same scene and a `step`: walk to that step (a step overwrites only
    /// what it mentions). Another scene: switch to it at `step` or where it was left.
    Navigate { workspace: String, scene: String, step: Option<u32> },
}

/// Server -> browser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
#[allow(clippy::large_enum_variant)]
pub enum ServerMsg {
    Welcome { protocol: u32 },
    /// Sent instead of `Welcome` when the browser's protocol differs.
    ReloadRequired { protocol: u32 },
    /// Another tab said `Hello`; this one stops receiving updates.
    TakenOver {},
    /// Full workspace list; sent on connect and whenever it changes.
    Workspaces { list: Vec<WorkspaceInfo> },
    /// Full state of the open workspace; drop it if `revision` is older than the last one.
    Snapshot { snapshot: Snapshot },
    Response {
        #[ts(type = "number")]
        id: u64,
        response: ServerResponse,
    },
    /// Server-initiated request (e.g. `get_state` needs the current view).
    Request {
        #[ts(type = "number")]
        id: u64,
        request: ServerRequest,
    },
    /// A new Claude scene in `workspace`: open it (the snapshot then shows the scene).
    Navigate { workspace: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum ServerResponse {
    Ok {},
    Error { message: String },
    Created { workspace: WorkspaceInfo },
    /// Shown once; the server keeps only its hash.
    Key { workspace: String, key: String },
    /// `apply` result lines (same text Claude gets, without the browser line).
    Applied { lines: Vec<String> },
    /// Rendered error lines; nothing was applied.
    Rejected { lines: Vec<String> },
    Requests { items: Vec<ReqView> },
}

/// A req with its change status, computed from the request log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReqView {
    #[ts(type = "number")]
    pub number: u64,
    pub src: String,
    /// History step of the req.
    #[ts(type = "number")]
    pub seq: u64,
    /// Written in the browser (otherwise by Claude).
    pub by_user: bool,
    pub done: bool,
    pub status: ReqStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum ReqStatus {
    Unchanged {},
    /// `via`: the referenced object whose change made this one change (indirect).
    Modified {
        #[ts(type = "number")]
        seq: u64,
        current: String,
        via: Option<String>,
    },
    /// `cause`: e.g. "f 삭제로"; `None` = deleted by name.
    Deleted {
        #[ts(type = "number")]
        seq: u64,
        cause: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum ServerRequest {
    /// Current scene, view and warnings of the open workspace.
    GetView {},
    /// PNG of the canvas, long side at most `max_side` px.
    Snapshot { max_side: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum BrowserReply {
    View {
        workspace: String,
        scene: String,
        view: ViewBox,
        warnings: Vec<Warning>,
        #[serde(default)]
        spans: Vec<SpanDim>,
        #[serde(default)]
        features: Vec<FeaturePoints>,
    },
    /// Base64 PNG.
    Png { workspace: String, data: String, width: u32, height: u32 },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkspaceInfo {
    pub id: String,
    pub name: String,
    pub has_key: bool,
    /// Last log sequence (0 = empty).
    #[ts(type = "number")]
    pub revision: u64,
    /// Unix milliseconds of the last change, for "최근" ordering.
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Snapshot {
    pub workspace: String,
    #[ts(type = "number")]
    pub revision: u64,
    pub state: State,
    /// Applied-result notes of the latest entry (e.g. for an "updated" hint).
    pub notes: Vec<Note>,
    pub nav: Nav,
}

/// Which scene and step the workspace shows. Held by the server (not logged).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Nav {
    pub scene: Option<String>,
    pub step: u32,
    /// Scenes changed while not shown ("갱신됨" dot on the tab).
    pub updated: Vec<String>,
}

/// Visible world rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ViewBox {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
}

/// How the browser drew a span from the current values: 0 = the origin (point),
/// 1 = a line, 2 = a plane (design.md "span").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SpanDim {
    pub key: String,
    pub dim: u32,
}

/// What a feature-point definition found at the current values (stage 5b), for
/// `get_state` and the `apply` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FeaturePoints {
    pub key: String,
    pub points: Vec<FeaturePoint>,
    /// More were found than drawn (count cap).
    #[serde(default)]
    pub capped: bool,
}

/// One point in the scene's axes. `y` is `None` for a vertical asymptote (a line).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FeaturePoint {
    pub x: f64,
    pub y: Option<f64>,
    pub class: FeatureClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum FeatureClass {
    Root,
    /// Root without a sign change (double root).
    TangentRoot,
    Intersection,
    /// Intersection where the curves touch.
    TangentIntersection,
    YIntercept,
    Max,
    Min,
    Inflection,
    IntervalMax,
    IntervalMin,
    Hole,
    /// Value at a hole that is defined elsewhere (removable discontinuity with a point).
    Filled,
    Asymptote,
    Jump,
    Endpoint,
    Cusp,
    HorizontalTangent,
    VerticalTangent,
}

impl FeatureClass {
    pub fn word(self) -> &'static str {
        match self {
            Self::Root => "근",
            Self::TangentRoot => "접하는 근",
            Self::Intersection => "교점",
            Self::TangentIntersection => "접하는 교점",
            Self::YIntercept => "y절편",
            Self::Max => "극대",
            Self::Min => "극소",
            Self::Inflection => "변곡점",
            Self::IntervalMax => "최댓값",
            Self::IntervalMin => "최솟값",
            Self::Hole => "구멍",
            Self::Filled => "구멍 자리의 값",
            Self::Asymptote => "수직 점근선",
            Self::Jump => "점프",
            Self::Endpoint => "정의역 끝점",
            Self::Cusp => "뾰족점",
            Self::HorizontalTangent => "수평 접선",
            Self::VerticalTangent => "수직 접선",
        }
    }
}

/// Render-time warning reported by the browser (shown under the definition and in `get_state`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Warning {
    /// Definition key, or `None` for scene-wide warnings.
    pub source: Option<String>,
    /// Fixed code, e.g. `W-budget`, `W-sum-terms`.
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_is_internally_tagged() {
        let json = serde_json::to_string(&ServerMsg::Welcome { protocol: 1 }).unwrap();
        assert_eq!(json, r#"{"type":"Welcome","protocol":1}"#);
        let req = ClientMsg::Request { id: 7, request: ClientRequest::OpenWorkspace { workspace: "w1".into() } };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["type"], "Request");
        assert_eq!(json["request"]["type"], "OpenWorkspace");
        assert_eq!(serde_json::from_value::<ClientMsg>(json).unwrap(), req);
    }

    #[test]
    fn user_op_round_trip() {
        let m = ClientMsg::Request {
            id: 1,
            request: ClientRequest::UserOp { workspace: "w1".into(), op: UserOp::SetValue { constant: "a".into(), value: 0.5 } },
        };
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<ClientMsg>(&s).unwrap(), m);
    }
}
