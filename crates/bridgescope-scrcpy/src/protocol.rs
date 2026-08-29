//! Byte-level parsing of the scrcpy video stream.
//!
//! Layouts follow `doc/develop.md` at scrcpy tag v3.3.4 (the "scrcpy v2.1"
//! wire protocol); see `docs/protocol-sources.md` for the normative links.

/// `u32::from_be_bytes(*b"h264")` — the codec id the server sends on the
/// video socket.
pub const H264_CODEC_ID: u32 = 0x6832_3634;

/// Codec metadata: codec id, width, height — three `u32` BE values.
pub const CODEC_METADATA_BYTES: usize = 12;
/// Frame header: flags+PTS (`u64` BE) then payload size (`u32` BE).
pub const PACKET_HEADER_BYTES: usize = 12;

/// Codec and video dimensions announced before the first packet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodecMetadata {
    pub codec_id: u32,
    pub width: u32,
    pub height: u32,
}

impl CodecMetadata {
    /// Parses the 12-byte codec metadata block; `None` when short.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let fixed: &[u8; CODEC_METADATA_BYTES] = bytes.try_into().ok()?;
        Some(Self {
            codec_id: u32::from_be_bytes(fixed[0..4].try_into().expect("4 bytes")),
            width: u32::from_be_bytes(fixed[4..8].try_into().expect("4 bytes")),
            height: u32::from_be_bytes(fixed[8..12].try_into().expect("4 bytes")),
        })
    }
}

/// One media-packet header. The top two bits of the 8-byte BE integer carry
/// the config-packet and key-frame flags; the low 62 bits are the PTS.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacketHeader {
    /// Carries codec configuration (SPS/PPS) instead of frame data.
    pub config: bool,
    /// First packet after a config, decodable on its own.
    pub key_frame: bool,
    /// Presentation timestamp in microseconds.
    pub pts: u64,
    pub size: u32,
}

impl PacketHeader {
    /// Parses the 12-byte packet header; `None` when short.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let fixed: &[u8; PACKET_HEADER_BYTES] = bytes.try_into().ok()?;
        let flags_pts = u64::from_be_bytes(fixed[0..8].try_into().expect("8 bytes"));
        Some(Self {
            config: (flags_pts >> 63) & 1 == 1,
            key_frame: (flags_pts >> 62) & 1 == 1,
            pts: flags_pts & ((1 << 62) - 1),
            size: u32::from_be_bytes(fixed[8..12].try_into().expect("4 bytes")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_metadata_reads_h264_and_dimensions() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&H264_CODEC_ID.to_be_bytes());
        bytes.extend_from_slice(&1080u32.to_be_bytes());
        bytes.extend_from_slice(&1920u32.to_be_bytes());
        let metadata = CodecMetadata::parse(&bytes).expect("metadata");
        assert_eq!(metadata.codec_id, u32::from_be_bytes(*b"h264"));
        assert_eq!(metadata.width, 1080);
        assert_eq!(metadata.height, 1920);
        assert!(CodecMetadata::parse(&bytes[..11]).is_none());
    }

    #[test]
    fn packet_header_extracts_flags_pts_and_size() {
        let flags_pts = (1u64 << 63) | (1u64 << 62) | 123_456;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&flags_pts.to_be_bytes());
        bytes.extend_from_slice(&65_536u32.to_be_bytes());
        let header = PacketHeader::parse(&bytes).expect("header");
        assert!(header.config);
        assert!(header.key_frame);
        assert_eq!(header.pts, 123_456);
        assert_eq!(header.size, 65_536);
    }

    #[test]
    fn packet_header_delta_frame_has_no_flags() {
        let flags_pts = ((1u64 << 62) - 1) & 42; // small PTS, no flag bits
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&flags_pts.to_be_bytes());
        bytes.extend_from_slice(&7u32.to_be_bytes());
        let header = PacketHeader::parse(&bytes).expect("header");
        assert!(!header.config);
        assert!(!header.key_frame);
        assert_eq!(header.pts, 42);
        assert_eq!(header.size, 7);
    }
}
