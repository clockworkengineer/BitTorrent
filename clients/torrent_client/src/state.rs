//! Client State Management and Session Spawning
//!
//! Defines client-side state representations, settings storage structures,
//! sidebar status filtering, config loading/saving, and asynchronous torrent session creation.

use bittorrent_rs::{TorrentSession, Tracker};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::Duration;
use torrent_client_shared::{SessionState, PendingSession};

/// Sidebar filters for sorting active torrent sessions by status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SidebarFilter {
    /// Show all torrents.
    All,
    /// Show torrents actively downloading.
    Downloading,
    /// Show torrents seeding to the swarm.
    Seeding,
    /// Show paused torrents.
    Paused,
    /// Show completed torrents.
    Completed,
}

/// Active selected tabs in the details panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Overview,
    Files,
    Peers,
    Trackers,
    Logs,
}

/// Structure representing a saved torrent session details on disk.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct SavedTorrent {
    pub torrent_path: String,
    pub bitfield_hex: String,
}

/// Config schema representing serialized client application options.
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct ClientState {
    pub download_dir: String,
    pub torrents: Vec<SavedTorrent>,
}

/// Legacy client state layout for backward compatibility support.
#[derive(serde::Deserialize)]
pub struct LegacyClientState {
    pub download_dir: String,
    pub torrents: Vec<String>,
}

/// Determines the file path where configuration and state will be saved.
pub fn get_config_path() -> std::path::PathBuf {
    if let Some(mut proj_dirs) = dirs::data_local_dir() {
        proj_dirs.push("BitTorrent-rs");
        let _ = std::fs::create_dir_all(&proj_dirs);
        proj_dirs.push("client_state.json");
        proj_dirs
    } else {
        std::path::PathBuf::from("torrent_client_state.json")
    }
}

/// Returns true if a given session matches the selected status filter.
pub fn matches_filter(session: &SessionState, filter: SidebarFilter) -> bool {
    match filter {
        SidebarFilter::All => true,
        SidebarFilter::Downloading => session.last_status == "Downloading",
        SidebarFilter::Seeding => session.last_status == "Seeding",
        SidebarFilter::Paused => session.last_status == "Paused",
        SidebarFilter::Completed => session.last_progress >= 1.0 || session.last_status == "Seeding",
    }
}

use crate::app::TorrentClientApp;

impl TorrentClientApp {
    /// Saves the current client state (active download directory and torrent file paths with bitfields) to disk.
    pub fn save_state(&self) {
        let torrents = self.sessions.iter().map(|s| {
            let bitfield_hex = if let Ok(ctx) = s.session.context().lock() {
                ctx.bitfield.iter().map(|b| format!("{:02x}", b)).collect::<String>()
            } else {
                String::new()
            };
            SavedTorrent {
                torrent_path: s.torrent_path.clone(),
                bitfield_hex,
            }
        })
        .chain(self.pending_sessions.iter().map(|s| SavedTorrent {
            torrent_path: s.torrent_path.clone(),
            bitfield_hex: String::new(),
        }))
        .collect::<Vec<_>>();

        let state = ClientState {
            download_dir: self.download_dir.clone(),
            torrents,
        };
        let state_path = get_config_path();
        if let Ok(content) = serde_json::to_string_pretty(&state) {
            let _ = std::fs::write(state_path, content);
        }
    }

    /// Loads the client configuration and restores active torrent sessions from local storage.
    pub fn load_state(&mut self) {
        let state_path = get_config_path();
        let content = if state_path.exists() {
            std::fs::read_to_string(state_path).ok()
        } else {
            std::fs::read_to_string("torrent_client_state.txt").ok()
        };

        if let Some(content) = content {
            if let Ok(state) = serde_json::from_str::<ClientState>(&content) {
                self.download_dir = state.download_dir;
                for t in state.torrents {
                    self.add_session_by_path(t.torrent_path, self.download_dir.clone(), Some(t.bitfield_hex));
                }
            } else if let Ok(legacy) = serde_json::from_str::<LegacyClientState>(&content) {
                self.download_dir = legacy.download_dir;
                for path in legacy.torrents {
                    self.add_session_by_path(path, self.download_dir.clone(), None);
                }
            } else {
                let mut lines = content.lines();
                if let Some(dir) = lines.next() {
                    self.download_dir = dir.to_string();
                }
                for line in lines {
                    let path = line.trim().to_string();
                    if !path.is_empty() {
                        self.add_session_by_path(path, self.download_dir.clone(), None);
                    }
                }
            }
        }
    }

    /// Creates and launches a new torrent download session from the path and destination directory provided in the UI inputs.
    pub fn create_session(&mut self) {
        let torrent_path = self.torrent_path.trim().to_string();
        let download_dir = self.download_dir.trim().to_string();
        if torrent_path.is_empty() || download_dir.is_empty() {
            self.messages
                .push("Please provide both torrent file and download directory.".into());
            return;
        }

        if self.sessions.iter().any(|s| s.torrent_path == torrent_path)
            || self.pending_sessions.iter().any(|s| s.torrent_path == torrent_path)
        {
            self.messages.push(format!("Torrent is already added: {}", torrent_path));
            return;
        }

        self.add_session_by_path(torrent_path, download_dir, None);
    }

    /// Spawns a background thread to asynchronously resolve metadata, verify disk space, and establish tracker connections for a torrent session.
    pub fn add_session_by_path(&mut self, torrent_path: String, download_dir: String, bitfield_hex: Option<String>) {
        let (session_tx, session_rx) = mpsc::channel::<TorrentSession>();
        let msg_tx = self.log_tx.clone();
        self.pending_sessions.push(PendingSession {
            torrent_path: torrent_path.clone(),
            rx: session_rx,
        });
        self.save_state();

        let bitfield_hex_clone = bitfield_hex.clone();
        std::thread::spawn(move || {
            let torrent_path_buf = PathBuf::from(&torrent_path);
            let download_dir_buf = PathBuf::from(&download_dir);
            let session_id = if torrent_path.starts_with("magnet:?") {
                if let Ok(mag) = bittorrent_rs::MagnetLink::parse(&torrent_path) {
                    mag.display_name.clone().unwrap_or_else(|| bittorrent_rs::util::info_hash_to_string(&mag.info_hash))
                } else {
                    "Magnet Link".to_string()
                }
            } else {
                torrent_path_buf.display().to_string()
            };

            let _ = msg_tx.send(format!("[{}] Connecting to tracker…", session_id));
            println!("[{}] Connecting to tracker…", session_id);

            let log_err = |msg: String| {
                let err_msg = format!("[{}] {}", session_id, msg);
                let _ = msg_tx.send(err_msg.clone());
                eprintln!("{}", err_msg);
            };

            let mut config = bittorrent_rs::session::SessionConfig::default();
            if bitfield_hex_clone.is_some() {
                config.skip_hash_check = true;
            }

            let session_res = if torrent_path.starts_with("magnet:?") {
                TorrentSession::new_magnet_with_options(&torrent_path, &download_dir_buf, config, Arc::new(bittorrent_rs::RarestFirstSelector))
            } else {
                TorrentSession::new_with_options(&torrent_path_buf, &download_dir_buf, false, config, Arc::new(bittorrent_rs::RarestFirstSelector))
            };

            let mut session = match session_res {
                Ok(s) => s,
                Err(e) => {
                    log_err(format!("Failed to create session: {}", e));
                    return;
                }
            };

            // Restore bitfield from cached state if present
            if let Some(ref hex_str) = bitfield_hex_clone {
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
                log_err(format!("Failed to start download: {}", e));
                return;
            }

            let context_clone = session.context.clone();
            let mut tracker = match Tracker::new(context_clone.clone()) {
                Ok(t) => t,
                Err(e) => {
                    log_err(format!("Tracker setup failed: {}", e));
                    let _ = session_tx.send(session);
                    return;
                }
            };

            let task_tx = session.task_tx.clone();
            let peer_workers = session.peer_workers.clone();
            let manager = session.manager.clone();

            // Send TorrentSession to the UI thread immediately so the client displays it instantly
            let _ = session_tx.send(session);

            let _ = msg_tx.send(format!("[{}] Announcing to trackers...", session_id));
            println!("[{}] Announcing to trackers...", session_id);
            match tracker.start_announcing() {
                Ok(response) => {
                    let peer_count = response.peer_list.len();
                    let msg = format!(
                        "[{}] Tracker returned {} peers",
                        session_id, peer_count
                    );
                    let _ = msg_tx.send(msg.clone());
                    println!("{}", msg);
                    if peer_count == 0 {
                        let msg = format!("[{}] No peers; waiting.", session_id);
                        let _ = msg_tx.send(msg.clone());
                        println!("{}", msg);
                    } else {
                        for peer_details in response.peer_list {
                            if let Some(ref mgr) = manager {
                                if mgr.is_peer_dead(&peer_details.ip) {
                                    continue;
                                }
                            }
                            let ctx2 = context_clone.clone();
                            let mgr2 = manager.clone();
                            let peer_workers2 = peer_workers.clone();
                            let handle = std::thread::spawn(move || {
                                futures::executor::block_on(bittorrent_rs::session::worker::handle_peer_session(peer_details, ctx2, mgr2));
                            });
                            peer_workers2.lock().unwrap().push(handle);
                        }
                        let msg = format!("[{}] Download started.", session_id);
                        let _ = msg_tx.send(msg.clone());
                        println!("{}", msg);
                    }

                    // Start the reannounce loop
                    let context_reannounce = context_clone.clone();
                    let peer_workers_reannounce = peer_workers.clone();
                    let manager_reannounce = manager.clone();
                    let _ = task_tx.send(Box::pin(async move {
                        let mut announced_completed = false;
                        loop {
                            let min_reannounce = {
                                let ctx = context_reannounce.lock().unwrap();
                                ctx.config.min_reannounce_interval
                            };
                            let interval = tracker.interval.max(min_reannounce as usize);
                            let start_time = std::time::Instant::now();
                            let duration = Duration::from_secs(interval as u64);
                            let mut ended = false;
                            while start_time.elapsed() < duration {
                                if context_reannounce.lock().unwrap().status == bittorrent_rs::TorrentStatus::Ended {
                                    ended = true;
                                    break;
                                }
                                bittorrent_rs::session::worker::delay(Duration::from_millis(100)).await;
                            }
                            if ended {
                                break;
                            }

                            let status = {
                                let ctx = context_reannounce.lock().unwrap();
                                if ctx.status == bittorrent_rs::TorrentStatus::Ended {
                                    break;
                                }
                                ctx.status
                            };

                            if status == bittorrent_rs::TorrentStatus::Seeding && !announced_completed {
                                announced_completed = true;
                                let _ = tracker.announce_completed();
                                continue;
                            }

                            match tracker.announce_once() {
                                Ok(response) => {
                                    for peer_details in response.peer_list {
                                        let ctx2 = context_reannounce.clone();
                                        let mgr2 = manager_reannounce.clone();
                                        let peer_workers2 = peer_workers_reannounce.clone();

                                        let handle = std::thread::spawn(move || {
                                            futures::executor::block_on(bittorrent_rs::session::worker::handle_peer_session(peer_details, ctx2, mgr2));
                                        });
                                        peer_workers2.lock().unwrap().push(handle);
                                    }
                                }
                                Err(_) => {}
                            }
                        }
                        let _ = tracker.announce_stopped();
                    }));
                }
                Err(e) => {
                    log_err(format!("Tracker announce failed: {}", e));
                }
            }
        });
    }
}
