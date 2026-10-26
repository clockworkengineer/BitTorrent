//! Client State Management and Session Spawning
//!
//! Defines client-side state representations, settings storage structures,
//! sidebar status filtering, config loading/saving, and asynchronous torrent session creation.

use crate::theme::AppTheme;
use bittorrent_rs::TorrentSession;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use torrent_client_shared::{PendingSession, SessionState};

/// Sidebar filters for sorting active torrent sessions by status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum SidebarFilter {
    /// Show all torrents.
    #[default]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetailTab {
    #[default]
    Overview,
    Files,
    Peers,
    Pieces,
    Trackers,
    Logs,
}

/// Library presentation mode for the active torrent list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum LibraryViewMode {
    /// Pro high-density data table with sortable columns.
    #[default]
    Table,
    /// Media-rich card tiles with progress rings and instant streaming.
    Cards,
}

/// Sorting criteria for torrent sessions in the main list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortColumn {
    #[default]
    Name,
    Size,
    Progress,
    DownSpeed,
    UpSpeed,
    Peers,
    Status,
}

/// User configurable application settings.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppSettings {
    pub default_download_dir: String,
    pub listen_port: u16,
    pub max_active_downloads: usize,
    pub enable_dht: bool,
    pub enable_lsd: bool,
    pub enable_utp: bool,
    pub enable_mse: bool,
    pub max_download_speed_kib: u64,
    pub max_upload_speed_kib: u64,
    pub dark_mode: bool,
    #[serde(default)]
    pub theme: AppTheme,
    #[serde(default)]
    pub view_mode: LibraryViewMode,
    #[serde(default)]
    pub turtle_mode: bool,
    #[serde(default = "default_turtle_down")]
    pub turtle_download_kib: u64,
    #[serde(default = "default_turtle_up")]
    pub turtle_upload_kib: u64,
}

fn default_turtle_down() -> u64 {
    256
}
fn default_turtle_up() -> u64 {
    128
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            default_download_dir: String::new(),
            listen_port: 6881,
            max_active_downloads: 5,
            enable_dht: true,
            enable_lsd: true,
            enable_utp: true,
            enable_mse: false,
            max_download_speed_kib: 0,
            max_upload_speed_kib: 0,
            dark_mode: true,
            theme: AppTheme::ObsidianDark,
            view_mode: LibraryViewMode::Table,
            turtle_mode: false,
            turtle_download_kib: 256,
            turtle_upload_kib: 128,
        }
    }
}

/// File metadata entry for the Add Torrent inspector dialog.
#[derive(Debug, Clone)]
pub struct AddTorrentFileItem {
    pub index: usize,
    pub path: String,
    pub length: u64,
    pub selected: bool,
    pub priority: bittorrent_rs::FilePriority,
}

/// State representation for the interactive Add Torrent inspector dialog.
#[derive(Debug, Clone)]
pub struct AddTorrentModalState {
    pub source_path: String,
    pub name: String,
    pub download_dir: String,
    pub total_bytes: u64,
    pub piece_length: u32,
    pub pieces_count: usize,
    pub files: Vec<AddTorrentFileItem>,
    pub is_magnet: bool,
    pub sequential_download: bool,
    pub start_immediately: bool,
    pub available_disk_space: Option<u64>,
    pub error_message: Option<String>,
}

impl AddTorrentModalState {
    /// Computes the sum of bytes for all files marked as selected.
    pub fn selected_bytes(&self) -> u64 {
        if self.files.is_empty() {
            self.total_bytes
        } else {
            self.files
                .iter()
                .filter(|f| f.selected)
                .map(|f| f.length)
                .sum()
        }
    }
}

/// Rolling metrics ring buffer for the last 60 seconds of network activity.
#[derive(Debug, Clone, Default)]
pub struct RollingMetrics {
    pub samples: std::collections::VecDeque<(std::time::Instant, u64, u64)>,
}

impl RollingMetrics {
    /// Appends a new speed measurement (down_bps, up_bps) and trims samples older than 60 seconds.
    pub fn push(&mut self, down_bps: u64, up_bps: u64) {
        let now = std::time::Instant::now();
        self.samples.push_back((now, down_bps, up_bps));
        while let Some(front) = self.samples.front() {
            if now.duration_since(front.0) > std::time::Duration::from_secs(60) {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }
}

/// Queries available free disk space on the given path.
#[cfg(windows)]
pub fn get_available_disk_space(path: &std::path::Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let check_path = if path.exists() {
        path.to_path_buf()
    } else if let Some(parent) = path.parent() {
        if parent.exists() {
            parent.to_path_buf()
        } else {
            path.to_path_buf()
        }
    } else {
        path.to_path_buf()
    };
    let mut wide: Vec<u16> = check_path.as_os_str().encode_wide().collect();
    wide.push(0);
    let mut free_bytes_available: u64 = 0;
    let mut total_number_of_bytes: u64 = 0;
    let mut total_number_of_free_bytes: u64 = 0;
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }
    unsafe {
        let res = GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_bytes_available,
            &mut total_number_of_bytes,
            &mut total_number_of_free_bytes,
        );
        if res != 0 {
            Some(free_bytes_available)
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
pub fn get_available_disk_space(_path: &std::path::Path) -> Option<u64> {
    None
}

/// Prepares an `AddTorrentModalState` by inspecting the provided torrent path or magnet link.
pub fn inspect_torrent_for_modal(source: &str, default_download_dir: &str) -> AddTorrentModalState {
    let source_trimmed = source.trim();
    let is_mag = source_trimmed.starts_with("magnet:?");
    let dir = if default_download_dir.is_empty() {
        dirs::download_dir()
            .map(|d| d.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string())
    } else {
        default_download_dir.to_string()
    };

    let mut modal = AddTorrentModalState {
        source_path: source_trimmed.to_string(),
        name: String::new(),
        download_dir: dir.clone(),
        total_bytes: 0,
        piece_length: 0,
        pieces_count: 0,
        files: Vec::new(),
        is_magnet: is_mag,
        sequential_download: false,
        start_immediately: true,
        available_disk_space: get_available_disk_space(std::path::Path::new(&dir)),
        error_message: None,
    };

    if is_mag {
        if let Ok(mag) = bittorrent_rs::MagnetLink::parse(source_trimmed) {
            modal.name = mag
                .display_name
                .unwrap_or_else(|| bittorrent_rs::util::info_hash_to_string(&mag.info_hash));
        } else {
            modal.name = "Magnet Link".to_string();
        }
    } else {
        let path = std::path::Path::new(source_trimmed);
        modal.name = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| source_trimmed.to_string());

        match bittorrent_rs::metainfo::MetaInfoFile::new(path) {
            Ok(mut meta) => {
                if let Err(e) = meta.parse() {
                    modal.error_message = Some(format!("Failed to parse torrent metainfo: {}", e));
                } else {
                    if let Ok(name) = meta.get_name() {
                        modal.name = name;
                    }
                    if let Ok(piece_len) = meta.get_piece_length() {
                        modal.piece_length = piece_len;
                    }
                    if let Ok(pieces) = meta.get_pieces_info_hash() {
                        modal.pieces_count = pieces.len() / 20;
                    }
                    if let Ok((total, files)) = meta.local_files_to_download_list(&dir) {
                        modal.total_bytes = total;
                        modal.files = files
                            .into_iter()
                            .enumerate()
                            .map(|(idx, f)| AddTorrentFileItem {
                                index: idx,
                                path: f.torrent_path,
                                length: f.length,
                                selected: true,
                                priority: bittorrent_rs::FilePriority::Normal,
                            })
                            .collect();
                    } else if let Ok(size) = meta.get_total_size() {
                        modal.total_bytes = size;
                    }
                }
            }
            Err(e) => {
                modal.error_message = Some(format!("Failed to read torrent file: {}", e));
            }
        }
    }
    modal
}

/// Structure representing a saved torrent session details on disk.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct SavedTorrent {
    pub torrent_path: String,
    pub bitfield_hex: String,
}

/// Persistent window placement and geometry configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct WindowState {
    #[serde(default)]
    pub width: Option<f32>,
    #[serde(default)]
    pub height: Option<f32>,
    #[serde(default)]
    pub x: Option<f32>,
    #[serde(default)]
    pub y: Option<f32>,
    #[serde(default)]
    pub maximized: Option<bool>,
}

/// Config schema representing serialized client application options.
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct ClientState {
    pub download_dir: String,
    pub torrents: Vec<SavedTorrent>,
    #[serde(default)]
    pub settings: AppSettings,
    #[serde(default)]
    pub window: Option<WindowState>,
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

/// Determines the directory where cached .torrent metadata files are stored.
pub fn get_torrents_cache_dir() -> std::path::PathBuf {
    if let Some(mut proj_dirs) = dirs::data_local_dir() {
        proj_dirs.push("BitTorrent-rs");
        proj_dirs.push("torrents");
        let _ = std::fs::create_dir_all(&proj_dirs);
        proj_dirs
    } else {
        let p = std::path::PathBuf::from("cached_torrents");
        let _ = std::fs::create_dir_all(&p);
        p
    }
}

/// Loads the saved window geometry and placement from local storage, if present.
pub fn load_saved_window_state() -> Option<WindowState> {
    let state_path = get_config_path();
    let content = if state_path.exists() {
        std::fs::read_to_string(state_path).ok()
    } else {
        std::fs::read_to_string("torrent_client_state.json").ok()
    }?;
    serde_json::from_str::<ClientState>(&content)
        .ok()
        .and_then(|s| s.window)
}

/// Returns true if a given session matches the selected status filter.
pub fn matches_filter(session: &SessionState, filter: SidebarFilter) -> bool {
    match filter {
        SidebarFilter::All => true,
        SidebarFilter::Downloading => {
            session.last_status == "Downloading" || session.last_status == "Fetching Metadata"
        }
        SidebarFilter::Seeding => session.last_status == "Seeding",
        SidebarFilter::Paused => session.last_status == "Paused",
        SidebarFilter::Completed => {
            (session.last_progress >= 1.0 && session.last_total > 0)
                || session.last_status == "Seeding"
        }
    }
}

use crate::app::TorrentClientApp;

impl TorrentClientApp {
    /// Saves the current client state (active download directory, torrents, and window placement) to disk.
    pub fn save_state(&self) {
        let torrents = self
            .sessions
            .iter()
            .map(|s| {
                let bitfield_hex = if let Ok(ctx) = s.session.context().lock() {
                    bittorrent_rs::util::to_hex(&ctx.bitfield)
                } else {
                    String::new()
                };
                SavedTorrent {
                    torrent_path: s.torrent_path.clone(),
                    bitfield_hex,
                }
            })
            .chain(
                self.pending_sessions
                    .iter()
                    .filter(|s| s.error.is_none())
                    .map(|s| SavedTorrent {
                        torrent_path: s.torrent_path.clone(),
                        bitfield_hex: String::new(),
                    }),
            )
            .collect::<Vec<_>>();

        let window = if self.window_width.is_some()
            || self.window_x.is_some()
            || self.window_maximized.is_some()
        {
            Some(WindowState {
                width: self.window_width,
                height: self.window_height,
                x: self.window_x,
                y: self.window_y,
                maximized: self.window_maximized,
            })
        } else {
            None
        };

        let cache_dir = get_torrents_cache_dir();
        for s in &self.sessions {
            if let Ok(ctx) = s.session.context().lock() {
                if !ctx.pieces_info_hash.is_empty() {
                    let hash_hex = bittorrent_rs::util::info_hash_to_string(&ctx.info_hash);
                    let target_cache = cache_dir.join(format!("{}.torrent", hash_hex));
                    if !target_cache.exists() {
                        let candidate = ctx.download_path.join(format!("{}.torrent", hash_hex));
                        if candidate.exists() {
                            let _ = std::fs::copy(&candidate, &target_cache);
                        }
                    }
                }
            }
        }

        let state = ClientState {
            download_dir: self.download_dir.clone(),
            torrents,
            settings: self.settings.clone(),
            window,
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
                self.settings = state.settings;
                if let Some(ref win) = state.window {
                    self.window_width = win.width;
                    self.window_height = win.height;
                    self.window_x = win.x;
                    self.window_y = win.y;
                    self.window_maximized = win.maximized;
                }
                if self.download_dir.is_empty() && !self.settings.default_download_dir.is_empty() {
                    self.download_dir = self.settings.default_download_dir.clone();
                }
                for t in state.torrents {
                    self.add_session_with_options(
                        t.torrent_path,
                        self.download_dir.clone(),
                        Some(t.bitfield_hex),
                        None,
                        false,
                        true,
                    );
                }
            } else if let Ok(legacy) = serde_json::from_str::<LegacyClientState>(&content) {
                self.download_dir = legacy.download_dir;
                for path in legacy.torrents {
                    self.add_session_with_options(
                        path,
                        self.download_dir.clone(),
                        None,
                        None,
                        false,
                        true,
                    );
                }
            } else {
                let mut lines = content.lines();
                if let Some(dir) = lines.next() {
                    self.download_dir = dir.to_string();
                }
                for line in lines {
                    let path = line.trim().to_string();
                    if !path.is_empty() {
                        self.add_session_with_options(
                            path,
                            self.download_dir.clone(),
                            None,
                            None,
                            false,
                            true,
                        );
                    }
                }
            }
        }
    }

    /// Convenient helper to add a session by path using default options.
    #[allow(dead_code)]
    pub fn add_session_by_path(
        &mut self,
        torrent_path: String,
        download_dir: String,
        bitfield_hex: Option<String>,
    ) {
        self.add_session_with_options(torrent_path, download_dir, bitfield_hex, None, false, true);
    }

    /// Spawns a background thread to asynchronously resolve metadata, verify disk space, and establish tracker connections.
    pub fn add_session_with_options(
        &mut self,
        torrent_path: String,
        download_dir: String,
        bitfield_hex: Option<String>,
        file_priorities: Option<Vec<(usize, bittorrent_rs::FilePriority)>>,
        sequential: bool,
        start_immediately: bool,
    ) {
        let (session_tx, session_rx) = mpsc::channel::<Result<TorrentSession, String>>();
        let msg_tx = self.log_tx.clone();
        self.pending_sessions.push(PendingSession {
            torrent_path: torrent_path.clone(),
            rx: session_rx,
            error: None,
        });
        self.save_state();

        let bitfield_hex_clone = bitfield_hex.clone();
        let settings_clone = self.settings.clone();
        std::thread::spawn(move || {
            let torrent_path_buf = PathBuf::from(&torrent_path);
            let download_dir_buf = PathBuf::from(&download_dir);
            let session_id = if torrent_path.starts_with("magnet:?") {
                if let Ok(mag) = bittorrent_rs::MagnetLink::parse(&torrent_path) {
                    mag.display_name
                        .clone()
                        .unwrap_or_else(|| bittorrent_rs::util::info_hash_to_string(&mag.info_hash))
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
            config.listen_port = settings_clone.listen_port;
            config.dht_enabled = settings_clone.enable_dht;
            config.mse_enabled = settings_clone.enable_mse;
            if bitfield_hex_clone.is_some() {
                config.skip_hash_check = true;
            }

            let selector: Arc<dyn bittorrent_rs::PieceSelector> = if sequential {
                Arc::new(bittorrent_rs::SequentialSelector)
            } else {
                Arc::new(bittorrent_rs::RarestFirstSelector)
            };

            let cached_torrent = if torrent_path.starts_with("magnet:?") {
                if let Ok(mag) = bittorrent_rs::MagnetLink::parse(&torrent_path) {
                    let hash_hex = bittorrent_rs::util::info_hash_to_string(&mag.info_hash);
                    let in_dl = download_dir_buf.join(format!("{}.torrent", hash_hex));
                    let in_cache = get_torrents_cache_dir().join(format!("{}.torrent", hash_hex));
                    if in_dl.exists() {
                        Some(in_dl)
                    } else if in_cache.exists() {
                        Some(in_cache)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            let is_completed = if let Some(ref hex_str) = bitfield_hex_clone {
                if let Ok(bytes) = bittorrent_rs::util::from_hex(hex_str) {
                    let check_torrent_file = cached_torrent.as_ref().unwrap_or(&torrent_path_buf);
                    if let Ok(meta) = bittorrent_rs::metainfo::MetaInfoFile::new(check_torrent_file) {
                        if let Ok(pieces) = meta.get_pieces_info_hash() {
                            let num = pieces.len() / 20;
                            num > 0
                                && (0..num as u32).all(|p| {
                                    let (b_idx, mask) =
                                        bittorrent_rs::util::get_bitfield_index_and_mask(p);
                                    b_idx < bytes.len() && (bytes[b_idx] & mask) != 0
                                })
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };

            let session_res = if let Some(ref tf) = cached_torrent {
                let builder = TorrentSession::builder(tf, &download_dir_buf)
                    .config(config)
                    .selector(selector)
                    .seeding(is_completed);
                builder.build().map(|mut sess| {
                    if let Ok(mag) = bittorrent_rs::MagnetLink::parse(&torrent_path) {
                        if !mag.trackers.is_empty() {
                            let ctx_arc = sess.context();
                            let mut ctx = ctx_arc.lock().unwrap();
                            for tr in mag.trackers {
                                ctx.add_tracker_url(tr);
                            }
                        }
                        if !mag.web_seeds.is_empty() {
                            sess.inject_web_seeds(mag.web_seeds);
                        }
                    }
                    sess
                })
            } else if torrent_path.starts_with("magnet:?") {
                TorrentSession::from_magnet(&torrent_path, &download_dir_buf)
                    .config(config)
                    .selector(selector)
                    .seeding(is_completed)
                    .build()
            } else {
                TorrentSession::builder(&torrent_path_buf, &download_dir_buf)
                    .config(config)
                    .selector(selector)
                    .seeding(is_completed)
                    .build()
            };

            let mut session = match session_res {
                Ok(s) => s,
                Err(e) => {
                    let err_str = format!("Failed to create session: {}", e);
                    log_err(err_str.clone());
                    let _ = session_tx.send(Err(err_str));
                    return;
                }
            };

            // Apply custom file priorities if specified
            if let Some(prio_list) = file_priorities
                && let Ok(mut ctx) = session.context.lock()
            {
                for (idx, prio) in prio_list {
                    let _ = ctx.set_file_priority(idx, prio);
                }
            }

            // Restore bitfield from cached state if present
            if let Some(ref hex_str) = bitfield_hex_clone
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
                    if missing_count == 0 && downloaded > 0 {
                        ctx.set_status(bittorrent_rs::TorrentStatus::Seeding);
                        ctx.download_finished.set();
                    }
                }
            }

            if let Ok(mut ctx) = session.context.lock() {
                if ctx.is_download_complete() {
                    ctx.set_status(bittorrent_rs::TorrentStatus::Seeding);
                    ctx.download_finished.set();
                }
            }

            if let Err(e) = session.start_download() {
                let err_str = format!("Failed to start download: {}", e);
                log_err(err_str.clone());
                let _ = session_tx.send(Err(err_str));
                return;
            }

            if !start_immediately {
                let _ = session.pause();
            }

            // Send TorrentSession to the UI thread immediately so the client displays it instantly
            let _ = session_tx.send(Ok(session));
        });
    }
}
