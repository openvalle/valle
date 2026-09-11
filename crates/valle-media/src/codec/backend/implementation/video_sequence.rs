//! ABI-specific sequential readers used by frame-derived media tools.
use super::ff;
use super::ffi::{
    ffmpeg_init, guessed_sample_aspect_ratio, matrix_for, read_plane0, set_sws_colorspace,
};
#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
use crate::codec::sequence::DecodedVideoTiming;
use crate::codec::sequence::{TimedVideoFrame, VideoMetadata, rotate_rgba};
use crate::frame::RgbaFrame;
#[cfg(any(feature = "tool-inpaint", feature = "tool-segment"))]
use crate::{codec::sequence::rotate_gray8, frame::Gray8Frame};
use anyhow::{Context as _, Result, anyhow, ensure};
use ff::{Packet, Rational, codec, color, format, frame, media, software};
use std::{mem::size_of, path::Path};
/// Reopen a staged video, decode every frame, and validate its zero-based, strictly increasing
/// PTS. This catches truncated outputs and muxer timestamp regressions while retaining only one
/// decoder frame and scalar timing state.
#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
pub(crate) fn decode_video_timing(path: &Path, threads: usize) -> Result<DecodedVideoTiming> {
    ffmpeg_init()?;
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
        ffmpeg_init()?;
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
                time_base: time_base.into(),
                start_pts,
                rotation_degrees,
                sample_aspect_ratio: sample_aspect_ratio.into(),
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
pub(crate) struct SequentialRgbaDecoder(SequentialDecoder);

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
pub(crate) struct SequentialGray8Decoder(SequentialDecoder);

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

#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
pub(crate) fn audio_stream_start_seconds(path: &Path) -> Result<Option<f64>> {
    ffmpeg_init()?;
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
#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-interpolate",
    feature = "tool-upscale"
))]
fn rational_seconds(value: impl Into<Rational>) -> f64 {
    let value = value.into();
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
