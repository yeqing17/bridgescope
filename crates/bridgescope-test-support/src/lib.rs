use std::{collections::BTreeMap, path::Path, sync::Arc};

use async_trait::async_trait;
use bridgescope_adb::{AdbTransport, ShellOutputChunk, ShellSessionHandle, ShellStream};
use bridgescope_domain::{
    AdbEndpoint, ApplicationDetails, ApplicationIconData, ApplicationRecord, BridgeError,
    DeviceCapabilities, DeviceDescriptor, DeviceOverview, DeviceSerial, DeviceState, DeviceTarget,
    ErrorCode, LayoutSnapshot, MdnsService, OverwritePolicy, PackageName, PerformanceMetrics,
    ProcessInfo, RemoteFileEntry, RemoteFileKind, RemotePath, ShellSize,
};
use image::{ImageEncoder, codecs::png::PngEncoder};
use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;

const FAKE_SHELL_WELCOME: &[u8] =
    b"\x1b[1;36mBridgeScope fake shell\x1b[0m\r\n\x1b[32mfake-device $\x1b[0m ";
const FAKE_SHELL_INTERRUPT: &[u8] = b"^C\r\n\x1b[32mfake-device $\x1b[0m ";

#[derive(Clone, Debug)]
pub struct FakeAdbTransport {
    devices: Arc<RwLock<BTreeMap<DeviceSerial, FakeDevice>>>,
    files: Arc<RwLock<BTreeMap<RemotePath, Option<Vec<u8>>>>>,
    applications: Arc<RwLock<BTreeMap<String, FakeApplication>>>,
}

#[derive(Clone, Debug)]
struct FakeDevice {
    descriptor: DeviceDescriptor,
    overview: DeviceOverview,
}

#[derive(Clone, Debug)]
struct FakeApplication {
    system: bool,
    disabled: bool,
}

impl Default for FakeAdbTransport {
    fn default() -> Self {
        let serial = DeviceSerial::new("FAKE-PIXEL-001").expect("constant fake serial is valid");
        let descriptor = DeviceDescriptor {
            serial: serial.clone(),
            state: DeviceState::Online,
            product: Some("bridgescope_sdk".to_owned()),
            model: Some("BridgeScope Pixel".to_owned()),
            device: Some("virtual_device".to_owned()),
            transport_id: Some(1),
        };
        let overview = DeviceOverview {
            serial: serial.clone(),
            model: descriptor.model.clone(),
            manufacturer: Some("BridgeScope Labs".to_owned()),
            android_version: Some("16".to_owned()),
            api_level: Some(36),
            abi: Some("x86_64".to_owned()),
            battery_percent: Some(84),
            memory_total_kib: Some(8 * 1024 * 1024),
            storage_total_kib: Some(128 * 1024 * 1024),
            storage_used_kib: Some(37 * 1024 * 1024),
            capabilities: DeviceCapabilities::basic_online(),
        };
        let files = BTreeMap::from([
            (RemotePath::new("/").expect("valid path"), None),
            (RemotePath::new("/sdcard").expect("valid path"), None),
            (
                RemotePath::new("/sdcard/Download").expect("valid path"),
                None,
            ),
            (
                RemotePath::new("/sdcard/Download/example.txt").expect("valid path"),
                Some(b"BridgeScope fake file\n".to_vec()),
            ),
            (
                RemotePath::new("/sdcard/示例 文件.txt").expect("valid path"),
                Some("Unicode file\n".as_bytes().to_vec()),
            ),
        ]);
        let application = |package: &str, system: bool, disabled: bool| {
            (package.to_owned(), FakeApplication { system, disabled })
        };
        let applications = BTreeMap::from([
            application("com.android.chrome", true, false),
            application("com.android.settings", true, false),
            application("com.google.android.gms", true, false),
            application("com.android.webview", true, true),
            application("com.bridgescope.demo", false, false),
            application("com.example.notes", false, false),
            application("com.example.podcast", false, true),
            application("org.mozilla.firefox", false, false),
            application("com.tencent.mm", false, false),
        ]);
        Self {
            devices: Arc::new(RwLock::new(BTreeMap::from([(
                serial,
                FakeDevice {
                    descriptor,
                    overview,
                },
            )]))),
            files: Arc::new(RwLock::new(files)),
            applications: Arc::new(RwLock::new(applications)),
        }
    }
}

impl FakeAdbTransport {
    pub async fn set_state(
        &self,
        serial: &DeviceSerial,
        state: DeviceState,
    ) -> Result<(), BridgeError> {
        let mut devices = self.devices.write().await;
        let device = devices.get_mut(serial).ok_or_else(|| {
            BridgeError::new(
                ErrorCode::DeviceNotFound,
                "device.not_found",
                serial.redacted(),
            )
        })?;
        device.descriptor.state = state;
        Ok(())
    }

    pub async fn add_device(&self, descriptor: DeviceDescriptor, overview: DeviceOverview) {
        self.devices.write().await.insert(
            descriptor.serial.clone(),
            FakeDevice {
                descriptor,
                overview,
            },
        );
    }

    pub async fn remove_device(&self, serial: &DeviceSerial) {
        self.devices.write().await.remove(serial);
    }

    async fn online_device(&self, serial: &DeviceSerial) -> Result<(), BridgeError> {
        let devices = self.devices.read().await;
        let device = devices.get(serial).ok_or_else(|| {
            BridgeError::new(
                ErrorCode::DeviceNotFound,
                "device.not_found",
                serial.redacted(),
            )
        })?;
        if device.descriptor.state.is_online() {
            Ok(())
        } else {
            Err(BridgeError::new(
                ErrorCode::DeviceUnavailable,
                "device.unavailable",
                serial.redacted(),
            ))
        }
    }

    /// A deterministic gradient tile so icon-grid layouts render stable
    /// fixtures; real transports decode the APK's launcher icon instead.
    #[allow(clippy::unused_self)]
    fn fake_icon(&self, package: &PackageName) -> ApplicationIconData {
        let hash = package
            .as_str()
            .bytes()
            .fold(2_166_136_261_u32, |acc, byte| {
                acc.wrapping_mul(16_777_619).wrapping_add(u32::from(byte))
            });
        let (width, height) = (48_u32, 48_u32);
        let hue_a = hash & 0xFF;
        let hue_b = (hash >> 8) & 0xFF;
        let mut rgba = Vec::with_capacity(usize::try_from(width * height).expect("fits") * 4);
        for row in 0..height {
            for column in 0..width {
                let blend = (row + column) * 255 / (width + height - 2);
                // Blend from hue_a toward hue_b without u32 underflow when
                // hue_b < hue_a.
                let value = if hue_b >= hue_a {
                    hue_a + (hue_b - hue_a) * blend / 255
                } else {
                    hue_a - (hue_a - hue_b) * blend / 255
                };
                rgba.push(u8::try_from(value).unwrap_or(128));
                rgba.push(u8::try_from(hue_b).unwrap_or(128));
                rgba.push(u8::try_from(255 - hue_a).unwrap_or(128));
                rgba.push(255);
            }
        }
        ApplicationIconData {
            width,
            height,
            rgba,
        }
    }

    /// The device must be online and the package installed for an action to
    /// succeed — mirroring what real `pm`/`am` calls would report.
    async fn require_application(
        &self,
        serial: &DeviceSerial,
        package: &PackageName,
    ) -> Result<(), BridgeError> {
        self.online_device(serial).await?;
        let applications = self.applications.read().await;
        if applications.contains_key(package.as_str()) {
            Ok(())
        } else {
            Err(BridgeError::new(
                ErrorCode::DeviceNotFound,
                "application.not_found",
                package.to_string(),
            ))
        }
    }
}

async fn run_fake_shell(
    mut input: mpsc::Receiver<Vec<u8>>,
    output: mpsc::Sender<ShellOutputChunk>,
    cancellation: CancellationToken,
) -> Result<Option<i32>, BridgeError> {
    send_shell_output(&output, FAKE_SHELL_WELCOME).await?;
    let mut command = Vec::new();
    loop {
        tokio::select! {
            () = cancellation.cancelled() => return Ok(Some(0)),
            bytes = input.recv() => {
                let Some(bytes) = bytes else { return Ok(Some(0)) };
                let mut segment_start = 0;
                let mut exit_requested = false;
                for (index, byte) in bytes.iter().copied().enumerate() {
                    if byte == 0x03 {
                        send_shell_output(&output, &bytes[segment_start..index]).await?;
                        send_shell_output(&output, FAKE_SHELL_INTERRUPT).await?;
                        command.clear();
                        segment_start = index + 1;
                    } else {
                        command.push(byte);
                        if matches!(byte, b'\r' | b'\n') {
                            exit_requested |= command
                                .strip_suffix(&[byte])
                                .is_some_and(|line| line == b"exit");
                            command.clear();
                        }
                    }
                }
                send_shell_output(&output, &bytes[segment_start..]).await?;
                if exit_requested {
                    return Ok(Some(0));
                }
            }
        }
    }
}

fn fake_screenshot_png() -> Result<Vec<u8>, BridgeError> {
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            &[0x24, 0x9d, 0xd8, 0xff],
            1,
            1,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| {
            BridgeError::new(
                ErrorCode::Internal,
                "fake.screenshot.encode_failed",
                error.to_string(),
            )
        })?;
    Ok(png)
}

async fn send_shell_output(
    output: &mpsc::Sender<ShellOutputChunk>,
    bytes: &[u8],
) -> Result<(), BridgeError> {
    if bytes.is_empty() {
        return Ok(());
    }
    output
        .send(ShellOutputChunk {
            stream: ShellStream::Stdout,
            bytes: bytes.to_vec(),
        })
        .await
        .map_err(|_| {
            BridgeError::new(
                ErrorCode::Internal,
                "fake.shell.output_closed",
                "fake shell output receiver closed",
            )
        })
}

#[async_trait]
impl AdbTransport for FakeAdbTransport {
    async fn version(&self) -> Result<String, BridgeError> {
        Ok("BridgeScope fake ADB 0.1".to_owned())
    }

    async fn list_devices(&self) -> Result<Vec<DeviceDescriptor>, BridgeError> {
        Ok(self
            .devices
            .read()
            .await
            .values()
            .map(|device| device.descriptor.clone())
            .collect())
    }

    async fn connect_endpoint(&self, endpoint: &AdbEndpoint) -> Result<String, BridgeError> {
        Ok(format!("connected to {}", endpoint.adb_target()))
    }

    async fn device_overview(&self, serial: &DeviceSerial) -> Result<DeviceOverview, BridgeError> {
        let devices = self.devices.read().await;
        let device = devices.get(serial).ok_or_else(|| {
            BridgeError::new(
                ErrorCode::DeviceNotFound,
                "device.not_found",
                serial.redacted(),
            )
        })?;
        if !device.descriptor.state.is_online() {
            return Err(BridgeError::new(
                ErrorCode::DeviceUnavailable,
                "device.unavailable",
                serial.redacted(),
            ));
        }
        Ok(device.overview.clone())
    }

    async fn list_processes(&self, serial: &DeviceSerial) -> Result<Vec<ProcessInfo>, BridgeError> {
        self.online_device(serial).await?;
        Ok(vec![
            ProcessInfo {
                pid: 1,
                name: "init".to_owned(),
                user: Some("root".to_owned()),
                state: Some("S".to_owned()),
                cpu_percent: Some(0.3),
                memory_percent: Some(0.2),
                resident_memory_kib: Some(4096),
            },
            ProcessInfo {
                pid: 4242,
                name: "com.bridgescope.fake".to_owned(),
                user: Some("u0_a123".to_owned()),
                state: Some("R".to_owned()),
                cpu_percent: Some(8.7),
                memory_percent: Some(1.4),
                resident_memory_kib: Some(32 * 1024),
            },
        ])
    }

    async fn list_applications(
        &self,
        serial: &DeviceSerial,
    ) -> Result<Vec<ApplicationRecord>, BridgeError> {
        self.online_device(serial).await?;
        let applications = self.applications.read().await;
        let mut records = applications
            .iter()
            .map(|(package, application)| ApplicationRecord {
                package: PackageName::new(package).expect("fake package names are valid"),
                system: application.system,
                disabled: application.disabled,
            })
            .collect::<Vec<_>>();
        records.sort_by(|left, right| {
            left.system
                .cmp(&right.system)
                .then_with(|| left.package.cmp(&right.package))
        });
        Ok(records)
    }

    async fn application_details(
        &self,
        serial: &DeviceSerial,
        package: &PackageName,
    ) -> Result<ApplicationDetails, BridgeError> {
        self.online_device(serial).await?;
        let applications = self.applications.read().await;
        let Some(application) = applications.get(package.as_str()) else {
            return Err(BridgeError::new(
                ErrorCode::DeviceNotFound,
                "application.not_found",
                package.to_string(),
            ));
        };
        Ok(ApplicationDetails {
            package: package.clone(),
            version_name: Some("1.4.2".to_owned()),
            version_code: Some(142_003),
            min_sdk: Some(24),
            target_sdk: Some(34),
            first_install_time: Some("2024-01-05 09:12:00".to_owned()),
            last_update_time: Some("2025-06-30 18:40:11".to_owned()),
            installer: (!application.system).then(|| "com.android.vending".to_owned()),
            apk_path: Some(if application.system {
                format!("/system/app/{}/base.apk", package.as_str())
            } else {
                format!("/data/app/~~demo/{}/base.apk", package.as_str())
            }),
            permissions: vec![
                "android.permission.INTERNET".to_owned(),
                "android.permission.FOREGROUND_SERVICE".to_owned(),
            ],
        })
    }

    async fn launch_application(
        &self,
        serial: &DeviceSerial,
        package: &PackageName,
    ) -> Result<(), BridgeError> {
        self.require_application(serial, package).await
    }

    async fn force_stop_application(
        &self,
        serial: &DeviceSerial,
        package: &PackageName,
    ) -> Result<(), BridgeError> {
        self.require_application(serial, package).await
    }

    async fn clear_application_data(
        &self,
        serial: &DeviceSerial,
        package: &PackageName,
    ) -> Result<(), BridgeError> {
        self.require_application(serial, package).await
    }

    async fn set_application_frozen(
        &self,
        serial: &DeviceSerial,
        package: &PackageName,
        frozen: bool,
    ) -> Result<(), BridgeError> {
        self.require_application(serial, package).await?;
        let mut applications = self.applications.write().await;
        if let Some(application) = applications.get_mut(package.as_str()) {
            application.disabled = frozen;
        }
        Ok(())
    }

    async fn uninstall_application(
        &self,
        serial: &DeviceSerial,
        package: &PackageName,
    ) -> Result<(), BridgeError> {
        self.require_application(serial, package).await?;
        self.applications.write().await.remove(package.as_str());
        Ok(())
    }

    async fn application_icon(
        &self,
        serial: &DeviceSerial,
        package: &PackageName,
    ) -> Result<Option<ApplicationIconData>, BridgeError> {
        self.require_application(serial, package).await?;
        Ok(Some(self.fake_icon(package)))
    }

    async fn performance_metrics(
        &self,
        serial: &DeviceSerial,
    ) -> Result<PerformanceMetrics, BridgeError> {
        self.online_device(serial).await?;
        Ok(PerformanceMetrics {
            cpu_usage_percent: Some(23.0),
            load_average_1m: Some(0.42),
            memory_total_kib: Some(8 * 1024 * 1024),
            memory_available_kib: Some(5 * 1024 * 1024),
            storage_total_kib: Some(128 * 1024 * 1024),
            storage_used_kib: Some(37 * 1024 * 1024),
            battery_percent: Some(84),
        })
    }

    async fn start_shell(
        &self,
        serial: &DeviceSerial,
        _size: ShellSize,
    ) -> Result<ShellSessionHandle, BridgeError> {
        self.online_device(serial).await?;
        Ok(ShellSessionHandle::from_handler(run_fake_shell))
    }

    async fn capture_screenshot(&self, serial: &DeviceSerial) -> Result<Vec<u8>, BridgeError> {
        self.online_device(serial).await?;
        fake_screenshot_png()
    }

    async fn list_directory(
        &self,
        serial: &DeviceSerial,
        path: &RemotePath,
    ) -> Result<Vec<RemoteFileEntry>, BridgeError> {
        self.online_device(serial).await?;
        let files = self.files.read().await;
        if !files.contains_key(path) {
            return Err(BridgeError::new(
                ErrorCode::PathNotFound,
                "file.path_not_found",
                path.to_string(),
            ));
        }
        if files.get(path).and_then(Option::as_ref).is_some() {
            return Err(BridgeError::invalid_input("file.not_directory"));
        }
        let prefix = if path.as_str() == "/" {
            "/".to_owned()
        } else {
            format!("{}/", path.as_str())
        };
        let mut entries = Vec::new();
        for (entry_path, content) in files.iter() {
            let rest = entry_path
                .as_str()
                .strip_prefix(&prefix)
                .unwrap_or_default();
            if rest.is_empty() || rest.contains('/') {
                continue;
            }
            entries.push(RemoteFileEntry {
                path: entry_path.clone(),
                name: rest.to_owned(),
                kind: if content.is_some() {
                    RemoteFileKind::File
                } else {
                    RemoteFileKind::Directory
                },
                size_bytes: content.as_ref().map(|bytes| bytes.len() as u64),
                modified_unix_seconds: None,
                permissions: None,
            });
        }
        Ok(entries)
    }

    async fn push_file(
        &self,
        serial: &DeviceSerial,
        local_path: &Path,
        remote_path: &RemotePath,
        overwrite: OverwritePolicy,
        cancellation: CancellationToken,
    ) -> Result<(), BridgeError> {
        if cancellation.is_cancelled() {
            return Err(BridgeError::new(
                ErrorCode::Cancelled,
                "file.cancelled",
                "file operation cancelled",
            ));
        }
        self.online_device(serial).await?;
        let bytes = tokio::fs::read(local_path).await.map_err(|error| {
            BridgeError::new(
                ErrorCode::PathNotFound,
                "file.local_source_missing",
                error.to_string(),
            )
        })?;
        let mut files = self.files.write().await;
        if files.get(remote_path).is_some_and(Option::is_none) {
            return Err(BridgeError::invalid_input("file.remote_is_directory"));
        }
        if files.contains_key(remote_path) && overwrite == OverwritePolicy::Deny {
            return Err(BridgeError::new(
                ErrorCode::AlreadyExists,
                "file.remote_exists",
                remote_path.to_string(),
            ));
        }
        files.insert(remote_path.clone(), Some(bytes));
        Ok(())
    }

    async fn pull_file(
        &self,
        serial: &DeviceSerial,
        remote_path: &RemotePath,
        local_path: &Path,
        overwrite: OverwritePolicy,
        cancellation: CancellationToken,
    ) -> Result<(), BridgeError> {
        if cancellation.is_cancelled() {
            return Err(BridgeError::new(
                ErrorCode::Cancelled,
                "file.cancelled",
                "file operation cancelled",
            ));
        }
        self.online_device(serial).await?;
        let files = self.files.read().await;
        let Some(content) = files.get(remote_path).and_then(Option::as_ref) else {
            return Err(BridgeError::new(
                ErrorCode::PathNotFound,
                "file.remote_not_file",
                remote_path.to_string(),
            ));
        };
        if tokio::fs::try_exists(local_path).await.unwrap_or(false)
            && overwrite == OverwritePolicy::Deny
        {
            return Err(BridgeError::new(
                ErrorCode::AlreadyExists,
                "file.local_exists",
                local_path.display().to_string(),
            ));
        }
        tokio::fs::write(local_path, content)
            .await
            .map_err(|error| {
                BridgeError::new(
                    ErrorCode::AdbFailed,
                    "file.local_write_failed",
                    error.to_string(),
                )
            })
    }

    async fn create_directory(
        &self,
        serial: &DeviceSerial,
        path: &RemotePath,
    ) -> Result<(), BridgeError> {
        self.online_device(serial).await?;
        let mut files = self.files.write().await;
        if files.contains_key(path) {
            return Err(BridgeError::new(
                ErrorCode::AlreadyExists,
                "file.remote_exists",
                path.to_string(),
            ));
        }
        if !files.get(&path.parent()).is_some_and(Option::is_none) {
            return Err(BridgeError::new(
                ErrorCode::PathNotFound,
                "file.parent_missing",
                path.parent().to_string(),
            ));
        }
        files.insert(path.clone(), None);
        Ok(())
    }

    async fn rename_entry(
        &self,
        serial: &DeviceSerial,
        source: &RemotePath,
        destination: &RemotePath,
    ) -> Result<(), BridgeError> {
        self.online_device(serial).await?;
        if source.as_str() == "/" || source == destination {
            return Err(BridgeError::invalid_input("file.rename_invalid"));
        }
        let mut files = self.files.write().await;
        if !files.contains_key(source) {
            return Err(BridgeError::new(
                ErrorCode::PathNotFound,
                "file.path_not_found",
                source.to_string(),
            ));
        }
        if files.contains_key(destination) {
            return Err(BridgeError::new(
                ErrorCode::AlreadyExists,
                "file.remote_exists",
                destination.to_string(),
            ));
        }
        if !files
            .get(&destination.parent())
            .is_some_and(Option::is_none)
        {
            return Err(BridgeError::new(
                ErrorCode::PathNotFound,
                "file.parent_missing",
                destination.parent().to_string(),
            ));
        }
        let prefix = format!("{}/", source.as_str());
        let moved = files
            .iter()
            .filter(|(path, _)| *path == source || path.as_str().starts_with(&prefix))
            .map(|(path, content)| (path.clone(), content.clone()))
            .collect::<Vec<_>>();
        for (path, _) in &moved {
            files.remove(path);
        }
        for (path, content) in moved {
            let suffix = path
                .as_str()
                .strip_prefix(source.as_str())
                .unwrap_or_default();
            let new_path = RemotePath::new(format!("{}{suffix}", destination.as_str()))?;
            files.insert(new_path, content);
        }
        Ok(())
    }

    async fn delete_file(
        &self,
        serial: &DeviceSerial,
        path: &RemotePath,
    ) -> Result<(), BridgeError> {
        self.online_device(serial).await?;
        let mut files = self.files.write().await;
        let directory_prefix = format!("{}/", path.as_str().trim_end_matches('/'));
        match files.get(path) {
            Some(Some(_)) => {
                files.remove(path);
                Ok(())
            }
            Some(None) => {
                files.retain(|entry, _| {
                    entry != path && !entry.as_str().starts_with(&directory_prefix)
                });
                files.remove(path);
                Ok(())
            }
            None => Err(BridgeError::new(
                ErrorCode::PathNotFound,
                "file.path_not_found",
                path.to_string(),
            )),
        }
    }

    async fn start_logcat(&self, serial: &DeviceSerial) -> Result<ShellSessionHandle, BridgeError> {
        self.online_device(serial).await?;
        Ok(ShellSessionHandle::from_handler(
            |mut input: mpsc::Receiver<Vec<u8>>,
             output: mpsc::Sender<ShellOutputChunk>,
             cancellation: CancellationToken| async move {
                // A canned "recent log" line set per severity, then the stream
                // idles until cancelled — mirroring a real logcat session.
                for line in [
                    "08-29 10:00:01.100  1000  1000 I ActivityTaskManager: Displayed com.example/.Main",
                    "08-29 10:00:01.200  1000  1001 D BluetoothAdapter: isLeEnabled(): true",
                    "08-29 10:00:01.300 10042 10042 W View: requestLayout() improperly called",
                    "08-29 10:00:01.400  3193  3250 E AndroidRuntime: FATAL EXCEPTION: main",
                    "08-29 10:00:01.500  3193  3193 V FakeTag: verbose line",
                ] {
                    tokio::select! {
                        () = cancellation.cancelled() => return Ok(Some(0)),
                        result = send_shell_output(&output, line.as_bytes()) => result?,
                        () = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
                    }
                }
                loop {
                    tokio::select! {
                        () = cancellation.cancelled() => return Ok(Some(0)),
                        bytes = input.recv() => {
                            if bytes.is_none() {
                                return Ok(Some(0));
                            }
                        }
                    }
                }
            },
        ))
    }

    async fn dump_layout(&self, serial: &DeviceSerial) -> Result<LayoutSnapshot, BridgeError> {
        self.online_device(serial).await?;
        let xml = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy rotation="0">
  <node index="0" text="" resource-id="" class="android.widget.FrameLayout" package="com.example" bounds="[0,0][1080,2400]">
    <node index="1" text="BridgeScope" resource-id="com.example:id/title" class="android.widget.TextView" package="com.example" clickable="true" enabled="true" bounds="[24,96][540,192]" />
    <node index="2" text="" resource-id="" class="android.widget.Button" package="com.example" clickable="true" enabled="false" bounds="[24,200][1056,296]" />
  </node>
</hierarchy>"#;
        let root = bridgescope_adb::parse_hierarchy(xml).map_err(|error| {
            BridgeError::new(
                ErrorCode::Internal,
                "layout.parse_failed",
                error.to_string(),
            )
        })?;
        Ok(LayoutSnapshot {
            target: DeviceTarget::new(serial.clone(), 1),
            root,
            raw_xml: xml.to_owned(),
            captured_at_unix_seconds: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        })
    }

    async fn list_webview_sockets(
        &self,
        serial: &DeviceSerial,
    ) -> Result<Vec<String>, BridgeError> {
        self.online_device(serial).await?;
        Ok(vec!["com.android.chrome_devtools_remote".to_owned()])
    }

    async fn forward_port(
        &self,
        serial: &DeviceSerial,
        _port: u16,
        _socket: &str,
    ) -> Result<(), BridgeError> {
        self.online_device(serial).await
    }

    async fn remove_forward(&self, serial: &DeviceSerial, _port: u16) -> Result<(), BridgeError> {
        self.online_device(serial).await
    }

    async fn install_apk(
        &self,
        serial: &DeviceSerial,
        _apk_path: &std::path::Path,
    ) -> Result<(), BridgeError> {
        self.online_device(serial).await
    }

    async fn list_avds(&self) -> Result<Vec<String>, BridgeError> {
        Ok(vec!["Fake_Pixel_9a".to_owned(), "Fake_Tablet".to_owned()])
    }

    async fn launch_avd(&self, _name: &str, _wipe_data: bool) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn running_avd_name(&self, serial: &DeviceSerial) -> Result<Option<String>, BridgeError> {
        self.online_device(serial).await?;
        Ok((serial.as_str().starts_with("emulator-")).then(|| "Fake_Pixel_9a".to_owned()))
    }

    async fn kill_emulator(&self, serial: &DeviceSerial) -> Result<(), BridgeError> {
        self.online_device(serial).await
    }

    async fn pair_device(&self, host: &str, _port: u16, code: &str) -> Result<(), BridgeError> {
        if host.trim().is_empty() || !(6..=8).contains(&code.len()) {
            return Err(BridgeError::invalid_input("wireless.pair_invalid"));
        }
        Ok(())
    }

    async fn enable_tcpip(&self, serial: &DeviceSerial, _port: u16) -> Result<(), BridgeError> {
        self.online_device(serial).await
    }

    async fn mdns_services(&self) -> Result<Vec<MdnsService>, BridgeError> {
        Ok(vec![MdnsService {
            name: "Fake_Pixel_9a".to_owned(),
            service_type: "_adb-tls-connect._tcp".to_owned(),
            address: "192.168.1.20:5555".to_owned(),
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_supports_state_changes() {
        let fake = FakeAdbTransport::default();
        let serial = fake_serial();
        fake.set_state(&serial, DeviceState::Unauthorized)
            .await
            .expect("state change succeeds");
        let devices = fake.list_devices().await.expect("list succeeds");
        assert_eq!(devices[0].state, DeviceState::Unauthorized);
        assert!(fake.device_overview(&serial).await.is_err());
        assert!(
            fake.start_shell(
                &serial,
                ShellSize::new(80, 24).expect("valid terminal size")
            )
            .await
            .is_err()
        );
        assert!(fake.capture_screenshot(&serial).await.is_err());
    }

    #[tokio::test]
    async fn fake_applications_support_listing_and_actions() {
        let fake = FakeAdbTransport::default();
        let serial = fake_serial();

        let apps = fake
            .list_applications(&serial)
            .await
            .expect("list succeeds");
        assert!(
            apps.iter()
                .any(|app| app.package.as_str() == "com.example.notes" && !app.system)
        );
        assert!(apps.iter().any(|app| app.system));
        assert!(
            apps.iter()
                .any(|app| app.package.as_str() == "com.example.podcast" && app.disabled)
        );

        let package = PackageName::new("com.example.podcast").expect("valid package");
        fake.set_application_frozen(&serial, &package, false)
            .await
            .expect("unfreeze succeeds");
        let apps = fake
            .list_applications(&serial)
            .await
            .expect("relist succeeds");
        let podcast = apps
            .iter()
            .find(|app| app.package == package)
            .expect("package still listed");
        assert!(!podcast.disabled);

        let details = fake
            .application_details(&serial, &package)
            .await
            .expect("details succeed");
        assert_eq!(details.package, package);
        assert_eq!(details.version_name.as_deref(), Some("1.4.2"));

        let icon = fake
            .application_icon(&serial, &package)
            .await
            .expect("icon succeeds");
        let icon = icon.expect("fake icon present");
        assert_eq!((icon.width, icon.height), (48, 48));
        assert_eq!(icon.rgba.len(), 48 * 48 * 4);
        let again = fake
            .application_icon(&serial, &package)
            .await
            .expect("icon succeeds")
            .expect("fake icon present");
        assert_eq!(icon.rgba, again.rgba);

        fake.uninstall_application(&serial, &package)
            .await
            .expect("uninstall succeeds");
        assert!(fake.application_icon(&serial, &package).await.is_err());
        assert!(fake.application_details(&serial, &package).await.is_err());
        assert!(fake.launch_application(&serial, &package).await.is_err());
    }

    #[tokio::test]
    async fn fake_shell_welcomes_echoes_interrupts_and_exits() {
        let fake = FakeAdbTransport::default();
        let mut shell = fake
            .start_shell(
                &fake_serial(),
                ShellSize::new(80, 24).expect("valid terminal size"),
            )
            .await
            .expect("shell starts");

        assert_eq!(next_output(&mut shell).await, FAKE_SHELL_WELCOME);
        shell
            .input()
            .send(b"hello".to_vec())
            .await
            .expect("input accepted");
        assert_eq!(next_output(&mut shell).await, b"hello");
        shell
            .input()
            .send(vec![0x03])
            .await
            .expect("interrupt accepted");
        assert_eq!(next_output(&mut shell).await, FAKE_SHELL_INTERRUPT);
        shell
            .input()
            .send(b"exit\n".to_vec())
            .await
            .expect("exit accepted");
        assert_eq!(next_output(&mut shell).await, b"exit\n");
        assert_eq!(shell.close().await.expect("shell closes"), Some(0));
    }

    #[tokio::test]
    async fn fake_shell_close_stops_persistent_session() {
        let fake = FakeAdbTransport::default();
        let mut shell = fake
            .start_shell(
                &fake_serial(),
                ShellSize::new(80, 24).expect("valid terminal size"),
            )
            .await
            .expect("shell starts");
        assert_eq!(next_output(&mut shell).await, FAKE_SHELL_WELCOME);
        assert_eq!(shell.close().await.expect("shell closes"), Some(0));
    }

    #[tokio::test]
    async fn fake_screenshot_is_a_decodable_png() {
        let fake = FakeAdbTransport::default();
        let png = fake
            .capture_screenshot(&fake_serial())
            .await
            .expect("screenshot succeeds");
        assert_eq!(
            png,
            fake.capture_screenshot(&fake_serial())
                .await
                .expect("second screenshot succeeds")
        );
        let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .expect("screenshot is valid PNG");
        assert_eq!((image.width(), image.height()), (1, 1));
    }

    fn fake_serial() -> DeviceSerial {
        DeviceSerial::new("FAKE-PIXEL-001").expect("valid serial")
    }

    async fn next_output(shell: &mut ShellSessionHandle) -> Vec<u8> {
        let chunk = shell.output_mut().recv().await.expect("shell emits output");
        assert_eq!(chunk.stream, ShellStream::Stdout);
        chunk.bytes
    }
}
