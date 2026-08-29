use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use bridgescope_domain::{
    BackendCommand, BackendEvent, BridgeError, DeviceRecord, DeviceTarget, OperationId,
    ScreenshotData, ScreenshotFormat,
};
use eframe::egui::{self, Color32};

use crate::i18n::{Language, error_text, text};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DisplayMode {
    Fit,
    Actual,
}

pub struct ScreenshotPanelState {
    target: Option<DeviceTarget>,
    request_id: Option<OperationId>,
    loading: bool,
    error: Option<BridgeError>,
    texture: Option<egui::TextureHandle>,
    color_image: Option<egui::ColorImage>,
    raw_png: Option<Vec<u8>>,
    dimensions: Option<[usize; 2]>,
    display_mode: DisplayMode,
    saved_path: Option<PathBuf>,
}

impl Default for ScreenshotPanelState {
    fn default() -> Self {
        Self {
            target: None,
            request_id: None,
            loading: false,
            error: None,
            texture: None,
            color_image: None,
            raw_png: None,
            dimensions: None,
            display_mode: DisplayMode::Fit,
            saved_path: None,
        }
    }
}

impl ScreenshotPanelState {
    pub fn reconcile_target(&mut self, selected: Option<&DeviceRecord>) {
        let target = selected.map(DeviceRecord::target);
        if self.target != target {
            self.target = target;
            self.request_id = None;
            self.loading = false;
            self.error = None;
            self.texture = None;
            self.color_image = None;
            self.raw_png = None;
            self.dimensions = None;
            self.saved_path = None;
        }
    }

    pub fn handle_event(&mut self, context: &egui::Context, event: &BackendEvent) {
        match event {
            BackendEvent::ScreenshotLoading {
                target, request_id, ..
            } if self.target.as_ref() == Some(target) && self.request_id == Some(*request_id) => {
                self.loading = true;
                self.error = None;
            }
            BackendEvent::ScreenshotCaptured {
                target,
                request_id,
                data,
            } if self.target.as_ref() == Some(target) && self.request_id == Some(*request_id) => {
                self.loading = false;
                self.error = None;
                match data {
                    ScreenshotData::DecodedRgba8(image) => {
                        let size = [
                            usize::try_from(image.width()).unwrap_or_default(),
                            usize::try_from(image.height()).unwrap_or_default(),
                        ];
                        let color_image =
                            egui::ColorImage::from_rgba_unmultiplied(size, image.rgba());
                        if let Some(texture) = self.texture.as_mut() {
                            texture.set(color_image.clone(), egui::TextureOptions::LINEAR);
                        } else {
                            self.texture = Some(context.load_texture(
                                "bridgescope-device-screenshot",
                                color_image.clone(),
                                egui::TextureOptions::LINEAR,
                            ));
                        }
                        self.color_image = Some(color_image);
                        self.dimensions = Some(size);
                    }
                    ScreenshotData::RawPng(png) => {
                        self.raw_png = Some(png.as_bytes().to_vec());
                    }
                    ScreenshotData::DecodedWithPng { image, png } => {
                        let size = [
                            usize::try_from(image.width()).unwrap_or_default(),
                            usize::try_from(image.height()).unwrap_or_default(),
                        ];
                        let color_image =
                            egui::ColorImage::from_rgba_unmultiplied(size, image.rgba());
                        if let Some(texture) = self.texture.as_mut() {
                            texture.set(color_image.clone(), egui::TextureOptions::LINEAR);
                        } else {
                            self.texture = Some(context.load_texture(
                                "bridgescope-device-screenshot",
                                color_image.clone(),
                                egui::TextureOptions::LINEAR,
                            ));
                        }
                        self.color_image = Some(color_image);
                        self.dimensions = Some(size);
                        self.raw_png = Some(png.as_bytes().to_vec());
                    }
                }
            }
            BackendEvent::ScreenshotFailed {
                target,
                request_id,
                error,
            } if self.target.as_ref() == Some(target) && self.request_id == Some(*request_id) => {
                self.loading = false;
                self.error = Some(error.clone());
            }
            _ => {}
        }
    }

    fn capture(&mut self) -> Option<BackendCommand> {
        let target = self.target.clone()?;
        let request_id = OperationId::new();
        self.request_id = Some(request_id);
        self.loading = true;
        self.error = None;
        Some(BackendCommand::CaptureScreenshot {
            target,
            request_id,
            format: ScreenshotFormat::RawPng,
        })
    }

    fn save_png(&mut self) -> Result<PathBuf, std::io::Error> {
        let bytes = self.raw_png.as_ref().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "raw PNG is not available")
        })?;
        let directory = std::env::current_dir()?.join("screenshots");
        fs::create_dir_all(&directory)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let path = directory.join(format!("bridgescope-{timestamp}.png"));
        fs::write(&path, bytes)?;
        self.saved_path = Some(path.clone());
        Ok(path)
    }
}

#[allow(clippy::too_many_lines)]
pub fn show(
    ui: &mut egui::Ui,
    language: Language,
    selected: Option<&DeviceRecord>,
    state: &mut ScreenshotPanelState,
) -> Vec<BackendCommand> {
    state.reconcile_target(selected);
    let mut commands = Vec::new();
    ui.horizontal(|ui| {
        ui.heading(text(language, "screenshot"));
        let online = selected.is_some_and(|record| record.descriptor.state.is_online());
        let label = if state.texture.is_some() {
            text(language, "screenshot_retake")
        } else {
            text(language, "screenshot_capture")
        };
        if ui
            .add_enabled(online && !state.loading, egui::Button::new(label))
            .clicked()
            && let Some(command) = state.capture()
        {
            commands.push(command);
        }
        if ui
            .add_enabled(
                state.raw_png.is_some(),
                egui::Button::new(text(language, "screenshot_save_png")),
            )
            .clicked()
            && let Err(error) = state.save_png()
        {
            state.error = Some(BridgeError::new(
                bridgescope_domain::ErrorCode::Internal,
                "screenshot.save_failed",
                error.to_string(),
            ));
        }
        ui.selectable_value(
            &mut state.display_mode,
            DisplayMode::Fit,
            text(language, "screenshot_fit"),
        );
        ui.selectable_value(
            &mut state.display_mode,
            DisplayMode::Actual,
            text(language, "screenshot_actual"),
        );
        if ui
            .add_enabled(
                state.color_image.is_some(),
                egui::Button::new(text(language, "screenshot_copy_image")),
            )
            .clicked()
            && let Some(image) = state.color_image.clone()
        {
            ui.ctx().copy_image(image);
        }
    });

    if state.loading {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(text(language, "screenshot_capturing"));
        });
    }
    if let Some(error) = &state.error {
        ui.colored_label(Color32::LIGHT_RED, error_text(language, error));
    }
    if let Some(path) = &state.saved_path {
        ui.small(format!(
            "{}{}",
            text(language, "screenshot_saved"),
            path.display()
        ));
    }
    if let Some([width, height]) = state.dimensions {
        ui.small(format!(
            "{width} × {height} {}",
            text(language, "screenshot_pixels")
        ));
    }

    let Some(texture) = &state.texture else {
        ui.vertical_centered(|ui| {
            ui.add_space(90.0);
            ui.label(text(language, "screenshot_hint"));
        });
        return commands;
    };

    egui::ScrollArea::both()
        .id_salt("screenshot-view")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let natural = texture.size_vec2();
            let size = match state.display_mode {
                DisplayMode::Actual => natural,
                DisplayMode::Fit => {
                    let available = ui.available_size();
                    let scale = (available.x / natural.x)
                        .min(available.y / natural.y)
                        .clamp(0.01, 1.0);
                    natural * scale
                }
            };
            ui.add(
                egui::Image::from_texture(texture)
                    .fit_to_exact_size(size)
                    .maintain_aspect_ratio(true),
            );
        });
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_state_has_no_capture_target() {
        let state = ScreenshotPanelState::default();
        assert!(state.target.is_none());
        assert!(!state.loading);
    }
}
