use std::{
    collections::HashMap,
    io::Cursor,
    path::PathBuf,
    sync::{Arc, RwLock},
    thread,
    time::Duration,
};

use bridgescope_adb::{AdbLocator, AdbTransport, ProcessAdbTransport, ShellSessionHandle};
use bridgescope_device::DeviceRegistry;
use bridgescope_domain::{
    ApplicationAction, ApplicationSnapshot, BackendCommand, BackendEvent, BridgeError,
    DeviceTarget, ErrorCode, FileTransferDirection, FileTransferSummary, LogcatSessionId,
    OperationId, OverwritePolicy, PackageName, PerformanceSnapshot, ProcessSnapshot,
    RawScreenshotPng, RemoteFileMutationKind, RemoteFileMutationSummary, RemotePath,
    ScreenshotData, ScreenshotFormat, ScreenshotImage, ShellSessionId, ShellSize, WebViewPage,
};
use bridgescope_test_support::FakeAdbTransport;
use eframe::egui;
use futures_util::StreamExt;
use tokio::{runtime::Builder, sync::mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

const CHANNEL_CAPACITY: usize = 64;
const MAX_SHELL_OUTPUT_BATCH_CHUNKS: usize = 8;
const SHELL_OUTPUT_COALESCE_WINDOW: Duration = Duration::from_millis(8);
const MAX_SCREENSHOT_DIMENSION: u32 = 8192;
const MAX_SCREENSHOT_PIXELS: u64 = 16_777_216;
const DEVTOOLS_HTTP_TIMEOUT: Duration = Duration::from_secs(4);

struct ActiveShell {
    handle: ShellSessionHandle,
}

struct ActiveTransfer {
    target: DeviceTarget,
    cancellation: CancellationToken,
}

#[derive(Clone)]
struct ShellEventContext {
    events: mpsc::Sender<BackendEvent>,
    context: egui::Context,
    inputs: Arc<RwLock<HashMap<ShellSessionId, mpsc::Sender<Vec<u8>>>>>,
}

enum FileTaskResult {
    Transfer {
        request_id: OperationId,
        target: DeviceTarget,
        result: Result<FileTransferSummary, BridgeError>,
    },
    Mutation {
        request_id: OperationId,
        target: DeviceTarget,
        result: Result<RemoteFileMutationSummary, BridgeError>,
    },
}

pub struct RuntimeBridge {
    command_tx: mpsc::Sender<BackendCommand>,
    shell_inputs: Arc<RwLock<HashMap<ShellSessionId, mpsc::Sender<Vec<u8>>>>>,
    event_rx: mpsc::Receiver<BackendEvent>,
    context: egui::Context,
}

impl RuntimeBridge {
    pub fn spawn(context: egui::Context) -> Self {
        let (command_tx, command_rx) = mpsc::channel(CHANNEL_CAPACITY);
        let (event_tx, event_rx) = mpsc::channel(CHANNEL_CAPACITY);
        let shell_inputs = Arc::new(RwLock::new(HashMap::new()));

        let backend_context = context.clone();
        let backend_shell_inputs = Arc::clone(&shell_inputs);
        thread::Builder::new()
            .name("bridgescope-backend".to_owned())
            .spawn(move || {
                let runtime = Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("Tokio runtime must initialize");
                runtime.block_on(run_backend(
                    command_rx,
                    event_tx,
                    backend_context,
                    backend_shell_inputs,
                ));
            })
            .expect("backend thread must start");

        Self {
            command_tx,
            shell_inputs,
            event_rx,
            context,
        }
    }

    pub fn try_send(&self, command: BackendCommand) -> Result<(), BridgeError> {
        if let BackendCommand::WriteShell { session_id, input } = command {
            let inputs = self.shell_inputs.read().map_err(|error| {
                BridgeError::new(
                    ErrorCode::Internal,
                    "runtime.shell_registry_unavailable",
                    error.to_string(),
                )
            })?;
            let input_tx = inputs.get(&session_id).ok_or_else(|| {
                BridgeError::new(
                    ErrorCode::InvalidInput,
                    "shell.session_not_found",
                    session_id.to_string(),
                )
            })?;
            return input_tx.try_send(input.into_bytes()).map_err(|error| {
                BridgeError::new(
                    ErrorCode::OutputLimit,
                    "shell.input_queue_full",
                    error.to_string(),
                )
            });
        }

        self.command_tx.try_send(command).map_err(|error| {
            BridgeError::new(
                ErrorCode::Internal,
                "runtime.command_queue_full",
                error.to_string(),
            )
        })
    }

    pub fn context(&self) -> egui::Context {
        self.context.clone()
    }

    pub fn drain(&mut self) -> Vec<BackendEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }
}

#[allow(clippy::too_many_lines)]
async fn run_backend(
    mut commands: mpsc::Receiver<BackendCommand>,
    events: mpsc::Sender<BackendEvent>,
    context: egui::Context,
    shell_inputs: Arc<RwLock<HashMap<ShellSessionId, mpsc::Sender<Vec<u8>>>>>,
) {
    let transport = initialize_transport(&events, &context).await;
    let mut ai_provider = initialize_ai(&events, &context).await;
    let mut registry = DeviceRegistry::default();
    let mut shells = HashMap::<ShellSessionId, ActiveShell>::new();
    let mut logcats = HashMap::<LogcatSessionId, ShellSessionHandle>::new();
    let webview_forwards = Arc::new(std::sync::Mutex::new(Vec::<u16>::new()));
    let mut transfers = HashMap::<OperationId, ActiveTransfer>::new();
    let shell_event_context = ShellEventContext {
        events: events.clone(),
        context: context.clone(),
        inputs: shell_inputs,
    };
    let (file_result_tx, mut file_result_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let _ = refresh_devices(transport.as_ref(), &mut registry, &events, &context).await;
    cancel_stale_transfers(&registry, &transfers);

    loop {
        tokio::select! {
            result = file_result_rx.recv() => {
                if let Some(result) = result {
                    finish_file_task(result, &mut transfers, &events, &context).await;
                }
            }
            command = commands.recv() => {
                let Some(command) = command else { break };
                match command {
                    BackendCommand::RefreshDevices => {
                        let _ =
                            refresh_devices(transport.as_ref(), &mut registry, &events, &context)
                                .await;
                        cancel_stale_transfers(&registry, &transfers);
                    }
                    BackendCommand::ConnectDevice(endpoint) => {
                        send_event(
                            &events,
                            &context,
                            BackendEvent::AdbConnecting(endpoint.clone()),
                        )
                        .await;
                        match transport.connect_endpoint(&endpoint).await {
                            Ok(_) => {
                                if refresh_devices(
                                    transport.as_ref(),
                                    &mut registry,
                                    &events,
                                    &context,
                                )
                                .await
                                .is_ok()
                                {
                                    cancel_stale_transfers(&registry, &transfers);
                                    send_event(
                                        &events,
                                        &context,
                                        BackendEvent::AdbConnected(endpoint),
                                    )
                                    .await;
                                } else {
                                    send_event(
                                        &events,
                                        &context,
                                        BackendEvent::AdbConnectFailed {
                                            endpoint,
                                            error: BridgeError::new(
                                                ErrorCode::AdbFailed,
                                                "adb.devices.refresh_failed",
                                                "ADB connected but device discovery failed",
                                            ),
                                        },
                                    )
                                    .await;
                                }
                            }
                            Err(error) => {
                                send_event(
                                    &events,
                                    &context,
                                    BackendEvent::AdbConnectFailed { endpoint, error },
                                )
                                .await;
                            }
                        }
                    }
                    BackendCommand::SelectDevice(serial) => {
                        match registry.select(serial.clone()) {
                            Ok(snapshot) => send_event(&events, &context, BackendEvent::DevicesChanged(snapshot)).await,
                            Err(error) => send_event(&events, &context, BackendEvent::OperationFailed(error)).await,
                        }
                        if let Some(serial) = serial {
                            load_overview(transport.as_ref(), &registry, serial, &events, &context).await;
                        }
                    }
                    BackendCommand::LoadOverview(serial) => {
                        load_overview(transport.as_ref(), &registry, serial, &events, &context).await;
                    }
                    BackendCommand::LoadProcesses(target) => {
                        load_processes(transport.as_ref(), &registry, target, &events, &context)
                            .await;
                    }
                    BackendCommand::LoadPerformance(target) => {
                        load_performance(transport.as_ref(), &registry, target, &events, &context)
                            .await;
                    }
                    BackendCommand::LoadApplications(target) => {
                        load_applications(transport.as_ref(), &registry, target, &events, &context)
                            .await;
                    }
                    BackendCommand::LoadApplicationDetails { request_id, target, package } => {
                        load_application_details(
                            transport.as_ref(),
                            &registry,
                            request_id,
                            target,
                            package,
                            &events,
                            &context,
                        )
                        .await;
                    }
                    BackendCommand::LoadApplicationIcons { target, packages } => {
                        load_application_icons(
                            transport.as_ref(),
                            &registry,
                            target,
                            packages,
                            &events,
                            &context,
                        )
                        .await;
                    }
                    BackendCommand::RunApplicationAction { request_id, action, target, package } => {
                        run_application_action(
                            transport.as_ref(),
                            &registry,
                            request_id,
                            action,
                            target,
                            package,
                            &events,
                            &context,
                        )
                        .await;
                    }
                    BackendCommand::OpenShell { target, session_id, size } => {
                        open_shell(
                            transport.clone(),
                            &registry,
                            &mut shells,
                            target,
                            session_id,
                            size,
                            shell_event_context.clone(),
                        ).await;
                    }
                    BackendCommand::WriteShell { session_id, .. } => {
                        send_event(&events, &context, BackendEvent::ShellFailed {
                            session_id,
                            error: BridgeError::new(
                                ErrorCode::Internal,
                                "shell.input_routing_failed",
                                "shell input reached the control queue",
                            ),
                        }).await;
                    }
                    BackendCommand::ResizeShell { session_id: _, size: _ } => {
                        // Native shell-v2 resize is a later transport milestone.
                    }
                    BackendCommand::CloseShell(session_id) => {
                        remove_shell_input(&shell_event_context.inputs, session_id);
                        if let Some(shell) = shells.remove(&session_id) {
                            tokio::spawn(async move { let _result = shell.handle.close().await; });
                        }
                    }
                    BackendCommand::CaptureScreenshot { request_id, target, format } => {
                        capture_screenshot(
                            transport.clone(),
                            &registry,
                            request_id,
                            target,
                            format,
                            events.clone(),
                            context.clone(),
                        ).await;
                    }
                    BackendCommand::SendAiChat { request_id, prompt } => {
                        run_ai_chat(
                            ai_provider.clone(),
                            request_id,
                            prompt,
                            events.clone(),
                            context.clone(),
                        ).await;
                    }
                    BackendCommand::ConfigureAi(settings) => {
                        configure_ai(&mut ai_provider, settings, &events, &context).await;
                    }
                    BackendCommand::ListDirectory { request_id, target, path } => {
                        list_directory(transport.as_ref(), &registry, request_id, target, path, &events, &context).await;
                    }
                    BackendCommand::UploadFile { request_id, target, local_path, remote_path, overwrite } => {
                        start_transfer(transport.clone(), &registry, &mut transfers, file_result_tx.clone(), request_id, target, local_path, remote_path, overwrite, true, &events, &context).await;
                    }
                    BackendCommand::DownloadFile { request_id, target, remote_path, local_path, overwrite } => {
                        start_transfer(transport.clone(), &registry, &mut transfers, file_result_tx.clone(), request_id, target, local_path, remote_path, overwrite, false, &events, &context).await;
                    }
                    BackendCommand::CancelFileOperation(request_id) => {
                        if let Some(active) = transfers.get(&request_id) {
                            active.cancellation.cancel();
                        }
                    }
                    BackendCommand::CreateDirectory { request_id, target, path } => {
                        start_mutation(transport.clone(), &registry, file_result_tx.clone(), request_id, target, RemoteFileMutationKind::CreateDirectory, path, None, true).await;
                    }
                    BackendCommand::RenameRemoteEntry { request_id, target, source, destination } => {
                        start_mutation(transport.clone(), &registry, file_result_tx.clone(), request_id, target, RemoteFileMutationKind::Rename, source, Some(destination), true).await;
                    }
                    BackendCommand::DeleteRemoteFile { request_id, target, path, confirmed } => {
                        start_mutation(transport.clone(), &registry, file_result_tx.clone(), request_id, target, RemoteFileMutationKind::DeleteFile, path, None, confirmed).await;
                    }
                    BackendCommand::StartLogcat { target, session_id } => {
                        start_logcat_session(
                            transport.clone(),
                            &registry,
                            &mut logcats,
                            target,
                            session_id,
                            shell_event_context.clone(),
                        )
                        .await;
                    }
                    BackendCommand::StopLogcat(session_id) => {
                        // Dropping the handle kills the adb child; the session's
                        // forwarder task observes the closed channel and emits
                        // `LogcatClosed` for the UI.
                        logcats.remove(&session_id);
                    }
                    BackendCommand::CaptureLayout { request_id, target } => {
                        capture_layout(
                            transport.clone(),
                            &registry,
                            request_id,
                            target,
                            events.clone(),
                            context.clone(),
                        )
                        .await;
                    }
                    BackendCommand::ListWebviewSockets { request_id, target } => {
                        list_webview_sockets(
                            transport.clone(),
                            &registry,
                            Arc::clone(&webview_forwards),
                            request_id,
                            target,
                            events.clone(),
                            context.clone(),
                        )
                        .await;
                    }
                    BackendCommand::ListWebviewPages { request_id, target, socket, port } => {
                        list_webview_pages(
                            transport.clone(),
                            &registry,
                            Arc::clone(&webview_forwards),
                            request_id,
                            target,
                            socket,
                            port,
                            events.clone(),
                            context.clone(),
                        )
                        .await;
                    }
                }
            }
        }
    }
}

async fn open_shell(
    transport: Arc<dyn AdbTransport>,
    registry: &DeviceRegistry,
    shells: &mut HashMap<ShellSessionId, ActiveShell>,
    target: DeviceTarget,
    session_id: ShellSessionId,
    size: ShellSize,
    shell_event_context: ShellEventContext,
) {
    if registry.current_online(&target).is_none() {
        send_event(
            &shell_event_context.events,
            &shell_event_context.context,
            BackendEvent::ShellFailed {
                session_id,
                error: BridgeError::new(
                    ErrorCode::DeviceUnavailable,
                    "device.target_stale",
                    target.serial.redacted(),
                ),
            },
        )
        .await;
        return;
    }
    match transport.start_shell(&target.serial, size).await {
        Ok(mut handle) => {
            let input = handle.input();
            insert_shell_input(&shell_event_context.inputs, session_id, input);
            let mut output = std::mem::replace(handle.output_mut(), mpsc::channel(1).1);
            let output_events = shell_event_context.events.clone();
            let output_context = shell_event_context.context.clone();
            let output_shell_inputs = Arc::clone(&shell_event_context.inputs);
            tokio::spawn(async move {
                while let Some(chunk) = output.recv().await {
                    let bytes = collect_shell_output_batch(chunk.bytes, &mut output).await;
                    send_event(
                        &output_events,
                        &output_context,
                        BackendEvent::ShellOutput { session_id, bytes },
                    )
                    .await;
                }
                remove_shell_input(&output_shell_inputs, session_id);
                send_event(
                    &output_events,
                    &output_context,
                    BackendEvent::ShellClosed {
                        session_id,
                        exit_code: None,
                    },
                )
                .await;
            });
            shells.insert(session_id, ActiveShell { handle });
            send_event(
                &shell_event_context.events,
                &shell_event_context.context,
                BackendEvent::ShellOpened { target, session_id },
            )
            .await;
        }
        Err(error) => {
            send_event(
                &shell_event_context.events,
                &shell_event_context.context,
                BackendEvent::ShellFailed { session_id, error },
            )
            .await;
        }
    }
}

fn insert_shell_input(
    shell_inputs: &RwLock<HashMap<ShellSessionId, mpsc::Sender<Vec<u8>>>>,
    session_id: ShellSessionId,
    input: mpsc::Sender<Vec<u8>>,
) {
    if let Ok(mut inputs) = shell_inputs.write() {
        inputs.insert(session_id, input);
    }
}

fn remove_shell_input(
    shell_inputs: &RwLock<HashMap<ShellSessionId, mpsc::Sender<Vec<u8>>>>,
    session_id: ShellSessionId,
) {
    if let Ok(mut inputs) = shell_inputs.write() {
        inputs.remove(&session_id);
    }
}

async fn collect_shell_output_batch(
    first: Vec<u8>,
    output: &mut mpsc::Receiver<bridgescope_adb::ShellOutputChunk>,
) -> Vec<u8> {
    let mut bytes = first;
    let deadline = tokio::time::Instant::now() + SHELL_OUTPUT_COALESCE_WINDOW;
    for _ in 1..MAX_SHELL_OUTPUT_BATCH_CHUNKS {
        match tokio::time::timeout_at(deadline, output.recv()).await {
            Ok(Some(chunk)) => bytes.extend_from_slice(&chunk.bytes),
            Ok(None) | Err(_) => break,
        }
    }
    bytes
}

async fn start_logcat_session(
    transport: Arc<dyn AdbTransport>,
    registry: &DeviceRegistry,
    logcats: &mut HashMap<LogcatSessionId, ShellSessionHandle>,
    target: DeviceTarget,
    session_id: LogcatSessionId,
    shell_event_context: ShellEventContext,
) {
    if registry.current_online(&target).is_none() {
        let error = stale_target_error(&target);
        send_event(
            &shell_event_context.events,
            &shell_event_context.context,
            BackendEvent::LogcatFailed { session_id, error },
        )
        .await;
        return;
    }
    match transport.start_logcat(&target.serial).await {
        Ok(mut handle) => {
            let mut output = std::mem::replace(handle.output_mut(), mpsc::channel(1).1);
            let events = shell_event_context.events.clone();
            let context = shell_event_context.context.clone();
            tokio::spawn(async move {
                while let Some(chunk) = output.recv().await {
                    let bytes = collect_shell_output_batch(chunk.bytes, &mut output).await;
                    send_event(
                        &events,
                        &context,
                        BackendEvent::LogcatOutput { session_id, bytes },
                    )
                    .await;
                }
                send_event(&events, &context, BackendEvent::LogcatClosed { session_id }).await;
            });
            logcats.insert(session_id, handle);
            send_event(
                &shell_event_context.events,
                &shell_event_context.context,
                BackendEvent::LogcatStarted { target, session_id },
            )
            .await;
        }
        Err(error) => {
            send_event(
                &shell_event_context.events,
                &shell_event_context.context,
                BackendEvent::LogcatFailed { session_id, error },
            )
            .await;
        }
    }
}

async fn capture_layout(
    transport: Arc<dyn AdbTransport>,
    registry: &DeviceRegistry,
    request_id: OperationId,
    target: DeviceTarget,
    events: mpsc::Sender<BackendEvent>,
    context: egui::Context,
) {
    let Some(record) = registry.current_online(&target) else {
        let error = stale_target_error(&target);
        send_event(
            &events,
            &context,
            BackendEvent::LayoutFailed {
                request_id,
                target,
                error,
            },
        )
        .await;
        return;
    };
    let current = record.target();
    send_event(
        &events,
        &context,
        BackendEvent::LayoutLoading {
            request_id,
            target: current.clone(),
        },
    )
    .await;
    // Spawned: a retried `uiautomator dump` can take tens of seconds on busy
    // screens and must not stall the control loop.
    tokio::spawn(async move {
        let event = match transport.dump_layout(&current.serial).await {
            Ok(mut snapshot) => {
                snapshot.target = current.clone();
                BackendEvent::LayoutCaptured {
                    request_id,
                    snapshot,
                }
            }
            Err(error) => BackendEvent::LayoutFailed {
                request_id,
                target: current,
                error,
            },
        };
        send_event(&events, &context, event).await;
    });
}

async fn list_webview_sockets(
    transport: Arc<dyn AdbTransport>,
    registry: &DeviceRegistry,
    forwards: Arc<std::sync::Mutex<Vec<u16>>>,
    request_id: OperationId,
    target: DeviceTarget,
    events: mpsc::Sender<BackendEvent>,
    context: egui::Context,
) {
    if registry.current_online(&target).is_none() {
        let error = stale_target_error(&target);
        send_event(
            &events,
            &context,
            BackendEvent::WebviewFailed {
                request_id,
                target,
                error,
            },
        )
        .await;
        return;
    }
    send_event(
        &events,
        &context,
        BackendEvent::WebviewSocketsLoading {
            request_id,
            target: target.clone(),
        },
    )
    .await;
    let stale = forwards
        .lock()
        .map(|mut ports| std::mem::take(&mut *ports))
        .unwrap_or_default();
    for port in stale {
        let cleanup_transport = Arc::clone(&transport);
        let cleanup_target = target.clone();
        tokio::spawn(async move {
            let _ = cleanup_transport
                .remove_forward(&cleanup_target.serial, port)
                .await;
        });
    }
    match transport.list_webview_sockets(&target.serial).await {
        Ok(sockets) => {
            send_event(
                &events,
                &context,
                BackendEvent::WebviewSocketsLoaded {
                    request_id,
                    target,
                    sockets,
                },
            )
            .await;
        }
        Err(error) => {
            send_event(
                &events,
                &context,
                BackendEvent::WebviewFailed {
                    request_id,
                    target,
                    error,
                },
            )
            .await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn list_webview_pages(
    transport: Arc<dyn AdbTransport>,
    registry: &DeviceRegistry,
    forwards: Arc<std::sync::Mutex<Vec<u16>>>,
    request_id: OperationId,
    target: DeviceTarget,
    socket: String,
    port: u16,
    events: mpsc::Sender<BackendEvent>,
    context: egui::Context,
) {
    if registry.current_online(&target).is_none() {
        let error = stale_target_error(&target);
        send_event(
            &events,
            &context,
            BackendEvent::WebviewFailed {
                request_id,
                target,
                error,
            },
        )
        .await;
        return;
    }
    send_event(
        &events,
        &context,
        BackendEvent::WebviewPagesLoading {
            request_id,
            target: target.clone(),
            socket: socket.clone(),
        },
    )
    .await;
    tokio::spawn(async move {
        if let Err(error) = transport.forward_port(&target.serial, port, &socket).await {
            send_event(
                &events,
                &context,
                BackendEvent::WebviewFailed {
                    request_id,
                    target,
                    error,
                },
            )
            .await;
            return;
        }
        match fetch_devtools_pages(port).await {
            Ok(pages) => {
                if let Ok(mut tracked) = forwards.lock()
                    && !tracked.contains(&port)
                {
                    tracked.push(port);
                }
                send_event(
                    &events,
                    &context,
                    BackendEvent::WebviewPagesLoaded {
                        request_id,
                        target,
                        socket,
                        port,
                        pages,
                    },
                )
                .await;
            }
            Err(error) => {
                // The forward is useless without a working DevTools endpoint;
                // drop it so the next attempt starts clean.
                let _ = transport.remove_forward(&target.serial, port).await;
                send_event(
                    &events,
                    &context,
                    BackendEvent::WebviewFailed {
                        request_id,
                        target,
                        error,
                    },
                )
                .await;
            }
        }
    });
}

/// Fetches the DevTools HTTP page list through the forwarded local port.
/// The forward stays installed so the returned debugger WebSocket URLs keep
/// working for the UI's "open DevTools" action.
async fn fetch_devtools_pages(port: u16) -> Result<Vec<WebViewPage>, BridgeError> {
    #[derive(serde::Deserialize)]
    struct DevtoolsPage {
        #[serde(default)]
        title: String,
        #[serde(default)]
        url: String,
        #[serde(default, rename = "type")]
        kind: String,
        #[serde(default, rename = "webSocketDebuggerUrl")]
        debugger_url: String,
    }

    let url = format!("http://127.0.0.1:{port}/json/list");
    let client = reqwest::Client::builder()
        .timeout(DEVTOOLS_HTTP_TIMEOUT)
        .build()
        .map_err(|error| {
            BridgeError::new(
                ErrorCode::Internal,
                "webview.pages_unreachable",
                error.to_string(),
            )
        })?;
    let response = client
        .get(&url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| {
            BridgeError::new(
                ErrorCode::AdbFailed,
                "webview.pages_unreachable",
                error.to_string(),
            )
        })?;
    let payload: Vec<DevtoolsPage> =
        response
            .json::<Vec<DevtoolsPage>>()
            .await
            .map_err(|error| {
                BridgeError::new(
                    ErrorCode::Internal,
                    "webview.pages_unreachable",
                    error.to_string(),
                )
            })?;
    Ok(payload
        .into_iter()
        .map(|page| WebViewPage {
            title: page.title,
            url: page.url,
            kind: page.kind,
            debugger_url: page.debugger_url,
        })
        .collect())
}

fn stale_target_error(target: &DeviceTarget) -> BridgeError {
    BridgeError::new(
        ErrorCode::DeviceUnavailable,
        "device.target_stale",
        target.serial.redacted(),
    )
}

async fn capture_screenshot(
    transport: Arc<dyn AdbTransport>,
    registry: &DeviceRegistry,
    request_id: bridgescope_domain::OperationId,
    target: DeviceTarget,
    format: ScreenshotFormat,
    events: mpsc::Sender<BackendEvent>,
    context: egui::Context,
) {
    if registry.current_online(&target).is_none() {
        send_event(
            &events,
            &context,
            BackendEvent::ScreenshotFailed {
                request_id,
                target,
                error: BridgeError::new(
                    ErrorCode::DeviceUnavailable,
                    "device.target_stale",
                    "device is no longer online",
                ),
            },
        )
        .await;
        return;
    }
    send_event(
        &events,
        &context,
        BackendEvent::ScreenshotLoading {
            request_id,
            target: target.clone(),
            format,
        },
    )
    .await;
    match transport.capture_screenshot(&target.serial).await {
        Ok(png) => {
            let raw_png = png.clone();
            let decoded = tokio::task::spawn_blocking(move || decode_screenshot(&png)).await;
            let data = match decoded {
                Ok(Ok(image)) => Ok(ScreenshotData::DecodedWithPng {
                    image,
                    png: RawScreenshotPng::new(raw_png),
                }),
                Ok(Err(error)) => Err(error),
                Err(error) => Err(BridgeError::new(
                    ErrorCode::Internal,
                    "screenshot.decode_task_failed",
                    error.to_string(),
                )),
            };
            match data {
                Ok(data) => {
                    send_event(
                        &events,
                        &context,
                        BackendEvent::ScreenshotCaptured {
                            request_id,
                            target,
                            data,
                        },
                    )
                    .await;
                }
                Err(error) => {
                    send_event(
                        &events,
                        &context,
                        BackendEvent::ScreenshotFailed {
                            request_id,
                            target,
                            error,
                        },
                    )
                    .await;
                }
            }
        }
        Err(error) => {
            send_event(
                &events,
                &context,
                BackendEvent::ScreenshotFailed {
                    request_id,
                    target,
                    error,
                },
            )
            .await;
        }
    }
}

async fn list_directory(
    transport: &dyn AdbTransport,
    registry: &DeviceRegistry,
    request_id: bridgescope_domain::OperationId,
    target: DeviceTarget,
    path: RemotePath,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    if registry.current_online(&target).is_none() {
        send_event(
            events,
            context,
            BackendEvent::DirectoryFailed {
                request_id,
                target,
                path,
                error: BridgeError::new(
                    ErrorCode::DeviceUnavailable,
                    "device.target_stale",
                    "device is no longer online",
                ),
            },
        )
        .await;
        return;
    }
    send_event(
        events,
        context,
        BackendEvent::DirectoryLoading {
            request_id,
            target: target.clone(),
            path: path.clone(),
        },
    )
    .await;
    match transport.list_directory(&target.serial, &path).await {
        Ok(entries) => {
            send_event(
                events,
                context,
                BackendEvent::DirectoryLoaded {
                    request_id,
                    listing: bridgescope_domain::DirectoryListing {
                        target,
                        directory: path,
                        entries,
                    },
                },
            )
            .await;
        }
        Err(error) => {
            send_event(
                events,
                context,
                BackendEvent::DirectoryFailed {
                    request_id,
                    target,
                    path,
                    error,
                },
            )
            .await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn start_transfer(
    transport: Arc<dyn AdbTransport>,
    registry: &DeviceRegistry,
    transfers: &mut HashMap<OperationId, ActiveTransfer>,
    results: mpsc::Sender<FileTaskResult>,
    request_id: OperationId,
    target: DeviceTarget,
    local_path: PathBuf,
    remote_path: RemotePath,
    overwrite: OverwritePolicy,
    upload: bool,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    if registry.current_online(&target).is_none() {
        send_event(
            events,
            context,
            BackendEvent::FileTransferFailed {
                request_id,
                target,
                error: BridgeError::new(
                    ErrorCode::DeviceUnavailable,
                    "device.target_stale",
                    "device is no longer online",
                ),
            },
        )
        .await;
        return;
    }
    let direction = if upload {
        FileTransferDirection::Upload
    } else {
        FileTransferDirection::Download
    };
    let cancellation = CancellationToken::new();
    transfers.insert(
        request_id,
        ActiveTransfer {
            target: target.clone(),
            cancellation: cancellation.clone(),
        },
    );
    send_event(
        events,
        context,
        BackendEvent::FileTransferStarted {
            request_id,
            direction,
            target: target.clone(),
            remote_path: remote_path.clone(),
            local_path: local_path.clone(),
        },
    )
    .await;
    tokio::spawn(async move {
        let transfer = if upload {
            transport
                .push_file(
                    &target.serial,
                    &local_path,
                    &remote_path,
                    overwrite,
                    cancellation,
                )
                .await
        } else {
            transport
                .pull_file(
                    &target.serial,
                    &remote_path,
                    &local_path,
                    overwrite,
                    cancellation,
                )
                .await
        };
        let result = transfer.map(|()| FileTransferSummary {
            direction,
            target: target.clone(),
            remote_path,
            local_path,
        });
        let _ = results
            .send(FileTaskResult::Transfer {
                request_id,
                target,
                result,
            })
            .await;
    });
}

fn cancel_stale_transfers(
    registry: &DeviceRegistry,
    transfers: &HashMap<OperationId, ActiveTransfer>,
) {
    for active in transfers.values() {
        if registry.current_online(&active.target).is_none() {
            active.cancellation.cancel();
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn start_mutation(
    transport: Arc<dyn AdbTransport>,
    registry: &DeviceRegistry,
    results: mpsc::Sender<FileTaskResult>,
    request_id: OperationId,
    target: DeviceTarget,
    kind: RemoteFileMutationKind,
    path: RemotePath,
    destination: Option<RemotePath>,
    confirmed: bool,
) {
    let unavailable = registry.current_online(&target).is_none();
    let invalid_delete = kind == RemoteFileMutationKind::DeleteFile && !confirmed;
    let result = if unavailable {
        Err(BridgeError::new(
            ErrorCode::DeviceUnavailable,
            "device.target_stale",
            "device is no longer online",
        ))
    } else if invalid_delete {
        Err(BridgeError::invalid_input(
            "file.delete_confirmation_required",
        ))
    } else {
        Ok(())
    };
    if let Err(error) = result {
        let _ = results
            .send(FileTaskResult::Mutation {
                request_id,
                target: target.clone(),
                result: Err(error),
            })
            .await;
        return;
    }
    tokio::spawn(async move {
        let operation = match kind {
            RemoteFileMutationKind::CreateDirectory => {
                transport.create_directory(&target.serial, &path).await
            }
            RemoteFileMutationKind::Rename => match destination.as_ref() {
                Some(destination) => {
                    transport
                        .rename_entry(&target.serial, &path, destination)
                        .await
                }
                None => Err(BridgeError::invalid_input(
                    "file.rename_destination_missing",
                )),
            },
            RemoteFileMutationKind::DeleteFile => {
                transport.delete_file(&target.serial, &path).await
            }
        };
        let result = operation.map(|()| RemoteFileMutationSummary {
            kind,
            target: target.clone(),
            path,
            destination,
        });
        let _ = results
            .send(FileTaskResult::Mutation {
                request_id,
                target,
                result,
            })
            .await;
    });
}

async fn finish_file_task(
    task: FileTaskResult,
    transfers: &mut HashMap<OperationId, ActiveTransfer>,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    let event = match task {
        FileTaskResult::Transfer {
            request_id,
            target,
            result,
        } => {
            transfers.remove(&request_id);
            match result {
                Ok(summary) => BackendEvent::FileTransferCompleted {
                    request_id,
                    summary,
                },
                Err(error) if error.code == ErrorCode::Cancelled => {
                    BackendEvent::FileTransferCancelled { request_id, target }
                }
                Err(error) => BackendEvent::FileTransferFailed {
                    request_id,
                    target,
                    error,
                },
            }
        }
        FileTaskResult::Mutation {
            request_id,
            target,
            result,
        } => match result {
            Ok(summary) => BackendEvent::FileMutationCompleted {
                request_id,
                summary,
            },
            Err(error) => BackendEvent::FileMutationFailed {
                request_id,
                target,
                error,
            },
        },
    };
    send_event(events, context, event).await;
}

fn decode_screenshot(png: &[u8]) -> Result<ScreenshotImage, BridgeError> {
    let image = image::ImageReader::with_format(Cursor::new(png), image::ImageFormat::Png)
        .decode()
        .map_err(|error| {
            BridgeError::new(
                ErrorCode::InvalidInput,
                "screenshot.decode_failed",
                error.to_string(),
            )
        })?;
    let width = image.width();
    let height = image.height();
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if width > MAX_SCREENSHOT_DIMENSION
        || height > MAX_SCREENSHOT_DIMENSION
        || pixels > MAX_SCREENSHOT_PIXELS
    {
        return Err(BridgeError::new(
            ErrorCode::OutputLimit,
            "screenshot.dimensions_too_large",
            format!("{width}x{height}"),
        ));
    }
    ScreenshotImage::new(width, height, image.to_rgba8().into_raw())
}

async fn initialize_transport(
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) -> Arc<dyn AdbTransport> {
    if fake_backend_enabled() {
        let transport: Arc<dyn AdbTransport> = Arc::new(FakeAdbTransport::default());
        send_event(
            events,
            context,
            BackendEvent::AdbReady {
                path: "fake://adb".to_owned(),
                version: transport
                    .version()
                    .await
                    .unwrap_or_else(|_| "fake".to_owned()),
            },
        )
        .await;
        return transport;
    }

    let explicit = std::env::var_os("BRIDGESCOPE_ADB").map(PathBuf::from);
    match AdbLocator::new(explicit).locate() {
        Ok(path) => {
            let transport: Arc<dyn AdbTransport> = Arc::new(ProcessAdbTransport::new(path.clone()));
            match transport.version().await {
                Ok(version) => {
                    info!(adb = %path.display(), "ADB initialized");
                    send_event(
                        events,
                        context,
                        BackendEvent::AdbReady {
                            path: path.display().to_string(),
                            version,
                        },
                    )
                    .await;
                    transport
                }
                Err(error) => {
                    warn!(detail = %error.detail, "ADB version check failed; using fake backend");
                    send_event(events, context, BackendEvent::AdbUnavailable(error)).await;
                    Arc::new(FakeAdbTransport::default())
                }
            }
        }
        Err(error) => {
            warn!(detail = %error.detail, "ADB not found; using fake backend");
            send_event(events, context, BackendEvent::AdbUnavailable(error)).await;
            Arc::new(FakeAdbTransport::default())
        }
    }
}

/// Resolve the AI provider for this session.
///
/// A placeholder [`FakeAiProvider`] is advertised in fake mode; otherwise the
/// session starts unconfigured and the UI installs a real provider with
/// [`BackendCommand::ConfigureAi`], which reports `AiReady`/`AiUnavailable`.
async fn initialize_ai(
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) -> Option<Arc<dyn bridgescope_ai::AiProvider>> {
    if fake_backend_enabled() {
        let provider: Arc<dyn bridgescope_ai::AiProvider> =
            Arc::new(bridgescope_ai::FakeAiProvider::new());
        send_event(
            events,
            context,
            BackendEvent::AiReady {
                kind: provider.kind().to_owned(),
                model: provider.model().to_owned(),
            },
        )
        .await;
        return Some(provider);
    }

    send_event(
        events,
        context,
        BackendEvent::AiUnavailable {
            reason: "no provider configured".to_owned(),
        },
    )
    .await;
    None
}

/// Install or remove the session's AI provider in response to
/// [`BackendCommand::ConfigureAi`].
///
/// The API key travels only through the command and the provider's in-memory
/// config; it is never logged and never included in error details.
async fn configure_ai(
    provider_slot: &mut Option<Arc<dyn bridgescope_ai::AiProvider>>,
    settings: Option<bridgescope_domain::AiSettings>,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    let Some(settings) = settings else {
        *provider_slot = None;
        send_event(
            events,
            context,
            BackendEvent::AiUnavailable {
                reason: "disabled by user".to_owned(),
            },
        )
        .await;
        return;
    };
    let config = bridgescope_ai::AiProviderConfig {
        kind: bridgescope_ai::OPENAI_COMPATIBLE_KIND.to_owned(),
        endpoint: settings.endpoint,
        model: settings.model,
        auth: bridgescope_ai::AuthTokenSource::Inline {
            value: settings.api_key,
        },
        timeout_seconds: settings.timeout_seconds,
    };
    match bridgescope_ai::OpenAiCompatibleProvider::new(config) {
        Ok(provider) => {
            let kind = bridgescope_ai::AiProvider::kind(&provider).to_owned();
            let model = bridgescope_ai::AiProvider::model(&provider).to_owned();
            info!(kind = %kind, model = %model, "AI provider configured");
            *provider_slot = Some(Arc::new(provider));
            send_event(events, context, BackendEvent::AiReady { kind, model }).await;
        }
        Err(error) => {
            *provider_slot = None;
            warn!(reason = %error, "AI configuration rejected");
            send_event(
                events,
                context,
                BackendEvent::AiUnavailable {
                    reason: error.to_string(),
                },
            )
            .await;
        }
    }
}

async fn run_ai_chat(
    provider: Option<Arc<dyn bridgescope_ai::AiProvider>>,
    request_id: bridgescope_domain::OperationId,
    prompt: String,
    events: mpsc::Sender<BackendEvent>,
    context: egui::Context,
) {
    let Some(provider) = provider else {
        send_event(
            &events,
            &context,
            BackendEvent::AiChatFailed {
                request_id,
                error: BridgeError::new(
                    ErrorCode::Internal,
                    "ai.not_configured",
                    "no AI provider is configured",
                ),
            },
        )
        .await;
        return;
    };

    let request = bridgescope_ai::ChatRequest::new(vec![
        bridgescope_ai::ChatMessage::new(
            bridgescope_ai::ChatRole::System,
            BRIDGESCOPE_SYSTEM_PROMPT,
        ),
        bridgescope_ai::ChatMessage::user(prompt),
    ])
    .authorized_by(
        bridgescope_ai::ContextAuthorization::none()
            .grant(bridgescope_ai::ContextGrant::SystemPrompt),
    );

    let result = provider.complete(&request).await;
    match result {
        Ok(response) => {
            send_event(
                &events,
                &context,
                BackendEvent::AiChatCompleted {
                    request_id,
                    reply: response.message.content,
                },
            )
            .await;
        }
        Err(error) => {
            send_event(
                &events,
                &context,
                BackendEvent::AiChatFailed {
                    request_id,
                    error: BridgeError::new(
                        ErrorCode::Internal,
                        "ai.request_failed",
                        error.to_string(),
                    ),
                },
            )
            .await;
        }
    }
}

const BRIDGESCOPE_SYSTEM_PROMPT: &str = "You are BridgeScope's assistant. BridgeScope is a pure-Rust Android debugging tool. \
Keep replies concise and focused on Android debugging, ADB, and the device the user is testing.";

async fn refresh_devices(
    transport: &dyn AdbTransport,
    registry: &mut DeviceRegistry,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) -> Result<(), BridgeError> {
    match transport.list_devices().await {
        Ok(devices) => {
            let selected = registry.snapshot().selected;
            let snapshot = registry.reconcile(devices);
            let selected_online = snapshot.selected.as_ref().and_then(|serial| {
                snapshot
                    .devices
                    .iter()
                    .find(|record| &record.descriptor.serial == serial)
                    .filter(|record| record.descriptor.state.is_online())
                    .map(|_| serial.clone())
            });
            send_event(events, context, BackendEvent::DevicesChanged(snapshot)).await;
            if selected == selected_online
                && let Some(serial) = selected_online
            {
                load_overview(transport, registry, serial, events, context).await;
            }
            Ok(())
        }
        Err(error) => {
            send_event(
                events,
                context,
                BackendEvent::OperationFailed(error.clone()),
            )
            .await;
            Err(error)
        }
    }
}

async fn load_overview(
    transport: &dyn AdbTransport,
    registry: &DeviceRegistry,
    serial: bridgescope_domain::DeviceSerial,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    let Some(generation) = registry.generation(&serial) else {
        return;
    };
    send_event(
        events,
        context,
        BackendEvent::OverviewLoading(serial.clone()),
    )
    .await;
    match bridgescope_device::load_overview(transport, &serial, generation, registry).await {
        Ok(overview) => send_event(events, context, BackendEvent::OverviewLoaded(overview)).await,
        Err(error) => send_event(events, context, BackendEvent::OperationFailed(error)).await,
    }
}

async fn load_processes(
    transport: &dyn AdbTransport,
    registry: &DeviceRegistry,
    target: DeviceTarget,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    if registry.current_online(&target).is_none() {
        send_event(
            events,
            context,
            BackendEvent::ProcessesFailed {
                target,
                error: BridgeError::new(
                    ErrorCode::DeviceUnavailable,
                    "device.target_stale",
                    "selected device is no longer online",
                ),
            },
        )
        .await;
        return;
    }
    let event_target = target.clone();
    send_event(
        events,
        context,
        BackendEvent::ProcessesLoading(target.clone()),
    )
    .await;
    match transport.list_processes(&target.serial).await {
        Ok(processes) => {
            send_event(
                events,
                context,
                BackendEvent::ProcessesLoaded(ProcessSnapshot {
                    target: event_target,
                    processes,
                }),
            )
            .await;
        }
        Err(error) => {
            send_event(
                events,
                context,
                BackendEvent::ProcessesFailed { target, error },
            )
            .await;
        }
    }
}

async fn load_performance(
    transport: &dyn AdbTransport,
    registry: &DeviceRegistry,
    target: DeviceTarget,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    if registry.current_online(&target).is_none() {
        send_event(
            events,
            context,
            BackendEvent::PerformanceFailed {
                target,
                error: BridgeError::new(
                    ErrorCode::DeviceUnavailable,
                    "device.target_stale",
                    "selected device is no longer online",
                ),
            },
        )
        .await;
        return;
    }
    let event_target = target.clone();
    send_event(
        events,
        context,
        BackendEvent::PerformanceLoading(target.clone()),
    )
    .await;
    match transport.performance_metrics(&target.serial).await {
        Ok(metrics) => {
            send_event(
                events,
                context,
                BackendEvent::PerformanceLoaded(PerformanceSnapshot {
                    target: event_target,
                    metrics,
                }),
            )
            .await;
        }
        Err(error) => {
            send_event(
                events,
                context,
                BackendEvent::PerformanceFailed { target, error },
            )
            .await;
        }
    }
}

async fn send_event(
    sender: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
    event: BackendEvent,
) {
    if sender.send(event).await.is_ok() {
        context.request_repaint();
    }
}

async fn load_applications(
    transport: &dyn AdbTransport,
    registry: &DeviceRegistry,
    target: DeviceTarget,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    if registry.current_online(&target).is_none() {
        send_event(
            events,
            context,
            BackendEvent::ApplicationsFailed {
                target,
                error: BridgeError::new(
                    ErrorCode::DeviceUnavailable,
                    "device.target_stale",
                    "selected device is no longer online",
                ),
            },
        )
        .await;
        return;
    }
    let event_target = target.clone();
    send_event(
        events,
        context,
        BackendEvent::ApplicationsLoading(target.clone()),
    )
    .await;
    match transport.list_applications(&target.serial).await {
        Ok(applications) => {
            send_event(
                events,
                context,
                BackendEvent::ApplicationsLoaded(ApplicationSnapshot {
                    target: event_target,
                    applications,
                }),
            )
            .await;
        }
        Err(error) => {
            send_event(
                events,
                context,
                BackendEvent::ApplicationsFailed { target, error },
            )
            .await;
        }
    }
}

/// Extracts launcher icons one package at a time; failures are silently
/// skipped because the grid renders a fallback tile for missing icons.
/// Concurrent per-package icon requests: each extraction is a half dozen
/// adb round trips, so serial fetching made the grid trickle in. The
/// device serializes real work in adbd anyway; this only overlaps the
/// process spawns, transfers, and host-side parsing.
const ICON_FETCH_CONCURRENCY: usize = 6;

async fn load_application_icons(
    transport: &dyn AdbTransport,
    registry: &DeviceRegistry,
    target: DeviceTarget,
    packages: Vec<PackageName>,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    if registry.current_online(&target).is_none() {
        return;
    }
    futures_util::stream::iter(packages.into_iter().map(|package| async {
        let icon = transport.application_icon(&target.serial, &package).await;
        if let Ok(Some(icon)) = icon {
            send_event(
                events,
                context,
                BackendEvent::ApplicationIconLoaded {
                    target: target.clone(),
                    package,
                    icon,
                },
            )
            .await;
        }
    }))
    .buffer_unordered(ICON_FETCH_CONCURRENCY)
    .for_each(|()| async {})
    .await;
}

async fn load_application_details(
    transport: &dyn AdbTransport,
    registry: &DeviceRegistry,
    request_id: OperationId,
    target: DeviceTarget,
    package: PackageName,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    if registry.current_online(&target).is_none() {
        send_event(
            events,
            context,
            BackendEvent::ApplicationDetailsFailed {
                request_id,
                target,
                package,
                error: BridgeError::new(
                    ErrorCode::DeviceUnavailable,
                    "device.target_stale",
                    "selected device is no longer online",
                ),
            },
        )
        .await;
        return;
    }
    send_event(
        events,
        context,
        BackendEvent::ApplicationDetailsLoading {
            request_id,
            target: target.clone(),
            package: package.clone(),
        },
    )
    .await;
    match transport
        .application_details(&target.serial, &package)
        .await
    {
        Ok(details) => {
            send_event(
                events,
                context,
                BackendEvent::ApplicationDetailsLoaded {
                    request_id,
                    details,
                },
            )
            .await;
        }
        Err(error) => {
            send_event(
                events,
                context,
                BackendEvent::ApplicationDetailsFailed {
                    request_id,
                    target,
                    package,
                    error,
                },
            )
            .await;
        }
    }
}

// One argument per event field; the transport is borrowed like the other
// handlers.
#[allow(clippy::too_many_arguments)]
async fn run_application_action(
    transport: &dyn AdbTransport,
    registry: &DeviceRegistry,
    request_id: OperationId,
    action: ApplicationAction,
    target: DeviceTarget,
    package: PackageName,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    if registry.current_online(&target).is_none() {
        send_event(
            events,
            context,
            BackendEvent::ApplicationActionFailed {
                request_id,
                action,
                target,
                package,
                error: BridgeError::new(
                    ErrorCode::DeviceUnavailable,
                    "device.target_stale",
                    "selected device is no longer online",
                ),
            },
        )
        .await;
        return;
    }
    send_event(
        events,
        context,
        BackendEvent::ApplicationActionStarted {
            request_id,
            action,
            target: target.clone(),
            package: package.clone(),
        },
    )
    .await;
    let result = perform_application_action(transport, action, &target, &package).await;
    match result {
        Ok(()) => {
            send_event(
                events,
                context,
                BackendEvent::ApplicationActionCompleted {
                    request_id,
                    action,
                    target,
                    package,
                },
            )
            .await;
        }
        Err(error) => {
            send_event(
                events,
                context,
                BackendEvent::ApplicationActionFailed {
                    request_id,
                    action,
                    target,
                    package,
                    error,
                },
            )
            .await;
        }
    }
}

/// Dispatches one package action against the transport.
async fn perform_application_action(
    transport: &dyn AdbTransport,
    action: ApplicationAction,
    target: &DeviceTarget,
    package: &PackageName,
) -> Result<(), BridgeError> {
    match action {
        ApplicationAction::Launch => transport.launch_application(&target.serial, package).await,
        ApplicationAction::ForceStop => {
            transport
                .force_stop_application(&target.serial, package)
                .await
        }
        ApplicationAction::ClearData => {
            transport
                .clear_application_data(&target.serial, package)
                .await
        }
        ApplicationAction::Freeze => {
            transport
                .set_application_frozen(&target.serial, package, true)
                .await
        }
        ApplicationAction::Unfreeze => {
            transport
                .set_application_frozen(&target.serial, package, false)
                .await
        }
        ApplicationAction::Uninstall => {
            transport
                .uninstall_application(&target.serial, package)
                .await
        }
    }
}

fn fake_backend_enabled() -> bool {
    std::env::var("BRIDGESCOPE_FAKE")
        .is_ok_and(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}
