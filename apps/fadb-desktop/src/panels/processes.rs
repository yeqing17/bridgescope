use eframe::egui;
use fadb_domain::{BackendCommand, BackendEvent, DeviceTarget, ProcessInfo};

use crate::i18n::{Language, text};
use crate::theme;

#[derive(Default)]
pub struct ProcessesPanelState {
    pub target: Option<DeviceTarget>,
    pub processes: Vec<ProcessInfo>,
    pub loading: bool,
    /// Search box contents; matches name, PID and user, case-insensitively.
    search: String,
    /// Column the list is currently sorted by, descending. The backend list
    /// order (roughly by PID) is the neutral default.
    sort: ProcessSort,
}

/// Sortable columns; every sort is descending so hot / heavy processes lead.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ProcessSort {
    #[default]
    None,
    Cpu,
    Memory,
    Resident,
}

impl ProcessSort {
    #[allow(clippy::cast_precision_loss)] // KiB counts fit f64 exactly below 2^53
    fn key(self, process: &ProcessInfo) -> Option<f64> {
        match self {
            // Exact zero and missing data both sort to the back.
            ProcessSort::Cpu => process.cpu_percent.map(f64::from),
            ProcessSort::Memory => process.memory_percent.map(f64::from),
            ProcessSort::Resident => process.resident_memory_kib.map(|kib| kib as f64),
            ProcessSort::None => None,
        }
    }
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
        // Same flicker avoidance as the performance panel: the button stays
        // enabled while the 3-second auto snapshot is in flight, and a click
        // during a load is ignored.
        theme::button_aligned_label(ui, text(language, "process_snapshot_hint"));
        if ui
            .add_enabled(
                state.target.is_some(),
                egui::Button::new(text(language, "refresh")),
            )
            .clicked()
            && !state.loading
            && let Some(target) = state.target.clone()
        {
            state.loading = true;
            commands.push(BackendCommand::LoadProcesses(target));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.search)
                    .desired_width(170.0)
                    .hint_text(text(language, "processes_search_hint")),
            );
        });
    });
    ui.add_space(8.0);

    if state.target.is_none() {
        ui.label(text(language, "select_device"));
        return commands;
    }

    let mut rows: Vec<&ProcessInfo> = state
        .processes
        .iter()
        .filter(|process| matches_search(process, &state.search))
        .collect();
    if state.sort != ProcessSort::None {
        let key = |process: &ProcessInfo| state.sort.key(process);
        rows.sort_by(|a, b| {
            match (key(a), key(b)) {
                (Some(x), Some(y)) => y.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal),
                // Missing data always sinks to the bottom.
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });
    }

    if rows.is_empty() {
        ui.label(if state.processes.is_empty() {
            text(language, "no_processes")
        } else {
            text(language, "processes_no_match")
        });
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
            sort_header(ui, language, "cpu", ProcessSort::Cpu, &mut state.sort);
            sort_header(ui, language, "memory", ProcessSort::Memory, &mut state.sort);
            sort_header(
                ui,
                language,
                "resident",
                ProcessSort::Resident,
                &mut state.sort,
            );
            ui.end_row();
            for process in rows {
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

/// Case-insensitive substring match over the process name, PID and user; an
/// empty needle matches everything.
fn matches_search(process: &ProcessInfo, needle: &str) -> bool {
    let needle = needle.trim().to_lowercase();
    needle.is_empty()
        || process.name.to_lowercase().contains(&needle)
        || process.pid.to_string().contains(&needle)
        || process
            .user
            .as_deref()
            .unwrap_or("")
            .to_lowercase()
            .contains(&needle)
}

/// A clickable grid column header for one sortable column: clicking sorts by
/// it (descending), clicking the active one again returns to backend order.
/// An arrow marks the active column.
fn sort_header(
    ui: &mut egui::Ui,
    language: Language,
    label_key: &'static str,
    column: ProcessSort,
    sort: &mut ProcessSort,
) {
    let active = *sort == column;
    let marker = if active { " ▼" } else { "" };
    let response = ui
        .add(
            egui::Button::new(
                egui::RichText::new(format!("{}{marker}", text(language, label_key))).strong(),
            )
            .frame(false),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(text(language, "processes_sort_hint"));
    if response.clicked() {
        *sort = if active { ProcessSort::None } else { column };
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32, name: &str, cpu: Option<f32>, resident: Option<u64>) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.to_owned(),
            user: None,
            state: None,
            cpu_percent: cpu,
            memory_percent: None,
            resident_memory_kib: resident,
        }
    }

    #[test]
    fn search_matches_name_pid_and_user_case_insensitively() {
        let process = ProcessInfo {
            pid: 4321,
            name: "system_server".to_owned(),
            user: Some("system".to_owned()),
            state: None,
            cpu_percent: None,
            memory_percent: None,
            resident_memory_kib: None,
        };
        assert!(matches_search(&process, ""));
        assert!(matches_search(&process, "SYSTEM_SERVER"));
        assert!(matches_search(&process, "432"));
        assert!(matches_search(&process, "Syst"));
        assert!(!matches_search(&process, "webview"));
    }

    #[test]
    fn resident_sort_orders_descending_with_missing_last() {
        let state = ProcessesPanelState {
            processes: vec![
                process(1, "small", Some(1.0), Some(1024)),
                process(2, "big", Some(9.0), Some(8192)),
                process(3, "unknown", Some(0.5), None),
            ],
            ..ProcessesPanelState::default()
        };
        let mut rows: Vec<&ProcessInfo> = state.processes.iter().collect();
        rows.sort_by(
            |a, b| match (ProcessSort::Resident.key(a), ProcessSort::Resident.key(b)) {
                (Some(x), Some(y)) => y.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            },
        );
        let pids: Vec<u32> = rows.iter().map(|process| process.pid).collect();
        assert_eq!(pids, [2, 1, 3]);
    }
}
