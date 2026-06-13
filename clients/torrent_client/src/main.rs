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

fn main() {
    let mut args = env::args_os().skip(1);
    let initial_torrent_path = args.next().and_then(|arg| arg.into_string().ok());
    let initial_download_dir = args.next().and_then(|arg| arg.into_string().ok());

    let (log_tx, log_rx) = mpsc::channel::<String>();
    bittorrent_rs::util::set_log_sender(log_tx.clone());

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([700.0, 500.0])
            .with_position(egui::pos2(100.0, 100.0)),
        ..Default::default()
    };
    eframe::run_native(
        "BitTorrent Client",
        options,
        Box::new(move |_cc| {
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
        }
    }
}

impl eframe::App for TorrentClientApp {
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

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.heading("BitTorrent Client");
        });

        if !self.messages.is_empty() {
            egui::TopBottomPanel::bottom("log_panel")
                .resizable(true)
                .default_height(120.0)
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new("Log:").strong());
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for message in &self.messages {
                                ui.label(message);
                            }
                        });
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Torrent file:");
                ui.text_edit_singleline(&mut self.torrent_path);
            });
            ui.horizontal(|ui| {
                ui.label("Download dir:");
                if ui.text_edit_singleline(&mut self.download_dir).changed() {
                    self.save_state();
                }
            });
            if ui.button("Add Session").clicked() {
                self.create_session();
            }

            ui.separator();

            egui::ScrollArea::vertical()
                .id_source("sessions_scroll")
                .show(ui, |ui| {
                    for session_state in &mut self.sessions {
                        if let Ok(ctx_guard) = session_state.session.context().try_lock() {
                            session_state.update_fields(&ctx_guard);
                        }

                        let progress = session_state.last_progress;

                        ui.group(|ui| {
                            ui.label(egui::RichText::new(&session_state.last_file_name).strong());

                            let bar = egui::ProgressBar::new(progress)
                                .text(format!("{:.1}%", progress * 100.0))
                                .animate(progress < 1.0);
                            ui.add(bar);

                            ui.horizontal(|ui| {
                                ui.label(format!("Status: {}", session_state.last_status));
                                ui.separator();
                                ui.label(format!(
                                    "Downloaded: {} / {}",
                                    fmt_bytes(session_state.last_downloaded),
                                    fmt_bytes(session_state.last_total)
                                ));
                                ui.separator();
                                ui.label(format!("Uploaded: {}", fmt_bytes(session_state.last_uploaded)));
                                ui.separator();
                                ui.label(format!("Speed: {}/s", fmt_bytes(session_state.last_bps)));
                            });
                            ui.horizontal(|ui| {
                                ui.label(format!("Peers: {} connected", session_state.last_peers_connected));
                                ui.separator();
                                ui.label(format!("{} unchoked (active)", session_state.last_peers_active));
                            });
                        });

                        ui.add_space(4.0);
                    }
                });
        });
    }
}


impl TorrentClientApp {
    fn save_state(&self) {
        let state_path = "torrent_client_state.txt";
        let mut content = String::new();
        content.push_str(&self.download_dir);
        content.push('\n');
        for session in &self.sessions {
            content.push_str(&session.torrent_path);
            content.push('\n');
        }
        for pending in &self.pending_sessions {
            content.push_str(&pending.torrent_path);
            content.push('\n');
        }
        if let Err(e) = std::fs::write(state_path, content) {
            eprintln!("Failed to save client state: {}", e);
        }
    }

    fn load_state(&mut self) {
        let state_path = "torrent_client_state.txt";
        if let Ok(content) = std::fs::read_to_string(state_path) {
            let mut lines = content.lines();
            if let Some(dir) = lines.next() {
                self.download_dir = dir.to_string();
            }
            let mut paths_to_load = Vec::new();
            for line in lines {
                let path = line.trim().to_string();
                if !path.is_empty() {
                    paths_to_load.push(path);
                }
            }
            for path in paths_to_load {
                self.add_session_by_path(path, self.download_dir.clone());
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

        // All blocking network work happens in a background thread so the UI
        // is never frozen and the window appears immediately.
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

            // Helper to log errors and send them to the GUI messages panel
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

                    // Start the re-announce loop thread (runs in background)
                    let _reannounce_thread = session.start_reannounce_loop(tracker);

                    // Send the session to the GUI thread immediately so it shows up in the UI
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
