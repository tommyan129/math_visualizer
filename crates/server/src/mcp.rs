//! MCP endpoint (public listener): `apply`, `get_state`, `get_requests`, `snapshot`, `describe`.
//! Tool descriptions stay short because they sit in every conversation's context.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;

use crate::describe::describe;
use crate::engine::{EngineHandle, WsRef};
use crate::hub::Hub;
use crate::report;

#[derive(Clone)]
pub struct Mcp {
    engine: EngineHandle,
    hub: Hub,
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ApplyArgs {
    /// Workspace key the user gave you.
    pub key: String,
    /// DSL request (see `describe`).
    pub source: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct KeyArgs {
    /// Workspace key the user gave you.
    pub key: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct RequestsArgs {
    /// Workspace key the user gave you.
    pub key: String,
    /// Also list requests already marked done.
    pub all: Option<bool>,
    /// Request numbers to mark done once you have handled them.
    pub done: Option<Vec<u64>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct DescribeArgs {
    /// Command or topic; omit for the list.
    pub name: Option<String>,
}

fn ok(text: String) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text)])
}

fn err(text: String) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(text)])
}

#[tool_router]
impl Mcp {
    pub fn new(engine: EngineHandle, hub: Hub) -> Self {
        Mcp { engine, hub, tool_router: Self::tool_router() }
    }

    #[tool(description = "Apply a DSL request to the workspace (all or nothing). Returns what changed and what the server did automatically, or every error as `line:col [code] message → suggestion`. Syntax: describe.")]
    async fn apply(&self, Parameters(a): Parameters<ApplyArgs>) -> CallToolResult {
        match self.engine.apply(WsRef::Key(a.key), a.source).await {
            Ok(r) => {
                let browser = self.hub.wait_rendered(&r.workspace, r.revision).await;
                ok(report::apply_ok(&r, &browser))
            }
            Err(e) => err(report::apply_err(&e)),
        }
    }

    #[tool(description = "Current screen state of the workspace: scene, definitions, slider values, current view, warnings.")]
    async fn get_state(&self, Parameters(a): Parameters<KeyArgs>) -> CallToolResult {
        match self.engine.resolve(a.key).await {
            Ok(w) => {
                let browser = self.hub.get_view(&w.info.id).await;
                ok(report::state_text(&w, &browser))
            }
            Err(e) => err(report::key_error(&e)),
        }
    }

    #[tool(description = "The user's feedback requests (req): what each points at, its memo, and whether the target changed since (unchanged / modified / deleted). Pending ones by default. Pass done=[n] to mark requests you have handled.")]
    async fn get_requests(&self, Parameters(a): Parameters<RequestsArgs>) -> CallToolResult {
        let marked = a.done.unwrap_or_default();
        match self.engine.requests(WsRef::Key(a.key), marked.clone()).await {
            Ok(out) => ok(report::requests_text(&out, a.all.unwrap_or(false), &marked)),
            Err(e) => err(report::apply_err(&e)),
        }
    }

    #[tool(description = "PNG of the browser canvas including labels. Only for visual questions get_state cannot answer (color distinction, overlapping labels).")]
    async fn snapshot(&self, Parameters(a): Parameters<KeyArgs>) -> CallToolResult {
        match self.engine.resolve(a.key).await {
            Ok(w) => match self.hub.snapshot(&w.info.id).await {
                Ok((png, width, height)) => CallToolResult::success(vec![ContentBlock::image(png, "image/png"), ContentBlock::text(format!("{width}×{height} px · rev {}", w.info.revision))]),
                Err(m) => err(m),
            },
            Err(e) => err(report::key_error(&e)),
        }
    }

    #[tool(description = "DSL help. No name: list of commands. With a name: syntax and examples. No key needed.")]
    async fn describe(&self, Parameters(a): Parameters<DescribeArgs>) -> String {
        describe(a.name.as_deref())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Mcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("math-visualizer", env!("CARGO_PKG_VERSION")))
            .with_instructions("수식 시각화 도구. 사용자가 준 작업 공간 key를 apply/get_state/get_requests/snapshot에 넣는다. 문법은 describe로 필요할 때만 조회. 사용자가 요청사항을 확인해 달라고 하면 get_requests.")
    }
}
