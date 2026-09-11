//! Version-independent muxer interface. Native state stays in the selected adapter.
use crate::codec::backend::{self, Inner, Version, backend_type};
use crate::transport::{SharedVideoFrame, SharedVideoFramePool, VideoFrameTransport};
use crate::{
    codec::TimeBase,
    frame::{AudioBuffer, RgbaFrame},
};
use anyhow::Result;
use std::path::Path;
backend_type!(Muxer, muxer);
impl Muxer {
    pub fn open(
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: Option<usize>,
    ) -> Result<Self> {
        match backend::version()? {
            Version::V7 => {
                backend::v7::muxer::Muxer::open(path, width, height, fps, bitrate).map(Into::into)
            }
            Version::V8 => {
                backend::v8::muxer::Muxer::open(path, width, height, fps, bitrate).map(Into::into)
            }
            Version::V9 => {
                backend::v9::muxer::Muxer::open(path, width, height, fps, bitrate).map(Into::into)
            }
        }
    }
    pub fn open_ext(
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: Option<usize>,
        hw: bool,
        encode_threads: Option<usize>,
    ) -> Result<Self> {
        match backend::version()? {
            Version::V7 => backend::v7::muxer::Muxer::open_ext(
                path,
                width,
                height,
                fps,
                bitrate,
                hw,
                encode_threads,
            )
            .map(Into::into),
            Version::V8 => backend::v8::muxer::Muxer::open_ext(
                path,
                width,
                height,
                fps,
                bitrate,
                hw,
                encode_threads,
            )
            .map(Into::into),
            Version::V9 => backend::v9::muxer::Muxer::open_ext(
                path,
                width,
                height,
                fps,
                bitrate,
                hw,
                encode_threads,
            )
            .map(Into::into),
        }
    }
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
        match backend::version()? {
            Version::V7 => backend::v7::muxer::Muxer::open_ext_with_transport(
                path,
                width,
                height,
                fps,
                bitrate,
                hw,
                encode_threads,
                video_transport,
            )
            .map(Into::into),
            Version::V8 => backend::v8::muxer::Muxer::open_ext_with_transport(
                path,
                width,
                height,
                fps,
                bitrate,
                hw,
                encode_threads,
                video_transport,
            )
            .map(Into::into),
            Version::V9 => backend::v9::muxer::Muxer::open_ext_with_transport(
                path,
                width,
                height,
                fps,
                bitrate,
                hw,
                encode_threads,
                video_transport,
            )
            .map(Into::into),
        }
    }
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
        match backend::version()? {
            Version::V7 => backend::v7::muxer::Muxer::open_ext_rational_with_transport(
                path,
                width,
                height,
                fps_num,
                fps_den,
                bitrate,
                hw,
                encode_threads,
                video_transport,
            )
            .map(Into::into),
            Version::V8 => backend::v8::muxer::Muxer::open_ext_rational_with_transport(
                path,
                width,
                height,
                fps_num,
                fps_den,
                bitrate,
                hw,
                encode_threads,
                video_transport,
            )
            .map(Into::into),
            Version::V9 => backend::v9::muxer::Muxer::open_ext_rational_with_transport(
                path,
                width,
                height,
                fps_num,
                fps_den,
                bitrate,
                hw,
                encode_threads,
                video_transport,
            )
            .map(Into::into),
        }
    }
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
        match backend::version()? {
            Version::V7 => backend::v7::muxer::Muxer::open_media_rational_with_transport(
                path,
                width,
                height,
                fps_num,
                fps_den,
                include_audio,
                bitrate,
                hw,
                encode_threads,
                video_transport,
            )
            .map(Into::into),
            Version::V8 => backend::v8::muxer::Muxer::open_media_rational_with_transport(
                path,
                width,
                height,
                fps_num,
                fps_den,
                include_audio,
                bitrate,
                hw,
                encode_threads,
                video_transport,
            )
            .map(Into::into),
            Version::V9 => backend::v9::muxer::Muxer::open_media_rational_with_transport(
                path,
                width,
                height,
                fps_num,
                fps_den,
                include_audio,
                bitrate,
                hw,
                encode_threads,
                video_transport,
            )
            .map(Into::into),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn open_timestamped_with_transport(
        path: &Path,
        width: u32,
        height: u32,
        time_base: TimeBase,
        nominal_fps_num: u32,
        nominal_fps_den: u32,
        bitrate: Option<usize>,
        hw: bool,
        encode_threads: Option<usize>,
        video_transport: VideoFrameTransport,
    ) -> Result<Self> {
        match backend::version()? {
            Version::V7 => backend::v7::muxer::Muxer::open_timestamped_with_transport(
                path,
                width,
                height,
                time_base.into(),
                nominal_fps_num,
                nominal_fps_den,
                bitrate,
                hw,
                encode_threads,
                video_transport,
            )
            .map(Into::into),
            Version::V8 => backend::v8::muxer::Muxer::open_timestamped_with_transport(
                path,
                width,
                height,
                time_base.into(),
                nominal_fps_num,
                nominal_fps_den,
                bitrate,
                hw,
                encode_threads,
                video_transport,
            )
            .map(Into::into),
            Version::V9 => backend::v9::muxer::Muxer::open_timestamped_with_transport(
                path,
                width,
                height,
                time_base.into(),
                nominal_fps_num,
                nominal_fps_den,
                bitrate,
                hw,
                encode_threads,
                video_transport,
            )
            .map(Into::into),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn open_timestamped_media_with_transport(
        path: &Path,
        width: u32,
        height: u32,
        time_base: TimeBase,
        nominal_fps_num: u32,
        nominal_fps_den: u32,
        include_audio: bool,
        bitrate: Option<usize>,
        hw: bool,
        encode_threads: Option<usize>,
        video_transport: VideoFrameTransport,
    ) -> Result<Self> {
        match backend::version()? {
            Version::V7 => backend::v7::muxer::Muxer::open_timestamped_media_with_transport(
                path,
                width,
                height,
                time_base.into(),
                nominal_fps_num,
                nominal_fps_den,
                include_audio,
                bitrate,
                hw,
                encode_threads,
                video_transport,
            )
            .map(Into::into),
            Version::V8 => backend::v8::muxer::Muxer::open_timestamped_media_with_transport(
                path,
                width,
                height,
                time_base.into(),
                nominal_fps_num,
                nominal_fps_den,
                include_audio,
                bitrate,
                hw,
                encode_threads,
                video_transport,
            )
            .map(Into::into),
            Version::V9 => backend::v9::muxer::Muxer::open_timestamped_media_with_transport(
                path,
                width,
                height,
                time_base.into(),
                nominal_fps_num,
                nominal_fps_den,
                include_audio,
                bitrate,
                hw,
                encode_threads,
                video_transport,
            )
            .map(Into::into),
        }
    }
    pub fn shared_frame_pool(&self) -> Option<SharedVideoFramePool> {
        match &self.0 {
            Inner::V7(inner) => inner.shared_frame_pool().map(Into::into),
            Inner::V8(inner) => inner.shared_frame_pool().map(Into::into),
            Inner::V9(inner) => inner.shared_frame_pool().map(Into::into),
        }
    }
    pub fn expect_external_audio(&mut self) {
        match &mut self.0 {
            Inner::V7(inner) => inner.expect_external_audio(),
            Inner::V8(inner) => inner.expect_external_audio(),
            Inner::V9(inner) => inner.expect_external_audio(),
        }
    }
    pub fn audio_format(&self) -> (u32, u16) {
        match &self.0 {
            Inner::V7(inner) => inner.audio_format(),
            Inner::V8(inner) => inner.audio_format(),
            Inner::V9(inner) => inner.audio_format(),
        }
    }
    pub fn encode_audio(&mut self, buf: &AudioBuffer) -> Result<()> {
        match &mut self.0 {
            Inner::V7(inner) => inner.encode_audio(buf),
            Inner::V8(inner) => inner.encode_audio(buf),
            Inner::V9(inner) => inner.encode_audio(buf),
        }
    }
    pub fn encode_video(&mut self, frame: &RgbaFrame) -> Result<()> {
        match &mut self.0 {
            Inner::V7(inner) => inner.encode_video(frame),
            Inner::V8(inner) => inner.encode_video(frame),
            Inner::V9(inner) => inner.encode_video(frame),
        }
    }
    pub fn encode_video_at_pts(&mut self, frame: &RgbaFrame, pts: i64) -> Result<()> {
        match &mut self.0 {
            Inner::V7(inner) => inner.encode_video_at_pts(frame, pts),
            Inner::V8(inner) => inner.encode_video_at_pts(frame, pts),
            Inner::V9(inner) => inner.encode_video_at_pts(frame, pts),
        }
    }
    pub fn encode_video_at_pts_with_duration(
        &mut self,
        frame: &RgbaFrame,
        pts: i64,
        duration_ticks: Option<i64>,
    ) -> Result<()> {
        match &mut self.0 {
            Inner::V7(inner) => inner.encode_video_at_pts_with_duration(frame, pts, duration_ticks),
            Inner::V8(inner) => inner.encode_video_at_pts_with_duration(frame, pts, duration_ticks),
            Inner::V9(inner) => inner.encode_video_at_pts_with_duration(frame, pts, duration_ticks),
        }
    }
    pub fn encode_yuv(&mut self, yuv: YuvFrame) -> Result<Option<YuvFrame>> {
        match &mut self.0 {
            Inner::V7(inner) => inner.encode_yuv(yuv.into_v7()?).map(|v| v.map(Into::into)),
            Inner::V8(inner) => inner.encode_yuv(yuv.into_v8()?).map(|v| v.map(Into::into)),
            Inner::V9(inner) => inner.encode_yuv(yuv.into_v9()?).map(|v| v.map(Into::into)),
        }
    }
    pub fn encode_shared(&mut self, frame: SharedVideoFrame) -> Result<()> {
        match &mut self.0 {
            Inner::V7(inner) => inner.encode_shared(frame.into_v7()?),
            Inner::V8(inner) => inner.encode_shared(frame.into_v8()?),
            Inner::V9(inner) => inner.encode_shared(frame.into_v9()?),
        }
    }
    pub fn video_duration_seconds(&self) -> Result<f64> {
        match &self.0 {
            Inner::V7(inner) => inner.video_duration_seconds(),
            Inner::V8(inner) => inner.video_duration_seconds(),
            Inner::V9(inner) => inner.video_duration_seconds(),
        }
    }
    #[cfg(any(
        feature = "tool-inpaint",
        feature = "tool-interpolate",
        feature = "tool-upscale"
    ))]
    pub(crate) fn pump_silence_until(&mut self, target_s: f64) -> Result<()> {
        match &mut self.0 {
            Inner::V7(inner) => inner.pump_silence_until(target_s),
            Inner::V8(inner) => inner.pump_silence_until(target_s),
            Inner::V9(inner) => inner.pump_silence_until(target_s),
        }
    }
    pub fn finish(&mut self) -> Result<()> {
        match &mut self.0 {
            Inner::V7(inner) => inner.finish(),
            Inner::V8(inner) => inner.finish(),
            Inner::V9(inner) => inner.finish(),
        }
    }
}
backend_type!(YuvFrame, muxer);
impl YuvFrame {
    pub fn is_writable(&self) -> bool {
        match &self.0 {
            Inner::V7(inner) => inner.is_writable(),
            Inner::V8(inner) => inner.is_writable(),
            Inner::V9(inner) => inner.is_writable(),
        }
    }
}
backend_type!(RgbaToYuv, muxer);
impl RgbaToYuv {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        match backend::version()? {
            Version::V7 => backend::v7::muxer::RgbaToYuv::new(width, height).map(Into::into),
            Version::V8 => backend::v8::muxer::RgbaToYuv::new(width, height).map(Into::into),
            Version::V9 => backend::v9::muxer::RgbaToYuv::new(width, height).map(Into::into),
        }
    }
    pub fn convert(&mut self, frame: &RgbaFrame, recycled: Option<YuvFrame>) -> Result<YuvFrame> {
        match &mut self.0 {
            Inner::V7(inner) => inner
                .convert(frame, recycled.map(YuvFrame::into_v7).transpose()?)
                .map(Into::into),
            Inner::V8(inner) => inner
                .convert(frame, recycled.map(YuvFrame::into_v8).transpose()?)
                .map(Into::into),
            Inner::V9(inner) => inner
                .convert(frame, recycled.map(YuvFrame::into_v9).transpose()?)
                .map(Into::into),
        }
    }
}
backend_type!(AudioMuxer, muxer);
impl AudioMuxer {
    pub fn open(path: &Path) -> Result<Self> {
        match backend::version()? {
            Version::V7 => backend::v7::muxer::AudioMuxer::open(path).map(Into::into),
            Version::V8 => backend::v8::muxer::AudioMuxer::open(path).map(Into::into),
            Version::V9 => backend::v9::muxer::AudioMuxer::open(path).map(Into::into),
        }
    }
    pub fn audio_format(&self) -> (u32, u16) {
        match &self.0 {
            Inner::V7(inner) => inner.audio_format(),
            Inner::V8(inner) => inner.audio_format(),
            Inner::V9(inner) => inner.audio_format(),
        }
    }
    pub fn encode_audio(&mut self, buf: &AudioBuffer) -> Result<()> {
        match &mut self.0 {
            Inner::V7(inner) => inner.encode_audio(buf),
            Inner::V8(inner) => inner.encode_audio(buf),
            Inner::V9(inner) => inner.encode_audio(buf),
        }
    }
    pub fn finish(&mut self) -> Result<()> {
        match &mut self.0 {
            Inner::V7(inner) => inner.finish(),
            Inner::V8(inner) => inner.finish(),
            Inner::V9(inner) => inner.finish(),
        }
    }
}
pub fn hw_h264_available() -> bool {
    match match backend::version() {
        Ok(v) => v,
        Err(_) => return false,
    } {
        Version::V7 => backend::v7::muxer::hw_h264_available(),
        Version::V8 => backend::v8::muxer::hw_h264_available(),
        Version::V9 => backend::v9::muxer::hw_h264_available(),
    }
}
backend::owned_frame!(YuvFrame, muxer);
