//! Backend-neutral shared GPU video-frame contract.
//!
//! On the output side, a [`SharedVideoFramePool`] is created by the encoder so every acquired frame
//! is already backed by memory that the platform encoder accepts. The renderer imports
//! [`SharedVideoFrame::handle`] into its GPU API, renders in place, and returns the same frame to
//! [`crate::Muxer`]. Decoder-owned frames use the same backend/handle vocabulary through
//! [`crate::SourceFrame`]. This keeps platform handles out of the timeline/render contracts and
//! gives D3D, DMA-BUF and AHardwareBuffer backends the same seam as VideoToolbox.

use std::ffi::c_void;
#[cfg(target_os = "macos")]
use std::ptr;

use anyhow::{Result, anyhow};
use ffmpeg_next as ff;
use ffmpeg_next::frame;

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

/// Reference-counted FFmpeg hardware-frame pool.
pub struct SharedVideoFramePool {
    frames_ref: *mut ff::ffi::AVBufferRef,
    backend: SharedFrameBackend,
    width: u32,
    height: u32,
}

// AVBufferRef itself is atomic-reference-counted. Frame acquisition from the VideoToolbox pool is
// thread-safe, and every acquired AVFrame owns its own buffer reference.
unsafe impl Send for SharedVideoFramePool {}
unsafe impl Sync for SharedVideoFramePool {}

impl Clone for SharedVideoFramePool {
    fn clone(&self) -> Self {
        let frames_ref = unsafe { ff::ffi::av_buffer_ref(self.frames_ref) };
        debug_assert!(!frames_ref.is_null());
        Self {
            frames_ref,
            backend: self.backend,
            width: self.width,
            height: self.height,
        }
    }
}

impl Drop for SharedVideoFramePool {
    fn drop(&mut self) {
        unsafe { ff::ffi::av_buffer_unref(&mut self.frames_ref) };
    }
}

impl SharedVideoFramePool {
    /// Build the Apple VideoToolbox pool used by both the encoder and Metal renderer.
    #[cfg(target_os = "macos")]
    pub(crate) fn videotoolbox_bgra(width: u32, height: u32) -> Result<Self> {
        let mut device_ref = ptr::null_mut();
        let code = unsafe {
            ff::ffi::av_hwdevice_ctx_create(
                &mut device_ref,
                ff::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                ptr::null(),
                ptr::null_mut(),
                0,
            )
        };
        av_result(code, "create VideoToolbox device")?;
        if device_ref.is_null() {
            return Err(anyhow!("VideoToolbox returned a null hardware device"));
        }

        let frames_ref = unsafe { ff::ffi::av_hwframe_ctx_alloc(device_ref) };
        unsafe { ff::ffi::av_buffer_unref(&mut device_ref) };
        if frames_ref.is_null() {
            return Err(anyhow!("allocate VideoToolbox frame context"));
        }

        let frames_ctx = unsafe { (*frames_ref).data.cast::<ff::ffi::AVHWFramesContext>() };
        if frames_ctx.is_null() {
            let mut frames_ref = frames_ref;
            unsafe { ff::ffi::av_buffer_unref(&mut frames_ref) };
            return Err(anyhow!("VideoToolbox frame context has no payload"));
        }
        unsafe {
            (*frames_ctx).format = ff::ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX;
            // BGRA lets Skia render directly into one Metal texture. VideoToolbox performs its
            // encoder-side color conversion without a CPU-visible RGBA readback.
            (*frames_ctx).sw_format = ff::ffi::AVPixelFormat::AV_PIX_FMT_BGRA;
            (*frames_ctx).width = width as i32;
            (*frames_ctx).height = height as i32;
            // VideoToolbox's CoreVideo pool grows dynamically; this is only an initial hint.
            (*frames_ctx).initial_pool_size = 4;
        }
        if let Err(error) = av_result(
            unsafe { ff::ffi::av_hwframe_ctx_init(frames_ref) },
            "initialize VideoToolbox BGRA frame pool",
        ) {
            let mut frames_ref = frames_ref;
            unsafe { ff::ffi::av_buffer_unref(&mut frames_ref) };
            return Err(error);
        }
        Ok(Self {
            frames_ref,
            backend: SharedFrameBackend::VideoToolbox,
            width,
            height,
        })
    }

    pub fn backend(&self) -> SharedFrameBackend {
        self.backend
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Acquire an encoder-owned surface. Dropping the returned frame recycles it into CoreVideo's
    /// pool once both Skia and VideoToolbox have released their references.
    pub fn acquire(&self) -> Result<SharedVideoFrame> {
        let mut inner = frame::Video::empty();
        av_result(
            unsafe { ff::ffi::av_hwframe_get_buffer(self.frames_ref, inner.as_mut_ptr(), 0) },
            "acquire shared GPU video frame",
        )?;
        let native = unsafe { (*inner.as_ptr()).data[3] };
        if native.is_null() {
            return Err(anyhow!(
                "shared {:?} frame has no native handle",
                self.backend
            ));
        }
        Ok(SharedVideoFrame {
            inner,
            backend: self.backend,
            width: self.width,
            height: self.height,
        })
    }

    /// Clone a reference for `AVCodecContext.hw_frames_ctx`; libavcodec takes ownership of it.
    pub(crate) fn encoder_frames_ref(&self) -> Result<*mut ff::ffi::AVBufferRef> {
        let cloned = unsafe { ff::ffi::av_buffer_ref(self.frames_ref) };
        (!cloned.is_null())
            .then_some(cloned)
            .ok_or_else(|| anyhow!("retain shared GPU frame pool for encoder"))
    }
}

/// One encoder-owned shared video surface.
pub struct SharedVideoFrame {
    inner: frame::Video,
    backend: SharedFrameBackend,
    width: u32,
    height: u32,
}

// The AVFrame and its AVBufferRefs are uniquely owned here. It is moved (never concurrently
// accessed) from raster worker to the main-thread encoder.
unsafe impl Send for SharedVideoFrame {}

impl SharedVideoFrame {
    pub fn backend(&self) -> SharedFrameBackend {
        self.backend
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn handle(&self) -> SharedVideoFrameHandle {
        let native = unsafe { (*self.inner.as_ptr()).data[3].cast::<c_void>() };
        match self.backend {
            SharedFrameBackend::VideoToolbox => SharedVideoFrameHandle::VideoToolbox(native),
            SharedFrameBackend::D3d11 => SharedVideoFrameHandle::D3d11Texture(native),
            SharedFrameBackend::DmaBuf => SharedVideoFrameHandle::DmaBuf {
                fd: -1,
                modifier: 0,
            },
            SharedFrameBackend::AndroidHardwareBuffer => {
                SharedVideoFrameHandle::AndroidHardwareBuffer(native)
            }
        }
    }

    pub(crate) fn into_inner(self) -> frame::Video {
        self.inner
    }
}

fn av_result(code: i32, operation: &str) -> Result<()> {
    if code < 0 {
        Err(anyhow!("{operation}: {}", ff::Error::from(code)))
    } else {
        Ok(())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a real VideoToolbox device"]
    fn allocates_videotoolbox_bgra_frame() {
        crate::ffi::ffmpeg_init();
        let pool = SharedVideoFramePool::videotoolbox_bgra(64, 48).unwrap();
        let frame = pool.acquire().unwrap();
        assert_eq!(frame.dimensions(), (64, 48));
        assert!(matches!(
            frame.handle(),
            SharedVideoFrameHandle::VideoToolbox(ptr) if !ptr.is_null()
        ));
    }
}
