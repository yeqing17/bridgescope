use std::{ffi::OsString, path::PathBuf, process::Stdio, time::Duration};

use bridgescope_domain::{BridgeError, DeviceSerial, ErrorCode};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, Command},
    sync::mpsc,
    task::JoinHandle,
    time::timeout,
};
use tokio_util::sync::CancellationToken;

const CHUNK_BYTES: usize = 8192;
const CHANNEL_CAPACITY: usize = 128;
const CLOSE_GRACE: Duration = Duration::from_millis(750);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellOutputChunk {
    pub stream: ShellStream,
    pub bytes: Vec<u8>,
}

pub struct ShellSessionHandle {
    input: mpsc::Sender<Vec<u8>>,
    output: mpsc::Receiver<ShellOutputChunk>,
    cancellation: CancellationToken,
    completion: JoinHandle<Result<Option<i32>, BridgeError>>,
}

impl ShellSessionHandle {
    pub fn input(&self) -> mpsc::Sender<Vec<u8>> {
        self.input.clone()
    }

    pub fn output_mut(&mut self) -> &mut mpsc::Receiver<ShellOutputChunk> {
        &mut self.output
    }

    pub async fn close(self) -> Result<Option<i32>, BridgeError> {
        self.cancellation.cancel();
        self.completion.await.map_err(|error| {
            BridgeError::new(
                ErrorCode::Internal,
                "runtime.task_failed",
                error.to_string(),
            )
        })?
    }
}

pub(crate) fn shell_arguments(serial: &DeviceSerial) -> Vec<OsString> {
    vec![
        OsString::from("-s"),
        OsString::from(serial.as_str()),
        OsString::from("shell"),
        OsString::from("-tt"),
    ]
}

pub(crate) fn start_shell(
    executable: PathBuf,
    serial: &DeviceSerial,
) -> Result<ShellSessionHandle, BridgeError> {
    let mut child = Command::new(executable)
        .args(shell_arguments(serial))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            BridgeError::new(
                ErrorCode::AdbFailed,
                "shell.spawn_failed",
                error.to_string(),
            )
        })?;

    let stdin = child.stdin.take().ok_or_else(|| missing_pipe("stdin"))?;
    let stdout = child.stdout.take().ok_or_else(|| missing_pipe("stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| missing_pipe("stderr"))?;
    let (input_tx, input_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (output_tx, output_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let cancellation = CancellationToken::new();
    let completion_cancel = cancellation.clone();
    let completion = tokio::spawn(async move {
        run_shell(
            child,
            stdin,
            input_rx,
            output_tx,
            stdout,
            stderr,
            completion_cancel,
        )
        .await
    });

    Ok(ShellSessionHandle {
        input: input_tx,
        output: output_rx,
        cancellation,
        completion,
    })
}

async fn run_shell<Out, Err>(
    mut child: Child,
    mut stdin: ChildStdin,
    mut input_rx: mpsc::Receiver<Vec<u8>>,
    output_tx: mpsc::Sender<ShellOutputChunk>,
    stdout: Out,
    stderr: Err,
    cancellation: CancellationToken,
) -> Result<Option<i32>, BridgeError>
where
    Out: AsyncRead + Unpin + Send + 'static,
    Err: AsyncRead + Unpin + Send + 'static,
{
    let stdout_task = tokio::spawn(forward_output(
        stdout,
        ShellStream::Stdout,
        output_tx.clone(),
        cancellation.child_token(),
    ));
    let stderr_task = tokio::spawn(forward_output(
        stderr,
        ShellStream::Stderr,
        output_tx,
        cancellation.child_token(),
    ));

    loop {
        tokio::select! {
            () = cancellation.cancelled() => break,
            input = input_rx.recv() => {
                let Some(input) = input else { break };
                stdin.write_all(&input).await.map_err(|error| {
                    BridgeError::new(ErrorCode::AdbFailed, "shell.write_failed", error.to_string())
                })?;
                stdin.flush().await.map_err(|error| {
                    BridgeError::new(ErrorCode::AdbFailed, "shell.flush_failed", error.to_string())
                })?;
            }
            status = child.wait() => {
                cancellation.cancel();
                finish_reader(stdout_task).await?;
                finish_reader(stderr_task).await?;
                return status
                    .map(|status| status.code())
                    .map_err(|error| BridgeError::new(ErrorCode::AdbFailed, "shell.wait_failed", error.to_string()));
            }
        }
    }

    drop(stdin);
    let status = match timeout(CLOSE_GRACE, child.wait()).await {
        Ok(result) => result.map_err(|error| {
            BridgeError::new(ErrorCode::AdbFailed, "shell.wait_failed", error.to_string())
        })?,
        Err(_) => {
            child.start_kill().map_err(|error| {
                BridgeError::new(ErrorCode::AdbFailed, "shell.kill_failed", error.to_string())
            })?;
            timeout(CLOSE_GRACE, child.wait())
                .await
                .map_err(|_| {
                    BridgeError::new(
                        ErrorCode::TimedOut,
                        "shell.close_timed_out",
                        "shell did not exit",
                    )
                })?
                .map_err(|error| {
                    BridgeError::new(ErrorCode::AdbFailed, "shell.wait_failed", error.to_string())
                })?
        }
    };
    cancellation.cancel();
    finish_reader(stdout_task).await?;
    finish_reader(stderr_task).await?;
    Ok(status.code())
}

async fn forward_output<R>(
    mut reader: R,
    stream: ShellStream,
    sender: mpsc::Sender<ShellOutputChunk>,
    cancellation: CancellationToken,
) -> Result<(), BridgeError>
where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; CHUNK_BYTES];
    loop {
        let count = tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            result = reader.read(&mut buffer) => result.map_err(|error| {
                BridgeError::new(ErrorCode::AdbFailed, "shell.read_failed", error.to_string())
            })?,
        };
        if count == 0 {
            return Ok(());
        }
        let chunk = ShellOutputChunk {
            stream,
            bytes: buffer[..count].to_vec(),
        };
        tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            result = sender.send(chunk) => {
                if result.is_err() {
                    return Ok(());
                }
            }
        }
    }
}

async fn finish_reader(task: JoinHandle<Result<(), BridgeError>>) -> Result<(), BridgeError> {
    match timeout(CLOSE_GRACE, task).await {
        Ok(result) => result.map_err(|error| {
            BridgeError::new(
                ErrorCode::Internal,
                "runtime.task_failed",
                error.to_string(),
            )
        })?,
        Err(_) => Ok(()),
    }
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
    fn builds_expected_shell_arguments() {
        let serial = DeviceSerial::new("emulator-5554").expect("valid serial");
        assert_eq!(
            shell_arguments(&serial),
            ["-s", "emulator-5554", "shell", "-tt"]
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>()
        );
    }
}
