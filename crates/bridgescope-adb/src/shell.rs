use std::{ffi::OsString, future::Future, path::PathBuf, process::Stdio, time::Duration};

use crate::process::configure_command;
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
const MAX_INPUT_BATCH_CHUNKS: usize = 16;
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
    completion: Option<JoinHandle<Result<Option<i32>, BridgeError>>>,
}

impl ShellSessionHandle {
    /// Creates a channel-backed shell session.
    ///
    /// This allows transport implementations that do not own a child process to
    /// expose the same interactive shell interface. The handler receives input,
    /// sends output chunks, and is cancelled when [`Self::close`] is called.
    pub fn from_handler<F, Fut>(handler: F) -> Self
    where
        F: FnOnce(
                mpsc::Receiver<Vec<u8>>,
                mpsc::Sender<ShellOutputChunk>,
                CancellationToken,
            ) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = Result<Option<i32>, BridgeError>> + Send + 'static,
    {
        let (input, input_rx) = mpsc::channel(CHANNEL_CAPACITY);
        let (output_tx, output) = mpsc::channel(CHANNEL_CAPACITY);
        let cancellation = CancellationToken::new();
        let handler_cancel = cancellation.clone();
        let completion = tokio::spawn(handler(input_rx, output_tx, handler_cancel));
        Self {
            input,
            output,
            cancellation,
            completion: Some(completion),
        }
    }

    pub fn input(&self) -> mpsc::Sender<Vec<u8>> {
        self.input.clone()
    }

    pub fn output_mut(&mut self) -> &mut mpsc::Receiver<ShellOutputChunk> {
        &mut self.output
    }

    pub async fn close(mut self) -> Result<Option<i32>, BridgeError> {
        self.cancellation.cancel();
        let completion = self.completion.take().ok_or_else(|| {
            BridgeError::new(
                ErrorCode::Internal,
                "shell.already_closed",
                "shell is already closed",
            )
        })?;
        completion.await.map_err(|error| {
            BridgeError::new(
                ErrorCode::Internal,
                "runtime.task_failed",
                error.to_string(),
            )
        })?
    }
}

impl Drop for ShellSessionHandle {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(completion) = self.completion.take() {
            completion.abort();
        }
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
    let mut command = Command::new(executable);
    command
        .args(shell_arguments(serial))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_command(&mut command);
    let mut child = command.spawn().map_err(|error| {
        BridgeError::new(
            ErrorCode::AdbFailed,
            "shell.spawn_failed",
            error.to_string(),
        )
    })?;

    let stdin = child.stdin.take().ok_or_else(|| missing_pipe("stdin"))?;
    let stdout = child.stdout.take().ok_or_else(|| missing_pipe("stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| missing_pipe("stderr"))?;
    Ok(ShellSessionHandle::from_handler(
        move |input, output, cancellation| async move {
            run_shell(child, stdin, input, output, stdout, stderr, cancellation).await
        },
    ))
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
                let input = collect_input_batch(input, &mut input_rx);
                stdin.write_all(&input).await.map_err(|error| {
                    BridgeError::new(ErrorCode::AdbFailed, "shell.write_failed", error.to_string())
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
    let status = if let Ok(result) = timeout(CLOSE_GRACE, child.wait()).await {
        result.map_err(|error| {
            BridgeError::new(ErrorCode::AdbFailed, "shell.wait_failed", error.to_string())
        })?
    } else {
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
    };
    cancellation.cancel();
    finish_reader(stdout_task).await?;
    finish_reader(stderr_task).await?;
    Ok(status.code())
}

/// Coalesce already-queued keystrokes before writing to the OS pipe.
///
/// `ChildStdin` is not buffered, so `write_all` makes the bytes available to
/// adb immediately. Combining a small bounded burst avoids one runtime wakeup
/// and pipe write per key when egui delivers multiple input events in a frame.
fn collect_input_batch(mut input: Vec<u8>, input_rx: &mut mpsc::Receiver<Vec<u8>>) -> Vec<u8> {
    for _ in 1..MAX_INPUT_BATCH_CHUNKS {
        match input_rx.try_recv() {
            Ok(next) => input.extend_from_slice(&next),
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {
                break;
            }
        }
    }
    input
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

    #[test]
    fn input_batch_preserves_order_and_leaves_later_chunks_queued() {
        let (sender, mut receiver) = mpsc::channel(20);
        for byte in 0_u8..20 {
            sender.try_send(vec![byte]).expect("queue has capacity");
        }

        let first = receiver.try_recv().expect("first input chunk");
        assert_eq!(
            collect_input_batch(first, &mut receiver),
            (0_u8..MAX_INPUT_BATCH_CHUNKS as u8)
                .map(|byte| vec![byte])
                .flatten()
                .collect::<Vec<_>>()
        );
        assert_eq!(receiver.try_recv().expect("later chunk remains"), vec![16]);
    }
}
