//! Graphical BitTorrent Client Application
//!
//! A desktop application built using `eframe`/`egui` that utilizes the
//! `bittorrent-rs` library. It supports managing multiple torrent sessions,
//! viewing real-time download and upload progress, and displaying log output.

use bittorrent_rs::{TorrentSession, Tracker};
use eframe::egui;
use std::env;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;
use torrent_client_shared::{SessionState, PendingSession, fmt_bytes};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum SidebarFilter {
    All,
    Downloading,
    Seeding,
    Paused,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailTab {
    Overview,
    Files,
    Peers,
    Trackers,
    Logs,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct ClientState {
    download_dir: String,
    torrents: Vec<String>,
}

fn get_config_path() -> std::path::PathBuf {
    if let Some(mut proj_dirs) = dirs::data_local_dir() {
        proj_dirs.push("BitTorrent-rs");
        let _ = std::fs::create_dir_all(&proj_dirs);
        proj_dirs.push("client_state.json");
        proj_dirs
    } else {
        std::path::PathBuf::from("torrent_client_state.json")
    }
}

fn matches_filter(session: &SessionState, filter: SidebarFilter) -> bool {
    match filter {
        SidebarFilter::All => true,
        SidebarFilter::Downloading => session.last_status == "Downloading",
        SidebarFilter::Seeding => session.last_status == "Seeding",
        SidebarFilter::Paused => session.last_status == "Paused",
        SidebarFilter::Completed => session.last_progress >= 1.0 || session.last_status == "Seeding",
    }
}

fn configure_visuals(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_rounding = 8.0.into();
    visuals.menu_rounding = 6.0.into();
    visuals.widgets.noninteractive.rounding = 6.0.into();
    visuals.widgets.inactive.rounding = 4.0.into();
    visuals.widgets.hovered.rounding = 4.0.into();
    visuals.widgets.active.rounding = 4.0.into();
    visuals.widgets.open.rounding = 4.0.into();
    // Modern accent blue
    visuals.selection.bg_fill = egui::Color32::from_rgb(41, 128, 185);
    ctx.set_visuals(visuals);
}

fn main() {
    let mut args = env::args_os().skip(1);
    let initial_torrent_path = args.next().and_then(|arg| arg.into_string().ok());
    let initial_download_dir = args.next().and_then(|arg| arg.into_string().ok());

    let (log_tx, log_rx) = mpsc::channel::<String>();
    bittorrent_rs::util::set_log_sender(log_tx.clone());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_position(egui::pos2(100.0, 100.0)),
        ..Default::default()
    };
    eframe::run_native(
        "BitTorrent Client",
        options,
        Box::new(move |cc| {
            configure_visuals(&cc.egui_ctx);
            let mut app = TorrentClientApp::new(log_rx, log_tx);
            app.load_state();
            if let (Some(torrent_path), Some(download_dir)) =
                (initial_torrent_path, initial_download_dir)
            {
                app.torrent_path = torrent_path;
                app.download_dir = download_dir;
                app.create_session();
            }
            Ok(Box::new(app))
        }),
    )
    .unwrap();
}

struct TorrentClientApp {
    torrent_path: String,
    download_dir: String,
    messages: Vec<String>,
    sessions: Vec<SessionState>,
    pending_sessions: Vec<PendingSession>,
    log_rx: mpsc::Receiver<String>,
    log_tx: mpsc::Sender<String>,
    // New UI state
    selected_session_index: Option<usize>,
    active_filter: SidebarFilter,
    active_detail_tab: DetailTab,
    log_search_query: String,
}

impl TorrentClientApp {
    fn new(log_rx: mpsc::Receiver<String>, log_tx: mpsc::Sender<String>) -> Self {
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
}

impl eframe::App for TorrentClientApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Stop all active sessions so background threads don't outlive the process.
        for state in &mut self.sessions {
            let _ = state.session.stop();
        }
    }

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
                    if let Ok(ctx_guard) = s.session.context().try_lock() {
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

                    for (i, session_state) in self.sessions.iter_mut().enumerate() {
                        if !matches_filter(session_state, self.active_filter) {
                            continue;
                        }

                        if let Ok(ctx_guard) = session_state.session.context().try_lock() {
                            session_state.update_fields(&ctx_guard);
                        }

                        let is_selected = self.selected_session_index == Some(i);

                        let response = ui.group(|ui| {
                            ui.horizontal(|ui| {
                                let name = egui::RichText::new(&session_state.last_file_name).strong();
                                if is_selected {
                                    ui.label(name.color(ui.visuals().selection.bg_fill));
                                } else {
                                    ui.label(name);
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

                        let click_response = response.response.interact(egui::Sense::click());
                        if click_response.clicked() {
                            select_idx = Some(i);
                        }

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
                    }
                });
        });
    }
}

impl TorrentClientApp {
    fn count_torrents_for_filter(&self, filter: SidebarFilter) -> usize {
        self.sessions.iter().filter(|s| matches_filter(s, filter)).count()
    }

    fn draw_details_panel(&mut self, ui: &mut egui::Ui) {
        let Some(selected_idx) = self.selected_session_index else { return; };
        if selected_idx >= self.sessions.len() {
            self.selected_session_index = None;
            return;
        }

        // Extract context and fields to avoid borrowing self.sessions mutably while borrowing self for rendering tabs
        let (context, last_file_name, last_downloaded, last_uploaded, last_total) = {
            let session_state = &self.sessions[selected_idx];
            (
                session_state.session.context(),
                session_state.last_file_name.clone(),
                session_state.last_downloaded,
                session_state.last_uploaded,
                session_state.last_total,
            )
        };

        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.active_detail_tab, DetailTab::Overview, "🔍 Overview");
            ui.selectable_value(&mut self.active_detail_tab, DetailTab::Files, "📁 Files");
            ui.selectable_value(&mut self.active_detail_tab, DetailTab::Peers, "👥 Peers");
            ui.selectable_value(&mut self.active_detail_tab, DetailTab::Trackers, "📡 Trackers");
            ui.selectable_value(&mut self.active_detail_tab, DetailTab::Logs, "📝 Logs");

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("❌ Close Details").clicked() {
                    self.selected_session_index = None;
                }
            });
        });

        ui.separator();

        egui::ScrollArea::vertical()
            .id_source("details_scroll")
            .show(ui, |ui| {
                match self.active_detail_tab {
                    DetailTab::Overview => self.draw_overview_tab(ui, &context, &last_file_name, last_downloaded, last_uploaded, last_total),
                    DetailTab::Files => self.draw_files_tab(ui, &context),
                    DetailTab::Peers => self.draw_peers_tab(ui, &context),
                    DetailTab::Trackers => self.draw_trackers_tab(ui, &context),
                    DetailTab::Logs => self.draw_logs_tab(ui),
                }
            });
    }

    fn draw_overview_tab(
        &self,
        ui: &mut egui::Ui,
        context: &std::sync::Mutex<bittorrent_rs::TorrentContext>,
        file_name: &str,
        downloaded: u64,
        uploaded: u64,
        total: u64,
    ) {
        if let Ok(ctx) = context.try_lock() {
            egui::Grid::new("overview_grid")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Name:");
                    ui.label(file_name);
                    ui.end_row();

                    ui.label("Save Path:");
                    ui.label(ctx.download_path.to_string_lossy().to_string());
                    ui.end_row();

                    let hash_hex = bittorrent_rs::util::info_hash_to_string(&ctx.info_hash);
                    ui.label("Info Hash:");
                    ui.label(hash_hex);
                    ui.end_row();

                    ui.label("Piece Size:");
                    ui.label(fmt_bytes(ctx.piece_length as u64));
                    ui.end_row();

                    ui.label("Total Pieces:");
                    ui.label(ctx.number_of_pieces.to_string());
                    ui.end_row();

                    ui.label("Total Bytes:");
                    ui.label(fmt_bytes(total));
                    ui.end_row();

                    ui.label("Downloaded:");
                    ui.label(fmt_bytes(downloaded));
                    ui.end_row();

                    ui.label("Uploaded:");
                    ui.label(fmt_bytes(uploaded));
                    ui.end_row();

                    let ratio = if downloaded > 0 {
                        uploaded as f64 / downloaded as f64
                    } else {
                        0.0
                    };
                    ui.label("Share Ratio:");
                    ui.label(format!("{:.3}", ratio));
                    ui.end_row();
                });
        } else {
            ui.label("Unable to lock session context.");
        }
    }

    fn draw_files_tab(&self, ui: &mut egui::Ui, context: &std::sync::Mutex<bittorrent_rs::TorrentContext>) {
        if let Ok(ctx) = context.try_lock() {
            if ctx.files_to_download.is_empty() {
                ui.label("No files list found (Magnet bootstrapping...)");
                return;
            }

            egui::Grid::new("files_grid")
                .num_columns(3)
                .spacing([12.0, 6.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Relative Path");
                    ui.strong("Size");
                    ui.strong("Offset");
                    ui.end_row();

                    for file in &ctx.files_to_download {
                        ui.label(&file.torrent_path);
                        ui.label(fmt_bytes(file.length));
                        ui.label(fmt_bytes(file.offset));
                        ui.end_row();
                    }
                });
        } else {
            ui.label("Unable to lock session context.");
        }
    }

    fn draw_peers_tab(&self, ui: &mut egui::Ui, context: &std::sync::Mutex<bittorrent_rs::TorrentContext>) {
        if let Ok(ctx) = context.try_lock() {
            let swarm = ctx.peer_swarm.read().unwrap();
            if swarm.is_empty() {
                ui.label("No peers connected.");
                return;
            }

            egui::Grid::new("peers_grid")
                .num_columns(5)
                .spacing([12.0, 6.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("IP Address");
                    ui.strong("Port");
                    ui.strong("Down Rate");
                    ui.strong("Up Rate");
                    ui.strong("Flags");
                    ui.end_row();

                    for peer_arc in swarm.values() {
                        if let Ok(peer) = peer_arc.try_lock() {
                            ui.label(&peer.ip);
                            ui.label(peer.port.to_string());
                            ui.label(format!("{}/s", fmt_bytes(peer.rolling_download_rate as u64)));
                            ui.label(format!("{}/s", fmt_bytes(peer.rolling_upload_rate as u64)));

                            let mut flags = String::new();
                            if peer.am_interested { flags.push('I'); } else { flags.push('i'); }
                            if peer.am_choking { flags.push('C'); } else { flags.push('c'); }
                            if peer.peer_interested { flags.push('H'); } else { flags.push('h'); }
                            if peer.peer_choking.wait_one(0) { flags.push('U'); } else { flags.push('u'); }

                            ui.label(flags);
                            ui.end_row();
                        }
                    }
                });
        } else {
            ui.label("Unable to lock session context.");
        }
    }

    fn draw_trackers_tab(&self, ui: &mut egui::Ui, context: &std::sync::Mutex<bittorrent_rs::TorrentContext>) {
        if let Ok(ctx) = context.try_lock() {
            egui::Grid::new("trackers_grid")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Tracker URL");
                    ui.strong("Status");
                    ui.end_row();

                    for tracker_url in &ctx.tracker_urls {
                        ui.label(tracker_url);
                        ui.label("Announcing");
                        ui.end_row();
                    }
                });
        } else {
            ui.label("Unable to lock session context.");
        }
    }

    fn draw_logs_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Search:");
            ui.text_edit_singleline(&mut self.log_search_query);
            if ui.button("Clear Logs").clicked() {
                self.messages.clear();
            }
        });

        ui.add_space(4.0);

        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for message in &self.messages {
                    if !self.log_search_query.is_empty() {
                        if !message.to_lowercase().contains(&self.log_search_query.to_lowercase()) {
                            continue;
                        }
                    }

                    let color = if message.contains("Failed") || message.contains("failed") || message.contains("Error") {
                        egui::Color32::from_rgb(231, 76, 60)
                    } else if message.contains("Warning") || message.contains("warning") {
                        egui::Color32::from_rgb(230, 126, 34)
                    } else {
                        ui.visuals().text_color()
                    };

                    ui.label(egui::RichText::new(message).color(color));
                }
            });
    }
}

impl TorrentClientApp {
    fn save_state(&self) {
        let state = ClientState {
            download_dir: self.download_dir.clone(),
            torrents: self.sessions.iter().map(|s| s.torrent_path.clone())
                .chain(self.pending_sessions.iter().map(|s| s.torrent_path.clone()))
                .collect(),
        };
        let state_path = get_config_path();
        if let Ok(content) = serde_json::to_string_pretty(&state) {
            let _ = std::fs::write(state_path, content);
        }
    }

    fn load_state(&mut self) {
        let state_path = get_config_path();
        let content = if state_path.exists() {
            std::fs::read_to_string(state_path).ok()
        } else {
            std::fs::read_to_string("torrent_client_state.txt").ok()
        };

        if let Some(content) = content {
            if let Ok(state) = serde_json::from_str::<ClientState>(&content) {
                self.download_dir = state.download_dir;
                for path in state.torrents {
                    self.add_session_by_path(path, self.download_dir.clone());
                }
            } else {
                let mut lines = content.lines();
                if let Some(dir) = lines.next() {
                    self.download_dir = dir.to_string();
                }
                for line in lines {
                    let path = line.trim().to_string();
                    if !path.is_empty() {
                        self.add_session_by_path(path, self.download_dir.clone());
                    }
                }
            }
        }
    }

    fn create_session(&mut self) {
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

        self.add_session_by_path(torrent_path, download_dir);
    }

    fn add_session_by_path(&mut self, torrent_path: String, download_dir: String) {
        let (session_tx, session_rx) = mpsc::channel::<TorrentSession>();
        let msg_tx = self.log_tx.clone();
        self.pending_sessions.push(PendingSession {
            torrent_path: torrent_path.clone(),
            rx: session_rx,
        });
        self.save_state();

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

            let session_res = if torrent_path.starts_with("magnet:?") {
                TorrentSession::new_magnet(&torrent_path, &download_dir_buf)
            } else {
                TorrentSession::new(&torrent_path_buf, &download_dir_buf, false)
            };

            let mut session = match session_res {
                Ok(s) => s,
                Err(e) => {
                    log_err(format!("Failed to create session: {}", e));
                    return;
                }
            };

            if let Err(e) = session.start_download() {
                log_err(format!("Failed to start download: {}", e));
                return;
            }

            let mut tracker = match Tracker::new(session.context()) {
                Ok(t) => t,
                Err(e) => {
                    log_err(format!("Tracker setup failed: {}", e));
                    let _ = session_tx.send(session);
                    return;
                }
            };

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
                    } else if let Err(e) = session.download_from_peers(response.peer_list) {
                        let msg = format!(
                            "[{}] Download from peers failed: {}",
                            session_id, e
                        );
                        let _ = msg_tx.send(msg.clone());
                        eprintln!("{}", msg);
                    } else {
                        let msg = format!("[{}] Download started.", session_id);
                        let _ = msg_tx.send(msg.clone());
                        println!("{}", msg);
                    }

                    let _reannounce_thread = session.start_reannounce_loop(tracker);
                    let _ = session_tx.send(session);
                }
                Err(e) => {
                    log_err(format!("Tracker announce failed: {}", e));
                    let _ = session_tx.send(session);
                }
            }
        });
    }
}
