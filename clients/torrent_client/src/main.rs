use bittorrent_rs::{TorrentSession, Tracker};
use eframe::egui;
use std::env;
use std::path::PathBuf;

fn main() {
    let mut args = env::args_os().skip(1);
    let initial_torrent_path = args.next().and_then(|arg| arg.into_string().ok());
    let initial_download_dir = args.next().and_then(|arg| arg.into_string().ok());

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "BitTorrent Client",
        options,
        Box::new(move |_cc| {
            let mut app = TorrentClientApp::default();
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

#[derive(Default)]
struct TorrentClientApp {
    torrent_path: String,
    download_dir: String,
    messages: Vec<String>,
    session_infos: Vec<(String, String)>,
    sessions: Vec<TorrentSession>,
}

impl eframe::App for TorrentClientApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.heading("BitTorrent Client");
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Torrent file:");
                ui.text_edit_singleline(&mut self.torrent_path);
            });
            ui.horizontal(|ui| {
                ui.label("Download dir:");
                ui.text_edit_singleline(&mut self.download_dir);
            });
            if ui.button("Create Session").clicked() {
                self.create_session();
            }

            ui.separator();
            ui.label("Torrent sessions:");
            for (torrent, status) in &self.session_infos {
                ui.label(format!("{} — {}", torrent, status));
            }

            ui.separator();
            ui.label("Messages:");
            for message in &self.messages {
                ui.label(message);
            }
        });
    }
}

impl TorrentClientApp {
    fn create_session(&mut self) {
        let torrent_path = self.torrent_path.trim();
        let download_dir = self.download_dir.trim();
        if torrent_path.is_empty() || download_dir.is_empty() {
            self.messages
                .push("Please provide both torrent file and download directory.".into());
            return;
        }

        let torrent_path = PathBuf::from(torrent_path);
        let download_dir = PathBuf::from(download_dir);

        match TorrentSession::new(&torrent_path, &download_dir, false) {
            Ok(mut session) => {
                let session_id = torrent_path.display().to_string();
                match session.start_download() {
                    Ok(()) => match Tracker::new(session.context.clone()) {
                        Ok(mut tracker) => match tracker.start_announcing() {
                            Ok(response) => {
                                let peer_count = response.peer_list.len();
                                self.messages.push(format!(
                                    "Tracker announce returned {} peers",
                                    peer_count
                                ));
                                if peer_count == 0 {
                                    self.messages.push(
                                            "No peers were returned from the tracker; download cannot start.".into(),
                                        );
                                    self.session_infos
                                        .push((session_id.clone(), "Waiting for peers".into()));
                                    self.sessions.push(session);
                                } else if let Err(err) =
                                    session.download_from_peers(response.peer_list, None)
                                {
                                    self.messages
                                        .push(format!("Download from peers failed: {}", err));
                                    self.session_infos.push((
                                        session_id.clone(),
                                        format!("Download failed: {}", err),
                                    ));
                                    self.sessions.push(session);
                                } else {
                                    self.session_infos
                                        .push((session_id.clone(), "Downloading".into()));
                                    self.sessions.push(session);
                                    self.messages
                                        .push(format!("Session started: {}", session_id));
                                }
                            }
                            Err(err) => {
                                self.messages
                                    .push(format!("Tracker announce failed: {}", err));
                                self.session_infos.push((
                                    session_id.clone(),
                                    format!("Tracker announce failed: {}", err),
                                ));
                                self.sessions.push(session);
                            }
                        },
                        Err(err) => {
                            self.messages.push(format!("Tracker setup failed: {}", err));
                            self.session_infos.push((
                                session_id.clone(),
                                format!("Tracker setup failed: {}", err),
                            ));
                            self.sessions.push(session);
                        }
                    },
                    Err(err) => {
                        self.messages.push(format!(
                            "Session created but failed to start download: {}",
                            err
                        ));
                    }
                }
            }
            Err(err) => {
                self.messages
                    .push(format!("Failed to create session: {}", err));
            }
        }
    }
}
