use eframe::egui::{self, Color32};
use fadb_domain::{BackendCommand, BackendEvent, BridgeError, OperationId};

use crate::i18n::{Language, text};

/// One line in the assistant transcript.
#[derive(Clone, Debug)]
pub struct AssistantTurn {
    pub from_user: bool,
    pub text: String,
}

/// Editable copy of [`fadb_domain::AiSettings`] backing the settings
/// form; the timeout stays a string until it is parsed on save.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AiSettingsForm {
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
    pub timeout: String,
}

impl AiSettingsForm {
    #[must_use]
    pub fn from_settings(settings: Option<&fadb_domain::AiSettings>) -> Self {
        settings.map_or_else(Self::default, |settings| Self {
            endpoint: settings.endpoint.clone(),
            model: settings.model.clone(),
            api_key: settings.api_key.clone(),
            timeout: settings.timeout_seconds.to_string(),
        })
    }

    /// Normalize into [`fadb_domain::AiSettings`] when endpoint and
    /// model are filled. An empty or unparsable timeout falls back to 30 s.
    #[must_use]
    pub fn to_settings(&self) -> Option<fadb_domain::AiSettings> {
        let endpoint = self.endpoint.trim().to_owned();
        let model = self.model.trim().to_owned();
        if endpoint.is_empty() || model.is_empty() {
            return None;
        }
        let timeout_seconds = self
            .timeout
            .trim()
            .parse::<u64>()
            .unwrap_or(30)
            .clamp(1, 600);
        Some(fadb_domain::AiSettings {
            endpoint,
            model,
            api_key: self.api_key.trim().to_owned(),
            timeout_seconds,
        })
    }
}

#[derive(Default)]
pub struct AssistantPanelState {
    turns: Vec<AssistantTurn>,
    input: String,
    pending: Option<OperationId>,
    ready: bool,
    model: Option<String>,
    unavailable_reason: Option<String>,
    error: Option<BridgeError>,
    /// Set when the user asks to open the AI settings dialog; the app owns the
    /// window and resets the flag when it opens.
    pub open_settings: bool,
    /// Set when the panel's collapse button (×) is used; the app resets the
    /// flag when it hides the dock.
    pub close_requested: bool,
}

impl AssistantPanelState {
    /// Seeds a short demo transcript for visual checks (dev env only; see
    /// `FADB_ASSISTANT=2` in `app.rs`).
    pub fn seed_demo_transcript(&mut self) {
        self.turns = vec![
            AssistantTurn {
                from_user: true,
                text: "帮我看看这台设备的基本情况".to_owned(),
            },
            AssistantTurn {
                from_user: false,
                text: "当前授权范围内没有设备摘要数据。请先在设备管理里连接设备，并授予设备概览上下文后我再帮你解读。".to_owned(),
            },
            AssistantTurn {
                from_user: true,
                text: "好的，顺便解释一下刚才能看到哪些信息、不会上传什么？".to_owned(),
            },
        ];
    }

    pub fn handle_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::AiReady { model, .. } => {
                self.ready = true;
                self.model = Some(model.clone());
                self.error = None;
            }
            BackendEvent::AiUnavailable { reason } => {
                self.ready = false;
                self.model = None;
                self.unavailable_reason = Some(reason.clone());
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

/// The header strip: status dot, title, and the active model as a chip, with
/// settings and close ghost buttons at the right end.
fn header(ui: &mut egui::Ui, language: Language, state: &mut AssistantPanelState) {
    let palette = crate::theme::palette(ui.visuals().dark_mode);
    ui.horizontal(|ui| {
        let (dot, tip) = if state.ready {
            (
                Color32::from_rgb(80, 200, 120),
                text(language, "assistant_ready"),
            )
        } else {
            (
                Color32::from_rgb(245, 158, 11),
                text(language, "assistant_not_configured"),
            )
        };
        let (dot_rect, dot_response) =
            ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
        ui.painter().circle_filled(dot_rect.center(), 4.0, dot);
        dot_response.on_hover_text(tip);
        ui.strong(text(language, "assistant"));
        if let Some(model) = state.model.as_deref() {
            crate::theme::chip(ui, model, palette.chip_fill);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("×")
                .on_hover_text(text(language, "close"))
                .clicked()
            {
                state.close_requested = true;
            }
            if ui
                .button(format!("⚙ {}", text(language, "assistant_configure")))
                .clicked()
            {
                state.open_settings = true;
            }
        });
    });
    ui.add_space(2.0);
}

/// One chat bubble, right-aligned for the user and left-aligned with a faint
/// border for the assistant — the layout people know from chat apps.
fn bubble(ui: &mut egui::Ui, turn: &AssistantTurn, max_width: f32) {
    let palette = crate::theme::palette(ui.visuals().dark_mode);
    let (fill, stroke, layout) = if turn.from_user {
        (
            palette.user_bubble,
            egui::Stroke::NONE,
            egui::Layout::right_to_left(egui::Align::Min),
        )
    } else {
        (
            palette.ai_bubble,
            egui::Stroke::new(1.0, palette.bubble_stroke),
            egui::Layout::left_to_right(egui::Align::Min),
        )
    };
    ui.with_layout(layout, |ui| {
        egui::Frame::new()
            .fill(fill)
            .stroke(stroke)
            .corner_radius(10.0)
            .inner_margin(egui::Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.set_max_width(max_width);
                // Bubbles live in a horizontal (LTR/RTL) layout whose default
                // wrap mode is `Extend`; long messages must wrap or they push
                // the whole panel out to its width limit.
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.label(&turn.text);
            });
    });
    ui.add_space(4.0);
}

/// The scrollable transcript with a centered empty state.
fn transcript(ui: &mut egui::Ui, language: Language, state: &AssistantPanelState) {
    if state.turns.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(56.0);
            crate::app::logo(ui, 44.0);
            ui.add_space(12.0);
            ui.label(egui::RichText::new(text(language, "assistant_empty_hint")).weak());
        });
        return;
    }
    let max_bubble_width = (ui.available_width() * 0.85).max(120.0);
    for turn in &state.turns {
        bubble(ui, turn, max_bubble_width);
    }
}

/// The message composer: a full-width input with the accent send pill (and
/// the Enter hint) underneath.
fn composer(
    ui: &mut egui::Ui,
    language: Language,
    state: &mut AssistantPanelState,
) -> Vec<BackendCommand> {
    let mut commands = Vec::new();
    ui.add_space(6.0);
    let can_send = state.ready && state.pending.is_none();
    let response = ui.add_enabled(
        can_send,
        egui::TextEdit::multiline(&mut state.input)
            .hint_text(text(language, "assistant_input_hint"))
            .desired_rows(3)
            .desired_width(ui.available_width()),
    );
    // Enter sends; Shift+Enter inserts a newline like in every chat app.
    let submit = response.lost_focus()
        && ui.input(|input| input.key_pressed(egui::Key::Enter) && !input.modifiers.shift)
        && can_send;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let send = egui::Button::new(
            egui::RichText::new(text(language, "assistant_send"))
                .strong()
                .color(crate::theme::palette(ui.visuals().dark_mode).on_accent),
        )
        .fill(crate::theme::ACCENT)
        .corner_radius(8.0);
        if (ui.add_enabled(can_send, send).clicked() || submit)
            && let Some(command) = state.send()
        {
            commands.push(command);
        }
        ui.small(
            egui::RichText::new(text(language, "assistant_send_hint"))
                .size(11.0)
                .weak(),
        );
    });
    commands
}

pub fn show(
    ui: &mut egui::Ui,
    language: Language,
    state: &mut AssistantPanelState,
) -> Vec<BackendCommand> {
    let palette = crate::theme::palette(ui.visuals().dark_mode);
    let mut commands = Vec::new();
    header(ui, language, state);
    ui.small(text(language, "assistant_privacy_note"));
    if !state.ready
        && let Some(reason) = state.unavailable_reason.as_deref()
        && !reason.is_empty()
    {
        ui.small(
            egui::RichText::new(format!("ⓘ {reason}"))
                .size(11.5)
                .color(Color32::from_rgb(245, 158, 11))
                .weak(),
        );
    }
    ui.add_space(4.0);

    egui::ScrollArea::vertical()
        .id_salt("assistant-transcript")
        .auto_shrink([false, true])
        .stick_to_bottom(true)
        .max_height((ui.available_height() - 104.0).max(120.0))
        .show(ui, |ui| transcript(ui, language, state));

    if state.pending.is_some() {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.small(text(language, "assistant_waiting"));
        });
    }
    if let Some(error) = &state.error {
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(palette.danger_fill)
            .stroke(egui::Stroke::new(1.0, palette.danger_stroke))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("{}: {}", error.message_key, error.detail))
                        .size(12.0),
                );
            });
    }

    commands.extend(composer(ui, language, state));
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_settings_form_roundtrips_settings() {
        let settings = fadb_domain::AiSettings {
            endpoint: " https://api.example.com/v1/ ".to_owned(),
            model: "demo".to_owned(),
            api_key: " key ".to_owned(),
            timeout_seconds: 45,
        };
        let form = AiSettingsForm::from_settings(Some(&settings));
        let restored = form.to_settings().expect("valid form");
        assert_eq!(restored.endpoint, "https://api.example.com/v1/");
        assert_eq!(restored.model, "demo");
        assert_eq!(restored.api_key, "key");
        assert_eq!(restored.timeout_seconds, 45);
        assert_eq!(
            AiSettingsForm::from_settings(Some(&restored)).to_settings(),
            Some(restored)
        );
    }

    #[test]
    fn ai_settings_form_rejects_missing_endpoint_or_model() {
        let mut form = AiSettingsForm {
            model: "demo".to_owned(),
            ..AiSettingsForm::default()
        };
        assert!(form.to_settings().is_none());
        form.endpoint = "https://api.example.com/v1".to_owned();
        assert!(form.to_settings().is_some());
        form.model = String::new();
        assert!(form.to_settings().is_none());
    }

    #[test]
    fn ai_settings_form_defaults_and_clamps_timeout() {
        let mut form = AiSettingsForm {
            endpoint: "https://e/v1".to_owned(),
            model: "m".to_owned(),
            ..AiSettingsForm::default()
        };
        assert_eq!(form.to_settings().unwrap().timeout_seconds, 30);
        form.timeout = "not-a-number".to_owned();
        assert_eq!(form.to_settings().unwrap().timeout_seconds, 30);
        form.timeout = "99999".to_owned();
        assert_eq!(form.to_settings().unwrap().timeout_seconds, 600);
    }

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
