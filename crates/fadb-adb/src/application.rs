//! Package-manager queries and actions (`pm`, `am`, `monkey`).
//!
//! Every command here receives the package name as a standalone argument and
//! [`PackageName`] guarantees it cannot carry shell metacharacters, so the
//! device-side `adb shell` join is safe.

use std::collections::BTreeMap;

use fadb_domain::{
    ApplicationDetails, ApplicationRecord, BridgeError, DeviceSerial, ErrorCode, PackageName,
};

/// `pm list packages -3`: third-party packages only.
pub const LIST_THIRD_PARTY: &[&str] = &["pm", "list", "packages", "-3"];
/// `pm list packages -s`: system packages only.
pub const LIST_SYSTEM: &[&str] = &["pm", "list", "packages", "-s"];
/// `pm list packages -d`: disabled packages only.
pub const LIST_DISABLED: &[&str] = &["pm", "list", "packages", "-d"];

/// Upper bound on reported permissions, so a pathological `dumpsys` output
/// cannot balloon the event payload.
const MAX_PERMISSIONS: usize = 256;

/// Extracts bare package names from `pm list packages` output
/// (`package:com.example.app` per line).
pub fn parse_package_names(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("package:"))
        .map(str::to_owned)
        .collect()
}

/// Combines the `pm list packages` filter outputs into one snapshot.
///
/// System packages are inserted first so an updated system app that also
/// shows up in `-3` on some builds keeps its system classification. The
/// result is ordered third-party first, alphabetical within each group.
pub fn parse_applications(
    third_party: &str,
    system: &str,
    disabled: &str,
) -> Vec<ApplicationRecord> {
    let mut by_package = BTreeMap::<String, ApplicationRecord>::new();
    for name in parse_package_names(system) {
        if let Ok(package) = PackageName::new(name) {
            by_package
                .entry(package.as_str().to_owned())
                .or_insert_with(|| ApplicationRecord {
                    package,
                    system: true,
                    disabled: false,
                });
        }
    }
    for name in parse_package_names(third_party) {
        if let Ok(package) = PackageName::new(name) {
            by_package
                .entry(package.as_str().to_owned())
                .or_insert_with(|| ApplicationRecord {
                    package,
                    system: false,
                    disabled: false,
                });
        }
    }
    for name in parse_package_names(disabled) {
        if let Some(record) = by_package.get_mut(name.as_str()) {
            record.disabled = true;
        }
    }
    let mut records: Vec<ApplicationRecord> = by_package.into_values().collect();
    records.sort_by(|left, right| {
        left.system
            .cmp(&right.system)
            .then_with(|| left.package.cmp(&right.package))
    });
    records
}

/// Best-effort parse of `dumpsys package <name>`.
///
/// Android's dumpsys layout varies across releases; parsing therefore keys on
/// the `key=value` fields themselves instead of fixed line positions, and
/// every field stays optional.
pub fn parse_application_details(package: &PackageName, output: &str) -> ApplicationDetails {
    let mut details = empty_details(package);
    // Skip the resolver tables and other preamble: the package section starts
    // at a `Package [<name>]` header. When the header is absent (unusual
    // builds), fall back to parsing everything.
    let scoped = match output
        .lines()
        .position(|line| line.contains(&format!("Package [{}]", package.as_str())))
    {
        Some(index) => output.lines().skip(index).collect::<Vec<_>>(),
        None => output.lines().collect::<Vec<_>>(),
    };
    let mut in_requested_permissions = false;
    for line in scoped {
        let trimmed = line.trim();
        if trimmed == "requested permissions:" {
            in_requested_permissions = true;
            continue;
        }
        if in_requested_permissions {
            if looks_like_permission(trimmed) && details.permissions.len() < MAX_PERMISSIONS {
                details.permissions.push(trimmed.to_owned());
                continue;
            }
            in_requested_permissions = false;
        }
        if let Some(value) = trimmed.strip_prefix("versionName=") {
            details.version_name = non_empty(value);
        } else if let Some(value) = trimmed.strip_prefix("firstInstallTime=") {
            details.first_install_time = non_empty(value);
        } else if let Some(value) = trimmed.strip_prefix("lastUpdateTime=") {
            details.last_update_time = non_empty(value);
        } else if let Some(value) = trimmed.strip_prefix("installerPackageName=") {
            details.installer = non_empty(value);
        } else if let Some(value) = trimmed.strip_prefix("codePath=") {
            if details.apk_path.is_none() {
                details.apk_path = non_empty(value);
            }
        } else {
            for token in trimmed.split_whitespace() {
                if let Some(value) = token.strip_prefix("versionCode=") {
                    details.version_code = details.version_code.or_else(|| value.parse().ok());
                } else if let Some(value) = token.strip_prefix("minSdk=") {
                    details.min_sdk = details.min_sdk.or_else(|| value.parse().ok());
                } else if let Some(value) = token.strip_prefix("targetSdk=") {
                    details.target_sdk = details.target_sdk.or_else(|| value.parse().ok());
                }
            }
        }
    }
    details
}

fn empty_details(package: &PackageName) -> ApplicationDetails {
    ApplicationDetails {
        package: package.clone(),
        version_name: None,
        version_code: None,
        min_sdk: None,
        target_sdk: None,
        first_install_time: None,
        last_update_time: None,
        installer: None,
        apk_path: None,
        permissions: Vec::new(),
    }
}

/// A permission line is a bare dotted identifier with no spaces or `=`.
fn looks_like_permission(line: &str) -> bool {
    !line.contains(' ') && !line.contains('=') && line.contains('.')
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Installs are streamed to the device and can legitimately take minutes on a
/// cold device or a slow Wi-Fi connection, so this command gets its own
/// generous bound instead of the transport-wide 8 second default.
const INSTALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Installs (or replaces) a package from a host APK via `adb install -r`.
pub async fn install_apk(
    executable: &std::path::Path,
    serial: &DeviceSerial,
    apk_path: &std::path::Path,
    max_output_bytes: usize,
) -> Result<(), BridgeError> {
    let output = crate::process::run_bounded(
        executable,
        vec![
            std::ffi::OsString::from("-s"),
            std::ffi::OsString::from(serial.as_str()),
            std::ffi::OsString::from("install"),
            std::ffi::OsString::from("-r"),
            std::ffi::OsString::from(apk_path.as_os_str()),
        ],
        INSTALL_TIMEOUT,
        max_output_bytes,
        max_output_bytes,
    )
    .await?;
    install_result(&output)
}

/// Judges one `adb install` run by its well-known textual result: adb prints
/// `Success` or `Failure [reason]` regardless of the process exit status.
///
/// Some devices (MIUI TVs in particular) abort the install session without a
/// reason: adb's stderr keeps the bare `failed to install <path>:` prefix with
/// nothing after the colon. Those get a dedicated message key so the UI can
/// attach actionable guidance instead of a dangling colon.
fn install_result(output: &crate::process::ProcessOutput) -> Result<(), BridgeError> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.lines().any(|line| line.trim() == "Success") {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    let reason = stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("Failure"))
        .map(str::trim)
        // `Failure []` — brackets with nothing inside are reasonless too.
        .filter(|reason| !reason.is_empty() && *reason != "[]")
        .map(str::to_owned)
        .or_else(|| {
            // `... install <path>:` — adb gave no device-side reason.
            if stderr.ends_with(':') {
                None
            } else {
                (!stderr.is_empty()).then(|| stderr.to_owned())
            }
        });
    let (message_key, detail) = match reason {
        Some(reason) => ("applications.install_failed", reason),
        None => (
            "applications.install_no_reason",
            if stderr.is_empty() {
                format!("adb exited with {:?}", output.exit_code)
            } else {
                stderr.to_owned()
            },
        ),
    };
    Err(BridgeError::new(ErrorCode::AdbFailed, message_key, detail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_package_lists_into_ordered_records() {
        let third_party = "package:com.example.app\npackage:org.mozilla.firefox\n";
        let system = "package:com.android.settings\n";
        let disabled = "package:org.mozilla.firefox\npackage:com.android.unknown\n";

        let records = parse_applications(third_party, system, disabled);

        assert_eq!(records.len(), 3);
        // Third-party first, alphabetical; system group last.
        assert_eq!(records[0].package.as_str(), "com.example.app");
        assert!(!records[0].system);
        assert_eq!(records[1].package.as_str(), "org.mozilla.firefox");
        assert!(records[1].disabled);
        assert_eq!(records[2].package.as_str(), "com.android.settings");
        assert!(records[2].system);
    }

    #[test]
    fn updated_system_apps_keep_their_system_classification() {
        // Updated system apps can show up in `-3` on some builds; `-s` wins.
        let records = parse_applications(
            "package:com.android.webview\n",
            "package:com.android.webview\n",
            "",
        );
        assert_eq!(records.len(), 1);
        assert!(records[0].system);
    }

    #[test]
    fn package_list_parser_ignores_noise_lines() {
        assert!(parse_package_names("package:com.a\nnot a package line\n").len() == 1);
        assert!(parse_package_names("").is_empty());
    }

    #[test]
    fn parses_realistic_dumpsys_package_output() {
        let package = PackageName::new("com.example.app").expect("valid package");
        let output = "DUMP OF SERVICE package:\n  Activity Resolver Table:\n    \
            Schemes:\n      https:\n  Packages:\n    Package [com.example.app] (12ab):\n      \
            userId=10086\n      pkgFlags=[ HAS_CODE ALLOW_CLEAR_USER_DATA ]\n      \
            versionCode=1234567 minSdk=24 targetSdk=34\n      versionName=2.3.1\n      \
            firstInstallTime=2023-11-02 10:14:33\n      lastUpdateTime=2024-05-11 08:02:10\n      \
            installerPackageName=com.android.vending\n      \
            codePath=/data/app/~~abc/com.example.app-xyz/base.apk\n      \
            pkg=PackageInfo{1234 com.example.app}\n      \
            requested permissions:\n        android.permission.INTERNET\n        \
            android.permission.CAMERA\n      install permissions:\n        \
            android.permission.INTERNET granted=true\n";

        let details = parse_application_details(&package, output);

        assert_eq!(details.package, package);
        assert_eq!(details.version_name.as_deref(), Some("2.3.1"));
        assert_eq!(details.version_code, Some(1_234_567));
        assert_eq!(details.min_sdk, Some(24));
        assert_eq!(details.target_sdk, Some(34));
        assert_eq!(
            details.first_install_time.as_deref(),
            Some("2023-11-02 10:14:33")
        );
        assert_eq!(details.installer.as_deref(), Some("com.android.vending"));
        assert_eq!(
            details.apk_path.as_deref(),
            Some("/data/app/~~abc/com.example.app-xyz/base.apk")
        );
        assert_eq!(
            details.permissions,
            vec![
                "android.permission.INTERNET".to_owned(),
                "android.permission.CAMERA".to_owned()
            ]
        );
    }

    #[test]
    fn details_parser_stays_empty_on_unrecognized_output() {
        let package = PackageName::new("com.example.app").expect("valid package");
        let details = parse_application_details(&package, "DUMP OF SERVICE package:\n(empty)\n");
        assert_eq!(details, empty_details(&package));
    }

    fn install_output(
        stdout: &str,
        stderr: &str,
        exit_code: Option<i32>,
    ) -> crate::process::ProcessOutput {
        crate::process::ProcessOutput {
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
            exit_code,
        }
    }

    #[test]
    fn install_accepts_success_line() {
        let output = install_output("Performing Streamed Install\nSuccess\n", "", Some(0));
        assert!(install_result(&output).is_ok());
    }

    #[test]
    fn install_surfaces_the_failure_reason() {
        let output = install_output(
            "Performing Streamed Install\nFailure [INSTALL_FAILED_ALREADY_EXISTS]\n",
            "",
            Some(1),
        );
        let error = install_result(&output).expect_err("must fail");
        assert_eq!(error.message_key, "applications.install_failed");
        assert!(error.detail.contains("INSTALL_FAILED_ALREADY_EXISTS"));
    }

    #[test]
    fn install_falls_back_to_stderr_then_exit_code() {
        let silent = install_output("", "", Some(1));
        assert!(install_result(&silent).is_err());
        let noisy = install_output("", "adb: no devices/emulators found", Some(1));
        let error = install_result(&noisy).expect_err("must fail");
        assert!(error.detail.contains("no devices"));
    }

    #[test]
    fn install_without_a_reason_gets_the_dedicated_key() {
        // MIUI-style abort: adb keeps the stderr prefix but no reason follows
        // the colon (the exact output from the field report).
        let output = install_output(
            "Performing Streamed Install\n",
            "adb.exe: failed to install D:\\Users\\x\\app.apk:",
            Some(1),
        );
        let error = install_result(&output).expect_err("must fail");
        assert_eq!(error.message_key, "applications.install_no_reason");
        assert!(error.detail.contains("failed to install"));

        // A bare `Failure` line with empty brackets is equally reasonless.
        let empty_brackets = install_output("Failure []\n", "", Some(1));
        let error = install_result(&empty_brackets).expect_err("must fail");
        assert_eq!(error.message_key, "applications.install_no_reason");
        assert!(error.detail.contains("exited with"));
    }

    /// Real-device acceptance: pulls an already-installed APK and reinstalls
    /// it in place. Ignored by default; run with `-- --ignored` plus
    /// `FADB_REAL_DEVICE_TEST=1` and an online device.
    #[tokio::test]
    #[ignore = "requires a real adb device"]
    async fn install_roundtrip_reinstalls_a_pulled_apk() {
        use crate::AdbTransport;
        if std::env::var_os("FADB_REAL_DEVICE_TEST").is_none() {
            return;
        }
        let executable = std::env::var_os("FADB_ADB")
            .expect("FADB_ADB must point at adb")
            .into();
        let transport = crate::ProcessAdbTransport::new(executable);
        let serial = transport
            .list_devices()
            .await
            .expect("device list")
            .into_iter()
            .find(|device| device.state.is_online())
            .expect("an online device")
            .serial;
        let remote = transport
            .shell(&serial, &["pm", "path", "com.android.browser"])
            .await
            .expect("package path");
        let apk = remote
            .lines()
            .find_map(|line| line.trim().strip_prefix("package:"))
            .expect("a package: line")
            .to_owned();
        let local = std::env::temp_dir().join("fadb-install-test.apk");
        let remote_path = fadb_domain::RemotePath::new(apk).expect("valid remote path");
        transport
            .pull_file(
                &serial,
                &remote_path,
                &local,
                fadb_domain::OverwritePolicy::ReplaceConfirmed,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .expect("pull apk");
        transport
            .install_apk(&serial, &local)
            .await
            .expect("reinstall succeeds");
        let _ = std::fs::remove_file(&local);
    }
}
