use std::{collections::BTreeMap, path::Path, sync::Arc};

use async_trait::async_trait;
use bridgescope_adb::{AdbTransport, ShellOutputChunk, ShellSessionHandle, ShellStream};
use bridgescope_domain::{
    AdbEndpoint, BridgeError, DeviceCapabilities, DeviceDescriptor, DeviceOverview, DeviceSerial,
    DeviceState, ErrorCode, OverwritePolicy, PerformanceMetrics, ProcessInfo, RemoteFileEntry,
    RemoteFileKind, RemotePath, ShellSize,
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
}

#[derive(Clone, Debug)]
struct FakeDevice {
    descriptor: DeviceDescriptor,
    overview: DeviceOverview,
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
        Self {
            devices: Arc::new(RwLock::new(BTreeMap::from([(
                serial,
                FakeDevice {
                    descriptor,
                    overview,
                },
            )]))),
            files: Arc::new(RwLock::new(files)),
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
        match files.get(path) {
            Some(Some(_)) => {
                files.remove(path);
                Ok(())
            }
            Some(None) => Err(BridgeError::invalid_input("file.delete_not_regular_file")),
            None => Err(BridgeError::new(
                ErrorCode::PathNotFound,
                "file.path_not_found",
                path.to_string(),
            )),
        }
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
