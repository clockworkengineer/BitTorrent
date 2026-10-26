use bittorrent_rs::TorrentSession;
use bittorrent_rs::internals::TorrentContext;
use std::sync::mpsc;

pub mod ipc;
pub use ipc::IpcClient;

pub mod media;
pub use media::{detect_mime_type, find_primary_media_file, is_media_filename};

pub mod stream;
pub use stream::{
    ByteRange, HttpRequest, parse_range_spec, read_http_request, send_cors_preflight_response,
    send_http_error, serve_http_range_stream,
};

pub struct SessionState {
    pub session: TorrentSession,
    pub torrent_path: String,
    pub last_file_name: String,
    pub last_progress: f32,
    pub last_status: String,
    pub last_peers_connected: usize,
    pub last_peers_active: usize,
    pub last_bps: u64,
    pub last_upload_bps: u64,
    pub last_downloaded: u64,
    pub last_total: u64,
    pub last_uploaded: u64,
    pub last_peer_seen_time: Option<std::time::Instant>,
    pub created_at: std::time::Instant,
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
            last_upload_bps: 0,
            last_downloaded: 0,
            last_total: 0,
            last_uploaded: 0,
            last_peer_seen_time: None,
            created_at: std::time::Instant::now(),
        };
        if let Ok(ctx_guard) = state.session.context().lock() {
            state.update_fields(&ctx_guard);
        }
        state
    }

    /// Synchronizes the local session state fields with the underlying TorrentContext.
    pub fn update_fields(&mut self, ctx_guard: &TorrentContext) {
        let mut meaningful_name = String::new();
        let raw = ctx_guard.file_name.trim();
        let is_raw_hex = (raw.len() == 40 || (raw.len() == 48 && raw.ends_with(".torrent")))
            && raw.chars().take(40).all(|c| c.is_ascii_hexdigit());

        if !raw.is_empty() && !is_raw_hex && !raw.starts_with("Magnet (") {
            let candidate = std::path::Path::new(raw)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| raw.to_string());
            meaningful_name = candidate;
        }

        if meaningful_name.is_empty() && self.torrent_path.starts_with("magnet:?") {
            if let Ok(mag) = bittorrent_rs::MagnetLink::parse(&self.torrent_path) {
                if let Some(ref dn) = mag.display_name {
                    if !dn.trim().is_empty() {
                        meaningful_name = dn.clone();
                    }
                }
            }
        }

        if meaningful_name.is_empty() && !self.torrent_path.starts_with("magnet:?") {
            let path = std::path::Path::new(&self.torrent_path);
            if let Some(stem) = path.file_stem() {
                let s = stem.to_string_lossy().to_string();
                let is_stem_hex = s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit());
                if !is_stem_hex && !s.is_empty() {
                    meaningful_name = s;
                }
            }
        }

        if meaningful_name.is_empty() && ctx_guard.files_to_download.len() == 1 {
            meaningful_name = ctx_guard.files_to_download[0].torrent_path.clone();
        }

        if meaningful_name.is_empty() {
            if !raw.is_empty() {
                meaningful_name = raw.to_string();
            } else {
                let hex = bittorrent_rs::util::info_hash_to_string(&ctx_guard.info_hash);
                let short = if hex.len() >= 8 { &hex[..8] } else { &hex };
                meaningful_name = format!("Magnet ({})", short);
            }
        }

        if meaningful_name.ends_with(".torrent") && meaningful_name.len() > 8 {
            meaningful_name = meaningful_name.strip_suffix(".torrent").unwrap().to_string();
        }

        if self.last_file_name != meaningful_name {
            self.last_file_name = meaningful_name;
        }
        self.last_progress = ctx_guard.progress_percent() / 100.0;
        let status_str = match ctx_guard.status {
            bittorrent_rs::TorrentStatus::Initialised => "Initialised",
            bittorrent_rs::TorrentStatus::Downloading => {
                if ctx_guard.pieces_info_hash.is_empty() {
                    "Fetching Metadata"
                } else {
                    "Downloading"
                }
            }
            bittorrent_rs::TorrentStatus::Seeding => "Seeding",
            bittorrent_rs::TorrentStatus::Paused => "Paused",
            bittorrent_rs::TorrentStatus::Ended => "Ended",
        };
        if self.last_status != status_str {
            self.last_status.clear();
            self.last_status.push_str(status_str);
        }

        let current_peers = ctx_guard.peer_swarm.read().unwrap().len();
        if current_peers > 0 {
            self.last_peers_connected = current_peers;
            self.last_peer_seen_time = Some(std::time::Instant::now());
        } else if let Some(last_seen) = self.last_peer_seen_time {
            // Retain the last known positive peer count for a 2.5-second grace period
            // to prevent flickering to 0 during momentary reconnect/handshake transitions
            if last_seen.elapsed() > std::time::Duration::from_millis(2500) {
                self.last_peers_connected = 0;
            }
        } else {
            self.last_peers_connected = 0;
        }

        let active_peers = ctx_guard.number_of_unchoked_peers();
        if active_peers > 0 || self.last_peers_connected == 0 {
            self.last_peers_active = active_peers;
        }

        self.last_bps = ctx_guard.bytes_per_second() as u64;
        self.last_upload_bps = ctx_guard.upload_bytes_per_second() as u64;
        self.last_downloaded = ctx_guard
            .total_bytes_downloaded
            .load(std::sync::atomic::Ordering::Relaxed);
        self.last_total = ctx_guard.total_bytes_to_download;
        self.last_uploaded = ctx_guard
            .total_bytes_uploaded
            .load(std::sync::atomic::Ordering::Relaxed);
    }
}

pub struct PendingSession {
    pub torrent_path: String,
    pub rx: mpsc::Receiver<Result<TorrentSession, String>>,
    pub error: Option<String>,
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

use serde::{Deserialize, Serialize};

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
    pub download_dir: String,
    pub total_size: u64,
    #[serde(default)]
    pub is_super_seeding: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileInfo {
    pub index: usize,
    pub name: String,
    pub length: u64,
    pub is_media: bool,
    #[serde(default = "default_priority")]
    pub priority: String,
}

fn default_priority() -> String {
    "normal".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PeerInfo {
    pub ip: String,
    pub port: u16,
    pub client: String,
    pub progress: f32,
    pub down_rate: u64,
    pub up_rate: u64,
    pub flags: String,
    pub connection_type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PieceAvailabilityInfo {
    pub total_pieces: usize,
    pub have_pieces: usize,
    pub piece_length: u32,
    pub local_have: Vec<bool>,
    pub swarm_rarity: Vec<u16>,
}

/// Extracts structured peer information from an active session's TorrentContext.
pub fn extract_peer_infos(ctx: &bittorrent_rs::internals::TorrentContext) -> Vec<PeerInfo> {
    let mut result = Vec::new();
    let total_pieces = ctx.number_of_pieces;
    if let Ok(swarm) = ctx.peer_swarm.read() {
        for peer_arc in swarm.values() {
            if let Ok(peer) = peer_arc.try_lock() {
                let client = peer
                    .remote_peer_id
                    .as_deref()
                    .map(bittorrent_rs::peer_id::parse_peer_client)
                    .unwrap_or_else(|| "Unknown".to_string());

                let pieces_have: usize = peer
                    .remote_piece_bitfield
                    .iter()
                    .map(|b| b.count_ones() as usize)
                    .sum();
                let progress = if total_pieces > 0 {
                    (pieces_have.min(total_pieces) as f32) / (total_pieces as f32)
                } else {
                    0.0
                };

                let mut flags = String::new();
                if peer.am_interested {
                    flags.push('I');
                } else {
                    flags.push('i');
                }
                if peer.am_choking {
                    flags.push('C');
                } else {
                    flags.push('c');
                }
                if peer.peer_interested {
                    flags.push('H');
                } else {
                    flags.push('h');
                }
                if peer.peer_choking.wait_one(0) {
                    flags.push('U');
                } else {
                    flags.push('u');
                }
                if peer.supports_extensions {
                    flags.push('E');
                }
                if peer.supports_fast_extension {
                    flags.push('F');
                }

                result.push(PeerInfo {
                    ip: peer.ip.clone(),
                    port: peer.port,
                    client,
                    progress,
                    down_rate: peer.rolling_download_rate as u64,
                    up_rate: peer.rolling_upload_rate as u64,
                    flags,
                    connection_type: peer.connection_type.to_string(),
                });
            }
        }
    }
    result.sort_by(|a, b| b.down_rate.cmp(&a.down_rate).then_with(|| a.ip.cmp(&b.ip)));
    result
}

/// Extracts piece verification status and swarm rarity from an active session's TorrentContext.
pub fn extract_piece_availability(
    ctx: &bittorrent_rs::internals::TorrentContext,
) -> PieceAvailabilityInfo {
    let (local_have, swarm_rarity) = ctx.get_piece_availability();
    let have_pieces = local_have.iter().filter(|&&h| h).count();
    PieceAvailabilityInfo {
        total_pieces: ctx.number_of_pieces,
        have_pieces,
        piece_length: ctx.piece_length,
        local_have,
        swarm_rarity,
    }
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
    Files {
        info_hash: String,
    },
    Peers {
        info_hash: String,
    },
    Pieces {
        info_hash: String,
    },
    SetFilePriority {
        info_hash: String,
        file_index: usize,
        priority: String,
    },
    SetSuperSeeding {
        info_hash: String,
        enabled: bool,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", content = "data")]
pub enum IpcReply {
    Success { message: String },
    StatusList { torrents: Vec<TorrentStatusInfo> },
    FileList { files: Vec<FileInfo> },
    PeerList { peers: Vec<PeerInfo> },
    PieceStates { info: PieceAvailabilityInfo },
    Error { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipc_peers_and_pieces_serialization_roundtrip() {
        let msg_peers = IpcMessage::Peers {
            info_hash: "1234567890abcdef1234".to_string(),
        };
        let json_peers = serde_json::to_string(&msg_peers).unwrap();
        let decoded_peers: IpcMessage = serde_json::from_str(&json_peers).unwrap();
        match decoded_peers {
            IpcMessage::Peers { info_hash } => assert_eq!(info_hash, "1234567890abcdef1234"),
            _ => panic!("Expected IpcMessage::Peers"),
        }

        let msg_pieces = IpcMessage::Pieces {
            info_hash: "fedcba0987654321fedc".to_string(),
        };
        let json_pieces = serde_json::to_string(&msg_pieces).unwrap();
        let decoded_pieces: IpcMessage = serde_json::from_str(&json_pieces).unwrap();
        match decoded_pieces {
            IpcMessage::Pieces { info_hash } => assert_eq!(info_hash, "fedcba0987654321fedc"),
            _ => panic!("Expected IpcMessage::Pieces"),
        }

        let msg_ss = IpcMessage::SetSuperSeeding {
            info_hash: "abcdef1234567890abcdef12".to_string(),
            enabled: true,
        };
        let json_ss = serde_json::to_string(&msg_ss).unwrap();
        let decoded_ss: IpcMessage = serde_json::from_str(&json_ss).unwrap();
        match decoded_ss {
            IpcMessage::SetSuperSeeding { info_hash, enabled } => {
                assert_eq!(info_hash, "abcdef1234567890abcdef12");
                assert_eq!(enabled, true);
            }
            _ => panic!("Expected IpcMessage::SetSuperSeeding"),
        }
    }

    #[test]
    fn test_ipc_reply_peer_list_and_piece_states_roundtrip() {
        let peer = PeerInfo {
            ip: "127.0.0.1".to_string(),
            port: 6881,
            client: "qBittorrent 4.6.0".to_string(),
            progress: 0.75,
            down_rate: 1048576,
            up_rate: 524288,
            flags: "dUE".to_string(),
            connection_type: "TCP".to_string(),
        };

        let reply_peers = IpcReply::PeerList {
            peers: vec![peer.clone()],
        };
        let json_peers = serde_json::to_string(&reply_peers).unwrap();
        let decoded_reply: IpcReply = serde_json::from_str(&json_peers).unwrap();
        match decoded_reply {
            IpcReply::PeerList { peers } => {
                assert_eq!(peers.len(), 1);
                assert_eq!(peers[0].client, "qBittorrent 4.6.0");
                assert_eq!(peers[0].down_rate, 1048576);
                assert_eq!(peers[0].flags, "dUE");
            }
            _ => panic!("Expected IpcReply::PeerList"),
        }

        let pieces_info = PieceAvailabilityInfo {
            total_pieces: 8,
            have_pieces: 6,
            piece_length: 16384,
            local_have: vec![true, true, true, false, true, true, true, false],
            swarm_rarity: vec![2, 3, 1, 0, 4, 2, 1, 0],
        };

        let reply_pieces = IpcReply::PieceStates { info: pieces_info };
        let json_pieces = serde_json::to_string(&reply_pieces).unwrap();
        let decoded_pieces_reply: IpcReply = serde_json::from_str(&json_pieces).unwrap();
        match decoded_pieces_reply {
            IpcReply::PieceStates { info } => {
                assert_eq!(info.total_pieces, 8);
                assert_eq!(info.have_pieces, 6);
                assert_eq!(info.local_have[0], true);
                assert_eq!(info.local_have[3], false);
                assert_eq!(info.swarm_rarity[1], 3);
            }
            _ => panic!("Expected IpcReply::PieceStates"),
        }
    }
}
