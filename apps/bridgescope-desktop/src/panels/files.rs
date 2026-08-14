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

#[derive(Default)]
pub struct FilesPanelState {
    target: Option<DeviceTarget>,
    directory: Option<RemotePath>,
    path_input: String,
    entries: Vec<RemoteFileEntry>,
    selected: Option<usize>,
    history: Vec<RemotePath>,
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
        {
            match rfd::FileDialog::new().set_title("Upload file").pick_file() {
                Some(local_path) => {
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
                            state.error = Some(
                                "Upload failed: local file name is not valid UTF-8.".to_owned(),
                            );
                        }
                    }
                }
                None => {}
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
    let entries = state.entries.clone();
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("files-grid").striped(true).show(ui, |ui| {
            ui.strong("Name");
            ui.strong("Type");
            ui.strong("Size");
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
        let can_delete = can_mutate
            && selected
                .as_ref()
                .is_some_and(|entry| entry.kind == RemoteFileKind::File);
        if ui
            .add_enabled(can_delete, egui::Button::new("Delete"))
            .clicked()
        {
            state.mutation_modal = Some(MutationModal {
                kind: MutationModalKind::Delete,
                input: String::new(),
                entry: Some(entry),
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
                    ui.label("Delete the selected regular file? This cannot be undone.");
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
                            (MutationModalKind::Delete, Some(entry))
                                if entry.kind == RemoteFileKind::File =>
                            {
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
}
