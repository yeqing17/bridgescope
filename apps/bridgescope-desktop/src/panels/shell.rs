use bridgescope_domain::{
    BackendCommand, BackendEvent, BridgeError, DeviceRecord, DeviceTarget, MAX_SHELL_INPUT_BYTES,
    ShellInput, ShellSessionId, ShellSize,
};
use eframe::egui::{self, Color32};

const REMOTE_TERMINAL_ROWS: u16 = 24;
const TERMINAL_COLUMNS: u16 = 80;
const SCROLLBACK_ROWS: usize = 10_000;
const TERMINAL_FONT_SIZE: f32 = 14.0;
const TERMINAL_PADDING: f32 = 10.0;
const TERMINAL_CURSOR_VERTICAL_OFFSET: f32 = 1.0;
const MAX_VIEWPORT_ROWS: u16 = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellStatus {
    Disconnected,
    Connecting,
    Connected,
    Closing,
    Exited,
    Failed,
}

pub struct ShellPanelState {
    parser: vt100::Parser,
    viewport_rows: u16,
    target: Option<DeviceTarget>,
    session_id: Option<ShellSessionId>,
    status: ShellStatus,
    error: Option<BridgeError>,
    focus_terminal: bool,
}

impl Default for ShellPanelState {
    fn default() -> Self {
        Self {
            parser: vt100::Parser::new(REMOTE_TERMINAL_ROWS, TERMINAL_COLUMNS, SCROLLBACK_ROWS),
            viewport_rows: REMOTE_TERMINAL_ROWS,
            target: None,
            session_id: None,
            status: ShellStatus::Disconnected,
            error: None,
            focus_terminal: false,
        }
    }
}

impl ShellPanelState {
    pub fn handle_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ShellOpened { target, session_id }
                if self.target.as_ref() == Some(target) && self.session_id == Some(*session_id) =>
            {
                self.status = ShellStatus::Connected;
                self.error = None;
                self.focus_terminal = true;
            }
            BackendEvent::ShellOutput { session_id, bytes }
                if self.session_id == Some(*session_id) =>
            {
                self.parser.process(bytes);
            }
            BackendEvent::ShellClosed {
                session_id,
                exit_code: _,
            } if self.session_id == Some(*session_id) => {
                self.status = ShellStatus::Exited;
                self.session_id = None;
            }
            BackendEvent::ShellFailed { session_id, error }
                if self.session_id == Some(*session_id) =>
            {
                self.status = ShellStatus::Failed;
                self.error = Some(error.clone());
                self.session_id = None;
            }
            _ => {}
        }
    }

    pub fn reconcile_target(&mut self, selected: Option<&DeviceRecord>) -> Vec<BackendCommand> {
        let selected_target = selected.map(DeviceRecord::target);
        if self.target == selected_target {
            return Vec::new();
        }
        let mut commands = Vec::new();
        if let Some(session_id) = self.session_id.take() {
            commands.push(BackendCommand::CloseShell(session_id));
        }
        self.target = selected_target;
        self.status = ShellStatus::Disconnected;
        self.error = None;
        self.clear_display();
        commands
    }

    fn connect(&mut self) -> Option<BackendCommand> {
        let target = self.target.clone()?;
        let session_id = ShellSessionId::new();
        self.session_id = Some(session_id);
        self.status = ShellStatus::Connecting;
        self.error = None;
        let size = ShellSize::new(TERMINAL_COLUMNS, REMOTE_TERMINAL_ROWS)
            .expect("fixed shell size is valid");
        Some(BackendCommand::OpenShell {
            target,
            session_id,
            size,
        })
    }

    fn write(&mut self, bytes: Vec<u8>) -> Option<BackendCommand> {
        let session_id = self.session_id?;
        let input = ShellInput::new(bytes).ok()?;
        Some(BackendCommand::WriteShell { session_id, input })
    }

    fn close(&mut self) -> Option<BackendCommand> {
        let session_id = self.session_id?;
        self.status = ShellStatus::Closing;
        Some(BackendCommand::CloseShell(session_id))
    }

    fn clear_display(&mut self) {
        self.parser = vt100::Parser::new(self.viewport_rows, TERMINAL_COLUMNS, SCROLLBACK_ROWS);
    }

    fn resize_viewport(&mut self, rows: u16) {
        if self.viewport_rows != rows {
            self.parser.set_size(rows, TERMINAL_COLUMNS);
            self.viewport_rows = rows;
        }
    }

    fn connected(&self) -> bool {
        self.status == ShellStatus::Connected && self.session_id.is_some()
    }
}

pub fn show(
    ui: &mut egui::Ui,
    selected: Option<&DeviceRecord>,
    state: &mut ShellPanelState,
) -> Vec<BackendCommand> {
    let mut commands = state.reconcile_target(selected);
    ui.horizontal(|ui| {
        ui.heading("Interactive Shell");
        ui.separator();
        let (label, color) = status_style(state.status);
        ui.colored_label(color, label);
    });
    ui.small("Expert interface · Android PTY · remote stderr usually merged · fixed 80×24 adapter");
    ui.add_space(8.0);

    let online = selected.is_some_and(|record| record.descriptor.state.is_online());
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                online && state.session_id.is_none(),
                egui::Button::new("Connect"),
            )
            .clicked()
            && let Some(command) = state.connect()
        {
            commands.push(command);
        }
        if ui
            .add_enabled(state.session_id.is_some(), egui::Button::new("Close"))
            .clicked()
            && let Some(command) = state.close()
        {
            commands.push(command);
        }
        if ui.button("Clear display").clicked() {
            state.clear_display();
        }
        if ui.button("Copy visible").clicked() {
            ui.ctx().copy_text(state.parser.screen().contents());
        }
        if ui.button("Focus terminal").clicked() {
            state.focus_terminal = true;
        }
    });

    if !online {
        ui.colored_label(
            Color32::from_rgb(245, 158, 11),
            "Select an online device before connecting.",
        );
    }
    if let Some(error) = &state.error {
        ui.colored_label(
            Color32::LIGHT_RED,
            format!("{}: {}", error.message_key, error.detail),
        );
    }

    ui.add_space(6.0);
    let desired = ui.available_size().max(egui::vec2(400.0, 260.0));
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());
    state.resize_viewport(viewport_rows(ui, rect));
    if response.clicked() || state.focus_terminal {
        response.request_focus();
        state.focus_terminal = false;
    }
    if response.has_focus() {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
        if state.connected() {
            for bytes in terminal_input(ui, state.parser.screen()) {
                if let Some(command) = state.write(bytes) {
                    commands.push(command);
                }
            }
        }
    }
    paint_terminal(ui, rect, state.parser.screen(), response.has_focus());
    commands
}

fn viewport_rows(ui: &egui::Ui, rect: egui::Rect) -> u16 {
    let font_id = egui::FontId::monospace(TERMINAL_FONT_SIZE);
    let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font_id));
    rows_for_viewport(rect.height(), row_height)
}

fn rows_for_viewport(viewport_height: f32, row_height: f32) -> u16 {
    let usable_height = (viewport_height - 2.0 * TERMINAL_PADDING).max(row_height);
    let rows = (usable_height / row_height).floor();
    rows.clamp(1.0, f32::from(MAX_VIEWPORT_ROWS)) as u16
}

fn terminal_input(ui: &egui::Ui, screen: &vt100::Screen) -> Vec<Vec<u8>> {
    let mut output = Vec::new();
    let events = ui.input(|input| input.events.clone());
    let text_supplies_enter = events.iter().any(event_has_line_ending);
    for event in events {
        match event {
            egui::Event::Text(text) if !text.is_empty() => {
                append_text_input(&mut output, &text);
            }
            egui::Event::Paste(text) if !text.is_empty() => {
                if screen.bracketed_paste() {
                    append_terminal_input(&mut output, b"\x1b[200~");
                }
                append_terminal_input(&mut output, text.as_bytes());
                if screen.bracketed_paste() {
                    append_terminal_input(&mut output, b"\x1b[201~");
                }
            }
            egui::Event::Copy => append_terminal_input(&mut output, &[3]),
            egui::Event::Cut => append_terminal_input(&mut output, &[24]),
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                if key == egui::Key::Enter && text_supplies_enter {
                    continue;
                }
                if let Some(bytes) = key_bytes(key, modifiers, screen.application_cursor()) {
                    append_terminal_input(&mut output, &bytes);
                }
            }
            _ => {}
        }
    }
    output
}

fn event_has_line_ending(event: &egui::Event) -> bool {
    matches!(event, egui::Event::Text(text) if text.contains(['\r', '\n']))
}

fn append_text_input(output: &mut Vec<Vec<u8>>, text: &str) {
    let mut printable = String::new();
    let mut previous_was_line_ending = false;
    for character in text.chars() {
        match character {
            '\r' | '\n' => {
                if !previous_was_line_ending {
                    if !printable.is_empty() {
                        append_terminal_input(output, printable.as_bytes());
                        printable.clear();
                    }
                    append_terminal_input(output, b"\n");
                }
                previous_was_line_ending = true;
            }
            _ if character.is_control() => {}
            _ => {
                printable.push(character);
                previous_was_line_ending = false;
            }
        }
    }
    if !printable.is_empty() {
        append_terminal_input(output, printable.as_bytes());
    }
}

fn append_terminal_input(output: &mut Vec<Vec<u8>>, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        if output
            .last()
            .is_none_or(|chunk| chunk.len() == MAX_SHELL_INPUT_BYTES)
        {
            output.push(Vec::with_capacity(bytes.len().min(MAX_SHELL_INPUT_BYTES)));
        }
        let chunk = output.last_mut().expect("input chunk exists");
        let count = bytes
            .len()
            .min(MAX_SHELL_INPUT_BYTES.saturating_sub(chunk.len()));
        chunk.extend_from_slice(&bytes[..count]);
        bytes = &bytes[count..];
    }
}

fn key_bytes(
    key: egui::Key,
    modifiers: egui::Modifiers,
    application_cursor: bool,
) -> Option<Vec<u8>> {
    let fixed: Option<&[u8]> = match key {
        egui::Key::Enter => Some(b"\n"),
        egui::Key::Tab => Some(b"\t"),
        egui::Key::Backspace => Some(b"\x7f"),
        egui::Key::Escape => Some(b"\x1b"),
        egui::Key::ArrowUp => Some(if application_cursor {
            b"\x1bOA"
        } else {
            b"\x1b[A"
        }),
        egui::Key::ArrowDown => Some(if application_cursor {
            b"\x1bOB"
        } else {
            b"\x1b[B"
        }),
        egui::Key::ArrowRight => Some(if application_cursor {
            b"\x1bOC"
        } else {
            b"\x1b[C"
        }),
        egui::Key::ArrowLeft => Some(if application_cursor {
            b"\x1bOD"
        } else {
            b"\x1b[D"
        }),
        egui::Key::Home => Some(b"\x1b[H"),
        egui::Key::End => Some(b"\x1b[F"),
        egui::Key::Insert => Some(b"\x1b[2~"),
        egui::Key::Delete => Some(b"\x1b[3~"),
        egui::Key::PageUp => Some(b"\x1b[5~"),
        egui::Key::PageDown => Some(b"\x1b[6~"),
        _ => None,
    };
    if let Some(bytes) = fixed {
        return Some(bytes.to_vec());
    }
    if modifiers.ctrl
        && !modifiers.shift
        && let Some(index) = control_letter(key)
    {
        return Some(vec![index]);
    }
    None
}

fn control_letter(key: egui::Key) -> Option<u8> {
    let keys = [
        egui::Key::A,
        egui::Key::B,
        egui::Key::C,
        egui::Key::D,
        egui::Key::E,
        egui::Key::F,
        egui::Key::G,
        egui::Key::H,
        egui::Key::I,
        egui::Key::J,
        egui::Key::K,
        egui::Key::L,
        egui::Key::M,
        egui::Key::N,
        egui::Key::O,
        egui::Key::P,
        egui::Key::Q,
        egui::Key::R,
        egui::Key::S,
        egui::Key::T,
        egui::Key::U,
        egui::Key::V,
        egui::Key::W,
        egui::Key::X,
        egui::Key::Y,
        egui::Key::Z,
    ];
    keys.iter()
        .position(|candidate| *candidate == key)
        .and_then(|index| u8::try_from(index + 1).ok())
}

fn paint_terminal(ui: &egui::Ui, rect: egui::Rect, screen: &vt100::Screen, focused: bool) {
    let dark_mode = ui.visuals().dark_mode;
    let background = if dark_mode {
        Color32::from_rgb(14, 17, 22)
    } else {
        Color32::from_rgb(248, 250, 252)
    };
    let foreground = if dark_mode {
        Color32::from_rgb(220, 226, 235)
    } else {
        Color32::from_rgb(24, 31, 42)
    };
    let focus_color = if dark_mode {
        Color32::LIGHT_BLUE
    } else {
        Color32::from_rgb(37, 99, 235)
    };
    let cursor_color = if dark_mode {
        Color32::from_white_alpha(80)
    } else {
        Color32::from_black_alpha(64)
    };
    // Shell output can be much larger than its fixed VT viewport. Clip both
    // text and cursor so scrollback never paints into adjacent UI regions.
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 4.0, background);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(
            if focused { 2.0 } else { 1.0 },
            if focused {
                focus_color
            } else {
                ui.visuals().widgets.inactive.bg_stroke.color
            },
        ),
        egui::StrokeKind::Inside,
    );
    let contents = screen.contents();
    let text = if contents.is_empty() {
        "Click Connect, then focus this terminal.".to_owned()
    } else {
        contents
    };
    let font_id = egui::FontId::monospace(TERMINAL_FONT_SIZE);
    painter.text(
        rect.left_top() + egui::vec2(TERMINAL_PADDING, TERMINAL_PADDING),
        egui::Align2::LEFT_TOP,
        text,
        font_id.clone(),
        foreground,
    );
    if !screen.hide_cursor() && focused {
        let (row, column) = screen.cursor_position();
        let (cell_width, cell_height) =
            ui.fonts_mut(|fonts| (fonts.glyph_width(&font_id, 'W'), fonts.row_height(&font_id)));
        let cursor_height = (cell_height - TERMINAL_CURSOR_VERTICAL_OFFSET).max(1.0);
        let position = rect.left_top()
            + egui::vec2(
                TERMINAL_PADDING + f32::from(column) * cell_width,
                TERMINAL_PADDING + f32::from(row) * cell_height + TERMINAL_CURSOR_VERTICAL_OFFSET,
            );
        painter.rect_filled(
            egui::Rect::from_min_size(position, egui::vec2(cell_width, cursor_height)),
            0.0,
            cursor_color,
        );
    }
}

fn status_style(status: ShellStatus) -> (&'static str, Color32) {
    match status {
        ShellStatus::Disconnected => ("Disconnected", Color32::GRAY),
        ShellStatus::Connecting => ("Connecting", Color32::YELLOW),
        ShellStatus::Connected => ("Connected", Color32::LIGHT_GREEN),
        ShellStatus::Closing => ("Closing", Color32::YELLOW),
        ShellStatus::Exited => ("Exited", Color32::GRAY),
        ShellStatus::Failed => ("Failed", Color32::LIGHT_RED),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_keys_are_encoded() {
        assert_eq!(
            key_bytes(egui::Key::C, egui::Modifiers::CTRL, false),
            Some(vec![3])
        );
        assert_eq!(
            key_bytes(egui::Key::ArrowUp, egui::Modifiers::NONE, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            key_bytes(egui::Key::Enter, egui::Modifiers::NONE, false),
            Some(b"\n".to_vec())
        );
    }

    #[test]
    fn terminal_input_batches_and_preserves_order() {
        let mut output = Vec::new();
        append_terminal_input(&mut output, b"hello");
        append_terminal_input(&mut output, b"\r");
        assert_eq!(output, [b"hello\r".to_vec()]);
    }

    #[test]
    fn text_input_normalizes_crlf_to_one_line_ending() {
        let mut output = Vec::new();
        append_text_input(&mut output, "echo ready\r\n");

        assert_eq!(output, [b"echo ready\n".to_vec()]);
    }

    #[test]
    fn only_text_events_with_line_endings_suppress_enter_key_events() {
        assert!(event_has_line_ending(&egui::Event::Text("\r".to_owned())));
        assert!(!event_has_line_ending(&egui::Event::Text(
            "echo ready".to_owned()
        )));
    }

    #[test]
    fn terminal_input_splits_at_domain_limit() {
        let input = vec![b'x'; MAX_SHELL_INPUT_BYTES + 7];
        let mut output = Vec::new();
        append_terminal_input(&mut output, &input);
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].len(), MAX_SHELL_INPUT_BYTES);
        assert_eq!(output[1], vec![b'x'; 7]);
    }

    #[test]
    fn viewport_rows_fill_available_terminal_height() {
        assert_eq!(rows_for_viewport(660.0, 16.0), 40);
        assert_eq!(rows_for_viewport(20.0, 16.0), 1);
        assert_eq!(rows_for_viewport(20_000.0, 16.0), MAX_VIEWPORT_ROWS);
    }

    #[test]
    fn vt100_parser_handles_fragmented_ansi() {
        let mut state = ShellPanelState::default();
        state.parser.process(b"\x1b[3");
        state.parser.process(b"1mred\x1b[0m");
        assert!(state.parser.screen().contents().contains("red"));
    }
}
