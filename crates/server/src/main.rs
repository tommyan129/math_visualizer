//! Two listeners, split by port so a proxy mistake cannot expose key management:
//! public = MCP only; private = browser page, WebSocket, key management.

mod describe;
mod engine;
mod hub;
mod keys;
mod mcp;
mod private;
mod report;
mod requests;
mod store;

use std::path::PathBuf;

use dsl::interp::Limits;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager};
use tracing_subscriber::EnvFilter;

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

#[tokio::main]
async fn main() -> Result<(), String> {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())).init();

    let public_addr = env("MV_PUBLIC_ADDR", "127.0.0.1:7800");
    let private_addr = env("MV_PRIVATE_ADDR", "127.0.0.1:7801");
    let db = PathBuf::from(env("MV_DB", "data/mv.db"));
    let web_dist = PathBuf::from(env("MV_WEB_DIST", "web/dist"));
    if let Some(dir) = db.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }

    let engine = engine::start(&db, Limits::default())?;
    let hub = hub::Hub::new(engine.clone());

    let mcp_service: StreamableHttpService<mcp::Mcp, LocalSessionManager> = StreamableHttpService::new(
        {
            let (engine, hub) = (engine.clone(), hub.clone());
            move || Ok(mcp::Mcp::new(engine.clone(), hub.clone()))
        },
        Default::default(),
        StreamableHttpServerConfig::default(),
    );
    let public = axum::Router::new().nest_service("/mcp", mcp_service);
    let private = private::router(hub, web_dist);

    let public_listener = tokio::net::TcpListener::bind(&public_addr).await.map_err(|e| format!("{public_addr}: {e}"))?;
    let private_listener = tokio::net::TcpListener::bind(&private_addr).await.map_err(|e| format!("{private_addr}: {e}"))?;
    tracing::info!("MCP     http://{public_addr}/mcp");
    tracing::info!("browser http://{private_addr}/  (db {})", db.display());

    let shutdown = || async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let (a, b) = tokio::join!(
        axum::serve(public_listener, public).with_graceful_shutdown(shutdown()),
        axum::serve(private_listener, private).with_graceful_shutdown(shutdown()),
    );
    a.and(b).map_err(|e| e.to_string())
}
