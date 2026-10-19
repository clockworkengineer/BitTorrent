//! Graphical BitTorrent Client Application
//!
//! A desktop application built using `eframe`/`egui` that utilizes the
//! `bittorrent-rs` library. It supports managing multiple torrent sessions,
//! viewing real-time download and upload progress, and displaying log output.

mod app;
mod state;
mod stream_server;
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

/// Generates an embedded application icon for the desktop window and taskbar.
fn create_app_icon() -> egui::IconData {
    const SIZE: usize = 64;
    let mut rgba = Vec::with_capacity(SIZE * SIZE * 4);
    let center = 31.5f32;
    let radius = 28.0f32;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist <= radius {
                let t = (y as f32) / (SIZE as f32);
                let r = (25.0 * (1.0 - t) + 41.0 * t) as u8;
                let g = (42.0 * (1.0 - t) + 128.0 * t) as u8;
                let b = (86.0 * (1.0 - t) + 185.0 * t) as u8;

                // Central download arrow shaft
                let in_shaft = (27..=36).contains(&x) && (16..=36).contains(&y);
                // Arrow head
                let in_head = (36..=47).contains(&y) && {
                    let head_width = 18.0 - (y - 36) as f32 * 1.3;
                    dx.abs() <= head_width.max(1.0)
                };
                // Horizontal base line
                let in_base = (20..=43).contains(&x) && (50..=53).contains(&y);

                if in_shaft || in_head || in_base {
                    rgba.extend_from_slice(&[255, 255, 255, 255]);
                } else if dist >= radius - 1.5 {
                    rgba.extend_from_slice(&[52, 152, 219, 255]);
                } else {
                    rgba.extend_from_slice(&[r, g, b, 255]);
                }
            } else if dist < radius + 1.0 {
                let alpha = ((radius + 1.0 - dist) * 255.0).clamp(0.0, 255.0) as u8;
                rgba.extend_from_slice(&[41, 128, 185, alpha]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }

    egui::IconData {
        rgba,
        width: SIZE as u32,
        height: SIZE as u32,
    }
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
            .with_inner_size([1100.0, 750.0])
            .with_min_inner_size([850.0, 520.0])
            .with_title("BitTorrent Desktop Client")
            .with_icon(create_app_icon())
            .with_position(egui::pos2(100.0, 100.0)),
        ..Default::default()
    };
    let _ = eframe::run_native(
        "BitTorrent Desktop Client",
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
    );
    std::process::exit(0);
}
