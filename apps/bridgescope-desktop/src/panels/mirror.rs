use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bridgescope_domain::{BackendCommand, BackendEvent, BridgeError, DeviceTarget, OperationId};
use bridgescope_scrcpy::ScrcpySessionPlan;
use eframe::egui::{self, RichText};

use crate::i18n::{Language, error_text, text};
use crate::runtime::MirrorFrameBuffer;

/// FPS is re-computed at most this often.
const FPS_WINDOW: f32 = 0.5;
/// Backoff between auto-start attempts while idle (dev hook only).
const AUTO_MIRROR_RETRY: Duration = Duration::from_secs(8);

/// Resolution cap presets offered by the panel (short-side pixels).
pub const MAX_SIZE_OPTIONS: [Option<u32>; 5] = [None, Some(1920), Some(1280), Some(960), Some(640)];
/// Video bit rate presets, in Mbit/s.
pub const BIT_RATE_MBPS_OPTIONS: [u32; 5] = [1, 2, 4, 8, 16];

#[derive(Default)]
pub struct MirrorPanelState {
    /// Start request awaiting its [`BackendEvent::MirrorStarted`].
    starting: Option<OperationId>,
    running: Option<OperationId>,
    stream: Option<(u32, u32)>,
    error: Option<BridgeError>,
    /// User-selected resolution cap; `None` means the native resolution.
    max_size: Option<u32>,
    /// User-selected video bit rate in bits per second.
    video_bit_rate: u32,
    texture: Option<egui::TextureHandle>,
    last_decoded: u64,
    fps: f32,
    fps_window: Option<(u64, Instant)>,
    last_auto_attempt: Option<Instant>,
}

impl MirrorPanelState {
    pub fn new() -> Self {
        Self {
            max_size: Some(ScrcpySessionPlan::DEFAULT_MAX_SIZE),
            video_bit_rate: ScrcpySessionPlan::DEFAULT_BIT_RATE,
            ..Self::default()
        }
    }

    pub fn handle_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::MirrorStarted {
                request_id,
                width,
                height,
                ..
            } => {
                if self.starting == Some(*request_id) {
                    self.starting = None;
                    self.running = Some(*request_id);
                    self.stream = Some((*width, *height));
                    self.error = None;
                }
            }
            BackendEvent::MirrorStopped { request_id, .. } => {
                self.end_session_if_matching(*request_id);
            }
            BackendEvent::MirrorFailed {
                request_id, error, ..
            } => {
                if self.starting == Some(*request_id) || self.running == Some(*request_id) {
                    self.end_session();
                    self.error = Some(error.clone());
                }
            }
            _ => {}
        }
    }

    fn end_session_if_matching(&mut self, request_id: OperationId) {
        if self.starting == Some(request_id) || self.running == Some(request_id) {
            self.end_session();
        }
    }

    fn end_session(&mut self) {
        self.starting = None;
        self.running = None;
        self.stream = None;
        self.texture = None;
        self.fps = 0.0;
        self.fps_window = None;
    }
}

/// Renders the mirror panel: quality presets, start/stop, and the live video.
#[allow(clippy::too_many_lines)]
pub fn show(
    ui: &mut egui::Ui,
    language: Language,
    context: &egui::Context,
    state: &mut MirrorPanelState,
    frames: &Arc<Mutex<MirrorFrameBuffer>>,
    selected: Option<&DeviceTarget>,
) -> Vec<BackendCommand> {
    let mut commands = Vec::new();
    let busy = state.starting.is_some() || state.running.is_some();
    ui.horizontal(|ui| {
        ui.heading(text(language, "mirror"));
        if state.starting.is_some() {
            ui.spinner();
        }
    });
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.add_enabled_ui(!busy, |ui| {
            egui::ComboBox::from_id_salt("mirror-max-size")
                .selected_text(max_size_label(language, state.max_size))
                .show_ui(ui, |ui| {
                    for option in MAX_SIZE_OPTIONS {
                        ui.selectable_value(
                            &mut state.max_size,
                            option,
                            max_size_label(language, option),
                        );
                    }
                });
            egui::ComboBox::from_id_salt("mirror-bit-rate")
                .selected_text(format!("{} Mbps", state.video_bit_rate / 1_000_000))
                .show_ui(ui, |ui| {
                    for option in BIT_RATE_MBPS_OPTIONS {
                        ui.selectable_value(
                            &mut state.video_bit_rate,
                            option * 1_000_000,
                            format!("{option} Mbps"),
                        );
                    }
                });
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if busy {
                if ui
                    .add_enabled(true, egui::Button::new(text(language, "mirror_stop")))
                    .clicked()
                {
                    commands.push(BackendCommand::StopMirror);
                }
            } else if ui
                .add_enabled(
                    selected.is_some(),
                    egui::Button::new(text(language, "mirror_start")),
                )
                .on_disabled_hover_text(text(language, "mirror_need_device"))
                .clicked()
            {
                let request_id = OperationId::new();
                state.starting = Some(request_id);
                state.error = None;
                if let Some(target) = selected {
                    commands.push(BackendCommand::StartMirror {
                        request_id,
                        target: target.clone(),
                        max_size: state.max_size,
                        video_bit_rate: state.video_bit_rate,
                    });
                } else {
                    state.starting = None;
                }
            }
        });
    });
    ui.add_space(4.0);

    match (state.stream, state.running) {
        (Some((width, height)), Some(_)) => {
            ui.label(
                RichText::new(format!(
                    "{} · {width}×{height} · {:.0} fps",
                    text(language, "mirror_running"),
                    state.fps
                ))
                .color(egui::Color32::from_rgb(74, 222, 128)),
            );
        }
        (None, _) if state.starting.is_some() => {
            ui.label(text(language, "mirror_starting"));
        }
        _ => {
            ui.label(text(language, "mirror_hint"));
        }
    }
    if let Some(error) = &state.error {
        ui.label(
            RichText::new(error_text(language, error))
                .color(egui::Color32::from_rgb(248, 113, 113)),
        );
    }
    ui.add_space(6.0);

    absorb_frames(state, frames, context);
    if let Some(texture) = state.texture.as_ref() {
        let natural = texture.size_vec2();
        let available = ui.available_size();
        let scale = (available.x / natural.x)
            .min(available.y / natural.y)
            .clamp(0.01, 1.0);
        egui::ScrollArea::both()
            .auto_shrink(false)
            .id_salt("mirror-video")
            .show(ui, |ui| {
                ui.add(
                    egui::Image::from_texture(texture)
                        .fit_to_exact_size(natural * scale)
                        .maintain_aspect_ratio(true),
                );
            });
    } else if state.running.is_some() || state.starting.is_some() {
        ui.centered_and_justified(|ui| {
            ui.weak(text(language, "mirror_waiting"));
        });
    }
    commands
}

/// Picks up newly decoded frames from the shared buffer and refreshes the
/// texture plus the FPS estimate.
#[allow(clippy::cast_precision_loss)]
fn absorb_frames(
    state: &mut MirrorPanelState,
    frames: &Arc<Mutex<MirrorFrameBuffer>>,
    context: &egui::Context,
) {
    let Ok(buffer) = frames.lock() else {
        return;
    };
    if buffer.decoded == state.last_decoded {
        return;
    }
    let decoded = buffer.decoded;
    let frame = buffer.frame.clone();
    drop(buffer);
    state.last_decoded = decoded;
    // Frames landed since the last repaint; drive the next repaint now so the
    // video tracks the stream instead of waiting for ambient UI events.
    context.request_repaint();

    if let Some(frame) = frame {
        let size = [frame.width, frame.height];
        let image = egui::ColorImage::from_rgba_unmultiplied(size, &frame.rgba);
        if let Some(texture) = state.texture.as_mut() {
            texture.set(image, egui::TextureOptions::LINEAR);
        } else {
            state.texture = Some(context.load_texture(
                "bridgescope-mirror",
                image,
                egui::TextureOptions::LINEAR,
            ));
        }
        let now = Instant::now();
        match state.fps_window {
            Some((count, started)) => {
                let elapsed = now.duration_since(started).as_secs_f32();
                if elapsed >= FPS_WINDOW {
                    state.fps = (decoded - count) as f32 / elapsed;
                    state.fps_window = Some((decoded, now));
                }
            }
            None => state.fps_window = Some((decoded, now)),
        }
    }
}

fn max_size_label(language: Language, option: Option<u32>) -> String {
    match option {
        Some(size) => size.to_string(),
        None => text(language, "mirror_native").to_owned(),
    }
}

/// Dev hook (`BRIDGESCOPE_MIRROR_AUTO=1`): starts mirroring automatically so
/// visual checks run without scripted input.
pub fn auto(state: &mut MirrorPanelState, selected: Option<&DeviceTarget>) -> Vec<BackendCommand> {
    let mut commands = Vec::new();
    let idle = state.starting.is_none() && state.running.is_none();
    let backed_off = state
        .last_auto_attempt
        .is_none_or(|last| last.elapsed() >= AUTO_MIRROR_RETRY);
    if std::env::var_os("BRIDGESCOPE_MIRROR_AUTO").is_some()
        && idle
        && backed_off
        && let Some(target) = selected
    {
        state.last_auto_attempt = Some(Instant::now());
        let request_id = OperationId::new();
        state.starting = Some(request_id);
        state.error = None;
        commands.push(BackendCommand::StartMirror {
            request_id,
            target: target.clone(),
            max_size: state.max_size,
            video_bit_rate: state.video_bit_rate,
        });
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridgescope_domain::{DeviceSerial, ErrorCode};

    #[test]
    fn started_then_stopped_transitions_the_session() {
        let mut state = MirrorPanelState::new();
        assert_eq!(state.max_size, Some(1280));
        let request_id = OperationId::new();
        state.starting = Some(request_id);
        state.handle_event(&BackendEvent::MirrorStarted {
            request_id,
            target: DeviceTarget::new(DeviceSerial::new("s").expect("serial"), 1),
            width: 1280,
            height: 720,
        });
        assert_eq!(state.running, Some(request_id));
        assert_eq!(state.stream, Some((1280, 720)));
        state.handle_event(&BackendEvent::MirrorStopped {
            request_id,
            target: DeviceTarget::new(DeviceSerial::new("s").expect("serial"), 1),
        });
        assert!(state.running.is_none() && state.starting.is_none());
        assert!(state.texture.is_none());
    }

    #[test]
    fn failure_clears_the_session_and_reports() {
        let mut state = MirrorPanelState::new();
        let request_id = OperationId::new();
        state.starting = Some(request_id);
        state.handle_event(&BackendEvent::MirrorFailed {
            request_id,
            target: DeviceTarget::new(DeviceSerial::new("s").expect("serial"), 1),
            error: BridgeError::new(ErrorCode::Internal, "mirror.push_failed", "x"),
        });
        assert!(state.starting.is_none() && state.running.is_none());
        assert_eq!(
            state.error.map(|error| error.message_key),
            Some("mirror.push_failed".to_owned())
        );
    }

    #[test]
    fn unrelated_events_are_ignored() {
        let mut state = MirrorPanelState::new();
        state.handle_event(&BackendEvent::MirrorStarted {
            request_id: OperationId::new(),
            target: DeviceTarget::new(DeviceSerial::new("s").expect("serial"), 1),
            width: 1,
            height: 1,
        });
        assert!(state.running.is_none() && state.starting.is_none());
    }
}
