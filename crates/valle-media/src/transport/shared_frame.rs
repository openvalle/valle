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

#[cfg(all(test, target_os = "macos"))]
mod contract_tests {
    use super::*;

    #[link(name = "CoreVideo", kind = "framework")]
    unsafe extern "C" {
        fn CVPixelBufferGetWidth(buffer: *mut c_void) -> usize;
        fn CVPixelBufferGetHeight(buffer: *mut c_void) -> usize;
    }

    #[test]
    #[ignore = "requires a real VideoToolbox frame pool"]
    fn native_frame_outlives_pool_and_encoder_and_keeps_its_geometry() {
        let root = tempfile::tempdir().unwrap();
        let muxer = crate::Muxer::open_ext_with_transport(
            &root.path().join("shared.mp4"),
            64,
            48,
            30,
            None,
            true,
            None,
            VideoFrameTransport::SharedGpu,
        )
        .unwrap();
        let pool = muxer.shared_frame_pool().unwrap();
        let frame = pool.acquire().unwrap();
        drop(pool);
        drop(muxer);
        let backend = std::hint::black_box(
            SharedVideoFrame::backend as fn(&SharedVideoFrame) -> SharedFrameBackend,
        );
        let dimensions = std::hint::black_box(
            SharedVideoFrame::dimensions as fn(&SharedVideoFrame) -> (u32, u32),
        );
        assert_eq!(backend(&frame), SharedFrameBackend::VideoToolbox);
        assert_eq!(dimensions(&frame), (64, 48));
        let SharedVideoFrameHandle::VideoToolbox(buffer) = frame.handle() else {
            panic!("expected a VideoToolbox frame")
        };
        assert!(!buffer.is_null());
        // The frame owns this borrowed CVPixelBuffer after both producers have gone away.
        unsafe {
            assert_eq!(CVPixelBufferGetWidth(buffer), 64);
            assert_eq!(CVPixelBufferGetHeight(buffer), 48);
        }
    }
}
