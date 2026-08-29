use std::time::{Duration, Instant};

use bridgescope_domain::{
    AvdEntry, BackendCommand, BackendEvent, BridgeError, DeviceSerial, OperationId,
};
use eframe::egui::{self, RichText};

use crate::i18n::{Language, error_text, text};

/// Backoff between automatic AVD list loads while the panel is open and empty.
const AUTO_LOAD_RETRY: Duration = Duration::from_secs(5);
/// Spacing between device-list re-polls while waiting for a launched AVD to
/// come online (about a minute of patience in total).
const BOOT_POLL_INTERVAL: Duration = Duration::from_secs(5);
const BOOT_POLLS: u32 = 12;
/// Spacing between list/device resyncs after an emulator was asked to exit.
const SETTLE_POLL_INTERVAL: Duration = Duration::from_secs(3);
const SETTLE_POLLS: u32 = 3;

#[derive(Default)]
pub struct AvdPanelState {
    avds: Vec<AvdEntry>,
    loading: bool,
    error: Option<BridgeError>,
    /// Launch/kill notice stored as an i18n key.
    notice: Option<&'static str>,
    last_load_attempt: Option<Instant>,
    /// Polls left while waiting for a launched emulator to appear online.
    boot_polls: u32,
    last_poll: Option<Instant>,
    /// Polls left to resync after a kill.
    settle_polls: u32,
    /// An emulator stop awaiting confirmation.
    confirm_stop: Option<DeviceSerial>,
}

impl AvdPanelState {
    pub fn handle_event(&mut self, event: &BackendEvent) -> Vec<BackendCommand> {
        let mut commands = Vec::new();
        match event {
            BackendEvent::AvdsLoaded { avds } => {
                self.loading = false;
                self.error = None;
                self.avds.clone_from(avds);
            }
            BackendEvent::AvdsFailed { error } => {
                self.loading = false;
                self.error = Some(error.clone());
            }
            BackendEvent::AvdLaunchFinished { .. } => {
                self.notice = Some("avd_started_hint");
                self.boot_polls = BOOT_POLLS;
                self.last_poll = Some(Instant::now());
            }
            BackendEvent::AvdLaunchFailed { error, .. } => {
                self.notice = None;
                self.error = Some(error.clone());
            }
            BackendEvent::EmulatorKillFinished { .. } => {
                self.notice = Some("avd_kill_done");
                self.confirm_stop = None;
                self.settle_polls = SETTLE_POLLS;
                self.last_poll = Some(Instant::now());
                commands.push(BackendCommand::RefreshDevices);
            }
            BackendEvent::EmulatorKillFailed { error, .. } => {
                self.notice = None;
                self.confirm_stop = None;
                self.error = Some(error.clone());
            }
            _ => {}
        }
        commands
    }
}

/// Renders the AVD manager: the emulator list with per-AVD launch/stop
/// actions and boot/settle polling.
pub fn show(
    ui: &mut egui::Ui,
    language: Language,
    state: &mut AvdPanelState,
) -> Vec<BackendCommand> {
    let mut commands = Vec::new();
    ui.horizontal(|ui| {
        ui.heading(text(language, "avd"));
        if state.loading {
            ui.spinner();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(!state.loading, egui::Button::new(text(language, "refresh")))
                .clicked()
            {
                state.loading = true;
                commands.push(BackendCommand::ListAvds);
            }
        });
    });
    ui.add_space(4.0);

    if let Some(error) = &state.error {
        ui.label(
            RichText::new(error_text(language, error))
                .color(egui::Color32::from_rgb(248, 113, 113)),
        );
    }
    if let Some(key) = state.notice {
        ui.label(RichText::new(text(language, key)).color(egui::Color32::from_rgb(74, 222, 128)));
    }
    ui.add_space(4.0);

    if state.avds.is_empty() && !state.loading && state.error.is_none() {
        ui.label(text(language, "avd_none"));
        return commands;
    }

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .id_salt("avd-list")
        .show(ui, |ui| {
            egui::Grid::new("avd-grid")
                .striped(true)
                .num_columns(3)
                .min_col_width(120.0)
                .show(ui, |ui| {
                    ui.strong(text(language, "avd_name"));
                    ui.strong(text(language, "avd_status"));
                    ui.strong(text(language, "action"));
                    ui.end_row();
                    for entry in state.avds.clone() {
                        ui.label(&entry.name);
                        match &entry.running_serial {
                            Some(serial) => {
                                ui.label(format!(
                                    "{} · {}",
                                    text(language, "avd_running"),
                                    serial.as_str()
                                ));
                            }
                            None => {
                                ui.label(text(language, "avd_stopped"));
                            }
                        }
                        ui.horizontal(|ui| {
                            avd_actions(ui, language, state, &entry, &mut commands);
                        });
                        ui.end_row();
                    }
                });
        });
    ui.add_space(6.0);
    ui.small(text(language, "avd_hint"));
    commands
}

fn avd_actions(
    ui: &mut egui::Ui,
    language: Language,
    state: &mut AvdPanelState,
    entry: &AvdEntry,
    commands: &mut Vec<BackendCommand>,
) {
    if let Some(serial) = &entry.running_serial {
        if state.confirm_stop.as_ref() == Some(serial) {
            if ui.button(text(language, "confirm")).clicked() {
                commands.push(BackendCommand::KillEmulator {
                    request_id: OperationId::new(),
                    serial: serial.clone(),
                });
            }
            if ui.button(text(language, "cancel")).clicked() {
                state.confirm_stop = None;
            }
        } else if ui.button(text(language, "avd_stop")).clicked() {
            state.confirm_stop = Some(serial.clone());
        }
        return;
    }
    if ui
        .button(text(language, "avd_launch"))
        .on_hover_text(text(language, "avd_launch_hint"))
        .clicked()
    {
        state.notice = Some("avd_starting");
        commands.push(BackendCommand::LaunchAvd {
            request_id: OperationId::new(),
            name: entry.name.clone(),
            wipe_data: false,
        });
    }
    if ui
        .button(text(language, "avd_launch_wipe"))
        .on_hover_text(text(language, "avd_launch_wipe_hint"))
        .clicked()
    {
        state.notice = Some("avd_starting");
        commands.push(BackendCommand::LaunchAvd {
            request_id: OperationId::new(),
            name: entry.name.clone(),
            wipe_data: true,
        });
    }
}

/// Called by the app shell each frame: loads the AVD list when the panel is
/// first shown, and keeps device state fresh right after a launch or kill.
pub fn auto(state: &mut AvdPanelState) -> Vec<BackendCommand> {
    let mut commands = Vec::new();
    if state.boot_polls > 0
        && state
            .last_poll
            .is_some_and(|last| last.elapsed() >= BOOT_POLL_INTERVAL)
    {
        state.boot_polls -= 1;
        state.last_poll = Some(Instant::now());
        commands.push(BackendCommand::RefreshDevices);
        // A new device also changes which AVD entry counts as running.
        commands.push(BackendCommand::ListAvds);
    }
    if state.settle_polls > 0
        && state
            .last_poll
            .is_some_and(|last| last.elapsed() >= SETTLE_POLL_INTERVAL)
    {
        state.settle_polls -= 1;
        state.last_poll = Some(Instant::now());
        commands.push(BackendCommand::ListAvds);
    }
    let empty_and_idle = state.avds.is_empty() && !state.loading && state.error.is_none();
    let recently_attempted = state
        .last_load_attempt
        .is_some_and(|last| last.elapsed() < AUTO_LOAD_RETRY);
    if empty_and_idle && !recently_attempted {
        state.last_load_attempt = Some(Instant::now());
        state.loading = true;
        commands.push(BackendCommand::ListAvds);
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_event_starts_boot_polling() {
        let mut state = AvdPanelState::default();
        state.handle_event(&BackendEvent::AvdsLoaded {
            avds: vec![AvdEntry {
                name: "Pixel_9a".to_owned(),
                running_serial: None,
            }],
        });
        state.handle_event(&BackendEvent::AvdLaunchFinished {
            request_id: OperationId::new(),
            name: "Pixel_9a".to_owned(),
        });
        let commands = auto(&mut state);
        // The first poll is deferred by the interval, so nothing yet.
        assert!(commands.is_empty());
        assert_eq!(state.boot_polls, BOOT_POLLS);
        assert_eq!(state.notice, Some("avd_started_hint"));
    }

    #[test]
    fn kill_event_confirms_stops_and_schedules_resync() {
        let mut state = AvdPanelState::default();
        let serial = DeviceSerial::new("emulator-5554").expect("serial");
        let commands = state.handle_event(&BackendEvent::EmulatorKillFinished {
            request_id: OperationId::new(),
            serial: serial.clone(),
        });
        assert!(commands.contains(&BackendCommand::RefreshDevices));
        assert_eq!(state.confirm_stop, None);
        assert_eq!(state.settle_polls, SETTLE_POLLS);
    }

    #[test]
    fn auto_loads_once_and_backs_off() {
        let mut state = AvdPanelState::default();
        let commands = auto(&mut state);
        assert_eq!(commands.len(), 1);
        assert!(matches!(commands[0], BackendCommand::ListAvds));
        state.handle_event(&BackendEvent::AvdsLoaded { avds: Vec::new() });
        // Still empty, but inside the backoff window: no repeat load.
        assert!(auto(&mut state).is_empty());
        assert!(!state.loading);
    }
}
