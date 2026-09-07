#[cfg(all(target_os = "macos", feature = "native"))]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
};

#[cfg(all(target_os = "macos", feature = "native"))]
use skia_safe::gpu;
use skia_safe::{
    AlphaType, ColorSpace, ColorType, IRect, Image, ImageInfo, Surface, named_primaries,
    named_transfer_fn, surfaces,
};
#[cfg(all(target_os = "macos", feature = "native"))]
use skia_safe::{BlendMode, Color4f, EncodedOrigin, Paint, Shader, YUVAInfo, YUVColorSpace};

/// Physical Skia storage selected for a runner. This is execution evidence, not a capability
/// discriminator: Engine lowering still consumes only the immutable capability contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkiaBackendKind {
    Raster,
    #[cfg(all(target_os = "macos", feature = "native"))]
    Metal,
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
}

impl PlanImage {
    pub(crate) fn new(image: Image, roi: valle_engine::prepare::DeviceRect) -> Self {
        Self {
            image: Some(image),
            roi,
        }
    }

    pub(crate) const fn transparent() -> Self {
        Self {
            image: None,
            roi: valle_engine::prepare::DeviceRect::new(0, 0, 0, 0),
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
    _source: Arc<valle_media::SourceFrame>,
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
#[cfg(feature = "native")]
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
                AlphaType::Opaque,
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
            let yuv_color_space = match (decoded.matrix(), decoded.is_full_range()) {
                (YuvMatrix::Bt601, true) => YUVColorSpace::JPEG_Full,
                (YuvMatrix::Bt601, false) => YUVColorSpace::Rec601_Limited,
                (YuvMatrix::Bt709, true) => YUVColorSpace::Rec709_Full,
                (YuvMatrix::Bt709, false) => YUVColorSpace::Rec709_Limited,
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
            let image =
                gpu::images::texture_from_yuva_textures(context, &backend, Some(image_color_space))
                    .ok_or(SurfaceError::InvalidDecodedGpuFrame)?;
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
        _source: Arc::clone(source),
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
    backend: SurfaceBackend,
    generation: Option<SurfaceGeneration>,
    generation_id: u64,
    invalidations: u64,
    reported_invalidations: u64,
    entries: Vec<SurfaceEntry>,
    scratch_free: Vec<SurfaceEntry>,
    delivery_surface: Option<SurfaceEntry>,
    #[cfg(all(target_os = "macos", feature = "native"))]
    pending_external_imports: Vec<ImportedMetalImage>,
}

enum SurfaceBackend {
    Raster,
    #[cfg(all(target_os = "macos", feature = "native"))]
    Metal {
        context: gpu::DirectContext,
        texture_cache: CoreFoundationHandle,
    },
}

impl core::fmt::Debug for SurfaceBackend {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Raster => formatter.write_str("Raster"),
            #[cfg(all(target_os = "macos", feature = "native"))]
            Self::Metal { .. } => formatter.write_str("Metal"),
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
    pub(crate) fn new() -> Self {
        Self {
            backend: SurfaceBackend::Raster,
            generation: None,
            generation_id: 0,
            invalidations: 0,
            reported_invalidations: 0,
            entries: Vec::new(),
            scratch_free: Vec::new(),
            delivery_surface: None,
            #[cfg(all(target_os = "macos", feature = "native"))]
            pending_external_imports: Vec::new(),
        }
    }

    pub(crate) const fn backend_kind(&self) -> SkiaBackendKind {
        match &self.backend {
            SurfaceBackend::Raster => SkiaBackendKind::Raster,
            #[cfg(all(target_os = "macos", feature = "native"))]
            SurfaceBackend::Metal { .. } => SkiaBackendKind::Metal,
        }
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn metal() -> Result<Self, SurfaceError> {
        use objc2_metal::MTLDevice;

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
        Ok(Self {
            backend: SurfaceBackend::Metal {
                context,
                texture_cache: CoreFoundationHandle(texture_cache),
            },
            generation: None,
            generation_id: 0,
            invalidations: 0,
            reported_invalidations: 0,
            entries: Vec::new(),
            scratch_free: Vec::new(),
            delivery_surface: None,
            #[cfg(feature = "native")]
            pending_external_imports: Vec::new(),
        })
    }

    fn create_surface(&mut self, info: &ImageInfo) -> Result<Surface, SurfaceError> {
        match &mut self.backend {
            SurfaceBackend::Raster => raster_surface(info),
            #[cfg(all(target_os = "macos", feature = "native"))]
            SurfaceBackend::Metal { context, .. } => {
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
                Ok(surface)
            }
        }
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
            if spins < 64 {
                std::hint::spin_loop();
            } else {
                std::thread::yield_now();
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
            validate_slot(template_slot, slot, extent)?;
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
                let surface = self.create_surface(&info)?;
                let index = self.entries.len();
                self.entries.push(SurfaceEntry {
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
        let max_bytes = max_frame_bytes.min(MAX_REUSABLE_SURFACE_POOL_BYTES);
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
    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) const fn backend_kind(&self) -> SkiaBackendKind {
        self.arena.backend_kind()
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

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn release_completed_external_imports(&mut self) {
        self.arena.pending_external_imports.clear();
    }

    pub(crate) fn prepare_cpu_image(&mut self, image: &Image) -> Result<Image, SurfaceError> {
        match &mut self.arena.backend {
            SurfaceBackend::Raster => Ok(image.clone()),
            #[cfg(all(target_os = "macos", feature = "native"))]
            SurfaceBackend::Metal { context, .. } => {
                context.flush_submit_and_sync_cpu();
                image
                    .make_raster_image(Some(context), skia_safe::image::CachingHint::Disallow)
                    .ok_or(SurfaceError::Readback)
            }
        }
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn render_output_shader_to_raster(
        &mut self,
        shader: Shader,
        info: &ImageInfo,
    ) -> Result<Image, SurfaceError> {
        if !matches!(self.arena.backend, SurfaceBackend::Metal { .. }) {
            return Err(SurfaceError::SharedFrameRequiresMetal);
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
            let SurfaceBackend::Metal { context, .. } = &mut self.arena.backend else {
                return Err(SurfaceError::SharedFrameRequiresMetal);
            };
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

    #[cfg(all(target_os = "macos", feature = "native"))]
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
        Ok(ScratchSurfaces {
            frame: self,
            entries,
            bytes,
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
}

impl ScratchSurfaces<'_, '_> {
    pub(crate) fn surface_mut(&mut self, index: usize) -> Result<&mut Surface, SurfaceError> {
        self.entries
            .get_mut(index)
            .map(|entry| &mut entry.surface)
            .ok_or(SurfaceError::MissingScratchSurface { index })
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
    render_extent: Extent2d,
) -> Result<(), SurfaceError> {
    if slot.texture.extent != render_extent
        || slot.texture.format != TextureFormat::Rgba16Float
        || slot.texture.working_space != WorkingColorSpace::LinearRec2020D65
        || slot.texture.alpha != WorkingAlphaMode::PremultipliedCoverage
        || slot.texture.sample_count() != 1
        || slot.estimated_bytes
            != u64::from(render_extent.width())
                .checked_mul(u64::from(render_extent.height()))
                .and_then(|pixels| pixels.checked_mul(8))
                .ok_or(SurfaceError::ByteOverflow)?
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
    #[error("render extent does not fit the Skia coordinate domain")]
    InvalidExtent,
    #[error("Skia cannot construct Linear Rec.2020 D65")]
    UnsupportedWorkingColorSpace,
    #[error("Skia cannot construct the declared external color space")]
    UnsupportedInputColorSpace,
    #[error("Skia surface allocation failed")]
    Allocation,
    #[error("GPU working image could not be materialized for a CPU sink")]
    Readback,
    #[cfg(all(target_os = "macos", feature = "native"))]
    #[error("Metal compositor unavailable: {0}")]
    MetalUnavailable(&'static str),
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
