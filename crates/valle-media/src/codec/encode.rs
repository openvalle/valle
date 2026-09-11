//! Version-independent encode interface. Native state stays in the selected adapter.
use crate::codec::backend::{self, Inner, Version, backend_type};
use crate::{codec::Encoder, frame::RgbaFrame};
use anyhow::Result;
use std::path::Path;
backend_type!(Mp4Encoder, encode);
impl Mp4Encoder {
    pub fn new(
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: Option<usize>,
    ) -> Result<Self> {
        match backend::version()? {
            Version::V7 => backend::v7::encode::Mp4Encoder::new(path, width, height, fps, bitrate)
                .map(Into::into),
            Version::V8 => backend::v8::encode::Mp4Encoder::new(path, width, height, fps, bitrate)
                .map(Into::into),
            Version::V9 => backend::v9::encode::Mp4Encoder::new(path, width, height, fps, bitrate)
                .map(Into::into),
        }
    }
}
impl Encoder for Mp4Encoder {
    fn encode_frame(&mut self, frame: &RgbaFrame) -> Result<()> {
        match &mut self.0 {
            Inner::V7(inner) => inner.encode_frame(frame),
            Inner::V8(inner) => inner.encode_frame(frame),
            Inner::V9(inner) => inner.encode_frame(frame),
        }
    }
    fn finish(&mut self) -> Result<()> {
        match &mut self.0 {
            Inner::V7(inner) => inner.finish(),
            Inner::V8(inner) => inner.finish(),
            Inner::V9(inner) => inner.finish(),
        }
    }
}
