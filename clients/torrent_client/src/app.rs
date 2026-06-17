//! Application Shell and Egui Integration
//!
//! Orchestrates app startup, window repaint loop timing, sidebar selection updates,
//! status metrics tracking, and mapping background messages to the main UI logger.

use eframe::egui;
use std::sync::mpsc;
use std::time::Duration;
use torrent_client_shared::{SessionState, PendingSession, fmt_bytes};
use crate::state::{SidebarFilter, DetailTab, matches_filter};

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
}

impl TorrentClientApp {
    /// Creates a new, uninitialized `TorrentClientApp` instance using the log channels.
    pub fn new(log_rx: mpsc::Receiver<String>, log_tx: mpsc::Sender<String>) -> Self {
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
        }
    }

    /// Counts how many of the currently loaded sessions match a given status category filter.
    pub fn count_torrents_for_filter(&self, filter: SidebarFilter) -> usize {
        self.sessions.iter().filter(|s| matches_filter(s, filter)).count()
    }
}

impl eframe::App for TorrentClientApp {
    /// Callback executed when the desktop window is closed.
    /// Saves the current configuration state and halts all active torrent worker threads.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Save final state before stopping sessions to capture the exact downloaded bitfields
        self.save_state();

        // Stop all active sessions so background threads don't outlive the process.
        for state in &mut self.sessions {
            let _ = state.session.stop();
        }
    }

    /// Primary UI draw and events update loop called by the egui framework backend.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Repaint every 500 ms so progress updates live
        ctx.request_repaint_after(Duration::from_millis(500));

        // Drain log messages from background threads
        while let Ok(msg) = self.log_rx.try_recv() {
            self.messages.push(msg);
        }

        // Move ready sessions into self.sessions
        let mut ready = vec![];
        let mut should_save = false;
        for (i, pending) in self.pending_sessions.iter().enumerate() {
            if let Ok(session) = pending.rx.try_recv() {
                self.sessions.push(SessionState::new(session, pending.torrent_path.clone()));
                ready.push(i);
                should_save = true;
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
                        ctx.set_visuals(if is_dark { egui::Visuals::dark() } else { egui::Visuals::light() });
                    }
                });
            });
            ui.add_space(8.0);
        });

        // Bottom Status Bar
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let mut total_down_bps = 0u64;
                let mut total_up_bps = 0u64;
                let mut active_count = 0;
                for s in &self.sessions {
                    if s.last_status == "Downloading" {
                        total_down_bps += s.last_bps;
                        active_count += 1;
                    }
                    let context_arc = s.session.context.clone();
                    if let Ok(ctx_guard) = context_arc.try_lock() {
                        if let Ok(swarm) = ctx_guard.peer_swarm.read() {
                            for peer_arc in swarm.values() {
                                if let Ok(peer) = peer_arc.try_lock() {
                                    total_up_bps += peer.rolling_upload_rate as u64;
                                }
                            }
                        }
                    }
                }

                ui.label(format!("Active Torrents: {}", active_count));
                ui.separator();
                ui.label(format!("Total Speed: ↓ {}/s  ↑ {}/s", fmt_bytes(total_down_bps), fmt_bytes(total_up_bps)));
                ui.separator();
                ui.label(format!("Total Peers: {}", self.sessions.iter().map(|s| s.last_peers_connected).sum::<usize>()));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new("✔ Online").color(egui::Color32::from_rgb(46, 204, 113)));
                });
            });
        });

        // Sidebar Filter Panel (Left)
        egui::SidePanel::left("left_sidebar")
            .resizable(false)
            .default_width(150.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("STATUS").strong().size(11.0).color(ui.visuals().weak_text_color()));
                ui.add_space(6.0);

                let mut draw_filter_button = |ui: &mut egui::Ui, filter: SidebarFilter, icon: &str, text: &str| {
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
                    ui.add_sized([300.0, 20.0], egui::TextEdit::singleline(&mut self.torrent_path));
                    if ui.button("📂 Browse File").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Torrent Files", &["torrent"])
                            .pick_file()
                        {
                            self.torrent_path = path.to_string_lossy().to_string();
                        }
                    }
                });

                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    ui.label("Download Directory:");
                    ui.add_sized([300.0, 20.0], egui::TextEdit::singleline(&mut self.download_dir));
                    if ui.button("📂 Browse Directory").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.download_dir = path.to_string_lossy().to_string();
                            self.save_state();
                        }
                    }
                    ui.add_space(10.0);
                    if ui.button(egui::RichText::new("➕ Add Torrent").strong()).clicked() {
                        self.create_session();
                    }
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            egui::ScrollArea::vertical()
                .id_source("sessions_scroll")
                .show(ui, |ui| {
                    let mut delete_indices = vec![];
                    let mut select_idx = None;

                    // Display pending/loading sessions with a spinner for instant feedback
                    for pending in &self.pending_sessions {
                        let name = std::path::Path::new(&pending.torrent_path)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| pending.torrent_path.clone());

                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&name).strong().color(egui::Color32::from_rgb(230, 126, 34)));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.add(egui::Spinner::new());
                                    ui.label("Loading... ");
                                });
                            });
                            ui.horizontal(|ui| {
                                ui.label("Verifying local storage and resolving metadata...");
                            });
                        });
                        ui.add_space(6.0);
                    }

                    let mut should_save_progress = false;
                    for (i, session_state) in self.sessions.iter_mut().enumerate() {
                        if !matches_filter(session_state, self.active_filter) {
                            continue;
                        }

                        let context_arc = session_state.session.context.clone();
                        if let Ok(ctx_guard) = context_arc.try_lock() {
                            let old_progress = session_state.last_progress;
                            session_state.update_fields(&ctx_guard);
                            if session_state.last_progress != old_progress {
                                should_save_progress = true;
                            }
                        }

                        let is_selected = self.selected_session_index == Some(i);

                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                let name = egui::RichText::new(&session_state.last_file_name).strong();
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

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button("🗑 Delete").clicked() {
                                        delete_indices.push(i);
                                    }
                                    ui.add_space(6.0);
                                    if session_state.last_status == "Paused" {
                                        if ui.button("▶ Resume").clicked() {
                                            let _ = session_state.session.resume();
                                        }
                                    } else if session_state.last_status != "Ended" {
                                        if ui.button("⏸ Pause").clicked() {
                                            let _ = session_state.session.pause();
                                        }
                                    }
                                });
                            });

                            ui.horizontal(|ui| {
                                let progress = session_state.last_progress;
                                let bar = egui::ProgressBar::new(progress)
                                    .text(format!("{:.1}%", progress * 100.0))
                                    .animate(progress < 1.0 && session_state.last_status == "Downloading");
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

                    if !delete_indices.is_empty() {
                        for idx in delete_indices.into_iter().rev() {
                            let mut state = self.sessions.remove(idx);
                            let _ = state.session.stop();
                            if self.selected_session_index == Some(idx) {
                                self.selected_session_index = None;
                            } else if let Some(s_idx) = self.selected_session_index {
                                if s_idx > idx {
                                    self.selected_session_index = Some(s_idx - 1);
                                }
                            }
                        }
                        self.save_state();
                    } else if should_save_progress {
                        self.save_state();
                    }
                });
        });
    }
}
