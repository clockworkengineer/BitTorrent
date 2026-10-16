//! In-App HTTP Media Streaming Server for Desktop GUI Client
//!
//! Exposes an active TorrentSession or File as an HTTP streaming endpoint
//! supporting Range requests (`206 Partial Content`), MIME type detection,
//! seek operations, and media player integration.

use bittorrent_rs::internals::TorrentContext;
use bittorrent_rs::storage::stream::{StorageBlockReader, TorrentStream};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use torrent_client_shared::detect_mime_type;

/// State of an active stream exposed by the server.
#[derive(Clone)]
pub struct ActiveStream {
    pub torrent_name: String,
    pub file_name: String,
    pub context: Arc<Mutex<TorrentContext>>,
    pub file_index: Option<usize>,
    pub total_size: u64,
    pub current_position: Arc<AtomicU64>,
}

/// An embedded HTTP media streaming server.
pub struct StreamServer {
    port: u16,
    running: Arc<AtomicBool>,
    active_stream: Arc<Mutex<Option<ActiveStream>>>,
}

impl StreamServer {
    /// Spawns a new streaming server on a free ephemeral loopback port (`127.0.0.1:0`).
    pub fn new() -> Result<Self, std::io::Error> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let running = Arc::new(AtomicBool::new(true));
        let active_stream = Arc::new(Mutex::new(None));

        let running_clone = running.clone();
        let active_stream_clone = active_stream.clone();

        std::thread::spawn(move || {
            let _ = listener.set_nonblocking(true);
            while running_clone.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let active = active_stream_clone.clone();
                        std::thread::spawn(move || {
                            handle_stream_connection(stream, active);
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            port,
            running,
            active_stream,
        })
    }

    /// Returns the port the server is listening on.
    #[allow(dead_code)]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Returns the URL pointing to the active media stream.
    pub fn stream_url(&self) -> String {
        format!("http://127.0.0.1:{}/stream", self.port)
    }

    /// Sets the active torrent and file to stream.
    pub fn set_active_stream(
        &self,
        torrent_name: String,
        file_name: String,
        context: Arc<Mutex<TorrentContext>>,
        file_index: Option<usize>,
        total_size: u64,
    ) {
        let mut stream_guard = self.active_stream.lock().unwrap();
        *stream_guard = Some(ActiveStream {
            torrent_name,
            file_name,
            context,
            file_index,
            total_size,
            current_position: Arc::new(AtomicU64::new(0)),
        });
    }

    /// Returns the currently active stream details, if any.
    pub fn active_stream(&self) -> Option<ActiveStream> {
        self.active_stream.lock().unwrap().clone()
    }

    /// Stops the currently active stream.
    pub fn stop_active_stream(&self) {
        let mut stream_guard = self.active_stream.lock().unwrap();
        *stream_guard = None;
    }

    /// Opens the current stream URL in the system's default media player / browser.
    pub fn open_in_player(&self) -> bool {
        let url = self.stream_url();
        webbrowser::open(&url).is_ok()
    }
}

impl Drop for StreamServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

fn create_torrent_stream(
    context: Arc<Mutex<TorrentContext>>,
    file_index: Option<usize>,
) -> Result<TorrentStream, bittorrent_rs::error::BitTorrentError> {
    let cursor = Arc::new(core::sync::atomic::AtomicU32::new(0));
    {
        let mut ctx = context.lock().unwrap();
        ctx.selector = Arc::new(bittorrent_rs::core::selector::StreamingPieceSelector::new(
            cursor.clone(),
            8,
        ));
    }

    match file_index {
        Some(idx) => TorrentStream::for_file(context, idx, Some(cursor)),
        None => {
            let (storage, total_len) = {
                let ctx = context.lock().unwrap();
                (ctx.storage.clone(), ctx.total_bytes_to_download)
            };
            Ok(TorrentStream::new(
                context,
                Arc::new(StorageBlockReader(storage)),
                0,
                total_len,
                Some(cursor),
            ))
        }
    }
}

fn handle_stream_connection(mut socket: TcpStream, active_stream: Arc<Mutex<Option<ActiveStream>>>) {
    let _ = socket.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let _ = socket.set_write_timeout(Some(std::time::Duration::from_secs(10)));

    let mut reader = BufReader::new(match socket.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }

    let mut range_header = None;
    let mut header_line = String::new();
    while reader.read_line(&mut header_line).unwrap_or(0) > 0 {
        let trimmed = header_line.trim();
        if trimmed.is_empty() {
            break;
        }
        if trimmed.to_lowercase().starts_with("range:") {
            range_header = Some(trimmed["range:".len()..].trim().to_string());
        }
        header_line.clear();
    }

    let active = match active_stream.lock().unwrap().clone() {
        Some(s) => s,
        None => {
            let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 17\r\n\r\nNo stream active\n";
            let _ = socket.write_all(resp.as_bytes());
            return;
        }
    };

    let is_head = request_line.starts_with("HEAD");

    let mut torrent_stream = match create_torrent_stream(active.context.clone(), active.file_index) {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("Failed to initialize stream: {}", e);
            let resp = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\n\r\n{}",
                msg.len(),
                msg
            );
            let _ = socket.write_all(resp.as_bytes());
            return;
        }
    };

    let total_len = torrent_stream.total_len();
    if total_len == 0 {
        let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        return;
    }

    let mime_type = detect_mime_type(&active.file_name);

    let mut range_start = 0u64;
    let mut range_end = total_len - 1;
    let is_range_request = if let Some(ref r) = range_header {
        if r.starts_with("bytes=") {
            let spec = &r["bytes=".len()..];
            let parts: Vec<&str> = spec.split('-').collect();
            if parts.len() == 2 {
                if let Ok(s) = parts[0].parse::<u64>() {
                    range_start = s;
                }
                if !parts[1].is_empty() {
                    if let Ok(e) = parts[1].parse::<u64>() {
                        range_end = e.min(total_len - 1);
                    }
                }
                true
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    if range_start >= total_len {
        let response = format!(
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\n\r\n",
            total_len
        );
        let _ = socket.write_all(response.as_bytes());
        return;
    }

    let content_len = range_end - range_start + 1;
    let status_line = if is_range_request {
        "HTTP/1.1 206 Partial Content"
    } else {
        "HTTP/1.1 200 OK"
    };

    let mut headers = format!(
        "{}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n",
        status_line, mime_type, content_len
    );

    if is_range_request {
        headers.push_str(&format!(
            "Content-Range: bytes {}-{}/{}\r\n",
            range_start, range_end, total_len
        ));
    }
    headers.push_str("\r\n");

    if socket.write_all(headers.as_bytes()).is_err() {
        return;
    }

    if is_head {
        return;
    }

    if torrent_stream.seek(SeekFrom::Start(range_start)).is_err() {
        return;
    }

    active.current_position.store(range_start, Ordering::Relaxed);

    let mut remaining = content_len;
    let mut buf = vec![0u8; 64 * 1024];

    while remaining > 0 {
        let to_read = (buf.len() as u64).min(remaining) as usize;
        match torrent_stream.read(&mut buf[..to_read]) {
            Ok(0) => break,
            Ok(n) => {
                if socket.write_all(&buf[..n]).is_err() {
                    break;
                }
                let _ = socket.flush();
                remaining -= n as u64;
                active
                    .current_position
                    .fetch_add(n as u64, Ordering::Relaxed);
            }
            Err(_) => break,
        }
    }
}
