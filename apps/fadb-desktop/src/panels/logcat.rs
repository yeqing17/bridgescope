use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};
use fadb_domain::{BackendCommand, BackendEvent, DeviceTarget, LogcatSessionId};

use crate::i18n::{Language, text};

const MAX_LINES: usize = 10_000;
const AUTO_START_RETRY: Duration = Duration::from_secs(3);
const ROW_HEIGHT: f32 = 17.0;

/// Severity letters as emitted by `logcat -v threadtime`, in ascending order.
const LEVELS: [char; 6] = ['V', 'D', 'I', 'W', 'E', 'F'];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LogLine {
    pub time: String,
    pub pid: String,
    pub tid: String,
    pub level: char,
    pub tag: String,
    pub message: String,
}

impl LogLine {
    /// Parses one `threadtime` line: `MM-DD HH:MM:SS.mmm PID TID L TAG: msg`.
    /// Runs of whitespace are collapsed, and everything past the severity
    /// letter is kept verbatim. Unrecognized lines survive as gray level-'?'
    /// rows so device noise is never silently dropped.
    fn parse(raw: &str) -> Self {
        if let Some((fields, rest)) = split_prefix_fields(raw, 5)
            && fields[4].len() == 1
        {
            let level = fields[4].chars().next().unwrap_or('?');
            if LEVELS.contains(&level) {
                let (tag, message) = match rest.split_once(": ") {
                    Some((tag, message)) => (tag, message),
                    None => (rest, ""),
                };
                return Self {
                    time: format!("{} {}", fields[0], fields[1]),
                    pid: fields[2].to_owned(),
                    tid: fields[3].to_owned(),
                    level,
                    tag: tag.to_owned(),
                    message: message.to_owned(),
                };
            }
        }
        Self {
            message: raw.to_owned(),
            level: '?',
            ..Self::default()
        }
    }

    fn severity_index(&self) -> Option<usize> {
        LEVELS.iter().position(|candidate| candidate == &self.level)
    }

    fn format(&self) -> String {
        if self.level == '?' {
            self.message.clone()
        } else {
            format!(
                "{} {}/{} {}: {}",
                self.time, self.level, self.tag, self.pid, self.message
            )
        }
    }
}

/// Splits the first `count` whitespace-separated fields off `raw`,
/// collapsing runs of spaces; returns the fields and the untrimmed remainder.
fn split_prefix_fields(raw: &str, count: usize) -> Option<(Vec<&str>, &str)> {
    let mut fields = Vec::with_capacity(count);
    let mut rest = raw;
    for _ in 0..count {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        if end == 0 {
            return None;
        }
        fields.push(&rest[..end]);
        rest = &rest[end..];
    }
    Some((fields, rest.trim_start()))
}

fn level_color(level: char) -> Color32 {
    match level {
        'V' => Color32::from_gray(140),
        'D' => Color32::from_rgb(96, 165, 250),
        'I' => Color32::from_rgb(134, 239, 172),
        'W' => Color32::from_rgb(250, 204, 21),
        'E' => Color32::from_rgb(248, 113, 113),
        'F' => Color32::from_rgb(244, 63, 94),
        _ => Color32::from_gray(110),
    }
}

// The plain bools are the small session state machine (starting/running/
// user-stopped/paused) plus the auto-scroll preference.
#[allow(clippy::struct_excessive_bools)]
#[derive(Default)]
pub struct LogcatPanelState {
    pub target: Option<DeviceTarget>,
    session: Option<LogcatSessionId>,
    /// A `StartLogcat` is in flight for this session id.
    starting: bool,
    /// The stream is live (started and not yet closed/failed).
    running: bool,
    /// The user pressed stop; suppress auto-start until the target changes.
    user_stopped: bool,
    paused: bool,
    auto_scroll: bool,
    /// Minimum severity to display (index into `LEVELS`).
    level_filter: usize,
    query: String,
    lines: Vec<LogLine>,
    pending_bytes: Vec<u8>,
    last_start_attempt: Option<Instant>,
    /// Row indices passing the current filters, rebuilt each frame.
    visible: Vec<usize>,
}

impl LogcatPanelState {
    pub fn reset_for(&mut self, target: Option<DeviceTarget>) -> Vec<BackendCommand> {
        let mut commands = Vec::new();
        if self.target != target {
            self.target = target;
            commands.extend(self.take_down());
            self.user_stopped = false;
        }
        commands
    }

    /// Stops any live stream and clears the buffer.
    fn take_down(&mut self) -> Vec<BackendCommand> {
        let mut commands = Vec::new();
        if let Some(session) = self.session.take() {
            commands.push(BackendCommand::StopLogcat(session));
        }
        self.starting = false;
        self.running = false;
        self.lines.clear();
        self.pending_bytes.clear();
        commands
    }

    pub fn handle_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::LogcatStarted { target, session_id }
                if self.target.as_ref() == Some(target)
                    && self.session.as_ref() == Some(session_id) =>
            {
                self.starting = false;
                self.running = true;
                self.user_stopped = false;
            }
            BackendEvent::LogcatOutput { session_id, bytes }
                if self.session.as_ref() == Some(session_id) && !bytes.is_empty() =>
            {
                self.ingest(bytes);
            }
            BackendEvent::LogcatClosed { session_id }
            | BackendEvent::LogcatFailed { session_id, .. }
                if self.session.as_ref() == Some(session_id) =>
            {
                self.session = None;
                self.starting = false;
                self.running = false;
            }
            _ => {}
        }
    }

    /// Splits streamed bytes into lines and appends them while not paused.
    /// A paused stream keeps consuming (dropping) data, matching the usual
    /// "resume from now" expectation.
    fn ingest(&mut self, bytes: &[u8]) {
        let mut start = 0;
        for (index, byte) in bytes.iter().copied().enumerate() {
            if byte == b'\n' {
                self.pending_bytes.extend_from_slice(&bytes[start..index]);
                start = index + 1;
                let line = String::from_utf8_lossy(&self.pending_bytes);
                if !self.paused {
                    self.append(LogLine::parse(line.trim_end_matches('\r')));
                }
                self.pending_bytes.clear();
            }
        }
        self.pending_bytes.extend_from_slice(&bytes[start..]);
        if self.lines.len() > MAX_LINES {
            self.lines.drain(0..self.lines.len() - MAX_LINES);
        }
    }

    fn append(&mut self, line: LogLine) {
        self.lines.push(line);
        if self.lines.len() > MAX_LINES {
            self.lines.remove(0);
        }
    }
}

/// Renders the logcat console; returns any backend commands the user issued.
#[allow(clippy::too_many_lines)]
pub fn show(
    ui: &mut egui::Ui,
    language: Language,
    state: &mut LogcatPanelState,
) -> Vec<BackendCommand> {
    let mut commands = Vec::new();
    ui.horizontal(|ui| {
        ui.heading(text(language, "logcat"));
        ui.add_space(6.0);
        if state.running {
            ui.label(RichText::new("●").color(Color32::from_rgb(74, 222, 128)));
            ui.label(text(language, "logcat_streaming"));
        } else if state.starting {
            ui.spinner();
            ui.label(text(language, "logcat_starting"));
        } else {
            ui.label(text(language, "logcat_idle"));
        }
        if state.paused {
            ui.label(RichText::new(text(language, "logcat_paused")).weak());
        }
    });
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        let online = state.target.is_some();
        if !state.running
            && !state.starting
            && ui
                .add_enabled(
                    online && !state.user_stopped,
                    egui::Button::new(text(language, "logcat_start")),
                )
                .clicked()
            && let Some(target) = state.target.clone()
        {
            let session_id = LogcatSessionId::new();
            state.session = Some(session_id);
            state.starting = true;
            state.paused = false;
            state.user_stopped = false;
            state.last_start_attempt = Some(Instant::now());
            commands.push(BackendCommand::StartLogcat { target, session_id });
        }
        if state.running
            && ui
                .add_enabled(online, egui::Button::new(text(language, "logcat_stop")))
                .clicked()
        {
            commands.extend(state.take_down());
            state.user_stopped = true;
        }
        if ui
            .add_enabled(
                state.running,
                egui::Button::new(if state.paused {
                    text(language, "logcat_resume")
                } else {
                    text(language, "logcat_pause")
                }),
            )
            .clicked()
        {
            state.paused = !state.paused;
        }
        if ui.button(text(language, "logcat_clear")).clicked() {
            state.lines.clear();
        }
        if ui.button(text(language, "logcat_save")).clicked()
            && let Some(path) = rfd::FileDialog::new()
                .set_file_name("fadb-logcat.txt")
                .add_filter("Text", &["txt", "log"])
                .save_file()
        {
            let mut body = String::new();
            for line in &state.lines {
                body.push_str(&line.format());
                body.push('\n');
            }
            if let Err(error) = std::fs::write(&path, body) {
                tracing::warn!(%error, "logcat export failed");
            }
        }
        ui.separator();
        ui.checkbox(&mut state.auto_scroll, text(language, "logcat_autoscroll"));
        ui.separator();
        ui.label(text(language, "logcat_level"));
        let level_names = [
            text(language, "logcat_level_all"),
            "V",
            "D",
            "I",
            "W",
            "E",
            "F",
        ];
        egui::ComboBox::from_id_salt("logcat-level-filter")
            .selected_text(level_names[state.level_filter])
            .width(64.0)
            .show_ui(ui, |ui| {
                for (index, name) in level_names.iter().enumerate() {
                    ui.selectable_value(&mut state.level_filter, index, *name);
                }
            });
        ui.add(
            egui::TextEdit::singleline(&mut state.query)
                .desired_width(180.0)
                .hint_text(text(language, "logcat_search_hint")),
        );
    });
    ui.add_space(6.0);

    if !online_and_selected(state) {
        ui.label(text(language, "files_select_device"));
        return commands;
    }

    // Rebuild the visible-row set for this frame's filters.
    let query = state.query.trim().to_lowercase();
    state.visible.clear();
    for (index, line) in state.lines.iter().enumerate() {
        let level_index = line.severity_index().unwrap_or(0);
        if state.level_filter > 0 && level_index < state.level_filter {
            continue;
        }
        if !query.is_empty()
            && !line.tag.to_lowercase().contains(&query)
            && !line.message.to_lowercase().contains(&query)
        {
            continue;
        }
        state.visible.push(index);
    }

    let total = state.lines.len();
    ui.label(
        RichText::new(text_with_count(
            language,
            "logcat_line_count",
            state.visible.len(),
            total,
        ))
        .weak(),
    );

    let visible = std::mem::take(&mut state.visible);
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .stick_to_bottom(state.auto_scroll && !state.paused)
        .show_rows(ui, ROW_HEIGHT, visible.len(), |ui, range| {
            for row in range {
                let line = &state.lines[visible[row]];
                let color = level_color(line.level);
                // Not add_sized: it centers on the x axis (centered_and_justified
                // layout) and short rows would float mid-panel.
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), ROW_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(line.format())
                                    .monospace()
                                    .size(11.5)
                                    .color(color),
                            )
                            .truncate(),
                        );
                    },
                );
            }
        });
    state.visible = visible;
    commands
}

fn online_and_selected(state: &LogcatPanelState) -> bool {
    state.target.is_some()
}

/// "显示 123 / 4560 行" without pulling a formatting crate in.
fn text_with_count(language: Language, key: &str, visible: usize, total: usize) -> String {
    let template = text(language, key);
    template
        .replace("{visible}", &visible.to_string())
        .replace("{total}", &total.to_string())
}

/// Called by the app shell each frame: auto-start the stream when the panel
/// is on screen, a device is online, and no session exists yet.
pub fn auto_start(state: &mut LogcatPanelState, target_online: bool) -> Vec<BackendCommand> {
    let mut commands = Vec::new();
    if !target_online
        || state.running
        || state.starting
        || state.user_stopped
        || state.session.is_some()
    {
        return commands;
    }
    let recently_attempted = state
        .last_start_attempt
        .is_some_and(|last| last.elapsed() < AUTO_START_RETRY);
    if recently_attempted {
        return commands;
    }
    let Some(target) = state.target.clone() else {
        return commands;
    };
    let session_id = LogcatSessionId::new();
    state.session = Some(session_id);
    state.starting = true;
    state.last_start_attempt = Some(Instant::now());
    commands.push(BackendCommand::StartLogcat { target, session_id });
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_threadtime_lines() {
        let line = LogLine::parse(
            "08-29 10:00:01.100  1000  1001 I ActivityTaskManager: Displayed com.example/.Main",
        );
        assert_eq!(line.time, "08-29 10:00:01.100");
        assert_eq!(line.pid, "1000");
        assert_eq!(line.tid, "1001");
        assert_eq!(line.level, 'I');
        assert_eq!(line.tag, "ActivityTaskManager");
        assert_eq!(line.message, "Displayed com.example/.Main");
    }

    #[test]
    fn keeps_unrecognized_lines_as_raw_rows() {
        let line = LogLine::parse("--------- beginning of main");
        assert_eq!(line.level, '?');
        assert_eq!(line.message, "--------- beginning of main");
    }

    #[test]
    fn severity_orders_for_filtering() {
        assert_eq!(
            LogLine::parse("08-29 10:00:00.000 1 1 V T: m").severity_index(),
            Some(0)
        );
        assert_eq!(
            LogLine::parse("08-29 10:00:00.000 1 1 D T: m").severity_index(),
            Some(1)
        );
        assert_eq!(
            LogLine::parse("08-29 10:00:00.000 1 1 W T: m").severity_index(),
            Some(3)
        );
        assert_eq!(
            LogLine::parse("08-29 10:00:00.000 1 1 F T: m").severity_index(),
            Some(5)
        );
        assert_eq!(LogLine::parse("junk").severity_index(), None);
    }

    #[test]
    fn ingest_splits_lines_and_respects_pause() {
        let mut state = LogcatPanelState::default();
        state.ingest(b"08-29 10:00:01.100 1 1 I Tag: hello\n08-29 10:00:01.200 1 1 W");
        assert_eq!(state.lines.len(), 1);
        assert_eq!(state.pending_bytes, b"08-29 10:00:01.200 1 1 W");
        state.ingest(b" Tag: partial\n");
        assert_eq!(state.lines.len(), 2);
        assert_eq!(state.lines[1].tag, "Tag");

        state.paused = true;
        state.ingest(b"08-29 10:00:01.300 1 1 E Tag: dropped\n");
        assert_eq!(state.lines.len(), 2);
    }

    #[test]
    fn auto_start_requires_online_target_and_backs_off() {
        let mut state = LogcatPanelState::default();
        assert!(auto_start(&mut state, false).is_empty());

        let target = DeviceTarget::new(
            fadb_domain::DeviceSerial::new("emulator-5554").expect("serial"),
            1,
        );
        state.target = Some(target);
        let commands = auto_start(&mut state, true);
        assert_eq!(commands.len(), 1);
        // A session is already pending: no double start.
        assert!(auto_start(&mut state, true).is_empty());
        // The user pressed stop: auto-start stays suppressed.
        let mut stopped = LogcatPanelState {
            target: state.target.clone(),
            user_stopped: true,
            ..LogcatPanelState::default()
        };
        assert!(auto_start(&mut stopped, true).is_empty());
    }
}
