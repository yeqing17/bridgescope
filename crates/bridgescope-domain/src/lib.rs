use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct DeviceSerial(String);

impl DeviceSerial {
    pub fn new(value: impl Into<String>) -> Result<Self, BridgeError> {
        let value = value.into();
        if value.trim().is_empty() || value.contains(['\0', '\n', '\r']) {
            return Err(BridgeError::invalid_input("device.serial.invalid"));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn redacted(&self) -> String {
        let visible = self.0.chars().rev().take(4).collect::<Vec<_>>();
        let suffix = visible.into_iter().rev().collect::<String>();
        format!("device-…{suffix}")
    }
}

impl fmt::Display for DeviceSerial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    Online,
    Offline,
    Unauthorized,
    Unknown,
}

impl DeviceState {
    #[must_use]
    pub fn is_online(self) -> bool {
        self == Self::Online
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeviceDescriptor {
    pub serial: DeviceSerial,
    pub state: DeviceState,
    pub product: Option<String>,
    pub model: Option<String>,
    pub device: Option<String>,
    pub transport_id: Option<u64>,
}

impl DeviceDescriptor {
    #[must_use]
    pub fn display_name(&self) -> &str {
        self.model.as_deref().unwrap_or(self.serial.as_str())
    }
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeviceCapabilities {
    pub shell: bool,
    pub files: bool,
    pub applications: bool,
    pub screenshots: bool,
    pub logcat: bool,
}

impl DeviceCapabilities {
    #[must_use]
    pub fn basic_online() -> Self {
        Self {
            shell: true,
            files: true,
            applications: true,
            screenshots: true,
            logcat: true,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeviceOverview {
    pub serial: DeviceSerial,
    pub model: Option<String>,
    pub manufacturer: Option<String>,
    pub android_version: Option<String>,
    pub api_level: Option<u32>,
    pub abi: Option<String>,
    pub battery_percent: Option<u8>,
    pub memory_total_kib: Option<u64>,
    pub storage_total_kib: Option<u64>,
    pub storage_used_kib: Option<u64>,
    pub capabilities: DeviceCapabilities,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeviceRecord {
    pub descriptor: DeviceDescriptor,
    pub generation: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeviceSnapshot {
    pub devices: Vec<DeviceRecord>,
    pub selected: Option<DeviceSerial>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationRisk {
    ReadOnly,
    Mutating,
    Destructive,
    Privileged,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OperationId(Uuid);

impl OperationId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for OperationId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    AdbNotFound,
    AdbFailed,
    DeviceNotFound,
    DeviceUnavailable,
    InvalidInput,
    TimedOut,
    OutputLimit,
    Internal,
}

#[derive(Clone, Debug, Error, Deserialize, Eq, PartialEq, Serialize)]
#[error("{message_key}: {detail}")]
pub struct BridgeError {
    pub code: ErrorCode,
    pub message_key: String,
    pub detail: String,
}

impl BridgeError {
    pub fn new(code: ErrorCode, message_key: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code,
            message_key: message_key.into(),
            detail: detail.into(),
        }
    }

    pub fn invalid_input(message_key: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message_key, "invalid input")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendCommand {
    RefreshDevices,
    SelectDevice(Option<DeviceSerial>),
    LoadOverview(DeviceSerial),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendEvent {
    AdbReady { path: String, version: String },
    AdbUnavailable(BridgeError),
    DevicesChanged(DeviceSnapshot),
    OverviewLoading(DeviceSerial),
    OverviewLoaded(DeviceOverview),
    OperationFailed(BridgeError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_rejects_control_characters() {
        assert!(DeviceSerial::new("serial\nother").is_err());
        assert!(DeviceSerial::new("").is_err());
    }

    #[test]
    fn serial_redaction_keeps_only_suffix() {
        let serial = DeviceSerial::new("ABCDEF123456").expect("valid serial");
        assert_eq!(serial.redacted(), "device-…3456");
    }

    #[test]
    fn descriptor_falls_back_to_serial_for_display() {
        let descriptor = DeviceDescriptor {
            serial: DeviceSerial::new("emulator-5554").expect("valid serial"),
            state: DeviceState::Online,
            product: None,
            model: None,
            device: None,
            transport_id: None,
        };
        assert_eq!(descriptor.display_name(), "emulator-5554");
    }
}
