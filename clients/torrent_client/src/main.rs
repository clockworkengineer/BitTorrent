use bittorrent_rs::TorrentSession;
use eframe::egui;
use std::path::PathBuf;

fn main() {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "BitTorrent Client",
        options,
        Box::new(|_cc| Ok(Box::new(TorrentClientApp::default()))),
    )
    .unwrap();
}

#[derive(Default)]
struct TorrentClientApp {
    torrent_path: String,
    download_dir: String,
    messages: Vec<String>,
    added_torrents: Vec<String>,
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
            ui.label("Added torrents:");
            for torrent in &self.added_torrents {
                ui.label(torrent);
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
            Ok(session) => {
                self.added_torrents
                    .push(format!("{}", torrent_path.display()));
                self.messages
                    .push(format!("Session created: {:?}", session.status()));
            }
            Err(err) => {
                self.messages
                    .push(format!("Failed to create session: {}", err));
            }
        }
    }
}
