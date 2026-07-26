use std::{ffi::OsString, path::Path, process::Stdio, time::Duration};

use bridgescope_domain::{BridgeError, ErrorCode};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time::sleep,
};
use tokio_util::sync::CancellationToken;

pub(crate) struct ProcessOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
}

pub(crate) fn configure_command(command: &mut Command) {
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
}

pub(crate) async fn run_bounded(
    executable: &Path,
    arguments: Vec<OsString>,
    command_timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<ProcessOutput, BridgeError> {
    run_bounded_cancellable(
        executable,
        arguments,
        command_timeout,
        stdout_limit,
        stderr_limit,
        CancellationToken::new(),
    )
    .await
}

pub(crate) async fn run_bounded_cancellable(
    executable: &Path,
    arguments: Vec<OsString>,
    command_timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
    cancellation: CancellationToken,
) -> Result<ProcessOutput, BridgeError> {
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_command(&mut command);
    let mut child = command.spawn().map_err(|error| {
        BridgeError::new(ErrorCode::AdbFailed, "adb.spawn_failed", error.to_string())
    })?;

    let stdout = child.stdout.take().ok_or_else(|| {
        BridgeError::new(
            ErrorCode::Internal,
            "adb.stdout_missing",
            "stdout pipe missing",
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        BridgeError::new(
            ErrorCode::Internal,
            "adb.stderr_missing",
            "stderr pipe missing",
        )
    })?;
    let stdout_task = tokio::spawn(read_limited(stdout, stdout_limit));
    let stderr_task = tokio::spawn(read_limited(stderr, stderr_limit));

    let status = tokio::select! {
        result = child.wait() => result.map_err(|error| {
            BridgeError::new(ErrorCode::AdbFailed, "adb.wait_failed", error.to_string())
        }),
        () = sleep(command_timeout) => Err(BridgeError::new(
            ErrorCode::TimedOut,
            "adb.timed_out",
            "adb command timed out",
        )),
        () = cancellation.cancelled() => Err(BridgeError::new(
            ErrorCode::Cancelled,
            "adb.cancelled",
            "adb command cancelled",
        )),
    };

    match status {
        Ok(status) => {
            let stdout = stdout_task.await.map_err(|error| join_error(&error))??;
            let stderr = stderr_task.await.map_err(|error| join_error(&error))??;
            Ok(ProcessOutput {
                stdout,
                stderr,
                exit_code: status.code(),
            })
        }
        Err(error) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            Err(error)
        }
    }
}

async fn read_limited<R>(mut reader: R, limit: usize) -> Result<Vec<u8>, BridgeError>
where
    R: AsyncRead + Unpin,
{
    let mut output = Vec::with_capacity(limit.min(64 * 1024));
    let mut chunk = [0_u8; 8192];
    loop {
        let count = reader.read(&mut chunk).await.map_err(|error| {
            BridgeError::new(ErrorCode::AdbFailed, "adb.read_failed", error.to_string())
        })?;
        if count == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(count) > limit {
            return Err(BridgeError::new(
                ErrorCode::OutputLimit,
                "adb.output_limit",
                "adb output exceeded the configured limit",
            ));
        }
        output.extend_from_slice(&chunk[..count]);
    }
}

fn join_error(error: &tokio::task::JoinError) -> BridgeError {
    BridgeError::new(
        ErrorCode::Internal,
        "runtime.task_failed",
        error.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncWriteExt;

    use super::*;

    #[tokio::test]
    async fn bounded_reader_preserves_binary_bytes() {
        let (mut writer, reader) = tokio::io::duplex(32);
        let bytes = vec![0, 13, 10, 255, 0, 128];
        let expected = bytes.clone();
        tokio::spawn(async move {
            writer.write_all(&bytes).await.expect("write fixture");
        });
        assert_eq!(
            read_limited(reader, 32).await.expect("read fixture"),
            expected
        );
    }

    #[tokio::test]
    async fn bounded_reader_rejects_limit_plus_one() {
        let (mut writer, reader) = tokio::io::duplex(32);
        tokio::spawn(async move {
            writer
                .write_all(&[1, 2, 3, 4])
                .await
                .expect("write fixture");
        });
        assert_eq!(
            read_limited(reader, 3).await.expect_err("must reject").code,
            ErrorCode::OutputLimit
        );
    }
}
