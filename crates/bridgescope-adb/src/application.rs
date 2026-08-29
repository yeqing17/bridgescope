//! Package-manager queries and actions (`pm`, `am`, `monkey`).
//!
//! Every command here receives the package name as a standalone argument and
//! [`PackageName`] guarantees it cannot carry shell metacharacters, so the
//! device-side `adb shell` join is safe.

use std::collections::BTreeMap;

use bridgescope_domain::{ApplicationDetails, ApplicationRecord, PackageName};

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
}
