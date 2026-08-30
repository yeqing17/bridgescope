use std::path::PathBuf;

use eframe::egui;
use fadb_domain::{
    BackendCommand, BackendEvent, BridgeError, DeviceTarget, ErrorCode, FileTransferDirection,
    OperationId, OverwritePolicy, RemoteFileEntry, RemoteFileKind, RemotePath,
};

use crate::i18n::{Language, error_text, text};

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

fn sort_header(
    ui: &mut egui::Ui,
    language: Language,
    state: &mut FilesPanelState,
    key: SortKey,
    label: &str,
) {
    let arrow = match (state.sort_key == key, state.sort_reverse) {
        (true, false) => " ▼",
        (true, true) => " ▲",
        (false, _) => "",
    };
    if ui
        .small_button(format!("{}{arrow}", text(language, label)))
        .clicked()
    {
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
    pub fn handle_event(
        &mut self,
        language: Language,
        event: &BackendEvent,
    ) -> Vec<BackendCommand> {
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
                self.error = Some(format_error(language, error));
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
                    self.error = Some(format_error(language, error));
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
                self.error = Some(format_error(language, error));
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

fn format_error(language: Language, error: &BridgeError) -> String {
    error_text(language, error)
}

fn sort_entries(entries: &mut [RemoteFileEntry], key: SortKey, reverse: bool) {
    entries.sort_by(|left, right| {
        let directory_first = u8::from(left.kind != RemoteFileKind::Directory)
            .cmp(&u8::from(right.kind != RemoteFileKind::Directory));
        let ordering = match key {
            SortKey::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
            SortKey::Size => left.size_bytes.cmp(&right.size_bytes),
            SortKey::Modified => left.modified_unix_seconds.cmp(&right.modified_unix_seconds),
        };
        let ordered = if reverse {
            ordering.reverse()
        } else {
            ordering
        };
        directory_first.then(ordered)
    });
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
    (
        if month <= 2 { year + 1 } else { year },
        u32::try_from(month).expect("month is 1..=12"),
        u32::try_from(day).expect("day is 1..=31"),
    )
}

fn format_modified_time(unix_seconds: i64) -> String {
    let days = unix_seconds.div_euclid(86_400);
    let seconds_of_day = unix_seconds.rem_euclid(86_400);
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
    language: Language,
    state: &mut FilesPanelState,
    target: Option<&DeviceTarget>,
) -> Vec<BackendCommand> {
    let mut commands = Vec::new();
    let Some(target) = target.cloned() else {
        ui.centered_and_justified(|ui| ui.label(text(language, "files_select_device")));
        return commands;
    };

    ui.horizontal(|ui| {
        if ui.button(text(language, "files_back")).clicked() {
            if let Some(previous) = state.history.pop() {
                commands.push(state.list(target.clone(), previous));
            } else if let Some(directory) = state.directory.clone() {
                commands.push(state.list(target.clone(), directory.parent()));
            }
        }
        if ui.button(text(language, "files_up")).clicked()
            && let Some(directory) = state.directory.clone()
        {
            commands.push(state.navigate(target.clone(), directory.parent()));
        }
        if ui.button(text(language, "refresh")).clicked()
            && let Some(directory) = state.directory.clone()
        {
            commands.push(state.list(target.clone(), directory));
        }
        let response =
            ui.add(egui::TextEdit::singleline(&mut state.path_input).desired_width(260.0));
        if (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
            || ui.button(text(language, "files_go")).clicked()
        {
            match RemotePath::new(state.path_input.clone()) {
                Ok(path) => commands.push(state.navigate(target.clone(), path)),
                Err(error) => state.error = Some(format_error(language, &error)),
            }
        }
        if ui
            .add_enabled(
                state.mutation.is_none(),
                egui::Button::new(text(language, "files_new_folder")),
            )
            .clicked()
        {
            state.mutation_modal = Some(MutationModal {
                kind: MutationModalKind::CreateDirectory,
                input: String::new(),
                entry: None,
            });
        }
        if ui
            .add_enabled(
                state.transfer.is_none(),
                egui::Button::new(text(language, "files_upload")),
            )
            .clicked()
            && let Some(directory) = state.directory.clone()
            && let Some(local_path) = rfd::FileDialog::new()
                .set_title(text(language, "files_upload_dialog"))
                .pick_file()
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
                Some(Err(error)) => state.error = Some(format_error(language, &error)),
                None => {
                    state.error = Some(text(language, "files_upload_invalid_name").to_owned());
                }
            }
        }
        let can_download = state.transfer.is_none()
            && state
                .selected_entry()
                .is_some_and(|entry| entry.kind == RemoteFileKind::File);
        if ui
            .add_enabled(
                can_download,
                egui::Button::new(text(language, "files_download")),
            )
            .clicked()
            && let Some(entry) = state.selected_entry().cloned()
            && let Some(local_path) = rfd::FileDialog::new()
                .set_title(text(language, "files_download_dialog"))
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
            && ui.button(text(language, "files_cancel_transfer")).clicked()
        {
            commands.push(BackendCommand::CancelFileOperation(request_id));
        }
    });
    ui.separator();
    if state.loading {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(text(language, "files_loading"));
        });
    }
    let mut entries = state.entries.clone();
    sort_entries(&mut entries, state.sort_key, state.sort_reverse);
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("files-grid").striped(true).show(ui, |ui| {
            sort_header(ui, language, state, SortKey::Name, "files_name");
            ui.strong(text(language, "files_type"));
            sort_header(ui, language, state, SortKey::Size, "files_size");
            sort_header(ui, language, state, SortKey::Modified, "files_modified");
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
                response.context_menu(|ui| {
                    state.selected = Some(index);
                    entry_context_menu(ui, language, state, &mut commands, &target, entry);
                });
                ui.label(kind_label(language, entry.kind));
                ui.label(
                    entry
                        .size_bytes
                        .map_or("—".to_owned(), |size| size.to_string()),
                );
                ui.label(
                    entry
                        .modified_unix_seconds
                        .map_or_else(|| "—".to_owned(), format_modified_time),
                );
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
                egui::Button::new(text(language, "files_rename")),
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
            .add_enabled(
                can_delete,
                egui::Button::new(text(language, "files_delete")),
            )
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
            ui.label(text(language, "files_transferring"));
        });
    }

    if let Some(intent) = state.overwrite_prompt.clone() {
        let mut open = true;
        let (title, message) = match intent.direction {
            FileTransferDirection::Upload => (
                text(language, "files_overwrite_remote_title"),
                text(language, "files_overwrite_body")
                    .replace("{}", &intent.remote_path.to_string()),
            ),
            FileTransferDirection::Download => (
                text(language, "files_overwrite_local_title"),
                text(language, "files_overwrite_body")
                    .replace("{}", &intent.local_path.display().to_string()),
            ),
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label(message);
                ui.horizontal(|ui| {
                    if ui.button(text(language, "files_replace")).clicked() {
                        state.overwrite_prompt = None;
                        commands.push(
                            state.start_transfer(intent.clone(), OverwritePolicy::ReplaceConfirmed),
                        );
                    }
                    if ui.button(text(language, "cancel")).clicked() {
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
            MutationModalKind::CreateDirectory => text(language, "files_new_folder"),
            MutationModalKind::Rename => text(language, "files_rename"),
            MutationModalKind::Delete => text(language, "files_delete_title"),
        };
        let mut open = true;
        let mut submitted = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                if matches!(kind, MutationModalKind::Delete) {
                    ui.label(text(language, "files_delete_body"));
                } else {
                    ui.add(egui::TextEdit::singleline(&mut modal.input).desired_width(260.0));
                }
                ui.horizontal(|ui| {
                    let confirm = if matches!(kind, MutationModalKind::Delete) {
                        ui.button(text(language, "files_delete")).clicked()
                    } else {
                        ui.button(text(language, "confirm")).clicked()
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
                            state.error = Some(text(language, "files_invalid_name").to_owned());
                        }
                    }
                });
                if ui.button(text(language, "cancel")).clicked() {
                    submitted = true;
                }
            });
        if open && !submitted {
            state.mutation_modal = Some(modal);
        }
    }
    commands
}

/// Row context menu, mirroring the toolbar actions so files can be managed
/// in place like a desktop file manager.
fn entry_context_menu(
    ui: &mut egui::Ui,
    language: Language,
    state: &mut FilesPanelState,
    commands: &mut Vec<BackendCommand>,
    target: &DeviceTarget,
    entry: &RemoteFileEntry,
) {
    if entry.kind == RemoteFileKind::Directory && ui.button(text(language, "files_enter")).clicked()
    {
        commands.push(state.navigate(target.clone(), entry.path.clone()));
        ui.close();
    }
    if entry.kind == RemoteFileKind::File
        && ui
            .add_enabled(
                state.transfer.is_none(),
                egui::Button::new(text(language, "files_download")),
            )
            .clicked()
        && let Some(local_path) = rfd::FileDialog::new()
            .set_title(text(language, "files_download_dialog"))
            .set_file_name(&entry.name)
            .save_file()
    {
        commands.push(state.start_transfer(
            TransferIntent {
                direction: FileTransferDirection::Download,
                target: target.clone(),
                local_path,
                remote_path: entry.path.clone(),
            },
            OverwritePolicy::Deny,
        ));
        ui.close();
    }
    if ui.button(text(language, "files_copy_path")).clicked() {
        ui.ctx().copy_text(entry.path.to_string());
        ui.close();
    }
    if state.mutation.is_none() {
        if ui.button(text(language, "files_rename")).clicked() {
            state.mutation_modal = Some(MutationModal {
                kind: MutationModalKind::Rename,
                input: entry.name.clone(),
                entry: Some(entry.clone()),
            });
            ui.close();
        }
        if ui.button(text(language, "files_delete")).clicked() {
            state.mutation_modal = Some(MutationModal {
                kind: MutationModalKind::Delete,
                input: String::new(),
                entry: Some(entry.clone()),
            });
            ui.close();
        }
    }
}

fn kind_label(language: Language, kind: RemoteFileKind) -> &'static str {
    match kind {
        RemoteFileKind::Directory => text(language, "files_kind_directory"),
        RemoteFileKind::File => text(language, "files_kind_file"),
        RemoteFileKind::Symlink => text(language, "files_kind_symlink"),
        RemoteFileKind::Other => text(language, "files_kind_other"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_change_clears_listing() {
        let mut state = FilesPanelState::default();
        let serial = fadb_domain::DeviceSerial::new("a").expect("valid");
        let target = DeviceTarget::new(serial, 1);
        let _ = state.reconcile_target(Some(target));
        assert!(state.loading);
    }

    #[test]
    fn navigation_records_history_for_back() {
        let mut state = FilesPanelState::default();
        let serial = fadb_domain::DeviceSerial::new("a").expect("valid");
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
    fn formats_known_timestamp() {
        assert_eq!(format_modified_time(1_700_000_000), "2023-11-14 22:13");
    }
}
