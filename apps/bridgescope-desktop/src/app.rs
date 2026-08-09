use std::time::{Duration, Instant};

use bridgescope_domain::{
    AdbEndpoint, BackendCommand, BackendEvent, BridgeError, DeviceOverview, DeviceRecord,
    DeviceSnapshot,
};
use eframe::egui::{self, Color32, RichText};

use crate::{
    i18n::{Language, text},
    panels::{assistant, files, overview, performance, processes, screenshot, shell},
    runtime::RuntimeBridge,
    theme,
};

const RECENT_ENDPOINTS_STORAGE_KEY: &str = "bridgescope.recent_adb_endpoints";
const MAX_RECENT_ENDPOINTS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Panel {
    Overview,
    Files,
    Applications,
    Processes,
    Performance,
    Shell,
    Layout,
    Screenshot,
    Logcat,
    WebView,
}

impl Panel {
    const ALL: [Self; 10] = [
        Self::Overview,
        Self::Files,
        Self::Applications,
        Self::Processes,
        Self::Performance,
        Self::Shell,
        Self::Layout,
        Self::Screenshot,
        Self::Logcat,
        Self::WebView,
    ];

    const fn key(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Files => "files",
            Self::Applications => "applications",
            Self::Processes => "processes",
            Self::Performance => "performance",
            Self::Shell => "shell",
            Self::Layout => "layout",
            Self::Screenshot => "screenshot",
            Self::Logcat => "logcat",
            Self::WebView => "webview",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum AssistantPlacement {
    #[default]
    Hidden,
    DockedRight,
    Floating,
}

#[derive(Default)]
struct WindowState {
    devices: bool,
    diagnostics: bool,
}

pub struct BridgeScopeApp {
    runtime: RuntimeBridge,
    snapshot: DeviceSnapshot,
    overview: Option<DeviceOverview>,
    loading_overview: bool,
    shell: shell::ShellPanelState,
    screenshot: screenshot::ScreenshotPanelState,
    assistant: assistant::AssistantPanelState,
    assistant_placement: AssistantPlacement,
    files: files::FilesPanelState,
    processes: processes::ProcessesPanelState,
    performance: performance::PerformancePanelState,
    last_process_refresh: Option<Instant>,
    last_performance_refresh: Option<Instant>,
    active_panel: Panel,
    language: Language,
    dark_mode: bool,
    adb_path: Option<String>,
    adb_version: Option<String>,
    recent_endpoints: Vec<AdbEndpoint>,
    endpoint_host: String,
    endpoint_port: String,
    connecting_endpoint: Option<AdbEndpoint>,
    last_error: Option<BridgeError>,
    windows: WindowState,
}

impl BridgeScopeApp {
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        theme::configure(&creation_context.egui_ctx);
        let runtime = RuntimeBridge::spawn(creation_context.egui_ctx.clone());
        Self {
            runtime,
            snapshot: DeviceSnapshot::default(),
            overview: None,
            loading_overview: false,
            shell: shell::ShellPanelState::default(),
            screenshot: screenshot::ScreenshotPanelState::default(),
            assistant: assistant::AssistantPanelState::default(),
            assistant_placement: AssistantPlacement::Hidden,
            files: files::FilesPanelState::default(),
            processes: processes::ProcessesPanelState::default(),
            performance: performance::PerformancePanelState::default(),
            last_process_refresh: None,
            last_performance_refresh: None,
            active_panel: Panel::Overview,
            language: Language::Chinese,
            dark_mode: true,
            adb_path: None,
            adb_version: None,
            recent_endpoints: load_recent_endpoints(creation_context.storage),
            endpoint_host: String::new(),
            endpoint_port: "5555".to_owned(),
            connecting_endpoint: None,
            last_error: None,
            windows: WindowState::default(),
        }
    }

    fn process_events(&mut self) {
        for event in self.runtime.drain() {
            self.shell.handle_event(&event);
            self.screenshot
                .handle_event(&self.runtime.context(), &event);
            self.assistant.handle_event(&event);
            self.processes.handle_event(&event);
            self.performance.handle_event(&event);
            let file_commands = self.files.handle_event(&event);
            for command in file_commands {
                self.send(command);
            }
            match event {
                BackendEvent::AdbReady { path, version } => {
                    self.adb_path = Some(path);
                    self.adb_version = Some(version);
                    self.last_error = None;
                }
                BackendEvent::AdbUnavailable(error) => self.last_error = Some(error),
                BackendEvent::AdbConnecting(endpoint) => {
                    self.connecting_endpoint = Some(endpoint);
                    self.last_error = None;
                }
                BackendEvent::AdbConnected(endpoint) => {
                    self.connecting_endpoint = None;
                    self.remember_endpoint(endpoint);
                    self.last_error = None;
                }
                BackendEvent::AdbConnectFailed { endpoint, error } => {
                    self.connecting_endpoint = None;
                    self.last_error = Some(BridgeError::new(
                        error.code,
                        error.message_key,
                        format!("{} ({endpoint})", error.detail),
                    ));
                }
                BackendEvent::DevicesChanged(snapshot) => {
                    if snapshot.selected != self.snapshot.selected {
                        self.overview = None;
                    }
                    self.snapshot = snapshot;
                    let target = self.selected_record().map(DeviceRecord::target);
                    self.processes.reset_for(target.clone());
                    self.performance.reset_for(target);
                    self.last_process_refresh = None;
                    self.last_performance_refresh = None;
                }
                BackendEvent::OverviewLoading(serial) => {
                    self.loading_overview = self.snapshot.selected.as_ref() == Some(&serial);
                }
                BackendEvent::OverviewLoaded(overview) => {
                    if self.snapshot.selected.as_ref() == Some(&overview.serial) {
                        self.overview = Some(overview);
                        self.loading_overview = false;
                        self.last_error = None;
                    }
                }
                BackendEvent::ProcessesFailed { error, .. }
                | BackendEvent::PerformanceFailed { error, .. } => {
                    self.last_error = Some(error);
                }
                BackendEvent::OperationFailed(error) => {
                    self.loading_overview = false;
                    self.last_error = Some(error);
                }
                BackendEvent::ShellOpened { .. }
                | BackendEvent::ShellOutput { .. }
                | BackendEvent::ShellClosed { .. }
                | BackendEvent::ShellFailed { .. }
                | BackendEvent::ProcessesLoading(_)
                | BackendEvent::ProcessesLoaded(_)
                | BackendEvent::PerformanceLoading(_)
                | BackendEvent::PerformanceLoaded(_)
                | BackendEvent::ScreenshotLoading { .. }
                | BackendEvent::ScreenshotCaptured { .. }
                | BackendEvent::ScreenshotFailed { .. }
                | BackendEvent::AiReady { .. }
                | BackendEvent::AiUnavailable { .. }
                | BackendEvent::AiChatCompleted { .. }
                | BackendEvent::AiChatFailed { .. }
                | BackendEvent::DirectoryLoading { .. }
                | BackendEvent::DirectoryLoaded { .. }
                | BackendEvent::DirectoryFailed { .. }
                | BackendEvent::FileTransferStarted { .. }
                | BackendEvent::FileTransferCompleted { .. }
                | BackendEvent::FileTransferFailed { .. }
                | BackendEvent::FileTransferCancelled { .. }
                | BackendEvent::FileMutationStarted { .. }
                | BackendEvent::FileMutationCompleted { .. }
                | BackendEvent::FileMutationFailed { .. } => {}
            }
        }
    }

    fn selected_record(&self) -> Option<&DeviceRecord> {
        let selected = self.snapshot.selected.as_ref()?;
        self.snapshot
            .devices
            .iter()
            .find(|record| &record.descriptor.serial == selected)
    }

    fn send(&mut self, command: BackendCommand) {
        if let Err(error) = self.runtime.try_send(command) {
            self.last_error = Some(error);
        }
    }

    fn endpoint_from_inputs(&self) -> Result<AdbEndpoint, BridgeError> {
        let port = self
            .endpoint_port
            .trim()
            .parse::<u16>()
            .map_err(|_| BridgeError::invalid_input("adb.endpoint.port_invalid"))?;
        AdbEndpoint::new(self.endpoint_host.clone(), port)
    }

    fn connect_endpoint(&mut self, endpoint: AdbEndpoint) {
        endpoint.host().clone_into(&mut self.endpoint_host);
        self.endpoint_port = endpoint.port().to_string();
        self.connecting_endpoint = Some(endpoint.clone());
        self.send(BackendCommand::ConnectDevice(endpoint));
    }

    fn remember_endpoint(&mut self, endpoint: AdbEndpoint) {
        self.recent_endpoints.retain(|known| known != &endpoint);
        self.recent_endpoints.insert(0, endpoint);
        self.recent_endpoints.truncate(MAX_RECENT_ENDPOINTS);
    }

    fn top_bar(&mut self, context: &egui::Context) {
        egui::TopBottomPanel::top("top-bar").show(context, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("BridgeScope").size(21.0).strong());
                ui.separator();

                let selected_text = self.selected_record().map_or_else(
                    || text(self.language, "select_device").to_owned(),
                    |record| {
                        format!(
                            "{} ({})",
                            record.descriptor.display_name(),
                            record.descriptor.serial
                        )
                    },
                );

                egui::ComboBox::from_id_salt("device-selector")
                    .selected_text(selected_text)
                    .width(300.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(
                                self.snapshot.selected.is_none(),
                                text(self.language, "select_device"),
                            )
                            .clicked()
                        {
                            self.send(BackendCommand::SelectDevice(None));
                        }
                        let devices = self.snapshot.devices.clone();
                        for record in devices {
                            let serial = record.descriptor.serial.clone();
                            let is_selected = self.snapshot.selected.as_ref() == Some(&serial);
                            let label = format!(
                                "{} · {:?} · {}",
                                record.descriptor.display_name(),
                                record.descriptor.state,
                                serial
                            );
                            if ui.selectable_label(is_selected, label).clicked() {
                                self.send(BackendCommand::SelectDevice(Some(serial)));
                            }
                        }
                    });

                if ui.button(text(self.language, "refresh")).clicked() {
                    self.send(BackendCommand::RefreshDevices);
                    if let Some(serial) = self.snapshot.selected.clone() {
                        self.send(BackendCommand::LoadOverview(serial));
                    }
                }
                if ui.button(text(self.language, "device_manager")).clicked() {
                    self.windows.devices = true;
                }
                if ui.button(text(self.language, "diagnostics")).clicked() {
                    self.windows.diagnostics = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(self.language.short_name()).clicked() {
                        self.language = self.language.toggle();
                    }
                    let theme_label = if self.dark_mode {
                        text(self.language, "light")
                    } else {
                        text(self.language, "dark")
                    };
                    if ui.button(theme_label).clicked() {
                        self.dark_mode = !self.dark_mode;
                        context.set_visuals(if self.dark_mode {
                            egui::Visuals::dark()
                        } else {
                            egui::Visuals::light()
                        });
                    }
                });
            });
        });
    }

    fn side_bar(&mut self, context: &egui::Context) {
        egui::SidePanel::left("navigation")
            .resizable(false)
            .default_width(170.0)
            .show(context, |ui| {
                ui.add_space(8.0);
                for panel in Panel::ALL {
                    if ui
                        .selectable_label(
                            self.active_panel == panel,
                            text(self.language, panel.key()),
                        )
                        .clicked()
                    {
                        self.active_panel = panel;
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.small("BridgeScope 0.1 foundation");
                    if ui
                        .selectable_label(
                            self.assistant_placement != AssistantPlacement::Hidden,
                            text(self.language, "assistant"),
                        )
                        .clicked()
                    {
                        self.assistant_placement =
                            if self.assistant_placement == AssistantPlacement::Hidden {
                                AssistantPlacement::DockedRight
                            } else {
                                AssistantPlacement::Hidden
                            };
                    }
                });
            });
    }

    fn central_panel(&mut self, context: &egui::Context) {
        self.refresh_live_panels(context);
        egui::CentralPanel::default().show(context, |ui| {
            if let Some(error) = &self.last_error {
                egui::Frame::new()
                    .fill(Color32::from_rgb(90, 34, 34))
                    .corner_radius(6.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new(&error.message_key).strong());
                        ui.label(&error.detail);
                    });
                ui.add_space(10.0);
            }

            let selected = self.selected_record().cloned();
            let commands = match self.active_panel {
                Panel::Overview => {
                    overview::show(
                        ui,
                        self.language,
                        selected.as_ref(),
                        self.overview.as_ref(),
                        self.loading_overview,
                    );
                    Vec::new()
                }
                Panel::Shell => shell::show(ui, selected.as_ref(), &mut self.shell),
                Panel::Screenshot => screenshot::show(ui, selected.as_ref(), &mut self.screenshot),
                Panel::Files => files::show(
                    ui,
                    &mut self.files,
                    selected.as_ref().map(DeviceRecord::target).as_ref(),
                ),
                Panel::Processes => processes::show(ui, self.language, &mut self.processes),
                Panel::Performance => performance::show(ui, self.language, &mut self.performance),
                _ => {
                    ui.vertical_centered(|ui| {
                        ui.add_space(100.0);
                        ui.heading(text(self.language, self.active_panel.key()));
                        ui.add_space(8.0);
                        ui.label(text(self.language, "coming_soon"));
                        ui.small("A panel is enabled only after its backend, UI states, tests, and real-device acceptance pass.");
                    });
                    Vec::new()
                }
            };
            for command in commands {
                self.send(command);
            }
        });
    }

    fn refresh_live_panels(&mut self, context: &egui::Context) {
        let target = self
            .selected_record()
            .filter(|record| record.descriptor.state.is_online())
            .map(DeviceRecord::target);
        self.processes.reset_for(target.clone());
        self.performance.reset_for(target.clone());
        let now = Instant::now();
        if self.active_panel == Panel::Processes {
            context.request_repaint_after(Duration::from_millis(250));
            if let Some(target) = target.clone()
                && !self.processes.loading
                && self
                    .last_process_refresh
                    .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(3))
            {
                self.last_process_refresh = Some(now);
                self.send(BackendCommand::LoadProcesses(target));
            }
        }
        if self.active_panel == Panel::Performance {
            context.request_repaint_after(Duration::from_millis(250));
            if let Some(target) = target
                && !self.performance.loading
                && self
                    .last_performance_refresh
                    .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
            {
                self.last_performance_refresh = Some(now);
                self.send(BackendCommand::LoadPerformance(target));
            }
        }
    }

    fn assistant_dock(&mut self, context: &egui::Context) {
        egui::SidePanel::right("assistant-dock")
            .resizable(true)
            .default_width(360.0)
            .width_range(280.0..=640.0)
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Float").clicked() {
                        self.assistant_placement = AssistantPlacement::Floating;
                    }
                    if ui.button("Close").clicked() {
                        self.assistant_placement = AssistantPlacement::Hidden;
                    }
                });
                for command in assistant::show(ui, &mut self.assistant) {
                    self.send(command);
                }
            });
    }

    fn assistant_window(&mut self, context: &egui::Context) {
        let mut open = true;
        egui::Window::new(text(self.language, "assistant"))
            .open(&mut open)
            .default_size([420.0, 600.0])
            .show(context, |ui| {
                if ui.button("Dock right").clicked() {
                    self.assistant_placement = AssistantPlacement::DockedRight;
                }
                for command in assistant::show(ui, &mut self.assistant) {
                    self.send(command);
                }
            });
        if !open {
            self.assistant_placement = AssistantPlacement::Hidden;
        }
    }

    fn device_manager(&mut self, context: &egui::Context) {
        let mut open = self.windows.devices;
        egui::Window::new(text(self.language, "device_manager"))
            .open(&mut open)
            .default_size([760.0, 520.0])
            .show(context, |ui| {
                ui.heading(text(self.language, "connect_android"));
                ui.horizontal(|ui| {
                    ui.label(text(self.language, "ip_host"));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.endpoint_host)
                            .desired_width(220.0)
                            .hint_text("192.168.1.20"),
                    );
                    ui.label(text(self.language, "port"));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.endpoint_port)
                            .desired_width(80.0)
                            .hint_text("5555"),
                    );
                    let connecting = self.connecting_endpoint.is_some();
                    if ui
                        .add_enabled(
                            !connecting,
                            egui::Button::new(text(self.language, "connect")),
                        )
                        .clicked()
                    {
                        match self.endpoint_from_inputs() {
                            Ok(endpoint) => self.connect_endpoint(endpoint),
                            Err(error) => self.last_error = Some(error),
                        }
                    }
                });
                if let Some(endpoint) = &self.connecting_endpoint {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("{} {endpoint}…", text(self.language, "connecting")));
                    });
                }
                if !self.recent_endpoints.is_empty() {
                    ui.separator();
                    ui.strong(text(self.language, "recent_network_devices"));
                    let recent = self.recent_endpoints.clone();
                    for endpoint in recent {
                        ui.horizontal(|ui| {
                            ui.label(endpoint.to_string());
                            if ui
                                .add_enabled(
                                    self.connecting_endpoint.is_none(),
                                    egui::Button::new(text(self.language, "connect")),
                                )
                                .clicked()
                            {
                                self.connect_endpoint(endpoint.clone());
                            }
                            if ui.button(text(self.language, "forget")).clicked() {
                                self.recent_endpoints.retain(|known| known != &endpoint);
                            }
                        });
                    }
                }
                ui.separator();
                ui.heading(text(self.language, "connected_devices"));
                if self.snapshot.devices.is_empty() {
                    ui.label(text(self.language, "no_device"));
                } else {
                    egui::Grid::new("device-manager-grid")
                        .striped(true)
                        .num_columns(5)
                        .show(ui, |ui| {
                            ui.strong(text(self.language, "model"));
                            ui.strong(text(self.language, "serial"));
                            ui.strong(text(self.language, "state"));
                            ui.strong(text(self.language, "product"));
                            ui.strong(text(self.language, "action"));
                            ui.end_row();
                            let devices = self.snapshot.devices.clone();
                            for record in devices {
                                ui.label(record.descriptor.display_name());
                                ui.label(record.descriptor.serial.as_str());
                                ui.label(format!("{:?}", record.descriptor.state));
                                ui.label(record.descriptor.product.as_deref().unwrap_or("—"));
                                let selected = self.snapshot.selected.as_ref()
                                    == Some(&record.descriptor.serial);
                                if ui
                                    .selectable_label(selected, text(self.language, "select"))
                                    .clicked()
                                {
                                    self.send(BackendCommand::SelectDevice(Some(
                                        record.descriptor.serial.clone(),
                                    )));
                                }
                                ui.end_row();
                            }
                        });
                }
            });
        self.windows.devices = open;
    }

    fn diagnostics(&mut self, context: &egui::Context) {
        let mut open = self.windows.diagnostics;
        egui::Window::new(text(self.language, "diagnostics"))
            .open(&mut open)
            .default_width(620.0)
            .show(context, |ui| {
                ui.strong("ADB executable");
                ui.label(self.adb_path.as_deref().unwrap_or("Not available; fake fallback may be active"));
                ui.add_space(8.0);
                ui.strong("Version");
                ui.label(self.adb_version.as_deref().unwrap_or("Unknown"));
                ui.add_space(8.0);
                ui.label(format!("Detected devices: {}", self.snapshot.devices.len()));
                ui.small("Set BRIDGESCOPE_ADB to choose an explicit adb executable. Set BRIDGESCOPE_FAKE=1 for deterministic development data.");
            });
        self.windows.diagnostics = open;
    }
}

impl eframe::App for BridgeScopeApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_events();
        if let Some(command) = self
            .files
            .reconcile_target(self.selected_record().map(DeviceRecord::target))
        {
            self.send(command);
        }
        self.top_bar(context);
        self.side_bar(context);
        if self.assistant_placement == AssistantPlacement::DockedRight {
            self.assistant_dock(context);
        }
        self.central_panel(context);
        if self.assistant_placement == AssistantPlacement::Floating {
            self.assistant_window(context);
        }
        if self.windows.devices {
            self.device_manager(context);
        }
        if self.windows.diagnostics {
            self.diagnostics(context);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(serialized) = serde_json::to_string(&self.recent_endpoints) {
            storage.set_string(RECENT_ENDPOINTS_STORAGE_KEY, serialized);
        }
    }
}

fn load_recent_endpoints(storage: Option<&dyn eframe::Storage>) -> Vec<AdbEndpoint> {
    let Some(serialized) =
        storage.and_then(|storage| storage.get_string(RECENT_ENDPOINTS_STORAGE_KEY))
    else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<AdbEndpoint>>(&serialized)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|endpoint| AdbEndpoint::new(endpoint.host(), endpoint.port()).ok())
        .take(MAX_RECENT_ENDPOINTS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_expected_panels_are_present() {
        assert_eq!(Panel::ALL.len(), 10);
        assert_eq!(Panel::ALL[0], Panel::Overview);
        assert_eq!(Panel::ALL[9], Panel::WebView);
    }

    #[test]
    fn initial_device_snapshot_has_no_implicit_selection() {
        assert_eq!(
            DeviceSnapshot::default().selected,
            None::<bridgescope_domain::DeviceSerial>
        );
    }
}
