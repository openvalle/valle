//! Version-independent alpha interface. Native state stays in the selected adapter.
#[cfg(feature = "tool-segment")]
use crate::codec::TimeBase;
use crate::codec::backend::{self, Inner, Version, backend_type};
use crate::frame::RgbaFrame;
use anyhow::Result;
use std::path::Path;
backend_type!(TransparentVideoMuxer, alpha);
impl TransparentVideoMuxer {
    pub fn open(path: &Path, width: u32, height: u32, fps_num: u32, fps_den: u32) -> Result<Self> {
        match backend::version()? {
            Version::V7 => backend::v7::alpha::TransparentVideoMuxer::open(
                path, width, height, fps_num, fps_den,
            )
            .map(Into::into),
            Version::V8 => backend::v8::alpha::TransparentVideoMuxer::open(
                path, width, height, fps_num, fps_den,
            )
            .map(Into::into),
            Version::V9 => backend::v9::alpha::TransparentVideoMuxer::open(
                path, width, height, fps_num, fps_den,
            )
            .map(Into::into),
        }
    }
    #[cfg(feature = "tool-segment")]
    pub(crate) fn open_timed(
        path: &Path,
        width: u32,
        height: u32,
        time_base: TimeBase,
        nominal_frame_rate: Option<TimeBase>,
    ) -> Result<Self> {
        match backend::version()? {
            Version::V7 => backend::v7::alpha::TransparentVideoMuxer::open_timed(
                path,
                width,
                height,
                time_base.into(),
                nominal_frame_rate.map(Into::into),
            )
            .map(Into::into),
            Version::V8 => backend::v8::alpha::TransparentVideoMuxer::open_timed(
                path,
                width,
                height,
                time_base.into(),
                nominal_frame_rate.map(Into::into),
            )
            .map(Into::into),
            Version::V9 => backend::v9::alpha::TransparentVideoMuxer::open_timed(
                path,
                width,
                height,
                time_base.into(),
                nominal_frame_rate.map(Into::into),
            )
            .map(Into::into),
        }
    }
    pub fn encode_video(&mut self, rgba: &RgbaFrame) -> Result<()> {
        match &mut self.0 {
            Inner::V7(inner) => inner.encode_video(rgba),
            Inner::V8(inner) => inner.encode_video(rgba),
            Inner::V9(inner) => inner.encode_video(rgba),
        }
    }
    #[cfg(feature = "tool-segment")]
    pub(crate) fn encode_video_at(&mut self, rgba: &RgbaFrame, pts: i64) -> Result<()> {
        match &mut self.0 {
            Inner::V7(inner) => inner.encode_video_at(rgba, pts),
            Inner::V8(inner) => inner.encode_video_at(rgba, pts),
            Inner::V9(inner) => inner.encode_video_at(rgba, pts),
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
