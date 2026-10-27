//! Bottom details panel and tab views (Overview, Files, Peers, Pieces, Trackers, Logs).

use super::format::get_file_icon;
use crate::app::TorrentClientApp;
use crate::state::DetailTab;
use bittorrent_rs::internals::TorrentContext;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
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

        let (context, last_file_name, torrent_path, last_downloaded, last_uploaded, last_total) = {
            let session_state = &self.sessions[selected_idx];
            (
                session_state.session.context.clone(),
                session_state.last_file_name.clone(),
                session_state.torrent_path.clone(),
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
                let can_recheck = self
                    .sessions
                    .get(selected_idx)
                    .map(|s| s.last_status != "Ended")
                    .unwrap_or(false);
                if ui
                    .add_enabled(can_recheck, egui::Button::new("🔍 Force Recheck"))
                    .on_hover_text(if can_recheck {
                        "Recheck all piece hashes on disk to detect and repair corrupted data"
                    } else {
                        "Session ended"
                    })
                    .clicked()
                {
                    if let Some(s) = self.sessions.get_mut(selected_idx) {
                        let _ = s.session.recheck_pieces();
                        self.messages.push(format!(
                            "[recheck] Initiated file hash verification for '{}'",
                            s.last_file_name
                        ));
                    }
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
                    &torrent_path,
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

    /// Renders general information about the selected torrent session.
    pub fn draw_overview_tab(
        &mut self,
        ui: &mut egui::Ui,
        context: &std::sync::Mutex<TorrentContext>,
        file_name: &str,
        torrent_path: &str,
        downloaded: u64,
        uploaded: u64,
        total: u64,
    ) {
        let mut switch_to_files = false;
        if let Ok(ctx) = context.lock() {
            egui::Grid::new("overview_grid")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Name:");
                    ui.horizontal(|ui| {
                        ui.strong(file_name);
                        if ui
                            .small_button("📋 Copy")
                            .on_hover_text("Copy torrent name")
                            .clicked()
                        {
                            ui.output_mut(|o| o.copied_text = file_name.to_string());
                        }
                    });
                    ui.end_row();

                    ui.label("Torrent File / Source:");
                    ui.horizontal(|ui| {
                        let display_source = if torrent_path.starts_with("magnet:?") {
                            if torrent_path.len() > 60 {
                                format!("{}...", &torrent_path[..60])
                            } else {
                                torrent_path.to_string()
                            }
                        } else {
                            torrent_path.to_string()
                        };
                        ui.label(&display_source).on_hover_text(torrent_path);
                        if ui
                            .small_button("📋 Copy")
                            .on_hover_text("Copy source path or magnet URL")
                            .clicked()
                        {
                            ui.output_mut(|o| o.copied_text = torrent_path.to_string());
                        }
                        let source_p = std::path::Path::new(torrent_path);
                        if source_p.exists() {
                            if ui
                                .small_button("📂 Open Folder")
                                .on_hover_text("Open folder containing .torrent file")
                                .clicked()
                            {
                                super::format::open_containing_folder(source_p);
                            }
                        }
                    });
                    ui.end_row();

                    let hash_hex = bittorrent_rs::util::info_hash_to_string(&ctx.info_hash);
                    let cached_in_dl = ctx.download_path.join(format!("{}.torrent", hash_hex));
                    let cached_in_app = dirs::data_local_dir().map(|d| {
                        d.join("bittorrent-client")
                            .join("torrents")
                            .join(format!("{}.torrent", hash_hex))
                    });
                    let cached_torrent = if cached_in_dl.exists() {
                        Some(cached_in_dl)
                    } else if cached_in_app.as_ref().map(|p| p.exists()).unwrap_or(false) {
                        cached_in_app
                    } else {
                        None
                    };

                    if let Some(cached_path) = cached_torrent {
                        ui.label("Cached .torrent:");
                        ui.horizontal(|ui| {
                            let p_str = cached_path.to_string_lossy().to_string();
                            ui.label(&p_str).on_hover_text(&p_str);
                            if ui
                                .small_button("📋 Copy")
                                .on_hover_text("Copy cached .torrent file path")
                                .clicked()
                            {
                                ui.output_mut(|o| o.copied_text = p_str);
                            }
                            if ui
                                .small_button("📂 Open Folder")
                                .on_hover_text("Open folder containing cached .torrent file")
                                .clicked()
                            {
                                super::format::open_containing_folder(&cached_path);
                            }
                        });
                        ui.end_row();
                    }

                    ui.label("Save Path:");
                    ui.horizontal(|ui| {
                        let dl_str = ctx.download_path.to_string_lossy().to_string();
                        ui.label(&dl_str);
                        if ui
                            .small_button("📂 Open Folder")
                            .on_hover_text("Open folder in file explorer")
                            .clicked()
                        {
                            super::format::open_containing_folder(&ctx.download_path);
                        }
                        if ui
                            .small_button("📋 Copy")
                            .on_hover_text("Copy save path")
                            .clicked()
                        {
                            ui.output_mut(|o| o.copied_text = dl_str);
                        }
                    });
                    ui.end_row();

                    ui.label("Content Files:");
                    ui.vertical(|ui| {
                        let count = ctx.files_to_download.len();
                        if count == 0 {
                            ui.label("Resolving metadata from swarm...");
                        } else if count == 1 {
                            let fn_str = &ctx.files_to_download[0].torrent_path;
                            let full_path = ctx.download_path.join(fn_str);
                            let full_path_str = full_path.to_string_lossy().to_string();
                            ui.horizontal(|ui| {
                                ui.strong(fn_str);
                                if ui
                                    .small_button("📋 Copy Name")
                                    .on_hover_text("Copy filename")
                                    .clicked()
                                {
                                    ui.output_mut(|o| o.copied_text = fn_str.clone());
                                }
                                if ui
                                    .small_button("📋 Copy Full Path")
                                    .on_hover_text(format!("Copy full path:\n{}", full_path_str))
                                    .clicked()
                                {
                                    ui.output_mut(|o| o.copied_text = full_path_str);
                                }
                                if full_path.exists() {
                                    if ui
                                        .small_button("📂 Reveal")
                                        .on_hover_text("Reveal file in file explorer")
                                        .clicked()
                                    {
                                        super::format::open_containing_folder(&full_path);
                                    }
                                }
                            });
                        } else {
                            ui.horizontal(|ui| {
                                ui.strong(format!("{} files in torrent", count));
                                if ui.small_button("📁 View in Files Tab").clicked() {
                                    switch_to_files = true;
                                }
                                if ui
                                    .small_button("📋 Copy All Names")
                                    .on_hover_text("Copy list of all filenames to clipboard")
                                    .clicked()
                                {
                                    let names = ctx
                                        .files_to_download
                                        .iter()
                                        .map(|f| f.torrent_path.as_str())
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    ui.output_mut(|o| o.copied_text = names);
                                }
                                if ui
                                    .small_button("📋 Copy Full Paths")
                                    .on_hover_text("Copy all full disk paths to clipboard")
                                    .clicked()
                                {
                                    let paths = ctx
                                        .files_to_download
                                        .iter()
                                        .map(|f| {
                                            ctx.download_path
                                                .join(&f.torrent_path)
                                                .to_string_lossy()
                                                .to_string()
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    ui.output_mut(|o| o.copied_text = paths);
                                }
                            });
                            ui.collapsing(format!("Files preview ({})", count), |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(140.0)
                                    .show(ui, |ui| {
                                        for f in &ctx.files_to_download {
                                            let file_full = ctx.download_path.join(&f.torrent_path);
                                            let icon = get_file_icon(&f.torrent_path);
                                            ui.horizontal(|ui| {
                                                ui.label(format!("{} {}", icon, f.torrent_path));
                                                if ui
                                                    .small_button("📋")
                                                    .on_hover_text("Copy filename")
                                                    .clicked()
                                                {
                                                    ui.output_mut(|o| {
                                                        o.copied_text = f.torrent_path.clone()
                                                    });
                                                }
                                                if file_full.exists() {
                                                    if ui
                                                        .small_button("📂")
                                                        .on_hover_text("Reveal in Explorer")
                                                        .clicked()
                                                    {
                                                        super::format::open_containing_folder(
                                                            &file_full,
                                                        );
                                                    }
                                                }
                                            });
                                        }
                                    });
                            });
                        }
                    });
                    ui.end_row();

                    let hash_hex = bittorrent_rs::util::info_hash_to_string(&ctx.info_hash);
                    ui.label("Info Hash:");
                    ui.horizontal(|ui| {
                        ui.label(&hash_hex);
                        if ui
                            .small_button("📋 Copy")
                            .on_hover_text("Copy info hash")
                            .clicked()
                        {
                            ui.output_mut(|o| o.copied_text = hash_hex);
                        }
                    });
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
        if switch_to_files {
            self.active_detail_tab = DetailTab::Files;
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

            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{} files total", ctx.files_to_download.len()))
                        .strong(),
                );
                if ui
                    .small_button("📋 Copy All Relative Paths")
                    .on_hover_text("Copy relative paths of all files to clipboard")
                    .clicked()
                {
                    let names = ctx
                        .files_to_download
                        .iter()
                        .map(|f| f.torrent_path.as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    ui.output_mut(|o| o.copied_text = names);
                }
                if ui
                    .small_button("📋 Copy All Full Paths")
                    .on_hover_text("Copy full absolute paths of all files on disk")
                    .clicked()
                {
                    let paths = ctx
                        .files_to_download
                        .iter()
                        .map(|f| {
                            ctx.download_path
                                .join(&f.torrent_path)
                                .to_string_lossy()
                                .to_string()
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    ui.output_mut(|o| o.copied_text = paths);
                }
            });
            ui.add_space(4.0);

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
                        let icon = get_file_icon(&file.torrent_path);
                        let file_full = ctx.download_path.join(&file.torrent_path);
                        let full_str = file_full.to_string_lossy().to_string();
                        ui.horizontal(|ui| {
                            ui.label(format!("{} {}", icon, file.torrent_path));
                            if ui
                                .small_button("📋")
                                .on_hover_text("Copy relative file path")
                                .clicked()
                            {
                                ui.output_mut(|o| o.copied_text = file.torrent_path.clone());
                            }
                            if ui
                                .small_button("📋 Full")
                                .on_hover_text(format!("Copy full path on disk:\n{}", full_str))
                                .clicked()
                            {
                                ui.output_mut(|o| o.copied_text = full_str.clone());
                            }
                            if file_full.exists() {
                                if ui
                                    .small_button("📂")
                                    .on_hover_text("Reveal file in file explorer")
                                    .clicked()
                                {
                                    super::format::open_containing_folder(&file_full);
                                }
                            }
                        });
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

                        let is_media = torrent_client_shared::is_media_filename(&file.torrent_path);
                        let not_skipped = file.priority != bittorrent_rs::FilePriority::Skip;
                        let can_stream = is_media && not_skipped;
                        if ui
                            .add_enabled(can_stream, egui::Button::new("▶ Stream"))
                            .on_hover_text(if !is_media {
                                "Not a playable media file"
                            } else if !not_skipped {
                                "File is set to Skip"
                            } else {
                                "Stream this file"
                            })
                            .clicked()
                        {
                            stream_target = Some(idx);
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
                    self.settings.theme.success_color(),
                    format!("↓ {}/s", fmt_bytes(total_down)),
                );
                ui.separator();
                ui.colored_label(
                    self.settings.theme.upload_color(),
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
                                self.settings.theme.success_color(),
                                format!("{}/s", fmt_bytes(p.down_rate)),
                            );
                        } else {
                            ui.label("-");
                        }

                        if p.up_rate > 0 {
                            ui.colored_label(
                                self.settings.theme.upload_color(),
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
                let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 12.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 2.0, self.settings.theme.success_color());
                ui.label("Downloaded");

                let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 12.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 2.0, self.settings.theme.accent_color());
                ui.label("In Swarm");

                let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 12.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 2.0, self.settings.theme.surface_color());
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
                Vec2::new(available_width, total_height),
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
                let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(block_size, block_size));

                let color = if local_have.get(i).copied().unwrap_or(false) {
                    self.settings.theme.success_color()
                } else {
                    let rarity = swarm_rarity.get(i).copied().unwrap_or(0);
                    if rarity == 0 {
                        self.settings.theme.surface_color()
                    } else if rarity == 1 {
                        Color32::from_rgb(168, 85, 247) // Purple
                    } else if rarity <= 3 {
                        self.settings.theme.accent_color()
                    } else {
                        Color32::from_rgb(20, 184, 166) // Teal
                    }
                };

                painter.rect_filled(rect, 2.0, color);

                if let Some(pos) = hover_pos
                    && rect.contains(pos)
                {
                    hovered_piece = Some(i);
                    painter.rect_stroke(rect, 2.0, Stroke::new(2.0f32, Color32::WHITE));
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
                                self.settings.theme.success_color(),
                                "✔ Downloaded & Verified",
                            );
                        } else {
                            let rarity = swarm_rarity.get(p_idx).copied().unwrap_or(0);
                            if rarity > 0 {
                                ui.colored_label(
                                    self.settings.theme.accent_color(),
                                    format!("Available from {} peer(s)", rarity),
                                );
                            } else {
                                ui.colored_label(
                                    self.settings.theme.error_color(),
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
            let has_logs = !self.messages.is_empty();
            if ui
                .add_enabled(has_logs, egui::Button::new("Clear Logs"))
                .on_hover_text(if has_logs {
                    "Clear all logged messages"
                } else {
                    "Log buffer is empty"
                })
                .clicked()
            {
                self.messages.clear();
            }
        });

        ui.add_space(4.0);

        let search_query = if self.log_search_query.trim().is_empty() {
            None
        } else {
            Some(self.log_search_query.trim().to_lowercase())
        };

        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for message in &self.messages {
                    if let Some(ref q) = search_query {
                        if !message.to_lowercase().contains(q) {
                            continue;
                        }
                    }

                    let color = if message.contains("Failed")
                        || message.contains("failed")
                        || message.contains("Error")
                    {
                        self.settings.theme.error_color()
                    } else if message.contains("Warning") || message.contains("warning") {
                        self.settings.theme.warning_color()
                    } else {
                        ui.visuals().text_color()
                    };

                    ui.label(egui::RichText::new(message).color(color));
                }
            });
    }
}
