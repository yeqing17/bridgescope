//! Wireless debugging: pairing (`adb pair`), switching a device to TCP
//! listening mode (`adb -s SERIAL tcpip`), and mDNS service discovery
//! (`adb mdns services`).

use std::{ffi::OsString, path::Path, time::Duration};

use bridgescope_domain::{BridgeError, DeviceSerial, ErrorCode, MdnsService};

/// The pairing exchange involves a key handshake with the device, so allow
/// more than the plain-command timeout.
const PAIR_TIMEOUT: Duration = Duration::from_secs(30);
/// `tcpip` restarts adbd, which can take a moment to answer.
const TCPIP_TIMEOUT: Duration = Duration::from_secs(15);
const MDNS_TIMEOUT: Duration = Duration::from_secs(10);
const STDOUT_LIMIT: usize = 64 * 1024;
const STDERR_LIMIT: usize = 16 * 1024;

/// Runs `adb pair HOST:PORT CODE` after validating the inputs, mirroring the
/// checks the UI performs.
pub async fn pair(executable: &Path, host: &str, port: u16, code: &str) -> Result<(), BridgeError> {
    let host = host.trim();
    let code = code.trim();
    if host.is_empty() {
        return Err(BridgeError::new(
            ErrorCode::InvalidInput,
            "wireless.pair_invalid",
            "empty host",
        ));
    }
    if !valid_pairing_code(code) {
        return Err(BridgeError::new(
            ErrorCode::InvalidInput,
            "wireless.pair_invalid",
            "pairing code must be 6-8 digits",
        ));
    }
    let endpoint = format!("{host}:{port}");
    let output = crate::process::run_bounded(
        executable,
        vec![
            OsString::from("pair"),
            OsString::from(endpoint),
            OsString::from(code),
        ],
        PAIR_TIMEOUT,
        STDOUT_LIMIT,
        STDERR_LIMIT,
    )
    .await?;
    pair_result(&output)
}

/// Android's pairing codes are 6 digits; some builds accept up to 8.
fn valid_pairing_code(code: &str) -> bool {
    (6..=8).contains(&code.len()) && code.chars().all(|c| c.is_ascii_digit())
}

/// Judges one `adb pair` run by its well-known success line; failures print
/// a reason on stdout or stderr.
fn pair_result(output: &crate::process::ProcessOutput) -> Result<(), BridgeError> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.contains("Successfully paired") {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut detail = stdout.trim().to_owned();
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        if !detail.is_empty() {
            detail.push_str(" — ");
        }
        detail.push_str(stderr);
    }
    Err(BridgeError::new(
        ErrorCode::AdbFailed,
        "adb.pair_failed",
        detail,
    ))
}

/// Runs `adb -s SERIAL tcpip PORT` to make the device listen for network
/// connections after the next adbd restart.
pub async fn enable_tcpip(
    executable: &Path,
    serial: &DeviceSerial,
    port: u16,
) -> Result<(), BridgeError> {
    let output = crate::process::run_bounded(
        executable,
        vec![
            OsString::from("-s"),
            OsString::from(serial.as_str()),
            OsString::from("tcpip"),
            OsString::from(port.to_string()),
        ],
        TCPIP_TIMEOUT,
        STDOUT_LIMIT,
        STDERR_LIMIT,
    )
    .await?;
    tcpip_result(&output)
}

fn tcpip_result(output: &crate::process::ProcessOutput) -> Result<(), BridgeError> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.contains("restarting in TCP mode") {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(BridgeError::new(
        ErrorCode::AdbFailed,
        "adb.tcpip_failed",
        if stdout.trim().is_empty() {
            stderr.trim().to_owned()
        } else {
            stdout.trim().to_owned()
        },
    ))
}

/// Parses `adb mdns services` output: a header line, then one
/// `name\ttype\taddress` service per line.
pub fn parse_mdns_services(output: &str) -> Vec<MdnsService> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.trim().split('\t');
            let name = parts.next()?.trim();
            let service_type = parts.next()?.trim();
            let address = parts.next()?.trim();
            if name.is_empty() || service_type.is_empty() || address.is_empty() {
                return None;
            }
            Some(MdnsService {
                name: name.to_owned(),
                service_type: service_type.to_owned(),
                address: address.to_owned(),
            })
        })
        .collect()
}

/// Lists the wireless-debugging services currently visible to the adb
/// server's mDNS daemon.
pub async fn mdns_services(executable: &Path) -> Result<Vec<MdnsService>, BridgeError> {
    let output = crate::process::run_bounded(
        executable,
        vec![OsString::from("mdns"), OsString::from("services")],
        MDNS_TIMEOUT,
        STDOUT_LIMIT,
        STDERR_LIMIT,
    )
    .await?;
    Ok(parse_mdns_services(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mdns_parser_skips_header_and_blank_lines() {
        let services = parse_mdns_services(
            "List of discovered mdns services\r\n\
             Pixel_8\t_adb-tls-connect._tcp\t192.168.1.20:5555\r\n\
             \r\n\
             pairing-abc\t_adb-tls-pairing._tcp\t192.168.1.20:41234\r\n",
        );
        assert_eq!(
            services,
            vec![
                MdnsService {
                    name: "Pixel_8".to_owned(),
                    service_type: "_adb-tls-connect._tcp".to_owned(),
                    address: "192.168.1.20:5555".to_owned(),
                },
                MdnsService {
                    name: "pairing-abc".to_owned(),
                    service_type: "_adb-tls-pairing._tcp".to_owned(),
                    address: "192.168.1.20:41234".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn mdns_parser_tolerates_empty_output() {
        assert!(parse_mdns_services("List of discovered mdns services\n").is_empty());
        assert!(parse_mdns_services("").is_empty());
    }

    #[test]
    fn pair_result_accepts_the_success_line() {
        let output = crate::process::ProcessOutput {
            stdout: b"Successfully paired to 192.168.1.20:41234".to_vec(),
            stderr: Vec::new(),
            exit_code: Some(0),
        };
        assert!(pair_result(&output).is_ok());
    }

    #[test]
    fn pair_result_surfaces_the_failure_reason() {
        let output = crate::process::ProcessOutput {
            stdout: Vec::new(),
            stderr: b"error: protocol fault".to_vec(),
            exit_code: Some(1),
        };
        let error = pair_result(&output).expect_err("must fail");
        assert_eq!(error.message_key, "adb.pair_failed");
        assert!(error.detail.contains("protocol fault"));
    }

    #[test]
    fn tcpip_result_accepts_the_restart_line() {
        let output = crate::process::ProcessOutput {
            stdout: b"restarting in TCP mode port: 5555".to_vec(),
            stderr: Vec::new(),
            exit_code: Some(0),
        };
        assert!(tcpip_result(&output).is_ok());
    }

    #[test]
    fn pair_validates_host_and_code_before_spawning() {
        // These fail on input validation without ever touching adb, so the
        // executable path is never consulted.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let missing_host = runtime.block_on(pair(std::path::Path::new("adb"), " ", 5555, "123456"));
        assert_eq!(
            missing_host.expect_err("must fail").message_key,
            "wireless.pair_invalid"
        );
        let short_code = runtime.block_on(pair(
            std::path::Path::new("adb"),
            "192.168.1.20",
            5555,
            "12345",
        ));
        assert_eq!(
            short_code.expect_err("must fail").message_key,
            "wireless.pair_invalid"
        );
    }

    #[test]
    fn pairing_codes_must_be_six_to_eight_digits() {
        assert!(valid_pairing_code("123456"));
        assert!(valid_pairing_code("12345678"));
        assert!(!valid_pairing_code("12345"));
        assert!(!valid_pairing_code("123456789"));
        assert!(!valid_pairing_code("12345a"));
        assert!(!valid_pairing_code(""));
    }
}
