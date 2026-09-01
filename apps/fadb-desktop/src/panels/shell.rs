use eframe::egui::{self, Color32};
use fadb_domain::{
    BackendCommand, BackendEvent, BridgeError, DeviceRecord, DeviceTarget, MAX_SHELL_INPUT_BYTES,
    ShellInput, ShellSessionId, ShellSize,
};

use crate::i18n::{Language, error_text, text};
use crate::quick_commands::{self, QuickCommand, QuickCommandStore};

const INITIAL_TERMINAL_ROWS: u16 = 24;
const TERMINAL_COLUMNS: u16 = 80;
const SCROLLBACK_ROWS: usize = 10_000;
const TERMINAL_FONT_SIZE: f32 = 14.0;
const TERMINAL_PADDING: f32 = 10.0;
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
    /// Lazily created system-clipboard handle for the paste actions; egui
    /// only exposes clipboard reads through the Ctrl+V event.
    clipboard: Option<arboard::Clipboard>,
    /// Grid-cell selection over the visible viewport: `(anchor, head)`, each
    /// a `(row, column)` pair in `TerminalCell` units.
    selection: Option<(TerminalCell, TerminalCell)>,
    /// One-click commands (Xshell-style quick commands) for this panel.
    pub quick_commands: QuickCommandStore,
}

/// A `(row, column)` position in the terminal grid.
type TerminalCell = (u16, u16);

impl Default for ShellPanelState {
    fn default() -> Self {
        Self {
            parser: vt100::Parser::new(INITIAL_TERMINAL_ROWS, TERMINAL_COLUMNS, SCROLLBACK_ROWS),
            viewport_rows: INITIAL_TERMINAL_ROWS,
            target: None,
            session_id: None,
            status: ShellStatus::Disconnected,
            error: None,
            focus_terminal: false,
            clipboard: None,
            selection: None,
            quick_commands: QuickCommandStore::default(),
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
        self.selection = None;
        self.clear_display();
        commands
    }

    /// The selection in reading order, or `None` when empty or collapsed.
    fn ordered_selection(&self) -> Option<(TerminalCell, TerminalCell)> {
        let (anchor, head) = self.selection?;
        Some(if (anchor.0, anchor.1) <= (head.0, head.1) {
            (anchor, head)
        } else {
            (head, anchor)
        })
    }

    /// Extracts the selected text from the visible viewport. Wide (CJK)
    /// cells contribute their glyph only once; rows are trimmed of trailing
    /// blanks and joined with newlines.
    fn selection_text(&self) -> Option<String> {
        let (start, end) = self.ordered_selection()?;
        let screen = self.parser.screen();
        let mut lines = Vec::new();
        for row in start.0..=end.0 {
            let first = if row == start.0 { start.1 } else { 0 };
            let last = if row == end.0 {
                end.1
            } else {
                TERMINAL_COLUMNS.saturating_sub(1)
            };
            let mut line = String::new();
            for column in first..=last {
                if let Some(cell) = screen.cell(row, column) {
                    line.push_str(cell.contents());
                }
            }
            lines.push(line.trim_end().to_owned());
        }
        Some(lines.join("\n"))
    }

    /// Extracts a paste payload: bracketed-paste wrapped when the program
    /// behind the terminal asked for it.
    fn paste_payload(&self, clipboard: &str) -> Vec<u8> {
        let screen = self.parser.screen();
        let mut bytes = Vec::new();
        if screen.bracketed_paste() {
            bytes.extend_from_slice(b"\x1b[200~");
        }
        bytes.extend_from_slice(clipboard.as_bytes());
        if screen.bracketed_paste() {
            bytes.extend_from_slice(b"\x1b[201~");
        }
        bytes
    }

    /// Reads the system clipboard through a lazily created handle; failures
    /// (headless environments, lost ownership) surface as `None`.
    fn clipboard_text(&mut self) -> Option<String> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()?.get_text().ok()
    }

    fn connect(&mut self) -> Option<BackendCommand> {
        let target = self.target.clone()?;
        let session_id = ShellSessionId::new();
        self.session_id = Some(session_id);
        self.status = ShellStatus::Connecting;
        self.error = None;
        let size = ShellSize::new(TERMINAL_COLUMNS, self.viewport_rows)
            .expect("viewport shell size is valid");
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

    /// Sends a quick command's bytes to the live session. Focusing the
    /// terminal afterwards keeps the keyboard ready for follow-up input.
    fn send_quick_command(&mut self, command: &QuickCommand) -> Vec<BackendCommand> {
        if !self.connected() || !command.is_sendable() {
            return Vec::new();
        }
        let commands = quick_commands::payload_chunks(command)
            .into_iter()
            .filter_map(|chunk| self.write(chunk))
            .collect();
        self.focus_terminal = true;
        commands
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
            self.parser.screen_mut().set_size(rows, TERMINAL_COLUMNS);
            self.viewport_rows = rows;
        }
    }

    fn connected(&self) -> bool {
        self.status == ShellStatus::Connected && self.session_id.is_some()
    }
}

#[allow(clippy::too_many_lines)]
pub fn show(
    ui: &mut egui::Ui,
    context: &egui::Context,
    language: Language,
    selected: Option<&DeviceRecord>,
    state: &mut ShellPanelState,
) -> Vec<BackendCommand> {
    let mut commands = state.reconcile_target(selected);
    // The panel hint is too chatty to spend a permanent line on; it surfaces
    // as a tooltip on the title instead.
    ui.horizontal(|ui| {
        ui.heading(text(language, "shell_title"))
            .on_hover_text(text(language, "shell_hint"));
        ui.separator();
        let (label, color) = status_style(language, state.status);
        ui.colored_label(color, label);
    });
    ui.add_space(6.0);

    let online = selected.is_some_and(|record| record.descriptor.state.is_online());
    let connected = state.connected();
    // One wrapping row: session controls first, quick commands after a small
    // separator, so both share the same line and the terminal keeps the rest
    // of the panel.
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                online && state.session_id.is_none(),
                egui::Button::new(text(language, "connect")),
            )
            .clicked()
            && let Some(command) = state.connect()
        {
            commands.push(command);
        }
        if ui
            .add_enabled(
                state.session_id.is_some(),
                egui::Button::new(text(language, "shell_disconnect")),
            )
            .clicked()
            && let Some(command) = state.close()
        {
            commands.push(command);
        }
        if ui.button(text(language, "shell_clear_display")).clicked() {
            state.clear_display();
        }
        if ui.button(text(language, "shell_copy_visible")).clicked() {
            ui.ctx().copy_text(state.parser.screen().contents());
        }
        if ui
            .add_enabled(
                state.session_id.is_some(),
                egui::Button::new(text(language, "shell_paste")),
            )
            .clicked()
            && let Some(clipboard) = state.clipboard_text().filter(|text| !text.is_empty())
        {
            let payload = state.paste_payload(&clipboard);
            if let Some(command) = state.write(payload) {
                commands.push(command);
            }
        }
        if ui.button(text(language, "shell_focus_terminal")).clicked() {
            state.focus_terminal = true;
        }
        ui.separator();
        // Quick commands: click to send. Disabled until a session is live.
        for command in
            quick_commands::show_inline(ui, language, &mut state.quick_commands, connected)
        {
            for command in state.send_quick_command(&command) {
                commands.push(command);
            }
        }
    });
    if state.quick_commands.manage_open {
        quick_commands::show_manage_window(context, language, &mut state.quick_commands);
    }

    if !online {
        ui.colored_label(
            Color32::from_rgb(245, 158, 11),
            text(language, "shell_select_online"),
        );
    }
    if let Some(error) = &state.error {
        ui.colored_label(Color32::LIGHT_RED, error_text(language, error));
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
    // Text selection: drag to highlight, release to copy. A plain click
    // collapses the selection again. The context menu mirrors the toolbar
    // clipboard actions for mouse-only workflows.
    handle_terminal_pointer(ui, language, state, &response, rect, &mut commands);
    paint_terminal(
        ui,
        language,
        rect,
        state.parser.screen(),
        state.ordered_selection(),
        response.has_focus(),
        state.status == ShellStatus::Disconnected,
    );
    commands
}

/// Selection tracking over the terminal surface (drag-select with
/// release-to-copy) plus the right-click clipboard menu.
fn handle_terminal_pointer(
    ui: &egui::Ui,
    language: Language,
    state: &mut ShellPanelState,
    response: &egui::Response,
    rect: egui::Rect,
    commands: &mut Vec<BackendCommand>,
) {
    if response.drag_started()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let cell = cell_at_pointer(ui, rect, state.viewport_rows, pos);
        state.selection = Some((cell, cell));
    }
    if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some((_, head)) = state.selection.as_mut()
    {
        *head = cell_at_pointer(ui, rect, state.viewport_rows, pos);
    }
    if response.drag_stopped()
        && state
            .ordered_selection()
            .is_some_and(|(start, end)| start != end)
        && let Some(selected) = state.selection_text()
        && selected.chars().any(|character| !character.is_whitespace())
    {
        ui.ctx().copy_text(selected);
    }
    if response.clicked() {
        state.selection = None;
    }
    response.context_menu(|ui| {
        let selected = state.selection_text();
        if ui
            .add_enabled(
                selected
                    .as_ref()
                    .is_some_and(|text| !text.trim().is_empty()),
                egui::Button::new(text(language, "shell_copy_selection")),
            )
            .clicked()
        {
            if let Some(selected) = selected {
                ui.ctx().copy_text(selected);
            }
            ui.close();
        }
        if ui
            .add_enabled(
                state.session_id.is_some(),
                egui::Button::new(text(language, "shell_paste")),
            )
            .clicked()
        {
            if let Some(clipboard) = state.clipboard_text().filter(|text| !text.is_empty()) {
                let payload = state.paste_payload(&clipboard);
                if let Some(command) = state.write(payload) {
                    commands.push(command);
                }
            }
            ui.close();
        }
        if ui.button(text(language, "shell_copy_visible")).clicked() {
            ui.ctx().copy_text(state.parser.screen().contents());
            ui.close();
        }
    });
}

/// Maps a pointer position to the terminal grid cell it falls in, clamped to
/// the visible viewport. The conversion is safe: both coordinates are floored,
/// clamped to the viewport bounds, and far below `u16::MAX`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn cell_at_pointer(ui: &egui::Ui, rect: egui::Rect, rows: u16, pos: egui::Pos2) -> TerminalCell {
    let font_id = egui::FontId::monospace(TERMINAL_FONT_SIZE);
    let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font_id)).max(1.0);
    let cell_width = ui
        .fonts_mut(|fonts| fonts.glyph_width(&font_id, 'W'))
        .max(1.0);
    let origin = rect.left_top() + egui::vec2(TERMINAL_PADDING, TERMINAL_PADDING);
    let column = ((pos.x - origin.x) / cell_width).floor().max(0.0);
    let row = ((pos.y - origin.y) / row_height).floor().max(0.0);
    (
        (row as u16).min(rows.saturating_sub(1)),
        (column as u16).min(TERMINAL_COLUMNS - 1),
    )
}

/// What a blank terminal area shows: the start-up guidance only before the
/// first session — 清屏 on a live or finished session must not resurrect the
/// "click connect" hint.
fn screen_text(language: Language, screen: &vt100::Screen, disconnected: bool) -> String {
    let contents = screen.contents();
    if contents.is_empty() && disconnected {
        text(language, "shell_empty_hint").to_owned()
    } else {
        contents
    }
}

fn viewport_rows(ui: &egui::Ui, rect: egui::Rect) -> u16 {
    let font_id = egui::FontId::monospace(TERMINAL_FONT_SIZE);
    let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font_id));
    rows_for_viewport(rect.height(), row_height)
}

fn rows_for_viewport(viewport_height: f32, row_height: f32) -> u16 {
    let usable_height = (viewport_height - 2.0 * TERMINAL_PADDING).max(row_height);
    let mut lower = 1;
    let mut upper = MAX_VIEWPORT_ROWS;
    while lower < upper {
        let middle = lower + (upper - lower).div_ceil(2);
        if f32::from(middle) * row_height <= usable_height {
            lower = middle;
        } else {
            upper = middle - 1;
        }
    }
    lower
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

/// Also used by the quick-command feature so both input paths split at
/// `MAX_SHELL_INPUT_BYTES` identically.
pub(crate) fn append_terminal_input(output: &mut Vec<Vec<u8>>, mut bytes: &[u8]) {
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

fn paint_terminal(
    ui: &egui::Ui,
    language: Language,
    rect: egui::Rect,
    screen: &vt100::Screen,
    selection: Option<(TerminalCell, TerminalCell)>,
    focused: bool,
    disconnected: bool,
) {
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
    let text = screen_text(language, screen, disconnected);
    let font_id = egui::FontId::monospace(TERMINAL_FONT_SIZE);
    let content_origin = rect.left_top() + egui::vec2(TERMINAL_PADDING, TERMINAL_PADDING);
    if let Some((start, end)) = selection {
        let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font_id));
        let cell_width = ui.fonts_mut(|fonts| fonts.glyph_width(&font_id, 'W'));
        let highlight = if dark_mode {
            Color32::from_rgba_premultiplied(60, 110, 180, 110)
        } else {
            Color32::from_rgba_premultiplied(120, 170, 240, 110)
        };
        for row in start.0..=end.0 {
            let first = if row == start.0 { start.1 } else { 0 };
            let last = if row == end.0 {
                end.1
            } else {
                TERMINAL_COLUMNS - 1
            };
            let position = content_origin
                + egui::vec2(f32::from(first) * cell_width, f32::from(row) * row_height);
            let size = egui::vec2(f32::from(last - first + 1) * cell_width, row_height);
            painter.rect_filled(egui::Rect::from_min_size(position, size), 2.0, highlight);
        }
    }
    let galley = painter.layout_no_wrap(text, font_id.clone(), foreground);
    if !galley.is_empty() {
        painter.galley(content_origin, galley.clone(), foreground);
    }
    if !screen.hide_cursor() && focused {
        let (row, column) = screen.cursor_position();
        // `Screen::contents` omits blank grid rows while the VT cursor keeps
        // its full-grid row index. When those differ, the final rendered row
        // is the active prompt or command line and is the best cursor anchor.
        let placed_row = galley
            .rows
            .get(usize::from(row))
            .or_else(|| galley.rows.last());
        if let Some(placed_row) = placed_row {
            let cell_width = ui.fonts_mut(|fonts| fonts.glyph_width(&font_id, 'W'));
            let cursor_position = content_origin
                + placed_row.pos.to_vec2()
                + egui::vec2(placed_row.x_offset(usize::from(column)), 0.0);
            let cursor_size = egui::vec2(cell_width, placed_row.rect().height());
            painter.rect_filled(
                egui::Rect::from_min_size(cursor_position, cursor_size),
                0.0,
                cursor_color,
            );
        }
    }
}

fn status_style(language: Language, status: ShellStatus) -> (&'static str, Color32) {
    match status {
        ShellStatus::Disconnected => (text(language, "shell_status_disconnected"), Color32::GRAY),
        ShellStatus::Connecting => (text(language, "shell_status_connecting"), Color32::YELLOW),
        ShellStatus::Connected => (
            text(language, "shell_status_connected"),
            Color32::LIGHT_GREEN,
        ),
        ShellStatus::Closing => (text(language, "shell_status_closing"), Color32::YELLOW),
        ShellStatus::Exited => (text(language, "shell_status_exited"), Color32::GRAY),
        ShellStatus::Failed => (text(language, "shell_status_failed"), Color32::LIGHT_RED),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_output(bytes: &[u8]) -> ShellPanelState {
        let mut state = ShellPanelState::default();
        state.parser.process(bytes);
        state
    }

    #[test]
    fn selection_text_extracts_grid_range() {
        let mut state = state_with_output(b"hello world\r\nsecond line");
        state.selection = Some(((0, 0), (0, 4)));
        assert_eq!(state.selection_text(), Some("hello".to_owned()));
        state.selection = Some(((0, 6), (1, 5)));
        assert_eq!(state.selection_text(), Some("world\nsecond".to_owned()));
        state.selection = Some(((1, 0), (1, 79)));
        assert_eq!(state.selection_text(), Some("second line".to_owned()));
    }

    #[test]
    fn ordered_selection_normalizes_drag_direction() {
        let mut state = ShellPanelState {
            selection: Some(((3, 10), (1, 2))),
            ..ShellPanelState::default()
        };
        assert_eq!(state.ordered_selection(), Some(((1, 2), (3, 10))));
        state.selection = None;
        assert_eq!(state.ordered_selection(), None);
    }

    #[test]
    fn paste_payload_respects_bracketed_paste_mode() {
        let state = state_with_output(b"");
        assert_eq!(state.paste_payload("ls"), b"ls".to_vec());
        let state = state_with_output(b"\x1b[?2004h");
        assert_eq!(state.paste_payload("ls"), b"\x1b[200~ls\x1b[201~".to_vec());
    }

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

    #[test]
    fn blank_screen_hints_only_before_the_first_session() {
        let parser = vt100::Parser::new(4, TERMINAL_COLUMNS, SCROLLBACK_ROWS);
        let screen = parser.screen();
        // Never connected: the start-up guidance shows.
        assert_eq!(
            screen_text(Language::Chinese, screen, true),
            text(Language::Chinese, "shell_empty_hint")
        );
        // Connected (清屏 or fresh session): the terminal stays blank.
        assert_eq!(screen_text(Language::Chinese, screen, false), "");
        // Output wins over both flags.
        let mut with_output = vt100::Parser::new(4, TERMINAL_COLUMNS, SCROLLBACK_ROWS);
        with_output.process(b"hello");
        assert_eq!(
            screen_text(Language::Chinese, with_output.screen(), false),
            "hello"
        );
    }
}
