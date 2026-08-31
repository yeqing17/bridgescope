use std::{fmt, net::IpAddr, path::PathBuf, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const MAX_SHELL_INPUT_BYTES: usize = 64 * 1024;
const MAX_REMOTE_PATH_BYTES: usize = 4096;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RemotePath(String);

impl RemotePath {
    pub fn new(value: impl Into<String>) -> Result<Self, BridgeError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_REMOTE_PATH_BYTES
            || !value.starts_with('/')
            || value.contains(['\0', '\r', '\n'])
        {
            return Err(BridgeError::invalid_input("file.path.invalid"));
        }
        let mut components = Vec::new();
        for component in value.split('/') {
            match component {
                "" | "." => {}
                ".." => {
                    if components.pop().is_none() {
                        return Err(BridgeError::invalid_input("file.path.escapes_root"));
                    }
                }
                component if component.chars().any(char::is_control) => {
                    return Err(BridgeError::invalid_input("file.path.invalid"));
                }
                component => components.push(component),
            }
        }
        let normalized = if components.is_empty() {
            "/".to_owned()
        } else {
            format!("/{}", components.join("/"))
        };
        Ok(Self(normalized))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn parent(&self) -> Self {
        if self.0 == "/" {
            return Self("/".to_owned());
        }
        let parent =
            self.0.rsplit_once('/').map_or(
                "/",
                |(prefix, _)| if prefix.is_empty() { "/" } else { prefix },
            );
        Self(parent.to_owned())
    }

    pub fn join_component(&self, component: &str) -> Result<Self, BridgeError> {
        if component.is_empty() || component.contains('/') || component == "." || component == ".."
        {
            return Err(BridgeError::invalid_input("file.path.component_invalid"));
        }
        Self::new(format!("{}/{}", self.0.trim_end_matches('/'), component))
    }

    #[must_use]
    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or("")
    }
}

impl fmt::Display for RemotePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RemotePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteFileKind {
    Directory,
    File,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RemoteFileEntry {
    pub path: RemotePath,
    pub name: String,
    pub kind: RemoteFileKind,
    pub size_bytes: Option<u64>,
    pub modified_unix_seconds: Option<i64>,
    pub permissions: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DirectoryListing {
    pub target: DeviceTarget,
    pub directory: RemotePath,
    pub entries: Vec<RemoteFileEntry>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileTransferDirection {
    Upload,
    Download,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverwritePolicy {
    Deny,
    ReplaceConfirmed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FileTransferSummary {
    pub direction: FileTransferDirection,
    pub target: DeviceTarget,
    pub remote_path: RemotePath,
    pub local_path: PathBuf,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteFileMutationKind {
    CreateDirectory,
    Rename,
    DeleteFile,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RemoteFileMutationSummary {
    pub kind: RemoteFileMutationKind,
    pub target: DeviceTarget,
    pub path: RemotePath,
    pub destination: Option<RemotePath>,
}

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

/// A host and TCP port accepted by adb connect.
///
/// Hosts may be IPv4/IPv6 addresses or DNS names. Delimiters and whitespace
/// are rejected because the value is passed as one positional ADB argument.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct AdbEndpoint {
    host: String,
    port: u16,
}

impl AdbEndpoint {
    pub fn new(host: impl Into<String>, port: u16) -> Result<Self, BridgeError> {
        let raw_host = host.into();
        let host = raw_host.trim().to_owned();
        if host.is_empty()
            || raw_host != host
            || port == 0
            || host
                .chars()
                .any(|character| character.is_ascii_control() || character.is_whitespace())
            || host.contains(['[', ']', '/', '\\'])
            || (host.contains(':') && host.parse::<IpAddr>().is_err())
        {
            return Err(BridgeError::invalid_input("adb.endpoint.invalid"));
        }
        Ok(Self { host, port })
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Formats the endpoint as the single argument expected by adb connect.
    #[must_use]
    pub fn adb_target(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// Parses a `host:port` target string, e.g. a network device serial.
    ///
    /// Returns `None` when the port is not a valid `u16` or the host is
    /// rejected by `AdbEndpoint::new`.
    #[must_use]
    pub fn parse_target(host: &str, port: &str) -> Option<Self> {
        let port = u16::from_str(port).ok()?;
        Self::new(host, port).ok()
    }
}

impl fmt::Display for AdbEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.adb_target())
    }
}

/// Identifies one observed lifetime of a device connection.
///
/// The generation prevents work started for an older connection from being
/// delivered to a device which later reused the same serial number.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct DeviceTarget {
    pub serial: DeviceSerial,
    pub generation: u64,
}

impl DeviceTarget {
    #[must_use]
    pub const fn new(serial: DeviceSerial, generation: u64) -> Self {
        Self { serial, generation }
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
    /// `ro.product.brand` — marketing brand, often but not always the
    /// manufacturer in different casing.
    pub brand: Option<String>,
    /// Kernel release from `uname -r` (e.g. `5.15.178-android13`).
    pub kernel_version: Option<String>,
    /// SoC / board name: `ro.soc.model`, falling back to `ro.board.platform`
    /// then `ro.hardware`.
    pub soc: Option<String>,
    pub cpu_cores: Option<u32>,
    /// Physical display size from `wm size` (e.g. `1080x2400`).
    pub screen_physical: Option<String>,
    /// Physical display density from `wm density` (e.g. `440`).
    pub screen_density: Option<String>,
    /// Override display size from `wm size`, present only when the user or a
    /// developer setting changed the resolution.
    pub screen_override: Option<String>,
    /// System font scale from `settings get system font_scale` (e.g. `1.0`).
    pub font_scale: Option<String>,
    pub wifi_ssid: Option<String>,
    pub ip_address: Option<String>,
    pub mac_address: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeviceRecord {
    pub descriptor: DeviceDescriptor,
    pub generation: u64,
}

impl DeviceRecord {
    #[must_use]
    pub fn target(&self) -> DeviceTarget {
        DeviceTarget::new(self.descriptor.serial.clone(), self.generation)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeviceSnapshot {
    pub devices: Vec<DeviceRecord>,
    pub selected: Option<DeviceSerial>,
}

/// An Android package name (e.g. `com.example.app`).
///
/// The allow-list (ASCII letters, digits, underscores, dots) mirrors the Java
/// package rules Android enforces and doubles as the injection guard: the
/// value is embedded as one argument of an `adb shell pm …` command, so any
/// whitespace, quote, or shell metacharacter is impossible by construction.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct PackageName(String);

const MAX_PACKAGE_NAME_BYTES: usize = 255;

impl PackageName {
    pub fn new(value: impl Into<String>) -> Result<Self, BridgeError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= MAX_PACKAGE_NAME_BYTES
            && value.starts_with(|character: char| character.is_ascii_alphanumeric())
            && value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '.')
            });
        if !valid {
            return Err(BridgeError::invalid_input("application.package.invalid"));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackageName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// One installed application as reported by the package manager.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApplicationRecord {
    pub package: PackageName,
    pub system: bool,
    pub disabled: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ApplicationSnapshot {
    pub target: DeviceTarget,
    pub applications: Vec<ApplicationRecord>,
}

/// One wireless-debugging service advertised over mDNS, as reported by
/// `adb mdns services`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MdnsService {
    pub name: String,
    pub service_type: String,
    pub address: String,
}

/// Best-effort details for one application, parsed from `dumpsys package`.
///
/// No `Default`: an empty [`PackageName`] would be invalid by construction.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ApplicationDetails {
    pub package: PackageName,
    pub version_name: Option<String>,
    pub version_code: Option<u64>,
    pub min_sdk: Option<u32>,
    pub target_sdk: Option<u32>,
    pub first_install_time: Option<String>,
    pub last_update_time: Option<String>,
    pub installer: Option<String>,
    pub apk_path: Option<String>,
    pub permissions: Vec<String>,
}

/// The actions Fadb can perform on one installed application.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationAction {
    Launch,
    ForceStop,
    ClearData,
    Freeze,
    Unfreeze,
    Uninstall,
}

impl ApplicationAction {
    /// True when completing the action changes the package list, so the
    /// panel should refresh its snapshot afterwards.
    #[must_use]
    pub fn mutates_listing(self) -> bool {
        matches!(self, Self::Freeze | Self::Unfreeze | Self::Uninstall)
    }
}

/// Decoded launcher-icon pixels for one application.
///
/// The transport extracts the icon from the installed APK and decodes it;
/// only ready-to-upload RGBA travels toward the UI.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ApplicationIconData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub user: Option<String>,
    pub state: Option<String>,
    pub cpu_percent: Option<f32>,
    pub memory_percent: Option<f32>,
    pub resident_memory_kib: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProcessSnapshot {
    pub target: DeviceTarget,
    pub processes: Vec<ProcessInfo>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PerformanceMetrics {
    pub cpu_usage_percent: Option<f32>,
    pub load_average_1m: Option<f32>,
    pub memory_total_kib: Option<u64>,
    pub memory_available_kib: Option<u64>,
    pub storage_total_kib: Option<u64>,
    pub storage_used_kib: Option<u64>,
    pub battery_percent: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PerformanceSnapshot {
    pub target: DeviceTarget,
    pub metrics: PerformanceMetrics,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationRisk {
    ReadOnly,
    Mutating,
    Destructive,
    Privileged,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
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

/// Correlates every command and event belonging to one persistent shell.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ShellSessionId(Uuid);

impl ShellSessionId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for ShellSessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ShellSessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Identifier of one live logcat stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct LogcatSessionId(Uuid);

impl LogcatSessionId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for LogcatSessionId {
    fn default() -> Self {
        Self::new()
    }
}

/// One parsed node of the foreground window hierarchy.
// The booleans mirror the `uiautomator` dump attributes one to one.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct LayoutNode {
    /// Stable pre-order index assigned during parsing, used as the UI
    /// selection key.
    pub id: usize,
    pub class: String,
    pub resource_id: String,
    pub text: String,
    pub content_description: String,
    /// `[x, y, width, height]` in screen pixels.
    pub bounds: [i32; 4],
    pub clickable: bool,
    pub scrollable: bool,
    pub enabled: bool,
    pub selected: bool,
    pub focused: bool,
    pub package: String,
    pub children: Vec<LayoutNode>,
}

impl LayoutNode {
    /// Depth-first node count including `self`.
    #[must_use]
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(Self::count).sum::<usize>()
    }

    /// Collect every node (self first) matching `predicate`.
    pub fn find_all<'a>(
        &'a self,
        predicate: &dyn Fn(&LayoutNode) -> bool,
        out: &mut Vec<&'a Self>,
    ) {
        if predicate(self) {
            out.push(self);
        }
        for child in &self.children {
            child.find_all(predicate, out);
        }
    }
}

/// Parsed `uiautomator dump` result for one capture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayoutSnapshot {
    pub target: DeviceTarget,
    pub root: LayoutNode,
    /// The untouched hierarchy XML, kept for export.
    pub raw_xml: String,
    pub captured_at_unix_seconds: u64,
}

/// One inspectable page exposed by a WebView's devtools socket.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebViewPage {
    pub title: String,
    pub url: String,
    /// `page`, `webview`, `service_worker`, … as reported by CDP.
    pub kind: String,
    /// `webSocketDebuggerUrl` from `/json/list`.
    pub debugger_url: String,
}

/// One bounded, non-empty byte chunk written to an interactive shell.
///
/// The bytes are intentionally not UTF-8 text: terminals must preserve escape
/// sequences, control characters, and arbitrary pasted input without loss.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ShellInput(Vec<u8>);

impl ShellInput {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, BridgeError> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(BridgeError::invalid_input("shell.input.empty"));
        }
        if bytes.len() > MAX_SHELL_INPUT_BYTES {
            return Err(BridgeError::new(
                ErrorCode::OutputLimit,
                "shell.input.too_large",
                format!("maximum input chunk is {MAX_SHELL_INPUT_BYTES} bytes"),
            ));
        }
        Ok(Self(bytes))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl<'de> Deserialize<'de> for ShellInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        Self::new(bytes).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ShellSize {
    columns: u16,
    rows: u16,
}

impl ShellSize {
    pub fn new(columns: u16, rows: u16) -> Result<Self, BridgeError> {
        if columns == 0 || rows == 0 {
            return Err(BridgeError::invalid_input("shell.size.invalid"));
        }
        Ok(Self { columns, rows })
    }

    #[must_use]
    pub const fn columns(self) -> u16 {
        self.columns
    }

    #[must_use]
    pub const fn rows(self) -> u16 {
        self.rows
    }
}

impl<'de> Deserialize<'de> for ShellSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawShellSize {
            columns: u16,
            rows: u16,
        }

        let raw = RawShellSize::deserialize(deserializer)?;
        Self::new(raw.columns, raw.rows).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotFormat {
    DecodedRgba8,
    RawPng,
}

/// UI-independent decoded screenshot pixels in row-major RGBA8 order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenshotImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl ScreenshotImage {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, BridgeError> {
        let expected_len = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| BridgeError::invalid_input("screenshot.dimensions.invalid"))?;

        if width == 0 || height == 0 || rgba.len() != expected_len {
            return Err(BridgeError::new(
                ErrorCode::InvalidInput,
                "screenshot.rgba.invalid",
                format!(
                    "expected {expected_len} RGBA bytes, received {}",
                    rgba.len()
                ),
            ));
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    #[must_use]
    pub fn into_rgba(self) -> Vec<u8> {
        self.rgba
    }
}

/// Raw bytes returned by a PNG screenshot capture.
///
/// This wrapper intentionally makes no validity guarantee. Consumers which
/// decode or save the payload remain responsible for full PNG validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawScreenshotPng(Vec<u8>);

impl RawScreenshotPng {
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScreenshotData {
    DecodedRgba8(ScreenshotImage),
    RawPng(RawScreenshotPng),
    DecodedWithPng {
        image: ScreenshotImage,
        png: RawScreenshotPng,
    },
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
    PathNotFound,
    AlreadyExists,
    PermissionDenied,
    Cancelled,
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

/// AI provider settings as supplied by the UI.
///
/// The API key lives only in memory (and in the desktop app's private local
/// storage); [`fmt::Debug`] masks it so it never reaches logs or error text.
#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AiSettings {
    /// Base URL of an OpenAI-compatible chat-completions API,
    /// e.g. `https://api.openai.com/v1`.
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
    #[serde(default = "default_ai_timeout_seconds")]
    pub timeout_seconds: u64,
}

fn default_ai_timeout_seconds() -> u64 {
    30
}

impl AiSettings {
    /// True when endpoint and model are present, i.e. the settings describe a
    /// provider the runtime could try to construct. The key may still be empty.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        !self.endpoint.trim().is_empty() && !self.model.trim().is_empty()
    }
}

impl fmt::Debug for AiSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiSettings")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "<empty>"
                } else {
                    "<redacted>"
                },
            )
            .field("timeout_seconds", &self.timeout_seconds)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendCommand {
    RefreshDevices,
    ConnectDevice(AdbEndpoint),
    DisconnectDevice(AdbEndpoint),
    SelectDevice(Option<DeviceSerial>),
    LoadOverview(DeviceSerial),
    LoadProcesses(DeviceTarget),
    LoadPerformance(DeviceTarget),
    LoadApplications(DeviceTarget),
    LoadApplicationDetails {
        request_id: OperationId,
        target: DeviceTarget,
        package: PackageName,
    },
    RunApplicationAction {
        request_id: OperationId,
        action: ApplicationAction,
        target: DeviceTarget,
        package: PackageName,
    },
    /// Extract launcher icons for the listed packages. One
    /// [`BackendEvent::ApplicationIconLoaded`] is emitted per package that
    /// yields an icon; packages without an extractable icon are skipped so
    /// the UI keeps its fallback tile.
    LoadApplicationIcons {
        target: DeviceTarget,
        packages: Vec<PackageName>,
    },
    OpenShell {
        target: DeviceTarget,
        session_id: ShellSessionId,
        size: ShellSize,
    },
    WriteShell {
        session_id: ShellSessionId,
        input: ShellInput,
    },
    ResizeShell {
        session_id: ShellSessionId,
        size: ShellSize,
    },
    CloseShell(ShellSessionId),
    CaptureScreenshot {
        request_id: OperationId,
        target: DeviceTarget,
        format: ScreenshotFormat,
    },
    /// Ask the configured AI provider for a completion. Reserved surface: the
    /// runtime resolves the provider independently; this command carries only
    /// the transcript the UI has assembled.
    SendAiChat {
        request_id: OperationId,
        prompt: String,
    },
    /// Install (or, with `None`, remove) the AI provider for this session.
    /// Constructed from the settings dialog; the runtime answers with
    /// [`BackendEvent::AiReady`] or [`BackendEvent::AiUnavailable`].
    ConfigureAi(Option<AiSettings>),
    ListDirectory {
        request_id: OperationId,
        target: DeviceTarget,
        path: RemotePath,
    },
    UploadFile {
        request_id: OperationId,
        target: DeviceTarget,
        local_path: PathBuf,
        remote_path: RemotePath,
        overwrite: OverwritePolicy,
    },
    DownloadFile {
        request_id: OperationId,
        target: DeviceTarget,
        remote_path: RemotePath,
        local_path: PathBuf,
        overwrite: OverwritePolicy,
    },
    CancelFileOperation(OperationId),
    CreateDirectory {
        request_id: OperationId,
        target: DeviceTarget,
        path: RemotePath,
    },
    RenameRemoteEntry {
        request_id: OperationId,
        target: DeviceTarget,
        source: RemotePath,
        destination: RemotePath,
    },
    DeleteRemoteFile {
        request_id: OperationId,
        target: DeviceTarget,
        path: RemotePath,
        confirmed: bool,
    },
    /// Start a live `logcat` stream for the target. The runtime answers with
    /// [`BackendEvent::LogcatStarted`] and then [`BackendEvent::LogcatOutput`]
    /// chunks until [`BackendEvent::LogcatClosed`] or [`LogcatFailed`].
    StartLogcat {
        target: DeviceTarget,
        session_id: LogcatSessionId,
    },
    StopLogcat(LogcatSessionId),
    /// Dump and parse the foreground window hierarchy via `uiautomator`.
    CaptureLayout {
        request_id: OperationId,
        target: DeviceTarget,
    },
    /// Enumerate the device's WebView devtools sockets (from
    /// `/proc/net/unix`).
    ListWebviewSockets {
        request_id: OperationId,
        target: DeviceTarget,
    },
    /// Forward a local TCP port to a devtools socket and list the inspectable
    /// pages over the Chrome DevTools HTTP endpoint.
    ListWebviewPages {
        request_id: OperationId,
        target: DeviceTarget,
        socket: String,
        port: u16,
    },
    /// Install an APK file from the host filesystem (`adb install -r`).
    InstallApk {
        request_id: OperationId,
        target: DeviceTarget,
        apk_path: PathBuf,
    },
    /// Pair with a wireless-debugging device using its pairing code.
    PairDevice {
        request_id: OperationId,
        host: String,
        port: u16,
        code: String,
    },
    /// Switch a USB-connected device to TCP listening mode.
    EnableTcpIp {
        request_id: OperationId,
        serial: DeviceSerial,
        port: u16,
    },
    /// List the wireless-debugging services currently advertised via mDNS.
    ListMdnsServices,
    /// Start the single video-mirror session (stops any current one first).
    StartMirror {
        request_id: OperationId,
        target: DeviceTarget,
        max_size: Option<u32>,
        video_bit_rate: u32,
    },
    /// Stop the current mirror session if one is running.
    StopMirror,
    /// Begin recording the running mirror session to an MP4 file next to the
    /// app. The file is announced by
    /// [`BackendEvent::MirrorRecordingSaved`] once recording stops.
    StartMirrorRecording,
    /// Finalize the running mirror recording, if any.
    StopMirrorRecording,
    /// Deliver one key event to the device (remote-control click). The
    /// runtime answers [`BackendEvent::OperationFailed`] on failure only.
    SendKeyEvent {
        target: DeviceTarget,
        keycode: u32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum BackendEvent {
    AdbReady {
        path: String,
        version: String,
    },
    AdbUnavailable(BridgeError),
    AdbConnecting(AdbEndpoint),
    AdbConnected(AdbEndpoint),
    AdbConnectFailed {
        endpoint: AdbEndpoint,
        error: BridgeError,
    },
    DevicesChanged(DeviceSnapshot),
    OverviewLoading(DeviceSerial),
    OverviewLoaded(DeviceOverview),
    ProcessesLoading(DeviceTarget),
    ProcessesLoaded(ProcessSnapshot),
    ProcessesFailed {
        target: DeviceTarget,
        error: BridgeError,
    },
    PerformanceLoading(DeviceTarget),
    PerformanceLoaded(PerformanceSnapshot),
    PerformanceFailed {
        target: DeviceTarget,
        error: BridgeError,
    },
    ApplicationsLoading(DeviceTarget),
    ApplicationsLoaded(ApplicationSnapshot),
    ApplicationsFailed {
        target: DeviceTarget,
        error: BridgeError,
    },
    ApplicationDetailsLoading {
        request_id: OperationId,
        target: DeviceTarget,
        package: PackageName,
    },
    ApplicationDetailsLoaded {
        request_id: OperationId,
        details: ApplicationDetails,
    },
    ApplicationDetailsFailed {
        request_id: OperationId,
        target: DeviceTarget,
        package: PackageName,
        error: BridgeError,
    },
    ApplicationActionStarted {
        request_id: OperationId,
        action: ApplicationAction,
        target: DeviceTarget,
        package: PackageName,
    },
    ApplicationActionCompleted {
        request_id: OperationId,
        action: ApplicationAction,
        target: DeviceTarget,
        package: PackageName,
    },
    ApplicationActionFailed {
        request_id: OperationId,
        action: ApplicationAction,
        target: DeviceTarget,
        package: PackageName,
        error: BridgeError,
    },
    ApplicationIconLoaded {
        target: DeviceTarget,
        package: PackageName,
        icon: ApplicationIconData,
    },
    ApkInstallLoading {
        request_id: OperationId,
        target: DeviceTarget,
    },
    ApkInstallFinished {
        request_id: OperationId,
        target: DeviceTarget,
    },
    ApkInstallFailed {
        request_id: OperationId,
        target: DeviceTarget,
        error: BridgeError,
    },
    PairFinished {
        request_id: OperationId,
    },
    PairFailed {
        request_id: OperationId,
        error: BridgeError,
    },
    TcpIpEnabled {
        request_id: OperationId,
        serial: DeviceSerial,
    },
    TcpIpFailed {
        request_id: OperationId,
        serial: DeviceSerial,
        error: BridgeError,
    },
    MdnsServicesLoaded {
        services: Vec<MdnsService>,
    },
    MdnsFailed {
        error: BridgeError,
    },
    /// The mirror session connected and announced its video dimensions.
    MirrorStarted {
        request_id: OperationId,
        target: DeviceTarget,
        width: u32,
        height: u32,
    },
    /// The mirror session ended without a protocol error (user stop or the
    /// server closed the stream).
    MirrorStopped {
        request_id: OperationId,
        target: DeviceTarget,
    },
    MirrorFailed {
        request_id: OperationId,
        target: DeviceTarget,
        error: BridgeError,
    },
    /// A mirror recording was finalized and written to `path`.
    MirrorRecordingSaved {
        target: DeviceTarget,
        path: PathBuf,
        frames: u64,
    },
    /// A mirror recording could not be produced (write failure or no frames).
    MirrorRecordingFailed {
        target: DeviceTarget,
        error: BridgeError,
    },
    OperationFailed(BridgeError),
    ShellOpened {
        target: DeviceTarget,
        session_id: ShellSessionId,
    },
    ShellOutput {
        session_id: ShellSessionId,
        bytes: Vec<u8>,
    },
    ShellClosed {
        session_id: ShellSessionId,
        exit_code: Option<i32>,
    },
    ShellFailed {
        session_id: ShellSessionId,
        error: BridgeError,
    },
    LogcatStarted {
        target: DeviceTarget,
        session_id: LogcatSessionId,
    },
    LogcatOutput {
        session_id: LogcatSessionId,
        bytes: Vec<u8>,
    },
    LogcatClosed {
        session_id: LogcatSessionId,
    },
    LogcatFailed {
        session_id: LogcatSessionId,
        error: BridgeError,
    },
    LayoutLoading {
        request_id: OperationId,
        target: DeviceTarget,
    },
    LayoutCaptured {
        request_id: OperationId,
        snapshot: LayoutSnapshot,
    },
    LayoutFailed {
        request_id: OperationId,
        target: DeviceTarget,
        error: BridgeError,
    },
    WebviewSocketsLoading {
        request_id: OperationId,
        target: DeviceTarget,
    },
    WebviewSocketsLoaded {
        request_id: OperationId,
        target: DeviceTarget,
        sockets: Vec<String>,
    },
    WebviewPagesLoading {
        request_id: OperationId,
        target: DeviceTarget,
        socket: String,
    },
    WebviewPagesLoaded {
        request_id: OperationId,
        target: DeviceTarget,
        socket: String,
        port: u16,
        pages: Vec<WebViewPage>,
    },
    WebviewFailed {
        request_id: OperationId,
        target: DeviceTarget,
        error: BridgeError,
    },
    ScreenshotLoading {
        request_id: OperationId,
        target: DeviceTarget,
        format: ScreenshotFormat,
    },
    ScreenshotCaptured {
        request_id: OperationId,
        target: DeviceTarget,
        data: ScreenshotData,
    },
    ScreenshotFailed {
        request_id: OperationId,
        target: DeviceTarget,
        error: BridgeError,
    },
    /// The AI provider became available (or a placeholder is active in dev).
    AiReady {
        kind: String,
        model: String,
    },
    /// No AI provider is configured; the panel should show a setup prompt.
    AiUnavailable {
        reason: String,
    },
    /// A single completion finished. Failures arrive as [`AiChatFailed`].
    AiChatCompleted {
        request_id: OperationId,
        reply: String,
    },
    AiChatFailed {
        request_id: OperationId,
        error: BridgeError,
    },
    DirectoryLoading {
        request_id: OperationId,
        target: DeviceTarget,
        path: RemotePath,
    },
    DirectoryLoaded {
        request_id: OperationId,
        listing: DirectoryListing,
    },
    DirectoryFailed {
        request_id: OperationId,
        target: DeviceTarget,
        path: RemotePath,
        error: BridgeError,
    },
    FileTransferStarted {
        request_id: OperationId,
        direction: FileTransferDirection,
        target: DeviceTarget,
        remote_path: RemotePath,
        local_path: PathBuf,
    },
    FileTransferCompleted {
        request_id: OperationId,
        summary: FileTransferSummary,
    },
    FileTransferFailed {
        request_id: OperationId,
        target: DeviceTarget,
        error: BridgeError,
    },
    FileTransferCancelled {
        request_id: OperationId,
        target: DeviceTarget,
    },
    FileMutationStarted {
        request_id: OperationId,
        kind: RemoteFileMutationKind,
        target: DeviceTarget,
        path: RemotePath,
        destination: Option<RemotePath>,
    },
    FileMutationCompleted {
        request_id: OperationId,
        summary: RemoteFileMutationSummary,
    },
    FileMutationFailed {
        request_id: OperationId,
        target: DeviceTarget,
        error: BridgeError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_paths_normalize_and_reject_escape() {
        assert_eq!(
            RemotePath::new("/sdcard/./Download/../demo.txt")
                .expect("valid")
                .as_str(),
            "/sdcard/demo.txt"
        );
        assert_eq!(RemotePath::new("/").expect("root").parent().as_str(), "/");
        assert!(RemotePath::new("relative").is_err());
        assert!(RemotePath::new("/../../escape").is_err());
        assert!(RemotePath::new("/bad\nname").is_err());
    }

    #[test]
    fn remote_path_joins_safe_components() {
        let path = RemotePath::new("/sdcard").expect("valid");
        assert_eq!(
            path.join_component("Download").expect("valid").as_str(),
            "/sdcard/Download"
        );
        assert!(path.join_component("../escape").is_err());
    }

    #[test]
    fn serial_rejects_control_characters() {
        assert!(DeviceSerial::new("serial\nother").is_err());
        assert!(DeviceSerial::new("").is_err());
    }

    #[test]
    fn package_names_reject_shell_metacharacters() {
        assert!(PackageName::new("com.example.app").is_ok());
        assert!(PackageName::new("com.example.android_app naïve").is_err());
        assert!(PackageName::new("com;rm -rf /").is_err());
        assert!(PackageName::new("$(reboot)").is_err());
        assert!(PackageName::new("com..example\n").is_err());
        assert!(PackageName::new(".starts.with.dot").is_err());
        assert!(PackageName::new("").is_err());
        assert!(PackageName::new("a".repeat(256)).is_err());
    }

    #[test]
    fn serial_redaction_keeps_only_suffix() {
        let serial = DeviceSerial::new("ABCDEF123456").expect("valid serial");
        assert_eq!(serial.redacted(), "device-…3456");
    }

    #[test]
    fn adb_endpoint_formats_ipv4_and_ipv6_targets() {
        let ipv4 = AdbEndpoint::new("192.168.1.20", 5555).expect("valid endpoint");
        assert_eq!(ipv4.adb_target(), "192.168.1.20:5555");
        let ipv6 = AdbEndpoint::new("2001:db8::20", 5555).expect("valid endpoint");
        assert_eq!(ipv6.to_string(), "[2001:db8::20]:5555");
    }

    #[test]
    fn adb_endpoint_rejects_unsafe_hosts_and_zero_ports() {
        assert!(AdbEndpoint::new("", 5555).is_err());
        assert!(AdbEndpoint::new("192.168.1.20:5555", 5555).is_err());
        assert!(AdbEndpoint::new("192.168.1.20", 0).is_err());
        assert!(AdbEndpoint::new("192.168.1.20\n", 5555).is_err());
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

    #[test]
    fn record_produces_generation_bound_target() {
        let record = DeviceRecord {
            descriptor: DeviceDescriptor {
                serial: DeviceSerial::new("device").expect("valid serial"),
                state: DeviceState::Online,
                product: None,
                model: None,
                device: None,
                transport_id: Some(7),
            },
            generation: 12,
        };

        assert_eq!(
            record.target(),
            DeviceTarget::new(record.descriptor.serial, 12)
        );
    }

    #[test]
    fn shell_input_accepts_binary_and_maximum_chunk() {
        let mut bytes = vec![0; MAX_SHELL_INPUT_BYTES];
        bytes[0] = 0xff;
        let input = ShellInput::new(bytes.clone()).expect("bounded binary input");
        assert_eq!(input.as_bytes(), bytes);
    }

    #[test]
    fn shell_input_rejects_empty_and_oversized_chunks() {
        let empty = ShellInput::new(Vec::new()).expect_err("empty input must fail");
        assert_eq!(empty.message_key, "shell.input.empty");

        let oversized = ShellInput::new(vec![0; MAX_SHELL_INPUT_BYTES + 1])
            .expect_err("oversized input must fail");
        assert_eq!(oversized.code, ErrorCode::OutputLimit);
        assert_eq!(oversized.message_key, "shell.input.too_large");
    }

    #[test]
    fn shell_size_rejects_zero_dimensions() {
        assert!(ShellSize::new(0, 24).is_err());
        assert!(ShellSize::new(80, 0).is_err());
        let size = ShellSize::new(80, 24).expect("valid terminal size");
        assert_eq!((size.columns(), size.rows()), (80, 24));
    }

    #[test]
    fn shell_size_deserialization_enforces_invariants() {
        use serde::de::value::{Error, MapDeserializer};

        let valid =
            MapDeserializer::<_, Error>::new([("columns", 80_u16), ("rows", 24_u16)].into_iter());
        assert_eq!(
            ShellSize::deserialize(valid).expect("valid size"),
            ShellSize::new(80, 24).expect("valid size")
        );

        let invalid =
            MapDeserializer::<_, Error>::new([("columns", 0_u16), ("rows", 24_u16)].into_iter());
        assert!(ShellSize::deserialize(invalid).is_err());
    }

    #[test]
    fn screenshot_image_requires_exact_rgba_length() {
        assert!(ScreenshotImage::new(2, 2, vec![0; 15]).is_err());
        assert!(ScreenshotImage::new(0, 2, Vec::new()).is_err());

        let image = ScreenshotImage::new(2, 2, vec![0; 16]).expect("valid RGBA image");
        assert_eq!((image.width(), image.height()), (2, 2));
        assert_eq!(image.rgba().len(), 16);
    }

    #[test]
    fn raw_screenshot_png_preserves_unvalidated_capture_bytes() {
        let bytes = b"transport payload; decoder validates later".to_vec();
        let png = RawScreenshotPng::new(bytes.clone());
        assert_eq!(png.as_bytes(), bytes);
        assert_eq!(png.into_bytes(), bytes);
    }

    #[test]
    fn session_ids_support_deterministic_construction() {
        let uuid = Uuid::from_u128(42);
        let id = ShellSessionId::from_uuid(uuid);
        assert_eq!(id.as_uuid(), uuid);
        assert_eq!(id.to_string(), uuid.to_string());
    }

    #[test]
    fn ai_settings_debug_masks_the_api_key() {
        let settings = AiSettings {
            endpoint: "https://example.invalid/v1".to_owned(),
            model: "demo".to_owned(),
            api_key: "sk-secret-value".to_owned(),
            timeout_seconds: 30,
        };
        let rendered = format!("{settings:?}");
        assert!(!rendered.contains("sk-secret-value"), "{rendered}");
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn ai_settings_usable_requires_endpoint_and_model() {
        assert!(!AiSettings::default().is_usable());
        let mut settings = AiSettings {
            endpoint: "https://example.invalid/v1".to_owned(),
            ..AiSettings::default()
        };
        assert!(!settings.is_usable());
        settings.model = "demo".to_owned();
        assert!(settings.is_usable());
    }

    #[test]
    fn ai_settings_deserialize_applies_default_timeout() {
        let settings: AiSettings = serde_json::from_str(
            "{\"endpoint\":\"https://e/v1\",\"model\":\"m\",\"api_key\":\"k\"}",
        )
        .expect("valid json");
        assert_eq!(settings.timeout_seconds, 30);
    }
}
