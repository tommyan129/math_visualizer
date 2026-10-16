//! Private listener (VPN / localhost only): browser page, WebSocket, key management.

use std::path::PathBuf;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures::{SinkExt, StreamExt};
use model::protocol::ClientMsg;
use tower_http::services::{ServeDir, ServeFile};

use crate::hub::Hub;

pub fn router(hub: Hub, web_dist: PathBuf) -> Router {
    let index = web_dist.join("index.html");
    Router::new().route("/ws", get(ws)).with_state(hub).fallback_service(ServeDir::new(web_dist).fallback(ServeFile::new(index)))
}

/// WebSockets are not covered by CORS: without this check any web page the user
/// visits could open the socket and create keys. Origin must match Host.
fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        // Non-browser clients send no Origin.
        return true;
    };
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else { return false };
    origin.split_once("://").is_some_and(|(_, authority)| authority.trim_end_matches('/') == host)
}

async fn ws(State(hub): State<Hub>, headers: HeaderMap, upgrade: WebSocketUpgrade) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "origin").into_response();
    }
    upgrade.on_upgrade(move |socket| session(hub, socket))
}

async fn session(hub: Hub, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let mut conn = hub.connect();
    loop {
        tokio::select! {
            incoming = stream.next() => {
                let Some(Ok(msg)) = incoming else { break };
                match msg {
                    Message::Text(text) => match serde_json::from_str::<ClientMsg>(&text) {
                        Ok(m) => hub.on_message(&conn, m).await,
                        Err(e) => tracing::warn!("bad client message: {e}"),
                    },
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            outgoing = conn.rx.recv() => {
                let Some(m) = outgoing else { break };
                let text = serde_json::to_string(&m).expect("server message serializes");
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
        }
    }
    hub.disconnect(conn.id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(origin: Option<&str>, host: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_str(host).unwrap());
        if let Some(o) = origin {
            h.insert(header::ORIGIN, HeaderValue::from_str(o).unwrap());
        }
        h
    }

    #[test]
    fn origin_must_match_host() {
        assert!(same_origin(&headers(Some("http://127.0.0.1:7801"), "127.0.0.1:7801")));
        assert!(same_origin(&headers(Some("http://127.0.0.1:5173"), "127.0.0.1:5173")));
        assert!(!same_origin(&headers(Some("https://evil.example"), "127.0.0.1:7801")));
        assert!(same_origin(&headers(None, "127.0.0.1:7801")));
    }
}
