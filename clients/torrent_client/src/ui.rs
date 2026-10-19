//! User Interface Layout and Drawing Components
//!
//! Handles layout structures for sidebar elements, status labels, progress bars,
//! options settings panels, and detail tab views.

use crate::app::TorrentClientApp;
use crate::state::DetailTab;
use bittorrent_rs::internals::TorrentContext;
use eframe::egui;
use torrent_client_shared::fmt_bytes;

impl TorrentClientApp {
    /// Renders the bottom details container panel showing overview, file list, peer swarm, trackers, and logs.
    pub fn draw_details_panel(&mut self, ui: &mut egui::Ui) {
        let Some(selected_idx) = self.selected_session_index else {
            return;
        };
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
            ui.selectable_value(
                &mut self.active_detail_tab,
                DetailTab::Overview,
                "🔍 Overview",
            );
            ui.selectable_value(&mut self.active_detail_tab, DetailTab::Files, "📁 Files");
            ui.selectable_value(&mut self.active_detail_tab, DetailTab::Peers, "👥 Peers");
            ui.selectable_value(&mut self.active_detail_tab, DetailTab::Pieces, "🧩 Pieces");
            ui.selectable_value(
                &mut self.active_detail_tab,
                DetailTab::Trackers,
                "📡 Trackers",
            );
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
            .show(ui, |ui| match self.active_detail_tab {
                DetailTab::Overview => self.draw_overview_tab(
                    ui,
                    &context,
                    &last_file_name,
                    last_downloaded,
                    last_uploaded,
                    last_total,
                ),
                DetailTab::Files => self.draw_files_tab(ui, &context, selected_idx),
                DetailTab::Peers => self.draw_peers_tab(ui, &context),
                DetailTab::Pieces => self.draw_pieces_tab(ui, &context),
                DetailTab::Trackers => self.draw_trackers_tab(ui, &context),
                DetailTab::Logs => self.draw_logs_tab(ui),
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

                    ui.label("Super-Seeding (BEP 16):");
                    let mut is_ss = ctx.is_super_seeding();
                    let label = if is_ss {
                        "Enabled (Initial Seeding)"
                    } else {
                        "Disabled"
                    };
                    if ui.checkbox(&mut is_ss, label).clicked() {
                        drop(ctx);
                        if let Ok(mut ctx_mut) = context.lock() {
                            ctx_mut.set_super_seeding(is_ss);
                        }
                    }
                    ui.end_row();
                });
        } else {
            ui.label("Unable to lock session context.");
        }
    }

    /// Renders a list of the relative paths, lengths, and file offsets of all files packaged inside the torrent.
    pub fn draw_files_tab(
        &mut self,
        ui: &mut egui::Ui,
        context: &std::sync::Mutex<TorrentContext>,
        session_idx: usize,
    ) {
        if let Ok(mut ctx) = context.lock() {
            if ctx.files_to_download.is_empty() {
                ui.label("No files list found (Magnet bootstrapping...)");
                return;
            }

            let mut stream_target: Option<usize> = None;
            let mut priority_changes: Vec<(usize, bittorrent_rs::FilePriority)> = Vec::new();

            egui::Grid::new("files_grid")
                .num_columns(5)
                .spacing([12.0, 6.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Relative Path");
                    ui.strong("Size");
                    ui.strong("Offset");
                    ui.strong("Priority");
                    ui.strong("Action");
                    ui.end_row();

                    for (idx, file) in ctx.files_to_download.iter().enumerate() {
                        ui.label(&file.torrent_path);
                        ui.label(fmt_bytes(file.length));
                        ui.label(fmt_bytes(file.offset));

                        let mut current_priority = file.priority;
                        egui::ComboBox::from_id_source(format!("prio_{}_{}", session_idx, idx))
                            .selected_text(current_priority.as_str())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut current_priority,
                                    bittorrent_rs::FilePriority::Skip,
                                    "skip",
                                );
                                ui.selectable_value(
                                    &mut current_priority,
                                    bittorrent_rs::FilePriority::Low,
                                    "low",
                                );
                                ui.selectable_value(
                                    &mut current_priority,
                                    bittorrent_rs::FilePriority::Normal,
                                    "normal",
                                );
                                ui.selectable_value(
                                    &mut current_priority,
                                    bittorrent_rs::FilePriority::High,
                                    "high",
                                );
                            });

                        if current_priority != file.priority {
                            priority_changes.push((idx, current_priority));
                        }

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

            for (idx, prio) in priority_changes {
                let _ = ctx.set_file_priority(idx, prio);
            }

            if let Some(idx) = stream_target {
                self.start_streaming(session_idx, Some(idx));
            }
        } else {
            ui.label("Unable to lock session context.");
        }
    }

    /// Renders a list of all connected peer swarm members, showing their client identity, IP, port, rates, and progress.
    pub fn draw_peers_tab(&self, ui: &mut egui::Ui, context: &std::sync::Mutex<TorrentContext>) {
        if let Ok(ctx) = context.lock() {
            let peers = torrent_client_shared::extract_peer_infos(&ctx);
            if peers.is_empty() {
                ui.label("No peers connected.");
                return;
            }

            ui.horizontal(|ui| {
                ui.label(format!("Connected Peers: {}", peers.len()));
                ui.separator();
                let total_down: u64 = peers.iter().map(|p| p.down_rate).sum();
                let total_up: u64 = peers.iter().map(|p| p.up_rate).sum();
                ui.colored_label(
                    egui::Color32::from_rgb(46, 204, 113),
                    format!("↓ {}/s", fmt_bytes(total_down)),
                );
                ui.separator();
                ui.colored_label(
                    egui::Color32::from_rgb(52, 152, 219),
                    format!("↑ {}/s", fmt_bytes(total_up)),
                );
            });

            ui.add_space(4.0);

            egui::Grid::new("peers_grid")
                .num_columns(6)
                .spacing([12.0, 6.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Client");
                    ui.strong("IP:Port");
                    ui.strong("Progress");
                    ui.strong("Down Rate");
                    ui.strong("Up Rate");
                    ui.strong("Flags");
                    ui.end_row();

                    for p in &peers {
                        ui.label(&p.client);
                        ui.label(format!("{}:{}", p.ip, p.port));

                        let bar = egui::ProgressBar::new(p.progress)
                            .text(format!("{:.1}%", p.progress * 100.0));
                        ui.add_sized([100.0, 14.0], bar);

                        if p.down_rate > 0 {
                            ui.colored_label(
                                egui::Color32::from_rgb(46, 204, 113),
                                format!("{}/s", fmt_bytes(p.down_rate)),
                            );
                        } else {
                            ui.label("-");
                        }

                        if p.up_rate > 0 {
                            ui.colored_label(
                                egui::Color32::from_rgb(52, 152, 219),
                                format!("{}/s", fmt_bytes(p.up_rate)),
                            );
                        } else {
                            ui.label("-");
                        }

                        let flag_response = ui.label(&p.flags);
                        flag_response.on_hover_text(
                            "Flags Legend:\n\
                             I/i: Am Interested / Not Interested\n\
                             C/c: Am Choking / Not Choking\n\
                             H/h: Peer Interested / Not Interested\n\
                             U/u: Peer Unchoked / Choked\n\
                             E: BEP 10 Extension Protocol\n\
                             F: BEP 6 Fast Extension",
                        );

                        ui.end_row();
                    }
                });
        } else {
            ui.label("Unable to lock session context.");
        }
    }

    /// Renders an interactive piece availability and swarm rarity heatmap canvas.
    pub fn draw_pieces_tab(&self, ui: &mut egui::Ui, context: &std::sync::Mutex<TorrentContext>) {
        if let Ok(ctx) = context.lock() {
            let (local_have, swarm_rarity) = ctx.get_piece_availability();
            let total = ctx.number_of_pieces;
            if total == 0 {
                ui.label("No piece metadata available.");
                return;
            }

            let completed = local_have.iter().filter(|&&h| h).count();
            let pct = (completed as f32 / total as f32) * 100.0;
            let piece_size = ctx.piece_length;

            ui.horizontal(|ui| {
                ui.strong(format!(
                    "Completed: {} / {} pieces ({:.1}%)",
                    completed, total, pct
                ));
                ui.separator();
                ui.label(format!("Piece Size: {}", fmt_bytes(piece_size as u64)));
            });

            ui.add_space(4.0);

            // Legend
            ui.horizontal(|ui| {
                ui.label("Legend:");
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 2.0, egui::Color32::from_rgb(46, 204, 113));
                ui.label("Downloaded");

                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 2.0, egui::Color32::from_rgb(52, 152, 219));
                ui.label("In Swarm");

                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 2.0, egui::Color32::from_rgb(44, 62, 80));
                ui.label("Missing");
            });

            ui.add_space(8.0);

            // Canvas Grid
            let available_width = ui.available_width().max(200.0);
            let block_size = 14.0;
            let gap = 2.0;
            let cols = ((available_width + gap) / (block_size + gap))
                .floor()
                .max(1.0) as usize;
            let rows = total.div_ceil(cols);
            let total_height = rows as f32 * (block_size + gap);

            let (response, painter) = ui.allocate_painter(
                egui::vec2(available_width, total_height),
                egui::Sense::hover(),
            );
            let origin = response.rect.min;
            let hover_pos = response.hover_pos();

            let mut hovered_piece: Option<usize> = None;

            for i in 0..total {
                let col = i % cols;
                let row = i / cols;
                let x = origin.x + col as f32 * (block_size + gap);
                let y = origin.y + row as f32 * (block_size + gap);
                let rect =
                    egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(block_size, block_size));

                let color = if local_have.get(i).copied().unwrap_or(false) {
                    egui::Color32::from_rgb(46, 204, 113) // Emerald green
                } else {
                    let rarity = swarm_rarity.get(i).copied().unwrap_or(0);
                    if rarity == 0 {
                        egui::Color32::from_rgb(44, 62, 80) // Dark slate
                    } else if rarity == 1 {
                        egui::Color32::from_rgb(155, 89, 182) // Purple
                    } else if rarity <= 3 {
                        egui::Color32::from_rgb(52, 152, 219) // Sky Blue
                    } else {
                        egui::Color32::from_rgb(26, 188, 156) // Turquoise
                    }
                };

                painter.rect_filled(rect, 2.0, color);

                if let Some(pos) = hover_pos
                    && rect.contains(pos)
                {
                    hovered_piece = Some(i);
                    painter.rect_stroke(rect, 2.0, egui::Stroke::new(2.0f32, egui::Color32::WHITE));
                }
            }

            if let Some(p_idx) = hovered_piece {
                egui::show_tooltip_at_pointer(
                    ui.ctx(),
                    ui.layer_id(),
                    response.id.with(p_idx),
                    |ui: &mut egui::Ui| {
                        ui.strong(format!("Piece #{}", p_idx));
                        ui.label(format!(
                            "Byte Offset: {}",
                            fmt_bytes(p_idx as u64 * piece_size as u64)
                        ));
                        if local_have.get(p_idx).copied().unwrap_or(false) {
                            ui.colored_label(
                                egui::Color32::from_rgb(46, 204, 113),
                                "✔ Downloaded & Verified",
                            );
                        } else {
                            let rarity = swarm_rarity.get(p_idx).copied().unwrap_or(0);
                            if rarity > 0 {
                                ui.colored_label(
                                    egui::Color32::from_rgb(52, 152, 219),
                                    format!("Available from {} peer(s)", rarity),
                                );
                            } else {
                                ui.colored_label(
                                    egui::Color32::from_rgb(231, 76, 60),
                                    "✖ Missing in swarm",
                                );
                            }
                        }
                    },
                );
            }
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
                    if !self.log_search_query.is_empty()
                        && !message
                            .to_lowercase()
                            .contains(&self.log_search_query.to_lowercase())
                    {
                        continue;
                    }

                    let color = if message.contains("Failed")
                        || message.contains("failed")
                        || message.contains("Error")
                    {
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
