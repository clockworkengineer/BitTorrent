//! BitTorrent WebUI Gateway (bt_webui)
//!
//! Exposes an Axum HTTP and WebSocket server that serves an embedded HTML dashboard
//! and forwards commands to the running bt_daemon via Unix Domain Sockets or Named Pipes.

use axum::{
    routing::{get, post},
    response::{Html, IntoResponse},
    http::StatusCode,
    extract::{Path as AxumPath, Query, ws::{WebSocketUpgrade, WebSocket, Message}, State},
    Json, Router,
};
use torrent_client_shared::{IpcMessage, IpcReply};
use std::time::Duration;
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Clone)]
struct AppState {
    notify: Arc<tokio::sync::Notify>,
}

fn execute_ipc(msg: IpcMessage) -> Result<IpcReply, String> {
    let serialized = serde_json::to_string(&msg).map_err(|e| e.to_string())?;
    
    #[cfg(unix)]
    let response_str = {
        use std::os::unix::net::UnixStream;
        use std::io::{Write, BufRead, BufReader};
        let mut stream = UnixStream::connect("/tmp/bt-daemon.sock").map_err(|e| e.to_string())?;
        let mut msg_bytes = serialized.into_bytes();
        msg_bytes.push(b'\n');
        stream.write_all(&msg_bytes)?;
        stream.flush()?;
        let mut reader = BufReader::new(stream);
        let mut reply = String::new();
        reader.read_line(&mut reply)?;
        reply
    };

    #[cfg(windows)]
    let response_str = {
        use std::fs::OpenOptions;
        use std::io::{Write, BufReader, BufRead};
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("\\\\.\\pipe\\bt-daemon")
            .map_err(|e| e.to_string())?;
        let mut bytes = serialized.into_bytes();
        bytes.push(b'\n');
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(file);
        let mut reply = String::new();
        reader.read_line(&mut reply).map_err(|e| e.to_string())?;
        reply
    };

    let reply: IpcReply = serde_json::from_str(&response_str).map_err(|e| e.to_string())?;
    Ok(reply)
}

// REST Handlers

#[derive(serde::Deserialize)]
struct AddPayload {
    torrent_path: String,
    download_dir: Option<String>,
}

async fn add_torrent(State(state): State<AppState>, Json(payload): Json<AddPayload>) -> impl IntoResponse {
    match execute_ipc(IpcMessage::Add {
        torrent_path: payload.torrent_path,
        download_dir: payload.download_dir,
    }) {
        Ok(IpcReply::Success { message }) => {
            state.notify.notify_waiters();
            (StatusCode::OK, Json(serde_json::json!({ "status": "success", "message": message }))).into_response()
        }
        Ok(IpcReply::Error { reason }) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "status": "error", "reason": reason }))).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "status": "error", "reason": err }))).into_response(),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "status": "error", "reason": "Unexpected daemon reply" }))).into_response(),
    }
}

async fn pause_torrent(State(state): State<AppState>, AxumPath(info_hash): AxumPath<String>) -> impl IntoResponse {
    match execute_ipc(IpcMessage::Pause { info_hash }) {
        Ok(IpcReply::Success { message }) => {
            state.notify.notify_waiters();
            (StatusCode::OK, Json(serde_json::json!({ "status": "success", "message": message }))).into_response()
        }
        Ok(IpcReply::Error { reason }) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "status": "error", "reason": reason }))).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "status": "error", "reason": err }))).into_response(),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "status": "error", "reason": "Unexpected daemon reply" }))).into_response(),
    }
}

async fn resume_torrent(State(state): State<AppState>, AxumPath(info_hash): AxumPath<String>) -> impl IntoResponse {
    match execute_ipc(IpcMessage::Resume { info_hash }) {
        Ok(IpcReply::Success { message }) => {
            state.notify.notify_waiters();
            (StatusCode::OK, Json(serde_json::json!({ "status": "success", "message": message }))).into_response()
        }
        Ok(IpcReply::Error { reason }) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "status": "error", "reason": reason }))).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "status": "error", "reason": err }))).into_response(),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "status": "error", "reason": "Unexpected daemon reply" }))).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct DeleteParams {
    #[serde(default)]
    purge: bool,
}

async fn delete_torrent(State(state): State<AppState>, AxumPath(info_hash): AxumPath<String>, Query(params): Query<DeleteParams>) -> impl IntoResponse {
    match execute_ipc(IpcMessage::Remove { info_hash, delete_data: params.purge }) {
        Ok(IpcReply::Success { message }) => {
            state.notify.notify_waiters();
            (StatusCode::OK, Json(serde_json::json!({ "status": "success", "message": message }))).into_response()
        }
        Ok(IpcReply::Error { reason }) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "status": "error", "reason": reason }))).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "status": "error", "reason": err }))).into_response(),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "status": "error", "reason": "Unexpected daemon reply" }))).into_response(),
    }
}

// WebSocket Handler

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    loop {
        let reply = match execute_ipc(IpcMessage::Status) {
            Ok(IpcReply::StatusList { torrents }) => {
                serde_json::json!({ "torrents": torrents })
            }
            Ok(IpcReply::Error { reason }) => {
                serde_json::json!({ "error": reason })
            }
            Err(e) => {
                serde_json::json!({ "error": e })
            }
            _ => {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        if let Ok(serialized) = serde_json::to_string(&reply) {
            if socket.send(Message::Text(serialized.into())).await.is_err() {
                break;
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            _ = state.notify.notified() => {}
        }
    }
}

// Embedded Static Assets

async fn serve_html() -> impl IntoResponse {
    Html(include_str!("../frontend/index.html"))
}

async fn serve_css() -> impl IntoResponse {
    (
        [("content-type", "text/css")],
        include_str!("../frontend/style.css"),
    )
}

async fn serve_js() -> impl IntoResponse {
    (
        [("content-type", "application/javascript")],
        include_str!("../frontend/app.js"),
    )
}

#[tokio::main]
async fn main() {
    let notify = Arc::new(tokio::sync::Notify::new());
    let state = AppState { notify };

    let app = Router::new()
        // Serve SPA assets
        .route("/", get(serve_html))
        .route("/style.css", get(serve_css))
        .route("/app.js", get(serve_js))
        // API routes
        .route("/api/ws", get(ws_handler))
        .route("/api/torrents/add", post(add_torrent))
        .route("/api/torrents/:info_hash/pause", post(pause_torrent))
        .route("/api/torrents/:info_hash/resume", post(resume_torrent))
        .route("/api/torrents/:info_hash", axum::routing::delete(delete_torrent))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
    println!("WebUI Server running at http://{}", addr);
    
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
