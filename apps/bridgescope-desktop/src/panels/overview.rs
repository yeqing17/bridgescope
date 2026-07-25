use bridgescope_domain::{DeviceOverview, DeviceRecord, DeviceState};
use eframe::egui::{self, Color32, RichText};

use crate::i18n::{Language, text};

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
            warning(
                ui,
                "Authorize USB debugging on the Android device, then refresh.",
            );
            return;
        }
        DeviceState::Offline => {
            warning(
                ui,
                "The selected device is offline. Reconnect it or restart ADB.",
            );
            return;
        }
        DeviceState::Unknown => {
            warning(ui, "The selected device is in an unsupported ADB state.");
            return;
        }
        DeviceState::Online => {}
    }

    if loading {
        ui.spinner();
        ui.label(text(language, "loading"));
    }

    if let Some(overview) = overview {
        egui::Grid::new("overview-grid")
            .num_columns(2)
            .spacing([24.0, 12.0])
            .striped(true)
            .show(ui, |ui| {
                row(ui, "Device", overview.model.as_deref());
                row(ui, "Serial", Some(overview.serial.as_str()));
                row(ui, "Manufacturer", overview.manufacturer.as_deref());
                row(ui, "Android", overview.android_version.as_deref());
                row_owned(
                    ui,
                    "API level",
                    overview.api_level.map(|value| value.to_string()),
                );
                row(ui, "ABI", overview.abi.as_deref());
                row_owned(
                    ui,
                    "Battery",
                    overview.battery_percent.map(|value| format!("{value}%")),
                );
                row_owned(ui, "Memory", overview.memory_total_kib.map(format_kib));
                row_owned(
                    ui,
                    "Storage",
                    overview.storage_total_kib.map(|total| {
                        let used = overview.storage_used_kib.unwrap_or_default();
                        format!("{} / {}", format_kib(used), format_kib(total))
                    }),
                );
            });
    } else if !loading {
        ui.label("Select Refresh to load overview data.");
    }
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

fn row(ui: &mut egui::Ui, label: &str, value: Option<&str>) {
    ui.strong(label);
    ui.label(value.unwrap_or("Not available"));
    ui.end_row();
}

fn row_owned(ui: &mut egui::Ui, label: &str, value: Option<String>) {
    ui.strong(label);
    ui.label(value.unwrap_or_else(|| "Not available".to_owned()));
    ui.end_row();
}

fn format_kib(value: u64) -> String {
    const KIB_PER_GIB: u128 = 1024 * 1024;
    let tenths = u128::from(value) * 10 / KIB_PER_GIB;
    format!("{}.{:01} GiB", tenths / 10, tenths % 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_kib_as_gib() {
        assert_eq!(format_kib(8 * 1024 * 1024), "8.0 GiB");
    }
}
