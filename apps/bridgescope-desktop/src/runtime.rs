use std::{path::PathBuf, sync::Arc, thread, time::Duration};

use bridgescope_adb::{AdbLocator, AdbTransport, ProcessAdbTransport};
use bridgescope_device::DeviceRegistry;
use bridgescope_domain::{BackendCommand, BackendEvent, BridgeError, ErrorCode};
use bridgescope_test_support::FakeAdbTransport;
use eframe::egui;
use tokio::{runtime::Builder, sync::mpsc, time::MissedTickBehavior};
use tracing::{info, warn};

const CHANNEL_CAPACITY: usize = 64;
const REFRESH_INTERVAL: Duration = Duration::from_secs(3);

pub struct RuntimeBridge {
    command_tx: mpsc::Sender<BackendCommand>,
    event_rx: mpsc::Receiver<BackendEvent>,
}

impl RuntimeBridge {
    pub fn spawn(context: egui::Context) -> Self {
        let (command_tx, command_rx) = mpsc::channel(CHANNEL_CAPACITY);
        let (event_tx, event_rx) = mpsc::channel(CHANNEL_CAPACITY);

        thread::Builder::new()
            .name("bridgescope-backend".to_owned())
            .spawn(move || {
                let runtime = Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("Tokio runtime must initialize");
                runtime.block_on(run_backend(command_rx, event_tx, context));
            })
            .expect("backend thread must start");

        Self {
            command_tx,
            event_rx,
        }
    }

    pub fn try_send(&self, command: BackendCommand) -> Result<(), BridgeError> {
        self.command_tx.try_send(command).map_err(|error| {
            BridgeError::new(
                ErrorCode::Internal,
                "runtime.command_queue_full",
                error.to_string(),
            )
        })
    }

    pub fn drain(&mut self) -> Vec<BackendEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }
}

async fn run_backend(
    mut commands: mpsc::Receiver<BackendCommand>,
    events: mpsc::Sender<BackendEvent>,
    context: egui::Context,
) {
    let transport = initialize_transport(&events, &context).await;
    let mut registry = DeviceRegistry::default();
    let mut interval = tokio::time::interval(REFRESH_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                refresh_devices(transport.as_ref(), &mut registry, &events, &context).await;
            }
            command = commands.recv() => {
                let Some(command) = command else { break };
                match command {
                    BackendCommand::RefreshDevices => {
                        refresh_devices(transport.as_ref(), &mut registry, &events, &context).await;
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
                }
            }
        }
    }
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

async fn refresh_devices(
    transport: &dyn AdbTransport,
    registry: &mut DeviceRegistry,
    events: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
) {
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
        }
        Err(error) => send_event(events, context, BackendEvent::OperationFailed(error)).await,
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

async fn send_event(
    sender: &mpsc::Sender<BackendEvent>,
    context: &egui::Context,
    event: BackendEvent,
) {
    if sender.send(event).await.is_ok() {
        context.request_repaint();
    }
}

fn fake_backend_enabled() -> bool {
    std::env::var("BRIDGESCOPE_FAKE")
        .is_ok_and(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}
