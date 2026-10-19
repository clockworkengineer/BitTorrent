//! Application Shell and Egui Integration
//!
//! Orchestrates app startup, window repaint loop timing, sidebar selection updates,
//! status metrics tracking, and mapping background messages to the main UI logger.

use crate::state::{AppSettings, DetailTab, SidebarFilter, SortColumn, matches_filter};
use eframe::egui;
use std::sync::mpsc;
use std::time::Duration;
use torrent_client_shared::{PendingSession, SessionState, fmt_bytes};

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
        }
    }

    /// Counts how many of the currently loaded sessions match a given status category filter.
    pub fn count_torrents_for_filter(&self, filter: SidebarFilter) -> usize {
        self.sessions
            .iter()
            .filter(|s| matches_filter(s, filter))
            .count()
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
                    // Automatically find the largest media file
                    let mut best = None;
                    let mut max_len = 0;
                    for (i, f) in ctx.files_to_download.iter().enumerate() {
                        if torrent_client_shared::is_media_filename(&f.torrent_path)
                            && f.length > max_len
                        {
                            max_len = f.length;
                            best = Some(i);
                        }
                    }
                    if best.is_none() {
                        for (i, f) in ctx.files_to_download.iter().enumerate() {
                            if f.length > max_len {
                                max_len = f.length;
                                best = Some(i);
                            }
                        }
                    }
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
}

impl eframe::App for TorrentClientApp {
    /// Callback executed when the desktop window is closed.
    /// Saves the current configuration state and halts all active torrent worker threads.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Save final state before stopping sessions to capture the exact downloaded bitfields
        self.save_state();

        self.stop_streaming();

        // Stop all active sessions so background threads don't outlive the process.
        for state in &mut self.sessions {
            let _ = state.session.stop();
        }

        std::process::exit(0);
    }

    /// Primary UI draw and events update loop called by the egui framework backend.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Repaint every 500 ms so progress updates live
        ctx.request_repaint_after(Duration::from_millis(500));

        // Global keyboard shortcuts
        let mut trigger_browse = false;
        let mut toggle_pause = false;
        let mut trigger_delete = false;

        ctx.input(|i| {
            if i.modifiers.command && i.key_pressed(egui::Key::O) {
                trigger_browse = true;
            }
            if i.modifiers.command && i.key_pressed(egui::Key::Comma) {
                self.show_settings_dialog = true;
            }
            if !ctx.wants_keyboard_input() {
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
            self.torrent_path = path.to_string_lossy().to_string();
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

        // Top Toolbar
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.heading(egui::RichText::new("📥 BitTorrent Client").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut is_dark = ctx.style().visuals.dark_mode;
                    if ui.checkbox(&mut is_dark, "Dark Mode").changed() {
                        ctx.set_visuals(if is_dark {
                            egui::Visuals::dark()
                        } else {
                            egui::Visuals::light()
                        });
                    }
                    ui.add_space(8.0);
                    if ui.button("ℹ About").clicked() {
                        self.show_about_dialog = true;
                    }
                    ui.add_space(6.0);
                    if ui.button("⚙ Settings").clicked() {
                        self.show_settings_dialog = true;
                    }
                });
            });
            ui.add_space(8.0);
        });

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
                            .color(egui::Color32::from_rgb(52, 152, 219)),
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

        // Modal Dialog Windows
        self.draw_about_dialog(ctx);
        self.draw_settings_dialog(ctx);
        self.draw_delete_confirm_dialog(ctx);

        // Bottom Status Bar
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
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

                ui.label(format!("Active Torrents: {}", active_count));
                ui.separator();
                ui.label(format!(
                    "Total Speed: ↓ {}/s  ↑ {}/s",
                    fmt_bytes(total_down_bps),
                    fmt_bytes(total_up_bps)
                ));
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
                        egui::RichText::new("✔ Online")
                            .color(egui::Color32::from_rgb(46, 204, 113)),
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

        // Central Panel (Input details + active Torrent grid list)
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Torrent File / Magnet:");
                    ui.add_sized(
                        [300.0, 20.0],
                        egui::TextEdit::singleline(&mut self.torrent_path),
                    );
                    if ui.button("📂 Browse File").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("Torrent Files", &["torrent"])
                            .pick_file()
                    {
                        self.torrent_path = path.to_string_lossy().to_string();
                    }
                });

                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    ui.label("Download Directory:");
                    ui.add_sized(
                        [300.0, 20.0],
                        egui::TextEdit::singleline(&mut self.download_dir),
                    );
                    if ui.button("📂 Browse Directory").clicked()
                        && let Some(path) = rfd::FileDialog::new().pick_folder()
                    {
                        self.download_dir = path.to_string_lossy().to_string();
                        self.save_state();
                    }
                    ui.add_space(10.0);
                    if ui
                        .button(egui::RichText::new("➕ Add Torrent").strong())
                        .clicked()
                    {
                        self.create_session();
                    }
                });
            });

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            // Sorting bar
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Sort by:").strong().size(12.0));
                let mut sort_btn = |ui: &mut egui::Ui, col: SortColumn, name: &str| {
                    let arrow = if self.sort_column == col {
                        if self.sort_ascending { " ⬆" } else { " ⬇" }
                    } else {
                        ""
                    };
                    let is_active = self.sort_column == col;
                    let text = format!("{}{}", name, arrow);
                    let resp = if is_active {
                        ui.button(egui::RichText::new(text).strong())
                    } else {
                        ui.button(text)
                    };
                    if resp.clicked() {
                        if self.sort_column == col {
                            self.sort_ascending = !self.sort_ascending;
                        } else {
                            self.sort_column = col;
                            self.sort_ascending = true;
                        }
                    }
                };
                sort_btn(ui, SortColumn::Name, "Name");
                sort_btn(ui, SortColumn::Progress, "Progress");
                sort_btn(ui, SortColumn::Size, "Size");
                sort_btn(ui, SortColumn::Status, "Status");
                sort_btn(ui, SortColumn::DownSpeed, "↓ Speed");
                sort_btn(ui, SortColumn::UpSpeed, "↑ Speed");
                sort_btn(ui, SortColumn::Peers, "Peers");
            });

            ui.add_space(6.0);

            egui::ScrollArea::vertical()
                .id_source("sessions_scroll")
                .show(ui, |ui| {
                    let mut select_idx = None;
                    let mut stream_torrent_idx = None;

                    let mut delete_pending_indices = vec![];

                    // Display pending/loading sessions with a spinner or error state and delete button
                    for (p_idx, pending) in self.pending_sessions.iter().enumerate() {
                        let name = std::path::Path::new(&pending.torrent_path)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| pending.torrent_path.clone());

                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                let title_color = if pending.error.is_some() {
                                    egui::Color32::from_rgb(231, 76, 60)
                                } else {
                                    egui::Color32::from_rgb(230, 126, 34)
                                };
                                ui.label(egui::RichText::new(&name).strong().color(title_color));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button("🗑 Delete").clicked() {
                                            delete_pending_indices.push(p_idx);
                                        }
                                        ui.add_space(6.0);
                                        if pending.error.is_some() {
                                            ui.label(
                                                egui::RichText::new("Failed")
                                                    .color(egui::Color32::from_rgb(231, 76, 60))
                                                    .strong(),
                                            );
                                        } else {
                                            ui.add(egui::Spinner::new());
                                            ui.label("Loading... ");
                                        }
                                    },
                                );
                            });
                            ui.horizontal(|ui| {
                                if let Some(ref err) = pending.error {
                                    ui.label(
                                        egui::RichText::new(format!("⚠ {}", err))
                                            .color(egui::Color32::from_rgb(231, 76, 60)),
                                    );
                                } else {
                                    ui.label("Verifying local storage and resolving metadata...");
                                }
                            });
                        });
                        ui.add_space(6.0);
                    }

                    if !delete_pending_indices.is_empty() {
                        for idx in delete_pending_indices.into_iter().rev() {
                            self.pending_sessions.remove(idx);
                        }
                        self.save_state();
                    }

                    let mut should_save_progress = false;
                    for session_state in &mut self.sessions {
                        let context_arc = session_state.session.context.clone();
                        if let Ok(ctx_guard) = context_arc.try_lock() {
                            let old_progress = session_state.last_progress;
                            session_state.update_fields(&ctx_guard);
                            if session_state.last_progress != old_progress {
                                should_save_progress = true;
                            }
                        }
                    }

                    let mut visible_indices: Vec<usize> = self
                        .sessions
                        .iter()
                        .enumerate()
                        .filter(|(_, s)| matches_filter(s, self.active_filter))
                        .map(|(idx, _)| idx)
                        .collect();

                    visible_indices.sort_by(|&a, &b| {
                        let sa = &self.sessions[a];
                        let sb = &self.sessions[b];
                        let ord = match self.sort_column {
                            SortColumn::Name => sa
                                .last_file_name
                                .to_lowercase()
                                .cmp(&sb.last_file_name.to_lowercase()),
                            SortColumn::Size => sa.last_total.cmp(&sb.last_total),
                            SortColumn::Progress => sa.last_progress.total_cmp(&sb.last_progress),
                            SortColumn::DownSpeed => sa.last_bps.cmp(&sb.last_bps),
                            SortColumn::UpSpeed => sa.last_upload_bps.cmp(&sb.last_upload_bps),
                            SortColumn::Peers => {
                                sa.last_peers_connected.cmp(&sb.last_peers_connected)
                            }
                            SortColumn::Status => sa.last_status.cmp(&sb.last_status),
                        };
                        if self.sort_ascending {
                            ord
                        } else {
                            ord.reverse()
                        }
                    });

                    for &i in &visible_indices {
                        let session_state = &mut self.sessions[i];
                        let is_selected = self.selected_session_index == Some(i);

                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                let name =
                                    egui::RichText::new(&session_state.last_file_name).strong();
                                let label_response = if is_selected {
                                    ui.label(name.color(ui.visuals().selection.bg_fill))
                                } else {
                                    ui.label(name)
                                };

                                // Make only name label clickable to select the torrent
                                let label_click = label_response.interact(egui::Sense::click());
                                if label_click.clicked() {
                                    select_idx = Some(i);
                                }

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button("🗑 Delete").clicked() {
                                            self.delete_confirm_idx = Some(i);
                                            self.delete_files_checked = false;
                                        }
                                        ui.add_space(6.0);
                                        if session_state.last_status == "Paused" {
                                            if ui.button("▶ Resume").clicked() {
                                                let _ = session_state.session.resume();
                                            }
                                        } else if session_state.last_status != "Ended"
                                            && ui.button("⏸ Pause").clicked()
                                        {
                                            let _ = session_state.session.pause();
                                        }
                                        ui.add_space(6.0);
                                        if session_state.last_status != "Ended"
                                            && ui.button("▶ Stream").clicked()
                                        {
                                            stream_torrent_idx = Some(i);
                                        }
                                    },
                                );
                            });

                            ui.horizontal(|ui| {
                                let progress = session_state.last_progress;
                                let bar = egui::ProgressBar::new(progress)
                                    .text(format!("{:.1}%", progress * 100.0))
                                    .animate(
                                        progress < 1.0
                                            && session_state.last_status == "Downloading",
                                    );
                                ui.add_sized([180.0, 16.0], bar);

                                ui.separator();
                                ui.label(format!("Size: {}", fmt_bytes(session_state.last_total)));
                                ui.separator();
                                ui.label(format!("Status: {}", session_state.last_status));
                                ui.separator();
                                ui.label(format!("↓ {}/s", fmt_bytes(session_state.last_bps)));
                                ui.separator();
                                ui.label(format!("Peers: {}", session_state.last_peers_connected));
                            });
                        });

                        ui.add_space(6.0);
                    }

                    if let Some(idx) = select_idx {
                        self.selected_session_index = Some(idx);
                    }

                    if let Some(idx) = stream_torrent_idx {
                        self.start_streaming(idx, None);
                    }

                    if should_save_progress {
                        self.save_state();
                    }
                });
        });
    }
}
