use std::{ffi::OsString, path::Path, time::Duration};

use bridgescope_domain::{BridgeError, DeviceSerial, ErrorCode};

use crate::process::run_bounded;

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);
const PNG_LIMIT: usize = 32 * 1024 * 1024;
const STDERR_LIMIT: usize = 64 * 1024;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

pub(crate) fn screenshot_arguments(serial: &DeviceSerial) -> Vec<OsString> {
    vec![
        OsString::from("-s"),
        OsString::from(serial.as_str()),
        OsString::from("exec-out"),
        OsString::from("screencap"),
        OsString::from("-p"),
    ]
}

pub(crate) async fn capture_screenshot(
    executable: &Path,
    serial: &DeviceSerial,
) -> Result<Vec<u8>, BridgeError> {
    let output = run_bounded(
        executable,
        screenshot_arguments(serial),
        CAPTURE_TIMEOUT,
        PNG_LIMIT,
        STDERR_LIMIT,
    )
    .await?;
    if output.exit_code != Some(0) {
        return Err(BridgeError::new(
            ErrorCode::AdbFailed,
            "screenshot.capture_failed",
            bounded_stderr(&output.stderr, output.exit_code),
        ));
    }
    if !output.stdout.starts_with(PNG_SIGNATURE) {
        return Err(BridgeError::new(
            ErrorCode::AdbFailed,
            "screenshot.invalid_png",
            "screencap did not return a PNG image",
        ));
    }
    Ok(output.stdout)
}

fn bounded_stderr(stderr: &[u8], code: Option<i32>) -> String {
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    if message.is_empty() {
        format!(
            "adb exited with {}",
            code.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
        )
    } else {
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_expected_screenshot_arguments() {
        let serial = DeviceSerial::new("ABC123").expect("valid serial");
        assert_eq!(
            screenshot_arguments(&serial),
            ["-s", "ABC123", "exec-out", "screencap", "-p"]
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn recognizes_png_signature_without_text_conversion() {
        let bytes = [PNG_SIGNATURE.as_slice(), &[0, 13, 10, 255]].concat();
        assert!(bytes.starts_with(PNG_SIGNATURE));
    }
}
