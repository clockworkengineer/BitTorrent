//! HTTP Media Streaming Server (Single Responsibility Principle).
//!
//! Provides HTTP range-request media streaming from active torrent sessions
//! via `torrent_client_shared`.

use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use torrent_client_shared::{
    FileInfo, find_primary_media_file, is_media_filename, read_http_request,
    send_cors_preflight_response, send_http_error, serve_http_range_stream,
};

use crate::engine::Engine;
use crate::session_pool::info_hash_hex;

/// Handles an incoming HTTP streaming connection.
pub fn handle_stream_client(mut stream: TcpStream, engine: Arc<Mutex<Engine>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });

    let req = match read_http_request(&mut reader) {
        Ok(Some(r)) => r,
        _ => return,
    };

    if req.method == "OPTIONS" {
        let _ = send_cors_preflight_response(&mut stream);
        return;
    }

    if req.method != "GET" && req.method != "HEAD" {
        let _ = send_http_error(&mut stream, 405, "Method Not Allowed", "Method Not Allowed");
        return;
    }

    let is_head = req.method == "HEAD";

    // Path pattern: /stream/:info_hash or /stream/:info_hash/:file_index or /stream/:info_hash/files
    let clean_path = req.path.split('?').next().unwrap_or(&req.path);
    let segments: Vec<&str> = clean_path.trim_start_matches('/').split('/').collect();
    if segments.is_empty() || segments[0] != "stream" || segments.len() < 2 {
        let _ = send_http_error(&mut stream, 404, "Not Found", "Not Found");
        return;
    }

    let target_hash = segments[1];

    let (torrent_stream_res, file_name) = {
        let e = match engine.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        let session_state = match e
            .pool
            .sessions()
            .iter()
            .find(|s| info_hash_hex(&s.session).eq_ignore_ascii_case(target_hash))
        {
            Some(s) => s,
            None => {
                let _ = send_http_error(&mut stream, 404, "Not Found", "Torrent not found");
                return;
            }
        };

        if segments.len() >= 3
            && segments[2] == "files"
            && let Ok(ctx) = session_state.session.context().lock()
        {
            let files: Vec<FileInfo> = ctx
                .files_to_download
                .iter()
                .enumerate()
                .map(|(idx, f)| FileInfo {
                    index: idx,
                    name: f.torrent_path.clone(),
                    length: f.length,
                    is_media: is_media_filename(&f.torrent_path),
                    priority: f.priority.as_str().to_string(),
                })
                .collect();
            drop(ctx);
            drop(e);
            let json = serde_json::to_string(&files).unwrap_or_else(|_| "[]".into());
            let response = format!(
                "HTTP/1.1 200 OK\r\n\
Content-Type: application/json\r\n\
Access-Control-Allow-Origin: *\r\n\
Content-Length: {}\r\n\r\n{}",
                json.len(),
                json
            );
            let _ = stream.write_all(response.as_bytes());
            return;
        }

        let file_index_opt: Option<usize> = if segments.len() >= 3 {
            segments[2].parse::<usize>().ok()
        } else {
            None
        };

        let target_file_index = match file_index_opt {
            Some(idx) => Some(idx),
            None => {
                if let Ok(ctx) = session_state.session.context().lock() {
                    find_primary_media_file(
                        ctx.files_to_download
                            .iter()
                            .map(|f| (&f.torrent_path, f.length)),
                    )
                } else {
                    None
                }
            }
        };

        let file_name = if let Some(idx) = target_file_index {
            if let Ok(ctx) = session_state.session.context().lock() {
                ctx.files_to_download
                    .get(idx)
                    .map(|f| f.torrent_path.clone())
                    .unwrap_or_else(|| session_state.last_file_name.clone())
            } else {
                session_state.last_file_name.clone()
            }
        } else {
            session_state.last_file_name.clone()
        };

        let stream_res = match target_file_index {
            Some(idx) => session_state.session.create_file_stream(idx),
            None => session_state.session.create_stream(),
        };

        (stream_res, file_name)
    };

    let mut torrent_stream = match torrent_stream_res {
        Ok(s) => s,
        Err(err) => {
            let msg = format!("Failed to create stream: {}", err);
            let _ = send_http_error(&mut stream, 500, "Internal Server Error", &msg);
            return;
        }
    };

    let total_len = torrent_stream.total_len();

    let _ = serve_http_range_stream(
        &mut stream,
        &mut torrent_stream,
        &file_name,
        total_len,
        is_head,
        req.range_header.as_deref(),
        |_, _| {},
    );
}

/// Starts the HTTP media streaming background listener on the given port.
pub fn start_stream_server(engine: Arc<Mutex<Engine>>, port: u16) {
    std::thread::spawn(move || {
        let addr = format!("0.0.0.0:{}", port);
        let listener = match TcpListener::bind(&addr) {
            Ok(l) => {
                println!(
                    "HTTP Media Streaming Server listening on http://127.0.0.1:{}",
                    port
                );
                l
            }
            Err(e) => {
                eprintln!("Failed to bind HTTP stream server on {}: {}", addr, e);
                return;
            }
        };

        for tcp_stream in listener.incoming().flatten() {
            let eng = engine.clone();
            std::thread::spawn(move || {
                handle_stream_client(tcp_stream, eng);
            });
        }
    });
}
