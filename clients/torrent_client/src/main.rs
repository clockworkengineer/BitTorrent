use bittorrent_rs::{TorrentSession, Tracker};
use eframe::egui;
use std::env;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

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

struct SessionState {
    session: TorrentSession,
    last_file_name: String,
    last_progress: f32,
    last_status: String,
    last_peers_connected: usize,
    last_peers_active: usize,
    last_bps: u64,
    last_downloaded: u64,
    last_total: u64,
}

impl SessionState {
    fn new(session: TorrentSession) -> Self {
        let mut state = Self {
            session,
            last_file_name: String::new(),
            last_progress: 0.0,
            last_status: String::new(),
            last_peers_connected: 0,
            last_peers_active: 0,
            last_bps: 0,
            last_downloaded: 0,
            last_total: 0,
        };
        if let Ok(ctx_guard) = state.session.context().lock() {
            state.last_file_name = std::path::Path::new(&ctx_guard.file_name)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| ctx_guard.file_name.clone());
            state.last_progress = ctx_guard.progress_percent() / 100.0;
            state.last_status = format!("{:?}", ctx_guard.status);
            state.last_peers_connected = ctx_guard.peer_swarm.read().unwrap().len();
            state.last_peers_active = ctx_guard.number_of_unchoked_peers();
            state.last_bps = ctx_guard.bytes_per_second() as u64;
            state.last_downloaded = ctx_guard.total_bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed);
            state.last_total = ctx_guard.total_bytes_to_download;
        }
        state
    }
}

struct TorrentClientApp {
    torrent_path: String,
    download_dir: String,
    messages: Vec<String>,
    sessions: Vec<SessionState>,
    pending_sessions: Vec<mpsc::Receiver<TorrentSession>>,
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
        for (i, rx) in self.pending_sessions.iter().enumerate() {
            if let Ok(session) = rx.try_recv() {
                self.sessions.push(SessionState::new(session));
                ready.push(i);
            }
        }
        for i in ready.into_iter().rev() {
            self.pending_sessions.remove(i);
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
                ui.text_edit_singleline(&mut self.download_dir);
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
                            session_state.last_file_name = std::path::Path::new(&ctx_guard.file_name)
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| ctx_guard.file_name.clone());

                            session_state.last_progress = ctx_guard.progress_percent() / 100.0;
                            session_state.last_status = format!("{:?}", ctx_guard.status);
                            session_state.last_peers_connected = ctx_guard.peer_swarm.read().unwrap().len();
                            session_state.last_peers_active = ctx_guard.number_of_unchoked_peers();
                            session_state.last_bps = ctx_guard.bytes_per_second() as u64;
                            session_state.last_downloaded = ctx_guard.total_bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed);
                            session_state.last_total = ctx_guard.total_bytes_to_download;
                        }

                        let file_name = &session_state.last_file_name;
                        let progress = session_state.last_progress;
                        let status = &session_state.last_status;
                        let peers_connected = session_state.last_peers_connected;
                        let peers_active = session_state.last_peers_active;
                        let bps = session_state.last_bps;
                        let downloaded = session_state.last_downloaded;
                        let total = session_state.last_total;

                        ui.group(|ui| {
                            ui.label(egui::RichText::new(file_name).strong());

                            let bar = egui::ProgressBar::new(progress)
                                .text(format!("{:.1}%", progress * 100.0))
                                .animate(progress < 1.0);
                            ui.add(bar);

                            ui.horizontal(|ui| {
                                ui.label(format!("Status: {}", status));
                                ui.separator();
                                ui.label(format!(
                                    "Downloaded: {} / {}",
                                    fmt_bytes(downloaded),
                                    fmt_bytes(total)
                                ));
                                ui.separator();
                                ui.label(format!("Speed: {}/s", fmt_bytes(bps)));
                            });
                            ui.horizontal(|ui| {
                                ui.label(format!("Peers: {} connected", peers_connected));
                                ui.separator();
                                ui.label(format!("{} unchoked (active)", peers_active));
                            });
                        });

                        ui.add_space(4.0);
                    }
                });
        });
    }
}

fn fmt_bytes(bytes: u64) -> String {
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

impl TorrentClientApp {
    fn create_session(&mut self) {
        let torrent_path = self.torrent_path.trim().to_string();
        let download_dir = self.download_dir.trim().to_string();
        if torrent_path.is_empty() || download_dir.is_empty() {
            self.messages
                .push("Please provide both torrent file and download directory.".into());
            return;
        }

        let (session_tx, session_rx) = mpsc::channel::<TorrentSession>();
        let msg_tx = self.log_tx.clone();
        self.pending_sessions.push(session_rx);

        // All blocking network work happens in a background thread so the UI
        // is never frozen and the window appears immediately.
        std::thread::spawn(move || {
            let torrent_path = PathBuf::from(&torrent_path);
            let download_dir = PathBuf::from(&download_dir);
            let session_id = torrent_path.display().to_string();

            let _ = msg_tx.send(format!("[{}] Connecting to tracker…", session_id));
            println!("[{}] Connecting to tracker…", session_id);

            let mut session = match TorrentSession::new(&torrent_path, &download_dir, false) {
                Ok(s) => s,
                Err(e) => {
                    let err_msg = format!("Failed to create session: {}", e);
                    let _ = msg_tx.send(err_msg.clone());
                    eprintln!("{}", err_msg);
                    return;
                }
            };

            if let Err(e) = session.start_download() {
                let err_msg = format!("[{}] Failed to start download: {}", session_id, e);
                let _ = msg_tx.send(err_msg.clone());
                eprintln!("{}", err_msg);
                return;
            }

            let mut tracker = match Tracker::new(session.context()) {
                Ok(t) => t,
                Err(e) => {
                    let err_msg = format!("[{}] Tracker setup failed: {}", session_id, e);
                    let _ = msg_tx.send(err_msg.clone());
                    eprintln!("{}", err_msg);
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
                    let err_msg = format!("[{}] Tracker announce failed: {}", session_id, e);
                    let _ = msg_tx.send(err_msg.clone());
                    eprintln!("{}", err_msg);
                    let _ = session_tx.send(session);
                }
            }
        });
    }
}
