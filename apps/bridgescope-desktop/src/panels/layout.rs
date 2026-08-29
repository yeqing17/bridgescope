use bridgescope_domain::{
    BackendCommand, BackendEvent, BridgeError, DeviceTarget, LayoutNode, LayoutSnapshot,
    OperationId,
};
use eframe::egui::{self, RichText};
use std::time::{Duration, Instant};

use crate::i18n::{Language, error_text, text};

/// Backoff between automatic captures so a failing dump does not spin.
const AUTO_CAPTURE_RETRY: Duration = Duration::from_secs(8);

#[derive(Default)]
pub struct LayoutPanelState {
    pub target: Option<DeviceTarget>,
    request: Option<OperationId>,
    pub loading: bool,
    pub snapshot: Option<LayoutSnapshot>,
    pub error: Option<BridgeError>,
    selected: Option<usize>,
    query: String,
    last_capture_attempt: Option<Instant>,
}

impl LayoutPanelState {
    pub fn reset_for(&mut self, target: Option<DeviceTarget>) {
        if self.target != target {
            self.target = target;
            self.request = None;
            self.loading = false;
            self.snapshot = None;
            self.error = None;
            self.selected = None;
            self.last_capture_attempt = None;
        }
    }

    pub fn handle_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::LayoutLoading { request_id, .. }
                if self.request.as_ref() == Some(request_id) =>
            {
                self.loading = true;
                self.error = None;
            }
            BackendEvent::LayoutCaptured {
                request_id,
                snapshot,
            } if self.request.as_ref() == Some(request_id) => {
                self.loading = false;
                self.error = None;
                self.request = None;
                self.snapshot = Some(snapshot.clone());
                self.selected = None;
            }
            BackendEvent::LayoutFailed {
                request_id, error, ..
            } if self.request.as_ref() == Some(request_id) => {
                self.loading = false;
                self.error = Some(error.clone());
                self.request = None;
            }
            _ => {}
        }
    }

    fn selected_node(&self) -> Option<&LayoutNode> {
        let root = &self.snapshot.as_ref()?.root;
        let id = self.selected?;
        find_node(root, id)
    }
}

fn find_node(node: &LayoutNode, id: usize) -> Option<&LayoutNode> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|child| find_node(child, id))
}

/// Case-insensitive match against the fields users search by.
fn node_matches(node: &LayoutNode, query: &str) -> bool {
    node.class.to_lowercase().contains(query)
        || node.resource_id.to_lowercase().contains(query)
        || node.text.to_lowercase().contains(query)
        || node.content_description.to_lowercase().contains(query)
}

fn subtree_matches(node: &LayoutNode, query: &str) -> bool {
    node_matches(node, query)
        || node
            .children
            .iter()
            .any(|child| subtree_matches(child, query))
}

/// Renders the layout inspector: capture controls, a searchable view tree and
/// an attribute pane for the selected node.
#[allow(clippy::too_many_lines)]
pub fn show(
    ui: &mut egui::Ui,
    language: Language,
    state: &mut LayoutPanelState,
) -> Vec<BackendCommand> {
    let mut commands = Vec::new();
    ui.horizontal(|ui| {
        ui.heading(text(language, "layout"));
        ui.add_space(6.0);
        if state.loading {
            ui.spinner();
            ui.label(text(language, "layout_loading"));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(
                    state.target.is_some() && !state.loading,
                    egui::Button::new(text(language, "refresh")),
                )
                .clicked()
                && let Some(target) = state.target.clone()
            {
                let request_id = OperationId::new();
                state.request = Some(request_id);
                state.error = None;
                state.last_capture_attempt = Some(Instant::now());
                commands.push(BackendCommand::CaptureLayout { request_id, target });
            }
            if ui
                .add_enabled(
                    state.snapshot.is_some(),
                    egui::Button::new(text(language, "layout_export")),
                )
                .clicked()
                && let Some(snapshot) = state.snapshot.as_ref()
                && let Some(path) = rfd::FileDialog::new()
                    .set_file_name("bridgescope-ui.xml")
                    .add_filter("XML", &["xml"])
                    .save_file()
                && let Err(error) = std::fs::write(&path, &snapshot.raw_xml)
            {
                tracing::warn!(%error, "layout export failed");
            }
        });
    });
    ui.add_space(4.0);

    if let Some(error) = &state.error {
        ui.label(
            RichText::new(error_text(language, error))
                .color(egui::Color32::from_rgb(248, 113, 113)),
        );
        ui.add_space(4.0);
    }

    let Some(snapshot) = state.snapshot.as_ref() else {
        if state.target.is_some() && !state.loading {
            ui.label(text(language, "layout_empty"));
        } else if state.target.is_none() {
            ui.label(text(language, "select_device"));
        }
        return commands;
    };

    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.query)
                .desired_width(240.0)
                .hint_text(text(language, "layout_search_hint")),
        );
        ui.label(
            RichText::new(format!(
                "{}: {}",
                text(language, "layout_node_count"),
                snapshot.root.count()
            ))
            .weak(),
        );
    });
    ui.add_space(4.0);

    egui::SidePanel::right("layout-attributes")
        .resizable(true)
        .default_width(320.0)
        .width_range(240.0..=480.0)
        .show_inside(ui, |ui| {
            ui.heading(text(language, "layout_attributes"));
            ui.add_space(4.0);
            match state.selected_node() {
                Some(node) => attribute_grid(ui, language, node),
                None => {
                    ui.label(text(language, "layout_no_selection"));
                }
            }
        });

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .id_salt("layout-tree")
        .show(ui, |ui| {
            let query = state.query.trim().to_lowercase();
            let mut selected = state.selected;
            render_node(ui, language, &snapshot.root, &query, 0, &mut selected);
            state.selected = selected;
        });
    commands
}

fn render_node(
    ui: &mut egui::Ui,
    language: Language,
    node: &LayoutNode,
    query: &str,
    depth: usize,
    selected: &mut Option<usize>,
) -> bool {
    let filtered = !query.is_empty();
    if filtered && !subtree_matches(node, query) {
        return false;
    }
    let matches = filtered && node_matches(node, query);
    let label = node_label(language, node);
    let header = if matches {
        RichText::new(label)
            .strong()
            .color(egui::Color32::from_rgb(250, 204, 21))
    } else {
        RichText::new(label)
    };
    if node.children.is_empty() {
        if ui
            .selectable_label(*selected == Some(node.id), header)
            .clicked()
        {
            *selected = Some(node.id);
        }
        return true;
    }
    let response = egui::CollapsingHeader::new(header)
        .id_salt(("layout-node", node.id))
        .default_open(depth < 2 || filtered)
        .show(ui, |ui| {
            for child in &node.children {
                render_node(ui, language, child, query, depth + 1, selected);
            }
        });
    if response.header_response.clicked() {
        *selected = Some(node.id);
    }
    true
}

/// `TextView · "Hello" · …/id/title` — short class, quoted text, resource id.
fn node_label(language: Language, node: &LayoutNode) -> String {
    let mut label = node
        .class
        .rsplit('.')
        .next()
        .unwrap_or(node.class.as_str())
        .to_owned();
    if !node.text.is_empty() {
        label = format!("{label} · {:?}", node.text);
    } else if !node.content_description.is_empty() {
        label = format!("{label} · [{}]", node.content_description);
    }
    if !node.resource_id.is_empty() {
        let id = node
            .resource_id
            .rsplit('/')
            .next()
            .unwrap_or(node.resource_id.as_str());
        label = format!("{label} · {id}");
    }
    if !node.enabled {
        label = format!("{label} · {}", text(language, "attr_disabled"));
    }
    label
}

fn attribute_grid(ui: &mut egui::Ui, language: Language, node: &LayoutNode) {
    let bounds = format!(
        "({}, {}, {}, {})",
        node.bounds[0], node.bounds[1], node.bounds[2], node.bounds[3]
    );
    let flag = |value: bool| {
        if value {
            text(language, "attr_yes")
        } else {
            text(language, "attr_no")
        }
    };
    egui::Grid::new("layout-attr-grid")
        .striped(true)
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            row(ui, text(language, "attr_class"), node.class.clone());
            row(
                ui,
                text(language, "attr_resource_id"),
                node.resource_id.clone(),
            );
            row(ui, text(language, "attr_text"), node.text.clone());
            row(
                ui,
                text(language, "attr_content_desc"),
                node.content_description.clone(),
            );
            row(ui, text(language, "attr_package"), node.package.clone());
            row(ui, text(language, "attr_bounds"), bounds);
            row(
                ui,
                text(language, "attr_children"),
                node.children.len().to_string(),
            );
            row(
                ui,
                text(language, "attr_clickable"),
                flag(node.clickable).to_owned(),
            );
            row(
                ui,
                text(language, "attr_scrollable"),
                flag(node.scrollable).to_owned(),
            );
            row(
                ui,
                text(language, "attr_enabled"),
                flag(node.enabled).to_owned(),
            );
            row(
                ui,
                text(language, "attr_selected"),
                flag(node.selected).to_owned(),
            );
            row(
                ui,
                text(language, "attr_focused"),
                flag(node.focused).to_owned(),
            );
        });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button(text(language, "copy")).clicked() {
            ui.ctx().copy_text(format_node_dump(node));
        }
        ui.small(text(language, "layout_copy_hint"));
    });
}

fn row(ui: &mut egui::Ui, label: &str, value: String) {
    ui.strong(label);
    if value.is_empty() {
        ui.label("—");
    } else {
        ui.label(value);
    }
    ui.end_row();
}

fn format_node_dump(node: &LayoutNode) -> String {
    let mut dump = String::new();
    walk_node(node, 0, &mut dump);
    dump
}

fn walk_node(node: &LayoutNode, depth: usize, out: &mut String) {
    use std::fmt::Write as _;
    let _ = writeln!(
        out,
        "{}{} bounds=({}, {}, {}, {}) text={:?} id={}",
        "  ".repeat(depth),
        node.class,
        node.bounds[0],
        node.bounds[1],
        node.bounds[2],
        node.bounds[3],
        node.text,
        node.resource_id,
    );
    for child in &node.children {
        walk_node(child, depth + 1, out);
    }
}

/// Called by the app shell each frame: capture the hierarchy automatically the
/// first time the panel is on screen, retrying with backoff while it is empty.
pub fn auto_capture(state: &mut LayoutPanelState, target_online: bool) -> Option<BackendCommand> {
    if !target_online || state.loading || state.request.is_some() || state.snapshot.is_some() {
        return None;
    }
    let recently_attempted = state
        .last_capture_attempt
        .is_some_and(|last| last.elapsed() < AUTO_CAPTURE_RETRY);
    if recently_attempted {
        return None;
    }
    let target = state.target.clone()?;
    let request_id = OperationId::new();
    state.request = Some(request_id);
    state.last_capture_attempt = Some(Instant::now());
    Some(BackendCommand::CaptureLayout { request_id, target })
}
