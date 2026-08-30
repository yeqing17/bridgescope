use bridgescope_domain::{DeviceOverview, DeviceRecord, DeviceState};
use eframe::egui::{
    self, Align, Color32, CornerRadius, FontId, Layout, Pos2, RichText, Sense, Stroke, StrokeKind,
    Vec2,
};

use crate::i18n::{Language, text};
use crate::theme;

pub fn show(
    ui: &mut egui::Ui,
    language: Language,
    selected: Option<&DeviceRecord>,
    overview: Option<&DeviceOverview>,
    loading: bool,
) {
    ui.heading(text(language, "overview"));
    ui.add_space(12.0);

    let Some(device) = selected else {
        empty_state(ui, language);
        return;
    };

    match device.descriptor.state {
        DeviceState::Unauthorized => {
            warning(ui, text(language, "overview_unauthorized"));
            return;
        }
        DeviceState::Offline => {
            warning(ui, text(language, "overview_offline"));
            return;
        }
        DeviceState::Unknown => {
            warning(ui, text(language, "overview_unknown_state"));
            return;
        }
        DeviceState::Online => {}
    }

    if loading {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(text(language, "loading"));
        });
    }

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .id_salt("overview-card")
        .show(ui, |ui| match overview {
            Some(overview) => device_card(ui, language, device, overview),
            None if !loading => {
                ui.label(text(language, "overview_not_loaded"));
            }
            None => {}
        });
}

/// The raised surface holding the device hero line and the field grid.
fn device_card(
    ui: &mut egui::Ui,
    language: Language,
    device: &DeviceRecord,
    overview: &DeviceOverview,
) {
    let palette = theme::palette(ui.visuals().dark_mode);
    egui::Frame::new()
        .fill(palette.ai_bubble)
        .stroke(Stroke::new(1.0, palette.bubble_stroke))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::same(20))
        .show(ui, |ui| {
            hero(ui, overview);
            ui.add_space(14.0);
            ui.separator();
            ui.add_space(16.0);
            let fields = overview_fields(language, device, overview);
            field_grid(ui, language, &fields);
        });
}

/// Device name with brand / Android chips, echoing the card's monochrome tile.
fn hero(ui: &mut egui::Ui, overview: &DeviceOverview) {
    let palette = theme::palette(ui.visuals().dark_mode);
    ui.horizontal(|ui| {
        let (tile, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
        let painter = ui.painter();
        // A raised gray slab — the card carries no hue at all.
        let (tile_fill, glyph) = if ui.visuals().dark_mode {
            (Color32::from_rgb(56, 60, 70), Color32::WHITE)
        } else {
            (
                Color32::from_rgb(212, 216, 224),
                Color32::from_rgb(28, 30, 36),
            )
        };
        painter.rect_filled(tile, 12.0, tile_fill);
        paint_field_icon(painter, tile.center(), FieldIcon::Name, glyph, 2.0);
        ui.add_space(6.0);
        ui.vertical(|ui| {
            let name = overview
                .model
                .clone()
                .unwrap_or_else(|| overview.serial.as_str().to_owned());
            ui.label(RichText::new(name).size(22.0).strong());
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if let Some(brand) = overview
                    .brand
                    .as_deref()
                    .or(overview.manufacturer.as_deref())
                {
                    theme::chip(ui, brand, palette.chip_fill);
                }
                if let Some(version) = &overview.android_version {
                    theme::chip(ui, &format!("Android {version}"), palette.chip_fill);
                }
                if let Some(api) = overview.api_level {
                    theme::chip(ui, &format!("API {api}"), palette.chip_fill);
                }
            });
        });
    });
}

struct Field {
    icon: FieldIcon,
    label: &'static str,
    /// Main value, rendered large and strong.
    value: Option<String>,
    /// Unit or qualifier (`GiB`, `%`, `API 34`), rendered small and weak
    /// after the value so numbers read at a glance.
    unit: Option<String>,
}

fn field(icon: FieldIcon, label: &'static str, value: Option<String>) -> Field {
    Field {
        icon,
        label,
        value,
        unit: None,
    }
}

fn field_with_unit(
    icon: FieldIcon,
    label: &'static str,
    value: Option<String>,
    unit: Option<String>,
) -> Field {
    Field {
        icon,
        label,
        value,
        unit,
    }
}

/// The AYA-style field set: identity, system, hardware, display, network.
fn overview_fields(
    language: Language,
    device: &DeviceRecord,
    overview: &DeviceOverview,
) -> Vec<Field> {
    let (android, api_unit) = android_summary(overview);
    let mut fields = vec![
        field(
            FieldIcon::Name,
            "overview_field_name",
            overview.model.clone(),
        ),
        field(
            FieldIcon::Brand,
            "overview_field_brand",
            overview
                .brand
                .clone()
                .or_else(|| overview.manufacturer.clone()),
        ),
        field(
            FieldIcon::ModelCode,
            "overview_field_model",
            device.descriptor.model.clone(),
        ),
        field(
            FieldIcon::Serial,
            "overview_field_serial",
            Some(device.descriptor.serial.as_str().to_owned()),
        ),
        field_with_unit(
            FieldIcon::Android,
            "overview_field_android",
            android,
            api_unit,
        ),
        field(
            FieldIcon::Kernel,
            "overview_field_kernel",
            overview.kernel_version.clone(),
        ),
    ];
    fields.extend(hardware_fields(language, overview));
    fields
}

/// Processor, memory, storage, battery, display and network fields.
fn hardware_fields(language: Language, overview: &DeviceOverview) -> Vec<Field> {
    let (processor, processor_unit) = processor_summary(language, overview);
    let storage = overview.storage_total_kib.map(|total| {
        format!(
            "{} / {}",
            format_kib_parts(overview.storage_used_kib.unwrap_or_default()).0,
            format_kib_parts(total).0
        )
    });
    let (physical, density_unit) = match (&overview.screen_physical, &overview.screen_density) {
        (Some(size), Some(density)) => (Some(size.clone()), Some(format!("({density}dpi)"))),
        (Some(size), None) => (Some(size.clone()), None),
        _ => (None, None),
    };
    let font_scale = overview
        .font_scale
        .as_deref()
        .and_then(|value| value.parse::<f32>().ok())
        .map(|value| value.to_string());
    let memory = overview.memory_total_kib.map(|kib| format_kib_parts(kib).0);
    let battery_value = overview.battery_percent.map(|percent| percent.to_string());
    vec![
        field_with_unit(
            FieldIcon::Cpu,
            "overview_field_cpu",
            processor,
            processor_unit,
        ),
        field_with_unit(
            FieldIcon::Memory,
            "overview_field_memory",
            memory,
            overview.memory_total_kib.map(|_| "GiB".to_owned()),
        ),
        field_with_unit(
            FieldIcon::Storage,
            "overview_field_storage",
            storage,
            overview.storage_total_kib.map(|_| "GiB".to_owned()),
        ),
        field_with_unit(
            FieldIcon::Battery(overview.battery_percent),
            "overview_field_battery",
            battery_value,
            overview.battery_percent.map(|_| "%".to_owned()),
        ),
        field_with_unit(
            FieldIcon::Physical,
            "overview_field_physical",
            physical,
            density_unit,
        ),
        field(
            FieldIcon::Resolution,
            "overview_field_resolution",
            overview
                .screen_override
                .clone()
                .or_else(|| overview.screen_physical.clone()),
        ),
        field_with_unit(
            FieldIcon::Font,
            "overview_field_font",
            font_scale,
            Some("×".to_owned()),
        ),
        field(
            FieldIcon::Wifi,
            "overview_field_wifi",
            overview.wifi_ssid.clone(),
        ),
        field(
            FieldIcon::Ip,
            "overview_field_ip",
            overview.ip_address.clone(),
        ),
        field(
            FieldIcon::Mac,
            "overview_field_mac",
            overview.mac_address.clone(),
        ),
    ]
}

/// `Android 14` with `API 34` as a small unit, degrading gracefully when
/// either part is missing.
fn android_summary(overview: &DeviceOverview) -> (Option<String>, Option<String>) {
    match (&overview.android_version, overview.api_level) {
        (Some(version), Some(api)) => (
            Some(format!("Android {version}")),
            Some(format!("API {api}")),
        ),
        (Some(version), None) => (Some(format!("Android {version}")), None),
        (None, Some(api)) => (Some(format!("API {api}")), None),
        (None, None) => (None, None),
    }
}

/// SoC · core count as the value, ABI as the small unit,
/// e.g. `SM8475 · 8 核` + `(arm64-v8a)`.
fn processor_summary(
    language: Language,
    overview: &DeviceOverview,
) -> (Option<String>, Option<String>) {
    let mut parts: Vec<String> = Vec::new();
    if let Some(soc) = &overview.soc {
        parts.push(soc.clone());
    }
    if let Some(cores) = overview.cpu_cores {
        parts.push(format!("{cores} {}", text(language, "overview_cpu_cores")));
    }
    let unit = overview.abi.as_ref().map(|abi| format!("({abi})"));
    let value = (!parts.is_empty()).then(|| parts.join(" · "));
    (value, unit)
}

/// Three columns that stay aligned across rows and together span the full
/// card width: `egui::Grid` keeps shared column origins while
/// `min_col_width` spreads the columns out.
fn field_grid(ui: &mut egui::Ui, language: Language, fields: &[Field]) {
    const COLUMNS: u8 = 3;
    const GAP_X: f32 = 32.0;
    let columns = f32::from(COLUMNS);
    let min_col_width = ((ui.available_width() - GAP_X * (columns - 1.0)) / columns).max(140.0);
    egui::Grid::new("overview-fields")
        .num_columns(usize::from(COLUMNS))
        .spacing([GAP_X, 18.0])
        .min_col_width(min_col_width)
        .show(ui, |ui| {
            for (index, entry) in fields.iter().enumerate() {
                field_cell(ui, language, entry);
                if (index + 1) % usize::from(COLUMNS) == 0 {
                    ui.end_row();
                }
            }
        });
}

/// Small icon + weak label on top, the value (or 暂无) below.
fn field_cell(ui: &mut egui::Ui, language: Language, entry: &Field) {
    const ICON_SIZE: f32 = 17.0;
    const VALUE_SIZE: f32 = 17.0;
    const UNIT_SIZE: f32 = 13.0;
    // Muted gray icons: one family, set apart from the values, no hue.
    let icon_color = ui.visuals().text_color().gamma_multiply(0.55);
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(ICON_SIZE), Sense::hover());
            paint_field_icon(ui.painter(), rect.center(), entry.icon, icon_color, 1.3);
            ui.label(RichText::new(text(language, entry.label)).size(13.5).weak());
        });
        ui.add_space(1.0);
        match &entry.value {
            Some(value) => {
                ui.with_layout(Layout::left_to_right(Align::Max), |ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    // Extend, or Grid measures the wrapped width and squeezes
                    // the column until the value does wrap.
                    ui.add(
                        egui::Label::new(RichText::new(value).size(VALUE_SIZE).strong())
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                    if let Some(unit) = &entry.unit {
                        ui.add(
                            egui::Label::new(RichText::new(unit).size(UNIT_SIZE).weak())
                                .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    }
                });
            }
            None => {
                ui.label(
                    RichText::new(text(language, "overview_na"))
                        .size(15.0)
                        .weak(),
                );
            }
        }
    });
}

fn empty_state(ui: &mut egui::Ui, language: Language) {
    ui.vertical_centered(|ui| {
        ui.add_space(80.0);
        ui.label(RichText::new(text(language, "select_device")).size(24.0));
        ui.add_space(8.0);
        ui.label(text(language, "explicit_selection"));
    });
}

fn warning(ui: &mut egui::Ui, message: &str) {
    ui.colored_label(
        Color32::from_rgb(245, 158, 11),
        RichText::new(message).strong(),
    );
}

/// `(number, unit)` so the UI can shrink the unit: `("8.0", "GiB")`.
fn format_kib_parts(value: u64) -> (String, &'static str) {
    const KIB_PER_GIB: u128 = 1024 * 1024;
    let tenths = u128::from(value) * 10 / KIB_PER_GIB;
    (format!("{}.{:01}", tenths / 10, tenths % 10), "GiB")
}

#[derive(Clone, Copy)]
enum FieldIcon {
    Name,
    Brand,
    ModelCode,
    Serial,
    Android,
    Kernel,
    Cpu,
    Memory,
    Storage,
    Battery(Option<u8>),
    Physical,
    Resolution,
    Font,
    Wifi,
    Ip,
    Mac,
}

/// Hand-drawn 12x12 vector glyphs (scaled by `scale`); the bundled fonts
/// render dingbat/emoji codepoints as tofu, so icons are painted primitives.
#[allow(clippy::too_many_lines)]
fn paint_field_icon(
    painter: &egui::Painter,
    center: Pos2,
    icon: FieldIcon,
    color: Color32,
    scale: f32,
) {
    let stroke = Stroke::new(1.3 * scale, color);
    let p = |dx: f32, dy: f32| center + Vec2::new(dx * scale, dy * scale);
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| egui::Rect::from_two_pos(p(x0, y0), p(x1, y1));
    match icon {
        FieldIcon::Name => {
            painter.rect_stroke(
                rect(-4.5, -5.5, 4.5, 5.5),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Inside,
            );
            painter.line_segment([p(-1.2, 3.4), p(1.2, 3.4)], stroke);
        }
        FieldIcon::Brand => {
            painter.circle_stroke(center, 4.3 * scale, stroke);
            painter.circle_filled(center, 1.5 * scale, color);
        }
        FieldIcon::ModelCode => {
            painter.rect_stroke(
                rect(-5.0, -4.5, 5.0, 4.5),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Inside,
            );
            painter.line_segment([p(-2.8, -1.6), p(2.8, -1.6)], stroke);
            painter.line_segment([p(-2.8, 1.4), p(1.0, 1.4)], stroke);
        }
        FieldIcon::Serial => {
            painter.line_segment([p(-1.6, -5.2), p(-1.6, 5.2)], stroke);
            painter.line_segment([p(1.6, -5.2), p(1.6, 5.2)], stroke);
            painter.line_segment([p(-5.0, -1.6), p(5.0, -1.6)], stroke);
            painter.line_segment([p(-5.0, 1.6), p(5.0, 1.6)], stroke);
        }
        FieldIcon::Android => {
            painter.rect_stroke(
                rect(-5.0, -1.4, 5.0, 4.8),
                CornerRadius {
                    nw: 4,
                    ne: 4,
                    sw: 1,
                    se: 1,
                },
                stroke,
                StrokeKind::Inside,
            );
            painter.circle_filled(p(-2.3, 0.9), 0.8 * scale, color);
            painter.circle_filled(p(2.3, 0.9), 0.8 * scale, color);
            painter.line_segment([p(-2.7, -1.4), p(-3.8, -3.5)], stroke);
            painter.line_segment([p(2.7, -1.4), p(3.8, -3.5)], stroke);
        }
        FieldIcon::Kernel => {
            painter.add(egui::Shape::line(
                vec![p(-4.4, -3.2), p(-1.2, 0.0), p(-4.4, 3.2)],
                stroke,
            ));
            painter.line_segment([p(0.8, 3.4), p(4.6, 3.4)], stroke);
        }
        FieldIcon::Cpu => {
            painter.rect_stroke(
                rect(-3.2, -3.2, 3.2, 3.2),
                CornerRadius::same(1),
                stroke,
                StrokeKind::Inside,
            );
            painter.rect_filled(rect(-1.2, -1.2, 1.2, 1.2), 1.0, color);
            for offset in [-1.6_f32, 1.6] {
                painter.line_segment([p(offset, -5.0), p(offset, -3.2)], stroke);
                painter.line_segment([p(offset, 3.2), p(offset, 5.0)], stroke);
                painter.line_segment([p(-5.0, offset), p(-3.2, offset)], stroke);
                painter.line_segment([p(3.2, offset), p(5.0, offset)], stroke);
            }
        }
        FieldIcon::Memory => {
            painter.rect_stroke(
                rect(-4.8, -3.2, 4.8, 3.2),
                CornerRadius::same(1),
                stroke,
                StrokeKind::Inside,
            );
            for x in [-2.4_f32, 0.0, 2.4] {
                painter.line_segment([p(x, -3.2), p(x, 3.2)], stroke);
            }
            for x in [-3.6_f32, -1.2, 1.2, 3.6] {
                painter.line_segment([p(x, 3.2), p(x, 5.0)], stroke);
            }
        }
        FieldIcon::Storage => {
            painter.rect_stroke(
                rect(-4.6, -3.4, 4.6, 3.4),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Inside,
            );
            painter.line_segment([p(-4.6, -1.1), p(4.6, -1.1)], stroke);
            painter.circle_filled(p(2.6, 1.2), 0.9 * scale, color);
        }
        FieldIcon::Battery(percent) => {
            painter.rect_stroke(
                rect(-5.6, -3.4, 3.8, 3.4),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Inside,
            );
            painter.rect_filled(rect(3.8, -1.6, 5.4, 1.6), 1.0, color);
            if let Some(percent) = percent {
                let width = 7.4 * f32::from(percent) / 100.0;
                painter.rect_filled(rect(-4.6, -2.4, -4.6 + width, 2.4), 1.0, color);
            }
        }
        FieldIcon::Physical => {
            painter.rect_stroke(
                rect(-5.2, -3.8, 5.2, 3.8),
                CornerRadius::same(1),
                stroke,
                StrokeKind::Inside,
            );
            painter.line_segment([p(-3.2, 2.4), p(3.2, -2.4)], stroke);
        }
        FieldIcon::Resolution => {
            painter.rect_stroke(
                rect(-5.2, -3.8, 5.2, 3.8),
                CornerRadius::same(1),
                stroke,
                StrokeKind::Inside,
            );
            painter.rect_filled(rect(0.8, -2.2, 3.8, 0.2), 1.0, color);
        }
        FieldIcon::Font => {
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                "A",
                FontId::proportional(11.0 * scale),
                color,
            );
        }
        FieldIcon::Wifi => {
            let arc = |radius: f32| {
                let points = (0_u8..=6)
                    .map(|step| {
                        let angle = (30.0 + 120.0 * f32::from(step) / 6.0).to_radians();
                        p(radius * angle.cos(), 3.6 - radius * angle.sin())
                    })
                    .collect::<Vec<_>>();
                painter.add(egui::Shape::line(points, stroke));
            };
            arc(5.4);
            arc(3.0);
            painter.circle_filled(p(0.0, 2.8), 1.1 * scale, color);
        }
        FieldIcon::Ip => {
            painter.circle_stroke(center, 4.6 * scale, stroke);
            painter.line_segment([p(-4.6, 0.0), p(4.6, 0.0)], stroke);
            painter.line_segment([p(0.0, -4.6), p(0.0, 4.6)], stroke);
            painter.line_segment([p(-2.3, -3.9), p(-2.3, 3.9)], stroke);
            painter.line_segment([p(2.3, -3.9), p(2.3, 3.9)], stroke);
        }
        FieldIcon::Mac => {
            painter.rect_stroke(
                rect(-5.4, 0.4, 0.6, 3.4),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Inside,
            );
            painter.rect_stroke(
                rect(-0.6, -3.4, 5.4, -0.4),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Inside,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_kib_into_value_and_unit() {
        assert_eq!(format_kib_parts(8 * 1024 * 1024), ("8.0".to_owned(), "GiB"));
    }
}
