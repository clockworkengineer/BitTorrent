use bittorrent_rs::{TorrentSession, TorrentContext};
use std::sync::mpsc;

pub struct SessionState {
    pub session: TorrentSession,
    pub torrent_path: String,
    pub last_file_name: String,
    pub last_progress: f32,
    pub last_status: String,
    pub last_peers_connected: usize,
    pub last_peers_active: usize,
    pub last_bps: u64,
    pub last_downloaded: u64,
    pub last_total: u64,
    pub last_uploaded: u64,
}

impl SessionState {
    pub fn new(session: TorrentSession, torrent_path: String) -> Self {
        let mut state = Self {
            session,
            torrent_path,
            last_file_name: String::new(),
            last_progress: 0.0,
            last_status: String::new(),
            last_peers_connected: 0,
            last_peers_active: 0,
            last_bps: 0,
            last_downloaded: 0,
            last_total: 0,
            last_uploaded: 0,
        };
        if let Ok(ctx_guard) = state.session.context().lock() {
            state.update_fields(&ctx_guard);
        }
        state
    }

    /// Synchronizes the local session state fields with the underlying TorrentContext.
    pub fn update_fields(&mut self, ctx_guard: &TorrentContext) {
        self.last_file_name = std::path::Path::new(&ctx_guard.file_name)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| ctx_guard.file_name.clone());
        self.last_progress = ctx_guard.progress_percent() / 100.0;
        self.last_status = format!("{:?}", ctx_guard.status);
        self.last_peers_connected = ctx_guard.peer_swarm.read().unwrap().len();
        self.last_peers_active = ctx_guard.number_of_unchoked_peers();
        self.last_bps = ctx_guard.bytes_per_second() as u64;
        self.last_downloaded = ctx_guard.total_bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed);
        self.last_total = ctx_guard.total_bytes_to_download;
        self.last_uploaded = ctx_guard.total_bytes_uploaded.load(std::sync::atomic::Ordering::Relaxed);
    }
}

pub struct PendingSession {
    pub torrent_path: String,
    pub rx: mpsc::Receiver<TorrentSession>,
}

pub fn fmt_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.2} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TorrentStatusInfo {
    pub name: String,
    pub info_hash: String,
    pub progress: f32,
    pub status: String,
    pub peers_connected: usize,
    pub peers_active: usize,
    pub download_rate: u64,
    pub upload_rate: u64,
    pub downloaded: u64,
    pub uploaded: u64,
    pub total_size: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", content = "data")]
pub enum IpcMessage {
    Add {
        torrent_path: String,
        download_dir: Option<String>,
    },
    Status,
    Pause {
        info_hash: String,
    },
    Resume {
        info_hash: String,
    },
    Remove {
        info_hash: String,
        delete_data: bool,
    },
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", content = "data")]
pub enum IpcReply {
    Success {
        message: String,
    },
    StatusList {
        torrents: Vec<TorrentStatusInfo>,
    },
    Error {
        reason: String,
    },
}
