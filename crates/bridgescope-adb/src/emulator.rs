//! AVD management through the SDK `emulator` binary, plus per-emulator
//! console queries routed over adb (`emu avd name`, `emu kill`).
//!
//! Launching an AVD is a plain detached host process spawn: the emulator
//! outlives the caller, advertises itself over the normal ADB device list,
//! and from there the rest of BridgeScope treats it like any other device.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};

use bridgescope_domain::{BridgeError, DeviceSerial, ErrorCode};

use crate::process::run_bounded;

const STDERR_LIMIT: usize = 16 * 1024;
const STDOUT_LIMIT: usize = 64 * 1024;

fn emulator_executable_name() -> &'static str {
    if cfg!(windows) {
        "emulator.exe"
    } else {
        "emulator"
    }
}

/// Candidate locations for the SDK emulator binary: derived from the resolved
/// adb path (`<sdk>/platform-tools/adb` implies `<sdk>/emulator/emulator`)
/// and the usual SDK environment variables, mirroring [`crate::AdbLocator`].
pub fn emulator_candidates(adb_path: &Path) -> Vec<PathBuf> {
    let name = emulator_executable_name();
    let mut candidates = Vec::new();
    if let Some(platform_tools) = adb_path.parent()
        && let Some(sdk_root) = platform_tools.parent()
    {
        candidates.push(sdk_root.join("emulator").join(name));
    }
    for variable in ["ANDROID_SDK_ROOT", "ANDROID_HOME"] {
        if let Some(root) = std::env::var_os(variable) {
            candidates.push(PathBuf::from(root).join("emulator").join(name));
        }
    }
    candidates
}

/// Parses `emulator -list-avds` output: one non-empty AVD name per line,
/// sorted and deduplicated for stable display.
pub fn parse_avd_list(output: &str) -> Vec<String> {
    let mut names: Vec<String> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    names.sort();
    names.dedup();
    names
}

/// `emu avd name` answers the AVD name followed by an `OK` line.
pub fn parse_avd_name(output: &str) -> Option<String> {
    let mut lines = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let name = lines.next()?;
    (lines.next() == Some("OK")).then(|| name.to_owned())
}

pub async fn list_avds(executable: &Path, timeout: Duration) -> Result<Vec<String>, BridgeError> {
    let output = run_bounded(
        executable,
        vec![OsString::from("-list-avds")],
        timeout,
        STDOUT_LIMIT,
        STDERR_LIMIT,
    )
    .await?;
    if output.exit_code != Some(0) {
        return Err(emulator_error(
            "avd.list_failed",
            &output.stderr,
            output.exit_code,
        ));
    }
    Ok(parse_avd_list(&String::from_utf8_lossy(&output.stdout)))
}

/// Spawns an emulator for the named AVD without waiting on it: its stdio is
/// detached, so the process keeps running after this returns.
pub fn launch_avd(executable: &Path, name: &str, wipe_data: bool) -> Result<(), BridgeError> {
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err(BridgeError::new(
            ErrorCode::InvalidInput,
            "avd.launch_failed",
            "invalid AVD name",
        ));
    }
    let mut command = tokio::process::Command::new(executable);
    command.arg("-avd").arg(name);
    if wipe_data {
        command.arg("-wipe-data");
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    crate::process::configure_command(&mut command);
    command.spawn().map(|_| ()).map_err(|error| {
        BridgeError::new(ErrorCode::AdbFailed, "avd.launch_failed", error.to_string())
    })
}

/// The AVD name behind an emulator serial, or `None` when the serial is not
/// an emulator (adb exits non-zero for `emu` commands on other devices).
pub async fn running_avd_name(
    executable: &Path,
    serial: &DeviceSerial,
    timeout: Duration,
) -> Result<Option<String>, BridgeError> {
    let output = run_bounded(
        executable,
        vec![
            OsString::from("-s"),
            OsString::from(serial.as_str()),
            OsString::from("emu"),
            OsString::from("avd"),
            OsString::from("name"),
        ],
        timeout,
        STDOUT_LIMIT,
        STDERR_LIMIT,
    )
    .await?;
    if output.exit_code != Some(0) {
        return Ok(None);
    }
    Ok(parse_avd_name(&String::from_utf8_lossy(&output.stdout)))
}

/// Asks an emulator to exit through its console (`emu kill`).
pub async fn kill_emulator(
    executable: &Path,
    serial: &DeviceSerial,
    timeout: Duration,
) -> Result<(), BridgeError> {
    let output = run_bounded(
        executable,
        vec![
            OsString::from("-s"),
            OsString::from(serial.as_str()),
            OsString::from("emu"),
            OsString::from("kill"),
        ],
        timeout,
        STDOUT_LIMIT,
        STDERR_LIMIT,
    )
    .await?;
    if output.exit_code != Some(0) {
        return Err(emulator_error(
            "avd.kill_failed",
            &output.stderr,
            output.exit_code,
        ));
    }
    Ok(())
}

fn emulator_error(message_key: &'static str, stderr: &[u8], exit_code: Option<i32>) -> BridgeError {
    let detail = String::from_utf8_lossy(stderr).trim().to_owned();
    BridgeError::new(
        ErrorCode::AdbFailed,
        message_key,
        if detail.is_empty() {
            format!("emulator exited with {exit_code:?}")
        } else {
            detail
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avd_list_parser_trims_sorts_and_dedupes() {
        let avds = parse_avd_list("Television_1080p\n\n  Pixel_9a \nPixel_9a\n");
        assert_eq!(
            avds,
            vec!["Pixel_9a".to_owned(), "Television_1080p".to_owned()]
        );
    }

    #[test]
    fn avd_list_parser_tolerates_empty_output() {
        assert!(parse_avd_list("\n \n").is_empty());
    }

    #[test]
    fn avd_name_parser_requires_the_ok_line() {
        assert_eq!(parse_avd_name("Pixel_9a\nOK"), Some("Pixel_9a".to_owned()));
        assert_eq!(parse_avd_name("Pixel_9a"), None);
        assert_eq!(parse_avd_name(""), None);
    }

    /// Real acceptance: launch the first available AVD, wait for it to come
    /// online as a new emulator serial, confirm the name query, then stop it.
    /// Boots take minutes and open a window, so this only runs when
    /// BRIDGESCOPE_REAL_DEVICE_TEST is set.
    #[tokio::test]
    #[ignore = "requires the Android SDK emulator and boots a real AVD"]
    async fn launch_and_kill_roundtrip_boots_a_real_avd() {
        use crate::AdbTransport;
        if std::env::var_os("BRIDGESCOPE_REAL_DEVICE_TEST").is_none() {
            return;
        }
        let executable = std::env::var_os("BRIDGESCOPE_ADB")
            .map(std::path::PathBuf::from)
            .expect("set BRIDGESCOPE_ADB");
        let transport = crate::ProcessAdbTransport::new(executable);
        let before: Vec<_> = transport
            .list_devices()
            .await
            .expect("baseline device list")
            .into_iter()
            .map(|device| device.serial)
            .collect();
        let name = transport
            .list_avds()
            .await
            .expect("avd list")
            .into_iter()
            .next()
            .expect("at least one AVD");

        transport.launch_avd(&name, false).await.expect("launch");
        // Cold boots can take several minutes, in particular while other
        // emulators are hogging memory, so wait up to ~10 minutes. Whatever
        // the outcome, the new emulator is shut down afterwards.
        let mut appeared: Option<DeviceSerial> = None;
        let mut online: Option<DeviceSerial> = None;
        for _ in 0..120 {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let devices = transport.list_devices().await.unwrap_or_default();
            for device in devices {
                if device.serial.as_str().starts_with("emulator-")
                    && !before.contains(&device.serial)
                {
                    appeared = Some(device.serial.clone());
                    if device.state.is_online() {
                        online = Some(device.serial.clone());
                    }
                }
            }
            if online.is_some() {
                break;
            }
        }
        let outcome: Result<DeviceSerial, String> = async {
            let serial = online.clone().ok_or_else(|| {
                "launched emulator did not come online within 10 minutes".to_owned()
            })?;
            let running = transport
                .running_avd_name(&serial)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "emulator did not report its AVD name".to_owned())?;
            if running != name {
                return Err(format!("avd name mismatch: {running} != {name}"));
            }
            transport
                .kill_emulator(&serial)
                .await
                .map_err(|error| error.to_string())?;
            Ok(serial)
        }
        .await;
        // Shut the emulator down even when verification failed mid-way.
        let stopped = if let Ok(serial) = &outcome {
            Ok(serial.clone())
        } else {
            let leftover = appeared.as_ref().expect("no new emulator appeared");
            transport
                .kill_emulator(leftover)
                .await
                .map(|()| leftover.clone())
        };
        outcome.expect("AVD launch roundtrip failed");
        stopped.expect("emu kill");
    }
}
