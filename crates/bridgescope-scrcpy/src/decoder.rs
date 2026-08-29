//! H.264 decoding into RGBA frames via the bundled OpenH264 (BSD-2) build.

use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use openh264::nal_units;

/// One decoded frame, ready for `egui::ColorImage::from_rgba_unmultiplied`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RgbaFrame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Wraps an OpenH264 decoder; `decode` accepts MediaCodec-style packets
/// (what scrcpy sends: a full access unit per packet) and yields a frame
/// whenever the decoder completes one.
pub struct VideoDecoder {
    inner: Decoder,
}

impl VideoDecoder {
    pub fn new() -> Result<Self, openh264::Error> {
        Ok(Self {
            inner: Decoder::new()?,
        })
    }

    /// Feeds one packet (config or frame payload), splitting it into NAL
    /// units; returns the most recent decoded picture, if any.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Option<RgbaFrame>, openh264::Error> {
        let mut frame = None;
        for unit in nal_units(packet) {
            if let Some(yuv) = self.inner.decode(unit)? {
                let (width, height) = yuv.dimensions();
                let mut rgba = vec![0u8; yuv.rgba8_len()];
                yuv.write_rgba8(&mut rgba);
                frame = Some(RgbaFrame {
                    width,
                    height,
                    rgba,
                });
            }
        }
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openh264::encoder::Encoder;
    use openh264::formats::{RgbSliceU8, YUVBuffer};

    /// Encodes a 64x64 RGB gradient into H.264 and decodes it back through
    /// [`VideoDecoder`]; chroma subsampling means pixels are close, not exact.
    #[allow(clippy::cast_possible_truncation)]
    #[test]
    fn roundtrips_an_encoded_gradient() {
        const WIDTH: usize = 64;
        const HEIGHT: usize = 64;
        let mut rgb = Vec::with_capacity(WIDTH * HEIGHT * 3);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                rgb.push((x * 4) as u8);
                rgb.push((y * 4) as u8);
                rgb.push(128);
            }
        }
        let yuv = YUVBuffer::from_rgb_source(RgbSliceU8::new(&rgb, (WIDTH, HEIGHT)));
        let mut encoder = Encoder::new().expect("encoder");
        let bitstream = encoder.encode(&yuv).expect("encode").to_vec();
        assert!(!bitstream.is_empty());

        let mut decoder = VideoDecoder::new().expect("decoder");
        let frame = decoder
            .decode(&bitstream)
            .expect("decode")
            .expect("a keyframe yields a picture");
        assert_eq!(frame.width, WIDTH);
        assert_eq!(frame.height, HEIGHT);
        assert_eq!(frame.rgba.len(), WIDTH * HEIGHT * 4);

        // Top-left should stay near (0, 0, 128, 255); OpenH264 keeps RGBA
        // alpha at 255 for every pixel.
        assert!(frame.rgba[0] < 8, "r: {}", frame.rgba[0]);
        assert!(frame.rgba[1] < 8, "g: {}", frame.rgba[1]);
        assert!(frame.rgba[3] == 255, "a: {}", frame.rgba[3]);
    }
}
