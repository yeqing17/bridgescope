use bridgescope_domain::{BackendCommand, BackendEvent, BridgeError, OperationId};
use eframe::egui::{self, Color32};

/// One line in the assistant transcript.
#[derive(Clone, Debug)]
pub struct AssistantTurn {
    pub from_user: bool,
    pub text: String,
}

#[derive(Default)]
pub struct AssistantPanelState {
    turns: Vec<AssistantTurn>,
    input: String,
    pending: Option<OperationId>,
    ready: bool,
    model: Option<String>,
    error: Option<BridgeError>,
}

impl AssistantPanelState {
    pub fn handle_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::AiReady { model, .. } => {
                self.ready = true;
                self.model = Some(model.clone());
                self.error = None;
            }
            BackendEvent::AiUnavailable { .. } => {
                self.ready = false;
                self.model = None;
            }
            BackendEvent::AiChatCompleted { request_id, reply } => {
                if self.pending.as_ref() == Some(request_id) {
                    self.pending = None;
                    self.error = None;
                    self.turns.push(AssistantTurn {
                        from_user: false,
                        text: reply.clone(),
                    });
                }
            }
            BackendEvent::AiChatFailed { request_id, error } => {
                if self.pending.as_ref() == Some(request_id) {
                    self.pending = None;
                    self.error = Some(error.clone());
                }
            }
            _ => {}
        }
    }

    fn send(&mut self) -> Option<BackendCommand> {
        let trimmed = self.input.trim();
        if trimmed.is_empty() || self.pending.is_some() || !self.ready {
            return None;
        }
        let request_id = OperationId::new();
        self.pending = Some(request_id);
        self.turns.push(AssistantTurn {
            from_user: true,
            text: trimmed.to_owned(),
        });
        let prompt = std::mem::take(&mut self.input);
        Some(BackendCommand::SendAiChat { request_id, prompt })
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut AssistantPanelState) -> Vec<BackendCommand> {
    let mut commands = Vec::new();
    ui.horizontal(|ui| {
        ui.heading("AI Assistant");
        ui.separator();
        if state.ready {
            ui.colored_label(
                Color32::LIGHT_GREEN,
                format!("Ready · {}", state.model.as_deref().unwrap_or("model")),
            );
        } else {
            ui.colored_label(Color32::from_rgb(245, 158, 11), "Not configured");
        }
    });
    ui.small(
        "Provider-neutral placeholder. No data leaves the device unless a provider is configured \
and a context grant is given. Streaming output is a planned milestone.",
    );
    ui.add_space(8.0);

    egui::ScrollArea::vertical()
        .id_salt("assistant-transcript")
        .auto_shrink([false, true])
        .max_height(ui.available_height() - 70.0)
        .show(ui, |ui| {
            if state.turns.is_empty() {
                ui.label(
                    "Ask about the selected device, an ADB error, or a log line. The assistant has \
no access to device data until you grant it.",
                );
            }
            for turn in &state.turns {
                let (label, color) = if turn.from_user {
                    ("You", Color32::from_rgb(140, 180, 255))
                } else {
                    ("Assistant", Color32::from_rgb(180, 220, 180))
                };
                ui.colored_label(color, format!("{label}: {}", turn.text));
                ui.add_space(2.0);
            }
        });

    if state.pending.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Waiting for the assistant…");
        });
    }
    if let Some(error) = &state.error {
        ui.colored_label(
            Color32::LIGHT_RED,
            format!("{}: {}", error.message_key, error.detail),
        );
    }

    ui.horizontal(|ui| {
        let input_width = (ui.available_width() - 90.0).max(80.0);
        let response = ui.add_enabled(
            state.ready && state.pending.is_none(),
            egui::TextEdit::multiline(&mut state.input)
                .hint_text("Ask the assistant…")
                .desired_rows(2)
                .desired_width(input_width),
        );
        let submit = response.lost_focus()
            && ui.input(|input| input.key_pressed(egui::Key::Enter))
            && state.ready
            && state.pending.is_none();
        if (ui
            .add_enabled(
                state.ready && state.pending.is_none(),
                egui::Button::new("Send"),
            )
            .clicked()
            || submit)
            && let Some(command) = state.send()
        {
            commands.push(command);
        }
    });
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_requires_non_empty_input_and_ready_provider() {
        let mut state = AssistantPanelState::default();
        assert!(state.send().is_none());
        state.ready = true;
        state.input = "  ".to_owned();
        assert!(state.send().is_none());
        state.input = "hello".to_owned();
        let command = state.send().expect("valid send");
        assert!(matches!(command, BackendCommand::SendAiChat { .. }));
        assert!(state.pending.is_some());
        // Cannot send again while pending.
        state.input = "again".to_owned();
        assert!(state.send().is_none());
    }

    #[test]
    fn completed_event_appends_assistant_turn() {
        let request_id = OperationId::new();
        let mut state = AssistantPanelState {
            ready: true,
            pending: Some(request_id),
            ..Default::default()
        };
        state.handle_event(&BackendEvent::AiChatCompleted {
            request_id,
            reply: "hi".to_owned(),
        });
        assert!(state.pending.is_none());
        assert_eq!(
            state.turns.last().map(|turn| turn.text.as_str()),
            Some("hi")
        );
    }

    #[test]
    fn unrelated_request_completions_are_ignored() {
        let mut state = AssistantPanelState {
            pending: Some(OperationId::new()),
            ..Default::default()
        };
        state.handle_event(&BackendEvent::AiChatCompleted {
            request_id: OperationId::new(),
            reply: "stale".to_owned(),
        });
        assert!(state.pending.is_some());
        assert!(state.turns.is_empty());
    }
}
