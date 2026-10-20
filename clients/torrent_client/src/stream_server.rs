//! In-App HTTP Media Streaming Server for Desktop GUI Client
//!
//! Exposes an active TorrentSession or File as an HTTP streaming endpoint
//! supporting Range requests (`206 Partial Content`), MIME type detection,
//! seek operations, and media player integration.

use bittorrent_rs::internals::TorrentContext;
use bittorrent_rs::storage::stream::{StorageBlockReader, TorrentStream};
use std::io::BufReader;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use torrent_client_shared::{
    read_http_request, send_cors_preflight_response, send_http_error, serve_http_range_stream,
};

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

fn handle_stream_connection(
    mut socket: TcpStream,
    active_stream: Arc<Mutex<Option<ActiveStream>>>,
) {
    let _ = socket.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let _ = socket.set_write_timeout(Some(std::time::Duration::from_secs(10)));

    let mut reader = BufReader::new(match socket.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });

    let req = match read_http_request(&mut reader) {
        Ok(Some(r)) => r,
        _ => return,
    };

    if req.method == "OPTIONS" {
        let _ = send_cors_preflight_response(&mut socket);
        return;
    }

    let active = match active_stream.lock().unwrap().clone() {
        Some(s) => s,
        None => {
            let _ = send_http_error(&mut socket, 404, "Not Found", "No stream active\n");
            return;
        }
    };

    let is_head = req.method == "HEAD";

    let mut torrent_stream = match create_torrent_stream(active.context.clone(), active.file_index)
    {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("Failed to initialize stream: {}", e);
            let _ = send_http_error(&mut socket, 500, "Internal Server Error", &msg);
            return;
        }
    };

    let total_len = torrent_stream.total_len();
    let current_pos = active.current_position.clone();

    let _ = serve_http_range_stream(
        &mut socket,
        &mut torrent_stream,
        &active.file_name,
        total_len,
        is_head,
        req.range_header.as_deref(),
        |offset, _chunk_len| {
            current_pos.store(offset, Ordering::Relaxed);
        },
    );
}
