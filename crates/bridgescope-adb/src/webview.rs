//! WebView inspection plumbing: discover devtools sockets on the device and
//! forward local TCP ports to them over `adb forward`.
//!
//! The DevTools HTTP/WS endpoints themselves are consumed by the app process
//! (plain HTTP against `127.0.0.1:<forwarded port>`), keeping the transport
//! crate free of an HTTP dependency.

use std::{path::Path, time::Duration};

use bridgescope_domain::{BridgeError, DeviceSerial, ErrorCode};

use crate::process::run_bounded;

const STDERR_LIMIT: usize = 16 * 1024;
const STDOUT_LIMIT: usize = 1024 * 1024;

fn adb_arguments(serial: &DeviceSerial, rest: &[&str]) -> Vec<String> {
    let mut arguments = vec![
        "-s".to_owned(),
        serial.as_str().to_owned(),
        "forward".to_owned(),
    ];
    arguments.extend(rest.iter().map(|item| (*item).to_owned()));
    arguments
}

async fn adb_forward(
    executable: &Path,
    serial: &DeviceSerial,
    rest: &[&str],
    timeout: Duration,
) -> Result<String, BridgeError> {
    let arguments: Vec<_> = adb_arguments(serial, rest)
        .into_iter()
        .map(std::ffi::OsString::from)
        .collect();
    let output = run_bounded(executable, arguments, timeout, STDOUT_LIMIT, STDERR_LIMIT).await?;
    if output.exit_code != Some(0) {
        return Err(BridgeError::new(
            ErrorCode::AdbFailed,
            "webview.forward_failed",
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(crate) async fn forward_port(
    executable: &Path,
    serial: &DeviceSerial,
    port: u16,
    socket: &str,
    timeout: Duration,
) -> Result<(), BridgeError> {
    adb_forward(
        executable,
        serial,
        &[&format!("tcp:{port}"), &format!("localabstract:{socket}")],
        timeout,
    )
    .await
    .map(|_| ())
}

pub(crate) async fn remove_forward(
    executable: &Path,
    serial: &DeviceSerial,
    port: u16,
    timeout: Duration,
) -> Result<(), BridgeError> {
    let spec = format!("tcp:{port}");
    adb_forward(executable, serial, &["--remove", &spec], timeout)
        .await
        .map(|_| ())
}

/// Lists candidate WebView devtools socket names for the device.
///
/// Reads `/proc/net/unix` and keeps sockets whose name marks a devtools
/// endpoint (`*_devtools_remote*`). Abstract sockets (leading `@` in the
/// kernel table) map to `localabstract:` names without the marker byte, so it
/// is stripped here.
pub(crate) async fn list_devtools_sockets(
    executable: &Path,
    serial: &DeviceSerial,
    timeout: Duration,
) -> Result<Vec<String>, BridgeError> {
    let output = run_bounded(
        executable,
        [
            "-s".to_owned(),
            serial.as_str().to_owned(),
            "shell".to_owned(),
            "cat /proc/net/unix".to_owned(),
        ]
        .into_iter()
        .map(std::ffi::OsString::from)
        .collect(),
        timeout,
        STDOUT_LIMIT,
        STDERR_LIMIT,
    )
    .await?;
    Ok(parse_unix_sockets(&String::from_utf8_lossy(&output.stdout)))
}

/// Extracts, deduplicates, and sorts devtools socket names from a
/// `/proc/net/unix` table.
pub fn parse_unix_sockets(table: &str) -> Vec<String> {
    let mut sockets: Vec<String> = Vec::new();
    for line in table.lines() {
        let columns: Vec<&str> = line.split_whitespace().collect();
        let Some(path) = columns.get(7) else {
            continue;
        };
        if !path.contains("devtools_remote") {
            continue;
        }
        let name = path.strip_prefix('@').unwrap_or(path).to_owned();
        if !name.is_empty() && !sockets.contains(&name) {
            sockets.push(name);
        }
    }
    sockets.sort();
    sockets
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str =
        "00000000: 00000002 00000000 00010000 0001 01  9793 @com.example.app_devtools_remote
00000000: 00000002 00000000 00010000 0001 01 12345 @chrome_devtools_remote
00000000: 00000003 00000000 00010000 0001 03 23456 /dev/socket/mdns
00000000: 00000002 00000000 00010000 0001 01  9793 @com.example.app_devtools_remote
short line
";

    #[test]
    fn extracts_deduplicated_sorted_socket_names() {
        assert_eq!(
            parse_unix_sockets(TABLE),
            vec!["chrome_devtools_remote", "com.example.app_devtools_remote",]
        );
    }

    #[test]
    fn empty_table_yields_no_sockets() {
        assert!(parse_unix_sockets("").is_empty());
        assert!(parse_unix_sockets("garbage columns only").is_empty());
    }
}
