use std::{fs, path::Path};

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke,
    Vec2,
};

/// Accent orange, reserved for primary buttons, links, and search hits. The
/// chrome itself stays monochrome: selection and hover states are grays.
pub const ACCENT: Color32 = Color32::from_rgb(249, 115, 22);

/// Colors the hand-drawn chrome (bubbles, cards, status dots) needs beyond
/// what `Visuals` exposes; one struct per theme so panels can stay in sync
/// with the active theme via `ui.visuals().dark_mode`.
pub struct Palette {
    /// Fill of the central content area — the darkest layer, like an editor.
    pub central_fill: Color32,
    /// Slightly lifted fill for the assistant's reply bubbles.
    pub ai_bubble: Color32,
    /// Faint border for reply bubbles so they read on both themes.
    pub bubble_stroke: Color32,
    /// Neutral fill for the user's own messages.
    pub user_bubble: Color32,
    /// Fill of small rounded chips (model name, tags).
    pub chip_fill: Color32,
    /// Error card fill/border.
    pub danger_fill: Color32,
    pub danger_stroke: Color32,
    /// Text color readable on top of [`Self::ACCENT`].
    pub on_accent: Color32,
}

pub fn palette(dark_mode: bool) -> Palette {
    if dark_mode {
        Palette {
            central_fill: Color32::from_rgb(21, 22, 26),
            ai_bubble: Color32::from_rgb(36, 38, 46),
            bubble_stroke: Color32::from_rgb(48, 50, 59),
            user_bubble: Color32::from_rgb(50, 55, 64),
            chip_fill: Color32::from_rgb(42, 44, 53),
            danger_fill: Color32::from_rgb(62, 28, 32),
            danger_stroke: Color32::from_rgb(150, 66, 72),
            on_accent: Color32::WHITE,
        }
    } else {
        Palette {
            central_fill: Color32::from_rgb(244, 245, 248),
            ai_bubble: Color32::from_rgb(255, 255, 255),
            bubble_stroke: Color32::from_rgb(224, 227, 235),
            user_bubble: Color32::from_rgb(227, 230, 236),
            chip_fill: Color32::from_rgb(232, 234, 241),
            danger_fill: Color32::from_rgb(253, 235, 236),
            danger_stroke: Color32::from_rgb(226, 134, 140),
            on_accent: Color32::WHITE,
        }
    }
}

pub fn configure(context: &egui::Context) {
    set_fonts(context);
    // An explicit preference, not `ThemePreference::System`: egui keeps two
    // style slots (dark/light) and `set_visuals` writes into whichever slot is
    // active, so on a light-themed OS a visuals call at startup lands in the
    // wrong slot and the app renders light. `set_theme` pins the preference.
    context.set_theme(egui::ThemePreference::Dark);
    apply_style(context);
}

/// A small rounded badge (model name, app flags).
pub(crate) fn chip(ui: &mut egui::Ui, label: &str, fill: Color32) {
    egui::Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(7, 1))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(label).size(11.0).weak());
        });
}

/// Restyle both theme slots: rounded widgets, ghost buttons (transparent when
/// idle, filled on hover), a layered background, and the shared accent.
fn apply_style(context: &egui::Context) {
    context.set_visuals_of(egui::Theme::Dark, build_visuals(true));
    context.set_visuals_of(egui::Theme::Light, build_visuals(false));
    context.all_styles_mut(|style| {
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(10.0, 4.0);
        style.spacing.window_margin = Margin::same(12);
        style.text_styles = [
            (egui::TextStyle::Small, FontId::proportional(11.5)),
            (egui::TextStyle::Body, FontId::proportional(13.5)),
            (egui::TextStyle::Button, FontId::proportional(13.0)),
            (egui::TextStyle::Heading, FontId::proportional(19.0)),
            (egui::TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
    });
}

fn build_visuals(dark_mode: bool) -> egui::Visuals {
    let mut visuals = if dark_mode {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let colors = palette(dark_mode);
    let radius = CornerRadius::same(6);

    if dark_mode {
        visuals.panel_fill = Color32::from_rgb(26, 27, 32);
        visuals.window_fill = Color32::from_rgb(30, 31, 37);
        visuals.extreme_bg_color = Color32::from_rgb(16, 17, 21);
        visuals.faint_bg_color = Color32::from_rgb(34, 36, 43);
        visuals.code_bg_color = Color32::from_rgb(38, 40, 48);
    } else {
        visuals.panel_fill = Color32::from_rgb(238, 240, 244);
        visuals.window_fill = Color32::from_rgb(252, 252, 254);
        visuals.extreme_bg_color = Color32::from_rgb(255, 255, 255);
        visuals.faint_bg_color = Color32::from_rgb(230, 233, 239);
        visuals.code_bg_color = Color32::from_rgb(236, 238, 243);
    }

    visuals.hyperlink_color = ACCENT;
    visuals.weak_text_alpha = 0.55;
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.window_stroke = Stroke::new(1.0, colors.bubble_stroke);
    visuals.indent_has_left_vline = false;
    // Neutral slate for text selection and selectable lists — the sidebar pill
    // included. Bright enough to read on both themes.
    visuals.selection.bg_fill = if dark_mode {
        Color32::from_rgba_unmultiplied(142, 148, 160, 85)
    } else {
        Color32::from_rgba_unmultiplied(125, 135, 155, 75)
    };

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.corner_radius = radius;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, colors.bubble_stroke);
    // Ghost buttons: nothing when idle, a subtle chip on hover, a slightly
    // stronger gray while pressed — no colored ring.
    widgets.inactive.corner_radius = radius;
    widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    widgets.inactive.bg_stroke = Stroke::NONE;
    widgets.hovered.corner_radius = radius;
    widgets.hovered.weak_bg_fill = if dark_mode {
        Color32::from_rgb(44, 46, 55)
    } else {
        Color32::from_rgb(223, 226, 233)
    };
    widgets.hovered.bg_stroke = Stroke::new(1.0, colors.bubble_stroke);
    widgets.active.corner_radius = radius;
    widgets.active.weak_bg_fill = if dark_mode {
        Color32::from_rgb(52, 55, 66)
    } else {
        Color32::from_rgb(212, 216, 226)
    };
    widgets.active.bg_stroke = Stroke::NONE;
    widgets.open.corner_radius = radius;
    visuals
}

/// Install the CJK font.
///
/// Fonts live on the shared egui `Context` (all viewports/windows of the app
/// share it), so calling this once at startup covers every window. Loads the
/// system font, so never call it per frame.
///
/// The CJK font is the *primary* proportional font rather than a fallback:
/// with it last, a label mixing Latin and CJK ("AI 助手") takes its
/// ascent/descent from two different fonts, and the taller galley centers
/// with its baseline a pixel off the pure-CJK labels beside it. One font for
/// everything keeps every baseline on the same grid.
pub fn set_fonts(context: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for candidate in font_candidates() {
        if let Ok(bytes) = fs::read(candidate) {
            fonts
                .font_data
                .insert("fadb-cjk".to_owned(), FontData::from_owned(bytes).into());
            if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
                family.insert(0, "fadb-cjk".to_owned());
            }
            // The terminal paints with the monospace family; without the
            // fallback its Chinese text renders as tofu boxes.
            if let Some(family) = fonts.families.get_mut(&FontFamily::Monospace) {
                family.push("fadb-cjk".to_owned());
            }
            break;
        }
    }
    context.set_fonts(fonts);
}

fn font_candidates() -> impl Iterator<Item = &'static Path> {
    [
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simhei.ttf",
        "/System/Library/Fonts/PingFang.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    ]
    .into_iter()
    .map(Path::new)
}
