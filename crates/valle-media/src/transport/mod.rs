//! Host-owned media sources and shared platform frame handles.

#[cfg(feature = "libav")]
pub mod shared_frame;
pub mod source;

#[cfg(feature = "libav")]
pub use shared_frame::{
    SharedFrameBackend, SharedVideoFrame, SharedVideoFrameHandle, SharedVideoFramePool,
    VideoFrameTransport,
};
pub use source::{AudioSource, SourceFrame, SourceMeta, VideoSource};
