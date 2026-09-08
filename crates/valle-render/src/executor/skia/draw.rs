use std::{collections::BTreeMap, sync::Arc};

use skia_safe::{
    BlendMode as SkBlendMode, ClipOp, Color4f, CubicResampler, Data, FilterMode, Font, IRect,
    Image, ImageInfo, Matrix, MipmapMode, Paint as SkPaint, PaintCap, PaintJoin, PaintStyle,
    Path as SkPath, PathBuilder, PathEffect, PathFillType, Point as SkPoint, RRect, Rect as SkRect,
    RuntimeEffect, SamplingOptions, Shader, Surface, TextBlobBuilder, TileMode, Typeface,
    canvas::PointMode, color_filters, gradient, image_filters, runtime_effect::ChildPtr,
};
use thiserror::Error;
use valle_draw::{
    Rect,
    program::{
        BatchGeometry, BlendMode, Clip, DrawProgram, FillRule, Filter, GlyphRun, MaskMode, Node,
        Paint, PaintId, PathData, PathStroke, PathVerb, RoundRect, ShaderLayer,
        ShaderTextureBinding, ShaderUniformBinding, ShaderUniformValue, SpreadMode, StrokeCap,
        StrokeJoin,
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
    effect::{image_filter, straight_color},
    glass::{
        admit_motion_glass_shader, render_motion_glass_foreground_into, render_motion_glass_into,
    },
    surface::{PlanImage, ScratchSurfaces, SurfaceError, working_color_space},
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
}

#[derive(Debug, Clone)]
pub(crate) struct ProgramTexture {
    image: Image,
    interpretation: VisualInterpretation,
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
            let ResourceInterpretation::Visual { interpretation } = object.key().interpretation
            else {
                return Err(DrawError::MissingTexture(requirement.key.clone()));
            };
            let image = object
                .visual_image()
                .ok_or_else(|| DrawError::MissingTexture(requirement.key.clone()))?
                .clone();
            if textures
                .insert(
                    requirement.clone(),
                    ProgramTexture {
                        image,
                        interpretation,
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

        Ok(Self {
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
        let output_roi = schedule
            .resources()
            .iter()
            .find(|resource| resource.resource() == plan.local_plan().output())
            .map(|resource| resource.device_roi())
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
        let resource_rois = schedule
            .resources()
            .iter()
            .map(|resource| (resource.resource(), resource.device_roi()))
            .collect::<BTreeMap<_, _>>();
        let mut last_uses = BTreeMap::new();
        for pass in plan.local_plan().passes() {
            last_uses.insert(pass.kind.output(), pass.id);
            for input in pass.kind.reads() {
                last_uses.insert(input, pass.id);
            }
            if let Some(ProgramStorageKind::Alias { source }) =
                plan.local_schedule().storage(pass.kind.output())
            {
                last_uses
                    .entry(source)
                    .and_modify(|last| *last = (*last).max(pass.id))
                    .or_insert(pass.id);
            }
        }
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

        for pass in plan.local_plan().passes() {
            let output = pass.kind.output();
            let storage = plan
                .local_schedule()
                .storage(output)
                .ok_or(DrawError::MissingProgramResource(output.get()))?;
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
                    let roi = *resource_rois
                        .get(&output)
                        .ok_or(DrawError::MissingProgramResource(output.get()))?;
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
                    let roi = *resource_rois
                        .get(&output)
                        .ok_or(DrawError::MissingProgramResource(output.get()))?;
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
            for dead in last_uses
                .iter()
                .filter_map(|(resource, last)| (*last == pass.id).then_some(*resource))
                .filter(|resource| *resource != plan.local_plan().output())
                .collect::<Vec<_>>()
            {
                resources.remove(&dead);
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

        let target = program_target_mut(surfaces, target_location)?;
        self.render_program_pass_into(
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
        )?;
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
                if let Some(image) = resource(resources, *input)? {
                    let local_to_device = self.device_matrix_values(plan, output, transform)?;
                    apply_filter_program_into(target, image, filter, local_to_device, target_origin)
                } else {
                    clear_surface(target)
                }
            }
            ProgramPassKind::ApplyMask {
                input, mask, mode, ..
            } => apply_mask_into(
                target,
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
                ..
            } => self.blend_source_into(
                target,
                resource(resources, *source)?,
                resource(resources, *destination)?,
                *mode,
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
        self.blend
            .as_ref()
            .ok_or_else(|| DrawError::Internal("creative blend kernel was not admitted".into()))?
            .effective_source_into(target, &source.image, &destination.image, mode)
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
                let path = path(
                    self.program
                        .paths()
                        .get(node.path.raw() as usize)
                        .ok_or(DrawError::MissingPath(node.path.raw()))?,
                    node.fill_rule,
                )?;
                if let Some(fill) = node.fill {
                    let mut paint = self.paint(fill)?;
                    paint.set_style(PaintStyle::Fill);
                    canvas.draw_path(&path, &paint);
                }
                if let Some(stroke) = &node.stroke {
                    let paint = self.stroke(stroke)?;
                    canvas.draw_path(&path, &paint);
                }
            }
            Node::GeometryBatch(batch) => {
                let mut paint = SkPaint::default();
                paint.set_anti_alias(true);
                let space =
                    working_color_space().map_err(|error| DrawError::Surface(error.to_string()))?;
                if batch.geometry == BatchGeometry::Circle
                    && let Some(first) = batch.instances.first()
                    && batch.instances.iter().all(|instance| {
                        instance.size == first.size && instance.color == first.color
                    })
                {
                    let points = batch
                        .instances
                        .iter()
                        .map(|instance| {
                            SkPoint::new(instance.position[0] as f32, instance.position[1] as f32)
                        })
                        .collect::<Vec<_>>();
                    paint.set_color4f(straight_color(first.color), &space);
                    paint.set_stroke_cap(PaintCap::Round);
                    paint.set_stroke_width(first.size[0].min(first.size[1]) as f32);
                    canvas.draw_points(PointMode::Points, &points, &paint);
                    return Ok(());
                }
                for instance in &batch.instances {
                    paint.set_color4f(straight_color(instance.color), &space);
                    match batch.geometry {
                        BatchGeometry::Circle => canvas.draw_circle(
                            SkPoint::new(instance.position[0] as f32, instance.position[1] as f32),
                            (instance.size[0].min(instance.size[1]) * 0.5) as f32,
                            &paint,
                        ),
                        BatchGeometry::Rect => canvas.draw_rect(
                            SkRect::from_xywh(
                                instance.position[0] as f32,
                                instance.position[1] as f32,
                                instance.size[0] as f32,
                                instance.size[1] as f32,
                            ),
                            &paint,
                        ),
                    };
                }
            }
            Node::Image(node) => {
                let texture = self
                    .textures
                    .get(&node.texture)
                    .ok_or_else(|| DrawError::MissingTexture(node.texture.key.clone()))?;
                draw_program_texture(canvas, texture, node)?;
            }
            Node::GlyphRun(run) => self.draw_glyph_run(canvas, run)?,
            Node::Shadow(shadow) => draw_shadow(canvas, shadow),
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
                let space =
                    working_color_space().map_err(|error| DrawError::Surface(error.to_string()))?;
                result.set_color4f(straight_color(*color), &space);
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

    fn apply_shader_into(
        &self,
        surface: &mut Surface,
        input: Option<&ProgramImage>,
        shader: &ShaderLayer,
        matrix: Matrix,
        _info: &ImageInfo,
        target_origin: [i32; 2],
    ) -> Result<(), DrawError> {
        let Some(input) = input else {
            return clear_surface(surface);
        };
        let canvas = surface.canvas();
        canvas.clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
        canvas.save();
        canvas.translate((-target_origin[0] as f32, -target_origin[1] as f32));
        canvas.concat(&matrix);
        if let Some(runtime) = self.runtime_shader(
            Some(&input.image),
            &shader.shader.uri,
            shader.bounds,
            &shader.uniforms,
            &shader.textures,
        )? {
            let mut paint = SkPaint::default();
            paint.set_shader(runtime);
            canvas.draw_rect(sk_rect(shader.bounds), &paint);
        }
        canvas.restore();
        Ok(())
    }

    fn runtime_shader(
        &self,
        content: Option<&Image>,
        uri: &str,
        bounds: Rect,
        uniforms: &[ShaderUniformBinding],
        textures: &[ShaderTextureBinding],
    ) -> Result<Option<Shader>, DrawError> {
        let effect = self
            .shaders
            .get(uri)
            .ok_or_else(|| DrawError::MissingShader(uri.to_owned()))?;
        let data = shader_uniforms(effect, bounds, uniforms)?;
        let mut children = Vec::with_capacity(effect.children().len());
        for child in effect.children() {
            if child.name() == "content" && content.is_none() {
                children.push(ChildPtr::Shader(skia_safe::shaders::empty()));
                continue;
            }
            let image = if child.name() == "content" {
                content.expect("content presence was handled above")
            } else {
                let texture = textures
                    .iter()
                    .find(|texture| texture.name == child.name())
                    .ok_or_else(|| DrawError::ShaderChild {
                        uri: uri.to_owned(),
                        child: child.name().to_owned(),
                    })?;
                &self
                    .textures
                    .get(&texture.texture)
                    .ok_or_else(|| DrawError::MissingTexture(texture.texture.key.clone()))?
                    .image
            };
            let mode = textures
                .iter()
                .find(|texture| texture.name == child.name())
                .map_or(
                    SamplingOptions::new(FilterMode::Linear, MipmapMode::None),
                    |texture| sampling(texture.sampling),
                );
            let shader = image
                .to_shader((TileMode::Clamp, TileMode::Clamp), mode, None)
                .ok_or_else(|| DrawError::ShaderChild {
                    uri: uri.to_owned(),
                    child: child.name().to_owned(),
                })?;
            children.push(ChildPtr::Shader(shader));
        }
        let local = Matrix::translate((-bounds.x as f32, -bounds.y as f32));
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
) -> Result<(), DrawError> {
    let ExternalSample::Texture {
        texture_from_content,
        input_sample_bounds,
        ..
    } = ExternalSample::from_display_rect(node.src, texture.interpretation)
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
    paint.set_alpha_f(node.opacity);
    canvas.draw_image_rect_with_sampling_options(
        &texture.image,
        Some((&src, skia_safe::canvas::SrcRectConstraint::Strict)),
        src,
        sampling(node.sampling),
        &paint,
    );
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
    let _ = shader_uniforms(effect, bounds, uniforms)?;
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
        if !textures.contains_key(&binding.texture) {
            return Err(DrawError::MissingTexture(binding.texture.key.clone()));
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

fn copy_optional_into(
    surface: &mut Surface,
    image: Option<&ProgramImage>,
    target_origin: [i32; 2],
) -> Result<(), DrawError> {
    clear_surface(surface)?;
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
    clear_surface(surface)?;
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
    clear_surface(surface)?;
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
    let mut paint = SkPaint::default();
    paint.set_anti_alias(true);
    paint.set_blend_mode(SkBlendMode::Src);
    paint.set_color4f(Color4f::new(1.0, 1.0, 1.0, 1.0), None);
    let canvas = surface.canvas();
    canvas.save();
    canvas.translate((-target_origin[0] as f32, -target_origin[1] as f32));
    canvas.concat(&matrix);
    draw_clip_shape(canvas, clip, paths, &paint)?;
    canvas.restore();
    draw_program_image_with_mode(surface, input, target_origin, SkBlendMode::SrcIn, 1.0);
    Ok(())
}

fn apply_mask_into(
    surface: &mut Surface,
    input: Option<&ProgramImage>,
    mask: Option<&ProgramImage>,
    mode: MaskMode,
    target_origin: [i32; 2],
) -> Result<(), DrawError> {
    clear_surface(surface)?;
    let (Some(input), Some(mask)) = (input, mask) else {
        return Ok(());
    };
    draw_program_image_with_mode(surface, input, target_origin, SkBlendMode::Src, 1.0);
    let mut paint = SkPaint::default();
    paint.set_blend_mode(SkBlendMode::DstIn);
    if mode == MaskMode::Luminance {
        paint.set_color_filter(color_filters::matrix_row_major(
            &[
                0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.2627,
                0.6780, 0.0593, 0.0, 0.0,
            ],
            None,
        ));
    }
    draw_program_image_with_paint(surface, mask, target_origin, &paint);
    Ok(())
}

fn apply_filter_program_into(
    surface: &mut Surface,
    input: &ProgramImage,
    filter: &Filter,
    local_to_device: [f64; 9],
    target_origin: [i32; 2],
) -> Result<(), DrawError> {
    clear_surface(surface)?;
    let mut paint = SkPaint::default();
    paint.set_blend_mode(SkBlendMode::Src);
    let filter = program_filter_in_device_space(filter, local_to_device)?;
    if let Some(filter) = image_filter(&filter)? {
        paint.set_image_filter(filter);
    }
    draw_program_image_with_paint(surface, input, target_origin, &paint);
    Ok(())
}

fn apply_filter_chain_program_into(
    surface: &mut Surface,
    input: &ProgramImage,
    filters: &[Filter],
    target_origin: [i32; 2],
) -> Result<(), DrawError> {
    clear_surface(surface)?;
    let mut chain = None;
    for filter in filters {
        let Some(filter) = image_filter(filter)? else {
            continue;
        };
        chain = Some(match chain {
            Some(inner) => image_filters::compose(filter, inner)
                .ok_or_else(|| DrawError::Unsupported("composed image filter".into()))?,
            None => filter,
        });
    }
    let mut paint = SkPaint::default();
    paint.set_blend_mode(SkBlendMode::Src);
    if let Some(filter) = chain {
        paint.set_image_filter(filter);
    }
    draw_program_image_with_paint(surface, input, target_origin, &paint);
    Ok(())
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

fn draw_clip_shape(
    canvas: &skia_safe::Canvas,
    clip: &Clip,
    paths: &[PathData],
    paint: &SkPaint,
) -> Result<(), DrawError> {
    match clip {
        Clip::Rect(rect) => {
            canvas.draw_rect(sk_rect(*rect), paint);
        }
        Clip::RoundRect(rect) => {
            canvas.draw_rrect(sk_rrect(*rect), paint);
        }
        Clip::Path {
            path: id,
            fill_rule,
        } => {
            let data = paths
                .get(id.raw() as usize)
                .ok_or(DrawError::MissingPath(id.raw()))?;
            canvas.draw_path(&path(data, *fill_rule)?, paint);
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

fn draw_shadow(canvas: &skia_safe::Canvas, shadow: &valle_draw::program::ShadowNode) {
    let mut paint = SkPaint::default();
    paint.set_anti_alias(true);
    let space = working_color_space().ok();
    paint.set_color4f(straight_color(shadow.color), space.as_ref());
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
}

fn shader_uniforms(
    effect: &RuntimeEffect,
    bounds: Rect,
    bindings: &[ShaderUniformBinding],
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
            ShaderUniformValue::Color(color) => {
                let color = straight_color(color);
                vec![color.r, color.g, color.b, color.a]
            }
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
        != bindings.len()
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
    #[error("DrawProgram content identity {content_hash} resolves to different packed bytes")]
    ProgramCacheCollision { content_hash: String },
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
    #[error("extension implementation digest does not match engine-owned ABI {abi}")]
    ExtensionImplementationMismatch { abi: &'static str },
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
