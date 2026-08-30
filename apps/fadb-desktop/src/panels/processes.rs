use eframe::egui;
use fadb_domain::{BackendCommand, BackendEvent, DeviceTarget, ProcessInfo};

use crate::i18n::{Language, text};

#[derive(Default)]
pub struct ProcessesPanelState {
    pub target: Option<DeviceTarget>,
    pub processes: Vec<ProcessInfo>,
    pub loading: bool,
}

impl ProcessesPanelState {
    pub fn reset_for(&mut self, target: Option<DeviceTarget>) {
        if self.target != target {
            self.target = target;
            self.processes.clear();
        }
    }

    pub fn handle_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ProcessesLoading(target) if self.target.as_ref() == Some(target) => {
                self.loading = true;
            }
            BackendEvent::ProcessesLoaded(snapshot)
                if self.target.as_ref() == Some(&snapshot.target) =>
            {
                self.loading = false;
                self.processes.clone_from(&snapshot.processes);
            }
            BackendEvent::ProcessesFailed { target, .. }
                if self.target.as_ref() == Some(target) =>
            {
                self.loading = false;
            }
            _ => {}
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    language: Language,
    state: &mut ProcessesPanelState,
) -> Vec<BackendCommand> {
    let mut commands = Vec::new();
    ui.heading(text(language, "processes"));
    ui.horizontal(|ui| {
        ui.label(text(language, "process_snapshot_hint"));
        if ui
            .add_enabled(
                state.target.is_some() && !state.loading,
                egui::Button::new(text(language, "refresh")),
            )
            .clicked()
            && let Some(target) = state.target.clone()
        {
            state.loading = true;
            commands.push(BackendCommand::LoadProcesses(target));
        }
        if state.loading {
            ui.spinner();
        }
    });
    ui.add_space(8.0);

    if state.target.is_none() {
        ui.label(text(language, "select_device"));
        return commands;
    }
    if state.processes.is_empty() && !state.loading {
        ui.label(text(language, "no_processes"));
        return commands;
    }

    egui::Grid::new("processes-grid")
        .striped(true)
        .num_columns(7)
        .spacing([18.0, 8.0])
        .show(ui, |ui| {
            ui.strong(text(language, "pid"));
            ui.strong(text(language, "process_name"));
            ui.strong(text(language, "user"));
            ui.strong(text(language, "state"));
            ui.strong(text(language, "cpu"));
            ui.strong(text(language, "memory"));
            ui.strong(text(language, "resident"));
            ui.end_row();
            for process in &state.processes {
                ui.label(process.pid.to_string());
                ui.label(&process.name);
                ui.label(process.user.as_deref().unwrap_or("-"));
                ui.label(process.state.as_deref().unwrap_or("-"));
                ui.label(format_percent(process.cpu_percent));
                ui.label(format_percent(process.memory_percent));
                ui.label(format_kib(process.resident_memory_kib));
                ui.end_row();
            }
        });
    commands
}

fn format_percent(value: Option<f32>) -> String {
    value.map_or_else(|| "-".to_owned(), |value| format!("{value:.1}%"))
}

#[allow(clippy::cast_precision_loss)]
fn format_kib(value: Option<u64>) -> String {
    value.map_or_else(
        || "-".to_owned(),
        |value| format!("{:.1} MiB", value as f64 / 1024.0),
    )
}
