//! Media data, codecs, analysis, and platform transport with shared private implementation types
//! and no dependency on Engine or Timeline.

pub mod analysis;
pub mod codec;
pub mod frame;
pub mod models;
pub mod tools;
pub mod transport;

pub use codec::*;
pub use frame::{AudioBuffer, RgbaFrame};
pub use transport::{AudioSource, SourceFrame, SourceMeta, VideoSource};
#[cfg(feature = "libav")]
pub use transport::{
    SharedFrameBackend, SharedVideoFrame, SharedVideoFrameHandle, SharedVideoFramePool,
    VideoFrameTransport,
};
