//! An encoder's frames as wgpu textures: on macOS a VideoToolbox pixel buffer's two planes, bound
//! through CoreVideo's Metal texture cache, so the GPU draws Y'CbCr into the encoder's own memory,
//! with nothing read back or converted on the CPU. Elsewhere there are none.

pub use imp::{Planes, Textures};

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_metal::{MTLTexture, MTLTextureType};

    use crate::media::Surface;

    #[link(name = "CoreVideo", kind = "framework")]
    unsafe extern "C" {
        fn CVMetalTextureCacheCreate(
            allocator: *const c_void,
            cache_attributes: *const c_void,
            metal_device: *mut c_void,
            texture_attributes: *const c_void,
            cache_out: *mut *mut c_void,
        ) -> i32;
        fn CVMetalTextureCacheCreateTextureFromImage(
            allocator: *const c_void,
            texture_cache: *mut c_void,
            source_image: *mut c_void,
            texture_attributes: *const c_void,
            pixel_format: usize,
            width: usize,
            height: usize,
            plane_index: usize,
            texture_out: *mut *mut c_void,
        ) -> i32;
        fn CVMetalTextureGetTexture(texture: *mut c_void) -> *mut c_void;
        fn CVMetalTextureCacheFlush(texture_cache: *mut c_void, options: u64);
        fn CVPixelBufferGetWidthOfPlane(buffer: *mut c_void, plane: usize) -> usize;
        fn CVPixelBufferGetHeightOfPlane(buffer: *mut c_void, plane: usize) -> usize;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: *const c_void);
    }

    /// Metal's `R8Unorm` and `RG8Unorm`.
    const R8: usize = 10;
    const RG8: usize = 30;

    /// One owned reference to a CoreFoundation object.
    struct Cf(*mut c_void);

    impl Drop for Cf {
        fn drop(&mut self) {
            // SAFETY: the reference is ours.
            unsafe { CFRelease(self.0) };
        }
    }

    /// CoreVideo's cache of Metal textures of pixel buffers, for one device.
    pub struct Textures {
        cache: Cf,
    }

    /// A surface's planes as textures to draw into: luma, and Cb and Cr at half size. They
    /// stay bound to its pixel buffer while this is kept, which is until the GPU is done.
    pub struct Planes {
        pub luma: wgpu::TextureView,
        pub chroma: wgpu::TextureView,
        _textures: [wgpu::Texture; 2],
        _bound: [Cf; 2],
    }

    impl Textures {
        /// The cache for `device`: `None` where it is not a Metal device.
        pub fn new(device: &wgpu::Device) -> Option<Textures> {
            // SAFETY: the raw device is only passed to CoreVideo, which retains it.
            let hal = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }?;
            let raw = Retained::as_ptr(hal.raw_device())
                .cast_mut()
                .cast::<c_void>();
            let mut cache = null_mut();
            // SAFETY: CoreVideo fills `cache` or fails.
            let code =
                unsafe { CVMetalTextureCacheCreate(null(), null(), raw, null(), &mut cache) };
            (code == 0 && !cache.is_null()).then(|| Textures { cache: Cf(cache) })
        }

        /// The planes of `surface`'s pixel buffer, NV12, as textures of `device`, this cache's.
        pub fn planes(&self, device: &wgpu::Device, surface: &Surface) -> Result<Planes, String> {
            let (luma, l) =
                self.plane(device, surface.buffer, 0, R8, wgpu::TextureFormat::R8Unorm)?;
            let (chroma, c) = self.plane(
                device,
                surface.buffer,
                1,
                RG8,
                wgpu::TextureFormat::Rg8Unorm,
            )?;
            Ok(Planes {
                luma: luma.create_view(&Default::default()),
                chroma: chroma.create_view(&Default::default()),
                _textures: [luma, chroma],
                _bound: [l, c],
            })
        }

        /// Lets the cache release the textures nothing uses any more, and with them their pixel
        /// buffers, which go back to the encoder's pool.
        pub fn flush(&self) {
            // SAFETY: the cache is ours.
            unsafe { CVMetalTextureCacheFlush(self.cache.0, 0) };
        }

        /// Plane `plane` of `buffer` as a wgpu texture of `format`, Metal's and wgpu's, and the
        /// CoreVideo texture that binds it.
        fn plane(
            &self,
            device: &wgpu::Device,
            buffer: *mut c_void,
            plane: usize,
            metal: usize,
            format: wgpu::TextureFormat,
        ) -> Result<(wgpu::Texture, Cf), String> {
            // SAFETY: `buffer` is a live pixel buffer with the plane.
            let (w, h) = unsafe {
                (
                    CVPixelBufferGetWidthOfPlane(buffer, plane) as u32,
                    CVPixelBufferGetHeightOfPlane(buffer, plane) as u32,
                )
            };
            let mut cv = null_mut();
            // SAFETY: CoreVideo fills `cv` or fails.
            let code = unsafe {
                CVMetalTextureCacheCreateTextureFromImage(
                    null(),
                    self.cache.0,
                    buffer,
                    null(),
                    metal,
                    w as usize,
                    h as usize,
                    plane,
                    &mut cv,
                )
            };
            if code != 0 || cv.is_null() {
                return Err(format!(
                    "a Metal texture of the encoder's frame: CVReturn {code}"
                ));
            }
            let cv = Cf(cv);
            // SAFETY: the CoreVideo texture has a Metal texture, which wgpu retains as well.
            let raw =
                unsafe { CVMetalTextureGetTexture(cv.0) }.cast::<ProtocolObject<dyn MTLTexture>>();
            let raw = unsafe { Retained::retain(raw) }.ok_or("a Metal texture without one")?;
            let size = wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            };
            // SAFETY: the texture is the device's, of the format and size given, and drawn
            // into whole before it is read.
            let texture = unsafe {
                let hal = wgpu::hal::metal::Device::texture_from_raw(
                    raw,
                    format,
                    MTLTextureType::Type2D,
                    1,
                    1,
                    wgpu::hal::CopyExtent {
                        width: w,
                        height: h,
                        depth: 1,
                    },
                    None,
                );
                device.create_texture_from_hal::<wgpu::hal::api::Metal>(
                    hal,
                    &wgpu::TextureDescriptor {
                        label: Some("encoder frame"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    },
                    wgpu::TextureUses::UNINITIALIZED,
                )
            };
            Ok((texture, cv))
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use crate::media::Surface;

    /// None: encoders here take frames through `Write::push` only.
    pub struct Textures;

    pub struct Planes {
        pub luma: wgpu::TextureView,
        pub chroma: wgpu::TextureView,
    }

    impl Textures {
        pub fn new(_: &wgpu::Device) -> Option<Textures> {
            None
        }

        pub fn planes(&self, _: &wgpu::Device, _: &Surface) -> Result<Planes, String> {
            Err("no encoder frames to draw into here".into())
        }

        pub fn flush(&self) {}
    }
}
