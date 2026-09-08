use std::{collections::BTreeMap, sync::Arc};

use skia_safe::{
    BlendMode, ClipOp, Color4f, ColorChannel, Data, Image, ImageFilter, ImageInfo, Matrix, Paint,
    Rect as SkRect, RuntimeEffect, SamplingOptions, Shader, Surface, TileMode, color_filters,
    image_filters, runtime_effect::ChildPtr, shaders,
};
use valle_draw::program::{Filter, LinearColor};
use valle_engine::{
    compositor::lower::{
        CopyOperation, ExecutionPassKind, KernelInvocation, PlanEffect, RenderBindings,
        RenderPlanTemplate, SurfaceSlotId,
    },
    prepare::{
        DeviceRect, DeviceTransform, DynamicBindingId, DynamicBindingKind, DynamicValue,
        PreparedBlurAxis, PreparedEffectKernel, PreparedEffectSpace, PreparedMask,
        PreparedMaskShape, PreparedTransitionKernel, PreparedUnitRect,
    },
    render::{
        EXTENSION_COLOR_GAIN_ABI, EXTENSION_CROSS_FADE_ABI,
        engine_owned_kernel_implementation_sha256,
    },
    resource::{
        ColorPrimaries, Dither, GamutMap, OutputAlphaMode, OutputBitDepth, OutputSpec, ToneMap,
        TransferFunction,
    },
};

use super::{draw::DrawError, surface::ScratchSurfaces};

const DIRECTIONAL_BLUR: &str =
    include_str!("../../../../valle-draw/assets/shaders/directionalblur.sksl");
const EXTENSION_COLOR_GAIN: &str =
    include_str!("../../../../valle-draw/assets/shaders/extensioncolorgain.sksl");
const OUTPUT_TRANSFORM: &str =
    include_str!("../../../../valle-draw/assets/shaders/outputtransform.sksl");

#[derive(Debug, Clone)]
pub(crate) struct EffectRuntime {
    transitions: BTreeMap<PreparedTransitionKernel, Arc<RuntimeEffect>>,
    color_grade: Option<Arc<RuntimeEffect>>,
    mosaic: Option<Arc<RuntimeEffect>>,
    directional_blur: Option<Arc<RuntimeEffect>>,
    spotlight: Option<Arc<RuntimeEffect>>,
    extension_color_gain: Option<Arc<RuntimeEffect>>,
    chroma_key: Option<Arc<RuntimeEffect>>,
    output_transform: Option<Arc<RuntimeEffect>>,
    counters: EffectCacheCounters,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct EffectCacheCounters {
    pub(crate) hits: u64,
    pub(crate) misses: u64,
}

impl EffectCacheCounters {
    pub(crate) fn since(self, before: Self) -> Self {
        Self {
            hits: self.hits.saturating_sub(before.hits),
            misses: self.misses.saturating_sub(before.misses),
        }
    }
}

impl EffectRuntime {
    pub(crate) fn new() -> Self {
        Self {
            transitions: BTreeMap::new(),
            color_grade: None,
            mosaic: None,
            directional_blur: None,
            spotlight: None,
            extension_color_gain: None,
            chroma_key: None,
            output_transform: None,
            counters: EffectCacheCounters::default(),
        }
    }

    pub(crate) const fn counters(&self) -> EffectCacheCounters {
        self.counters
    }

    pub(crate) fn admit(
        template: &RenderPlanTemplate,
        bindings: &RenderBindings,
    ) -> Result<Self, DrawError> {
        let mut runtime = Self::new();
        runtime.admit_template(template, bindings)?;
        Ok(runtime)
    }

    pub(crate) fn admit_template(
        &mut self,
        template: &RenderPlanTemplate,
        bindings: &RenderBindings,
    ) -> Result<(), DrawError> {
        for pass in template.passes() {
            match &pass.kind {
                ExecutionPassKind::DispatchKernel {
                    invocation: KernelInvocation::Transition { kernel, .. },
                } => {
                    if self.transitions.contains_key(kernel) {
                        self.counters.hits = self.counters.hits.saturating_add(1);
                    } else {
                        self.counters.misses = self.counters.misses.saturating_add(1);
                        self.transitions.insert(
                            *kernel,
                            Arc::new(compile(transition_source(*kernel)?, "transition")?),
                        );
                    }
                }
                ExecutionPassKind::DispatchKernel {
                    invocation:
                        KernelInvocation::Filter { effect, .. }
                        | KernelInvocation::AdjustmentEffect { effect, .. },
                } => {
                    preflight_effect_domain(effect, bindings)?;
                    self.admit_kernel(effect.kernel)?;
                }
                ExecutionPassKind::ImportRegion {
                    source_pipeline, ..
                } => {
                    if let Some(effect) = &source_pipeline.chroma_key {
                        self.admit_kernel(effect.kernel)?;
                    }
                }
                ExecutionPassKind::CopyConvert {
                    operation: CopyOperation::OutputTransform { .. },
                    ..
                } => {
                    if self.output_transform.is_none() {
                        self.counters.misses = self.counters.misses.saturating_add(1);
                        self.output_transform =
                            Some(Arc::new(compile(OUTPUT_TRANSFORM, "outputTransform")?));
                    } else {
                        self.counters.hits = self.counters.hits.saturating_add(1);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn admit_kernel(&mut self, kernel: PreparedEffectKernel) -> Result<(), DrawError> {
        match kernel {
            PreparedEffectKernel::ColorGrade { .. } => {
                if self.color_grade.is_none() {
                    self.counters.misses = self.counters.misses.saturating_add(1);
                    self.color_grade = Some(Arc::new(compile(
                        include_str!("../../../../valle-draw/assets/shaders/colorgrade.sksl"),
                        "colorGrade",
                    )?));
                } else {
                    self.counters.hits = self.counters.hits.saturating_add(1);
                }
            }
            PreparedEffectKernel::Mosaic { .. } => {
                if self.mosaic.is_none() {
                    self.counters.misses = self.counters.misses.saturating_add(1);
                    self.mosaic = Some(Arc::new(compile(
                        include_str!("../../../../valle-draw/assets/shaders/mosaic.sksl"),
                        "mosaic",
                    )?));
                } else {
                    self.counters.hits = self.counters.hits.saturating_add(1);
                }
            }
            PreparedEffectKernel::DirectionalBlur { .. } => {
                if self.directional_blur.is_none() {
                    self.counters.misses = self.counters.misses.saturating_add(1);
                    self.directional_blur =
                        Some(Arc::new(compile(DIRECTIONAL_BLUR, "directionalBlur")?));
                } else {
                    self.counters.hits = self.counters.hits.saturating_add(1);
                }
            }
            PreparedEffectKernel::Spotlight { .. } => {
                if self.spotlight.is_none() {
                    self.counters.misses = self.counters.misses.saturating_add(1);
                    self.spotlight = Some(Arc::new(compile(
                        include_str!("../../../../valle-draw/assets/shaders/spotlight.sksl"),
                        "spotlight",
                    )?));
                } else {
                    self.counters.hits = self.counters.hits.saturating_add(1);
                }
            }
            PreparedEffectKernel::ChromaKey { .. } => {
                if self.chroma_key.is_none() {
                    self.counters.misses = self.counters.misses.saturating_add(1);
                    self.chroma_key = Some(Arc::new(compile(
                        include_str!("../../../../valle-draw/assets/shaders/chromakey.sksl"),
                        "chromaKey",
                    )?));
                } else {
                    self.counters.hits = self.counters.hits.saturating_add(1);
                }
            }
            PreparedEffectKernel::GaussianBlur { .. } => {}
            PreparedEffectKernel::ExtensionColorGain {
                implementation_sha256,
                ..
            } => {
                if implementation_sha256
                    != engine_owned_kernel_implementation_sha256(EXTENSION_COLOR_GAIN_ABI)
                        .expect("color-gain ABI has an engine-owned implementation")
                {
                    return Err(DrawError::ExtensionImplementationMismatch {
                        abi: EXTENSION_COLOR_GAIN_ABI,
                    });
                }
                if self.extension_color_gain.is_none() {
                    self.counters.misses = self.counters.misses.saturating_add(1);
                    self.extension_color_gain = Some(Arc::new(compile(
                        EXTENSION_COLOR_GAIN,
                        "extensionColorGain",
                    )?));
                } else {
                    self.counters.hits = self.counters.hits.saturating_add(1);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn transition(
        &self,
        surfaces: &mut ScratchSurfaces<'_, '_>,
        output_slot: SurfaceSlotId,
        backdrop: &Image,
        from: &Image,
        to: &Image,
        kernel: PreparedTransitionKernel,
        progress: f32,
        from_opacity: f32,
        to_opacity: f32,
        info: &ImageInfo,
    ) -> Result<(), DrawError> {
        let from = {
            let surface = surfaces.surface_mut(0)?;
            scale_image_into(surface, from, from_opacity);
            surface.image_snapshot()
        };
        let to = {
            let surface = surfaces.surface_mut(1)?;
            scale_image_into(surface, to, to_opacity);
            surface.image_snapshot()
        };
        let effect = self
            .transitions
            .get(&kernel)
            .ok_or_else(|| DrawError::Unsupported(format!("transition {kernel:?}")))?;
        let local = {
            let surface = surfaces.surface_mut(2)?;
            render_runtime_into(
                surface,
                effect,
                &[&from, &to],
                &[info.width() as f32, info.height() as f32, progress],
            )?;
            surface.image_snapshot()
        };
        let output = surfaces.output_mut(output_slot)?;
        draw_composite(output, &local, backdrop, BlendMode::SrcOver, 1.0)
    }

    pub(crate) fn output_shader(
        &self,
        input: &Image,
        spec: OutputSpec,
    ) -> Result<Shader, DrawError> {
        if spec.dither() != Dither::None || spec.bit_depth() != OutputBitDepth::Eight {
            return Err(DrawError::Unsupported(
                "direct GPU output requires non-dithered eight-bit delivery".into(),
            ));
        }
        let effect = self
            .output_transform
            .as_ref()
            .ok_or_else(|| DrawError::Unsupported("outputTransform".into()))?;
        let primaries = match spec.target().primaries {
            ColorPrimaries::Rec709 => 0.0,
            ColorPrimaries::DisplayP3 => 1.0,
            ColorPrimaries::Rec2020 => 2.0,
        };
        let transfer = match spec.target().transfer {
            TransferFunction::Linear => 0.0,
            TransferFunction::Srgb => 1.0,
            TransferFunction::Rec709 => 2.0,
            TransferFunction::Pq => 3.0,
            TransferFunction::Hlg => 4.0,
        };
        let tone_map = match spec.tone_map() {
            ToneMap::None => 0.0,
            ToneMap::ReinhardLuminance => 1.0,
        };
        let gamut_map = match spec.gamut_map() {
            GamutMap::Clip => 0.0,
            GamutMap::ChromaCompress => 1.0,
        };
        let alpha = match spec.alpha() {
            OutputAlphaMode::Opaque => 0.0,
            OutputAlphaMode::StraightCoverage => 1.0,
            OutputAlphaMode::PremultipliedCoverage => 2.0,
        };
        let uniforms = [
            primaries,
            transfer,
            tone_map,
            gamut_map,
            alpha,
            f32::from(spec.reference_white().get()),
            f32::from(spec.peak_luminance().get()),
        ];
        let mut bytes = Vec::with_capacity(uniforms.len() * 4);
        for value in uniforms {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        if bytes.len() != effect.uniform_size() || effect.children().len() != 1 {
            return Err(DrawError::Internal(
                "outputTransform RuntimeEffect ABI mismatch".into(),
            ));
        }
        let child = input
            .to_raw_shader(
                (TileMode::Clamp, TileMode::Clamp),
                SamplingOptions::new(skia_safe::FilterMode::Nearest, skia_safe::MipmapMode::None),
                None,
            )
            .ok_or_else(|| DrawError::Unsupported("raw output image shader".into()))?;
        effect
            .make_shader(Data::new_copy(&bytes), &[ChildPtr::Shader(child)], None)
            .ok_or_else(|| DrawError::Unsupported("outputTransform shader".into()))
    }

    pub(crate) fn apply_prepared_into(
        &self,
        output: &mut Surface,
        input: &Image,
        effect: &PlanEffect,
        bindings: &RenderBindings,
        info: &ImageInfo,
    ) -> Result<(), DrawError> {
        let scalar = |id| dynamic_scalar(bindings, id, DynamicBindingKind::DeviceLength);
        let region = effect_region(effect);
        let restricted = !matches!(effect.space, PreparedEffectSpace::Root) || region.is_some();
        let canvas = output.canvas();
        canvas.clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
        if restricted {
            let mut input_paint = Paint::default();
            input_paint.set_blend_mode(BlendMode::Src);
            canvas.draw_image(input, (0.0, 0.0), Some(&input_paint));
            canvas.save();
            match effect.space {
                PreparedEffectSpace::Root => {
                    if let Some(region) = region {
                        canvas.clip_rect(root_region_rect(region, info), ClipOp::Intersect, false);
                    }
                }
                PreparedEffectSpace::Layer { transform, bounds } => {
                    canvas.clip_rect(
                        device_rect(dynamic_bounds(bindings, bounds)?),
                        ClipOp::Intersect,
                        false,
                    );
                    if let Some(region) = region {
                        canvas.concat(&matrix(dynamic_transform(bindings, transform)?.matrix()));
                        canvas.clip_rect(unit_rect(region), ClipOp::Intersect, false);
                        canvas.reset_matrix();
                    }
                }
            }
        }

        let result = match effect.kernel {
            PreparedEffectKernel::ChromaKey {
                key_working_linear_rec2020,
                intensity,
                shadow,
                feather_sigma_device_px,
                edge_clean,
            } => draw_runtime(
                canvas,
                self.chroma_key
                    .as_ref()
                    .ok_or_else(|| DrawError::Unsupported("chroma key".into()))?,
                &[input],
                &[
                    key_working_linear_rec2020[0],
                    key_working_linear_rec2020[1],
                    key_working_linear_rec2020[2],
                    intensity,
                    shadow,
                    scalar(feather_sigma_device_px)?,
                    edge_clean,
                ],
            ),
            PreparedEffectKernel::ColorGrade {
                brightness,
                contrast,
                saturation,
                temperature,
                vignette,
            } => draw_runtime(
                canvas,
                self.color_grade
                    .as_ref()
                    .ok_or_else(|| DrawError::Unsupported("color grade".into()))?,
                &[input],
                &[
                    info.width() as f32,
                    info.height() as f32,
                    brightness,
                    contrast,
                    saturation,
                    temperature,
                    vignette,
                ],
            ),
            PreparedEffectKernel::GaussianBlur {
                sigma_device_px, ..
            } => {
                let sigma = scalar(sigma_device_px)?;
                draw_filter(
                    canvas,
                    input,
                    &Filter::Blur {
                        sigma_x: sigma,
                        sigma_y: sigma,
                    },
                )
            }
            PreparedEffectKernel::Mosaic {
                block_size_device_px,
                ..
            } => {
                let anchor = effect_anchor(effect, bindings)?;
                draw_runtime(
                    canvas,
                    self.mosaic
                        .as_ref()
                        .ok_or_else(|| DrawError::Unsupported("mosaic".into()))?,
                    &[input],
                    &[
                        info.width() as f32,
                        info.height() as f32,
                        scalar(block_size_device_px)?,
                        anchor[0],
                        anchor[1],
                    ],
                )
            }
            PreparedEffectKernel::DirectionalBlur {
                axis,
                span_device_px,
            } => {
                let direction = match axis {
                    PreparedBlurAxis::Horizontal => [1.0, 0.0],
                    PreparedBlurAxis::Vertical => [0.0, 1.0],
                };
                draw_runtime(
                    canvas,
                    self.directional_blur
                        .as_ref()
                        .ok_or_else(|| DrawError::Unsupported("directional blur".into()))?,
                    &[input],
                    &[direction[0], direction[1], scalar(span_device_px)?],
                )
            }
            PreparedEffectKernel::Spotlight {
                center,
                radius,
                feather,
                intensity,
            } => draw_runtime(
                canvas,
                self.spotlight
                    .as_ref()
                    .ok_or_else(|| DrawError::Unsupported("spotlight".into()))?,
                &[input],
                &[
                    info.width() as f32,
                    info.height() as f32,
                    center[0],
                    center[1],
                    radius,
                    feather,
                    intensity,
                ],
            ),
            PreparedEffectKernel::ExtensionColorGain { gain, .. } => draw_runtime(
                canvas,
                self.extension_color_gain
                    .as_ref()
                    .ok_or_else(|| DrawError::Unsupported("extension color gain".into()))?,
                &[input],
                &[gain],
            ),
        };
        if restricted {
            canvas.restore();
        }
        result
    }
}

fn preflight_effect_domain(
    effect: &PlanEffect,
    bindings: &RenderBindings,
) -> Result<(), DrawError> {
    let PreparedEffectSpace::Layer { transform, bounds } = effect.space else {
        return Ok(());
    };
    let transform = dynamic_transform(bindings, transform)?;
    let bounds = dynamic_bounds(bindings, bounds)?;
    if effect_region(effect).is_some()
        && !bounds.is_empty()
        && invert_homography(transform.matrix()).is_none()
    {
        return Err(DrawError::Internal(format!(
            "effect {} has a non-invertible layer transform",
            effect.semantic_path
        )));
    }
    Ok(())
}

fn effect_region(effect: &PlanEffect) -> Option<PreparedUnitRect> {
    match effect.kernel {
        PreparedEffectKernel::GaussianBlur { region, .. }
        | PreparedEffectKernel::Mosaic { region, .. } => region,
        _ => None,
    }
}

fn effect_anchor(effect: &PlanEffect, bindings: &RenderBindings) -> Result<[f32; 2], DrawError> {
    match effect.space {
        PreparedEffectSpace::Root => Ok([0.0, 0.0]),
        PreparedEffectSpace::Layer { transform, .. } => {
            let point =
                project_homography(dynamic_transform(bindings, transform)?.matrix(), [0.0, 0.0])
                    .ok_or_else(|| {
                        DrawError::Internal(format!(
                            "effect {} has an invalid mosaic anchor",
                            effect.semantic_path
                        ))
                    })?;
            Ok([point[0] as f32, point[1] as f32])
        }
    }
}

fn dynamic_scalar(
    bindings: &RenderBindings,
    id: DynamicBindingId,
    kind: DynamicBindingKind,
) -> Result<f32, DrawError> {
    match dynamic_value(bindings, id, kind)? {
        DynamicValue::Scalar(value) => Ok(*value as f32),
        _ => Err(invalid_dynamic(id, kind)),
    }
}

fn dynamic_transform(
    bindings: &RenderBindings,
    id: DynamicBindingId,
) -> Result<DeviceTransform, DrawError> {
    match dynamic_value(bindings, id, DynamicBindingKind::DeviceTransform)? {
        DynamicValue::DeviceTransform(value) => Ok(*value),
        _ => Err(invalid_dynamic(id, DynamicBindingKind::DeviceTransform)),
    }
}

fn dynamic_bounds(
    bindings: &RenderBindings,
    id: DynamicBindingId,
) -> Result<DeviceRect, DrawError> {
    match dynamic_value(bindings, id, DynamicBindingKind::Bounds)? {
        DynamicValue::Bounds(value) => Ok(*value),
        _ => Err(invalid_dynamic(id, DynamicBindingKind::Bounds)),
    }
}

fn dynamic_value(
    bindings: &RenderBindings,
    id: DynamicBindingId,
    kind: DynamicBindingKind,
) -> Result<&DynamicValue, DrawError> {
    bindings
        .dynamic()
        .get(id)
        .filter(|binding| binding.binding_kind == kind)
        .map(|binding| &binding.value)
        .ok_or_else(|| invalid_dynamic(id, kind))
}

fn invalid_dynamic(id: DynamicBindingId, kind: DynamicBindingKind) -> DrawError {
    DrawError::Internal(format!("dynamic binding {} is not a {kind:?}", id.get()))
}

fn root_region_rect(region: PreparedUnitRect, info: &ImageInfo) -> SkRect {
    SkRect::from_xywh(
        region.x * info.width() as f32,
        region.y * info.height() as f32,
        region.width * info.width() as f32,
        region.height * info.height() as f32,
    )
}

fn unit_rect(region: PreparedUnitRect) -> SkRect {
    SkRect::from_xywh(region.x, region.y, region.width, region.height)
}

fn device_rect(rect: DeviceRect) -> SkRect {
    SkRect::from_xywh(
        rect.x as f32,
        rect.y as f32,
        rect.width as f32,
        rect.height as f32,
    )
}

fn project_homography(matrix: [f64; 9], point: [f64; 2]) -> Option<[f64; 2]> {
    let [x, y] = point;
    let w = matrix[6] * x + matrix[7] * y + matrix[8];
    if !w.is_finite() || w.abs() <= 1.0e-12 {
        return None;
    }
    let projected = [
        (matrix[0] * x + matrix[1] * y + matrix[2]) / w,
        (matrix[3] * x + matrix[4] * y + matrix[5]) / w,
    ];
    projected
        .iter()
        .all(|value| value.is_finite())
        .then_some(projected)
}

fn invert_homography(matrix: [f64; 9]) -> Option<[f64; 9]> {
    let [a, b, c, d, e, f, g, h, i] = matrix;
    let cofactors = [
        e * i - f * h,
        c * h - b * i,
        b * f - c * e,
        f * g - d * i,
        a * i - c * g,
        c * d - a * f,
        d * h - e * g,
        b * g - a * h,
        a * e - b * d,
    ];
    let determinant = a * cofactors[0] + b * cofactors[3] + c * cofactors[6];
    if !determinant.is_finite() || determinant.abs() <= 1.0e-15 {
        return None;
    }
    Some(cofactors.map(|value| value / determinant))
}

fn compile(source: &str, name: &str) -> Result<RuntimeEffect, DrawError> {
    RuntimeEffect::make_for_shader(source, None).map_err(|message| DrawError::ShaderCompile {
        uri: format!("builtin://{name}"),
        message,
    })
}

fn transition_source(kernel: PreparedTransitionKernel) -> Result<&'static str, DrawError> {
    Ok(match kernel {
        PreparedTransitionKernel::Fade => {
            include_str!("../../../../valle-draw/assets/shaders/fade.sksl")
        }
        PreparedTransitionKernel::WipeLeft => {
            include_str!("../../../../valle-draw/assets/shaders/wipeleft.sksl")
        }
        PreparedTransitionKernel::WipeRight => {
            include_str!("../../../../valle-draw/assets/shaders/wiperight.sksl")
        }
        PreparedTransitionKernel::CircleOpen => {
            include_str!("../../../../valle-draw/assets/shaders/circleopen.sksl")
        }
        PreparedTransitionKernel::SimpleZoom => {
            include_str!("../../../../valle-draw/assets/shaders/simplezoom.sksl")
        }
        PreparedTransitionKernel::CrossWarp => {
            include_str!("../../../../valle-draw/assets/shaders/crosswarp.sksl")
        }
        PreparedTransitionKernel::LinearBlur => {
            include_str!("../../../../valle-draw/assets/shaders/linearblur.sksl")
        }
        PreparedTransitionKernel::DirectionalWarp => {
            include_str!("../../../../valle-draw/assets/shaders/directionalwarp.sksl")
        }
        PreparedTransitionKernel::DreamyZoom => {
            include_str!("../../../../valle-draw/assets/shaders/dreamyzoom.sksl")
        }
        PreparedTransitionKernel::Ripple => {
            include_str!("../../../../valle-draw/assets/shaders/ripple.sksl")
        }
        PreparedTransitionKernel::FlyEye => {
            include_str!("../../../../valle-draw/assets/shaders/flyeye.sksl")
        }
        PreparedTransitionKernel::MultiplyBlend => {
            include_str!("../../../../valle-draw/assets/shaders/multiplyblend.sksl")
        }
        PreparedTransitionKernel::Perlin => {
            include_str!("../../../../valle-draw/assets/shaders/perlin.sksl")
        }
        PreparedTransitionKernel::ExtensionCrossFade {
            implementation_sha256,
            ..
        } => {
            if implementation_sha256
                != engine_owned_kernel_implementation_sha256(EXTENSION_CROSS_FADE_ABI)
                    .expect("cross-fade ABI has an engine-owned implementation")
            {
                return Err(DrawError::ExtensionImplementationMismatch {
                    abi: EXTENSION_CROSS_FADE_ABI,
                });
            }
            include_str!("../../../../valle-draw/assets/shaders/fade.sksl")
        }
    })
}

pub(crate) fn render_runtime_into(
    surface: &mut Surface,
    effect: &RuntimeEffect,
    inputs: &[&Image],
    scalars: &[f32],
) -> Result<(), DrawError> {
    surface.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    draw_runtime(surface.canvas(), effect, inputs, scalars)
}

fn draw_runtime(
    canvas: &skia_safe::Canvas,
    effect: &RuntimeEffect,
    inputs: &[&Image],
    scalars: &[f32],
) -> Result<(), DrawError> {
    let shader = runtime_shader(effect, inputs, scalars)?;
    let mut paint = Paint::default();
    paint.set_blend_mode(BlendMode::Src);
    paint.set_shader(shader);
    canvas.draw_paint(&paint);
    Ok(())
}

fn runtime_shader(
    effect: &RuntimeEffect,
    inputs: &[&Image],
    scalars: &[f32],
) -> Result<Shader, DrawError> {
    let mut bytes = Vec::with_capacity(scalars.len() * 4);
    for scalar in scalars {
        bytes.extend_from_slice(&scalar.to_ne_bytes());
    }
    if bytes.len() != effect.uniform_size() || inputs.len() != effect.children().len() {
        return Err(DrawError::Internal(
            "builtin RuntimeEffect ABI mismatch".into(),
        ));
    }
    let children = inputs
        .iter()
        .map(|image| {
            image
                .to_shader(
                    (TileMode::Clamp, TileMode::Clamp),
                    SamplingOptions::new(
                        skia_safe::FilterMode::Linear,
                        skia_safe::MipmapMode::None,
                    ),
                    None,
                )
                .map(ChildPtr::Shader)
                .ok_or_else(|| DrawError::Unsupported("builtin shader child".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    effect
        .make_shader(Data::new_copy(&bytes), &children, None)
        .ok_or_else(|| DrawError::Unsupported("builtin RuntimeEffect instantiation".into()))
}

fn scale_image_into(surface: &mut Surface, input: &Image, opacity: f32) {
    surface.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    let mut paint = Paint::default();
    paint.set_blend_mode(BlendMode::Src);
    paint.set_alpha_f(opacity);
    surface.canvas().draw_image(input, (0.0, 0.0), Some(&paint));
}

fn draw_composite(
    surface: &mut Surface,
    source: &Image,
    destination: &Image,
    mode: BlendMode,
    opacity: f32,
) -> Result<(), DrawError> {
    surface.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    let mut destination_paint = Paint::default();
    destination_paint.set_blend_mode(BlendMode::Src);
    surface
        .canvas()
        .draw_image(destination, (0.0, 0.0), Some(&destination_paint));
    let mut source_paint = Paint::default();
    source_paint.set_blend_mode(mode);
    source_paint.set_alpha_f(opacity);
    surface
        .canvas()
        .draw_image(source, (0.0, 0.0), Some(&source_paint));
    Ok(())
}

pub(crate) fn apply_prepared_mask(
    surfaces: &mut ScratchSurfaces<'_, '_>,
    output_slot: SurfaceSlotId,
    input: &Image,
    mask: PreparedMask,
) -> Result<(), DrawError> {
    let mask_surface = surfaces.surface_mut(0)?;
    let canvas = mask_surface.canvas();
    canvas.clear(if mask.invert() {
        Color4f::new(1.0, 1.0, 1.0, 1.0)
    } else {
        Color4f::new(0.0, 0.0, 0.0, 0.0)
    });
    canvas.save();
    canvas.concat(&matrix(mask.device_from_mask().matrix()));
    let mut shape = Paint::default();
    shape.set_anti_alias(true);
    shape.set_blend_mode(BlendMode::Src);
    shape.set_color4f(
        if mask.invert() {
            Color4f::new(0.0, 0.0, 0.0, 0.0)
        } else {
            Color4f::new(1.0, 1.0, 1.0, 1.0)
        },
        None,
    );
    let unit = skia_safe::Rect::from_xywh(0.0, 0.0, 1.0, 1.0);
    match mask.shape() {
        PreparedMaskShape::Rect => canvas.draw_rect(unit, &shape),
        PreparedMaskShape::Ellipse => canvas.draw_oval(unit, &shape),
    };
    canvas.restore();
    let mut mask_image = mask_surface.image_snapshot();
    let sigma = mask.feather_sigma_device_px() as f32;
    if sigma > 0.0 {
        let filtered = surfaces.surface_mut(1)?;
        filtered.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
        draw_filter(
            filtered.canvas(),
            &mask_image,
            &Filter::Blur {
                sigma_x: sigma,
                sigma_y: sigma,
            },
        )?;
        mask_image = filtered.image_snapshot();
    }

    let output = surfaces.output_mut(output_slot)?;
    output.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    let mut source = Paint::default();
    source.set_blend_mode(BlendMode::Src);
    output.canvas().draw_image(input, (0.0, 0.0), Some(&source));
    let mut coverage = Paint::default();
    coverage.set_blend_mode(BlendMode::DstIn);
    output
        .canvas()
        .draw_image(&mask_image, (0.0, 0.0), Some(&coverage));
    Ok(())
}

fn matrix(value: [f64; 9]) -> Matrix {
    Matrix::new_all(
        value[0] as f32,
        value[1] as f32,
        value[2] as f32,
        value[3] as f32,
        value[4] as f32,
        value[5] as f32,
        value[6] as f32,
        value[7] as f32,
        value[8] as f32,
    )
}

pub(crate) fn apply_filter_into(
    surface: &mut Surface,
    input: &Image,
    filter: &Filter,
) -> Result<(), DrawError> {
    surface
        .canvas()
        .clear(skia_safe::Color4f::new(0.0, 0.0, 0.0, 0.0));
    draw_filter(surface.canvas(), input, filter)
}

fn draw_filter(
    canvas: &skia_safe::Canvas,
    input: &Image,
    filter: &Filter,
) -> Result<(), DrawError> {
    let mut paint = Paint::default();
    paint.set_blend_mode(BlendMode::Src);
    if let Some(image_filter) = image_filter(filter)? {
        paint.set_image_filter(image_filter);
    }
    canvas.draw_image(input, (0.0, 0.0), Some(&paint));
    Ok(())
}

pub(crate) fn image_filter(filter: &Filter) -> Result<Option<ImageFilter>, DrawError> {
    let result = match filter {
        Filter::Blur { sigma_x, sigma_y } => {
            if *sigma_x == 0.0 && *sigma_y == 0.0 {
                return Ok(None);
            }
            image_filters::blur((*sigma_x, *sigma_y), TileMode::Decal, None, None)
        }
        Filter::ColorMatrix { matrix } => matrix_filter(matrix.as_ref()),
        Filter::Brightness { amount } => matrix_filter(&diagonal(*amount, *amount, *amount, 0.0)),
        Filter::Contrast { amount } => {
            matrix_filter(&diagonal(*amount, *amount, *amount, 0.5 - 0.5 * *amount))
        }
        Filter::Grayscale { amount } => matrix_filter(&grayscale(*amount)),
        Filter::HueRotate { degrees } => matrix_filter(&hue_rotate(*degrees)),
        Filter::Invert { amount } => matrix_filter(&diagonal(
            1.0 - 2.0 * *amount,
            1.0 - 2.0 * *amount,
            1.0 - 2.0 * *amount,
            *amount,
        )),
        Filter::Opacity { amount } => {
            let mut matrix = diagonal(1.0, 1.0, 1.0, 0.0);
            matrix[18] = *amount;
            matrix_filter(&matrix)
        }
        Filter::Saturate { amount } => matrix_filter(&saturate(*amount)),
        Filter::Sepia { amount } => matrix_filter(&sepia(*amount)),
        Filter::DropShadow {
            offset,
            sigma_x,
            sigma_y,
            color,
        } => image_filters::drop_shadow(
            (offset[0], offset[1]),
            (*sigma_x, *sigma_y),
            straight_color(*color).to_color(),
            None,
            None,
            None,
        ),
        Filter::NoiseDisplacement {
            frequency,
            octaves,
            seed,
            scale,
            turbulence,
        } => {
            let noise = if *turbulence {
                shaders::turbulence(
                    (frequency[0], frequency[1]),
                    usize::from(*octaves),
                    *seed as f32,
                    None,
                )
            } else {
                shaders::fractal_noise(
                    (frequency[0], frequency[1]),
                    usize::from(*octaves),
                    *seed as f32,
                    None,
                )
            }
            .ok_or_else(|| DrawError::Unsupported("noise displacement shader".into()))?;
            let displacement = image_filters::shader(noise, None)
                .ok_or_else(|| DrawError::Unsupported("noise displacement input".into()))?;
            image_filters::displacement_map(
                (ColorChannel::R, ColorChannel::G),
                *scale,
                displacement,
                None,
                None,
            )
        }
        Filter::VelocityBlur {
            velocity,
            shutter_angle_degrees,
        } => {
            let span = valle_draw::math::sqrt(f64::from(
                velocity[0] * velocity[0] + velocity[1] * velocity[1],
            )) as f32
                * (*shutter_angle_degrees / 360.0).clamp(0.0, 1.0);
            if span <= 1.0e-6 {
                return Ok(None);
            }
            let angle = valle_draw::math::atan2(f64::from(velocity[1]), f64::from(velocity[0]))
                .to_degrees() as f32;
            let rotated = image_filters::matrix_transform(
                &Matrix::rotate_deg(-angle),
                SamplingOptions::default(),
                None,
            )
            .ok_or_else(|| DrawError::Unsupported("velocity blur input rotation".into()))?;
            let blurred = image_filters::blur((span / 3.0, 0.0), TileMode::Decal, rotated, None)
                .ok_or_else(|| DrawError::Unsupported("velocity blur".into()))?;
            image_filters::matrix_transform(
                &Matrix::rotate_deg(angle),
                SamplingOptions::default(),
                blurred,
            )
        }
    };
    result
        .map(Some)
        .ok_or_else(|| DrawError::Unsupported(format!("Skia filter {filter:?}")))
}

pub(crate) fn straight_color(color: LinearColor) -> Color4f {
    if color.alpha == 0.0 {
        Color4f::new(0.0, 0.0, 0.0, 0.0)
    } else {
        Color4f::new(
            color.red / color.alpha,
            color.green / color.alpha,
            color.blue / color.alpha,
            color.alpha,
        )
    }
}

fn matrix_filter(matrix: &[f32; 20]) -> Option<ImageFilter> {
    let color_filter = color_filters::matrix_row_major(matrix, None);
    image_filters::color_filter(color_filter, None, None)
}

fn diagonal(red: f32, green: f32, blue: f32, offset: f32) -> [f32; 20] {
    [
        red, 0.0, 0.0, 0.0, offset, 0.0, green, 0.0, 0.0, offset, 0.0, 0.0, blue, 0.0, offset, 0.0,
        0.0, 0.0, 1.0, 0.0,
    ]
}

fn grayscale(amount: f32) -> [f32; 20] {
    let a = amount;
    [
        0.2126 + 0.7874 * (1.0 - a),
        0.7152 - 0.7152 * (1.0 - a),
        0.0722 - 0.0722 * (1.0 - a),
        0.0,
        0.0,
        0.2126 - 0.2126 * (1.0 - a),
        0.7152 + 0.2848 * (1.0 - a),
        0.0722 - 0.0722 * (1.0 - a),
        0.0,
        0.0,
        0.2126 - 0.2126 * (1.0 - a),
        0.7152 - 0.7152 * (1.0 - a),
        0.0722 + 0.9278 * (1.0 - a),
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    ]
}

fn saturate(amount: f32) -> [f32; 20] {
    [
        0.213 + 0.787 * amount,
        0.715 - 0.715 * amount,
        0.072 - 0.072 * amount,
        0.0,
        0.0,
        0.213 - 0.213 * amount,
        0.715 + 0.285 * amount,
        0.072 - 0.072 * amount,
        0.0,
        0.0,
        0.213 - 0.213 * amount,
        0.715 - 0.715 * amount,
        0.072 + 0.928 * amount,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    ]
}

fn hue_rotate(degrees: f32) -> [f32; 20] {
    let radians = f64::from(degrees).to_radians();
    let cosine = valle_draw::math::cos(radians) as f32;
    let sine = valle_draw::math::sin(radians) as f32;
    [
        0.213 + cosine * 0.787 - sine * 0.213,
        0.715 - cosine * 0.715 - sine * 0.715,
        0.072 - cosine * 0.072 + sine * 0.928,
        0.0,
        0.0,
        0.213 - cosine * 0.213 + sine * 0.143,
        0.715 + cosine * 0.285 + sine * 0.140,
        0.072 - cosine * 0.072 - sine * 0.283,
        0.0,
        0.0,
        0.213 - cosine * 0.213 - sine * 0.787,
        0.715 - cosine * 0.715 + sine * 0.715,
        0.072 + cosine * 0.928 + sine * 0.072,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    ]
}

fn sepia(amount: f32) -> [f32; 20] {
    let inverse = 1.0 - amount;
    [
        0.393 * amount + inverse,
        0.769 * amount,
        0.189 * amount,
        0.0,
        0.0,
        0.349 * amount,
        0.686 * amount + inverse,
        0.168 * amount,
        0.0,
        0.0,
        0.272 * amount,
        0.534 * amount,
        0.131 * amount + inverse,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::skia::{
        output::{rgba8_target_info, stage_output},
        surface::working_color_space,
    };
    use skia_safe::{AlphaType, ColorType, ImageInfo, images, surfaces};
    use valle_engine::resource::{Extent2d, OutputBackground};

    fn premul_pixel_image(pixel: [f32; 4]) -> Image {
        let bytes = pixel
            .into_iter()
            .flat_map(f32::to_ne_bytes)
            .collect::<Vec<_>>();
        let info = ImageInfo::new(
            (1, 1),
            ColorType::RGBAF32,
            AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        images::raster_from_data(&info, Data::new_copy(&bytes), bytes.len()).unwrap()
    }

    fn read_premul_pixel(surface: &mut Surface) -> [f32; 4] {
        let info = ImageInfo::new(
            (1, 1),
            ColorType::RGBAF32,
            AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let mut bytes = [0_u8; 16];
        assert!(surface.read_pixels(&info, &mut bytes, 16, (0, 0)));
        std::array::from_fn(|index| {
            f32::from_ne_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap())
        })
    }

    fn assert_pixel_close(actual: [f32; 4], expected: [f32; 4]) {
        for (index, (actual_channel, expected_channel)) in
            actual.into_iter().zip(expected).enumerate()
        {
            assert!(
                (actual_channel - expected_channel).abs() <= 1.0e-3,
                "channel {index}: {actual_channel} != {expected_channel}; actual={actual:?}, expected={expected:?}"
            );
        }
    }

    #[test]
    fn extension_color_gain_is_admitted_cached_and_executable() {
        let rejected = PreparedEffectKernel::ExtensionColorGain {
            implementation_sha256: [2; 32],
            gain: 1.5,
            past_frames: 0,
            future_frames: 0,
        };
        assert!(matches!(
            EffectRuntime::new().admit_kernel(rejected),
            Err(DrawError::ExtensionImplementationMismatch {
                abi: EXTENSION_COLOR_GAIN_ABI
            })
        ));
        let kernel = PreparedEffectKernel::ExtensionColorGain {
            implementation_sha256: engine_owned_kernel_implementation_sha256(
                EXTENSION_COLOR_GAIN_ABI,
            )
            .unwrap(),
            gain: 1.5,
            past_frames: 0,
            future_frames: 0,
        };
        let mut runtime = EffectRuntime::new();
        runtime.admit_kernel(kernel).unwrap();
        assert_eq!(
            runtime.counters(),
            EffectCacheCounters { hits: 0, misses: 1 }
        );
        runtime.admit_kernel(kernel).unwrap();
        assert_eq!(
            runtime.counters(),
            EffectCacheCounters { hits: 1, misses: 1 }
        );

        let input_pixel = [0.2, 0.1, 0.05, 0.5];
        let input = premul_pixel_image(input_pixel);
        let info = ImageInfo::new(
            (1, 1),
            ColorType::RGBAF32,
            AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let mut target = surfaces::raster(&info, None, None).unwrap();
        render_runtime_into(
            &mut target,
            runtime.extension_color_gain.as_deref().unwrap(),
            &[&input],
            &[1.5],
        )
        .unwrap();
        assert_pixel_close(read_premul_pixel(&mut target), [0.3, 0.15, 0.075, 0.5]);
    }

    #[test]
    fn extension_cross_fade_executes_the_builtin_linear_law() {
        let rejected = PreparedTransitionKernel::ExtensionCrossFade {
            implementation_sha256: [4; 32],
            past_frames: 0,
            future_frames: 0,
        };
        assert!(matches!(
            transition_source(rejected),
            Err(DrawError::ExtensionImplementationMismatch {
                abi: EXTENSION_CROSS_FADE_ABI
            })
        ));
        let kernel = PreparedTransitionKernel::ExtensionCrossFade {
            implementation_sha256: engine_owned_kernel_implementation_sha256(
                EXTENSION_CROSS_FADE_ABI,
            )
            .unwrap(),
            past_frames: 0,
            future_frames: 0,
        };
        assert_eq!(
            transition_source(kernel).unwrap(),
            transition_source(PreparedTransitionKernel::Fade).unwrap()
        );
        let effect = compile(transition_source(kernel).unwrap(), "extensionCrossFade").unwrap();
        let from_pixel = [0.4, 0.2, 0.1, 0.5];
        let to_pixel = [0.1, 0.3, 0.2, 0.75];
        let from = premul_pixel_image(from_pixel);
        let to = premul_pixel_image(to_pixel);
        let info = ImageInfo::new(
            (1, 1),
            ColorType::RGBAF32,
            AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let mut target = surfaces::raster(&info, None, None).unwrap();
        render_runtime_into(&mut target, &effect, &[&from, &to], &[1.0, 1.0, 0.25]).unwrap();
        assert_pixel_close(
            read_premul_pixel(&mut target),
            [0.325, 0.225, 0.125, 0.5625],
        );
    }

    #[test]
    fn output_shader_tracks_the_exact_cpu_reference_within_two_codes() {
        use valle_engine::resource::{OutputColorEncoding, SignalLuminance};
        for target in [OutputColorEncoding::SRGB, OutputColorEncoding::REC709] {
            for alpha in [
                OutputAlphaMode::Opaque,
                OutputAlphaMode::StraightCoverage,
                OutputAlphaMode::PremultipliedCoverage,
            ] {
                for tone in [ToneMap::None, ToneMap::ReinhardLuminance] {
                    for gamut in [GamutMap::Clip, GamutMap::ChromaCompress] {
                        let background = if alpha == OutputAlphaMode::Opaque {
                            OutputBackground::opaque_srgb([0, 0, 0])
                        } else {
                            OutputBackground::Transparent
                        };
                        let spec = OutputSpec::new(
                            target,
                            alpha,
                            background,
                            tone,
                            gamut,
                            Dither::None,
                            OutputBitDepth::Eight,
                            SignalLuminance::SDR_100,
                        )
                        .unwrap();
                        assert_output_shader_matches_reference(spec);
                    }
                }
            }
        }
    }

    fn assert_output_shader_matches_reference(spec: OutputSpec) {
        let mut samples = [
            [0.0_f32, 0.0, 0.0, 0.0],
            [0.18, 0.18, 0.18, 1.0],
            [1.2, 0.15, 0.04, 1.0],
            [0.10, 0.30, 0.05, 0.5],
        ];
        if spec.alpha() == OutputAlphaMode::Opaque {
            for sample in &mut samples {
                sample[3] = 1.0;
            }
        }
        let mut source_bytes = Vec::with_capacity(samples.len() * 4 * size_of::<f32>());
        for sample in samples {
            for channel in sample {
                source_bytes.extend_from_slice(&channel.to_ne_bytes());
            }
        }
        let source_info = ImageInfo::new(
            (samples.len() as i32, 1),
            ColorType::RGBAF32,
            AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let source = images::raster_from_data(
            &source_info,
            Data::new_copy(&source_bytes),
            source_bytes.len(),
        )
        .unwrap();
        let target_info =
            rgba8_target_info(spec, Extent2d::new(samples.len() as u32, 1).unwrap()).unwrap();
        let expected = stage_output(&source, spec, &target_info).unwrap();

        let runtime = EffectRuntime {
            output_transform: Some(Arc::new(
                compile(OUTPUT_TRANSFORM, "outputTransform").unwrap(),
            )),
            ..EffectRuntime::new()
        };
        let shader = runtime.output_shader(&source, spec).unwrap();
        let mut target = surfaces::raster(&target_info, None, None).unwrap();
        let mut paint = Paint::default();
        paint.set_blend_mode(BlendMode::Src);
        paint.set_shader(shader);
        target.canvas().draw_paint(&paint);
        let mut actual = vec![0_u8; target_info.compute_min_byte_size()];
        assert!(target.read_pixels(&target_info, &mut actual, samples.len() * 4, (0, 0),));

        let maximum_delta = actual
            .iter()
            .zip(expected.pixels())
            .map(|(actual, expected)| actual.abs_diff(*expected))
            .max()
            .unwrap_or(0);
        let mean_square_error = actual
            .iter()
            .zip(expected.pixels())
            .map(|(actual, expected)| {
                let delta = f64::from(*actual) - f64::from(*expected);
                delta * delta
            })
            .sum::<f64>()
            / actual.len() as f64;
        let psnr = if mean_square_error == 0.0 {
            f64::INFINITY
        } else {
            10.0 * (255.0_f64 * 255.0 / mean_square_error).log10()
        };
        assert!(
            maximum_delta <= 2 && psnr >= 50.0,
            "output shader drifted from the CPU reference: max delta={maximum_delta}, PSNR={psnr:.2} dB; actual={actual:?}, expected={:?}",
            expected.pixels(),
        );
    }
}
