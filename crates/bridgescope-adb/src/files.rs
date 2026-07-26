use std::{ffi::OsString, path::Path, time::Duration};

use bridgescope_domain::{
    BridgeError, ErrorCode, OverwritePolicy, RemoteFileEntry, RemoteFileKind, RemotePath,
};

use crate::process;

const TRANSFER_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const METADATA_LIMIT: usize = 8 * 1024 * 1024;
const STDERR_LIMIT: usize = 1024 * 1024;
const LIST_SCRIPT: &str = r#"directory=$1
for entry in "$directory"/* "$directory"/.[!.]* "$directory"/..?*; do
  [ -e "$entry" ] || [ -L "$entry" ] || continue
  name=${entry##*/}
  if [ -L "$entry" ]; then kind=l; elif [ -d "$entry" ]; then kind=d; elif [ -f "$entry" ]; then kind=f; else kind=o; fi
  size=$(stat -c %s "$entry" 2>/dev/null || true)
  modified=$(stat -c %Y "$entry" 2>/dev/null || true)
  permissions=$(stat -c %A "$entry" 2>/dev/null || true)
  printf '%s\034%s\034%s\034%s\034%s\000' "$kind" "$name" "$size" "$modified" "$permissions"
done"#;

pub(crate) async fn list_directory(
    executable: &Path,
    serial: &bridgescope_domain::DeviceSerial,
    path: &RemotePath,
    timeout: Duration,
) -> Result<Vec<RemoteFileEntry>, BridgeError> {
    let output = process::run_bounded(
        executable,
        vec![
            OsString::from("-s"),
            OsString::from(serial.as_str()),
            OsString::from("shell"),
            OsString::from("sh"),
            OsString::from("-c"),
            OsString::from(LIST_SCRIPT),
            OsString::from("bridgescope-files"),
            OsString::from(path.as_str()),
        ],
        timeout,
        METADATA_LIMIT,
        STDERR_LIMIT,
    )
    .await?;
    if output.exit_code != Some(0) {
        return Err(map_command_error(&output.stderr, "file.list_failed"));
    }
    parse_directory_entries(path, &output.stdout)
}

pub(crate) async fn push_file(
    executable: &Path,
    serial: &bridgescope_domain::DeviceSerial,
    local_path: &Path,
    remote_path: &RemotePath,
    overwrite: OverwritePolicy,
) -> Result<(), BridgeError> {
    let metadata = tokio::fs::metadata(local_path).await.map_err(|error| {
        BridgeError::new(
            ErrorCode::PathNotFound,
            "file.local_source_missing",
            error.to_string(),
        )
    })?;
    if !metadata.is_file() {
        return Err(BridgeError::invalid_input("file.local_source_not_file"));
    }
    if overwrite == OverwritePolicy::Deny && remote_exists(executable, serial, remote_path).await? {
        return Err(BridgeError::new(
            ErrorCode::AlreadyExists,
            "file.remote_exists",
            remote_path.to_string(),
        ));
    }
    run_transfer(
        executable,
        vec![
            OsString::from("-s"),
            OsString::from(serial.as_str()),
            OsString::from("push"),
            local_path.as_os_str().to_owned(),
            OsString::from(remote_path.as_str()),
        ],
        "file.upload_failed",
    )
    .await
}

pub(crate) async fn pull_file(
    executable: &Path,
    serial: &bridgescope_domain::DeviceSerial,
    remote_path: &RemotePath,
    local_path: &Path,
    overwrite: OverwritePolicy,
) -> Result<(), BridgeError> {
    if let Ok(metadata) = tokio::fs::symlink_metadata(local_path).await {
        if metadata.file_type().is_symlink() {
            return Err(BridgeError::new(
                ErrorCode::PermissionDenied,
                "file.local_symlink_refused",
                local_path.display().to_string(),
            ));
        }
        if overwrite == OverwritePolicy::Deny {
            return Err(BridgeError::new(
                ErrorCode::AlreadyExists,
                "file.local_exists",
                local_path.display().to_string(),
            ));
        }
    }
    run_transfer(
        executable,
        vec![
            OsString::from("-s"),
            OsString::from(serial.as_str()),
            OsString::from("pull"),
            OsString::from(remote_path.as_str()),
            local_path.as_os_str().to_owned(),
        ],
        "file.download_failed",
    )
    .await
}

async fn remote_exists(
    executable: &Path,
    serial: &bridgescope_domain::DeviceSerial,
    path: &RemotePath,
) -> Result<bool, BridgeError> {
    let output = process::run_bounded(
        executable,
        vec![
            OsString::from("-s"),
            OsString::from(serial.as_str()),
            OsString::from("shell"),
            OsString::from("sh"),
            OsString::from("-c"),
            OsString::from("test -e \"$1\" || test -L \"$1\""),
            OsString::from("bridgescope-files"),
            OsString::from(path.as_str()),
        ],
        Duration::from_secs(8),
        1024,
        1024,
    )
    .await?;
    Ok(output.exit_code == Some(0))
}

async fn run_transfer(
    executable: &Path,
    arguments: Vec<OsString>,
    message_key: &'static str,
) -> Result<(), BridgeError> {
    let output = process::run_bounded(
        executable,
        arguments,
        TRANSFER_TIMEOUT,
        64 * 1024,
        STDERR_LIMIT,
    )
    .await?;
    if output.exit_code == Some(0) {
        Ok(())
    } else {
        Err(map_command_error(&output.stderr, message_key))
    }
}

fn parse_directory_entries(
    directory: &RemotePath,
    output: &[u8],
) -> Result<Vec<RemoteFileEntry>, BridgeError> {
    let mut entries = Vec::new();
    for record in output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let fields = record.split(|byte| *byte == 0x1c).collect::<Vec<_>>();
        if fields.len() != 5 {
            return Err(BridgeError::new(
                ErrorCode::AdbFailed,
                "file.list_invalid_record",
                "device returned an invalid directory record",
            ));
        }
        let text = |bytes: &[u8]| {
            String::from_utf8(bytes.to_vec()).map_err(|error| {
                BridgeError::new(
                    ErrorCode::AdbFailed,
                    "file.name_not_utf8",
                    error.to_string(),
                )
            })
        };
        let kind = match fields[0] {
            b"d" => RemoteFileKind::Directory,
            b"f" => RemoteFileKind::File,
            b"l" => RemoteFileKind::Symlink,
            _ => RemoteFileKind::Other,
        };
        let name = text(fields[1])?;
        entries.push(RemoteFileEntry {
            path: directory.join_component(&name)?,
            name,
            kind,
            size_bytes: text(fields[2])?.parse().ok(),
            modified_unix_seconds: text(fields[3])?.parse().ok(),
            permissions: match text(fields[4])? {
                value if value.is_empty() => None,
                value => Some(value),
            },
        });
    }
    entries.sort_by(|left, right| {
        let left_rank = u8::from(left.kind != RemoteFileKind::Directory);
        let right_rank = u8::from(right.kind != RemoteFileKind::Directory);
        left_rank
            .cmp(&right_rank)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(entries)
}

fn map_command_error(stderr: &[u8], message_key: &'static str) -> BridgeError {
    let detail = String::from_utf8_lossy(stderr).trim().to_owned();
    let lower = detail.to_ascii_lowercase();
    let code = if lower.contains("permission denied") {
        ErrorCode::PermissionDenied
    } else if lower.contains("no such file") || lower.contains("not found") {
        ErrorCode::PathNotFound
    } else {
        ErrorCode::AdbFailed
    };
    BridgeError::new(
        code,
        message_key,
        if detail.is_empty() {
            "adb file command failed".to_owned()
        } else {
            detail
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nul_delimited_directory_entries() {
        let directory = RemotePath::new("/sdcard").expect("valid path");
        let output = b"d\x1cDownload\x1c4096\x1c1700000000\x1cdrwxr-xr-x\0f\x1cspace name.txt\x1c12\x1c\x1c-rw-r--r--\0";
        let entries = parse_directory_entries(&directory, output).expect("valid listing");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "Download");
        assert_eq!(entries[1].path.as_str(), "/sdcard/space name.txt");
    }
}
