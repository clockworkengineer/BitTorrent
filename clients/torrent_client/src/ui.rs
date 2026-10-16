//! User Interface Layout and Drawing Components
//!
//! Handles layout structures for sidebar elements, status labels, progress bars,
//! options settings panels, and detail tab views.

use eframe::egui;
use crate::app::TorrentClientApp;
use crate::state::DetailTab;
use torrent_client_shared::fmt_bytes;
use bittorrent_rs::internals::TorrentContext;

impl TorrentClientApp {
    /// Renders the bottom details container panel showing overview, file list, peer swarm, trackers, and logs.
    pub fn draw_details_panel(&mut self, ui: &mut egui::Ui) {
        let Some(selected_idx) = self.selected_session_index else { return; };
        if selected_idx >= self.sessions.len() {
            self.selected_session_index = None;
            return;
        }

        // Extract context and fields to avoid borrowing self.sessions mutably while borrowing self for rendering tabs
        let (context, last_file_name, last_downloaded, last_uploaded, last_total) = {
            let session_state = &self.sessions[selected_idx];
            (
                session_state.session.context.clone(),
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
                    DetailTab::Files => self.draw_files_tab(ui, &context, selected_idx),
                    DetailTab::Peers => self.draw_peers_tab(ui, &context),
                    DetailTab::Trackers => self.draw_trackers_tab(ui, &context),
                    DetailTab::Logs => self.draw_logs_tab(ui),
                }
            });
    }

    /// Renders general information about the selected torrent session, such as info hash, save path, size, etc.
    pub fn draw_overview_tab(
        &self,
        ui: &mut egui::Ui,
        context: &std::sync::Mutex<TorrentContext>,
        file_name: &str,
        downloaded: u64,
        uploaded: u64,
        total: u64,
    ) {
        if let Ok(ctx) = context.lock() {
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

    /// Renders a list of the relative paths, lengths, and file offsets of all files packaged inside the torrent.
    pub fn draw_files_tab(&mut self, ui: &mut egui::Ui, context: &std::sync::Mutex<TorrentContext>, session_idx: usize) {
        if let Ok(ctx) = context.lock() {
            if ctx.files_to_download.is_empty() {
                ui.label("No files list found (Magnet bootstrapping...)");
                return;
            }

            let mut stream_target: Option<usize> = None;

            egui::Grid::new("files_grid")
                .num_columns(4)
                .spacing([12.0, 6.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Relative Path");
                    ui.strong("Size");
                    ui.strong("Offset");
                    ui.strong("Action");
                    ui.end_row();

                    for (idx, file) in ctx.files_to_download.iter().enumerate() {
                        ui.label(&file.torrent_path);
                        ui.label(fmt_bytes(file.length));
                        ui.label(fmt_bytes(file.offset));

                        if torrent_client_shared::is_media_filename(&file.torrent_path) {
                            if ui.button("▶ Stream").clicked() {
                                stream_target = Some(idx);
                            }
                        } else {
                            ui.label("-");
                        }
                        ui.end_row();
                    }
                });

            if let Some(idx) = stream_target {
                self.start_streaming(session_idx, Some(idx));
            }
        } else {
            ui.label("Unable to lock session context.");
        }
    }

    /// Renders a list of all connected peer swarm members, showing their IP, port, upload/download speeds, and choking state flags.
    pub fn draw_peers_tab(&self, ui: &mut egui::Ui, context: &std::sync::Mutex<TorrentContext>) {
        if let Ok(ctx) = context.lock() {
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

    /// Renders the configured announce URLs and tracker endpoints.
    pub fn draw_trackers_tab(&self, ui: &mut egui::Ui, context: &std::sync::Mutex<TorrentContext>) {
        if let Ok(ctx) = context.lock() {
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

    /// Renders logged events and error messages, supporting text filters.
    pub fn draw_logs_tab(&mut self, ui: &mut egui::Ui) {
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

    /// Renders an interactive About dialog window with version info, feature list, and support links.
    pub fn draw_about_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_about_dialog {
            return;
        }

        let mut close_clicked = false;
        let mut is_open = self.show_about_dialog;

        egui::Window::new("ℹ About BitTorrent Client")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut is_open)
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(4.0);
                    ui.heading(egui::RichText::new("📥 BitTorrent Client").strong().size(20.0));
                    ui.label(egui::RichText::new("Version 0.1.1 (Rust Edition)").small().color(ui.visuals().weak_text_color()));
                    ui.add_space(4.0);
                });

                ui.separator();
                ui.add_space(6.0);

                ui.label("A high-performance, modular BitTorrent client library and desktop application implemented in pure Rust.");
                ui.add_space(8.0);

                ui.strong("Protocol & Architecture Features:");
                ui.label(" • BitTorrent v1 (SHA-1) & v2 (SHA-256) Metainfo");
                ui.label(" • Kademlia DHT (BEP 5) & LSD Multicast (BEP 14)");
                ui.label(" • Peer Exchange (PEX — BEP 11) with IPv6");
                ui.label(" • Message Stream Encryption (MSE/PE) via RC4");
                ui.label(" • uTorrent Transport Protocol (uTP — BEP 29)");
                ui.label(" • NAT-PMP Auto Port Mapping (BEP 27 / BEP 52)");
                ui.label(" • WebSeed HTTP Mirroring (BEP 17 / BEP 19)");
                ui.label(" • Clean SOLID Principles System Architecture");

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label("Support Development:");
                    ui.hyperlink_to("☕ Buy Me a Coffee", "https://buymeacoffee.com/roberttizz1");
                });

                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    if ui.button("  Close  ").clicked() {
                        close_clicked = true;
                    }
                });
                ui.add_space(4.0);
            });

        if close_clicked || !is_open {
            self.show_about_dialog = false;
        }
    }
}
