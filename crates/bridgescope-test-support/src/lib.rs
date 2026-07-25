use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use bridgescope_adb::AdbTransport;
use bridgescope_domain::{
    BridgeError, DeviceCapabilities, DeviceDescriptor, DeviceOverview, DeviceSerial, DeviceState,
    ErrorCode,
};
use tokio::sync::RwLock;

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_supports_state_changes() {
        let fake = FakeAdbTransport::default();
        let serial = DeviceSerial::new("FAKE-PIXEL-001").expect("valid serial");
        fake.set_state(&serial, DeviceState::Unauthorized)
            .await
            .expect("state change succeeds");
        let devices = fake.list_devices().await.expect("list succeeds");
        assert_eq!(devices[0].state, DeviceState::Unauthorized);
        assert!(fake.device_overview(&serial).await.is_err());
    }
}
