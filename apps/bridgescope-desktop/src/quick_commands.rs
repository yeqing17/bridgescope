//! Quick commands: one-click buttons that send frequently used commands to
//! the shell, in the spirit of Xshell's quick-command bar. The list is user
//! editable, persisted through eframe storage, and can be exported to or
//! imported from a JSON file so setups travel between machines.

use std::path::Path;

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::i18n::{Language, text};

/// Bumped whenever the on-disk format changes; readers stay tolerant of any
/// value so files from newer builds still import.
const FILE_VERSION: u32 = 1;
/// Buttons longer than this would crowd the shell toolbar.
const MAX_BUTTON_LABEL_CHARS: usize = 24;
/// The manage window shows a scrollable list; rows stay visible above it.
const MANAGE_LIST_MAX_HEIGHT: f32 = 320.0;

/// One saved command: a short button label plus the text to type, and whether
/// pressing the button also presses Enter afterwards.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QuickCommand {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub command: String,
    #[serde(default = "default_run")]
    pub run: bool,
}

fn default_run() -> bool {
    true
}

impl Default for QuickCommand {
    /// A blank row that still executes on click: that is what a newly added
    /// quick command almost always wants to be.
    fn default() -> Self {
        Self {
            label: String::new(),
            command: String::new(),
            run: true,
        }
    }
}

impl QuickCommand {
    pub fn new(label: impl Into<String>, command: impl Into<String>, run: bool) -> Self {
        Self {
            label: label.into(),
            command: command.into(),
            run,
        }
    }

    /// The text shown on the bar button: the label, or the command itself when
    /// no label was set, truncated so a long entry cannot eat the toolbar.
    pub fn display_label(&self) -> String {
        let source = if self.label.trim().is_empty() {
            self.command.trim()
        } else {
            self.label.trim()
        };
        if source.chars().count() <= MAX_BUTTON_LABEL_CHARS {
            return source.to_owned();
        }
        let mut truncated: String = source.chars().take(MAX_BUTTON_LABEL_CHARS).collect();
        truncated.push('…');
        truncated
    }

    pub fn is_sendable(&self) -> bool {
        !self.command.trim().is_empty()
    }
}

/// Serialised shape of an export file. Imports also accept a bare array of
/// commands, because that is what users hand-write most easily.
#[derive(Serialize, Deserialize, Default)]
struct QuickCommandFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    commands: Vec<QuickCommand>,
}

/// Outcome of the last import/export click, rendered with the current UI
/// language instead of a frozen string.
#[derive(Clone, Debug)]
pub enum QuickCommandStatus {
    Imported(usize),
    Exported(String),
    Failed(String),
}

/// Everything the shell panel needs to render and edit the quick commands.
pub struct QuickCommandStore {
    pub commands: Vec<QuickCommand>,
    pub manage_open: bool,
    pub status: Option<QuickCommandStatus>,
}

impl Default for QuickCommandStore {
    fn default() -> Self {
        Self {
            commands: default_commands(),
            manage_open: false,
            status: None,
        }
    }
}

impl QuickCommandStore {
    /// Appends imported commands, skipping blank and duplicate entries;
    /// returns how many were actually added.
    pub fn import(&mut self, incoming: Vec<QuickCommand>) -> usize {
        let mut added = 0;
        for command in incoming {
            if !command.is_sendable() {
                continue;
            }
            let duplicate = self
                .commands
                .iter()
                .any(|known| known.label == command.label && known.command == command.command);
            if duplicate {
                continue;
            }
            self.commands.push(command);
            added += 1;
        }
        added
    }
}

/// Read-only Android commands seeded on first launch so the bar demonstrates
/// itself; deleting them persists (seeding only happens when nothing is
/// stored yet).
fn default_commands() -> Vec<QuickCommand> {
    vec![
        QuickCommand::new("安卓版本", "getprop ro.build.version.release", true),
        QuickCommand::new("电池信息", "dumpsys battery", true),
        QuickCommand::new("磁盘占用", "df -h /data", true),
    ]
}

/// Byte chunks to write to the shell for one quick command: the command text,
/// a trailing newline when `run` is set, split at the domain input limit.
pub fn payload_chunks(command: &QuickCommand) -> Vec<Vec<u8>> {
    let mut bytes = command.command.clone().into_bytes();
    if command.run {
        bytes.push(b'\n');
    }
    let mut chunks = Vec::new();
    crate::panels::shell::append_terminal_input(&mut chunks, &bytes);
    chunks
}

/// Writes the export file; returns the exported count.
pub fn export_to_path(path: &Path, commands: &[QuickCommand]) -> Result<usize, String> {
    let file = QuickCommandFile {
        version: FILE_VERSION,
        commands: commands.to_vec(),
    };
    let body = serde_json::to_string_pretty(&file).map_err(|error| error.to_string())?;
    std::fs::write(path, body + "\n").map_err(|error| error.to_string())?;
    Ok(commands.len())
}

/// Reads an export file (wrapped object or bare array) and returns valid
/// commands; blank entries are dropped.
pub fn import_from_path(path: &Path) -> Result<Vec<QuickCommand>, String> {
    let body = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let commands = match serde_json::from_str::<QuickCommandFile>(&body) {
        Ok(file) => file.commands,
        Err(error) => {
            serde_json::from_str::<Vec<QuickCommand>>(&body).map_err(|_| error.to_string())?
        }
    };
    Ok(commands
        .into_iter()
        .filter(QuickCommand::is_sendable)
        .collect())
}

/// Quick-command buttons occupy at most this many wrapped toolbar lines;
/// anything beyond collapses into the ⋯ overflow menu so a long list cannot
/// push the terminal out of the panel.
const MAX_TOOLBAR_ROWS: u16 = 2;

/// The quick-command segment of the shell toolbar: a small identifying label,
/// one button per command (capped at `MAX_TOOLBAR_ROWS` wrapped lines, with
/// the rest in a vertical ⋯ menu), then the manage entry point. Renders
/// inline into the caller's row — the shell panel places it after a
/// separator, next to the session controls. Returns the commands the user
/// clicked this frame.
pub fn show_inline(
    ui: &mut egui::Ui,
    language: Language,
    store: &mut QuickCommandStore,
    connected: bool,
) -> Vec<QuickCommand> {
    ui.add(egui::Label::new(
        egui::RichText::new(text(language, "quick_commands_tag"))
            .small()
            .weak(),
    ))
    .on_hover_text(text(language, "quick_commands_manage_hint"));

    let mut clicked = Vec::new();
    let no_run_tip = text(language, "quick_commands_no_run_tip");
    let max_right = ui.max_rect().right();
    let item_spacing = ui.spacing().item_spacing;
    // Geometry of the first rendered button anchors the two-row budget.
    let mut row_origin_y = 0.0;
    let mut line_height = 0.0;
    let mut seen_first = false;
    let mut overflow_start: Option<usize> = None;

    for (index, command) in store.commands.iter().enumerate() {
        if seen_first {
            // Predict where the button will land: egui wraps a button that
            // does not fit the current line, and the wrap decision happens
            // before a widget exists to measure.
            let cursor = ui.cursor();
            let width = estimated_button_width(ui, &command.display_label());
            let predicted_y = if cursor.min.x + width >= max_right {
                cursor.min.y + line_height
            } else {
                cursor.min.y
            };
            if !within_row_budget(predicted_y - row_origin_y, line_height, MAX_TOOLBAR_ROWS) {
                overflow_start = Some(index);
                break;
            }
        }
        let (was_clicked, rect) = command_button(ui, command, connected, no_run_tip);
        if was_clicked {
            clicked.push(command.clone());
        }
        if !seen_first {
            // The first button anchors the budget: its top is row zero and
            // egui separates wrapped toolbar lines by item_spacing.y on top
            // of the button height.
            row_origin_y = rect.min.y;
            line_height = rect.height() + item_spacing.y;
            seen_first = true;
        }
    }

    // Keep the ⋯ menu and the manage button on the same line: when the pair
    // would straddle a wrap, an invisible spacer flushes the cursor to the
    // next line first (a nested non-wrapping scope would just get clipped).
    if overflow_start.is_some() {
        let group_width = estimated_button_width(ui, "…")
            + estimated_button_width(ui, text(language, "quick_commands_manage"));
        let remaining = max_right - ui.cursor().min.x;
        if group_width > remaining {
            ui.allocate_exact_size(egui::vec2(remaining + 1.0, 0.0), egui::Sense::hover());
        }
    }
    if let Some(start) = overflow_start {
        ui.menu_button("…", |ui| {
            for command in &store.commands[start..] {
                let (was_clicked, _) = command_button(ui, command, connected, no_run_tip);
                if was_clicked {
                    clicked.push(command.clone());
                }
            }
        })
        .response
        .on_hover_text(text(language, "quick_commands_more"));
    }
    if ui.button(text(language, "quick_commands_manage")).clicked() {
        store.manage_open = true;
    }
    clicked
}

/// One quick-command button, inline or in the overflow menu. Returns whether
/// it was clicked plus its placed rectangle (the first inline button's rect
/// anchors the row budget).
fn command_button(
    ui: &mut egui::Ui,
    command: &QuickCommand,
    connected: bool,
    no_run_tip: &str,
) -> (bool, egui::Rect) {
    let tooltip = if command.run {
        command.command.clone()
    } else {
        format!("{}\n({no_run_tip})", command.command)
    };
    let response = ui
        .add_enabled(
            connected && command.is_sendable(),
            egui::Button::new(command.display_label()),
        )
        .on_hover_text(tooltip);
    (response.clicked(), response.rect)
}

/// Estimated rendered width of a quick-command button, used to predict a line
/// wrap before the button exists to be measured. Slightly padded on purpose:
/// over-predicting only moves a button into the ⋯ menu, under-predicting
/// would strand it on a third toolbar line.
fn estimated_button_width(ui: &egui::Ui, label: &str) -> f32 {
    let font_id = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Button)
        .cloned()
        .unwrap_or_default();
    let text_width = ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(label.to_owned(), font_id, egui::Color32::WHITE)
            .rect
            .width()
    });
    let padding = ui.style().spacing.button_padding;
    text_width + 2.0 * padding.x + ui.spacing().item_spacing.x + 2.0
}

/// Whether a button landing `offset_y` below the first quick-command row
/// still fits within the first `max_rows` toolbar lines.
fn within_row_budget(offset_y: f32, line_height: f32, max_rows: u16) -> bool {
    if line_height <= 0.0 {
        return true;
    }
    let row = (offset_y / line_height).round();
    row < f32::from(max_rows)
}

/// The manage window: edit, reorder, add and delete commands, plus JSON
/// import/export. Rendered against the egui context because `egui::Window`
/// needs one; the shell panel owns the call site.
pub fn show_manage_window(
    context: &egui::Context,
    language: Language,
    store: &mut QuickCommandStore,
) {
    let mut open = store.manage_open;
    egui::Window::new(text(language, "quick_commands_manage_title"))
        .open(&mut open)
        .default_width(640.0)
        .show(context, |ui| {
            ui.small(text(language, "quick_commands_manage_hint"));
            ui.add_space(6.0);
            let count = store.commands.len();
            egui::ScrollArea::vertical()
                .max_height(MANAGE_LIST_MAX_HEIGHT)
                .show(ui, |ui| {
                    if store.commands.is_empty() {
                        ui.small(text(language, "quick_commands_empty"));
                    }
                    for index in 0..count {
                        if edit_row(ui, language, store, index, count) {
                            // Row indices shifted; redraw everything next frame.
                            break;
                        }
                        ui.add_space(2.0);
                    }
                });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(text(language, "quick_commands_add")).clicked() {
                    store.commands.push(QuickCommand::default());
                }
                import_export_buttons(ui, language, store);
                render_status(ui, language, store.status.as_ref());
            });
        });
    store.manage_open = open;
}

/// One editable list row. Borrow-limited actions (delete, reorder) are
/// captured here and applied once the row's mutable borrow has ended.
/// Returns whether the list changed, so the caller can stop rendering the
/// remaining rows of this frame (their indices shifted underneath them).
fn edit_row(
    ui: &mut egui::Ui,
    language: Language,
    store: &mut QuickCommandStore,
    index: usize,
    count: usize,
) -> bool {
    let mut move_up = false;
    let mut move_down = false;
    let mut delete = false;
    let monospace = egui::FontId::monospace(13.0);
    ui.horizontal(|ui| {
        move_up = ui
            .add_enabled(
                index > 0,
                egui::Button::new(text(language, "quick_commands_up")),
            )
            .on_hover_text(text(language, "quick_commands_up"))
            .clicked();
        move_down = ui
            .add_enabled(
                index + 1 < count,
                egui::Button::new(text(language, "quick_commands_down")),
            )
            .on_hover_text(text(language, "quick_commands_down"))
            .clicked();
        let command = &mut store.commands[index];
        ui.add(
            egui::TextEdit::singleline(&mut command.label)
                .desired_width(120.0)
                .hint_text(text(language, "quick_commands_label")),
        );
        ui.add(
            egui::TextEdit::singleline(&mut command.command)
                .desired_width(280.0)
                .font(monospace)
                .hint_text(text(language, "quick_commands_command")),
        );
        ui.checkbox(&mut command.run, text(language, "quick_commands_run"))
            .on_hover_text(text(language, "quick_commands_run_tip"));
        // The multiplication sign renders in both bundled fonts; the more
        // obvious ✕ does not (see the window-control note in app.rs).
        delete = ui
            .button("×")
            .on_hover_text(text(language, "quick_commands_delete"))
            .clicked();
    });
    if move_up {
        store.commands.swap(index - 1, index);
    }
    if move_down {
        store.commands.swap(index, index + 1);
    }
    if delete {
        store.commands.remove(index);
    }
    move_up || move_down || delete
}

fn import_export_buttons(ui: &mut egui::Ui, language: Language, store: &mut QuickCommandStore) {
    if ui.button(text(language, "quick_commands_import")).clicked()
        && let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .pick_file()
    {
        store.status = Some(match import_from_path(&path) {
            Ok(commands) => QuickCommandStatus::Imported(store.import(commands)),
            Err(error) => QuickCommandStatus::Failed(error),
        });
    }
    if ui.button(text(language, "quick_commands_export")).clicked()
        && let Some(path) = rfd::FileDialog::new()
            .set_file_name("bridgescope-quick-commands.json")
            .add_filter("JSON", &["json"])
            .save_file()
    {
        store.status = Some(match export_to_path(&path, &store.commands) {
            Ok(count) => QuickCommandStatus::Exported(format!("{} ×{}", path.display(), count)),
            Err(error) => QuickCommandStatus::Failed(error),
        });
    }
}

fn render_status(ui: &mut egui::Ui, language: Language, status: Option<&QuickCommandStatus>) {
    let Some(status) = status else {
        return;
    };
    let message = match status {
        QuickCommandStatus::Imported(count) => format!(
            "{} {count} {}",
            text(language, "quick_commands_imported_prefix"),
            text(language, "quick_commands_imported_suffix"),
        ),
        QuickCommandStatus::Exported(detail) => format!(
            "{} {detail}",
            text(language, "quick_commands_exported_prefix")
        ),
        QuickCommandStatus::Failed(detail) => {
            format!("{}: {detail}", text(language, "quick_commands_failed"))
        }
    };
    let failure = matches!(status, QuickCommandStatus::Failed(_));
    let message = if failure {
        egui::RichText::new(message).color(egui::Color32::LIGHT_RED)
    } else {
        egui::RichText::new(message)
    };
    ui.small(message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_appends_newline_only_when_run() {
        let executing = QuickCommand::new("dmesg", "dmesg", true);
        assert_eq!(payload_chunks(&executing), [b"dmesg\n".to_vec()]);

        let typing = QuickCommand::new("cd", "cd /data/local/tmp", false);
        assert_eq!(payload_chunks(&typing), [b"cd /data/local/tmp".to_vec()]);
    }

    #[test]
    fn payload_splits_at_domain_limit() {
        let chunk_limit = bridgescope_domain::MAX_SHELL_INPUT_BYTES;
        let long = QuickCommand {
            command: "x".repeat(chunk_limit + 3),
            ..QuickCommand::new("big", "x", true)
        };
        let chunks = payload_chunks(&long);
        // limit+3 letters plus the trailing newline straddle two chunks.
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), chunk_limit);
        assert_eq!(chunks[1], b"xxx\n".to_vec());
    }

    #[test]
    fn import_skips_blank_and_duplicate_entries() {
        let mut store = QuickCommandStore::default();
        let baseline = store.commands.len();
        let known = store.commands[0].clone();
        let added = store.import(vec![
            QuickCommand::new("", "   ", true),
            QuickCommand::new(known.label.clone(), known.command.clone(), !known.run),
            QuickCommand::new("fresh", "echo fresh", false),
        ]);
        assert_eq!(added, 1);
        assert_eq!(store.commands.len(), baseline + 1);
        assert_eq!(
            store.commands.last().map(|command| command.label.as_str()),
            Some("fresh")
        );
    }

    #[test]
    fn export_import_round_trip_preserves_commands() {
        let commands = vec![
            QuickCommand::new("版本", "getprop ro.build.version.release", true),
            QuickCommand::new("", "cd /data/local/tmp", false),
        ];
        let path = std::env::temp_dir().join(format!(
            "bridgescope-quick-commands-test-{}.json",
            uuid::Uuid::new_v4()
        ));
        let exported = export_to_path(&path, &commands).expect("export succeeds");
        assert_eq!(exported, commands.len());
        let imported = import_from_path(&path).expect("import succeeds");
        std::fs::remove_file(&path).expect("cleanup succeeds");
        assert_eq!(imported, commands);
    }

    #[test]
    fn import_accepts_bare_arrays_and_defaults_run_to_true() {
        let path = std::env::temp_dir().join(format!(
            "bridgescope-quick-commands-array-{}.json",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(
            &path,
            r#"[{"label":"ls","command":"ls -l"},{"label":"d","command":"dmesg","run":false}]"#,
        )
        .expect("write succeeds");
        let imported = import_from_path(&path).expect("import succeeds");
        std::fs::remove_file(&path).expect("cleanup succeeds");
        assert_eq!(
            imported,
            vec![
                QuickCommand::new("ls", "ls -l", true),
                QuickCommand::new("d", "dmesg", false),
            ]
        );
    }

    #[test]
    fn display_label_falls_back_to_command_and_truncates() {
        let labelled = QuickCommand::new("  清理日志  ", "logcat -c", true);
        assert_eq!(labelled.display_label(), "清理日志");

        let unlabelled = QuickCommand::new("", "getprop", true);
        assert_eq!(unlabelled.display_label(), "getprop");

        let long_command = "x".repeat(MAX_BUTTON_LABEL_CHARS + 5);
        let truncated = QuickCommand::new("", long_command, true);
        let label = truncated.display_label();
        assert_eq!(label.chars().count(), MAX_BUTTON_LABEL_CHARS + 1);
        assert!(label.ends_with('…'));
    }

    #[test]
    fn row_budget_admits_two_lines_only() {
        let line_height = 26.0;
        // Row zero and row one stay inline; row two goes to the ⋯ menu.
        assert!(within_row_budget(0.0, line_height, MAX_TOOLBAR_ROWS));
        assert!(within_row_budget(
            line_height,
            line_height,
            MAX_TOOLBAR_ROWS
        ));
        assert!(!within_row_budget(
            2.0 * line_height,
            line_height,
            MAX_TOOLBAR_ROWS
        ));
        // Degenerate geometry never overflows (nothing rendered yet).
        assert!(within_row_budget(100.0, 0.0, MAX_TOOLBAR_ROWS));
    }
}
