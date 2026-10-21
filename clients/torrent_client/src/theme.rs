//! Design System and Theme Engine for BitTorrent Client
//!
//! Provides curated color palettes, visual tokens, and custom styling rules
//! for eframe/egui, including Obsidian Dark, Catppuccin Mocha, OLED Black, and Light Modern.

use eframe::egui::{self, Color32, Rounding, Stroke, Vec2};
use serde::{Deserialize, Serialize};

/// Supported visual theme variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AppTheme {
    #[default]
    ObsidianDark,
    CatppuccinMocha,
    OledBlack,
    LightModern,
}

impl AppTheme {
    /// Returns the human-readable display name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::ObsidianDark => "Obsidian Dark",
            Self::CatppuccinMocha => "Catppuccin Mocha",
            Self::OledBlack => "OLED Pure Black",
            Self::LightModern => "Light Modern",
        }
    }

    /// Primary accent color (buttons, highlights, active states).
    pub fn accent_color(&self) -> Color32 {
        match self {
            Self::ObsidianDark => Color32::from_rgb(88, 166, 255), // GitHub Blue
            Self::CatppuccinMocha => Color32::from_rgb(137, 180, 250), // Catppuccin Blue
            Self::OledBlack => Color32::from_rgb(0, 229, 255),    // Electric Cyan
            Self::LightModern => Color32::from_rgb(9, 105, 218),  // Azure
        }
    }

    /// Background color for central application panels.
    pub fn background_color(&self) -> Color32 {
        match self {
            Self::ObsidianDark => Color32::from_rgb(13, 17, 23),
            Self::CatppuccinMocha => Color32::from_rgb(30, 30, 46),
            Self::OledBlack => Color32::from_rgb(0, 0, 0),
            Self::LightModern => Color32::from_rgb(246, 248, 250),
        }
    }

    /// Background color for elevated containers, sidebar panels, and cards.
    pub fn panel_color(&self) -> Color32 {
        match self {
            Self::ObsidianDark => Color32::from_rgb(22, 27, 34),
            Self::CatppuccinMocha => Color32::from_rgb(24, 24, 37),
            Self::OledBlack => Color32::from_rgb(14, 14, 14),
            Self::LightModern => Color32::from_rgb(255, 255, 255),
        }
    }

    /// Subtle background color for alternating table rows and inputs.
    pub fn surface_color(&self) -> Color32 {
        match self {
            Self::ObsidianDark => Color32::from_rgb(33, 38, 45),
            Self::CatppuccinMocha => Color32::from_rgb(49, 50, 68),
            Self::OledBlack => Color32::from_rgb(26, 26, 26),
            Self::LightModern => Color32::from_rgb(240, 242, 245),
        }
    }

    /// Border stroke color for panels, dialogs, and dividers.
    pub fn border_color(&self) -> Color32 {
        match self {
            Self::ObsidianDark => Color32::from_rgb(48, 54, 61),
            Self::CatppuccinMocha => Color32::from_rgb(69, 71, 90),
            Self::OledBlack => Color32::from_rgb(38, 38, 38),
            Self::LightModern => Color32::from_rgb(208, 215, 222),
        }
    }

    /// Color indicating download activity or successful completion.
    pub fn success_color(&self) -> Color32 {
        match self {
            Self::ObsidianDark => Color32::from_rgb(46, 160, 67),
            Self::CatppuccinMocha => Color32::from_rgb(166, 227, 161),
            Self::OledBlack => Color32::from_rgb(0, 230, 118),
            Self::LightModern => Color32::from_rgb(26, 127, 55),
        }
    }

    /// Color indicating seeding or upload activity.
    pub fn upload_color(&self) -> Color32 {
        match self {
            Self::ObsidianDark => Color32::from_rgb(56, 189, 248),
            Self::CatppuccinMocha => Color32::from_rgb(137, 220, 235),
            Self::OledBlack => Color32::from_rgb(0, 176, 255),
            Self::LightModern => Color32::from_rgb(2, 132, 199),
        }
    }

    /// Color indicating warnings, paused state, or verification in progress.
    pub fn warning_color(&self) -> Color32 {
        match self {
            Self::ObsidianDark => Color32::from_rgb(210, 153, 34),
            Self::CatppuccinMocha => Color32::from_rgb(250, 179, 135),
            Self::OledBlack => Color32::from_rgb(255, 171, 0),
            Self::LightModern => Color32::from_rgb(154, 103, 0),
        }
    }

    /// Color indicating errors or stop states.
    pub fn error_color(&self) -> Color32 {
        match self {
            Self::ObsidianDark => Color32::from_rgb(248, 81, 73),
            Self::CatppuccinMocha => Color32::from_rgb(243, 139, 168),
            Self::OledBlack => Color32::from_rgb(255, 23, 68),
            Self::LightModern => Color32::from_rgb(207, 34, 46),
        }
    }

    /// Color for streaming badges and indicators.
    pub fn stream_color(&self) -> Color32 {
        Color32::from_rgb(168, 85, 247) // Vibrant Purple
    }

    /// Applies this theme configuration to the egui Context.
    pub fn apply(&self, ctx: &egui::Context) {
        let is_dark = *self != Self::LightModern;
        let mut visuals = if is_dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };

        // Panel and window background fills
        visuals.panel_fill = self.panel_color();
        visuals.window_fill = self.panel_color();
        visuals.faint_bg_color = self.surface_color();
        visuals.extreme_bg_color = self.background_color();

        // Window & widget rounding
        visuals.window_rounding = Rounding::same(8.0);
        visuals.menu_rounding = Rounding::same(6.0);

        // Borders and strokes
        visuals.window_stroke = Stroke::new(1.0_f32, self.border_color());

        // Inactive widgets (buttons, inputs when idle)
        visuals.widgets.inactive.bg_fill = self.surface_color();
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, self.border_color());
        visuals.widgets.inactive.rounding = Rounding::same(5.0);

        // Hovered widgets
        visuals.widgets.hovered.bg_fill = self.surface_color();
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.5_f32, self.accent_color());
        visuals.widgets.hovered.rounding = Rounding::same(5.0);

        // Active / Pressed widgets
        visuals.widgets.active.bg_fill = self.accent_color();
        visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, self.accent_color());
        visuals.widgets.active.rounding = Rounding::same(5.0);
        if is_dark {
            visuals.widgets.active.fg_stroke = Stroke::new(1.5_f32, Color32::WHITE);
        }

        // Selected items
        visuals.selection.bg_fill = self.accent_color();
        if is_dark {
            visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
        }

        // Non-interactive elements
        visuals.widgets.noninteractive.bg_fill = self.panel_color();
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, self.border_color());
        visuals.widgets.noninteractive.rounding = Rounding::same(5.0);

        ctx.set_visuals(visuals);

        // Tweak spacing for a clean, modern aesthetic
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(8.0, 5.0);
        style.spacing.indent = 16.0;
        ctx.set_style(style);
    }
}
