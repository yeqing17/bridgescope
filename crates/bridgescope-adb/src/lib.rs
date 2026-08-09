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
    AdbEndpoint, BridgeError, DeviceCapabilities, DeviceDescriptor, DeviceOverview, DeviceSerial,
    DeviceState, ErrorCode, OverwritePolicy, PerformanceMetrics, ProcessInfo, RemoteFileEntry,
    RemotePath, ShellSize,
};
use tokio_util::sync::CancellationToken;

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
    async fn connect_endpoint(&self, endpoint: &AdbEndpoint) -> Result<String, BridgeError>;
    async fn device_overview(&self, serial: &DeviceSerial) -> Result<DeviceOverview, BridgeError>;
    async fn list_processes(&self, serial: &DeviceSerial) -> Result<Vec<ProcessInfo>, BridgeError>;
    async fn performance_metrics(
        &self,
        serial: &DeviceSerial,
    ) -> Result<PerformanceMetrics, BridgeError>;
    async fn start_shell(
        &self,
        serial: &DeviceSerial,
        size: ShellSize,
    ) -> Result<ShellSessionHandle, BridgeError>;
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
        cancellation: CancellationToken,
    ) -> Result<(), BridgeError>;
    async fn pull_file(
        &self,
        serial: &DeviceSerial,
        remote_path: &RemotePath,
        local_path: &Path,
        overwrite: OverwritePolicy,
        cancellation: CancellationToken,
    ) -> Result<(), BridgeError>;
    async fn create_directory(
        &self,
        serial: &DeviceSerial,
        path: &RemotePath,
    ) -> Result<(), BridgeError>;
    async fn rename_entry(
        &self,
        serial: &DeviceSerial,
        source: &RemotePath,
        destination: &RemotePath,
    ) -> Result<(), BridgeError>;
    async fn delete_file(
        &self,
        serial: &DeviceSerial,
        path: &RemotePath,
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

    async fn connect_endpoint(&self, endpoint: &AdbEndpoint) -> Result<String, BridgeError> {
        self.run(connect_arguments(endpoint)).await
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

    async fn list_processes(&self, serial: &DeviceSerial) -> Result<Vec<ProcessInfo>, BridgeError> {
        let output = self
            .shell(
                serial,
                &["ps", "-A", "-o", "PID,USER,STAT,%CPU,%MEM,RSS,NAME"],
            )
            .await?;
        parse_processes(&output)
    }

    async fn performance_metrics(
        &self,
        serial: &DeviceSerial,
    ) -> Result<PerformanceMetrics, BridgeError> {
        let meminfo = self.shell(serial, &["cat", "/proc/meminfo"]).await?;
        let loadavg = self.optional_shell(serial, &["cat", "/proc/loadavg"]).await;
        let cpuinfo = self.optional_shell(serial, &["dumpsys", "cpuinfo"]).await;
        let battery = self.optional_shell(serial, &["dumpsys", "battery"]).await;
        let storage = self.optional_shell(serial, &["df", "-k", "/data"]).await;
        Ok(PerformanceMetrics {
            cpu_usage_percent: cpuinfo.as_deref().and_then(parse_cpu_usage_percent),
            load_average_1m: loadavg.as_deref().and_then(parse_load_average_1m),
            memory_total_kib: parse_mem_value_kib(&meminfo, "MemTotal"),
            memory_available_kib: parse_mem_value_kib(&meminfo, "MemAvailable")
                .or_else(|| parse_mem_value_kib(&meminfo, "MemFree")),
            storage_total_kib: storage
                .as_deref()
                .and_then(parse_storage_kib)
                .map(|value| value.0),
            storage_used_kib: storage
                .as_deref()
                .and_then(parse_storage_kib)
                .map(|value| value.1),
            battery_percent: battery
                .as_deref()
                .and_then(|value| parse_named_u64(value, "level"))
                .and_then(|value| u8::try_from(value).ok()),
        })
    }

    async fn start_shell(
        &self,
        serial: &DeviceSerial,
        size: ShellSize,
    ) -> Result<ShellSessionHandle, BridgeError> {
        shell::start_shell(self.executable.clone(), serial, size)
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
        cancellation: CancellationToken,
    ) -> Result<(), BridgeError> {
        files::push_file(
            &self.executable,
            serial,
            local_path,
            remote_path,
            overwrite,
            cancellation,
            self.timeout,
        )
        .await
    }

    async fn pull_file(
        &self,
        serial: &DeviceSerial,
        remote_path: &RemotePath,
        local_path: &Path,
        overwrite: OverwritePolicy,
        cancellation: CancellationToken,
    ) -> Result<(), BridgeError> {
        files::pull_file(
            &self.executable,
            serial,
            remote_path,
            local_path,
            overwrite,
            cancellation,
            self.timeout,
        )
        .await
    }

    async fn create_directory(
        &self,
        serial: &DeviceSerial,
        path: &RemotePath,
    ) -> Result<(), BridgeError> {
        files::create_directory(&self.executable, serial, path, self.timeout).await
    }

    async fn rename_entry(
        &self,
        serial: &DeviceSerial,
        source: &RemotePath,
        destination: &RemotePath,
    ) -> Result<(), BridgeError> {
        files::rename_entry(&self.executable, serial, source, destination, self.timeout).await
    }

    async fn delete_file(
        &self,
        serial: &DeviceSerial,
        path: &RemotePath,
    ) -> Result<(), BridgeError> {
        files::delete_file(&self.executable, serial, path, self.timeout).await
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

pub fn parse_processes(output: &str) -> Result<Vec<ProcessInfo>, BridgeError> {
    let mut processes = Vec::new();
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if line.to_ascii_uppercase().starts_with("USER")
            || line.to_ascii_uppercase().starts_with("PID")
        {
            continue;
        }
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 7 {
            continue;
        }
        let Ok(pid) = fields[0].parse() else {
            continue;
        };
        processes.push(ProcessInfo {
            pid,
            user: (!fields[1].is_empty()).then(|| fields[1].to_owned()),
            state: (!fields[2].is_empty()).then(|| fields[2].to_owned()),
            cpu_percent: parse_percent(fields[3]),
            memory_percent: parse_percent(fields[4]),
            resident_memory_kib: fields[5].parse().ok(),
            name: fields[6..].join(" "),
        });
    }
    processes.sort_by(|left, right| {
        right
            .cpu_percent
            .unwrap_or_default()
            .total_cmp(&left.cpu_percent.unwrap_or_default())
            .then_with(|| left.pid.cmp(&right.pid))
    });
    Ok(processes)
}

fn parse_named_u64(output: &str, name: &str) -> Option<u64> {
    output.lines().find_map(|line| {
        let (key, value) = line.trim().split_once(':')?;
        (key == name).then(|| value.trim().parse().ok()).flatten()
    })
}

fn parse_mem_total_kib(output: &str) -> Option<u64> {
    parse_mem_value_kib(output, "MemTotal")
}

fn parse_mem_value_kib(output: &str, key: &str) -> Option<u64> {
    output.lines().find_map(|line| {
        let (name, value) = line.trim().split_once(':')?;
        (name == key)
            .then(|| value.split_whitespace().next()?.parse().ok())
            .flatten()
    })
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

fn parse_percent(value: &str) -> Option<f32> {
    value.trim_end_matches('%').parse().ok()
}

fn parse_load_average_1m(output: &str) -> Option<f32> {
    output.split_whitespace().next()?.parse().ok()
}

fn parse_cpu_usage_percent(output: &str) -> Option<f32> {
    output.lines().find_map(|line| {
        let upper = line.to_ascii_uppercase();
        if !upper.contains("TOTAL") {
            return None;
        }
        line.split_whitespace()
            .find_map(|field| field.strip_suffix('%').and_then(|value| value.parse().ok()))
    })
}

fn connect_arguments(endpoint: &AdbEndpoint) -> [OsString; 2] {
    [
        OsString::from("connect"),
        OsString::from(endpoint.adb_target()),
    ]
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

    #[test]
    fn builds_expected_connect_arguments() {
        let endpoint = AdbEndpoint::new("192.168.1.20", 5555).expect("valid endpoint");
        assert_eq!(
            connect_arguments(&endpoint),
            [
                OsString::from("connect"),
                OsString::from("192.168.1.20:5555")
            ]
        );
    }

    #[test]
    fn parses_process_table_and_sorts_by_cpu() {
        let output = "PID USER STAT %CPU %MEM RSS NAME\n42 root S 0.5 1.2 128 system_server\n7 user S 12.5 2.0 256 app process\n";
        let processes = parse_processes(output).expect("valid process output");
        assert_eq!(processes[0].pid, 7);
        assert_eq!(processes[0].name, "app process");
        assert_eq!(processes[1].resident_memory_kib, Some(128));
    }

    #[test]
    fn parses_performance_values() {
        assert_eq!(
            parse_mem_value_kib("MemTotal:       1024 kB", "MemTotal"),
            Some(1024)
        );
        assert_eq!(parse_load_average_1m("1.25 0.50 0.25 1/20 42"), Some(1.25));
        assert_eq!(
            parse_cpu_usage_percent("TOTAL: 18% user + 4% kernel"),
            Some(18.0)
        );
    }
}
