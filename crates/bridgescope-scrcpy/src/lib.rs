//! scrcpy video mirroring: session planning, wire demux, and H.264 decoding.
//!
//! The wire protocol, pinned server artifact, and decoder choice are recorded
//! in `docs/protocol-sources.md`; byte layouts follow the `doc/develop.md`
//! shipped with the pinned scrcpy 3.3.4 release (which documents the "scrcpy
//! v2.1" protocol). 0.7 mirrors video only: control injection and audio are
//! out of scope.

pub mod decoder;
pub mod protocol;
pub mod recorder;
pub mod server;
pub mod session;

use bridgescope_domain::DeviceSerial;

/// One pending mirror session for a device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScrcpySessionPlan {
    pub device: DeviceSerial,
    /// Upper bound of the shorter video side in pixels; `None` means the
    /// device's native resolution.
    pub max_size: Option<u32>,
    /// Target video bit rate in bits per second.
    pub video_bit_rate: u32,
}

impl ScrcpySessionPlan {
    /// Default cap so a 4K device does not melt host CPUs decoding into an
    /// egui texture; users can raise or disable it in the mirror panel.
    pub const DEFAULT_MAX_SIZE: u32 = 1280;
    pub const DEFAULT_BIT_RATE: u32 = 8_000_000;

    pub fn new(device: DeviceSerial) -> Self {
        Self {
            device,
            max_size: Some(Self::DEFAULT_MAX_SIZE),
            video_bit_rate: Self::DEFAULT_BIT_RATE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_cap_resolution_and_bit_rate() {
        let plan = ScrcpySessionPlan::new(DeviceSerial::new("emulator-5554").expect("serial"));
        assert_eq!(plan.max_size, Some(1280));
        assert_eq!(plan.video_bit_rate, 8_000_000);
    }
}
