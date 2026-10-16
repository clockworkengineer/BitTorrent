//! BitTorrent Seeding & Backup Daemon (bt_daemon)
//!
//! A background daemon that hosts a bittorrent session manager, persists active torrent state,
//! and listens on a local Unix socket or Named Pipe for command-line instructions.

use bittorrent_rs::TorrentSession;
use torrent_client_shared::{
    extract_peer_infos, extract_piece_availability, FileInfo, IpcMessage, IpcReply,
    SessionState, TorrentStatusInfo,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::io::{Read, Write, Seek, SeekFrom, BufRead, BufReader};
use std::time::Duration;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Default)]
pub struct DaemonState {
    pub download_dir: String,
    pub torrents: Vec<SavedTorrent>,
}

#[derive(Serialize, Deserialize)]
pub struct SavedTorrent {
    pub torrent_path: String,
    pub bitfield_hex: String,
    #[serde(default)]
    pub fast_resume_file: Option<String>,
}

fn info_hash_hex(session: &TorrentSession) -> String {
    if let Ok(ctx) = session.context().lock() {
        bittorrent_rs::util::info_hash_to_string(&ctx.info_hash)
    } else {
        String::new()
    }
}

struct Engine {
    download_dir: String,
    sessions: Vec<SessionState>,
    state_file: PathBuf,
}

impl Engine {
    fn new(state_file: PathBuf, download_dir: String) -> Self {
        Self {
            download_dir,
            sessions: Vec::new(),
            state_file,
        }
    }

    fn save_state(&self) {
        let torrents = self.sessions.iter().map(|s| {
            let (bitfield_hex, hash_hex) = if let Ok(ctx) = s.session.context().lock() {
                let bf = ctx.bitfield.iter().map(|b| format!("{:02x}", b)).collect::<String>();
                let ih = bittorrent_rs::util::info_hash_to_string(&ctx.info_hash);
                (bf, ih)
            } else {
                (String::new(), String::new())
            };

            let fast_resume_file = if !hash_hex.is_empty() {
                let resume_path = self.state_file.with_file_name(format!("{}.fastresume", hash_hex));
                if s.session.save_fast_resume(&resume_path).is_ok() {
                    Some(resume_path.to_string_lossy().to_string())
                } else {
                    None
                }
            } else {
                None
            };

            SavedTorrent {
                torrent_path: s.torrent_path.clone(),
                bitfield_hex,
                fast_resume_file,
            }
        }).collect::<Vec<_>>();

        let state = DaemonState {
            download_dir: self.download_dir.clone(),
            torrents,
        };

        if let Ok(content) = serde_json::to_string_pretty(&state) {
            let _ = std::fs::write(&self.state_file, content);
        }
    }

    fn load_state(&mut self) {
        if self.state_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&self.state_file) {
                if let Ok(state) = serde_json::from_str::<DaemonState>(&content) {
                    self.download_dir = state.download_dir;
                    for t in state.torrents {
                        let _ = self.add_session(t.torrent_path, Some(t.bitfield_hex), t.fast_resume_file);
                    }
                }
            }
        }
    }

    fn add_session(
        &mut self,
        torrent_path: String,
        bitfield_hex: Option<String>,
        fast_resume_file: Option<String>,
    ) -> Result<(), String> {
        let torrent_path_buf = PathBuf::from(&torrent_path);
        let download_dir_buf = PathBuf::from(&self.download_dir);

        if self.sessions.iter().any(|s| s.torrent_path == torrent_path) {
            return Err("Torrent is already added".to_string());
        }

        let mut config = bittorrent_rs::session::SessionConfig::default();
        if bitfield_hex.is_some() && fast_resume_file.is_none() {
            config.skip_hash_check = true;
        }

        let session_res = if torrent_path.starts_with("magnet:?") {
            TorrentSession::from_magnet(&torrent_path, &download_dir_buf)
                .config(config)
                .selector(Arc::new(bittorrent_rs::RarestFirstSelector))
                .build()
        } else {
            let mut builder = TorrentSession::builder(&torrent_path_buf, &download_dir_buf)
                .config(config)
                .selector(Arc::new(bittorrent_rs::RarestFirstSelector));
            if let Some(ref fr_path) = fast_resume_file {
                builder = builder.fast_resume_file(fr_path);
            }
            builder.build()
        };

        let mut session = match session_res {
            Ok(s) => s,
            Err(e) => return Err(format!("Failed to create session: {}", e)),
        };

        // Restore bitfield
        if let Some(ref hex_str) = bitfield_hex {
            if let Ok(mut ctx) = session.context.lock() {
                let mut bytes = Vec::new();
                for i in (0..hex_str.len()).step_by(2) {
                    if i + 2 <= hex_str.len() {
                        if let Ok(b) = u8::from_str_radix(&hex_str[i..i+2], 16) {
                            bytes.push(b);
                        }
                    }
                }
                if bytes.len() == ctx.bitfield.len() {
                    ctx.bitfield = bytes.clone();
                    let mut downloaded = 0u64;
                    let mut missing_count = 0;
                    let number_of_pieces = ctx.number_of_pieces;
                    for piece_num in 0..number_of_pieces {
                        let (byte_idx, mask) = bittorrent_rs::util::get_bitfield_index_and_mask(piece_num as u32);
                        let local = (bytes[byte_idx] & mask) != 0;
                        ctx.mark_piece_missing(piece_num as u32, !local);
                        if local {
                            downloaded += ctx.get_piece_length(piece_num as u32) as u64;
                        } else {
                            missing_count += 1;
                        }
                    }
                    ctx.total_bytes_downloaded.store(downloaded, std::sync::atomic::Ordering::Relaxed);
                    ctx.missing_pieces_count = missing_count;
                    ctx.initial_bytes_downloaded = downloaded;
                }
            }
        }

        if let Err(e) = session.start_download() {
            return Err(format!("Failed to start download: {}", e));
        }

        let state = SessionState::new(session, torrent_path);
        self.sessions.push(state);
        self.save_state();
        Ok(())
    }

    fn pause_session(&mut self, target_hash: &str) -> Result<(), String> {
        if let Some(s) = self.sessions.iter_mut().find(|s| info_hash_hex(&s.session).to_lowercase() == target_hash.to_lowercase()) {
            s.session.pause().map_err(|e| e.to_string())?;
            self.save_state();
            Ok(())
        } else {
            Err("Torrent not found".to_string())
        }
    }

    fn resume_session(&mut self, target_hash: &str) -> Result<(), String> {
        if let Some(s) = self.sessions.iter_mut().find(|s| info_hash_hex(&s.session).to_lowercase() == target_hash.to_lowercase()) {
            s.session.resume().map_err(|e| e.to_string())?;
            self.save_state();
            Ok(())
        } else {
            Err("Torrent not found".to_string())
        }
    }

    fn remove_session(&mut self, target_hash: &str, delete_data: bool) -> Result<(), String> {
        if let Some(pos) = self.sessions.iter().position(|s| info_hash_hex(&s.session).to_lowercase() == target_hash.to_lowercase()) {
            let mut state = self.sessions.remove(pos);
            let _ = state.session.stop();
            if delete_data {
                if let Ok(ctx) = state.session.context().lock() {
                    for file in &ctx.files_to_download {
                        let path = Path::new(&file.name);
                        if path.exists() {
                            let _ = std::fs::remove_file(path);
                        }
                    }
                }
            }
            std::thread::spawn(move || {
                state.session.join_peer_workers();
            });
            self.save_state();
            Ok(())
        } else {
            Err("Torrent not found".to_string())
        }
    }

    fn get_status_info(&mut self) -> Vec<TorrentStatusInfo> {
        let mut torrent_infos = Vec::new();
        for session_state in &mut self.sessions {
            if let Ok(ctx_guard) = session_state.session.context().lock() {
                session_state.update_fields(&ctx_guard);
            }
            torrent_infos.push(TorrentStatusInfo {
                name: session_state.last_file_name.clone(),
                info_hash: info_hash_hex(&session_state.session),
                progress: session_state.last_progress,
                status: session_state.last_status.clone(),
                peers_connected: session_state.last_peers_connected,
                peers_active: session_state.last_peers_active,
                download_rate: session_state.last_bps,
                upload_rate: session_state.last_upload_bps,
                downloaded: session_state.last_downloaded,
                uploaded: session_state.last_uploaded,
                download_dir: session_state.session.download_path().display().to_string(),
                total_size: session_state.last_total,
                is_super_seeding: session_state.session.is_super_seeding(),
            });
        }
        torrent_infos
    }

    fn stop_all(&mut self) {
        for s in &mut self.sessions {
            let _ = s.session.stop();
        }
        for s in &mut self.sessions {
            s.session.join_peer_workers();
        }
    }
}

// Media Streaming Server Implementations

pub fn is_media_filename(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".mp4") || lower.ends_with(".mkv") || lower.ends_with(".webm") ||
    lower.ends_with(".avi") || lower.ends_with(".mov") || lower.ends_with(".m4v") ||
    lower.ends_with(".flv") || lower.ends_with(".wmv") || lower.ends_with(".mp3") ||
    lower.ends_with(".flac") || lower.ends_with(".ogg") || lower.ends_with(".wav") ||
    lower.ends_with(".aac") || lower.ends_with(".m4a")
}

pub fn detect_mime_type(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.ends_with(".mp4") || lower.ends_with(".m4v") {
        "video/mp4"
    } else if lower.ends_with(".mkv") {
        "video/x-matroska"
    } else if lower.ends_with(".webm") {
        "video/webm"
    } else if lower.ends_with(".avi") {
        "video/x-msvideo"
    } else if lower.ends_with(".mov") {
        "video/quicktime"
    } else if lower.ends_with(".mp3") {
        "audio/mpeg"
    } else if lower.ends_with(".flac") {
        "audio/flac"
    } else if lower.ends_with(".ogg") {
        "audio/ogg"
    } else if lower.ends_with(".wav") {
        "audio/wav"
    } else {
        "video/mp4"
    }
}

fn start_stream_server(engine: Arc<Mutex<Engine>>, port: u16) {
    std::thread::spawn(move || {
        let addr = format!("0.0.0.0:{}", port);
        let listener = match std::net::TcpListener::bind(&addr) {
            Ok(l) => {
                println!("HTTP Media Streaming Server listening on http://127.0.0.1:{}", port);
                l
            }
            Err(e) => {
                eprintln!("Failed to bind HTTP stream server on {}: {}", addr, e);
                return;
            }
        };

        for stream in listener.incoming() {
            if let Ok(tcp_stream) = stream {
                let eng = engine.clone();
                std::thread::spawn(move || {
                    handle_stream_client(tcp_stream, eng);
                });
            }
        }
    });
}

fn handle_stream_client(mut stream: std::net::TcpStream, engine: Arc<Mutex<Engine>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
        return;
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }

    let method = parts[0];
    let path = parts[1];

    if method == "OPTIONS" {
        let response = "HTTP/1.1 204 No Content\r\n\
Access-Control-Allow-Origin: *\r\n\
Access-Control-Allow-Methods: GET, HEAD, OPTIONS\r\n\
Access-Control-Allow-Headers: Range, Content-Type, Accept\r\n\
Access-Control-Max-Age: 86400\r\n\r\n";
        let _ = stream.write_all(response.as_bytes());
        return;
    }

    if method != "GET" && method != "HEAD" {
        let _ = stream.write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n");
        return;
    }

    // Read remaining headers to find Range
    let mut range_header: Option<String> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
            break;
        }
        if line.to_lowercase().starts_with("range:") {
            range_header = Some(line.trim()["range:".len()..].trim().to_string());
        }
    }

    // Path pattern: /stream/:info_hash or /stream/:info_hash/:file_index or /stream/:info_hash/files
    let clean_path = path.split('?').next().unwrap_or(path);
    let segments: Vec<&str> = clean_path.trim_start_matches('/').split('/').collect();
    if segments.is_empty() || segments[0] != "stream" || segments.len() < 2 {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        return;
    }

    let target_hash = segments[1];

    let (torrent_stream_res, file_name) = {
        let e = engine.lock().unwrap();
        let session_state = match e.sessions.iter().find(|s| info_hash_hex(&s.session).eq_ignore_ascii_case(target_hash)) {
            Some(s) => s,
            None => {
                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\n\r\nTorrent not found");
                return;
            }
        };

        if segments.len() >= 3 && segments[2] == "files" {
            if let Ok(ctx) = session_state.session.context().lock() {
                let files: Vec<FileInfo> = ctx.files_to_download.iter().enumerate().map(|(idx, f)| {
                    FileInfo {
                        index: idx,
                        name: f.torrent_path.clone(),
                        length: f.length,
                        is_media: is_media_filename(&f.torrent_path),
                        priority: f.priority.as_str().to_string(),
                    }
                }).collect();
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
                    let mut best_idx = None;
                    let mut max_len = 0;
                    for (i, f) in ctx.files_to_download.iter().enumerate() {
                        if is_media_filename(&f.torrent_path) && f.length > max_len {
                            max_len = f.length;
                            best_idx = Some(i);
                        }
                    }
                    if best_idx.is_none() {
                        for (i, f) in ctx.files_to_download.iter().enumerate() {
                            if f.length > max_len {
                                max_len = f.length;
                                best_idx = Some(i);
                            }
                        }
                    }
                    best_idx
                } else {
                    None
                }
            }
        };

        let file_name = if let Some(idx) = target_file_index {
            if let Ok(ctx) = session_state.session.context().lock() {
                ctx.files_to_download.get(idx).map(|f| f.torrent_path.clone()).unwrap_or_else(|| session_state.last_file_name.clone())
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
            let response = format!("HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\n\r\n{}", msg.len(), msg);
            let _ = stream.write_all(response.as_bytes());
            return;
        }
    };

    let total_len = torrent_stream.total_len();
    if total_len == 0 {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        return;
    }

    let mime_type = detect_mime_type(&file_name);

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
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\n\r\n",
            total_len
        );
        let _ = stream.write_all(response.as_bytes());
        return;
    }

    let content_len = range_end.saturating_sub(range_start) + 1;

    let header = if is_range_request {
        format!(
            "HTTP/1.1 206 Partial Content\r\n\
Content-Type: {}\r\n\
Content-Range: bytes {}-{}/{}\r\n\
Content-Length: {}\r\n\
Accept-Ranges: bytes\r\n\
Access-Control-Allow-Origin: *\r\n\
Access-Control-Expose-Headers: Content-Range, Accept-Ranges, Content-Length\r\n\
Connection: keep-alive\r\n\r\n",
            mime_type, range_start, range_end, total_len, content_len
        )
    } else {
        format!(
            "HTTP/1.1 200 OK\r\n\
Content-Type: {}\r\n\
Content-Length: {}\r\n\
Accept-Ranges: bytes\r\n\
Access-Control-Allow-Origin: *\r\n\
Access-Control-Expose-Headers: Content-Range, Accept-Ranges, Content-Length\r\n\
Connection: keep-alive\r\n\r\n",
            mime_type, total_len
        )
    };

    if stream.write_all(header.as_bytes()).is_err() {
        return;
    }

    if method == "HEAD" {
        return;
    }

    if torrent_stream.seek(SeekFrom::Start(range_start)).is_err() {
        return;
    }

    let mut remaining = content_len;
    let mut buf = [0u8; 65536];

    while remaining > 0 {
        let to_read = (buf.len() as u64).min(remaining) as usize;
        match torrent_stream.read(&mut buf[..to_read]) {
            Ok(0) => break,
            Ok(bytes_read) => {
                if stream.write_all(&buf[..bytes_read]).is_err() {
                    break;
                }
                remaining -= bytes_read as u64;
            }
            Err(_) => break,
        }
    }
}

// IPC Transport Implementations

#[cfg(unix)]
mod ipc {
    use std::os::unix::net::UnixListener;
    use std::io::{Write, BufRead, BufReader};

    pub struct IpcServer {
        socket_path: String,
    }

    impl IpcServer {
        pub fn new(name: &str) -> Self {
            Self { socket_path: format!("/tmp/{}.sock", name) }
        }

        pub fn listen<F>(&self, handler: F) -> std::io::Result<()>
        where
            F: Fn(String) -> String + Send + Sync + 'static
        {
            let _ = std::fs::remove_file(&self.socket_path);
            let listener = UnixListener::bind(&self.socket_path)?;
            let handler = std::sync::Arc::new(handler);

            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let handler_clone = handler.clone();
                        std::thread::spawn(move || {
                            // Clone the stream so the reader and writer operate on distinct
                            // owned handles, avoiding borrow checker errors (E0502).
                            let mut writer = match stream.try_clone() {
                                Ok(w) => w,
                                Err(_) => return,
                            };
                            let mut reader = BufReader::new(stream);
                            let mut line = String::new();
                            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                                let trimmed = line.trim();
                                if !trimmed.is_empty() {
                                    let reply = handler_clone(trimmed.to_string());
                                    let mut reply_bytes = reply.into_bytes();
                                    reply_bytes.push(b'\n');
                                    if writer.write_all(&reply_bytes).is_err() {
                                        break;
                                    }
                                    let _ = writer.flush();
                                }
                                line.clear();
                            }
                        });
                    }
                    Err(_) => {}
                }
            }
            Ok(())
        }
    }
}

#[cfg(windows)]
mod ipc {
    use std::ptr;
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, CloseHandle};
    use windows_sys::Win32::System::Pipes::{CreateNamedPipeW, ConnectNamedPipe, DisconnectNamedPipe, PIPE_TYPE_BYTE, PIPE_READMODE_BYTE, PIPE_WAIT};
    use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile, PIPE_ACCESS_DUPLEX};

    pub struct IpcServer {
        pipe_name: Vec<u16>,
    }

    impl IpcServer {
        pub fn new(name: &str) -> Self {
            let full_name = format!("\\\\.\\pipe\\{}", name);
            let mut wname: Vec<u16> = full_name.encode_utf16().collect();
            wname.push(0);
            Self { pipe_name: wname }
        }

        pub fn listen<F>(&self, handler: F) -> std::io::Result<()>
        where
            F: Fn(String) -> String + Send + Sync + 'static
        {
            let handler = std::sync::Arc::new(handler);
            loop {
                let pipe = unsafe {
                    CreateNamedPipeW(
                        self.pipe_name.as_ptr(),
                        PIPE_ACCESS_DUPLEX,
                        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                        255, // max instances
                        4096, // out buffer
                        4096, // in buffer
                        0, // default timeout
                        ptr::null(),
                    )
                };

                if pipe == INVALID_HANDLE_VALUE {
                    return Err(std::io::Error::last_os_error());
                }

                let connected = unsafe { ConnectNamedPipe(pipe, ptr::null_mut()) };
                let last_err = if connected == 0 {
                    unsafe { windows_sys::Win32::Foundation::GetLastError() }
                } else {
                    0
                };
                if connected != 0 || last_err == 997 || last_err == windows_sys::Win32::Foundation::ERROR_PIPE_CONNECTED {
                    let handler_clone = handler.clone();
                    let pipe_raw = pipe as usize;
                    std::thread::spawn(move || {
                        let h_pipe = pipe_raw as HANDLE;
                        let mut buf = [0u8; 4096];
                        let mut read_bytes = 0u32;
                        let mut request_str = String::new();
                        loop {
                            let ok = unsafe {
                                ReadFile(
                                    h_pipe,
                                    buf.as_mut_ptr() as *mut _,
                                    buf.len() as u32,
                                    &mut read_bytes,
                                    ptr::null_mut(),
                                )
                            };
                            if ok == 0 || read_bytes == 0 {
                                break;
                            }
                            if let Ok(s) = std::str::from_utf8(&buf[..read_bytes as usize]) {
                                request_str.push_str(s);
                                if request_str.contains('\n') {
                                    let mut lines: Vec<&str> = request_str.split('\n').collect();
                                    let remaining = lines.pop().unwrap_or("").to_string();
                                    for line in lines {
                                        let trimmed = line.trim();
                                        if !trimmed.is_empty() {
                                            let reply = handler_clone(trimmed.to_string());
                                            let mut reply_bytes = reply.into_bytes();
                                            reply_bytes.push(b'\n');
                                            let mut written = 0u32;
                                            unsafe {
                                                WriteFile(
                                                    h_pipe,
                                                    reply_bytes.as_ptr() as *const _,
                                                    reply_bytes.len() as u32,
                                                    &mut written,
                                                    ptr::null_mut(),
                                                );
                                            }
                                        }
                                    }
                                    request_str = remaining;
                                }
                            }
                        }
                        unsafe {
                            DisconnectNamedPipe(h_pipe);
                            CloseHandle(h_pipe);
                        }
                    });
                } else {
                    unsafe { CloseHandle(pipe); }
                }
            }
        }
    }
}

fn main() {
    let mut stream_port = 8081u16;
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        if (args[i] == "--stream-port" || args[i] == "-s") && i + 1 < args.len() {
            if let Ok(p) = args[i + 1].parse::<u16>() {
                stream_port = p;
            }
            i += 2;
        } else {
            i += 1;
        }
    }

    let mut state_dir = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    state_dir.push("BitTorrent-rs");
    let _ = std::fs::create_dir_all(&state_dir);
    let state_file = state_dir.join("daemon_state.json");

    let engine = Arc::new(Mutex::new(Engine::new(state_file, "downloads".to_string())));
    
    // Load existing state
    {
        let mut e = engine.lock().unwrap();
        e.load_state();
        println!("Daemon initialized. Loaded {} active sessions.", e.sessions.len());
    }

    // Launch HTTP Media Stream Server
    let engine_stream = engine.clone();
    start_stream_server(engine_stream, stream_port);


    // Set up shutdown channel
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel::<()>();

    // Ctrl+C Handler
    let shutdown_tx_ctrlc = shutdown_tx.clone();
    ctrlc::set_handler(move || {
        println!("\nShutdown signal received (Ctrl+C)...");
        let _ = shutdown_tx_ctrlc.send(());
    }).expect("Error setting Ctrl-C handler");

    // Launch IPC Server
    let engine_ipc = engine.clone();
    let shutdown_tx_ipc = shutdown_tx.clone();
    std::thread::spawn(move || {
        let server = ipc::IpcServer::new("bt-daemon");
        println!("IPC server listening...");
        let _ = server.listen(move |req_str| {
            let req: IpcMessage = match serde_json::from_str(&req_str) {
                Ok(m) => m,
                Err(e) => return serde_json::to_string(&IpcReply::Error { reason: format!("JSON Parse Error: {}", e) }).unwrap(),
            };

            let reply = match req {
                IpcMessage::Add { torrent_path, download_dir } => {
                    let mut e = engine_ipc.lock().unwrap();
                    if let Some(dir) = download_dir {
                        e.download_dir = dir;
                    }
                    match e.add_session(torrent_path, None, None) {
                        Ok(_) => IpcReply::Success { message: "Torrent added successfully".to_string() },
                        Err(err) => IpcReply::Error { reason: err },
                    }
                }
                IpcMessage::Status => {
                    let mut e = engine_ipc.lock().unwrap();
                    IpcReply::StatusList { torrents: e.get_status_info() }
                }
                IpcMessage::Pause { info_hash } => {
                    let mut e = engine_ipc.lock().unwrap();
                    match e.pause_session(&info_hash) {
                        Ok(_) => IpcReply::Success { message: "Torrent paused".to_string() },
                        Err(err) => IpcReply::Error { reason: err },
                    }
                }
                IpcMessage::Resume { info_hash } => {
                    let mut e = engine_ipc.lock().unwrap();
                    match e.resume_session(&info_hash) {
                        Ok(_) => IpcReply::Success { message: "Torrent resumed".to_string() },
                        Err(err) => IpcReply::Error { reason: err },
                    }
                }
                IpcMessage::Remove { info_hash, delete_data } => {
                    let mut e = engine_ipc.lock().unwrap();
                    match e.remove_session(&info_hash, delete_data) {
                        Ok(_) => IpcReply::Success { message: "Torrent removed".to_string() },
                        Err(err) => IpcReply::Error { reason: err },
                    }
                }
                IpcMessage::Shutdown => {
                    let _ = shutdown_tx_ipc.send(());
                    IpcReply::Success { message: "Daemon shutting down...".to_string() }
                }
                IpcMessage::Files { info_hash } => {
                    let e = engine_ipc.lock().unwrap();
                    let files = if let Some(s) = e.sessions.iter().find(|s| info_hash_hex(&s.session).eq_ignore_ascii_case(&info_hash)) {
                        if let Ok(ctx) = s.session.context().lock() {
                            ctx.files_to_download.iter().enumerate().map(|(idx, f)| {
                                FileInfo {
                                    index: idx,
                                    name: f.torrent_path.clone(),
                                    length: f.length,
                                    is_media: is_media_filename(&f.torrent_path),
                                    priority: f.priority.as_str().to_string(),
                                }
                            }).collect()
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    };
                    IpcReply::FileList { files }
                }
                IpcMessage::Peers { info_hash } => {
                    let e = engine_ipc.lock().unwrap();
                    let peers = if let Some(s) = e.sessions.iter().find(|s| info_hash_hex(&s.session).eq_ignore_ascii_case(&info_hash)) {
                        if let Ok(ctx) = s.session.context().lock() {
                            extract_peer_infos(&ctx)
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    };
                    IpcReply::PeerList { peers }
                }
                IpcMessage::Pieces { info_hash } => {
                    let e = engine_ipc.lock().unwrap();
                    if let Some(s) = e.sessions.iter().find(|s| info_hash_hex(&s.session).eq_ignore_ascii_case(&info_hash)) {
                        if let Ok(ctx) = s.session.context().lock() {
                            let info = extract_piece_availability(&ctx);
                            IpcReply::PieceStates { info }
                        } else {
                            IpcReply::Error { reason: "Unable to lock session context".to_string() }
                        }
                    } else {
                        IpcReply::Error { reason: "Torrent not found".to_string() }
                    }
                }
                IpcMessage::SetFilePriority { info_hash, file_index, priority } => {
                    let mut e = engine_ipc.lock().unwrap();
                    if let Some(s) = e.sessions.iter_mut().find(|s| info_hash_hex(&s.session).eq_ignore_ascii_case(&info_hash)) {
                        let prio = bittorrent_rs::FilePriority::from_str_loose(&priority);
                        match s.session.set_file_priority(file_index, prio) {
                            Ok(()) => IpcReply::Success { message: format!("Set file {} priority to {}", file_index, prio.as_str()) },
                            Err(err) => IpcReply::Error { reason: err.to_string() },
                        }
                    } else {
                        IpcReply::Error { reason: "Torrent not found".to_string() }
                    }
                }
                IpcMessage::SetSuperSeeding { info_hash, enabled } => {
                    let mut e = engine_ipc.lock().unwrap();
                    if let Some(s) = e.sessions.iter_mut().find(|s| info_hash_hex(&s.session).eq_ignore_ascii_case(&info_hash)) {
                        s.session.set_super_seeding(enabled);
                        IpcReply::Success { message: format!("Super-seeding set to {} for {}", enabled, info_hash) }
                    } else {
                        IpcReply::Error { reason: "Torrent not found".to_string() }
                    }
                }
            };

            serde_json::to_string(&reply).unwrap_or_else(|_| "{\"type\":\"Error\",\"data\":{\"reason\":\"Serialization error\"}}".to_string())
        });
    });

    // Wait for shutdown signal
    let _ = shutdown_rx.recv();
    println!("Saving state and stopping all torrent sessions... This may take a moment.");
    let mut e = engine.lock().unwrap();
    e.save_state();
    e.stop_all();
    println!("Daemon terminated cleanly.");
}
