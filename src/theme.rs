//! The application's colors.
//!
//! Colours are app-owned; craft-ui's [`WidgetPalette`] only applies them to egui's five widget
//! states, which keeps the shared controls consistent with the rest of the window.

use craft_ui::theme::WidgetPalette;
use egui::{Color32, CornerRadius, Stroke, Visuals};

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub background: Color32,
    pub card: Color32,
    pub input: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub green: Color32,
    pub red: Color32,
    pub amber: Color32,
    pub accent: Color32,
    pub button: Color32,
    pub button_hover: Color32,
    pub button_active: Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            background: Color32::from_rgb(18, 20, 24),
            card: Color32::from_rgb(29, 32, 38),
            input: Color32::from_rgb(23, 25, 30),
            border: Color32::from_rgb(66, 72, 84),
            text: Color32::from_rgb(233, 236, 241),
            muted: Color32::from_rgb(148, 156, 169),
            green: Color32::from_rgb(74, 222, 128),
            red: Color32::from_rgb(248, 113, 113),
            amber: Color32::from_rgb(251, 191, 36),
            accent: Color32::from_rgb(96, 165, 250),
            button: Color32::from_rgb(40, 44, 52),
            button_hover: Color32::from_rgb(52, 57, 67),
            button_active: Color32::from_rgb(64, 70, 82),
        }
    } else {
        Palette {
            background: Color32::from_rgb(245, 246, 249),
            card: Color32::from_rgb(255, 255, 255),
            input: Color32::from_rgb(238, 240, 244),
            border: Color32::from_rgb(203, 208, 217),
            text: Color32::from_rgb(28, 32, 38),
            muted: Color32::from_rgb(100, 108, 120),
            green: Color32::from_rgb(21, 128, 61),
            red: Color32::from_rgb(220, 38, 38),
            amber: Color32::from_rgb(180, 83, 9),
            accent: Color32::from_rgb(37, 99, 235),
            button: Color32::from_rgb(235, 238, 243),
            button_hover: Color32::from_rgb(224, 228, 235),
            button_active: Color32::from_rgb(211, 218, 228),
        }
    }
}

pub fn install(ctx: &egui::Context, dark: bool) {
    let palette = palette(dark);
    let theme = if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    let mut visuals = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.background;
    visuals.extreme_bg_color = palette.input;
    visuals.override_text_color = Some(palette.text);
    visuals.hyperlink_color = palette.accent;
    visuals.selection.bg_fill = palette.accent.gamma_multiply(0.30);
    visuals.selection.stroke = Stroke::new(1.0, palette.accent);

    WidgetPalette {
        noninteractive: palette.background,
        inactive: palette.button,
        hovered: palette.button_hover,
        active: palette.button_active,
        open: palette.button,
    }
    .apply_to(
        &mut visuals.widgets,
        CornerRadius::same(6),
        Stroke::new(1.0, palette.text),
    );
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, palette.border);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style_of(theme)).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    ctx.set_style_of(theme, style);
}
