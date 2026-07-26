use bridgescope_domain::{
    BackendCommand, BackendEvent, DeviceTarget, OperationId, RemoteFileEntry, RemoteFileKind,
    RemotePath,
};
use eframe::egui;

#[derive(Default)]
pub struct FilesPanelState {
    target: Option<DeviceTarget>,
    directory: Option<RemotePath>,
    path_input: String,
    entries: Vec<RemoteFileEntry>,
    selected: Option<usize>,
    listing_request: Option<OperationId>,
    loading: bool,
    transfer: Option<OperationId>,
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
        self.listing_request = None;
        self.transfer = None;
        self.error = None;
        target.map(|target| {
            self.list(
                target,
                RemotePath::new("/sdcard").expect("valid default path"),
            )
        })
    }

    pub fn handle_event(&mut self, event: &BackendEvent) {
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
                self.error = Some(format!("{}: {}", error.message_key, error.detail));
            }
            BackendEvent::FileTransferStarted {
                request_id, target, ..
            } if self.target.as_ref() == Some(target) => self.transfer = Some(*request_id),
            BackendEvent::FileTransferCompleted {
                request_id,
                summary,
            } if self.transfer.as_ref() == Some(request_id)
                && self.target.as_ref() == Some(&summary.target) =>
            {
                self.transfer = None;
            }
            BackendEvent::FileTransferFailed {
                request_id,
                target,
                error,
            } if self.transfer.as_ref() == Some(request_id)
                && self.target.as_ref() == Some(target) =>
            {
                self.transfer = None;
                self.error = Some(format!("{}: {}", error.message_key, error.detail));
            }
            BackendEvent::FileTransferCancelled { request_id, target }
                if self.transfer.as_ref() == Some(request_id)
                    && self.target.as_ref() == Some(target) =>
            {
                self.transfer = None;
            }
            _ => {}
        }
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
}

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
        if ui.button("Back").clicked()
            && let Some(directory) = state.directory.clone()
        {
            commands.push(state.list(target.clone(), directory.parent()));
        }
        if ui.button("Up").clicked()
            && let Some(directory) = state.directory.clone()
        {
            commands.push(state.list(target.clone(), directory.parent()));
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
                Ok(path) => commands.push(state.list(target.clone(), path)),
                Err(error) => state.error = Some(error.to_string()),
            }
        }
        ui.add_enabled(false, egui::Button::new("Upload (path picker unavailable)"));
        ui.add_enabled(
            false,
            egui::Button::new("Download (path picker unavailable)"),
        );
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
                    commands.push(state.list(target.clone(), entry.path.clone()));
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
    if let Some(error) = &state.error {
        ui.colored_label(egui::Color32::LIGHT_RED, error);
    }
    if state.transfer.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Transferring…");
        });
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
}
