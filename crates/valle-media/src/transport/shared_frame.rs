//! Version-independent shared frame interface. Native state stays in the selected adapter.
use crate::codec::backend::{self, Inner, backend_type};
use anyhow::Result;
use std::ffi::c_void;
/// How rendered video frames cross the renderer/encoder boundary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VideoFrameTransport {
    /// CPU-owned RGBA/YUV frames. This remains the portable default and fallback.
    #[default]
    Cpu,
    /// Platform surfaces imported by the GPU renderer. Depending on direction, the decoder or
    /// encoder owns the backing allocation while the frame object keeps it alive.
    SharedGpu,
}

/// Native API that owns a shared frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedFrameBackend {
    VideoToolbox,
    D3d11,
    DmaBuf,
    AndroidHardwareBuffer,
}

/// A borrowed native handle. It is valid only while the owning [`SharedVideoFrame`] or decoder
/// [`crate::SourceFrame`] is alive.
#[derive(Clone, Copy, Debug)]
pub enum SharedVideoFrameHandle {
    /// `CVPixelBufferRef`, backed by IOSurface and suitable for Metal import.
    VideoToolbox(*mut c_void),
    /// Reserved cross-platform contract; Windows implementation follows this shape.
    D3d11Texture(*mut c_void),
    /// Reserved cross-platform contract; Linux implementation follows this shape.
    DmaBuf { fd: i32, modifier: u64 },
    /// Reserved cross-platform contract; Android implementation follows this shape.
    AndroidHardwareBuffer(*mut c_void),
}

backend_type!(SharedVideoFramePool, shared_frame);
impl SharedVideoFramePool {
    pub fn backend(&self) -> SharedFrameBackend {
        match &self.0 {
            Inner::V7(inner) => inner.backend(),
            Inner::V8(inner) => inner.backend(),
            Inner::V9(inner) => inner.backend(),
        }
    }
    pub fn dimensions(&self) -> (u32, u32) {
        match &self.0 {
            Inner::V7(inner) => inner.dimensions(),
            Inner::V8(inner) => inner.dimensions(),
            Inner::V9(inner) => inner.dimensions(),
        }
    }
    pub fn acquire(&self) -> Result<SharedVideoFrame> {
        match &self.0 {
            Inner::V7(inner) => inner.acquire().map(Into::into),
            Inner::V8(inner) => inner.acquire().map(Into::into),
            Inner::V9(inner) => inner.acquire().map(Into::into),
        }
    }
}
backend_type!(SharedVideoFrame, shared_frame);
impl SharedVideoFrame {
    pub fn backend(&self) -> SharedFrameBackend {
        match &self.0 {
            Inner::V7(inner) => inner.backend(),
            Inner::V8(inner) => inner.backend(),
            Inner::V9(inner) => inner.backend(),
        }
    }
    pub fn dimensions(&self) -> (u32, u32) {
        match &self.0 {
            Inner::V7(inner) => inner.dimensions(),
            Inner::V8(inner) => inner.dimensions(),
            Inner::V9(inner) => inner.dimensions(),
        }
    }
    pub fn handle(&self) -> SharedVideoFrameHandle {
        match &self.0 {
            Inner::V7(inner) => inner.handle(),
            Inner::V8(inner) => inner.handle(),
            Inner::V9(inner) => inner.handle(),
        }
    }
}
backend::owned_frame!(SharedVideoFrame, shared_frame);
impl Clone for SharedVideoFramePool {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
