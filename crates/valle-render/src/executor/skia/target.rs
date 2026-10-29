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

/// Copy directly into Skia-owned storage instead of first allocating a byte Vec.
/// The returned Data owns its bytes independently of the raster frame.
#[cfg(test)]
fn copy_rgba16f_data(pixels: &[u16]) -> Data {
    #[cfg(target_endian = "little")]
    {
        // SAFETY: every initialized u16 consists of two initialized bytes. u8 has
        // alignment 1, and the borrowed view lives only until new_copy returns.
        let bytes = unsafe {
            std::slice::from_raw_parts(pixels.as_ptr().cast::<u8>(), std::mem::size_of_val(pixels))
        };
        Data::new_copy(bytes)
    }
    #[cfg(target_endian = "big")]
    {
        let bytes: Vec<_> = pixels
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        Data::new_copy(&bytes)
    }
}

pub type SkiaObjectTable = ExternalObjectTable<SkiaExternalObject>;

/// One executor-owned object. Metadata is the immutable Engine contract; the payload never enters
/// a plan, hash or WASM packet.
#[derive(Clone)]
pub struct SkiaExternalObject {
    memory: Option<valle_engine::motion::memory::MemoryLease>,
    key: ResourceKey,
    descriptor: ExternalResourceDesc,
    payload: SkiaObjectPayload,
}

#[derive(Clone)]
enum SkiaObjectPayload {
    Visual(Image),
    Svg {
        image: Image,
        picture: skia_safe::Picture,
        retained_bytes: u64,
    },
    DataTexture(Image),
    #[cfg(feature = "native")]
    DecodedVideo(Arc<valle_media::SourceFrame>),
    FontBytes(Arc<[u8]>),
    RuntimeShader(Arc<[u8]>),
    Scene3d(Image),
    #[cfg(feature = "native")]
    Scene3dFrames(Arc<super::scene3d_frames::Scene3dFrames>),
    Scene3dDof(super::scene3d_dof::Scene3dDofInput),
    #[cfg(feature = "native")]
    Scene3dGpu(Arc<super::scene3d_gpu::Scene3dGpuInput>),
}

impl core::fmt::Debug for SkiaExternalObject {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let payload = match &self.payload {
            SkiaObjectPayload::Visual(_) => "visual",
            SkiaObjectPayload::Svg { .. } => "svg",
            SkiaObjectPayload::DataTexture(_) => "dataTexture",
            #[cfg(feature = "native")]
            SkiaObjectPayload::DecodedVideo(_) => "decodedVideo",
            SkiaObjectPayload::FontBytes(_) => "fontBytes",
            SkiaObjectPayload::RuntimeShader(_) => "runtimeShader",
            SkiaObjectPayload::Scene3d(_) => "scene3d",
            #[cfg(feature = "native")]
            SkiaObjectPayload::Scene3dFrames(_) => "scene3dFrames",
            SkiaObjectPayload::Scene3dDof(_) => "scene3dDof",
            #[cfg(feature = "native")]
            SkiaObjectPayload::Scene3dGpu(_) => "scene3dGpu",
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
    pub(crate) fn charge_memory(
        &mut self,
    ) -> Result<(), valle_engine::motion::memory::MemoryError> {
        if self.memory.is_none() {
            self.memory = valle_engine::motion::memory::reserve(
                valle_engine::motion::memory::MemoryKind::Resource,
                self.resident_bytes()
                    .ok_or(valle_engine::motion::memory::MemoryError::Overflow)?,
            )?;
        }
        Ok(())
    }
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
        // A same-extent Src draw writes every pixel, alpha included, so no clear is needed.
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

    pub fn visual_svg(
        key: ResourceKey,
        layout: ExternalPixelLayout,
        extent: Extent2d,
        encoded: &[u8],
        fonts: &[&[u8]],
    ) -> Result<Self, SkiaObjectError> {
        let doc =
            valle_engine::motion::svg::compile(encoded, fonts).map_err(SkiaObjectError::Svg)?;
        if (doc.width.ceil() as u32, doc.height.ceil() as u32) != (extent.width(), extent.height())
        {
            return Err(SkiaObjectError::Svg(
                "SVG extent differs from admitted descriptor".into(),
            ));
        }
        let picture = super::svg::picture(&doc).map_err(SkiaObjectError::Svg)?;
        // The fallback image is only for consumers requiring discrete texels (e.g. atlases).
        // Ordinary images and shader children retain the resolution-independent picture.
        let image = images::deferred_from_picture(
            picture.clone(),
            (extent.width() as i32, extent.height() as i32),
            None,
            None,
            skia_safe::image::BitDepth::F16,
            working_color_space().ok(),
            None,
        )
        .ok_or(SkiaObjectError::InvalidWorkingPayload)?;
        let retained_bytes =
            encoded.len() as u64 + picture.approximate_bytes_used() as u64 + doc.raster_bytes();
        let mut object = Self::visual(key, layout, image.clone())?;
        object.payload = SkiaObjectPayload::Svg {
            image,
            picture,
            retained_bytes,
        };
        Ok(object)
    }

    pub(crate) fn svg_picture(&self) -> Option<&skia_safe::Picture> {
        match &self.payload {
            SkiaObjectPayload::Svg { picture, .. } => Some(picture),
            _ => None,
        }
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
            memory: None,
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
            memory: None,
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
        Ok(Self {
            memory: None,
            key,
            descriptor: ExternalResourceDesc::FontBytes,
            payload: SkiaObjectPayload::FontBytes(bytes.into()),
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
            memory: None,
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
            memory: None,
            key,
            descriptor: ExternalResourceDesc::Scene3d,
            payload: SkiaObjectPayload::Scene3d(image),
        })
    }

    pub(crate) fn scene3d_deferred_dof(
        key: ResourceKey,
        extent: Extent2d,
        premul_rgba16f: &[u16],
        camera_depth: &[f32],
        focus_distance: f32,
        max_blur_radius: f32,
        far: f32,
    ) -> Result<Self, SkiaObjectError> {
        if !matches!(key.interpretation, ResourceInterpretation::Scene3d { .. }) {
            return Err(SkiaObjectError::WrongInterpretation {
                expected: "scene3d",
            });
        }
        Ok(Self {
            memory: None,
            key,
            descriptor: ExternalResourceDesc::Scene3d,
            payload: SkiaObjectPayload::Scene3dDof(super::scene3d_dof::Scene3dDofInput::new(
                extent,
                premul_rgba16f,
                camera_depth,
                focus_distance,
                max_blur_radius,
                far,
            )?),
        })
    }

    pub(crate) fn deferred_scene3d_dof(&self) -> Option<&super::scene3d_dof::Scene3dDofInput> {
        match &self.payload {
            SkiaObjectPayload::Scene3dDof(input) => Some(input),
            _ => None,
        }
    }

    #[cfg(feature = "native")]
    pub(crate) fn scene3d_frames(
        key: ResourceKey,
        frame: valle_engine::motion::SealedScene3DFrame,
        resources: valle_engine::motion::scene3d::SceneResources,
    ) -> Self {
        Self {
            memory: None,
            descriptor: ExternalResourceDesc::Scene3d,
            payload: SkiaObjectPayload::Scene3dFrames(Arc::new(
                super::scene3d_frames::Scene3dFrames {
                    key: key.clone(),
                    frame,
                    resources,
                },
            )),
            key,
        }
    }

    #[cfg(feature = "native")]
    pub(crate) fn deferred_scene3d_frames(
        &self,
    ) -> Option<&Arc<super::scene3d_frames::Scene3dFrames>> {
        match &self.payload {
            SkiaObjectPayload::Scene3dFrames(input) => Some(input),
            _ => None,
        }
    }

    pub(crate) fn is_deferred_scene3d(&self) -> bool {
        match &self.payload {
            SkiaObjectPayload::Scene3dDof(_) => true,
            #[cfg(feature = "native")]
            SkiaObjectPayload::Scene3dGpu(_) => true,
            _ => false,
        }
    }

    #[cfg(feature = "native")]
    pub(crate) fn scene3d_gpu(
        key: ResourceKey,
        geometry: valle_engine::motion::scene3d::GpuRasterGeometry,
        prepared: Arc<valle_engine::motion::scene3d::PreparedScene>,
        frame: Arc<valle_engine::motion::scene3d::Frame3DState>,
    ) -> Result<Self, SkiaObjectError> {
        if !matches!(key.interpretation, ResourceInterpretation::Scene3d { .. }) {
            return Err(SkiaObjectError::WrongInterpretation {
                expected: "scene3d",
            });
        }
        let input = super::scene3d_gpu::Scene3dGpuInput::new(geometry, prepared, frame)?;
        Ok(Self {
            memory: None,
            key,
            descriptor: ExternalResourceDesc::Scene3d,
            payload: SkiaObjectPayload::Scene3dGpu(Arc::new(input)),
        })
    }

    #[cfg(feature = "native")]
    pub(crate) fn deferred_scene3d_gpu(&self) -> Option<&super::scene3d_gpu::Scene3dGpuInput> {
        match &self.payload {
            SkiaObjectPayload::Scene3dGpu(input) => Some(input),
            _ => None,
        }
    }

    #[cfg(feature = "native")]
    pub(crate) fn scene3d_gpu_fallback(&self) -> Result<Self, SkiaObjectError> {
        let input = self
            .deferred_scene3d_gpu()
            .ok_or(SkiaObjectError::InvalidWorkingPayload)?;
        let _working = valle_engine::motion::memory::reserve(
            valle_engine::motion::memory::MemoryKind::Temporary,
            u64::from(input.geometry.width) * u64::from(input.geometry.height) * 96,
        )
        .map_err(|error| SkiaObjectError::Scene3dFallback(error.to_string()))?;
        let raster = valle_engine::motion::scene3d::render_scene(&input.prepared, &input.frame)
            .map_err(|error| SkiaObjectError::Scene3dFallback(error.to_string()))?;
        let extent = Extent2d::new(raster.width, raster.height)
            .map_err(|_| SkiaObjectError::InvalidExtent)?;
        Self::scene3d_rgba16f(self.key.clone(), extent, &raster.premul_rgba16f)
    }

    /// Normalize premultiplied sRGB half-float Scene3D pixels into the working color space.
    pub fn scene3d_rgba16f(
        key: ResourceKey,
        extent: Extent2d,
        premul_rgba: &[u16],
    ) -> Result<Self, SkiaObjectError> {
        if !matches!(key.interpretation, ResourceInterpretation::Scene3d { .. }) {
            return Err(SkiaObjectError::WrongInterpretation {
                expected: "scene3d",
            });
        }
        let width = usize::try_from(extent.width()).map_err(|_| SkiaObjectError::InvalidExtent)?;
        let height =
            usize::try_from(extent.height()).map_err(|_| SkiaObjectError::InvalidExtent)?;
        width.checked_mul(8).ok_or(SkiaObjectError::InvalidExtent)?;
        if premul_rgba.len()
            != width
                .checked_mul(height)
                .and_then(|pixels| pixels.checked_mul(4))
                .ok_or(SkiaObjectError::InvalidExtent)?
        {
            return Err(SkiaObjectError::InvalidPixelPayload);
        }
        let working_info =
            working_info(extent).map_err(|_| SkiaObjectError::InvalidWorkingPayload)?;
        let mut surface =
            raster_surface(&working_info).map_err(|_| SkiaObjectError::InvalidWorkingPayload)?;
        // Raster surfaces start zeroed. Convert only nonzero source pixels; retain
        // the full logical extent so later placement and filtering are unchanged.
        let Some(region) = super::scene3d_region::Region::rgba(premul_rgba, width) else {
            return Self::scene3d(key, surface.image_snapshot());
        };
        let input_info = ImageInfo::new(
            (region.width() as i32, region.height() as i32),
            ColorType::RGBAF16,
            AlphaType::Premul,
            Some(
                input_color_space(valle_engine::resource::ColorDescription::SRGB)
                    .map_err(|_| SkiaObjectError::InvalidInputColorSpace)?,
            ),
        );
        let input = images::raster_from_data(
            &input_info,
            region.color_data(premul_rgba, width),
            region.width() * 8,
        )
        .ok_or(SkiaObjectError::InvalidPixelPayload)?;
        let mut paint = Paint::default();
        paint.set_blend_mode(BlendMode::Src);
        surface
            .canvas()
            .draw_image(&input, (region.x0 as f32, region.y0 as f32), Some(&paint));
        if std::env::var_os("VALLE_PERF_SCENE3D").is_some() {
            eprintln!(
                "[valle scene3d-region] cpu-convert pixels={} full={}",
                region.pixels(),
                width * height
            );
        }
        Self::scene3d(key, surface.image_snapshot())
    }

    pub fn data_texture_encoded(
        key: ResourceKey,
        extent: Extent2d,
        encoded: &[u8],
    ) -> Result<Self, SkiaObjectError> {
        if !matches!(key.interpretation, ResourceInterpretation::DataTexture {}) {
            return Err(SkiaObjectError::WrongInterpretation {
                expected: "dataTexture",
            });
        }
        let packed = valle_engine::motion::shader::decode_data_texture(
            encoded,
            extent.width(),
            extent.height(),
        )
        .map_err(SkiaObjectError::DataTexture)?;
        let info = ImageInfo::new(
            ((extent.width() * 2) as i32, (extent.height() * 4) as i32),
            ColorType::Alpha8,
            AlphaType::Premul,
            None,
        );
        let image =
            images::raster_from_data(&info, Data::new_copy(&packed), extent.width() as usize * 2)
                .ok_or(SkiaObjectError::InvalidPixelPayload)?;
        Ok(Self {
            memory: None,
            key,
            descriptor: ExternalResourceDesc::DataTexture { extent },
            payload: SkiaObjectPayload::DataTexture(image),
        })
    }

    pub(crate) fn data_image(&self) -> Option<&Image> {
        match &self.payload {
            SkiaObjectPayload::DataTexture(image) => Some(image),
            _ => None,
        }
    }

    pub(crate) fn visual_image(&self) -> Option<&Image> {
        match &self.payload {
            SkiaObjectPayload::Visual(image)
            | SkiaObjectPayload::Scene3d(image)
            | SkiaObjectPayload::Svg { image, .. } => Some(image),
            #[cfg(feature = "native")]
            SkiaObjectPayload::DecodedVideo(_) => None,
            #[cfg(feature = "native")]
            SkiaObjectPayload::Scene3dGpu(_) | SkiaObjectPayload::Scene3dFrames(_) => None,
            SkiaObjectPayload::Scene3dDof(_)
            | SkiaObjectPayload::DataTexture(_)
            | SkiaObjectPayload::FontBytes(_)
            | SkiaObjectPayload::RuntimeShader(_) => None,
        }
    }

    pub(crate) fn is_deferred_video(&self) -> bool {
        #[cfg(feature = "native")]
        if matches!(self.payload, SkiaObjectPayload::DecodedVideo(_)) {
            return true;
        }
        false
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

    pub(crate) fn resident_bytes(&self) -> Option<u64> {
        match &self.payload {
            #[cfg(feature = "native")]
            SkiaObjectPayload::Scene3dGpu(input) => input.resident_bytes(),
            #[cfg(feature = "native")]
            SkiaObjectPayload::Scene3dFrames(input) => Some(input.frame.canonical().len() as u64),
            SkiaObjectPayload::Scene3dDof(input) => Some(input.resident_bytes()),
            SkiaObjectPayload::Svg {
                image,
                retained_bytes,
                ..
            } => retained_bytes.checked_add(image.width() as u64 * image.height() as u64 * 8),
            SkiaObjectPayload::Visual(image)
            | SkiaObjectPayload::Scene3d(image)
            | SkiaObjectPayload::DataTexture(image) => {
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

    #[cfg(all(
        feature = "native",
        any(target_os = "macos", target_os = "linux", target_os = "windows")
    ))]
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

    #[cfg(all(
        feature = "native",
        any(target_os = "macos", target_os = "linux", target_os = "windows")
    ))]
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
    #[error("SVG resource: {0}")]
    Svg(String),
    #[cfg(feature = "native")]
    #[error("Scene3D CPU fallback failed: {0}")]
    Scene3dFallback(String),
    #[error("invalid data texture: {0}")]
    DataTexture(String),
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

#[cfg(all(test, feature = "native"))]
mod contract_tests {
    use super::*;
    use valle_engine::resource::ContentDigest;

    #[test]
    fn compact_scene3d_conversion_matches_full_frame_half_float_bits() {
        for (width, height) in [(1, 1), (37, 23), (128, 65)] {
            let extent = Extent2d::new(width, height).unwrap();
            let key = ResourceKey::new(
                ContentDigest::of_bytes(b"region"),
                ResourceInterpretation::Scene3d {
                    topology_digest: ContentDigest::of_bytes(b"topology"),
                },
            );
            for placement in 0..4 {
                let mut pixels = vec![0_u16; (width * height * 4) as usize];
                for y in 0..height {
                    for x in 0..width {
                        let covered = match placement {
                            0 => false,
                            1 => {
                                x >= width / 3
                                    && x < (width * 2 / 3).max(1)
                                    && y >= height / 3
                                    && y < (height * 2 / 3).max(1)
                            }
                            2 => x + 1 == width || y + 1 == height,
                            _ => true,
                        };
                        if covered {
                            let color = if (x + y) % 2 == 0 {
                                [0x3400, 0x3000, 0x3555, 0x3800]
                            } else {
                                [0x3bff, 0x0001, 0x3555, 0x3c00]
                            };
                            pixels[((y * width + x) * 4) as usize
                                ..((y * width + x) * 4 + 4) as usize]
                                .copy_from_slice(&color);
                        }
                    }
                }
                let input_info = ImageInfo::new(
                    (width as i32, height as i32),
                    ColorType::RGBAF16,
                    AlphaType::Premul,
                    Some(
                        input_color_space(valle_engine::resource::ColorDescription::SRGB).unwrap(),
                    ),
                );
                let input = images::raster_from_data(
                    &input_info,
                    copy_rgba16f_data(&pixels),
                    width as usize * 8,
                )
                .unwrap();
                let info = working_info(extent).unwrap();
                let mut expected = raster_surface(&info).unwrap();
                let mut paint = Paint::default();
                paint.set_blend_mode(BlendMode::Src);
                expected
                    .canvas()
                    .draw_image(&input, (0.0, 0.0), Some(&paint));
                let actual =
                    SkiaExternalObject::scene3d_rgba16f(key.clone(), extent, &pixels).unwrap();
                let read = |image: &Image| {
                    let mut bytes = vec![0; (width * height * 8) as usize];
                    assert!(image.read_pixels(
                        &info,
                        &mut bytes,
                        width as usize * 8,
                        (0, 0),
                        CachingHint::Disallow
                    ));
                    bytes
                };
                assert_eq!(
                    read(actual.visual_image().unwrap()),
                    read(&expected.image_snapshot()),
                    "{width}x{height}, placement {placement}"
                );
            }
        }
    }

    #[test]
    fn half_float_upload_owns_an_exact_copy_of_the_source() {
        let data = {
            let mut pixels = [0, 0x3c00, 0x3555, 0x0001];
            let data = copy_rgba16f_data(&pixels);
            pixels.fill(0xffff);
            data
        };
        assert_eq!(data.as_bytes(), &[0, 0, 0, 0x3c, 0x55, 0x35, 1, 0]);
        assert!(copy_rgba16f_data(&[]).is_empty());
    }

    #[test]
    fn resource_payload_accessors_and_resident_accounting_follow_the_admitted_kind() {
        let bytes = include_bytes!("../../../../../assets/fonts/noto/NotoSans-Regular.ttf");
        let key = ResourceKey::new(
            ContentDigest::of_bytes(bytes),
            ResourceInterpretation::FontFace { face_index: 0 },
        );
        let font = SkiaExternalObject::font_bytes(key.clone(), bytes.as_slice()).unwrap();
        let diagnostic = format!("{font:?}");
        assert!(diagnostic.contains("fontBytes"));
        assert!(
            diagnostic.len() < 512,
            "diagnostics must not dump font bytes"
        );
        assert_eq!(font.font_data().unwrap(), bytes);
        assert!(font.visual_image().is_none());
        assert!(font.shader_data().is_none());
        assert!(font.data_image().is_none());
        assert_eq!(font.resident_bytes(), Some(bytes.len() as u64));
        assert!(font.decoded_raster_image().is_err());
        assert!(
            SkiaExternalObject::visual_rgba8(key.clone(), Extent2d::new(1, 1).unwrap(), &[0; 4])
                .is_err()
        );
        assert!(
            SkiaExternalObject::visual_encoded(
                key.clone(),
                ExternalPixelLayout::Rgba8,
                Extent2d::new(1, 1).unwrap(),
                b"bad"
            )
            .is_err()
        );
        assert!(
            SkiaExternalObject::data_texture_encoded(key, Extent2d::new(1, 1).unwrap(), b"bad")
                .is_err()
        );
        let key = ResourceKey::new(
            ContentDigest::of_bytes(b"data"),
            ResourceInterpretation::DataTexture {},
        );
        assert!(SkiaExternalObject::font_bytes(key.clone(), bytes.as_slice()).is_err());
        assert!(
            SkiaExternalObject::data_texture_encoded(key, Extent2d::new(1, 1).unwrap(), b"bad")
                .is_err()
        );
    }
}
