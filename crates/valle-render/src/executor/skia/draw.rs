use std::{collections::BTreeMap, sync::Arc};

use skia_safe::{
    AlphaType, BlendMode as SkBlendMode, ClipOp, Color, Color4f, ColorType, CubicResampler, Data,
    FilterMode, Font, IRect, Image, ImageInfo, Matrix, MipmapMode, Paint as SkPaint, PaintCap,
    PaintJoin, PaintStyle, Path as SkPath, PathBuilder, PathEffect, PathFillType, Point as SkPoint,
    RRect, RSXform, Rect as SkRect, RuntimeEffect, SamplingOptions, Shader, Surface,
    TextBlobBuilder, TileMode, Typeface, canvas::PointMode, color_filters, gradient,
    image::CachingHint, images, runtime_effect::ChildPtr,
};
use thiserror::Error;
use valle_draw::{
    Rect,
    program::{
        BlendMode, Clip, DrawProgram, FillRule, Filter, GlyphRun, ImageNode, InstanceBatchNode,
        InstanceShape, LinearColor, MaskMode, Node, Paint, PaintId, PathData, PathStroke, PathVerb,
        RoundRect, ShaderLayer, ShaderTextureBinding, ShaderUniformBinding, ShaderUniformValue,
        SpreadMode, StrokeCap, StrokeJoin, Transform2d,
    },
    requirements::{DrawCapability, ExternalTexture, FontKey, RuntimeShaderKey, SamplingMode},
};
use valle_engine::compositor::{
    BoundExternalObjects, ExternalObject,
    lower::{
        BoundProgramSchedule, ExternalSlotId, PlanProgram, PlanResourceId, ProgramPassKind,
        ProgramResourceId, ProgramStorageKind, SurfaceSlotId,
    },
};
use valle_engine::{
    prepare::{DeviceRect, DeviceTransform, ExternalSample},
    resource::{ContentDigest, Extent2d, ResourceInterpretation, VisualInterpretation},
};

use super::{
    SkiaExternalObject,
    blend::BlendRuntime,
    cache::BackendCaches,
    effect::{draw_filters, set_working_color, straight_color},
    glass::{
        admit_motion_glass_shader, render_motion_glass_foreground_into, render_motion_glass_into,
    },
    surface::{
        PlanImage, ScratchSurfaces, SkiaBackendKind, SurfaceError, raster_surface,
        working_color_space,
    },
};

#[derive(Debug, Clone, Copy)]
pub(crate) enum ProgramTerminal {
    Plan {
        slot: SurfaceSlotId,
        roi: DeviceRect,
    },
    Scratch(usize),
}

#[derive(Clone)]
struct ProgramImage {
    image: Image,
    roi: DeviceRect,
}

/// Fully decoded and backend-admitted DrawProgram. All fallible font parsing and runtime shader
/// compilation happens before any compositor surface is allocated.
pub(crate) struct ProgramRuntime {
    pub(crate) program: Arc<DrawProgram>,
    pub(crate) textures: BTreeMap<ExternalTexture, ProgramTexture>,
    pub(crate) fonts: BTreeMap<(ContentDigest, u32), Typeface>,
    pub(crate) shaders: BTreeMap<String, Arc<RuntimeEffect>>,
    pub(crate) scenes: BTreeMap<ContentDigest, Image>,
    glass: Option<RuntimeEffect>,
    blend: Option<BlendRuntime>,
    mask_coverage: Option<RuntimeEffect>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProgramTexture {
    image: Image,
    interpretation: Option<VisualInterpretation>,
    size: [u32; 2],
}

impl core::fmt::Debug for ProgramRuntime {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ProgramRuntime")
            .field("nodes", &self.program.nodes().len())
            .field("textures", &self.textures.len())
            .field("fonts", &self.fonts.len())
            .field("shaders", &self.shaders.len())
            .field("scenes", &self.scenes.len())
            .field("motionGlass", &self.glass.is_some())
            .field("creativeBlend", &self.blend.is_some())
            .finish()
    }
}

impl ProgramRuntime {
    pub(crate) fn admit(
        plan: &PlanProgram,
        bound: &BoundExternalObjects<'_, SkiaExternalObject>,
        caches: &mut BackendCaches,
    ) -> Result<Self, DrawError> {
        let program = caches.program(plan)?;
        if program.viewport() != plan.viewport || program.requirements() != &plan.requirements {
            return Err(DrawError::ProgramContractMismatch {
                program: plan.id.get(),
            });
        }

        let mut textures = BTreeMap::new();
        for (requirement, binding) in plan
            .requirements
            .external_textures
            .iter()
            .zip(&plan.resources.textures)
        {
            if binding.key != requirement.key {
                return Err(DrawError::MissingTexture(requirement.key.clone()));
            }
            let slot = binding.slot;
            let object = bound
                .get(slot)
                .ok_or_else(|| DrawError::MissingTexture(requirement.key.clone()))?;
            let (image, interpretation, size) =
                match (&object.key().interpretation, requirement.color_domain) {
                    (
                        ResourceInterpretation::DataTexture {},
                        valle_draw::requirements::ColorDomain::Data,
                    ) => {
                        let image = object
                            .data_image()
                            .ok_or_else(|| DrawError::MissingTexture(requirement.key.clone()))?;
                        (
                            image.clone(),
                            None,
                            [image.width() as u32 / 2, image.height() as u32 / 4],
                        )
                    }
                    (ResourceInterpretation::Visual { interpretation }, domain)
                        if domain != valle_draw::requirements::ColorDomain::Data =>
                    {
                        let image = object
                            .visual_image()
                            .ok_or_else(|| DrawError::MissingTexture(requirement.key.clone()))?;
                        (
                            image.clone(),
                            Some(*interpretation),
                            [image.width() as u32, image.height() as u32],
                        )
                    }
                    _ => return Err(DrawError::MissingTexture(requirement.key.clone())),
                };
            if textures
                .insert(
                    requirement.clone(),
                    ProgramTexture {
                        image,
                        interpretation,
                        size,
                    },
                )
                .is_some()
            {
                return Err(DrawError::DuplicateResource(requirement.key.clone()));
            }
        }

        let mut fonts = BTreeMap::new();
        for requirement in &plan.requirements.fonts {
            let digest = ContentDigest::from_bytes(requirement.face_hash.into_bytes());
            let slot = font_slot(plan, requirement, digest)?;
            let bytes = bound
                .get(slot)
                .and_then(SkiaExternalObject::font_data)
                .ok_or_else(|| DrawError::MissingFont {
                    hash: requirement.face_hash.as_hex(),
                    index: requirement.face_index,
                })?;
            let key = (digest, requirement.face_index);
            let face = caches.font(key.clone(), bytes)?;
            if fonts.insert(key, face).is_some() {
                return Err(DrawError::DuplicateResource(requirement.face_hash.as_hex()));
            }
        }

        let mut shaders = BTreeMap::new();
        for requirement in &plan.requirements.runtime_shaders {
            let slot = structure_slot(
                &plan.resources.runtime_shaders,
                &requirement.uri,
                "runtime shader",
            )?;
            let object = bound
                .get(slot)
                .ok_or_else(|| DrawError::MissingShader(requirement.uri.clone()))?;
            let binding = plan
                .resources
                .runtime_shaders
                .iter()
                .find(|binding| binding.slot == slot)
                .ok_or_else(|| DrawError::MissingShader(requirement.uri.clone()))?;
            let effect = caches.shader(binding, object, &requirement.uri)?;
            validate_shader_abi(&effect, requirement)?;
            if shaders.insert(requirement.uri.clone(), effect).is_some() {
                return Err(DrawError::DuplicateResource(requirement.uri.clone()));
            }
        }

        let mut scenes = BTreeMap::new();
        for requirement in &plan.requirements.scene3d {
            let digest = ContentDigest::from_bytes(requirement.content_hash.into_bytes());
            let binding = plan
                .resources
                .scenes
                .iter()
                .find(|binding| {
                    bound
                        .get(binding.slot)
                        .is_some_and(|object| object.key().content == digest)
                })
                .ok_or_else(|| DrawError::MissingScene(requirement.content_hash.as_hex()))?;
            let image = visual(bound, binding.slot)
                .ok_or_else(|| DrawError::MissingScene(requirement.content_hash.as_hex()))?
                .clone();
            if scenes.insert(digest, image).is_some() {
                return Err(DrawError::DuplicateResource(
                    requirement.content_hash.as_hex(),
                ));
            }
        }

        for node in program.nodes() {
            let shader = match node {
                Node::RuntimeShader(shader) => Some((
                    &shader.shader.uri,
                    shader.bounds,
                    shader.uniforms.as_slice(),
                    shader.textures.as_slice(),
                )),
                Node::Group(group) => group.shader.as_ref().map(|shader| {
                    (
                        &shader.shader.uri,
                        shader.bounds,
                        shader.uniforms.as_slice(),
                        shader.textures.as_slice(),
                    )
                }),
                _ => None,
            };
            if let Some((uri, bounds, uniforms, texture_bindings)) = shader {
                preflight_shader_instance(
                    uri,
                    bounds,
                    uniforms,
                    texture_bindings,
                    &shaders,
                    &textures,
                )?;
            }
        }

        for pass in plan.local_plan().passes() {
            if let ProgramPassKind::ApplyTransition { transition, .. } = &pass.kind {
                for (uri, source) in [
                    (
                        format!("builtin://transition/{:?}", transition.kind),
                        transition.kind.source(),
                    ),
                    (
                        "builtin://transition-input".into(),
                        include_str!("../../../../valle-draw/assets/shaders/transitioninput.sksl"),
                    ),
                ] {
                    if !shaders.contains_key(&uri) {
                        let effect =
                            RuntimeEffect::make_for_shader(source, None).map_err(|message| {
                                DrawError::ShaderCompile {
                                    uri: uri.clone(),
                                    message,
                                }
                            })?;
                        shaders.insert(uri, Arc::new(effect));
                    }
                }
            }
        }
        let blend = plan
            .local_plan()
            .passes()
            .iter()
            .any(|pass| matches!(pass.kind, ProgramPassKind::Blend { mode, .. } if mode != BlendMode::Normal))
            .then(BlendRuntime::admit)
            .transpose()?;

        let glass = plan
            .requirements
            .capabilities
            .contains(&DrawCapability::MotionGlass)
            .then(admit_motion_glass_shader)
            .transpose()?;

        let mask_coverage = plan.local_plan().passes().iter()
            .any(|pass| matches!(pass.kind, ProgramPassKind::ApplyMask { mode, .. } if mode.is_luminance()))
            .then(|| RuntimeEffect::make_for_shader(
                include_str!("../../../../valle-draw/assets/shaders/maskcoverage.sksl"), None)
                .map_err(|message| DrawError::ShaderCompile { uri: "builtin://mask-coverage".into(), message }))
            .transpose()?;

        if std::env::var_os("VALLE_COMPOSITOR_TRACE_PASSES").is_some() {
            eprintln!(
                "[valle program] nodes={} local-passes={}",
                program.nodes().len(),
                plan.local_plan().passes().len()
            );
        }
        Ok(Self {
            mask_coverage,
            program,
            textures,
            fonts,
            shaders,
            scenes,
            glass,
            blend,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute(
        &self,
        plan: &PlanProgram,
        schedule: &BoundProgramSchedule,
        transform: DeviceTransform,
        destination_inputs: &[PlanResourceId],
        outer: &BTreeMap<PlanResourceId, PlanImage>,
        extent: Extent2d,
        info: &ImageInfo,
        surfaces: &mut ScratchSurfaces<'_, '_>,
        terminal: ProgramTerminal,
        helper_index: Option<usize>,
    ) -> Result<PlanImage, DrawError> {
        if schedule.passes().len() != plan.local_plan().passes().len() {
            return Err(DrawError::Internal(
                "bound program pass count does not match its plan".into(),
            ));
        }
        let output_roi = schedule
            .passes()
            .last()
            .filter(|pass| pass.output() == plan.local_plan().output())
            .map(|pass| pass.device_roi())
            .ok_or(DrawError::MissingProgramOutput)?;
        if output_roi.is_empty() {
            return Ok(PlanImage::transparent());
        }
        let destinations = destination_inputs
            .iter()
            .map(|resource| {
                outer
                    .get(resource)
                    .and_then(|image| {
                        image.image().cloned().map(|pixels| ProgramImage {
                            image: pixels,
                            roi: image.roi(),
                        })
                    })
                    .ok_or(DrawError::MissingOuterResource(resource.get()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let local_slots = schedule
            .surface_slots()
            .iter()
            .filter(|slot| slot.extent().is_some())
            .enumerate()
            .map(|(index, slot)| (slot.id(), index))
            .collect::<BTreeMap<_, _>>();
        let mut resources = BTreeMap::<ProgramResourceId, Option<ProgramImage>>::new();
        let terminal_origin = match terminal {
            ProgramTerminal::Plan { roi, .. } => {
                if roi != output_roi {
                    return Err(DrawError::Internal(
                        "program terminal ROI does not match its bound output".into(),
                    ));
                }
                [roi.x, roi.y]
            }
            ProgramTerminal::Scratch(_) => [0, 0],
        };

        for (pass, step) in plan.local_plan().passes().iter().zip(schedule.passes()) {
            let output = step.output();
            if step.pass() != pass.id || output != pass.kind.output() {
                return Err(DrawError::Internal(
                    "bound program pass identity does not match its plan".into(),
                ));
            }
            let storage = step.storage();
            let image = match storage {
                ProgramStorageKind::Transparent {} => None,
                ProgramStorageKind::Destination { destination } => Some(
                    destinations
                        .get(destination.get() as usize - 1)
                        .cloned()
                        .ok_or(DrawError::MissingDestination(destination.get()))?,
                ),
                ProgramStorageKind::Alias { source } => resource(&resources, source)?.cloned(),
                ProgramStorageKind::Surface { slot } => {
                    let roi = step.device_roi();
                    if roi.is_empty() {
                        None
                    } else {
                        let index = *local_slots
                            .get(&slot)
                            .ok_or(DrawError::MissingProgramSurface(slot.get()))?;
                        let image = self.render_program_pass_at(
                            surfaces,
                            ProgramTerminal::Scratch(index),
                            helper_index,
                            plan,
                            &pass.kind,
                            output,
                            transform,
                            &destinations,
                            &resources,
                            extent,
                            info,
                            [roi.x, roi.y],
                            roi,
                        )?;
                        Some(ProgramImage { image, roi })
                    }
                }
                ProgramStorageKind::Output {} => {
                    let roi = step.device_roi();
                    let image = self.render_program_pass_at(
                        surfaces,
                        terminal,
                        helper_index,
                        plan,
                        &pass.kind,
                        output,
                        transform,
                        &destinations,
                        &resources,
                        extent,
                        info,
                        terminal_origin,
                        roi,
                    )?;
                    Some(ProgramImage {
                        image,
                        roi: output_roi,
                    })
                }
            };
            if resources.insert(output, image).is_some() {
                return Err(DrawError::DuplicateProgramResource(output.get()));
            }
            for resource in step.retire_after() {
                resources.remove(resource);
            }
        }

        let image = resources
            .remove(&plan.local_plan().output())
            .flatten()
            .ok_or(DrawError::MissingProgramOutput)?;
        Ok(match terminal {
            ProgramTerminal::Plan { roi, .. } => PlanImage::new(image.image, roi),
            ProgramTerminal::Scratch(_) => PlanImage::root(image.image, extent),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn render_program_pass_at(
        &self,
        surfaces: &mut ScratchSurfaces<'_, '_>,
        target_location: ProgramTerminal,
        helper_index: Option<usize>,
        plan: &PlanProgram,
        pass: &ProgramPassKind,
        output: ProgramResourceId,
        transform: DeviceTransform,
        destinations: &[ProgramImage],
        resources: &BTreeMap<ProgramResourceId, Option<ProgramImage>>,
        extent: Extent2d,
        info: &ImageInfo,
        target_origin: [i32; 2],
        output_roi: DeviceRect,
    ) -> Result<Image, DrawError> {
        if let ProgramPassKind::Backdrop {
            input,
            bounds,
            filters,
            ..
        } = pass
            && !filters.is_empty()
        {
            let helper_index = helper_index.ok_or(DrawError::MissingProgramHelper)?;
            let input = resource(resources, *input)?;
            {
                let helper = surfaces.surface_mut(helper_index)?;
                clip_image_into(
                    helper,
                    input,
                    &Clip::Rect(*bounds),
                    self.device_matrix(plan, output, transform)?,
                    self.program.paths(),
                    [output_roi.x, output_roi.y],
                )?;
            }
            let clipped = ProgramImage {
                image: surfaces.surface_mut(helper_index)?.image_snapshot(),
                roi: output_roi,
            };
            let target = program_target_mut(surfaces, target_location)?;
            apply_filter_chain_program_into(target, &clipped, filters, target_origin)?;
            return snapshot_program_target(target, target_location, output_roi);
        }

        // Shared F16 filters consume CPU bytes. Ganesh images need their Metal context
        // for readback, which must happen before borrowing the destination surface.
        let cpu_filter_input = if let ProgramPassKind::ApplyFilter { input, filter, .. } = pass {
            if uses_cpu_f16_filter(filter) && !uses_gpu_f16_filter(filter, surfaces.backend_kind())
            {
                if let Some(source) = resource(resources, *input)? {
                    let started = std::time::Instant::now();
                    let image = surfaces.prepare_cpu_image(&source.image)?;
                    if std::env::var_os("VALLE_TRACE_F16_STAGES").is_some() {
                        eprintln!(
                            "[valle f16] gpu-to-cpu {:.3}ms",
                            started.elapsed().as_secs_f64() * 1_000.0
                        );
                    }
                    Some(ProgramImage {
                        image,
                        roi: source.roi,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let (target, clip) = match target_location {
            ProgramTerminal::Scratch(index) if draws_within_roi(pass) => {
                // A program's final pass may render into a root-sized scratch at origin zero.
                let bounds = IRect::from_xywh(
                    output_roi.x - target_origin[0],
                    output_roi.y - target_origin[1],
                    i32::try_from(output_roi.width)
                        .map_err(|_| DrawError::Internal("program ROI width overflow".into()))?,
                    i32::try_from(output_roi.height)
                        .map_err(|_| DrawError::Internal("program ROI height overflow".into()))?,
                );
                let (target, count) = surfaces.bounded_surface_mut(index, bounds)?;
                (target, Some(count))
            }
            _ => (program_target_mut(surfaces, target_location)?, None),
        };
        let rendered = self.render_program_pass_into(
            target,
            plan,
            pass,
            output,
            transform,
            destinations,
            resources,
            extent,
            info,
            target_origin,
            output_roi,
            cpu_filter_input.as_ref(),
        );
        if let Some(count) = clip {
            target.canvas().restore_to_count(count);
        }
        rendered?;
        snapshot_program_target(target, target_location, output_roi)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_program_pass_into(
        &self,
        target: &mut Surface,
        plan: &PlanProgram,
        pass: &ProgramPassKind,
        output: ProgramResourceId,
        transform: DeviceTransform,
        destinations: &[ProgramImage],
        resources: &BTreeMap<ProgramResourceId, Option<ProgramImage>>,
        extent: Extent2d,
        info: &ImageInfo,
        target_origin: [i32; 2],
        output_roi: DeviceRect,
        cpu_filter_input: Option<&ProgramImage>,
    ) -> Result<(), DrawError> {
        match pass {
            ProgramPassKind::Clear { .. } => clear_surface(target),
            ProgramPassKind::RasterNode { node, .. } => self.raster_node_into(
                target,
                plan,
                *node,
                output,
                transform,
                extent,
                info,
                target_origin,
            ),
            ProgramPassKind::RasterTree { roots, .. } => {
                self.raster_nodes_into(target, plan, roots, output, transform, target_origin)
            }
            ProgramPassKind::ReadDestination {
                external,
                local_inputs,
                ..
            } => {
                clear_surface(target)?;
                if let Some(destination) = external {
                    let image = destinations
                        .get(destination.get() as usize - 1)
                        .ok_or(DrawError::MissingDestination(destination.get()))?;
                    draw_program_image_with_mode(
                        target,
                        image,
                        target_origin,
                        SkBlendMode::Src,
                        1.0,
                    );
                }
                for input in local_inputs {
                    if let Some(image) = resource(resources, *input)? {
                        draw_program_image_with_mode(
                            target,
                            image,
                            target_origin,
                            SkBlendMode::SrcOver,
                            1.0,
                        );
                    }
                }
                Ok(())
            }
            ProgramPassKind::Backdrop {
                input,
                bounds,
                filters,
                ..
            } => {
                if !filters.is_empty() {
                    return Err(DrawError::MissingProgramHelper);
                }
                clip_image_into(
                    target,
                    resource(resources, *input)?,
                    &Clip::Rect(*bounds),
                    self.device_matrix(plan, output, transform)?,
                    self.program.paths(),
                    target_origin,
                )
            }
            ProgramPassKind::MotionGlass { input, program, .. } => {
                let input = resource(resources, *input)?.ok_or_else(|| {
                    DrawError::Internal("Motion Glass input is transparent".into())
                })?;
                render_motion_glass_into(
                    target,
                    self.glass.as_ref().ok_or_else(|| {
                        DrawError::Internal("Motion Glass pass has no admitted kernel".into())
                    })?,
                    &input.image,
                    input.roi,
                    program,
                    self.device_matrix_values(plan, output, transform)?,
                    target_origin,
                )
            }
            ProgramPassKind::ApplyMotionGlassForeground {
                input,
                owner_to_program,
                program,
                ..
            } => {
                let input = resource(resources, *input)?.ok_or_else(|| {
                    DrawError::Internal("Motion Glass foreground input is transparent".into())
                })?;
                render_motion_glass_foreground_into(
                    target,
                    self.glass.as_ref().ok_or_else(|| {
                        DrawError::Internal("Motion Glass pass has no admitted kernel".into())
                    })?,
                    &input.image,
                    input.roi,
                    program,
                    self.program_transform_values(*owner_to_program, transform)?,
                    target_origin,
                )
            }
            ProgramPassKind::SourceOver {
                source,
                destination,
                ..
            } => source_over_into(
                target,
                resource(resources, *source)?,
                resource(resources, *destination)?,
                target_origin,
            ),
            ProgramPassKind::ApplyClip { input, clip, .. } => clip_image_into(
                target,
                resource(resources, *input)?,
                clip,
                self.device_matrix(plan, output, transform)?,
                self.program.paths(),
                target_origin,
            ),
            ProgramPassKind::ApplyFilter { input, filter, .. } => {
                if let Some(image) = cpu_filter_input.or(resource(resources, *input)?) {
                    let local_to_device = self.device_matrix_values(plan, output, transform)?;
                    let lens_frame = if matches!(filter, Filter::LensDistortion { .. }) {
                        let [x, y] = program_gaussian_in_device_space([1.0, 1.0], local_to_device)?;
                        if (x - y).abs() > x.max(y).max(1.0) * 1.0e-9 {
                            return Err(DrawError::Unsupported(
                                "lens distortion needs a similarity device transform".into(),
                            ));
                        }
                        let bounds = plan
                            .local_plan()
                            .resources()
                            .get((input.get() - 1) as usize)
                            .filter(|resource| resource.id == *input)
                            .and_then(|resource| resource.bounds.rect())
                            .and_then(|rect| Transform2d(local_to_device).map_bounds(rect))
                            .ok_or_else(|| {
                                DrawError::Unsupported(
                                    "lens distortion has no finite source frame".into(),
                                )
                            })?;
                        Some([
                            checked_device_filter_value(bounds.x)?,
                            checked_device_filter_value(bounds.y)?,
                            checked_device_filter_value(bounds.width)?,
                            checked_device_filter_value(bounds.height)?,
                        ])
                    } else {
                        None
                    };
                    apply_filter_program_into(
                        target,
                        image,
                        filter,
                        local_to_device,
                        target_origin,
                        output_roi,
                        lens_frame,
                    )
                } else {
                    clear_surface(target)
                }
            }
            ProgramPassKind::ApplyMask {
                input, mask, mode, ..
            } => apply_mask_into(
                target,
                self.mask_coverage.as_ref(),
                resource(resources, *input)?,
                resource(resources, *mask)?,
                *mode,
                target_origin,
            ),
            ProgramPassKind::ApplyOpacity { input, opacity, .. } => opacity_image_into(
                target,
                resource(resources, *input)?,
                *opacity,
                target_origin,
            ),
            ProgramPassKind::ApplyTransition {
                from,
                to,
                transition,
                ..
            } => self.apply_transition_into(
                target,
                [resource(resources, *from)?, resource(resources, *to)?],
                transition,
                self.device_matrix(plan, output, transform)?,
                target_origin,
            ),
            ProgramPassKind::ApplyShader { input, shader, .. } => self.apply_shader_into(
                target,
                resource(resources, *input)?,
                shader,
                self.device_matrix(plan, output, transform)?,
                info,
                target_origin,
            ),
            ProgramPassKind::ApplyTransform { input, .. } => {
                copy_optional_into(target, resource(resources, *input)?, target_origin)
            }
            ProgramPassKind::Blend {
                source,
                destination,
                mode,
                space,
                ..
            } => self.blend_source_into(
                target,
                resource(resources, *source)?,
                resource(resources, *destination)?,
                *mode,
                *space,
                target_origin,
            ),
        }
    }

    fn blend_source_into(
        &self,
        target: &mut Surface,
        source: Option<&ProgramImage>,
        destination: Option<&ProgramImage>,
        mode: BlendMode,
        space: valle_draw::program::BlendSpace,
        target_origin: [i32; 2],
    ) -> Result<(), DrawError> {
        let Some(source) = source else {
            return clear_surface(target);
        };
        let Some(destination) = destination else {
            return copy_optional_into(target, Some(source), target_origin);
        };
        if mode == BlendMode::Normal {
            return copy_optional_into(target, Some(source), target_origin);
        }
        let blend = self
            .blend
            .as_ref()
            .ok_or_else(|| DrawError::Internal("creative blend kernel was not admitted".into()))?;
        let dimensions = target.image_info().dimensions();
        if source.roi == destination.roi
            && source.roi.x == target_origin[0]
            && source.roi.y == target_origin[1]
            && i32::try_from(source.roi.width).ok() == Some(dimensions.width)
            && i32::try_from(source.roi.height).ok() == Some(dimensions.height)
            && source.image.dimensions() == dimensions
            && destination.image.dimensions() == dimensions
        {
            return blend.effective_source_into(
                target,
                &source.image,
                &destination.image,
                mode,
                space,
            );
        }
        // Local program resources are cropped independently. A blend samples both operands in
        // output coordinates, so align their ROI origins before handing images to the kernel.
        // Passing the raw slot snapshots would treat each ROI's top-left as the same pixel.
        let info = target.image_info();
        let mut source_surface = target
            .new_surface(&info)
            .ok_or_else(|| DrawError::Internal("blend source surface allocation failed".into()))?;
        source_surface
            .canvas()
            .clear(skia_safe::Color4f::new(0.0, 0.0, 0.0, 0.0));
        draw_program_image_with_mode(
            &mut source_surface,
            source,
            target_origin,
            SkBlendMode::Src,
            1.0,
        );
        let mut destination_surface = target.new_surface(&info).ok_or_else(|| {
            DrawError::Internal("blend destination surface allocation failed".into())
        })?;
        destination_surface
            .canvas()
            .clear(skia_safe::Color4f::new(0.0, 0.0, 0.0, 0.0));
        draw_program_image_with_mode(
            &mut destination_surface,
            destination,
            target_origin,
            SkBlendMode::Src,
            1.0,
        );
        let source_image = source_surface.image_snapshot();
        let destination_image = destination_surface.image_snapshot();
        blend.effective_source_into(target, &source_image, &destination_image, mode, space)
    }

    #[allow(clippy::too_many_arguments)]
    fn raster_node_into(
        &self,
        surface: &mut Surface,
        plan: &PlanProgram,
        node_id: valle_draw::program::NodeId,
        output: ProgramResourceId,
        transform: DeviceTransform,
        _extent: Extent2d,
        _info: &ImageInfo,
        target_origin: [i32; 2],
    ) -> Result<(), DrawError> {
        self.raster_nodes_into(
            surface,
            plan,
            std::slice::from_ref(&node_id),
            output,
            transform,
            target_origin,
        )
    }

    fn raster_nodes_into(
        &self,
        surface: &mut Surface,
        plan: &PlanProgram,
        node_ids: &[valle_draw::program::NodeId],
        output: ProgramResourceId,
        transform: DeviceTransform,
        target_origin: [i32; 2],
    ) -> Result<(), DrawError> {
        let canvas = surface.canvas();
        canvas.clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
        canvas.save();
        canvas.translate((-target_origin[0] as f32, -target_origin[1] as f32));
        canvas.concat(&self.device_matrix(plan, output, transform)?);
        for node_id in node_ids {
            self.raster_node(canvas, *node_id)?;
        }
        canvas.restore();
        Ok(())
    }

    fn raster_node(
        &self,
        canvas: &skia_safe::Canvas,
        node_id: valle_draw::program::NodeId,
    ) -> Result<(), DrawError> {
        let node = self
            .program
            .nodes()
            .get(node_id.raw() as usize)
            .ok_or(DrawError::MissingNode(node_id.raw()))?;

        match node {
            Node::Path(node) => {
                let data = self
                    .program
                    .paths()
                    .get(node.path.raw() as usize)
                    .ok_or(DrawError::MissingPath(node.path.raw()))?;
                let path = path(data, node.fill_rule)?;
                if let Some(fill) = node.fill {
                    let mut paint = self.paint(fill)?;
                    paint.set_style(PaintStyle::Fill);
                    let solid = matches!(
                        self.program.paints().get(fill.raw() as usize),
                        Some(Paint::Solid(_))
                    );
                    if !solid
                        || node.stroke.is_some()
                        || !draw_rounded_path_with_shared_coverage(canvas, data, &path, &paint)?
                    {
                        canvas.draw_path(&path, &paint);
                    }
                }
                if let Some(stroke) = &node.stroke {
                    let paint = self.stroke(stroke)?;
                    canvas.draw_path(&path, &paint);
                }
            }
            Node::InstanceBatch(batch) => {
                draw_instance_batch(&self.program, &self.textures, canvas, batch)?
            }
            Node::Image(node) => {
                let texture = self
                    .textures
                    .get(&node.texture)
                    .ok_or_else(|| DrawError::MissingTexture(node.texture.key.clone()))?;
                draw_program_texture(canvas, texture, node, None, None)?;
            }
            Node::GlyphRun(run) => self.draw_glyph_run(canvas, run)?,
            Node::Shadow(shadow) => draw_shadow(canvas, shadow)?,
            Node::RuntimeShader(node) => {
                if let Some(runtime) = self.runtime_shader(
                    None,
                    &node.shader.uri,
                    node.bounds,
                    &node.uniforms,
                    &node.textures,
                )? {
                    let mut paint = SkPaint::default();
                    paint.set_shader(runtime);
                    canvas.draw_rect(sk_rect(node.bounds), &paint);
                }
            }
            Node::Scene3d(scene) => {
                let digest = ContentDigest::from_bytes(scene.scene.content_hash.into_bytes());
                let image = self
                    .scenes
                    .get(&digest)
                    .ok_or_else(|| DrawError::MissingScene(scene.scene.content_hash.as_hex()))?;
                canvas.draw_image_rect_with_sampling_options(
                    image,
                    None,
                    sk_rect(scene.bounds),
                    SamplingOptions::new(FilterMode::Linear, MipmapMode::None),
                    &SkPaint::default(),
                );
            }
            Node::Group(group) if group.is_raster_tree_group() => {
                if group.opacity == 0.0 {
                    return Ok(());
                }
                let path_coverage = match &group.clip {
                    Some(Clip::Path {
                        path: id,
                        fill_rule,
                    }) => Some(path(&self.program.paths()[id.raw() as usize], *fill_rule)?),
                    _ => None,
                };
                if group.opacity == 1.0 {
                    canvas.save();
                } else {
                    let bounds = self
                        .program
                        .geometry(node_id)
                        .and_then(|geometry| geometry.output_bounds.rect())
                        .map(sk_rect);
                    canvas.save_layer_alpha_f(bounds, group.opacity);
                }
                canvas.concat(&sk_matrix(group.transform.0));
                if let Some(clip) = &group.clip {
                    if path_coverage.is_none() {
                        clip_canvas(canvas, clip, self.program.paths())?;
                    }
                    // A new layer inherits the clip's bounds, not its edge coverage. Apply
                    // coverage on restore, once after overlapping children have been painted.
                    canvas.save_layer(&skia_safe::canvas::SaveLayerRec::default());
                }
                let result = group
                    .children
                    .iter()
                    .try_for_each(|child| self.raster_node(canvas, *child));
                if let Some(outline) = path_coverage {
                    // Use filled-path coverage, exactly as an opaque alpha mask does. Skia's
                    // clipPath rasterizer otherwise disagrees at a few antialiased edge pixels.
                    let mut restore = SkPaint::default();
                    restore.set_blend_mode(SkBlendMode::DstIn);
                    canvas.save_layer(&skia_safe::canvas::SaveLayerRec::default().paint(&restore));
                    let mut coverage = SkPaint::default();
                    coverage.set_anti_alias(true).set_color(Color::WHITE);
                    canvas.draw_path(&outline, &coverage);
                    canvas.restore();
                }
                if group.clip.is_some() {
                    canvas.restore();
                }
                canvas.restore();
                result?;
            }
            Node::Group(_) => return Err(DrawError::UnexpectedGroupRaster(node_id.raw())),
        }
        Ok(())
    }

    fn device_matrix(
        &self,
        plan: &PlanProgram,
        resource: ProgramResourceId,
        device: DeviceTransform,
    ) -> Result<Matrix, DrawError> {
        Ok(sk_matrix(
            self.device_matrix_values(plan, resource, device)?,
        ))
    }

    fn device_matrix_values(
        &self,
        plan: &PlanProgram,
        resource: ProgramResourceId,
        device: DeviceTransform,
    ) -> Result<[f64; 9], DrawError> {
        let resource = plan
            .local_plan()
            .resources()
            .get(resource.get() as usize - 1)
            .filter(|candidate| candidate.id == resource)
            .ok_or(DrawError::MissingProgramResource(resource.get()))?;
        self.program_transform_values(resource.local_to_program, device)
    }

    fn program_transform_values(
        &self,
        local_to_program: valle_draw::program::Transform2d,
        device: DeviceTransform,
    ) -> Result<[f64; 9], DrawError> {
        let viewport = self.program.viewport();
        if viewport.is_empty() {
            return Err(DrawError::Internal(
                "DrawProgram viewport is empty during execution".into(),
            ));
        }
        let normalize = [
            1.0 / viewport.width,
            0.0,
            -viewport.x / viewport.width,
            0.0,
            1.0 / viewport.height,
            -viewport.y / viewport.height,
            0.0,
            0.0,
            1.0,
        ];
        Ok(mul3(device.matrix(), mul3(normalize, local_to_program.0)))
    }

    fn paint(&self, id: PaintId) -> Result<SkPaint, DrawError> {
        let paint = self
            .program
            .paints()
            .get(id.raw() as usize)
            .ok_or(DrawError::MissingPaint(id.raw()))?;
        let mut result = SkPaint::default();
        result.set_anti_alias(true);
        match paint {
            Paint::Solid(color) => {
                set_working_color(&mut result, *color)?;
            }
            Paint::LinearGradient {
                start,
                end,
                stops,
                spread,
            } => {
                let (colors, positions) = gradient_parts(stops);
                let colors = gradient::Colors::new(
                    &colors,
                    Some(&positions),
                    tile_mode(*spread),
                    Some(
                        working_color_space()
                            .map_err(|error| DrawError::Surface(error.to_string()))?,
                    ),
                );
                let gradient = gradient::Gradient::new(colors, gradient_interpolation());
                let shader = gradient::shaders::linear_gradient(
                    (
                        SkPoint::new(start[0] as f32, start[1] as f32),
                        SkPoint::new(end[0] as f32, end[1] as f32),
                    ),
                    &gradient,
                    None,
                )
                .ok_or_else(|| DrawError::Unsupported("linear gradient".into()))?;
                result.set_shader(shader);
            }
            Paint::RadialGradient {
                center,
                radii,
                stops,
                spread,
            } => {
                let (colors, positions) = gradient_parts(stops);
                let colors = gradient::Colors::new(
                    &colors,
                    Some(&positions),
                    tile_mode(*spread),
                    Some(
                        working_color_space()
                            .map_err(|error| DrawError::Surface(error.to_string()))?,
                    ),
                );
                let gradient = gradient::Gradient::new(colors, gradient_interpolation());
                let radius = radii[0].max(radii[1]) as f32;
                let mut matrix = Matrix::new_identity();
                matrix.pre_scale(
                    (radii[0] as f32 / radius, radii[1] as f32 / radius),
                    Some(SkPoint::new(center[0] as f32, center[1] as f32)),
                );
                let shader = gradient::shaders::radial_gradient(
                    (SkPoint::new(center[0] as f32, center[1] as f32), radius),
                    &gradient,
                    Some(&matrix),
                )
                .ok_or_else(|| DrawError::Unsupported("radial gradient".into()))?;
                result.set_shader(shader);
            }
            Paint::TwoCircleGradient {
                start,
                start_radius,
                end,
                end_radius,
                stops,
                spread,
            } => {
                let (colors, positions) = gradient_parts(stops);
                let colors = gradient::Colors::new(
                    &colors,
                    Some(&positions),
                    tile_mode(*spread),
                    Some(working_color_space().map_err(|e| DrawError::Surface(e.to_string()))?),
                );
                let gradient = gradient::Gradient::new(colors, gradient_interpolation());
                let shader = gradient::shaders::two_point_conical_gradient(
                    (
                        SkPoint::new(start[0] as f32, start[1] as f32),
                        *start_radius as f32,
                    ),
                    (
                        SkPoint::new(end[0] as f32, end[1] as f32),
                        *end_radius as f32,
                    ),
                    &gradient,
                    None,
                )
                .ok_or_else(|| DrawError::Unsupported("two-circle gradient".into()))?;
                result.set_shader(shader);
            }
            Paint::ConicGradient {
                center,
                start_angle_degrees,
                sweep_angle_degrees,
                stops,
                spread,
            } => {
                let (colors, positions) = gradient_parts(stops);
                let colors = gradient::Colors::new(
                    &colors,
                    Some(&positions),
                    tile_mode(*spread),
                    Some(
                        working_color_space()
                            .map_err(|error| DrawError::Surface(error.to_string()))?,
                    ),
                );
                let gradient = gradient::Gradient::new(colors, gradient_interpolation());
                let shader = gradient::shaders::sweep_gradient(
                    SkPoint::new(center[0] as f32, center[1] as f32),
                    (
                        *start_angle_degrees as f32,
                        (*start_angle_degrees + *sweep_angle_degrees) as f32,
                    ),
                    &gradient,
                    None,
                )
                .ok_or_else(|| DrawError::Unsupported("conic gradient".into()))?;
                result.set_shader(shader);
            }
        }
        Ok(result)
    }

    fn stroke(&self, stroke: &PathStroke) -> Result<SkPaint, DrawError> {
        let mut paint = self.paint(stroke.paint)?;
        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width(stroke.width);
        paint.set_stroke_miter(stroke.miter_limit);
        paint.set_stroke_cap(match stroke.cap {
            StrokeCap::Butt => PaintCap::Butt,
            StrokeCap::Round => PaintCap::Round,
            StrokeCap::Square => PaintCap::Square,
        });
        paint.set_stroke_join(match stroke.join {
            StrokeJoin::Miter => PaintJoin::Miter,
            StrokeJoin::Round => PaintJoin::Round,
            StrokeJoin::Bevel => PaintJoin::Bevel,
        });
        if !stroke.dash.is_empty() {
            paint.set_path_effect(PathEffect::dash(&stroke.dash, stroke.dash_offset));
        }
        Ok(paint)
    }

    fn draw_glyph_run(&self, canvas: &skia_safe::Canvas, run: &GlyphRun) -> Result<(), DrawError> {
        if let Some(outline) = run.outline {
            let ink = path(
                &self.program.paths()[outline.raw() as usize],
                FillRule::NonZero,
            )?;
            canvas.draw_path(&ink, &self.paint(run.paint)?);
            if let Some(stroke) = &run.stroke {
                canvas.draw_path(&ink, &self.stroke(stroke)?);
            }
            return Ok(());
        }
        let face_hash = ContentDigest::from_bytes(run.font.face_hash.into_bytes());
        let typeface = self
            .fonts
            .get(&(face_hash, run.font.face_index))
            .ok_or_else(|| DrawError::MissingFont {
                hash: run.font.face_hash.as_hex(),
                index: run.font.face_index,
            })?;
        let mut font = Font::from_typeface(typeface.clone(), run.font_size);
        super::surface::GlyphCoverageProfile::native_default().configure(&mut font);
        let mut builder = TextBlobBuilder::new();
        {
            let (ids, positions) = builder.alloc_run_pos(&font, run.glyphs.len(), None);
            for (index, glyph) in run.glyphs.iter().enumerate() {
                ids[index] =
                    u16::try_from(glyph.id).map_err(|_| DrawError::InvalidGlyph(glyph.id))?;
                positions[index] = SkPoint::new(glyph.x as f32, glyph.y as f32);
            }
        }
        if let Some(blob) = builder.make() {
            canvas.draw_text_blob(&blob, (0.0, 0.0), &self.paint(run.paint)?);
            if let Some(stroke) = &run.stroke {
                canvas.draw_text_blob(&blob, (0.0, 0.0), &self.stroke(stroke)?);
            }
        }
        Ok(())
    }

    fn apply_transition_into(
        &self,
        surface: &mut Surface,
        inputs: [Option<&ProgramImage>; 2],
        transition: &valle_draw::program::TransitionLayer,
        matrix: Matrix,
        target_origin: [i32; 2],
    ) -> Result<(), DrawError> {
        let inverse = matrix.invert().ok_or_else(|| {
            DrawError::Unsupported("transition owner transform is not invertible".into())
        })?;
        let uri = format!("builtin://transition/{:?}", transition.kind);
        let effect = self
            .shaders
            .get(&uri)
            .ok_or_else(|| DrawError::MissingShader(uri.clone()))?;
        let input_effect = self
            .shaders
            .get("builtin://transition-input")
            .ok_or_else(|| DrawError::MissingShader("builtin://transition-input".into()))?;
        let resolution = [
            transition.bounds.width as f32,
            transition.bounds.height as f32,
        ];
        let pack = |values: &[f32]| {
            Data::new_copy(
                &values
                    .iter()
                    .flat_map(|value| value.to_ne_bytes())
                    .collect::<Vec<_>>(),
            )
        };
        let mut children = Vec::new();
        for input in inputs {
            let content = match input {
                Some(input) => input
                    .image
                    .to_shader(
                        (TileMode::Decal, TileMode::Decal),
                        SamplingOptions::new(FilterMode::Linear, MipmapMode::None),
                        &Matrix::concat(
                            &inverse,
                            &Matrix::translate((input.roi.x as f32, input.roi.y as f32)),
                        ),
                    )
                    .ok_or_else(|| {
                        DrawError::Unsupported("transition input shader allocation failed".into())
                    })?,
                None => skia_safe::shaders::color(Color::TRANSPARENT),
            };
            let child = input_effect
                .make_shader(pack(&resolution), &[ChildPtr::Shader(content)], None)
                .ok_or_else(|| DrawError::ShaderAbi {
                    uri: "builtin://transition-input".into(),
                })?;
            children.push(ChildPtr::Shader(child));
        }
        let shader = effect
            .make_shader(
                pack(&transition.params.uniforms(
                    resolution[0],
                    resolution[1],
                    transition.progress,
                )),
                &children,
                None,
            )
            .ok_or_else(|| DrawError::ShaderAbi { uri })?;
        let mut paint = SkPaint::default();
        paint.set_shader(shader);
        let canvas = surface.canvas();
        canvas.clear(Color::TRANSPARENT);
        canvas.save();
        canvas.translate((-target_origin[0] as f32, -target_origin[1] as f32));
        canvas.concat(&matrix);
        canvas.draw_rect(sk_rect(transition.bounds), &paint);
        canvas.restore();
        Ok(())
    }

    fn apply_shader_into(
        &self,
        surface: &mut Surface,
        input: Option<&ProgramImage>,
        shader: &ShaderLayer,
        matrix: Matrix,
        _info: &ImageInfo,
        target_origin: [i32; 2],
    ) -> Result<(), DrawError> {
        let device_to_owner = matrix.invert().ok_or_else(|| {
            DrawError::Unsupported("shader owner transform is not invertible".into())
        })?;
        let content = input.map(|input| {
            // A content snapshot is in device pixels. The runtime entry and its child
            // evaluations use pixels relative to the shader's local border box.
            let image_to_shader = Matrix::concat(
                &Matrix::translate((-shader.bounds.x as f32, -shader.bounds.y as f32)),
                &Matrix::concat(
                    &device_to_owner,
                    &Matrix::translate((input.roi.x as f32, input.roi.y as f32)),
                ),
            );
            (&input.image, image_to_shader)
        });
        let canvas = surface.canvas();
        canvas.clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
        canvas.save();
        canvas.translate((-target_origin[0] as f32, -target_origin[1] as f32));
        canvas.concat(&matrix);
        if let Some(runtime) = self.runtime_shader(
            content,
            &shader.shader.uri,
            shader.bounds,
            &shader.uniforms,
            &shader.textures,
        )? {
            let mut paint = SkPaint::default();
            paint.set_shader(runtime);
            canvas.draw_rect(sk_rect(shader.output_bounds()), &paint);
        }
        canvas.restore();
        Ok(())
    }

    fn runtime_shader(
        &self,
        content: Option<(&Image, Matrix)>,
        uri: &str,
        bounds: Rect,
        uniforms: &[ShaderUniformBinding],
        textures: &[ShaderTextureBinding],
    ) -> Result<Option<Shader>, DrawError> {
        let effect = self
            .shaders
            .get(uri)
            .ok_or_else(|| DrawError::MissingShader(uri.to_owned()))?;
        let data = shader_uniforms(effect, bounds, uniforms, textures, &self.textures)?;
        let mut children = Vec::with_capacity(effect.children().len());
        for child in effect.children() {
            if child.name() == "content" && content.is_none() {
                // An empty shader aborts the raster pipeline when sampled. Missing inputs must
                // evaluate to zero and let the parent shader continue computing its output.
                children.push(ChildPtr::Shader(skia_safe::shaders::color(
                    Color::TRANSPARENT,
                )));
                continue;
            }
            let image = if child.name() == "content" {
                content
                    .as_ref()
                    .expect("content presence was handled above")
                    .0
            } else {
                let texture = textures
                    .iter()
                    .find(|texture| texture.name == child.name())
                    .ok_or_else(|| DrawError::ShaderChild {
                        uri: uri.to_owned(),
                        child: child.name().to_owned(),
                    })?;
                let Some(texture) = &texture.texture else {
                    children.push(ChildPtr::Shader(skia_safe::shaders::color(
                        Color::TRANSPARENT,
                    )));
                    continue;
                };
                &self
                    .textures
                    .get(texture)
                    .ok_or_else(|| DrawError::MissingTexture(texture.key.clone()))?
                    .image
            };
            let mode = textures
                .iter()
                .find(|texture| texture.name == child.name())
                .map_or(
                    SamplingOptions::new(FilterMode::Linear, MipmapMode::None),
                    // The shared shader code filters individual working-linear texels.
                    |_| SamplingOptions::new(FilterMode::Nearest, MipmapMode::None),
                );
            let wrap = textures
                .iter()
                .find(|texture| texture.name == child.name())
                .map_or(TileMode::Decal, |texture| tile_mode(texture.wrap));
            let local = (child.name() == "content").then(|| &content.as_ref().unwrap().1);
            let shader = image.to_shader((wrap, wrap), mode, local).ok_or_else(|| {
                DrawError::ShaderChild {
                    uri: uri.to_owned(),
                    child: child.name().to_owned(),
                }
            })?;
            children.push(ChildPtr::Shader(shader));
        }
        let local = Matrix::translate((bounds.x as f32, bounds.y as f32));
        Ok(effect.make_shader(Data::new_copy(&data), &children, Some(&local)))
    }
}

fn visual<'a>(
    bound: &BoundExternalObjects<'a, SkiaExternalObject>,
    slot: ExternalSlotId,
) -> Option<&'a Image> {
    bound.get(slot).and_then(SkiaExternalObject::visual_image)
}

fn draw_program_texture(
    canvas: &skia_safe::Canvas,
    texture: &ProgramTexture,
    node: &valle_draw::program::ImageNode,
    tint: Option<LinearColor>,
    atlas_subset: Option<&AtlasPixelSubset>,
) -> Result<(), DrawError> {
    let ExternalSample::Texture {
        texture_from_content,
        input_sample_bounds,
        ..
    } = ExternalSample::from_display_rect(
        node.src,
        texture.interpretation.ok_or_else(|| {
            DrawError::Unsupported("data texture cannot be drawn as an image".into())
        })?,
    )
    .map_err(|error| DrawError::Unsupported(error.to_string()))?
    else {
        return Ok(());
    };
    let inverse = inverse3(texture_from_content)
        .ok_or_else(|| DrawError::Unsupported("non-invertible program texture sample".into()))?;
    let width = f64::from(texture.image.width());
    let height = f64::from(texture.image.height());
    let normalized_from_pixel = [1.0 / width, 0.0, 0.0, 0.0, 1.0 / height, 0.0, 0.0, 0.0, 1.0];
    let destination_from_content = [
        node.dst.width,
        0.0,
        node.dst.x,
        0.0,
        node.dst.height,
        node.dst.y,
        0.0,
        0.0,
        1.0,
    ];
    let destination_from_pixel = mul3(
        destination_from_content,
        mul3(inverse, normalized_from_pixel),
    );
    let src = SkRect::from_xywh(
        (input_sample_bounds.x * width) as f32,
        (input_sample_bounds.y * height) as f32,
        (input_sample_bounds.width * width) as f32,
        (input_sample_bounds.height * height) as f32,
    );
    canvas.save();
    canvas.clip_rect(sk_rect(node.dst), ClipOp::Intersect, true);
    canvas.concat(&sk_matrix(destination_from_pixel));
    let mut paint = SkPaint::default();
    paint.set_anti_alias(true);
    paint.set_alpha_f(node.opacity * tint.map_or(1.0, |color| color.alpha));
    if let Some(color) = tint
        && color != LinearColor::new(1.0, 1.0, 1.0, 1.0)
    {
        let inverse_alpha = 1.0 / color.alpha;
        let matrix = [
            color.red * inverse_alpha,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            color.green * inverse_alpha,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            color.blue * inverse_alpha,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
        ];
        paint.set_color_filter(color_filters::matrix_row_major(&matrix, None));
    }
    if tint.is_some() {
        let shader = atlas_subset
            .map_or(&texture.image, |subset| &subset.image)
            .to_shader(
                (TileMode::Clamp, TileMode::Clamp),
                sampling(node.sampling),
                None,
            )
            .ok_or_else(|| DrawError::Unsupported("atlas image shader is unavailable".into()))?;
        paint.set_shader(shader);
        if let Some(subset) = atlas_subset {
            canvas.translate((subset.left, subset.top));
            canvas.draw_rect(
                SkRect::from_xywh(
                    src.left - subset.left,
                    src.top - subset.top,
                    src.width(),
                    src.height(),
                ),
                &paint,
            );
        } else {
            canvas.draw_rect(src, &paint);
        }
    } else {
        canvas.draw_image_rect_with_sampling_options(
            &texture.image,
            Some((&src, skia_safe::canvas::SrcRectConstraint::Strict)),
            src,
            sampling(node.sampling),
            &paint,
        );
    }
    canvas.restore();
    Ok(())
}

fn font_slot(
    plan: &PlanProgram,
    key: &FontKey,
    digest: ContentDigest,
) -> Result<ExternalSlotId, DrawError> {
    plan.resources
        .fonts
        .iter()
        .find(|binding| binding.face_hash == digest && binding.face_index == key.face_index)
        .map(|binding| binding.slot)
        .ok_or_else(|| DrawError::MissingFont {
            hash: key.face_hash.as_hex(),
            index: key.face_index,
        })
}

fn structure_slot(
    bindings: &[valle_engine::compositor::lower::PlanStructureBinding],
    key: &str,
    kind: &'static str,
) -> Result<ExternalSlotId, DrawError> {
    bindings
        .iter()
        .find(|binding| binding.key == key)
        .map(|binding| binding.slot)
        .ok_or_else(|| DrawError::MissingStructure {
            kind,
            key: key.to_owned(),
        })
}

fn validate_shader_abi(
    effect: &RuntimeEffect,
    requirement: &RuntimeShaderKey,
) -> Result<(), DrawError> {
    let has_resolution = effect
        .find_uniform("resolution")
        .is_some_and(|uniform| uniform.size_in_bytes() == 8);
    let has_content = effect
        .find_child("content")
        .is_some_and(|child| child.index() == 0);
    if !has_resolution || !has_content || !effect.allow_shader() {
        return Err(DrawError::ShaderAbi {
            uri: requirement.uri.clone(),
        });
    }
    Ok(())
}

fn preflight_shader_instance(
    uri: &str,
    bounds: Rect,
    uniforms: &[ShaderUniformBinding],
    texture_bindings: &[ShaderTextureBinding],
    shaders: &BTreeMap<String, Arc<RuntimeEffect>>,
    textures: &BTreeMap<ExternalTexture, ProgramTexture>,
) -> Result<(), DrawError> {
    let effect = shaders
        .get(uri)
        .ok_or_else(|| DrawError::MissingShader(uri.to_owned()))?;
    let _ = shader_uniforms(effect, bounds, uniforms, texture_bindings, textures)?;
    let mut bound_children = 0_usize;
    for child in effect.children() {
        if child.name() == "content" {
            bound_children += 1;
            continue;
        }
        let binding = texture_bindings
            .iter()
            .find(|binding| binding.name == child.name())
            .ok_or_else(|| DrawError::ShaderChild {
                uri: uri.to_owned(),
                child: child.name().to_owned(),
            })?;
        if let Some(texture) = &binding.texture
            && !textures.contains_key(texture)
        {
            return Err(DrawError::MissingTexture(texture.key.clone()));
        }
        bound_children += 1;
    }
    if bound_children != texture_bindings.len() + 1
        || effect
            .children()
            .iter()
            .filter(|child| child.name() == "content")
            .count()
            != 1
    {
        return Err(DrawError::ShaderAbi {
            uri: uri.to_owned(),
        });
    }
    Ok(())
}

fn resource(
    resources: &BTreeMap<ProgramResourceId, Option<ProgramImage>>,
    id: ProgramResourceId,
) -> Result<Option<&ProgramImage>, DrawError> {
    resources
        .get(&id)
        .map(Option::as_ref)
        .ok_or(DrawError::MissingProgramResource(id.get()))
}

/// Pass kinds whose result is their output ROI, drawn through the canvas, so a scratch target
/// may be clipped to that ROI. Clipping renders exactly as a ROI-sized surface would; without
/// it, Skia's antialiasing and wide blurs near the ROI edge depended on how large a slot
/// surface other resources had made. Blends can write whole surfaces directly, and shader,
/// glass and mask passes keep the unclipped path.
fn draws_within_roi(pass: &ProgramPassKind) -> bool {
    matches!(
        pass,
        ProgramPassKind::Clear { .. }
            | ProgramPassKind::RasterNode { .. }
            | ProgramPassKind::RasterTree { .. }
            | ProgramPassKind::SourceOver { .. }
            | ProgramPassKind::ApplyClip { .. }
            | ProgramPassKind::ApplyFilter { .. }
            | ProgramPassKind::ApplyOpacity { .. }
            | ProgramPassKind::ApplyTransform { .. }
    )
}

fn clear_surface(surface: &mut Surface) -> Result<(), DrawError> {
    surface.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    Ok(())
}

fn program_target_mut<'a>(
    surfaces: &'a mut ScratchSurfaces<'_, '_>,
    target: ProgramTerminal,
) -> Result<&'a mut Surface, DrawError> {
    match target {
        ProgramTerminal::Plan { slot, .. } => Ok(surfaces.output_mut(slot)?),
        ProgramTerminal::Scratch(index) => Ok(surfaces.surface_mut(index)?),
    }
}

fn snapshot_program_target(
    target: &mut Surface,
    location: ProgramTerminal,
    roi: DeviceRect,
) -> Result<Image, DrawError> {
    match location {
        ProgramTerminal::Plan { .. } => {
            let width = i32::try_from(roi.width)
                .map_err(|_| DrawError::Internal("program ROI width overflow".into()))?;
            let height = i32::try_from(roi.height)
                .map_err(|_| DrawError::Internal("program ROI height overflow".into()))?;
            target
                .image_snapshot_with_bounds(IRect::from_xywh(0, 0, width, height))
                .ok_or_else(|| DrawError::Internal("program target snapshot failed".into()))
        }
        ProgramTerminal::Scratch(_) => Ok(target.image_snapshot()),
    }
}

fn draw_program_image_with_mode(
    surface: &mut Surface,
    image: &ProgramImage,
    target_origin: [i32; 2],
    mode: SkBlendMode,
    opacity: f32,
) {
    let mut paint = SkPaint::default();
    paint.set_blend_mode(mode);
    paint.set_alpha_f(opacity);
    draw_program_image_with_paint(surface, image, target_origin, &paint);
}

fn draw_program_image_with_paint(
    surface: &mut Surface,
    image: &ProgramImage,
    target_origin: [i32; 2],
    paint: &SkPaint,
) {
    if image.roi.is_empty() {
        return;
    }
    let source = SkRect::from_xywh(0.0, 0.0, image.roi.width as f32, image.roi.height as f32);
    let destination = SkRect::from_xywh(
        (image.roi.x - target_origin[0]) as f32,
        (image.roi.y - target_origin[1]) as f32,
        image.roi.width as f32,
        image.roi.height as f32,
    );
    surface.canvas().draw_image_rect_with_sampling_options(
        &image.image,
        Some((&source, skia_safe::canvas::SrcRectConstraint::Strict)),
        destination,
        SamplingOptions::default(),
        paint,
    );
}

/// Whether an unscaled `Src` draw of `image` overwrites every pixel the canvas can write, making
/// a clear before it redundant. Accumulating layers copy a full-frame destination this way for
/// every source they add.
fn covers_target(surface: &mut Surface, image: &ProgramImage, target_origin: [i32; 2]) -> bool {
    let (Ok(width), Ok(height)) = (
        i32::try_from(image.roi.width),
        i32::try_from(image.roi.height),
    ) else {
        return false;
    };
    let canvas = surface.canvas();
    let drawn = IRect::from_xywh(
        image.roi.x - target_origin[0],
        image.roi.y - target_origin[1],
        width,
        height,
    );
    canvas.local_to_device_as_3x3().is_identity()
        && canvas.device_clip_bounds().is_some_and(|target| {
            drawn.left <= target.left
                && drawn.top <= target.top
                && drawn.right >= target.right
                && drawn.bottom >= target.bottom
        })
}

fn copy_optional_into(
    surface: &mut Surface,
    image: Option<&ProgramImage>,
    target_origin: [i32; 2],
) -> Result<(), DrawError> {
    if !image.is_some_and(|image| covers_target(surface, image, target_origin)) {
        clear_surface(surface)?;
    }
    if let Some(image) = image {
        draw_program_image_with_mode(surface, image, target_origin, SkBlendMode::Src, 1.0);
    }
    Ok(())
}

fn source_over_into(
    surface: &mut Surface,
    source: Option<&ProgramImage>,
    destination: Option<&ProgramImage>,
    target_origin: [i32; 2],
) -> Result<(), DrawError> {
    if !destination.is_some_and(|destination| covers_target(surface, destination, target_origin)) {
        clear_surface(surface)?;
    }
    if let Some(destination) = destination {
        draw_program_image_with_mode(surface, destination, target_origin, SkBlendMode::Src, 1.0);
    }
    if let Some(source) = source {
        draw_program_image_with_mode(surface, source, target_origin, SkBlendMode::SrcOver, 1.0);
    }
    Ok(())
}

fn opacity_image_into(
    surface: &mut Surface,
    input: Option<&ProgramImage>,
    opacity: f32,
    target_origin: [i32; 2],
) -> Result<(), DrawError> {
    if !(opacity == 1.0 && input.is_some_and(|input| covers_target(surface, input, target_origin)))
    {
        clear_surface(surface)?;
    }
    if let Some(input) = input
        && opacity > 0.0
    {
        draw_program_image_with_mode(surface, input, target_origin, SkBlendMode::Src, opacity);
    }
    Ok(())
}

fn clip_image_into(
    surface: &mut Surface,
    input: Option<&ProgramImage>,
    clip: &Clip,
    matrix: Matrix,
    paths: &[PathData],
    target_origin: [i32; 2],
) -> Result<(), DrawError> {
    clear_surface(surface)?;
    let Some(input) = input else {
        return Ok(());
    };
    // Skia's transformed rrect/rect mask and raster-clip paths use different edge
    // coverage algorithms. Keep the existing coverage for those transforms and GPU devices.
    let mask_coverage = surface.canvas().peek_pixels().is_none()
        || match clip {
            Clip::Rect(_) => matrix.has_perspective(),
            Clip::RoundRect(_) => !matrix.rect_stays_rect(),
            Clip::Path { .. } => true,
        };
    if mask_coverage {
        let outline = match clip {
            Clip::Path {
                path: id,
                fill_rule,
            } => Some(path(
                paths
                    .get(id.raw() as usize)
                    .ok_or(DrawError::MissingPath(id.raw()))?,
                *fill_rule,
            )?),
            _ => None,
        };
        let mut paint = SkPaint::default();
        paint
            .set_anti_alias(true)
            .set_blend_mode(SkBlendMode::Src)
            .set_color(skia_safe::Color::WHITE);
        let canvas = surface.canvas();
        canvas.save();
        canvas.translate((-target_origin[0] as f32, -target_origin[1] as f32));
        canvas.concat(&matrix);
        match clip {
            Clip::Rect(rect) => {
                canvas.draw_rect(sk_rect(*rect), &paint);
            }
            Clip::RoundRect(rect) => {
                canvas.draw_rrect(sk_rrect(*rect), &paint);
            }
            Clip::Path { .. } => {
                canvas.draw_path(outline.as_ref().expect("path outline"), &paint);
            }
        }
        canvas.restore();
        draw_program_image_with_mode(surface, input, target_origin, SkBlendMode::SrcIn, 1.0);
        return Ok(());
    }
    let canvas = surface.canvas();
    let saved_matrix = canvas.local_to_device();
    canvas.save();
    canvas.translate((-target_origin[0] as f32, -target_origin[1] as f32));
    canvas.concat(&matrix);
    let result = clip_canvas(canvas, clip, paths);
    canvas.set_matrix(&saved_matrix);
    if result.is_ok() {
        // The input already contains the complete group: apply coverage once, after
        // composing overlapping children, just as the former mask + SrcIn path did.
        draw_program_image_with_mode(surface, input, target_origin, SkBlendMode::SrcOver, 1.0);
    }
    surface.canvas().restore();
    result?;
    Ok(())
}

fn apply_mask_into(
    surface: &mut Surface,
    effect: Option<&RuntimeEffect>,
    input: Option<&ProgramImage>,
    mask: Option<&ProgramImage>,
    mode: MaskMode,
    target_origin: [i32; 2],
) -> Result<(), DrawError> {
    clear_surface(surface)?;
    let Some(input) = input else {
        return Ok(());
    };
    if mask.is_none() && !mode.is_inverted() {
        return Ok(());
    }
    draw_program_image_with_mode(surface, input, target_origin, SkBlendMode::Src, 1.0);
    let Some(mask) = mask else {
        return Ok(());
    };
    let mut paint = SkPaint::default();
    paint.set_blend_mode(if mode.is_inverted() {
        SkBlendMode::DstOut
    } else {
        SkBlendMode::DstIn
    });
    let local = Matrix::translate((
        (mask.roi.x - target_origin[0]) as f32,
        (mask.roi.y - target_origin[1]) as f32,
    ));
    let shader = mask
        .image
        .to_shader(
            (TileMode::Decal, TileMode::Decal),
            SamplingOptions::default(),
            Some(&local),
        )
        .ok_or_else(|| DrawError::Internal("mask image shader failed".into()))?;
    let shader = if mode.is_luminance() {
        effect
            .ok_or_else(|| DrawError::Internal("mask coverage kernel was not admitted".into()))?
            .make_shader(Data::new_empty(), &[ChildPtr::Shader(shader)], None)
            .ok_or_else(|| DrawError::Internal("mask coverage shader failed".into()))?
    } else {
        shader
    };
    paint.set_shader(shader);
    surface.canvas().draw_paint(&paint);
    Ok(())
}

fn apply_filter_program_into(
    surface: &mut Surface,
    input: &ProgramImage,
    filter: &Filter,
    local_to_device: [f64; 9],
    target_origin: [i32; 2],
    output_roi: DeviceRect,
    lens_frame: Option<[f32; 4]>,
) -> Result<(), DrawError> {
    let filter = program_filter_in_device_space(filter, local_to_device)?;
    if surface.canvas().peek_pixels().is_none() {
        let source_roi = IRect::from_xywh(0, 0, input.roi.width as i32, input.roi.height as i32);
        let offset = [
            input.roi.x - target_origin[0],
            input.roi.y - target_origin[1],
        ];
        let output_bounds = IRect::from_xywh(
            output_roi.x - target_origin[0],
            output_roi.y - target_origin[1],
            i32::try_from(output_roi.width)
                .map_err(|_| DrawError::Internal("filter ROI width overflow".into()))?,
            i32::try_from(output_roi.height)
                .map_err(|_| DrawError::Internal("filter ROI height overflow".into()))?,
        );
        match &filter {
            Filter::Bloom {
                threshold,
                knee,
                intensity,
                radius,
            } if std::env::var_os("VALLE_BLOOM_CPU_REFERENCE").is_none() => {
                return super::bloom_gpu::apply_bloom_into(
                    surface,
                    &input.image,
                    source_roi,
                    offset,
                    output_bounds,
                    valle_engine::compositor::bloom::BloomParams {
                        threshold: *threshold,
                        knee: *knee,
                        intensity: *intensity,
                        radius: *radius,
                    },
                );
            }
            Filter::Glow {
                color,
                intensity,
                radius,
            } if std::env::var_os("VALLE_GLOW_CPU_REFERENCE").is_none() => {
                let color = color.to_working();
                return super::bloom_gpu::apply_glow_into(
                    surface,
                    &input.image,
                    source_roi,
                    offset,
                    output_bounds,
                    valle_engine::compositor::bloom::GlowParams {
                        color: [color.red, color.green, color.blue, color.alpha],
                        intensity: *intensity,
                        radius: *radius,
                    },
                );
            }
            Filter::RadialBlur { center, amount }
                if std::env::var_os("VALLE_RADIAL_BLUR_CPU_REFERENCE").is_none() =>
            {
                return super::radial_gpu::apply_radial_blur_into(
                    surface,
                    &input.image,
                    [input.roi.width as i32, input.roi.height as i32],
                    offset,
                    target_origin,
                    valle_engine::compositor::radial::RadialBlurParams {
                        center: *center,
                        amount: *amount,
                    },
                );
            }
            Filter::FilmGrain { seed, amount, size }
                if std::env::var_os("VALLE_FILM_GRAIN_CPU_REFERENCE").is_none() =>
            {
                return super::film_gpu::apply_film_grain_into(
                    surface,
                    &input.image,
                    [input.roi.width as i32, input.roi.height as i32],
                    offset,
                    target_origin,
                    valle_engine::compositor::film::FilmGrainParams {
                        seed: *seed,
                        amount: *amount,
                        size: *size,
                    },
                );
            }
            Filter::LensDistortion { k1, k2 }
                if std::env::var_os("VALLE_LENS_DISTORTION_CPU_REFERENCE").is_none() =>
            {
                return super::lens_gpu::apply_lens_distortion_into(
                    surface,
                    &input.image,
                    [input.roi.width as i32, input.roi.height as i32],
                    offset,
                    target_origin,
                    valle_engine::compositor::lens::LensDistortionParams {
                        k1: *k1,
                        k2: *k2,
                        frame: lens_frame.ok_or_else(|| {
                            DrawError::Unsupported("lens distortion frame is missing".into())
                        })?,
                    },
                );
            }
            _ => {}
        }
    }
    if uses_cpu_f16_filter(&filter) {
        return apply_f16_filter_program_into(
            surface,
            input,
            target_origin,
            output_roi,
            &filter,
            lens_frame,
        );
    }
    apply_filter_chain_program_into(surface, input, std::slice::from_ref(&filter), target_origin)
}

fn uses_cpu_f16_filter(filter: &Filter) -> bool {
    matches!(
        filter,
        Filter::Bloom { .. }
            | Filter::Glow { .. }
            | Filter::RadialBlur { .. }
            | Filter::FilmGrain { .. }
            | Filter::LensDistortion { .. }
    )
}

// Keep the CPU implementation selectable for pixel/performance reference runs.
fn uses_gpu_f16_filter(filter: &Filter, backend: SkiaBackendKind) -> bool {
    backend != SkiaBackendKind::Raster
        && match filter {
            Filter::Bloom { .. } => std::env::var_os("VALLE_BLOOM_CPU_REFERENCE").is_none(),
            Filter::Glow { .. } => std::env::var_os("VALLE_GLOW_CPU_REFERENCE").is_none(),
            Filter::RadialBlur { .. } => {
                std::env::var_os("VALLE_RADIAL_BLUR_CPU_REFERENCE").is_none()
            }
            Filter::FilmGrain { .. } => {
                std::env::var_os("VALLE_FILM_GRAIN_CPU_REFERENCE").is_none()
            }
            Filter::LensDistortion { .. } => {
                std::env::var_os("VALLE_LENS_DISTORTION_CPU_REFERENCE").is_none()
            }
            _ => false,
        }
}

fn apply_f16_filter_program_into(
    surface: &mut Surface,
    input: &ProgramImage,
    target_origin: [i32; 2],
    output_roi: DeviceRect,
    filter: &Filter,
    lens_frame: Option<[f32; 4]>,
) -> Result<(), DrawError> {
    let width = input.roi.width as usize;
    let height = input.roi.height as usize;
    let info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::RGBAF16,
        AlphaType::Premul,
        input.image.color_space(),
    );
    let mut bytes = vec![0_u8; width * height * 8];
    let copy_started = std::time::Instant::now();
    if !input
        .image
        .read_pixels(&info, &mut bytes, width * 8, (0, 0), CachingHint::Disallow)
    {
        return Err(DrawError::Surface(
            "shared F16 filter input read failed".into(),
        ));
    }
    if std::env::var_os("VALLE_TRACE_F16_STAGES").is_some() {
        eprintln!(
            "[valle f16] cpu-input-copy {:.3}ms",
            copy_started.elapsed().as_secs_f64() * 1_000.0
        );
    }
    // The bound pass owns only this ROI; pooled surface capacity is not output.
    let output_info = surface.image_info().with_dimensions((
        i32::try_from(output_roi.width)
            .map_err(|_| DrawError::Internal("filter ROI width overflow".into()))?,
        i32::try_from(output_roi.height)
            .map_err(|_| DrawError::Internal("filter ROI height overflow".into()))?,
    ));
    let dimensions = (
        width as u32,
        height as u32,
        output_info.width() as u32,
        output_info.height() as u32,
    );
    let offset = [input.roi.x - output_roi.x, input.roi.y - output_roi.y];
    let output_origin = [output_roi.x, output_roi.y];
    let kernel_started = std::time::Instant::now();
    let output = match filter {
        Filter::Bloom {
            threshold,
            knee,
            intensity,
            radius,
        } => valle_engine::compositor::bloom::apply_bloom_f16(
            &bytes,
            dimensions.0,
            dimensions.1,
            dimensions.2,
            dimensions.3,
            offset,
            valle_engine::compositor::bloom::BloomParams {
                threshold: *threshold,
                knee: *knee,
                intensity: *intensity,
                radius: *radius,
            },
        ),
        Filter::Glow {
            color,
            intensity,
            radius,
        } => {
            let color = color.to_working();
            valle_engine::compositor::bloom::apply_glow_f16(
                &bytes,
                dimensions.0,
                dimensions.1,
                dimensions.2,
                dimensions.3,
                offset,
                valle_engine::compositor::bloom::GlowParams {
                    color: [color.red, color.green, color.blue, color.alpha],
                    intensity: *intensity,
                    radius: *radius,
                },
            )
        }
        Filter::RadialBlur { center, amount } => {
            valle_engine::compositor::radial::apply_radial_blur_f16(
                &bytes,
                dimensions.0,
                dimensions.1,
                dimensions.2,
                dimensions.3,
                offset,
                output_origin,
                valle_engine::compositor::radial::RadialBlurParams {
                    center: *center,
                    amount: *amount,
                },
            )
        }
        Filter::FilmGrain { seed, amount, size } => {
            valle_engine::compositor::film::apply_film_grain_f16(
                &bytes,
                dimensions.0,
                dimensions.1,
                dimensions.2,
                dimensions.3,
                offset,
                output_origin,
                valle_engine::compositor::film::FilmGrainParams {
                    seed: *seed,
                    amount: *amount,
                    size: *size,
                },
            )
        }
        Filter::LensDistortion { k1, k2 } => {
            valle_engine::compositor::lens::apply_lens_distortion_f16(
                &bytes,
                dimensions.0,
                dimensions.1,
                dimensions.2,
                dimensions.3,
                offset,
                output_origin,
                valle_engine::compositor::lens::LensDistortionParams {
                    k1: *k1,
                    k2: *k2,
                    frame: lens_frame.ok_or_else(|| {
                        DrawError::Unsupported("lens distortion frame is missing".into())
                    })?,
                },
            )
        }
        _ => unreachable!("only shared F16 filters reach this pass"),
    }
    .map_err(|error| DrawError::Surface(error.to_string()))?;
    if std::env::var_os("VALLE_TRACE_F16_STAGES").is_some() {
        eprintln!(
            "[valle f16] cpu-kernel {:.3}ms ({}x{})",
            kernel_started.elapsed().as_secs_f64() * 1_000.0,
            dimensions.2,
            dimensions.3,
        );
    }
    let upload_started = std::time::Instant::now();
    let raster_info = info.with_dimensions((output_info.width(), output_info.height()));
    let image = images::raster_from_data(
        &raster_info,
        Data::new_copy(&output),
        output_info.width() as usize * 8,
    )
    .ok_or_else(|| DrawError::Surface("shared F16 output allocation failed".into()))?;
    clear_surface(surface)?;
    let mut paint = SkPaint::default();
    paint.set_blend_mode(SkBlendMode::Src);
    surface.canvas().draw_image_with_sampling_options(
        &image,
        (
            (output_roi.x - target_origin[0]) as f32,
            (output_roi.y - target_origin[1]) as f32,
        ),
        SamplingOptions::default(),
        Some(&paint),
    );
    if std::env::var_os("VALLE_TRACE_F16_STAGES").is_some() {
        eprintln!(
            "[valle f16] cpu-to-gpu-submit {:.3}ms",
            upload_started.elapsed().as_secs_f64() * 1_000.0
        );
    }
    Ok(())
}

fn apply_filter_chain_program_into(
    surface: &mut Surface,
    input: &ProgramImage,
    filters: &[Filter],
    target_origin: [i32; 2],
) -> Result<(), DrawError> {
    clear_surface(surface)?;
    draw_filters(
        surface.canvas(),
        &input.image,
        IRect::from_xywh(0, 0, input.roi.width as i32, input.roi.height as i32),
        [
            input.roi.x - target_origin[0],
            input.roi.y - target_origin[1],
        ],
        filters,
    )
}

/// Map the spatial part of a program-local filter into the materialized device ROI.
///
/// The production executor currently has axis-aligned Skia image-filter kernels. Caption's
/// affine similarity preserves its isotropic Gaussian exactly, including rotation/reflection.
/// Other affine transforms are accepted only when their mapped Gaussian covariance remains
/// axis-aligned; projective or rotated-anisotropic cases fail closed instead of silently changing
/// program-local sigma into device pixels.
fn program_filter_in_device_space(
    filter: &Filter,
    local_to_device: [f64; 9],
) -> Result<Filter, DrawError> {
    match filter {
        Filter::Blur { sigma_x, sigma_y } => {
            let [sigma_x, sigma_y] = program_gaussian_in_device_space(
                [f64::from(*sigma_x), f64::from(*sigma_y)],
                local_to_device,
            )?;
            Ok(Filter::Blur {
                sigma_x: checked_device_filter_value(sigma_x)?,
                sigma_y: checked_device_filter_value(sigma_y)?,
            })
        }
        Filter::DropShadow {
            offset,
            sigma_x,
            sigma_y,
            color,
        } => {
            let [sigma_x, sigma_y] = program_gaussian_in_device_space(
                [f64::from(*sigma_x), f64::from(*sigma_y)],
                local_to_device,
            )?;
            let [a, b, _, d, e, _, _, _, _] = local_to_device;
            let offset = [
                a * f64::from(offset[0]) + b * f64::from(offset[1]),
                d * f64::from(offset[0]) + e * f64::from(offset[1]),
            ];
            Ok(Filter::DropShadow {
                offset: [
                    checked_device_filter_value(offset[0])?,
                    checked_device_filter_value(offset[1])?,
                ],
                sigma_x: checked_device_filter_value(sigma_x)?,
                sigma_y: checked_device_filter_value(sigma_y)?,
                color: *color,
            })
        }
        Filter::Glow {
            color,
            radius,
            intensity,
        } => {
            let [x, y] = program_gaussian_in_device_space(
                [f64::from(*radius), f64::from(*radius)],
                local_to_device,
            )?;
            if (x - y).abs() > x.max(y).max(1.0) * 1.0e-9 {
                return Err(DrawError::Unsupported(
                    "glow radius needs a similarity device transform".into(),
                ));
            }
            Ok(Filter::Glow {
                color: *color,
                radius: checked_device_filter_value((x + y) * 0.5)?,
                intensity: *intensity,
            })
        }
        Filter::Bloom {
            threshold,
            knee,
            intensity,
            radius,
        } => {
            let [x, y] = program_gaussian_in_device_space(
                [f64::from(*radius), f64::from(*radius)],
                local_to_device,
            )?;
            if (x - y).abs() > x.max(y).max(1.0) * 1.0e-9 {
                return Err(DrawError::Unsupported(
                    "bloom radius needs a similarity device transform".into(),
                ));
            }
            Ok(Filter::Bloom {
                threshold: *threshold,
                knee: *knee,
                intensity: *intensity,
                radius: checked_device_filter_value((x + y) * 0.5)?,
            })
        }
        Filter::RadialBlur { center, amount } => {
            let [x, y] = program_gaussian_in_device_space(
                [f64::from(*amount), f64::from(*amount)],
                local_to_device,
            )?;
            if (x - y).abs() > x.max(y).max(1.0) * 1.0e-9 {
                return Err(DrawError::Unsupported(
                    "radial blur amount needs a similarity device transform".into(),
                ));
            }
            let [a, b, c, d, e, f, _, _, _] = local_to_device;
            Ok(Filter::RadialBlur {
                center: [
                    checked_device_filter_value(
                        a * f64::from(center[0]) + b * f64::from(center[1]) + c,
                    )?,
                    checked_device_filter_value(
                        d * f64::from(center[0]) + e * f64::from(center[1]) + f,
                    )?,
                ],
                amount: checked_device_filter_value((x + y) * 0.5)?,
            })
        }
        Filter::FilmGrain { seed, amount, size } => {
            let [x, y] = program_gaussian_in_device_space(
                [f64::from(*size), f64::from(*size)],
                local_to_device,
            )?;
            if (x - y).abs() > x.max(y).max(1.0) * 1.0e-9 {
                return Err(DrawError::Unsupported(
                    "film grain size needs a similarity device transform".into(),
                ));
            }
            Ok(Filter::FilmGrain {
                seed: *seed,
                amount: *amount,
                size: checked_device_filter_value(((x + y) * 0.5).max(1.0))?,
            })
        }
        Filter::ChromaticAberration { offset } => {
            let [a, b, _, d, e, _, g, h, i] = local_to_device;
            if local_to_device.iter().any(|value| !value.is_finite())
                || g.abs() > 1.0e-12
                || h.abs() > 1.0e-12
                || (i - 1.0).abs() > 1.0e-12
            {
                return Err(DrawError::Unsupported(
                    "chromatic aberration needs an affine device transform".into(),
                ));
            }
            Ok(Filter::ChromaticAberration {
                offset: [
                    checked_device_filter_value(
                        a * f64::from(offset[0]) + b * f64::from(offset[1]),
                    )?,
                    checked_device_filter_value(
                        d * f64::from(offset[0]) + e * f64::from(offset[1]),
                    )?,
                ],
            })
        }
        _ => Ok(filter.clone()),
    }
}

fn program_gaussian_in_device_space(
    sigma: [f64; 2],
    local_to_device: [f64; 9],
) -> Result<[f64; 2], DrawError> {
    let [a, b, _, d, e, _, g, h, i] = local_to_device;
    if local_to_device.iter().any(|value| !value.is_finite())
        || g.abs() > 1.0e-12
        || h.abs() > 1.0e-12
        || (i - 1.0).abs() > 1.0e-12
    {
        return Err(DrawError::Unsupported(
            "program-local spatial filter requires an affine device transform".into(),
        ));
    }
    let variance_x = (a * sigma[0]).powi(2) + (b * sigma[1]).powi(2);
    let variance_y = (d * sigma[0]).powi(2) + (e * sigma[1]).powi(2);
    let covariance = a * d * sigma[0].powi(2) + b * e * sigma[1].powi(2);
    let variance_scale = variance_x.max(variance_y).max(1.0);
    if covariance.abs() > variance_scale * 1.0e-9 {
        return Err(DrawError::Unsupported(
            "rotated program-local anisotropic blur has no axis-aligned Skia equivalent".into(),
        ));
    }
    Ok([variance_x.sqrt(), variance_y.sqrt()])
}

fn checked_device_filter_value(value: f64) -> Result<f32, DrawError> {
    if !value.is_finite() || value.abs() > f64::from(f32::MAX) {
        return Err(DrawError::Unsupported(
            "program-local spatial filter exceeds the device numeric domain".into(),
        ));
    }
    Ok(value as f32)
}

fn clip_canvas(
    canvas: &skia_safe::Canvas,
    clip: &Clip,
    paths: &[PathData],
) -> Result<(), DrawError> {
    match clip {
        Clip::Rect(rect) => {
            canvas.clip_rect(sk_rect(*rect), ClipOp::Intersect, true);
        }
        Clip::RoundRect(rect) => {
            canvas.clip_rrect(sk_rrect(*rect), ClipOp::Intersect, true);
        }
        Clip::Path {
            path: id,
            fill_rule,
        } => {
            let data = paths
                .get(id.raw() as usize)
                .ok_or(DrawError::MissingPath(id.raw()))?;
            canvas.clip_path(&path(data, *fill_rule)?, ClipOp::Intersect, true);
        }
    }
    Ok(())
}

fn path(data: &PathData, fill_rule: FillRule) -> Result<SkPath, DrawError> {
    let mut builder = PathBuilder::new();
    builder.set_fill_type(match fill_rule {
        FillRule::NonZero => PathFillType::Winding,
        FillRule::EvenOdd => PathFillType::EvenOdd,
    });
    let mut point = 0_usize;
    let take = |index: &mut usize| -> Result<SkPoint, DrawError> {
        let value = data
            .points
            .get(*index)
            .ok_or_else(|| DrawError::Internal("path point underflow".into()))?;
        *index += 1;
        Ok(SkPoint::new(value[0] as f32, value[1] as f32))
    };
    for verb in &data.verbs {
        match verb {
            PathVerb::MoveTo => {
                builder.move_to(take(&mut point)?);
            }
            PathVerb::LineTo => {
                builder.line_to(take(&mut point)?);
            }
            PathVerb::QuadTo => {
                let control = take(&mut point)?;
                let end = take(&mut point)?;
                builder.quad_to(control, end);
            }
            PathVerb::CubicTo => {
                let first = take(&mut point)?;
                let second = take(&mut point)?;
                let end = take(&mut point)?;
                builder.cubic_to(first, second, end);
            }
            PathVerb::Close => {
                builder.close();
            }
        };
    }
    if point != data.points.len() {
        return Err(DrawError::Internal("path has trailing points".into()));
    }
    Ok(builder.detach())
}

/// Skia's CPU and Metal path rasterizers disagree at the antialiased edge of a
/// four-corner rounded box. Shared F16 filters magnify those few differing input
/// pixels, so feed the Metal surface CPU-generated coverage for this small solid
/// shape. Raster keeps its normal path and pixel output.
fn draw_rounded_path_with_shared_coverage(
    canvas: &skia_safe::Canvas,
    data: &PathData,
    path: &SkPath,
    paint: &SkPaint,
) -> Result<bool, DrawError> {
    use PathVerb::{Close, CubicTo, LineTo, MoveTo};

    // The shape emitted by PathBuilder::elliptical_rrect when all four corners
    // are rounded. Keep other paths on their normal GPU/CPU rendering route.
    if data.verbs.as_slice()
        != [
            MoveTo, LineTo, CubicTo, LineTo, CubicTo, LineTo, CubicTo, LineTo, CubicTo, Close,
        ]
        || data.points.len() != 17
    {
        return Ok(false);
    }
    // CPU surfaces already use the reference coverage and retain their output.
    if canvas.peek_pixels().is_some() {
        return Ok(false);
    }
    let matrix = canvas.local_to_device();
    let unit_translation = (0..4).all(|row| {
        (0..4).all(|col| {
            let value = matrix.rc(row, col);
            if col == 3 && row < 2 {
                value.is_finite() && value.fract() == 0.0
            } else {
                value == if row == col { 1.0 } else { 0.0 }
            }
        })
    });
    if !unit_translation {
        return Ok(false);
    }
    let bounds = path.bounds();
    if ![bounds.left, bounds.top, bounds.right, bounds.bottom]
        .into_iter()
        .all(f32::is_finite)
    {
        return Ok(false);
    }
    let left = bounds.left.floor() - 1.0;
    let top = bounds.top.floor() - 1.0;
    let width = (bounds.right.ceil() + 1.0 - left) as i32;
    let height = (bounds.bottom.ceil() + 1.0 - top) as i32;
    if width <= 0 || height <= 0 || width > 512 || height > 512 {
        return Ok(false);
    }
    let info = ImageInfo::new(
        (width, height),
        ColorType::RGBAF16,
        AlphaType::Premul,
        Some(working_color_space()?),
    );
    let mut coverage = raster_surface(&info)?;
    coverage.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    coverage.canvas().translate((-left, -top));
    coverage.canvas().draw_path(path, paint);
    canvas.draw_image_with_sampling_options(
        coverage.image_snapshot(),
        (left, top),
        SamplingOptions::default(),
        Some(&SkPaint::default()),
    );
    Ok(true)
}

fn gradient_parts(stops: &[valle_draw::program::GradientStop]) -> (Vec<Color4f>, Vec<f32>) {
    let colors = stops
        .iter()
        .map(|stop| straight_color(stop.color))
        .collect::<Vec<_>>();
    let positions = stops.iter().map(|stop| stop.offset).collect::<Vec<_>>();
    (colors, positions)
}

fn gradient_interpolation() -> gradient::Interpolation {
    gradient::Interpolation {
        in_premul: gradient::interpolation::InPremul::Yes,
        ..Default::default()
    }
}

fn draw_shadow(
    canvas: &skia_safe::Canvas,
    shadow: &valle_draw::program::ShadowNode,
) -> Result<(), DrawError> {
    let mut paint = SkPaint::default();
    paint.set_anti_alias(true);
    set_working_color(&mut paint, shadow.color)?;
    if shadow.sigma_x > 0.0 || shadow.sigma_y > 0.0 {
        paint.set_mask_filter(skia_safe::MaskFilter::blur(
            skia_safe::BlurStyle::Normal,
            shadow.sigma_x.max(shadow.sigma_y),
            None,
        ));
    }
    let mut shape = shadow.shape;
    shape.rect.x += f64::from(shadow.offset[0] - shadow.spread);
    shape.rect.y += f64::from(shadow.offset[1] - shadow.spread);
    shape.rect.width += f64::from(shadow.spread * 2.0);
    shape.rect.height += f64::from(shadow.spread * 2.0);
    for radii in &mut shape.radii {
        radii[0] = (radii[0] + f64::from(shadow.spread)).max(0.0);
        radii[1] = (radii[1] + f64::from(shadow.spread)).max(0.0);
    }
    if shadow.inset {
        canvas.save();
        canvas.clip_rrect(sk_rrect(shadow.shape), ClipOp::Intersect, true);
        paint.set_blend_mode(SkBlendMode::SrcOver);
        canvas.draw_rrect(sk_rrect(shape), &paint);
        canvas.restore();
    } else {
        canvas.draw_rrect(sk_rrect(shape), &paint);
    }
    Ok(())
}

fn shader_uniforms(
    effect: &RuntimeEffect,
    bounds: Rect,
    bindings: &[ShaderUniformBinding],
    texture_bindings: &[ShaderTextureBinding],
    textures: &BTreeMap<ExternalTexture, ProgramTexture>,
) -> Result<Vec<u8>, DrawError> {
    let mut data = vec![0_u8; effect.uniform_size()];
    let resolution = effect
        .find_uniform("resolution")
        .ok_or_else(|| DrawError::Internal("shader has no resolution uniform".into()))?;
    write_f32s(
        &mut data,
        resolution.offset(),
        &[bounds.width as f32, bounds.height as f32],
    )?;
    for binding in texture_bindings {
        let name = format!("valle_size_{}", binding.name);
        let uniform = effect
            .find_uniform(&name)
            .ok_or_else(|| DrawError::ShaderUniform(name.clone()))?;
        if uniform.size_in_bytes() != 8 {
            return Err(DrawError::ShaderUniform(name));
        }
        let size = if let Some(texture) = &binding.texture {
            textures
                .get(texture)
                .ok_or_else(|| DrawError::MissingTexture(texture.key.clone()))?
                .size
        } else {
            [1, 1]
        };
        write_f32s(
            &mut data,
            uniform.offset(),
            &[size[0] as f32, size[1] as f32],
        )?;
    }
    for binding in bindings {
        let backend_name = if matches!(binding.value, ShaderUniformValue::Bool(_)) {
            format!("valle_uniform_{}", binding.name)
        } else {
            binding.name.clone()
        };
        let uniform = effect
            .find_uniform(&backend_name)
            .ok_or_else(|| DrawError::ShaderUniform(binding.name.clone()))?;
        let values = match binding.value {
            ShaderUniformValue::Float(value) => vec![value],
            ShaderUniformValue::Float2(value) => value.to_vec(),
            ShaderUniformValue::Float3(value) => value.to_vec(),
            ShaderUniformValue::Float4(value) => value.to_vec(),
            ShaderUniformValue::Float2x2(value) => value.to_vec(),
            ShaderUniformValue::Float3x3(value) => value.to_vec(),
            ShaderUniformValue::Float4x4(value) => value.to_vec(),
            ShaderUniformValue::Color(color) => color.to_vec(),
            ShaderUniformValue::Bool(value) => vec![f32::from(value)],
        };
        if uniform.size_in_bytes() != values.len() * 4 {
            return Err(DrawError::ShaderUniform(binding.name.clone()));
        }
        write_f32s(&mut data, uniform.offset(), &values)?;
    }
    if effect
        .uniforms()
        .iter()
        .filter(|uniform| uniform.name() != "resolution")
        .count()
        != bindings.len() + texture_bindings.len()
    {
        return Err(DrawError::ShaderUniformLayout);
    }
    Ok(data)
}

fn write_f32s(data: &mut [u8], offset: usize, values: &[f32]) -> Result<(), DrawError> {
    let end = offset
        .checked_add(values.len() * 4)
        .ok_or_else(|| DrawError::Internal("shader uniform offset overflow".into()))?;
    let destination = data
        .get_mut(offset..end)
        .ok_or_else(|| DrawError::Internal("shader uniform outside ABI buffer".into()))?;
    for (chunk, value) in destination.chunks_exact_mut(4).zip(values) {
        chunk.copy_from_slice(&value.to_ne_bytes());
    }
    Ok(())
}

fn draw_instance_batch(
    program: &DrawProgram,
    textures: &BTreeMap<ExternalTexture, ProgramTexture>,
    canvas: &skia_safe::Canvas,
    batch: &InstanceBatchNode,
) -> Result<(), DrawError> {
    let mut paint = SkPaint::default();
    paint.set_anti_alias(true);
    if let InstanceShape::Image(region) = &batch.shape
        && draw_opaque_image_atlas(textures, canvas, batch, region, &paint)?
    {
        return Ok(());
    }
    let (batch_path, path_local_bounds) = match &batch.shape {
        InstanceShape::Path(id) => {
            let data = program
                .paths()
                .get(id.raw() as usize)
                .ok_or(DrawError::MissingPath(id.raw()))?;
            // Use the DrawProgram's f64 control-point hull, just like ordinary Path geometry.
            // Skia's f32 path bounds can otherwise change the opacity layer's edge coverage.
            let bounds = data.points.first().map(|first| {
                let (mut left, mut top, mut right, mut bottom) =
                    (first[0], first[1], first[0], first[1]);
                for [x, y] in data.points.iter().copied().skip(1) {
                    left = left.min(x);
                    top = top.min(y);
                    right = right.max(x);
                    bottom = bottom.max(y);
                }
                Rect::from_edges(left, top, right, bottom)
            });
            (Some(path(data, FillRule::NonZero)?), bounds)
        }
        _ => (None, None),
    };
    if batch.shape == InstanceShape::Circle
        && let Some(first) = batch.instances.transforms.first()
        && first.0[0] > 0.0
        && batch.instances.transforms.iter().all(|transform| {
            transform.0[0] == first.0[0]
                && transform.0[1] == 0.0
                && transform.0[2] == 0.0
                && transform.0[3] == first.0[0]
        })
        && batch
            .instances
            .colors
            .iter()
            .all(|color| *color == batch.instances.colors[0])
        && batch
            .instances
            .opacities
            .iter()
            .all(|opacity| *opacity == 1.0)
        && batch
            .instances
            .stroke_widths
            .iter()
            .all(|width| *width == 0.0)
    {
        let points = batch
            .instances
            .transforms
            .iter()
            .map(|transform| SkPoint::new(transform.0[4] as f32, transform.0[5] as f32))
            .collect::<Vec<_>>();
        set_working_color(&mut paint, batch.instances.colors[0])?;
        paint.set_stroke_cap(PaintCap::Round);
        paint.set_stroke_width((first.0[0] * 2.0) as f32);
        canvas.draw_points(PointMode::Points, &points, &paint);
        return Ok(());
    }
    if let Some(style) = &batch.path_style {
        paint.set_stroke_cap(match style.cap {
            StrokeCap::Butt => PaintCap::Butt,
            StrokeCap::Round => PaintCap::Round,
            StrokeCap::Square => PaintCap::Square,
        });
        paint.set_stroke_join(match style.join {
            StrokeJoin::Miter => PaintJoin::Miter,
            StrokeJoin::Round => PaintJoin::Round,
            StrokeJoin::Bevel => PaintJoin::Bevel,
        });
        paint.set_stroke_miter(style.miter_limit);
    }
    let atlas_subset = if let InstanceShape::Image(region) = &batch.shape {
        let texture = textures
            .get(&region.texture)
            .ok_or_else(|| DrawError::MissingTexture(region.texture.key.clone()))?;
        atlas_pixel_subset(texture, region)?
    } else {
        None
    };
    for row in 0..batch.instances.len() {
        let opacity = batch.instances.opacities[row];
        let color = batch.instances.colors[row];
        let stroke_color = batch
            .instances
            .stroke_colors
            .get(row)
            .copied()
            .unwrap_or(color);
        let transform = batch.instances.transforms[row];
        let stroke_width = batch.instances.stroke_widths[row];
        if opacity <= 0.0
            || color.alpha <= 0.0 && (stroke_width <= 0.0 || stroke_color.alpha <= 0.0)
        {
            continue;
        }
        let local_bounds = match &batch.shape {
            InstanceShape::Path(_) => path_local_bounds.map(|bounds| {
                // Match an ordinary Path group's stroke footprint before saving its opacity
                // layer. An unbounded layer changes a few antialiased edge pixels after affine
                // transforms even when the path, paint and matrix are otherwise identical.
                let join_scale = batch
                    .path_style
                    .as_ref()
                    .map_or(4.0, |style| match style.join {
                        StrokeJoin::Miter => style.miter_limit,
                        StrokeJoin::Round | StrokeJoin::Bevel => 1.0,
                    });
                let outset = f64::from(stroke_width * 0.5 * join_scale);
                Rect::from_edges(
                    bounds.left() - outset,
                    bounds.top() - outset,
                    bounds.right() + outset,
                    bounds.bottom() + outset,
                )
            }),
            InstanceShape::Rect if stroke_width == 0.0 => Some(Rect::new(0.0, 0.0, 1.0, 1.0)),
            InstanceShape::RoundRect(round_rect) if stroke_width == 0.0 => Some(round_rect.rect),
            InstanceShape::Image(_) => Some(Rect::new(0.0, 0.0, 1.0, 1.0)),
            _ => None,
        };
        if opacity < 1.0 {
            let bounds = local_bounds
                .and_then(|rect| Transform2d::from_affine(transform).map_bounds(rect))
                .map(sk_rect);
            canvas.save_layer_alpha_f(bounds, opacity);
        }
        let [a, b, c, d, e, f] = transform.0;
        canvas.save();
        canvas.concat(&Matrix::new_all(
            a as f32, c as f32, e as f32, b as f32, d as f32, f as f32, 0.0, 0.0, 1.0,
        ));
        if let InstanceShape::Image(region) = &batch.shape {
            let texture = textures
                .get(&region.texture)
                .ok_or_else(|| DrawError::MissingTexture(region.texture.key.clone()))?;
            draw_program_texture(
                canvas,
                texture,
                &ImageNode {
                    texture: region.texture.clone(),
                    src: region.src,
                    dst: Rect::new(0.0, 0.0, 1.0, 1.0),
                    sampling: region.sampling,
                    opacity: 1.0,
                },
                Some(color),
                atlas_subset.as_ref(),
            )?;
        } else if color.alpha > 0.0 && batch.path_style.as_ref().is_none_or(|style| style.fill) {
            set_working_color(&mut paint, color)?;
            paint.set_style(PaintStyle::Fill);
            paint.set_path_effect(None);
            match &batch.shape {
                InstanceShape::Circle => canvas.draw_circle(SkPoint::new(0.0, 0.0), 1.0, &paint),
                InstanceShape::Rect => {
                    canvas.draw_rect(SkRect::from_xywh(0.0, 0.0, 1.0, 1.0), &paint)
                }
                InstanceShape::RoundRect(round_rect) => {
                    canvas.draw_rrect(sk_rrect(*round_rect), &paint)
                }
                InstanceShape::Path(_) => canvas.draw_path(
                    batch_path.as_ref().ok_or(DrawError::MissingPath(0))?,
                    &paint,
                ),
                InstanceShape::Image(_) => unreachable!("image batch uses texture sampling"),
            };
        }
        if stroke_width > 0.0 && stroke_color.alpha > 0.0 {
            let dashed_paint = if let Some(style) = &batch.path_style
                && !style.dash.is_empty()
            {
                let mut row_paint = SkPaint::default();
                row_paint.set_anti_alias(true);
                set_working_color(&mut row_paint, stroke_color)?;
                row_paint.set_style(PaintStyle::Stroke);
                row_paint.set_stroke_width(stroke_width);
                row_paint.set_stroke_miter(style.miter_limit);
                row_paint.set_stroke_cap(match style.cap {
                    StrokeCap::Butt => PaintCap::Butt,
                    StrokeCap::Round => PaintCap::Round,
                    StrokeCap::Square => PaintCap::Square,
                });
                row_paint.set_stroke_join(match style.join {
                    StrokeJoin::Miter => PaintJoin::Miter,
                    StrokeJoin::Round => PaintJoin::Round,
                    StrokeJoin::Bevel => PaintJoin::Bevel,
                });
                let offset = batch
                    .instances
                    .dash_offsets
                    .get(row)
                    .copied()
                    .unwrap_or(style.dash_offset);
                row_paint.set_path_effect(PathEffect::dash(&style.dash, offset));
                Some(row_paint)
            } else {
                None
            };
            let stroke_paint = if let Some(row_paint) = dashed_paint.as_ref() {
                row_paint
            } else {
                set_working_color(&mut paint, stroke_color)?;
                paint.set_style(PaintStyle::Stroke);
                paint.set_stroke_width(stroke_width);
                &paint
            };
            match &batch.shape {
                InstanceShape::Circle => {
                    canvas.draw_circle(SkPoint::new(0.0, 0.0), 1.0, stroke_paint)
                }
                InstanceShape::Rect => {
                    canvas.draw_rect(SkRect::from_xywh(0.0, 0.0, 1.0, 1.0), stroke_paint)
                }
                InstanceShape::RoundRect(round_rect) => {
                    canvas.draw_rrect(sk_rrect(*round_rect), stroke_paint)
                }
                InstanceShape::Path(_) => canvas.draw_path(
                    batch_path.as_ref().ok_or(DrawError::MissingPath(0))?,
                    stroke_paint,
                ),
                InstanceShape::Image(_) => unreachable!("image batch stroke rejected by admission"),
            };
        }
        canvas.restore();
        if opacity < 1.0 {
            canvas.restore();
        }
    }
    Ok(())
}

fn draw_opaque_image_atlas(
    textures: &BTreeMap<ExternalTexture, ProgramTexture>,
    canvas: &skia_safe::Canvas,
    batch: &InstanceBatchNode,
    region: &valle_draw::program::AtlasRegion,
    paint: &SkPaint,
) -> Result<bool, DrawError> {
    if batch.instances.is_empty()
        || batch
            .instances
            .colors
            .iter()
            .any(|color| *color != LinearColor::new(1.0, 1.0, 1.0, 1.0))
        || batch
            .instances
            .opacities
            .iter()
            .any(|opacity| *opacity != 1.0)
        || batch
            .instances
            .stroke_widths
            .iter()
            .any(|width| *width != 0.0)
    {
        return Ok(false);
    }
    let texture = textures
        .get(&region.texture)
        .ok_or_else(|| DrawError::MissingTexture(region.texture.key.clone()))?;
    let ExternalSample::Texture {
        texture_from_content,
        input_sample_bounds,
        ..
    } = ExternalSample::from_display_rect(
        region.src,
        texture.interpretation.ok_or_else(|| {
            DrawError::Unsupported("data texture cannot be drawn as an atlas".into())
        })?,
    )
    .map_err(|error| DrawError::Unsupported(error.to_string()))?
    else {
        return Ok(false);
    };
    let inverse = inverse3(texture_from_content)
        .ok_or_else(|| DrawError::Unsupported("non-invertible atlas sample".into()))?;
    let width = f64::from(texture.image.width());
    let height = f64::from(texture.image.height());
    let from_pixel = mul3(
        inverse,
        [1.0 / width, 0.0, 0.0, 0.0, 1.0 / height, 0.0, 0.0, 0.0, 1.0],
    );
    let mut src = SkRect::from_xywh(
        (input_sample_bounds.x * width) as f32,
        (input_sample_bounds.y * height) as f32,
        (input_sample_bounds.width * width) as f32,
        (input_sample_bounds.height * height) as f32,
    );
    let subset = atlas_pixel_subset(texture, region)?;
    let (atlas, from_pixel) = if let Some(subset) = subset.as_ref() {
        let offset = [
            1.0,
            0.0,
            f64::from(subset.left),
            0.0,
            1.0,
            f64::from(subset.top),
            0.0,
            0.0,
            1.0,
        ];
        let mapped = mul3(from_pixel, offset);
        src = SkRect::from_xywh(
            src.left - subset.left,
            src.top - subset.top,
            src.width(),
            src.height(),
        );
        (&subset.image, mapped)
    } else {
        (&texture.image, from_pixel)
    };
    let mut transforms = Vec::with_capacity(batch.instances.len());
    for transform in &batch.instances.transforms {
        let [a, b, c, d, e, f] = transform.0;
        let mapped = mul3([a, c, e, b, d, f, 0.0, 0.0, 1.0], from_pixel);
        let [scos, neg_ssin, tx, ssin, other_cos, ty, ..] = mapped;
        if !mapped.iter().all(|value| value.is_finite())
            || (scos - other_cos).abs() > 1e-6
            || (neg_ssin + ssin).abs() > 1e-6
            || mapped[6] != 0.0
            || mapped[7] != 0.0
            || mapped[8] != 1.0
        {
            return Ok(false);
        }
        transforms.push(RSXform::new(
            scos as f32,
            ssin as f32,
            // drawAtlas maps each source rectangle from a local (0, 0), including
            // fractional origins left after cropping the integer pixel subset.
            (
                (tx + scos * f64::from(src.left) + neg_ssin * f64::from(src.top)) as f32,
                (ty + ssin * f64::from(src.left) + other_cos * f64::from(src.top)) as f32,
            ),
        ));
    }
    canvas.draw_atlas(
        atlas,
        &transforms,
        &vec![src; transforms.len()],
        None::<&[Color]>,
        SkBlendMode::SrcOver,
        sampling(region.sampling),
        None::<SkRect>,
        Some(paint),
    );
    Ok(true)
}

struct AtlasPixelSubset {
    image: Image,
    left: f32,
    top: f32,
}

fn atlas_pixel_subset(
    texture: &ProgramTexture,
    region: &valle_draw::program::AtlasRegion,
) -> Result<Option<AtlasPixelSubset>, DrawError> {
    let ExternalSample::Texture {
        input_sample_bounds,
        ..
    } = ExternalSample::from_display_rect(
        region.src,
        texture.interpretation.ok_or_else(|| {
            DrawError::Unsupported("data texture cannot be drawn as an atlas".into())
        })?,
    )
    .map_err(|error| DrawError::Unsupported(error.to_string()))?
    else {
        return Ok(None);
    };
    let width = f64::from(texture.image.width());
    let height = f64::from(texture.image.height());
    let edges = [
        input_sample_bounds.x * width,
        input_sample_bounds.y * height,
        input_sample_bounds.right() * width,
        input_sample_bounds.bottom() * height,
    ];
    // Keep the texels surrounding fractional sample bounds, but exclude adjacent tiles.
    let [left, top, right, bottom] = [
        (edges[0].floor() as i32).max(0),
        (edges[1].floor() as i32).max(0),
        (edges[2].ceil() as i32).min(texture.image.width()),
        (edges[3].ceil() as i32).min(texture.image.height()),
    ];
    Ok(texture
        .image
        .make_subset(
            None,
            IRect::from_xywh(left, top, right - left, bottom - top),
            Default::default(),
        )
        .map(|image| AtlasPixelSubset {
            image,
            left: left as f32,
            top: top as f32,
        }))
}

fn sk_rect(rect: Rect) -> SkRect {
    SkRect::from_xywh(
        rect.x as f32,
        rect.y as f32,
        rect.width as f32,
        rect.height as f32,
    )
}

fn sk_rrect(rect: RoundRect) -> RRect {
    RRect::new_rect_radii(
        sk_rect(rect.rect),
        &rect
            .radii
            .map(|radius| SkPoint::new(radius[0] as f32, radius[1] as f32)),
    )
}

fn sampling(mode: SamplingMode) -> SamplingOptions {
    match mode {
        SamplingMode::NearestClamp => SamplingOptions::default(),
        SamplingMode::LinearClamp | SamplingMode::LinearDecal => {
            SamplingOptions::new(FilterMode::Linear, MipmapMode::None)
        }
        SamplingMode::CubicClamp => CubicResampler::mitchell().into(),
    }
}

fn tile_mode(mode: SpreadMode) -> TileMode {
    match mode {
        SpreadMode::Pad => TileMode::Clamp,
        SpreadMode::Repeat => TileMode::Repeat,
        SpreadMode::Reflect => TileMode::Mirror,
    }
}

fn sk_matrix(matrix: [f64; 9]) -> Matrix {
    Matrix::new_all(
        matrix[0] as f32,
        matrix[1] as f32,
        matrix[2] as f32,
        matrix[3] as f32,
        matrix[4] as f32,
        matrix[5] as f32,
        matrix[6] as f32,
        matrix[7] as f32,
        matrix[8] as f32,
    )
}

fn mul3(left: [f64; 9], right: [f64; 9]) -> [f64; 9] {
    let mut output = [0.0; 9];
    for row in 0..3 {
        for column in 0..3 {
            output[row * 3 + column] = (0..3)
                .map(|index| left[row * 3 + index] * right[index * 3 + column])
                .sum();
        }
    }
    output
}

fn inverse3(value: [f64; 9]) -> Option<[f64; 9]> {
    let [a, b, c, d, e, f, g, h, i] = value;
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

#[derive(Debug, Error)]
pub(crate) enum DrawError {
    #[error("DrawProgram decode failed: {0}")]
    ProgramDecode(String),
    #[error("DrawProgram {program} differs from its admitted plan metadata")]
    ProgramContractMismatch { program: u32 },
    #[error("missing DrawProgram texture {0}")]
    MissingTexture(String),
    #[error("missing font {hash} face {index}")]
    MissingFont { hash: String, index: u32 },
    #[error("font {hash} face {index} cannot be parsed by Skia")]
    InvalidFont { hash: String, index: u32 },
    #[error("missing runtime shader {0}")]
    MissingShader(String),
    #[error("runtime shader {0} is not UTF-8 SkSL")]
    InvalidShaderEncoding(String),
    #[error("runtime shader {uri} compilation failed: {message}")]
    ShaderCompile { uri: String, message: String },
    #[error("runtime shader {uri} does not match the Product Compositor ABI")]
    ShaderAbi { uri: String },
    #[error("missing Scene3D raster {0}")]
    MissingScene(String),
    #[error("missing {kind} structure {key}")]
    MissingStructure { kind: &'static str, key: String },
    #[error("duplicate program resource {0}")]
    DuplicateResource(String),
    #[error("surface operation failed: {0}")]
    Surface(String),
    #[error("unsupported DrawProgram operation: {0}")]
    Unsupported(String),
    #[error("missing outer resource {0}")]
    MissingOuterResource(u32),
    #[error("missing program destination {0}")]
    MissingDestination(u32),
    #[error("missing program node {0}")]
    MissingNode(u32),
    #[error("missing program path {0}")]
    MissingPath(u32),
    #[error("missing program paint {0}")]
    MissingPaint(u32),
    #[error("missing program resource {0}")]
    MissingProgramResource(u32),
    #[error("missing bound program surface slot {0}")]
    MissingProgramSurface(u32),
    #[error("DrawProgram backdrop filter has no admitted helper surface")]
    MissingProgramHelper,
    #[error("DrawProgram did not materialize its terminal output")]
    MissingProgramOutput,
    #[error("program resource {0} is written twice")]
    DuplicateProgramResource(u32),
    #[error("group node {0} reached a leaf raster pass")]
    UnexpectedGroupRaster(u32),
    #[error("glyph id {0} is outside Skia's glyph domain")]
    InvalidGlyph(u32),
    #[error("runtime shader child {child} is missing for {uri}")]
    ShaderChild { uri: String, child: String },
    #[error("runtime shader uniform {0} is missing or has the wrong type")]
    ShaderUniform(String),
    #[error("runtime shader uniform layout has missing or extra bindings")]
    ShaderUniformLayout,
    #[error("internal DrawProgram executor invariant failed: {0}")]
    Internal(String),
}

impl From<SurfaceError> for DrawError {
    fn from(value: SurfaceError) -> Self {
        Self::Surface(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f16_filters_use_owned_roi_in_an_oversized_scratch_surface() {
        let info = ImageInfo::new(
            (19, 13),
            ColorType::RGBAF16,
            AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let mut source = skia_safe::surfaces::raster(&info, None, None).unwrap();
        source.canvas().clear(Color4f::new(0.25, 0.1, 0.5, 0.8));
        let mut paint = SkPaint::default();
        paint.set_color4f(Color4f::new(0.9, 0.7, 0.2, 1.0), None);
        source
            .canvas()
            .draw_rect(SkRect::from_xywh(3.0, 4.0, 7.0, 5.0), &paint);
        let input = ProgramImage {
            image: source.image_snapshot(),
            roi: DeviceRect::new(-26, 34, 19, 13),
        };
        let roi = DeviceRect::new(-19, 23, 83, 61);
        let read_info = info.with_dimensions((83, 61));
        // If a padded pyramid processes this whole slot, it exceeds MAX_PIXELS.
        // The owned ROI must still succeed, independently of pool capacity.
        let mut large =
            skia_safe::surfaces::raster(&info.with_dimensions((4096, 4096)), None, None).unwrap();
        let origin = [-32, 16];
        let at = (roi.x - origin[0], roi.y - origin[1]);
        large.canvas().clip_rect(
            SkRect::from_xywh(
                at.0 as f32,
                at.1 as f32,
                roi.width as f32,
                roi.height as f32,
            ),
            None,
            false,
        );
        for filter in [
            Filter::Glow {
                color: valle_draw::program::AuthorColor {
                    red: 0.2,
                    green: 0.6,
                    blue: 0.8,
                    alpha: 0.8,
                },
                intensity: 0.75,
                radius: 8.25,
            },
            Filter::Bloom {
                threshold: 0.25,
                knee: 0.15,
                intensity: 0.75,
                radius: 8.25,
            },
            Filter::RadialBlur {
                center: [-15.0, 32.0],
                amount: 8.25,
            },
            Filter::FilmGrain {
                seed: 17,
                amount: 0.2,
                size: 2.5,
            },
            Filter::LensDistortion {
                k1: -0.275,
                k2: 0.125,
            },
        ] {
            let mut small = skia_safe::surfaces::raster(&read_info, None, None).unwrap();
            let frame = Some([-30.0, 20.0, 40.0, 35.0]);
            apply_f16_filter_program_into(&mut small, &input, [roi.x, roi.y], roi, &filter, frame)
                .unwrap();
            apply_f16_filter_program_into(&mut large, &input, origin, roi, &filter, frame).unwrap();
            let mut expected = vec![0_u8; 83 * 61 * 8];
            let mut actual = expected.clone();
            assert!(small.read_pixels(&read_info, &mut expected, 83 * 8, (0, 0)));
            assert!(large.read_pixels(&read_info, &mut actual, 83 * 8, at));
            assert!(
                expected.iter().any(|byte| *byte != 0),
                "empty output for {filter:?}"
            );
            assert_eq!(
                actual, expected,
                "slot capacity or origin changed {filter:?}"
            );
        }
    }

    #[test]
    fn instance_round_rect_affine_and_stroke_change_raster() {
        use valle_draw::program::{Affine2d, DrawProgramBuilder, InstanceColumns, LinearColor};

        let render = |skew: f64, stroke_width: f32| {
            let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 64.0, 64.0));
            let root = builder.push_node(Node::InstanceBatch(InstanceBatchNode {
                shape: InstanceShape::RoundRect(RoundRect::circular(
                    Rect::new(0.0, 0.0, 1.0, 1.0),
                    [0.2; 4],
                )),
                instances: InstanceColumns {
                    transforms: vec![Affine2d([24.0, skew, 0.0, 24.0, 14.0, 12.0])],
                    colors: vec![LinearColor::new(1.0, 0.0, 0.0, 1.0)],
                    stroke_colors: Vec::new(),
                    dash_offsets: Vec::new(),
                    opacities: vec![0.75],
                    stroke_widths: vec![stroke_width],
                },
                path_style: None,
            }));
            builder.add_root(root);
            let program = builder.finish().unwrap();
            let info = ImageInfo::new(
                (64, 64),
                skia_safe::ColorType::RGBA8888,
                skia_safe::AlphaType::Premul,
                Some(working_color_space().unwrap()),
            );
            let mut surface = skia_safe::surfaces::raster(&info, None, None).unwrap();
            surface.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
            draw_instance_batch(
                &program,
                &BTreeMap::new(),
                surface.canvas(),
                match &program.nodes()[0] {
                    Node::InstanceBatch(batch) => batch,
                    _ => unreachable!(),
                },
            )
            .unwrap();
            let mut pixels = vec![0; 64 * 64 * 4];
            assert!(surface.read_pixels(&info, &mut pixels, 64 * 4, (0, 0)));
            pixels
        };
        let axis_aligned = render(0.0, 0.0);
        let skewed = render(10.0, 0.0);
        let stroked = render(10.0, 0.2);
        assert_ne!(axis_aligned, skewed);
        assert_ne!(skewed, stroked);
    }

    #[test]
    fn glyph_outline_changes_render_without_font_resources() {
        use valle_draw::program::{DrawProgramBuilder, Glyph, LinearColor};
        use valle_draw::requirements::DigestBytes;

        let info = ImageInfo::new(
            (32, 32),
            skia_safe::ColorType::RGBAF32,
            skia_safe::AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let mut surface = skia_safe::surfaces::raster(&info, None, None).unwrap();
        let mut render = |width: f64| {
            let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 32.0, 32.0));
            let outline = builder.push_path(PathData {
                verbs: vec![
                    PathVerb::MoveTo,
                    PathVerb::LineTo,
                    PathVerb::LineTo,
                    PathVerb::LineTo,
                    PathVerb::Close,
                ],
                points: vec![
                    [4.0, 5.0],
                    [4.0 + width, 5.0],
                    [4.0 + width, 29.0],
                    [4.0, 29.0],
                ],
            });
            let paint = builder.push_paint(Paint::Solid(LinearColor::new(1.0, 1.0, 1.0, 1.0)));
            // Keep the font and glyph identity unchanged; resolved ink determines the pixels.
            let root = builder.push_node(Node::GlyphRun(GlyphRun {
                font: FontKey {
                    face_hash: DigestBytes::from_bytes([7; 32]),
                    face_index: 0,
                },
                font_size: 10.0,
                glyphs: vec![Glyph {
                    id: 1,
                    x: 20.0,
                    y: 30.0,
                }],
                outline: Some(outline),
                bounds: Rect::new(4.0, 5.0, width, 24.0),
                paint,
                stroke: None,
                source_node: None,
                source_ranges: vec![],
            }));
            builder.add_root(root);
            let runtime = ProgramRuntime {
                program: Arc::new(builder.finish().unwrap()),
                textures: BTreeMap::new(),
                fonts: BTreeMap::new(),
                shaders: BTreeMap::new(),
                scenes: BTreeMap::new(),
                glass: None,
                blend: None,
                mask_coverage: None,
            };
            surface.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
            for root in runtime.program.roots() {
                runtime.raster_node(surface.canvas(), *root).unwrap();
            }
            let mut bytes = vec![0u8; 32 * 32 * 16];
            assert!(surface.read_pixels(&info, &mut bytes, 32 * 16, (0, 0)));
            bytes
        };
        let ink = |pixels: &[u8]| {
            pixels
                .chunks_exact(16)
                .map(|pixel| f32::from_ne_bytes(pixel[12..16].try_into().unwrap()))
                .sum::<f32>()
        };
        let thin = render(4.0);
        let thick = render(12.0);
        assert_eq!(ink(&thin), 4.0 * 24.0);
        assert_eq!(ink(&thick), 12.0 * 24.0);
        assert_eq!(render(4.0), thin);
    }

    #[test]
    fn emitted_rounded_border_keeps_its_ink() {
        let artifact = valle_compiler::motion::compile_motion(r##"export default function Border(){return <Scene style={{width:1920,height:1080,backgroundColor:"#000000"}}><View style={{position:"absolute",left:64,top:64,width:1792,height:952,borderRadius:24,borderStyle:"solid",borderWidth:2,borderColor:"#ffffff33",backgroundColor:"transparent"}} /></Scene>;}"##).unwrap().artifact;
        let prepared = valle_motion::prepare_scene(&artifact).unwrap();
        let props = valle_motion::resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
        let ctx =
            valle_motion::motion_context_at_frame(0, 30, serde_json::from_str("\"30/1\"").unwrap())
                .unwrap();
        let fonts = valle_motion::Fonts::default();
        let tree = valle_motion::build_tree(
            &prepared,
            &ctx,
            &props,
            &valle_motion::LayoutOptions {
                viewport: valle_motion::Viewport::new((1920, 1080)),
                fonts: &fonts,
                styles: None,
            },
        )
        .unwrap();
        let report = valle_motion::emit(&tree, &valle_motion::default_font_naming).unwrap();
        let runtime = ProgramRuntime {
            program: Arc::new(report.program),
            textures: BTreeMap::new(),
            fonts: BTreeMap::new(),
            shaders: BTreeMap::new(),
            scenes: BTreeMap::new(),
            glass: None,
            blend: None,
            mask_coverage: None,
        };
        let info = ImageInfo::new(
            (1920, 1080),
            skia_safe::ColorType::RGBAF32,
            skia_safe::AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let mut surface = skia_safe::surfaces::raster(&info, None, None).unwrap();
        for root in runtime.program.roots() {
            runtime.raster_node(surface.canvas(), *root).unwrap();
        }
        let mut bytes = vec![0u8; 1920 * 1080 * 16];
        assert!(surface.read_pixels(&info, &mut bytes, 1920 * 16, (0, 0)));
        let red = |x: usize, y: usize| {
            f32::from_ne_bytes(
                bytes[(y * 1920 + x) * 16..(y * 1920 + x) * 16 + 4]
                    .try_into()
                    .unwrap(),
            )
        };
        for (x, y) in [(65, 540), (960, 65), (1854, 540), (960, 1014)] {
            assert!(
                (red(x, y) - 0.2).abs() < 0.001,
                "border ({x},{y}) red={}",
                red(x, y)
            );
        }
        assert_eq!(red(960, 540), 0.0);
    }

    #[test]
    fn direct_tree_keeps_nested_transforms_and_translucent_painter_order() {
        use valle_draw::program::{
            Affine2d, DrawProgramBuilder, Group, LinearColor, PathNode, Transform2d,
        };
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 32.0, 32.0));
        let mut roots = Vec::new();
        for (offset, color) in [
            (0.0, LinearColor::new(1.0, 0.0, 0.0, 1.0)),
            (4.0, LinearColor::new(0.0, 0.0, 0.5, 0.5)),
        ] {
            let paint = builder.push_paint(Paint::Solid(color));
            let path = builder.push_path(PathData {
                verbs: vec![
                    PathVerb::MoveTo,
                    PathVerb::LineTo,
                    PathVerb::LineTo,
                    PathVerb::LineTo,
                    PathVerb::Close,
                ],
                points: vec![[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]],
            });
            let child = builder.push_node(Node::Path(PathNode {
                path,
                fill_rule: FillRule::NonZero,
                fill: Some(paint),
                stroke: None,
            }));
            let mut group = Group::plain(vec![child]);
            group.transform = Transform2d::from_affine(Affine2d::translate(offset, 0.0));
            roots.push(builder.push_node(Node::Group(group)));
        }
        let mut group = Group::plain(roots);
        group.transform =
            Transform2d::from_affine(Affine2d::scale(2.0, 2.0).then(Affine2d::translate(3.0, 5.0)));
        let root = builder.push_node(Node::Group(group));
        builder.add_root(root);
        let runtime = ProgramRuntime {
            program: Arc::new(builder.finish().unwrap()),
            textures: BTreeMap::new(),
            fonts: BTreeMap::new(),
            shaders: BTreeMap::new(),
            scenes: BTreeMap::new(),
            glass: None,
            blend: None,
            mask_coverage: None,
        };
        let info = ImageInfo::new(
            (32, 32),
            skia_safe::ColorType::RGBAF32,
            skia_safe::AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let mut actual = skia_safe::surfaces::raster(&info, None, None).unwrap();
        actual.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
        for root in runtime.program.roots() {
            runtime.raster_node(actual.canvas(), *root).unwrap();
        }
        let mut bytes = vec![0_u8; 32 * 32 * 16];
        assert!(actual.read_pixels(&info, &mut bytes, 32 * 16, (0, 0)));
        let pixels = bytes
            .chunks_exact(4)
            .map(|bytes| f32::from_ne_bytes(bytes.try_into().unwrap()))
            .collect::<Vec<_>>();
        for (x, y, expected) in [
            (4, 6, [1.0, 0.0, 0.0, 1.0]),
            (12, 6, [0.5, 0.0, 0.5, 1.0]),
            (22, 6, [0.0, 0.0, 0.5, 0.5]),
            (2, 6, [0.0; 4]),
            (4, 22, [0.0; 4]),
        ] {
            let start = (y * 32 + x) * 4;
            for (actual, expected) in pixels[start..start + 4].iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 0.001,
                    "pixel ({x}, {y}) = {:?}",
                    &pixels[start..start + 4]
                );
            }
        }
    }

    #[test]
    fn direct_tree_isolates_nested_group_opacity() {
        use valle_draw::program::{
            Affine2d, DrawProgramBuilder, Group, LinearColor, PathNode, Transform2d,
        };
        let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 32.0, 32.0));
        let mut roots = Vec::new();
        for (offset, color) in [
            (0.0, LinearColor::new(1.0, 0.0, 0.0, 1.0)),
            (4.0, LinearColor::new(0.0, 0.0, 0.5, 0.5)),
        ] {
            let paint = builder.push_paint(Paint::Solid(color));
            let path = builder.push_path(PathData {
                verbs: vec![
                    PathVerb::MoveTo,
                    PathVerb::LineTo,
                    PathVerb::LineTo,
                    PathVerb::LineTo,
                    PathVerb::Close,
                ],
                points: vec![[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]],
            });
            let child = builder.push_node(Node::Path(PathNode {
                path,
                fill_rule: FillRule::NonZero,
                fill: Some(paint),
                stroke: None,
            }));
            let mut group = Group::plain(vec![child]);
            group.opacity = 0.5;
            group.transform = Transform2d::from_affine(Affine2d::translate(offset, 0.0));
            roots.push(builder.push_node(Node::Group(group)));
        }
        let mut group = Group::plain(roots);
        group.opacity = 0.5;
        group.transform =
            Transform2d::from_affine(Affine2d::scale(2.0, 2.0).then(Affine2d::translate(3.0, 5.0)));
        let root = builder.push_node(Node::Group(group));
        builder.add_root(root);
        let runtime = ProgramRuntime {
            program: Arc::new(builder.finish().unwrap()),
            textures: BTreeMap::new(),
            fonts: BTreeMap::new(),
            shaders: BTreeMap::new(),
            scenes: BTreeMap::new(),
            glass: None,
            blend: None,
            mask_coverage: None,
        };
        let info = ImageInfo::new(
            (32, 32),
            skia_safe::ColorType::RGBAF32,
            skia_safe::AlphaType::Premul,
            Some(working_color_space().unwrap()),
        );
        let mut actual = skia_safe::surfaces::raster(&info, None, None).unwrap();
        actual.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
        for root in runtime.program.roots() {
            runtime.raster_node(actual.canvas(), *root).unwrap();
        }
        let mut bytes = vec![0_u8; 32 * 32 * 16];
        assert!(actual.read_pixels(&info, &mut bytes, 32 * 16, (0, 0)));
        let pixels = bytes
            .chunks_exact(4)
            .map(|bytes| f32::from_ne_bytes(bytes.try_into().unwrap()))
            .collect::<Vec<_>>();
        for (x, y, expected) in [
            (4, 6, [0.25, 0.0, 0.0, 0.25]),
            (12, 6, [0.1875, 0.0, 0.125, 0.3125]),
            (22, 6, [0.0, 0.0, 0.125, 0.125]),
            (2, 6, [0.0; 4]),
            (4, 22, [0.0; 4]),
        ] {
            let start = (y * 32 + x) * 4;
            for (actual, expected) in pixels[start..start + 4].iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 0.001,
                    "pixel ({x}, {y}) = {:?}",
                    &pixels[start..start + 4]
                );
            }
        }
    }

    #[test]
    fn group_pixel_operations_keep_the_scheduled_path() {
        use valle_draw::program::Group;
        let plain = Group::plain(Vec::new());
        assert!(plain.is_transform_only());
        let mut opacity = plain.clone();
        opacity.opacity = 0.5;
        let mut clip = plain.clone();
        clip.clip = Some(Clip::Rect(Rect::new(0.0, 0.0, 8.0, 8.0)));
        let mut filter = plain.clone();
        filter.filters.push(Filter::Blur {
            sigma_x: 1.0,
            sigma_y: 1.0,
        });
        let mut blend = plain.clone();
        blend.internal_blend = BlendMode::Multiply;
        for group in [opacity, clip, filter, blend] {
            assert!(!group.is_transform_only());
        }
    }

    #[test]
    fn raster_tree_clip_matches_clipping_the_completed_overlap() {
        use valle_draw::program::{DrawProgramBuilder, Group, LinearColor, PathNode};
        let viewport = Rect::new(0.0, 0.0, 32.0, 32.0);
        for clip in [
            Clip::Rect(Rect::new(7.3, 3.7, 17.2, 22.6)),
            Clip::RoundRect(RoundRect {
                rect: Rect::new(7.3, 3.7, 17.2, 22.6),
                radii: [[6.5, 6.5]; 4],
            }),
        ] {
            for opacity in [1.0, 0.63] {
                let mut builder = DrawProgramBuilder::new(viewport);
                let mut children = Vec::new();
                for (x, color) in [
                    (0.0, LinearColor::new(0.8, 0.0, 0.0, 0.8)),
                    (8.0, LinearColor::new(0.0, 0.0, 0.6, 0.6)),
                ] {
                    let path = builder.push_path(PathData {
                        verbs: vec![
                            PathVerb::MoveTo,
                            PathVerb::LineTo,
                            PathVerb::LineTo,
                            PathVerb::LineTo,
                            PathVerb::Close,
                        ],
                        points: vec![[x, 0.0], [x + 24.0, 0.0], [x + 24.0, 32.0], [x, 32.0]],
                    });
                    let paint = builder.push_paint(Paint::Solid(color));
                    children.push(builder.push_node(Node::Path(PathNode {
                        path,
                        fill_rule: FillRule::NonZero,
                        fill: Some(paint),
                        stroke: None,
                    })));
                }
                let mut group = Group::plain(children.clone());
                group.clip = Some(clip.clone());
                group.opacity = opacity;
                let root = builder.push_node(Node::Group(group));
                builder.add_root(root);
                let runtime = ProgramRuntime {
                    program: Arc::new(builder.finish().unwrap()),
                    textures: BTreeMap::new(),
                    fonts: BTreeMap::new(),
                    shaders: BTreeMap::new(),
                    scenes: BTreeMap::new(),
                    glass: None,
                    blend: None,
                    mask_coverage: None,
                };
                let root = runtime.program.roots()[0];
                let Node::Group(group) = &runtime.program.nodes()[root.raw() as usize] else {
                    unreachable!()
                };
                let children = group.children.clone();
                for color_type in [skia_safe::ColorType::RGBAF16, skia_safe::ColorType::RGBAF32] {
                    let info = ImageInfo::new(
                        (32, 32),
                        color_type,
                        skia_safe::AlphaType::Premul,
                        Some(working_color_space().unwrap()),
                    );
                    let surface = || skia_safe::surfaces::raster(&info, None, None).unwrap();
                    let mut actual = surface();
                    clear_surface(&mut actual).unwrap();
                    runtime.raster_node(actual.canvas(), root).unwrap();
                    let mut source = surface();
                    clear_surface(&mut source).unwrap();
                    for child in &children {
                        runtime.raster_node(source.canvas(), *child).unwrap();
                    }
                    let roi = DeviceRect::new(0, 0, 32, 32);
                    let input = ProgramImage {
                        image: source.image_snapshot(),
                        roi,
                    };
                    let mut clipped = surface();
                    clip_image_into(
                        &mut clipped,
                        Some(&input),
                        &clip,
                        Matrix::new_identity(),
                        runtime.program.paths(),
                        [0, 0],
                    )
                    .unwrap();
                    let mut expected = surface();
                    clear_surface(&mut expected).unwrap();
                    draw_program_image_with_mode(
                        &mut expected,
                        &ProgramImage {
                            image: clipped.image_snapshot(),
                            roi,
                        },
                        [0, 0],
                        SkBlendMode::SrcOver,
                        opacity,
                    );
                    let read = info.with_color_type(skia_safe::ColorType::RGBAF32);
                    let mut a = vec![0_f32; 32 * 32 * 4];
                    let mut b = a.clone();
                    assert!(actual.image_snapshot().read_pixels(
                        &read,
                        &mut a,
                        32 * 16,
                        (0, 0),
                        skia_safe::image::CachingHint::Disallow
                    ));
                    assert!(expected.image_snapshot().read_pixels(
                        &read,
                        &mut b,
                        32 * 16,
                        (0, 0),
                        skia_safe::image::CachingHint::Disallow
                    ));
                    let maximum = a
                        .iter()
                        .zip(&b)
                        .map(|(a, b)| (a - b).abs())
                        .fold(0_f32, f32::max);
                    assert!(
                        maximum <= 0.001,
                        "clip {clip:?}, opacity {opacity}, {color_type:?}: {maximum}"
                    );
                }
            }
        }
    }

    #[test]
    fn path_clip_clears_pixels_outside_the_outline() {
        let mut source = skia_safe::surfaces::raster_n32_premul((8, 8)).unwrap();
        source.canvas().clear(skia_safe::Color::RED);
        let input = ProgramImage {
            image: source.image_snapshot(),
            roi: DeviceRect {
                x: 0,
                y: 0,
                width: 8,
                height: 8,
            },
        };
        let mut output = skia_safe::surfaces::raster_n32_premul((8, 8)).unwrap();
        let path = PathData {
            verbs: vec![
                PathVerb::MoveTo,
                PathVerb::LineTo,
                PathVerb::LineTo,
                PathVerb::Close,
            ],
            points: vec![[0.0, 0.0], [8.0, 0.0], [0.0, 8.0]],
        };
        clip_image_into(
            &mut output,
            Some(&input),
            &Clip::Path {
                path: valle_draw::program::PathId::from_raw(0),
                fill_rule: FillRule::NonZero,
            },
            Matrix::new_identity(),
            &[path],
            [0, 0],
        )
        .unwrap();
        let info = skia_safe::ImageInfo::new(
            (8, 8),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Unpremul,
            None,
        );
        let mut pixels = vec![0u8; 8 * 8 * 4];
        assert!(output.read_pixels(&info, &mut pixels, 8 * 4, (0, 0)));
        assert_eq!(pixels[(8 + 1) * 4 + 3], 255);
        assert_eq!(pixels[(6 * 8 + 6) * 4 + 3], 0);
    }

    #[test]
    fn image_copies_exclude_unused_surface_capacity() {
        let mut source = skia_safe::surfaces::raster_n32_premul((64, 64)).unwrap();
        source.canvas().clear(skia_safe::Color::GREEN);
        let mut paint = SkPaint::default();
        paint.set_color(skia_safe::Color::RED);
        source
            .canvas()
            .draw_rect(SkRect::from_xywh(0.0, 0.0, 20.0, 16.0), &paint);
        let input = ProgramImage {
            image: source.image_snapshot(),
            roi: DeviceRect::new(7, 9, 20, 16),
        };
        let mut output = skia_safe::surfaces::raster_n32_premul((64, 64)).unwrap();
        copy_optional_into(&mut output, Some(&input), [0, 0]).unwrap();
        let info = ImageInfo::new(
            (64, 64),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
        let mut pixels = vec![0_u8; 64 * 64 * 4];
        assert!(output.read_pixels(&info, &mut pixels, 64 * 4, (0, 0)));
        for y in 0..64 {
            for x in 0..64 {
                let expected = if (7..27).contains(&x) && (9..25).contains(&y) {
                    [255, 0, 0, 255]
                } else {
                    [0; 4]
                };
                assert_eq!(&pixels[(y * 64 + x) * 4..(y * 64 + x + 1) * 4], &expected);
            }
        }
    }

    #[test]
    fn native_clip_preserves_transformed_group_coverage() {
        use skia_safe::{AlphaType, ColorType};
        let paths = [PathData {
            verbs: vec![
                PathVerb::MoveTo,
                PathVerb::LineTo,
                PathVerb::LineTo,
                PathVerb::Close,
            ],
            points: vec![[4.0, 3.0], [22.0, 5.0], [7.0, 23.0]],
        }];
        let clips = [
            Clip::Rect(Rect::new(4.0, 3.0, 18.0, 20.0)),
            Clip::RoundRect(RoundRect {
                rect: Rect::new(4.0, 3.0, 18.0, 20.0),
                radii: [[5.0, 5.0]; 4],
            }),
            Clip::Path {
                path: valle_draw::program::PathId::from_raw(0),
                fill_rule: FillRule::NonZero,
            },
        ];
        for color in [ColorType::RGBAF16, ColorType::RGBAF32] {
            let info = ImageInfo::new(
                (48, 48),
                color,
                AlphaType::Premul,
                Some(working_color_space().unwrap()),
            );
            let mut source = skia_safe::surfaces::raster(&info, None, None).unwrap();
            source.canvas().clear(Color4f::new(1.4, -0.1, 0.6, 0.65));
            let input = ProgramImage {
                image: source.image_snapshot(),
                roi: DeviceRect {
                    x: -2,
                    y: 3,
                    width: 48,
                    height: 48,
                },
            };
            for clip in &clips {
                for matrix in [
                    Matrix::new_identity(),
                    Matrix::new_all(0.94, -0.34, 9.3, 0.34, 0.94, 2.7, 0.0, 0.0, 1.0),
                    Matrix::new_all(1.0, 0.2, 1.0, 0.1, 1.0, 2.0, 0.001, 0.002, 1.0),
                ] {
                    let origin = [-2, 3];
                    let mut actual = skia_safe::surfaces::raster(&info, None, None).unwrap();
                    actual.canvas().clear(skia_safe::Color::GREEN);
                    clip_image_into(&mut actual, Some(&input), clip, matrix, &paths, origin)
                        .unwrap();
                    let mut expected = skia_safe::surfaces::raster(&info, None, None).unwrap();
                    expected.canvas().clear(skia_safe::Color::TRANSPARENT);
                    let mut paint = SkPaint::default();
                    paint
                        .set_anti_alias(true)
                        .set_blend_mode(SkBlendMode::Src)
                        .set_color(skia_safe::Color::WHITE);
                    let canvas = expected.canvas();
                    canvas.save();
                    canvas
                        .translate((-origin[0] as f32, -origin[1] as f32))
                        .concat(&matrix);
                    match clip {
                        Clip::Rect(rect) => {
                            canvas.draw_rect(sk_rect(*rect), &paint);
                        }
                        Clip::RoundRect(rect) => {
                            canvas.draw_rrect(sk_rrect(*rect), &paint);
                        }
                        Clip::Path {
                            path: id,
                            fill_rule,
                        } => {
                            canvas.draw_path(
                                &path(&paths[id.raw() as usize], *fill_rule).unwrap(),
                                &paint,
                            );
                        }
                    }
                    canvas.restore();
                    draw_program_image_with_mode(
                        &mut expected,
                        &input,
                        origin,
                        SkBlendMode::SrcIn,
                        1.0,
                    );
                    let read_info = info.with_color_type(ColorType::RGBAF32);
                    let mut left = vec![0.0_f32; 48 * 48 * 4];
                    let mut right = left.clone();
                    for (surface, pixels) in [(&mut actual, &mut left), (&mut expected, &mut right)]
                    {
                        assert!(surface.image_snapshot().read_pixels(
                            &read_info,
                            pixels,
                            48 * 16,
                            (0, 0),
                            skia_safe::image::CachingHint::Disallow
                        ));
                    }
                    let error = left
                        .iter()
                        .zip(&right)
                        .map(|(a, b)| (a - b).abs())
                        .fold(0.0_f32, f32::max);
                    assert!(
                        error < 0.008,
                        "clip={clip:?} matrix={matrix:?} color={color:?} error={error}"
                    );
                }
            }
        }
    }

    #[test]
    fn program_filter_maps_similarity_into_device_space() {
        let color = valle_draw::program::LinearColor::new(0.0, 0.0, 0.0, 1.0);
        let mapped = program_filter_in_device_space(
            &Filter::DropShadow {
                offset: [2.0, 3.0],
                sigma_x: 5.0,
                sigma_y: 5.0,
                color,
            },
            [0.0, -2.0, 8.0, 2.0, 0.0, 9.0, 0.0, 0.0, 1.0],
        )
        .unwrap();
        assert_eq!(
            mapped,
            Filter::DropShadow {
                offset: [-6.0, 4.0],
                sigma_x: 10.0,
                sigma_y: 10.0,
                color,
            }
        );

        assert_eq!(
            program_filter_in_device_space(
                &Filter::Blur {
                    sigma_x: 3.0,
                    sigma_y: 3.0,
                },
                [-2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0],
            )
            .unwrap(),
            Filter::Blur {
                sigma_x: 6.0,
                sigma_y: 6.0,
            }
        );
    }

    #[test]
    fn program_filter_accepts_exact_axis_aligned_nonuniform_mapping() {
        assert_eq!(
            program_filter_in_device_space(
                &Filter::Blur {
                    sigma_x: 5.0,
                    sigma_y: 5.0,
                },
                [2.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 1.0],
            )
            .unwrap(),
            Filter::Blur {
                sigma_x: 10.0,
                sigma_y: 15.0,
            }
        );
    }

    #[test]
    fn program_filter_rejects_unrepresentable_and_projective_transforms() {
        let blur = Filter::Blur {
            sigma_x: 5.0,
            sigma_y: 5.0,
        };
        for matrix in [
            [1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.01, 0.0, 1.0],
            [1.0, 0.0, 1.0e12, 0.0, 1.0, 1.0e12, 1.0e-6, 0.0, 1.0],
            [f64::MAX, 0.0, 0.0, 0.0, f64::MAX, 0.0, 0.0, 0.0, 1.0],
        ] {
            assert!(matches!(
                program_filter_in_device_space(&blur, matrix),
                Err(DrawError::Unsupported(_))
            ));
        }
    }
}
