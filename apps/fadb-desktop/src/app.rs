use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};
use fadb_domain::{
    AdbEndpoint, AiSettings, BackendCommand, BackendEvent, BridgeError, DeviceOverview,
    DeviceRecord, DeviceSnapshot,
};

use crate::{
    i18n::{Language, text},
    panels::{
        applications, assistant, files, layout, logcat, mirror, overview, performance, processes,
        screenshot, shell, webview,
    },
    platform,
    quick_commands::{QuickCommand, QuickCommandStore},
    runtime::{MirrorFrameBuffer, RuntimeBridge},
    theme, wireless,
};

const RECENT_ENDPOINTS_STORAGE_KEY: &str = "fadb.recent_adb_endpoints";
const AI_SETTINGS_STORAGE_KEY: &str = "fadb.ai_settings";
const QUICK_COMMANDS_STORAGE_KEY: &str = "fadb.shell_quick_commands";
const MAX_RECENT_ENDPOINTS: usize = 8;
const SLOGAN: &str = "a featherweight ADB toolbox, in Rust";
/// How long the pointer must rest on the logo before the slogan shows.
const SLOGAN_HOVER_SECONDS: f32 = 1.5;

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
    Mirror,
}

impl Panel {
    const ALL: [Self; 11] = [
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
        Self::Mirror,
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
            Self::Mirror => "mirror",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
// Dev hooks add more flags to the (already small) set of plain bools.
#[allow(clippy::struct_excessive_bools)]
struct WindowState {
    devices: bool,
    diagnostics: bool,
    ai_settings: bool,
    settings: bool,
}

// Dev hooks add one more flag to the (already small) set of plain bools.
#[allow(clippy::struct_excessive_bools)]
pub struct FadbApp {
    runtime: RuntimeBridge,
    snapshot: DeviceSnapshot,
    overview: Option<DeviceOverview>,
    loading_overview: bool,
    shell: shell::ShellPanelState,
    screenshot: screenshot::ScreenshotPanelState,
    assistant: assistant::AssistantPanelState,
    /// Whether the assistant dock panel is shown on the right of the main
    /// window. In-window on purpose: no second OS window, so no flashing,
    /// Alt+Tab duplication, or window-ownership hacks.
    assistant_open: bool,
    /// Whether a window-resize BeginResize command was already sent for the
    /// current pointer press (see `handle_window_resize`).
    resize_sent: bool,
    /// How long the pointer has hovered the top-bar logo, for the slogan
    /// easter egg.
    /// When the pointer started hovering the top-bar logo, for the slogan
    /// easter egg (`None` while not hovering).
    logo_hover_started: Option<std::time::Instant>,
    ai_form: assistant::AiSettingsForm,
    files: files::FilesPanelState,
    applications: applications::ApplicationsPanelState,
    processes: processes::ProcessesPanelState,
    performance: performance::PerformancePanelState,
    logcat: logcat::LogcatPanelState,
    layout: layout::LayoutPanelState,
    webview: webview::WebviewPanelState,
    /// Live video mirroring for the selected device.
    mirror: mirror::MirrorPanelState,
    /// Shared decoded-frame buffer written by the backend session task.
    mirror_frames: Arc<Mutex<MirrorFrameBuffer>>,
    /// Wireless-debugging section of the device-manager window.
    wireless: wireless::WirelessState,
    last_process_refresh: Option<Instant>,
    last_performance_refresh: Option<Instant>,
    last_application_load: Option<Instant>,
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
    /// Dev-only (`FADB_SELECT=1`): pick the first discovered device so
    /// visual checks run without touching the device combo box.
    auto_select_requested: bool,
    /// The app flag additionally selects the first listed application once
    /// loaded, so screenshots include the details pane.
    auto_select_app: bool,
}

impl FadbApp {
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        theme::configure(&creation_context.egui_ctx);
        platform::apply_window_chrome(creation_context);
        let runtime = RuntimeBridge::spawn(creation_context.egui_ctx.clone());
        let mirror_frames = runtime.mirror_frames();
        let stored_ai = load_ai_settings(creation_context.storage);
        let mut app = Self {
            runtime,
            snapshot: DeviceSnapshot::default(),
            overview: None,
            loading_overview: false,
            shell: shell::ShellPanelState::default(),
            screenshot: screenshot::ScreenshotPanelState::default(),
            assistant: assistant::AssistantPanelState::default(),
            assistant_open: false,
            resize_sent: false,
            logo_hover_started: None,
            ai_form: assistant::AiSettingsForm::from_settings(stored_ai.as_ref()),
            files: files::FilesPanelState::default(),
            applications: applications::ApplicationsPanelState::default(),
            processes: processes::ProcessesPanelState::default(),
            performance: performance::PerformancePanelState::default(),
            logcat: logcat::LogcatPanelState::default(),
            layout: layout::LayoutPanelState::default(),
            webview: webview::WebviewPanelState::default(),
            mirror: mirror::MirrorPanelState::new(),
            mirror_frames,
            wireless: wireless::WirelessState::default(),
            last_process_refresh: None,
            last_performance_refresh: None,
            last_application_load: None,
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
            auto_select_requested: std::env::var_os("FADB_SELECT").is_some(),
            auto_select_app: std::env::var_os("FADB_SELECT").is_some(),
        };
        // Reconnect automatically when complete settings were persisted; the
        // key is required so a half-configured install still shows the setup
        // prompt instead of failing every request with 401.
        app.shell.quick_commands = load_quick_commands(creation_context.storage);
        if let Some(settings) =
            stored_ai.filter(|settings| settings.is_usable() && !settings.api_key.trim().is_empty())
        {
            let _ = app
                .runtime
                .try_send(BackendCommand::ConfigureAi(Some(settings)));
        }
        // Dev convenience for visual checks: start with the assistant dock
        // open (pair with FADB_FAKE=1 for offline runs). Setting the
        // value to "2" additionally seeds a demo transcript so the chat
        // bubbles can be inspected without typing.
        if std::env::var_os("FADB_ASSISTANT").is_some() {
            app.assistant_open = true;
            if std::env::var("FADB_ASSISTANT").as_deref() == Ok("2") {
                app.assistant.seed_demo_transcript();
            }
        }
        // Dev hook for screenshots: open the device-manager window at start.
        if std::env::var_os("FADB_DEVICES").is_some() {
            app.windows.devices = true;
        }
        // Same idea for panel screenshots: `FADB_PANEL=applications`
        // starts on that panel (values are the panel i18n keys).
        if let Some(panel) = std::env::var_os("FADB_PANEL")
            .and_then(|value| value.into_string().ok())
            .and_then(|value| {
                Panel::ALL
                    .iter()
                    .copied()
                    .find(|panel| panel.key() == value)
            })
        {
            app.active_panel = panel;
        }
        // Shell-panel screenshots: `FADB_QUICK_COMMANDS=10` seeds that
        // many synthetic quick commands, so the two-row toolbar cap and the ⋯
        // overflow menu can be inspected without typing them in by hand.
        if let Some(count) = std::env::var_os("FADB_QUICK_COMMANDS")
            .and_then(|value| value.into_string().ok())
            .and_then(|value| value.trim().parse::<usize>().ok())
        {
            for index in 0..count {
                app.shell.quick_commands.commands.push(QuickCommand::new(
                    format!("示例命令 {}", index + 1),
                    format!("echo demo-{}", index + 1),
                    true,
                ));
            }
        }
        app
    }

    // A flat event switch: verbose but exhaustive over BackendEvent, so a
    // new event forces a decision here.
    #[allow(clippy::too_many_lines)]
    fn process_events(&mut self) {
        for event in self.runtime.drain() {
            self.shell.handle_event(&event);
            self.screenshot
                .handle_event(&self.runtime.context(), &event);
            self.assistant.handle_event(&event);
            let application_commands = self.applications.handle_event(&event);
            for command in application_commands {
                self.send(command);
            }
            self.processes.handle_event(&event);
            self.performance.handle_event(&event);
            let file_commands = self.files.handle_event(self.language, &event);
            for command in file_commands {
                self.send(command);
            }
            self.logcat.handle_event(&event);
            self.layout.handle_event(&event);
            self.webview.handle_event(&event);
            self.wireless.handle_event(&event);
            self.mirror.handle_event(&event);
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
                    let auto_select = self.auto_select_requested
                        && self.snapshot.selected.is_none()
                        && snapshot.selected.is_none()
                        && snapshot
                            .devices
                            .first()
                            .is_some_and(|record| record.descriptor.state.is_online());
                    if auto_select
                        && let Some(serial) = snapshot
                            .devices
                            .first()
                            .map(|record| &record.descriptor.serial)
                    {
                        // The next DevicesChanged carries the selection; the
                        // clone below keeps this event's state intact.
                        let serial = serial.clone();
                        self.snapshot = snapshot;
                        self.send(BackendCommand::SelectDevice(Some(serial)));
                        self.auto_select_requested = false;
                        continue;
                    }
                    self.snapshot = snapshot;
                    let target = self.selected_record().map(DeviceRecord::target);
                    self.applications.reset_for(target.clone());
                    self.processes.reset_for(target.clone());
                    self.performance.reset_for(target.clone());
                    self.layout.reset_for(target.clone());
                    self.webview.reset_for(target.clone());
                    for command in self.logcat.reset_for(target) {
                        self.send(command);
                    }
                    self.last_process_refresh = None;
                    self.last_performance_refresh = None;
                    self.last_application_load = None;
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
                | BackendEvent::PerformanceFailed { error, .. }
                | BackendEvent::ApplicationsFailed { error, .. }
                | BackendEvent::ApplicationDetailsFailed { error, .. }
                | BackendEvent::ApplicationActionFailed { error, .. }
                | BackendEvent::ApkInstallFailed { error, .. }
                | BackendEvent::MirrorFailed { error, .. }
                | BackendEvent::MirrorRecordingFailed { error, .. } => {
                    self.last_error = Some(error);
                }
                BackendEvent::OperationFailed(error) => {
                    self.loading_overview = false;
                    self.last_error = Some(error);
                }
                BackendEvent::ApplicationsLoaded(_) => {
                    // Dev hook: mirror a first-row click so panel screenshots
                    // include the details pane without scripted input.
                    if self.auto_select_app
                        && let Some(target) = self.applications.target.clone()
                        && let Some(package) = self
                            .applications
                            .applications
                            .first()
                            .map(|app| app.package.clone())
                    {
                        self.auto_select_app = false;
                        self.applications.selected = Some(package.clone());
                        self.send(BackendCommand::LoadApplicationDetails {
                            request_id: fadb_domain::OperationId::new(),
                            target,
                            package,
                        });
                    }
                }
                BackendEvent::ShellOpened { .. }
                | BackendEvent::ShellOutput { .. }
                | BackendEvent::ShellClosed { .. }
                | BackendEvent::ShellFailed { .. }
                | BackendEvent::ProcessesLoading(_)
                | BackendEvent::ProcessesLoaded(_)
                | BackendEvent::PerformanceLoading(_)
                | BackendEvent::PerformanceLoaded(_)
                | BackendEvent::ApplicationsLoading(_)
                | BackendEvent::ApplicationDetailsLoading { .. }
                | BackendEvent::ApplicationDetailsLoaded { .. }
                | BackendEvent::ApplicationActionStarted { .. }
                | BackendEvent::ApplicationActionCompleted { .. }
                | BackendEvent::ApplicationIconLoaded { .. }
                | BackendEvent::ApkInstallLoading { .. }
                | BackendEvent::ApkInstallFinished { .. }
                | BackendEvent::PairFinished { .. }
                | BackendEvent::PairFailed { .. }
                | BackendEvent::TcpIpEnabled { .. }
                | BackendEvent::TcpIpFailed { .. }
                | BackendEvent::MdnsServicesLoaded { .. }
                | BackendEvent::MdnsFailed { .. }
                | BackendEvent::MirrorStarted { .. }
                | BackendEvent::MirrorStopped { .. }
                | BackendEvent::MirrorRecordingSaved { .. }
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
                | BackendEvent::FileMutationFailed { .. }
                | BackendEvent::LogcatStarted { .. }
                | BackendEvent::LogcatOutput { .. }
                | BackendEvent::LogcatClosed { .. }
                | BackendEvent::LogcatFailed { .. }
                | BackendEvent::LayoutLoading { .. }
                | BackendEvent::LayoutCaptured { .. }
                | BackendEvent::LayoutFailed { .. }
                | BackendEvent::WebviewSocketsLoading { .. }
                | BackendEvent::WebviewSocketsLoaded { .. }
                | BackendEvent::WebviewPagesLoading { .. }
                | BackendEvent::WebviewPagesLoaded { .. }
                | BackendEvent::WebviewFailed { .. } => {}
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

    /// The window controls (— □ ✕) plus the language/theme toggles, anchored
    /// at the right end of the title bar.
    fn top_bar_controls(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Window controls, rightmost first. The close button turns
            // Windows-style red on hover.
            let bar_height = ui.available_height().max(18.0);
            // Glyphs stay out of the Dingbats block (✕/❐ render as tofu with
            // the bundled fonts); × and ▣ exist in both Ubuntu Light and the
            // CJK fallback font.
            if close_button(ui, text(self.language, "win_close"), bar_height).clicked() {
                context.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            let maximized = ui.input(|input| input.viewport().maximized.unwrap_or(false));
            let (restore_label, restore_tip) = if maximized {
                ("▣", text(self.language, "win_restore"))
            } else {
                ("□", text(self.language, "win_maximize"))
            };
            if caption_button(ui, restore_label, restore_tip, bar_height).clicked() {
                context.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            if caption_button(ui, "—", text(self.language, "win_minimize"), bar_height).clicked()
            {
                context.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            ui.add_space(6.0);

            // Settings live in their own window (theme, language, ADB info,
            // about) behind one gear button.
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("⚙").size(14.0))
                        .selected(self.windows.settings),
                )
                .on_hover_text(text(self.language, "settings"))
                .clicked()
            {
                self.windows.settings = !self.windows.settings;
            }
        });
    }

    /// The settings window: grouped rows for appearance, ADB info and about,
    /// opened from the gear button in the top bar.
    fn settings_window(&mut self, context: &egui::Context) {
        let mut open = self.windows.settings;
        egui::Window::new(text(self.language, "settings"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(context, |ui| {
                ui.set_min_width(320.0);

                ui.label(
                    egui::RichText::new(text(self.language, "appearance"))
                        .strong()
                        .small(),
                );
                ui.add_space(4.0);
                egui::Grid::new("settings-appearance")
                    .num_columns(2)
                    .spacing([24.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(text(self.language, "theme"));
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(!self.dark_mode, text(self.language, "light"))
                                .clicked()
                                && self.dark_mode
                            {
                                self.dark_mode = false;
                                context.set_theme(egui::ThemePreference::Light);
                            }
                            if ui
                                .selectable_label(self.dark_mode, text(self.language, "dark"))
                                .clicked()
                                && !self.dark_mode
                            {
                                self.dark_mode = true;
                                context.set_theme(egui::ThemePreference::Dark);
                            }
                        });
                        ui.end_row();

                        ui.label(text(self.language, "language"));
                        ui.horizontal(|ui| {
                            let is_chinese = self.language == Language::Chinese;
                            if ui.selectable_label(is_chinese, "中文").clicked() && !is_chinese {
                                self.language = Language::Chinese;
                            }
                            if ui.selectable_label(!is_chinese, "English").clicked() && is_chinese {
                                self.language = Language::English;
                            }
                        });
                        ui.end_row();
                    });
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                ui.label(egui::RichText::new("ADB").strong().small());
                ui.add_space(4.0);
                egui::Grid::new("settings-adb")
                    .num_columns(2)
                    .spacing([24.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(text(self.language, "diagnostics"));
                        ui.horizontal(|ui| {
                            ui.label(
                                self.adb_path
                                    .as_deref()
                                    .unwrap_or(text(self.language, "unknown")),
                            );
                            // Toggle: click once to expand the ADB details
                            // window, click again to close it.
                            let label = if self.windows.diagnostics {
                                text(self.language, "hide")
                            } else {
                                text(self.language, "details")
                            };
                            if ui.button(label).clicked() {
                                self.windows.diagnostics = !self.windows.diagnostics;
                            }
                        });
                        ui.end_row();
                    });
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                ui.label(
                    egui::RichText::new(text(self.language, "about"))
                        .strong()
                        .small(),
                );
                ui.add_space(4.0);
                egui::Grid::new("settings-about")
                    .num_columns(2)
                    .spacing([24.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(concat!("Fadb v", env!("CARGO_PKG_VERSION")));
                        ui.hyperlink_to("GitHub", "https://github.com/yeqing17/fadb");
                        ui.end_row();
                    });
            });
        self.windows.settings = open;
    }

    /// The top bar doubles as the window's title bar: the window is
    /// undecorated, so dragging it moves the window, double-clicking toggles
    /// maximize, and the window controls (— □ ✕) live at its right end.
    #[allow(clippy::too_many_lines)]
    fn top_bar(&mut self, context: &egui::Context) {
        egui::TopBottomPanel::top("top-bar").show(context, |ui| {
            // Register the drag zone first so the interactive widgets added
            // below it win the hit-test and stay clickable.
            let title_bar = ui.interact(
                ui.max_rect(),
                ui.id().with("title-bar-drag"),
                egui::Sense::drag(),
            );
            // Edge seams win over the title-bar drag: dragging the top edge or
            // the top corners should resize the window, not move it.
            if title_bar.drag_started() && seam_direction(ui.ctx()).is_none() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if title_bar.double_clicked() {
                let maximized = ui.input(|input| input.viewport().maximized.unwrap_or(false));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }

            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(12, 5))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        self.brand_with_slogan(ui);

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
                            .width(280.0)
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
                                    let is_selected =
                                        self.snapshot.selected.as_ref() == Some(&serial);
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
                        // Toggle buttons: a second click closes the window
                        // again, and the highlighted state mirrors it.
                        window_toggle_button(
                            ui,
                            text(self.language, "device_manager"),
                            &mut self.windows.devices,
                        );
                        if ui
                            .add(
                                egui::Button::new(text(self.language, "assistant"))
                                    .selected(self.assistant_open),
                            )
                            .clicked()
                        {
                            self.assistant_open = !self.assistant_open;
                        }

                        ui.add_space(10.0);
                        self.top_bar_controls(ui, context);
                    });
                });
        });
    }

    /// Logo plus app name; hovering the logo for a while reveals the project
    /// slogan as an easter egg.
    fn brand_with_slogan(&mut self, ui: &mut egui::Ui) {
        let logo_response = logo(ui, 22.0);
        // Pure geometry: widget hover can be swallowed by the title-bar drag
        // zone registered over this area, a rect test cannot. Wall-clock
        // timing, because egui's smoothed frame delta under-counts when the
        // app wakes up from an idle state.
        if ui.rect_contains_pointer(logo_response.rect) {
            let hovering_since = self
                .logo_hover_started
                .get_or_insert(std::time::Instant::now());
            let elapsed = hovering_since.elapsed().as_secs_f32();
            if elapsed <= SLOGAN_HOVER_SECONDS {
                // The idle app does not repaint, so the hover state would
                // never be re-evaluated without asking for frames explicitly.
                // Once the slogan is up the polling stops: any pointer
                // movement wakes the loop on its own, so hovering costs
                // nothing beyond the first second and a half.
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            }
            if elapsed > SLOGAN_HOVER_SECONDS {
                egui::containers::Tooltip::always_open(
                    ui.ctx().clone(),
                    ui.layer_id(),
                    ui.id().with("logo-slogan"),
                    logo_response.rect,
                )
                .show(|ui| {
                    ui.label(RichText::new(SLOGAN).weak());
                });
            }
        } else {
            self.logo_hover_started = None;
        }
        // Not selectable: selectable text would swallow the drag and turn
        // window-moves into text selections.
        ui.add(egui::Label::new(RichText::new("Fadb").size(20.0).strong()).selectable(false));
        ui.add_space(28.0);
    }

    fn side_bar(&mut self, context: &egui::Context) {
        egui::SidePanel::left("navigation")
            .resizable(false)
            .default_width(170.0)
            .show(context, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(8, 10))
                    .show(ui, |ui| {
                        for panel in Panel::ALL {
                            let label = RichText::new(text(self.language, panel.key())).size(13.5);
                            let selected = self.active_panel == panel;
                            if ui
                                .add_sized(
                                    [ui.available_width(), 28.0],
                                    egui::Button::new(label).selected(selected),
                                )
                                .clicked()
                            {
                                self.active_panel = panel;
                            }
                        }
                    });
            });
    }

    fn central_panel(&mut self, context: &egui::Context) {
        self.refresh_live_panels(context);
        // The content area is the darkest layer so the surrounding panels and
        // cards read as raised surfaces against it.
        let central_fill = theme::palette(context.theme() == egui::Theme::Dark).central_fill;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(central_fill)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(context, |ui| {
                if let Some(error) = &self.last_error {
                    let palette = theme::palette(ui.visuals().dark_mode);
                    egui::Frame::new()
                        .fill(palette.danger_fill)
                        .stroke(egui::Stroke::new(1.0, palette.danger_stroke))
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::same(10))
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
                    Panel::Shell => shell::show(
                        ui,
                        context,
                        self.language,
                        selected.as_ref(),
                        &mut self.shell,
                    ),
                    Panel::Screenshot => {
                        screenshot::show(ui, self.language, selected.as_ref(), &mut self.screenshot)
                    }
                    Panel::Files => files::show(
                        ui,
                        self.language,
                        &mut self.files,
                        selected.as_ref().map(DeviceRecord::target).as_ref(),
                    ),
                    Panel::Applications => {
                        applications::show(ui, self.language, &mut self.applications)
                    }
                    Panel::Processes => processes::show(ui, self.language, &mut self.processes),
                    Panel::Performance => {
                        performance::show(ui, self.language, &mut self.performance)
                    }
                    Panel::Logcat => logcat::show(ui, self.language, &mut self.logcat),
                    Panel::Layout => layout::show(ui, self.language, &mut self.layout),
                    Panel::WebView => webview::show(ui, self.language, &mut self.webview),
                    Panel::Mirror => mirror::show(
                        ui,
                        self.language,
                        context,
                        &mut self.mirror,
                        &self.mirror_frames,
                        selected.as_ref().map(DeviceRecord::target).as_ref(),
                    ),
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
        self.applications.reset_for(target.clone());
        self.processes.reset_for(target.clone());
        self.performance.reset_for(target.clone());
        self.layout.reset_for(target.clone());
        self.webview.reset_for(target.clone());
        for command in self.logcat.reset_for(target.clone()) {
            self.send(command);
        }
        let now = Instant::now();
        let target_online = target.is_some();
        if self.active_panel == Panel::Applications {
            // The list is a snapshot, not a live feed: fetch it once per
            // visit (only while empty), retrying at most every 5 seconds so
            // a failed load does not spin.
            if let Some(target) = target.clone()
                && !self.applications.loading
                && self.applications.applications.is_empty()
                && self
                    .last_application_load
                    .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(5))
            {
                self.last_application_load = Some(now);
                self.send(BackendCommand::LoadApplications(target));
            }
        }
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
        if self.active_panel == Panel::Logcat {
            // The stream buffers even while hidden; this only (re)opens it.
            for command in logcat::auto_start(&mut self.logcat, target_online) {
                self.send(command);
            }
        }
        if self.active_panel == Panel::Layout
            && let Some(command) = layout::auto_capture(&mut self.layout, target_online)
        {
            self.send(command);
        }
        if self.active_panel == Panel::WebView
            && let Some(command) = webview::auto_refresh(&mut self.webview, target_online)
        {
            self.send(command);
        }
        if self.active_panel == Panel::Mirror {
            let target = self.selected_record().map(DeviceRecord::target);
            for command in mirror::auto(&mut self.mirror, target.as_ref()) {
                self.send(command);
            }
        }
    }

    /// The assistant dock: a resizable panel on the right of the main window.
    ///
    /// Rendered in-window on purpose. A separate OS viewport (tried before)
    /// fights the platform's window management: creation flashes, a duplicate
    /// Alt+Tab/taskbar entry, and ownership hacks to glue the two windows
    /// together. An in-window dock has none of those failure modes and is
    /// truly one surface with the rest of the app.
    fn assistant_dock(&mut self, context: &egui::Context) {
        let language = self.language;
        let assistant = &mut self.assistant;
        let mut commands = Vec::new();
        egui::SidePanel::right("assistant-dock")
            .resizable(true)
            .default_width(340.0)
            .width_range(280.0..=640.0)
            .show(context, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        commands = assistant::show(ui, language, assistant);
                    });
            });
        if self.assistant.close_requested {
            self.assistant_open = false;
            self.assistant.close_requested = false;
        }
        for command in commands {
            self.send(command);
        }
    }

    fn ai_settings_window(&mut self, context: &egui::Context) {
        let mut open = self.windows.ai_settings;
        egui::Window::new(text(self.language, "ai_settings"))
            .open(&mut open)
            .default_width(440.0)
            .show(context, |ui| {
                ui.small(text(self.language, "ai_settings_hint"));
                ui.add_space(8.0);
                egui::Grid::new("ai-settings-grid")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(text(self.language, "ai_endpoint"));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.ai_form.endpoint)
                                .desired_width(280.0)
                                .hint_text("https://api.openai.com/v1"),
                        );
                        ui.end_row();
                        ui.label(text(self.language, "ai_model_name"));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.ai_form.model)
                                .desired_width(280.0)
                                .hint_text("gpt-4o-mini / deepseek-chat / glm-4.6"),
                        );
                        ui.end_row();
                        ui.label(text(self.language, "ai_api_key"));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.ai_form.api_key)
                                .desired_width(280.0)
                                .password(true),
                        );
                        ui.end_row();
                        ui.label(text(self.language, "ai_timeout_seconds"));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.ai_form.timeout)
                                .desired_width(80.0)
                                .hint_text("30"),
                        );
                        ui.end_row();
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let save = egui::Button::new(
                        RichText::new(text(self.language, "ai_save"))
                            .strong()
                            .color(theme::palette(ui.visuals().dark_mode).on_accent),
                    )
                    .fill(theme::ACCENT)
                    .corner_radius(8.0);
                    if ui.add(save).clicked() {
                        match self.ai_form.to_settings() {
                            Some(settings) => {
                                self.send(BackendCommand::ConfigureAi(Some(settings)));
                            }
                            None => {
                                self.last_error =
                                    Some(BridgeError::invalid_input("ai.settings_invalid"));
                            }
                        }
                    }
                    if ui.button(text(self.language, "ai_disable")).clicked() {
                        self.send(BackendCommand::ConfigureAi(None));
                    }
                });
            });
        self.windows.ai_settings = open;
    }

    #[allow(clippy::too_many_lines)]
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
                // Wireless debugging: pairing, tcpip mode, mDNS discovery.
                let selected_serial = self.snapshot.selected.clone();
                let (wireless_commands, wireless_connect) = wireless::show(
                    ui,
                    self.language,
                    &mut self.wireless,
                    selected_serial.as_ref(),
                );
                for command in wireless_commands {
                    self.send(command);
                }
                if let Some(endpoint) = wireless_connect {
                    self.connect_endpoint(endpoint);
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
                        .num_columns(6)
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
                                // Network devices (serial is host:port) get a
                                // disconnect button; USB serials cannot be
                                // disconnected.
                                if let Some(endpoint) =
                                    record.descriptor.serial.as_str().rsplit_once(':').and_then(
                                        |(host, port)| AdbEndpoint::parse_target(host, port),
                                    )
                                    && ui.button(text(self.language, "disconnect")).clicked()
                                {
                                    self.send(BackendCommand::DisconnectDevice(endpoint));
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
                ui.strong(text(self.language, "adb_executable"));
                ui.label(
                    self.adb_path
                        .as_deref()
                        .unwrap_or(text(self.language, "adb_not_available")),
                );
                ui.add_space(8.0);
                ui.strong(text(self.language, "version"));
                ui.label(
                    self.adb_version
                        .as_deref()
                        .unwrap_or(text(self.language, "unknown")),
                );
                ui.add_space(8.0);
                ui.label(format!(
                    "{}: {}",
                    text(self.language, "detected_devices"),
                    self.snapshot.devices.len()
                ));
                ui.small(text(self.language, "adb_env_hint"));
            });
        self.windows.diagnostics = open;
    }
}

impl eframe::App for FadbApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_events();
        if let Some(command) = self
            .files
            .reconcile_target(self.selected_record().map(DeviceRecord::target))
        {
            self.send(command);
        }
        if self.assistant.open_settings {
            self.windows.ai_settings = true;
            self.assistant.open_settings = false;
        }
        handle_window_resize(context, &mut self.resize_sent);
        self.top_bar(context);
        self.side_bar(context);
        if self.assistant_open {
            self.assistant_dock(context);
        }
        self.central_panel(context);
        if self.windows.devices {
            for command in wireless::auto(&mut self.wireless) {
                self.send(command);
            }
            self.device_manager(context);
        }
        if self.windows.ai_settings {
            self.ai_settings_window(context);
        }
        if self.windows.diagnostics {
            self.diagnostics(context);
        }
        if self.windows.settings {
            self.settings_window(context);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(serialized) = serde_json::to_string(&self.recent_endpoints) {
            storage.set_string(RECENT_ENDPOINTS_STORAGE_KEY, serialized);
        }
        // Persist the form as-is when it describes a usable provider; store an
        // empty document otherwise so a cleared form is not resurrected.
        let ai = self.ai_form.to_settings().unwrap_or_default();
        if let Ok(serialized) = serde_json::to_string(&ai) {
            storage.set_string(AI_SETTINGS_STORAGE_KEY, serialized);
        }
        // Quick commands save as-is (an emptied list stays empty); only
        // sendable entries are worth round-tripping.
        let sendable: Vec<_> = self
            .shell
            .quick_commands
            .commands
            .iter()
            .filter(|command| command.is_sendable())
            .cloned()
            .collect();
        if let Ok(serialized) = serde_json::to_string(&sendable) {
            storage.set_string(QUICK_COMMANDS_STORAGE_KEY, serialized);
        }
    }
}

fn load_quick_commands(storage: Option<&dyn eframe::Storage>) -> QuickCommandStore {
    let Some(serialized) =
        storage.and_then(|storage| storage.get_string(QUICK_COMMANDS_STORAGE_KEY))
    else {
        return QuickCommandStore::default();
    };
    // A stored (possibly emptied) list always wins over the seed defaults;
    // only unreadable data falls back to them.
    match serde_json::from_str::<Vec<QuickCommand>>(&serialized) {
        Ok(commands) => QuickCommandStore {
            commands,
            ..QuickCommandStore::default()
        },
        Err(error) => {
            tracing::warn!(%error, "stored quick commands were unreadable; using defaults");
            QuickCommandStore::default()
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

fn load_ai_settings(storage: Option<&dyn eframe::Storage>) -> Option<AiSettings> {
    let serialized = storage.and_then(|storage| storage.get_string(AI_SETTINGS_STORAGE_KEY))?;
    let settings = serde_json::from_str::<AiSettings>(&serialized).ok()?;
    settings.is_usable().then_some(settings)
}

/// Undecorated windows lose the OS resize frame; re-create it with invisible
/// drag zones along the window edges (corners included).
/// Begin the OS resize drag at most once per pointer press: when the native
/// resize loop exits, winit posts a synthetic button-up that egui may see one
/// frame late, and an unconditional re-send would immediately re-enter the
/// resize loop with the button already released (the "stuck resizing" bug).
fn handle_window_resize(context: &egui::Context, resize_sent: &mut bool) {
    let Some(direction) = seam_direction(context) else {
        return;
    };
    context.set_cursor_icon(seam_cursor(direction));
    let (dragging, primary_down) = context.input(|input| {
        (
            input.pointer.is_decidedly_dragging(),
            input.pointer.primary_down(),
        )
    });
    // Latch only while a gesture is actively dragging; the moment it is not
    // (button up, or pressed but not yet moved) the latch clears, so a
    // swallowed send can never poison later gestures.
    if !primary_down || !dragging {
        *resize_sent = false;
        return;
    }
    if !*resize_sent {
        *resize_sent = true;
        context.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
    }
}

/// Which resize seam the pointer currently sits in, if any.
fn seam_direction(context: &egui::Context) -> Option<egui::ResizeDirection> {
    if context.input(|input| input.viewport().maximized.unwrap_or(false)) {
        return None;
    }
    context
        .pointer_hover_pos()
        .and_then(|pos| resize_direction(pos, context.content_rect()))
}

/// One of the three window-control buttons in the title bar.
fn caption_button(
    ui: &mut egui::Ui,
    label: &str,
    tooltip: &str,
    bar_height: f32,
) -> egui::Response {
    ui.add_sized(
        [bar_height * 1.6, bar_height],
        egui::Button::new(RichText::new(label).size(13.0)),
    )
    .on_hover_text(tooltip)
}

/// A top-bar button that toggles a floating window: clicking again closes
/// it, and the highlighted state mirrors whether the window is open.
fn window_toggle_button(ui: &mut egui::Ui, label: &str, open: &mut bool) {
    if ui.add(egui::Button::new(label).selected(*open)).clicked() {
        *open = !*open;
    }
}

/// The title-bar close button: neutral when idle, Windows-style red on hover.
fn close_button(ui: &mut egui::Ui, tooltip: &str, bar_height: f32) -> egui::Response {
    ui.scope(|ui| {
        let widgets = &mut ui.style_mut().visuals.widgets;
        widgets.hovered.weak_bg_fill = Color32::from_rgb(196, 44, 52);
        widgets.hovered.bg_stroke = egui::Stroke::NONE;
        widgets.hovered.fg_stroke = egui::Stroke::new(1.0, Color32::WHITE);
        widgets.active.weak_bg_fill = Color32::from_rgb(224, 54, 62);
        widgets.active.bg_stroke = egui::Stroke::NONE;
        ui.add_sized(
            [bar_height * 1.6, bar_height],
            egui::Button::new(RichText::new("×").size(13.0)),
        )
        .on_hover_text(tooltip)
    })
    .inner
}

/// The Fadb logo tile from `assets/` (same mark as the taskbar and exe
/// icon). Decoded once and cached as a texture on the context, so both the
/// title bar and the assistant dock share it.
pub(crate) fn logo(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let texture = logo_texture(ui.ctx());
    ui.add(
        egui::Image::from_texture(&texture)
            .max_size(egui::vec2(size, size))
            .corner_radius(size * 0.22),
    )
}

fn logo_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let logo_id = egui::Id::new("fadb-logo");
    if let Some(handle) = ctx.data_mut(|cache| cache.get_temp::<egui::TextureHandle>(logo_id)) {
        return handle;
    }
    let img = image::load_from_memory(include_bytes!("../assets/icon-64.png"))
        .expect("embedded logo is a valid PNG")
        .to_rgba8();
    let [width, height] = [img.width() as usize, img.height() as usize];
    let handle = ctx.load_texture(
        "fadb-logo",
        egui::ColorImage::from_rgba_unmultiplied([width, height], img.as_raw()),
        egui::TextureOptions::default(),
    );
    ctx.data_mut(|cache| cache.insert_temp(logo_id, handle.clone()));
    handle
}

/// Invisible thickness of the window's resize seams, in points.
const RESIZE_SEAM_THICKNESS: f32 = 8.0;

/// Which resize direction a pointer position corresponds to, when it sits in
/// an edge or corner seam of the given screen rect.
fn resize_direction(pos: egui::Pos2, screen: egui::Rect) -> Option<egui::ResizeDirection> {
    if !screen.contains(pos) {
        return None;
    }
    let near_left = pos.x - screen.left() < RESIZE_SEAM_THICKNESS;
    let near_right = screen.right() - pos.x < RESIZE_SEAM_THICKNESS;
    let near_top = pos.y - screen.top() < RESIZE_SEAM_THICKNESS;
    let near_bottom = screen.bottom() - pos.y < RESIZE_SEAM_THICKNESS;
    match (near_left, near_right, near_top, near_bottom) {
        (true, _, true, _) => Some(egui::ResizeDirection::NorthWest),
        (true, _, _, true) => Some(egui::ResizeDirection::SouthWest),
        (_, true, true, _) => Some(egui::ResizeDirection::NorthEast),
        (_, true, _, true) => Some(egui::ResizeDirection::SouthEast),
        (true, _, _, _) => Some(egui::ResizeDirection::West),
        (_, true, _, _) => Some(egui::ResizeDirection::East),
        (_, _, true, _) => Some(egui::ResizeDirection::North),
        (_, _, _, true) => Some(egui::ResizeDirection::South),
        _ => None,
    }
}

fn seam_cursor(direction: egui::ResizeDirection) -> egui::CursorIcon {
    match direction {
        egui::ResizeDirection::North => egui::CursorIcon::ResizeNorth,
        egui::ResizeDirection::South => egui::CursorIcon::ResizeSouth,
        egui::ResizeDirection::East => egui::CursorIcon::ResizeEast,
        egui::ResizeDirection::West => egui::CursorIcon::ResizeWest,
        egui::ResizeDirection::NorthEast => egui::CursorIcon::ResizeNorthEast,
        egui::ResizeDirection::NorthWest => egui::CursorIcon::ResizeNorthWest,
        egui::ResizeDirection::SouthEast => egui::CursorIcon::ResizeSouthEast,
        egui::ResizeDirection::SouthWest => egui::CursorIcon::ResizeSouthWest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_expected_panels_are_present() {
        assert_eq!(Panel::ALL.len(), 11);
        assert_eq!(Panel::ALL[0], Panel::Overview);
        assert_eq!(Panel::ALL[9], Panel::WebView);
        assert_eq!(Panel::ALL[10], Panel::Mirror);
    }

    #[test]
    fn initial_device_snapshot_has_no_implicit_selection() {
        assert_eq!(
            DeviceSnapshot::default().selected,
            None::<fadb_domain::DeviceSerial>
        );
    }

    #[test]
    fn resize_direction_covers_edges_and_corners() {
        use egui::ResizeDirection as Rd;
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 800.0));
        assert_eq!(
            resize_direction(egui::pos2(500.0, 400.0), screen),
            None,
            "center is not a resize seam"
        );
        assert_eq!(
            resize_direction(egui::pos2(2.0, 400.0), screen),
            Some(Rd::West)
        );
        assert_eq!(
            resize_direction(egui::pos2(998.0, 400.0), screen),
            Some(Rd::East)
        );
        assert_eq!(
            resize_direction(egui::pos2(500.0, 2.0), screen),
            Some(Rd::North)
        );
        assert_eq!(
            resize_direction(egui::pos2(500.0, 798.0), screen),
            Some(Rd::South)
        );
        assert_eq!(
            resize_direction(egui::pos2(3.0, 3.0), screen),
            Some(Rd::NorthWest)
        );
        assert_eq!(
            resize_direction(egui::pos2(997.0, 3.0), screen),
            Some(Rd::NorthEast)
        );
        assert_eq!(
            resize_direction(egui::pos2(3.0, 797.0), screen),
            Some(Rd::SouthWest)
        );
        assert_eq!(
            resize_direction(egui::pos2(997.0, 797.0), screen),
            Some(Rd::SouthEast)
        );
        assert_eq!(
            resize_direction(egui::pos2(-5.0, 400.0), screen),
            None,
            "outside the window is not a seam"
        );
    }
}
