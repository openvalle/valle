#[cfg(all(target_os = "macos", feature = "native"))]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
};

use skia_safe::{
    AlphaType, ColorSpace, ColorType, IRect, Image, ImageInfo, Surface, named_primaries,
    named_transfer_fn, surfaces,
};
#[cfg(all(
    feature = "native",
    any(target_os = "macos", target_os = "linux", target_os = "windows")
))]
use skia_safe::{BlendMode, Color4f, Paint, Shader, gpu};
#[cfg(all(target_os = "macos", feature = "native"))]
use skia_safe::{EncodedOrigin, YUVAInfo, YUVColorSpace};

/// Physical Skia storage selected for a runner. This is execution evidence, not a capability
/// discriminator: Engine lowering still consumes only the immutable capability contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkiaBackendKind {
    Raster,
    #[cfg(all(target_os = "macos", feature = "native"))]
    Metal,
    #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
    Vulkan,
}

impl SkiaBackendKind {
    /// Choose an available device once at the delivery boundary. Sandboxed/headless macOS
    /// processes may have no Metal device even though the binary includes Metal support.
    pub fn preferred_available() -> Self {
        #[cfg(all(target_os = "macos", feature = "native"))]
        if objc2_metal::MTLCreateSystemDefaultDevice().is_some() {
            return Self::Metal;
        }
        #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
        match super::vulkan::VulkanContext::new() {
            Ok(_) => return Self::Vulkan,
            Err(error) if std::env::var_os("VALLE_PERF").is_some() => {
                eprintln!("[valle] {error}; using Raster");
            }
            Err(_) => {}
        }
        Self::Raster
    }

    /// GPU backends can retain deferred Scene3D input and fall back on unsupported devices.
    pub const fn supports_scene3d_gpu(self) -> bool {
        match self {
            #[cfg(all(target_os = "macos", feature = "native"))]
            Self::Metal => true,
            #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
            Self::Vulkan => true,
            _ => false,
        }
    }
}

/// Font-outline coverage implementation used by the platform `SkFontMgr` selected for this
/// executor. This is intentionally separate from [`SkiaBackendKind`]: a Metal surface on macOS
/// still rasterizes `TextBlob` glyphs through CoreText, while CanvasKit uses FreeType.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphRasterizerKind {
    CoreText,
    DirectWrite,
    FreeType,
    PlatformDefault,
}

impl GlyphRasterizerKind {
    pub const fn native_default() -> Self {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            return Self::CoreText;
        }
        #[cfg(target_os = "windows")]
        {
            return Self::DirectWrite;
        }
        #[cfg(any(
            target_os = "linux",
            target_os = "android",
            target_os = "freebsd",
            target_os = "openbsd"
        ))]
        {
            return Self::FreeType;
        }
        #[allow(unreachable_code)]
        Self::PlatformDefault
    }
}

/// Where the glyph rasterizer implementation comes from. The broad rasterizer family is not a
/// sufficient pixel identity: CanvasKit embeds FreeType, while a normal Linux rust-skia build
/// dynamically uses the host FreeType. Those profiles must not share strict pixel baselines just
/// because both report [`GlyphRasterizerKind::FreeType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphLibraryKind {
    PlatformFramework,
    SystemLibrary,
    RendererEmbedded,
    PlatformDefault,
}

impl GlyphLibraryKind {
    pub const fn native_default() -> Self {
        #[cfg(any(target_os = "macos", target_os = "ios", target_os = "windows"))]
        {
            return Self::PlatformFramework;
        }
        #[cfg(target_os = "android")]
        {
            return Self::RendererEmbedded;
        }
        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd"))]
        {
            return Self::SystemLibrary;
        }
        #[allow(unreachable_code)]
        Self::PlatformDefault
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphHintingKind {
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphEdgingKind {
    AntiAlias,
}

/// Immutable coverage policy for product glyph drawing. Font bytes, face, size, glyph ids and
/// positions remain backend-free DrawProgram semantics; this profile identifies the physical
/// outline-to-mask implementation and the SkFont knobs that may change delivery pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphCoverageProfile {
    pub rasterizer: GlyphRasterizerKind,
    pub library: GlyphLibraryKind,
    pub hinting: GlyphHintingKind,
    pub edging: GlyphEdgingKind,
    pub subpixel_positioning: bool,
}

impl GlyphCoverageProfile {
    pub const fn native_default() -> Self {
        Self {
            rasterizer: GlyphRasterizerKind::native_default(),
            library: GlyphLibraryKind::native_default(),
            hinting: GlyphHintingKind::None,
            edging: GlyphEdgingKind::AntiAlias,
            subpixel_positioning: false,
        }
    }

    pub(crate) fn configure(self, font: &mut skia_safe::Font) {
        match self.hinting {
            GlyphHintingKind::None => font.set_hinting(skia_safe::FontHinting::None),
        };
        match self.edging {
            GlyphEdgingKind::AntiAlias => font.set_edging(skia_safe::font::Edging::AntiAlias),
        };
        font.set_subpixel(self.subpixel_positioning);
    }
}

/// Immutable physical execution identity carried by every Native frame report. RenderPlan stays
/// backend-free; this profile makes platform-dependent delivery pixels auditable instead of
/// pretending that `Raster`/`Metal` also identifies the glyph rasterizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkiaExecutionProfile {
    pub surface_backend: SkiaBackendKind,
    pub glyph_coverage: GlyphCoverageProfile,
    pub skia_milestone: usize,
}

impl SkiaExecutionProfile {
    pub const fn native(surface_backend: SkiaBackendKind) -> Self {
        Self {
            surface_backend,
            glyph_coverage: GlyphCoverageProfile::native_default(),
            skia_milestone: skia_safe::MILESTONE,
        }
    }
}

#[derive(Clone)]
pub(crate) struct PlanImage {
    image: Option<Image>,
    roi: valle_engine::prepare::DeviceRect,
    retention: Option<valle_engine::motion::memory::MemoryLease>,
}

impl PlanImage {
    pub(crate) fn new(image: Image, roi: valle_engine::prepare::DeviceRect) -> Self {
        Self {
            image: Some(image),
            roi,
            retention: None,
        }
    }

    pub(crate) const fn transparent() -> Self {
        Self {
            image: None,
            roi: valle_engine::prepare::DeviceRect::new(0, 0, 0, 0),
            retention: None,
        }
    }

    pub(crate) fn root(image: Image, extent: Extent2d) -> Self {
        Self::new(
            image,
            valle_engine::prepare::DeviceRect::full(extent.width(), extent.height()),
        )
    }

    pub(crate) const fn roi(&self) -> valle_engine::prepare::DeviceRect {
        self.roi
    }

    pub(crate) fn image(&self) -> Option<&Image> {
        self.image.as_ref()
    }

    /// Retaining a snapshot can trigger copy-on-write on its original surface. Charge that
    /// independent storage before caching, and keep the lease on every returned image clone.
    pub(crate) fn admit_retention(&mut self, bytes: u64) -> bool {
        if self.retention.is_some() {
            return true;
        }
        match valle_engine::motion::memory::reserve_retained(
            valle_engine::motion::memory::MemoryKind::Surface,
            bytes,
        ) {
            Ok(lease) => {
                self.retention = lease;
                true
            }
            Err(_) => false, // Reuse is optional; an otherwise valid frame must still draw.
        }
    }
}

#[cfg(all(target_os = "macos", feature = "native"))]
pub(crate) struct SharedMetalFrame {
    surface: Option<Surface>,
    texture: CoreFoundationHandle,
    frame: SharedVideoFrame,
}

#[cfg(all(target_os = "macos", feature = "native"))]
impl SharedMetalFrame {
    pub(crate) fn surface_mut(&mut self) -> Result<&mut Surface, SurfaceError> {
        self.surface
            .as_mut()
            .ok_or(SurfaceError::SharedFrameSubmitted)
    }
}

#[cfg(all(target_os = "macos", feature = "native"))]
pub(crate) struct SubmittedSharedMetalFrame {
    surface: Option<Surface>,
    texture: CoreFoundationHandle,
    frame: SharedVideoFrame,
    completion: GpuCompletion,
    #[cfg(feature = "native")]
    external_imports: Vec<ImportedMetalImage>,
}

#[cfg(all(target_os = "macos", feature = "native"))]
struct ImportedMetalImage {
    image: Image,
    _textures: Vec<CoreFoundationHandle>,
    _source: ImportedMetalOwner,
}

#[cfg(all(target_os = "macos", feature = "native"))]
enum ImportedMetalOwner {
    Video {
        _frame: Arc<valle_media::SourceFrame>,
    },
    Scene {
        _resources: Box<(
            super::scene3d_gpu::GpuRasterOutput,
            super::SkiaExternalObject,
        )>,
    },
}

#[cfg(all(target_os = "macos", feature = "native"))]
#[derive(Clone)]
struct GpuCompletion(Arc<AtomicBool>);

#[cfg(all(target_os = "macos", feature = "native"))]
impl GpuCompletion {
    fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    fn callback_context(&self) -> *mut core::ffi::c_void {
        Arc::into_raw(Arc::clone(&self.0))
            .cast_mut()
            .cast::<core::ffi::c_void>()
    }

    fn is_ready(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[cfg(all(target_os = "macos", feature = "native"))]
unsafe extern "C" fn gpu_finished(context: *mut core::ffi::c_void) {
    if context.is_null() {
        return;
    }
    let done = unsafe { Arc::<AtomicBool>::from_raw(context.cast::<AtomicBool>()) };
    done.store(true, Ordering::Release);
}
use thiserror::Error;
#[cfg(all(target_os = "macos", feature = "native"))]
use valle_engine::compositor::ExternalObject;
use valle_engine::{
    compositor::lower::{
        BoundProgramSchedules, BoundSurfaceSlot, RenderPlanTemplate, SurfaceSlotId,
    },
    resource::{
        ColorDescription, ColorPrimaries, Extent2d, OutputSpec, TextureFormat, TransferFunction,
        WorkingAlphaMode, WorkingColorSpace,
    },
};
#[cfg(all(target_os = "macos", feature = "native"))]
use valle_media::{SharedVideoFrame, SharedVideoFrameHandle, SharedVideoFramePool};

#[cfg(all(target_os = "macos", feature = "native"))]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {}

#[cfg(all(target_os = "macos", feature = "native"))]
#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVMetalTextureCacheCreate(
        allocator: *const core::ffi::c_void,
        cache_attributes: *const core::ffi::c_void,
        metal_device: *mut core::ffi::c_void,
        texture_attributes: *const core::ffi::c_void,
        cache_out: *mut *mut core::ffi::c_void,
    ) -> i32;
    fn CVMetalTextureCacheCreateTextureFromImage(
        allocator: *const core::ffi::c_void,
        texture_cache: *mut core::ffi::c_void,
        source_image: *mut core::ffi::c_void,
        texture_attributes: *const core::ffi::c_void,
        pixel_format: usize,
        width: usize,
        height: usize,
        plane_index: usize,
        texture_out: *mut *mut core::ffi::c_void,
    ) -> i32;
    fn CVMetalTextureGetTexture(texture: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    #[cfg(feature = "native")]
    fn CVPixelBufferGetPixelFormatType(pixel_buffer: *mut core::ffi::c_void) -> u32;
    #[cfg(feature = "native")]
    fn CVPixelBufferGetPlaneCount(pixel_buffer: *mut core::ffi::c_void) -> usize;
    #[cfg(feature = "native")]
    fn CVPixelBufferGetWidth(pixel_buffer: *mut core::ffi::c_void) -> usize;
    #[cfg(feature = "native")]
    fn CVPixelBufferGetHeight(pixel_buffer: *mut core::ffi::c_void) -> usize;
    #[cfg(feature = "native")]
    fn CVPixelBufferGetWidthOfPlane(
        pixel_buffer: *mut core::ffi::c_void,
        plane_index: usize,
    ) -> usize;
    #[cfg(feature = "native")]
    fn CVPixelBufferGetHeightOfPlane(
        pixel_buffer: *mut core::ffi::c_void,
        plane_index: usize,
    ) -> usize;
    fn CVPixelBufferLockBaseAddress(pixel_buffer: *mut core::ffi::c_void, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pixel_buffer: *mut core::ffi::c_void, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(pixel_buffer: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    fn CVPixelBufferGetBytesPerRow(pixel_buffer: *mut core::ffi::c_void) -> usize;
}

/// Acquires an encoder-owned VideoToolbox frame and runs `draw` on a Raster surface over its BGRA
/// pixels, which stay locked for CPU writes until `draw` returns. VideoToolbox converts BGRA to
/// YUV as it encodes, so Raster delivery needs neither a readback nor a CPU colour conversion.
#[cfg(all(target_os = "macos", feature = "native"))]
pub(crate) fn draw_shared_raster_frame<R>(
    pool: &SharedVideoFramePool,
    info: &ImageInfo,
    draw: impl FnOnce(&mut Surface) -> R,
) -> Result<(SharedVideoFrame, R), SurfaceError> {
    struct Locked(*mut core::ffi::c_void);
    impl Drop for Locked {
        fn drop(&mut self) {
            unsafe { CVPixelBufferUnlockBaseAddress(self.0, 0) };
        }
    }

    let frame = pool
        .acquire()
        .map_err(|error| SurfaceError::SharedFrame(error.to_string()))?;
    let SharedVideoFrameHandle::VideoToolbox(pixel_buffer) = frame.handle() else {
        return Err(SurfaceError::SharedFrame(
            "Raster shared target requires a VideoToolbox frame".into(),
        ));
    };
    let (width, height) = frame.dimensions();
    if pixel_buffer.is_null()
        || unsafe { CVPixelBufferGetPixelFormatType(pixel_buffer) } != CV_PIXEL_FORMAT_BGRA
        || info.color_type() != ColorType::BGRA8888
        || (info.width(), info.height()) != (width as i32, height as i32)
    {
        return Err(SurfaceError::SharedFrame(
            "Raster shared target must match the frame's BGRA extent".into(),
        ));
    }
    if unsafe { CVPixelBufferLockBaseAddress(pixel_buffer, 0) } != 0 {
        return Err(SurfaceError::SharedFrame(
            "cannot lock the VideoToolbox frame for CPU writes".into(),
        ));
    }
    let locked = Locked(pixel_buffer);
    let base = unsafe { CVPixelBufferGetBaseAddress(pixel_buffer) }.cast::<u8>();
    let row_bytes = unsafe { CVPixelBufferGetBytesPerRow(pixel_buffer) };
    if base.is_null() || row_bytes < info.min_row_bytes() {
        return Err(SurfaceError::SharedFrame(
            "VideoToolbox frame has no CPU mapping".into(),
        ));
    }
    // SAFETY: the lock maps `row_bytes` × `height` bytes, and the surface over them is dropped
    // before the lock is released.
    let pixels = unsafe { std::slice::from_raw_parts_mut(base, row_bytes * height as usize) };
    let mut surface =
        surfaces::wrap_pixels(info, pixels, row_bytes, None).ok_or(SurfaceError::Allocation)?;
    let result = draw(&mut surface);
    drop(surface);
    drop(locked);
    Ok((frame, result))
}

#[cfg(all(target_os = "macos", feature = "native"))]
fn import_decoded_frame(
    backend: &mut SurfaceBackend,
    source: &Arc<valle_media::SourceFrame>,
    object: &crate::executor::skia::SkiaExternalObject,
) -> Result<ImportedMetalImage, SurfaceError> {
    use valle_engine::resource::ResourceInterpretation;
    use valle_media::{SharedVideoFrameHandle, YuvMatrix};

    let SurfaceBackend::Metal {
        context,
        texture_cache,
        ..
    } = backend
    else {
        return Err(SurfaceError::InvalidDecodedGpuFrame);
    };
    let decoded = source
        .decoded_gpu()
        .ok_or(SurfaceError::InvalidDecodedGpuFrame)?;
    let SharedVideoFrameHandle::VideoToolbox(pixel_buffer) = decoded.handle() else {
        return Err(SurfaceError::InvalidDecodedGpuFrame);
    };
    if pixel_buffer.is_null() {
        return Err(SurfaceError::InvalidDecodedGpuFrame);
    }
    let ResourceInterpretation::Visual { interpretation } = &object.key().interpretation else {
        return Err(SurfaceError::InvalidDecodedGpuFrame);
    };
    let image_color_space = input_color_space(interpretation.color)?;
    let pixel_format = unsafe { CVPixelBufferGetPixelFormatType(pixel_buffer) };
    let (display_width, display_height) = decoded.dimensions();
    let origin = match decoded.rotation_deg() {
        90 => EncodedOrigin::RightTop,
        180 => EncodedOrigin::BottomRight,
        270 => EncodedOrigin::LeftBottom,
        _ => EncodedOrigin::TopLeft,
    };
    let (image, textures) = match pixel_format {
        CV_PIXEL_FORMAT_BGRA if origin == EncodedOrigin::TopLeft => {
            let width = unsafe { CVPixelBufferGetWidth(pixel_buffer) };
            let height = unsafe { CVPixelBufferGetHeight(pixel_buffer) };
            let (texture, backend) = import_texture_plane(
                texture_cache,
                pixel_buffer,
                MTL_PIXEL_FORMAT_BGRA8_UNORM,
                width,
                height,
                0,
                "decoded-bgra",
            )?;
            let image = gpu::images::borrow_texture_from(
                context,
                &backend,
                gpu::SurfaceOrigin::TopLeft,
                ColorType::BGRA8888,
                match interpretation.alpha {
                    valle_engine::resource::InputAlphaMode::Opaque => AlphaType::Opaque,
                    valle_engine::resource::InputAlphaMode::StraightCoverage => AlphaType::Unpremul,
                    valle_engine::resource::InputAlphaMode::PremultipliedCoverage => {
                        AlphaType::Premul
                    }
                },
                Some(image_color_space),
            )
            .ok_or(SurfaceError::InvalidDecodedGpuFrame)?;
            (image, vec![texture])
        }
        CV_PIXEL_FORMAT_NV12_VIDEO
        | CV_PIXEL_FORMAT_NV12_FULL
        | CV_PIXEL_FORMAT_P010_VIDEO
        | CV_PIXEL_FORMAT_P010_FULL => {
            if unsafe { CVPixelBufferGetPlaneCount(pixel_buffer) } != 2 {
                return Err(SurfaceError::InvalidDecodedGpuFrame);
            }
            let y_width = unsafe { CVPixelBufferGetWidthOfPlane(pixel_buffer, 0) };
            let y_height = unsafe { CVPixelBufferGetHeightOfPlane(pixel_buffer, 0) };
            let uv_width = unsafe { CVPixelBufferGetWidthOfPlane(pixel_buffer, 1) };
            let uv_height = unsafe { CVPixelBufferGetHeightOfPlane(pixel_buffer, 1) };
            let ten_bit = matches!(
                pixel_format,
                CV_PIXEL_FORMAT_P010_VIDEO | CV_PIXEL_FORMAT_P010_FULL
            );
            let (y_texture, y_backend) = import_texture_plane(
                texture_cache,
                pixel_buffer,
                if ten_bit {
                    MTL_PIXEL_FORMAT_R16_UNORM
                } else {
                    MTL_PIXEL_FORMAT_R8_UNORM
                },
                y_width,
                y_height,
                0,
                "decoded-y",
            )?;
            let (uv_texture, uv_backend) = import_texture_plane(
                texture_cache,
                pixel_buffer,
                if ten_bit {
                    MTL_PIXEL_FORMAT_RG16_UNORM
                } else {
                    MTL_PIXEL_FORMAT_RG8_UNORM
                },
                uv_width,
                uv_height,
                1,
                "decoded-uv",
            )?;
            let yuv_color_space = if ten_bit {
                YUVColorSpace::Identity
            } else {
                match (decoded.matrix(), decoded.is_full_range()) {
                    (YuvMatrix::Bt601, true) => YUVColorSpace::JPEG_Full,
                    (YuvMatrix::Bt601, false) => YUVColorSpace::Rec601_Limited,
                    (YuvMatrix::Bt709, true) => YUVColorSpace::Rec709_Full,
                    (YuvMatrix::Bt709, false) => YUVColorSpace::Rec709_Limited,
                }
            };
            let info = YUVAInfo::new(
                (display_width as i32, display_height as i32),
                skia_safe::yuva_info::PlaneConfig::Y_UV,
                skia_safe::yuva_info::Subsampling::S420,
                yuv_color_space,
                origin,
                None,
            )
            .ok_or(SurfaceError::InvalidDecodedGpuFrame)?;
            let backend = gpu::YUVABackendTextures::new(
                &info,
                &[y_backend, uv_backend],
                gpu::SurfaceOrigin::TopLeft,
            )
            .ok_or(SurfaceError::InvalidDecodedGpuFrame)?;
            let image = if ten_bit {
                let planes = gpu::images::texture_from_yuva_textures(context, &backend, None)
                    .ok_or(SurfaceError::InvalidDecodedGpuFrame)?;
                super::decoded_yuv::p010_image(
                    context,
                    &planes,
                    image_color_space,
                    decoded.matrix(),
                    decoded.is_full_range(),
                )?
            } else {
                gpu::images::texture_from_yuva_textures(context, &backend, Some(image_color_space))
                    .ok_or(SurfaceError::InvalidDecodedGpuFrame)?
            };
            (image, vec![y_texture, uv_texture])
        }
        _ => return Err(SurfaceError::UnsupportedDecodedPixelFormat { pixel_format }),
    };
    if image.width() != display_width as i32 || image.height() != display_height as i32 {
        return Err(SurfaceError::InvalidDecodedGpuFrame);
    }
    Ok(ImportedMetalImage {
        image,
        _textures: textures,
        _source: ImportedMetalOwner::Video {
            _frame: Arc::clone(source),
        },
    })
}

#[cfg(all(target_os = "macos", feature = "native"))]
fn import_texture_plane(
    cache: &CoreFoundationHandle,
    pixel_buffer: *mut core::ffi::c_void,
    metal_format: usize,
    width: usize,
    height: usize,
    plane: usize,
    label: &str,
) -> Result<(CoreFoundationHandle, gpu::BackendTexture), SurfaceError> {
    let mut cv_texture = std::ptr::null_mut();
    let code = unsafe {
        CVMetalTextureCacheCreateTextureFromImage(
            std::ptr::null(),
            cache.0,
            pixel_buffer,
            std::ptr::null(),
            metal_format,
            width,
            height,
            plane,
            &mut cv_texture,
        )
    };
    if code != 0 || cv_texture.is_null() {
        return Err(SurfaceError::DecodedTextureImport { plane, code });
    }
    let texture = CoreFoundationHandle(cv_texture);
    let metal_texture = unsafe { CVMetalTextureGetTexture(cv_texture) };
    if metal_texture.is_null() {
        return Err(SurfaceError::InvalidDecodedGpuFrame);
    }
    let info = unsafe { gpu::mtl::TextureInfo::new(metal_texture as gpu::mtl::Handle) };
    let backend = unsafe {
        gpu::backend_textures::make_mtl(
            (width as i32, height as i32),
            gpu::Mipmapped::No,
            &info,
            label,
        )
    };
    Ok((texture, backend))
}

#[cfg(all(target_os = "macos", feature = "native"))]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: *const core::ffi::c_void);
}

#[cfg(all(target_os = "macos", feature = "native"))]
const MTL_PIXEL_FORMAT_BGRA8_UNORM: usize = 80;
#[cfg(all(target_os = "macos", feature = "native"))]
const MTL_PIXEL_FORMAT_R8_UNORM: usize = 10;
#[cfg(all(target_os = "macos", feature = "native"))]
const MTL_PIXEL_FORMAT_RG8_UNORM: usize = 30;
#[cfg(all(target_os = "macos", feature = "native"))]
const MTL_PIXEL_FORMAT_R16_UNORM: usize = 20;
#[cfg(all(target_os = "macos", feature = "native"))]
const MTL_PIXEL_FORMAT_RG16_UNORM: usize = 60;
#[cfg(all(target_os = "macos", feature = "native"))]
const CV_PIXEL_FORMAT_BGRA: u32 = u32::from_be_bytes(*b"BGRA");
#[cfg(all(target_os = "macos", feature = "native"))]
const CV_PIXEL_FORMAT_NV12_VIDEO: u32 = u32::from_be_bytes(*b"420v");
#[cfg(all(target_os = "macos", feature = "native"))]
const CV_PIXEL_FORMAT_NV12_FULL: u32 = u32::from_be_bytes(*b"420f");
#[cfg(all(target_os = "macos", feature = "native"))]
const CV_PIXEL_FORMAT_P010_VIDEO: u32 = u32::from_be_bytes(*b"x420");
#[cfg(all(target_os = "macos", feature = "native"))]
const CV_PIXEL_FORMAT_P010_FULL: u32 = u32::from_be_bytes(*b"xf20");

#[cfg(all(target_os = "macos", feature = "native"))]
struct CoreFoundationHandle(*mut core::ffi::c_void);

#[cfg(all(target_os = "macos", feature = "native"))]
impl Drop for CoreFoundationHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0.cast_const()) };
        }
    }
}

pub(crate) fn working_color_space() -> Result<ColorSpace, SurfaceError> {
    ColorSpace::new_cicp(
        named_primaries::CicpId::Rec2020,
        named_transfer_fn::CicpId::Linear,
    )
    .ok_or(SurfaceError::UnsupportedWorkingColorSpace)
}

pub(crate) fn working_info(extent: Extent2d) -> Result<ImageInfo, SurfaceError> {
    let width = i32::try_from(extent.width()).map_err(|_| SurfaceError::InvalidExtent)?;
    let height = i32::try_from(extent.height()).map_err(|_| SurfaceError::InvalidExtent)?;
    Ok(ImageInfo::new(
        (width, height),
        ColorType::RGBAF16,
        AlphaType::Premul,
        Some(working_color_space()?),
    ))
}

pub(crate) fn input_color_space(color: ColorDescription) -> Result<ColorSpace, SurfaceError> {
    let primaries = match color.primaries {
        ColorPrimaries::Rec709 => named_primaries::CicpId::Rec709,
        ColorPrimaries::DisplayP3 => named_primaries::CicpId::SMPTE_EG_432_1,
        ColorPrimaries::Rec2020 => named_primaries::CicpId::Rec2020,
    };
    let transfer = match color.transfer {
        TransferFunction::Linear => named_transfer_fn::CicpId::Linear,
        TransferFunction::Srgb => named_transfer_fn::CicpId::IEC61966_2_1,
        TransferFunction::Rec709 => named_transfer_fn::CicpId::Rec709,
        TransferFunction::Pq => named_transfer_fn::CicpId::PQ,
        TransferFunction::Hlg => named_transfer_fn::CicpId::HLG,
    };
    ColorSpace::new_cicp(primaries, transfer).ok_or(SurfaceError::UnsupportedInputColorSpace)
}

pub(crate) fn raster_surface(info: &ImageInfo) -> Result<Surface, SurfaceError> {
    let surface = surfaces::raster(info, None, None).ok_or(SurfaceError::Allocation)?;
    count_surface_allocation();
    Ok(surface)
}

fn count_surface_allocation() {
    FRAME_ALLOCATIONS.with(|counter| {
        if let Some(value) = counter.get() {
            counter.set(Some(value.saturating_add(1)));
        }
    });
}

thread_local! {
    static FRAME_ALLOCATIONS: Cell<Option<u64>> = const { Cell::new(None) };
}

/// Counts every explicit Skia surface allocation made on the current executor thread. Product
/// evidence can therefore distinguish planned pool growth from still-unpooled helper scratch.
pub(crate) struct SurfaceAllocationScope {
    active: bool,
}

impl SurfaceAllocationScope {
    pub(crate) fn begin() -> Result<Self, SurfaceError> {
        FRAME_ALLOCATIONS.with(|counter| {
            if counter.get().is_some() {
                return Err(SurfaceError::AllocationScopeAlreadyActive);
            }
            counter.set(Some(0));
            Ok(Self { active: true })
        })
    }

    pub(crate) fn finish(mut self) -> u64 {
        self.active = false;
        FRAME_ALLOCATIONS.with(|counter| counter.replace(None).unwrap_or(0))
    }
}

impl Drop for SurfaceAllocationScope {
    fn drop(&mut self) {
        if self.active {
            FRAME_ALLOCATIONS.with(|counter| counter.set(None));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SurfaceGeneration {
    extent: Extent2d,
    output: OutputSpec,
}

struct SurfaceEntry {
    _memory: Option<valle_engine::motion::memory::MemoryLease>,
    extent: Extent2d,
    surface: Surface,
    bytes: u64,
    reserved: bool,
}

// Admission answers whether one frame can execute; it is deliberately much larger than the
// amount of completed-frame storage worth retaining. Conflating those budgets lets animated ROI
// shapes pin gigabytes of otherwise idle Metal surfaces before the admission ceiling is reached.
const MAX_REUSABLE_SURFACE_POOL_BYTES: u64 = 128 * 1024 * 1024;
const MAX_REUSABLE_SURFACE_POOL_SURFACES: usize = 32;

impl core::fmt::Debug for SurfaceEntry {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SurfaceEntry")
            .field("extent", &self.extent)
            .field("bytes", &self.bytes)
            .field("reserved", &self.reserved)
            .finish()
    }
}

/// Long-lived CPU surface pool. A frame reserves one exact backing surface for every physical
/// plan slot; non-overlapping logical allocations therefore reuse the same pixels inside the
/// frame, while subsequent frames reuse the same backend allocation.
pub(crate) struct SurfaceArena {
    pool_byte_limit: u64,
    generation: Option<SurfaceGeneration>,
    generation_id: u64,
    invalidations: u64,
    reported_invalidations: u64,
    entries: Vec<SurfaceEntry>,
    scratch_free: Vec<SurfaceEntry>,
    delivery_surface: Option<SurfaceEntry>,
    output_staging: super::output::OutputStaging,
    #[cfg(all(target_os = "macos", feature = "native"))]
    pending_external_imports: Vec<ImportedMetalImage>,
    #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
    pending_scene3d_imports: Vec<(
        super::scene3d_gpu::GpuRasterOutput,
        super::SkiaExternalObject,
    )>,
    // Drop all retained surfaces before the context, device and Vulkan loader.
    backend: SurfaceBackend,
}

impl Drop for SurfaceArena {
    fn drop(&mut self) {
        // An error can abandon a frame before its normal output readback. Flush before
        // field destruction so borrowed wgpu textures cannot die under queued Skia reads.
        #[cfg(all(
            feature = "native",
            any(target_os = "macos", target_os = "linux", target_os = "windows")
        ))]
        if let Some(context) = self.backend.context_mut() {
            context.flush_submit_and_sync_cpu();
        }
    }
}

enum SurfaceBackend {
    Raster,
    #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
    Vulkan(Box<super::vulkan::VulkanContext>),
    #[cfg(all(target_os = "macos", feature = "native"))]
    Metal {
        context: gpu::DirectContext,
        texture_cache: CoreFoundationHandle,
        device: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLDevice>>,
        queue:
            objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLCommandQueue>>,
        scene3d_gpu: Option<Result<Box<super::scene3d_gpu::GpuRasterizer>, String>>,
    },
}

impl core::fmt::Debug for SurfaceBackend {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Raster => formatter.write_str("Raster"),
            #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
            Self::Vulkan(_) => formatter.write_str("Vulkan"),
            #[cfg(all(target_os = "macos", feature = "native"))]
            Self::Metal { .. } => formatter.write_str("Metal"),
        }
    }
}

impl SurfaceBackend {
    #[cfg(all(
        feature = "native",
        any(target_os = "macos", target_os = "linux", target_os = "windows")
    ))]
    fn context_mut(&mut self) -> Option<&mut gpu::DirectContext> {
        match self {
            Self::Raster => None,
            #[cfg(all(target_os = "macos", feature = "native"))]
            Self::Metal { context, .. } => Some(context),
            #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
            Self::Vulkan(backend) => Some(&mut backend.context),
        }
    }
}

impl core::fmt::Debug for SurfaceArena {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SurfaceArena")
            .field("backend", &self.backend)
            .field("generation", &self.generation)
            .field("generationId", &self.generation_id)
            .field("entries", &self.entries)
            .field("scratchFree", &self.scratch_free)
            .field("deliverySurface", &self.delivery_surface)
            .finish()
    }
}

impl SurfaceArena {
    pub(crate) fn set_gpu_resource_cache_limit(&mut self, bytes: usize) {
        #[cfg(all(target_os = "macos", feature = "native"))]
        if let SurfaceBackend::Metal {
            context,
            scene3d_gpu,
            ..
        } = &mut self.backend
        {
            // A soft target for live plus purgeable resources; referenced resources cannot
            // be purged immediately, so this is not a hard limit on driver allocation.
            context.set_resource_cache_limit(bytes);
            if let Some(Ok(renderer)) = scene3d_gpu {
                renderer.trim_assets(bytes);
            }
        }
        #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
        if let SurfaceBackend::Vulkan(backend) = &mut self.backend {
            backend.context.set_resource_cache_limit(bytes);
            if let Some(Ok(renderer)) = &mut backend.scene3d_gpu {
                renderer.trim_assets(bytes);
            }
        }
        let _ = bytes;
    }
    pub(crate) fn set_pool_byte_limit(&mut self, bytes: u64) {
        self.pool_byte_limit = bytes;
    }
    pub(crate) fn new() -> Self {
        Self {
            pool_byte_limit: MAX_REUSABLE_SURFACE_POOL_BYTES,
            backend: SurfaceBackend::Raster,
            generation: None,
            generation_id: 0,
            invalidations: 0,
            reported_invalidations: 0,
            entries: Vec::new(),
            scratch_free: Vec::new(),
            delivery_surface: None,
            output_staging: super::output::OutputStaging::default(),
            #[cfg(all(target_os = "macos", feature = "native"))]
            pending_external_imports: Vec::new(),
            #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
            pending_scene3d_imports: Vec::new(),
        }
    }

    pub(crate) const fn backend_kind(&self) -> SkiaBackendKind {
        match &self.backend {
            SurfaceBackend::Raster => SkiaBackendKind::Raster,
            #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
            SurfaceBackend::Vulkan(_) => SkiaBackendKind::Vulkan,
            #[cfg(all(target_os = "macos", feature = "native"))]
            SurfaceBackend::Metal { .. } => SkiaBackendKind::Metal,
        }
    }

    #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
    pub(crate) fn vulkan() -> Result<Self, SurfaceError> {
        let mut arena = Self::new();
        arena.backend = SurfaceBackend::Vulkan(Box::new(super::vulkan::VulkanContext::new()?));
        Ok(arena)
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn metal() -> Result<Self, SurfaceError> {
        use objc2_metal::MTLDevice;

        let started = std::env::var_os("VALLE_PERF").map(|_| std::time::Instant::now());
        let device = objc2_metal::MTLCreateSystemDefaultDevice()
            .ok_or(SurfaceError::MetalUnavailable("no Metal device"))?;
        let queue = device
            .newCommandQueue()
            .ok_or(SurfaceError::MetalUnavailable(
                "cannot create Metal command queue",
            ))?;
        let device_ptr = std::ptr::from_ref(&*device).cast::<core::ffi::c_void>();
        let queue_ptr = std::ptr::from_ref(&*queue).cast::<core::ffi::c_void>();
        // SAFETY: BackendContext retains both Objective-C objects while Ganesh establishes its
        // own references. The resulting DirectContext remains executor-owned.
        let backend = unsafe { gpu::mtl::BackendContext::new(device_ptr, queue_ptr) };
        let mut context = gpu::direct_contexts::make_metal(&backend, None).ok_or(
            SurfaceError::MetalUnavailable("cannot create Ganesh Metal context"),
        )?;
        context.set_resource_cache_limit(512 * 1024 * 1024);
        let mut texture_cache = std::ptr::null_mut();
        let code = unsafe {
            CVMetalTextureCacheCreate(
                std::ptr::null(),
                std::ptr::null(),
                device_ptr.cast_mut(),
                std::ptr::null(),
                &mut texture_cache,
            )
        };
        if code != 0 || texture_cache.is_null() {
            return Err(SurfaceError::MetalUnavailable(
                "cannot create CoreVideo Metal texture cache",
            ));
        }
        if let Some(started) = started {
            eprintln!(
                "[valle metal] initialize={:.3}ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        Ok(Self {
            pool_byte_limit: MAX_REUSABLE_SURFACE_POOL_BYTES,
            backend: SurfaceBackend::Metal {
                context,
                texture_cache: CoreFoundationHandle(texture_cache),
                device,
                queue,
                scene3d_gpu: None,
            },
            generation: None,
            generation_id: 0,
            invalidations: 0,
            reported_invalidations: 0,
            entries: Vec::new(),
            scratch_free: Vec::new(),
            delivery_surface: None,
            output_staging: super::output::OutputStaging::default(),
            #[cfg(feature = "native")]
            pending_external_imports: Vec::new(),
        })
    }

    pub(super) fn create_surface(&mut self, info: &ImageInfo) -> Result<Surface, SurfaceError> {
        #[cfg(all(
            feature = "native",
            any(target_os = "macos", target_os = "linux", target_os = "windows")
        ))]
        if let Some(context) = self.backend.context_mut() {
            let surface = gpu::surfaces::render_target(
                context,
                gpu::Budgeted::Yes,
                info,
                None::<usize>,
                Some(gpu::SurfaceOrigin::TopLeft),
                None,
                Some(false),
                Some(false),
            )
            .ok_or(SurfaceError::Allocation)?;
            count_surface_allocation();
            return Ok(surface);
        }
        raster_surface(info)
    }

    #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
    pub(crate) fn vulkan_context_mut(&mut self) -> Option<&mut super::vulkan::VulkanContext> {
        match &mut self.backend {
            SurfaceBackend::Vulkan(context) => Some(context),
            _ => None,
        }
    }

    #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
    pub(crate) fn submit_vulkan_output(
        &mut self,
        output: super::vulkan_delivery::VulkanOutput,
    ) -> Result<super::vulkan_delivery::SubmittedVulkanOutput, SurfaceError> {
        let SurfaceBackend::Vulkan(context) = &mut self.backend else {
            return Err(SurfaceError::VulkanUnavailable(
                "output requires Vulkan".into(),
            ));
        };
        output.submit(
            &mut context.context,
            std::mem::take(&mut self.pending_scene3d_imports),
        )
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn acquire_shared_frame(
        &mut self,
        pool: &SharedVideoFramePool,
        output: OutputSpec,
    ) -> Result<SharedMetalFrame, SurfaceError> {
        let SurfaceBackend::Metal {
            context,
            texture_cache,
            ..
        } = &mut self.backend
        else {
            return Err(SurfaceError::SharedFrameRequiresMetal);
        };
        let frame = pool
            .acquire()
            .map_err(|error| SurfaceError::SharedFrame(error.to_string()))?;
        let (width, height) = frame.dimensions();
        let SharedVideoFrameHandle::VideoToolbox(pixel_buffer) = frame.handle() else {
            return Err(SurfaceError::SharedFrame(
                "Metal shared target requires a VideoToolbox frame".into(),
            ));
        };
        let mut cv_texture = std::ptr::null_mut();
        let code = unsafe {
            CVMetalTextureCacheCreateTextureFromImage(
                std::ptr::null(),
                texture_cache.0,
                pixel_buffer,
                std::ptr::null(),
                MTL_PIXEL_FORMAT_BGRA8_UNORM,
                width as usize,
                height as usize,
                0,
                &mut cv_texture,
            )
        };
        if code != 0 || cv_texture.is_null() {
            return Err(SurfaceError::SharedFrame(format!(
                "CoreVideo Metal import failed with CVReturn {code}"
            )));
        }
        let texture = CoreFoundationHandle(cv_texture);
        let metal_texture = unsafe { CVMetalTextureGetTexture(cv_texture) };
        if metal_texture.is_null() {
            return Err(SurfaceError::SharedFrame(
                "CVMetalTexture has no MTLTexture".into(),
            ));
        }
        let texture_info = unsafe { gpu::mtl::TextureInfo::new(metal_texture as gpu::mtl::Handle) };
        let target =
            gpu::backend_render_targets::make_mtl((width as i32, height as i32), &texture_info);
        let color_space = super::output::output_color_space(output)
            .map_err(|error| SurfaceError::SharedFrame(error.to_string()))?;
        let mut surface = gpu::surfaces::wrap_backend_render_target(
            context,
            &target,
            gpu::SurfaceOrigin::TopLeft,
            ColorType::BGRA8888,
            Some(color_space),
            None,
        )
        .ok_or_else(|| {
            SurfaceError::SharedFrame("Skia cannot wrap the VideoToolbox Metal texture".into())
        })?;
        surface.canvas().clear(skia_safe::Color::BLACK);
        Ok(SharedMetalFrame {
            surface: Some(surface),
            texture,
            frame,
        })
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn submit_shared_frame(
        &mut self,
        mut target: SharedMetalFrame,
    ) -> Result<SubmittedSharedMetalFrame, SurfaceError> {
        let SurfaceBackend::Metal { context, .. } = &mut self.backend else {
            return Err(SurfaceError::SharedFrameRequiresMetal);
        };
        let completion = GpuCompletion::new();
        let callback_context = completion.callback_context();
        let Some(surface) = target.surface.as_mut() else {
            unsafe { gpu_finished(callback_context) };
            return Err(SurfaceError::SharedFrameSubmitted);
        };
        let raw_info = skia_bindings::GrFlushInfo {
            fNumSemaphores: 0,
            fGpuStatsFlags: skia_bindings::skgpu_GpuStatsFlags_kNone,
            fSignalSemaphores: std::ptr::null_mut(),
            fFinishedProc: Some(gpu_finished),
            fFinishedWithStatsProc: None,
            fFinishedContext: callback_context,
            fSubmittedProc: None,
            fSubmittedContext: std::ptr::null_mut(),
        };
        const {
            assert!(
                std::mem::size_of::<gpu::FlushInfo>()
                    == std::mem::size_of::<skia_bindings::GrFlushInfo>()
            );
            assert!(
                std::mem::align_of::<gpu::FlushInfo>()
                    == std::mem::align_of::<skia_bindings::GrFlushInfo>()
            );
        }
        let info =
            unsafe { std::mem::transmute::<skia_bindings::GrFlushInfo, gpu::FlushInfo>(raw_info) };
        context.flush_surface_with_access(surface, surfaces::BackendSurfaceAccess::NoAccess, &info);
        if !context.submit(gpu::SyncCpu::No) {
            return Err(SurfaceError::SharedFrame(
                "Metal command-buffer submission failed".into(),
            ));
        }
        if std::env::var_os("VALLE_TRACE_F16_STAGES").is_some() {
            let usage = context.resource_cache_usage();
            eprintln!(
                "[valle ganesh] cache-bytes={} purgeable-bytes={} limit-bytes={}",
                usage.resource_bytes,
                context.resource_cache_purgeable_bytes(),
                context.resource_cache_limit(),
            );
        }
        Ok(SubmittedSharedMetalFrame {
            surface: target.surface,
            texture: target.texture,
            frame: target.frame,
            completion,
            #[cfg(feature = "native")]
            external_imports: std::mem::take(&mut self.pending_external_imports),
        })
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn finish_shared_frame(
        &mut self,
        mut submitted: SubmittedSharedMetalFrame,
    ) -> Result<SharedVideoFrame, SurfaceError> {
        let SurfaceBackend::Metal { context, .. } = &mut self.backend else {
            return Err(SurfaceError::SharedFrameRequiresMetal);
        };
        let mut spins = 0_usize;
        while !submitted.completion.is_ready() {
            context.check_async_work_completion();
            if submitted.completion.is_ready() {
                break;
            }
            if spins < 8 {
                std::hint::spin_loop();
            } else {
                // Ganesh retires Metal buffers and invokes the finished callback
                // in check_async_work_completion, so this thread must keep polling.
                // Yielding continuously burns a CPU core during a GPU backlog.
                // Brief bounded sleeps retain prompt delivery without busy waiting.
                let micros = 50_u64 << (spins - 8).min(2);
                std::thread::sleep(std::time::Duration::from_micros(micros));
            }
            spins = spins.saturating_add(1);
        }
        drop(submitted.surface.take());
        drop(submitted.texture);
        #[cfg(feature = "native")]
        drop(submitted.external_imports);
        Ok(submitted.frame)
    }

    pub(crate) fn invalidate(&mut self) {
        #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
        if !self.pending_scene3d_imports.is_empty() {
            if let Some(context) = self.backend.context_mut() {
                context.flush_submit_and_sync_cpu();
            }
            self.pending_scene3d_imports.clear();
        }
        #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
        if let SurfaceBackend::Vulkan(backend) = &mut self.backend {
            if let Some(Ok(renderer)) = &mut backend.scene3d_gpu {
                renderer.trim_assets(0);
            }
        }
        if self.generation.take().is_some()
            || !self.entries.is_empty()
            || !self.scratch_free.is_empty()
            || self.delivery_surface.is_some()
        {
            self.invalidations = self.invalidations.saturating_add(1);
        }
        self.entries.clear();
        self.scratch_free.clear();
        self.delivery_surface = None;
        self.output_staging.clear_cache();
        #[cfg(all(target_os = "macos", feature = "native"))]
        if let SurfaceBackend::Metal {
            scene3d_gpu: Some(Ok(renderer)),
            ..
        } = &mut self.backend
        {
            renderer.trim_assets(0);
        }
        #[cfg(all(target_os = "macos", feature = "native"))]
        self.pending_external_imports.clear();
        self.generation_id = self.generation_id.saturating_add(1).max(1);
    }

    pub(crate) fn begin_frame<'a>(
        &'a mut self,
        template: &RenderPlanTemplate,
        schedules: &BoundProgramSchedules,
        max_surface_bytes: u64,
        max_frame_bytes: u64,
    ) -> Result<SurfaceFrame<'a>, SurfaceError> {
        if self.entries.iter().any(|entry| entry.reserved) {
            return Err(SurfaceError::FrameAlreadyActive);
        }
        let extent = Extent2d::new(
            template.render_spec().width(),
            template.render_spec().height(),
        )
        .map_err(|_| SurfaceError::InvalidExtent)?;
        let generation = SurfaceGeneration {
            extent,
            output: template.render_spec().output(),
        };
        if self.generation.is_some_and(|current| current != generation) {
            self.invalidate();
        }
        if self.generation.is_none() {
            self.generation = Some(generation);
        }
        if self.generation_id == 0 {
            self.generation_id = 1;
        }

        // `entries` is the physical assignment set for exactly one frame. A bound ROI may change
        // shape on every sample (scrolling text is a common case), so retaining every prior
        // assignment here would keep a strong reference to one backend surface per historical
        // shape. Retire the completed frame into the byte-bounded free pool before selecting the
        // next assignment set; exact shapes are reused below and stale shapes become evictable.
        self.scratch_free.append(&mut self.entries);

        let mut assignments = BTreeMap::new();
        let mut selected = BTreeSet::new();
        let mut allocations = 0_u64;
        let mut reuses = 0_u64;
        let mut evictions = 0_u64;
        let mut physical_bytes = 0_u64;
        let mut logical_bytes = 0_u64;
        let required_physical_bytes = schedules
            .surface_slots()
            .iter()
            .try_fold(0_u64, |bytes, slot| {
                bytes.checked_add(slot.estimated_bytes())
            })
            .ok_or(SurfaceError::ByteOverflow)?;
        if required_physical_bytes > max_frame_bytes {
            return Err(SurfaceError::FrameBudgetExceeded {
                required_bytes: required_physical_bytes,
                max_bytes: max_frame_bytes,
            });
        }
        for slot in schedules.surface_slots() {
            let template_slot = template
                .surface_slots()
                .get(slot.id().get() as usize - 1)
                .filter(|candidate| candidate.id == slot.id())
                .ok_or(SurfaceError::MissingBoundSlot {
                    slot: slot.id().get(),
                })?;
            validate_slot(template_slot, slot)?;
            let Some(bound_extent) = slot.extent() else {
                continue;
            };
            if slot.estimated_bytes() > max_surface_bytes {
                return Err(SurfaceError::SurfaceBudgetExceeded {
                    required_bytes: slot.estimated_bytes(),
                    max_bytes: max_surface_bytes,
                });
            }
            let index = if let Some(index) = self
                .entries
                .iter()
                .enumerate()
                .find(|(index, entry)| !selected.contains(index) && entry.extent == bound_extent)
                .map(|(index, _)| index)
            {
                reuses = reuses.saturating_add(1);
                index
            } else if let Some(index) = self
                .scratch_free
                .iter()
                .rposition(|entry| entry.extent == bound_extent)
            {
                let entry = self.scratch_free.remove(index);
                let index = self.entries.len();
                self.entries.push(entry);
                reuses = reuses.saturating_add(1);
                index
            } else {
                evictions = evictions.saturating_add(
                    self.evict_scratch_until_fits(slot.estimated_bytes(), max_frame_bytes)?,
                );
                let info = working_info(bound_extent)?;
                let memory = valle_engine::motion::memory::reserve(
                    valle_engine::motion::memory::MemoryKind::Surface,
                    slot.estimated_bytes(),
                )?;
                let surface = self.create_surface(&info)?;
                let index = self.entries.len();
                self.entries.push(SurfaceEntry {
                    _memory: memory,
                    extent: bound_extent,
                    surface,
                    bytes: slot.estimated_bytes(),
                    reserved: false,
                });
                allocations = allocations.saturating_add(1);
                index
            };
            selected.insert(index);
            physical_bytes = physical_bytes
                .checked_add(self.entries[index].bytes)
                .ok_or(SurfaceError::ByteOverflow)?;
            logical_bytes =
                slot.allocations()
                    .iter()
                    .try_fold(logical_bytes, |bytes, allocation| {
                        bytes
                            .checked_add(surface_bytes_for_rect(allocation.device_roi())?)
                            .ok_or(SurfaceError::ByteOverflow)
                    })?;
            if assignments.insert(slot.id(), index).is_some() {
                return Err(SurfaceError::DuplicateSlot {
                    slot: slot.id().get(),
                });
            }
        }

        evictions = evictions.saturating_add(self.trim_reusable_pool(max_frame_bytes));

        let pool_bytes = self
            .entries
            .iter()
            .chain(self.scratch_free.iter())
            .chain(self.delivery_surface.iter())
            .try_fold(0_u64, |bytes, entry| bytes.checked_add(entry.bytes))
            .ok_or(SurfaceError::ByteOverflow)?;
        let generation_invalidations = self
            .invalidations
            .saturating_sub(self.reported_invalidations);
        self.reported_invalidations = self.invalidations;
        for index in assignments.values().copied() {
            self.entries[index].reserved = true;
        }
        let report = SurfaceFrameReport {
            generation: self.generation_id,
            generation_invalidations,
            plan_slots: assignments.len(),
            logical_allocations: template
                .surface_slots()
                .iter()
                .map(|slot| slot.allocations.len())
                .sum(),
            allocations,
            reuses,
            physical_bytes,
            logical_bytes,
            alias_saved_bytes: logical_bytes.saturating_sub(physical_bytes),
            estimated_peak_bytes: schedules.outer_peak_surface_bytes(),
            pool_surfaces: self
                .entries
                .len()
                .saturating_add(self.scratch_free.len())
                .saturating_add(usize::from(self.delivery_surface.is_some())),
            pool_bytes,
            scratch_allocations: 0,
            scratch_reuses: 0,
            scratch_peak_surfaces: 0,
            scratch_peak_bytes: 0,
            scratch_pool_surfaces: 0,
            scratch_pool_bytes: 0,
            pool_evictions: evictions,
        };
        Ok(SurfaceFrame {
            arena: self,
            assignments,
            report,
            scratch_allocations: 0,
            scratch_reuses: 0,
            scratch_checked_out_surfaces: 0,
            scratch_checked_out_bytes: 0,
            scratch_peak_surfaces: 0,
            scratch_peak_bytes: 0,
            max_surface_bytes,
            max_frame_bytes,
            pool_evictions: evictions,
        })
    }

    fn evict_scratch_until_fits(
        &mut self,
        additional_bytes: u64,
        max_frame_bytes: u64,
    ) -> Result<u64, SurfaceError> {
        let mut evictions = 0_u64;
        while self
            .pool_bytes()?
            .checked_add(additional_bytes)
            .ok_or(SurfaceError::ByteOverflow)?
            > max_frame_bytes
        {
            if self.scratch_free.is_empty() {
                if self.delivery_surface.take().is_some() {
                    evictions = evictions.saturating_add(1);
                    continue;
                }
                return Err(SurfaceError::FrameBudgetExceeded {
                    required_bytes: self
                        .pool_bytes()?
                        .checked_add(additional_bytes)
                        .ok_or(SurfaceError::ByteOverflow)?,
                    max_bytes: max_frame_bytes,
                });
            }
            self.scratch_free.remove(0);
            evictions = evictions.saturating_add(1);
        }
        Ok(evictions)
    }

    fn trim_reusable_pool(&mut self, max_frame_bytes: u64) -> u64 {
        let max_bytes = max_frame_bytes.min(self.pool_byte_limit);
        let mut bytes = self
            .scratch_free
            .iter()
            .fold(0_u64, |total, entry| total.saturating_add(entry.bytes))
            .saturating_add(
                self.delivery_surface
                    .as_ref()
                    .map_or(0, |entry| entry.bytes),
            );
        let mut surfaces = self
            .scratch_free
            .len()
            .saturating_add(usize::from(self.delivery_surface.is_some()));
        let mut evictions = 0_u64;
        while bytes > max_bytes || surfaces > MAX_REUSABLE_SURFACE_POOL_SURFACES {
            if !self.scratch_free.is_empty() {
                let entry = self.scratch_free.remove(0);
                bytes = bytes.saturating_sub(entry.bytes);
                surfaces = surfaces.saturating_sub(1);
                evictions = evictions.saturating_add(1);
                continue;
            }
            if let Some(entry) = self.delivery_surface.take() {
                bytes = bytes.saturating_sub(entry.bytes);
                surfaces = surfaces.saturating_sub(1);
                evictions = evictions.saturating_add(1);
                continue;
            }
            break;
        }
        evictions
    }

    fn pool_bytes(&self) -> Result<u64, SurfaceError> {
        self.entries
            .iter()
            .chain(&self.scratch_free)
            .chain(self.delivery_surface.iter())
            .try_fold(0_u64, |bytes, entry| bytes.checked_add(entry.bytes))
            .ok_or(SurfaceError::ByteOverflow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SurfaceFrameReport {
    pub(crate) generation: u64,
    pub(crate) generation_invalidations: u64,
    pub(crate) plan_slots: usize,
    pub(crate) logical_allocations: usize,
    pub(crate) allocations: u64,
    pub(crate) reuses: u64,
    pub(crate) physical_bytes: u64,
    pub(crate) logical_bytes: u64,
    pub(crate) alias_saved_bytes: u64,
    pub(crate) estimated_peak_bytes: u64,
    pub(crate) pool_surfaces: usize,
    pub(crate) pool_bytes: u64,
    pub(crate) scratch_allocations: u64,
    pub(crate) scratch_reuses: u64,
    pub(crate) scratch_peak_surfaces: usize,
    pub(crate) scratch_peak_bytes: u64,
    pub(crate) scratch_pool_surfaces: usize,
    pub(crate) scratch_pool_bytes: u64,
    pub(crate) pool_evictions: u64,
}

pub(crate) struct SurfaceFrame<'a> {
    arena: &'a mut SurfaceArena,
    assignments: BTreeMap<SurfaceSlotId, usize>,
    report: SurfaceFrameReport,
    scratch_allocations: u64,
    scratch_reuses: u64,
    scratch_checked_out_surfaces: usize,
    scratch_checked_out_bytes: u64,
    scratch_peak_surfaces: usize,
    scratch_peak_bytes: u64,
    max_surface_bytes: u64,
    max_frame_bytes: u64,
    pool_evictions: u64,
}

impl<'arena> SurfaceFrame<'arena> {
    pub(crate) const fn backend_kind(&self) -> SkiaBackendKind {
        self.arena.backend_kind()
    }

    #[cfg(all(
        feature = "native",
        any(target_os = "macos", target_os = "linux", target_os = "windows")
    ))]
    pub(crate) fn render_scene3d_gpu(
        &mut self,
        object: &super::SkiaExternalObject,
    ) -> Result<Option<(Image, u64)>, SurfaceError> {
        let input = object
            .deferred_scene3d_gpu()
            .ok_or_else(|| SurfaceError::Scene3dGpu("missing deferred geometry".into()))?;
        let extent = Extent2d::new(input.geometry.width, input.geometry.height)
            .map_err(|_| SurfaceError::InvalidExtent)?;
        let bytes = working_info(extent)?.compute_min_byte_size() as u64;
        if bytes > self.max_surface_bytes {
            return Err(SurfaceError::SurfaceBudgetExceeded {
                required_bytes: bytes,
                max_bytes: self.max_surface_bytes,
            });
        }
        // RGBA16F + Depth32Float stay alive until the frame completion fence. Deduct
        // them from the remaining surface allowance before allocating either texture.
        let dof = input
            .frame
            .camera
            .depth_of_field
            .filter(|d| d.max_blur_radius > 0.0);
        let gpu_bytes = input
            .texture_bytes()
            .checked_add(input.upload_bytes().ok_or(SurfaceError::ByteOverflow)?)
            .ok_or(SurfaceError::ByteOverflow)?;
        let required_bytes = self
            .report
            .physical_bytes
            .checked_add(self.scratch_checked_out_bytes)
            .and_then(|value| value.checked_add(bytes * if dof.is_some() { 2 } else { 1 }))
            .and_then(|value| value.checked_add(gpu_bytes))
            .ok_or(SurfaceError::ByteOverflow)?;
        let output = match &mut self.arena.backend {
            SurfaceBackend::Raster => return Ok(None),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            SurfaceBackend::Vulkan(backend) => {
                if backend.scene3d_gpu.is_none() {
                    let Some((device, queue)) = backend.shared_device() else {
                        return Ok(None);
                    };
                    backend.scene3d_gpu =
                        Some(super::scene3d_gpu::GpuRasterizer::new(device, queue).map(Box::new));
                }
                let Some(Ok(renderer)) = &mut backend.scene3d_gpu else {
                    return Ok(None);
                };
                if required_bytes > self.max_frame_bytes {
                    return Err(SurfaceError::FrameBudgetExceeded {
                        required_bytes,
                        max_bytes: self.max_frame_bytes,
                    });
                }
                let Some(output) = renderer
                    .render(input, &mut backend.context)
                    .map_err(SurfaceError::Scene3dGpu)?
                else {
                    return Ok(None);
                };
                self.max_frame_bytes -= gpu_bytes;
                output
            }
            #[cfg(target_os = "macos")]
            SurfaceBackend::Metal {
                context,
                device,
                queue,
                scene3d_gpu,
                ..
            } => {
                if scene3d_gpu.is_none() {
                    let renderer = super::scene3d_gpu::metal::device(device.clone(), queue.clone())
                        .and_then(|(device, queue)| {
                            super::scene3d_gpu::GpuRasterizer::new(device, queue)
                        })
                        .map(Box::new);
                    if let Err(reason) = &renderer {
                        eprintln!("warning: Scene3D GPU raster unavailable, using CPU: {reason}");
                    }
                    *scene3d_gpu = Some(renderer);
                }
                let Some(Ok(renderer)) = scene3d_gpu.as_mut() else {
                    return Ok(None);
                };
                if required_bytes > self.max_frame_bytes {
                    return Err(SurfaceError::FrameBudgetExceeded {
                        required_bytes,
                        max_bytes: self.max_frame_bytes,
                    });
                }
                let Some(output) = renderer
                    .render(input, context)
                    .map_err(SurfaceError::Scene3dGpu)?
                else {
                    return Ok(None);
                };
                self.max_frame_bytes -= gpu_bytes;
                output
            }
        };
        // Keep the cached image Skia-owned. The only copy is GPU -> GPU, into a frame-budgeted
        // scratch surface; borrowed wgpu textures never escape into persistent program caches.
        let upload_bytes = output.upload_bytes;
        let color = output.image.clone();
        let depth = output.depth_image.clone();
        #[cfg(target_os = "macos")]
        self.arena
            .pending_external_imports
            .push(ImportedMetalImage {
                image: color.clone(),
                _textures: Vec::new(),
                _source: ImportedMetalOwner::Scene {
                    _resources: Box::new((output, object.clone())),
                },
            });
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        self.arena
            .pending_scene3d_imports
            .push((output, object.clone()));
        let image = if let Some(dof) = dof {
            let input = super::scene3d_dof::Scene3dDofInput::from_gpu(
                extent,
                color,
                depth.ok_or_else(|| SurfaceError::Scene3dGpu("missing GPU depth".into()))?,
                dof,
                input.frame.camera.far,
            );
            super::scene3d_dof::render(&input, self)
                .map_err(|e| SurfaceError::Scene3dGpu(e.to_string()))?
        } else {
            let mut scratch = self.scratch(&[extent])?;
            let target = scratch.surface_mut(0)?;
            let mut paint = Paint::default();
            paint.set_blend_mode(BlendMode::Src);
            target.canvas().draw_image(&color, (0, 0), Some(&paint));
            target.image_snapshot()
        };
        Ok(Some((image, upload_bytes)))
    }

    /// CPU output staging buffers retained by the executor across frames.
    pub(crate) fn output_staging_mut(&mut self) -> &mut super::output::OutputStaging {
        &mut self.arena.output_staging
    }

    #[cfg(feature = "native")]
    pub(crate) fn import_decoded_video(
        &mut self,
        object: &crate::executor::skia::SkiaExternalObject,
    ) -> Result<Image, SurfaceError> {
        let source = object
            .decoded_video()
            .ok_or(SurfaceError::InvalidDecodedGpuFrame)?;
        #[cfg(all(target_os = "macos", feature = "native"))]
        {
            let imported = import_decoded_frame(&mut self.arena.backend, source, object)?;
            let image = imported.image.clone();
            self.arena.pending_external_imports.push(imported);
            Ok(image)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = source;
            Err(SurfaceError::InvalidDecodedGpuFrame)
        }
    }

    #[cfg(all(
        feature = "native",
        any(target_os = "macos", target_os = "linux", target_os = "windows")
    ))]
    pub(crate) fn release_completed_external_imports(&mut self) {
        #[cfg(target_os = "macos")]
        self.arena.pending_external_imports.clear();
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        self.arena.pending_scene3d_imports.clear();
    }

    pub(crate) fn prepare_cpu_image(&mut self, image: &Image) -> Result<Image, SurfaceError> {
        #[cfg(all(
            feature = "native",
            any(target_os = "macos", target_os = "linux", target_os = "windows")
        ))]
        if let Some(context) = self.arena.backend.context_mut() {
            context.flush_submit_and_sync_cpu();
            return image
                .make_raster_image(Some(context), skia_safe::image::CachingHint::Disallow)
                .ok_or(SurfaceError::Readback);
        }
        Ok(image.clone())
    }

    #[cfg(all(
        feature = "native",
        any(target_os = "macos", target_os = "linux", target_os = "windows")
    ))]
    pub(crate) fn render_output_shader_to_raster(
        &mut self,
        shader: Shader,
        info: &ImageInfo,
    ) -> Result<Image, SurfaceError> {
        if self.arena.backend.context_mut().is_none() {
            return Err(SurfaceError::Readback);
        }
        let width = u32::try_from(info.width()).map_err(|_| SurfaceError::InvalidExtent)?;
        let height = u32::try_from(info.height()).map_err(|_| SurfaceError::InvalidExtent)?;
        let extent = Extent2d::new(width, height).map_err(|_| SurfaceError::InvalidExtent)?;
        let bytes =
            u64::try_from(info.compute_min_byte_size()).map_err(|_| SurfaceError::ByteOverflow)?;
        if bytes > self.max_surface_bytes {
            return Err(SurfaceError::SurfaceBudgetExceeded {
                required_bytes: bytes,
                max_bytes: self.max_surface_bytes,
            });
        }

        let mut entry = match self.arena.delivery_surface.take() {
            Some(entry) if entry.extent == extent && entry.surface.image_info() == *info => {
                self.scratch_reuses = self.scratch_reuses.saturating_add(1);
                entry
            }
            Some(_) => {
                self.pool_evictions = self.pool_evictions.saturating_add(1);
                self.allocate_delivery_surface(extent, bytes, info)?
            }
            None => self.allocate_delivery_surface(extent, bytes, info)?,
        };
        self.scratch_checked_out_surfaces = self
            .scratch_checked_out_surfaces
            .checked_add(1)
            .ok_or(SurfaceError::ByteOverflow)?;
        self.scratch_checked_out_bytes = self
            .scratch_checked_out_bytes
            .checked_add(bytes)
            .ok_or(SurfaceError::ByteOverflow)?;
        self.scratch_peak_surfaces = self
            .scratch_peak_surfaces
            .max(self.scratch_checked_out_surfaces);
        self.scratch_peak_bytes = self.scratch_peak_bytes.max(self.scratch_checked_out_bytes);

        let result = (|| {
            entry
                .surface
                .canvas()
                .clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
            let mut paint = Paint::default();
            paint.set_blend_mode(BlendMode::Src);
            paint.set_shader(shader);
            entry.surface.canvas().draw_paint(&paint);
            let image = entry.surface.image_snapshot();
            let context = self
                .arena
                .backend
                .context_mut()
                .ok_or(SurfaceError::Readback)?;
            context.flush_submit_and_sync_cpu();
            image
                .make_raster_image(Some(context), skia_safe::image::CachingHint::Disallow)
                .ok_or(SurfaceError::Readback)
        })();
        self.scratch_checked_out_surfaces = self.scratch_checked_out_surfaces.saturating_sub(1);
        self.scratch_checked_out_bytes = self.scratch_checked_out_bytes.saturating_sub(bytes);
        self.arena.delivery_surface = Some(entry);
        self.pool_evictions = self
            .pool_evictions
            .saturating_add(self.arena.trim_reusable_pool(self.max_frame_bytes));
        result
    }

    #[cfg(all(
        feature = "native",
        any(target_os = "macos", target_os = "linux", target_os = "windows")
    ))]
    fn allocate_delivery_surface(
        &mut self,
        extent: Extent2d,
        bytes: u64,
        info: &ImageInfo,
    ) -> Result<SurfaceEntry, SurfaceError> {
        let evictions = self
            .arena
            .evict_scratch_until_fits(bytes, self.max_frame_bytes)?;
        self.pool_evictions = self.pool_evictions.saturating_add(evictions);
        self.scratch_allocations = self.scratch_allocations.saturating_add(1);
        Ok(SurfaceEntry {
            _memory: valle_engine::motion::memory::reserve(
                valle_engine::motion::memory::MemoryKind::Delivery,
                bytes,
            )?,
            extent,
            surface: self.arena.create_surface(info)?,
            bytes,
            reserved: false,
        })
    }

    pub(crate) fn surface_mut(
        &mut self,
        slot: SurfaceSlotId,
    ) -> Result<&mut Surface, SurfaceError> {
        let index = *self
            .assignments
            .get(&slot)
            .ok_or(SurfaceError::MissingSlot { slot: slot.get() })?;
        Ok(&mut self.arena.entries[index].surface)
    }

    /// Exchanges the Raster surfaces behind two slots for the rest of the frame when their
    /// extents match, so a pass may draw over a dying input's pixels and leave its output in its
    /// own slot. Returns whether the surfaces were exchanged.
    pub(crate) fn swap_raster_slots(
        &mut self,
        first: SurfaceSlotId,
        second: SurfaceSlotId,
    ) -> Result<bool, SurfaceError> {
        let first_index = *self
            .assignments
            .get(&first)
            .ok_or(SurfaceError::MissingSlot { slot: first.get() })?;
        let second_index = *self
            .assignments
            .get(&second)
            .ok_or(SurfaceError::MissingSlot { slot: second.get() })?;
        if !matches!(self.arena.backend, SurfaceBackend::Raster)
            || first_index == second_index
            || self.arena.entries[first_index].extent != self.arena.entries[second_index].extent
        {
            return Ok(false);
        }
        self.assignments.insert(first, second_index);
        self.assignments.insert(second, first_index);
        Ok(true)
    }

    pub(crate) fn snapshot(
        &mut self,
        slot: SurfaceSlotId,
        roi: valle_engine::prepare::DeviceRect,
    ) -> Result<PlanImage, SurfaceError> {
        if roi.is_empty() {
            return Ok(PlanImage::transparent());
        }
        let surface = self.surface_mut(slot)?;
        let width = i32::try_from(roi.width).map_err(|_| SurfaceError::InvalidExtent)?;
        let height = i32::try_from(roi.height).map_err(|_| SurfaceError::InvalidExtent)?;
        if width > surface.width() || height > surface.height() {
            return Err(SurfaceError::ImageExtentMismatch { slot: slot.get() });
        }
        let bounds = IRect::from_xywh(0, 0, width, height);
        let image = surface
            .image_snapshot_with_bounds(bounds)
            .ok_or(SurfaceError::Allocation)?;
        Ok(PlanImage::new(image, roi))
    }

    /// Program image copies use strict logical ROIs. Bucket their backing capacity so
    /// small animated bounds changes can reuse a surface; image kernels keep exact extents.
    pub(crate) fn scratch_program<'frame>(
        &'frame mut self,
        extents: &[Extent2d],
    ) -> Result<ScratchSurfaces<'frame, 'arena>, SurfaceError> {
        // GPU image sampling retains exact backing extents; this reuse policy targets
        // CPU allocation churn and its strict source-rectangle copies.
        if !matches!(&self.arena.backend, SurfaceBackend::Raster) {
            return self.scratch(extents);
        }
        let mut capacity = Vec::with_capacity(extents.len());
        let mut bytes = self
            .report
            .physical_bytes
            .checked_add(self.scratch_checked_out_bytes)
            .ok_or(SurfaceError::ByteOverflow)?;
        for extent in extents {
            let width = extent.width().checked_add(31).map(|value| value / 32 * 32);
            let height = extent.height().checked_add(31).map(|value| value / 32 * 32);
            let Some((width, height)) = width.zip(height) else {
                return self.scratch(extents);
            };
            let rounded = Extent2d::new(width, height).map_err(|_| SurfaceError::InvalidExtent)?;
            let size = surface_bytes(rounded)?;
            let Some(total) = bytes.checked_add(size) else {
                return self.scratch(extents);
            };
            if size > self.max_surface_bytes || total > self.max_frame_bytes {
                return self.scratch(extents);
            }
            bytes = total;
            capacity.push(rounded);
        }
        // scratch accounts allocated capacity bytes and retains the existing bounded pool.
        self.scratch(&capacity)
    }

    pub(crate) fn scratch<'frame>(
        &'frame mut self,
        extents: &[Extent2d],
    ) -> Result<ScratchSurfaces<'frame, 'arena>, SurfaceError> {
        let requested = extents
            .iter()
            .map(|extent| surface_bytes(*extent))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(required_bytes) = requested
            .iter()
            .copied()
            .find(|bytes| *bytes > self.max_surface_bytes)
        {
            return Err(SurfaceError::SurfaceBudgetExceeded {
                required_bytes,
                max_bytes: self.max_surface_bytes,
            });
        }
        let requested_bytes = requested.iter().try_fold(0_u64, |total, bytes| {
            total.checked_add(*bytes).ok_or(SurfaceError::ByteOverflow)
        })?;
        let active_bytes = self
            .report
            .physical_bytes
            .checked_add(self.scratch_checked_out_bytes)
            .and_then(|bytes| bytes.checked_add(requested_bytes))
            .ok_or(SurfaceError::ByteOverflow)?;
        if active_bytes > self.max_frame_bytes {
            return Err(SurfaceError::FrameBudgetExceeded {
                required_bytes: active_bytes,
                max_bytes: self.max_frame_bytes,
            });
        }

        let mut needed = BTreeMap::<Extent2d, usize>::new();
        for extent in extents {
            *needed.entry(*extent).or_default() += 1;
        }
        let mut protected = vec![false; self.arena.scratch_free.len()];
        for (index, entry) in self.arena.scratch_free.iter().enumerate().rev() {
            if let Some(remaining) = needed.get_mut(&entry.extent)
                && *remaining > 0
            {
                protected[index] = true;
                *remaining -= 1;
            }
        }
        let new_bytes = needed.iter().try_fold(0_u64, |total, (extent, count)| {
            let count = u64::try_from(*count).map_err(|_| SurfaceError::ByteOverflow)?;
            total
                .checked_add(
                    surface_bytes(*extent)?
                        .checked_mul(count)
                        .ok_or(SurfaceError::ByteOverflow)?,
                )
                .ok_or(SurfaceError::ByteOverflow)
        })?;
        let plan_pool_bytes = self
            .arena
            .entries
            .iter()
            .try_fold(0_u64, |total, entry| total.checked_add(entry.bytes))
            .ok_or(SurfaceError::ByteOverflow)?;
        let free_pool_bytes = self
            .arena
            .scratch_free
            .iter()
            .try_fold(0_u64, |total, entry| total.checked_add(entry.bytes))
            .ok_or(SurfaceError::ByteOverflow)?;
        let delivery_pool_bytes = self
            .arena
            .delivery_surface
            .as_ref()
            .map_or(0, |entry| entry.bytes);
        let mut persistent_bytes = plan_pool_bytes
            .checked_add(free_pool_bytes)
            .and_then(|bytes| bytes.checked_add(delivery_pool_bytes))
            .and_then(|bytes| bytes.checked_add(self.scratch_checked_out_bytes))
            .and_then(|bytes| bytes.checked_add(new_bytes))
            .ok_or(SurfaceError::ByteOverflow)?;
        if persistent_bytes > self.max_frame_bytes {
            let old = core::mem::take(&mut self.arena.scratch_free);
            let mut retained = Vec::with_capacity(old.len());
            for (index, entry) in old.into_iter().enumerate() {
                if persistent_bytes > self.max_frame_bytes && !protected[index] {
                    persistent_bytes = persistent_bytes.saturating_sub(entry.bytes);
                    self.pool_evictions = self.pool_evictions.saturating_add(1);
                } else {
                    retained.push(entry);
                }
            }
            self.arena.scratch_free = retained;
        }
        if persistent_bytes > self.max_frame_bytes
            && let Some(entry) = self.arena.delivery_surface.take()
        {
            persistent_bytes = persistent_bytes.saturating_sub(entry.bytes);
            self.pool_evictions = self.pool_evictions.saturating_add(1);
        }
        if persistent_bytes > self.max_frame_bytes {
            return Err(SurfaceError::FrameBudgetExceeded {
                required_bytes: persistent_bytes,
                max_bytes: self.max_frame_bytes,
            });
        }

        let mut entries = Vec::with_capacity(extents.len());
        let mut allocations = 0_u64;
        let mut reuses = 0_u64;
        for extent in extents {
            let entry = if let Some(index) = self
                .arena
                .scratch_free
                .iter()
                .rposition(|entry| entry.extent == *extent)
            {
                reuses = reuses.saturating_add(1);
                self.arena.scratch_free.remove(index)
            } else {
                let info = working_info(*extent)?;
                allocations = allocations.saturating_add(1);
                SurfaceEntry {
                    _memory: valle_engine::motion::memory::reserve(
                        valle_engine::motion::memory::MemoryKind::Surface,
                        surface_bytes(*extent)?,
                    )?,
                    extent: *extent,
                    surface: self.arena.create_surface(&info)?,
                    bytes: surface_bytes(*extent)?,
                    reserved: false,
                }
            };
            entries.push(entry);
        }
        let bytes = entries
            .iter()
            .try_fold(0_u64, |total, entry| total.checked_add(entry.bytes))
            .ok_or(SurfaceError::ByteOverflow)?;
        self.scratch_allocations = self.scratch_allocations.saturating_add(allocations);
        self.scratch_reuses = self.scratch_reuses.saturating_add(reuses);
        self.scratch_checked_out_surfaces = self
            .scratch_checked_out_surfaces
            .checked_add(entries.len())
            .ok_or(SurfaceError::ByteOverflow)?;
        self.scratch_checked_out_bytes = self
            .scratch_checked_out_bytes
            .checked_add(bytes)
            .ok_or(SurfaceError::ByteOverflow)?;
        self.scratch_peak_surfaces = self
            .scratch_peak_surfaces
            .max(self.scratch_checked_out_surfaces);
        self.scratch_peak_bytes = self.scratch_peak_bytes.max(self.scratch_checked_out_bytes);
        // Pooled surfaces keep whatever an earlier user drew.
        let painted = entries
            .iter()
            .map(|entry| IRect::from_wh(entry.surface.width(), entry.surface.height()))
            .collect();
        Ok(ScratchSurfaces {
            frame: self,
            entries,
            bytes,
            painted,
        })
    }

    pub(crate) fn report(&self) -> SurfaceFrameReport {
        debug_assert_eq!(self.scratch_checked_out_surfaces, 0);
        debug_assert_eq!(self.scratch_checked_out_bytes, 0);
        let delivery_pool_bytes = self
            .arena
            .delivery_surface
            .as_ref()
            .map_or(0, |entry| entry.bytes);
        let scratch_pool_bytes = self
            .arena
            .scratch_free
            .iter()
            .fold(delivery_pool_bytes, |total, entry| {
                total.saturating_add(entry.bytes)
            });
        let plan_pool_bytes = self
            .arena
            .entries
            .iter()
            .fold(0_u64, |total, entry| total.saturating_add(entry.bytes));
        SurfaceFrameReport {
            pool_surfaces: self
                .arena
                .entries
                .len()
                .saturating_add(self.arena.scratch_free.len())
                .saturating_add(usize::from(self.arena.delivery_surface.is_some())),
            pool_bytes: plan_pool_bytes.saturating_add(scratch_pool_bytes),
            scratch_allocations: self.scratch_allocations,
            scratch_reuses: self.scratch_reuses,
            scratch_peak_surfaces: self.scratch_peak_surfaces,
            scratch_peak_bytes: self.scratch_peak_bytes,
            scratch_pool_surfaces: self
                .arena
                .scratch_free
                .len()
                .saturating_add(usize::from(self.arena.delivery_surface.is_some())),
            scratch_pool_bytes,
            pool_evictions: self.pool_evictions,
            ..self.report
        }
    }
}

pub(crate) struct ScratchSurfaces<'frame, 'arena> {
    frame: &'frame mut SurfaceFrame<'arena>,
    entries: Vec<SurfaceEntry>,
    bytes: u64,
    /// Per entry, a rectangle containing every pixel that may be non-transparent.
    painted: Vec<IRect>,
}

impl ScratchSurfaces<'_, '_> {
    pub(crate) fn backend_kind(&self) -> SkiaBackendKind {
        self.frame.backend_kind()
    }

    pub(crate) fn prepare_cpu_image(&mut self, image: &Image) -> Result<Image, SurfaceError> {
        self.frame.prepare_cpu_image(image)
    }

    pub(crate) fn surface_mut(&mut self, index: usize) -> Result<&mut Surface, SurfaceError> {
        let entry = self
            .entries
            .get_mut(index)
            .ok_or(SurfaceError::MissingScratchSurface { index })?;
        // The caller may draw anywhere.
        self.painted[index] = IRect::from_wh(entry.surface.width(), entry.surface.height());
        Ok(&mut entry.surface)
    }

    /// Borrows a scratch surface for a pass that draws only inside `bounds`, with the canvas
    /// clipped to them; the caller restores the canvas to the returned save count.
    ///
    /// Program slots are shared by resources of different sizes, so a slot surface can be many
    /// times larger than the ROI a pass renders. Clearing all of it for every pass dominated
    /// frames with many small filtered layers. Instead, with `clear_outside`, pixels outside
    /// `bounds` are cleared only where an earlier pass may have painted, which leaves them
    /// transparent exactly as a full clear did. A result that nothing reads outside `bounds`
    /// leaves those pixels as earlier passes drew them, still counted as painted.
    pub(crate) fn bounded_surface_mut(
        &mut self,
        index: usize,
        bounds: IRect,
        clear_outside: bool,
    ) -> Result<(&mut Surface, usize), SurfaceError> {
        let entry = self
            .entries
            .get_mut(index)
            .ok_or(SurfaceError::MissingScratchSurface { index })?;
        let painted = &mut self.painted[index];
        let canvas = entry.surface.canvas();
        if clear_outside {
            if !painted.is_empty() && !covers(bounds, *painted) {
                canvas.save();
                canvas.clip_irect(*painted, skia_safe::ClipOp::Intersect);
                canvas.clip_irect(bounds, skia_safe::ClipOp::Difference);
                canvas.clear(skia_safe::Color4f::new(0.0, 0.0, 0.0, 0.0));
                canvas.restore();
            }
            *painted = bounds;
        } else {
            *painted = IRect::join(painted, &bounds);
        }
        let count = canvas.save();
        canvas.clip_irect(bounds, skia_safe::ClipOp::Intersect);
        Ok((&mut entry.surface, count))
    }

    pub(crate) fn output_mut(&mut self, slot: SurfaceSlotId) -> Result<&mut Surface, SurfaceError> {
        self.frame.surface_mut(slot)
    }

    pub(crate) fn snapshot_output(
        &mut self,
        slot: SurfaceSlotId,
        roi: valle_engine::prepare::DeviceRect,
    ) -> Result<PlanImage, SurfaceError> {
        self.frame.snapshot(slot, roi)
    }
}

fn covers(outer: IRect, inner: IRect) -> bool {
    outer.left <= inner.left
        && outer.top <= inner.top
        && outer.right >= inner.right
        && outer.bottom >= inner.bottom
}

impl Drop for ScratchSurfaces<'_, '_> {
    fn drop(&mut self) {
        self.frame.scratch_checked_out_surfaces = self
            .frame
            .scratch_checked_out_surfaces
            .saturating_sub(self.entries.len());
        self.frame.scratch_checked_out_bytes = self
            .frame
            .scratch_checked_out_bytes
            .saturating_sub(self.bytes);
        self.frame.arena.scratch_free.append(&mut self.entries);
        self.frame.pool_evictions = self.frame.pool_evictions.saturating_add(
            self.frame
                .arena
                .trim_reusable_pool(self.frame.max_frame_bytes),
        );
    }
}

impl Drop for SurfaceFrame<'_> {
    fn drop(&mut self) {
        for index in self.assignments.values().copied() {
            self.arena.entries[index].reserved = false;
        }
    }
}

fn validate_slot(
    slot: &valle_engine::compositor::lower::SurfaceSlot,
    bound: &BoundSurfaceSlot,
) -> Result<(), SurfaceError> {
    if slot.surface_class.format != TextureFormat::Rgba16Float
        || slot.surface_class.working_space != WorkingColorSpace::LinearRec2020D65
        || slot.surface_class.alpha != WorkingAlphaMode::PremultipliedCoverage
        || slot.surface_class.sample_count != 1
    {
        return Err(SurfaceError::UnsupportedPlanSlot {
            slot: slot.id.get(),
        });
    }
    if bound.id() != slot.id
        || bound.allocations().len() != slot.allocations.len()
        || bound
            .allocations()
            .iter()
            .zip(&slot.allocations)
            .any(|(actual, expected)| {
                actual.resource() != expected.resource
                    || actual.interval() != expected.interval
                    || actual.reason() != expected.reason
            })
        || bound.extent().map(surface_bytes).transpose()?.unwrap_or(0) != bound.estimated_bytes()
    {
        return Err(SurfaceError::InvalidBoundSlot {
            slot: slot.id.get(),
        });
    }
    Ok(())
}

fn surface_bytes(extent: Extent2d) -> Result<u64, SurfaceError> {
    u64::from(extent.width())
        .checked_mul(u64::from(extent.height()))
        .and_then(|pixels| pixels.checked_mul(8))
        .ok_or(SurfaceError::ByteOverflow)
}

fn surface_bytes_for_rect(rect: valle_engine::prepare::DeviceRect) -> Result<u64, SurfaceError> {
    u64::from(rect.width)
        .checked_mul(u64::from(rect.height))
        .and_then(|pixels| pixels.checked_mul(8))
        .ok_or(SurfaceError::ByteOverflow)
}

#[derive(Debug, Error)]
pub enum SurfaceError {
    #[error(transparent)]
    Memory(#[from] valle_engine::motion::memory::MemoryError),
    #[error("render extent does not fit the Skia coordinate domain")]
    InvalidExtent,
    #[error("Skia cannot construct Linear Rec.2020 D65")]
    UnsupportedWorkingColorSpace,
    #[error("Skia cannot construct the declared external color space")]
    UnsupportedInputColorSpace,
    #[error("Skia surface allocation failed")]
    Allocation,
    #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
    #[error("Vulkan compositor unavailable: {0}")]
    VulkanUnavailable(String),
    #[error("GPU working image could not be materialized for a CPU sink")]
    Readback,
    #[cfg(all(target_os = "macos", feature = "native"))]
    #[error("Metal compositor unavailable: {0}")]
    MetalUnavailable(&'static str),
    #[cfg(all(
        feature = "native",
        any(target_os = "macos", target_os = "linux", target_os = "windows")
    ))]
    #[error("Scene3D GPU raster failed: {0}")]
    Scene3dGpu(String),
    #[cfg(all(target_os = "macos", feature = "native"))]
    #[error("shared GPU frame requires the Metal compositor")]
    SharedFrameRequiresMetal,
    #[cfg(all(target_os = "macos", feature = "native"))]
    #[error("shared GPU frame failed: {0}")]
    SharedFrame(String),
    #[cfg(all(target_os = "macos", feature = "native"))]
    #[error("shared GPU frame was submitted more than once")]
    SharedFrameSubmitted,
    #[cfg(feature = "native")]
    #[error("decoded GPU frame cannot be imported by the active compositor backend")]
    InvalidDecodedGpuFrame,
    #[cfg(all(target_os = "macos", feature = "native"))]
    #[error("decoded GPU pixel format 0x{pixel_format:08x} is unsupported")]
    UnsupportedDecodedPixelFormat { pixel_format: u32 },
    #[cfg(all(target_os = "macos", feature = "native"))]
    #[error("decoded GPU texture plane {plane} import failed with CVReturn {code}")]
    DecodedTextureImport { plane: usize, code: i32 },
    #[error("surface-pool byte accounting overflowed")]
    ByteOverflow,
    #[error("surface-pool frame is already active")]
    FrameAlreadyActive,
    #[error("surface allocation accounting scope is already active on this executor thread")]
    AllocationScopeAlreadyActive,
    #[error("plan surface slot {slot} is outside the CPU Skia working contract")]
    UnsupportedPlanSlot { slot: u32 },
    #[error("plan surface slot {slot} has no corresponding bound schedule slot")]
    MissingBoundSlot { slot: u32 },
    #[error("bound plan surface slot {slot} does not match its immutable template slot")]
    InvalidBoundSlot { slot: u32 },
    #[error("plan surface slot {slot} occurs more than once")]
    DuplicateSlot { slot: u32 },
    #[error("plan surface slot {slot} is not reserved in this frame")]
    MissingSlot { slot: u32 },
    #[error("image extent does not match plan surface slot {slot}")]
    ImageExtentMismatch { slot: u32 },
    #[error("scratch surface index {index} is outside the checked-out set")]
    MissingScratchSurface { index: usize },
    #[error("surface requires {required_bytes} bytes, exceeding the {max_bytes}-byte limit")]
    SurfaceBudgetExceeded { required_bytes: u64, max_bytes: u64 },
    #[error("frame surfaces require {required_bytes} bytes, exceeding the {max_bytes}-byte limit")]
    FrameBudgetExceeded { required_bytes: u64, max_bytes: u64 },
}

#[cfg(test)]
mod capacity_tests {
    use super::*;

    #[cfg(all(target_os = "macos", feature = "native"))]
    #[test]
    #[ignore = "requires a real Metal device"]
    fn straight_output_transport_preserves_unassociated_bytes() {
        let mut arena = SurfaceArena::metal().unwrap();
        let info = ImageInfo::new((256, 1), ColorType::RGBA8888, AlphaType::Premul, None);
        let effect = skia_safe::RuntimeEffect::make_for_shader(
            "half4 main(float2 p) { return half4(1.0, 0.5, 0.25, floor(p.x) / 255.0); }",
            None,
        )
        .unwrap();
        let shader = effect
            .make_shader(skia_safe::Data::new_empty(), &[], None)
            .unwrap();
        let mut frame = test_frame(&mut arena, 1024 * 1024);
        let image = frame.render_output_shader_to_raster(shader, &info).unwrap();
        let pixels = image.peek_pixels().unwrap();
        for (alpha, pixel) in pixels
            .bytes()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
        {
            assert_eq!(*pixel, [255, 128, 64, alpha as u8], "alpha={alpha}");
        }
    }

    #[test]
    fn stable_gaussian_matches_direct_convolution_across_sigma_steps() {
        use skia_safe::{Paint, Rect};
        use valle_draw::program::Filter;
        let mut arena = match std::env::var("VALLE_TEST_NATIVE_BACKEND").as_deref() {
            #[cfg(all(target_os = "macos", feature = "native"))]
            Ok("metal") => SurfaceArena::metal().unwrap(),
            _ => SurfaceArena::new(),
        };
        let info = working_info(Extent2d::new(256, 256).unwrap()).unwrap();
        let read_info = info.with_color_type(ColorType::RGBAF32);
        let mut source = arena.create_surface(&info).unwrap();
        let mut reference_source = skia_safe::surfaces::raster(&info, None, None).unwrap();
        for surface in [&mut source, &mut reference_source] {
            surface.canvas().clear(skia_safe::Color::TRANSPARENT);
            let mut paint = Paint::default();
            paint
                .set_anti_alias(true)
                .set_color4f(skia_safe::Color4f::new(0.8, 0.3, 0.1, 0.75), None);
            surface
                .canvas()
                .draw_rect(Rect::from_xywh(108.0, 116.0, 40.0, 24.0), &paint);
        }
        let input = source.image_snapshot();
        let reference_input = reference_source.image_snapshot();
        for sigma in [
            0.0, 0.25, 1.0, 3.9, 4.1, 7.9, 8.1, 15.9, 16.0, 16.1, 31.9, 32.0,
        ] {
            for sy in [0.0, sigma] {
                let filter = Filter::Blur {
                    sigma_x: sigma,
                    sigma_y: sy,
                };
                let mut output = arena.create_surface(&info).unwrap();
                let mut reference = skia_safe::surfaces::raster(&info, None, None).unwrap();
                output.canvas().clear(skia_safe::Color::TRANSPARENT);
                reference.canvas().clear(skia_safe::Color::TRANSPARENT);
                let mut paint = Paint::default();
                paint.set_image_filter(super::super::gaussian_gpu::filter(&filter).unwrap());
                output.canvas().draw_image(&input, (0, 0), Some(&paint));
                assert!(
                    super::super::blur::draw(
                        reference.canvas(),
                        &reference_input,
                        IRect::from_wh(256, 256),
                        [0, 0],
                        &filter
                    )
                    .unwrap()
                );
                let read = |surface: &mut Surface| {
                    let mut bytes = vec![0u8; 256 * 256 * 16];
                    assert!(surface.read_pixels(&read_info, &mut bytes, 256 * 16, (0, 0)));
                    bytes
                        .chunks_exact(4)
                        .map(|value| f32::from_ne_bytes(value.try_into().unwrap()))
                        .collect::<Vec<_>>()
                };
                let actual = read(&mut output);
                let expected = read(&mut reference);
                let maximum = actual
                    .iter()
                    .zip(&expected)
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0_f32, f32::max);
                assert!(
                    maximum < 0.003,
                    "sigma=({sigma},{sy}) max float error={maximum}"
                );
            }
        }
    }

    #[test]
    fn wide_gaussian_preserves_edges_energy_hdr_and_sigma_continuity() {
        use skia_safe::{Color4f, Paint, Rect};
        use valle_draw::program::Filter;
        let mut arena = match std::env::var("VALLE_TEST_NATIVE_BACKEND").as_deref() {
            #[cfg(all(target_os = "macos", feature = "native"))]
            Ok("metal") => SurfaceArena::metal().unwrap(),
            _ => SurfaceArena::new(),
        };
        const LENGTH: usize = 2048;
        let started = std::time::Instant::now();
        let mut worst = 0.0_f32;
        for horizontal in [true, false] {
            let (width, height) = if horizontal { (LENGTH, 4) } else { (4, LENGTH) };
            let info = working_info(Extent2d::new(width as u32, height as u32).unwrap()).unwrap();
            let read_info = info.with_color_type(ColorType::RGBAF32);
            for stripes in [false, true] {
                let mut source = arena.create_surface(&info).unwrap();
                source.canvas().clear(skia_safe::Color::TRANSPARENT);
                let mut paint = Paint::default();
                paint.set_color4f(
                    Color4f::new(2.0, 0.0, 0.0, 1.0),
                    info.color_space().as_ref(),
                );
                let mut rect = |start: usize, length: usize| {
                    let rect = if horizontal {
                        Rect::from_xywh(start as f32, 0.0, length as f32, 4.0)
                    } else {
                        Rect::from_xywh(0.0, start as f32, 4.0, length as f32)
                    };
                    source.canvas().draw_rect(rect, &paint);
                };
                if stripes {
                    for position in (0..LENGTH).step_by(2) {
                        rect(position, 1);
                    }
                } else {
                    rect(0, LENGTH / 2);
                }
                let input = source.image_snapshot();
                let mut previous = 0.0_f32;
                let mut previous_sigma = 0.0_f32;
                let mut observed = BTreeMap::new();
                for sigma in [
                    16.0, 31.8, 31.9, 32.0, 32.1, 32.2, 40.0, 48.0, 63.8, 63.9, 64.0, 64.1, 64.2,
                    96.0, 127.8, 127.9, 128.0, 128.1, 128.2, 192.0, 255.9, 256.0, 256.1, 512.0,
                    1024.0, 32.1, 64.1, 16.0, 128.1, 32.1,
                ] {
                    let filter = Filter::Blur {
                        sigma_x: if horizontal { sigma } else { 0.0 },
                        sigma_y: if horizontal { 0.0 } else { sigma },
                    };
                    assert!(super::super::gaussian_gpu::supports(&filter));
                    let mut output = arena.create_surface(&info).unwrap();
                    output.canvas().clear(skia_safe::Color::TRANSPARENT);
                    let mut filtered = Paint::default();
                    filtered.set_image_filter(super::super::gaussian_gpu::filter(&filter).unwrap());
                    output.canvas().draw_image(&input, (0, 0), Some(&filtered));
                    let mut bytes = vec![0; width * height * 16];
                    assert!(output.read_pixels(&read_info, &mut bytes, width * 16, (0, 0)));
                    if let Some(prior) = observed.insert(sigma.to_bits(), bytes.clone()) {
                        assert_eq!(
                            bytes, prior,
                            "wide Gaussian changed after a seek at sigma={sigma}"
                        );
                    }
                    let pixels = bytes
                        .chunks_exact(4)
                        .map(|b| f32::from_ne_bytes(b.try_into().unwrap()))
                        .collect::<Vec<_>>();
                    let weights =
                        super::super::blur::kernel_with_radius_limit(sigma, 4096).unwrap();
                    let radius = (weights.len() / 2) as isize;
                    let mut prefix = vec![[0.0_f64; 2]; weights.len() + 1];
                    for (i, &weight) in weights.iter().enumerate() {
                        prefix[i + 1] = prefix[i];
                        prefix[i + 1][((i as isize - radius).rem_euclid(2)) as usize] +=
                            f64::from(weight);
                    }
                    let source_end = if stripes { LENGTH - 1 } else { LENGTH / 2 - 1 };
                    for position in 0..LENGTH {
                        let first = (position as isize - source_end as isize).max(-radius);
                        let last = (position as isize).min(radius);
                        let expected = if first > last {
                            0.0
                        } else {
                            let low = prefix[(first + radius) as usize];
                            let high = prefix[(last + radius + 1) as usize];
                            if stripes {
                                high[position % 2] - low[position % 2]
                            } else {
                                high[0] + high[1] - low[0] - low[1]
                            }
                        } as f32;
                        let index = if horizontal {
                            2 * width + position
                        } else {
                            position * width + 2
                        };
                        let alpha = pixels[index * 4 + 3];
                        let error = (alpha - expected).abs();
                        worst = worst.max(error);
                        assert!(
                            error < 0.003,
                            "sigma={sigma} horizontal={horizontal} stripes={stripes} at={position}: alpha={alpha} expected={expected} error={error}"
                        );
                        assert!(
                            (pixels[index * 4] - 2.0 * alpha).abs() < 0.001,
                            "HDR mismatch sigma={sigma} at={position} rgba=({}, {alpha})",
                            pixels[index * 4]
                        );
                        if !stripes && position == LENGTH / 2 + 24 {
                            if sigma >= previous_sigma && sigma <= 256.1 {
                                assert!(
                                    alpha + 0.0005 >= previous,
                                    "blur rebound at sigma={sigma}: {previous} -> {alpha}"
                                );
                            }
                            previous = alpha;
                            previous_sigma = sigma;
                        }
                    }
                }
            }
        }
        eprintln!(
            "wide Gaussian: backend={:?} max_alpha_error={worst} elapsed={:?}",
            arena.backend_kind(),
            started.elapsed()
        );
    }

    #[test]
    #[cfg(all(target_os = "macos", feature = "native"))]
    #[ignore = "requires real Metal; includes 4K wide blur and shadow timing"]
    fn wide_gaussian_metal_filter_chain_and_4k_probe() {
        use skia_safe::{Color4f, Paint, Rect};
        use valle_draw::program::{Filter, LinearColor};
        let mut arena = SurfaceArena::metal().unwrap();
        let integral = |sigma, coordinate: i32, first: i32, length: i32| {
            let weights = super::super::blur::kernel_with_radius_limit(sigma, 4096).unwrap();
            let radius = weights.len() as i32 / 2;
            weights
                .iter()
                .enumerate()
                .filter_map(|(i, &weight)| {
                    let source = coordinate - (i as i32 - radius);
                    (source >= first && source < first + length).then_some(f64::from(weight))
                })
                .sum::<f64>() as f32
        };
        for (width, height) in [(384u32, 256u32), (3840, 2160)] {
            let info = working_info(Extent2d::new(width, height).unwrap()).unwrap();
            let read_info = info.with_color_type(ColorType::RGBAF32);
            let (left, top, rw, rh) = (
                width as i32 * 3 / 8,
                height as i32 * 3 / 8,
                width as i32 / 4,
                height as i32 / 4,
            );
            let mut source = arena.create_surface(&info).unwrap();
            source.canvas().clear(skia_safe::Color::TRANSPARENT);
            let mut paint = Paint::default();
            paint.set_color4f(
                Color4f::new(0.8, 0.3, 0.1, 0.5),
                info.color_space().as_ref(),
            );
            source.canvas().draw_rect(
                Rect::from_xywh(left as f32, top as f32, rw as f32, rh as f32),
                &paint,
            );
            let input = source.image_snapshot();
            for (sx, sy) in [(32.0, 32.0), (32.1, 32.1), (64.1, 32.1), (128.1, 48.0)] {
                for shadow in [false, true] {
                    let filter = if shadow {
                        Filter::DropShadow {
                            sigma_x: sx,
                            sigma_y: sy,
                            offset: [9.0, -7.0],
                            color: LinearColor::new(0.0, 0.0, 0.0, 0.75),
                        }
                    } else {
                        Filter::Blur {
                            sigma_x: sx,
                            sigma_y: sy,
                        }
                    };
                    let mut output = arena.create_surface(&info).unwrap();
                    output.canvas().clear(skia_safe::Color::TRANSPARENT);
                    let started = std::time::Instant::now();
                    super::super::effect::draw_filters_into(
                        &mut output,
                        &input,
                        IRect::from_wh(width as i32, height as i32),
                        [0, 0],
                        std::slice::from_ref(&filter),
                    )
                    .unwrap();
                    let (read, origin) = if width == 384 {
                        (read_info.clone(), (0, 0))
                    } else {
                        (read_info.with_dimensions((1, 1)), (left + rw, top + rh / 2))
                    };
                    let stride = read.width() as usize * 16;
                    let mut bytes = vec![0; stride * read.height() as usize];
                    assert!(output.read_pixels(&read, &mut bytes, stride, origin));
                    let elapsed = started.elapsed();
                    let pixels = bytes
                        .chunks_exact(4)
                        .map(|b| f32::from_ne_bytes(b.try_into().unwrap()))
                        .collect::<Vec<_>>();
                    let offset = if shadow { [9, -7] } else { [0, 0] };
                    let xs = (0..read.width())
                        .map(|x| integral(sx, x + origin.0 - offset[0], left, rw))
                        .collect::<Vec<_>>();
                    let ys = (0..read.height())
                        .map(|y| integral(sy, y + origin.1 - offset[1], top, rh))
                        .collect::<Vec<_>>();
                    for (y, &fy) in ys.iter().enumerate() {
                        for (x, &fx) in xs.iter().enumerate() {
                            let mut expected = 0.5 * fx * fy;
                            if shadow {
                                let inside = (left..left + rw).contains(&(x as i32 + origin.0))
                                    && (top..top + rh).contains(&(y as i32 + origin.1));
                                let original = if inside { 0.5 } else { 0.0 };
                                expected = original + expected * 0.75 * (1.0 - original);
                            }
                            let alpha = pixels[(y * xs.len() + x) * 4 + 3];
                            assert!(
                                (alpha - expected).abs() < 0.003,
                                "sigma=({sx},{sy}) shadow={shadow} at=({x},{y}): {alpha} != {expected}"
                            );
                        }
                    }
                    if width == 3840 {
                        let mut warmed = [std::time::Duration::ZERO; 2];
                        for (path, timing) in warmed.iter_mut().enumerate() {
                            for pass in 0..3 {
                                output.canvas().clear(skia_safe::Color::TRANSPARENT);
                                let started = std::time::Instant::now();
                                if path == 0 {
                                    super::super::effect::draw_filters_into(
                                        &mut output,
                                        &input,
                                        IRect::from_wh(width as i32, height as i32),
                                        [0, 0],
                                        std::slice::from_ref(&filter),
                                    )
                                    .unwrap();
                                } else {
                                    let mut old = Paint::default();
                                    old.set_image_filter(
                                        super::super::effect::image_filter(&filter).unwrap(),
                                    );
                                    output.canvas().draw_image(&input, (0, 0), Some(&old));
                                }
                                assert!(output.read_pixels(&read, &mut bytes, stride, origin));
                                if pass > 0 {
                                    *timing += started.elapsed();
                                }
                            }
                            *timing /= 2;
                        }
                        // Compare the previous Skia path at the same size, including the same
                        // final one-pixel synchronization read. This is not an encoder benchmark.
                        eprintln!(
                            "4K Gaussian sigma=({sx},{sy}) shadow={shadow}: first={elapsed:?}, warmed stable={:?}, warmed Skia={:?}, cached_gpu_bytes={} (includes final pixel read)",
                            warmed[0],
                            warmed[1],
                            match &arena.backend {
                                SurfaceBackend::Metal { context, .. } =>
                                    context.resource_cache_usage().resource_bytes,
                                SurfaceBackend::Raster => 0,
                            }
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn solid_working_colors_keep_float_precision_and_repeat_exactly() {
        use valle_draw::program::LinearColor;
        let mut arena = match std::env::var("VALLE_TEST_NATIVE_BACKEND").as_deref() {
            Err(std::env::VarError::NotPresent) | Ok("raster") => SurfaceArena::new(),
            #[cfg(all(target_os = "macos", feature = "native"))]
            Ok("metal") => SurfaceArena::metal().unwrap(),
            other => panic!("unsupported test backend: {other:?}"),
        };
        let info = working_info(Extent2d::new(16, 16).unwrap()).unwrap();
        let mut surface = arena.create_surface(&info).unwrap();
        let read_info = ImageInfo::new(
            (16, 16),
            ColorType::RGBAF32,
            AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let colors = [
            LinearColor::from_srgb_straight([1.0, 0.0, 0.0, 1.0]),
            LinearColor::from_srgb_straight([0.0, 0.0, 1.0, 1.0]),
            LinearColor::from_srgb_straight([0.25, 0.5, 0.125, 0.4]),
            LinearColor::new(2.0, 0.17, -0.03, 0.5),
        ];
        let mut baseline = BTreeMap::new();
        for index in [2, 0, 3, 1, 2, 0] {
            let color = colors[index];
            let mut paint = skia_safe::Paint::default();
            paint.set_anti_alias(true);
            paint.set_blend_mode(skia_safe::BlendMode::Src);
            super::super::effect::set_working_color(&mut paint, color).unwrap();
            surface.canvas().clear(skia_safe::Color::TRANSPARENT);
            surface
                .canvas()
                .draw_rect(skia_safe::Rect::from_xywh(1.0, 1.0, 14.0, 14.0), &paint);
            let mut bytes = vec![0u8; 16 * 16 * 16];
            assert!(surface.read_pixels(&read_info, &mut bytes, 16 * 16, (0, 0)));
            let center = (8 * 16 + 8) * 16;
            for (actual, expected) in bytes[center..center + 16].chunks_exact(4).zip([
                color.red,
                color.green,
                color.blue,
                color.alpha,
            ]) {
                let actual = f32::from_ne_bytes(actual.try_into().unwrap());
                assert!(
                    (actual - expected).abs() <= 0.0005,
                    "color={index}: {actual} != {expected}; solid paint must retain F16 precision"
                );
            }
            if let Some(prior) = baseline.insert(index, bytes.clone()) {
                assert_eq!(bytes, prior, "solid paint depends on preceding colors");
            }
        }
    }

    fn test_frame(arena: &mut SurfaceArena, max_bytes: u64) -> SurfaceFrame<'_> {
        SurfaceFrame {
            arena,
            assignments: BTreeMap::new(),
            report: SurfaceFrameReport {
                generation: 0,
                generation_invalidations: 0,
                plan_slots: 0,
                logical_allocations: 0,
                allocations: 0,
                reuses: 0,
                physical_bytes: 0,
                logical_bytes: 0,
                alias_saved_bytes: 0,
                estimated_peak_bytes: 0,
                pool_surfaces: 0,
                pool_bytes: 0,
                scratch_allocations: 0,
                scratch_reuses: 0,
                scratch_peak_surfaces: 0,
                scratch_peak_bytes: 0,
                scratch_pool_surfaces: 0,
                scratch_pool_bytes: 0,
                pool_evictions: 0,
            },
            scratch_allocations: 0,
            scratch_reuses: 0,
            scratch_checked_out_surfaces: 0,
            scratch_checked_out_bytes: 0,
            scratch_peak_surfaces: 0,
            scratch_peak_bytes: 0,
            max_surface_bytes: max_bytes,
            max_frame_bytes: max_bytes,
            pool_evictions: 0,
        }
    }

    fn read_working(surface: &mut Surface, bounds: IRect) -> Vec<f32> {
        let info = ImageInfo::new(
            bounds.size(),
            ColorType::RGBAF32,
            AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let mut pixels = vec![0.0_f32; (bounds.width() * bounds.height() * 4) as usize];
        assert!(surface.image_snapshot().read_pixels(
            &info,
            &mut pixels,
            bounds.width() as usize * 16,
            (bounds.left, bounds.top),
            skia_safe::image::CachingHint::Disallow,
        ));
        pixels
    }

    fn covers_pixel(rect: IRect, x: i32, y: i32) -> bool {
        x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
    }

    #[test]
    fn bounded_scratch_passes_clear_stale_pixels_and_render_like_exact_surfaces() {
        let mut arena = SurfaceArena::new();
        let mut frame = test_frame(&mut arena, 64 * 1024 * 1024);
        let extent = Extent2d::new(96, 64).unwrap();
        let mut scratch = frame.scratch(&[extent]).unwrap();
        let everything = IRect::from_wh(96, 64);
        let mut fill = skia_safe::Paint::default();
        fill.set_color(skia_safe::Color::GREEN);
        let draw_disc = |canvas: &skia_safe::Canvas| {
            let mut paint = skia_safe::Paint::default();
            paint.set_anti_alias(true);
            paint.set_color4f(skia_safe::Color4f::new(0.1, 0.8, 0.9, 0.9), None);
            // Crosses the bottom and right edges of a 30x20 ROI.
            canvas.draw_circle((27.3, 17.6), 6.4, &paint);
        };

        // An unbounded user can leave pixels anywhere.
        scratch
            .surface_mut(0)
            .unwrap()
            .canvas()
            .clear(skia_safe::Color::RED);
        let first = IRect::from_xywh(0, 0, 30, 20);
        let (surface, count) = scratch.bounded_surface_mut(0, first, true).unwrap();
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        draw_disc(surface.canvas());
        surface
            .canvas()
            .draw_rect(skia_safe::Rect::from_xywh(0.0, 0.0, 4.0, 4.0), &fill);
        surface.canvas().restore_to_count(count);

        let mut exact = arena_surface(30, 20);
        exact.canvas().clear(skia_safe::Color::TRANSPARENT);
        draw_disc(exact.canvas());
        exact
            .canvas()
            .draw_rect(skia_safe::Rect::from_xywh(0.0, 0.0, 4.0, 4.0), &fill);
        let surface = scratch.surface_mut(0).unwrap();
        let everything_pixels = read_working(surface, everything);
        let clipped = read_working(surface, first);
        assert_eq!(
            clipped
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            read_working(&mut exact, IRect::from_wh(30, 20))
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            "a clipped pass must render exactly like a ROI-sized surface"
        );
        for y in 0..64 {
            for x in 0..96 {
                if !covers_pixel(first, x, y) {
                    let pixel = &everything_pixels[((y * 96 + x) * 4) as usize..][..4];
                    assert_eq!(pixel, [0.0; 4], "stale pixel at {x},{y}");
                }
            }
        }

        // A later, disjoint ROI leaves nothing from earlier passes outside it.
        let second = IRect::from_xywh(50, 30, 20, 20);
        let (surface, count) = scratch.bounded_surface_mut(0, second, true).unwrap();
        surface
            .canvas()
            .draw_rect(skia_safe::Rect::from_xywh(0.0, 0.0, 96.0, 64.0), &fill);
        surface.canvas().restore_to_count(count);
        let surface = scratch.surface_mut(0).unwrap();
        let pixels = read_working(surface, everything);
        for y in 0..64 {
            for x in 0..96 {
                let pixel = &pixels[((y * 96 + x) * 4) as usize..][..4];
                let expected = if covers_pixel(second, x, y) { 1.0 } else { 0.0 };
                assert_eq!(pixel[3], expected, "pixel at {x},{y}");
            }
        }
    }

    #[test]
    fn bounded_scratch_passes_keep_outside_pixels_until_a_pass_must_clear_them() {
        let mut arena = SurfaceArena::new();
        let mut frame = test_frame(&mut arena, 64 * 1024 * 1024);
        let extent = Extent2d::new(96, 64).unwrap();
        let mut scratch = frame.scratch(&[extent]).unwrap();
        let everything = IRect::from_wh(96, 64);
        scratch
            .surface_mut(0)
            .unwrap()
            .canvas()
            .clear(skia_safe::Color::RED);
        // Each pass fills its ROI with its own alpha and returns the whole surface.
        let fill = |surface: &mut Surface, alpha: f32| {
            surface
                .canvas()
                .clear(skia_safe::Color4f::new(0.0, 0.0, 0.0, alpha));
        };
        let mut pass = |bounds: IRect, clear_outside: bool, alpha: f32| {
            let (surface, count) = scratch
                .bounded_surface_mut(0, bounds, clear_outside)
                .unwrap();
            fill(surface, alpha);
            surface.canvas().restore_to_count(count);
            read_working(surface, everything)
        };
        let check = |pixels: &[f32], filled: &[(IRect, f32)]| {
            let filled = filled
                .iter()
                .map(|&(bounds, alpha)| {
                    let mut alone = arena_surface(1, 1);
                    fill(&mut alone, alpha);
                    (bounds, read_working(&mut alone, IRect::from_wh(1, 1)))
                })
                .collect::<Vec<_>>();
            for y in 0..64 {
                for x in 0..96 {
                    let pixel = &pixels[((y * 96 + x) * 4) as usize..][..4];
                    let expected = filled
                        .iter()
                        .find(|(bounds, _)| covers_pixel(*bounds, x, y))
                        .map_or(&[0.0; 4][..], |(_, value)| value.as_slice());
                    assert_eq!(pixel, expected, "pixel at {x},{y}");
                }
            }
        };
        let first = IRect::from_xywh(0, 0, 30, 20);
        check(&pass(first, true, 0.25), &[(first, 0.25)]);

        // Results that nothing reads outside their ROI leave what earlier passes drew there.
        let second = IRect::from_xywh(50, 30, 20, 20);
        let third = IRect::from_xywh(70, 0, 26, 10);
        pass(second, false, 0.5);
        check(
            &pass(third, false, 0.75),
            &[(first, 0.25), (second, 0.5), (third, 0.75)],
        );

        // A result read outside its ROI clears everything the passes before it left.
        let fourth = IRect::from_xywh(10, 40, 20, 20);
        check(&pass(fourth, true, 1.0), &[(fourth, 1.0)]);
    }

    fn arena_surface(width: u32, height: u32) -> Surface {
        SurfaceArena::new()
            .create_surface(&working_info(Extent2d::new(width, height).unwrap()).unwrap())
            .unwrap()
    }

    #[test]
    fn program_capacity_reuses_nearby_sizes_and_honors_exact_budget_fallback() {
        let mut arena = SurfaceArena::new();
        let mut frame = SurfaceFrame {
            arena: &mut arena,
            assignments: BTreeMap::new(),
            report: SurfaceFrameReport {
                generation: 0,
                generation_invalidations: 0,
                plan_slots: 0,
                logical_allocations: 0,
                allocations: 0,
                reuses: 0,
                physical_bytes: 0,
                logical_bytes: 0,
                alias_saved_bytes: 0,
                estimated_peak_bytes: 0,
                pool_surfaces: 0,
                pool_bytes: 0,
                scratch_allocations: 0,
                scratch_reuses: 0,
                scratch_peak_surfaces: 0,
                scratch_peak_bytes: 0,
                scratch_pool_surfaces: 0,
                scratch_pool_bytes: 0,
                pool_evictions: 0,
            },
            scratch_allocations: 0,
            scratch_reuses: 0,
            scratch_checked_out_surfaces: 0,
            scratch_checked_out_bytes: 0,
            scratch_peak_surfaces: 0,
            scratch_peak_bytes: 0,
            max_surface_bytes: 65536,
            max_frame_bytes: 65536,
            pool_evictions: 0,
        };
        let small = Extent2d::new(33, 35).unwrap();
        let nearby = Extent2d::new(36, 37).unwrap();
        {
            let mut scratch = frame.scratch_program(&[small]).unwrap();
            assert_eq!(scratch.bytes, 64 * 64 * 8);
            assert_eq!(scratch.surface_mut(0).unwrap().width(), 64);
        }
        {
            let _scratch = frame.scratch_program(&[nearby]).unwrap();
        }
        assert_eq!(frame.scratch_allocations, 1);
        assert_eq!(frame.scratch_reuses, 1);
        assert_eq!(frame.scratch_peak_bytes, 64 * 64 * 8);
        assert_eq!(frame.scratch_checked_out_bytes, 0);

        frame.max_surface_bytes = surface_bytes(nearby).unwrap();
        frame.max_frame_bytes = frame.max_surface_bytes;
        {
            let mut scratch = frame.scratch_program(&[nearby]).unwrap();
            assert_eq!(scratch.bytes, 36 * 37 * 8);
            assert_eq!(scratch.surface_mut(0).unwrap().width(), 36);
        }
        assert!(frame.arena.pool_bytes().unwrap() <= frame.max_frame_bytes);
        frame.max_frame_bytes = surface_bytes(small).unwrap() - 1;
        assert!(matches!(
            frame.scratch_program(&[small]),
            Err(SurfaceError::FrameBudgetExceeded { .. })
        ));
    }
}
