//! Central daemon engine orchestrating session management and persistence (SRP).
//!
//! Bridges `TorrentSessionPool` and `DaemonStateRepository` without entangling
//! HTTP streaming servers or IPC transports.

use std::path::PathBuf;
use std::sync::Arc;

use bittorrent_rs::TorrentSession;
use torrent_client_shared::{SessionState, TorrentStatusInfo};

use crate::repository::{DaemonState, DaemonStateRepository, SavedTorrent};
use crate::session_pool::TorrentSessionPool;

/// Core coordinator managing torrent sessions and persisting daemon state.
pub struct Engine {
    pub download_dir: String,
    pub pool: TorrentSessionPool,
    pub repository: DaemonStateRepository,
}

impl Engine {
    /// Creates a new `Engine` with the given repository and default download directory.
    pub fn new(repository: DaemonStateRepository, download_dir: String) -> Self {
        Self {
            download_dir,
            pool: TorrentSessionPool::new(),
            repository,
        }
    }

    /// Serializes active torrents and persists state and fastresume data to disk.
    pub fn save_state(&self) {
        let torrents = self
            .pool
            .sessions()
            .iter()
            .map(|s| {
                let (bitfield_hex, hash_hex) = if let Ok(ctx) = s.session.context().lock() {
                    let bf = bittorrent_rs::util::to_hex(&ctx.bitfield);
                    let ih = bittorrent_rs::util::info_hash_to_string(&ctx.info_hash);
                    (bf, ih)
                } else {
                    (String::new(), String::new())
                };

                let fast_resume_file = if !hash_hex.is_empty() {
                    let resume_path = self.repository.fast_resume_path_for_hash(&hash_hex);
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
            })
            .collect::<Vec<_>>();

        let state = DaemonState {
            download_dir: self.download_dir.clone(),
            torrents,
        };

        let _ = self.repository.save(&state);
    }

    /// Loads persisted torrent states from disk and restarts sessions.
    pub fn load_state(&mut self) {
        if let Ok(state) = self.repository.load() {
            if !state.download_dir.is_empty() {
                self.download_dir = state.download_dir;
            }
            for t in state.torrents {
                let _ = self.add_session(t.torrent_path, Some(t.bitfield_hex), t.fast_resume_file);
            }
        }
    }

    /// Initializes a new torrent session from a file path or magnet URI, starting download.
    pub fn add_session(
        &mut self,
        torrent_path: String,
        bitfield_hex: Option<String>,
        fast_resume_file: Option<String>,
    ) -> Result<(), String> {
        let torrent_path_buf = PathBuf::from(&torrent_path);
        let download_dir_buf = PathBuf::from(&self.download_dir);

        if self.pool.contains_path(&torrent_path) {
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
        if let Some(ref hex_str) = bitfield_hex
            && let Ok(mut ctx) = session.context.lock()
        {
            if let Ok(bytes) = bittorrent_rs::util::from_hex(hex_str)
                && bytes.len() == ctx.bitfield.len()
            {
                ctx.bitfield = bytes.clone();
                let mut downloaded = 0u64;
                let mut missing_count = 0;
                let number_of_pieces = ctx.number_of_pieces;
                for piece_num in 0..number_of_pieces {
                    let (byte_idx, mask) =
                        bittorrent_rs::util::get_bitfield_index_and_mask(piece_num as u32);
                    let local = (bytes[byte_idx] & mask) != 0;
                    ctx.mark_piece_missing(piece_num as u32, !local);
                    if local {
                        downloaded += ctx.get_piece_length(piece_num as u32) as u64;
                    } else {
                        missing_count += 1;
                    }
                }
                ctx.total_bytes_downloaded
                    .store(downloaded, std::sync::atomic::Ordering::Relaxed);
                ctx.missing_pieces_count = missing_count;
                ctx.initial_bytes_downloaded = downloaded;
            }
        }

        if let Err(e) = session.start_download() {
            return Err(format!("Failed to start download: {}", e));
        }

        let state = SessionState::new(session, torrent_path);
        self.pool.add(state);
        self.save_state();
        Ok(())
    }

    /// Pauses an active session by info-hash.
    pub fn pause_session(&mut self, target_hash: &str) -> Result<(), String> {
        self.pool.pause(target_hash)?;
        self.save_state();
        Ok(())
    }

    /// Resumes a paused session by info-hash.
    pub fn resume_session(&mut self, target_hash: &str) -> Result<(), String> {
        self.pool.resume(target_hash)?;
        self.save_state();
        Ok(())
    }

    /// Removes a session from the pool by info-hash, optionally deleting files.
    pub fn remove_session(&mut self, target_hash: &str, delete_data: bool) -> Result<(), String> {
        self.pool.remove(target_hash, delete_data)?;
        self.save_state();
        Ok(())
    }

    /// Retrieves status snapshots for all sessions in the pool.
    pub fn get_status_info(&mut self) -> Vec<TorrentStatusInfo> {
        self.pool.get_status_info()
    }

    /// Stops and terminates all sessions in the pool.
    pub fn stop_all(&mut self) {
        self.pool.stop_all();
    }
}
