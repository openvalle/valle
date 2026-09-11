//! Lossless binary-mask output with an ABI-independent clock.
use super::backend::{self, Inner, Version, backend_type};
use crate::{codec::TimeBase, frame::Gray8Frame};
use anyhow::Result as AnyResult;
use std::path::Path;
backend_type!(BinaryMaskVideoMuxer, mask);
impl BinaryMaskVideoMuxer {
    pub(crate) fn open(
        path: &Path,
        width: u32,
        height: u32,
        time_base: TimeBase,
        nominal_fps: f64,
    ) -> AnyResult<Self> {
        match backend::version()? {
            Version::V7 => backend::v7::mask::BinaryMaskVideoMuxer::open(
                path,
                width,
                height,
                time_base.into(),
                nominal_fps,
            )
            .map(Into::into),
            Version::V8 => backend::v8::mask::BinaryMaskVideoMuxer::open(
                path,
                width,
                height,
                time_base.into(),
                nominal_fps,
            )
            .map(Into::into),
            Version::V9 => backend::v9::mask::BinaryMaskVideoMuxer::open(
                path,
                width,
                height,
                time_base.into(),
                nominal_fps,
            )
            .map(Into::into),
        }
    }
    pub(crate) fn encode(&mut self, mask: &Gray8Frame, pts: i64) -> AnyResult<()> {
        match &mut self.0 {
            Inner::V7(inner) => inner.encode(mask, pts),
            Inner::V8(inner) => inner.encode(mask, pts),
            Inner::V9(inner) => inner.encode(mask, pts),
        }
    }
    pub(crate) fn finish(&mut self) -> AnyResult<()> {
        match &mut self.0 {
            Inner::V7(inner) => inner.finish(),
            Inner::V8(inner) => inner.finish(),
            Inner::V9(inner) => inner.finish(),
        }
    }
}
