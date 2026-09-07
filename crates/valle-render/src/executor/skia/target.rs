use std::sync::Arc;

use skia_safe::{
    AlphaType, BlendMode, Color4f, ColorType, Data, Image, ImageInfo, Paint, Shader, Surface,
    image::CachingHint, images,
};
use thiserror::Error;
use valle_engine::{
    compositor::{ExternalObject, ExternalObjectTable},
    resource::{
        Extent2d, ExternalPixelLayout, ExternalResourceDesc, InputAlphaMode, OutputSpec,
        ResourceInterpretation, ResourceKey,
    },
};

use super::surface::{input_color_space, raster_surface, working_color_space, working_info};

pub type SkiaObjectTable = ExternalObjectTable<SkiaExternalObject>;

/// One executor-owned object. Metadata is the immutable Engine contract; the payload never enters
/// a plan, hash or WASM packet.
#[derive(Clone)]
pub struct SkiaExternalObject {
    key: ResourceKey,
    descriptor: ExternalResourceDesc,
    payload: SkiaObjectPayload,
}

#[derive(Clone)]
enum SkiaObjectPayload {
    Visual(Image),
    #[cfg(feature = "native")]
    DecodedVideo(Arc<valle_media::SourceFrame>),
    FontBytes(Arc<[u8]>),
    RuntimeShader(Arc<[u8]>),
    Scene3d(Image),
}

impl core::fmt::Debug for SkiaExternalObject {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let payload = match &self.payload {
            SkiaObjectPayload::Visual(_) => "visual",
            #[cfg(feature = "native")]
            SkiaObjectPayload::DecodedVideo(_) => "decodedVideo",
            SkiaObjectPayload::FontBytes(_) => "fontBytes",
            SkiaObjectPayload::RuntimeShader(_) => "runtimeShader",
            SkiaObjectPayload::Scene3d(_) => "scene3d",
        };
        formatter
            .debug_struct("SkiaExternalObject")
            .field("key", &self.key)
            .field("descriptor", &self.descriptor)
            .field("payload", &payload)
            .finish()
    }
}

impl SkiaExternalObject {
    /// Normalize a tightly packed decoded RGBA8 frame into the compositor working domain. The
    /// declared transfer/primaries/alpha are applied exactly once here; crop/orientation/SAR stay
    /// in the Engine placement contract and are not baked into the resource.
    pub fn visual_rgba8(
        key: ResourceKey,
        extent: Extent2d,
        rgba: &[u8],
    ) -> Result<Self, SkiaObjectError> {
        Self::visual_rgba8_as(key, ExternalPixelLayout::Rgba8, extent, rgba)
    }

    /// Normalize decoded RGBA8 while retaining the layout declared by the fulfillment request.
    /// A decoder may expose NV12/P010 to the host and still use its deterministic CPU RGBA path
    /// for this Skia executor; the object-table descriptor describes the admitted external object
    /// contract, while the private payload is always normalized working-linear pixels.
    pub fn visual_rgba8_as(
        key: ResourceKey,
        declared_layout: ExternalPixelLayout,
        extent: Extent2d,
        rgba: &[u8],
    ) -> Result<Self, SkiaObjectError> {
        let ResourceInterpretation::Visual { interpretation } = &key.interpretation else {
            return Err(SkiaObjectError::WrongInterpretation { expected: "visual" });
        };
        let interpretation = *interpretation;
        let width = usize::try_from(extent.width()).map_err(|_| SkiaObjectError::InvalidExtent)?;
        let height =
            usize::try_from(extent.height()).map_err(|_| SkiaObjectError::InvalidExtent)?;
        let row_bytes = width.checked_mul(4).ok_or(SkiaObjectError::InvalidExtent)?;
        if rgba.len()
            != row_bytes
                .checked_mul(height)
                .ok_or(SkiaObjectError::InvalidExtent)?
        {
            return Err(SkiaObjectError::InvalidPixelPayload);
        }
        let alpha = match interpretation.alpha {
            InputAlphaMode::Opaque => AlphaType::Opaque,
            InputAlphaMode::StraightCoverage => AlphaType::Unpremul,
            InputAlphaMode::PremultipliedCoverage => AlphaType::Premul,
        };
        let input_info = ImageInfo::new(
            (
                i32::try_from(extent.width()).map_err(|_| SkiaObjectError::InvalidExtent)?,
                i32::try_from(extent.height()).map_err(|_| SkiaObjectError::InvalidExtent)?,
            ),
            ColorType::RGBA8888,
            alpha,
            Some(
                input_color_space(interpretation.color)
                    .map_err(|_| SkiaObjectError::InvalidInputColorSpace)?,
            ),
        );
        let input = images::raster_from_data(&input_info, Data::new_copy(rgba), row_bytes)
            .ok_or(SkiaObjectError::InvalidPixelPayload)?;
        let working_info =
            working_info(extent).map_err(|_| SkiaObjectError::InvalidWorkingPayload)?;
        let mut surface =
            raster_surface(&working_info).map_err(|_| SkiaObjectError::InvalidWorkingPayload)?;
        surface.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
        let mut paint = Paint::default();
        paint.set_blend_mode(BlendMode::Src);
        surface
            .canvas()
            .draw_image(&input, (0.0, 0.0), Some(&paint));
        Self::visual(key, declared_layout, surface.image_snapshot())
    }

    /// Decode an encoded still image into the declared input color/alpha interpretation, then
    /// normalize it through the same working-space path as video frames.
    pub fn visual_encoded(
        key: ResourceKey,
        declared_layout: ExternalPixelLayout,
        extent: Extent2d,
        encoded: &[u8],
    ) -> Result<Self, SkiaObjectError> {
        let ResourceInterpretation::Visual { interpretation } = &key.interpretation else {
            return Err(SkiaObjectError::WrongInterpretation { expected: "visual" });
        };
        let decoded = images::deferred_from_encoded_data(Data::new_copy(encoded), None)
            .ok_or(SkiaObjectError::InvalidEncodedPayload)?;
        if decoded.width() != i32::try_from(extent.width()).unwrap_or(-1)
            || decoded.height() != i32::try_from(extent.height()).unwrap_or(-1)
        {
            return Err(SkiaObjectError::EncodedExtentMismatch {
                expected_width: extent.width(),
                expected_height: extent.height(),
                actual_width: decoded.width(),
                actual_height: decoded.height(),
            });
        }
        let info = ImageInfo::new(
            (decoded.width(), decoded.height()),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            Some(
                input_color_space(interpretation.color)
                    .map_err(|_| SkiaObjectError::InvalidInputColorSpace)?,
            ),
        );
        let row_bytes = usize::try_from(extent.width())
            .ok()
            .and_then(|width| width.checked_mul(4))
            .ok_or(SkiaObjectError::InvalidExtent)?;
        let byte_len = row_bytes
            .checked_mul(
                usize::try_from(extent.height()).map_err(|_| SkiaObjectError::InvalidExtent)?,
            )
            .ok_or(SkiaObjectError::InvalidExtent)?;
        let mut rgba = vec![0_u8; byte_len];
        if !decoded.read_pixels(&info, &mut rgba, row_bytes, (0, 0), CachingHint::Disallow) {
            return Err(SkiaObjectError::InvalidEncodedPayload);
        }
        Self::visual_rgba8_as(key, declared_layout, extent, &rgba)
    }

    pub fn visual(
        key: ResourceKey,
        pixel_layout: ExternalPixelLayout,
        image: Image,
    ) -> Result<Self, SkiaObjectError> {
        if !matches!(key.interpretation, ResourceInterpretation::Visual { .. }) {
            return Err(SkiaObjectError::WrongInterpretation { expected: "visual" });
        }
        let width = u32::try_from(image.width()).map_err(|_| SkiaObjectError::InvalidExtent)?;
        let height = u32::try_from(image.height()).map_err(|_| SkiaObjectError::InvalidExtent)?;
        let extent = Extent2d::new(width, height).map_err(|_| SkiaObjectError::InvalidExtent)?;
        validate_working_image(&image)?;
        Ok(Self {
            key,
            descriptor: ExternalResourceDesc::VisualFrame {
                extent,
                pixel_layout,
            },
            payload: SkiaObjectPayload::Visual(image),
        })
    }

    /// Retain one decoder-owned platform frame without forcing CPU conversion. The executor may
    /// import it into its matching GPU backend; a non-GPU path uses the source's memoized RGBA
    /// fallback before target admission.
    #[cfg(feature = "native")]
    pub fn visual_decoded(
        key: ResourceKey,
        pixel_layout: ExternalPixelLayout,
        extent: Extent2d,
        source: Arc<valle_media::SourceFrame>,
    ) -> Result<Self, SkiaObjectError> {
        if !matches!(key.interpretation, ResourceInterpretation::Visual { .. }) {
            return Err(SkiaObjectError::WrongInterpretation { expected: "visual" });
        }
        let decoded = source
            .decoded_gpu()
            .ok_or(SkiaObjectError::InvalidDecodedGpuPayload)?;
        if decoded.dimensions() != (extent.width(), extent.height()) {
            return Err(SkiaObjectError::DecodedGpuExtentMismatch {
                expected_width: extent.width(),
                expected_height: extent.height(),
                actual_width: decoded.dimensions().0,
                actual_height: decoded.dimensions().1,
            });
        }
        Ok(Self {
            key,
            descriptor: ExternalResourceDesc::VisualFrame {
                extent,
                pixel_layout,
            },
            payload: SkiaObjectPayload::DecodedVideo(source),
        })
    }

    pub fn font_bytes(
        key: ResourceKey,
        bytes: impl Into<Arc<[u8]>>,
    ) -> Result<Self, SkiaObjectError> {
        let ResourceInterpretation::FontFace { .. } = key.interpretation else {
            return Err(SkiaObjectError::WrongInterpretation {
                expected: "fontFace",
            });
        };
        let bytes = bytes.into();
        let actual = valle_engine::resource::ContentDigest::of_bytes(bytes.as_ref());
        if actual != key.content {
            return Err(SkiaObjectError::DigestMismatch {
                expected: key.content.to_string(),
                actual: actual.to_string(),
            });
        }
        Ok(Self {
            key,
            descriptor: ExternalResourceDesc::FontBytes,
            payload: SkiaObjectPayload::FontBytes(bytes),
        })
    }

    /// Construct from the runtime record produced by the single Motion shader admission path.
    /// The package content digest and generated SkSL digest are intentionally different values:
    /// identity is proven by the admitted record, while SkSL is the backend payload compiled when
    /// the complete frame is admitted.
    pub fn runtime_shader(
        key: ResourceKey,
        admitted_content_hash: valle_engine::resource::ContentDigest,
        admitted_abi_hash: valle_engine::resource::ContentDigest,
        generated_sksl: impl Into<Arc<[u8]>>,
    ) -> Result<Self, SkiaObjectError> {
        let ResourceInterpretation::RuntimeShader { abi_digest, .. } = &key.interpretation else {
            return Err(SkiaObjectError::WrongInterpretation {
                expected: "runtimeShader",
            });
        };
        if admitted_content_hash != key.content {
            return Err(SkiaObjectError::DigestMismatch {
                expected: key.content.to_string(),
                actual: admitted_content_hash.to_string(),
            });
        }
        if admitted_abi_hash != *abi_digest {
            return Err(SkiaObjectError::AbiMismatch {
                expected: abi_digest.to_string(),
                actual: admitted_abi_hash.to_string(),
            });
        }
        let generated_sksl = generated_sksl.into();
        if generated_sksl.is_empty() {
            return Err(SkiaObjectError::EmptyShaderPayload);
        }
        Ok(Self {
            key,
            descriptor: ExternalResourceDesc::RuntimeShader,
            payload: SkiaObjectPayload::RuntimeShader(generated_sksl),
        })
    }

    /// Exact frame-local Scene3D raster admitted for this typed request. It cannot be supplied
    /// through an ordinary visual-texture slot.
    pub fn scene3d(key: ResourceKey, image: Image) -> Result<Self, SkiaObjectError> {
        if !matches!(key.interpretation, ResourceInterpretation::Scene3d { .. }) {
            return Err(SkiaObjectError::WrongInterpretation {
                expected: "scene3d",
            });
        }
        if image.width() <= 0 || image.height() <= 0 {
            return Err(SkiaObjectError::InvalidExtent);
        }
        validate_working_image(&image)?;
        Ok(Self {
            key,
            descriptor: ExternalResourceDesc::Scene3d,
            payload: SkiaObjectPayload::Scene3d(image),
        })
    }

    /// Normalize the deterministic software Scene3D raster. The Scene3D v1 raster contract emits
    /// premultiplied sRGB RGBA8; all later drawing sees only Linear Rec.2020 premultiplied pixels.
    pub fn scene3d_rgba8(
        key: ResourceKey,
        extent: Extent2d,
        premul_rgba: &[u8],
    ) -> Result<Self, SkiaObjectError> {
        if !matches!(key.interpretation, ResourceInterpretation::Scene3d { .. }) {
            return Err(SkiaObjectError::WrongInterpretation {
                expected: "scene3d",
            });
        }
        let width = usize::try_from(extent.width()).map_err(|_| SkiaObjectError::InvalidExtent)?;
        let height =
            usize::try_from(extent.height()).map_err(|_| SkiaObjectError::InvalidExtent)?;
        let row_bytes = width.checked_mul(4).ok_or(SkiaObjectError::InvalidExtent)?;
        if premul_rgba.len()
            != row_bytes
                .checked_mul(height)
                .ok_or(SkiaObjectError::InvalidExtent)?
        {
            return Err(SkiaObjectError::InvalidPixelPayload);
        }
        let input_info = ImageInfo::new(
            (
                i32::try_from(extent.width()).map_err(|_| SkiaObjectError::InvalidExtent)?,
                i32::try_from(extent.height()).map_err(|_| SkiaObjectError::InvalidExtent)?,
            ),
            ColorType::RGBA8888,
            AlphaType::Premul,
            Some(
                input_color_space(valle_engine::resource::ColorDescription::SRGB)
                    .map_err(|_| SkiaObjectError::InvalidInputColorSpace)?,
            ),
        );
        let input = images::raster_from_data(&input_info, Data::new_copy(premul_rgba), row_bytes)
            .ok_or(SkiaObjectError::InvalidPixelPayload)?;
        let working_info =
            working_info(extent).map_err(|_| SkiaObjectError::InvalidWorkingPayload)?;
        let mut surface =
            raster_surface(&working_info).map_err(|_| SkiaObjectError::InvalidWorkingPayload)?;
        surface.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
        let mut paint = Paint::default();
        paint.set_blend_mode(BlendMode::Src);
        surface
            .canvas()
            .draw_image(&input, (0.0, 0.0), Some(&paint));
        Self::scene3d(key, surface.image_snapshot())
    }

    pub(crate) fn visual_image(&self) -> Option<&Image> {
        match &self.payload {
            SkiaObjectPayload::Visual(image) | SkiaObjectPayload::Scene3d(image) => Some(image),
            #[cfg(feature = "native")]
            SkiaObjectPayload::DecodedVideo(_) => None,
            SkiaObjectPayload::FontBytes(_) | SkiaObjectPayload::RuntimeShader(_) => None,
        }
    }

    #[cfg(feature = "native")]
    pub(crate) fn decoded_video(&self) -> Option<&Arc<valle_media::SourceFrame>> {
        match &self.payload {
            SkiaObjectPayload::DecodedVideo(source) => Some(source),
            _ => None,
        }
    }

    #[cfg(feature = "native")]
    pub(crate) fn decoded_raster_image(&self) -> Result<Image, SkiaObjectError> {
        let source = self
            .decoded_video()
            .ok_or(SkiaObjectError::InvalidDecodedGpuPayload)?;
        let ExternalResourceDesc::VisualFrame {
            extent,
            pixel_layout,
        } = self.descriptor
        else {
            return Err(SkiaObjectError::InvalidDecodedGpuPayload);
        };
        let rgba = source
            .rgba()
            .map_err(|error| SkiaObjectError::DecodedGpuFallback(error.to_string()))?;
        let object = Self::visual_rgba8_as(self.key.clone(), pixel_layout, extent, &rgba.data)?;
        object
            .visual_image()
            .cloned()
            .ok_or(SkiaObjectError::InvalidDecodedGpuPayload)
    }

    pub(crate) fn font_data(&self) -> Option<&[u8]> {
        match &self.payload {
            SkiaObjectPayload::FontBytes(bytes) => Some(bytes),
            _ => None,
        }
    }

    pub(crate) fn shader_data(&self) -> Option<&[u8]> {
        match &self.payload {
            SkiaObjectPayload::RuntimeShader(bytes) => Some(bytes),
            _ => None,
        }
    }

    #[cfg(feature = "native")]
    pub(crate) fn resident_bytes(&self) -> Option<u64> {
        match &self.payload {
            SkiaObjectPayload::Visual(image) | SkiaObjectPayload::Scene3d(image) => {
                let info = image.image_info();
                u64::try_from(info.width())
                    .ok()?
                    .checked_mul(u64::try_from(info.height()).ok()?)?
                    .checked_mul(u64::try_from(info.color_type().bytes_per_pixel()).ok()?)
            }
            #[cfg(feature = "native")]
            SkiaObjectPayload::DecodedVideo(_) => match self.descriptor {
                ExternalResourceDesc::VisualFrame { extent, .. } => u64::from(extent.width())
                    .checked_mul(u64::from(extent.height()))?
                    .checked_mul(4),
                _ => None,
            },
            SkiaObjectPayload::FontBytes(bytes) | SkiaObjectPayload::RuntimeShader(bytes) => {
                u64::try_from(bytes.len()).ok()
            }
        }
    }
}

fn validate_working_image(image: &Image) -> Result<(), SkiaObjectError> {
    let info = image.image_info();
    let expected = working_color_space().map_err(|_| SkiaObjectError::InvalidWorkingPayload)?;
    if !matches!(info.color_type(), ColorType::RGBAF16 | ColorType::RGBAF32)
        || info.alpha_type() != AlphaType::Premul
        || info.color_space().as_ref() != Some(&expected)
    {
        return Err(SkiaObjectError::InvalidWorkingPayload);
    }
    Ok(())
}

impl ExternalObject for SkiaExternalObject {
    fn key(&self) -> &ResourceKey {
        &self.key
    }

    fn descriptor(&self) -> &ExternalResourceDesc {
        &self.descriptor
    }
}

/// Transactional product target. Execution renders into private storage and touches this surface
/// only once, after every pass has succeeded.
pub struct SkiaTarget<'a> {
    surface: &'a mut Surface,
    extent: Extent2d,
    output: OutputSpec,
    direct_gpu: bool,
}

impl<'a> SkiaTarget<'a> {
    pub fn new(surface: &'a mut Surface, output: OutputSpec) -> Result<Self, SkiaTargetError> {
        let width = u32::try_from(surface.width()).map_err(|_| SkiaTargetError::InvalidExtent)?;
        let height = u32::try_from(surface.height()).map_err(|_| SkiaTargetError::InvalidExtent)?;
        let extent = Extent2d::new(width, height).map_err(|_| SkiaTargetError::InvalidExtent)?;
        Ok(Self {
            surface,
            extent,
            output,
            direct_gpu: false,
        })
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn direct_gpu(
        surface: &'a mut Surface,
        output: OutputSpec,
    ) -> Result<Self, SkiaTargetError> {
        let mut target = Self::new(surface, output)?;
        target.direct_gpu = true;
        Ok(target)
    }

    pub const fn extent(&self) -> Extent2d {
        self.extent
    }

    pub const fn output(&self) -> OutputSpec {
        self.output
    }

    pub(crate) fn image_info(&self) -> skia_safe::ImageInfo {
        self.surface.image_info()
    }

    pub(crate) const fn is_direct_gpu(&self) -> bool {
        self.direct_gpu
    }

    pub(crate) fn commit_shader(&mut self, shader: Shader) -> bool {
        let canvas = self.surface.canvas();
        canvas.clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
        let mut paint = Paint::default();
        paint.set_blend_mode(BlendMode::Src);
        paint.set_shader(shader);
        canvas.draw_paint(&paint);
        true
    }

    pub(crate) fn commit_image(&mut self, image: &Image) -> bool {
        if *image.image_info() != self.surface.image_info() {
            return false;
        }
        let canvas = self.surface.canvas();
        canvas.clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
        let mut paint = Paint::default();
        paint.set_blend_mode(BlendMode::Src);
        canvas.draw_image(image, (0.0, 0.0), Some(&paint));
        true
    }

    pub(crate) fn commit_pixels(&mut self, pixels: &[u8], row_bytes: usize) -> bool {
        let info = self.surface.image_info();
        self.surface
            .canvas()
            .write_pixels(&info, pixels, row_bytes, (0, 0))
    }
}

#[derive(Debug, Error)]
pub enum SkiaObjectError {
    #[error("external object requires {expected} interpretation")]
    WrongInterpretation { expected: &'static str },
    #[error("external object has an invalid pixel extent")]
    InvalidExtent,
    #[error("visual payload must already be Linear Rec.2020 RGBA16F/32F premultiplied")]
    InvalidWorkingPayload,
    #[error("decoded RGBA byte length does not match its declared extent")]
    InvalidPixelPayload,
    #[error("encoded visual payload cannot be decoded")]
    InvalidEncodedPayload,
    #[cfg(feature = "native")]
    #[error("decoded GPU visual payload has no platform surface")]
    InvalidDecodedGpuPayload,
    #[cfg(feature = "native")]
    #[error("decoded GPU visual CPU fallback failed: {0}")]
    DecodedGpuFallback(String),
    #[cfg(feature = "native")]
    #[error(
        "decoded GPU visual extent mismatch: expected {expected_width}x{expected_height}, got {actual_width}x{actual_height}"
    )]
    DecodedGpuExtentMismatch {
        expected_width: u32,
        expected_height: u32,
        actual_width: u32,
        actual_height: u32,
    },
    #[error(
        "encoded visual extent mismatch: expected {expected_width}x{expected_height}, got {actual_width}x{actual_height}"
    )]
    EncodedExtentMismatch {
        expected_width: u32,
        expected_height: u32,
        actual_width: i32,
        actual_height: i32,
    },
    #[error("declared external input color space is not supported by Skia")]
    InvalidInputColorSpace,
    #[error("external object digest mismatch: expected {expected}, got {actual}")]
    DigestMismatch { expected: String, actual: String },
    #[error("runtime shader ABI mismatch: expected {expected}, got {actual}")]
    AbiMismatch { expected: String, actual: String },
    #[error("runtime shader generated SkSL payload is empty")]
    EmptyShaderPayload,
}

#[derive(Debug, Error)]
pub enum SkiaTargetError {
    #[error("Skia target must have a finite positive extent")]
    InvalidExtent,
}
