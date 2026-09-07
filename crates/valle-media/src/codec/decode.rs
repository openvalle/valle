//! Native libav video decoding with full-frame collection for verification and a streaming source
//! for rendering. Keep one lookahead frame and defer CPU color conversion or GPU surface
//! consumption until a frame is actually used.

use std::ffi::c_void;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use crate::frame::RgbaFrame;
use anyhow::{Context as _, Result, anyhow};
use ff::{Packet, Rational, codec, color, format, frame, media, software};
use ffmpeg_next as ff;

use crate::ffi::{
    YuvMatrix, ffmpeg_init, guessed_sample_aspect_ratio, matrix_for, read_plane0,
    set_sws_colorspace,
};
use crate::transport::shared_frame::{
    SharedFrameBackend, SharedVideoFrameHandle, VideoFrameTransport,
};
use crate::transport::source::{SourceFrame, SourceMeta, VideoSource};

/// Backward-seek tolerance in seconds, large enough to absorb floating-point noise but below normal
/// frame intervals.
const SEEK_BACK_EPS_S: f64 = 1e-3;
/// Seek across large forward gaps instead of decoding every skipped frame.
const FORWARD_SEEK_THRESHOLD_S: f64 = 1.0;
/// Timestamp comparison tolerance for requests exactly on frame boundaries.
const FRAME_TIME_EPS_S: f64 = 1e-6;

/// Decode up to max frames as RGBA; None decodes the full video.
pub fn decode_rgba_frames(path: &Path, max: Option<usize>) -> Result<Vec<RgbaFrame>> {
    ffmpeg_init();
    let mut ictx = format::input(&path).map_err(|e| anyhow!("open {}: {e}", path.display()))?;
    let (vidx, params) = {
        let s = ictx
            .streams()
            .best(media::Type::Video)
            .ok_or_else(|| anyhow!("no video stream"))?;
        (s.index(), s.parameters())
    };
    let mut decoder = codec::context::Context::from_parameters(params)?
        .decoder()
        .video()?;
    let (w, h) = (decoder.width(), decoder.height());
    let mut sws = software::scaling::Context::get(
        decoder.format(),
        w,
        h,
        format::Pixel::RGBA,
        w,
        h,
        software::scaling::Flags::BILINEAR,
    )?;
    // Match the streaming decoder's explicit color matrix and range selection.
    let yuv_full = decoder.color_range() == color::Range::JPEG;
    set_sws_colorspace(
        &mut sws,
        matrix_for(decoder.color_space(), h),
        yuv_full,
        true,
    );

    let mut out = Vec::new();
    let row = w as usize * 4;
    let mut recv = |decoder: &mut ff::decoder::Video, out: &mut Vec<RgbaFrame>| -> Result<()> {
        let mut f = frame::Video::empty();
        while decoder.receive_frame(&mut f).is_ok() {
            if max.is_some_and(|m| out.len() >= m) {
                break;
            }
            let mut rgba = frame::Video::empty();
            sws.run(&f, &mut rgba)?;
            out.push(RgbaFrame {
                width: w,
                height: h,
                data: read_plane0(&rgba, row, h),
            });
        }
        Ok(())
    };

    for (stream, pkt) in ictx.packets() {
        if max.is_some_and(|m| out.len() >= m) {
            break;
        }
        if stream.index() != vidx {
            continue;
        }
        decoder.send_packet(&pkt)?;
        recv(&mut decoder, &mut out)?;
    }
    decoder.send_eof()?;
    recv(&mut decoder, &mut out)?;
    Ok(out)
}

/// Probe video dimensions without decoding all frames.
pub fn probe_dimensions(path: &Path) -> Result<(u32, u32)> {
    ffmpeg_init();
    let ictx = format::input(&path).map_err(|e| anyhow!("open {}: {e}", path.display()))?;
    let s = ictx
        .streams()
        .best(media::Type::Video)
        .ok_or_else(|| anyhow!("no video stream"))?;
    let decoder = codec::context::Context::from_parameters(s.parameters())?
        .decoder()
        .video()?;
    Ok((decoder.width(), decoder.height()))
}

/// Container audio/video stream metadata without full decoding.
#[derive(Debug, Clone, PartialEq)]
pub struct AvProbe {
    /// Optional video width, height, and duration in seconds.
    pub video: Option<(u32, u32, f64)>,
    /// Optional audio duration in seconds.
    pub audio: Option<f64>,
}

/// Probe detail needed for run-report summaries but intentionally kept out of the public
/// [`AvProbe`] contract.
pub(crate) struct AvProbeDetails {
    pub probe: AvProbe,
    pub video_pixel_format: Option<String>,
    pub audio_sample_rate_hz: Option<u32>,
    pub audio_channels: Option<u16>,
    pub audio_sample_format: Option<String>,
}

/// Probe audio/video stream presence and duration.
pub fn probe_av(path: &Path) -> Result<AvProbe> {
    Ok(probe_av_details(path)?.probe)
}

/// The same lightweight container probe with decoder format details for internal reports.
pub(crate) fn probe_av_details(path: &Path) -> Result<AvProbeDetails> {
    ffmpeg_init();
    let ictx = format::input(&path).map_err(|e| anyhow!("open {}: {e}", path.display()))?;
    let mut details = AvProbeDetails {
        probe: AvProbe {
            video: None,
            audio: None,
        },
        video_pixel_format: None,
        audio_sample_rate_hz: None,
        audio_channels: None,
        audio_sample_format: None,
    };
    for s in ictx.streams() {
        let tb = s.time_base();
        // Convert stream duration ticks to seconds; missing duration or invalid time base yields
        // zero.
        let dur_s = if s.duration() > 0 && tb.denominator() != 0 {
            s.duration() as f64 * tb.numerator() as f64 / tb.denominator() as f64
        } else {
            0.0
        };
        match s.parameters().medium() {
            media::Type::Video if details.probe.video.is_none() => {
                let dec = codec::context::Context::from_parameters(s.parameters())?
                    .decoder()
                    .video()?;
                details.probe.video = Some((dec.width(), dec.height(), dur_s));
                details.video_pixel_format = dec
                    .format()
                    .descriptor()
                    .map(|descriptor| descriptor.name().to_owned());
            }
            media::Type::Audio if details.probe.audio.is_none() => {
                details.probe.audio = Some(dur_s);
                if let Ok(decoder) = codec::context::Context::from_parameters(s.parameters())
                    .and_then(|context| context.decoder().audio())
                {
                    details.audio_sample_rate_hz = (decoder.rate() > 0).then_some(decoder.rate());
                    details.audio_channels = (decoder.channels() > 0).then_some(decoder.channels());
                    details.audio_sample_format = sample_format_name(decoder.format());
                }
            }
            _ => {}
        }
    }
    Ok(details)
}

fn sample_format_name(sample: format::Sample) -> Option<String> {
    use format::{Sample, sample::Type};

    let name = match sample {
        Sample::None => return None,
        Sample::U8(Type::Packed) => "u8",
        Sample::U8(Type::Planar) => "u8p",
        Sample::I16(Type::Packed) => "s16",
        Sample::I16(Type::Planar) => "s16p",
        Sample::I32(Type::Packed) => "s32",
        Sample::I32(Type::Planar) => "s32p",
        Sample::I64(Type::Packed) => "s64",
        Sample::I64(Type::Planar) => "s64p",
        Sample::F32(Type::Packed) => "f32",
        Sample::F32(Type::Planar) => "f32p",
        Sample::F64(Type::Packed) => "f64",
        Sample::F64(Type::Planar) => "f64p",
    };
    Some(name.to_owned())
}

/// Streaming video source returning the latest frame at or before source-local time. Decode
/// sequentially for forward requests and seek for backward or large jumps. Keep only current and
/// lookahead frames. Convert returned frames lazily with shared cached results, explicit color
/// metadata, square-pixel normalization, and display rotation.
pub struct LibavVideoSource {
    ictx: format::context::Input,
    decoder: ff::decoder::Video,
    /// Conversion parameters captured at open and carried by lazy frames.
    conv: ConvertParams,
    stream_idx: usize,
    stream_tb: ff::Rational,
    /// Stream start timestamp used to translate between stream and source-local time.
    start_time_ticks: i64,
    /// Nominal frame duration in stream ticks, used only when timestamps are unavailable.
    frame_dur_ticks: f64,
    meta: SourceMeta,
    /// Latest frame at or before the last requested time.
    cur: Option<Decoded>,
    /// One decoded future frame retained for the next request.
    lookahead: Option<Decoded>,
    /// EOF has been sent and the decoder is draining.
    draining: bool,
    /// Decoded-frame count since the last seek for timestamp synthesis.
    since_seek: i64,
    /// Source EOF reached; forward requests reuse the final frame without repeated seeking and
    /// conversion. Seeking resets this state.
    exhausted: bool,
}

/// Decoded frame with lazy RGBA conversion; discarded catch-up frames only inspect timestamps.
struct Decoded {
    time_s: f64,
    /// Decoded YUV frame, moved into lazy storage on first use; mutually exclusive with lazy.
    yuv: Option<frame::Video>,
    /// Shared lazy frame cached on first use; conversion runs on the consumer thread.
    lazy: Option<Arc<SourceFrame>>,
}

/// Copyable parameters required to recreate color conversion, normalization, and rotation.
#[derive(Clone, Copy)]
struct ConvertParams {
    /// Pixel format captured at open; midstream format changes are unsupported.
    src_fmt: format::Pixel,
    /// Decoder/coded dimensions. A hardware surface always keeps this geometry.
    coded_w: u32,
    coded_h: u32,
    /// Square-pixel dimensions produced by swscale before display rotation. For ordinary SAR 1:1
    /// media these equal the coded dimensions.
    square_w: u32,
    square_h: u32,
    /// Tightly packed RGBA row size in bytes.
    row: usize,
    /// Clockwise display rotation: 0, 90, 180, or 270 degrees.
    rotation_deg: u32,
    /// Tagged color matrix and full-range flag, with dimension-based defaults when tags are absent.
    matrix: YuvMatrix,
    yuv_full: bool,
}

// SAFETY: All libav handles are exclusively owned and accessed by one thread at a time. Moving the
// source transfers ownership without shared aliases.
unsafe impl Send for LibavVideoSource {}

impl LibavVideoSource {
    /// Open a streaming video source; decode the first frame on demand.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_transport(path, VideoFrameTransport::Cpu)
    }

    /// Open a CPU-decoded source with an explicit software decoder thread ceiling.
    pub fn open_with_threads(path: &Path, threads: usize) -> Result<Self> {
        if threads == 0 {
            return Err(anyhow!("video decoder thread count must be positive"));
        }
        Self::open_with_transport_and_threads(path, VideoFrameTransport::Cpu, Some(threads))
    }

    /// Open a streaming source and optionally request platform hardware frames. Unsupported codecs,
    /// machines and formats fall back to the existing software decoder without changing semantics.
    pub fn open_with_transport(path: &Path, transport: VideoFrameTransport) -> Result<Self> {
        Self::open_with_transport_and_threads(path, transport, None)
    }

    fn open_with_transport_and_threads(
        path: &Path,
        transport: VideoFrameTransport,
        requested_threads: Option<usize>,
    ) -> Result<Self> {
        ffmpeg_init();
        let mut ictx = format::input(&path).map_err(|e| anyhow!("open {}: {e}", path.display()))?;
        let (stream_idx, params, stream_tb, start_time, duration, avg_fr) = {
            let s = ictx
                .streams()
                .best(media::Type::Video)
                .ok_or_else(|| anyhow!("no video stream"))?;
            (
                s.index(),
                s.parameters(),
                s.time_base(),
                s.start_time(),
                s.duration(),
                s.avg_frame_rate(),
            )
        };
        let rotation_deg = {
            let s = ictx
                .stream(stream_idx)
                .expect("stream index from best() is valid");
            stream_rotation_deg(&s)
        };
        let container_aspect_ratio = guessed_sample_aspect_ratio(&mut ictx, stream_idx)?;
        // Configure frame-threaded decoding before opening the codec context.
        let mut codec_ctx = codec::context::Context::from_parameters(params.clone())?;
        #[cfg(target_os = "macos")]
        let hardware_decode = transport == VideoFrameTransport::SharedGpu
            && configure_videotoolbox_decoder(&mut codec_ctx);
        #[cfg(not(target_os = "macos"))]
        let hardware_decode = {
            let _ = transport;
            false
        };
        let software_thread_count = requested_threads.unwrap_or_else(|| {
            std::env::var("VALLE_DECODE_THREADS")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or_else(|| {
                    std::thread::available_parallelism()
                        .map(|n| n.get().saturating_sub(2).max(1))
                        .unwrap_or(2)
                })
        });
        let mut th = codec::threading::Config::kind(codec::threading::Type::Frame);
        // Reserve CPU capacity for rendering and encoding. VALLE_DECODE_THREADS overrides the
        // default; zero delegates thread selection to libav.
        th.count = software_thread_count;
        if hardware_decode {
            // VideoToolbox owns the codec parallelism. Software frame threads around it only add
            // buffering and make the decoded CVPixelBuffer lifetime harder to bound.
            th.count = 1;
        }
        codec_ctx.set_threading(th);
        let decoder = match codec_ctx.decoder().video() {
            Ok(decoder) => decoder,
            Err(hardware_error) if hardware_decode => {
                // A host may expose a VideoToolbox device and codec config while rejecting this
                // particular stream/profile when avcodec_open2 creates the session. Rebuild a
                // pristine software context instead of turning an optional optimization into a
                // missing visual source.
                let mut fallback = codec::context::Context::from_parameters(params)?;
                let mut threading = codec::threading::Config::kind(codec::threading::Type::Frame);
                threading.count = software_thread_count;
                fallback.set_threading(threading);
                fallback.decoder().video().map_err(|software_error| {
                    anyhow!(
                        "open VideoToolbox decoder ({hardware_error}); software fallback also failed: {software_error}"
                    )
                })?
            }
            Err(error) => return Err(error.into()),
        };
        let (coded_w, coded_h) = (decoder.width(), decoder.height());
        let sample_aspect_ratio = if valid_aspect_ratio(container_aspect_ratio) {
            container_aspect_ratio
        } else {
            decoder.aspect_ratio()
        };
        let square_w = square_pixel_width(coded_w, sample_aspect_ratio)?;
        let square_h = coded_h;
        let square_row = usize::try_from(square_w)
            .ok()
            .and_then(|width| width.checked_mul(4))
            .context("square-pixel RGBA row size overflowed")?;
        // Capture conversion parameters so lazy conversion uses the same scaling, matrix, and range
        // configuration.
        let conv = ConvertParams {
            src_fmt: decoder.format(),
            coded_w,
            coded_h,
            square_w,
            square_h,
            row: square_row,
            rotation_deg,
            matrix: matrix_for(decoder.color_space(), coded_h),
            yuv_full: decoder.color_range() == color::Range::JPEG,
        };

        let start_time_ticks = if start_time == ff::ffi::AV_NOPTS_VALUE {
            0
        } else {
            start_time
        };
        // Convert ticks using the stream time-base ratio.
        let tb_secs = stream_tb.numerator() as f64 / stream_tb.denominator().max(1) as f64;
        let nominal_fps = {
            let (n, d) = (avg_fr.numerator() as f64, avg_fr.denominator() as f64);
            if n > 0.0 && d > 0.0 { n / d } else { 0.0 }
        };
        // Derive nominal frame duration from frame rate and time base; use one tick when unknown.
        let frame_dur_ticks = if nominal_fps > 0.0 && stream_tb.numerator() != 0 {
            stream_tb.denominator() as f64 / (stream_tb.numerator() as f64 * nominal_fps)
        } else {
            1.0
        };
        let duration_s = (duration != ff::ffi::AV_NOPTS_VALUE && duration > 0)
            .then_some(duration as f64 * tb_secs);

        // Display metadata swaps dimensions for quarter-turn rotations.
        let (dw, dh) = if rotation_deg % 180 == 90 {
            (square_h, square_w)
        } else {
            (square_w, square_h)
        };
        let meta = SourceMeta {
            width: dw,
            height: dh,
            rotation_deg: rotation_deg as i32,
            nominal_fps,
            start_time_s: start_time_ticks as f64 * tb_secs,
            duration_s,
        };

        Ok(Self {
            ictx,
            decoder,
            conv,
            stream_idx,
            stream_tb,
            start_time_ticks,
            frame_dur_ticks,
            meta,
            cur: None,
            lookahead: None,
            draining: false,
            since_seek: 0,
            exhausted: false,
        })
    }

    /// Convert stream ticks to seconds.
    fn ticks_to_secs(&self, ticks: i64) -> f64 {
        ticks as f64 * self.stream_tb.numerator() as f64
            / self.stream_tb.denominator().max(1) as f64
    }

    /// Convert seconds to stream ticks; invalid time bases return zero.
    fn secs_to_ticks(&self, s: f64) -> i64 {
        if self.stream_tb.numerator() == 0 {
            return 0;
        }
        (s * self.stream_tb.denominator() as f64 / self.stream_tb.numerator() as f64) as i64
    }

    /// Use the best-effort timestamp, then PTS, then decoded order and nominal duration to compute
    /// source-local presentation time.
    fn frame_time_s(&self, decoded: &frame::Video) -> f64 {
        let ticks = decoded
            .timestamp()
            .or_else(|| decoded.pts())
            .unwrap_or_else(|| {
                self.start_time_ticks
                    + (self.since_seek as f64 * self.frame_dur_ticks).round() as i64
            });
        self.ticks_to_secs(ticks - self.start_time_ticks)
    }

    /// Decode the next frame without RGBA conversion; return None at EOF.
    fn pull(&mut self) -> Result<Option<Decoded>> {
        loop {
            let mut decoded = frame::Video::empty();
            if self.decoder.receive_frame(&mut decoded).is_ok() {
                let t = self.frame_time_s(&decoded);
                self.since_seek += 1;
                return Ok(Some(Decoded {
                    time_s: t,
                    yuv: Some(decoded),
                    lazy: None,
                }));
            }
            if self.draining {
                return Ok(None); // No frame after decoder drain indicates EOF.
            }
            let mut pkt = Packet::empty();
            match pkt.read(&mut self.ictx) {
                Ok(()) => {
                    if pkt.stream() == self.stream_idx {
                        self.decoder.send_packet(&pkt).context("send_packet")?;
                    }
                    // Skip packets belonging to other streams.
                }
                Err(ff::Error::Eof) => {
                    self.decoder.send_eof().context("send_eof")?;
                    self.draining = true;
                }
                Err(e) => return Err(anyhow!("packet read: {e:?}")),
            }
        }
    }

    /// Seek to the preceding keyframe and reset decoder state. On seek failure, continue
    /// best-effort decoding from the current position.
    fn seek(&mut self, want_s: f64) {
        let target = self.start_time_ticks + self.secs_to_ticks(want_s);
        unsafe {
            let _ = ff::ffi::av_seek_frame(
                self.ictx.as_mut_ptr(),
                self.stream_idx as i32,
                target,
                ff::ffi::AVSEEK_FLAG_BACKWARD,
            );
        }
        self.decoder.flush(); // Flush after seeking to discard frames retained from the previous GOP.
        self.cur = None;
        self.lookahead = None;
        self.draining = false;
        self.since_seek = 0;
        self.exhausted = false; // Reset EOF state after seeking.
    }
}

impl LibavVideoSource {
    /// Shared cursor advancement and lazy-frame selection for eager and deferred frame access.
    fn lazy_at(&mut self, t: f64) -> Result<Arc<SourceFrame>> {
        let want = t.max(0.0);
        let need_seek = match &self.cur {
            // For the first nonzero request, seek directly unless a retained lookahead frame
            // already supports sequential decoding.
            None => want > 0.0 && self.lookahead.is_none(),
            Some(cur) => {
                let backward = want + SEEK_BACK_EPS_S < cur.time_s;
                // Suppress forward seeks after EOF, while allowing backward seeks to earlier
                // frames.
                let big_forward = want > cur.time_s + FORWARD_SEEK_THRESHOLD_S;
                backward || (big_forward && !self.exhausted)
            }
        };
        if need_seek {
            self.seek(want);
        }
        loop {
            if let Some(la) = &self.lookahead {
                if la.time_s <= want + FRAME_TIME_EPS_S {
                    self.cur = self.lookahead.take(); // Promote lookahead frames that do not exceed the requested time.
                    continue;
                }
                break; // A future lookahead frame leaves the current frame as the answer.
            }
            match self.pull()? {
                Some(d) => {
                    if d.time_s <= want + FRAME_TIME_EPS_S {
                        self.cur = Some(d); // Release replaced frames without converting their pixels.
                    } else {
                        self.lookahead = Some(d); // Retain a future frame for the next request.
                        break;
                    }
                }
                None => {
                    self.exhausted = true; // At EOF, retain the final frame and suppress later forward seeks.
                    break;
                }
            }
        }
        let conv = self.conv;
        if let Some(cur) = self.cur.as_mut() {
            ensure_lazy(cur, conv) // Share frames through Arc for parallel rendering and repeated requests.
        } else if let Some(la) = self.lookahead.as_mut() {
            // Clamp requests before the first timestamp to the earliest available frame.
            ensure_lazy(la, conv)
        } else {
            Err(anyhow!("no frames decoded from source"))
        }
    }
}

impl VideoSource for LibavVideoSource {
    fn meta(&self) -> &SourceMeta {
        &self.meta
    }

    fn frame_at(&mut self, t: f64) -> Result<Arc<RgbaFrame>> {
        self.lazy_at(t)?.rgba() // The eager API converts on the calling thread using the same cached result.
    }

    fn frame_at_lazy(&mut self, t: f64) -> Result<Arc<SourceFrame>> {
        self.lazy_at(t)
    }
}

/// Reuse an existing lazy frame or move decoded YUV into one. Pixel conversion is deferred until
/// first consumption.
fn ensure_lazy(slot: &mut Decoded, conv: ConvertParams) -> Result<Arc<SourceFrame>> {
    if let Some(l) = &slot.lazy {
        return Ok(l.clone());
    }
    let yuv = slot
        .yuv
        .take()
        .ok_or_else(|| anyhow!("decoded slot missing yuv (invariant: yuv xor lazy)"))?;
    let arc = if yuv.format() == format::Pixel::VIDEOTOOLBOX {
        Arc::new(SourceFrame::DecodedGpu(DecodedGpuFrame {
            frame: yuv,
            conv,
            rgba: OnceLock::new(),
        }))
    } else {
        Arc::new(SourceFrame::LazyYuv(LazyYuv {
            yuv: Mutex::new(Some(yuv)),
            conv,
            rgba: OnceLock::new(),
        }))
    };
    slot.lazy = Some(arc.clone());
    Ok(arc)
}

/// Lazy YUV-to-RGBA conversion executed once on the consuming thread. Recreate conversion state
/// from captured parameters, copy planes, apply rotation, then release YUV and cache the result.
pub struct LazyYuv {
    /// Take the decoded frame during conversion so YUV and cached RGBA are not retained together.
    yuv: Mutex<Option<frame::Video>>,
    conv: ConvertParams,
    /// Cache the first conversion result; OnceLock serializes concurrent first access.
    rgba: OnceLock<std::result::Result<Arc<RgbaFrame>, String>>,
}

// SAFETY: The decoded AVFrame is exclusively owned and no longer modified by the decoder. A mutex
// permits one take, OnceLock serializes conversion, and each swscale context is created and
// destroyed on the converting thread.
unsafe impl Send for LazyYuv {}
unsafe impl Sync for LazyYuv {}

impl LazyYuv {
    /// Convert once, record conversion timing, and share the resulting Arc across later callers.
    pub fn rgba(&self) -> Result<Arc<RgbaFrame>> {
        self.rgba
            .get_or_init(|| {
                crate::perf::time_src_convert(|| self.convert()).map_err(|e| format!("{e:#}"))
            })
            .clone()
            .map_err(|e| anyhow!("lazy yuv→rgba: {e}"))
    }

    /// Recreate swscale, convert and copy pixels, then apply rotation.
    fn convert(&self) -> Result<Arc<RgbaFrame>> {
        let p = self.conv;
        let yuv = self
            .yuv
            .lock()
            .expect("lazy yuv lock")
            .take()
            .ok_or_else(|| anyhow!("lazy yuv already consumed (invariant)"))?;
        convert_video_frame(yuv, p)
    }
}

/// A decoder-owned platform surface. The AVFrame retains its CVPixelBuffer until every raster job
/// referencing this object has completed. CPU access remains available through one memoized
/// `av_hwframe_transfer_data` + swscale conversion.
pub struct DecodedGpuFrame {
    frame: frame::Video,
    conv: ConvertParams,
    rgba: OnceLock<std::result::Result<Arc<RgbaFrame>, String>>,
}

unsafe impl Send for DecodedGpuFrame {}
unsafe impl Sync for DecodedGpuFrame {}

impl DecodedGpuFrame {
    pub fn backend(&self) -> SharedFrameBackend {
        SharedFrameBackend::VideoToolbox
    }

    /// Display dimensions after applying container rotation.
    pub fn dimensions(&self) -> (u32, u32) {
        if self.conv.rotation_deg % 180 == 90 {
            (self.conv.square_h, self.conv.square_w)
        } else {
            (self.conv.square_w, self.conv.square_h)
        }
    }

    pub fn coded_dimensions(&self) -> (u32, u32) {
        (self.conv.coded_w, self.conv.coded_h)
    }

    /// A decoder-owned surface can be imported directly only when its coded pixels already have
    /// square geometry. Otherwise the CPU fallback must run swscale to honor the source SAR.
    pub fn supports_direct_import(&self) -> bool {
        (self.conv.coded_w, self.conv.coded_h) == (self.conv.square_w, self.conv.square_h)
    }

    pub fn rotation_deg(&self) -> u32 {
        self.conv.rotation_deg
    }

    pub fn is_full_range(&self) -> bool {
        self.conv.yuv_full
    }

    pub fn matrix(&self) -> YuvMatrix {
        self.conv.matrix
    }

    pub fn handle(&self) -> SharedVideoFrameHandle {
        let native = unsafe { (*self.frame.as_ptr()).data[3].cast::<c_void>() };
        SharedVideoFrameHandle::VideoToolbox(native)
    }

    pub fn rgba(&self) -> Result<Arc<RgbaFrame>> {
        self.rgba
            .get_or_init(|| {
                crate::perf::time_src_convert(|| self.transfer_and_convert())
                    .map_err(|error| format!("{error:#}"))
            })
            .clone()
            .map_err(|error| anyhow!("hardware frame→rgba: {error}"))
    }

    fn transfer_and_convert(&self) -> Result<Arc<RgbaFrame>> {
        let mut software_frame = frame::Video::empty();
        let code = unsafe {
            ff::ffi::av_hwframe_transfer_data(software_frame.as_mut_ptr(), self.frame.as_ptr(), 0)
        };
        if code < 0 {
            return Err(anyhow!(
                "transfer hardware decode frame: {}",
                ff::Error::from(code)
            ));
        }
        let mut conv = self.conv;
        conv.src_fmt = software_frame.format();
        convert_video_frame(software_frame, conv)
    }
}

fn convert_video_frame(yuv: frame::Video, p: ConvertParams) -> Result<Arc<RgbaFrame>> {
    let mut sws = software::scaling::Context::get(
        p.src_fmt,
        p.coded_w,
        p.coded_h,
        format::Pixel::RGBA,
        p.square_w,
        p.square_h,
        software::scaling::Flags::BILINEAR,
    )?;
    set_sws_colorspace(&mut sws, p.matrix, p.yuv_full, true);
    let mut rgba = frame::Video::empty();
    sws.run(&yuv, &mut rgba)?;
    let f = RgbaFrame {
        width: p.square_w,
        height: p.square_h,
        data: read_plane0(&rgba, p.row, p.square_h),
    };
    Ok(Arc::new(rotate_rgba(f, p.rotation_deg)))
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn choose_videotoolbox_format(
    _context: *mut ff::ffi::AVCodecContext,
    formats: *const ff::ffi::AVPixelFormat,
) -> ff::ffi::AVPixelFormat {
    if formats.is_null() {
        return ff::ffi::AVPixelFormat::AV_PIX_FMT_NONE;
    }
    let mut cursor = formats;
    let mut software = ff::ffi::AVPixelFormat::AV_PIX_FMT_NONE;
    loop {
        let format = unsafe { *cursor };
        if format == ff::ffi::AVPixelFormat::AV_PIX_FMT_NONE {
            break;
        }
        if format == ff::ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX {
            return format;
        }
        if software == ff::ffi::AVPixelFormat::AV_PIX_FMT_NONE {
            software = format;
        }
        cursor = unsafe { cursor.add(1) };
    }
    software
}

#[cfg(target_os = "macos")]
fn configure_videotoolbox_decoder(context: &mut codec::context::Context) -> bool {
    unsafe {
        let raw = context.as_mut_ptr();
        let codec = ff::ffi::avcodec_find_decoder((*raw).codec_id);
        if codec.is_null() {
            return false;
        }
        let mut supported = false;
        let mut index = 0;
        loop {
            let config = ff::ffi::avcodec_get_hw_config(codec, index);
            if config.is_null() {
                break;
            }
            if (*config).device_type == ff::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX
                && (*config).pix_fmt == ff::ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX
                && ((*config).methods & ff::ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32)
                    != 0
            {
                supported = true;
                break;
            }
            index += 1;
        }
        if !supported {
            return false;
        }
        let mut device = std::ptr::null_mut();
        if ff::ffi::av_hwdevice_ctx_create(
            &mut device,
            ff::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
        ) < 0
            || device.is_null()
        {
            return false;
        }
        (*raw).hw_device_ctx = device;
        (*raw).get_format = Some(choose_videotoolbox_format);
        true
    }
}

/// Normalize display orientation with a clockwise quarter-turn rotation.
fn rotate_rgba(f: RgbaFrame, deg: u32) -> RgbaFrame {
    if deg == 0 || f.width == 0 || f.height == 0 {
        return f;
    }
    let (w, h) = (f.width, f.height);
    // Swap dimensions for quarter turns and use inverse pixel mapping.
    let (ow, oh) = if deg % 180 == 90 { (h, w) } else { (w, h) };
    let mut out = RgbaFrame::new(ow, oh);
    for y in 0..oh {
        for x in 0..ow {
            let (sx, sy) = match deg {
                90 => (y, h - 1 - x),          // CW90：dst(x,y) ← src(y, h−1−x)
                180 => (w - 1 - x, h - 1 - y), // Half-turn symmetry.
                270 => (w - 1 - y, x),         // CCW90（=CW270）
                _ => (x, y),
            };
            if let Some(p) = f.pixel(sx, sy) {
                out.set_pixel(x, y, p);
            }
        }
    }
    out
}

/// Read display-matrix rotation and snap to the nearest clockwise quarter turn; missing metadata
/// means zero.
fn stream_rotation_deg(s: &ff::format::stream::Stream) -> u32 {
    /// Byte size of a 3x3 i32 display matrix.
    const DISPLAY_MATRIX_BYTES: usize = 9 * size_of::<i32>();
    /// Degrees in a full turn.
    const FULL_TURN_DEG: f64 = 360.0;
    /// Quarter-turn snapping step in degrees.
    const QUARTER_TURN_DEG: f64 = 90.0;
    for sd in s.side_data() {
        if sd.kind() == ff::codec::packet::side_data::Type::DisplayMatrix
            && sd.data().len() >= DISPLAY_MATRIX_BYTES
        {
            // Convert libav's counterclockwise angle to clockwise [0,360) and snap to a quarter
            // turn.
            let ccw = unsafe { ff::ffi::av_display_rotation_get(sd.data().as_ptr() as *const i32) };
            if ccw.is_finite() {
                let cw = (-ccw).rem_euclid(FULL_TURN_DEG);
                return (((cw / QUARTER_TURN_DEG).round() as u32) % 4) * QUARTER_TURN_DEG as u32;
            }
        }
    }
    0
}

/// Convert coded width plus sample-aspect-ratio to the nearest square-pixel width. FFmpeg uses
/// zero/negative SAR to mean unspecified, which is treated as 1:1. Keeping this arithmetic in the
/// codec layer ensures sampling/model adapters only ever receive display geometry.
fn square_pixel_width(coded_width: u32, aspect_ratio: Rational) -> Result<u32> {
    if !valid_aspect_ratio(aspect_ratio) {
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
    if rounded == 0 || rounded > u64::from(u32::MAX) {
        return Err(anyhow!(
            "sample-aspect-ratio produced unsupported width {rounded}"
        ));
    }
    Ok(rounded as u32)
}

fn valid_aspect_ratio(aspect_ratio: Rational) -> bool {
    aspect_ratio.numerator() > 0 && aspect_ratio.denominator() > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::drain_video_encoder;
    use ff::{codec, encoder};
    use std::path::Path;

    /// Encode pixel positions in the red channel of a 2x3 frame for rotation assertions.
    fn tagged_2x3() -> RgbaFrame {
        let mut f = RgbaFrame::new(2, 3);
        for y in 0..3u32 {
            for x in 0..2u32 {
                f.set_pixel(x, y, [(x * 10 + y) as u8, 0, 0, 255]);
            }
        }
        f
    }

    #[test]
    fn rotate_0_is_identity() {
        let f = tagged_2x3();
        assert_eq!(rotate_rgba(f.clone(), 0), f);
    }

    #[test]
    fn rotate_90_cw_maps_corners() {
        // Clockwise rotation maps the top-left source corner to the top-right destination corner.
        let out = rotate_rgba(tagged_2x3(), 90);
        assert_eq!((out.width, out.height), (3, 2)); // Swap width and height.
        assert_eq!(out.pixel(2, 0).unwrap()[0], 0); // src(0,0)=0
        assert_eq!(out.pixel(0, 0).unwrap()[0], 2); // src(0,2)=2
        assert_eq!(out.pixel(0, 1).unwrap()[0], 12); // src(1,2)=12
    }

    #[test]
    fn rotate_180_center_symmetric() {
        let out = rotate_rgba(tagged_2x3(), 180);
        assert_eq!((out.width, out.height), (2, 3));
        assert_eq!(out.pixel(0, 0).unwrap()[0], 12); // src(1,2)
        assert_eq!(out.pixel(1, 2).unwrap()[0], 0); // src(0,0)
    }

    #[test]
    fn rotate_270_ccw_maps_corners() {
        // A clockwise 270-degree rotation maps the top-right source corner to the top-left
        // destination corner.
        let out = rotate_rgba(tagged_2x3(), 270);
        assert_eq!((out.width, out.height), (3, 2));
        assert_eq!(out.pixel(0, 0).unwrap()[0], 10); // src(1,0)=10
        assert_eq!(out.pixel(2, 1).unwrap()[0], 2); // src(0,2)=2
    }

    #[test]
    fn unspecified_or_fractional_sar_rounds_safely() {
        assert_eq!(square_pixel_width(720, Rational(0, 1)).unwrap(), 720);
        assert_eq!(square_pixel_width(720, Rational(16, 15)).unwrap(), 768);
        assert_eq!(square_pixel_width(720, Rational(8, 9)).unwrap(), 640);
    }

    #[test]
    fn streaming_source_reports_and_decodes_square_pixel_sar_geometry() {
        let temp = tempfile::tempdir().unwrap();
        let sar_path = temp.path().join("sar.mkv");
        write_sar_fixture(&sar_path, Rational(2, 1));

        let mut source = LibavVideoSource::open_with_threads(&sar_path, 2).unwrap();
        assert_eq!(source.decoder.threading().count, 2);
        assert_eq!((source.meta().width, source.meta().height), (128, 48));
        let frame = source.frame_at(0.0).unwrap();
        assert_eq!((frame.width, frame.height), (128, 48));
    }

    #[test]
    fn explicit_video_decoder_thread_budget_must_be_positive() {
        let error = match LibavVideoSource::open_with_threads(Path::new("missing.mp4"), 0) {
            Ok(_) => panic!("zero decoder threads must fail before opening the input"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("must be positive"));
    }

    fn write_sar_fixture(path: &Path, sar: Rational) {
        let mut output = format::output(path).unwrap();
        let codec = encoder::find(codec::Id::FFV1).unwrap();
        let mut builder = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()
            .unwrap();
        builder.set_width(64);
        builder.set_height(48);
        builder.set_format(format::Pixel::YUV420P);
        builder.set_time_base(Rational(1, 30));
        builder.set_frame_rate(Some(Rational(30, 1)));
        if output
            .format()
            .flags()
            .contains(format::Flags::GLOBAL_HEADER)
        {
            builder.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let mut video_encoder = builder.open().unwrap();
        let stream_index = {
            let mut stream = output.add_stream(codec).unwrap();
            stream.set_parameters(&video_encoder);
            stream.set_time_base(Rational(1, 30));
            unsafe {
                (*stream.as_mut_ptr()).sample_aspect_ratio = ff::ffi::AVRational {
                    num: sar.numerator(),
                    den: sar.denominator(),
                };
            }
            stream.index()
        };
        output.write_header().unwrap();
        let output_time_base = output.stream(stream_index).unwrap().time_base();
        for pts in 0..2 {
            let mut frame = frame::Video::new(format::Pixel::YUV420P, 64, 48);
            frame.data_mut(0).fill(128);
            frame.data_mut(1).fill(96);
            frame.data_mut(2).fill(160);
            frame.set_pts(Some(pts));
            video_encoder.send_frame(&frame).unwrap();
            drain_video_encoder(
                &mut video_encoder,
                &mut output,
                stream_index,
                Rational(1, 30),
                output_time_base,
            )
            .unwrap();
        }
        video_encoder.send_eof().unwrap();
        drain_video_encoder(
            &mut video_encoder,
            &mut output,
            stream_index,
            Rational(1, 30),
            output_time_base,
        )
        .unwrap();
        output.write_trailer().unwrap();
    }
}

/// Exact container clock and presentation timestamps for a native video source.
/// Display metadata describes the normalized RGBA frames returned by LibavVideoSource.
pub struct VideoPresentationProbe {
    pub display: SourceMeta,
    pub stream: u32,
    pub time_base: (i32, i32),
    pub duration_ticks: i64,
    pub presentation: Vec<(i64, i64)>,
}

pub fn probe_video_presentation(path: &Path) -> Result<VideoPresentationProbe> {
    ffmpeg_init();
    let mut input = format::input(path)?;
    let stream = input
        .streams()
        .best(media::Type::Video)
        .ok_or_else(|| anyhow!("no video stream"))?;
    let index = stream.index();
    let tb = stream.time_base();
    let start = stream.start_time();
    let duration = stream.duration();
    let mut presentation = Vec::new();
    for (stream, packet) in input.packets() {
        if stream.index() == index {
            let pts = packet
                .pts()
                .ok_or_else(|| anyhow!("video packet has no presentation timestamp"))?;
            presentation.push((pts, packet.duration()));
        }
    }
    presentation.sort_unstable();
    let origin = if start == ff::ffi::AV_NOPTS_VALUE {
        presentation.first().map(|p| p.0).unwrap_or(0)
    } else {
        start
    };
    let end = presentation
        .iter()
        .map(|(pts, d)| pts.saturating_add(*d))
        .max()
        .unwrap_or(origin);
    let duration_ticks = if duration > 0 {
        duration.max(end.saturating_sub(origin))
    } else {
        end.saturating_sub(origin)
    };
    if duration_ticks <= 0 || tb.numerator() <= 0 || tb.denominator() <= 0 {
        anyhow::bail!("video has no positive exact clock");
    }
    let source = LibavVideoSource::open(path)?;
    Ok(VideoPresentationProbe {
        display: source.meta().clone(),
        stream: index as u32,
        time_base: (tb.numerator(), tb.denominator()),
        duration_ticks,
        presentation,
    })
}
