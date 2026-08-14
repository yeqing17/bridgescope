use std::path::PathBuf;

use bridgescope_domain::{
    BackendCommand, BackendEvent, BridgeError, DeviceTarget, ErrorCode, FileTransferDirection,
    OperationId, OverwritePolicy, RemoteFileEntry, RemoteFileKind, RemotePath,
};
use eframe::egui;

#[derive(Clone)]
struct TransferIntent {
    direction: FileTransferDirection,
    target: DeviceTarget,
    local_path: PathBuf,
    remote_path: RemotePath,
}

#[derive(Clone, Copy)]
enum MutationModalKind {
    CreateDirectory,
    Rename,
    Delete,
}

#[derive(Clone)]
struct MutationModal {
    kind: MutationModalKind,
    input: String,
    entry: Option<RemoteFileEntry>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SortKey {
    #[default]
    Name,
    Size,
    Modified,
}

fn sort_header(ui: &mut egui::Ui, state: &mut FilesPanelState, key: SortKey, label: &str) {
    let arrow = match (state.sort_key == key, state.sort_reverse) {
        (true, false) => " ▼",
        (true, true) => " ▲",
        (false, _) => "",
    };
    if ui.small_button(format!("{label}{arrow}")).clicked() {
        if state.sort_key == key {
            state.sort_reverse = !state.sort_reverse;
        } else {
            state.sort_key = key;
            state.sort_reverse = false;
        }
    }
}

#[derive(Default)]
pub struct FilesPanelState {
    target: Option<DeviceTarget>,
    directory: Option<RemotePath>,
    path_input: String,
    entries: Vec<RemoteFileEntry>,
    selected: Option<usize>,
    history: Vec<RemotePath>,
    sort_key: SortKey,
    sort_reverse: bool,
    listing_request: Option<OperationId>,
    loading: bool,
    transfer: Option<OperationId>,
    transfer_intent: Option<TransferIntent>,
    overwrite_prompt: Option<TransferIntent>,
    mutation: Option<OperationId>,
    mutation_modal: Option<MutationModal>,
    error: Option<String>,
}

impl FilesPanelState {
    pub fn reconcile_target(&mut self, target: Option<DeviceTarget>) -> Option<BackendCommand> {
        if self.target == target {
            return None;
        }
        self.target.clone_from(&target);
        self.directory = None;
        self.path_input.clear();
        self.entries.clear();
        self.selected = None;
        self.history.clear();
        self.listing_request = None;
        self.loading = false;
        self.transfer = None;
        self.transfer_intent = None;
        self.overwrite_prompt = None;
        self.mutation = None;
        self.mutation_modal = None;
        self.error = None;
        target.map(|target| {
            self.list(
                target,
                RemotePath::new("/sdcard").expect("valid default path"),
            )
        })
    }

    #[allow(clippy::too_many_lines)]
    pub fn handle_event(&mut self, event: &BackendEvent) -> Vec<BackendCommand> {
        let mut commands = Vec::new();
        match event {
            BackendEvent::DirectoryLoading {
                request_id,
                target,
                path,
            } if self.listing_request.as_ref() == Some(request_id)
                && self.target.as_ref() == Some(target) =>
            {
                self.directory = Some(path.clone());
                self.path_input = path.to_string();
                self.loading = true;
                self.error = None;
            }
            BackendEvent::DirectoryLoaded {
                request_id,
                listing,
            } if self.listing_request.as_ref() == Some(request_id)
                && self.target.as_ref() == Some(&listing.target) =>
            {
                self.directory = Some(listing.directory.clone());
                self.path_input = listing.directory.to_string();
                self.entries.clone_from(&listing.entries);
                self.selected = None;
                self.loading = false;
                self.error = None;
            }
            BackendEvent::DirectoryFailed {
                request_id,
                target,
                error,
                ..
            } if self.listing_request.as_ref() == Some(request_id)
                && self.target.as_ref() == Some(target) =>
            {
                self.loading = false;
                self.error = Some(format_error(error));
            }
            BackendEvent::FileTransferStarted {
                request_id, target, ..
            } if self.transfer.as_ref() == Some(request_id)
                && self.target.as_ref() == Some(target) => {}
            BackendEvent::FileTransferCompleted {
                request_id,
                summary,
            } if self.transfer.as_ref() == Some(request_id)
                && self.target.as_ref() == Some(&summary.target) =>
            {
                self.transfer = None;
                self.transfer_intent = None;
                if let Some(directory) = self.directory.clone() {
                    commands.push(self.list(summary.target.clone(), directory));
                }
            }
            BackendEvent::FileTransferFailed {
                request_id,
                target,
                error,
            } if self.transfer.as_ref() == Some(request_id)
                && self.target.as_ref() == Some(target) =>
            {
                self.transfer = None;
                let intent = self.transfer_intent.take();
                if error.code == ErrorCode::AlreadyExists
                    && let Some(intent) = intent
                {
                    self.overwrite_prompt = Some(intent);
                } else {
                    self.error = Some(format_error(error));
                }
            }
            BackendEvent::FileTransferCancelled { request_id, target }
                if self.transfer.as_ref() == Some(request_id)
                    && self.target.as_ref() == Some(target) =>
            {
                self.transfer = None;
                self.transfer_intent = None;
            }
            BackendEvent::FileMutationStarted {
                request_id, target, ..
            } if self.mutation.as_ref() == Some(request_id)
                && self.target.as_ref() == Some(target) => {}
            BackendEvent::FileMutationCompleted {
                request_id,
                summary,
            } if self.mutation.as_ref() == Some(request_id)
                && self.target.as_ref() == Some(&summary.target) =>
            {
                self.mutation = None;
                self.mutation_modal = None;
                self.error = None;
                if let Some(directory) = self.directory.clone() {
                    commands.push(self.list(summary.target.clone(), directory));
                }
            }
            BackendEvent::FileMutationFailed {
                request_id,
                target,
                error,
            } if self.mutation.as_ref() == Some(request_id)
                && self.target.as_ref() == Some(target) =>
            {
                self.mutation = None;
                self.error = Some(format_error(error));
            }
            _ => {}
        }
        commands
    }

    /// Navigate to `path`, remembering the current directory so "Back" can return.
    fn navigate(&mut self, target: DeviceTarget, path: RemotePath) -> BackendCommand {
        if let Some(directory) = &self.directory
            && *directory != path
        {
            self.history.push(directory.clone());
        }
        self.list(target, path)
    }

    fn list(&mut self, target: DeviceTarget, path: RemotePath) -> BackendCommand {
        let request_id = OperationId::new();
        self.listing_request = Some(request_id);
        self.loading = true;
        BackendCommand::ListDirectory {
            request_id,
            target,
            path,
        }
    }

    fn start_transfer(
        &mut self,
        intent: TransferIntent,
        overwrite: OverwritePolicy,
    ) -> BackendCommand {
        let request_id = OperationId::new();
        let command = match intent.direction {
            FileTransferDirection::Upload => BackendCommand::UploadFile {
                request_id,
                target: intent.target.clone(),
                local_path: intent.local_path.clone(),
                remote_path: intent.remote_path.clone(),
                overwrite,
            },
            FileTransferDirection::Download => BackendCommand::DownloadFile {
                request_id,
                target: intent.target.clone(),
                remote_path: intent.remote_path.clone(),
                local_path: intent.local_path.clone(),
                overwrite,
            },
        };
        self.transfer = Some(request_id);
        self.transfer_intent = Some(intent);
        self.error = None;
        command
    }

    fn start_mutation(&mut self, command: BackendCommand) -> BackendCommand {
        self.mutation = Some(match &command {
            BackendCommand::CreateDirectory { request_id, .. }
            | BackendCommand::RenameRemoteEntry { request_id, .. }
            | BackendCommand::DeleteRemoteFile { request_id, .. } => *request_id,
            _ => unreachable!("file panel only creates mutation commands"),
        });
        self.error = None;
        command
    }

    fn selected_entry(&self) -> Option<&RemoteFileEntry> {
        self.selected.and_then(|index| self.entries.get(index))
    }
}

fn format_error(error: &BridgeError) -> String {
    format!("{}: {}", error.message_key, error.detail)
}

fn sort_entries(entries: &mut [RemoteFileEntry], key: SortKey, reverse: bool) {
    entries.sort_by(|left, right| {
        let directory_first = u8::from(left.kind != RemoteFileKind::Directory)
            .cmp(&u8::from(right.kind != RemoteFileKind::Directory));
        let ordering = match key {
            SortKey::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
            SortKey::Size => left.size_bytes.cmp(&right.size_bytes),
            SortKey::Modified => {
                left.modified_unix_seconds.cmp(&right.modified_unix_seconds)
            }
        };
        let ordered = if reverse { ordering.reverse() } else { ordering };
        directory_first.then(ordered)
    });
}

/// Seconds between local time and UTC right now, used to render device
/// timestamps in the user's local time without pulling in a time crate.
#[cfg(windows)]
fn local_utc_offset_seconds() -> i64 {
    #[derive(Clone, Copy)]
    #[repr(C)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    unsafe extern "system" {
        fn GetSystemTime(system_time: *mut SystemTime) -> ();
        fn GetLocalTime(system_time: *mut SystemTime) -> ();
    }
    let mut system = SystemTime {
        year: 0,
        month: 0,
        day_of_week: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        milliseconds: 0,
    };
    let mut local = system;
    unsafe {
        GetSystemTime(&mut system);
        GetLocalTime(&mut local);
    }
    let to_seconds = |time: &SystemTime| {
        days_from_civil(i64::from(time.year), i64::from(time.month), i64::from(time.day))
            .saturating_mul(86_400)
            .saturating_add(i64::from(time.hour) * 3_600)
            .saturating_add(i64::from(time.minute) * 60)
            .saturating_add(i64::from(time.second))
    };
    to_seconds(&local).saturating_sub(to_seconds(&system))
}

#[cfg(not(windows))]
fn local_utc_offset_seconds() -> i64 {
    0
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + (if month > 2 { -3 } else { 9 })) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Break days since 1970-01-01 back into (year, month, day).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { year + 1 } else { year }, month as u32, day as u32)
}

fn format_modified_time(unix_seconds: i64) -> String {
    let local = unix_seconds + local_utc_offset_seconds();
    let days = local.div_euclid(86_400);
    let seconds_of_day = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        seconds_of_day / 3_600,
        seconds_of_day % 3_600 / 60
    )
}

#[allow(clippy::too_many_lines)]
pub fn show(
    ui: &mut egui::Ui,
    state: &mut FilesPanelState,
    target: Option<&DeviceTarget>,
) -> Vec<BackendCommand> {
    let mut commands = Vec::new();
    let Some(target) = target.cloned() else {
        ui.centered_and_justified(|ui| ui.label("Select an online device to browse files."));
        return commands;
    };

    ui.horizontal(|ui| {
        if ui.button("Back").clicked() {
            if let Some(previous) = state.history.pop() {
                commands.push(state.list(target.clone(), previous));
            } else if let Some(directory) = state.directory.clone() {
                commands.push(state.list(target.clone(), directory.parent()));
            }
        }
        if ui.button("Up").clicked()
            && let Some(directory) = state.directory.clone()
        {
            commands.push(state.navigate(target.clone(), directory.parent()));
        }
        if ui.button("Refresh").clicked()
            && let Some(directory) = state.directory.clone()
        {
            commands.push(state.list(target.clone(), directory));
        }
        let response =
            ui.add(egui::TextEdit::singleline(&mut state.path_input).desired_width(260.0));
        if (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
            || ui.button("Go").clicked()
        {
            match RemotePath::new(state.path_input.clone()) {
                Ok(path) => commands.push(state.navigate(target.clone(), path)),
                Err(error) => state.error = Some(error.to_string()),
            }
        }
        if ui
            .add_enabled(state.mutation.is_none(), egui::Button::new("New folder"))
            .clicked()
        {
            state.mutation_modal = Some(MutationModal {
                kind: MutationModalKind::CreateDirectory,
                input: String::new(),
                entry: None,
            });
        }
        if ui
            .add_enabled(state.transfer.is_none(), egui::Button::new("Upload"))
            .clicked()
            && let Some(directory) = state.directory.clone()
            && let Some(local_path) = rfd::FileDialog::new().set_title("Upload file").pick_file()
        {
            match local_path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| directory.join_component(name))
            {
                Some(Ok(remote_path)) => {
                    commands.push(state.start_transfer(
                        TransferIntent {
                            direction: FileTransferDirection::Upload,
                            target: target.clone(),
                            local_path,
                            remote_path,
                        },
                        OverwritePolicy::Deny,
                    ));
                }
                Some(Err(error)) => state.error = Some(error.to_string()),
                None => {
                    state.error =
                        Some("Upload failed: local file name is not valid UTF-8.".to_owned());
                }
            }
        }
        let can_download = state.transfer.is_none()
            && state
                .selected_entry()
                .is_some_and(|entry| entry.kind == RemoteFileKind::File);
        if ui
            .add_enabled(can_download, egui::Button::new("Download"))
            .clicked()
            && let Some(entry) = state.selected_entry().cloned()
            && let Some(local_path) = rfd::FileDialog::new()
                .set_title("Download file")
                .set_file_name(&entry.name)
                .save_file()
        {
            commands.push(state.start_transfer(
                TransferIntent {
                    direction: FileTransferDirection::Download,
                    target: target.clone(),
                    local_path,
                    remote_path: entry.path,
                },
                OverwritePolicy::Deny,
            ));
        }
        if let Some(request_id) = state.transfer
            && ui.button("Cancel transfer").clicked()
        {
            commands.push(BackendCommand::CancelFileOperation(request_id));
        }
    });
    ui.separator();
    if state.loading {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Loading…");
        });
    }
    let mut entries = state.entries.clone();
    sort_entries(&mut entries, state.sort_key, state.sort_reverse);
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("files-grid").striped(true).show(ui, |ui| {
            sort_header(ui, state, SortKey::Name, "Name");
            ui.strong("Type");
            sort_header(ui, state, SortKey::Size, "Size");
            sort_header(ui, state, SortKey::Modified, "Modified");
            ui.end_row();
            for (index, entry) in entries.iter().enumerate() {
                let selected = state.selected == Some(index);
                let response = ui.selectable_label(selected, &entry.name);
                if response.clicked() {
                    state.selected = Some(index);
                }
                if response.double_clicked() && entry.kind == RemoteFileKind::Directory {
                    commands.push(state.navigate(target.clone(), entry.path.clone()));
                }
                ui.label(format!("{:?}", entry.kind));
                ui.label(
                    entry
                        .size_bytes
                        .map_or("—".to_owned(), |size| size.to_string()),
                );
                ui.label(entry.modified_unix_seconds.map_or_else(
                    || "—".to_owned(),
                    |seconds| format_modified_time(seconds),
                ));
                ui.end_row();
            }
        });
    });
    ui.horizontal(|ui| {
        let selected = state.selected_entry().cloned();
        let can_mutate = state.mutation.is_none();
        if ui
            .add_enabled(
                can_mutate && selected.is_some(),
                egui::Button::new("Rename"),
            )
            .clicked()
            && let Some(entry) = selected.clone()
        {
            state.mutation_modal = Some(MutationModal {
                kind: MutationModalKind::Rename,
                input: entry.name.clone(),
                entry: Some(entry),
            });
        }
        let can_delete = can_mutate && selected.is_some();
        if ui
            .add_enabled(can_delete, egui::Button::new("Delete"))
            .clicked()
        {
            state.mutation_modal = Some(MutationModal {
                kind: MutationModalKind::Delete,
                input: String::new(),
                entry: selected.clone(),
            });
        }
    });
    if let Some(error) = &state.error {
        ui.colored_label(egui::Color32::LIGHT_RED, error);
    }
    if state.transfer.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Transferring…");
        });
    }

    if let Some(intent) = state.overwrite_prompt.clone() {
        let mut open = true;
        let (title, message) = match intent.direction {
            FileTransferDirection::Upload => (
                "Overwrite remote file?",
                format!("{} already exists. Replace it?", intent.remote_path),
            ),
            FileTransferDirection::Download => (
                "Overwrite local file?",
                format!(
                    "{} already exists. Replace it?",
                    intent.local_path.display()
                ),
            ),
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label(message);
                ui.horizontal(|ui| {
                    if ui.button("Replace").clicked() {
                        state.overwrite_prompt = None;
                        commands.push(
                            state.start_transfer(intent.clone(), OverwritePolicy::ReplaceConfirmed),
                        );
                    }
                    if ui.button("Cancel").clicked() {
                        state.overwrite_prompt = None;
                    }
                });
            });
        if !open {
            state.overwrite_prompt = None;
        }
    }

    if let Some(mut modal) = state.mutation_modal.take() {
        let kind = modal.kind;
        let title = match kind {
            MutationModalKind::CreateDirectory => "New folder",
            MutationModalKind::Rename => "Rename",
            MutationModalKind::Delete => "Delete file?",
        };
        let mut open = true;
        let mut submitted = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                if matches!(kind, MutationModalKind::Delete) {
                    ui.label("Delete the selected entry? This cannot be undone.");
                } else {
                    ui.add(egui::TextEdit::singleline(&mut modal.input).desired_width(260.0));
                }
                ui.horizontal(|ui| {
                    let confirm = if matches!(kind, MutationModalKind::Delete) {
                        ui.button("Delete").clicked()
                    } else {
                        ui.button("Confirm").clicked()
                    };
                    if confirm {
                        let request_id = OperationId::new();
                        let command = match (kind, modal.entry.clone()) {
                            (MutationModalKind::CreateDirectory, _) => state
                                .directory
                                .clone()
                                .and_then(|directory| directory.join_component(&modal.input).ok())
                                .map(|path| BackendCommand::CreateDirectory {
                                    request_id,
                                    target: target.clone(),
                                    path,
                                }),
                            (MutationModalKind::Rename, Some(entry)) => state
                                .directory
                                .clone()
                                .and_then(|directory| directory.join_component(&modal.input).ok())
                                .map(|destination| BackendCommand::RenameRemoteEntry {
                                    request_id,
                                    target: target.clone(),
                                    source: entry.path,
                                    destination,
                                }),
                            (MutationModalKind::Delete, Some(entry)) => {
                                Some(BackendCommand::DeleteRemoteFile {
                                    request_id,
                                    target: target.clone(),
                                    path: entry.path,
                                    confirmed: true,
                                })
                            }
                            _ => None,
                        };
                        if let Some(command) = command {
                            commands.push(state.start_mutation(command));
                            submitted = true;
                        } else {
                            state.error = Some("Invalid remote name or selection.".to_owned());
                        }
                    }
                });
                if ui.button("Cancel").clicked() {
                    submitted = true;
                }
            });
        if open && !submitted {
            state.mutation_modal = Some(modal);
        }
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_change_clears_listing() {
        let mut state = FilesPanelState::default();
        let serial = bridgescope_domain::DeviceSerial::new("a").expect("valid");
        let target = DeviceTarget::new(serial, 1);
        let _ = state.reconcile_target(Some(target));
        assert!(state.loading);
    }

    #[test]
    fn navigation_records_history_for_back() {
        let mut state = FilesPanelState::default();
        let serial = bridgescope_domain::DeviceSerial::new("a").expect("valid");
        let target = DeviceTarget::new(serial, 1);
        let _ = state.reconcile_target(Some(target.clone()));
        state.directory = Some(RemotePath::new("/sdcard").expect("valid"));
        let _ = state.navigate(
            target.clone(),
            RemotePath::new("/sdcard/DCIM").expect("valid"),
        );
        assert_eq!(state.history.len(), 1);
        // Refreshing the same directory must not pollute history.
        let _ = state.list(target, RemotePath::new("/sdcard/DCIM").expect("valid"));
        assert_eq!(state.history.len(), 1);
        let back = state.history.pop().expect("history entry");
        assert_eq!(back.to_string(), "/sdcard");
    }

    fn entry(
        name: &str,
        kind: RemoteFileKind,
        size: Option<u64>,
        modified: Option<i64>,
    ) -> RemoteFileEntry {
        RemoteFileEntry {
            path: RemotePath::new(format!("/sdcard/{name}")).expect("valid"),
            name: name.to_owned(),
            kind,
            size_bytes: size,
            modified_unix_seconds: modified,
            permissions: None,
        }
    }

    #[test]
    fn sorts_entries_by_key_with_directories_first() {
        let mut entries = vec![
            entry("b.txt", RemoteFileKind::File, Some(2), Some(200)),
            entry("dir", RemoteFileKind::Directory, None, None),
            entry("a.txt", RemoteFileKind::File, Some(1), Some(100)),
        ];
        sort_entries(&mut entries, SortKey::Name, false);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, vec!["dir", "a.txt", "b.txt"]);
        sort_entries(&mut entries, SortKey::Modified, false);
        assert_eq!(entries[0].name, "dir");
        assert_eq!(entries[1].name, "a.txt");
        sort_entries(&mut entries, SortKey::Modified, true);
        assert_eq!(entries[0].name, "dir");
        assert_eq!(entries[1].name, "b.txt");
    }

    #[test]
    fn civil_date_conversion_round_trips() {
        for days in [-25_000, -1, 0, 19_000, 20_000] {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(
                days_from_civil(year, i64::from(month), i64::from(day)),
                days
            );
        }
    }

    #[test]
    fn formats_known_timestamp() {
        let local = 1_700_000_000 + local_utc_offset_seconds();
        assert_eq!(format_modified_time(local), "2023-11-14 22:13");
    }
}
