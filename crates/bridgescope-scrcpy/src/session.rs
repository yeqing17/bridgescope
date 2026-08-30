//! Async stream reader: socket header (dummy byte + codec metadata) and the
//! media-packet loop. The runtime calls these on the forwarded TCP stream;
//! every read is racing a cancellation token so a stop request interrupts
//! even a stalled stream between byte chunks.

use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use crate::decoder::{RgbaFrame, VideoDecoder};
use crate::protocol::{CODEC_METADATA_BYTES, CodecMetadata, PACKET_HEADER_BYTES};

/// Everything the first socket announces before the media packets start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamHeader {
    pub metadata: CodecMetadata,
}

/// Failures while consuming the video stream.
#[derive(Debug)]
pub enum SessionError {
    /// The session was cancelled by the caller.
    Cancelled,
    /// The socket failed mid-stream.
    Io(std::io::Error),
    /// The server did not speak the documented wire format.
    Protocol(&'static str),
    /// The stream is not the negotiated codec.
    Codec { id: u32 },
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(formatter, "mirror session cancelled"),
            Self::Io(error) => write!(formatter, "mirror stream io: {error}"),
            Self::Protocol(why) => write!(formatter, "mirror protocol: {why}"),
            Self::Codec { id } => {
                write!(formatter, "mirror stream codec id 0x{id:08x} is not h264")
            }
        }
    }
}

/// Reads the forward-tunnel dummy byte and the codec metadata block.
pub async fn read_stream_header(
    reader: &mut (impl tokio::io::AsyncRead + Unpin),
) -> Result<StreamHeader, SessionError> {
    let mut dummy = [0u8; 1];
    reader
        .read_exact(&mut dummy)
        .await
        .map_err(SessionError::Io)?;
    let mut bytes = [0u8; CODEC_METADATA_BYTES];
    reader
        .read_exact(&mut bytes)
        .await
        .map_err(SessionError::Io)?;
    let metadata =
        CodecMetadata::parse(&bytes).ok_or(SessionError::Protocol("short codec metadata"))?;
    if metadata.codec_id != crate::protocol::H264_CODEC_ID {
        return Err(SessionError::Codec {
            id: metadata.codec_id,
        });
    }
    Ok(StreamHeader { metadata })
}

/// Demuxes media packets until the stream ends or the token is cancelled,
/// decoding each packet and handing the raw payload plus any decoded frame
/// to `on_packet` (the recorder taps the raw access units). Returns the
/// decoded frame count; a server-side stream end counts as a normal stop.
pub async fn demux_packets(
    reader: &mut (impl tokio::io::AsyncRead + Unpin),
    decoder: &mut VideoDecoder,
    cancellation: &CancellationToken,
    on_packet: &mut impl FnMut(&crate::protocol::PacketHeader, &[u8], Option<RgbaFrame>),
) -> Result<u64, SessionError> {
    let mut frames = 0u64;
    let mut header = [0u8; PACKET_HEADER_BYTES];
    loop {
        if !read_chunk(reader, &mut header, cancellation).await? {
            return Ok(frames);
        }
        let packet = crate::protocol::PacketHeader::parse(&header)
            .ok_or(SessionError::Protocol("short packet header"))?;
        let mut payload = vec![0u8; packet.size as usize];
        if !read_chunk(reader, &mut payload, cancellation).await? {
            return Ok(frames);
        }
        match decoder.decode(&payload) {
            Ok(frame) => {
                if frame.is_some() {
                    frames += 1;
                }
                on_packet(&packet, &payload, frame);
            }
            Err(_) => return Err(SessionError::Protocol("decoder rejected the stream")),
        }
    }
}

/// `read_exact` with cancellation-first polling; returns `false` on a clean
/// end of stream.
async fn read_chunk(
    reader: &mut (impl tokio::io::AsyncRead + Unpin),
    buffer: &mut [u8],
    cancellation: &CancellationToken,
) -> Result<bool, SessionError> {
    tokio::select! {
        biased;
        () = cancellation.cancelled() => Err(SessionError::Cancelled),
        read = reader.read_exact(buffer) => match read {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
            Err(error) => Err(SessionError::Io(error)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openh264::encoder::Encoder;
    use openh264::formats::{RgbSliceU8, YUVBuffer};

    /// Builds a wire stream: dummy byte, codec metadata, one config packet
    /// carrying a real encoded keyframe.
    #[allow(clippy::cast_possible_truncation)]
    fn fixture_stream() -> Vec<u8> {
        const WIDTH: usize = 32;
        const HEIGHT: usize = 32;
        let mut rgb = vec![0u8; WIDTH * HEIGHT * 3];
        for (index, pixel) in rgb.chunks_exact_mut(3).enumerate() {
            pixel[0] = index as u8;
            pixel[1] = 90;
            pixel[2] = 160;
        }
        let yuv = YUVBuffer::from_rgb_source(RgbSliceU8::new(&rgb, (WIDTH, HEIGHT)));
        let mut encoder = Encoder::new().expect("encoder");
        let payload = encoder.encode(&yuv).expect("encode").to_vec();

        let mut stream = Vec::new();
        stream.push(0); // forward-tunnel dummy byte
        stream.extend_from_slice(&crate::protocol::H264_CODEC_ID.to_be_bytes());
        stream.extend_from_slice(&(WIDTH as u32).to_be_bytes());
        stream.extend_from_slice(&(HEIGHT as u32).to_be_bytes());
        let flags_pts = (1u64 << 63) | (1u64 << 62) | 5;
        stream.extend_from_slice(&flags_pts.to_be_bytes());
        stream.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        stream.extend_from_slice(&payload);
        stream
    }

    fn token() -> CancellationToken {
        CancellationToken::new()
    }

    #[tokio::test]
    async fn reads_header_and_demuxes_fixture() {
        let mut stream = std::io::Cursor::new(fixture_stream());
        let header = read_stream_header(&mut stream).await.expect("header");
        assert_eq!(header.metadata.width, 32);
        assert_eq!(header.metadata.height, 32);

        let mut decoder = VideoDecoder::new().expect("decoder");
        let mut frames_seen = Vec::new();
        let mut packets = Vec::new();
        let frames = demux_packets(
            &mut stream,
            &mut decoder,
            &token(),
            &mut |packet, payload, frame| {
                packets.push((*packet, payload.len()));
                if let Some(frame) = frame {
                    frames_seen.push(frame.width);
                }
            },
        )
        .await
        .expect("demux");
        assert_eq!(frames, 1);
        assert_eq!(frames_seen, vec![32]);
        // The fixture's single packet is a config + key-frame access unit.
        assert_eq!(packets.len(), 1);
        assert!(packets[0].0.config && packets[0].0.key_frame);
        assert!(packets[0].1 > 0);
    }

    #[tokio::test]
    async fn rejects_non_h264_codec_id() {
        let mut stream = std::io::Cursor::new(vec![0; 13]);
        let error = read_stream_header(&mut stream)
            .await
            .expect_err("codec mismatch");
        assert!(matches!(error, SessionError::Codec { .. }));
    }

    #[tokio::test]
    async fn clean_eof_returns_frame_count() {
        // Valid header, then the stream ends before any packet.
        let mut bytes = vec![0u8]; // dummy
        bytes.extend_from_slice(&crate::protocol::H264_CODEC_ID.to_be_bytes());
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(&1u32.to_be_bytes());
        let mut stream = std::io::Cursor::new(bytes);
        read_stream_header(&mut stream).await.expect("header");
        let mut decoder = VideoDecoder::new().expect("decoder");
        let frames = demux_packets(&mut stream, &mut decoder, &token(), &mut |_, _, _| {})
            .await
            .expect("demux");
        assert_eq!(frames, 0);
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_stalled_stream() {
        // Header only; the demux would wait forever for a packet header.
        let mut bytes = vec![0u8];
        bytes.extend_from_slice(&crate::protocol::H264_CODEC_ID.to_be_bytes());
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(&1u32.to_be_bytes());
        let mut stream = std::io::Cursor::new(bytes);
        read_stream_header(&mut stream).await.expect("header");
        let cancellation = token();
        let mut decoder = VideoDecoder::new().expect("decoder");
        cancellation.cancel();
        let error = demux_packets(&mut stream, &mut decoder, &cancellation, &mut |_, _, _| {})
            .await
            .expect_err("cancelled");
        assert!(matches!(error, SessionError::Cancelled));
    }
}
