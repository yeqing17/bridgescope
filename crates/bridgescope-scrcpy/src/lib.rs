use bridgescope_domain::DeviceSerial;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScrcpySessionPlan {
    pub device: DeviceSerial,
    pub max_size: Option<u32>,
    pub video_bit_rate: u32,
    pub audio: bool,
}

impl ScrcpySessionPlan {
    pub const fn protocol_3_1_defaults(device: DeviceSerial) -> Self {
        Self {
            device,
            max_size: None,
            video_bit_rate: 8_000_000,
            audio: true,
        }
    }
}

// The protocol implementation is intentionally deferred until its public source
// version, artifact hash, binary fixtures, decoder choice, and lifecycle tests
// are recorded in docs/protocol-sources.md.
