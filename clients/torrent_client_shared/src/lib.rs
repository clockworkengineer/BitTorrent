use bittorrent_rs::TorrentSession;
use bittorrent_rs::internals::TorrentContext;
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
    pub last_upload_bps: u64,
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
            last_upload_bps: 0,
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
        self.last_upload_bps = ctx_guard.upload_bytes_per_second() as u64;
        self.last_downloaded = ctx_guard.total_bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed);
        self.last_total = ctx_guard.total_bytes_to_download;
        self.last_uploaded = ctx_guard.total_bytes_uploaded.load(std::sync::atomic::Ordering::Relaxed);
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
    pub download_dir: String,
    pub total_size: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileInfo {
    pub index: usize,
    pub name: String,
    pub length: u64,
    pub is_media: bool,
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
                if peer.am_interested { flags.push('I'); } else { flags.push('i'); }
                if peer.am_choking { flags.push('C'); } else { flags.push('c'); }
                if peer.peer_interested { flags.push('H'); } else { flags.push('h'); }
                if peer.peer_choking.wait_one(0) { flags.push('U'); } else { flags.push('u'); }
                if peer.supports_extensions { flags.push('E'); }
                if peer.supports_fast_extension { flags.push('F'); }

                result.push(PeerInfo {
                    ip: peer.ip.clone(),
                    port: peer.port,
                    client,
                    progress,
                    down_rate: peer.rolling_download_rate as u64,
                    up_rate: peer.rolling_upload_rate as u64,
                    flags,
                    connection_type: "TCP".to_string(),
                });
            }
        }
    }
    result.sort_by(|a, b| b.down_rate.cmp(&a.down_rate).then_with(|| a.ip.cmp(&b.ip)));
    result
}

/// Extracts piece verification status and swarm rarity from an active session's TorrentContext.
pub fn extract_piece_availability(ctx: &bittorrent_rs::internals::TorrentContext) -> PieceAvailabilityInfo {
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
    FileList {
        files: Vec<FileInfo>,
    },
    PeerList {
        peers: Vec<PeerInfo>,
    },
    PieceStates {
        info: PieceAvailabilityInfo,
    },
    Error {
        reason: String,
    },
}

/// Returns true if the file name has a common video or audio extension.
pub fn is_media_filename(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".mp4") || lower.ends_with(".mkv") || lower.ends_with(".webm") ||
    lower.ends_with(".avi") || lower.ends_with(".mov") || lower.ends_with(".m4v") ||
    lower.ends_with(".flv") || lower.ends_with(".wmv") || lower.ends_with(".mp3") ||
    lower.ends_with(".flac") || lower.ends_with(".ogg") || lower.ends_with(".wav") ||
    lower.ends_with(".aac") || lower.ends_with(".m4a")
}

/// Detects the MIME content type based on the file extension.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipc_peers_and_pieces_serialization_roundtrip() {
        let msg_peers = IpcMessage::Peers { info_hash: "1234567890abcdef1234".to_string() };
        let json_peers = serde_json::to_string(&msg_peers).unwrap();
        let decoded_peers: IpcMessage = serde_json::from_str(&json_peers).unwrap();
        match decoded_peers {
            IpcMessage::Peers { info_hash } => assert_eq!(info_hash, "1234567890abcdef1234"),
            _ => panic!("Expected IpcMessage::Peers"),
        }

        let msg_pieces = IpcMessage::Pieces { info_hash: "fedcba0987654321fedc".to_string() };
        let json_pieces = serde_json::to_string(&msg_pieces).unwrap();
        let decoded_pieces: IpcMessage = serde_json::from_str(&json_pieces).unwrap();
        match decoded_pieces {
            IpcMessage::Pieces { info_hash } => assert_eq!(info_hash, "fedcba0987654321fedc"),
            _ => panic!("Expected IpcMessage::Pieces"),
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

        let reply_peers = IpcReply::PeerList { peers: vec![peer.clone()] };
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


