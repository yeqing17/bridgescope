mod files;
mod process;
mod screenshot;
mod shell;

use std::{
    collections::HashMap,
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};

use async_trait::async_trait;
use bridgescope_domain::{
    BridgeError, DeviceCapabilities, DeviceDescriptor, DeviceOverview, DeviceSerial, DeviceState,
    ErrorCode, OverwritePolicy, RemoteFileEntry, RemotePath,
};

pub use shell::{ShellOutputChunk, ShellSessionHandle, ShellStream};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct AdbLocator {
    explicit: Option<PathBuf>,
}

impl AdbLocator {
    pub fn new(explicit: Option<PathBuf>) -> Self {
        Self { explicit }
    }

    pub fn candidates(&self) -> Vec<PathBuf> {
        let executable = adb_executable_name();
        let mut candidates = Vec::new();
        if let Some(path) = &self.explicit {
            candidates.push(path.clone());
        }
        for variable in ["ANDROID_SDK_ROOT", "ANDROID_HOME"] {
            if let Some(root) = env::var_os(variable) {
                candidates.push(PathBuf::from(root).join("platform-tools").join(executable));
            }
        }
        if let Some(path) = env::var_os("PATH") {
            candidates.extend(env::split_paths(&path).map(|part| part.join(executable)));
        }
        deduplicate_paths(candidates)
    }

    pub fn locate(&self) -> Result<PathBuf, BridgeError> {
        self.candidates()
            .into_iter()
            .find(|path| path.is_file())
            .ok_or_else(|| {
                BridgeError::new(
                    ErrorCode::AdbNotFound,
                    "adb.not_found",
                    "adb was not found in settings, Android SDK, or PATH",
                )
            })
    }
}

fn adb_executable_name() -> &'static str {
    if cfg!(windows) { "adb.exe" } else { "adb" }
}

fn deduplicate_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut unique = Vec::new();
    for path in paths {
        if !unique
            .iter()
            .any(|existing: &PathBuf| paths_equal(existing, &path))
        {
            unique.push(path);
        }
    }
    unique
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

#[async_trait]
pub trait AdbTransport: Send + Sync {
    async fn version(&self) -> Result<String, BridgeError>;
    async fn list_devices(&self) -> Result<Vec<DeviceDescriptor>, BridgeError>;
    async fn device_overview(&self, serial: &DeviceSerial) -> Result<DeviceOverview, BridgeError>;
    async fn start_shell(&self, serial: &DeviceSerial) -> Result<ShellSessionHandle, BridgeError>;
    async fn capture_screenshot(&self, serial: &DeviceSerial) -> Result<Vec<u8>, BridgeError>;
    async fn list_directory(
        &self,
        serial: &DeviceSerial,
        path: &RemotePath,
    ) -> Result<Vec<RemoteFileEntry>, BridgeError>;
    async fn push_file(
        &self,
        serial: &DeviceSerial,
        local_path: &Path,
        remote_path: &RemotePath,
        overwrite: OverwritePolicy,
    ) -> Result<(), BridgeError>;
    async fn pull_file(
        &self,
        serial: &DeviceSerial,
        remote_path: &RemotePath,
        local_path: &Path,
        overwrite: OverwritePolicy,
    ) -> Result<(), BridgeError>;
}

#[derive(Clone, Debug)]
pub struct ProcessAdbTransport {
    executable: PathBuf,
    timeout: Duration,
    max_output_bytes: usize,
}

impl ProcessAdbTransport {
    pub fn new(executable: PathBuf) -> Self {
        Self {
            executable,
            timeout: DEFAULT_TIMEOUT,
            max_output_bytes: MAX_OUTPUT_BYTES,
        }
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    async fn run<I, S>(&self, arguments: I) -> Result<String, BridgeError>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let arguments = arguments.into_iter().map(Into::into).collect();
        let output = process::run_bounded(
            &self.executable,
            arguments,
            self.timeout,
            self.max_output_bytes,
            self.max_output_bytes,
        )
        .await?;
        if output.exit_code != Some(0) {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(BridgeError::new(
                ErrorCode::AdbFailed,
                "adb.command_failed",
                if detail.is_empty() {
                    format!("adb exited with {:?}", output.exit_code)
                } else {
                    detail
                },
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    async fn shell(&self, serial: &DeviceSerial, command: &[&str]) -> Result<String, BridgeError> {
        let mut arguments = vec![
            OsString::from("-s"),
            OsString::from(serial.as_str()),
            OsString::from("shell"),
        ];
        arguments.extend(command.iter().map(OsString::from));
        self.run(arguments).await
    }

    async fn optional_shell(&self, serial: &DeviceSerial, command: &[&str]) -> Option<String> {
        self.shell(serial, command)
            .await
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }
}

#[async_trait]
impl AdbTransport for ProcessAdbTransport {
    async fn version(&self) -> Result<String, BridgeError> {
        let output = self.run(["version"]).await?;
        Ok(output.lines().take(2).collect::<Vec<_>>().join(" · "))
    }

    async fn list_devices(&self) -> Result<Vec<DeviceDescriptor>, BridgeError> {
        let output = self.run(["devices", "-l"]).await?;
        parse_devices(&output)
    }

    async fn device_overview(&self, serial: &DeviceSerial) -> Result<DeviceOverview, BridgeError> {
        let descriptors = self.list_devices().await?;
        let descriptor = descriptors
            .into_iter()
            .find(|device| &device.serial == serial)
            .ok_or_else(|| {
                BridgeError::new(
                    ErrorCode::DeviceNotFound,
                    "device.not_found",
                    serial.redacted(),
                )
            })?;
        if !descriptor.state.is_online() {
            return Err(BridgeError::new(
                ErrorCode::DeviceUnavailable,
                "device.unavailable",
                format!("{} is {:?}", serial.redacted(), descriptor.state),
            ));
        }

        let model = self
            .optional_shell(serial, &["getprop", "ro.product.model"])
            .await;
        let manufacturer = self
            .optional_shell(serial, &["getprop", "ro.product.manufacturer"])
            .await;
        let android_version = self
            .optional_shell(serial, &["getprop", "ro.build.version.release"])
            .await;
        let api_level = self
            .optional_shell(serial, &["getprop", "ro.build.version.sdk"])
            .await
            .and_then(|value| value.parse().ok());
        let abi = self
            .optional_shell(serial, &["getprop", "ro.product.cpu.abi"])
            .await;
        let battery_percent = self
            .optional_shell(serial, &["dumpsys", "battery"])
            .await
            .and_then(|value| parse_named_u64(&value, "level"))
            .and_then(|value| u8::try_from(value).ok());
        let memory_total_kib = self
            .optional_shell(serial, &["cat", "/proc/meminfo"])
            .await
            .and_then(|value| parse_mem_total_kib(&value));
        let storage = self
            .optional_shell(serial, &["df", "-k", "/data"])
            .await
            .and_then(|value| parse_storage_kib(&value));

        Ok(DeviceOverview {
            serial: serial.clone(),
            model: model.or(descriptor.model),
            manufacturer,
            android_version,
            api_level,
            abi,
            battery_percent,
            memory_total_kib,
            storage_total_kib: storage.map(|(total, _)| total),
            storage_used_kib: storage.map(|(_, used)| used),
            capabilities: DeviceCapabilities::basic_online(),
        })
    }

    async fn start_shell(&self, serial: &DeviceSerial) -> Result<ShellSessionHandle, BridgeError> {
        shell::start_shell(self.executable.clone(), serial)
    }

    async fn capture_screenshot(&self, serial: &DeviceSerial) -> Result<Vec<u8>, BridgeError> {
        screenshot::capture_screenshot(&self.executable, serial).await
    }

    async fn list_directory(
        &self,
        serial: &DeviceSerial,
        path: &RemotePath,
    ) -> Result<Vec<RemoteFileEntry>, BridgeError> {
        files::list_directory(&self.executable, serial, path, self.timeout).await
    }

    async fn push_file(
        &self,
        serial: &DeviceSerial,
        local_path: &Path,
        remote_path: &RemotePath,
        overwrite: OverwritePolicy,
    ) -> Result<(), BridgeError> {
        files::push_file(&self.executable, serial, local_path, remote_path, overwrite).await
    }

    async fn pull_file(
        &self,
        serial: &DeviceSerial,
        remote_path: &RemotePath,
        local_path: &Path,
        overwrite: OverwritePolicy,
    ) -> Result<(), BridgeError> {
        files::pull_file(&self.executable, serial, remote_path, local_path, overwrite).await
    }
}

pub fn parse_devices(output: &str) -> Result<Vec<DeviceDescriptor>, BridgeError> {
    let mut devices = Vec::new();
    for line in output.lines().map(str::trim) {
        if line.is_empty() || line.starts_with("List of devices attached") || line.starts_with('*')
        {
            continue;
        }
        let mut fields = line.split_whitespace();
        let serial = fields.next().ok_or_else(|| {
            BridgeError::new(ErrorCode::AdbFailed, "adb.devices.invalid_line", line)
        })?;
        let state = match fields.next().unwrap_or("unknown") {
            "device" => DeviceState::Online,
            "offline" => DeviceState::Offline,
            "unauthorized" => DeviceState::Unauthorized,
            _ => DeviceState::Unknown,
        };
        let attributes = fields
            .filter_map(|field| field.split_once(':'))
            .collect::<HashMap<_, _>>();
        devices.push(DeviceDescriptor {
            serial: DeviceSerial::new(serial)?,
            state,
            product: attributes.get("product").map(|value| (*value).to_owned()),
            model: attributes.get("model").map(|value| value.replace('_', " ")),
            device: attributes.get("device").map(|value| (*value).to_owned()),
            transport_id: attributes
                .get("transport_id")
                .and_then(|value| value.parse().ok()),
        });
    }
    devices.sort_by(|left, right| left.serial.cmp(&right.serial));
    Ok(devices)
}

fn parse_named_u64(output: &str, name: &str) -> Option<u64> {
    output.lines().find_map(|line| {
        let (key, value) = line.trim().split_once(':')?;
        (key == name).then(|| value.trim().parse().ok()).flatten()
    })
}

fn parse_mem_total_kib(output: &str) -> Option<u64> {
    let line = output.lines().find(|line| line.starts_with("MemTotal:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

fn parse_storage_kib(output: &str) -> Option<(u64, u64)> {
    output.lines().skip(1).find_map(|line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 4 {
            return None;
        }
        Some((fields[1].parse().ok()?, fields[2].parse().ok()?))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_states_and_attributes() {
        let input = "List of devices attached\nemulator-5554 device product:sdk model:Pixel_8 device:emu transport_id:1\nABC offline transport_id:2\nXYZ unauthorized\n";
        let devices = parse_devices(input).expect("valid adb output");
        assert_eq!(devices.len(), 3);
        let emulator = devices
            .iter()
            .find(|device| device.serial.as_str() == "emulator-5554")
            .expect("emulator present");
        assert_eq!(emulator.model.as_deref(), Some("Pixel 8"));
        assert_eq!(emulator.state, DeviceState::Online);
        assert_eq!(devices[0].state, DeviceState::Offline);
        assert_eq!(devices[1].state, DeviceState::Unauthorized);
    }

    #[test]
    fn ignores_daemon_messages_and_header() {
        let input = "* daemon started successfully\nList of devices attached\n\n";
        assert!(parse_devices(input).expect("valid empty list").is_empty());
    }

    #[test]
    fn parses_overview_numbers() {
        assert_eq!(parse_named_u64(" level: 87\n", "level"), Some(87));
        assert_eq!(parse_mem_total_kib("MemTotal: 123456 kB\n"), Some(123_456));
        assert_eq!(
            parse_storage_kib(
                "Filesystem 1K-blocks Used Available Use% Mounted on\n/dev/a 1000 250 750 25% /data\n"
            ),
            Some((1000, 250))
        );
    }
}
