#![windows_subsystem = "windows"]

//! Graphical BitTorrent Client Application
//!
//! A desktop application built using `eframe`/`egui` that utilizes the
//! `bittorrent-rs` library. It supports managing multiple torrent sessions,
//! viewing real-time download and upload progress, and displaying log output.

mod app;
mod state;
mod stream_server;
pub mod theme;
mod ui;

use app::TorrentClientApp;
use eframe::egui;
use std::env;

/// The main entry point for the desktop GUI client application.
fn main() {
    let mut args = env::args_os().skip(1);
    let initial_torrent_path = args.next().and_then(|arg| arg.into_string().ok());
    let initial_download_dir = args.next().and_then(|arg| arg.into_string().ok());

    let (log_tx, log_rx) = std::sync::mpsc::channel::<String>();
    bittorrent_rs::util::set_log_sender(log_tx.clone());

    let icon = std::sync::Arc::new(egui::IconData {
        rgba: include_bytes!("../assets/icon_32.rgba").to_vec(),
        width: 32,
        height: 32,
    });

    let saved_window = state::load_saved_window_state();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([960.0, 620.0])
        .with_title("BitTorrent Desktop Client")
        .with_icon(icon);

    if let Some(ref win) = saved_window {
        if let (Some(w), Some(h)) = (win.width, win.height) {
            if w >= 400.0 && h >= 300.0 {
                viewport = viewport.with_inner_size([w, h]);
            }
        }
        if let (Some(x), Some(y)) = (win.x, win.y) {
            if x >= -5000.0 && y >= -5000.0 && x <= 20000.0 && y <= 20000.0 {
                viewport = viewport.with_position([x, y]);
            }
        }
        if win.maximized == Some(true) {
            viewport = viewport.with_maximized(true);
        }
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let _ = eframe::run_native(
        "BitTorrent Desktop Client",
        options,
        Box::new(move |cc| {
            theme::AppTheme::default().apply(&cc.egui_ctx);
            let mut app = TorrentClientApp::new(log_rx, log_tx);
            if let (Some(torrent_path), Some(download_dir)) =
                (initial_torrent_path, initial_download_dir)
            {
                app.torrent_path = torrent_path;
                app.download_dir = download_dir;
            }
            Ok(Box::new(app))
        }),
    );
    std::process::exit(0);
}
