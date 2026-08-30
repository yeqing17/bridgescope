//! MP4 recording of the mirror's H.264 elementary stream.
//!
//! The recorder taps the demux loop: it harvests SPS/PPS from config and
//! key-frame packets, starts the file at the first decodable key frame, and
//! stores one access unit per MP4 sample. scrcpy timestamps are microseconds,
//! so the MP4 timescale matches them one to one and sample durations are
//! plain PTS deltas. Parameter-set changes mid-recording (e.g. rotation) are
//! ignored — the file keeps the parameters it started with.

use std::fs::File;
use std::path::{Path, PathBuf};

use mp4::Mp4Writer;

/// Presentation timescale matching scrcpy's microsecond PTS, so durations
/// translate without rescaling.
const TIMESCALE: u32 = 1_000_000;
/// Duration (in timescale units) for a recording that holds a single sample;
/// the next packet normally fixes the real duration before this is used.
const SINGLE_SAMPLE_DURATION: u32 = 33_333;
/// NAL unit types carrying the H.264 parameter sets.
const SPS_NAL_TYPE: u8 = 7;
const PPS_NAL_TYPE: u8 = 8;
/// The recorder writes one video track; `add_track` assigns id 1 to it.
const TRACK_ID: u32 = 1;

/// What [`VideoRecorder::finish`] produced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecorderOutcome {
    /// The file was finalized with this many samples (frames).
    Written(u64),
    /// Stopped before any key frame arrived; no file was created.
    Empty,
    /// A write failed along the way; the file is incomplete.
    Failed(String),
}

/// A sample waiting for the next PTS to learn its duration.
struct PendingSample {
    start_time: u64,
    is_sync: bool,
    bytes: Vec<u8>,
}

/// Writes the mirrored H.264 stream into an MP4 file.
pub struct VideoRecorder {
    path: PathBuf,
    width: u32,
    height: u32,
    writer: Option<Mp4Writer<File>>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    /// PTS of the first kept sample; sample start times are relative to it.
    base_pts: Option<u64>,
    pending: Option<PendingSample>,
    /// Duration of the most recently closed sample, reused for the final one.
    last_delta: Option<u64>,
    samples: u64,
    /// First write failure; the recorder keeps draining packets but reports
    /// this when the recording is finalized.
    failure: Option<String>,
    /// Set once `finish` ran; `Drop` uses it to know the file is abandoned.
    finalized: bool,
}

impl VideoRecorder {
    /// Prepares a recorder for `path`; the file is created lazily, when the
    /// first decodable key frame arrives.
    pub fn new(path: PathBuf, width: u32, height: u32) -> Self {
        Self {
            path,
            width,
            height,
            writer: None,
            sps: None,
            pps: None,
            base_pts: None,
            pending: None,
            last_delta: None,
            samples: 0,
            failure: None,
            finalized: false,
        }
    }

    /// Destination the file will be written to (valid even before it exists).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Feeds one demuxed packet. Config packets only refresh the harvested
    /// parameter sets; samples start at the first key frame, everything
    /// before it is dropped.
    pub fn feed(&mut self, config: bool, key_frame: bool, pts: u64, payload: &[u8]) {
        if self.failure.is_some() {
            return;
        }
        for unit in openh264::nal_units(payload) {
            let unit = strip_start_code(unit);
            match nal_type(unit) {
                Some(t) if t == SPS_NAL_TYPE => self.sps = Some(unit.to_vec()),
                Some(t) if t == PPS_NAL_TYPE => self.pps = Some(unit.to_vec()),
                _ => {}
            }
        }
        if self.writer.is_none() {
            // Recording can only start at a decodable point: a key frame with
            // both parameter sets already seen.
            if !key_frame || self.sps.is_none() || self.pps.is_none() {
                return;
            }
            if let Err(error) = self.open(pts) {
                self.failure = Some(error);
                return;
            }
        }
        // Config packets carry only parameter sets, which are harvested
        // above; they never become samples.
        if config {
            return;
        }
        let bytes = avcc(payload);
        let start_time = pts - self.base_pts.unwrap_or_default();
        // Close out the previous sample: it ran until this one's PTS.
        if let Some(pending) = self.pending.take() {
            let delta = start_time.saturating_sub(pending.start_time).max(1);
            self.last_delta = Some(delta);
            self.write_pending(pending, duration_units(delta));
        }
        self.pending = Some(PendingSample {
            start_time,
            is_sync: key_frame,
            bytes,
        });
    }

    /// Closes the file and reports what happened. Consumes the pending
    /// sample; the recorder must not be fed afterwards.
    pub fn finish(&mut self) -> RecorderOutcome {
        self.finalized = true;
        if let Some(failure) = self.failure.take() {
            return RecorderOutcome::Failed(failure);
        }
        if self.writer.is_none() {
            return RecorderOutcome::Empty;
        }
        let duration = self
            .last_delta
            .map_or(SINGLE_SAMPLE_DURATION, duration_units);
        if let Some(pending) = self.pending.take() {
            self.write_pending(pending, duration);
        }
        if let Some(failure) = self.failure.take() {
            return RecorderOutcome::Failed(failure);
        }
        let samples = self.samples;
        match self.writer.as_mut().expect("writer exists").write_end() {
            Ok(()) => RecorderOutcome::Written(samples),
            Err(error) => RecorderOutcome::Failed(error.to_string()),
        }
    }

    /// Creates the file and the single video track; only called once, from
    /// `feed`, at the first decodable key frame.
    fn open(&mut self, first_pts: u64) -> Result<(), String> {
        let file = File::create(&self.path).map_err(|error| error.to_string())?;
        let config = mp4::Mp4Config {
            major_brand: mp4::FourCC::from(*b"isom"),
            minor_version: 0,
            compatible_brands: vec![
                mp4::FourCC::from(*b"isom"),
                mp4::FourCC::from(*b"iso2"),
                mp4::FourCC::from(*b"avc1"),
                mp4::FourCC::from(*b"mp41"),
            ],
            timescale: TIMESCALE,
        };
        let mut writer =
            Mp4Writer::write_start(file, &config).map_err(|error| error.to_string())?;
        // Not `TrackConfig::from(AvcConfig)`: that hardcodes a millisecond
        // timescale, while our samples are stamped in microseconds.
        writer
            .add_track(&mp4::TrackConfig {
                track_type: mp4::TrackType::Video,
                timescale: TIMESCALE,
                language: String::from("und"),
                media_conf: mp4::MediaConfig::AvcConfig(mp4::AvcConfig {
                    width: u16::try_from(self.width).unwrap_or(u16::MAX),
                    height: u16::try_from(self.height).unwrap_or(u16::MAX),
                    seq_param_set: self.sps.clone().expect("sps harvested before open"),
                    pic_param_set: self.pps.clone().expect("pps harvested before open"),
                }),
            })
            .map_err(|error| error.to_string())?;
        self.base_pts = Some(first_pts);
        self.writer = Some(writer);
        Ok(())
    }

    fn write_pending(&mut self, sample: PendingSample, duration: u32) {
        let Some(writer) = self.writer.as_mut() else {
            return;
        };
        let mp4_sample = mp4::Mp4Sample {
            start_time: sample.start_time,
            duration,
            rendering_offset: 0,
            is_sync: sample.is_sync,
            bytes: mp4::Bytes::from(sample.bytes),
        };
        match writer.write_sample(TRACK_ID, &mp4_sample) {
            Ok(()) => self.samples += 1,
            Err(error) => {
                if self.failure.is_none() {
                    self.failure = Some(error.to_string());
                }
            }
        }
    }
}

impl Drop for VideoRecorder {
    fn drop(&mut self) {
        // A recorder dropped without `finish` leaves a broken file (no moov
        // box) behind: close the handle, then remove it.
        self.writer.take();
        if !self.finalized {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// NAL header type nibble; `None` for empty units.
fn nal_type(unit: &[u8]) -> Option<u8> {
    unit.first().map(|header| header & 0x1F)
}

/// `nal_units` keeps the Annex B start codes; AVCC length-prefixing wants the
/// bare unit.
fn strip_start_code(unit: &[u8]) -> &[u8] {
    if unit.starts_with(&[0, 0, 0, 1]) {
        &unit[4..]
    } else if unit.starts_with(&[0, 0, 1]) {
        &unit[3..]
    } else {
        unit
    }
}

/// Converts one Annex B access unit into an AVCC sample body: every NAL unit
/// prefixed with its big-endian byte length.
fn avcc(payload: &[u8]) -> Vec<u8> {
    let mut sample = Vec::with_capacity(payload.len() + 16);
    for unit in openh264::nal_units(payload) {
        let unit = strip_start_code(unit);
        sample.extend_from_slice(&u32::try_from(unit.len()).unwrap_or(u32::MAX).to_be_bytes());
        sample.extend_from_slice(unit);
    }
    sample
}

/// Clamps a microsecond delta into the `u32` MP4 duration field.
fn duration_units(delta: u64) -> u32 {
    u32::try_from(delta).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openh264::encoder::Encoder;
    use openh264::formats::{RgbSliceU8, YUVBuffer};

    /// Encodes one solid-color frame through the real OpenH264 encoder; the
    /// first call emits SPS/PPS ahead of the IDR, like MediaCodec does.
    #[allow(clippy::cast_possible_truncation)]
    fn encoded_frame(red: u8) -> Vec<u8> {
        const WIDTH: usize = 64;
        const HEIGHT: usize = 64;
        let rgb = vec![red; WIDTH * HEIGHT * 3];
        let yuv = YUVBuffer::from_rgb_source(RgbSliceU8::new(&rgb, (WIDTH, HEIGHT)));
        let mut encoder = Encoder::new().expect("encoder");
        encoder.encode(&yuv).expect("encode").to_vec()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bridgescope-recorder-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn records_key_and_delta_frames_into_a_readable_mp4() {
        let dir = temp_dir("valid");
        let path = dir.join("out.mp4");
        let key = encoded_frame(10);
        let delta = encoded_frame(200);
        let mut recorder = VideoRecorder::new(path.clone(), 64, 64);
        recorder.feed(false, true, 1_000, &key);
        recorder.feed(false, false, 61_000, &delta);
        recorder.feed(false, false, 91_000, &delta);
        assert_eq!(recorder.finish(), RecorderOutcome::Written(3));
        assert!(path.exists());

        let file = std::fs::File::open(&path).expect("open");
        let size = file.metadata().expect("meta").len();
        let reader = mp4::Mp4Reader::read_header(file, size).expect("readable mp4");
        assert_eq!(reader.tracks().len(), 1);
        let track = reader.tracks().values().next().expect("track");
        assert_eq!(track.sample_count(), 3);
        // Duration spans from the first sample's PTS to the last one plus
        // its hold time (60 ms + 30 ms + 30 ms).
        assert_eq!(track.duration(), std::time::Duration::from_micros(120_000));
        drop(reader);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn drops_packets_before_the_first_key_frame() {
        let dir = temp_dir("wait-keyframe");
        let path = dir.join("out.mp4");
        let key = encoded_frame(30);
        let mut recorder = VideoRecorder::new(path.clone(), 64, 64);
        // A config packet and a non-key packet arrive first: the config only
        // harvests parameter sets, the frame is dropped (nothing is written).
        recorder.feed(true, false, 0, &key);
        recorder.feed(false, false, 20_000, &key);
        recorder.feed(false, true, 40_000, &key);
        recorder.feed(false, false, 80_000, &key);
        assert_eq!(recorder.finish(), RecorderOutcome::Written(2));
        let file = std::fs::File::open(&path).expect("open");
        let size = file.metadata().expect("meta").len();
        let reader = mp4::Mp4Reader::read_header(file, size).expect("readable mp4");
        assert_eq!(
            reader
                .tracks()
                .values()
                .next()
                .expect("track")
                .sample_count(),
            2
        );
        drop(reader);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stopping_before_any_key_frame_reports_empty() {
        let dir = temp_dir("empty");
        let path = dir.join("out.mp4");
        let mut recorder = VideoRecorder::new(path.clone(), 64, 64);
        assert_eq!(recorder.finish(), RecorderOutcome::Empty);
        assert!(!path.exists(), "no file should be created without samples");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_recorder_dropped_without_finish_removes_its_file() {
        let dir = temp_dir("abandoned");
        let path = dir.join("out.mp4");
        let key = encoded_frame(5);
        let mut recorder = VideoRecorder::new(path.clone(), 64, 64);
        recorder.feed(false, true, 0, &key);
        drop(recorder);
        assert!(!path.exists(), "abandoned recordings must not leave debris");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
