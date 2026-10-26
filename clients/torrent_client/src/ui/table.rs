//! Main table, responsive cards, and sparkline visualization.

use super::format::{fmt_eta, get_file_icon, open_containing_folder, urlencoding_simple};
use crate::app::TorrentClientApp;
use crate::state::SortColumn;
use eframe::egui::{self, Pos2, Rounding, Stroke, Vec2};
use torrent_client_shared::fmt_bytes;

impl TorrentClientApp {
    /// Renders the Pro High-Density Data Table view with sortable columns and contextual actions.
    pub fn draw_pro_table_view(&mut self, ui: &mut egui::Ui, visible_indices: &[usize]) {
        if visible_indices.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(
                    egui::RichText::new("No torrents match the active filter.")
                        .size(16.0)
                        .color(ui.visuals().weak_text_color()),
                );
                ui.add_space(10.0);
                if ui.button("➕ Add Torrent").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Torrent Files", &["torrent"])
                        .pick_file()
                {
                    self.open_add_torrent_modal(&path.to_string_lossy());
                }
            });
            return;
        }

        let mut select_idx = None;
        let mut stream_target = None;
        let mut toggle_pause_idx = None;
        let mut recheck_target = None;
        let mut delete_target_idx = None;
        let mut explore_target: Option<std::path::PathBuf> = None;

        egui::ScrollArea::vertical()
            .id_source("pro_table_scroll")
            .show(ui, |ui| {
                egui::Grid::new("torrents_pro_table_grid")
                    .num_columns(9)
                    .spacing([12.0, 8.0])
                    .striped(true)
                    .show(ui, |ui| {
                        // Header Row with Sort Triggers
                        let mut sort_header = |ui: &mut egui::Ui, col: SortColumn, name: &str| {
                            let arrow = if self.sort_column == col {
                                if self.sort_ascending { " ⬆" } else { " ⬇" }
                            } else {
                                ""
                            };
                            let text = format!("{}{}", name, arrow);
                            if ui.button(egui::RichText::new(text).strong()).clicked() {
                                if self.sort_column == col {
                                    self.sort_ascending = !self.sort_ascending;
                                } else {
                                    self.sort_column = col;
                                    self.sort_ascending = true;
                                }
                            }
                        };

                        ui.strong("#");
                        sort_header(ui, SortColumn::Name, "Name");
                        sort_header(ui, SortColumn::Size, "Size");
                        sort_header(ui, SortColumn::Progress, "Progress");
                        sort_header(ui, SortColumn::Status, "Status");
                        sort_header(ui, SortColumn::DownSpeed, "↓ Speed");
                        sort_header(ui, SortColumn::UpSpeed, "↑ Speed");
                        sort_header(ui, SortColumn::Peers, "Peers");
                        ui.strong("Actions");
                        ui.end_row();

                        for (order_idx, &session_idx) in visible_indices.iter().enumerate() {
                            let s = &self.sessions[session_idx];
                            let is_selected = self.selected_session_index == Some(session_idx);

                            ui.label(format!("{}", order_idx + 1));

                            // Name with icon and click selection
                            let icon = get_file_icon(&s.last_file_name);
                            let name_label =
                                egui::RichText::new(format!("{} {}", icon, s.last_file_name))
                                    .strong();
                            let mut name_resp = if is_selected {
                                ui.selectable_label(true, name_label)
                            } else {
                                ui.selectable_label(false, name_label)
                            };

                            if name_resp.clicked() {
                                select_idx = Some(session_idx);
                            }

                            name_resp = name_resp.on_hover_ui(|ui| {
                                ui.strong(&s.last_file_name);
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Source:").weak());
                                    ui.label(&s.torrent_path);
                                });
                                if let Ok(ctx) = s.session.context.lock() {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("Save Path:").weak());
                                        ui.label(ctx.download_path.to_string_lossy());
                                    });
                                    if !ctx.files_to_download.is_empty() {
                                        ui.add_space(2.0);
                                        ui.label(egui::RichText::new(format!("Content Files ({}):", ctx.files_to_download.len())).strong());
                                        for f in ctx.files_to_download.iter().take(5) {
                                            ui.label(format!("• {} ({})", f.torrent_path, fmt_bytes(f.length)));
                                        }
                                        if ctx.files_to_download.len() > 5 {
                                            ui.label(egui::RichText::new(format!("... and {} more files (see 'Files' tab)", ctx.files_to_download.len() - 5)).weak());
                                        }
                                    }
                                }
                            });

                            // Context menu on row
                            name_resp.context_menu(|ui| {
                                let is_paused = s.last_status == "Paused";
                                let is_ended = s.last_status == "Ended";
                                if is_paused {
                                    if ui
                                        .add_enabled(!is_ended, egui::Button::new("▶ Resume"))
                                        .clicked()
                                    {
                                        toggle_pause_idx = Some(session_idx);
                                        ui.close_menu();
                                    }
                                } else if ui
                                    .add_enabled(!is_ended, egui::Button::new("⏸ Pause"))
                                    .clicked()
                                {
                                    toggle_pause_idx = Some(session_idx);
                                    ui.close_menu();
                                }

                                let can_stream =
                                    torrent_client_shared::is_media_filename(&s.last_file_name);
                                if ui
                                    .add_enabled(can_stream, egui::Button::new("▶ Stream Media"))
                                    .on_hover_text(if can_stream {
                                        "Stream media content"
                                    } else {
                                        "Not a playable media file"
                                    })
                                    .clicked()
                                {
                                    stream_target = Some(session_idx);
                                    ui.close_menu();
                                }

                                let can_recheck = s.last_status != "Ended";
                                if ui
                                    .add_enabled(can_recheck, egui::Button::new("🔍 Force Recheck"))
                                    .on_hover_text(if can_recheck {
                                        "Verify piece hashes on disk"
                                    } else {
                                        "Torrent ended"
                                    })
                                    .clicked()
                                {
                                    recheck_target = Some(session_idx);
                                    ui.close_menu();
                                }

                                ui.separator();

                                let folder_exists = s
                                    .session
                                    .context
                                    .lock()
                                    .map(|ctx| ctx.download_path.exists())
                                    .unwrap_or(false);
                                if ui
                                    .add_enabled(folder_exists, egui::Button::new("📁 Open Folder"))
                                    .on_hover_text(if folder_exists {
                                        "Open containing folder"
                                    } else {
                                        "Download folder not found"
                                    })
                                    .clicked()
                                {
                                    if let Ok(ctx) = s.session.context.lock() {
                                        explore_target = Some(ctx.download_path.clone());
                                    }
                                    ui.close_menu();
                                }

                                if ui.button("📋 Copy Torrent Name").clicked() {
                                    ui.output_mut(|o| o.copied_text = s.last_file_name.clone());
                                    ui.close_menu();
                                }

                                if ui.button("📋 Copy Source Path / Magnet").clicked() {
                                    ui.output_mut(|o| o.copied_text = s.torrent_path.clone());
                                    ui.close_menu();
                                }

                                if ui.button("📋 Copy Magnet URL").clicked() {
                                    if let Ok(ctx) = s.session.context.lock() {
                                        let hex = bittorrent_rs::util::info_hash_to_string(
                                            &ctx.info_hash,
                                        );
                                        let mag = format!(
                                            "magnet:?xt=urn:btih:{}&dn={}",
                                            hex,
                                            urlencoding_simple(&ctx.file_name)
                                        );
                                        ui.output_mut(|o| o.copied_text = mag);
                                    }
                                    ui.close_menu();
                                }

                                if let Ok(ctx) = s.session.context.lock() {
                                    if !ctx.files_to_download.is_empty() {
                                        let label = if ctx.files_to_download.len() == 1 {
                                            "📋 Copy Content Filename"
                                        } else {
                                            "📋 Copy All Content Filenames"
                                        };
                                        if ui.button(label).clicked() {
                                            let names = ctx
                                                .files_to_download
                                                .iter()
                                                .map(|f| f.torrent_path.as_str())
                                                .collect::<Vec<_>>()
                                                .join("\n");
                                            ui.output_mut(|o| o.copied_text = names);
                                            ui.close_menu();
                                        }
                                    }
                                }

                                ui.separator();

                                if ui
                                    .button(
                                        egui::RichText::new("🗑 Delete Torrent")
                                            .color(self.settings.theme.error_color()),
                                    )
                                    .clicked()
                                {
                                    delete_target_idx = Some(session_idx);
                                    ui.close_menu();
                                }
                            });

                            // Size
                            ui.label(fmt_bytes(s.last_total));

                            // Progress bar with ETA
                            let prog = s.last_progress;
                            let remaining = s.last_total.saturating_sub(s.last_downloaded);
                            let eta_str = if s.last_status == "Downloading" && s.last_bps > 0 {
                                format!(" (ETA: {})", fmt_eta(remaining, s.last_bps))
                            } else {
                                String::new()
                            };

                            let is_fetching_metadata = s.last_status == "Fetching Metadata";
                            let is_searching_stalled = is_fetching_metadata
                                && s.last_peers_connected == 0
                                && s.created_at.elapsed() >= std::time::Duration::from_secs(30);

                            let bar_text = if is_fetching_metadata {
                                if s.last_peers_connected == 0 {
                                    if is_searching_stalled {
                                        "No seeders in swarm (searching DHT/trackers...)"
                                            .to_string()
                                    } else {
                                        "Finding peers (DHT/trackers)...".to_string()
                                    }
                                } else {
                                    format!(
                                        "Fetching metadata ({} peer{})...",
                                        s.last_peers_connected,
                                        if s.last_peers_connected == 1 { "" } else { "s" }
                                    )
                                }
                            } else {
                                format!("{:.1}%{}", prog * 100.0, eta_str)
                            };
                            let bar = egui::ProgressBar::new(prog).text(bar_text).animate(
                                is_fetching_metadata
                                    || (prog < 1.0 && s.last_status == "Downloading"),
                            );
                            ui.add_sized([160.0, 14.0], bar);

                            // Status badge
                            let (badge_color, status_text) = match s.last_status.as_str() {
                                "Fetching Metadata" => {
                                    if s.last_peers_connected == 0 {
                                        if is_searching_stalled {
                                            (self.settings.theme.warning_color(), "No Seeds")
                                        } else {
                                            (self.settings.theme.accent_color(), "Finding Peers")
                                        }
                                    } else {
                                        (self.settings.theme.accent_color(), "Metadata")
                                    }
                                }
                                "Downloading" => {
                                    (self.settings.theme.accent_color(), "Downloading")
                                }
                                "Seeding" => (self.settings.theme.success_color(), "Seeding"),
                                "Paused" => (self.settings.theme.warning_color(), "Paused"),
                                "Ended" => (self.settings.theme.success_color(), "Completed"),
                                _ => (ui.visuals().text_color(), s.last_status.as_str()),
                            };
                            ui.colored_label(badge_color, status_text);

                            // Speeds
                            if s.last_bps > 0 {
                                ui.colored_label(
                                    self.settings.theme.success_color(),
                                    format!("↓ {}/s", fmt_bytes(s.last_bps)),
                                );
                            } else {
                                ui.label("-");
                            }

                            if s.last_upload_bps > 0 {
                                ui.colored_label(
                                    self.settings.theme.upload_color(),
                                    format!("↑ {}/s", fmt_bytes(s.last_upload_bps)),
                                );
                            } else {
                                ui.label("-");
                            }

                            // Peers
                            ui.label(format!("{}", s.last_peers_connected));

                            // Action buttons
                            ui.horizontal(|ui| {
                                let is_paused = s.last_status == "Paused";
                                let is_ended = s.last_status == "Ended";
                                let can_toggle_pause = !is_ended;
                                let pause_icon = if is_paused { "▶" } else { "⏸" };
                                let pause_tip = if is_ended {
                                    "Session ended"
                                } else if is_paused {
                                    "Resume"
                                } else {
                                    "Pause"
                                };
                                if ui
                                    .add_enabled(can_toggle_pause, egui::Button::new(pause_icon))
                                    .on_hover_text(pause_tip)
                                    .clicked()
                                {
                                    toggle_pause_idx = Some(session_idx);
                                }

                                let can_stream =
                                    torrent_client_shared::is_media_filename(&s.last_file_name);
                                if ui
                                    .add_enabled(can_stream, egui::Button::new("🎬"))
                                    .on_hover_text(if can_stream {
                                        "Stream media"
                                    } else {
                                        "Not a media file"
                                    })
                                    .clicked()
                                {
                                    stream_target = Some(session_idx);
                                }

                                let can_recheck = s.last_status != "Ended";
                                if ui
                                    .add_enabled(can_recheck, egui::Button::new("🔍"))
                                    .on_hover_text(if can_recheck {
                                        "Force Recheck"
                                    } else {
                                        "Session ended"
                                    })
                                    .clicked()
                                {
                                    recheck_target = Some(session_idx);
                                }

                                if ui.button("🗑").on_hover_text("Delete").clicked() {
                                    delete_target_idx = Some(session_idx);
                                }
                            });

                            ui.end_row();
                        }
                    });
            });

        if let Some(idx) = select_idx {
            self.selected_session_index = Some(idx);
        }
        if let Some(idx) = stream_target {
            self.start_streaming(idx, None);
        }
        if let Some(idx) = toggle_pause_idx
            && let Some(s) = self.sessions.get_mut(idx)
        {
            if s.last_status == "Paused" {
                let _ = s.session.resume();
            } else if s.last_status != "Ended" {
                let _ = s.session.pause();
            }
        }
        if let Some(idx) = recheck_target
            && let Some(s) = self.sessions.get_mut(idx)
        {
            let _ = s.session.recheck_pieces();
            self.messages.push(format!(
                "[recheck] Initiated file hash verification for '{}'",
                s.last_file_name
            ));
        }
        if let Some(idx) = delete_target_idx {
            self.delete_confirm_idx = Some(idx);
            self.delete_files_checked = false;
        }
        if let Some(folder) = explore_target {
            open_containing_folder(&folder);
        }
    }

    /// Renders modern Full-Width Responsive Media Cards (Transmission/Folx style)
    /// stacked vertically, adapting dynamically to any window width from small to 4K maximized.
    pub fn draw_cards_view(&mut self, ui: &mut egui::Ui, visible_indices: &[usize]) {
        if visible_indices.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(
                    egui::RichText::new("No media torrents match the active filter.")
                        .size(16.0)
                        .color(ui.visuals().weak_text_color()),
                );
            });
            return;
        }

        let mut select_idx = None;
        let mut stream_target = None;
        let mut toggle_pause_idx = None;
        let mut recheck_target = None;
        let mut delete_target_idx = None;
        let mut explore_target: Option<std::path::PathBuf> = None;

        egui::ScrollArea::vertical()
            .id_source("cards_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;

                for &session_idx in visible_indices {
                    let s = &self.sessions[session_idx];
                    let is_selected = self.selected_session_index == Some(session_idx);
                    let icon = get_file_icon(&s.last_file_name);

                    let card_stroke = if is_selected {
                        Stroke::new(2.0_f32, self.settings.theme.accent_color())
                    } else {
                        Stroke::new(1.0_f32, self.settings.theme.border_color())
                    };

                    let bg_color = if is_selected {
                        self.settings.theme.surface_color()
                    } else {
                        self.settings.theme.panel_color()
                    };

                    egui::Frame::group(ui.style())
                        .fill(bg_color)
                        .stroke(card_stroke)
                        .rounding(Rounding::same(8.0))
                        .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());

                            // Top Row: Icon, Torrent Name, and Status Badge
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(icon).size(22.0));

                                let name_text =
                                    egui::RichText::new(&s.last_file_name).strong().size(14.0);
                                let mut name_resp = ui.selectable_label(is_selected, name_text);
                                if name_resp.clicked() {
                                    select_idx = Some(session_idx);
                                }
                                name_resp = name_resp.on_hover_ui(|ui| {
                                    ui.strong(&s.last_file_name);
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("Source:").weak());
                                        ui.label(&s.torrent_path);
                                    });
                                    if let Ok(ctx) = s.session.context.lock() {
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new("Save Path:").weak());
                                            ui.label(ctx.download_path.to_string_lossy());
                                        });
                                        if !ctx.files_to_download.is_empty() {
                                            ui.add_space(2.0);
                                            ui.label(egui::RichText::new(format!("Content Files ({}):", ctx.files_to_download.len())).strong());
                                            for f in ctx.files_to_download.iter().take(5) {
                                                ui.label(format!("• {} ({})", f.torrent_path, fmt_bytes(f.length)));
                                            }
                                            if ctx.files_to_download.len() > 5 {
                                                ui.label(egui::RichText::new(format!("... and {} more files (see 'Files' tab)", ctx.files_to_download.len() - 5)).weak());
                                            }
                                        }
                                    }
                                });

                                name_resp.context_menu(|ui| {
                                    let is_paused = s.last_status == "Paused";
                                    let is_ended = s.last_status == "Ended";
                                    if is_paused {
                                        if ui
                                            .add_enabled(!is_ended, egui::Button::new("▶ Resume"))
                                            .clicked()
                                        {
                                            toggle_pause_idx = Some(session_idx);
                                            ui.close_menu();
                                        }
                                    } else if ui
                                        .add_enabled(!is_ended, egui::Button::new("⏸ Pause"))
                                        .clicked()
                                    {
                                        toggle_pause_idx = Some(session_idx);
                                        ui.close_menu();
                                    }

                                    let can_stream =
                                        torrent_client_shared::is_media_filename(&s.last_file_name);
                                    if ui
                                        .add_enabled(can_stream, egui::Button::new("▶ Stream Media"))
                                        .clicked()
                                    {
                                        stream_target = Some(session_idx);
                                        ui.close_menu();
                                    }

                                    let can_recheck = s.last_status != "Ended";
                                    if ui
                                        .add_enabled(can_recheck, egui::Button::new("🔍 Force Recheck"))
                                        .clicked()
                                    {
                                        recheck_target = Some(session_idx);
                                        ui.close_menu();
                                    }

                                    ui.separator();

                                    let folder_exists = s
                                        .session
                                        .context
                                        .lock()
                                        .map(|ctx| ctx.download_path.exists())
                                        .unwrap_or(false);
                                    if ui
                                        .add_enabled(folder_exists, egui::Button::new("📁 Open Folder"))
                                        .clicked()
                                    {
                                        if let Ok(ctx) = s.session.context.lock() {
                                            explore_target = Some(ctx.download_path.clone());
                                        }
                                        ui.close_menu();
                                    }

                                    if ui.button("📋 Copy Torrent Name").clicked() {
                                        ui.output_mut(|o| o.copied_text = s.last_file_name.clone());
                                        ui.close_menu();
                                    }

                                    if ui.button("📋 Copy Source Path / Magnet").clicked() {
                                        ui.output_mut(|o| o.copied_text = s.torrent_path.clone());
                                        ui.close_menu();
                                    }

                                    if ui.button("📋 Copy Magnet URL").clicked() {
                                        if let Ok(ctx) = s.session.context.lock() {
                                            let hex = bittorrent_rs::util::info_hash_to_string(
                                                &ctx.info_hash,
                                            );
                                            let mag = format!(
                                                "magnet:?xt=urn:btih:{}&dn={}",
                                                hex,
                                                urlencoding_simple(&ctx.file_name)
                                            );
                                            ui.output_mut(|o| o.copied_text = mag);
                                        }
                                        ui.close_menu();
                                    }

                                    if let Ok(ctx) = s.session.context.lock() {
                                        if !ctx.files_to_download.is_empty() {
                                            let label = if ctx.files_to_download.len() == 1 {
                                                "📋 Copy Content Filename"
                                            } else {
                                                "📋 Copy All Content Filenames"
                                            };
                                            if ui.button(label).clicked() {
                                                let names = ctx
                                                    .files_to_download
                                                    .iter()
                                                    .map(|f| f.torrent_path.as_str())
                                                    .collect::<Vec<_>>()
                                                    .join("\n");
                                                ui.output_mut(|o| o.copied_text = names);
                                                ui.close_menu();
                                            }
                                        }
                                    }

                                    ui.separator();

                                    if ui
                                        .button(
                                            egui::RichText::new("🗑 Delete Torrent")
                                                .color(self.settings.theme.error_color()),
                                        )
                                        .clicked()
                                    {
                                        delete_target_idx = Some(session_idx);
                                        ui.close_menu();
                                    }
                                });

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let is_searching_stalled = s.last_status == "Fetching Metadata"
                                            && s.last_peers_connected == 0
                                            && s.created_at.elapsed()
                                                >= std::time::Duration::from_secs(30);

                                        let (badge_color, status_text) = match s
                                            .last_status
                                            .as_str()
                                        {
                                            "Fetching Metadata" => {
                                                if s.last_peers_connected == 0 {
                                                    if is_searching_stalled {
                                                        (
                                                            self.settings.theme.warning_color(),
                                                            "No Seeds Available",
                                                        )
                                                    } else {
                                                        (
                                                            self.settings.theme.accent_color(),
                                                            "Finding Peers",
                                                        )
                                                    }
                                                } else {
                                                    (
                                                        self.settings.theme.accent_color(),
                                                        "Fetching Metadata",
                                                    )
                                                }
                                            }
                                            "Downloading" => {
                                                (self.settings.theme.accent_color(), "Downloading")
                                            }
                                            "Seeding" => {
                                                (self.settings.theme.success_color(), "Seeding")
                                            }
                                            "Paused" => {
                                                (self.settings.theme.warning_color(), "Paused")
                                            }
                                            "Ended" => {
                                                (self.settings.theme.success_color(), "Completed")
                                            }
                                            _ => {
                                                (ui.visuals().text_color(), s.last_status.as_str())
                                            }
                                        };
                                        ui.colored_label(
                                            badge_color,
                                            egui::RichText::new(status_text).strong(),
                                        );
                                    },
                                );
                            });

                            ui.add_space(5.0);

                            // Full-width Progress Bar with ETA and downloaded/total metrics
                            let prog = s.last_progress;
                            let remaining = s.last_total.saturating_sub(s.last_downloaded);
                            let eta_str = if s.last_status == "Downloading" && s.last_bps > 0 {
                                format!(" • ETA: {}", fmt_eta(remaining, s.last_bps))
                            } else {
                                String::new()
                            };

                            let is_fetching_metadata = s.last_status == "Fetching Metadata";
                            let is_searching_stalled = is_fetching_metadata
                                && s.last_peers_connected == 0
                                && s.created_at.elapsed() >= std::time::Duration::from_secs(30);

                            let progress_text = if is_fetching_metadata {
                                if s.last_peers_connected == 0 {
                                    if is_searching_stalled {
                                        "Swarm inactive: 0 seeders found across DHT & trackers (waiting for peers...)".to_string()
                                    } else {
                                        "Searching DHT & trackers for peers sharing metadata...".to_string()
                                    }
                                } else {
                                    format!(
                                        "Fetching metadata from {} connected peer{}...",
                                        s.last_peers_connected,
                                        if s.last_peers_connected == 1 { "" } else { "s" }
                                    )
                                }
                            } else {
                                format!(
                                    "{:.1}% ({}/{} downloaded{})",
                                    prog * 100.0,
                                    fmt_bytes(s.last_downloaded),
                                    fmt_bytes(s.last_total),
                                    eta_str
                                )
                            };

                            let bar = egui::ProgressBar::new(prog).text(progress_text).animate(
                                is_fetching_metadata
                                    || (prog < 1.0 && s.last_status == "Downloading"),
                            );
                            ui.add_sized([ui.available_width(), 18.0], bar);

                            ui.add_space(6.0);

                            // Bottom Row: Metrics on Left, Quick Action Buttons on Right
                            ui.horizontal(|ui| {
                                // Left-aligned metrics
                                if s.last_bps > 0 {
                                    ui.colored_label(
                                        self.settings.theme.success_color(),
                                        format!("↓ {}/s", fmt_bytes(s.last_bps)),
                                    );
                                } else {
                                    ui.label("↓ 0 B/s");
                                }
                                ui.separator();
                                if s.last_upload_bps > 0 {
                                    ui.colored_label(
                                        self.settings.theme.upload_color(),
                                        format!("↑ {}/s", fmt_bytes(s.last_upload_bps)),
                                    );
                                } else {
                                    ui.label("↑ 0 B/s");
                                }
                                ui.separator();
                                ui.label(format!("👥 {} peers", s.last_peers_connected));
                                ui.separator();
                                ui.label(format!("📦 {}", fmt_bytes(s.last_total)));

                                // Right-aligned actions
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button("🗑 Delete").clicked() {
                                            delete_target_idx = Some(session_idx);
                                        }

                                        let folder_exists = s
                                            .session
                                            .context
                                            .lock()
                                            .map(|ctx| ctx.download_path.exists())
                                            .unwrap_or(false);
                                        if ui
                                            .add_enabled(
                                                folder_exists,
                                                egui::Button::new("📁 Folder"),
                                            )
                                            .on_hover_text(if folder_exists {
                                                "Open containing folder"
                                            } else {
                                                "Folder not created yet"
                                            })
                                            .clicked()
                                            && let Ok(ctx) = s.session.context.lock()
                                        {
                                            explore_target = Some(ctx.download_path.clone());
                                        }

                                        let can_stream = torrent_client_shared::is_media_filename(
                                            &s.last_file_name,
                                        );
                                        if ui
                                            .add_enabled(
                                                can_stream,
                                                egui::Button::new(
                                                    egui::RichText::new("🎬 Stream")
                                                        .color(self.settings.theme.stream_color()),
                                                ),
                                            )
                                            .on_hover_text(if can_stream {
                                                "Stream media"
                                            } else {
                                                "Not a media file"
                                            })
                                            .clicked()
                                        {
                                            stream_target = Some(session_idx);
                                        }

                                        let is_paused = s.last_status == "Paused";
                                        let is_ended = s.last_status == "Ended";
                                        let can_toggle = !is_ended;
                                        let toggle_label =
                                            if is_paused { "▶ Resume" } else { "⏸ Pause" };
                                        let toggle_tip = if is_ended {
                                            "Session ended"
                                        } else if is_paused {
                                            "Resume download"
                                        } else {
                                            "Pause download"
                                        };
                                        if ui
                                            .add_enabled(
                                                can_toggle,
                                                egui::Button::new(toggle_label),
                                            )
                                            .on_hover_text(toggle_tip)
                                            .clicked()
                                        {
                                            toggle_pause_idx = Some(session_idx);
                                        }

                                        let can_recheck = s.last_status != "Ended";
                                        if ui
                                            .add_enabled(
                                                can_recheck,
                                                egui::Button::new("🔍 Recheck"),
                                            )
                                            .on_hover_text(if can_recheck {
                                                "Force piece hash verification"
                                            } else {
                                                "Session ended"
                                            })
                                            .clicked()
                                        {
                                            recheck_target = Some(session_idx);
                                        }
                                    },
                                );
                            });
                        });
                }
            });

        if let Some(idx) = select_idx {
            self.selected_session_index = Some(idx);
        }
        if let Some(idx) = stream_target {
            self.start_streaming(idx, None);
        }
        if let Some(idx) = toggle_pause_idx
            && let Some(s) = self.sessions.get_mut(idx)
        {
            if s.last_status == "Paused" {
                let _ = s.session.resume();
            } else if s.last_status != "Ended" {
                let _ = s.session.pause();
            }
        }
        if let Some(idx) = recheck_target
            && let Some(s) = self.sessions.get_mut(idx)
        {
            let _ = s.session.recheck_pieces();
            self.messages.push(format!(
                "[recheck] Initiated file hash verification for '{}'",
                s.last_file_name
            ));
        }
        if let Some(idx) = delete_target_idx {
            self.delete_confirm_idx = Some(idx);
            self.delete_files_checked = false;
        }
        if let Some(folder) = explore_target {
            open_containing_folder(&folder);
        }
    }

    /// Renders a live visual sparkline showing rolling 60-second download and upload activity.
    pub fn draw_sparkline(&self, ui: &mut egui::Ui, width: f32, height: f32) {
        let (response, painter) =
            ui.allocate_painter(Vec2::new(width, height), egui::Sense::hover());
        let rect = response.rect;

        // Background box
        painter.rect_filled(
            rect,
            Rounding::same(3.0),
            self.settings.theme.surface_color(),
        );

        let samples = &self.rolling_metrics.samples;
        if samples.len() < 2 {
            return;
        }

        let max_val = samples
            .iter()
            .map(|s| s.1.max(s.2))
            .max()
            .unwrap_or(1024)
            .max(1024) as f32;

        let num_points = samples.len();
        let step_x = rect.width() / (num_points - 1) as f32;

        let mut down_points = Vec::with_capacity(num_points);
        let mut up_points = Vec::with_capacity(num_points);

        for (idx, sample) in samples.iter().enumerate() {
            let x = rect.min.x + idx as f32 * step_x;

            let down_norm = (sample.1 as f32 / max_val).clamp(0.0, 1.0);
            let down_y = rect.max.y - down_norm * (rect.height() - 4.0) - 2.0;
            down_points.push(Pos2::new(x, down_y));

            let up_norm = (sample.2 as f32 / max_val).clamp(0.0, 1.0);
            let up_y = rect.max.y - up_norm * (rect.height() - 4.0) - 2.0;
            up_points.push(Pos2::new(x, up_y));
        }

        // Draw download curve (green)
        for w in down_points.windows(2) {
            painter.line_segment(
                [w[0], w[1]],
                Stroke::new(1.8_f32, self.settings.theme.success_color()),
            );
        }

        // Draw upload curve (blue)
        for w in up_points.windows(2) {
            painter.line_segment(
                [w[0], w[1]],
                Stroke::new(1.4_f32, self.settings.theme.upload_color()),
            );
        }
    }
}
