use crate::app::TorrentClientApp;
use crate::state::LibraryViewMode;
use crate::theme::AppTheme;
use crate::ui::format::get_file_icon;
use eframe::egui::{self, Rounding, Stroke, Vec2};
use torrent_client_shared::fmt_bytes;

impl TorrentClientApp {
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
                            let can_deselect = sel_count > 0;
                            if ui
                                .add_enabled(can_deselect, egui::Button::new("Deselect All"))
                                .on_hover_text(if can_deselect {
                                    "Deselect all files"
                                } else {
                                    "All files already deselected"
                                })
                                .clicked()
                            {
                                for f in &mut modal.files {
                                    f.selected = false;
                                    f.priority = bittorrent_rs::FilePriority::Skip;
                                }
                            }
                            ui.add_space(4.0);
                            let can_select_all = sel_count < modal.files.len();
                            if ui
                                .add_enabled(can_select_all, egui::Button::new("Select All"))
                                .on_hover_text(if can_select_all {
                                    "Select all files"
                                } else {
                                    "All files already selected"
                                })
                                .clicked()
                            {
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
                    let can_start = !modal.source_path.trim().is_empty()
                        && (modal.files.is_empty() || modal.files.iter().any(|f| f.selected));
                    if ui
                        .add_enabled(
                            can_start,
                            egui::Button::new(
                                egui::RichText::new("🚀 Start Download")
                                    .strong()
                                    .color(self.settings.theme.accent_color()),
                            ),
                        )
                        .on_hover_text(if can_start {
                            "Begin downloading this torrent"
                        } else {
                            "Please select at least one file to download"
                        })
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
                        egui::RichText::new("Version 0.3.0 (Next-Gen Rust Edition)")
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
