//! Torrent session pool management (Single Responsibility Principle).
//!
//! Manages active in-memory `TorrentSession` handles, lookup by info-hash or path,
//! status metrics compilation, and cooperative session shutdown.

use bittorrent_rs::TorrentSession;
use std::path::Path;
use torrent_client_shared::{SessionState, TorrentStatusInfo};

/// Computes the 40-character hex string representation of a session's info hash.
pub fn info_hash_hex(session: &TorrentSession) -> String {
    if let Ok(ctx) = session.context().lock() {
        bittorrent_rs::util::info_hash_to_string(&ctx.info_hash)
    } else {
        String::new()
    }
}

/// In-memory collection and lifecycle manager for active torrent sessions (SRP).
pub struct TorrentSessionPool {
    sessions: Vec<SessionState>,
}

impl Default for TorrentSessionPool {
    fn default() -> Self {
        Self::new()
    }
}

impl TorrentSessionPool {
    /// Creates an empty `TorrentSessionPool`.
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
        }
    }

    /// Number of active sessions managed in the pool.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Returns true if no sessions exist in the pool.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Read-only slice access to managed session states.
    pub fn sessions(&self) -> &[SessionState] {
        &self.sessions
    }

    /// Mutable slice access to managed session states.
    pub fn sessions_mut(&mut self) -> &mut [SessionState] {
        &mut self.sessions
    }

    /// Checks if a session with the exact torrent path is already present.
    pub fn contains_path(&self, torrent_path: &str) -> bool {
        self.sessions.iter().any(|s| s.torrent_path == torrent_path)
    }

    /// Inserts a new active session into the pool.
    pub fn add(&mut self, state: SessionState) {
        self.sessions.push(state);
    }

    /// Looks up a session by its 40-character hex info hash (case-insensitive).
    pub fn find_by_hash(&self, target_hash: &str) -> Option<&SessionState> {
        self.sessions
            .iter()
            .find(|s| info_hash_hex(&s.session).eq_ignore_ascii_case(target_hash))
    }

    /// Looks up a mutable session by its 40-character hex info hash (case-insensitive).
    pub fn find_by_hash_mut(&mut self, target_hash: &str) -> Option<&mut SessionState> {
        self.sessions
            .iter_mut()
            .find(|s| info_hash_hex(&s.session).eq_ignore_ascii_case(target_hash))
    }

    /// Pauses an active torrent session by info hash.
    pub fn pause(&mut self, target_hash: &str) -> Result<(), String> {
        if let Some(s) = self.find_by_hash_mut(target_hash) {
            s.session.pause().map_err(|e| e.to_string())
        } else {
            Err("Torrent not found".to_string())
        }
    }

    /// Resumes a paused torrent session by info hash.
    pub fn resume(&mut self, target_hash: &str) -> Result<(), String> {
        if let Some(s) = self.find_by_hash_mut(target_hash) {
            s.session.resume().map_err(|e| e.to_string())
        } else {
            Err("Torrent not found".to_string())
        }
    }

    /// Removes a session from the pool by info hash, stops it, and optionally removes downloaded files.
    pub fn remove(&mut self, target_hash: &str, delete_data: bool) -> Result<(), String> {
        if let Some(pos) = self
            .sessions
            .iter()
            .position(|s| info_hash_hex(&s.session).eq_ignore_ascii_case(target_hash))
        {
            let mut state = self.sessions.remove(pos);
            let _ = state.session.stop();
            if delete_data && let Ok(ctx) = state.session.context().lock() {
                for file in &ctx.files_to_download {
                    let path = Path::new(&file.name);
                    if path.exists() {
                        let _ = std::fs::remove_file(path);
                    }
                }
            }
            std::thread::spawn(move || {
                state.session.join_peer_workers();
            });
            Ok(())
        } else {
            Err("Torrent not found".to_string())
        }
    }

    /// Polls live progress and statistics for all managed torrent sessions.
    pub fn get_status_info(&mut self) -> Vec<TorrentStatusInfo> {
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

    /// Gracefully stops and joins all active torrent sessions in the pool.
    pub fn stop_all(&mut self) {
        for s in &mut self.sessions {
            let _ = s.session.stop();
        }
        for s in &mut self.sessions {
            s.session.join_peer_workers();
        }
    }
}
