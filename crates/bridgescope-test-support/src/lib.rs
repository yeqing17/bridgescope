use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use bridgescope_adb::{AdbTransport, ShellOutputChunk, ShellSessionHandle, ShellStream};
use bridgescope_domain::{
    BridgeError, DeviceCapabilities, DeviceDescriptor, DeviceOverview, DeviceSerial, DeviceState,
    ErrorCode,
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
        Self {
            devices: Arc::new(RwLock::new(BTreeMap::from([(
                serial,
                FakeDevice {
                    descriptor,
                    overview,
                },
            )]))),
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

    async fn start_shell(&self, serial: &DeviceSerial) -> Result<ShellSessionHandle, BridgeError> {
        self.online_device(serial).await?;
        Ok(ShellSessionHandle::from_handler(run_fake_shell))
    }

    async fn capture_screenshot(&self, serial: &DeviceSerial) -> Result<Vec<u8>, BridgeError> {
        self.online_device(serial).await?;
        fake_screenshot_png()
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
        assert!(fake.start_shell(&serial).await.is_err());
        assert!(fake.capture_screenshot(&serial).await.is_err());
    }

    #[tokio::test]
    async fn fake_shell_welcomes_echoes_interrupts_and_exits() {
        let fake = FakeAdbTransport::default();
        let mut shell = fake
            .start_shell(&fake_serial())
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
            .start_shell(&fake_serial())
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
