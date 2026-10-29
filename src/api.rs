//! Minimal REST API so other programs can drive the serial connection.
//!
//! Runs a blocking `tiny_http` server on a background thread (no async runtime,
//! matching the rest of the app). The server only *issues commands* over an mpsc
//! channel and *reads a shared status snapshot* — it never touches serial state
//! directly, so there are no locks on the hot path in the UI thread.
//!
//! Endpoints (bind to 127.0.0.1 only):
//!
//! | Method | Path                | Action                                        |
//! |--------|---------------------|-----------------------------------------------|
//! | GET    | `/api/status`       | JSON snapshot: connected, port, reconnect_in  |
//! | POST   | `/api/disconnect`   | Disconnect the port now                        |
//! | POST   | `/api/reconnect`    | Reconnect to the last port/config now          |
//! | POST   | `/api/cycle?secs=30`| Disconnect now, auto-reconnect after N seconds |

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

/// Commands the API thread hands to the UI thread.
#[derive(Debug, Clone)]
pub enum ApiCommand {
    Disconnect,
    Reconnect,
    /// Disconnect immediately, then auto-reconnect after `secs` seconds.
    CycleReconnect {
        secs: u64,
    },
}

/// Status snapshot the UI thread publishes for the API thread to read.
#[derive(Debug, Default, Clone)]
pub struct ApiState {
    pub connected: bool,
    pub port: String,
    /// Seconds remaining until a pending auto-reconnect, if one is scheduled.
    pub reconnect_in_secs: Option<u64>,
}

/// Default port for the REST API (localhost only).
pub const DEFAULT_API_PORT: u16 = 8791;

/// Spawn the HTTP server on a background thread. Returns an error if the bind
/// fails (e.g. port already in use) so the caller can report it.
pub fn spawn_api_server(
    bind: &str,
    tx: Sender<ApiCommand>,
    state: Arc<Mutex<ApiState>>,
) -> Result<(), String> {
    let server = tiny_http::Server::http(bind).map_err(|e| e.to_string())?;
    thread::spawn(move || {
        for request in server.incoming_requests() {
            handle_request(request, &tx, &state);
        }
    });
    Ok(())
}

fn handle_request(
    request: tiny_http::Request,
    tx: &Sender<ApiCommand>,
    state: &Arc<Mutex<ApiState>>,
) {
    let method = request.method().clone();
    let url = request.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));

    let (code, body) = match (&method, path) {
        (tiny_http::Method::Get, "/api/status") | (tiny_http::Method::Get, "/") => {
            let snap = state.lock().map(|s| s.clone()).unwrap_or_default();
            let reconnect = snap
                .reconnect_in_secs
                .map(|n| n.to_string())
                .unwrap_or_else(|| "null".to_string());
            let json = format!(
                "{{\"connected\":{},\"port\":{:?},\"reconnect_in_secs\":{}}}",
                snap.connected, snap.port, reconnect
            );
            (200, json)
        }
        (tiny_http::Method::Post, "/api/disconnect") => {
            let _ = tx.send(ApiCommand::Disconnect);
            (202, r#"{"ok":true,"action":"disconnect"}"#.to_string())
        }
        (tiny_http::Method::Post, "/api/reconnect") => {
            let _ = tx.send(ApiCommand::Reconnect);
            (202, r#"{"ok":true,"action":"reconnect"}"#.to_string())
        }
        (tiny_http::Method::Post, "/api/cycle") => {
            let secs = parse_secs(query).unwrap_or(30);
            let _ = tx.send(ApiCommand::CycleReconnect { secs });
            (
                202,
                format!("{{\"ok\":true,\"action\":\"cycle\",\"secs\":{secs}}}"),
            )
        }
        _ => (404, r#"{"ok":false,"error":"not found"}"#.to_string()),
    };

    let response = tiny_http::Response::from_string(body).with_status_code(code);
    let response =
        match tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]) {
            Ok(h) => response.with_header(h),
            Err(_) => response,
        };
    let _ = request.respond(response);
}

/// Pull `secs=<u64>` out of a raw query string (`a=1&secs=30&b=2`).
fn parse_secs(query: &str) -> Option<u64> {
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == "secs")
        .and_then(|(_, v)| v.parse::<u64>().ok())
}
