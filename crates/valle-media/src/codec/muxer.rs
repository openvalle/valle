//! In-process H.264 MP4 muxing with optional AAC audio. Preserve the requested audio topology and
//! add all streams before writing the header. Use YUV420P with square pixels, set hardware bitrate
//! through the codec API, and let the encoder choose GOP picture types.

use std::{collections::VecDeque, path::Path};

use crate::frame::{AudioBuffer, RgbaFrame};
use anyhow::{Context, Result, anyhow};
use ff::{ChannelLayout, Dictionary, Rational, codec, encoder, format, frame, software};
use ffmpeg_next as ff;

use crate::encode::{DEFAULT_X264_CRF, DEFAULT_X264_PRESET};
use crate::ffi::{
    YuvMatrix, drain_audio_encoder, drain_video_encoder, ffmpeg_init, fill_plane0,
    set_sws_colorspace, tag_bt709_limited, write_mp4_header,
};
use crate::transport::shared_frame::{SharedVideoFrame, SharedVideoFramePool, VideoFrameTransport};

/// Platform hardware H.264 encoder name.
const HW_H264_ENCODER: &str = "h264_videotoolbox";

/// Probe hardware availability by opening a real minimal encoder context. Auto mode may fall back
/// to software; an explicit hardware request still fails if unavailable.
pub fn hw_h264_available() -> bool {
    ffmpeg_init();
    let Some(codec) = encoder::find_by_name(HW_H264_ENCODER) else {
        return false;
    };
    let Ok(mut video) = codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()
    else {
        return false;
    };
    video.set_width(16);
    video.set_height(16);
    video.set_format(format::Pixel::YUV420P);
    video.set_time_base(Rational(1, 30));
    video.set_frame_rate(Some(Rational(30, 1)));
    video.set_aspect_ratio(Rational(1, 1));
    video.set_bit_rate(16 * 16 * 30);
    tag_bt709_limited(&mut video);
    video.open_as(codec).is_ok()
}
/// Default hardware bitrate density in bits per pixel per frame, derived from resolution and frame
/// rate. Explicit bitrate overrides this estimate.
const HW_DEFAULT_BITS_PER_PIXEL_FRAME: f64 = 0.19;

/// AAC output sample rate in Hz.
const AUDIO_SAMPLE_RATE: u32 = 48_000;
/// AAC bitrate in bits per second.
const AUDIO_BITRATE: usize = 192_000;
/// Fallback AAC-LC frame size when the encoder reports zero.
const AAC_FRAME_SAMPLES: usize = 1024;

/// Mux video and optional audio into one MP4 output.
pub struct Muxer {
    octx: format::context::Output,
    venc: encoder::Video,
    aenc: Option<encoder::Audio>,
    sws: software::scaling::Context,
    v_sidx: usize,
    a_sidx: Option<usize>,
    v_enc_tb: Rational,
    v_ost_tb: Rational,
    a_enc_tb: Rational,
    a_ost_tb: Option<Rational>,
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    /// `true` when callers provide presentation timestamps in `v_enc_tb` rather than relying on
    /// the compositor's frame-index CFR clock.
    timestamped_video: bool,
    /// Fallback duration for the final timestamped frame, expressed in `v_enc_tb` ticks.
    nominal_frame_ticks: i64,
    /// Most recently submitted video PTS and observed inter-frame step. Timestamped outputs use
    /// these to preserve VFR spacing and derive the final frame duration without retaining PTS.
    last_video_pts: Option<i64>,
    last_video_step: Option<i64>,
    last_video_duration: Option<i64>,
    /// Input durations awaiting their encoded packet. Timestamped video disables B-frames, so
    /// packets retain presentation order; the queue remains bounded by encoder lookahead.
    timestamped_packet_durations: VecDeque<(i64, i64)>,
    /// Encoder audio frame size in samples per channel.
    audio_frame_size: usize,
    /// Number of submitted video frames. It is also the next PTS only on the CFR path.
    frame_idx: i64,
    /// Encoded audio sample count and next audio PTS per channel.
    audio_pts: i64,
    /// External audio mode disables automatic silence; the caller supplies samples in step with
    /// video.
    external_audio: bool,
    /// Interleaved sample FIFO, encoded in complete AAC frames and padded only at finalization.
    audio_fifo: Vec<f32>,
    finished: bool,
    /// Renderer/encoder frame handoff. SharedGpu owns the AVHWFramesContext used by `venc`.
    video_transport: VideoFrameTransport,
    shared_frame_pool: Option<SharedVideoFramePool>,
}

impl Muxer {
    /// Create a software H.264/AAC muxer with both streams added before the header; video
    /// dimensions must be even.
    pub fn open(
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: Option<usize>,
    ) -> Result<Self> {
        Self::open_ext(path, width, height, fps, bitrate, false, None)
    }

    /// Create a muxer with optional hardware encoding. Explicit unavailable hardware is an error.
    /// Hardware bitrate defaults to a resolution-based estimate. Software thread count defaults to
    /// auto; use a fixed count when encoded-byte reproducibility is required.
    pub fn open_ext(
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: Option<usize>,
        hw: bool,
        encode_threads: Option<usize>,
    ) -> Result<Self> {
        Self::open_ext_with_transport(
            path,
            width,
            height,
            fps,
            bitrate,
            hw,
            encode_threads,
            VideoFrameTransport::Cpu,
        )
    }

    /// Open a muxer with an explicit renderer/encoder frame transport. `SharedGpu` is currently
    /// implemented by VideoToolbox on macOS; other platforms reject it instead of silently
    /// copying through CPU memory.
    #[allow(clippy::too_many_arguments)]
    pub fn open_ext_with_transport(
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: Option<usize>,
        hw: bool,
        encode_threads: Option<usize>,
        video_transport: VideoFrameTransport,
    ) -> Result<Self> {
        Self::open_ext_rational_with_transport(
            path,
            width,
            height,
            fps.max(1),
            1,
            bitrate,
            hw,
            encode_threads,
            video_transport,
        )
    }

    /// Rational-CFR form used by the Product Compositor. The exact Timeline rate is preserved in
    /// encoder and stream time bases; 30000/1001 is never rounded to an integer delivery clock.
    #[allow(clippy::too_many_arguments)]
    pub fn open_ext_rational_with_transport(
        path: &Path,
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
        bitrate: Option<usize>,
        hw: bool,
        encode_threads: Option<usize>,
        video_transport: VideoFrameTransport,
    ) -> Result<Self> {
        Self::open_media_rational_with_transport(
            path,
            width,
            height,
            fps_num,
            fps_den,
            true,
            bitrate,
            hw,
            encode_threads,
            video_transport,
        )
    }

    /// CFR media-tool output with explicit audio topology. Existing render constructors continue
    /// to guarantee AAC; tools may pass `include_audio=false` to preserve a silent source's lack
    /// of an audio stream.
    #[allow(clippy::too_many_arguments)]
    pub fn open_media_rational_with_transport(
        path: &Path,
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
        include_audio: bool,
        bitrate: Option<usize>,
        hw: bool,
        encode_threads: Option<usize>,
        video_transport: VideoFrameTransport,
    ) -> Result<Self> {
        let v_enc_tb = Rational(fps_den as i32, fps_num as i32);
        Self::open_configured(
            path,
            width,
            height,
            v_enc_tb,
            fps_num,
            fps_den,
            false,
            1,
            include_audio,
            bitrate,
            hw,
            encode_threads,
            video_transport,
        )
    }

    /// Open a VFR-capable muxer whose video PTS are supplied explicitly in `time_base` ticks.
    ///
    /// `nominal_fps_*` is only an encoder hint and the fallback duration of a one-frame stream;
    /// it never replaces, rounds, duplicates or drops caller timestamps. Call
    /// [`Self::encode_video_at_pts`] for every frame.
    #[allow(clippy::too_many_arguments)]
    pub fn open_timestamped_with_transport(
        path: &Path,
        width: u32,
        height: u32,
        time_base: Rational,
        nominal_fps_num: u32,
        nominal_fps_den: u32,
        bitrate: Option<usize>,
        hw: bool,
        encode_threads: Option<usize>,
        video_transport: VideoFrameTransport,
    ) -> Result<Self> {
        Self::open_timestamped_media_with_transport(
            path,
            width,
            height,
            time_base,
            nominal_fps_num,
            nominal_fps_den,
            true,
            bitrate,
            hw,
            encode_threads,
            video_transport,
        )
    }

    /// Timestamped media-tool output that preserves source audio topology. `include_audio=false`
    /// writes a genuine video-only MP4 rather than inventing a silent track.
    #[allow(clippy::too_many_arguments)]
    pub fn open_timestamped_media_with_transport(
        path: &Path,
        width: u32,
        height: u32,
        time_base: Rational,
        nominal_fps_num: u32,
        nominal_fps_den: u32,
        include_audio: bool,
        bitrate: Option<usize>,
        hw: bool,
        encode_threads: Option<usize>,
        video_transport: VideoFrameTransport,
    ) -> Result<Self> {
        if time_base.numerator() <= 0 || time_base.denominator() <= 0 {
            return Err(anyhow!(
                "timestamped muxer time base must be positive (got {}/{})",
                time_base.numerator(),
                time_base.denominator()
            ));
        }
        if nominal_fps_num == 0
            || nominal_fps_den == 0
            || nominal_fps_num > i32::MAX as u32
            || nominal_fps_den > i32::MAX as u32
        {
            return Err(anyhow!(
                "timestamped muxer nominal FPS must be a positive i32 rational (got {nominal_fps_num}/{nominal_fps_den})"
            ));
        }
        let ticks = (i128::from(time_base.denominator()) * i128::from(nominal_fps_den)
            + i128::from(time_base.numerator()) * i128::from(nominal_fps_num) / 2)
            / (i128::from(time_base.numerator()) * i128::from(nominal_fps_num));
        let nominal_frame_ticks = i64::try_from(ticks.max(1))
            .context("timestamped muxer nominal frame duration exceeds i64")?;
        Self::open_configured(
            path,
            width,
            height,
            time_base,
            nominal_fps_num,
            nominal_fps_den,
            true,
            nominal_frame_ticks,
            include_audio,
            bitrate,
            hw,
            encode_threads,
            video_transport,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn open_configured(
        path: &Path,
        width: u32,
        height: u32,
        v_enc_tb: Rational,
        fps_num: u32,
        fps_den: u32,
        timestamped_video: bool,
        nominal_frame_ticks: i64,
        include_audio: bool,
        bitrate: Option<usize>,
        hw: bool,
        encode_threads: Option<usize>,
        video_transport: VideoFrameTransport,
    ) -> Result<Self> {
        ffmpeg_init();
        if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(anyhow!(
                "muxer dims must be positive even numbers (got {width}x{height})"
            ));
        }
        if fps_num == 0 || fps_den == 0 || fps_num > i32::MAX as u32 || fps_den > i32::MAX as u32 {
            return Err(anyhow!(
                "muxer fps must be a positive i32 rational (got {fps_num}/{fps_den})"
            ));
        }
        let frames_per_second = f64::from(fps_num) / f64::from(fps_den);
        let a_enc_tb = Rational(1, AUDIO_SAMPLE_RATE as i32);

        let mut octx = format::output(&path).with_context(|| format!("open {}", path.display()))?;
        let global_header = octx.format().flags().contains(format::Flags::GLOBAL_HEADER);

        // H.264 encoder selection.
        let vcodec = if hw {
            encoder::find_by_name(HW_H264_ENCODER).ok_or_else(|| {
                anyhow!(
                    "HW encoder '{HW_H264_ENCODER}' unavailable on this platform; software fallback is disabled"
                )
            })?
        } else {
            encoder::find(codec::Id::H264).ok_or_else(|| anyhow!("no H.264 encoder available"))?
        };
        if video_transport == VideoFrameTransport::SharedGpu && !hw {
            return Err(anyhow!(
                "shared GPU frames require a platform hardware encoder"
            ));
        }
        let shared_frame_pool: Option<SharedVideoFramePool> =
            if video_transport == VideoFrameTransport::SharedGpu {
                #[cfg(target_os = "macos")]
                {
                    Some(SharedVideoFramePool::videotoolbox_bgra(width, height)?)
                }
                #[cfg(not(target_os = "macos"))]
                {
                    return Err(anyhow!(
                        "shared GPU video frames are not implemented on this platform"
                    ));
                }
            } else {
                None
            };

        let mut vb = codec::context::Context::new_with_codec(vcodec)
            .encoder()
            .video()?;
        vb.set_width(width);
        vb.set_height(height);
        vb.set_format(if shared_frame_pool.is_some() {
            format::Pixel::VIDEOTOOLBOX
        } else {
            format::Pixel::YUV420P
        });
        vb.set_time_base(v_enc_tb);
        vb.set_frame_rate(Some(Rational(fps_num as i32, fps_den as i32)));
        // Caller-supplied VFR PTS and encoder B-frame reordering can otherwise produce DTS that
        // overtakes an earlier presentation timestamp. Keep timestamped tool outputs in display
        // order; the compositor's established CFR path retains its existing GOP behavior.
        if timestamped_video {
            vb.set_max_b_frames(0);
        }
        vb.set_aspect_ratio(Rational(1, 1));
        tag_bt709_limited(&mut vb); // Tag output as BT.709 limited range.
        if global_header {
            vb.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        if let Some(pool) = &shared_frame_pool {
            // SAFETY: the cloned AVBufferRef is transferred to AVCodecContext before open; the
            // context owns and releases it. `pool` independently keeps the same hardware pool
            // alive so raster workers can acquire frames.
            unsafe {
                (*vb.as_mut_ptr()).hw_frames_ctx = pool.encoder_frames_ref()?;
            }
        }
        // Hardware encoding always needs a bitrate; software encoding may use CRF.
        let effective_bitrate = if hw {
            Some(bitrate.unwrap_or(
                (width as f64 * height as f64 * frames_per_second * HW_DEFAULT_BITS_PER_PIXEL_FRAME)
                    as usize,
            ))
        } else {
            bitrate
        };
        let mut vopts = Dictionary::new();
        if let Some(b) = effective_bitrate {
            vb.set_bit_rate(b); // Use the bitrate setter required by the hardware encoder.
        } else {
            vopts.set("preset", DEFAULT_X264_PRESET);
            vopts.set("crf", DEFAULT_X264_CRF);
        }
        // Apply an explicit nonzero software thread count; otherwise keep automatic selection.
        if !hw && let Some(t) = encode_threads.filter(|&t| t > 0) {
            vopts.set("threads", &t.to_string());
        }
        let venc = vb.open_with(vopts)?;

        let audio = if include_audio {
            let acodec =
                encoder::find(codec::Id::AAC).ok_or_else(|| anyhow!("no AAC encoder available"))?;
            let mut ab = codec::context::Context::new_with_codec(acodec)
                .encoder()
                .audio()?;
            ab.set_rate(AUDIO_SAMPLE_RATE as i32);
            ab.set_channel_layout(ChannelLayout::STEREO);
            ab.set_format(format::Sample::F32(format::sample::Type::Planar));
            ab.set_bit_rate(AUDIO_BITRATE);
            ab.set_time_base(a_enc_tb);
            if global_header {
                ab.set_flags(codec::Flags::GLOBAL_HEADER);
            }
            let aenc = ab.open_as(acodec)?;
            let frame_size = if aenc.frame_size() > 0 {
                aenc.frame_size() as usize
            } else {
                AAC_FRAME_SAMPLES
            };
            Some((acodec, aenc, frame_size))
        } else {
            None
        };

        // Add all streams before writing the header.
        let v_sidx = {
            let mut vost = octx.add_stream(vcodec)?;
            let v_sidx = vost.index();
            vost.set_parameters(&venc);
            vost.set_time_base(v_enc_tb);
            v_sidx
        };

        let a_sidx = if let Some((acodec, aenc, _)) = audio.as_ref() {
            let mut aost = octx.add_stream(*acodec)?;
            let a_sidx = aost.index();
            aost.set_parameters(aenc);
            aost.set_time_base(a_enc_tb);
            Some(a_sidx)
        } else {
            None
        };

        write_mp4_header(&mut octx)?;
        let v_ost_tb = octx.stream(v_sidx).unwrap().time_base();
        let a_ost_tb = a_sidx.map(|index| octx.stream(index).unwrap().time_base());
        let (aenc, audio_frame_size) = audio
            .map_or((None, AAC_FRAME_SAMPLES), |(_, aenc, size)| {
                (Some(aenc), size)
            });

        let mut sws = software::scaling::Context::get(
            format::Pixel::RGBA,
            width,
            height,
            format::Pixel::YUV420P,
            width,
            height,
            software::scaling::Flags::BILINEAR,
        )?;
        // Match RGB-to-YUV conversion to the BT.709 limited-range output tags.
        set_sws_colorspace(&mut sws, YuvMatrix::Bt709, false, false);

        Ok(Muxer {
            octx,
            venc,
            aenc,
            sws,
            v_sidx,
            a_sidx,
            v_enc_tb,
            v_ost_tb,
            a_enc_tb,
            a_ost_tb,
            width,
            height,
            fps_num,
            fps_den,
            timestamped_video,
            nominal_frame_ticks,
            last_video_pts: None,
            last_video_step: None,
            last_video_duration: None,
            timestamped_packet_durations: VecDeque::new(),
            audio_frame_size,
            frame_idx: 0,
            audio_pts: 0,
            external_audio: false,
            audio_fifo: Vec::new(),
            finished: false,
            video_transport,
            shared_frame_pool,
        })
    }

    /// Clone the encoder-owned frame pool for a GPU raster worker.
    pub fn shared_frame_pool(&self) -> Option<SharedVideoFramePool> {
        self.shared_frame_pool.clone()
    }

    /// Declare externally supplied audio before the first video frame to disable automatic silence.
    pub fn expect_external_audio(&mut self) {
        self.external_audio = true;
    }

    /// Required interleaved audio sample rate and channel count.
    pub fn audio_format(&self) -> (u32, u16) {
        /// Stereo channel count.
        const STEREO_CH: u16 = 2;
        (AUDIO_SAMPLE_RATE, STEREO_CH)
    }

    /// Accept interleaved audio matching the muxer's output format. Buffer full AAC frames,
    /// interleave by DTS, and pad the final partial frame at finish.
    pub fn encode_audio(&mut self, buf: &AudioBuffer) -> Result<()> {
        if self.aenc.is_none() {
            return Err(anyhow!("muxer was opened without an audio stream"));
        }
        let (rate, ch) = self.audio_format();
        if buf.sample_rate != rate || buf.channels != ch {
            return Err(anyhow!(
                "audio buffer {}Hz/{}ch != muxer {}Hz/{}ch; resampling belongs to the source",
                buf.sample_rate,
                buf.channels,
                rate,
                ch
            ));
        }
        self.audio_fifo.extend_from_slice(&buf.samples);
        self.drain_audio_fifo(false)
    }

    /// Encode complete audio frames from the FIFO. Pad a partial frame only at finalization to
    /// avoid inserting silence midstream.
    fn drain_audio_fifo(&mut self, pad_tail: bool) -> Result<()> {
        if self.aenc.is_none() {
            if self.audio_fifo.is_empty() {
                return Ok(());
            }
            return Err(anyhow!("video-only muxer received buffered audio"));
        }
        let audio_stream = self.a_sidx.context("AAC stream index is missing")?;
        let audio_stream_time_base = self.a_ost_tb.context("AAC stream time base is missing")?;
        let ch = usize::from(self.audio_format().1);
        let frame_len = self.audio_frame_size * ch;
        while self.audio_fifo.len() >= frame_len || (pad_tail && !self.audio_fifo.is_empty()) {
            let take = self.audio_fifo.len().min(frame_len);
            let n = self.audio_frame_size;
            let mut af = frame::Audio::new(
                format::Sample::F32(format::sample::Type::Planar),
                n,
                ChannelLayout::STEREO,
            );
            af.set_rate(AUDIO_SAMPLE_RATE);
            // Convert interleaved samples to planar FLTP; leave final-frame padding silent.
            for p in 0..af.planes() {
                let plane = af.plane_mut::<f32>(p);
                plane.fill(0.0);
                for (i, s) in plane.iter_mut().enumerate().take(n) {
                    let idx = i * ch + p;
                    if idx < take {
                        *s = self.audio_fifo[idx];
                    }
                }
            }
            self.audio_fifo.drain(..take);
            af.set_pts(Some(self.audio_pts));
            let audio_encoder = self.aenc.as_mut().context("AAC encoder is missing")?;
            audio_encoder.send_frame(&af)?;
            drain_audio_encoder(
                audio_encoder,
                &mut self.octx,
                audio_stream,
                self.a_enc_tb,
                audio_stream_time_base,
            )?;
            self.audio_pts += n as i64;
            if pad_tail && self.audio_fifo.is_empty() {
                break;
            }
        }
        Ok(())
    }

    /// Encode one constant-frame-rate video frame and advance automatic silence when enabled.
    pub fn encode_video(&mut self, frame: &RgbaFrame) -> Result<()> {
        if self.timestamped_video {
            return Err(anyhow!(
                "timestamped muxer requires encode_video_at_pts instead of frame-index PTS"
            ));
        }
        let yuv = self.convert_rgba(frame)?;
        self.submit_yuv(yuv)?; // fallback path is not pooled; returned frame is dropped
        Ok(())
    }

    /// Encode one RGBA frame at an explicit, zero-based presentation timestamp.
    ///
    /// PTS are expressed in the time base passed to [`Self::open_timestamped_with_transport`]
    /// and must be strictly increasing. The first frame must start at zero. This preserves VFR
    /// spacing while keeping timestamp ownership in the media-tool host.
    pub fn encode_video_at_pts(&mut self, frame: &RgbaFrame, pts: i64) -> Result<()> {
        self.encode_video_at_pts_with_duration(frame, pts, None)
    }

    /// Timestamped encode with an optional exact frame duration in the same time base. Supplying
    /// decoder duration preserves the final VFR interval instead of inferring it from the prior
    /// presentation step.
    pub fn encode_video_at_pts_with_duration(
        &mut self,
        frame: &RgbaFrame,
        pts: i64,
        duration_ticks: Option<i64>,
    ) -> Result<()> {
        if !self.timestamped_video {
            return Err(anyhow!(
                "CFR muxer requires encode_video instead of caller-supplied PTS"
            ));
        }
        let yuv = self.convert_rgba(frame)?;
        self.submit_yuv_at_pts(yuv, pts, duration_ticks)?;
        Ok(())
    }

    fn convert_rgba(&mut self, frame: &RgbaFrame) -> Result<frame::Video> {
        if self.video_transport == VideoFrameTransport::SharedGpu {
            return Err(anyhow!(
                "shared-GPU muxer requires SharedVideoFrame input; CPU RGBA fallback is unavailable"
            ));
        }
        if frame.width != self.width || frame.height != self.height {
            return Err(anyhow!(
                "frame {}x{} != muxer {}x{}",
                frame.width,
                frame.height,
                self.width,
                self.height
            ));
        }
        let row = self.width as usize * 4;
        crate::perf::time_enc_convert(|| -> Result<frame::Video> {
            // Allocate a fresh frame in the fallback conversion path; worker conversion supports
            // recycling separately.
            let mut rgba_f = frame::Video::new(format::Pixel::RGBA, self.width, self.height);
            fill_plane0(&mut rgba_f, &frame.data, row, self.height);
            let mut yuv = frame::Video::new(format::Pixel::YUV420P, self.width, self.height);
            self.sws.run(&rgba_f, &mut yuv)?;
            Ok(yuv)
        })
    }

    /// Encode worker-converted YUV produced with matching RgbaToYuv parameters. Return the buffer
    /// for recycling only when the encoder has released all plane references; asynchronous hardware
    /// may retain them.
    pub fn encode_yuv(&mut self, yuv: YuvFrame) -> Result<Option<YuvFrame>> {
        if self.video_transport == VideoFrameTransport::SharedGpu {
            return Err(anyhow!(
                "shared-GPU muxer requires SharedVideoFrame input; CPU YUV fallback is unavailable"
            ));
        }
        if yuv.width != self.width || yuv.height != self.height {
            return Err(anyhow!(
                "yuv frame {}x{} != muxer {}x{}",
                yuv.width,
                yuv.height,
                self.width,
                self.height
            ));
        }
        let YuvFrame {
            inner,
            width,
            height,
        } = yuv;
        let inner = self.submit_yuv(inner)?;
        let back = YuvFrame {
            inner,
            width,
            height,
        };
        Ok(back.is_writable().then_some(back))
    }

    /// Submit an encoder-owned platform surface rendered in place by the GPU.
    pub fn encode_shared(&mut self, frame: SharedVideoFrame) -> Result<()> {
        if self.video_transport != VideoFrameTransport::SharedGpu {
            return Err(anyhow!("muxer was not opened for shared GPU frames"));
        }
        let (width, height) = frame.dimensions();
        if width != self.width || height != self.height {
            return Err(anyhow!(
                "shared frame {width}x{height} != muxer {}x{}",
                self.width,
                self.height
            ));
        }
        let mut inner = frame.into_inner();
        unsafe {
            let raw = inner.as_mut_ptr();
            (*raw).color_primaries = ff::ffi::AVColorPrimaries::AVCOL_PRI_BT709;
            (*raw).color_trc = ff::ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
            (*raw).colorspace = ff::ffi::AVColorSpace::AVCOL_SPC_BT709;
            (*raw).color_range = ff::ffi::AVColorRange::AVCOL_RANGE_MPEG;
        }
        self.submit_yuv(inner)?;
        Ok(())
    }

    /// Assign frame PTS, submit, drain packets, and advance automatic silence. Return the frame for
    /// buffer-reuse checks.
    fn submit_yuv(&mut self, yuv: frame::Video) -> Result<frame::Video> {
        if self.timestamped_video {
            return Err(anyhow!(
                "timestamped muxer requires a caller-supplied video PTS"
            ));
        }
        self.submit_yuv_at_pts(yuv, self.frame_idx, Some(1))
    }

    fn submit_yuv_at_pts(
        &mut self,
        mut yuv: frame::Video,
        pts: i64,
        duration_ticks: Option<i64>,
    ) -> Result<frame::Video> {
        if let Some(previous) = self.last_video_pts {
            if pts <= previous {
                return Err(anyhow!(
                    "video PTS must be strictly increasing: {pts} <= {previous}"
                ));
            }
        } else if pts != 0 {
            return Err(anyhow!(
                "first video PTS must be zero after output-range rebasing, got {pts}"
            ));
        }
        let observed_step = self.last_video_pts.map(|previous| pts - previous);
        if let Some(duration) = duration_ticks
            && duration <= 0
        {
            return Err(anyhow!(
                "video frame duration must be positive, got {duration}"
            ));
        }
        yuv.set_pts(Some(pts)); // pict_type not set -> encoder chooses GOP
        if self.timestamped_video {
            if let Some((previous_pts, previous_duration)) =
                self.timestamped_packet_durations.back_mut()
                && *previous_duration == 0
            {
                *previous_duration = pts - *previous_pts;
            }
            self.timestamped_packet_durations
                .push_back((pts, duration_ticks.unwrap_or(0)));
        }
        crate::perf::time_enc_submit(|| -> Result<()> {
            self.venc.send_frame(&yuv)?;
            self.drain_video_packets()
        })?;
        self.frame_idx += 1;
        self.last_video_pts = Some(pts);
        if let Some(step) = observed_step {
            self.last_video_step = Some(step);
        }
        self.last_video_duration = duration_ticks;

        if !self.external_audio {
            // Advance silence alongside video to keep the interleave queue bounded; external audio
            // is supplied by the caller.
            let video_time_s = self.video_duration_seconds()?;
            self.pump_silence_until(video_time_s)?;
        }
        Ok(yuv)
    }

    fn drain_video_packets(&mut self) -> Result<()> {
        if !self.timestamped_video {
            return drain_video_encoder(
                &mut self.venc,
                &mut self.octx,
                self.v_sidx,
                self.v_enc_tb,
                self.v_ost_tb,
            );
        }
        let mut packet = ff::Packet::empty();
        while self.venc.receive_packet(&mut packet).is_ok() {
            let pts = packet
                .pts()
                .context("timestamped video encoder returned a packet without PTS")?;
            let (expected_pts, declared_duration) =
                self.timestamped_packet_durations
                    .pop_front()
                    .context("timestamped video encoder returned an unexpected packet")?;
            if pts != expected_pts {
                return Err(anyhow!(
                    "timestamped encoder packet PTS drifted: {pts} != {expected_pts}"
                ));
            }
            let duration = if declared_duration > 0 {
                declared_duration
            } else {
                self.last_video_step
                    .unwrap_or(self.nominal_frame_ticks)
                    .max(1)
            };
            packet.set_duration(duration);
            packet.set_stream(self.v_sidx);
            packet.rescale_ts(self.v_enc_tb, self.v_ost_tb);
            packet.write_interleaved(&mut self.octx)?;
        }
        Ok(())
    }

    /// Current complete-video duration. Timestamped video prefers the declared final-frame
    /// duration, then the last observed PTS step, then the nominal one-frame fallback.
    pub fn video_duration_seconds(&self) -> Result<f64> {
        if self.timestamped_video {
            let Some(last_pts) = self.last_video_pts else {
                return Ok(0.0);
            };
            let final_step = self
                .last_video_duration
                .or(self.last_video_step)
                .unwrap_or(self.nominal_frame_ticks)
                .max(1);
            let end_ticks = last_pts
                .checked_add(final_step)
                .context("timestamped video duration overflowed")?;
            Ok(end_ticks as f64 * f64::from(self.v_enc_tb))
        } else {
            Ok(self.frame_idx as f64 * f64::from(self.fps_den) / f64::from(self.fps_num))
        }
    }

    /// Encode silent AAC frames until their sample count covers the requested time.
    pub(crate) fn pump_silence_until(&mut self, target_s: f64) -> Result<()> {
        if self.aenc.is_none() {
            return Ok(());
        }
        let audio_stream = self.a_sidx.context("AAC stream index is missing")?;
        let audio_stream_time_base = self.a_ost_tb.context("AAC stream time base is missing")?;
        let target_samples = (target_s * AUDIO_SAMPLE_RATE as f64).round() as i64;
        while self.audio_pts < target_samples {
            let n = self.audio_frame_size;
            let mut af = frame::Audio::new(
                format::Sample::F32(format::sample::Type::Planar),
                n,
                ChannelLayout::STEREO,
            );
            af.set_rate(AUDIO_SAMPLE_RATE);
            for p in 0..af.planes() {
                af.plane_mut::<f32>(p).fill(0.0); // Zero-filled samples are silence.
            }
            af.set_pts(Some(self.audio_pts));
            let audio_encoder = self.aenc.as_mut().context("AAC encoder is missing")?;
            audio_encoder.send_frame(&af)?;
            drain_audio_encoder(
                audio_encoder,
                &mut self.octx,
                audio_stream,
                self.a_enc_tb,
                audio_stream_time_base,
            )?;
            self.audio_pts += n as i64;
        }
        Ok(())
    }

    /// Flush audio tails and both encoders, complete audio timing, and write the trailer. Safe to
    /// call repeatedly.
    pub fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.drain_audio_fifo(true)?; // Pad and encode the final external-audio fragment.
        let total_video_s = self.video_duration_seconds()?;
        self.pump_silence_until(total_video_s)?;

        if self.timestamped_video
            && let Some((_, duration)) = self.timestamped_packet_durations.back_mut()
            && *duration == 0
        {
            *duration = self
                .last_video_duration
                .or(self.last_video_step)
                .unwrap_or(self.nominal_frame_ticks)
                .max(1);
        }
        self.venc.send_eof()?;
        self.drain_video_packets()?;
        if !self.timestamped_packet_durations.is_empty() {
            return Err(anyhow!(
                "timestamped video encoder did not emit {} submitted frames",
                self.timestamped_packet_durations.len()
            ));
        }
        if let Some(audio_encoder) = self.aenc.as_mut() {
            let audio_stream = self.a_sidx.context("AAC stream index is missing")?;
            let audio_stream_time_base =
                self.a_ost_tb.context("AAC stream time base is missing")?;
            audio_encoder.send_eof()?;
            drain_audio_encoder(
                audio_encoder,
                &mut self.octx,
                audio_stream,
                self.a_enc_tb,
                audio_stream_time_base,
            )?;
        }
        self.octx.write_trailer()?;
        self.finished = true;
        Ok(())
    }
}

/// Worker-converted YUV420P frame passed to encode_yuv.
pub struct YuvFrame {
    inner: frame::Video,
    width: u32,
    height: u32,
}

// SAFETY: The AVFrame is exclusively owned and moved between worker and encoder threads without
// shared mutable aliases.
unsafe impl Send for YuvFrame {}

impl YuvFrame {
    /// A writable frame has exclusive plane buffers and can be recycled. Asynchronous encoders may
    /// retain references, requiring a fresh allocation instead.
    pub fn is_writable(&self) -> bool {
        // SAFETY: This only queries buffer reference counts; the mutable pointer matches the C
        // signature without mutating the frame.
        unsafe { ff::ffi::av_frame_is_writable(self.inner.as_ptr() as *mut _) != 0 }
    }
}

/// Per-worker RGBA-to-YUV420P converter using the muxer's exact dimensions and BT.709 limited-range
/// settings. Reuse local RGBA staging and recycle YUV buffers only when dimensions match and
/// ownership is exclusive.
pub struct RgbaToYuv {
    sws: software::scaling::Context,
    staging: frame::Video,
    width: u32,
    height: u32,
}

impl RgbaToYuv {
    /// Create a converter with output dimensions matching the destination muxer.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        ffmpeg_init();
        let mut sws = software::scaling::Context::get(
            format::Pixel::RGBA,
            width,
            height,
            format::Pixel::YUV420P,
            width,
            height,
            software::scaling::Flags::BILINEAR,
        )?;
        // Match the muxer's BT.709 limited-range conversion.
        set_sws_colorspace(&mut sws, YuvMatrix::Bt709, false, false);
        Ok(RgbaToYuv {
            sws,
            staging: frame::Video::new(format::Pixel::RGBA, width, height),
            width,
            height,
        })
    }

    /// Convert one frame, reusing a compatible exclusively owned output buffer when available.
    /// Full-plane writes preserve identical output for fresh and recycled buffers.
    pub fn convert(&mut self, frame: &RgbaFrame, recycled: Option<YuvFrame>) -> Result<YuvFrame> {
        if frame.width != self.width || frame.height != self.height {
            return Err(anyhow!(
                "frame {}x{} != converter {}x{}",
                frame.width,
                frame.height,
                self.width,
                self.height
            ));
        }
        crate::perf::time_enc_convert(|| {
            let row = self.width as usize * 4;
            fill_plane0(&mut self.staging, &frame.data, row, self.height);
            let mut yuv = match recycled {
                Some(f) if f.width == self.width && f.height == self.height && f.is_writable() => {
                    f.inner
                }
                _ => frame::Video::new(format::Pixel::YUV420P, self.width, self.height),
            };
            self.sws.run(&self.staging, &mut yuv)?;
            Ok(YuvFrame {
                inner: yuv,
                width: self.width,
                height: self.height,
            })
        })
    }
}

/// Audio-only AAC muxer for MP4/M4A output without a video track. Use the same sample format and
/// FIFO semantics as the audiovisual muxer, padding only the final partial frame.
pub struct AudioMuxer {
    octx: format::context::Output,
    aenc: encoder::Audio,
    a_sidx: usize,
    a_enc_tb: Rational,
    a_ost_tb: Rational,
    audio_frame_size: usize,
    audio_pts: i64,
    audio_fifo: Vec<f32>,
    finished: bool,
}

impl AudioMuxer {
    /// Create an audio-only muxer and add its AAC stream before writing the header.
    pub fn open(path: &Path) -> Result<Self> {
        ffmpeg_init();
        let a_enc_tb = Rational(1, AUDIO_SAMPLE_RATE as i32);
        let mut octx = format::output(&path).with_context(|| format!("open {}", path.display()))?;
        let global_header = octx.format().flags().contains(format::Flags::GLOBAL_HEADER);

        let acodec =
            encoder::find(codec::Id::AAC).ok_or_else(|| anyhow!("no AAC encoder available"))?;
        let mut ab = codec::context::Context::new_with_codec(acodec)
            .encoder()
            .audio()?;
        ab.set_rate(AUDIO_SAMPLE_RATE as i32);
        ab.set_channel_layout(ChannelLayout::STEREO);
        ab.set_format(format::Sample::F32(format::sample::Type::Planar));
        ab.set_bit_rate(AUDIO_BITRATE);
        ab.set_time_base(a_enc_tb);
        if global_header {
            ab.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let aenc = ab.open_as(acodec)?;
        let audio_frame_size = if aenc.frame_size() > 0 {
            aenc.frame_size() as usize
        } else {
            AAC_FRAME_SAMPLES
        };

        let a_sidx = {
            let mut aost = octx.add_stream(acodec)?;
            let a_sidx = aost.index();
            aost.set_parameters(&aenc);
            aost.set_time_base(a_enc_tb);
            a_sidx
        };

        octx.write_header()?;
        let a_ost_tb = octx.stream(a_sidx).unwrap().time_base();

        Ok(AudioMuxer {
            octx,
            aenc,
            a_sidx,
            a_enc_tb,
            a_ost_tb,
            audio_frame_size,
            audio_pts: 0,
            audio_fifo: Vec::new(),
            finished: false,
        })
    }

    /// Required interleaved audio format: 48 kHz stereo.
    pub fn audio_format(&self) -> (u32, u16) {
        /// Stereo channel count.
        const STEREO_CH: u16 = 2;
        (AUDIO_SAMPLE_RATE, STEREO_CH)
    }

    /// Buffer interleaved samples into complete AAC frames; pad the final fragment at finish.
    pub fn encode_audio(&mut self, buf: &AudioBuffer) -> Result<()> {
        let (rate, ch) = self.audio_format();
        if buf.sample_rate != rate || buf.channels != ch {
            return Err(anyhow!(
                "audio buffer {}Hz/{}ch != muxer {}Hz/{}ch; resampling belongs to the source",
                buf.sample_rate,
                buf.channels,
                rate,
                ch
            ));
        }
        self.audio_fifo.extend_from_slice(&buf.samples);
        self.drain_fifo(false)
    }

    /// Apply the same FIFO encoding semantics as the audiovisual muxer.
    fn drain_fifo(&mut self, pad_tail: bool) -> Result<()> {
        let ch = usize::from(self.audio_format().1);
        let frame_len = self.audio_frame_size * ch;
        while self.audio_fifo.len() >= frame_len || (pad_tail && !self.audio_fifo.is_empty()) {
            let take = self.audio_fifo.len().min(frame_len);
            let n = self.audio_frame_size;
            let mut af = frame::Audio::new(
                format::Sample::F32(format::sample::Type::Planar),
                n,
                ChannelLayout::STEREO,
            );
            af.set_rate(AUDIO_SAMPLE_RATE);
            for p in 0..af.planes() {
                let plane = af.plane_mut::<f32>(p);
                plane.fill(0.0);
                for (i, s) in plane.iter_mut().enumerate().take(n) {
                    let idx = i * ch + p;
                    if idx < take {
                        *s = self.audio_fifo[idx];
                    }
                }
            }
            self.audio_fifo.drain(..take);
            af.set_pts(Some(self.audio_pts));
            self.aenc.send_frame(&af)?;
            drain_audio_encoder(
                &mut self.aenc,
                &mut self.octx,
                self.a_sidx,
                self.a_enc_tb,
                self.a_ost_tb,
            )?;
            self.audio_pts += n as i64;
            if pad_tail && self.audio_fifo.is_empty() {
                break;
            }
        }
        Ok(())
    }

    /// Flush the encoder and write the trailer; safe to call repeatedly.
    pub fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.drain_fifo(true)?;
        self.aenc.send_eof()?;
        drain_audio_encoder(
            &mut self.aenc,
            &mut self.octx,
            self.a_sidx,
            self.a_enc_tb,
            self.a_ost_tb,
        )?;
        self.octx.write_trailer()?;
        self.finished = true;
        Ok(())
    }
}
