//! Application Shell and Egui Integration
//!
//! Orchestrates app startup, window repaint loop timing, sidebar selection updates,
//! status metrics tracking, and mapping background messages to the main UI logger.

use crate::state::{
    AddTorrentModalState, AppSettings, DetailTab, LibraryViewMode, RollingMetrics, SidebarFilter,
    SortColumn, matches_filter,
};
use eframe::egui;
use std::collections::HashSet;
use std::sync::mpsc;
use std::time::Duration;
use torrent_client_shared::{PendingSession, SessionState, find_primary_media_file, fmt_bytes};

/// The main application state container holding UI variables, logs, and active session buffers.
pub struct TorrentClientApp {
    /// Input field containing the loaded torrent metainfo file path or magnet link.
    pub torrent_path: String,
    /// Destination directory path where files are downloaded.
    pub download_dir: String,
    /// Accumulated log entries printed to the logs console tab.
    pub messages: Vec<String>,
    /// Array of fully active, initialized downloading/seeding torrent session states.
    pub sessions: Vec<SessionState>,
    /// Array of torrents currently being parsed/verified in background threads.
    pub pending_sessions: Vec<PendingSession>,
    /// Receiver channel connecting background log outputs to the application UI logger.
    pub log_rx: mpsc::Receiver<String>,
    /// Sender channel for writing logs from spawned worker threads.
    pub log_tx: mpsc::Sender<String>,

    // UI state
    /// Index of the currently highlighted torrent session inside self.sessions.
    pub selected_session_index: Option<usize>,
    /// Filter applied to the list of torrents shown (e.g. Seeding, Completed).
    pub active_filter: SidebarFilter,
    /// Selected tab shown in the bottom metadata details container.
    pub active_detail_tab: DetailTab,
    /// Search query string used to filter logs.
    pub log_search_query: String,
    /// Visibility state of the About modal dialog.
    pub show_about_dialog: bool,
    /// Visibility state of the Preferences / Settings modal dialog.
    pub show_settings_dialog: bool,
    /// Session index pending user confirmation for removal.
    pub delete_confirm_idx: Option<usize>,
    /// Checkbox state for deleting files from disk during removal.
    pub delete_files_checked: bool,
    /// Active column used to sort the torrent list.
    pub sort_column: SortColumn,
    /// Sort direction (ascending if true, descending if false).
    pub sort_ascending: bool,
    /// User configurable application settings.
    pub settings: AppSettings,
    /// In-app media streaming server for the desktop GUI.
    pub stream_server: Option<std::sync::Arc<crate::stream_server::StreamServer>>,
    /// Whether initial persistent state has been restored on the first render frame.
    pub state_loaded: bool,

    // Advanced & Interactive UI State
    /// Active state of the Add Torrent inspector modal.
    pub add_torrent_modal: Option<AddTorrentModalState>,
    /// Visibility state of the Magnet Link input prompt modal.
    pub show_magnet_dialog: bool,
    /// Text input buffer for the Magnet Link modal.
    pub magnet_input_text: String,
    /// Search query string for filtering the active torrent library in real-time.
    pub search_filter_query: String,
    /// Ring buffer tracking rolling download and upload speeds for the live sparkline graph.
    pub rolling_metrics: RollingMetrics,
    /// Last recorded inner window width.
    pub window_width: Option<f32>,
    /// Last recorded inner window height.
    pub window_height: Option<f32>,
    /// Last recorded window screen X coordinate.
    pub window_x: Option<f32>,
    /// Last recorded window screen Y coordinate.
    pub window_y: Option<f32>,
    /// Whether the window was maximized.
    pub window_maximized: Option<bool>,
    /// Set of selected torrent indices for multi-selection.
    #[allow(dead_code)]
    pub selected_torrent_indices: HashSet<usize>,
    /// Prevents repeated execution of shutdown routines across exit hooks.
    pub shutting_down: bool,
}

impl TorrentClientApp {
    /// Creates a new, uninitialized `TorrentClientApp` instance using the log channels.
    pub fn new(log_rx: mpsc::Receiver<String>, log_tx: mpsc::Sender<String>) -> Self {
        let stream_server = crate::stream_server::StreamServer::new()
            .ok()
            .map(std::sync::Arc::new);
        Self {
            torrent_path: String::new(),
            download_dir: String::new(),
            messages: Vec::new(),
            sessions: Vec::new(),
            pending_sessions: Vec::new(),
            log_rx,
            log_tx,
            selected_session_index: None,
            active_filter: SidebarFilter::All,
            active_detail_tab: DetailTab::Overview,
            log_search_query: String::new(),
            show_about_dialog: false,
            show_settings_dialog: false,
            delete_confirm_idx: None,
            delete_files_checked: false,
            sort_column: SortColumn::default(),
            sort_ascending: true,
            settings: AppSettings::default(),
            stream_server,
            state_loaded: false,
            add_torrent_modal: None,
            show_magnet_dialog: false,
            magnet_input_text: String::new(),
            search_filter_query: String::new(),
            rolling_metrics: RollingMetrics::default(),
            selected_torrent_indices: HashSet::new(),
            window_width: None,
            window_height: None,
            window_x: None,
            window_y: None,
            window_maximized: None,
            shutting_down: false,
        }
    }

    /// Counts how many of the currently loaded sessions match a given status category filter.
    pub fn count_torrents_for_filter(&self, filter: SidebarFilter) -> usize {
        self.sessions
            .iter()
            .filter(|s| matches_filter(s, filter))
            .count()
    }

    /// Opens the interactive Add Torrent modal dialog for a given file path or magnet link.
    pub fn open_add_torrent_modal(&mut self, source_path: &str) {
        let download_dir = if !self.download_dir.is_empty() {
            &self.download_dir
        } else {
            &self.settings.default_download_dir
        };
        self.add_torrent_modal = Some(crate::state::inspect_torrent_for_modal(
            source_path,
            download_dir,
        ));
    }

    /// Pauses all active downloading or seeding torrent sessions.
    pub fn pause_all(&mut self) {
        for s in &mut self.sessions {
            if s.last_status != "Ended" && s.last_status != "Paused" {
                let _ = s.session.pause();
            }
        }
    }

    /// Resumes all currently paused torrent sessions.
    pub fn resume_all(&mut self) {
        for s in &mut self.sessions {
            if s.last_status == "Paused" {
                let _ = s.session.resume();
            }
        }
    }

    /// Toggles turtle speed throttle mode.
    pub fn toggle_turtle_mode(&mut self) {
        self.settings.turtle_mode = !self.settings.turtle_mode;
        self.save_state();
    }

    /// Starts streaming the selected torrent session or a specific file within it.
    pub fn start_streaming(&mut self, session_idx: usize, file_idx: Option<usize>) {
        let Some(session_state) = self.sessions.get(session_idx) else {
            return;
        };
        let Some(server) = &self.stream_server else {
            return;
        };

        let context = session_state.session.context.clone();
        let (file_name, total_size, actual_file_idx) = {
            if let Ok(ctx) = context.lock() {
                if let Some(idx) = file_idx {
                    if let Some(f) = ctx.files_to_download.get(idx) {
                        (f.torrent_path.clone(), f.length, Some(idx))
                    } else {
                        (
                            session_state.last_file_name.clone(),
                            ctx.total_bytes_to_download,
                            None,
                        )
                    }
                } else {
                    let best = find_primary_media_file(
                        ctx.files_to_download
                            .iter()
                            .map(|f| (&f.torrent_path, f.length)),
                    );
                    if let Some(idx) = best {
                        let f = &ctx.files_to_download[idx];
                        (f.torrent_path.clone(), f.length, Some(idx))
                    } else {
                        (
                            session_state.last_file_name.clone(),
                            ctx.total_bytes_to_download,
                            None,
                        )
                    }
                }
            } else {
                (
                    session_state.last_file_name.clone(),
                    session_state.last_total,
                    None,
                )
            }
        };

        server.set_active_stream(
            session_state.last_file_name.clone(),
            file_name,
            context,
            actual_file_idx,
            total_size,
        );

        let _ = server.open_in_player();
    }

    /// Stops the currently active media stream.
    pub fn stop_streaming(&mut self) {
        if let Some(server) = &self.stream_server {
            server.stop_active_stream();
        }
    }

    /// Gracefully flushes state, terminates stream servers, and stops all active torrent sessions once.
    pub fn shutdown(&mut self) {
        if self.shutting_down {
            return;
        }
        self.shutting_down = true;
        self.save_state();
        self.stop_streaming();
        for state in &mut self.sessions {
            let _ = state.session.stop();
        }
    }
}

impl eframe::App for TorrentClientApp {
    /// Callback executed when the desktop window is closed.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shutdown();
    }

    /// Primary UI draw and events update loop called by the egui framework backend.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Track live window geometry and placement
        ctx.input(|i| {
            let vp = i.viewport();
            let is_max = vp.maximized.unwrap_or(false);
            self.window_maximized = vp.maximized;
            if !is_max {
                if let Some(inner) = vp.inner_rect {
                    if inner.width() >= 400.0 && inner.height() >= 300.0 {
                        self.window_width = Some(inner.width());
                        self.window_height = Some(inner.height());
                    }
                }
                if let Some(outer) = vp.outer_rect {
                    self.window_x = Some(outer.min.x);
                    self.window_y = Some(outer.min.y);
                }
            }
        });

        if ctx.input(|i| i.viewport().close_requested()) {
            self.shutdown();
            return;
        }

        if !self.state_loaded {
            self.state_loaded = true;
            self.load_state();
            self.settings.theme.apply(ctx);
            if !self.torrent_path.is_empty() {
                self.open_add_torrent_modal(&self.torrent_path.clone());
            }
        }

        // Repaint every 500 ms so progress updates live
        ctx.request_repaint_after(Duration::from_millis(500));

        // Detect dropped files (drag-and-drop ingestion)
        let mut dropped_to_open = None;
        ctx.input(|i| {
            for dropped in &i.raw.dropped_files {
                if let Some(path) = &dropped.path {
                    let path_str = path.to_string_lossy().to_string();
                    if path_str.ends_with(".torrent") || path_str.starts_with("magnet:?") {
                        dropped_to_open = Some(path_str);
                        break;
                    }
                }
            }
        });
        if let Some(path) = dropped_to_open {
            self.open_add_torrent_modal(&path);
        }

        // Global keyboard shortcuts
        let mut trigger_browse = false;
        let mut trigger_magnet = false;
        let mut toggle_pause = false;
        let mut trigger_delete = false;

        let wants_keyboard = ctx.wants_keyboard_input();
        ctx.input(|i| {
            if i.modifiers.command && i.key_pressed(egui::Key::O) {
                trigger_browse = true;
            }
            if i.modifiers.command && i.key_pressed(egui::Key::M) {
                trigger_magnet = true;
            }
            if i.modifiers.command && i.key_pressed(egui::Key::Comma) {
                self.show_settings_dialog = true;
            }
            if !wants_keyboard {
                if i.key_pressed(egui::Key::Delete) {
                    trigger_delete = true;
                }
                if i.key_pressed(egui::Key::Space) {
                    toggle_pause = true;
                }
            }
        });

        if trigger_browse
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Torrent Files", &["torrent"])
                .pick_file()
        {
            self.open_add_torrent_modal(&path.to_string_lossy());
        }

        if trigger_magnet {
            self.show_magnet_dialog = true;
            self.magnet_input_text.clear();
        }

        if trigger_delete
            && let Some(idx) = self.selected_session_index
            && idx < self.sessions.len()
        {
            self.delete_confirm_idx = Some(idx);
            self.delete_files_checked = false;
        }

        if toggle_pause
            && let Some(idx) = self.selected_session_index
            && let Some(s) = self.sessions.get_mut(idx)
        {
            if s.last_status == "Paused" {
                let _ = s.session.resume();
            } else if s.last_status != "Ended" {
                let _ = s.session.pause();
            }
        }

        // Drain log messages from background threads
        while let Ok(msg) = self.log_rx.try_recv() {
            self.messages.push(msg);
        }

        // Move ready sessions into self.sessions
        let mut ready = vec![];
        let mut should_save = false;
        for (i, pending) in self.pending_sessions.iter_mut().enumerate() {
            match pending.rx.try_recv() {
                Ok(Ok(session)) => {
                    self.sessions
                        .push(SessionState::new(session, pending.torrent_path.clone()));
                    ready.push(i);
                    should_save = true;
                }
                Ok(Err(err)) => {
                    pending.error = Some(err);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if pending.error.is_none() {
                        pending.error = Some(
                            "Failed to initialize session (file missing or thread terminated)"
                                .to_string(),
                        );
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        for i in ready.into_iter().rev() {
            self.pending_sessions.remove(i);
        }
        if should_save {
            self.save_state();
        }

        // Top Command Toolbar
        self.draw_command_toolbar(ctx);

        // Active Streaming Banner
        if let Some(server) = &self.stream_server
            && let Some(active) = server.active_stream()
        {
            egui::TopBottomPanel::top("stream_banner").show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("🎬 Streaming:")
                            .strong()
                            .color(self.settings.theme.stream_color()),
                    );
                    let stream_title = if active.file_name != active.torrent_name {
                        format!("{}: {}", active.torrent_name, active.file_name)
                    } else {
                        active.file_name.clone()
                    };
                    ui.label(egui::RichText::new(stream_title).strong());

                    let cur = active
                        .current_position
                        .load(std::sync::atomic::Ordering::Relaxed);
                    let prog = if active.total_size > 0 {
                        (cur as f32) / (active.total_size as f32)
                    } else {
                        0.0
                    };
                    ui.add_sized(
                        [120.0, 14.0],
                        egui::ProgressBar::new(prog).text(format!("{:.1}%", prog * 100.0)),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("⏹ Stop").clicked() {
                            server.stop_active_stream();
                        }
                        ui.add_space(6.0);
                        if ui.button("📋 Copy URL").clicked() {
                            ui.output_mut(|o| o.copied_text = server.stream_url());
                        }
                        ui.add_space(6.0);
                        if ui.button("🚀 Open Player").clicked() {
                            let _ = server.open_in_player();
                        }
                    });
                });
                ui.add_space(4.0);
            });
        }

        // Drag & Drop Hover Overlay
        self.draw_drag_and_drop_overlay(ctx);

        // Modal Dialog Windows
        self.draw_add_torrent_dialog(ctx);
        self.draw_magnet_input_dialog(ctx);
        self.draw_about_dialog(ctx);
        self.draw_settings_dialog(ctx);
        self.draw_delete_confirm_dialog(ctx);

        // Bottom Status Bar with Live Speed Metrics & Sparkline
        let mut total_down_bps = 0u64;
        let mut total_up_bps = 0u64;
        let mut active_count = 0;
        for s in &self.sessions {
            if s.last_status == "Downloading" || s.last_status == "Seeding" {
                active_count += 1;
            }
            total_down_bps += s.last_bps;
            total_up_bps += s.last_upload_bps;
        }
        self.rolling_metrics.push(total_down_bps, total_up_bps);

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("Active Torrents: {}", active_count));
                ui.separator();
                ui.colored_label(
                    self.settings.theme.success_color(),
                    format!("↓ {}/s", fmt_bytes(total_down_bps)),
                );
                ui.colored_label(
                    self.settings.theme.upload_color(),
                    format!("↑ {}/s", fmt_bytes(total_up_bps)),
                );
                ui.separator();

                // Live Activity Sparkline Canvas
                self.draw_sparkline(ui, 100.0, 16.0);
                ui.separator();

                ui.label(format!(
                    "Total Peers: {}",
                    self.sessions
                        .iter()
                        .map(|s| s.last_peers_connected)
                        .sum::<usize>()
                ));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("✔ Online").color(self.settings.theme.success_color()),
                    );
                });
            });
        });

        // Sidebar Filter Panel (Left)
        egui::SidePanel::left("left_sidebar")
            .resizable(false)
            .default_width(150.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("STATUS")
                        .strong()
                        .size(11.0)
                        .color(ui.visuals().weak_text_color()),
                );
                ui.add_space(6.0);

                let mut draw_filter_button =
                    |ui: &mut egui::Ui, filter: SidebarFilter, icon: &str, text: &str| {
                        let count = self.count_torrents_for_filter(filter);
                        let label = format!("{} {} ({})", icon, text, count);
                        let is_selected = self.active_filter == filter;
                        if ui.selectable_label(is_selected, label).clicked() {
                            self.active_filter = filter;
                        }
                    };

                draw_filter_button(ui, SidebarFilter::All, "📁", "All");
                ui.add_space(4.0);
                draw_filter_button(ui, SidebarFilter::Downloading, "📥", "Downloading");
                ui.add_space(4.0);
                draw_filter_button(ui, SidebarFilter::Seeding, "📤", "Seeding");
                ui.add_space(4.0);
                draw_filter_button(ui, SidebarFilter::Paused, "⏸", "Paused");
                ui.add_space(4.0);
                draw_filter_button(ui, SidebarFilter::Completed, "✅", "Completed");
            });

        // Details Panel (Bottom-Right, resizable)
        if self.selected_session_index.is_some() {
            egui::TopBottomPanel::bottom("details_panel")
                .resizable(true)
                .default_height(220.0)
                .show(ctx, |ui| {
                    self.draw_details_panel(ui);
                });
        }

        // Central Panel (Active Library Grid/Table)
        egui::CentralPanel::default().show(ctx, |ui| {
            // Display pending/loading sessions with a spinner or error state
            let mut delete_pending_indices = vec![];
            for (p_idx, pending) in self.pending_sessions.iter().enumerate() {
                let name = std::path::Path::new(&pending.torrent_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| pending.torrent_path.clone());

                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        let title_color = if pending.error.is_some() {
                            self.settings.theme.error_color()
                        } else {
                            self.settings.theme.warning_color()
                        };
                        ui.label(egui::RichText::new(&name).strong().color(title_color));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("🗑 Cancel").clicked() {
                                delete_pending_indices.push(p_idx);
                            }
                            ui.add_space(6.0);
                            if pending.error.is_some() {
                                ui.colored_label(self.settings.theme.error_color(), "Failed");
                            } else {
                                ui.add(egui::Spinner::new());
                                ui.label("Loading... ");
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        if let Some(ref err) = pending.error {
                            ui.colored_label(
                                self.settings.theme.error_color(),
                                format!("⚠ {}", err),
                            );
                        } else {
                            ui.label("Connecting to swarm and verifying storage...");
                        }
                    });
                });
                ui.add_space(4.0);
            }

            if !delete_pending_indices.is_empty() {
                for idx in delete_pending_indices.into_iter().rev() {
                    self.pending_sessions.remove(idx);
                }
                self.save_state();
            }

            // Synchronize active torrent context fields
            let mut should_save_progress = false;
            for session_state in &mut self.sessions {
                let context_arc = session_state.session.context.clone();
                if let Ok(mut ctx_guard) = context_arc.try_lock() {
                    let storage = ctx_guard.storage.clone();
                    ctx_guard.drain_verification_results(&*storage);
                    let old_progress = session_state.last_progress;
                    session_state.update_fields(&ctx_guard);
                    if session_state.last_progress != old_progress {
                        should_save_progress = true;
                    }
                }
            }
            if should_save_progress {
                self.save_state();
            }

            // Filter by sidebar category AND quick search query
            let query = self.search_filter_query.to_lowercase();
            let mut visible_indices: Vec<usize> = self
                .sessions
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    matches_filter(s, self.active_filter)
                        && (query.is_empty() || s.last_file_name.to_lowercase().contains(&query))
                })
                .map(|(idx, _)| idx)
                .collect();

            // Sort visible torrents
            visible_indices.sort_by(|&a, &b| {
                let sa = &self.sessions[a];
                let sb = &self.sessions[b];
                let ord = match self.sort_column {
                    SortColumn::Name => sa
                        .last_file_name
                        .chars()
                        .map(|c| c.to_ascii_lowercase())
                        .cmp(sb.last_file_name.chars().map(|c| c.to_ascii_lowercase())),
                    SortColumn::Size => sa.last_total.cmp(&sb.last_total),
                    SortColumn::Progress => sa.last_progress.total_cmp(&sb.last_progress),
                    SortColumn::DownSpeed => sa.last_bps.cmp(&sb.last_bps),
                    SortColumn::UpSpeed => sa.last_upload_bps.cmp(&sb.last_upload_bps),
                    SortColumn::Peers => sa.last_peers_connected.cmp(&sb.last_peers_connected),
                    SortColumn::Status => sa.last_status.cmp(&sb.last_status),
                };
                if self.sort_ascending {
                    ord
                } else {
                    ord.reverse()
                }
            });

            // Render active view mode
            match self.settings.view_mode {
                LibraryViewMode::Table => self.draw_pro_table_view(ui, &visible_indices),
                LibraryViewMode::Cards => self.draw_cards_view(ui, &visible_indices),
            }
        });
    }
}
