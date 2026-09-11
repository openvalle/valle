//! Libav audio decoding to interleaved f32 in a fixed output format. Random-access requests return
//! the exact endpoint-rounded sample count, padding missing input with silence. Offline streams
//! expose only decoded samples and determine length at EOF. Backward or large seeks flush decoding
//! and rebuild resampling state.

use std::collections::VecDeque;
use std::path::Path;

use super::ff;
use crate::frame::AudioBuffer;
use anyhow::{Context as _, Result, anyhow};
use ff::{ChannelLayout, Packet, codec, format, frame, media, software};

use super::ffi::ffmpeg_init;
use crate::transport::source::AudioSource;

/// Backward-seek tolerance in seconds to ignore timestamp noise.
const SEEK_BACK_EPS_S: f64 = 1e-3;
/// Seek instead of decoding through forward gaps larger than this threshold.
const FORWARD_SEEK_THRESHOLD_S: f64 = 1.0;

#[derive(Clone, Copy, PartialEq, Eq)]
struct SwrInput {
    format: format::Sample,
    channel_layout: ChannelLayout,
    rate: u32,
}

/// Streaming audio source with a persistent decoder, resampler, and output-sample cursor.
pub struct LibavAudioSource {
    ictx: format::context::Input,
    adec: ff::decoder::Audio,
    swr: software::resampling::Context,
    /// Current resampler input format, rebuilt when actual decoded frames differ from decoder
    /// defaults.
    swr_input: SwrInput,
    stream_idx: usize,
    stream_tb: ff::Rational,
    /// Stream start timestamp in ticks, with missing values treated as zero; used for source-local
    /// time and seeking.
    start_time_ticks: i64,
    out_rate: u32,
    out_channels: u16,
    /// Stream metadata converted to the output sample grid. This is only a capacity/progress hint.
    frame_count_hint: Option<u64>,
    /// Pending interleaved samples on the output sample grid.
    buf: VecDeque<f32>,
    /// Output-grid index of the first buffered sample; None until the first decoded timestamp
    /// anchors it.
    buf_start: Option<i64>,
    /// Demuxing reached EOF and the decoder is draining.
    draining: bool,
    /// The resampler tail has been flushed after EOF.
    flushed: bool,
    /// Maximum buffered sample count for bounded-memory diagnostics.
    buf_high_water: usize,
}

/// Forward-only, bounded audio decoder used by offline model tools.
///
/// Each [`Self::read`] returns at most the requested number of interleaved frames and discards
/// consumed decoder state. The stream therefore does not retain PCM in proportion to media
/// duration. Reopen the source for a deliberate second pass such as Demucs normalization.
pub struct LibavAudioStream {
    source: LibavAudioSource,
    frame_count_hint: Option<u64>,
    /// Exact source-local end frame, known only after decoder + resampler EOF.
    total_frames: Option<u64>,
    cursor: u64,
    sample_rate: u32,
    channels: u16,
}

// SAFETY: The decoder and resampler handles are exclusively owned and accessed by one thread at a
// time; crossing threads transfers ownership.
unsafe impl Send for LibavAudioSource {}

impl LibavAudioSource {
    /// Open a streaming source with fixed interleaved f32 output rate and channel count without
    /// decoding frames.
    pub fn open(path: &Path, out_rate: u32, out_channels: u16) -> Result<Self> {
        ffmpeg_init()?;
        let ictx = format::input(&path).map_err(|e| anyhow!("open {}: {e}", path.display()))?;
        let (stream_idx, params, stream_tb, start_time, duration) = {
            let s = ictx
                .streams()
                .best(media::Type::Audio)
                .ok_or_else(|| anyhow!("no audio stream in {}", path.display()))?;
            (
                s.index(),
                s.parameters(),
                s.time_base(),
                s.start_time(),
                s.duration(),
            )
        };
        let adec = codec::context::Context::from_parameters(params)?
            .decoder()
            .audio()?;
        let swr_input = Self::decoder_input_def(&adec);
        let swr = Self::make_swr(swr_input, out_rate, out_channels)?;
        let start_time_ticks = if start_time == ff::ffi::AV_NOPTS_VALUE {
            0
        } else {
            start_time
        };
        let duration_seconds = if duration > 0 && stream_tb.denominator() > 0 {
            Some(duration as f64 * stream_tb.numerator() as f64 / stream_tb.denominator() as f64)
                .filter(|duration| duration.is_finite() && *duration > 0.0)
        } else {
            None
        };
        let frame_count_hint = duration_seconds.and_then(|duration| {
            let frames = (duration * f64::from(out_rate)).round();
            (frames.is_finite() && frames > 0.0 && frames <= i64::MAX as f64)
                .then_some(frames as u64)
        });

        Ok(Self {
            ictx,
            adec,
            swr,
            swr_input,
            stream_idx,
            stream_tb,
            start_time_ticks,
            out_rate,
            out_channels,
            frame_count_hint,
            buf: VecDeque::new(),
            buf_start: None,
            draining: false,
            flushed: false,
            buf_high_water: 0,
        })
    }

    /// Initial resampler input defaults; actual decoded-frame metadata takes precedence.
    fn decoder_input_def(adec: &ff::decoder::Audio) -> SwrInput {
        let in_layout = Self::layout_or_default(adec.channel_layout(), adec.channels());
        SwrInput {
            format: adec.format(),
            channel_layout: in_layout,
            rate: adec.rate(),
        }
    }

    /// Read the decoded frame's input format, deriving a default channel layout when absent.
    fn frame_input_def(&self, decoded: &frame::Audio) -> SwrInput {
        let channels = if decoded.channels() > 0 {
            decoded.channels()
        } else {
            self.adec.channels()
        };
        let in_layout = Self::layout_or_default(decoded.channel_layout(), channels);
        let in_format = if decoded.format() == format::Sample::None {
            self.adec.format()
        } else {
            decoded.format()
        };
        let in_rate = if decoded.rate() > 0 {
            decoded.rate()
        } else {
            self.adec.rate()
        };
        SwrInput {
            format: in_format,
            channel_layout: in_layout,
            rate: in_rate,
        }
    }

    fn layout_or_default(layout: ChannelLayout, channels: u16) -> ChannelLayout {
        if layout.is_empty() {
            ChannelLayout::default(channels as i32)
        } else {
            layout
        }
    }

    /// Create a resampler from the decoded format to interleaved f32.
    fn make_swr(
        input: SwrInput,
        out_rate: u32,
        out_channels: u16,
    ) -> Result<software::resampling::Context> {
        let out_layout = ChannelLayout::default(out_channels as i32);
        software::resampling::Context::get(
            input.format,
            input.channel_layout,
            input.rate,
            format::Sample::F32(format::sample::Type::Packed), // Interleaved f32.
            out_layout,
            out_rate,
        )
        .context("create swr")
    }

    /// Rebuild resampling state when format, channels, or rate change, and fill missing frame
    /// metadata before conversion.
    fn prepare_frame_for_swr(&mut self, decoded: &mut frame::Audio) -> Result<()> {
        let input = self.frame_input_def(decoded);
        if input == self.swr_input {
            Self::fill_missing_frame_input(decoded, input);
            return Ok(());
        }
        while self.flush_swr_once()? {}
        self.swr = Self::make_swr(input, self.out_rate, self.out_channels)?;
        self.swr_input = input;
        Self::fill_missing_frame_input(decoded, input);
        Ok(())
    }

    fn fill_missing_frame_input(decoded: &mut frame::Audio, input: SwrInput) {
        if decoded.channel_layout().is_empty() {
            decoded.set_channel_layout(input.channel_layout);
        }
        if decoded.format() == format::Sample::None {
            decoded.set_format(input.format);
        }
        if decoded.rate() == 0 {
            decoded.set_rate(input.rate);
        }
    }

    /// Convert stream ticks to source-local seconds.
    fn ticks_to_secs(&self, ticks: i64) -> f64 {
        (ticks - self.start_time_ticks) as f64 * self.stream_tb.numerator() as f64
            / self.stream_tb.denominator().max(1) as f64
    }

    /// Append interleaved samples from a resampled frame.
    fn push_resampled(&mut self, out: &frame::Audio) {
        let n = out.samples() * self.out_channels as usize;
        if n == 0 {
            return;
        }
        let bytes = &out.data(0)[..n * size_of::<f32>()];
        for c in bytes.chunks_exact(size_of::<f32>()) {
            self.buf
                .push_back(f32::from_ne_bytes([c[0], c[1], c[2], c[3]]));
        }
        self.buf_high_water = self.buf_high_water.max(self.buf.len());
    }

    /// Flush at most one bounded resampler tail frame. `ffmpeg-next` does not allocate the output
    /// frame in `Context::flush`, so an empty frame would report `OutputChanged` and silently lose
    /// delayed samples if the error were ignored.
    fn flush_swr_once(&mut self) -> Result<bool> {
        let Some(delay) = self.swr.delay() else {
            return Ok(false);
        };
        let samples = usize::try_from(delay.output)
            .context("resampler tail exceeds addressable sample count")?;
        if samples == 0 {
            return Ok(false);
        }
        let mut out = frame::Audio::new(
            format::Sample::F32(format::sample::Type::Packed),
            samples,
            ChannelLayout::default(self.out_channels as i32),
        );
        self.swr.flush(&mut out).context("swr flush")?;
        let produced = out.samples() > 0;
        self.push_resampled(&out);
        Ok(produced)
    }

    /// Decode and resample the next frame, anchoring the buffer on its first timestamp. Flush the
    /// resampler at EOF and report whether progress occurred.
    fn pull(&mut self) -> Result<bool> {
        loop {
            let mut decoded = frame::Audio::empty();
            if self.adec.receive_frame(&mut decoded).is_ok() {
                // Anchor the first decoded timestamp to the output grid, then advance by produced
                // samples.
                if self.buf_start.is_none() {
                    let ticks = decoded
                        .timestamp()
                        .or_else(|| decoded.pts())
                        .unwrap_or(self.start_time_ticks);
                    let t = self.ticks_to_secs(ticks);
                    self.buf_start = Some((t * self.out_rate as f64).round() as i64);
                }
                self.prepare_frame_for_swr(&mut decoded)?;
                let mut out = frame::Audio::empty();
                self.swr.run(&decoded, &mut out).context("swr run")?;
                self.push_resampled(&out);
                return Ok(true);
            }
            if self.draining {
                if !self.flushed {
                    // Flush samples retained by resampler group delay.
                    if self.flush_swr_once()? {
                        return Ok(true);
                    }
                    self.flushed = true;
                }
                return Ok(false); // True EOF; subsequent missing samples are silence.
            }
            let mut pkt = Packet::empty();
            match pkt.read(&mut self.ictx) {
                Ok(()) => {
                    if pkt.stream() == self.stream_idx {
                        self.adec.send_packet(&pkt).context("send_packet(audio)")?;
                    }
                }
                Err(ff::Error::Eof) => {
                    self.adec.send_eof().context("send_eof(audio)")?;
                    self.draining = true;
                }
                Err(e) => return Err(anyhow!("audio packet read: {e:?}")),
            }
        }
    }

    /// Seek before an output sample index and reset decoder, resampler, and buffer state.
    fn seek_to_sample(&mut self, s0: i64) -> Result<()> {
        let want_s = s0.max(0) as f64 / self.out_rate as f64;
        let tb = self.stream_tb;
        let ticks = if tb.numerator() == 0 {
            0
        } else {
            (want_s * tb.denominator() as f64 / tb.numerator() as f64) as i64
                + self.start_time_ticks
        };
        unsafe {
            let _ = ff::ffi::av_seek_frame(
                self.ictx.as_mut_ptr(),
                self.stream_idx as i32,
                ticks,
                ff::ffi::AVSEEK_FLAG_BACKWARD,
            );
        }
        self.adec.flush();
        // Rebuild the resampler after seeking to reset its delay and phase.
        self.swr_input = Self::decoder_input_def(&self.adec);
        self.swr = Self::make_swr(self.swr_input, self.out_rate, self.out_channels)?;
        self.buf.clear();
        self.buf_start = None;
        self.draining = false;
        self.flushed = false;
        Ok(())
    }

    /// Exclusive output-grid endpoint covered by the buffer.
    fn buf_end(&self) -> Option<i64> {
        self.buf_start
            .map(|s| s + (self.buf.len() / self.out_channels as usize) as i64)
    }

    /// Exact source-local end on the output sample grid once decoder + resampler EOF is known.
    fn eof_frame(&self) -> Option<u64> {
        if !self.flushed {
            return None;
        }
        Some(self.buf_end().unwrap_or(0).max(0) as u64)
    }

    /// Discard consumed samples during catch-up so memory stays bounded by the active chunk without
    /// changing decoded output.
    fn drop_consumed_before(&mut self, s0: i64) {
        let ch = self.out_channels as usize;
        if let Some(bs) = self.buf_start
            && s0 > bs
        {
            let drop_n = ((s0 - bs) as usize * ch).min(self.buf.len());
            self.buf.drain(..drop_n);
            self.buf_start = Some(bs + (drop_n / ch) as i64); // Advance by the actual number of discarded samples.
        }
    }

    /// Maximum buffered sample count for diagnostics; catch-up must not accumulate the entire
    /// skipped prefix.
    #[doc(hidden)]
    pub fn buf_high_water(&self) -> usize {
        self.buf_high_water
    }
}

impl LibavAudioStream {
    pub fn open(path: &Path, sample_rate: u32, channels: u16) -> Result<Self> {
        if sample_rate == 0 || channels == 0 {
            return Err(anyhow!(
                "audio stream output format must use a positive sample rate and channel count"
            ));
        }
        let source = LibavAudioSource::open(path, sample_rate, channels)?;
        let frame_count_hint = source.frame_count_hint;
        Ok(Self {
            source,
            frame_count_hint,
            total_frames: None,
            cursor: 0,
            sample_rate,
            channels,
        })
    }

    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub const fn channels(&self) -> u16 {
        self.channels
    }

    /// Container/stream duration converted to frames, for progress and storage planning only.
    /// It may be shorter or longer than the decoded PCM and must never terminate or pad reads.
    pub const fn frame_count_hint(&self) -> Option<u64> {
        self.frame_count_hint
    }

    /// Exact decoded frame count. This becomes available only after [`Self::read`] reaches EOF.
    pub const fn total_frames(&self) -> Option<u64> {
        self.total_frames
    }

    pub const fn position(&self) -> u64 {
        self.cursor
    }

    pub fn read(&mut self, maximum_frames: usize) -> Result<AudioBuffer> {
        if maximum_frames == 0 {
            return Err(anyhow!("audio stream read size must be greater than zero"));
        }
        if self.total_frames.is_some() {
            return Ok(AudioBuffer::silence(self.sample_rate, self.channels, 0));
        }
        let count = u64::try_from(maximum_frames).context("audio read size exceeds u64")?;
        let requested_end = self
            .cursor
            .checked_add(count)
            .context("audio cursor overflow")?;
        if requested_end > i64::MAX as u64 {
            return Err(anyhow!("audio cursor exceeds the supported sample grid"));
        }
        let start = i64::try_from(self.cursor).context("audio cursor exceeds i64")?;
        let end = requested_end as i64;
        let mut buffer = self.source.samples_by_index(start, end)?;
        let delivered_end = if let Some(eof_frame) = self.source.eof_frame() {
            if eof_frame < self.cursor {
                return Err(anyhow!(
                    "audio decoder EOF {eof_frame} precedes stream cursor {}",
                    self.cursor
                ));
            }
            self.total_frames = Some(eof_frame);
            eof_frame.min(requested_end)
        } else {
            requested_end
        };
        let delivered_frames = usize::try_from(delivered_end - self.cursor)
            .context("decoded audio chunk exceeds addressable memory")?;
        let delivered_samples = delivered_frames
            .checked_mul(usize::from(self.channels))
            .context("decoded audio chunk sample count overflow")?;
        buffer.samples.truncate(delivered_samples);
        self.cursor = delivered_end;
        Ok(buffer)
    }
}

impl AudioSource for LibavAudioSource {
    fn samples(&mut self, t: f64, dt: f64) -> Result<AudioBuffer> {
        let rate = self.out_rate as f64;
        // Quantize both window endpoints onto the sample grid for seamless adjacent requests.
        let s0 = (t.max(0.0) * rate).round() as i64;
        let s1 = ((t.max(0.0) + dt.max(0.0)) * rate).round() as i64;
        self.samples_by_index(s0, s1)
    }

    fn samples_by_index(&mut self, s0: i64, s1: i64) -> Result<AudioBuffer> {
        if s0 < 0 || s1 < s0 {
            return Err(anyhow!("invalid audio sample range [{s0}, {s1})"));
        }
        let ch = self.out_channels as usize;
        let rate = self.out_rate as f64;
        let n =
            usize::try_from(s1 - s0).context("audio sample range exceeds addressable memory")?;
        let sample_len = n
            .checked_mul(ch)
            .context("audio sample buffer exceeds addressable memory")?;
        let mut out = vec![0f32; sample_len];
        if n == 0 {
            return Ok(AudioBuffer {
                samples: out,
                sample_rate: self.out_rate,
                channels: self.out_channels,
            });
        }

        // Reset on backward or large forward requests; normal mixing advances sequentially.
        let back_eps = (SEEK_BACK_EPS_S * rate) as i64;
        let fwd_max = (FORWARD_SEEK_THRESHOLD_S * rate) as i64;
        match self.buf_start {
            Some(bs) if s0 + back_eps < bs => self.seek_to_sample(s0)?,
            _ => {
                if let Some(be) = self.buf_end()
                    && s0 > be + fwd_max
                {
                    self.seek_to_sample(s0)?;
                }
            }
        }

        // Decode until the requested window is covered or EOF is reached. Discard preceding samples
        // during catch-up to keep memory bounded.
        while self.buf_end().is_none_or(|be| be < s1) {
            if !self.pull()? {
                break;
            }
            self.drop_consumed_before(s0);
        }
        self.drop_consumed_before(s0);
        // Copy the buffered overlap; uncovered output remains silent.
        if let Some(bs) = self.buf_start {
            let avail = self.buf.len() / ch;
            // Compute destination and buffer offsets relative to their sample-grid origins.
            let out_off = (bs - s0).max(0) as usize;
            let buf_off = (s0 - bs).max(0) as usize;
            let copy_frames = n.saturating_sub(out_off).min(avail.saturating_sub(buf_off));
            let (a, b) = self.buf.as_slices();
            for i in 0..copy_frames * ch {
                let src_idx = buf_off * ch + i;
                let v = if src_idx < a.len() {
                    a[src_idx]
                } else {
                    b[src_idx - a.len()]
                };
                out[out_off * ch + i] = v;
            }
        }
        Ok(AudioBuffer {
            samples: out,
            sample_rate: self.out_rate,
            channels: self.out_channels,
        })
    }
}

/// Decode audio into mono f32 samples at the requested rate through libav. Use bounded streaming
/// chunks while accumulating the output samples.
pub fn decode_audio_mono_f32(path: &Path, sample_rate: u32) -> Result<Vec<f32>> {
    use crate::transport::source::AudioSource as _;
    let probe = crate::codec::decode::probe_av(path)?;
    let duration = probe
        .audio
        .filter(|d| *d > 0.0)
        .or(probe.video.map(|(_, _, d)| d).filter(|d| *d > 0.0))
        .ok_or_else(|| anyhow!("{}: no audio duration metadata", path.display()))?;
    let mut src = LibavAudioSource::open(path, sample_rate, 1)?;
    let mut samples = Vec::with_capacity((duration * sample_rate as f64) as usize + 1);
    let mut t = 0.0;
    while t < duration {
        let dt = (duration - t).min(1.0);
        samples.extend_from_slice(&src.samples(t, dt)?.samples);
        t += dt;
    }
    Ok(samples)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::codec::FloatWavWriter;

    pub(crate) fn forward_stream_uses_real_eof_instead_of_a_long_duration_hint() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("actual-eof.wav");
        let expected_frames = 2_137_usize;
        let mut writer = FloatWavWriter::create(&path, 16_000, 1).unwrap();
        writer
            .write(&AudioBuffer {
                samples: vec![0.125; expected_frames],
                sample_rate: 16_000,
                channels: 1,
            })
            .unwrap();
        writer.finish().unwrap();

        let mut stream = LibavAudioStream::open(&path, 16_000, 1).unwrap();
        // Reproduce overlong container/VBR metadata without relying on a demuxer-specific malformed
        // fixture. This field is deliberately only a hint and must not affect read termination.
        stream.frame_count_hint = Some(expected_frames as u64 + 10_000);
        assert_eq!(stream.total_frames(), None);
        let mut decoded_frames = 0_u64;
        loop {
            let chunk = stream.read(257).unwrap();
            assert!(chunk.frames() <= 257);
            if chunk.frames() == 0 {
                break;
            }
            decoded_frames += chunk.frames() as u64;
        }

        assert_eq!(decoded_frames, expected_frames as u64);
        assert_eq!(stream.position(), expected_frames as u64);
        assert_eq!(stream.total_frames(), Some(expected_frames as u64));
        assert_eq!(stream.read(257).unwrap().frames(), 0);
    }

    pub(crate) fn forward_stream_does_not_truncate_to_a_short_duration_hint() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("actual-eof.wav");
        let expected_frames = 1_123_usize;
        let mut writer = FloatWavWriter::create(&path, 16_000, 1).unwrap();
        writer
            .write(&AudioBuffer {
                samples: vec![-0.25; expected_frames],
                sample_rate: 16_000,
                channels: 1,
            })
            .unwrap();
        writer.finish().unwrap();

        let mut stream = LibavAudioStream::open(&path, 16_000, 1).unwrap();
        stream.frame_count_hint = Some(100);
        let mut decoded_frames = 0_u64;
        loop {
            let chunk = stream.read(211).unwrap();
            if chunk.frames() == 0 {
                break;
            }
            decoded_frames += chunk.frames() as u64;
        }
        assert_eq!(decoded_frames, expected_frames as u64);
        assert_eq!(stream.total_frames(), Some(expected_frames as u64));
    }
}

pub fn probe_audio_stream(path: &Path) -> Result<Option<crate::codec::audio::AudioStreamInfo>> {
    ffmpeg_init()?;
    let input = format::input(path)?;
    let Some(stream) = input.streams().best(media::Type::Audio) else {
        return Ok(None);
    };
    let decoder = codec::context::Context::from_parameters(stream.parameters())?
        .decoder()
        .audio()?;
    Ok(Some(crate::codec::audio::AudioStreamInfo {
        stream: u32::try_from(stream.index())?,
        channels: decoder.channels(),
    }))
}
