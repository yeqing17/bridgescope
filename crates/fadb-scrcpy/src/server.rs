//! The pinned scrcpy server artifact and the arguments Fadb passes to
//! it. Normative sources and the artifact hash are recorded in
//! `docs/protocol-sources.md`.

use crate::ScrcpySessionPlan;

/// The pinned scrcpy release; the server rejects a mismatching version.
pub const SCRCPY_VERSION: &str = "3.3.4";

/// Vendored from
/// <https://github.com/Genymobile/scrcpy/releases/download/v3.3.4/scrcpy-server-v3.3.4>
/// (Apache-2.0, redistributed unmodified; SHA-256 pinned in
/// `docs/protocol-sources.md` and asserted by a test below).
pub const SERVER_JAR: &[u8] = include_bytes!("../assets/scrcpy-server-v3.3.4");

/// Device-side location the jar is pushed to before launch.
pub const SERVER_REMOTE_PATH: &str = "/data/local/tmp/scrcpy-server.jar";

/// The abstract socket the server listens on in forward-tunnel mode: the
/// session id in lowercase hex, zero-padded to 8 digits (the server parses
/// `scid=` as radix-16 and formats the same way, verified on-device; see
/// `docs/protocol-sources.md`).
pub fn abstract_socket_name(scid: u32) -> String {
    format!("scrcpy_{scid:08x}")
}

/// Builds the arguments after `app_process / com.genymobile.scrcpy.Server`:
/// the version, then `key=value` options. Video only (`audio=false`,
/// `control=false`), forward tunnel, frame metadata on. Device metadata is
/// disabled so every byte after the documented dummy byte is covered by the
/// documented codec metadata and frame headers.
pub fn server_arguments(scid: u32, plan: &ScrcpySessionPlan) -> Vec<String> {
    let mut args = vec![
        SCRCPY_VERSION.to_owned(),
        "log_level=warn".to_owned(),
        format!("scid={scid:08x}"),
        "tunnel_forward=true".to_owned(),
        "audio=false".to_owned(),
        "control=false".to_owned(),
        "video_codec=h264".to_owned(),
        "send_device_meta=false".to_owned(),
    ];
    if let Some(max_size) = plan.max_size {
        args.push(format!("max_size={max_size}"));
    }
    // The server-side option is `video_bit_rate` (client `--video-bit-rate`,
    // doc/video.md); it has no `max_` prefix.
    args.push(format!("video_bit_rate={}", plan.video_bit_rate));
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    /// The vendored jar must stay byte-identical to the pinned release;
    /// SHA-256 recorded in docs/protocol-sources.md.
    #[allow(clippy::format_collect)]
    #[test]
    fn vendored_server_jar_matches_pinned_hash() {
        const PINNED_SHA256: &str =
            "8588238c9a5a00aa542906b6ec7e6d5541d9ffb9b5d0f6e1bc0e365e2303079e";
        let digest: String = Sha256::digest(SERVER_JAR)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(digest, PINNED_SHA256);
        assert_eq!(SERVER_JAR.len(), 90_980);
    }

    #[test]
    fn arguments_pin_video_only_forward_session() {
        let plan = ScrcpySessionPlan::new(DeviceSerial::new("emulator-5554").expect("serial"));
        let args = server_arguments(0x1234_5678, &plan);
        assert_eq!(args[0], "3.3.4");
        for required in [
            "tunnel_forward=true",
            "audio=false",
            "control=false",
            "video_codec=h264",
            "send_device_meta=false",
            "max_size=1280",
            "video_bit_rate=8000000",
        ] {
            assert!(args.contains(&required.to_owned()), "missing {required}");
        }
        // scid is hex, matching the socket name derivation.
        assert!(args.contains(&"scid=12345678".to_owned()));
    }

    #[test]
    fn arguments_omit_max_size_when_unset() {
        let mut plan = ScrcpySessionPlan::new(DeviceSerial::new("emulator-5554").expect("serial"));
        plan.max_size = None;
        let args = server_arguments(1, &plan);
        assert!(!args.iter().any(|arg| arg.starts_with("max_size=")));
    }

    use fadb_domain::DeviceSerial;

    #[test]
    fn socket_name_uses_padded_hex_scid() {
        assert_eq!(abstract_socket_name(42), "scrcpy_0000002a");
        assert_eq!(abstract_socket_name(0x1234_5678), "scrcpy_12345678");
    }
}
