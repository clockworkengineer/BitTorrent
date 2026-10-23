//! User Interface Layout and Drawing Components
//!
//! Handles layout structures for sidebar elements, command toolbar, pro data table,
//! media cards view, status labels, progress bars, interactive Add Torrent dialog,
//! settings panels, and detail tab views.

use crate::app::TorrentClientApp;
use crate::state::{DetailTab, LibraryViewMode, SortColumn};
use crate::theme::AppTheme;
use bittorrent_rs::internals::TorrentContext;
use eframe::egui::{self, Color32, Pos2, Rect, Rounding, Stroke, Vec2};
use torrent_client_shared::fmt_bytes;

/// Returns an appropriate emoji icon based on filename extension.
pub fn get_file_icon(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    if lower.ends_with(".mp4")
        || lower.ends_with(".mkv")
        || lower.ends_with(".avi")
        || lower.ends_with(".mov")
        || lower.ends_with(".webm")
        || lower.ends_with(".flv")
    {
        "🎬"
    } else if lower.ends_with(".mp3")
        || lower.ends_with(".flac")
        || lower.ends_with(".wav")
        || lower.ends_with(".ogg")
        || lower.ends_with(".m4a")
        || lower.ends_with(".aac")
    {
        "🎵"
    } else if lower.ends_with(".iso") || lower.ends_with(".img") || lower.ends_with(".dmg") {
        "💿"
    } else if lower.ends_with(".zip")
        || lower.ends_with(".tar")
        || lower.ends_with(".gz")
        || lower.ends_with(".7z")
        || lower.ends_with(".rar")
        || lower.ends_with(".bz2")
    {
        "📦"
    } else if lower.ends_with(".pdf")
        || lower.ends_with(".epub")
        || lower.ends_with(".txt")
        || lower.ends_with(".doc")
        || lower.ends_with(".docx")
    {
        "📄"
    } else if lower.ends_with(".exe")
        || lower.ends_with(".msi")
        || lower.ends_with(".deb")
        || lower.ends_with(".rpm")
    {
        "⚙"
    } else {
        "📄"
    }
}

/// Formats remaining duration into a concise, human-readable ETA string.
pub fn fmt_eta(remaining_bytes: u64, bps: u64) -> String {
    if bps == 0 {
        "∞".to_string()
    } else {
        let seconds = remaining_bytes / bps;
        if seconds == 0 {
            "Done".to_string()
        } else if seconds < 60 {
            format!("{}s", seconds)
        } else if seconds < 3600 {
            format!("{}m {}s", seconds / 60, seconds % 60)
        } else if seconds < 86400 {
            format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
        } else if seconds < 86400 * 30 {
            format!("{}d {}h", seconds / 86400, (seconds % 86400) / 3600)
        } else {
            "∞".to_string()
        }
    }
}

/// Opens the containing folder of a downloaded torrent in the native system file manager.
#[cfg(windows)]
pub fn open_containing_folder(path: &std::path::Path) {
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

#[cfg(not(windows))]
pub fn open_containing_folder(_path: &std::path::Path) {}

impl TorrentClientApp {
    /// Renders the sleek modern top command toolbar replacing static form inputs.
    pub fn draw_command_toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("command_toolbar").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                // Brand Logo & Title
                ui.heading(
                    egui::RichText::new("📥 BitTorrent-rs")
                        .strong()
                        .color(self.settings.theme.accent_color()),
                );
                ui.add_space(10.0);

                // Quick Action Buttons
                if ui
                    .button(egui::RichText::new("➕ Add Torrent").strong())
                    .on_hover_text("Open a .torrent file to inspect and download (Ctrl+O)")
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Torrent Files", &["torrent"])
                        .pick_file()
                {
                    self.open_add_torrent_modal(&path.to_string_lossy());
                }

                if ui
                    .button(egui::RichText::new("🧲 Add Magnet").strong())
                    .on_hover_text("Add a magnet link from clipboard or URL (Ctrl+M)")
                    .clicked()
                {
                    self.show_magnet_dialog = true;
                    self.magnet_input_text.clear();
                }

                ui.separator();

                if ui
                    .button("▶ Resume All")
                    .on_hover_text("Resume all paused torrents")
                    .clicked()
                {
                    self.resume_all();
                }

                if ui
                    .button("⏸ Pause All")
                    .on_hover_text("Pause all actively downloading or seeding torrents")
                    .clicked()
                {
                    self.pause_all();
                }

                if let Some(selected_idx) = self.selected_session_index {
                    if ui
                        .button("🔍 Recheck")
                        .on_hover_text("Re-verify piece hashes on disk for selected torrent")
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
                }

                ui.separator();

                // Turtle Mode Speed Limiter Toggle
                let turtle_active = self.settings.turtle_mode;
                let turtle_text = if turtle_active {
                    egui::RichText::new("🐢 Speed Limit ON")
                        .color(self.settings.theme.warning_color())
                        .strong()
                } else {
                    egui::RichText::new("🐢 Speed Limit OFF")
                };
                if ui
                    .button(turtle_text)
                    .on_hover_text("Toggle speed throttle limits (Turtle Mode)")
                    .clicked()
                {
                    self.toggle_turtle_mode();
                }

                ui.separator();

                // Real-time Library Search Filter
                ui.label("🔍");
                ui.add_sized(
                    [160.0, 20.0],
                    egui::TextEdit::singleline(&mut self.search_filter_query)
                        .hint_text("Filter torrents..."),
                );
                if !self.search_filter_query.is_empty() && ui.button("✖").clicked() {
                    self.search_filter_query.clear();
                }

                // Right aligned controls
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("ℹ About").clicked() {
                        self.show_about_dialog = true;
                    }
                    ui.add_space(4.0);
                    if ui.button("⚙ Settings").clicked() {
                        self.show_settings_dialog = true;
                    }
                    ui.add_space(6.0);

                    // Theme selector
                    let mut current_theme = self.settings.theme;
                    egui::ComboBox::from_id_source("theme_selector_dropdown")
                        .selected_text(format!("🎨 {}", current_theme.name()))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut current_theme,
                                AppTheme::ObsidianDark,
                                AppTheme::ObsidianDark.name(),
                            );
                            ui.selectable_value(
                                &mut current_theme,
                                AppTheme::CatppuccinMocha,
                                AppTheme::CatppuccinMocha.name(),
                            );
                            ui.selectable_value(
                                &mut current_theme,
                                AppTheme::OledBlack,
                                AppTheme::OledBlack.name(),
                            );
                            ui.selectable_value(
                                &mut current_theme,
                                AppTheme::LightModern,
                                AppTheme::LightModern.name(),
                            );
                        });
                    if current_theme != self.settings.theme {
                        self.settings.theme = current_theme;
                        self.settings.theme.apply(ctx);
                        self.save_state();
                    }

                    ui.add_space(6.0);

                    // View Mode Switcher: Table vs Cards
                    let is_table = self.settings.view_mode == LibraryViewMode::Table;
                    let table_btn = if is_table {
                        egui::RichText::new("☵ Table").strong()
                    } else {
                        egui::RichText::new("☵ Table")
                    };
                    if ui.selectable_label(is_table, table_btn).clicked() {
                        self.settings.view_mode = LibraryViewMode::Table;
                        self.save_state();
                    }

                    let is_cards = self.settings.view_mode == LibraryViewMode::Cards;
                    let cards_btn = if is_cards {
                        egui::RichText::new("⊞ Cards").strong()
                    } else {
                        egui::RichText::new("⊞ Cards")
                    };
                    if ui.selectable_label(is_cards, cards_btn).clicked() {
                        self.settings.view_mode = LibraryViewMode::Cards;
                        self.save_state();
                    }
                });
            });
            ui.add_space(6.0);
        });
    }

    /// Renders a prominent drop zone banner whenever files are dragged over the window.
    pub fn draw_drag_and_drop_overlay(&self, ctx: &egui::Context) {
        let is_hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if !is_hovering {
            return;
        }

        egui::Area::new(egui::Id::new("drag_drop_overlay"))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::window(ui.style())
                    .fill(self.settings.theme.panel_color())
                    .stroke(Stroke::new(2.5_f32, self.settings.theme.accent_color()))
                    .rounding(Rounding::same(12.0))
                    .show(ui, |ui| {
                        ui.set_min_size(Vec2::new(380.0, 160.0));
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            ui.heading(
                                egui::RichText::new("📥 Drop .torrent or Magnet Link")
                                    .size(22.0)
                                    .strong()
                                    .color(self.settings.theme.accent_color()),
                            );
                            ui.add_space(8.0);
                            ui.label("Release here to inspect file contents and start download.");
                            ui.add_space(20.0);
                        });
                    });
            });
    }

    /// Renders the interactive "Add Torrent" Inspector dialog modal.
    pub fn draw_add_torrent_dialog(&mut self, ctx: &egui::Context) {
        let Some(ref mut modal) = self.add_torrent_modal else {
            return;
        };

        let mut close_dialog = false;
        let mut add_confirmed = false;
        let mut is_open = true;

        egui::Window::new("➕ Add New Torrent")
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .default_height(480.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut is_open)
            .show(ctx, |ui| {
                ui.add_space(4.0);

                // Torrent Title & Size overview
                ui.horizontal(|ui| {
                    ui.heading(
                        egui::RichText::new(&modal.name)
                            .strong()
                            .color(self.settings.theme.accent_color()),
                    );
                });

                if modal.total_bytes > 0 {
                    ui.label(format!(
                        "Total Size: {}  •  Pieces: {} × {}",
                        fmt_bytes(modal.total_bytes),
                        modal.pieces_count,
                        fmt_bytes(modal.piece_length as u64)
                    ));
                }

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);

                // Save directory selection
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Save Folder:").strong());
                    ui.add_sized(
                        [340.0, 20.0],
                        egui::TextEdit::singleline(&mut modal.download_dir),
                    );
                    if ui.button("Browse...").clicked()
                        && let Some(folder) = rfd::FileDialog::new().pick_folder()
                    {
                        modal.download_dir = folder.to_string_lossy().to_string();
                        modal.available_disk_space = crate::state::get_available_disk_space(
                            std::path::Path::new(&modal.download_dir),
                        );
                    }
                });

                // Disk Capacity Indicator
                let selected_bytes = modal.selected_bytes();
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if let Some(free_bytes) = modal.available_disk_space {
                        let has_enough = free_bytes >= selected_bytes;
                        let status_color = if has_enough {
                            self.settings.theme.success_color()
                        } else {
                            self.settings.theme.error_color()
                        };
                        let ratio = if free_bytes > 0 {
                            (selected_bytes as f32 / free_bytes as f32).min(1.0)
                        } else {
                            1.0
                        };

                        let bar = egui::ProgressBar::new(ratio).text(format!(
                            "Disk Free: {}  |  Download: {}",
                            fmt_bytes(free_bytes),
                            fmt_bytes(selected_bytes)
                        ));
                        ui.add_sized([300.0, 16.0], bar);

                        if has_enough {
                            ui.colored_label(status_color, "✔ Sufficient Space");
                        } else {
                            ui.colored_label(status_color, "⚠ Warning: Low Free Space!");
                        }
                    } else {
                        ui.label(format!("Download Size: {}", fmt_bytes(selected_bytes)));
                    }
                });

                ui.add_space(8.0);
                ui.separator();

                // Interactive File Selection Tree
                if !modal.files.is_empty() {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let sel_count = modal.files.iter().filter(|f| f.selected).count();
                        ui.strong(format!(
                            "Files to Download ({}/{} selected):",
                            sel_count,
                            modal.files.len()
                        ));

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Deselect All").clicked() {
                                for f in &mut modal.files {
                                    f.selected = false;
                                    f.priority = bittorrent_rs::FilePriority::Skip;
                                }
                            }
                            ui.add_space(4.0);
                            if ui.button("Select All").clicked() {
                                for f in &mut modal.files {
                                    f.selected = true;
                                    f.priority = bittorrent_rs::FilePriority::Normal;
                                }
                            }
                        });
                    });

                    ui.add_space(4.0);

                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .id_source("add_torrent_files_scroll")
                        .show(ui, |ui| {
                            egui::Grid::new("add_files_grid")
                                .num_columns(4)
                                .spacing([10.0, 4.0])
                                .striped(true)
                                .show(ui, |ui| {
                                    ui.strong("Download");
                                    ui.strong("Path");
                                    ui.strong("Size");
                                    ui.strong("Priority");
                                    ui.end_row();

                                    for f in &mut modal.files {
                                        let old_selected = f.selected;
                                        ui.checkbox(&mut f.selected, "");
                                        if f.selected != old_selected {
                                            f.priority = if f.selected {
                                                bittorrent_rs::FilePriority::Normal
                                            } else {
                                                bittorrent_rs::FilePriority::Skip
                                            };
                                        }

                                        let icon = get_file_icon(&f.path);
                                        ui.label(format!("{} {}", icon, f.path));
                                        ui.label(fmt_bytes(f.length));

                                        egui::ComboBox::from_id_source(format!(
                                            "modal_prio_{}",
                                            f.index
                                        ))
                                        .selected_text(f.priority.as_str())
                                        .show_ui(ui, |ui| {
                                            if ui
                                                .selectable_value(
                                                    &mut f.priority,
                                                    bittorrent_rs::FilePriority::Skip,
                                                    "skip",
                                                )
                                                .clicked()
                                            {
                                                f.selected = false;
                                            }
                                            if ui
                                                .selectable_value(
                                                    &mut f.priority,
                                                    bittorrent_rs::FilePriority::Normal,
                                                    "normal",
                                                )
                                                .clicked()
                                            {
                                                f.selected = true;
                                            }
                                            if ui
                                                .selectable_value(
                                                    &mut f.priority,
                                                    bittorrent_rs::FilePriority::High,
                                                    "high",
                                                )
                                                .clicked()
                                            {
                                                f.selected = true;
                                            }
                                        });

                                        ui.end_row();
                                    }
                                });
                        });
                } else if modal.is_magnet {
                    ui.add_space(8.0);
                    ui.label("🧲 Magnet Link: Metadata and file hierarchy will be resolved automatically from the DHT/swarm.");
                }

                if let Some(ref err) = modal.error_message {
                    ui.add_space(6.0);
                    ui.colored_label(
                        self.settings.theme.error_color(),
                        format!("⚠ Warning: {}", err),
                    );
                }

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);

                // Options checkboxes
                ui.horizontal(|ui| {
                    ui.checkbox(&mut modal.start_immediately, "Start download immediately");
                    ui.add_space(12.0);
                    ui.checkbox(
                        &mut modal.sequential_download,
                        "Sequential download (stream optimized)",
                    );
                });

                ui.add_space(10.0);

                // Modal Footer Actions
                ui.horizontal(|ui| {
                    if ui
                        .button(
                            egui::RichText::new("🚀 Start Download")
                                .strong()
                                .color(self.settings.theme.accent_color()),
                        )
                        .clicked()
                    {
                        add_confirmed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close_dialog = true;
                    }
                });
            });

        if add_confirmed {
            let path = modal.source_path.clone();
            let dir = modal.download_dir.clone();
            let sequential = modal.sequential_download;
            let start_immediately = modal.start_immediately;

            let file_priorities: Vec<(usize, bittorrent_rs::FilePriority)> = modal
                .files
                .iter()
                .map(|f| {
                    if f.selected {
                        (f.index, f.priority)
                    } else {
                        (f.index, bittorrent_rs::FilePriority::Skip)
                    }
                })
                .collect();

            if self.download_dir.is_empty() {
                self.download_dir = dir.clone();
            }

            self.add_session_with_options(
                path,
                dir,
                None,
                Some(file_priorities),
                sequential,
                start_immediately,
            );
            self.add_torrent_modal = None;
        } else if close_dialog || !is_open {
            self.add_torrent_modal = None;
        }
    }

    /// Renders the Magnet Link text input modal.
    pub fn draw_magnet_input_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_magnet_dialog {
            return;
        }

        let mut is_open = true;
        let mut inspect_clicked = false;
        let mut close_dialog = false;

        egui::Window::new("🧲 Add Magnet Link")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut is_open)
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.add_space(4.0);
                ui.label("Paste a Magnet URL below to inspect its swarm metadata and download:");
                ui.add_space(6.0);

                ui.add_sized(
                    [440.0, 24.0],
                    egui::TextEdit::singleline(&mut self.magnet_input_text)
                        .hint_text("magnet:?xt=urn:btih:..."),
                );

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    let valid_magnet = self.magnet_input_text.trim().starts_with("magnet:?");
                    if ui
                        .add_enabled(
                            valid_magnet,
                            egui::Button::new(
                                egui::RichText::new("Inspect & Add")
                                    .strong()
                                    .color(self.settings.theme.accent_color()),
                            ),
                        )
                        .clicked()
                    {
                        inspect_clicked = true;
                    }

                    if ui.button("Cancel").clicked() {
                        close_dialog = true;
                    }
                });
            });

        if inspect_clicked {
            let magnet = self.magnet_input_text.trim().to_string();
            self.show_magnet_dialog = false;
            self.open_add_torrent_modal(&magnet);
        } else if close_dialog || !is_open {
            self.show_magnet_dialog = false;
        }
    }

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
                            let name_resp = if is_selected {
                                ui.selectable_label(true, name_label)
                            } else {
                                ui.selectable_label(false, name_label)
                            };

                            if name_resp.clicked() {
                                select_idx = Some(session_idx);
                            }

                            // Context menu on row
                            name_resp.context_menu(|ui| {
                                if s.last_status == "Paused" {
                                    if ui.button("▶ Resume").clicked() {
                                        toggle_pause_idx = Some(session_idx);
                                        ui.close_menu();
                                    }
                                } else if ui.button("⏸ Pause").clicked() {
                                    toggle_pause_idx = Some(session_idx);
                                    ui.close_menu();
                                }

                                if ui.button("▶ Stream Media").clicked() {
                                    stream_target = Some(session_idx);
                                    ui.close_menu();
                                }

                                if ui.button("🔍 Force Recheck").clicked() {
                                    recheck_target = Some(session_idx);
                                    ui.close_menu();
                                }

                                ui.separator();

                                if ui.button("📁 Open Folder").clicked() {
                                    if let Ok(ctx) = s.session.context.lock() {
                                        explore_target = Some(ctx.download_path.clone());
                                    }
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

                            let bar = egui::ProgressBar::new(prog)
                                .text(format!("{:.1}%{}", prog * 100.0, eta_str))
                                .animate(prog < 1.0 && s.last_status == "Downloading");
                            ui.add_sized([160.0, 14.0], bar);

                            // Status badge
                            let (badge_color, status_text) = match s.last_status.as_str() {
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
                                if s.last_status == "Paused" {
                                    if ui.button("▶").on_hover_text("Resume").clicked() {
                                        toggle_pause_idx = Some(session_idx);
                                    }
                                } else if s.last_status != "Ended"
                                    && ui.button("⏸").on_hover_text("Pause").clicked()
                                {
                                    toggle_pause_idx = Some(session_idx);
                                }

                                if torrent_client_shared::is_media_filename(&s.last_file_name)
                                    && ui.button("🎬").on_hover_text("Stream media").clicked()
                                {
                                    stream_target = Some(session_idx);
                                }

                                if ui.button("🔍").on_hover_text("Force Recheck").clicked() {
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
                                let name_resp = ui.selectable_label(is_selected, name_text);
                                if name_resp.clicked() {
                                    select_idx = Some(session_idx);
                                }
                                name_resp.on_hover_text(&s.last_file_name);

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let (badge_color, status_text) = match s
                                            .last_status
                                            .as_str()
                                        {
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

                            let progress_text = format!(
                                "{:.1}% ({}/{} downloaded{})",
                                prog * 100.0,
                                fmt_bytes(s.last_downloaded),
                                fmt_bytes(s.last_total),
                                eta_str
                            );

                            let bar = egui::ProgressBar::new(prog)
                                .text(progress_text)
                                .animate(prog < 1.0 && s.last_status == "Downloading");
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

                                        if ui.button("📁 Folder").clicked()
                                            && let Ok(ctx) = s.session.context.lock()
                                        {
                                            explore_target = Some(ctx.download_path.clone());
                                        }

                                        if torrent_client_shared::is_media_filename(
                                            &s.last_file_name,
                                        ) && ui
                                            .button(
                                                egui::RichText::new("🎬 Stream")
                                                    .color(self.settings.theme.stream_color()),
                                            )
                                            .clicked()
                                        {
                                            stream_target = Some(session_idx);
                                        }

                                        if s.last_status == "Paused" {
                                            if ui.button("▶ Resume").clicked() {
                                                toggle_pause_idx = Some(session_idx);
                                            }
                                        } else if s.last_status != "Ended"
                                            && ui.button("⏸ Pause").clicked()
                                        {
                                            toggle_pause_idx = Some(session_idx);
                                        }

                                        if ui
                                            .button("🔍 Recheck")
                                            .on_hover_text("Force piece hash verification")
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

    /// Renders the bottom details container panel showing overview, file list, peer swarm, trackers, and logs.
    pub fn draw_details_panel(&mut self, ui: &mut egui::Ui) {
        let Some(selected_idx) = self.selected_session_index else {
            return;
        };
        if selected_idx >= self.sessions.len() {
            self.selected_session_index = None;
            return;
        }

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
                if ui
                    .button("🔍 Force Recheck")
                    .on_hover_text(
                        "Recheck all piece hashes on disk to detect and repair corrupted data",
                    )
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
                        let icon = get_file_icon(&file.torrent_path);
                        ui.label(format!("{} {}", icon, file.torrent_path));
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
                ui.set_max_width(420.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(4.0);
                    ui.heading(
                        egui::RichText::new("📥 BitTorrent-rs Client")
                            .strong()
                            .size(20.0)
                            .color(self.settings.theme.accent_color()),
                    );
                    ui.label(
                        egui::RichText::new("Version 0.2.0 (Next-Gen Rust Edition)")
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.add_space(4.0);
                });

                ui.separator();
                ui.add_space(6.0);

                ui.label("A state-of-the-art, modular BitTorrent client library and desktop application built in pure Rust.");
                ui.add_space(8.0);

                ui.strong("Protocol & Architecture Highlights:");
                ui.label(" • BitTorrent v1 (SHA-1) & v2 (SHA-256 Merkle trees)");
                ui.label(" • Kademlia DHT (BEP 5) & Local Service Discovery (BEP 14)");
                ui.label(" • Peer Exchange (PEX — BEP 11) with IPv6 support");
                ui.label(" • Message Stream Encryption (MSE / PE) RC4 handshake");
                ui.label(" • uTorrent Transport Protocol (uTP — BEP 29)");
                ui.label(" • Built-in HTTP Range Media Streaming Server");
                ui.label(" • Pure Rust architecture with sub-30MB RAM footprint");

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

    /// Renders an interactive Settings / Preferences dialog window.
    pub fn draw_settings_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_settings_dialog {
            return;
        }

        let mut is_open = self.show_settings_dialog;
        let mut save_clicked = false;

        egui::Window::new("⚙ Preferences & Settings")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut is_open)
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.add_space(4.0);

                ui.group(|ui| {
                    ui.strong("🎨 Appearance & Interface");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Theme:");
                        let mut selected_theme = self.settings.theme;
                        egui::ComboBox::from_id_source("settings_theme_dropdown")
                            .selected_text(selected_theme.name())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut selected_theme,
                                    AppTheme::ObsidianDark,
                                    AppTheme::ObsidianDark.name(),
                                );
                                ui.selectable_value(
                                    &mut selected_theme,
                                    AppTheme::CatppuccinMocha,
                                    AppTheme::CatppuccinMocha.name(),
                                );
                                ui.selectable_value(
                                    &mut selected_theme,
                                    AppTheme::OledBlack,
                                    AppTheme::OledBlack.name(),
                                );
                                ui.selectable_value(
                                    &mut selected_theme,
                                    AppTheme::LightModern,
                                    AppTheme::LightModern.name(),
                                );
                            });
                        if selected_theme != self.settings.theme {
                            self.settings.theme = selected_theme;
                            self.settings.theme.apply(ctx);
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Default View Mode:");
                        ui.selectable_value(
                            &mut self.settings.view_mode,
                            LibraryViewMode::Table,
                            "☵ Table",
                        );
                        ui.selectable_value(
                            &mut self.settings.view_mode,
                            LibraryViewMode::Cards,
                            "⊞ Cards",
                        );
                    });
                });

                ui.add_space(6.0);

                ui.group(|ui| {
                    ui.strong("📁 Storage & Downloads");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Default Folder:");
                        ui.add_sized(
                            [220.0, 18.0],
                            egui::TextEdit::singleline(&mut self.settings.default_download_dir),
                        );
                        if ui.button("Browse").clicked()
                            && let Some(folder) = rfd::FileDialog::new().pick_folder()
                        {
                            self.settings.default_download_dir =
                                folder.to_string_lossy().to_string();
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Max Active Downloads:");
                        ui.add(
                            egui::DragValue::new(&mut self.settings.max_active_downloads)
                                .range(1..=50),
                        );
                    });
                });

                ui.add_space(6.0);

                ui.group(|ui| {
                    ui.strong("🐢 Turtle Mode (Bandwidth Throttling)");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Download Cap (KiB/s):");
                        ui.add(
                            egui::DragValue::new(&mut self.settings.turtle_download_kib)
                                .range(16..=102400),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Upload Cap (KiB/s):");
                        ui.add(
                            egui::DragValue::new(&mut self.settings.turtle_upload_kib)
                                .range(16..=102400),
                        );
                    });
                });

                ui.add_space(6.0);

                ui.group(|ui| {
                    ui.strong("🌐 Network & Protocols");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Listening Port:");
                        ui.add(
                            egui::DragValue::new(&mut self.settings.listen_port)
                                .range(1024..=65535),
                        );
                    });
                    ui.checkbox(&mut self.settings.enable_dht, "Enable Kademlia DHT (BEP 5)");
                    ui.checkbox(
                        &mut self.settings.enable_lsd,
                        "Enable Local Service Discovery (BEP 14)",
                    );
                    ui.checkbox(
                        &mut self.settings.enable_utp,
                        "Enable uTP Transport Protocol (BEP 29)",
                    );
                });

                ui.add_space(6.0);

                ui.group(|ui| {
                    ui.strong("🔒 Privacy & Encryption");
                    ui.add_space(4.0);
                    ui.checkbox(
                        &mut self.settings.enable_mse,
                        "Enable Message Stream Encryption (MSE / PE)",
                    );
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    if ui.button("✔ Save & Apply").clicked() {
                        save_clicked = true;
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_settings_dialog = false;
                    }
                });
            });

        if save_clicked {
            if self.download_dir.is_empty() && !self.settings.default_download_dir.is_empty() {
                self.download_dir = self.settings.default_download_dir.clone();
            }
            self.settings.theme.apply(ctx);
            self.save_state();
            self.show_settings_dialog = false;
        } else if !is_open {
            self.show_settings_dialog = false;
        }
    }

    /// Renders a confirmation dialog before removing a torrent session.
    pub fn draw_delete_confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(del_idx) = self.delete_confirm_idx else {
            return;
        };

        if del_idx >= self.sessions.len() {
            self.delete_confirm_idx = None;
            return;
        }

        let torrent_name = self.sessions[del_idx].last_file_name.clone();
        let mut is_open = true;
        let mut confirmed = false;
        let mut cancelled = false;

        egui::Window::new("🗑 Remove Torrent")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut is_open)
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                ui.add_space(4.0);
                ui.label(format!(
                    "Are you sure you want to remove \"{}\"?",
                    torrent_name
                ));
                ui.add_space(8.0);
                ui.checkbox(
                    &mut self.delete_files_checked,
                    "Also delete downloaded data files from disk",
                );
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    if ui
                        .button(
                            egui::RichText::new("Remove")
                                .color(self.settings.theme.error_color())
                                .strong(),
                        )
                        .clicked()
                    {
                        confirmed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancelled = true;
                    }
                });
            });

        if confirmed {
            let mut state = self.sessions.remove(del_idx);
            let _ = state.session.stop();

            if self.delete_files_checked
                && let Ok(ctx_guard) = state.session.context.lock()
            {
                let download_dir = ctx_guard.download_path.clone();
                for file in &ctx_guard.files_to_download {
                    let file_path = download_dir.join(&file.torrent_path);
                    let _ = std::fs::remove_file(file_path);
                }
            }

            if self.selected_session_index == Some(del_idx) {
                self.selected_session_index = None;
            } else if let Some(s_idx) = self.selected_session_index
                && s_idx > del_idx
            {
                self.selected_session_index = Some(s_idx - 1);
            }
            self.save_state();
            self.delete_confirm_idx = None;
        } else if cancelled || !is_open {
            self.delete_confirm_idx = None;
        }
    }
}

/// Simple helper to URL encode a string for magnet display name.
fn urlencoding_simple(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}
