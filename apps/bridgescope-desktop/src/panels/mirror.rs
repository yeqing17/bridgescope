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
/// Minimum spacing between auto-recording start/stop steps (dev hook only).
const RECORD_AUTO_STEP: Duration = Duration::from_millis(500);

/// Resolution cap presets offered by the panel (short-side pixels).
pub const MAX_SIZE_OPTIONS: [Option<u32>; 5] = [None, Some(1920), Some(1280), Some(960), Some(640)];
/// Video bit rate presets, in Mbit/s.
pub const BIT_RATE_MBPS_OPTIONS: [u32; 5] = [1, 2, 4, 8, 16];
/// Width reserved for the remote-control column, separator and card margins
/// included. Three keys per row (3×62 + spacing) must fit inside the card.
const REMOTE_COLUMN: f32 = 264.0;
/// Uniform size of one remote key.
const KEY_SIZE: egui::Vec2 = egui::Vec2::new(62.0, 34.0);

/// One remote-control button: display label plus the Android keycode it
/// delivers via `input keyevent`. Labels are static or already-localized
/// `text()` output, so the struct stays cheap enough to rebuild per frame.
#[derive(Clone, Copy)]
struct RemoteKey {
    label: &'static str,
    keycode: u32,
}

impl RemoteKey {
    const fn new(label: &'static str, keycode: u32) -> Self {
        Self { label, keycode }
    }
}

#[derive(Default)]
pub struct MirrorPanelState {
    /// Start request awaiting its [`BackendEvent::MirrorStarted`].
    starting: Option<OperationId>,
    running: Option<OperationId>,
    stream: Option<(u32, u32)>,
    error: Option<BridgeError>,
    /// When the current recording began (UI clock; the session owns the file).
    recording: Option<Instant>,
    /// Last finished recording, shown until the next one replaces it.
    saved: Option<(std::path::PathBuf, u64)>,
    /// User-selected resolution cap; `None` means the native resolution.
    max_size: Option<u32>,
    /// User-selected video bit rate in bits per second.
    video_bit_rate: u32,
    texture: Option<egui::TextureHandle>,
    last_decoded: u64,
    fps: f32,
    fps_window: Option<(u64, Instant)>,
    last_auto_attempt: Option<Instant>,
    /// Throttle for the `BRIDGESCOPE_MIRROR_RECORD` dev hook.
    last_record_auto: Option<Instant>,
    /// Set once the dev hook finished its one recording cycle.
    record_auto_done: bool,
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
            BackendEvent::MirrorRecordingSaved { path, frames, .. } => {
                self.recording = None;
                self.saved = Some((path.clone(), *frames));
            }
            BackendEvent::MirrorRecordingFailed { error, .. } => {
                self.recording = None;
                self.error = Some(error.clone());
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
        // The session-side recorder is finalized (and reports itself) before
        // the stop event lands; `saved` deliberately survives so the user
        // keeps seeing the file path.
        self.recording = None;
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
            // Recording rides on the running mirror: the button sits to the
            // left of start/stop and only works while mirroring.
            let recording = state.recording.is_some();
            let record_label = if recording {
                RichText::new(format!("● {}", text(language, "mirror_record_stop")))
                    .color(egui::Color32::from_rgb(248, 113, 113))
            } else {
                RichText::new(text(language, "mirror_record"))
            };
            let record = ui
                .add_enabled(busy, egui::Button::new(record_label))
                .on_disabled_hover_text(text(language, "mirror_need_mirror"));
            if record.clicked() {
                state.error = None;
                if recording {
                    commands.push(BackendCommand::StopMirrorRecording);
                } else {
                    state.recording = Some(Instant::now());
                    state.saved = None;
                    commands.push(BackendCommand::StartMirrorRecording);
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
    if let Some(started) = state.recording {
        ui.label(
            RichText::new(format!(
                "● {} · {}s",
                text(language, "mirror_recording"),
                started.elapsed().as_secs()
            ))
            .color(egui::Color32::from_rgb(248, 113, 113)),
        );
    }
    if let Some(error) = &state.error {
        ui.label(
            RichText::new(error_text(language, error))
                .color(egui::Color32::from_rgb(248, 113, 113)),
        );
    }
    if let Some((path, frames)) = &state.saved {
        ui.label(
            RichText::new(format!(
                "{}: {} ({frames} {})",
                text(language, "mirror_record_saved"),
                path.display(),
                text(language, "mirror_record_frames")
            ))
            .small(),
        );
    }
    ui.add_space(6.0);

    ui.add_space(6.0);

    // The remote owns a fixed full-height right column; the video takes the
    // rest. An embedded side panel keeps that width exact no matter how the
    // video scales, and its separator doubles as the column divider.
    egui::SidePanel::right("mirror-remote-column")
        .resizable(false)
        .exact_width(REMOTE_COLUMN)
        .frame(egui::Frame::NONE)
        .show_inside(ui, |ui| {
            remote_control(ui, language, selected, &mut commands);
        });

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

/// The remote-control card. Works independently of mirroring: every key is
/// just an `input keyevent` against the selected device.
fn remote_control(
    ui: &mut egui::Ui,
    language: Language,
    selected: Option<&DeviceTarget>,
    commands: &mut Vec<BackendCommand>,
) {
    let palette = crate::theme::palette(ui.visuals().dark_mode);
    egui::Frame::new()
        .fill(palette.ai_bubble)
        .stroke(egui::Stroke::new(1.0, palette.bubble_stroke))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_min_width(REMOTE_COLUMN - 26.0);
            ui.strong(text(language, "mirror_remote"));
            ui.add_space(4.0);

            // Every row places itself by arithmetic (see `key_row`), so the
            // rows need no centering container around them.
            key_row(
                ui,
                language,
                selected,
                commands,
                &[
                    RemoteKey::new(text(language, "mirror_key_power"), 26),
                    RemoteKey::new(text(language, "mirror_key_mute"), 164),
                    RemoteKey::new(text(language, "mirror_key_play"), 85),
                ],
            );
            key_row(
                ui,
                language,
                selected,
                commands,
                &[
                    RemoteKey::new(text(language, "mirror_key_vol_up"), 24),
                    RemoteKey::new(text(language, "mirror_key_vol_down"), 25),
                ],
            );
            ui.add_space(4.0);

            // D-pad cluster, laid out like the real thing: three rows through
            // the same path, so ▲ OK ▼ share one axis.
            key_row(ui, language, selected, commands, &[RemoteKey::new("▲", 19)]);
            key_row(
                ui,
                language,
                selected,
                commands,
                &[
                    RemoteKey::new("◀", 21),
                    RemoteKey::new("OK", 23),
                    RemoteKey::new("▶", 22),
                ],
            );
            key_row(ui, language, selected, commands, &[RemoteKey::new("▼", 20)]);
            ui.add_space(4.0);

            key_row(
                ui,
                language,
                selected,
                commands,
                &[
                    RemoteKey::new(text(language, "mirror_key_menu"), 82),
                    RemoteKey::new(text(language, "mirror_key_home"), 3),
                    RemoteKey::new(text(language, "mirror_key_back"), 4),
                ],
            );
        });
}

/// One centered row of remote keys. The card's flow is left-aligned and egui
/// refuses to center nested groups, so the row rect is placed by arithmetic:
/// `left + (available − row) / 2`. Each key is then `put` at its exact slot.
#[allow(clippy::cast_precision_loss)]
fn key_row(
    ui: &mut egui::Ui,
    language: Language,
    selected: Option<&DeviceTarget>,
    commands: &mut Vec<BackendCommand>,
    keys: &[RemoteKey],
) {
    let gap = ui.spacing().item_spacing.x;
    let row_width = keys.len() as f32 * KEY_SIZE.x + (keys.len() - 1) as f32 * gap;
    let left = ui.cursor().left() + ((ui.available_width() - row_width) / 2.0).max(0.0);
    let row_rect = egui::Rect::from_min_size(
        egui::pos2(left, ui.cursor().top()),
        egui::vec2(row_width, KEY_SIZE.y),
    );
    ui.allocate_rect(row_rect, egui::Sense::hover());
    ui.add_enabled_ui(selected.is_some(), |ui| {
        for (index, key) in keys.iter().enumerate() {
            let slot = egui::Rect::from_min_size(
                row_rect.left_top() + egui::vec2(index as f32 * (KEY_SIZE.x + gap), 0.0),
                KEY_SIZE,
            );
            let button = egui::Button::new(RichText::new(key.label).size(14.0))
                .min_size(KEY_SIZE)
                // Ghost buttons elsewhere idle invisibly; a remote needs
                // visible keys, so each one keeps the shared subtle chip fill.
                .fill(crate::theme::palette(ui.visuals().dark_mode).chip_fill)
                .corner_radius(egui::CornerRadius::same(6));
            let response = ui
                .put(slot, button)
                .on_disabled_hover_text(text(language, "mirror_need_device"));
            if response.clicked()
                && let Some(target) = selected
            {
                commands.push(BackendCommand::SendKeyEvent {
                    target: target.clone(),
                    keycode: key.keycode,
                });
            }
        }
    });
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
    // Dev hook (`BRIDGESCOPE_MIRROR_RECORD=<secs>`): records the running
    // mirror for the given number of seconds, then stops it — verifies the
    // recording path end to end without scripted input. One-shot by design:
    // the saved banner stays visible for inspection instead of the hook
    // cycling recordings forever.
    if !state.record_auto_done
        && let Some(secs) = std::env::var_os("BRIDGESCOPE_MIRROR_RECORD")
            .and_then(|raw| raw.to_str().and_then(|value| value.parse::<u64>().ok()))
        && state
            .last_record_auto
            .is_none_or(|last| last.elapsed() >= RECORD_AUTO_STEP)
    {
        if state.running.is_some() && state.recording.is_none() {
            state.last_record_auto = Some(Instant::now());
            state.recording = Some(Instant::now());
            state.saved = None;
            commands.push(BackendCommand::StartMirrorRecording);
        } else if state
            .recording
            .is_some_and(|started| started.elapsed().as_secs() >= secs)
        {
            state.last_record_auto = Some(Instant::now());
            state.record_auto_done = true;
            commands.push(BackendCommand::StopMirrorRecording);
        }
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

    #[test]
    fn recording_events_track_the_saved_file() {
        let mut state = MirrorPanelState::new();
        let target = DeviceTarget::new(DeviceSerial::new("s").expect("serial"), 1);
        state.recording = Some(Instant::now());
        state.handle_event(&BackendEvent::MirrorRecordingSaved {
            target: target.clone(),
            path: std::path::PathBuf::from("recordings/a.mp4"),
            frames: 42,
        });
        assert!(state.recording.is_none());
        assert_eq!(
            state.saved,
            Some((std::path::PathBuf::from("recordings/a.mp4"), 42))
        );

        state.recording = Some(Instant::now());
        state.handle_event(&BackendEvent::MirrorRecordingFailed {
            target,
            error: BridgeError::new(ErrorCode::Internal, "mirror.record_write_failed", "x"),
        });
        assert!(state.recording.is_none());
        assert_eq!(
            state.error.map(|error| error.message_key),
            Some("mirror.record_write_failed".to_owned())
        );
    }

    #[test]
    fn stopping_the_session_keeps_the_saved_path_visible() {
        let mut state = MirrorPanelState::new();
        let target = DeviceTarget::new(DeviceSerial::new("s").expect("serial"), 1);
        let request_id = OperationId::new();
        state.starting = Some(request_id);
        state.handle_event(&BackendEvent::MirrorStarted {
            request_id,
            target: target.clone(),
            width: 1280,
            height: 720,
        });
        state.recording = Some(Instant::now());
        state.handle_event(&BackendEvent::MirrorRecordingSaved {
            target,
            path: std::path::PathBuf::from("recordings/b.mp4"),
            frames: 3,
        });
        state.handle_event(&BackendEvent::MirrorStopped {
            request_id,
            target: DeviceTarget::new(DeviceSerial::new("s").expect("serial"), 1),
        });
        assert!(state.running.is_none() && state.recording.is_none());
        assert!(state.saved.is_some(), "the saved path survives the stop");
    }

    /// Drives three headless egui frames: a probe frame records where the
    /// single-key row actually landed, then press and release hit its center
    /// (egui registers a click only across frames).
    fn click_remote_key(selected: Option<&DeviceTarget>) -> Vec<BackendCommand> {
        let context = egui::Context::default();
        let spot = std::cell::Cell::new((0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32));
        let mut commands: Vec<BackendCommand> = Vec::new();
        {
            let commands = &mut commands;
            let mut surface = |context: &egui::Context| {
                egui::CentralPanel::default().show(context, |ui| {
                    let (left, top, width) =
                        (ui.cursor().left(), ui.cursor().top(), ui.available_width());
                    spot.set((left, top, width, ui.cursor().height()));
                    key_row(
                        ui,
                        Language::Chinese,
                        selected,
                        commands,
                        &[RemoteKey::new("key", 24)],
                    );
                });
            };
            let _ = context.run(egui::RawInput::default(), &mut surface);
        }
        let (left, top, width, _height) = spot.get();
        let pos = egui::pos2(left + width / 2.0, top + KEY_SIZE.y / 2.0);
        let pointer = |pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        let press = egui::RawInput {
            events: vec![egui::Event::PointerMoved(pos), pointer(true)],
            ..Default::default()
        };
        let release = egui::RawInput {
            events: vec![pointer(false)],
            ..Default::default()
        };
        let mut surface = |context: &egui::Context| {
            egui::CentralPanel::default().show(context, |ui| {
                key_row(
                    ui,
                    Language::Chinese,
                    selected,
                    &mut commands,
                    &[RemoteKey::new("key", 24)],
                );
            });
        };
        let _ = context.run(press, &mut surface);
        let _ = context.run(release, &mut surface);
        commands
    }

    #[test]
    fn clicking_a_key_with_a_device_queues_one_key_event() {
        let target = DeviceTarget::new(DeviceSerial::new("s").expect("serial"), 1);
        let commands = click_remote_key(Some(&target));
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            &commands[0],
            BackendCommand::SendKeyEvent { target, keycode: 24 }
                if target.serial.as_str() == "s"
        ));
    }

    #[test]
    fn clicking_a_key_without_a_device_queues_nothing() {
        assert!(click_remote_key(None).is_empty());
    }
}
