//! Streaming native-FLAC output for offline audio tools.
//!
//! FLAC cannot preserve the model's IEEE `f32` samples bit-for-bit. Valle therefore uses one
//! explicit, deterministic interchange policy: clamp finite samples to `[-1, 1]`, quantize to
//! signed 24-bit PCM with round-to-nearest (halfway values away from zero), and then encode that
//! integer stream losslessly. The writer retains at most one codec frame between calls.

use std::{fs::File, io::Read, path::Path};

use super::ff;
use anyhow::{Context, Result, anyhow, ensure};
use ff::{ChannelLayout, Packet, Rational, codec, encoder, format, frame, media};

use super::ffi::ffmpeg_init;
use crate::frame::AudioBuffer;

const FLAC_CONTAINER: &str = "flac";
const FLAC_MAX_FRAMES: u64 = (1_u64 << 36) - 1;
const PCM24_SCALE: f64 = 8_388_608.0;
const PCM24_MIN: i32 = -8_388_608;
const PCM24_MAX: i32 = 8_388_607;

/// PCM precision used by [`FlacPcm24Writer`].
pub const FLAC_BITS_PER_SAMPLE: u8 = 24;

/// A bounded, streaming, native-FLAC writer backed by the process-local libav build.
pub struct FlacPcm24Writer {
    output: format::context::Output,
    encoder: encoder::Audio,
    stream_index: usize,
    encoder_time_base: Rational,
    stream_time_base: Rational,
    sample_rate: u32,
    channels: u16,
    codec_frame_samples: usize,
    pending: Vec<i32>,
    frames_written: u64,
    frames_submitted: u64,
}

impl FlacPcm24Writer {
    /// Create a native FLAC stream. This fails closed when the linked libav build has no FLAC
    /// encoder; callers must not claim FLAC support after such a failure.
    pub fn create(path: &Path, sample_rate: u32, channels: u16) -> Result<Self> {
        ensure!(
            sample_rate > 0,
            "FLAC sample rate must be greater than zero"
        );
        ensure!(channels > 0, "FLAC channel count must be greater than zero");
        ensure!(
            sample_rate <= i32::MAX as u32,
            "FLAC sample rate is too large"
        );

        ffmpeg_init()?;
        let flac_codec = encoder::find(codec::Id::FLAC)
            .ok_or_else(|| anyhow!("the installed FFmpeg has no FLAC encoder; inspect it with `valle media capabilities`"))?;
        let encoder_time_base = Rational(1, sample_rate as i32);
        // The staging path keeps the user's `.flac` filename, but naming the muxer explicitly also
        // makes the codec/container choice independent from path probing.
        let mut output = format::output_as(path, FLAC_CONTAINER)
            .with_context(|| format!("create FLAC container {}", path.display()))?;
        let global_header = output
            .format()
            .flags()
            .contains(format::Flags::GLOBAL_HEADER);

        let channel_layout = ChannelLayout::default(i32::from(channels));
        let mut builder = codec::context::Context::new_with_codec(flac_codec)
            .encoder()
            .audio()
            .context("create FLAC encoder context")?;
        builder.set_rate(sample_rate as i32);
        builder.set_channel_layout(channel_layout);
        builder.set_format(format::Sample::I32(format::sample::Type::Packed));
        builder.set_time_base(encoder_time_base);
        builder.set_compression(Some(5));
        if global_header {
            builder.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        // AV_SAMPLE_FMT_S32 is a Q31 carrier. bits_per_raw_sample=24 tells the FLAC encoder that
        // the low eight bits are padding and makes the STREAMINFO precision explicit.
        unsafe {
            (*builder.as_mut_ptr()).bits_per_raw_sample = i32::from(FLAC_BITS_PER_SAMPLE);
        }
        let encoder = builder
            .open_as(flac_codec)
            .context("open native FLAC encoder")?;
        ensure!(
            encoder.format() == format::Sample::I32(format::sample::Type::Packed),
            "FLAC encoder changed the requested s32 carrier format to {}",
            encoder.format().name()
        );
        let codec_frame_samples = usize::try_from(encoder.frame_size())
            .context("FLAC codec frame size exceeds addressable memory")?;
        ensure!(
            codec_frame_samples > 0,
            "FLAC encoder reported a zero frame size"
        );

        let stream_index = {
            let mut stream = output
                .add_stream(flac_codec)
                .context("add FLAC audio stream")?;
            let stream_index = stream.index();
            stream.set_parameters(&encoder);
            stream.set_time_base(encoder_time_base);
            stream_index
        };
        output.write_header().context("write FLAC header")?;
        let stream_time_base = output
            .stream(stream_index)
            .ok_or_else(|| anyhow!("FLAC output stream disappeared after writing its header"))?
            .time_base();
        let pending_capacity = codec_frame_samples
            .checked_mul(usize::from(channels))
            .ok_or_else(|| anyhow!("FLAC codec frame sample count overflow"))?;

        Ok(Self {
            output,
            encoder,
            stream_index,
            encoder_time_base,
            stream_time_base,
            sample_rate,
            channels,
            codec_frame_samples,
            pending: Vec::with_capacity(pending_capacity),
            frames_written: 0,
            frames_submitted: 0,
        })
    }

    /// Quantize and stream one interleaved f32 buffer without retaining the complete signal.
    pub fn write(&mut self, buffer: &AudioBuffer) -> Result<()> {
        ensure!(
            buffer.sample_rate == self.sample_rate && buffer.channels == self.channels,
            "audio buffer {}Hz/{}ch does not match FLAC {}Hz/{}ch",
            buffer.sample_rate,
            buffer.channels,
            self.sample_rate,
            self.channels
        );
        ensure!(
            buffer
                .samples
                .len()
                .is_multiple_of(usize::from(self.channels)),
            "audio buffer is not an integral number of interleaved frames"
        );
        let new_frames = self
            .frames_written
            .checked_add(buffer.frames() as u64)
            .ok_or_else(|| anyhow!("FLAC frame count overflow"))?;
        ensure!(
            new_frames <= FLAC_MAX_FRAMES,
            "FLAC total sample count exceeds its 36-bit STREAMINFO field"
        );

        let frame_sample_capacity = self
            .codec_frame_samples
            .checked_mul(usize::from(self.channels))
            .ok_or_else(|| anyhow!("FLAC codec frame sample count overflow"))?;
        for &sample in &buffer.samples {
            self.pending.push(quantize_f32_to_flac_s32(sample)?);
            if self.pending.len() == frame_sample_capacity {
                let samples =
                    std::mem::replace(&mut self.pending, Vec::with_capacity(frame_sample_capacity));
                self.submit(&samples)?;
            }
        }
        self.frames_written = new_frames;
        Ok(())
    }

    /// Flush the final short frame, the encoder, and the native FLAC trailer.
    pub fn finish(mut self) -> Result<u64> {
        if !self.pending.is_empty() {
            let samples = std::mem::take(&mut self.pending);
            self.submit(&samples)?;
        }
        ensure!(
            self.frames_submitted == self.frames_written,
            "FLAC writer submitted {} frames but accepted {}",
            self.frames_submitted,
            self.frames_written
        );
        self.encoder.send_eof().context("flush FLAC encoder")?;
        self.drain_encoder(true)?;
        self.output.write_trailer().context("write FLAC trailer")?;
        Ok(self.frames_written)
    }

    fn submit(&mut self, samples: &[i32]) -> Result<()> {
        let channels = usize::from(self.channels);
        ensure!(
            samples.len().is_multiple_of(channels),
            "FLAC encoder input is not an integral number of frames"
        );
        let frames = samples.len() / channels;
        ensure!(frames > 0, "cannot submit an empty FLAC frame");
        let layout = ChannelLayout::default(i32::from(self.channels));
        let mut frame = frame::Audio::new(
            format::Sample::I32(format::sample::Type::Packed),
            frames,
            layout,
        );
        frame.set_rate(self.sample_rate);
        frame.set_pts(Some(
            i64::try_from(self.frames_submitted).context("FLAC PTS exceeds i64")?,
        ));
        frame.plane_mut::<i32>(0).copy_from_slice(samples);
        self.encoder
            .send_frame(&frame)
            .context("send PCM frame to FLAC encoder")?;
        self.frames_submitted = self
            .frames_submitted
            .checked_add(frames as u64)
            .ok_or_else(|| anyhow!("FLAC submitted frame count overflow"))?;
        self.drain_encoder(false)
    }

    fn drain_encoder(&mut self, require_eof: bool) -> Result<()> {
        loop {
            let mut packet = Packet::empty();
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => {
                    packet.set_stream(self.stream_index);
                    packet.rescale_ts(self.encoder_time_base, self.stream_time_base);
                    packet
                        .write_interleaved(&mut self.output)
                        .context("write FLAC packet")?;
                }
                Err(ff::Error::Other { errno }) if errno == ff::error::EAGAIN => {
                    ensure!(!require_eof, "FLAC encoder requested input after EOF");
                    return Ok(());
                }
                Err(ff::Error::Eof) => return Ok(()),
                Err(error) => return Err(anyhow!("receive FLAC packet: {error:?}")),
            }
        }
    }
}

/// Reopen and strictly validate a canonical FLAC file written by [`FlacPcm24Writer`].
///
/// Validation checks native STREAMINFO, codec identity, the exact decoded EOF frame count, every
/// decoded frame's format, and that the final demuxed FLAC frame ends at the filesystem EOF. The
/// last check rejects unaccounted trailing bytes instead of treating a decodable prefix as success.
pub fn validate_flac_pcm24(
    path: &Path,
    expected_sample_rate: u32,
    expected_channels: u16,
    expected_frames: u64,
) -> Result<()> {
    ensure!(
        expected_sample_rate > 0,
        "expected FLAC rate must be positive"
    );
    ensure!(
        expected_channels > 0,
        "expected FLAC channels must be positive"
    );
    let file_len = File::open(path)
        .with_context(|| format!("reopen FLAC {}", path.display()))?
        .metadata()
        .with_context(|| format!("read FLAC metadata {}", path.display()))?
        .len();
    let streaminfo = read_streaminfo(path)?;
    ensure!(
        streaminfo.sample_rate == expected_sample_rate,
        "FLAC STREAMINFO sample-rate mismatch: found {}, expected {expected_sample_rate}",
        streaminfo.sample_rate
    );
    ensure!(
        streaminfo.channels == expected_channels,
        "FLAC STREAMINFO channel mismatch: found {}, expected {expected_channels}",
        streaminfo.channels
    );
    ensure!(
        streaminfo.bits_per_sample == FLAC_BITS_PER_SAMPLE,
        "FLAC precision mismatch: found {} bits, expected {FLAC_BITS_PER_SAMPLE}",
        streaminfo.bits_per_sample
    );
    ensure!(
        streaminfo.total_frames == expected_frames,
        "FLAC STREAMINFO frame mismatch: found {}, expected {expected_frames}",
        streaminfo.total_frames
    );

    ffmpeg_init()?;
    let mut input = format::input(&path)
        .with_context(|| format!("open staged FLAC {} with libav", path.display()))?;
    ensure!(
        input.streams().count() == 1,
        "canonical FLAC output must contain exactly one stream"
    );
    let (stream_index, parameters) = {
        let stream = input
            .streams()
            .best(media::Type::Audio)
            .ok_or_else(|| anyhow!("FLAC has no audio stream"))?;
        (stream.index(), stream.parameters())
    };
    ensure!(
        parameters.id() == codec::Id::FLAC,
        "staged output codec is {:?}, expected FLAC",
        parameters.id()
    );
    let mut decoder = codec::context::Context::from_parameters(parameters)
        .context("create FLAC decoder context")?
        .decoder()
        .audio()
        .context("open FLAC decoder")?;
    ensure!(
        decoder.rate() == expected_sample_rate,
        "decoded FLAC sample-rate mismatch: found {}, expected {expected_sample_rate}",
        decoder.rate()
    );
    ensure!(
        decoder.channels() == expected_channels,
        "decoded FLAC channel mismatch: found {}, expected {expected_channels}",
        decoder.channels()
    );

    let mut decoded_frames = 0_u64;
    let mut final_packet_end = None;
    loop {
        let mut packet = Packet::empty();
        match packet.read(&mut input) {
            Ok(()) => {
                ensure!(
                    packet.stream() == stream_index,
                    "canonical FLAC output contains an unexpected packet stream"
                );
                ensure!(!packet.is_corrupt(), "FLAC demuxer marked a packet corrupt");
                let position = packet.position();
                ensure!(position >= 0, "FLAC packet has no filesystem position");
                let end = u64::try_from(position)
                    .context("FLAC packet position exceeds u64")?
                    .checked_add(packet.size() as u64)
                    .ok_or_else(|| anyhow!("FLAC packet end overflow"))?;
                ensure!(
                    end <= file_len,
                    "FLAC packet ends beyond the filesystem length"
                );
                final_packet_end = Some(end);
                decoder
                    .send_packet(&packet)
                    .context("send staged FLAC packet to decoder")?;
                drain_decoded_frames(
                    &mut decoder,
                    expected_sample_rate,
                    expected_channels,
                    &mut decoded_frames,
                    false,
                )?;
            }
            Err(ff::Error::Eof) => break,
            Err(error) => return Err(anyhow!("read staged FLAC packet: {error:?}")),
        }
    }
    decoder.send_eof().context("flush staged FLAC decoder")?;
    drain_decoded_frames(
        &mut decoder,
        expected_sample_rate,
        expected_channels,
        &mut decoded_frames,
        true,
    )?;
    ensure!(
        decoded_frames == expected_frames,
        "decoded FLAC frame mismatch: found {decoded_frames}, expected {expected_frames}"
    );
    let final_packet_end =
        final_packet_end.ok_or_else(|| anyhow!("FLAC contains no audio data"))?;
    ensure!(
        final_packet_end == file_len,
        "FLAC has unaccounted trailing bytes: last frame ends at {final_packet_end}, file has {file_len} bytes"
    );
    Ok(())
}

fn drain_decoded_frames(
    decoder: &mut ff::decoder::Audio,
    expected_sample_rate: u32,
    expected_channels: u16,
    decoded_frames: &mut u64,
    require_eof: bool,
) -> Result<()> {
    loop {
        let mut decoded = frame::Audio::empty();
        match decoder.receive_frame(&mut decoded) {
            Ok(()) => {
                ensure!(
                    decoded.rate() == expected_sample_rate,
                    "decoded FLAC frame changed sample rate to {}",
                    decoded.rate()
                );
                ensure!(
                    decoded.channels() == expected_channels,
                    "decoded FLAC frame changed channel count to {}",
                    decoded.channels()
                );
                *decoded_frames = decoded_frames
                    .checked_add(decoded.samples() as u64)
                    .ok_or_else(|| anyhow!("decoded FLAC frame count overflow"))?;
            }
            Err(ff::Error::Other { errno }) if errno == ff::error::EAGAIN => {
                ensure!(!require_eof, "FLAC decoder requested input after EOF");
                return Ok(());
            }
            Err(ff::Error::Eof) => return Ok(()),
            Err(error) => return Err(anyhow!("decode staged FLAC frame: {error:?}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FlacStreamInfo {
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u8,
    total_frames: u64,
}

fn read_streaminfo(path: &Path) -> Result<FlacStreamInfo> {
    // magic (4) + metadata block header (4) + STREAMINFO payload (34)
    let mut bytes = [0_u8; 42];
    File::open(path)
        .with_context(|| format!("reopen FLAC {}", path.display()))?
        .read_exact(&mut bytes)
        .with_context(|| format!("read FLAC STREAMINFO {}", path.display()))?;
    ensure!(&bytes[0..4] == b"fLaC", "native FLAC signature is missing");
    ensure!(
        bytes[4] & 0x7f == 0,
        "the first FLAC metadata block must be STREAMINFO"
    );
    let block_len = (u32::from(bytes[5]) << 16) | (u32::from(bytes[6]) << 8) | u32::from(bytes[7]);
    ensure!(block_len == 34, "FLAC STREAMINFO must be 34 bytes");
    let packed = u64::from_be_bytes(bytes[18..26].try_into().expect("fixed STREAMINFO slice"));
    Ok(FlacStreamInfo {
        sample_rate: (packed >> 44) as u32,
        channels: (((packed >> 41) & 0x7) + 1) as u16,
        bits_per_sample: (((packed >> 36) & 0x1f) + 1) as u8,
        total_frames: packed & FLAC_MAX_FRAMES,
    })
}

fn quantize_f32_to_flac_s32(sample: f32) -> Result<i32> {
    ensure!(sample.is_finite(), "cannot write non-finite FLAC sample");
    let quantized = if sample <= -1.0 {
        PCM24_MIN
    } else if sample >= 1.0 {
        PCM24_MAX
    } else {
        ((f64::from(sample) * PCM24_SCALE).round() as i32).clamp(PCM24_MIN, PCM24_MAX)
    };
    Ok(quantized.saturating_mul(1 << (32 - FLAC_BITS_PER_SAMPLE)))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::{Seek, SeekFrom, Write};

    use super::*;
    use crate::codec::LibavAudioStream;

    #[test]
    fn pcm24_quantization_clamps_and_rounds_halfway_away_from_zero() {
        let half_lsb = (0.5 / PCM24_SCALE) as f32;
        assert_eq!(quantize_f32_to_flac_s32(half_lsb).unwrap(), 1 << 8);
        assert_eq!(quantize_f32_to_flac_s32(-half_lsb).unwrap(), -(1 << 8));
        assert_eq!(quantize_f32_to_flac_s32(2.0).unwrap(), PCM24_MAX << 8);
        assert_eq!(quantize_f32_to_flac_s32(-2.0).unwrap(), i32::MIN);
        assert!(quantize_f32_to_flac_s32(f32::NAN).is_err());
    }

    pub(crate) fn streaming_flac_roundtrip_preserves_length_and_pcm24_precision() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("audio.flac");
        let samples = (0..10_037)
            .map(|index| ((index as f32 * 0.021).sin() * 0.95).clamp(-1.0, 1.0))
            .collect::<Vec<_>>();
        let mut writer = FlacPcm24Writer::create(&path, 48_000, 1).unwrap();
        for chunk in samples.chunks(997) {
            writer
                .write(&AudioBuffer {
                    samples: chunk.to_vec(),
                    sample_rate: 48_000,
                    channels: 1,
                })
                .unwrap();
            assert!(
                writer.pending.len() < writer.codec_frame_samples * usize::from(writer.channels),
                "writer retained more than one FLAC codec frame"
            );
        }
        assert_eq!(writer.finish().unwrap(), samples.len() as u64);
        validate_flac_pcm24(&path, 48_000, 1, samples.len() as u64).unwrap();

        let mut decoder = LibavAudioStream::open(&path, 48_000, 1).unwrap();
        let mut decoded = Vec::new();
        loop {
            let chunk = decoder.read(311).unwrap();
            if chunk.frames() == 0 {
                break;
            }
            decoded.extend_from_slice(&chunk.samples);
        }
        assert_eq!(decoded.len(), samples.len());
        let maximum_error = decoded
            .iter()
            .zip(&samples)
            .map(|(actual, expected)| (actual - expected).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            maximum_error <= 1.0 / 8_388_608.0,
            "PCM24 quantization error {maximum_error} exceeded one least-significant bit"
        );
    }

    pub(crate) fn strict_validation_rejects_truncated_and_trailing_data() {
        let root = tempfile::tempdir().unwrap();
        let valid = root.path().join("valid.flac");
        let mut writer = FlacPcm24Writer::create(&valid, 48_000, 1).unwrap();
        writer
            .write(&AudioBuffer {
                samples: vec![0.25; 8_000],
                sample_rate: 48_000,
                channels: 1,
            })
            .unwrap();
        writer.finish().unwrap();

        let truncated = root.path().join("truncated.flac");
        std::fs::copy(&valid, &truncated).unwrap();
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&truncated)
            .unwrap();
        file.set_len(file.metadata().unwrap().len() - 1).unwrap();
        assert!(validate_flac_pcm24(&truncated, 48_000, 1, 8_000).is_err());

        let trailing = root.path().join("trailing.flac");
        std::fs::copy(&valid, &trailing).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&trailing)
            .unwrap();
        file.seek(SeekFrom::End(0)).unwrap();
        file.write_all(b"not-a-flac-frame").unwrap();
        file.sync_all().unwrap();
        assert!(validate_flac_pcm24(&trailing, 48_000, 1, 8_000).is_err());
    }
}
