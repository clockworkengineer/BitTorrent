//! Top command toolbar component.

use crate::app::TorrentClientApp;
use crate::state::LibraryViewMode;
use crate::theme::AppTheme;
use eframe::egui;

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

                let has_paused = self.sessions.iter().any(|s| s.last_status == "Paused");
                if ui
                    .add_enabled(has_paused, egui::Button::new("▶ Resume All"))
                    .on_hover_text(if has_paused {
                        "Resume all paused torrents"
                    } else {
                        "No paused torrents to resume"
                    })
                    .clicked()
                {
                    self.resume_all();
                }

                let has_active = self.sessions.iter().any(|s| {
                    s.last_status == "Downloading"
                        || s.last_status == "Fetching Metadata"
                        || s.last_status == "Seeding"
                });
                if ui
                    .add_enabled(has_active, egui::Button::new("⏸ Pause All"))
                    .on_hover_text(if has_active {
                        "Pause all actively downloading or seeding torrents"
                    } else {
                        "No active torrents to pause"
                    })
                    .clicked()
                {
                    self.pause_all();
                }

                let can_recheck_selected = self
                    .selected_session_index
                    .and_then(|idx| self.sessions.get(idx))
                    .map(|s| s.last_status != "Ended")
                    .unwrap_or(false);
                if ui
                    .add_enabled(can_recheck_selected, egui::Button::new("🔍 Recheck"))
                    .on_hover_text(if can_recheck_selected {
                        "Re-verify piece hashes on disk for selected torrent"
                    } else {
                        "Select an active torrent to recheck"
                    })
                    .clicked()
                {
                    if let Some(selected_idx) = self.selected_session_index {
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
}
