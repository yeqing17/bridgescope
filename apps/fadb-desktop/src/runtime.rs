use std::{
    collections::HashMap,
    io::Cursor,
    path::PathBuf,
    sync::{Arc, RwLock},
    thread,
    time::Duration,
};

use eframe::egui;
use fadb_adb::{AdbLocator, AdbTransport, ProcessAdbTransport, ShellSessionHandle};
use fadb_device::DeviceRegistry;
use fadb_domain::{
    ApplicationAction, ApplicationSnapshot, BackendCommand, BackendEvent, BridgeError,
    DeviceSerial, DeviceTarget, ErrorCode, FileTransferDirection, FileTransferSummary,
    LogcatSessionId, OperationId, OverwritePolicy, PackageName, PerformanceSnapshot,
    ProcessSnapshot, RawScreenshotPng, RemoteFileMutationKind, RemoteFileMutationSummary,
    RemotePath, ScreenshotData, ScreenshotFormat, ScreenshotImage, ShellSessionId, ShellSize,
    WebViewPage,
};
use fadb_scrcpy::decoder::{RgbaFrame, VideoDecoder};
use fadb_scrcpy::{
    ScrcpySessionPlan, recorder as scrcpy_recorder, server as scrcpy_server,
    session as scrcpy_session, session::SessionError,
};
use fadb_test_support::FakeAdbTransport;
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
/// Total budget for the scrcpy server to come up and announce its video
/// stream through the forward tunnel.
const MIRROR_SERVER_TIMEOUT: Duration = Duration::from_secs(15);
/// One local connect attempt against adb's forward listener.
const MIRROR_CONNECT_ATTEMPT: Duration = Duration::from_secs(1);
/// Pause between connect attempts while the server is still booting.
const MIRROR_CONNECT_RETRY: Duration = Duration::from_millis(150);
/// Tail of the server shell output kept for surfacing startup failures.
const MIRROR_SERVER_LOG_TAIL: usize = 2_000;

struct ActiveShell {
    handle: ShellSessionHandle,
}

/// The latest decoded mirror frame plus a monotonic counter the UI diffs to
/// detect new frames; frames never travel through the event channel.
#[derive(Default)]
pub struct MirrorFrameBuffer {
    pub frame: Option<Arc<RgbaFrame>>,
    pub decoded: u64,
}

struct ActiveMirror {
    cancellation: CancellationToken,
    /// `Some(path)` while a recording was requested. The session's packet
    /// loop owns the actual recorder and polls this flag between packets;
    /// the file is written and announced by the session task.
    recording: Arc<std::sync::Mutex<Option<PathBuf>>>,
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
    mirror_frames: Arc<std::sync::Mutex<MirrorFrameBuffer>>,
}

impl RuntimeBridge {
    pub fn spawn(context: egui::Context) -> Self {
        let (command_tx, command_rx) = mpsc::channel(CHANNEL_CAPACITY);
        let (event_tx, event_rx) = mpsc::channel(CHANNEL_CAPACITY);
        let shell_inputs = Arc::new(RwLock::new(HashMap::new()));
        let mirror_frames = Arc::new(std::sync::Mutex::new(MirrorFrameBuffer::default()));

        let backend_context = context.clone();
        let backend_shell_inputs = Arc::clone(&shell_inputs);
        let backend_mirror_frames = Arc::clone(&mirror_frames);
        thread::Builder::new()
            .name("fadb-backend".to_owned())
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
                    backend_mirror_frames,
                ));
            })
            .expect("backend thread must start");

        Self {
            command_tx,
            shell_inputs,
            event_rx,
            context,
            mirror_frames,
        }
    }

    /// Shared mirror frame buffer written by the backend and read by the
    /// mirror panel on every repaint.
    pub fn mirror_frames(&self) -> Arc<std::sync::Mutex<MirrorFrameBuffer>> {
        Arc::clone(&self.mirror_frames)
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
    mirror_frames: Arc<std::sync::Mutex<MirrorFrameBuffer>>,
) {
    let transport = initialize_transport(&events, &context).await;
    let mut ai_provider = initialize_ai(&events, &context).await;
    let mut registry = DeviceRegistry::default();
    let mut shells = HashMap::<ShellSessionId, ActiveShell>::new();
    let mut logcats = HashMap::<LogcatSessionId, ShellSessionHandle>::new();
    let webview_forwards = Arc::new(std::sync::Mutex::new(Vec::<u16>::new()));
    let mut mirror: Option<ActiveMirror> = None;
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
                    BackendCommand::PairDevice {
                        request_id,
                        host,
                        port,
                        code,
                    } => {
                        pair_device(
                            transport.as_ref(),
                            request_id,
                            host,
                            port,
                            code,
                            &events,
                            &context,
                        )
                        .await;
                    }
                    BackendCommand::EnableTcpIp {
                        request_id,
                        serial,
                        port,
                    } => {
                        enable_tcpip(transport.as_ref(), request_id, serial, port, &events, &context)
                            .await;
                    }
                    BackendCommand::ListMdnsServices => {
                        list_mdns_services(transport.as_ref(), &events, &context).await;
                    }
                    BackendCommand::StartMirror {
                        request_id,
                        target,
                        max_size,
                        video_bit_rate,
                    } => {
                        start_mirror(
                            Arc::clone(&transport),
                            &registry,
                            &mut mirror,
                            Arc::clone(&mirror_frames),
                            request_id,
                            target,
                            max_size,
                            video_bit_rate,
                            &events,
                            &context,
                        )
                        .await;
                    }
                    BackendCommand::StopMirror => {
                        stop_mirror(&mut mirror);
                    }
                    BackendCommand::StartMirrorRecording => {
                        start_mirror_recording(mirror.as_ref(), &events, &context).await;
                    }
                    BackendCommand::StopMirrorRecording => {
                        if let Some(active) = mirror.as_ref()
                            && let Ok(mut wanted) = active.recording.lock()
                        {
                            // The session finalizes the file on the next
                            // packet (or at session end) and reports it.
                            *wanted = None;
                        }
                    }
                    BackendCommand::SendKeyEvent { target, keycode } => {
                        send_key_event(
                            transport.as_ref(),
                            &registry,
                            target,
                            keycode,
                            &events,
                            &context,
                        )
                        .await;
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
                    BackendCommand::InstallApk {
                        request_id,
                        target,
                        apk_path,
                    } => {
                        install_apk(
                            transport.clone(),
                            &registry,
                            request_id,
                            target,
                            apk_path,
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
    output: &mut mpsc::Receiver<fadb_adb::ShellOutputChunk>,
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

async fn install_apk(
    transport: Arc<dyn AdbTransport>,
    registry: &DeviceRegistry,
    request_id: OperationId,
    target: DeviceTarget,
    apk_path: std::path::PathBuf,
    events: mpsc::Sender<BackendEvent>,
    context: egui::Context,
) {
    let Some(record) = registry.current_online(&target) else {
        let error = stale_target_error(&target);
        send_event(
            &events,
            &context,
            BackendEvent::ApkInstallFailed {
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
        BackendEvent::ApkInstallLoading {
            request_id,
            target: current.clone(),
        },
    )
    .await;
    // Spawned: a streamed install can run for minutes and must not stall the
    // control loop.
    tokio::spawn(async move {
        let event = match transport.install_apk(&current.serial, &apk_path).await {
            Ok(()) => BackendEvent::ApkInstallFinished {
                request_id,
                target: current,
            },
            Err(error) => BackendEvent::ApkInstallFailed {
                request_id,
                target: current,
                error,
            },
        };
        send_event(&events, &context, event).await;
    });
}

/// Pairs with a wireless-debugging device; the success event only confirms
/// the pairing, connecting still goes through the regular connect flow.
async fn pair_device(
    transport: &dyn AdbTransport,
    request_id: OperationId,
    host: String,
    port: u16,
    code: String,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    let event = match transport.pair_device(&host, port, &code).await {
        Ok(()) => BackendEvent::PairFinished { request_id },
        Err(error) => BackendEvent::PairFailed { request_id, error },
    };
    send_event(events, context, event).await;
}

async fn enable_tcpip(
    transport: &dyn AdbTransport,
    request_id: OperationId,
    serial: DeviceSerial,
    port: u16,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    let event = match transport.enable_tcpip(&serial, port).await {
        Ok(()) => BackendEvent::TcpIpEnabled { request_id, serial },
        Err(error) => BackendEvent::TcpIpFailed {
            request_id,
            serial,
            error,
        },
    };
    send_event(events, context, event).await;
}

async fn list_mdns_services(
    transport: &dyn AdbTransport,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    let event = match transport.mdns_services().await {
        Ok(services) => BackendEvent::MdnsServicesLoaded { services },
        Err(error) => BackendEvent::MdnsFailed { error },
    };
    send_event(events, context, event).await;
}

/// Starts the single mirror session; a running session is stopped first so
/// there is never more than one active mirror.
#[allow(clippy::too_many_arguments)]
async fn start_mirror(
    transport: Arc<dyn AdbTransport>,
    registry: &DeviceRegistry,
    mirror: &mut Option<ActiveMirror>,
    mirror_frames: Arc<std::sync::Mutex<MirrorFrameBuffer>>,
    request_id: OperationId,
    target: DeviceTarget,
    max_size: Option<u32>,
    video_bit_rate: u32,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    stop_mirror(mirror);
    if registry.current_online(&target).is_none() {
        let error = stale_target_error(&target);
        send_event(
            events,
            context,
            BackendEvent::MirrorFailed {
                request_id,
                target,
                error,
            },
        )
        .await;
        return;
    }
    let plan = ScrcpySessionPlan {
        device: target.serial.clone(),
        max_size,
        video_bit_rate,
    };
    // 31-bit session id per the scrcpy protocol docs; derived from a v4 UUID
    // so concurrent Fadb instances pick distinct socket names.
    let scid = (uuid::Uuid::new_v4().as_u128() & 0x7fff_ffff) as u32;
    // Bare abstract name: the transport's forward_port adds the
    // `localabstract:` prefix itself.
    let socket = scrcpy_server::abstract_socket_name(scid);
    let args = scrcpy_server::server_arguments(scid, &plan);
    let cancellation = CancellationToken::new();
    let recording = Arc::new(std::sync::Mutex::new(None));
    *mirror = Some(ActiveMirror {
        cancellation: cancellation.clone(),
        recording: Arc::clone(&recording),
    });
    tokio::spawn(run_mirror_session(
        transport,
        target,
        request_id,
        args,
        socket,
        cancellation,
        recording,
        mirror_frames,
        events.clone(),
        context.clone(),
    ));
}

fn stop_mirror(mirror: &mut Option<ActiveMirror>) {
    if let Some(active) = mirror.take() {
        // The session task observes the cancellation, tears the server down,
        // and reports MirrorStopped; nothing to await here.
        active.cancellation.cancel();
    }
}

/// Records the running mirror session to `./recordings/fadb-<ts>.mp4`
/// (same convention as the screenshot panel). The session picks the request
/// up between packets and announces the file only when recording stops.
async fn start_mirror_recording(
    mirror: Option<&ActiveMirror>,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    let Some(active) = mirror else {
        send_event(
            events,
            context,
            BackendEvent::OperationFailed(BridgeError::new(
                ErrorCode::Internal,
                "mirror.record_not_running",
                String::new(),
            )),
        )
        .await;
        return;
    };
    let directory = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("recordings");
    let _ = std::fs::create_dir_all(&directory);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = directory.join(format!("fadb-{timestamp}.mp4"));
    if let Ok(mut wanted) = active.recording.lock() {
        *wanted = Some(path);
    }
}

/// Maps a finalized recording to the panel events. The demux packet loop
/// calls this from a sync closure where awaiting is impossible, so it uses
/// `try_send` — the app loop drains the channel continuously.
fn report_recording(
    outcome: scrcpy_recorder::RecorderOutcome,
    target: &DeviceTarget,
    path: &std::path::Path,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    let event = match outcome {
        scrcpy_recorder::RecorderOutcome::Written(frames) => BackendEvent::MirrorRecordingSaved {
            target: target.clone(),
            path: path.to_path_buf(),
            frames,
        },
        scrcpy_recorder::RecorderOutcome::Empty => BackendEvent::MirrorRecordingFailed {
            target: target.clone(),
            error: BridgeError::new(ErrorCode::Internal, "mirror.record_empty", String::new()),
        },
        scrcpy_recorder::RecorderOutcome::Failed(detail) => BackendEvent::MirrorRecordingFailed {
            target: target.clone(),
            error: BridgeError::new(ErrorCode::Internal, "mirror.record_write_failed", detail),
        },
    };
    let _ = events.try_send(event);
    context.request_repaint();
}

/// Runs one mirror session end to end: push the pinned server jar, forward a
/// local port to the server's abstract socket, launch it inside a shell
/// session, then demux and decode the video stream until stopped or ended.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn run_mirror_session(
    transport: Arc<dyn AdbTransport>,
    target: DeviceTarget,
    request_id: OperationId,
    args: Vec<String>,
    socket: String,
    cancellation: CancellationToken,
    recording: Arc<std::sync::Mutex<Option<PathBuf>>>,
    mirror_frames: Arc<std::sync::Mutex<MirrorFrameBuffer>>,
    events: mpsc::Sender<BackendEvent>,
    context: egui::Context,
) {
    let serial = target.serial.clone();
    let fail =
        |key: &'static str, detail: String| BridgeError::new(ErrorCode::Internal, key, detail);
    // Tail of the server shell output, kept for surfacing startup failures.
    let server_log: Arc<std::sync::Mutex<String>> = Arc::default();
    let mut forwarded_port: Option<u16> = None;

    // Early cancellation (user hit stop while the start was still queuing)
    // skips the whole launch sequence.
    if cancellation.is_cancelled() {
        send_event(
            &events,
            &context,
            BackendEvent::MirrorStopped { request_id, target },
        )
        .await;
        return;
    }

    let temp_jar = std::env::temp_dir().join("fadb-scrcpy-server.jar");
    let startup = async {
        tokio::fs::write(&temp_jar, scrcpy_server::SERVER_JAR)
            .await
            .map_err(|error| fail("mirror.jar_write_failed", error.to_string()))?;
        let remote = RemotePath::new(scrcpy_server::SERVER_REMOTE_PATH)
            .map_err(|error| fail("mirror.jar_write_failed", error.to_string()))?;
        transport
            .push_file(
                &serial,
                &temp_jar,
                &remote,
                OverwritePolicy::ReplaceConfirmed,
                cancellation.clone(),
            )
            .await
            .map_err(|error| fail("mirror.push_failed", error.detail.clone()))?;
        // Forward tunnel: pick a free local port and release it again, so
        // adb's forward listener can bind it (same probe trick as scrcpy).
        let probe = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|error| fail("mirror.listen_failed", error.to_string()))?;
        let port = probe
            .local_addr()
            .map_err(|error| fail("mirror.listen_failed", error.to_string()))?
            .port();
        drop(probe);
        transport
            .forward_port(&serial, port, &socket)
            .await
            .map_err(|error| fail("mirror.forward_failed", error.detail.clone()))?;
        forwarded_port = Some(port);
        let mut shell = transport
            .start_shell(
                &serial,
                ShellSize::new(200, 40)
                    .map_err(|error| fail("mirror.shell_failed", error.detail.clone()))?,
            )
            .await
            .map_err(|error| fail("mirror.shell_failed", error.detail.clone()))?;
        let command = format!(
            "CLASSPATH={} app_process / com.genymobile.scrcpy.Server {}\n",
            scrcpy_server::SERVER_REMOTE_PATH,
            args.join(" ")
        );
        shell
            .input()
            .send(command.into_bytes())
            .await
            .map_err(|error| fail("mirror.shell_failed", error.to_string()))?;
        let log_writer = Arc::clone(&server_log);
        let mut output = std::mem::replace(shell.output_mut(), mpsc::channel(1).1);
        tokio::spawn(async move {
            while let Some(chunk) = output.recv().await {
                if let Ok(mut tail) = log_writer.lock() {
                    tail.push_str(&String::from_utf8_lossy(&chunk.bytes));
                    let length = tail.len();
                    if length > MIRROR_SERVER_LOG_TAIL {
                        tail.drain(..length - MIRROR_SERVER_LOG_TAIL);
                    }
                }
            }
        });
        // The server listens on the device; adb accepts our local connection
        // even before the server is up (then closes it on a failed
        // device-side connect), so poll until the stream header arrives.
        let deadline = tokio::time::Instant::now() + MIRROR_SERVER_TIMEOUT;
        let (tcp, header) = loop {
            if cancellation.is_cancelled() {
                return Err(fail("mirror.stream_failed", "cancelled".to_owned()));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(fail("mirror.server_start_timeout", String::new()));
            }
            let Ok(Ok(mut tcp)) = tokio::time::timeout(
                MIRROR_CONNECT_ATTEMPT,
                tokio::net::TcpStream::connect(("127.0.0.1", port)),
            )
            .await
            else {
                retry_pause(&cancellation).await;
                continue;
            };
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, scrcpy_session::read_stream_header(&mut tcp))
                .await
            {
                Ok(Ok(header)) => break (tcp, header),
                // The server is not listening yet; adb closed the tunnel.
                Ok(Err(SessionError::Io(error)))
                    if error.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    retry_pause(&cancellation).await;
                }
                Ok(Err(error)) => return Err(mirror_stream_error(error)),
                Err(_) => return Err(fail("mirror.server_start_timeout", String::new())),
            }
        };
        Ok::<_, BridgeError>((
            tcp,
            shell,
            port,
            header.metadata.width,
            header.metadata.height,
        ))
    }
    .await;
    let (mut tcp, shell, port, width, height) = match startup {
        Ok(parts) => parts,
        Err(mut error) => {
            if let Ok(tail) = server_log.lock() {
                let trimmed = tail.trim();
                if !trimmed.is_empty() {
                    error.detail.push_str(" | server: ");
                    let end = trimmed.len().min(600);
                    error.detail.push_str(trimmed.get(..end).unwrap_or(trimmed));
                }
            }
            if let Some(port) = forwarded_port.take() {
                spawn_remove_forward(&transport, &serial, port);
            }
            send_event(
                &events,
                &context,
                BackendEvent::MirrorFailed {
                    request_id,
                    target,
                    error,
                },
            )
            .await;
            return;
        }
    };
    send_event(
        &events,
        &context,
        BackendEvent::MirrorStarted {
            request_id,
            target: target.clone(),
            width,
            height,
        },
    )
    .await;

    let frame_buffer = Arc::clone(&mirror_frames);
    let mut decoder = match VideoDecoder::new() {
        Ok(decoder) => decoder,
        Err(error) => {
            send_event(
                &events,
                &context,
                BackendEvent::MirrorFailed {
                    request_id,
                    target,
                    error: fail("mirror.stream_failed", error.to_string()),
                },
            )
            .await;
            return;
        }
    };
    // The recorder is owned by the packet loop; the flag is flipped by the
    // command handlers. A stop takes effect on the next packet, so latency is
    // bounded by one frame period.
    let mut recorder: Option<scrcpy_recorder::VideoRecorder> = None;
    let result = scrcpy_session::demux_packets(
        &mut tcp,
        &mut decoder,
        &cancellation,
        &mut |header, payload, frame| {
            if let Some(frame) = frame
                && let Ok(mut buffer) = frame_buffer.lock()
            {
                buffer.decoded += 1;
                buffer.frame = Some(Arc::new(frame));
            }
            let wanted = recording.lock().ok().and_then(|flag| flag.clone());
            match wanted {
                Some(path) if recorder.is_none() => {
                    recorder = Some(scrcpy_recorder::VideoRecorder::new(path, width, height));
                }
                None => {
                    if let Some(mut active) = recorder.take() {
                        let path = active.path().to_path_buf();
                        report_recording(active.finish(), &target, &path, &events, &context);
                    }
                }
                _ => {}
            }
            if let Some(active) = recorder.as_mut() {
                active.feed(header.config, header.key_frame, header.pts, payload);
            }
        },
    )
    .await;

    drop(tcp);
    let _ = shell.close().await;
    spawn_remove_forward(&transport, &serial, port);
    // A still-active recording is finalized even when the session ends first.
    if let Some(mut active) = recorder.take() {
        let path = active.path().to_path_buf();
        report_recording(active.finish(), &target, &path, &events, &context);
    }

    let event = match result {
        Ok(_) | Err(SessionError::Cancelled) => BackendEvent::MirrorStopped { request_id, target },
        Err(error) => BackendEvent::MirrorFailed {
            request_id,
            target,
            error: mirror_stream_error(error),
        },
    };
    send_event(&events, &context, event).await;
}

fn spawn_remove_forward(transport: &Arc<dyn AdbTransport>, serial: &DeviceSerial, port: u16) {
    let transport = Arc::clone(transport);
    let serial = serial.clone();
    tokio::spawn(async move {
        let _ = transport.remove_forward(&serial, port).await;
    });
}

/// Cancellable pause between forward-tunnel connect attempts.
async fn retry_pause(cancellation: &CancellationToken) {
    tokio::select! {
        () = cancellation.cancelled() => {}
        () = tokio::time::sleep(MIRROR_CONNECT_RETRY) => {}
    }
}

/// Maps stream errors to user-visible error keys.
fn mirror_stream_error(error: SessionError) -> BridgeError {
    match error {
        SessionError::Codec { id } => BridgeError::new(
            ErrorCode::Internal,
            "mirror.codec_mismatch",
            format!("codec id 0x{id:08x}"),
        ),
        other => BridgeError::new(
            ErrorCode::Internal,
            "mirror.stream_failed",
            other.to_string(),
        ),
    }
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
    request_id: fadb_domain::OperationId,
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
    request_id: fadb_domain::OperationId,
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
                    listing: fadb_domain::DirectoryListing {
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

    let explicit = std::env::var_os("FADB_ADB").map(PathBuf::from);
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
) -> Option<Arc<dyn fadb_ai::AiProvider>> {
    if fake_backend_enabled() {
        let provider: Arc<dyn fadb_ai::AiProvider> = Arc::new(fadb_ai::FakeAiProvider::new());
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
    provider_slot: &mut Option<Arc<dyn fadb_ai::AiProvider>>,
    settings: Option<fadb_domain::AiSettings>,
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
    let config = fadb_ai::AiProviderConfig {
        kind: fadb_ai::OPENAI_COMPATIBLE_KIND.to_owned(),
        endpoint: settings.endpoint,
        model: settings.model,
        auth: fadb_ai::AuthTokenSource::Inline {
            value: settings.api_key,
        },
        timeout_seconds: settings.timeout_seconds,
    };
    match fadb_ai::OpenAiCompatibleProvider::new(config) {
        Ok(provider) => {
            let kind = fadb_ai::AiProvider::kind(&provider).to_owned();
            let model = fadb_ai::AiProvider::model(&provider).to_owned();
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
    provider: Option<Arc<dyn fadb_ai::AiProvider>>,
    request_id: fadb_domain::OperationId,
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

    let request = fadb_ai::ChatRequest::new(vec![
        fadb_ai::ChatMessage::new(fadb_ai::ChatRole::System, FADB_SYSTEM_PROMPT),
        fadb_ai::ChatMessage::user(prompt),
    ])
    .authorized_by(
        fadb_ai::ContextAuthorization::none().grant(fadb_ai::ContextGrant::SystemPrompt),
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

const FADB_SYSTEM_PROMPT: &str = "You are Fadb's assistant. Fadb is a pure-Rust Android debugging tool. \
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
    serial: fadb_domain::DeviceSerial,
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
    match fadb_device::load_overview(transport, &serial, generation, registry).await {
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

/// Delivers one remote-control key press. Fire-and-forget by design: only
/// failures surface, through the generic error event.
async fn send_key_event(
    transport: &dyn AdbTransport,
    registry: &DeviceRegistry,
    target: DeviceTarget,
    keycode: u32,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
    let error = if registry.current_online(&target).is_none() {
        Some(BridgeError::new(
            ErrorCode::DeviceUnavailable,
            "device.target_stale",
            "selected device is no longer online",
        ))
    } else {
        transport
            .send_key_event(&target.serial, keycode)
            .await
            .err()
    };
    if let Some(error) = error {
        send_event(events, context, BackendEvent::OperationFailed(error)).await;
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
    std::env::var("FADB_FAKE")
        .is_ok_and(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}
