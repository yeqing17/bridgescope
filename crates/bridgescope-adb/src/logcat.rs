//! Live `logcat` streaming on top of the shell session plumbing.
//!
//! The stream is one long-lived `adb exec-out logcat` child process; output is
//! forwarded verbatim through the same channel-backed handle the interactive
//! shell uses, so the runtime treats both identically.

use std::{path::PathBuf, process::Stdio};

use bridgescope_domain::{BridgeError, DeviceSerial, ErrorCode};
use tokio::process::Command;

use crate::{
    process::configure_command,
    shell::{ShellSessionHandle, ShellStream, finish_reader, forward_output},
};

pub(crate) fn logcat_arguments(serial: &DeviceSerial) -> Vec<String> {
    vec![
        "-s".to_owned(),
        serial.as_str().to_owned(),
        "exec-out".to_owned(),
        "logcat".to_owned(),
        "-v".to_owned(),
        "threadtime".to_owned(),
    ]
}

pub(crate) fn start_logcat(
    executable: PathBuf,
    serial: &DeviceSerial,
) -> Result<ShellSessionHandle, BridgeError> {
    let mut command = Command::new(executable);
    command
        .args(logcat_arguments(serial))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_command(&mut command);
    let mut child = command.spawn().map_err(|error| {
        BridgeError::new(
            ErrorCode::AdbFailed,
            "logcat.spawn_failed",
            error.to_string(),
        )
    })?;

    let stdout = child.stdout.take().ok_or_else(|| missing_pipe("stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| missing_pipe("stderr"))?;
    Ok(ShellSessionHandle::from_handler(
        move |_input, output, cancellation| async move {
            let stdout_task = tokio::spawn(forward_output(
                stdout,
                ShellStream::Stdout,
                output.clone(),
                cancellation.child_token(),
            ));
            let stderr_task = tokio::spawn(forward_output(
                stderr,
                ShellStream::Stderr,
                output,
                cancellation.child_token(),
            ));
            let status = child.wait().await.map_err(|error| {
                BridgeError::new(
                    ErrorCode::AdbFailed,
                    "logcat.wait_failed",
                    error.to_string(),
                )
            })?;
            finish_reader(stdout_task).await?;
            finish_reader(stderr_task).await?;
            // logcat exits non-zero when the device disconnects; treat it as
            // a normal stream end, the UI keeps its buffered lines.
            let _ = status;
            Ok(None)
        },
    ))
}

fn missing_pipe(name: &str) -> BridgeError {
    BridgeError::new(
        ErrorCode::Internal,
        "shell.pipe_missing",
        format!("{name} pipe missing"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_expected_logcat_arguments() {
        let serial = DeviceSerial::new("emulator-5554").expect("valid serial");
        assert_eq!(
            logcat_arguments(&serial),
            [
                "-s",
                "emulator-5554",
                "exec-out",
                "logcat",
                "-v",
                "threadtime"
            ]
        );
    }
}
