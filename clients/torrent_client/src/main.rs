//! Graphical BitTorrent Client Application
//!
//! A desktop application built using `eframe`/`egui` that utilizes the
//! `bittorrent-rs` library. It supports managing multiple torrent sessions,
//! viewing real-time download and upload progress, and displaying log output.

mod app;
mod state;
mod ui;

use app::TorrentClientApp;
use eframe::egui;
use std::env;

/// Configures visual styling and dark mode options for egui widgets.
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

/// The main entry point for the desktop GUI client application.
fn main() {
    let mut args = env::args_os().skip(1);
    let initial_torrent_path = args.next().and_then(|arg| arg.into_string().ok());
    let initial_download_dir = args.next().and_then(|arg| arg.into_string().ok());

    let (log_tx, log_rx) = std::sync::mpsc::channel::<String>();
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
