use eframe::egui::{self, RichText};
use fadb_domain::{
    BackendCommand, BackendEvent, BridgeError, DeviceTarget, OperationId, WebViewPage,
};
use std::time::{Duration, Instant};

use crate::i18n::{Language, error_text, text};

/// Local TCP port the runtime forwards to the device's DevTools socket. One
/// port is enough: a fresh `adb forward` replaces the previous mapping.
pub const DEVTOOLS_LOCAL_PORT: u16 = 39222;

/// Backoff between automatic DevTools socket refreshes while none was found.
const AUTO_REFRESH_RETRY: Duration = Duration::from_secs(8);

#[derive(Default)]
enum Pending {
    #[default]
    None,
    Sockets(OperationId),
    Pages(OperationId),
}

#[derive(Default)]
pub struct WebviewPanelState {
    pub target: Option<DeviceTarget>,
    pending: Pending,
    pub sockets: Vec<String>,
    pub selected_socket: Option<String>,
    pub pages: Vec<WebViewPage>,
    pub active_port: Option<u16>,
    pub error: Option<BridgeError>,
    last_refresh: Option<Instant>,
    /// Socket the current page list belongs to: gates the auto fetch so a
    /// failing load does not spin, while a new selection fetches again.
    pages_requested_for: Option<String>,
}

impl WebviewPanelState {
    pub fn reset_for(&mut self, target: Option<DeviceTarget>) {
        if self.target != target {
            self.target = target;
            self.pending = Pending::None;
            self.sockets.clear();
            self.selected_socket = None;
            self.pages.clear();
            self.active_port = None;
            self.error = None;
            self.pages_requested_for = None;
        }
    }

    pub fn handle_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::WebviewSocketsLoading { request_id, .. } if matches!(&self.pending, Pending::Sockets(pending) if pending == request_id) =>
            {
                self.error = None;
            }
            BackendEvent::WebviewSocketsLoaded {
                request_id,
                sockets,
                ..
            } if matches!(&self.pending, Pending::Sockets(pending) if pending == request_id) => {
                self.pending = Pending::None;
                self.error = None;
                self.sockets.clone_from(sockets);
                self.selected_socket = sockets.first().cloned();
                self.pages.clear();
                self.active_port = None;
                self.pages_requested_for = None;
            }
            BackendEvent::WebviewPagesLoading { request_id, .. } if matches!(&self.pending, Pending::Pages(pending) if pending == request_id) =>
            {
                self.error = None;
            }
            BackendEvent::WebviewPagesLoaded {
                request_id,
                pages,
                port,
                ..
            } if matches!(&self.pending, Pending::Pages(pending) if pending == request_id) => {
                self.pending = Pending::None;
                self.error = None;
                self.pages.clone_from(pages);
                self.active_port = Some(*port);
            }
            BackendEvent::WebviewFailed {
                request_id, error, ..
            } if self.pending_request() == Some(*request_id) => {
                self.pending = Pending::None;
                self.error = Some(error.clone());
            }
            _ => {}
        }
    }

    fn pending_request(&self) -> Option<OperationId> {
        match &self.pending {
            Pending::Sockets(id) | Pending::Pages(id) => Some(*id),
            Pending::None => None,
        }
    }

    fn busy(&self) -> bool {
        !matches!(self.pending, Pending::None)
    }
}

/// Renders the WebView inspector: DevTools socket discovery, the debuggable
/// page list, and actions that open pages or the DevTools frontend.
#[allow(clippy::too_many_lines)]
pub fn show(
    ui: &mut egui::Ui,
    language: Language,
    state: &mut WebviewPanelState,
) -> Vec<BackendCommand> {
    let mut commands = Vec::new();
    ui.horizontal(|ui| {
        ui.heading(text(language, "webview"));
        ui.add_space(6.0);
        if state.busy() {
            ui.spinner();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(
                    state.target.is_some() && !state.busy(),
                    egui::Button::new(text(language, "webview_refresh_sockets")),
                )
                .clicked()
                && let Some(target) = state.target.clone()
            {
                let request_id = OperationId::new();
                state.pending = Pending::Sockets(request_id);
                state.last_refresh = Some(Instant::now());
                commands.push(BackendCommand::ListWebviewSockets { request_id, target });
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
    if state.target.is_none() {
        ui.label(text(language, "select_device"));
        return commands;
    }

    if state.sockets.is_empty() && !state.busy() && state.error.is_none() {
        ui.label(text(language, "webview_none_found"));
        return commands;
    }

    ui.strong(text(language, "webview_sockets"));
    let sockets = state.sockets.clone();
    egui::ScrollArea::horizontal().show(ui, |ui| {
        ui.horizontal(|ui| {
            for socket in &sockets {
                let selected = state.selected_socket.as_deref() == Some(socket.as_str());
                if ui.selectable_label(selected, socket).clicked() {
                    // Clicking the selected socket re-fetches: it doubles as
                    // a refresh for that socket's page list.
                    state.selected_socket = Some(socket.clone());
                    let request_id = OperationId::new();
                    state.pending = Pending::Pages(request_id);
                    state.pages_requested_for = Some(socket.clone());
                    commands.push(BackendCommand::ListWebviewPages {
                        request_id,
                        target: state.target.clone().expect("target checked above"),
                        socket: socket.clone(),
                        port: DEVTOOLS_LOCAL_PORT,
                    });
                }
            }
        });
    });
    ui.add_space(8.0);

    // The first socket is auto-selected when the list loads — an event
    // handler cannot emit commands, so fetch its pages here, once.
    if !state.busy()
        && state.pages.is_empty()
        && let Some(socket) = state.selected_socket.clone()
        && state.pages_requested_for.as_deref() != Some(socket.as_str())
        && let Some(target) = state.target.clone()
    {
        let request_id = OperationId::new();
        state.pending = Pending::Pages(request_id);
        state.pages_requested_for = Some(socket.clone());
        commands.push(BackendCommand::ListWebviewPages {
            request_id,
            target,
            socket,
            port: DEVTOOLS_LOCAL_PORT,
        });
    }

    ui.strong(text(language, "webview_pages"));
    if state.selected_socket.is_none() {
        ui.label(text(language, "webview_select_socket"));
        return commands;
    }
    if state.pages.is_empty() && !state.busy() {
        ui.label(text(language, "webview_no_pages"));
        return commands;
    }

    let port = state.active_port;
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .id_salt("webview-pages")
        .show(ui, |ui| {
            egui::Grid::new("webview-page-grid")
                .striped(true)
                .num_columns(4)
                .min_col_width(90.0)
                .show(ui, |ui| {
                    ui.strong(text(language, "webview_col_title"));
                    ui.strong(text(language, "webview_col_url"));
                    ui.strong(text(language, "webview_col_type"));
                    ui.strong(text(language, "webview_col_action"));
                    ui.end_row();
                    for page in &state.pages {
                        ui.label(if page.title.is_empty() {
                            "(untitled)"
                        } else {
                            page.title.as_str()
                        });
                        ui.add(
                            egui::Label::new(RichText::new(&page.url).monospace().size(11.5))
                                .truncate(),
                        );
                        ui.label(page.kind.clone());
                        ui.horizontal(|ui| {
                            if ui.button(text(language, "webview_open_page")).clicked() {
                                ui.ctx()
                                    .open_url(egui::OpenUrl::new_tab(page.url.clone()));
                            }
                            let ws_path = page
                                .debugger_url
                                .strip_prefix("ws://127.0.0.1")
                                .and_then(|rest| rest.split_once('/'))
                                .map(|(_, path)| path);
                            if let (Some(ws_path), Some(port)) = (ws_path, port)
                            {
                                let inspector = format!(
                                    "http://127.0.0.1:{port}/devtools/inspector.html?ws=127.0.0.1:{port}/{ws_path}"
                                );
                                if ui
                                    .button(text(language, "webview_open_devtools"))
                                    .clicked()
                                {
                                    ui.ctx()
                                        .open_url(egui::OpenUrl::new_tab(inspector.clone()));
                                }
                                if ui
                                    .button(text(language, "webview_copy_url"))
                                    .clicked()
                                {
                                    ui.ctx().copy_text(inspector);
                                }
                            }
                        });
                        ui.end_row();
                    }
                });
        });
    ui.add_space(6.0);
    ui.small(text(language, "webview_debug_hint"));
    commands
}

/// Called by the app shell each frame: discover DevTools sockets automatically
/// while none has been found yet, so plug-in WebViews show up without a
/// manual refresh.
pub fn auto_refresh(state: &mut WebviewPanelState, target_online: bool) -> Option<BackendCommand> {
    if !target_online || state.busy() || !state.sockets.is_empty() {
        return None;
    }
    let recently_refreshed = state
        .last_refresh
        .is_some_and(|last| last.elapsed() < AUTO_REFRESH_RETRY);
    if recently_refreshed {
        return None;
    }
    let target = state.target.clone()?;
    let request_id = OperationId::new();
    state.pending = Pending::Sockets(request_id);
    state.last_refresh = Some(Instant::now());
    Some(BackendCommand::ListWebviewSockets { request_id, target })
}
