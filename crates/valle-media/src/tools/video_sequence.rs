//! Strict, bounded video sequencing shared by frame-derived offline tools.
//!
//! [`crate::codec::LibavVideoSource`] intentionally implements random-access sampling by time.
//! Model tools such as inpaint, upscale and interpolation instead need every decoded frame and
//! its original presentation timestamp. This module owns that narrower contract. It never
//! guesses a missing timestamp, never duplicates or drops a frame, and keeps only decoder/
//! swscale state plus the current frame.

use std::{mem::size_of, path::Path};

use anyhow::{Context as _, Result, anyhow, ensure};
use ff::{Packet, Rational, codec, color, format, frame, media, software};
use ffmpeg_next as ff;

use crate::{
    codec::ffi::{
        ffmpeg_init, guessed_sample_aspect_ratio, matrix_for, read_plane0, set_sws_colorspace,
    },
    frame::RgbaFrame,
};

#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
use crate::{
    codec::{AudioSource, LibavAudioSource, Muxer},
    frame::AudioBuffer,
};

#[cfg(any(feature = "tool-inpaint", feature = "tool-segment"))]
use crate::frame::Gray8Frame;
#[cfg(feature = "tool-inpaint")]
use crate::tools::TimeRange;

#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
const AUDIO_SAMPLE_RATE: u32 = 48_000;
#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
const AUDIO_CHANNELS: u16 = 2;
#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
const AUDIO_CHUNK_FRAMES: i64 = AUDIO_SAMPLE_RATE as i64;

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // Individual fields are consumed by different feature-gated frame tools.
pub(super) struct VideoMetadata {
    pub width: u32,
    pub height: u32,
    pub duration_ticks: Option<i64>,
    pub nominal_fps: f64,
    pub time_base: Rational,
    pub start_pts: i64,
    pub rotation_degrees: u32,
    pub sample_aspect_ratio: Rational,
}

/// Timing facts obtained by decoding a staged video through real EOF without retaining frames.
#[derive(Debug, Clone, Copy)]
#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
pub(super) struct DecodedVideoTiming {
    pub frame_count: u64,
    pub duration_seconds: f64,
    pub variable_frame_rate: bool,
}

/// Reopen a staged video, decode every frame, and validate its zero-based, strictly increasing
/// PTS. This catches truncated outputs and muxer timestamp regressions while retaining only one
/// decoder frame and scalar timing state.
#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
pub(super) fn decode_video_timing(path: &Path, threads: usize) -> Result<DecodedVideoTiming> {
    ffmpeg_init();
    ensure!(threads > 0, "decoder thread count must be positive");
    let mut input = format::input(path).with_context(|| format!("open {}", path.display()))?;
    let (stream_index, parameters, time_base) = {
        let stream = input
            .streams()
            .best(media::Type::Video)
            .context("staged output has no video stream")?;
        (stream.index(), stream.parameters(), stream.time_base())
    };
    ensure!(
        time_base.numerator() > 0 && time_base.denominator() > 0,
        "staged video has invalid time base"
    );
    let mut codec_context = codec::context::Context::from_parameters(parameters)?;
    let mut threading = codec::threading::Config::kind(codec::threading::Type::Frame);
    threading.count = threads;
    codec_context.set_threading(threading);
    let mut decoder = codec_context.decoder().video()?;
    let mut state = DecodedTimingState::default();
    loop {
        let mut packet = Packet::empty();
        match packet.read(&mut input) {
            Ok(()) if packet.stream() == stream_index => {
                decoder.send_packet(&packet)?;
                receive_timing_frames(&mut decoder, &mut state)?;
            }
            Ok(()) => {}
            Err(ff::Error::Eof) => break,
            Err(error) => return Err(anyhow!("read staged video packet: {error}")),
        }
    }
    decoder.send_eof()?;
    receive_timing_frames(&mut decoder, &mut state)?;
    ensure!(state.frame_count > 0, "staged video decoded no frames");
    let last_pts = state.last_pts.context("staged video has no final PTS")?;
    let final_duration = state
        .last_duration
        .or(state.last_step)
        .context("staged video did not expose a final frame duration")?;
    let end_ticks = last_pts
        .checked_add(final_duration)
        .context("staged video duration overflowed")?;
    Ok(DecodedVideoTiming {
        frame_count: state.frame_count,
        duration_seconds: end_ticks as f64 * rational_seconds(time_base),
        variable_frame_rate: state.variable_frame_rate,
    })
}

#[derive(Default)]
#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
struct DecodedTimingState {
    frame_count: u64,
    first_step: Option<i64>,
    last_step: Option<i64>,
    last_pts: Option<i64>,
    last_duration: Option<i64>,
    variable_frame_rate: bool,
}

#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
fn receive_timing_frames(
    decoder: &mut ff::decoder::Video,
    state: &mut DecodedTimingState,
) -> Result<()> {
    let mut decoded = frame::Video::empty();
    while decoder.receive_frame(&mut decoded).is_ok() {
        let pts = decoded
            .timestamp()
            .or_else(|| decoded.pts())
            .context("staged video frame has no presentation timestamp")?;
        if let Some(previous) = state.last_pts {
            ensure!(
                pts > previous,
                "staged video PTS is not strictly increasing: {pts} <= {previous}"
            );
            let step = pts - previous;
            let first_step = *state.first_step.get_or_insert(step);
            state.variable_frame_rate |= step != first_step;
            state.last_step = Some(step);
        } else {
            ensure!(pts == 0, "staged video must start at PTS zero, got {pts}");
        }
        let packet_duration = decoded.packet().duration;
        state.last_duration = (packet_duration > 0).then_some(packet_duration);
        state.last_pts = Some(pts);
        state.frame_count = state
            .frame_count
            .checked_add(1)
            .context("staged video frame count overflowed")?;
    }
    Ok(())
}

impl VideoMetadata {
    #[allow(dead_code)] // Inpaint/upscale report source duration; interpolation derives its own.
    pub fn duration_seconds(self) -> Option<f64> {
        self.duration_ticks
            .map(|ticks| ticks as f64 * rational_seconds(self.time_base))
    }
}

#[derive(Debug)]
pub(super) struct TimedVideoFrame<T> {
    /// Presentation timestamp in the source stream's time base. It is not rebased.
    pub pts: i64,
    /// Decoder-provided presentation duration in the same time base, when the demuxer exposes it.
    pub duration_ticks: Option<i64>,
    pub frame: T,
}

impl<T> TimedVideoFrame<T> {
    #[allow(dead_code)] // Used by the single-stream frame tools, not the matched inpaint stream.
    pub fn absolute_seconds(&self, time_base: Rational) -> f64 {
        self.pts as f64 * rational_seconds(time_base)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecodeFormat {
    Rgba,
    #[cfg(any(feature = "tool-inpaint", feature = "tool-segment"))]
    Gray8,
}

enum DecodedPixels {
    Rgba(RgbaFrame),
    #[cfg(any(feature = "tool-inpaint", feature = "tool-segment"))]
    Gray8(Gray8Frame),
}

struct SequentialDecoder {
    input: format::context::Input,
    decoder: ff::decoder::Video,
    scaler: software::scaling::Context,
    stream_index: usize,
    format: DecodeFormat,
    last_pts: Option<i64>,
    draining: bool,
    unrotated_width: u32,
    unrotated_height: u32,
    metadata: VideoMetadata,
}

impl SequentialDecoder {
    fn open(path: &Path, threads: usize, output_format: DecodeFormat) -> Result<Self> {
        ffmpeg_init();
        ensure!(threads > 0, "decoder thread count must be positive");
        let mut input = format::input(path).with_context(|| format!("open {}", path.display()))?;
        let (
            stream_index,
            parameters,
            time_base,
            start_time,
            duration,
            nominal_rate,
            rotation_degrees,
        ) = {
            let stream = input
                .streams()
                .best(media::Type::Video)
                .context("input has no video stream")?;
            (
                stream.index(),
                stream.parameters(),
                stream.time_base(),
                stream.start_time(),
                stream.duration(),
                stream.avg_frame_rate(),
                stream_rotation_degrees(&stream),
            )
        };
        ensure!(
            time_base.numerator() > 0 && time_base.denominator() > 0,
            "video stream has invalid time base {}/{}",
            time_base.numerator(),
            time_base.denominator()
        );
        let container_aspect_ratio = guessed_sample_aspect_ratio(&mut input, stream_index)?;
        #[cfg(any(feature = "tool-inpaint", feature = "tool-segment"))]
        if output_format == DecodeFormat::Gray8 {
            ensure!(
                parameters.id() == codec::Id::FFV1,
                "mask video must use lossless FFV1, got {:?}",
                parameters.id()
            );
        }

        let mut codec_context = codec::context::Context::from_parameters(parameters)?;
        let mut threading = codec::threading::Config::kind(codec::threading::Type::Frame);
        threading.count = threads;
        codec_context.set_threading(threading);
        let decoder = codec_context.decoder().video()?;
        #[cfg(any(feature = "tool-inpaint", feature = "tool-segment"))]
        if output_format == DecodeFormat::Gray8 {
            ensure!(
                decoder.format() == format::Pixel::GRAY8,
                "mask video must decode natively as Gray8, got {:?}",
                decoder.format()
            );
        }
        let coded_width = decoder.width();
        let coded_height = decoder.height();
        ensure!(
            coded_width > 0 && coded_height > 0,
            "video dimensions must be positive"
        );
        let sample_aspect_ratio =
            if container_aspect_ratio.numerator() > 0 && container_aspect_ratio.denominator() > 0 {
                container_aspect_ratio
            } else {
                decoder.aspect_ratio()
            };
        let unrotated_width = square_pixel_width(coded_width, sample_aspect_ratio)?;
        let unrotated_height = coded_height;
        let pixel_format = match output_format {
            DecodeFormat::Rgba => format::Pixel::RGBA,
            #[cfg(any(feature = "tool-inpaint", feature = "tool-segment"))]
            DecodeFormat::Gray8 => format::Pixel::GRAY8,
        };
        let scaling_flags = match output_format {
            DecodeFormat::Rgba => software::scaling::Flags::BILINEAR,
            // A mask is categorical data. SAR normalization must never synthesize soft values.
            #[cfg(any(feature = "tool-inpaint", feature = "tool-segment"))]
            DecodeFormat::Gray8 => software::scaling::Flags::POINT,
        };
        let mut scaler = software::scaling::Context::get(
            decoder.format(),
            coded_width,
            coded_height,
            pixel_format,
            unrotated_width,
            unrotated_height,
            scaling_flags,
        )?;
        if output_format == DecodeFormat::Rgba {
            set_sws_colorspace(
                &mut scaler,
                matrix_for(decoder.color_space(), coded_height),
                decoder.color_range() == color::Range::JPEG,
                true,
            );
        }

        let start_pts = if start_time == ff::ffi::AV_NOPTS_VALUE {
            0
        } else {
            start_time
        };
        let duration_ticks =
            (duration > 0 && duration != ff::ffi::AV_NOPTS_VALUE).then_some(duration);
        let nominal_fps = rational_to_positive_f64(nominal_rate).unwrap_or(0.0);
        let (width, height) = if rotation_degrees % 180 == 90 {
            (unrotated_height, unrotated_width)
        } else {
            (unrotated_width, unrotated_height)
        };
        Ok(Self {
            input,
            decoder,
            scaler,
            stream_index,
            format: output_format,
            last_pts: None,
            draining: false,
            unrotated_width,
            unrotated_height,
            metadata: VideoMetadata {
                width,
                height,
                duration_ticks,
                nominal_fps,
                time_base,
                start_pts,
                rotation_degrees,
                sample_aspect_ratio,
            },
        })
    }

    fn next_frame(&mut self) -> Result<Option<TimedVideoFrame<DecodedPixels>>> {
        loop {
            let mut decoded = frame::Video::empty();
            match self.decoder.receive_frame(&mut decoded) {
                Ok(()) => return self.convert(decoded).map(Some),
                Err(ff::Error::Eof) => return Ok(None),
                Err(ff::Error::Other {
                    errno: ff::util::error::EAGAIN,
                }) => {}
                Err(error) => return Err(anyhow!("receive decoded video frame: {error}")),
            }
            if self.draining {
                return Ok(None);
            }
            loop {
                let mut packet = Packet::empty();
                match packet.read(&mut self.input) {
                    Ok(()) if packet.stream() == self.stream_index => {
                        self.decoder.send_packet(&packet)?;
                        break;
                    }
                    Ok(()) => {}
                    Err(ff::Error::Eof) => {
                        self.decoder.send_eof()?;
                        self.draining = true;
                        break;
                    }
                    Err(error) => return Err(anyhow!("read video packet: {error}")),
                }
            }
        }
    }

    fn convert(&mut self, decoded: frame::Video) -> Result<TimedVideoFrame<DecodedPixels>> {
        let pts = decoded
            .timestamp()
            .or_else(|| decoded.pts())
            .context("video frame has no presentation timestamp")?;
        let duration_ticks = (decoded.packet().duration > 0).then_some(decoded.packet().duration);
        if let Some(last) = self.last_pts {
            ensure!(
                pts > last,
                "video PTS is not strictly increasing: {pts} <= {last}"
            );
        }
        let mut converted = frame::Video::empty();
        self.scaler.run(&decoded, &mut converted)?;
        let pixels = match self.format {
            DecodeFormat::Rgba => {
                let data = read_plane0(
                    &converted,
                    self.unrotated_width as usize * 4,
                    self.unrotated_height,
                );
                DecodedPixels::Rgba(rotate_rgba(
                    RgbaFrame {
                        width: self.unrotated_width,
                        height: self.unrotated_height,
                        data,
                    },
                    self.metadata.rotation_degrees,
                ))
            }
            #[cfg(any(feature = "tool-inpaint", feature = "tool-segment"))]
            DecodeFormat::Gray8 => {
                let data = read_plane0(
                    &converted,
                    self.unrotated_width as usize,
                    self.unrotated_height,
                );
                let mask = Gray8Frame::from_data(self.unrotated_width, self.unrotated_height, data)
                    .map_err(anyhow::Error::msg)?;
                DecodedPixels::Gray8(rotate_gray8(mask, self.metadata.rotation_degrees))
            }
        };
        let dimensions = match &pixels {
            DecodedPixels::Rgba(frame) => (frame.width, frame.height),
            #[cfg(any(feature = "tool-inpaint", feature = "tool-segment"))]
            DecodedPixels::Gray8(frame) => (frame.width, frame.height),
        };
        ensure!(
            dimensions == (self.metadata.width, self.metadata.height),
            "display-orientation normalization drifted"
        );
        self.last_pts = Some(pts);
        Ok(TimedVideoFrame {
            pts,
            duration_ticks,
            frame: pixels,
        })
    }
}

/// Strict sequential decoder for display-oriented, square-pixel RGBA8 frames.
pub(super) struct SequentialRgbaDecoder(SequentialDecoder);

impl SequentialRgbaDecoder {
    pub fn open(path: &Path, threads: usize) -> Result<Self> {
        SequentialDecoder::open(path, threads, DecodeFormat::Rgba).map(Self)
    }

    pub const fn metadata(&self) -> VideoMetadata {
        self.0.metadata
    }

    pub fn next_frame(&mut self) -> Result<Option<TimedVideoFrame<RgbaFrame>>> {
        self.0.next_frame()?.map_or(Ok(None), |timed| {
            #[cfg(any(feature = "tool-inpaint", feature = "tool-segment"))]
            let frame = match timed.frame {
                DecodedPixels::Rgba(frame) => frame,
                DecodedPixels::Gray8(_) => {
                    return Err(anyhow!("RGBA decoder returned a grayscale frame"));
                }
            };
            #[cfg(not(any(feature = "tool-inpaint", feature = "tool-segment")))]
            let DecodedPixels::Rgba(frame) = timed.frame;
            Ok(Some(TimedVideoFrame {
                pts: timed.pts,
                duration_ticks: timed.duration_ticks,
                frame,
            }))
        })
    }
}

/// Strict sequential decoder for the lossless FFV1/Gray8 mask exchange format.
#[cfg(any(feature = "tool-inpaint", feature = "tool-segment"))]
pub(super) struct SequentialGray8Decoder(SequentialDecoder);

#[cfg(any(feature = "tool-inpaint", feature = "tool-segment"))]
impl SequentialGray8Decoder {
    pub fn open(path: &Path, threads: usize) -> Result<Self> {
        SequentialDecoder::open(path, threads, DecodeFormat::Gray8).map(Self)
    }

    pub const fn metadata(&self) -> VideoMetadata {
        self.0.metadata
    }

    pub fn next_frame(&mut self) -> Result<Option<TimedVideoFrame<Gray8Frame>>> {
        self.0.next_frame()?.map_or(Ok(None), |timed| {
            let DecodedPixels::Gray8(frame) = timed.frame else {
                return Err(anyhow!("Gray8 decoder returned an RGBA frame"));
            };
            ensure!(frame.is_binary_mask(), "mask frame is not binary Gray8");
            Ok(Some(TimedVideoFrame {
                pts: timed.pts,
                duration_ticks: timed.duration_ticks,
                frame,
            }))
        })
    }
}

#[derive(Debug)]
#[cfg(feature = "tool-inpaint")]
pub(super) struct MatchedVideoFrame {
    /// Source PTS rebased to the first decoded source frame, in `time_base`.
    pub pts: i64,
    /// Original source PTS before rebasing, used to preserve A/V start offsets.
    pub source_absolute_pts: i64,
    pub time_base: Rational,
    /// Decoder-provided duration for this source frame, when available.
    pub duration_ticks: Option<i64>,
    pub source: RgbaFrame,
    pub mask: Gray8Frame,
}

/// Lockstep source/mask reader. Full-file pairs compare absolute PTS. With a range, source frames
/// are selected in source-local time and rebased against an already-zero-based range mask. At most
/// one frame from each stream is live.
#[cfg(feature = "tool-inpaint")]
pub(super) struct MatchedVideoReader {
    source: SequentialRgbaDecoder,
    mask: SequentialGray8Decoder,
    range: Option<TimeRange>,
    first_source_pts: Option<i64>,
    frames: u64,
}

#[cfg(feature = "tool-inpaint")]
impl MatchedVideoReader {
    pub fn open(
        source: &Path,
        mask: &Path,
        range: Option<TimeRange>,
        threads: usize,
    ) -> Result<Self> {
        let source = SequentialRgbaDecoder::open(source, threads)?;
        let mask = SequentialGray8Decoder::open(mask, threads)?;
        let source_metadata = source.metadata();
        let mask_metadata = mask.metadata();
        ensure!(
            (source_metadata.width, source_metadata.height)
                == (mask_metadata.width, mask_metadata.height),
            "source display dimensions {}x{} and mask dimensions {}x{} must match exactly",
            source_metadata.width,
            source_metadata.height,
            mask_metadata.width,
            mask_metadata.height
        );
        if range.is_none()
            && let (Some(source_duration), Some(mask_duration)) =
                (source_metadata.duration_ticks, mask_metadata.duration_ticks)
        {
            let expected_mask_duration = rescale_timestamp_nearest(
                source_duration,
                source_metadata.time_base,
                mask_metadata.time_base,
            )?;
            ensure!(
                mask_duration == expected_mask_duration,
                "source/mask declared-duration mismatch after exact time-base rescale: expected mask={expected_mask_duration}, got {mask_duration}"
            );
        }
        Ok(Self {
            source,
            mask,
            range,
            first_source_pts: None,
            frames: 0,
        })
    }

    pub const fn source_metadata(&self) -> VideoMetadata {
        self.source.metadata()
    }

    pub const fn mask_metadata(&self) -> VideoMetadata {
        self.mask.metadata()
    }

    pub const fn frames_read(&self) -> u64 {
        self.frames
    }

    pub fn next_frame(&mut self) -> Result<Option<MatchedVideoFrame>> {
        let source = self.next_selected_source_frame()?;
        let mask = self.mask.next_frame()?;
        match (source, mask) {
            (None, None) => Ok(None),
            (Some(_), None) => Err(anyhow!(
                "mask video ended before source frame {}",
                self.frames
            )),
            (None, Some(_)) => Err(anyhow!(
                "mask video has extra frame {} after the source ended",
                self.frames
            )),
            (Some(source), Some(mask)) => {
                let first_source = *self.first_source_pts.get_or_insert(source.pts);
                let source_pts = source
                    .pts
                    .checked_sub(first_source)
                    .context("source PTS rebase overflowed")?;
                let source_tb = self.source.metadata().time_base;
                let mask_tb = self.mask.metadata().time_base;
                validate_pair_timestamp(
                    self.frames,
                    source.pts,
                    source_pts,
                    source_tb,
                    mask.pts,
                    mask_tb,
                    self.range.is_some(),
                )?;
                ensure!(
                    source.frame.width == mask.frame.width
                        && source.frame.height == mask.frame.height,
                    "source/mask frame dimensions drifted at frame {}",
                    self.frames
                );
                self.frames = self
                    .frames
                    .checked_add(1)
                    .context("matched video frame count overflowed")?;
                Ok(Some(MatchedVideoFrame {
                    pts: source_pts,
                    source_absolute_pts: source.pts,
                    time_base: source_tb,
                    duration_ticks: source.duration_ticks,
                    source: source.frame,
                    mask: mask.frame,
                }))
            }
        }
    }

    fn next_selected_source_frame(&mut self) -> Result<Option<TimedVideoFrame<RgbaFrame>>> {
        loop {
            let Some(frame) = self.source.next_frame()? else {
                return Ok(None);
            };
            let Some(range) = self.range else {
                return Ok(Some(frame));
            };
            let metadata = self.source.metadata();
            let local_pts = frame
                .pts
                .checked_sub(metadata.start_pts)
                .context("source-local PTS conversion overflowed")?;
            let local_seconds = local_pts as f64 * rational_seconds(metadata.time_base);
            if local_seconds < range.start {
                continue;
            }
            if range.end.is_some_and(|end| local_seconds >= end) {
                return Ok(None);
            }
            return Ok(Some(frame));
        }
    }
}

#[cfg(feature = "tool-inpaint")]
fn validate_pair_timestamp(
    frame_index: u64,
    source_absolute_pts: i64,
    source_rebased_pts: i64,
    source_time_base: Rational,
    mask_absolute_pts: i64,
    mask_time_base: Rational,
    range_rebased: bool,
) -> Result<()> {
    if range_rebased && frame_index == 0 {
        ensure!(
            mask_absolute_pts == 0,
            "range mask must start at absolute PTS zero, got {mask_absolute_pts} {}/{}",
            mask_time_base.numerator(),
            mask_time_base.denominator(),
        );
    }
    let expected_source_pts = if range_rebased {
        source_rebased_pts
    } else {
        source_absolute_pts
    };
    let expected_mask_pts =
        rescale_timestamp_nearest(expected_source_pts, source_time_base, mask_time_base)?;
    ensure!(
        mask_absolute_pts == expected_mask_pts,
        "source/mask {} PTS mismatch at frame {frame_index}: source={expected_source_pts} {}/{}, expected mask={expected_mask_pts} {}/{}, got mask={mask_absolute_pts}",
        if range_rebased {
            "range-rebased"
        } else {
            "absolute"
        },
        source_time_base.numerator(),
        source_time_base.denominator(),
        mask_time_base.numerator(),
        mask_time_base.denominator(),
    );
    Ok(())
}

/// Bounded source clock that preserves observed PTS instead of forcing frames onto a CFR grid.
///
/// The first selected frame is zero-based at the tool-output boundary. Every later timestamp must
/// be strictly increasing, but deltas may vary. Only the most recent timestamp and duration are
/// retained, so arbitrarily long VFR input does not grow this state.
#[derive(Debug, Clone, Copy)]
#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
pub(super) struct SourceTimeline {
    time_base: Rational,
    nominal_step: i64,
    fps_numerator: u32,
    fps_denominator: u32,
    frames: u64,
    first_step: Option<i64>,
    last_pts: Option<i64>,
    last_step: Option<i64>,
    last_duration: Option<i64>,
    variable_frame_rate: bool,
}

#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
impl SourceTimeline {
    pub fn derive(
        first_pts: i64,
        second_pts: Option<i64>,
        time_base: Rational,
        nominal_fps: f64,
    ) -> Result<Self> {
        ensure!(
            first_pts == 0,
            "source timeline requires first PTS rebased to zero"
        );
        let (nominal_step, fps_numerator, fps_denominator) = if let Some(second_pts) = second_pts {
            ensure!(second_pts > 0, "second video PTS must be greater than zero");
            let (num, den) = exact_fps(time_base, second_pts)?;
            (second_pts, num, den)
        } else {
            let (num, den) = approximate_fps(nominal_fps)
                .context("single-frame video has no valid nominal frame rate")?;
            let seconds_per_tick = rational_seconds(time_base);
            let step = (1.0 / nominal_fps / seconds_per_tick).round();
            ensure!(
                step.is_finite() && step >= 1.0 && step <= i64::MAX as f64,
                "single-frame video nominal duration is not representable"
            );
            (step as i64, num, den)
        };
        Ok(Self {
            time_base,
            nominal_step,
            fps_numerator,
            fps_denominator,
            frames: 0,
            first_step: None,
            last_pts: None,
            last_step: None,
            last_duration: None,
            variable_frame_rate: false,
        })
    }

    pub const fn fps(&self) -> (u32, u32) {
        (self.fps_numerator, self.fps_denominator)
    }

    pub fn observe(
        &mut self,
        frame_index: u64,
        pts: i64,
        duration_ticks: Option<i64>,
    ) -> Result<()> {
        ensure!(
            frame_index == self.frames,
            "source timeline frame index moved from {} to {frame_index}",
            self.frames
        );
        if let Some(previous) = self.last_pts {
            ensure!(
                pts > previous,
                "source PTS is not strictly increasing at frame {frame_index}: {pts} <= {previous}"
            );
            let step = pts - previous;
            let first_step = *self.first_step.get_or_insert(step);
            self.variable_frame_rate |= step != first_step;
            self.last_step = Some(step);
        } else {
            ensure!(pts == 0, "first source PTS must be zero, got {pts}");
        }
        if let Some(duration) = duration_ticks {
            ensure!(duration > 0, "source frame duration must be positive");
        }
        self.last_pts = Some(pts);
        self.last_duration = duration_ticks;
        self.frames = self
            .frames
            .checked_add(1)
            .context("source timeline frame count overflowed")?;
        Ok(())
    }

    pub fn duration_seconds(&self) -> Result<f64> {
        let last_pts = self.last_pts.context("source timeline has no frames")?;
        let duration = self
            .last_duration
            .or(self.last_step)
            .unwrap_or(self.nominal_step)
            .max(1);
        let end = last_pts
            .checked_add(duration)
            .context("source timeline duration overflowed")?;
        Ok(end as f64 * rational_seconds(self.time_base))
    }

    #[cfg(any(feature = "tool-inpaint", feature = "tool-upscale"))]
    pub fn pts_seconds(&self, pts: i64) -> Result<f64> {
        ensure!(pts >= 0, "source timeline PTS must be non-negative");
        Ok(pts as f64 * rational_seconds(self.time_base))
    }

    #[cfg(any(feature = "tool-inpaint", feature = "tool-upscale"))]
    pub const fn variable_frame_rate(&self) -> bool {
        self.variable_frame_rate
    }
}

/// Bounded source-audio decoder feeding the existing H.264/AAC muxer in output-time order.
///
/// The video is necessarily re-encoded after model inference, and the current muxer only accepts
/// decoded PCM, so source audio is decoded and encoded as AAC. Callers must emit a warning for
/// this lossy audio transcode. If the source has no audio, media-tool muxers are opened without an
/// audio stream so video-only input remains video-only.
#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
pub(super) struct VideoAudioBridge {
    source: Option<LibavAudioSource>,
    source_offset_frames: i64,
    output_cursor: i64,
}

#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
impl VideoAudioBridge {
    pub fn open(path: &Path, first_video_seconds: f64) -> Result<Self> {
        ensure!(
            first_video_seconds.is_finite(),
            "first video timestamp must be finite"
        );
        let audio_start = audio_stream_start_seconds(path)?;
        let source = match audio_start {
            Some(_) => Some(LibavAudioSource::open(
                path,
                AUDIO_SAMPLE_RATE,
                AUDIO_CHANNELS,
            )?),
            None => None,
        };
        let source_offset_frames = audio_start.map_or(0, |start| {
            ((first_video_seconds - start) * f64::from(AUDIO_SAMPLE_RATE)).round() as i64
        });
        Ok(Self {
            source,
            source_offset_frames,
            output_cursor: 0,
        })
    }

    pub const fn source_had_audio(&self) -> bool {
        self.source.is_some()
    }

    pub fn attach(&self, muxer: &mut Muxer) {
        // The bridge owns the exact output clock for source audio. Calling this for video-only
        // output is harmless and keeps callers topology-agnostic.
        muxer.expect_external_audio();
    }

    pub fn pump_to_seconds(&mut self, muxer: &mut Muxer, target_seconds: f64) -> Result<()> {
        ensure!(
            target_seconds.is_finite() && target_seconds >= 0.0,
            "audio target time must be finite and non-negative"
        );
        let target = (target_seconds * f64::from(AUDIO_SAMPLE_RATE)).round() as i64;
        ensure!(
            target >= self.output_cursor,
            "audio target moved backwards: {target} < {}",
            self.output_cursor
        );
        let Some(source) = self.source.as_mut() else {
            muxer.pump_silence_until(target_seconds)?;
            self.output_cursor = target;
            return Ok(());
        };
        while self.output_cursor < target {
            let end = target.min(self.output_cursor + AUDIO_CHUNK_FRAMES);
            let source_start = self.output_cursor + self.source_offset_frames;
            let source_end = end + self.source_offset_frames;
            let buffer = if source_end <= 0 {
                AudioBuffer::silence(
                    AUDIO_SAMPLE_RATE,
                    AUDIO_CHANNELS,
                    usize::try_from(end - self.output_cursor)
                        .context("audio silence length exceeds usize")?,
                )
            } else if source_start < 0 {
                let silence_frames = usize::try_from(-source_start)
                    .context("audio leading silence exceeds usize")?;
                let decoded = source.samples_by_index(0, source_end)?;
                let mut samples = vec![0.0; silence_frames * usize::from(AUDIO_CHANNELS)];
                samples.extend_from_slice(&decoded.samples);
                AudioBuffer {
                    samples,
                    sample_rate: AUDIO_SAMPLE_RATE,
                    channels: AUDIO_CHANNELS,
                }
            } else {
                source.samples_by_index(source_start, source_end)?
            };
            ensure!(
                buffer.frames() == usize::try_from(end - self.output_cursor)?,
                "audio bridge returned {} frames, expected {}",
                buffer.frames(),
                end - self.output_cursor
            );
            muxer.encode_audio(&buffer)?;
            self.output_cursor = end;
        }
        Ok(())
    }
}

#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
fn audio_stream_start_seconds(path: &Path) -> Result<Option<f64>> {
    ffmpeg_init();
    let input = format::input(path).with_context(|| format!("open {}", path.display()))?;
    let Some(stream) = input.streams().best(media::Type::Audio) else {
        return Ok(None);
    };
    let time_base = stream.time_base();
    ensure!(
        time_base.numerator() > 0 && time_base.denominator() > 0,
        "audio stream has invalid time base"
    );
    let start = if stream.start_time() == ff::ffi::AV_NOPTS_VALUE {
        0
    } else {
        stream.start_time()
    };
    Ok(Some(start as f64 * rational_seconds(time_base)))
}

#[cfg(any(feature = "tool-inpaint", feature = "tool-segment"))]
/// Rescale exactly as the packet path does: nearest integer, with half-way values away from zero.
/// An `i128` intermediate makes ordinary `i64` PTS / `i32` rational products lossless, while the
/// checked operations fail closed for adversarial time bases instead of wrapping.
pub(super) fn rescale_timestamp_nearest(
    value: i64,
    source_time_base: Rational,
    destination_time_base: Rational,
) -> Result<i64> {
    let source_num = i128::from(source_time_base.numerator());
    let source_den = i128::from(source_time_base.denominator());
    let destination_num = i128::from(destination_time_base.numerator());
    let destination_den = i128::from(destination_time_base.denominator());
    ensure!(
        source_num > 0 && source_den > 0 && destination_num > 0 && destination_den > 0,
        "cannot rescale a timestamp with a non-positive time base"
    );
    let scaled = i128::from(value)
        .checked_mul(source_num)
        .and_then(|value| value.checked_mul(destination_den))
        .context("timestamp rescale numerator overflowed")?;
    let divisor = source_den
        .checked_mul(destination_num)
        .context("timestamp rescale denominator overflowed")?;
    let half = divisor / 2;
    let rounded = if scaled >= 0 {
        scaled
            .checked_add(half)
            .context("positive timestamp rounding overflowed")?
    } else {
        scaled
            .checked_sub(half)
            .context("negative timestamp rounding overflowed")?
    } / divisor;
    i64::try_from(rounded).context("rescaled timestamp is outside the i64 range")
}

#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
fn exact_fps(time_base: Rational, step: i64) -> Result<(u32, u32)> {
    ensure!(
        time_base.numerator() > 0 && time_base.denominator() > 0 && step > 0,
        "cannot derive FPS from an invalid time base or PTS step"
    );
    let numerator = u64::try_from(time_base.denominator())?;
    let denominator = u64::try_from(time_base.numerator())?
        .checked_mul(u64::try_from(step)?)
        .context("FPS denominator overflowed")?;
    let divisor = gcd_u64(numerator, denominator);
    let numerator = numerator / divisor;
    let denominator = denominator / divisor;
    ensure!(
        numerator <= u64::from(u32::MAX) && denominator <= u64::from(u32::MAX),
        "exact source FPS {numerator}/{denominator} is outside the muxer range"
    );
    Ok((numerator as u32, denominator as u32))
}

#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
fn approximate_fps(fps: f64) -> Option<(u32, u32)> {
    const SCALE: f64 = 1_000_000.0;
    if !fps.is_finite() || fps <= 0.0 {
        return None;
    }
    let numerator = (fps * SCALE).round();
    if numerator <= 0.0 || numerator > u32::MAX as f64 {
        return None;
    }
    let numerator = numerator as u32;
    let denominator = SCALE as u32;
    let divisor = gcd_u64(u64::from(numerator), u64::from(denominator)) as u32;
    Some((numerator / divisor, denominator / divisor))
}

#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
fn gcd_u64(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left.max(1)
}

fn rational_seconds(value: Rational) -> f64 {
    value.numerator() as f64 / value.denominator() as f64
}

fn rational_to_positive_f64(value: Rational) -> Option<f64> {
    (value.numerator() > 0 && value.denominator() > 0)
        .then(|| value.numerator() as f64 / value.denominator() as f64)
}

fn square_pixel_width(coded_width: u32, aspect_ratio: Rational) -> Result<u32> {
    if aspect_ratio.numerator() <= 0 || aspect_ratio.denominator() <= 0 {
        return Ok(coded_width);
    }
    let scaled = u64::from(coded_width)
        .checked_mul(aspect_ratio.numerator() as u64)
        .context("sample-aspect-ratio width overflowed")?;
    let denominator = aspect_ratio.denominator() as u64;
    let rounded = scaled
        .checked_add(denominator / 2)
        .context("sample-aspect-ratio rounding overflowed")?
        / denominator;
    ensure!(
        rounded > 0 && rounded <= u64::from(u32::MAX),
        "sample-aspect-ratio produced unsupported width {rounded}"
    );
    Ok(rounded as u32)
}

fn rotate_rgba(source: RgbaFrame, degrees: u32) -> RgbaFrame {
    if degrees == 0 || source.width == 0 || source.height == 0 {
        return source;
    }
    let (width, height) = (source.width, source.height);
    let (output_width, output_height) = if degrees % 180 == 90 {
        (height, width)
    } else {
        (width, height)
    };
    let mut output = RgbaFrame::new(output_width, output_height);
    for y in 0..output_height {
        for x in 0..output_width {
            let (source_x, source_y) = rotated_source_coordinate(x, y, width, height, degrees);
            if let Some(pixel) = source.pixel(source_x, source_y) {
                output.set_pixel(x, y, pixel);
            }
        }
    }
    output
}

#[cfg(any(feature = "tool-inpaint", feature = "tool-segment"))]
fn rotate_gray8(source: Gray8Frame, degrees: u32) -> Gray8Frame {
    if degrees == 0 || source.width == 0 || source.height == 0 {
        return source;
    }
    let (width, height) = (source.width, source.height);
    let (output_width, output_height) = if degrees % 180 == 90 {
        (height, width)
    } else {
        (width, height)
    };
    let mut output = Gray8Frame::new(output_width, output_height);
    for y in 0..output_height {
        for x in 0..output_width {
            let (source_x, source_y) = rotated_source_coordinate(x, y, width, height, degrees);
            output.data[y as usize * output_width as usize + x as usize] =
                source.data[source_y as usize * width as usize + source_x as usize];
        }
    }
    output
}

fn rotated_source_coordinate(x: u32, y: u32, width: u32, height: u32, degrees: u32) -> (u32, u32) {
    match degrees {
        90 => (y, height - 1 - x),
        180 => (width - 1 - x, height - 1 - y),
        270 => (width - 1 - y, x),
        _ => (x, y),
    }
}

fn stream_rotation_degrees(stream: &ff::format::stream::Stream<'_>) -> u32 {
    const DISPLAY_MATRIX_BYTES: usize = 9 * size_of::<i32>();
    for side_data in stream.side_data() {
        if side_data.kind() == ff::codec::packet::side_data::Type::DisplayMatrix
            && side_data.data().len() >= DISPLAY_MATRIX_BYTES
        {
            let counter_clockwise = unsafe {
                ff::ffi::av_display_rotation_get(side_data.data().as_ptr().cast::<i32>())
            };
            if counter_clockwise.is_finite() {
                let clockwise = (-counter_clockwise).rem_euclid(360.0);
                return (((clockwise / 90.0).round() as u32) % 4) * 90;
            }
        }
    }
    0
}

#[cfg(all(
    test,
    any(
        feature = "tool-inpaint",
        feature = "tool-interpolate",
        feature = "tool-upscale"
    )
))]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "tool-inpaint")]
    fn rational_timestamp_rescale_is_exact_for_container_ticks() {
        assert_eq!(
            rescale_timestamp_nearest(3_003, Rational(1, 90_000), Rational(1, 1_000)).unwrap(),
            33
        );
        assert_eq!(
            rescale_timestamp_nearest(3_003, Rational(1, 90_000), Rational(1, 30_000)).unwrap(),
            1_001
        );
        assert_eq!(
            rescale_timestamp_nearest(3_005, Rational(1, 90_000), Rational(1, 30_000)).unwrap(),
            1_002
        );
        assert_eq!(
            rescale_timestamp_nearest(-3_003, Rational(1, 90_000), Rational(1, 1_000)).unwrap(),
            -33
        );
        assert_eq!(
            rescale_timestamp_nearest(-1, Rational(1, 2), Rational(1, 1)).unwrap(),
            -1
        );
        assert!(
            rescale_timestamp_nearest(i64::MAX, Rational(i32::MAX, 1), Rational(1, i32::MAX))
                .is_err()
        );
    }

    #[test]
    #[cfg(feature = "tool-inpaint")]
    fn full_pair_compares_absolute_start_instead_of_independent_rebases() {
        assert!(
            validate_pair_timestamp(
                0,
                90_000,
                0,
                Rational(1, 90_000),
                0,
                Rational(1, 1_000),
                false,
            )
            .is_err()
        );
        assert!(
            validate_pair_timestamp(
                0,
                90_000,
                0,
                Rational(1, 90_000),
                1_000,
                Rational(1, 1_000),
                false,
            )
            .is_ok()
        );
    }

    #[test]
    #[cfg(feature = "tool-inpaint")]
    fn range_pair_requires_zero_based_mask_timestamps() {
        assert!(
            validate_pair_timestamp(
                0,
                90_000,
                0,
                Rational(1, 90_000),
                0,
                Rational(1, 1_000),
                true,
            )
            .is_ok()
        );
        assert!(
            validate_pair_timestamp(
                0,
                90_000,
                0,
                Rational(1, 90_000),
                1_000,
                Rational(1, 1_000),
                true,
            )
            .is_err()
        );
        assert!(
            validate_pair_timestamp(
                1,
                93_003,
                3_003,
                Rational(1, 90_000),
                33,
                Rational(1, 1_000),
                true,
            )
            .is_ok()
        );
        assert!(
            validate_pair_timestamp(
                1,
                93_003,
                3_003,
                Rational(1, 90_000),
                34,
                Rational(1, 1_000),
                true,
            )
            .is_err()
        );
    }

    #[test]
    #[cfg(any(
        feature = "tool-inpaint",
        feature = "tool-interpolate",
        feature = "tool-upscale"
    ))]
    fn source_clock_preserves_fractional_hint_and_accepts_vfr_pts() {
        let mut clock = SourceTimeline::derive(0, Some(3_003), Rational(1, 90_000), 29.97).unwrap();
        assert_eq!(clock.fps(), (30_000, 1_001));
        clock.observe(0, 0, Some(3_003)).unwrap();
        clock.observe(1, 3_003, Some(2_997)).unwrap();
        clock.observe(2, 6_000, Some(4_000)).unwrap();
        assert!((clock.duration_seconds().unwrap() - 1.0 / 9.0).abs() < 1e-12);
        assert!(clock.observe(3, 6_000, None).is_err());
    }

    #[test]
    #[cfg(feature = "tool-inpaint")]
    fn rgba_and_gray_rotation_use_the_same_display_mapping() {
        let mut rgba = RgbaFrame::new(2, 1);
        rgba.set_pixel(0, 0, [10, 0, 0, 255]);
        rgba.set_pixel(1, 0, [20, 0, 0, 255]);
        let gray = Gray8Frame::from_data(2, 1, vec![0, 255]).unwrap();
        let rgba = rotate_rgba(rgba, 90);
        let gray = rotate_gray8(gray, 90);
        assert_eq!((rgba.width, rgba.height), (1, 2));
        assert_eq!((gray.width, gray.height), (1, 2));
        assert_eq!(rgba.pixel(0, 0).unwrap()[0], 10);
        assert_eq!(rgba.pixel(0, 1).unwrap()[0], 20);
        assert_eq!(gray.data, vec![0, 255]);
    }
}
