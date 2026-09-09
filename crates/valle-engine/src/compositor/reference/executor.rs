use std::{collections::BTreeMap, sync::Arc};

use thiserror::Error;
use ttf_parser::{Face, GlyphId, OutlineBuilder};
use valle_draw::{
    math::{exp, sqrt},
    program::{
        Clip, DrawProgram, FILTER_GAUSSIAN_SUPPORT_SIGMAS, Filter, GlyphRun, MaskMode, Node,
        NodeId, Paint, PathData, PathVerb, Transform2d,
    },
    requirements::SamplingMode,
};

use crate::{
    compositor::{
        BoundExternalObjects, ExternalBindError, ExternalObject, ExternalObjectTable,
        bind_external_objects,
    },
    prepare::{
        DeviceRect, DeviceTransform, DynamicBindingId, DynamicBindingKind, DynamicValue,
        ExternalPlacement, ExternalSample, MASK_GAUSSIAN_SUPPORT_SIGMAS, PreparedBlurAxis,
        PreparedEffectKernel, PreparedEffectSpace, PreparedExternalBackdrop, PreparedMask,
        PreparedMaskGeometry, PreparedMaskShape, PreparedUnitRect, ProgramId,
    },
    resource::{
        Extent2d, ExternalPixelLayout, ExternalResourceDesc, OutputSpec, ResourceInterpretation,
        ResourceKey,
    },
};

use super::{
    BlendError, CompositeError, EFFECT_GAUSSIAN_SUPPORT_SIGMAS, LayerPipeline, OutputMathError,
    PixelError, PremulRgba32, ReferenceBlendMode, ReferenceImage, ReferenceMaskMode,
    ReferenceOutputSample, ReferenceTransitionError, chroma_key_coverage, chroma_key_despill,
    color_grade_pixel, color_matrix_pixel, composite_layer, composite_transition,
    effective_blend_source, mask_coverage_pixel, spotlight_pixel, transform_output_image,
};
use crate::compositor::lower::{
    BackendCapabilities, BoundProgramSchedule, BoundProgramSchedules, CompositeMode, CopyOperation,
    ExecutionPassId, ExecutionPassKind, ExternalSlotId, KernelInvocation, PlanEffect, PlanProgram,
    PlanResourceId, PlanResourceKind, PlanSourcePipeline, ProgramBindingError,
    ProgramDestinationId, ProgramPassKind, ProgramResourceId, ProgramStorageKind, RenderBindings,
    RenderPlanTemplate,
};

const MASK_COVERAGE_SAMPLES_PER_AXIS: u32 = 4;
const PROGRAM_COVERAGE_SAMPLES_PER_AXIS: u32 = 4;
const DIRECTIONAL_BLUR_SAMPLES: u32 = 9;
const MAX_REFERENCE_KERNEL_SAMPLES: u64 = 16 * 1024 * 1024;
const MAX_REFERENCE_RASTER_SAMPLES: u64 = 128 * 1024 * 1024;
const MAX_REFERENCE_RASTER_GEOMETRY_TESTS: u64 = 1024 * 1024 * 1024;
const MAX_REFERENCE_OUTLINE_SEGMENTS: u64 = 1024 * 1024;
const GLYPH_CURVE_TOLERANCE_PER_EM: f64 = 1.0 / 2048.0;
const MAX_GLYPH_CURVE_SUBDIVISION_DEPTH: u8 = 16;

/// Pure-memory object admitted by the RGBA32F reference executor.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceExternalObject {
    key: ResourceKey,
    descriptor: ExternalResourceDesc,
    payload: ReferenceObjectPayload,
}

#[derive(Debug, Clone, PartialEq)]
enum ReferenceObjectPayload {
    Visual(ReferenceImage),
    FontBytes(Arc<[u8]>),
}

impl ReferenceExternalObject {
    /// Creates a visual fulfillment whose pixels have already been decoded through the C0 import
    /// color/alpha math into Linear Rec.2020 premultiplied RGBA32F. `pixel_layout` still records
    /// the frozen host-side object contract checked by bind; placement sampling never reinterprets
    /// that platform layout.
    pub fn visual(
        key: ResourceKey,
        pixel_layout: ExternalPixelLayout,
        image: ReferenceImage,
    ) -> Result<Self, ReferenceObjectError> {
        if !matches!(key.interpretation, ResourceInterpretation::Visual { .. }) {
            return Err(ReferenceObjectError::VisualKeyRequired);
        }
        Ok(Self {
            descriptor: ExternalResourceDesc::VisualFrame {
                extent: image.extent(),
                pixel_layout,
            },
            key,
            payload: ReferenceObjectPayload::Visual(image),
        })
    }

    pub fn visual_image(&self) -> Option<&ReferenceImage> {
        match &self.payload {
            ReferenceObjectPayload::Visual(image) => Some(image),
            ReferenceObjectPayload::FontBytes(_) => None,
        }
    }

    /// Creates an immutable font fulfillment. Unlike an interpreted visual frame, font bytes are
    /// the content-addressed payload itself, so construction re-hashes them instead of trusting a
    /// host-supplied key. Face parsing remains an executor preflight concern.
    pub fn font_bytes(
        key: ResourceKey,
        bytes: impl Into<Arc<[u8]>>,
    ) -> Result<Self, ReferenceObjectError> {
        if !matches!(key.interpretation, ResourceInterpretation::FontFace { .. }) {
            return Err(ReferenceObjectError::FontKeyRequired);
        }
        let bytes = bytes.into();
        let actual = crate::resource::ContentDigest::of_bytes(bytes.as_ref());
        if actual != key.content {
            return Err(ReferenceObjectError::FontDigestMismatch {
                expected: key.content.clone(),
                actual,
            });
        }
        Ok(Self {
            descriptor: ExternalResourceDesc::FontBytes,
            key,
            payload: ReferenceObjectPayload::FontBytes(bytes),
        })
    }

    pub fn font_data(&self) -> Option<&[u8]> {
        match &self.payload {
            ReferenceObjectPayload::Visual(_) => None,
            ReferenceObjectPayload::FontBytes(bytes) => Some(bytes),
        }
    }
}

impl ExternalObject for ReferenceExternalObject {
    fn key(&self) -> &ResourceKey {
        &self.key
    }

    fn descriptor(&self) -> &ExternalResourceDesc {
        &self.descriptor
    }
}

pub type ReferenceObjectTable = ExternalObjectTable<ReferenceExternalObject>;

/// Fully transformed target payload. Samples are row-major and match `extent` exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceOutputFrame {
    extent: Extent2d,
    spec: OutputSpec,
    samples: Vec<ReferenceOutputSample>,
}

impl ReferenceOutputFrame {
    pub const fn extent(&self) -> Extent2d {
        self.extent
    }

    pub const fn spec(&self) -> OutputSpec {
        self.spec
    }

    pub fn samples(&self) -> &[ReferenceOutputSample] {
        &self.samples
    }
}

/// Transactional reference target. `committed` changes only after the entire frame succeeds.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceTarget {
    extent: Extent2d,
    spec: OutputSpec,
    committed: Option<ReferenceOutputFrame>,
}

impl ReferenceTarget {
    pub const fn new(extent: Extent2d, spec: OutputSpec) -> Self {
        Self {
            extent,
            spec,
            committed: None,
        }
    }

    pub const fn extent(&self) -> Extent2d {
        self.extent
    }

    pub const fn spec(&self) -> OutputSpec {
        self.spec
    }

    pub const fn frame(&self) -> Option<&ReferenceOutputFrame> {
        self.committed.as_ref()
    }

    fn commit(&mut self, frame: ReferenceOutputFrame) {
        self.committed = Some(frame);
    }
}

/// Executes one fully admitted plan into scratch storage and commits the target exactly once.
pub fn execute_reference(
    template: &RenderPlanTemplate,
    bindings: &RenderBindings,
    capabilities: &BackendCapabilities,
    table: &ReferenceObjectTable,
    target: &mut ReferenceTarget,
) -> Result<(), ReferenceExecuteError> {
    admit_reference_frame(template, bindings, capabilities, table, target)?.execute(target)
}

/// A frame whose complete plan, bindings, external objects and reference-only work budgets have
/// passed admission without allocating compositor scratch or mutating the target.
///
/// The value immutably borrows the external object table, so the host cannot replace a fulfilled
/// handle between bind and execute. Construction is intentionally available only through
/// [`admit_reference_frame`].
#[derive(Debug)]
pub struct AdmittedReferenceFrame<'a> {
    template: &'a RenderPlanTemplate,
    bindings: &'a RenderBindings,
    bound: BoundExternalObjects<'a, ReferenceExternalObject>,
    preflight: ReferencePreflight,
}

/// Completes every reference-executor check that can fail before scratch allocation.
///
/// This is the observable bind/admission boundary used by `inspect-frame`: capability proof,
/// dynamic program schedules, external object fulfillment, supported-pass validation and all
/// cumulative raster/kernel budgets are closed here. A successful value may then be timed and
/// executed separately without repeating bind work.
pub fn admit_reference_frame<'a>(
    template: &'a RenderPlanTemplate,
    bindings: &'a RenderBindings,
    capabilities: &BackendCapabilities,
    table: &'a ReferenceObjectTable,
    target: &ReferenceTarget,
) -> Result<AdmittedReferenceFrame<'a>, ReferenceExecuteError> {
    preflight_target(template, target)?;
    let program_schedules = template.bind_program_schedules(bindings, capabilities)?;
    let bound = bind_external_objects(template, bindings, table)?;
    let template_hash = template.template_hash().map_err(ExternalBindError::from)?;
    if bound.template_hash() != &template_hash {
        return Err(ReferenceExecuteError::BoundTemplateMismatch);
    }
    let preflight = preflight_passes(template, bindings, &bound, program_schedules)?;

    Ok(AdmittedReferenceFrame {
        template,
        bindings,
        bound,
        preflight,
    })
}

impl AdmittedReferenceFrame<'_> {
    /// Allocates private reference scratch, executes the admitted plan and commits exactly once.
    pub fn execute(self, target: &mut ReferenceTarget) -> Result<(), ReferenceExecuteError> {
        // The target is not borrowed by the admitted value and may have been replaced by the host
        // between phases. Recheck its immutable contract before allocating scratch.
        preflight_target(self.template, target)?;
        let template = self.template;
        let bindings = self.bindings;
        let bound = self.bound;
        let preflight = self.preflight;

        let extent = Extent2d::new(
            template.render_spec().width(),
            template.render_spec().height(),
        )
        .map_err(|_| ReferenceExecuteError::InvalidRenderExtent)?;
        let mut surfaces = BTreeMap::<PlanResourceId, ReferenceImage>::new();
        let mut output = None;

        for pass in template.passes() {
            match &pass.kind {
                ExecutionPassKind::ClearRegion {
                    output,
                    working_linear_rec2020_premul,
                } => {
                    let pixel = PremulRgba32::from_premultiplied(*working_linear_rec2020_premul)?;
                    surfaces.insert(*output, ReferenceImage::solid(extent, pixel)?);
                }
                ExecutionPassKind::ImportRegion {
                    external,
                    source_pipeline,
                    output,
                    placement,
                    transform,
                    bounds,
                } => {
                    let image = raster_import(
                        pass.id,
                        template,
                        &bound,
                        bindings,
                        *external,
                        source_pipeline,
                        *placement,
                        *transform,
                        *bounds,
                        extent,
                    )?;
                    surfaces.insert(*output, image);
                }
                ExecutionPassKind::RasterProgram {
                    program,
                    destination_inputs,
                    output,
                    transform,
                    ..
                } => {
                    let schedule = preflight.program_schedule(*program, pass.id)?;
                    let image = raster_program(
                        preflight.program(*program)?,
                        plan_program(bindings, *program)?,
                        schedule,
                        &bound,
                        bindings,
                        *transform,
                        destination_inputs,
                        &surfaces,
                        extent,
                        pass.id,
                    )?;
                    surfaces.insert(*output, image);
                }
                ExecutionPassKind::RasterCaption {
                    program,
                    destination,
                    output,
                    transform,
                    opacity,
                    ..
                } => {
                    let schedule = preflight.program_schedule(*program, pass.id)?;
                    let source = raster_program(
                        preflight.program(*program)?,
                        plan_program(bindings, *program)?,
                        schedule,
                        &bound,
                        bindings,
                        *transform,
                        &[],
                        &surfaces,
                        extent,
                        pass.id,
                    )?;
                    let pipeline = LayerPipeline::new(
                        dynamic_scalar(bindings, *opacity)?,
                        ReferenceBlendMode::Normal,
                    )?;
                    let image =
                        composite_layer(&source, surface(&surfaces, *destination)?, &pipeline)?;
                    surfaces.insert(*output, image);
                }
                ExecutionPassKind::BindBackdropView { input, output, .. }
                | ExecutionPassKind::AliasResource { input, output, .. } => {
                    let image = surface(&surfaces, *input)?.clone();
                    surfaces.insert(*output, image);
                }
                ExecutionPassKind::ResolveRegion {
                    input,
                    output,
                    sample_bounds,
                    output_bounds,
                    ..
                } => {
                    let (sample, _) =
                        backdrop_bounds(bindings, *sample_bounds, *output_bounds, extent)?;
                    let image = copy_region(surface(&surfaces, *input)?, sample, extent)?;
                    surfaces.insert(*output, image);
                }
                ExecutionPassKind::DispatchKernel { invocation } => match invocation {
                    KernelInvocation::Group { input, output } => {
                        let image = surface(&surfaces, *input)?.clone();
                        surfaces.insert(*output, image);
                    }
                    KernelInvocation::Mask {
                        input,
                        output,
                        mask,
                    } => {
                        let image = apply_prepared_mask(surface(&surfaces, *input)?, *mask)?;
                        surfaces.insert(*output, image);
                    }
                    KernelInvocation::Transition {
                        backdrop,
                        from,
                        to,
                        output,
                        kernel,
                        progress,
                        from_opacity,
                        to_opacity,
                    } => {
                        let image = composite_transition(
                            surface(&surfaces, *backdrop)?,
                            surface(&surfaces, *from)?,
                            surface(&surfaces, *to)?,
                            *kernel,
                            dynamic_scalar_kind(
                                bindings,
                                *progress,
                                DynamicBindingKind::TransitionProgress,
                            )?,
                            dynamic_scalar(bindings, *from_opacity)?,
                            dynamic_scalar(bindings, *to_opacity)?,
                        )?;
                        surfaces.insert(*output, image);
                    }
                    KernelInvocation::Filter {
                        input,
                        output,
                        effect,
                        ..
                    }
                    | KernelInvocation::AdjustmentEffect {
                        input,
                        output,
                        effect,
                        ..
                    } => {
                        let image = apply_reference_effect(
                            pass.id,
                            surface(&surfaces, *input)?,
                            effect,
                            bindings,
                        )?;
                        surfaces.insert(*output, image);
                    }
                },
                ExecutionPassKind::CompositeRegion {
                    backdrop,
                    layer,
                    output,
                    opacity,
                    mode,
                } => {
                    let backdrop =
                        surfaces
                            .get(backdrop)
                            .ok_or(ReferenceExecuteError::MissingResource {
                                resource: *backdrop,
                            })?;
                    let layer = surfaces
                        .get(layer)
                        .ok_or(ReferenceExecuteError::MissingResource { resource: *layer })?;
                    let opacity = dynamic_scalar(bindings, *opacity)?;
                    let blend = match mode {
                        CompositeMode::SourceOver {} => ReferenceBlendMode::Normal,
                        CompositeMode::Blend { mode } => (*mode).into(),
                    };
                    let pipeline = LayerPipeline::new(opacity, blend)?;
                    let composite = composite_layer(layer, backdrop, &pipeline)?;
                    surfaces.insert(*output, composite);
                }
                ExecutionPassKind::CopyConvert {
                    input,
                    output: output_resource,
                    operation: CopyOperation::OutputTransform { spec },
                } => {
                    let image = surfaces
                        .get(input)
                        .ok_or(ReferenceExecuteError::MissingResource { resource: *input })?;
                    if *output_resource != template.output() {
                        return Err(ReferenceExecuteError::WrongOutputResource {
                            resource: *output_resource,
                        });
                    }
                    output = Some(ReferenceOutputFrame {
                        extent,
                        spec: *spec,
                        samples: transform_output_image(image, *spec)?,
                    });
                }
                _ => unreachable!("all unsupported passes are rejected by preflight_passes"),
            }
        }

        target.commit(output.ok_or(ReferenceExecuteError::MissingOutput)?);
        Ok(())
    }
}

fn preflight_target(
    template: &RenderPlanTemplate,
    target: &ReferenceTarget,
) -> Result<(), ReferenceExecuteError> {
    let expected_extent = Extent2d::new(
        template.render_spec().width(),
        template.render_spec().height(),
    )
    .map_err(|_| ReferenceExecuteError::InvalidRenderExtent)?;
    if target.extent != expected_extent {
        return Err(ReferenceExecuteError::TargetExtentMismatch {
            expected: expected_extent,
            actual: target.extent,
        });
    }
    let expected_spec = template.render_spec().output();
    if target.spec != expected_spec {
        return Err(ReferenceExecuteError::TargetSpecMismatch);
    }
    Ok(())
}

#[derive(Debug)]
struct ReferencePreflight {
    programs: BTreeMap<ProgramId, ReferenceProgramRuntime>,
    program_schedules: BoundProgramSchedules,
}

#[derive(Debug, Default)]
struct ReferenceRasterBudget {
    coverage_samples: u64,
    geometry_tests: u64,
    outline_segments: u64,
    kernel_samples: u64,
}

impl ReferencePreflight {
    fn program(&self, id: ProgramId) -> Result<&ReferenceProgramRuntime, ReferenceExecuteError> {
        self.programs
            .get(&id)
            .ok_or(ReferenceExecuteError::InvalidProgram { program: id })
    }

    fn program_schedule(
        &self,
        id: ProgramId,
        pass: ExecutionPassId,
    ) -> Result<&BoundProgramSchedule, ReferenceExecuteError> {
        self.program_schedules
            .programs()
            .iter()
            .find(|schedule| schedule.program() == id && schedule.execution_pass() == pass)
            .ok_or(ReferenceExecuteError::InvalidProgramSchedule { program: id, pass })
    }
}

fn preflight_passes(
    template: &RenderPlanTemplate,
    bindings: &RenderBindings,
    bound: &BoundExternalObjects<'_, ReferenceExternalObject>,
    program_schedules: BoundProgramSchedules,
) -> Result<ReferencePreflight, ReferenceExecuteError> {
    let extent = Extent2d::new(
        template.render_spec().width(),
        template.render_spec().height(),
    )
    .map_err(|_| ReferenceExecuteError::InvalidRenderExtent)?;
    let mut kernel_samples = 0_u64;
    let mut raster_budget = ReferenceRasterBudget::default();
    let mut preflight = ReferencePreflight {
        programs: BTreeMap::new(),
        program_schedules,
    };
    for pass in template.passes() {
        let supported = match &pass.kind {
            ExecutionPassKind::ClearRegion { .. }
            | ExecutionPassKind::CompositeRegion { .. }
            | ExecutionPassKind::BindBackdropView { .. }
            | ExecutionPassKind::AliasResource { .. }
            | ExecutionPassKind::DispatchKernel {
                invocation:
                    KernelInvocation::Group { .. }
                    | KernelInvocation::Mask { .. }
                    | KernelInvocation::Transition { .. },
            }
            | ExecutionPassKind::CopyConvert {
                operation: CopyOperation::OutputTransform { .. },
                ..
            } => true,
            ExecutionPassKind::DispatchKernel {
                invocation:
                    KernelInvocation::Filter { effect, .. }
                    | KernelInvocation::AdjustmentEffect { effect, .. },
            } => {
                preflight_reference_effect(pass.id, effect, bindings, extent, &mut kernel_samples)?;
                true
            }
            ExecutionPassKind::ImportRegion {
                source_pipeline,
                bounds,
                placement,
                ..
            } => {
                preflight_import(
                    pass.id,
                    source_pipeline,
                    *placement,
                    bindings,
                    *bounds,
                    extent,
                    &mut kernel_samples,
                )?;
                true
            }
            ExecutionPassKind::RasterProgram {
                program, transform, ..
            }
            | ExecutionPassKind::RasterCaption {
                program, transform, ..
            } => {
                preflight.program_schedule(*program, pass.id)?;
                let prepared = plan_program(bindings, *program)?;
                if !preflight.programs.contains_key(program) {
                    let runtime =
                        prepare_reference_program(prepared, bound, pass.id, &mut raster_budget)?;
                    preflight.programs.insert(*program, runtime);
                }
                preflight_program(
                    preflight.program(*program)?,
                    prepared,
                    preflight.program_schedule(*program, pass.id)?,
                    bindings,
                    pass.id,
                    *transform,
                    extent,
                    &mut raster_budget,
                )?;
                true
            }
            ExecutionPassKind::ResolveRegion {
                sample_bounds,
                output_bounds,
                ..
            } => {
                backdrop_bounds(bindings, *sample_bounds, *output_bounds, extent)?;
                true
            }
            _ => false,
        };
        if !supported {
            return Err(ReferenceExecuteError::UnsupportedPass { pass: pass.id });
        }
    }
    Ok(preflight)
}

fn plan_program(
    bindings: &RenderBindings,
    id: ProgramId,
) -> Result<&PlanProgram, ReferenceExecuteError> {
    bindings
        .programs()
        .get(id.get() as usize - 1)
        .filter(|program| program.id == id)
        .ok_or(ReferenceExecuteError::InvalidProgram { program: id })
}

fn preflight_program(
    program: &ReferenceProgramRuntime,
    prepared: &PlanProgram,
    schedule: &BoundProgramSchedule,
    bindings: &RenderBindings,
    pass: ExecutionPassId,
    transform: DynamicBindingId,
    extent: Extent2d,
    budget: &mut ReferenceRasterBudget,
) -> Result<(), ReferenceExecuteError> {
    let transform = dynamic_transform(bindings, transform)?;
    let samples_per_pixel =
        u64::from(PROGRAM_COVERAGE_SAMPLES_PER_AXIS * PROGRAM_COVERAGE_SAMPLES_PER_AXIS);
    for local_pass in prepared.local_plan().passes() {
        match &local_pass.kind {
            ProgramPassKind::RasterNode { node, output } => {
                let output_roi = bound_program_resource_roi(schedule, prepared.id, pass, *output)?;
                let resource_transform = DeviceTransform::from_projective(
                    program_resource_normalized_matrices(
                        prepared,
                        *output,
                        program.viewport,
                        transform.matrix(),
                    )?
                    .0,
                )
                .map_err(|_| ReferenceExecuteError::InvalidSampleCoordinate)?;
                preflight_program_raster_operation(
                    program,
                    program.node(*node)?,
                    output_roi,
                    resource_transform,
                    extent,
                    pass,
                    samples_per_pixel,
                    budget,
                )?;
            }
            ProgramPassKind::RasterTree { output, .. } => {
                let output_roi = bound_program_resource_roi(schedule, prepared.id, pass, *output)?;
                for (node, local_to_program) in program
                    .raster_tree(*output)?
                    .iter()
                    .filter_map(ReferenceTreeStep::draw)
                {
                    let local_to_device = program_transform_to_device(
                        prepared,
                        local_to_program,
                        program.viewport,
                        transform.matrix(),
                    )?;
                    let resource_transform = DeviceTransform::from_projective(
                        normalized_program_matrices(local_to_device, program.viewport)?.0,
                    )
                    .map_err(|_| ReferenceExecuteError::InvalidSampleCoordinate)?;
                    preflight_program_raster_operation(
                        program,
                        program.node(node)?,
                        output_roi,
                        resource_transform,
                        extent,
                        pass,
                        samples_per_pixel,
                        budget,
                    )?;
                }
            }
            ProgramPassKind::Backdrop {
                output, filters, ..
            } => {
                if !filters.is_empty() {
                    return Err(ReferenceExecuteError::UnsupportedPass { pass });
                }
                let output_roi = bound_program_resource_roi(schedule, prepared.id, pass, *output)?;
                reserve_raster_samples(
                    pass,
                    output_roi.pixels(),
                    1,
                    samples_per_pixel,
                    &mut budget.coverage_samples,
                )?;
            }
            ProgramPassKind::MotionGlass {
                output, program, ..
            } => {
                let output_roi = bound_program_resource_roi(schedule, prepared.id, pass, *output)?;
                let blur_radius = f64::from(program.material.roughness)
                    * f64::from(program.material.thickness)
                    * 0.65;
                // R/G/B Snell reads plus four diagonal roughness taps when diffusion is active.
                let texture_samples = if blur_radius >= 0.5 { 7 } else { 3 };
                reserve_kernel_samples(
                    pass,
                    output_roi.pixels(),
                    texture_samples,
                    &mut budget.kernel_samples,
                )?;
            }
            ProgramPassKind::ApplyMotionGlassForeground {
                output, program, ..
            } => {
                let output_roi = bound_program_resource_roi(schedule, prepared.id, pass, *output)?;
                reserve_raster_samples(
                    pass,
                    output_roi.pixels(),
                    1,
                    samples_per_pixel,
                    &mut budget.coverage_samples,
                )?;
                if program.shape == valle_draw::program::PackedGlassShapeKind::Path {
                    reserve_raster_geometry_tests(
                        pass,
                        output_roi.pixels(),
                        u64::try_from(program.path_points.len()).unwrap_or(u64::MAX),
                        samples_per_pixel,
                        &mut budget.geometry_tests,
                    )?;
                }
            }
            ProgramPassKind::ApplyClip { node, output, .. } => {
                let output_roi = bound_program_resource_roi(schedule, prepared.id, pass, *output)?;
                reserve_raster_samples(
                    pass,
                    output_roi.pixels(),
                    1,
                    samples_per_pixel,
                    &mut budget.coverage_samples,
                )?;
                if let ReferenceClip::Path(Some(shape)) = program.clip(*node)? {
                    let resource_transform = DeviceTransform::from_projective(
                        program_resource_normalized_matrices(
                            prepared,
                            *output,
                            program.viewport,
                            transform.matrix(),
                        )?
                        .0,
                    )
                    .map_err(|_| ReferenceExecuteError::InvalidSampleCoordinate)?;
                    reserve_raster_geometry_tests(
                        pass,
                        output_roi.pixels(),
                        1,
                        samples_per_pixel,
                        &mut budget.geometry_tests,
                    )?;
                    let shape_bounds = project_program_local_bounds(
                        program.viewport,
                        resource_transform,
                        shape.bounds,
                        extent,
                    )?
                    .intersect(output_roi);
                    reserve_raster_geometry_tests(
                        pass,
                        shape_bounds.pixels(),
                        shape.segments.len() as u64,
                        samples_per_pixel,
                        &mut budget.geometry_tests,
                    )?;
                }
            }
            ProgramPassKind::ApplyFilter {
                node,
                filter_index,
                output,
                ..
            } => {
                let filter = program.filter(*node, *filter_index)?;
                match filter {
                    Filter::Blur { sigma_x, sigma_y }
                    | Filter::DropShadow {
                        sigma_x, sigma_y, ..
                    } => {
                        let output_roi =
                            bound_program_resource_roi(schedule, prepared.id, pass, *output)?;
                        let support = f64::from(FILTER_GAUSSIAN_SUPPORT_SIGMAS);
                        let taps = gaussian_axis_tap_count(f64::from(*sigma_x), support)
                            .and_then(|horizontal| {
                                gaussian_axis_tap_count(f64::from(*sigma_y), support)
                                    .and_then(|vertical| horizontal.checked_mul(vertical))
                            })
                            .ok_or(ReferenceExecuteError::KernelSampleBudgetExceeded {
                                pass,
                                required: u64::MAX,
                                maximum: MAX_REFERENCE_KERNEL_SAMPLES,
                            })?;
                        reserve_kernel_samples(
                            pass,
                            output_roi.pixels(),
                            taps,
                            &mut budget.kernel_samples,
                        )?;
                    }
                    Filter::ColorMatrix { .. } => {}
                    _ => return Err(ReferenceExecuteError::UnsupportedPass { pass }),
                }
            }
            ProgramPassKind::ApplyShader { .. } => {
                return Err(ReferenceExecuteError::UnsupportedPass { pass });
            }
            _ => {}
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn preflight_program_raster_operation(
    program: &ReferenceProgramRuntime,
    operation: &ReferenceProgramOperation,
    output_roi: DeviceRect,
    transform: DeviceTransform,
    extent: Extent2d,
    pass: ExecutionPassId,
    samples_per_pixel: u64,
    budget: &mut ReferenceRasterBudget,
) -> Result<(), ReferenceExecuteError> {
    reserve_raster_samples(
        pass,
        output_roi.pixels(),
        1,
        samples_per_pixel,
        &mut budget.coverage_samples,
    )?;
    let ReferenceProgramOperation::Fill(fill) = operation else {
        return Ok(());
    };
    reserve_raster_geometry_tests(
        pass,
        output_roi.pixels(),
        fill.shapes.len() as u64,
        samples_per_pixel,
        &mut budget.geometry_tests,
    )?;
    for shape in &fill.shapes {
        let shape_bounds =
            project_program_local_bounds(program.viewport, transform, shape.bounds, extent)?
                .intersect(output_roi);
        reserve_raster_geometry_tests(
            pass,
            shape_bounds.pixels(),
            shape.segments.len() as u64,
            samples_per_pixel,
            &mut budget.geometry_tests,
        )?;
    }
    Ok(())
}

fn bound_program_resource_roi(
    schedule: &BoundProgramSchedule,
    program: ProgramId,
    pass: ExecutionPassId,
    resource: ProgramResourceId,
) -> Result<DeviceRect, ReferenceExecuteError> {
    schedule
        .resources()
        .get(resource.index())
        .filter(|binding| binding.resource() == resource)
        .map(|binding| binding.device_roi())
        .ok_or(ReferenceExecuteError::InvalidProgramSchedule { program, pass })
}

fn reserve_raster_samples(
    pass: ExecutionPassId,
    pixels: u64,
    paths: u64,
    samples_per_pixel: u64,
    raster_samples: &mut u64,
) -> Result<(), ReferenceExecuteError> {
    let required = pixels
        .checked_mul(paths)
        .and_then(|value| value.checked_mul(samples_per_pixel))
        .ok_or(ReferenceExecuteError::RasterSampleBudgetExceeded {
            pass,
            required: u64::MAX,
            maximum: MAX_REFERENCE_RASTER_SAMPLES,
        })?;
    *raster_samples = raster_samples.checked_add(required).ok_or(
        ReferenceExecuteError::RasterSampleBudgetExceeded {
            pass,
            required: u64::MAX,
            maximum: MAX_REFERENCE_RASTER_SAMPLES,
        },
    )?;
    if *raster_samples > MAX_REFERENCE_RASTER_SAMPLES {
        return Err(ReferenceExecuteError::RasterSampleBudgetExceeded {
            pass,
            required: *raster_samples,
            maximum: MAX_REFERENCE_RASTER_SAMPLES,
        });
    }
    Ok(())
}

fn reserve_raster_geometry_tests(
    pass: ExecutionPassId,
    pixels: u64,
    segments: u64,
    samples_per_pixel: u64,
    raster_geometry_tests: &mut u64,
) -> Result<(), ReferenceExecuteError> {
    let required = pixels
        .checked_mul(segments)
        .and_then(|value| value.checked_mul(samples_per_pixel))
        .ok_or(ReferenceExecuteError::RasterGeometryBudgetExceeded {
            pass,
            required: u64::MAX,
            maximum: MAX_REFERENCE_RASTER_GEOMETRY_TESTS,
        })?;
    *raster_geometry_tests = raster_geometry_tests.checked_add(required).ok_or(
        ReferenceExecuteError::RasterGeometryBudgetExceeded {
            pass,
            required: u64::MAX,
            maximum: MAX_REFERENCE_RASTER_GEOMETRY_TESTS,
        },
    )?;
    if *raster_geometry_tests > MAX_REFERENCE_RASTER_GEOMETRY_TESTS {
        return Err(ReferenceExecuteError::RasterGeometryBudgetExceeded {
            pass,
            required: *raster_geometry_tests,
            maximum: MAX_REFERENCE_RASTER_GEOMETRY_TESTS,
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum ReferenceTreeStep {
    Draw(NodeId, Transform2d),
    PushOpacity,
    PopOpacity(f32),
}

impl ReferenceTreeStep {
    fn draw(&self) -> Option<(NodeId, Transform2d)> {
        match *self {
            Self::Draw(node, transform) => Some((node, transform)),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct ReferenceProgramRuntime {
    id: ProgramId,
    viewport: valle_draw::Rect,
    nodes: Vec<Option<ReferenceProgramOperation>>,
    raster_trees: BTreeMap<ProgramResourceId, Vec<ReferenceTreeStep>>,
    clips: Vec<Option<ReferenceClip>>,
    filters: BTreeMap<(u32, u32), Filter>,
    masks: BTreeMap<u32, MaskMode>,
}

impl ReferenceProgramRuntime {
    fn raster_tree(
        &self,
        output: ProgramResourceId,
    ) -> Result<&[ReferenceTreeStep], ReferenceExecuteError> {
        self.raster_trees
            .get(&output)
            .map(Vec::as_slice)
            .ok_or(ReferenceExecuteError::InvalidProgram { program: self.id })
    }

    fn node(&self, node: NodeId) -> Result<&ReferenceProgramOperation, ReferenceExecuteError> {
        self.nodes
            .get(node.raw() as usize)
            .and_then(Option::as_ref)
            .ok_or(ReferenceExecuteError::InvalidProgramNode {
                program: self.id,
                node,
            })
    }

    fn clip(&self, node: NodeId) -> Result<&ReferenceClip, ReferenceExecuteError> {
        self.clips
            .get(node.raw() as usize)
            .and_then(Option::as_ref)
            .ok_or(ReferenceExecuteError::InvalidProgramClip {
                program: self.id,
                node,
            })
    }

    fn filter(&self, node: NodeId, filter_index: u32) -> Result<&Filter, ReferenceExecuteError> {
        self.filters.get(&(node.raw(), filter_index)).ok_or(
            ReferenceExecuteError::InvalidProgramFilter {
                program: self.id,
                node,
                filter_index,
            },
        )
    }

    fn mask_mode(&self, node: NodeId) -> Result<MaskMode, ReferenceExecuteError> {
        self.masks
            .get(&node.raw())
            .copied()
            .ok_or(ReferenceExecuteError::InvalidProgramMask {
                program: self.id,
                node,
            })
    }
}

#[derive(Debug)]
enum ReferenceProgramOperation {
    Fill(ReferenceFill),
    Image(ReferenceImageOperation),
}

#[derive(Debug)]
struct ReferenceFill {
    shapes: Vec<ReferenceShape>,
    color: PremulRgba32,
}

#[derive(Debug)]
struct ReferenceShape {
    segments: Vec<LineSegment>,
    bounds: valle_draw::Rect,
}

#[derive(Debug)]
enum ReferenceClip {
    Rect(valle_draw::Rect),
    Path(Option<ReferenceShape>),
}

impl ReferenceClip {
    fn contains(&self, point: [f64; 2]) -> bool {
        match self {
            Self::Rect(rect) => contains_half_open(*rect, point),
            Self::Path(Some(shape)) => {
                point[0] >= shape.bounds.left()
                    && point[0] <= shape.bounds.right()
                    && point[1] >= shape.bounds.top()
                    && point[1] <= shape.bounds.bottom()
                    && nonzero_line_path_winding(&shape.segments, point) != 0
            }
            Self::Path(None) => false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ReferenceImageOperation {
    slot: ExternalSlotId,
    sample: ExternalSample,
    dst: valle_draw::Rect,
    sampling: SamplingMode,
    opacity: f32,
}

#[derive(Debug, Clone, Copy)]
struct LineSegment {
    from: [f64; 2],
    to: [f64; 2],
}

fn prepare_reference_program(
    prepared: &PlanProgram,
    bound: &BoundExternalObjects<'_, ReferenceExternalObject>,
    pass: ExecutionPassId,
    budget: &mut ReferenceRasterBudget,
) -> Result<ReferenceProgramRuntime, ReferenceExecuteError> {
    let id = prepared.id;
    let program = DrawProgram::from_packed(prepared.packed())
        .map_err(|_| ReferenceExecuteError::InvalidProgram { program: id })?;
    let mut nodes = std::iter::repeat_with(|| None)
        .take(program.nodes().len())
        .collect::<Vec<_>>();
    let mut clips = std::iter::repeat_with(|| None)
        .take(program.nodes().len())
        .collect::<Vec<_>>();
    let mut filters = BTreeMap::new();
    let mut masks = BTreeMap::new();
    let mut raster_trees = BTreeMap::new();
    for local_pass in prepared.local_plan().passes() {
        match &local_pass.kind {
            ProgramPassKind::RasterNode { node, .. } => {
                let node_index = node.raw() as usize;
                let Some(slot) = nodes.get_mut(node_index) else {
                    return Err(ReferenceExecuteError::InvalidProgram { program: id });
                };
                if slot.is_none() {
                    *slot = Some(prepare_reference_node_operation(
                        &program, prepared, bound, pass, id, *node, budget,
                    )?);
                }
            }
            ProgramPassKind::RasterTree { roots, output } => {
                let local_to_program =
                    prepared.local_plan().resources()[output.index()].local_to_program;
                let draws = collect_reference_raster_tree(&program, roots, local_to_program, pass)?;
                for (node, _) in draws.iter().filter_map(ReferenceTreeStep::draw) {
                    let slot = &mut nodes[node.raw() as usize];
                    if slot.is_none() {
                        *slot = Some(prepare_reference_node_operation(
                            &program, prepared, bound, pass, id, node, budget,
                        )?);
                    }
                }
                raster_trees.insert(*output, draws);
            }
            ProgramPassKind::ApplyClip { node, clip, .. } => {
                let node_index = node.raw() as usize;
                let Some(slot) = clips.get_mut(node_index) else {
                    return Err(ReferenceExecuteError::InvalidProgram { program: id });
                };
                if slot.is_none() {
                    let Some(Node::Group(group)) = program.nodes().get(node_index) else {
                        return Err(ReferenceExecuteError::InvalidProgramClip {
                            program: id,
                            node: *node,
                        });
                    };
                    if group.clip.as_ref() != Some(clip) {
                        return Err(ReferenceExecuteError::InvalidProgramClip {
                            program: id,
                            node: *node,
                        });
                    }
                    *slot = Some(prepare_reference_clip(
                        &program,
                        clip,
                        pass,
                        id,
                        &mut budget.outline_segments,
                    )?);
                }
            }
            ProgramPassKind::ApplyFilter {
                node,
                filter_index,
                filter,
                ..
            } => {
                let node_index = node.raw() as usize;
                let Some(Node::Group(group)) = program.nodes().get(node_index) else {
                    return Err(ReferenceExecuteError::InvalidProgramFilter {
                        program: id,
                        node: *node,
                        filter_index: *filter_index,
                    });
                };
                let Ok(index) = usize::try_from(*filter_index) else {
                    return Err(ReferenceExecuteError::InvalidProgramFilter {
                        program: id,
                        node: *node,
                        filter_index: *filter_index,
                    });
                };
                if group.filters.get(index) != Some(filter)
                    || filters
                        .insert((node.raw(), *filter_index), filter.clone())
                        .is_some()
                {
                    return Err(ReferenceExecuteError::InvalidProgramFilter {
                        program: id,
                        node: *node,
                        filter_index: *filter_index,
                    });
                }
            }
            ProgramPassKind::ApplyMask { node, mode, .. } => {
                let node_index = node.raw() as usize;
                let Some(Node::Group(group)) = program.nodes().get(node_index) else {
                    return Err(ReferenceExecuteError::InvalidProgramMask {
                        program: id,
                        node: *node,
                    });
                };
                if group.mask.as_ref().map(|mask| mask.mode) != Some(*mode)
                    || masks.insert(node.raw(), *mode).is_some()
                {
                    return Err(ReferenceExecuteError::InvalidProgramMask {
                        program: id,
                        node: *node,
                    });
                }
            }
            ProgramPassKind::MotionGlass {
                node,
                program: glass,
                ..
            } => {
                let Some(Node::Group(group)) = program.nodes().get(node.raw() as usize) else {
                    return Err(ReferenceExecuteError::InvalidProgram { program: id });
                };
                if group.glass.as_deref() != Some(glass) {
                    return Err(ReferenceExecuteError::InvalidProgram { program: id });
                }
            }
            ProgramPassKind::ApplyMotionGlassForeground {
                node,
                program: foreground,
                ..
            } => {
                let Some(Node::Group(group)) = program.nodes().get(node.raw() as usize) else {
                    return Err(ReferenceExecuteError::InvalidProgram { program: id });
                };
                if group.glass_foreground.as_deref() != Some(foreground) {
                    return Err(ReferenceExecuteError::InvalidProgram { program: id });
                }
            }
            ProgramPassKind::Clear { .. }
            | ProgramPassKind::ReadDestination { .. }
            | ProgramPassKind::Backdrop { .. }
            | ProgramPassKind::SourceOver { .. }
            | ProgramPassKind::ApplyOpacity { .. }
            | ProgramPassKind::ApplyShader { .. }
            | ProgramPassKind::ApplyTransform { .. }
            | ProgramPassKind::Blend { .. } => {}
        }
    }
    Ok(ReferenceProgramRuntime {
        id,
        viewport: program.viewport(),
        nodes,
        raster_trees,
        clips,
        filters,
        masks,
    })
}

// Resolve transforms and opacity boundaries once; evaluate painter order per pixel.
fn collect_reference_raster_tree(
    program: &DrawProgram,
    roots: &[NodeId],
    local_to_program: Transform2d,
    pass: ExecutionPassId,
) -> Result<Vec<ReferenceTreeStep>, ReferenceExecuteError> {
    use ReferenceTreeStep::*;
    let mut pending = roots
        .iter()
        .rev()
        .map(|node| Draw(*node, local_to_program))
        .collect::<Vec<_>>();
    let mut draws = Vec::new();
    while let Some(step) = pending.pop() {
        let Draw(node, local_to_program) = step else {
            draws.push(step);
            continue;
        };
        match program.nodes().get(node.raw() as usize) {
            Some(Node::Group(group)) if group.is_raster_group() => {
                if group.opacity == 0.0 {
                    continue;
                }
                if group.opacity != 1.0 {
                    draws.push(PushOpacity);
                    pending.push(PopOpacity(group.opacity));
                }
                let child_to_program = group.transform.then(local_to_program);
                pending.extend(
                    group
                        .children
                        .iter()
                        .rev()
                        .map(|child| Draw(*child, child_to_program)),
                );
            }
            Some(Node::Group(_)) | None => {
                return Err(ReferenceExecuteError::UnsupportedPass { pass });
            }
            Some(_) => draws.push(step),
        }
    }
    Ok(draws)
}

#[allow(clippy::too_many_arguments)]
fn prepare_reference_node_operation(
    program: &DrawProgram,
    prepared: &PlanProgram,
    bound: &BoundExternalObjects<'_, ReferenceExternalObject>,
    pass: ExecutionPassId,
    id: ProgramId,
    node: NodeId,
    budget: &mut ReferenceRasterBudget,
) -> Result<ReferenceProgramOperation, ReferenceExecuteError> {
    let node_index = node.raw() as usize;
    let operation = match program.nodes().get(node_index) {
        Some(Node::Path(node)) => {
            if node.stroke.is_some() {
                return Err(ReferenceExecuteError::UnsupportedPass { pass });
            }
            let Some(fill) = node.fill else {
                return Err(ReferenceExecuteError::UnsupportedPass { pass });
            };
            let color = solid_program_paint(program, fill, pass)?;
            let Some(path) = program.paths().get(node.path.raw() as usize) else {
                return Err(ReferenceExecuteError::InvalidProgram { program: id });
            };
            if path
                .verbs
                .iter()
                .any(|verb| matches!(verb, PathVerb::QuadTo | PathVerb::CubicTo))
            {
                return Err(ReferenceExecuteError::UnsupportedPass { pass });
            }
            let segments = line_path_segments(path)
                .ok_or(ReferenceExecuteError::InvalidProgram { program: id })?;
            reserve_outline_segments(pass, segments.len() as u64, &mut budget.outline_segments)?;
            let shapes = line_segments_bounds(&segments)
                .map(|bounds| vec![ReferenceShape { segments, bounds }])
                .unwrap_or_default();
            ReferenceProgramOperation::Fill(ReferenceFill { shapes, color })
        }
        Some(Node::Image(image)) => {
            ReferenceProgramOperation::Image(prepare_image_operation(prepared, bound, id, image)?)
        }
        Some(Node::GlyphRun(run)) => ReferenceProgramOperation::Fill(prepare_glyph_fill(
            program,
            prepared,
            bound,
            pass,
            id,
            run,
            &mut budget.outline_segments,
        )?),
        _ => return Err(ReferenceExecuteError::UnsupportedPass { pass }),
    };
    Ok(operation)
}

fn prepare_reference_clip(
    program: &DrawProgram,
    clip: &Clip,
    pass: ExecutionPassId,
    id: ProgramId,
    outline_segments: &mut u64,
) -> Result<ReferenceClip, ReferenceExecuteError> {
    match clip {
        Clip::Rect(rect) => Ok(ReferenceClip::Rect(*rect)),
        Clip::RoundRect(_) => return Err(ReferenceExecuteError::UnsupportedPass { pass }),
        Clip::Path { path, .. } => {
            let Some(path) = program.paths().get(path.raw() as usize) else {
                return Err(ReferenceExecuteError::InvalidProgram { program: id });
            };
            if path
                .verbs
                .iter()
                .any(|verb| matches!(verb, PathVerb::QuadTo | PathVerb::CubicTo))
            {
                return Err(ReferenceExecuteError::UnsupportedPass { pass });
            }
            let segments = line_path_segments(path)
                .ok_or(ReferenceExecuteError::InvalidProgram { program: id })?;
            reserve_outline_segments(pass, segments.len() as u64, outline_segments)?;
            Ok(ReferenceClip::Path(
                line_segments_bounds(&segments).map(|bounds| ReferenceShape { segments, bounds }),
            ))
        }
    }
}

fn prepare_image_operation(
    prepared: &crate::compositor::lower::PlanProgram,
    bound: &BoundExternalObjects<'_, ReferenceExternalObject>,
    program: ProgramId,
    image: &valle_draw::program::ImageNode,
) -> Result<ReferenceImageOperation, ReferenceExecuteError> {
    let binding_index = prepared
        .requirements
        .external_textures
        .iter()
        .position(|required| required == &image.texture)
        .ok_or_else(|| ReferenceExecuteError::InvalidProgramTexture {
            program,
            key: image.texture.key.clone(),
        })?;
    let binding = prepared
        .resources
        .textures
        .get(binding_index)
        .filter(|binding| binding.key == image.texture.key)
        .ok_or_else(|| ReferenceExecuteError::InvalidProgramTexture {
            program,
            key: image.texture.key.clone(),
        })?;
    let object =
        bound
            .get(binding.slot)
            .ok_or_else(|| ReferenceExecuteError::InvalidProgramTexture {
                program,
                key: image.texture.key.clone(),
            })?;
    if object.visual_image().is_none() {
        return Err(ReferenceExecuteError::InvalidProgramTexture {
            program,
            key: image.texture.key.clone(),
        });
    }
    let ResourceInterpretation::Visual { interpretation } = &object.key().interpretation else {
        return Err(ReferenceExecuteError::InvalidProgramTexture {
            program,
            key: image.texture.key.clone(),
        });
    };
    let sample = ExternalSample::from_display_rect(image.src, *interpretation).map_err(|_| {
        ReferenceExecuteError::InvalidProgramTexture {
            program,
            key: image.texture.key.clone(),
        }
    })?;
    Ok(ReferenceImageOperation {
        slot: binding.slot,
        sample,
        dst: image.dst,
        sampling: image.sampling,
        opacity: image.opacity,
    })
}

fn solid_program_paint(
    program: &DrawProgram,
    paint: valle_draw::program::PaintId,
    pass: ExecutionPassId,
) -> Result<PremulRgba32, ReferenceExecuteError> {
    let Some(Paint::Solid(color)) = program.paints().get(paint.raw() as usize) else {
        return Err(ReferenceExecuteError::UnsupportedPass { pass });
    };
    PremulRgba32::from_premultiplied([color.red, color.green, color.blue, color.alpha])
        .map_err(Into::into)
}

fn prepare_glyph_fill(
    program: &DrawProgram,
    prepared: &crate::compositor::lower::PlanProgram,
    bound: &BoundExternalObjects<'_, ReferenceExternalObject>,
    pass: ExecutionPassId,
    program_id: ProgramId,
    run: &GlyphRun,
    outline_segments: &mut u64,
) -> Result<ReferenceFill, ReferenceExecuteError> {
    let color = solid_program_paint(program, run.paint, pass)?;
    let binding = prepared
        .resources
        .fonts
        .iter()
        .find(|binding| {
            crate::resource::ContentDigest::from_bytes(run.font.face_hash.into_bytes())
                == binding.face_hash
                && binding.face_index == run.font.face_index
        })
        .ok_or(ReferenceExecuteError::InvalidFontResource {
            program: program_id,
            face_index: run.font.face_index,
        })?;
    let bytes = bound
        .get(binding.slot)
        .and_then(ReferenceExternalObject::font_data)
        .ok_or(ReferenceExecuteError::InvalidFontResource {
            program: program_id,
            face_index: run.font.face_index,
        })?;
    let face = Face::parse(bytes, run.font.face_index).map_err(|_| {
        ReferenceExecuteError::InvalidFontFace {
            program: program_id,
            face_index: run.font.face_index,
        }
    })?;
    let scale = f64::from(run.font_size) / f64::from(face.units_per_em());
    let tolerance = f64::from(run.font_size) * GLYPH_CURVE_TOLERANCE_PER_EM;
    let mut shapes = Vec::with_capacity(run.glyphs.len());
    let mut actual_bounds = None;
    for glyph in &run.glyphs {
        let glyph_index =
            u16::try_from(glyph.id).map_err(|_| ReferenceExecuteError::InvalidGlyph {
                program: program_id,
                glyph: glyph.id,
            })?;
        if glyph_index >= face.number_of_glyphs() {
            return Err(ReferenceExecuteError::InvalidGlyph {
                program: program_id,
                glyph: glyph.id,
            });
        }
        let glyph_id = GlyphId(glyph_index);
        if face.is_color_glyph(glyph_id)
            || face.glyph_svg_image(glyph_id).is_some()
            || face.glyph_raster_image(glyph_id, u16::MAX).is_some()
        {
            return Err(ReferenceExecuteError::UnsupportedColorGlyph {
                program: program_id,
                glyph: glyph.id,
            });
        }
        let remaining = MAX_REFERENCE_OUTLINE_SEGMENTS.saturating_sub(*outline_segments);
        let mut builder =
            GlyphOutlineBuilder::new([glyph.x, glyph.y], scale, tolerance, remaining as usize);
        let font_bounds = face.outline_glyph(glyph_id, &mut builder);
        let segments = builder.finish().map_err(|error| match error {
            GlyphOutlineError::SegmentBudget => {
                ReferenceExecuteError::OutlineSegmentBudgetExceeded {
                    pass,
                    required: MAX_REFERENCE_OUTLINE_SEGMENTS.saturating_add(1),
                    maximum: MAX_REFERENCE_OUTLINE_SEGMENTS,
                }
            }
            GlyphOutlineError::Invalid | GlyphOutlineError::SubdivisionLimit => {
                ReferenceExecuteError::InvalidGlyphOutline {
                    program: program_id,
                    glyph: glyph.id,
                }
            }
        })?;
        reserve_outline_segments(pass, segments.len() as u64, outline_segments)?;
        if let Some(font_bounds) = font_bounds {
            if segments.is_empty() {
                return Err(ReferenceExecuteError::InvalidGlyphOutline {
                    program: program_id,
                    glyph: glyph.id,
                });
            }
            let bounds = glyph_local_bounds([glyph.x, glyph.y], scale, font_bounds);
            actual_bounds = Some(match actual_bounds {
                Some(current) => union_local_rect(current, bounds),
                None => bounds,
            });
            shapes.push(ReferenceShape { segments, bounds });
        } else if !segments.is_empty() {
            return Err(ReferenceExecuteError::InvalidGlyphOutline {
                program: program_id,
                glyph: glyph.id,
            });
        }
    }
    if let Some(actual) = actual_bounds {
        let epsilon = f64::from(run.font_size) * 1.0e-6;
        if actual.left() < run.bounds.left() - epsilon
            || actual.top() < run.bounds.top() - epsilon
            || actual.right() > run.bounds.right() + epsilon
            || actual.bottom() > run.bounds.bottom() + epsilon
        {
            return Err(ReferenceExecuteError::GlyphBoundsMismatch {
                program: program_id,
            });
        }
    }
    Ok(ReferenceFill { shapes, color })
}

fn reserve_outline_segments(
    pass: ExecutionPassId,
    additional: u64,
    outline_segments: &mut u64,
) -> Result<(), ReferenceExecuteError> {
    let required = outline_segments.checked_add(additional).ok_or(
        ReferenceExecuteError::OutlineSegmentBudgetExceeded {
            pass,
            required: u64::MAX,
            maximum: MAX_REFERENCE_OUTLINE_SEGMENTS,
        },
    )?;
    if required > MAX_REFERENCE_OUTLINE_SEGMENTS {
        return Err(ReferenceExecuteError::OutlineSegmentBudgetExceeded {
            pass,
            required,
            maximum: MAX_REFERENCE_OUTLINE_SEGMENTS,
        });
    }
    *outline_segments = required;
    Ok(())
}

fn glyph_local_bounds(origin: [f64; 2], scale: f64, bounds: ttf_parser::Rect) -> valle_draw::Rect {
    let left = origin[0] + f64::from(bounds.x_min) * scale;
    let right = origin[0] + f64::from(bounds.x_max) * scale;
    let top = origin[1] - f64::from(bounds.y_max) * scale;
    let bottom = origin[1] - f64::from(bounds.y_min) * scale;
    valle_draw::Rect::from_edges(left, top, right, bottom)
}

fn union_local_rect(left: valle_draw::Rect, right: valle_draw::Rect) -> valle_draw::Rect {
    valle_draw::Rect::from_edges(
        left.left().min(right.left()),
        left.top().min(right.top()),
        left.right().max(right.right()),
        left.bottom().max(right.bottom()),
    )
}

fn line_segments_bounds(segments: &[LineSegment]) -> Option<valle_draw::Rect> {
    let mut points = segments
        .iter()
        .flat_map(|segment| [segment.from, segment.to]);
    let first = points.next()?;
    let mut left = first[0];
    let mut top = first[1];
    let mut right = first[0];
    let mut bottom = first[1];
    for point in points {
        left = left.min(point[0]);
        top = top.min(point[1]);
        right = right.max(point[0]);
        bottom = bottom.max(point[1]);
    }
    (right > left && bottom > top).then(|| valle_draw::Rect::from_edges(left, top, right, bottom))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlyphOutlineError {
    Invalid,
    SegmentBudget,
    SubdivisionLimit,
}

struct GlyphOutlineBuilder {
    origin: [f64; 2],
    scale: f64,
    tolerance_squared: f64,
    maximum_segments: usize,
    segments: Vec<LineSegment>,
    first: Option<[f64; 2]>,
    current: Option<[f64; 2]>,
    error: Option<GlyphOutlineError>,
}

impl GlyphOutlineBuilder {
    fn new(origin: [f64; 2], scale: f64, tolerance: f64, maximum_segments: usize) -> Self {
        Self {
            origin,
            scale,
            tolerance_squared: tolerance * tolerance,
            maximum_segments,
            segments: Vec::new(),
            first: None,
            current: None,
            error: None,
        }
    }

    fn local_point(&self, x: f32, y: f32) -> Option<[f64; 2]> {
        let point = [
            self.origin[0] + f64::from(x) * self.scale,
            self.origin[1] - f64::from(y) * self.scale,
        ];
        point.iter().all(|value| value.is_finite()).then_some(point)
    }

    fn close_contour(&mut self) {
        if self.error.is_some() {
            return;
        }
        if let (Some(from), Some(to)) = (self.current, self.first) {
            self.push_segment(from, to);
        }
        self.first = None;
        self.current = None;
    }

    fn push_segment(&mut self, from: [f64; 2], to: [f64; 2]) {
        if self.error.is_some() || from == to {
            return;
        }
        if self.segments.len() == self.maximum_segments {
            self.error = Some(GlyphOutlineError::SegmentBudget);
            return;
        }
        self.segments.push(LineSegment { from, to });
    }

    fn line_to_local(&mut self, to: [f64; 2]) {
        let Some(from) = self.current else {
            self.error = Some(GlyphOutlineError::Invalid);
            return;
        };
        self.push_segment(from, to);
        self.current = Some(to);
    }

    fn flatten_quad(&mut self, from: [f64; 2], control: [f64; 2], to: [f64; 2], depth: u8) {
        if self.error.is_some() {
            return;
        }
        if point_within_segment_tolerance(control, from, to, self.tolerance_squared) {
            self.push_segment(from, to);
            return;
        }
        if depth == MAX_GLYPH_CURVE_SUBDIVISION_DEPTH {
            self.error = Some(GlyphOutlineError::SubdivisionLimit);
            return;
        }
        let first = midpoint(from, control);
        let second = midpoint(control, to);
        let middle = midpoint(first, second);
        self.flatten_quad(from, first, middle, depth + 1);
        self.flatten_quad(middle, second, to, depth + 1);
    }

    fn flatten_cubic(
        &mut self,
        from: [f64; 2],
        first_control: [f64; 2],
        second_control: [f64; 2],
        to: [f64; 2],
        depth: u8,
    ) {
        if self.error.is_some() {
            return;
        }
        if point_within_segment_tolerance(first_control, from, to, self.tolerance_squared)
            && point_within_segment_tolerance(second_control, from, to, self.tolerance_squared)
        {
            self.push_segment(from, to);
            return;
        }
        if depth == MAX_GLYPH_CURVE_SUBDIVISION_DEPTH {
            self.error = Some(GlyphOutlineError::SubdivisionLimit);
            return;
        }
        let first = midpoint(from, first_control);
        let middle_controls = midpoint(first_control, second_control);
        let last = midpoint(second_control, to);
        let first_middle = midpoint(first, middle_controls);
        let second_middle = midpoint(middle_controls, last);
        let middle = midpoint(first_middle, second_middle);
        self.flatten_cubic(from, first, first_middle, middle, depth + 1);
        self.flatten_cubic(middle, second_middle, last, to, depth + 1);
    }

    fn finish(mut self) -> Result<Vec<LineSegment>, GlyphOutlineError> {
        self.close_contour();
        match self.error {
            Some(error) => Err(error),
            None => Ok(self.segments),
        }
    }
}

impl OutlineBuilder for GlyphOutlineBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close_contour();
        let Some(point) = self.local_point(x, y) else {
            self.error = Some(GlyphOutlineError::Invalid);
            return;
        };
        self.first = Some(point);
        self.current = Some(point);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let Some(point) = self.local_point(x, y) else {
            self.error = Some(GlyphOutlineError::Invalid);
            return;
        };
        self.line_to_local(point);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (Some(from), Some(control), Some(to)) = (
            self.current,
            self.local_point(x1, y1),
            self.local_point(x, y),
        ) else {
            self.error = Some(GlyphOutlineError::Invalid);
            return;
        };
        self.flatten_quad(from, control, to, 0);
        self.current = Some(to);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (Some(from), Some(first_control), Some(second_control), Some(to)) = (
            self.current,
            self.local_point(x1, y1),
            self.local_point(x2, y2),
            self.local_point(x, y),
        ) else {
            self.error = Some(GlyphOutlineError::Invalid);
            return;
        };
        self.flatten_cubic(from, first_control, second_control, to, 0);
        self.current = Some(to);
    }

    fn close(&mut self) {
        self.close_contour();
    }
}

fn midpoint(left: [f64; 2], right: [f64; 2]) -> [f64; 2] {
    [(left[0] + right[0]) * 0.5, (left[1] + right[1]) * 0.5]
}

fn point_within_segment_tolerance(
    point: [f64; 2],
    from: [f64; 2],
    to: [f64; 2],
    tolerance_squared: f64,
) -> bool {
    let line = [to[0] - from[0], to[1] - from[1]];
    let length_squared = line[0] * line[0] + line[1] * line[1];
    if length_squared == 0.0 {
        let delta = [point[0] - from[0], point[1] - from[1]];
        return delta[0] * delta[0] + delta[1] * delta[1] <= tolerance_squared;
    }
    let relative = [point[0] - from[0], point[1] - from[1]];
    let projection = (relative[0] * line[0] + relative[1] * line[1]) / length_squared;
    if projection <= 0.0 {
        return relative[0] * relative[0] + relative[1] * relative[1] <= tolerance_squared;
    }
    if projection >= 1.0 {
        let delta = [point[0] - to[0], point[1] - to[1]];
        return delta[0] * delta[0] + delta[1] * delta[1] <= tolerance_squared;
    }
    let cross = line[0] * (point[1] - from[1]) - line[1] * (point[0] - from[0]);
    cross * cross <= tolerance_squared * length_squared
}

fn line_path_segments(path: &PathData) -> Option<Vec<LineSegment>> {
    let mut segments = Vec::new();
    let mut point_index = 0_usize;
    let mut first = None;
    let mut current = None;
    let close = |segments: &mut Vec<LineSegment>,
                 first: &mut Option<[f64; 2]>,
                 current: &mut Option<[f64; 2]>| {
        if let (Some(from), Some(to)) = (*current, *first) {
            segments.push(LineSegment { from, to });
        }
        *first = None;
        *current = None;
    };
    for verb in &path.verbs {
        match verb {
            PathVerb::MoveTo => {
                close(&mut segments, &mut first, &mut current);
                let point = *path.points.get(point_index)?;
                point_index += 1;
                first = Some(point);
                current = Some(point);
            }
            PathVerb::LineTo => {
                let from = current?;
                let to = *path.points.get(point_index)?;
                point_index += 1;
                segments.push(LineSegment { from, to });
                current = Some(to);
            }
            PathVerb::Close => close(&mut segments, &mut first, &mut current),
            PathVerb::QuadTo | PathVerb::CubicTo => return None,
        }
    }
    close(&mut segments, &mut first, &mut current);
    (point_index == path.points.len()).then_some(segments)
}

enum ReferenceRasterOperation<'program, 'object> {
    Fill(&'program ReferenceFill),
    Image {
        operation: &'program ReferenceImageOperation,
        source: &'object ReferenceImage,
    },
}

impl ReferenceRasterOperation<'_, '_> {
    fn pixel(
        &self,
        viewport: valle_draw::Rect,
        inverse: [f64; 9],
        x: u32,
        y: u32,
    ) -> Result<PremulRgba32, ReferenceExecuteError> {
        match self {
            Self::Fill(fill) => Ok(fill
                .color
                .scale_coverage(line_path_coverage(fill, viewport, inverse, x, y)?)?),
            Self::Image { operation, source } => {
                raster_program_image_pixel(operation, source, viewport, inverse, x, y)
            }
        }
    }
}

#[derive(Debug)]
struct ReferenceProgramSurface {
    extent: Extent2d,
    pixels: Vec<PremulRgba32>,
}

impl ReferenceProgramSurface {
    fn transparent(extent: Extent2d) -> Result<Self, ReferenceExecuteError> {
        let len = (extent.width() as usize)
            .checked_mul(extent.height() as usize)
            .ok_or(PixelError::ImageTooLarge)?;
        Ok(Self {
            extent,
            pixels: vec![PremulRgba32::TRANSPARENT; len],
        })
    }

    fn pixel(&self, x: u32, y: u32) -> Option<PremulRgba32> {
        if x >= self.extent.width() || y >= self.extent.height() {
            return None;
        }
        self.pixels
            .get(y as usize * self.extent.width() as usize + x as usize)
            .copied()
    }

    fn write(&mut self, x: u32, y: u32, pixel: PremulRgba32) -> bool {
        if x >= self.extent.width() || y >= self.extent.height() {
            return false;
        }
        let index = y as usize * self.extent.width() as usize + x as usize;
        let Some(destination) = self.pixels.get_mut(index) else {
            return false;
        };
        *destination = pixel;
        true
    }

    fn into_image(self) -> Result<ReferenceImage, ReferenceExecuteError> {
        ReferenceImage::new(self.extent, self.pixels).map_err(Into::into)
    }
}

struct ReferenceProgramExecution<'plan, 'surface> {
    program: ProgramId,
    pass: ExecutionPassId,
    prepared: &'plan PlanProgram,
    bound: &'plan BoundProgramSchedule,
    destinations: Vec<&'surface ReferenceImage>,
    slots: Vec<Option<ReferenceProgramSurface>>,
    output: ReferenceProgramSurface,
}

impl<'plan, 'surface> ReferenceProgramExecution<'plan, 'surface> {
    fn new(
        prepared: &'plan PlanProgram,
        bound: &'plan BoundProgramSchedule,
        destination_inputs: &[PlanResourceId],
        outer_surfaces: &'surface BTreeMap<PlanResourceId, ReferenceImage>,
        extent: Extent2d,
        pass: ExecutionPassId,
    ) -> Result<Self, ReferenceExecuteError> {
        if bound.program() != prepared.id
            || bound.execution_pass() != pass
            || bound.surface_slots().len() != prepared.local_schedule().surface_slots().len()
            || destination_inputs.len() != prepared.destination_uses.len()
        {
            return Err(ReferenceExecuteError::InvalidProgramSchedule {
                program: prepared.id,
                pass,
            });
        }
        let destinations = destination_inputs
            .iter()
            .map(|resource| surface(outer_surfaces, *resource))
            .collect::<Result<Vec<_>, _>>()?;
        let mut slots = Vec::with_capacity(bound.surface_slots().len());
        for (static_slot, dynamic_slot) in prepared
            .local_schedule()
            .surface_slots()
            .iter()
            .zip(bound.surface_slots())
        {
            if static_slot.id != dynamic_slot.id() {
                return Err(ReferenceExecuteError::InvalidProgramSchedule {
                    program: prepared.id,
                    pass,
                });
            }
            slots.push(
                dynamic_slot
                    .extent()
                    .map(ReferenceProgramSurface::transparent)
                    .transpose()?,
            );
        }
        Ok(Self {
            program: prepared.id,
            pass,
            prepared,
            bound,
            destinations,
            slots,
            output: ReferenceProgramSurface::transparent(extent)?,
        })
    }

    fn invalid_schedule(&self) -> ReferenceExecuteError {
        ReferenceExecuteError::InvalidProgramSchedule {
            program: self.program,
            pass: self.pass,
        }
    }

    fn resource_roi(
        &self,
        resource: ProgramResourceId,
    ) -> Result<DeviceRect, ReferenceExecuteError> {
        self.bound
            .resources()
            .get(resource.index())
            .filter(|binding| binding.resource() == resource)
            .map(|binding| binding.device_roi())
            .ok_or_else(|| self.invalid_schedule())
    }

    fn storage(
        &self,
        resource: ProgramResourceId,
    ) -> Result<ProgramStorageKind, ReferenceExecuteError> {
        self.prepared
            .local_schedule()
            .storage(resource)
            .ok_or_else(|| self.invalid_schedule())
    }

    fn destination_pixel(
        &self,
        destination: ProgramDestinationId,
        x: u32,
        y: u32,
    ) -> Result<PremulRgba32, ReferenceExecuteError> {
        self.destinations
            .get(destination.index())
            .and_then(|image| image.pixel(x, y))
            .ok_or_else(|| self.invalid_schedule())
    }

    fn read_pixel(
        &self,
        resource: ProgramResourceId,
        x: u32,
        y: u32,
    ) -> Result<PremulRgba32, ReferenceExecuteError> {
        let mut current = resource;
        for _ in 0..=self.prepared.local_plan().resources().len() {
            let roi = self.resource_roi(current)?;
            if !device_rect_contains_pixel(roi, x, y) {
                return Ok(PremulRgba32::TRANSPARENT);
            }
            match self.storage(current)? {
                ProgramStorageKind::Transparent {} => return Ok(PremulRgba32::TRANSPARENT),
                ProgramStorageKind::Destination { destination } => {
                    return self.destination_pixel(destination, x, y);
                }
                ProgramStorageKind::Alias { source } => current = source,
                ProgramStorageKind::Output {} => {
                    return self
                        .output
                        .pixel(x, y)
                        .ok_or_else(|| self.invalid_schedule());
                }
                ProgramStorageKind::Surface { slot } => {
                    let local_x = x
                        .checked_sub(u32::try_from(roi.x).map_err(|_| self.invalid_schedule())?)
                        .ok_or_else(|| self.invalid_schedule())?;
                    let local_y = y
                        .checked_sub(u32::try_from(roi.y).map_err(|_| self.invalid_schedule())?)
                        .ok_or_else(|| self.invalid_schedule())?;
                    return self
                        .slots
                        .get(slot.index())
                        .and_then(Option::as_ref)
                        .and_then(|surface| surface.pixel(local_x, local_y))
                        .ok_or_else(|| self.invalid_schedule());
                }
            }
        }
        Err(self.invalid_schedule())
    }

    fn read_texel(
        &self,
        resource: ProgramResourceId,
        x: i64,
        y: i64,
    ) -> Result<PremulRgba32, ReferenceExecuteError> {
        if x < 0
            || y < 0
            || x >= i64::from(self.output.extent.width())
            || y >= i64::from(self.output.extent.height())
        {
            return Ok(PremulRgba32::TRANSPARENT);
        }
        self.read_pixel(
            resource,
            u32::try_from(x).map_err(|_| self.invalid_schedule())?,
            u32::try_from(y).map_err(|_| self.invalid_schedule())?,
        )
    }

    fn sample_bilinear(
        &self,
        resource: ProgramResourceId,
        position: [f64; 2],
    ) -> Result<PremulRgba32, ReferenceExecuteError> {
        if !position.iter().all(|value| value.is_finite()) {
            return Err(ReferenceExecuteError::InvalidSampleCoordinate);
        }
        let coordinate = [position[0] - 0.5, position[1] - 0.5];
        let floor = [coordinate[0].floor(), coordinate[1].floor()];
        if floor
            .iter()
            .any(|value| *value < i64::MIN as f64 || *value >= i64::MAX as f64)
        {
            return Err(ReferenceExecuteError::InvalidSampleCoordinate);
        }
        let first = [floor[0] as i64, floor[1] as i64];
        let second = [
            first[0]
                .checked_add(1)
                .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)?,
            first[1]
                .checked_add(1)
                .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)?,
        ];
        let amount = [coordinate[0] - floor[0], coordinate[1] - floor[1]];
        let samples = [
            self.read_texel(resource, first[0], first[1])?,
            self.read_texel(resource, second[0], first[1])?,
            self.read_texel(resource, first[0], second[1])?,
            self.read_texel(resource, second[0], second[1])?,
        ];
        let top = lerp_channels(samples[0].channels(), samples[1].channels(), amount[0]);
        let bottom = lerp_channels(samples[2].channels(), samples[3].channels(), amount[0]);
        PremulRgba32::from_premultiplied(lerp_channels(top, bottom, amount[1])).map_err(Into::into)
    }

    fn write_pixel(
        &mut self,
        resource: ProgramResourceId,
        roi: DeviceRect,
        x: u32,
        y: u32,
        pixel: PremulRgba32,
    ) -> Result<(), ReferenceExecuteError> {
        let storage = self.storage(resource)?;
        let invalid = || ReferenceExecuteError::InvalidProgramSchedule {
            program: self.program,
            pass: self.pass,
        };
        let written = match storage {
            ProgramStorageKind::Output {} => self.output.write(x, y, pixel),
            ProgramStorageKind::Surface { slot } => {
                let local_x = x
                    .checked_sub(u32::try_from(roi.x).map_err(|_| invalid())?)
                    .ok_or_else(invalid)?;
                let local_y = y
                    .checked_sub(u32::try_from(roi.y).map_err(|_| invalid())?)
                    .ok_or_else(invalid)?;
                self.slots
                    .get_mut(slot.index())
                    .and_then(Option::as_mut)
                    .is_some_and(|surface| surface.write(local_x, local_y, pixel))
            }
            ProgramStorageKind::Transparent {}
            | ProgramStorageKind::Destination { .. }
            | ProgramStorageKind::Alias { .. } => false,
        };
        written.then_some(()).ok_or_else(invalid)
    }

    fn write_resource(
        &mut self,
        output: ProgramResourceId,
        mut operation: impl FnMut(&Self, u32, u32) -> Result<PremulRgba32, ReferenceExecuteError>,
    ) -> Result<(), ReferenceExecuteError> {
        if !matches!(
            self.storage(output)?,
            ProgramStorageKind::Output {} | ProgramStorageKind::Surface { .. }
        ) {
            return Ok(());
        }
        let roi = self.resource_roi(output)?;
        if roi.is_empty() {
            return Ok(());
        }
        let start_x = u32::try_from(roi.x).map_err(|_| self.invalid_schedule())?;
        let start_y = u32::try_from(roi.y).map_err(|_| self.invalid_schedule())?;
        let end_x = start_x
            .checked_add(roi.width)
            .ok_or_else(|| self.invalid_schedule())?;
        let end_y = start_y
            .checked_add(roi.height)
            .ok_or_else(|| self.invalid_schedule())?;
        for y in start_y..end_y {
            for x in start_x..end_x {
                let pixel = operation(self, x, y)?;
                self.write_pixel(output, roi, x, y, pixel)?;
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<ReferenceImage, ReferenceExecuteError> {
        self.output.into_image()
    }
}

#[allow(clippy::too_many_arguments)]
fn raster_program<'program, 'object>(
    program: &'program ReferenceProgramRuntime,
    prepared: &PlanProgram,
    schedule: &BoundProgramSchedule,
    bound: &BoundExternalObjects<'object, ReferenceExternalObject>,
    bindings: &RenderBindings,
    transform_binding: DynamicBindingId,
    destination_inputs: &[PlanResourceId],
    outer_surfaces: &BTreeMap<PlanResourceId, ReferenceImage>,
    extent: Extent2d,
    pass: ExecutionPassId,
) -> Result<ReferenceImage, ReferenceExecuteError> {
    let transform = dynamic_transform(bindings, transform_binding)?;
    let mut execution = ReferenceProgramExecution::new(
        prepared,
        schedule,
        destination_inputs,
        outer_surfaces,
        extent,
        pass,
    )?;
    for local_pass in prepared.local_plan().passes() {
        match &local_pass.kind {
            ProgramPassKind::Clear { .. } => {}
            ProgramPassKind::RasterNode { node, output } => {
                let (_, inverse) = program_resource_normalized_matrices(
                    prepared,
                    *output,
                    program.viewport,
                    transform.matrix(),
                )?;
                let operation = match program.node(*node)? {
                    ReferenceProgramOperation::Fill(fill) => ReferenceRasterOperation::Fill(fill),
                    ReferenceProgramOperation::Image(image) => {
                        let source = bound
                            .get(image.slot)
                            .and_then(ReferenceExternalObject::visual_image)
                            .ok_or(ReferenceExecuteError::InvalidProgramTextureSlot {
                                program: program.id,
                                slot: image.slot,
                            })?;
                        ReferenceRasterOperation::Image {
                            operation: image,
                            source,
                        }
                    }
                };
                execution.write_resource(*output, |_, x, y| {
                    operation.pixel(program.viewport, inverse, x, y)
                })?;
            }
            ProgramPassKind::RasterTree { output, .. } => {
                let operations = program
                    .raster_tree(*output)?
                    .iter()
                    .filter_map(ReferenceTreeStep::draw)
                    .map(|(node, local_to_program)| {
                        let local_to_device = program_transform_to_device(
                            prepared,
                            local_to_program,
                            program.viewport,
                            transform.matrix(),
                        )?;
                        let (_, inverse) =
                            normalized_program_matrices(local_to_device, program.viewport)?;
                        let operation = match program.node(node)? {
                            ReferenceProgramOperation::Fill(fill) => {
                                ReferenceRasterOperation::Fill(fill)
                            }
                            ReferenceProgramOperation::Image(image) => {
                                let source = bound
                                    .get(image.slot)
                                    .and_then(ReferenceExternalObject::visual_image)
                                    .ok_or(ReferenceExecuteError::InvalidProgramTextureSlot {
                                        program: program.id,
                                        slot: image.slot,
                                    })?;
                                ReferenceRasterOperation::Image {
                                    operation: image,
                                    source,
                                }
                            }
                        };
                        Ok((operation, inverse))
                    })
                    .collect::<Result<Vec<_>, ReferenceExecuteError>>()?;
                let steps = program.raster_tree(*output)?;
                let mut stack = Vec::new();
                execution.write_resource(*output, |_, x, y| {
                    let mut pixel = PremulRgba32::TRANSPARENT;
                    let mut leaves = operations.iter();
                    for step in steps {
                        match *step {
                            ReferenceTreeStep::Draw(..) => {
                                let (operation, inverse) = leaves.next().expect("admitted leaf");
                                pixel = operation
                                    .pixel(program.viewport, *inverse, x, y)?
                                    .source_over(pixel)?;
                            }
                            ReferenceTreeStep::PushOpacity => {
                                stack.push(pixel);
                                pixel = PremulRgba32::TRANSPARENT;
                            }
                            ReferenceTreeStep::PopOpacity(opacity) => {
                                pixel = pixel
                                    .scale_coverage(opacity)?
                                    .source_over(stack.pop().expect("balanced opacity group"))?;
                            }
                        }
                    }
                    Ok(pixel)
                })?;
            }
            ProgramPassKind::ReadDestination {
                external,
                local_inputs,
                output,
                ..
            } => execution.write_resource(*output, |execution, x, y| {
                let mut destination = match external {
                    Some(destination) => execution.destination_pixel(*destination, x, y)?,
                    None => PremulRgba32::TRANSPARENT,
                };
                for input in local_inputs {
                    destination = execution
                        .read_pixel(*input, x, y)?
                        .source_over(destination)?;
                }
                Ok(destination)
            })?,
            ProgramPassKind::Backdrop {
                input,
                output,
                bounds,
                sampling,
                ..
            } => {
                let (_, inverse) = program_resource_normalized_matrices(
                    prepared,
                    *output,
                    program.viewport,
                    transform.matrix(),
                )?;
                // The generic pass is an identity capture, not a blur/refraction kernel. Both
                // sampling modes therefore address the same device-space destination pixel; the
                // mode remains part of the immutable view contract for typed spatial consumers.
                match sampling {
                    SamplingMode::LinearClamp | SamplingMode::LinearDecal => {}
                    SamplingMode::NearestClamp | SamplingMode::CubicClamp => {
                        return Err(ReferenceExecuteError::UnsupportedPass { pass });
                    }
                }
                execution.write_resource(*output, |execution, x, y| {
                    let coverage = program_rect_coverage(*bounds, program.viewport, inverse, x, y)?;
                    if coverage == 0.0 {
                        return Ok(PremulRgba32::TRANSPARENT);
                    }
                    Ok(execution
                        .read_pixel(*input, x, y)?
                        .scale_coverage(coverage)?)
                })?;
            }
            ProgramPassKind::MotionGlass {
                input,
                output,
                program: glass,
                ..
            } => {
                let owner_to_device = program_resource_to_device(
                    prepared,
                    *output,
                    program.viewport,
                    transform.matrix(),
                )?;
                let mut pixels =
                    Vec::with_capacity(extent.width() as usize * extent.height() as usize);
                for y in 0..extent.height() {
                    for x in 0..extent.width() {
                        pixels.push(execution.read_pixel(*input, x, y)?);
                    }
                }
                let backdrop = ReferenceImage::new(extent, pixels)?;
                let contribution = match glass.owner_kind {
                    valle_draw::program::glass::GlassOwnerKind::Independent => {
                        crate::compositor::glass::render_glass_contribution_transformed(
                            glass,
                            owner_to_device,
                            &backdrop,
                        )
                    }
                    valle_draw::program::glass::GlassOwnerKind::Field => {
                        crate::compositor::glass::render_field_contribution_transformed(
                            glass,
                            owner_to_device,
                            &backdrop,
                        )
                    }
                }
                .map_err(|_| ReferenceExecuteError::InvalidProgram {
                    program: program.id,
                })?;
                execution.write_resource(*output, |execution, x, y| {
                    contribution
                        .pixel(x, y)
                        .ok_or_else(|| execution.invalid_schedule())
                })?;
            }
            ProgramPassKind::ApplyMotionGlassForeground {
                input,
                output,
                owner_to_program,
                program: foreground,
                ..
            } => {
                let owner_to_device = program_transform_to_device(
                    prepared,
                    *owner_to_program,
                    program.viewport,
                    transform.matrix(),
                )?;
                execution.write_resource(*output, |execution, x, y| {
                    let coverage =
                        program_glass_foreground_coverage(foreground, owner_to_device, x, y)?;
                    if coverage == 0.0 {
                        return Ok(PremulRgba32::TRANSPARENT);
                    }
                    execution
                        .read_pixel(*input, x, y)?
                        .scale_coverage(coverage)
                        .map_err(Into::into)
                })?;
            }
            ProgramPassKind::ApplyClip {
                node,
                input,
                output,
                ..
            } => {
                let clip = program.clip(*node)?;
                let (_, inverse) = program_resource_normalized_matrices(
                    prepared,
                    *output,
                    program.viewport,
                    transform.matrix(),
                )?;
                execution.write_resource(*output, |execution, x, y| {
                    let coverage = program_clip_coverage(clip, program.viewport, inverse, x, y)?;
                    if coverage == 0.0 {
                        return Ok(PremulRgba32::TRANSPARENT);
                    }
                    Ok(execution
                        .read_pixel(*input, x, y)?
                        .scale_coverage(coverage)?)
                })?;
            }
            ProgramPassKind::ApplyFilter {
                node,
                filter_index,
                input,
                output,
                ..
            } => match program.filter(*node, *filter_index)? {
                Filter::ColorMatrix { matrix } => {
                    execution.write_resource(*output, |execution, x, y| {
                        color_matrix_pixel(execution.read_pixel(*input, x, y)?, matrix)
                            .map_err(Into::into)
                    })?;
                }
                Filter::Blur { sigma_x, sigma_y } => {
                    let (local_to_device, inverse) = program_resource_normalized_matrices(
                        prepared,
                        *output,
                        program.viewport,
                        transform.matrix(),
                    )?;
                    let support = f64::from(FILTER_GAUSSIAN_SUPPORT_SIGMAS);
                    let horizontal =
                        GaussianKernel::new_with_support(pass, f64::from(*sigma_x), support)?;
                    let vertical =
                        GaussianKernel::new_with_support(pass, f64::from(*sigma_y), support)?;
                    execution.write_resource(*output, |execution, x, y| {
                        program_gaussian_blur_pixel(
                            execution,
                            *input,
                            program.viewport,
                            local_to_device,
                            inverse,
                            x,
                            y,
                            [0.0, 0.0],
                            &horizontal,
                            &vertical,
                        )
                    })?;
                }
                Filter::DropShadow {
                    offset,
                    sigma_x,
                    sigma_y,
                    color,
                } => {
                    let (local_to_device, inverse) = program_resource_normalized_matrices(
                        prepared,
                        *output,
                        program.viewport,
                        transform.matrix(),
                    )?;
                    let support = f64::from(FILTER_GAUSSIAN_SUPPORT_SIGMAS);
                    let horizontal =
                        GaussianKernel::new_with_support(pass, f64::from(*sigma_x), support)?;
                    let vertical =
                        GaussianKernel::new_with_support(pass, f64::from(*sigma_y), support)?;
                    execution.write_resource(*output, |execution, x, y| {
                        let blurred = program_gaussian_blur_pixel(
                            execution,
                            *input,
                            program.viewport,
                            local_to_device,
                            inverse,
                            x,
                            y,
                            [-f64::from(offset[0]), -f64::from(offset[1])],
                            &horizontal,
                            &vertical,
                        )?;
                        let shadow = PremulRgba32::from_premultiplied([
                            color.red * blurred.alpha(),
                            color.green * blurred.alpha(),
                            color.blue * blurred.alpha(),
                            color.alpha * blurred.alpha(),
                        ])?;
                        execution
                            .read_pixel(*input, x, y)?
                            .source_over(shadow)
                            .map_err(Into::into)
                    })?;
                }
                _ => return Err(ReferenceExecuteError::UnsupportedPass { pass }),
            },
            ProgramPassKind::ApplyMask {
                node,
                input,
                mask,
                output,
                ..
            } => {
                let mode = match program.mask_mode(*node)? {
                    MaskMode::Alpha => ReferenceMaskMode::Alpha,
                    MaskMode::Luminance => ReferenceMaskMode::Luminance,
                };
                execution.write_resource(*output, |execution, x, y| {
                    let coverage = mask_coverage_pixel(execution.read_pixel(*mask, x, y)?, mode);
                    Ok(execution
                        .read_pixel(*input, x, y)?
                        .scale_coverage(coverage)?)
                })?;
            }
            ProgramPassKind::SourceOver {
                source,
                destination,
                output,
            } => execution.write_resource(*output, |execution, x, y| {
                Ok(execution
                    .read_pixel(*source, x, y)?
                    .source_over(execution.read_pixel(*destination, x, y)?)?)
            })?,
            ProgramPassKind::ApplyOpacity {
                input,
                output,
                opacity,
                ..
            } => execution.write_resource(*output, |execution, x, y| {
                Ok(execution
                    .read_pixel(*input, x, y)?
                    .scale_coverage(*opacity)?)
            })?,
            ProgramPassKind::ApplyTransform { input, output, .. } => execution
                .write_resource(*output, |execution, x, y| {
                    execution.read_pixel(*input, x, y)
                })?,
            ProgramPassKind::ApplyShader { .. } => {
                return Err(ReferenceExecuteError::UnsupportedPass { pass });
            }
            ProgramPassKind::Blend {
                source,
                destination,
                output,
                mode,
                ..
            } => execution.write_resource(*output, |execution, x, y| {
                effective_blend_source(
                    execution.read_pixel(*source, x, y)?,
                    execution.read_pixel(*destination, x, y)?,
                    (*mode).into(),
                )
                .map_err(Into::into)
            })?,
        }
    }
    execution.finish()
}

fn raster_program_image_pixel(
    operation: &ReferenceImageOperation,
    source: &ReferenceImage,
    viewport: valle_draw::Rect,
    inverse: [f64; 9],
    x: u32,
    y: u32,
) -> Result<PremulRgba32, ReferenceExecuteError> {
    if operation.opacity == 0.0 || operation.dst.is_empty() {
        return Ok(PremulRgba32::TRANSPARENT);
    }
    let mut channels = [0.0_f64; 4];
    for sample_y in 0..PROGRAM_COVERAGE_SAMPLES_PER_AXIS {
        for sample_x in 0..PROGRAM_COVERAGE_SAMPLES_PER_AXIS {
            let device = [
                f64::from(x)
                    + (f64::from(sample_x) + 0.5) / f64::from(PROGRAM_COVERAGE_SAMPLES_PER_AXIS),
                f64::from(y)
                    + (f64::from(sample_y) + 0.5) / f64::from(PROGRAM_COVERAGE_SAMPLES_PER_AXIS),
            ];
            let normalized = project_homography(inverse, device)
                .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)?;
            let local = [
                viewport.x + normalized[0] * viewport.width,
                viewport.y + normalized[1] * viewport.height,
            ];
            if !contains_half_open(operation.dst, local) {
                continue;
            }
            let content = [
                (local[0] - operation.dst.x) / operation.dst.width,
                (local[1] - operation.dst.y) / operation.dst.height,
            ];
            let sample = sample_program_image_content(
                source,
                operation.sample,
                operation.sampling,
                content,
            )?;
            for (sum, value) in channels.iter_mut().zip(sample.channels()) {
                *sum += f64::from(value);
            }
        }
    }
    let sample_count =
        f64::from(PROGRAM_COVERAGE_SAMPLES_PER_AXIS * PROGRAM_COVERAGE_SAMPLES_PER_AXIS);
    premul_from_accumulated_channels(channels.map(|value| value / sample_count))?
        .scale_coverage(operation.opacity)
        .map_err(Into::into)
}

fn line_path_coverage(
    fill: &ReferenceFill,
    viewport: valle_draw::Rect,
    inverse: [f64; 9],
    x: u32,
    y: u32,
) -> Result<f32, ReferenceExecuteError> {
    let mut inside = 0_u32;
    for sample_y in 0..PROGRAM_COVERAGE_SAMPLES_PER_AXIS {
        for sample_x in 0..PROGRAM_COVERAGE_SAMPLES_PER_AXIS {
            let device = [
                f64::from(x)
                    + (f64::from(sample_x) + 0.5) / f64::from(PROGRAM_COVERAGE_SAMPLES_PER_AXIS),
                f64::from(y)
                    + (f64::from(sample_y) + 0.5) / f64::from(PROGRAM_COVERAGE_SAMPLES_PER_AXIS),
            ];
            let normalized = project_homography(inverse, device)
                .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)?;
            let local = [
                viewport.x + normalized[0] * viewport.width,
                viewport.y + normalized[1] * viewport.height,
            ];
            inside += u32::from(nonzero_fill_contains(fill, local));
        }
    }
    Ok(inside as f32
        / (PROGRAM_COVERAGE_SAMPLES_PER_AXIS * PROGRAM_COVERAGE_SAMPLES_PER_AXIS) as f32)
}

fn program_rect_coverage(
    rect: valle_draw::Rect,
    viewport: valle_draw::Rect,
    inverse: [f64; 9],
    x: u32,
    y: u32,
) -> Result<f32, ReferenceExecuteError> {
    program_local_coverage(viewport, inverse, x, y, |point| {
        contains_half_open(rect, point)
    })
}

fn program_clip_coverage(
    clip: &ReferenceClip,
    viewport: valle_draw::Rect,
    inverse: [f64; 9],
    x: u32,
    y: u32,
) -> Result<f32, ReferenceExecuteError> {
    program_local_coverage(viewport, inverse, x, y, |point| clip.contains(point))
}

fn program_glass_foreground_coverage(
    foreground: &valle_draw::program::MotionGlassForegroundProgram,
    owner_to_device: [f64; 9],
    x: u32,
    y: u32,
) -> Result<f32, ReferenceExecuteError> {
    let presence = crate::compositor::glass::materialization(f64::from(foreground.presence));
    if presence <= 0.0 {
        return Ok(0.0);
    }
    let mut inside = 0_u32;
    for sample_y in 0..PROGRAM_COVERAGE_SAMPLES_PER_AXIS {
        for sample_x in 0..PROGRAM_COVERAGE_SAMPLES_PER_AXIS {
            let device = [
                f64::from(x)
                    + (f64::from(sample_x) + 0.5) / f64::from(PROGRAM_COVERAGE_SAMPLES_PER_AXIS),
                f64::from(y)
                    + (f64::from(sample_y) + 0.5) / f64::from(PROGRAM_COVERAGE_SAMPLES_PER_AXIS),
            ];
            let Some((distance, band)) = crate::compositor::glass::transformed_foreground_distance(
                foreground,
                owner_to_device,
                device,
            ) else {
                continue;
            };
            let effective_distance = distance + (1.0 - presence) * band;
            inside += u32::from(effective_distance <= 0.0);
        }
    }
    Ok(inside as f32
        / (PROGRAM_COVERAGE_SAMPLES_PER_AXIS * PROGRAM_COVERAGE_SAMPLES_PER_AXIS) as f32)
}

fn program_resource_to_device(
    prepared: &PlanProgram,
    resource: ProgramResourceId,
    viewport: valle_draw::Rect,
    normalized_program_to_device: [f64; 9],
) -> Result<[f64; 9], ReferenceExecuteError> {
    let local_to_program = prepared
        .local_plan()
        .resources()
        .get(resource.index())
        .filter(|candidate| candidate.id == resource)
        .ok_or(ReferenceExecuteError::InvalidProgram {
            program: prepared.id,
        })?
        .local_to_program;
    program_transform_to_device(
        prepared,
        local_to_program,
        viewport,
        normalized_program_to_device,
    )
}

fn program_resource_normalized_matrices(
    prepared: &PlanProgram,
    resource: ProgramResourceId,
    viewport: valle_draw::Rect,
    normalized_program_to_device: [f64; 9],
) -> Result<([f64; 9], [f64; 9]), ReferenceExecuteError> {
    normalized_program_matrices(
        program_resource_to_device(prepared, resource, viewport, normalized_program_to_device)?,
        viewport,
    )
}

fn normalized_program_matrices(
    local_to_device: [f64; 9],
    viewport: valle_draw::Rect,
) -> Result<([f64; 9], [f64; 9]), ReferenceExecuteError> {
    let normalized_local_to_device = multiply_homography(
        local_to_device,
        [
            viewport.width,
            0.0,
            viewport.x,
            0.0,
            viewport.height,
            viewport.y,
            0.0,
            0.0,
            1.0,
        ],
    );
    let inverse = invert_homography(normalized_local_to_device)
        .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)?;
    Ok((normalized_local_to_device, inverse))
}

fn program_transform_to_device(
    prepared: &PlanProgram,
    local_to_program: valle_draw::program::Transform2d,
    viewport: valle_draw::Rect,
    normalized_program_to_device: [f64; 9],
) -> Result<[f64; 9], ReferenceExecuteError> {
    if viewport.is_empty() {
        return Err(ReferenceExecuteError::InvalidProgram {
            program: prepared.id,
        });
    }
    let program_to_normalized = [
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
    let owner_to_device = multiply_homography(
        normalized_program_to_device,
        multiply_homography(program_to_normalized, local_to_program.0),
    );
    owner_to_device
        .iter()
        .all(|value| value.is_finite())
        .then_some(owner_to_device)
        .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)
}

fn program_local_coverage(
    viewport: valle_draw::Rect,
    inverse: [f64; 9],
    x: u32,
    y: u32,
    mut contains: impl FnMut([f64; 2]) -> bool,
) -> Result<f32, ReferenceExecuteError> {
    let mut inside = 0_u32;
    for sample_y in 0..PROGRAM_COVERAGE_SAMPLES_PER_AXIS {
        for sample_x in 0..PROGRAM_COVERAGE_SAMPLES_PER_AXIS {
            let device = [
                f64::from(x)
                    + (f64::from(sample_x) + 0.5) / f64::from(PROGRAM_COVERAGE_SAMPLES_PER_AXIS),
                f64::from(y)
                    + (f64::from(sample_y) + 0.5) / f64::from(PROGRAM_COVERAGE_SAMPLES_PER_AXIS),
            ];
            let normalized = project_homography(inverse, device)
                .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)?;
            let local = [
                viewport.x + normalized[0] * viewport.width,
                viewport.y + normalized[1] * viewport.height,
            ];
            inside += u32::from(contains(local));
        }
    }
    Ok(inside as f32
        / (PROGRAM_COVERAGE_SAMPLES_PER_AXIS * PROGRAM_COVERAGE_SAMPLES_PER_AXIS) as f32)
}

#[allow(clippy::too_many_arguments)]
fn program_gaussian_blur_pixel(
    execution: &ReferenceProgramExecution<'_, '_>,
    input: ProgramResourceId,
    viewport: valle_draw::Rect,
    transform: [f64; 9],
    inverse: [f64; 9],
    x: u32,
    y: u32,
    center_offset: [f64; 2],
    horizontal: &GaussianKernel,
    vertical: &GaussianKernel,
) -> Result<PremulRgba32, ReferenceExecuteError> {
    if horizontal.taps.len() == 1 && vertical.taps.len() == 1 && center_offset == [0.0, 0.0] {
        return execution.read_pixel(input, x, y);
    }
    let normalized = project_homography(inverse, [f64::from(x) + 0.5, f64::from(y) + 0.5])
        .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)?;
    let local = [
        viewport.x + normalized[0] * viewport.width + center_offset[0],
        viewport.y + normalized[1] * viewport.height + center_offset[1],
    ];
    let mut channels = [0.0_f64; 4];
    for (offset_y, weight_y) in &vertical.taps {
        for (offset_x, weight_x) in &horizontal.taps {
            let sample_local = [
                local[0] + f64::from(*offset_x),
                local[1] + f64::from(*offset_y),
            ];
            let sample_normalized = [
                (sample_local[0] - viewport.x) / viewport.width,
                (sample_local[1] - viewport.y) / viewport.height,
            ];
            let sample_device = project_homography(transform, sample_normalized)
                .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)?;
            let sample = execution.sample_bilinear(input, sample_device)?;
            let weight = weight_x * weight_y;
            for (sum, value) in channels.iter_mut().zip(sample.channels()) {
                *sum += f64::from(value) * weight;
            }
        }
    }
    premul_from_accumulated_channels(channels)
}

fn nonzero_fill_contains(fill: &ReferenceFill, point: [f64; 2]) -> bool {
    for shape in &fill.shapes {
        if point[0] < shape.bounds.left()
            || point[0] > shape.bounds.right()
            || point[1] < shape.bounds.top()
            || point[1] > shape.bounds.bottom()
        {
            continue;
        }
        if nonzero_line_path_winding(&shape.segments, point) != 0 {
            return true;
        }
    }
    false
}

fn nonzero_line_path_winding(segments: &[LineSegment], point: [f64; 2]) -> i32 {
    let mut winding = 0_i32;
    for segment in segments {
        let side = (segment.to[0] - segment.from[0]) * (point[1] - segment.from[1])
            - (point[0] - segment.from[0]) * (segment.to[1] - segment.from[1]);
        if segment.from[1] <= point[1] {
            if segment.to[1] > point[1] && side > 0.0 {
                winding += 1;
            }
        } else if segment.to[1] <= point[1] && side < 0.0 {
            winding -= 1;
        }
    }
    winding
}

fn project_program_local_bounds(
    viewport: valle_draw::Rect,
    transform: DeviceTransform,
    bounds: valle_draw::Rect,
    extent: Extent2d,
) -> Result<DeviceRect, ReferenceExecuteError> {
    if bounds.is_empty() {
        return Ok(DeviceRect::new(0, 0, 0, 0));
    }
    let normalize = |point: [f64; 2]| {
        [
            (point[0] - viewport.x) / viewport.width,
            (point[1] - viewport.y) / viewport.height,
        ]
    };
    let corners = [
        [bounds.left(), bounds.top()],
        [bounds.right(), bounds.top()],
        [bounds.right(), bounds.bottom()],
        [bounds.left(), bounds.bottom()],
    ];
    let mut projected = [[0.0; 2]; 4];
    for (output, corner) in projected.iter_mut().zip(corners) {
        *output = project_homography(transform.matrix(), normalize(corner))
            .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)?;
    }
    let left = projected
        .iter()
        .map(|point| point[0])
        .fold(f64::INFINITY, f64::min);
    let top = projected
        .iter()
        .map(|point| point[1])
        .fold(f64::INFINITY, f64::min);
    let right = projected
        .iter()
        .map(|point| point[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let bottom = projected
        .iter()
        .map(|point| point[1])
        .fold(f64::NEG_INFINITY, f64::max);
    conservative_device_bounds(left, top, right, bottom, extent)
}

fn device_rect_contains_pixel(bounds: DeviceRect, x: u32, y: u32) -> bool {
    let x = i64::from(x);
    let y = i64::from(y);
    x >= i64::from(bounds.x)
        && x < i64::from(bounds.x) + i64::from(bounds.width)
        && y >= i64::from(bounds.y)
        && y < i64::from(bounds.y) + i64::from(bounds.height)
}

fn preflight_import(
    pass: ExecutionPassId,
    source_pipeline: &PlanSourcePipeline,
    placement: ExternalPlacement,
    bindings: &RenderBindings,
    bounds: DynamicBindingId,
    extent: Extent2d,
    kernel_samples: &mut u64,
) -> Result<(), ReferenceExecuteError> {
    let bounds = dynamic_bounds(bindings, bounds)?
        .intersect(DeviceRect::full(extent.width(), extent.height()));
    let pixels = u64::from(bounds.width)
        .checked_mul(u64::from(bounds.height))
        .ok_or(ReferenceExecuteError::KernelSampleBudgetExceeded {
            pass,
            required: u64::MAX,
            maximum: MAX_REFERENCE_KERNEL_SAMPLES,
        })?;
    if let Some(PreparedExternalBackdrop::Blur {
        sigma_device_px, ..
    }) = placement.backdrop
    {
        let sigma = f64::from(dynamic_scalar_kind(
            bindings,
            sigma_device_px,
            DynamicBindingKind::DeviceLength,
        )?);
        let taps =
            gaussian_tap_count(sigma).ok_or(ReferenceExecuteError::KernelSampleBudgetExceeded {
                pass,
                required: u64::MAX,
                maximum: MAX_REFERENCE_KERNEL_SAMPLES,
            })?;
        reserve_kernel_samples(pass, pixels, taps, kernel_samples)?;
    }
    if let Some(effect) = &source_pipeline.chroma_key {
        let PreparedEffectKernel::ChromaKey {
            feather_sigma_device_px,
            ..
        } = effect.kernel
        else {
            return Err(ReferenceExecuteError::UnsupportedPass { pass });
        };
        let sigma = f64::from(dynamic_scalar_kind(
            bindings,
            feather_sigma_device_px,
            DynamicBindingKind::DeviceLength,
        )?);
        let taps =
            gaussian_tap_count(sigma).ok_or(ReferenceExecuteError::KernelSampleBudgetExceeded {
                pass,
                required: u64::MAX,
                maximum: MAX_REFERENCE_KERNEL_SAMPLES,
            })?;
        reserve_kernel_samples(pass, pixels, taps, kernel_samples)?;
    }
    Ok(())
}

fn gaussian_tap_count(sigma: f64) -> Option<u64> {
    let axis = gaussian_axis_tap_count(sigma, EFFECT_GAUSSIAN_SUPPORT_SIGMAS)?;
    axis.checked_mul(axis)
}

fn gaussian_axis_tap_count(sigma: f64, support_sigmas: f64) -> Option<u64> {
    if sigma == 0.0 {
        return Some(1);
    }
    let radius = (sigma * support_sigmas).ceil();
    if !radius.is_finite() || radius < 0.0 || radius > u64::MAX as f64 {
        return None;
    }
    (radius as u64).checked_mul(2)?.checked_add(1)
}

fn preflight_reference_effect(
    pass: ExecutionPassId,
    effect: &PlanEffect,
    bindings: &RenderBindings,
    extent: Extent2d,
    kernel_samples: &mut u64,
) -> Result<(), ReferenceExecuteError> {
    let write_domain = effect_write_domain(effect, bindings, extent)?;
    let pixels = write_domain.pixel_count(pass)?;
    let taps = match effect.kernel {
        PreparedEffectKernel::ColorGrade { .. }
        | PreparedEffectKernel::Spotlight { .. }
        | PreparedEffectKernel::ExtensionColorGain { .. } => 0,
        PreparedEffectKernel::GaussianBlur {
            sigma_device_px, ..
        } => {
            let sigma = f64::from(dynamic_scalar_kind(
                bindings,
                sigma_device_px,
                DynamicBindingKind::DeviceLength,
            )?);
            gaussian_tap_count(sigma).ok_or(ReferenceExecuteError::KernelSampleBudgetExceeded {
                pass,
                required: u64::MAX,
                maximum: MAX_REFERENCE_KERNEL_SAMPLES,
            })?
        }
        PreparedEffectKernel::Mosaic {
            block_size_device_px,
            ..
        } => {
            dynamic_scalar_kind(
                bindings,
                block_size_device_px,
                DynamicBindingKind::DeviceLength,
            )?;
            mosaic_anchor(effect, bindings)?;
            1
        }
        PreparedEffectKernel::DirectionalBlur { span_device_px, .. } => {
            dynamic_scalar_kind(bindings, span_device_px, DynamicBindingKind::DeviceLength)?;
            u64::from(DIRECTIONAL_BLUR_SAMPLES)
        }
        PreparedEffectKernel::ChromaKey { .. } => {
            return Err(ReferenceExecuteError::UnsupportedPass { pass });
        }
    };
    if taps != 0 {
        reserve_kernel_samples(pass, pixels, taps, kernel_samples)?;
    }
    Ok(())
}

fn reserve_kernel_samples(
    pass: ExecutionPassId,
    pixels: u64,
    taps: u64,
    kernel_samples: &mut u64,
) -> Result<(), ReferenceExecuteError> {
    let required =
        pixels
            .checked_mul(taps)
            .ok_or(ReferenceExecuteError::KernelSampleBudgetExceeded {
                pass,
                required: u64::MAX,
                maximum: MAX_REFERENCE_KERNEL_SAMPLES,
            })?;
    *kernel_samples = kernel_samples.checked_add(required).ok_or(
        ReferenceExecuteError::KernelSampleBudgetExceeded {
            pass,
            required: u64::MAX,
            maximum: MAX_REFERENCE_KERNEL_SAMPLES,
        },
    )?;
    if *kernel_samples > MAX_REFERENCE_KERNEL_SAMPLES {
        return Err(ReferenceExecuteError::KernelSampleBudgetExceeded {
            pass,
            required: *kernel_samples,
            maximum: MAX_REFERENCE_KERNEL_SAMPLES,
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct EffectWriteDomain {
    bounds: DeviceRect,
    layer_region: Option<LayerWriteRegion>,
}

impl EffectWriteDomain {
    fn contains(self, position: [f64; 2]) -> Result<bool, ReferenceExecuteError> {
        match self.layer_region {
            Some(region) => region.contains(position),
            None => Ok(true),
        }
    }

    fn pixel_count(self, pass: ExecutionPassId) -> Result<u64, ReferenceExecuteError> {
        if self.layer_region.is_none() {
            return Ok(self.bounds.pixels());
        }
        let start_x =
            u32::try_from(self.bounds.x).map_err(|_| ReferenceExecuteError::InvalidEffectBounds)?;
        let start_y =
            u32::try_from(self.bounds.y).map_err(|_| ReferenceExecuteError::InvalidEffectBounds)?;
        let mut pixels = 0_u64;
        for y in start_y..start_y + self.bounds.height {
            for x in start_x..start_x + self.bounds.width {
                if self.contains([f64::from(x) + 0.5, f64::from(y) + 0.5])? {
                    pixels = pixels.checked_add(1).ok_or(
                        ReferenceExecuteError::KernelSampleBudgetExceeded {
                            pass,
                            required: u64::MAX,
                            maximum: MAX_REFERENCE_KERNEL_SAMPLES,
                        },
                    )?;
                }
            }
        }
        Ok(pixels)
    }
}

#[derive(Debug, Clone, Copy)]
struct LayerWriteRegion {
    local_from_device: [f64; 9],
    region: PreparedUnitRect,
}

impl LayerWriteRegion {
    fn contains(self, position: [f64; 2]) -> Result<bool, ReferenceExecuteError> {
        let local = project_homography(self.local_from_device, position)
            .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)?;
        let left = f64::from(self.region.x);
        let top = f64::from(self.region.y);
        let right = left + f64::from(self.region.width);
        let bottom = top + f64::from(self.region.height);
        Ok(local[0] >= left && local[0] < right && local[1] >= top && local[1] < bottom)
    }
}

fn effect_write_domain(
    effect: &PlanEffect,
    bindings: &RenderBindings,
    extent: Extent2d,
) -> Result<EffectWriteDomain, ReferenceExecuteError> {
    let root = DeviceRect::full(extent.width(), extent.height());
    let mut bounds = match effect.space {
        PreparedEffectSpace::Layer { bounds, .. } => {
            dynamic_bounds(bindings, bounds)?.intersect(root)
        }
        PreparedEffectSpace::Root => root,
    };
    let region = match effect.kernel {
        PreparedEffectKernel::GaussianBlur { region, .. }
        | PreparedEffectKernel::Mosaic { region, .. } => region,
        _ => None,
    };
    let mut layer_region = None;
    if let Some(region) = region {
        match effect.space {
            PreparedEffectSpace::Root => {
                bounds = bounds.intersect(root_unit_region_bounds(region, extent)?);
            }
            PreparedEffectSpace::Layer { transform, .. } => {
                let transform_value = dynamic_transform(bindings, transform)?;
                bounds = bounds.intersect(projected_unit_region_bounds(
                    region,
                    transform_value,
                    extent,
                )?);
                if !bounds.is_empty() {
                    let local_from_device = invert_homography(transform_value.matrix()).ok_or(
                        ReferenceExecuteError::NonInvertibleTransform { binding: transform },
                    )?;
                    layer_region = Some(LayerWriteRegion {
                        local_from_device,
                        region,
                    });
                }
            }
        }
    }
    u32::try_from(bounds.x).map_err(|_| ReferenceExecuteError::InvalidEffectBounds)?;
    u32::try_from(bounds.y).map_err(|_| ReferenceExecuteError::InvalidEffectBounds)?;
    Ok(EffectWriteDomain {
        bounds,
        layer_region,
    })
}

fn root_unit_region_bounds(
    region: PreparedUnitRect,
    extent: Extent2d,
) -> Result<DeviceRect, ReferenceExecuteError> {
    let width = f64::from(extent.width());
    let height = f64::from(extent.height());
    let left = f64::from(region.x) * width;
    let top = f64::from(region.y) * height;
    let right = (f64::from(region.x) + f64::from(region.width)) * width;
    let bottom = (f64::from(region.y) + f64::from(region.height)) * height;
    let start_x = (left - 0.5).ceil().clamp(0.0, width) as u32;
    let start_y = (top - 0.5).ceil().clamp(0.0, height) as u32;
    let end_x = (right - 0.5).ceil().clamp(0.0, width) as u32;
    let end_y = (bottom - 0.5).ceil().clamp(0.0, height) as u32;
    Ok(DeviceRect::new(
        i32::try_from(start_x).map_err(|_| ReferenceExecuteError::InvalidEffectBounds)?,
        i32::try_from(start_y).map_err(|_| ReferenceExecuteError::InvalidEffectBounds)?,
        end_x.saturating_sub(start_x),
        end_y.saturating_sub(start_y),
    ))
}

fn projected_unit_region_bounds(
    region: PreparedUnitRect,
    transform: DeviceTransform,
    extent: Extent2d,
) -> Result<DeviceRect, ReferenceExecuteError> {
    let left = f64::from(region.x);
    let top = f64::from(region.y);
    let right = left + f64::from(region.width);
    let bottom = top + f64::from(region.height);
    let projected = [[left, top], [right, top], [right, bottom], [left, bottom]].map(|point| {
        project_homography(transform.matrix(), point)
            .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)
    });
    let [first, second, third, fourth] = projected;
    let projected = [first?, second?, third?, fourth?];
    let left = projected
        .iter()
        .map(|point| point[0])
        .fold(f64::INFINITY, f64::min);
    let top = projected
        .iter()
        .map(|point| point[1])
        .fold(f64::INFINITY, f64::min);
    let right = projected
        .iter()
        .map(|point| point[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let bottom = projected
        .iter()
        .map(|point| point[1])
        .fold(f64::NEG_INFINITY, f64::max);
    conservative_device_bounds(left, top, right, bottom, extent)
}

fn conservative_device_bounds(
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
    extent: Extent2d,
) -> Result<DeviceRect, ReferenceExecuteError> {
    if ![left, top, right, bottom]
        .iter()
        .all(|value| value.is_finite())
        || left > right
        || top > bottom
    {
        return Err(ReferenceExecuteError::InvalidEffectBounds);
    }
    let width = f64::from(extent.width());
    let height = f64::from(extent.height());
    let start_x = (left - 0.5).ceil().clamp(0.0, width) as u32;
    let start_y = (top - 0.5).ceil().clamp(0.0, height) as u32;
    let end_x = ((right - 0.5).floor() + 1.0).clamp(0.0, width) as u32;
    let end_y = ((bottom - 0.5).floor() + 1.0).clamp(0.0, height) as u32;
    Ok(DeviceRect::new(
        i32::try_from(start_x).map_err(|_| ReferenceExecuteError::InvalidEffectBounds)?,
        i32::try_from(start_y).map_err(|_| ReferenceExecuteError::InvalidEffectBounds)?,
        end_x.saturating_sub(start_x),
        end_y.saturating_sub(start_y),
    ))
}

fn mosaic_anchor(
    effect: &PlanEffect,
    bindings: &RenderBindings,
) -> Result<[f64; 2], ReferenceExecuteError> {
    match effect.space {
        PreparedEffectSpace::Layer { transform, .. } => {
            let transform = dynamic_transform(bindings, transform)?;
            project_homography(transform.matrix(), [0.0, 0.0])
                .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)
        }
        PreparedEffectSpace::Root => Ok([0.0, 0.0]),
    }
}

fn surface(
    surfaces: &BTreeMap<PlanResourceId, ReferenceImage>,
    resource: PlanResourceId,
) -> Result<&ReferenceImage, ReferenceExecuteError> {
    surfaces
        .get(&resource)
        .ok_or(ReferenceExecuteError::MissingResource { resource })
}

fn backdrop_bounds(
    bindings: &RenderBindings,
    sample_binding: DynamicBindingId,
    output_binding: DynamicBindingId,
    extent: Extent2d,
) -> Result<(DeviceRect, DeviceRect), ReferenceExecuteError> {
    let sample = dynamic_bounds_kind(
        bindings,
        sample_binding,
        DynamicBindingKind::BackdropSampleBounds,
    )?;
    let output = dynamic_bounds_kind(
        bindings,
        output_binding,
        DynamicBindingKind::BackdropOutputBounds,
    )?;
    if !output.is_empty() && sample.intersect(output) != output {
        return Err(ReferenceExecuteError::InvalidBackdropBounds {
            sample: sample_binding,
            output: output_binding,
        });
    }
    let root = DeviceRect::full(extent.width(), extent.height());
    Ok((sample.intersect(root), output.intersect(root)))
}

fn copy_region(
    image: &ReferenceImage,
    bounds: DeviceRect,
    extent: Extent2d,
) -> Result<ReferenceImage, ReferenceExecuteError> {
    if image.extent() != extent {
        return Err(PixelError::ExtentMismatch {
            left: image.extent(),
            right: extent,
        }
        .into());
    }
    let mut pixels = vec![PremulRgba32::TRANSPARENT; image.pixels().len()];
    let start_x =
        u32::try_from(bounds.x).map_err(|_| ReferenceExecuteError::InvalidSampleCoordinate)?;
    let start_y =
        u32::try_from(bounds.y).map_err(|_| ReferenceExecuteError::InvalidSampleCoordinate)?;
    for y in start_y..start_y + bounds.height {
        let row = y as usize * extent.width() as usize;
        for x in start_x..start_x + bounds.width {
            let index = row + x as usize;
            pixels[index] = image.pixels()[index];
        }
    }
    ReferenceImage::new(extent, pixels).map_err(Into::into)
}

#[derive(Debug)]
enum ReferenceEffectRuntime {
    ColorGrade {
        brightness: f32,
        contrast: f32,
        saturation: f32,
        temperature: f32,
        vignette: f32,
    },
    GaussianBlur {
        gaussian: GaussianKernel,
    },
    Mosaic {
        block_size_device_px: f64,
        anchor_device_px: [f64; 2],
    },
    DirectionalBlur {
        axis: PreparedBlurAxis,
        span_device_px: f64,
    },
    Spotlight {
        center: [f32; 2],
        radius: f32,
        feather: f32,
        intensity: f32,
    },
    ColorGain {
        gain: f32,
    },
}

fn prepare_reference_effect_runtime(
    pass: ExecutionPassId,
    effect: &PlanEffect,
    bindings: &RenderBindings,
) -> Result<ReferenceEffectRuntime, ReferenceExecuteError> {
    validate_reference_effect_implementation(pass, effect.kernel)?;
    match effect.kernel {
        PreparedEffectKernel::ColorGrade {
            brightness,
            contrast,
            saturation,
            temperature,
            vignette,
        } => Ok(ReferenceEffectRuntime::ColorGrade {
            brightness,
            contrast,
            saturation,
            temperature,
            vignette,
        }),
        PreparedEffectKernel::GaussianBlur {
            sigma_device_px, ..
        } => {
            let sigma = f64::from(dynamic_scalar_kind(
                bindings,
                sigma_device_px,
                DynamicBindingKind::DeviceLength,
            )?);
            Ok(ReferenceEffectRuntime::GaussianBlur {
                gaussian: GaussianKernel::new(pass, sigma)?,
            })
        }
        PreparedEffectKernel::Mosaic {
            block_size_device_px,
            ..
        } => Ok(ReferenceEffectRuntime::Mosaic {
            block_size_device_px: f64::from(dynamic_scalar_kind(
                bindings,
                block_size_device_px,
                DynamicBindingKind::DeviceLength,
            )?)
            .max(1.0),
            anchor_device_px: mosaic_anchor(effect, bindings)?,
        }),
        PreparedEffectKernel::DirectionalBlur {
            axis,
            span_device_px,
        } => Ok(ReferenceEffectRuntime::DirectionalBlur {
            axis,
            span_device_px: f64::from(dynamic_scalar_kind(
                bindings,
                span_device_px,
                DynamicBindingKind::DeviceLength,
            )?),
        }),
        PreparedEffectKernel::Spotlight {
            center,
            radius,
            feather,
            intensity,
        } => Ok(ReferenceEffectRuntime::Spotlight {
            center,
            radius,
            feather,
            intensity,
        }),
        PreparedEffectKernel::ExtensionColorGain { gain, .. } => {
            Ok(ReferenceEffectRuntime::ColorGain { gain })
        }
        PreparedEffectKernel::ChromaKey { .. } => {
            Err(ReferenceExecuteError::UnsupportedPass { pass })
        }
    }
}

fn validate_reference_effect_implementation(
    pass: ExecutionPassId,
    kernel: PreparedEffectKernel,
) -> Result<(), ReferenceExecuteError> {
    if let PreparedEffectKernel::ExtensionColorGain {
        implementation_sha256,
        ..
    } = kernel
        && implementation_sha256
            != crate::render::engine_owned_kernel_implementation_sha256(
                crate::render::EXTENSION_COLOR_GAIN_ABI,
            )
            .expect("color-gain ABI has an engine-owned implementation")
    {
        return Err(ReferenceExecuteError::ExtensionImplementationMismatch { pass });
    }
    Ok(())
}

fn apply_reference_effect(
    pass: ExecutionPassId,
    image: &ReferenceImage,
    effect: &PlanEffect,
    bindings: &RenderBindings,
) -> Result<ReferenceImage, ReferenceExecuteError> {
    let extent = image.extent();
    let write_domain = effect_write_domain(effect, bindings, extent)?;
    if write_domain.bounds.is_empty() {
        return Ok(image.clone());
    }
    let runtime = prepare_reference_effect_runtime(pass, effect, bindings)?;
    let start_x = u32::try_from(write_domain.bounds.x)
        .map_err(|_| ReferenceExecuteError::InvalidEffectBounds)?;
    let start_y = u32::try_from(write_domain.bounds.y)
        .map_err(|_| ReferenceExecuteError::InvalidEffectBounds)?;
    let mut pixels = image.pixels().to_vec();
    let root_size = [f64::from(extent.width()), f64::from(extent.height())];

    for y in start_y..start_y + write_domain.bounds.height {
        let row = y as usize * extent.width() as usize;
        for x in start_x..start_x + write_domain.bounds.width {
            let index = row + x as usize;
            let position = [f64::from(x) + 0.5, f64::from(y) + 0.5];
            if !write_domain.contains(position)? {
                continue;
            }
            pixels[index] = match &runtime {
                ReferenceEffectRuntime::ColorGrade {
                    brightness,
                    contrast,
                    saturation,
                    temperature,
                    vignette,
                } => color_grade_pixel(
                    image.pixels()[index],
                    [position[0] / root_size[0], position[1] / root_size[1]],
                    *brightness,
                    *contrast,
                    *saturation,
                    *temperature,
                    *vignette,
                )?,
                ReferenceEffectRuntime::GaussianBlur { gaussian } => {
                    gaussian_blur_pixel(image, image.pixels()[index], position, gaussian)?
                }
                ReferenceEffectRuntime::Mosaic {
                    block_size_device_px,
                    anchor_device_px,
                } => mosaic_pixel(image, position, *block_size_device_px, *anchor_device_px)?,
                ReferenceEffectRuntime::DirectionalBlur {
                    axis,
                    span_device_px,
                } => directional_blur_pixel(
                    image,
                    image.pixels()[index],
                    position,
                    *axis,
                    *span_device_px,
                )?,
                ReferenceEffectRuntime::Spotlight {
                    center,
                    radius,
                    feather,
                    intensity,
                } => spotlight_pixel(
                    image.pixels()[index],
                    position,
                    root_size,
                    *center,
                    *radius,
                    *feather,
                    *intensity,
                )?,
                ReferenceEffectRuntime::ColorGain { gain } => {
                    let [red, green, blue, alpha] = image.pixels()[index].channels();
                    PremulRgba32::from_premultiplied([
                        red * *gain,
                        green * *gain,
                        blue * *gain,
                        alpha,
                    ])?
                }
            };
        }
    }
    ReferenceImage::new(extent, pixels).map_err(Into::into)
}

fn gaussian_blur_pixel(
    image: &ReferenceImage,
    original: PremulRgba32,
    position: [f64; 2],
    gaussian: &GaussianKernel,
) -> Result<PremulRgba32, ReferenceExecuteError> {
    if gaussian.taps.len() == 1 {
        return Ok(original);
    }
    let mut channels = [0.0_f64; 4];
    for (offset_y, weight_y) in &gaussian.taps {
        for (offset_x, weight_x) in &gaussian.taps {
            let sample = sample_bilinear_device(
                image,
                [
                    position[0] + f64::from(*offset_x),
                    position[1] + f64::from(*offset_y),
                ],
            )?;
            let weight = weight_x * weight_y;
            for (sum, value) in channels.iter_mut().zip(sample.channels()) {
                *sum += f64::from(value) * weight;
            }
        }
    }
    premul_from_accumulated_channels(channels)
}

fn mosaic_pixel(
    image: &ReferenceImage,
    position: [f64; 2],
    block_size_device_px: f64,
    anchor_device_px: [f64; 2],
) -> Result<PremulRgba32, ReferenceExecuteError> {
    let cell_center = std::array::from_fn(|axis| {
        anchor_device_px[axis]
            + (((position[axis] - anchor_device_px[axis]) / block_size_device_px).floor() + 0.5)
                * block_size_device_px
    });
    sample_bilinear_device(image, cell_center)
}

fn directional_blur_pixel(
    image: &ReferenceImage,
    original: PremulRgba32,
    position: [f64; 2],
    axis: PreparedBlurAxis,
    span_device_px: f64,
) -> Result<PremulRgba32, ReferenceExecuteError> {
    if span_device_px == 0.0 {
        return Ok(original);
    }
    let mut channels = [0.0_f64; 4];
    let intervals = f64::from(DIRECTIONAL_BLUR_SAMPLES - 1);
    for sample_index in 0..DIRECTIONAL_BLUR_SAMPLES {
        let offset = -0.5 * span_device_px + span_device_px * f64::from(sample_index) / intervals;
        let sample_position = match axis {
            PreparedBlurAxis::Horizontal => [position[0] + offset, position[1]],
            PreparedBlurAxis::Vertical => [position[0], position[1] + offset],
        };
        let sample = sample_bilinear_device(image, sample_position)?;
        for (sum, value) in channels.iter_mut().zip(sample.channels()) {
            *sum += f64::from(value);
        }
    }
    let divisor = f64::from(DIRECTIONAL_BLUR_SAMPLES);
    premul_from_accumulated_channels(channels.map(|value| value / divisor))
}

fn premul_from_accumulated_channels(
    channels: [f64; 4],
) -> Result<PremulRgba32, ReferenceExecuteError> {
    let mut channels = channels.map(|value| value as f32);
    channels[3] = channels[3].clamp(0.0, 1.0);
    PremulRgba32::from_premultiplied(channels).map_err(Into::into)
}

fn sample_bilinear_device(
    image: &ReferenceImage,
    position: [f64; 2],
) -> Result<PremulRgba32, ReferenceExecuteError> {
    if !position.iter().all(|value| value.is_finite()) {
        return Err(ReferenceExecuteError::InvalidSampleCoordinate);
    }
    let maximum = [
        f64::from(image.extent().width() - 1),
        f64::from(image.extent().height() - 1),
    ];
    let coordinate = [
        (position[0] - 0.5).clamp(0.0, maximum[0]),
        (position[1] - 0.5).clamp(0.0, maximum[1]),
    ];
    let floor = [coordinate[0].floor(), coordinate[1].floor()];
    let first = [floor[0] as u32, floor[1] as u32];
    let second = [
        first[0].saturating_add(1).min(image.extent().width() - 1),
        first[1].saturating_add(1).min(image.extent().height() - 1),
    ];
    let amount = [coordinate[0] - floor[0], coordinate[1] - floor[1]];
    let samples = [
        image
            .pixel(first[0], first[1])
            .expect("root-clamped sample is in bounds"),
        image
            .pixel(second[0], first[1])
            .expect("root-clamped sample is in bounds"),
        image
            .pixel(first[0], second[1])
            .expect("root-clamped sample is in bounds"),
        image
            .pixel(second[0], second[1])
            .expect("root-clamped sample is in bounds"),
    ];
    let top = lerp_channels(samples[0].channels(), samples[1].channels(), amount[0]);
    let bottom = lerp_channels(samples[2].channels(), samples[3].channels(), amount[0]);
    PremulRgba32::from_premultiplied(lerp_channels(top, bottom, amount[1])).map_err(Into::into)
}

fn apply_prepared_mask(
    image: &ReferenceImage,
    mask: PreparedMask,
) -> Result<ReferenceImage, ReferenceExecuteError> {
    let geometry = mask
        .geometry()
        .map_err(|_| ReferenceExecuteError::InvalidMask)?;
    let sigma = mask.feather_sigma_device_px();
    let support = mask.support_radius_device_px();
    let tail = (sigma > 0.0).then(|| normal_cdf(-MASK_GAUSSIAN_SUPPORT_SIGMAS));
    let samples = f64::from(MASK_COVERAGE_SAMPLES_PER_AXIS * MASK_COVERAGE_SAMPLES_PER_AXIS);
    let mut pixels = Vec::with_capacity(image.pixels().len());

    for y in 0..image.extent().height() {
        for x in 0..image.extent().width() {
            let mut coverage = 0.0;
            for sample_y in 0..MASK_COVERAGE_SAMPLES_PER_AXIS {
                for sample_x in 0..MASK_COVERAGE_SAMPLES_PER_AXIS {
                    let point = [
                        f64::from(x)
                            + (f64::from(sample_x) + 0.5)
                                / f64::from(MASK_COVERAGE_SAMPLES_PER_AXIS),
                        f64::from(y)
                            + (f64::from(sample_y) + 0.5)
                                / f64::from(MASK_COVERAGE_SAMPLES_PER_AXIS),
                    ];
                    let distance = mask_signed_distance(mask.shape(), geometry, point);
                    coverage += if sigma == 0.0 {
                        if distance <= 0.0 { 1.0 } else { 0.0 }
                    } else if distance <= -support {
                        1.0
                    } else if distance >= support {
                        0.0
                    } else {
                        let tail = tail.expect("positive sigma has one normalization tail");
                        ((normal_cdf(-distance / sigma) - tail) / (1.0 - 2.0 * tail))
                            .clamp(0.0, 1.0)
                    };
                }
            }
            coverage /= samples;
            if mask.invert() {
                coverage = 1.0 - coverage;
            }
            let pixel = image
                .pixel(x, y)
                .expect("iteration remains inside the reference image")
                .scale_coverage(coverage as f32)?;
            pixels.push(pixel);
        }
    }
    ReferenceImage::new(image.extent(), pixels).map_err(Into::into)
}

fn mask_signed_distance(
    shape: PreparedMaskShape,
    geometry: PreparedMaskGeometry,
    point: [f64; 2],
) -> f64 {
    let delta = [
        point[0] - geometry.center_device_px[0],
        point[1] - geometry.center_device_px[1],
    ];
    let local = [
        delta[0] * geometry.axis_x[0] + delta[1] * geometry.axis_x[1],
        delta[0] * geometry.axis_y[0] + delta[1] * geometry.axis_y[1],
    ];
    let half = geometry.half_extent_device_px;
    match shape {
        PreparedMaskShape::Rect => {
            let q = [local[0].abs() - half[0], local[1].abs() - half[1]];
            sqrt(q[0].max(0.0) * q[0].max(0.0) + q[1].max(0.0) * q[1].max(0.0))
                + q[0].max(q[1]).min(0.0)
        }
        PreparedMaskShape::Ellipse => {
            let normalized = sqrt(
                (local[0] / half[0]) * (local[0] / half[0])
                    + (local[1] / half[1]) * (local[1] / half[1]),
            );
            let gradient = sqrt(
                (local[0] / (half[0] * half[0])) * (local[0] / (half[0] * half[0]))
                    + (local[1] / (half[1] * half[1])) * (local[1] / (half[1] * half[1])),
            );
            if gradient == 0.0 {
                -half[0].min(half[1])
            } else {
                normalized * (normalized - 1.0) / gradient
            }
        }
    }
}

fn normal_cdf(value: f64) -> f64 {
    0.5 * (1.0 + erf_approx(value / sqrt(2.0)))
}

fn erf_approx(value: f64) -> f64 {
    if value == 0.0 {
        return 0.0;
    }
    let sign = value.signum();
    let x = value.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let polynomial =
        (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t
            + 0.254_829_592)
            * t;
    sign * (1.0 - polynomial * exp(-x * x))
}

#[derive(Debug)]
struct ChromaRuntime {
    key_working_linear_rec2020: [f32; 3],
    intensity: f32,
    shadow: f32,
    edge_clean: f32,
    gaussian: GaussianKernel,
}

#[derive(Debug)]
enum ImportBackdropRuntime {
    Transparent,
    Color(PremulRgba32),
    Blur {
        sample: ExternalSample,
        gaussian: GaussianKernel,
    },
}

#[derive(Debug)]
struct GaussianKernel {
    taps: Vec<(i32, f64)>,
}

impl GaussianKernel {
    fn new(pass: ExecutionPassId, sigma: f64) -> Result<Self, ReferenceExecuteError> {
        Self::new_with_support(pass, sigma, EFFECT_GAUSSIAN_SUPPORT_SIGMAS)
    }

    fn new_with_support(
        pass: ExecutionPassId,
        sigma: f64,
        support_sigmas: f64,
    ) -> Result<Self, ReferenceExecuteError> {
        if sigma == 0.0 {
            return Ok(Self {
                taps: vec![(0, 1.0)],
            });
        }
        let radius = (sigma * support_sigmas).ceil();
        let radius = i32::try_from(radius as i64).map_err(|_| {
            ReferenceExecuteError::KernelSampleBudgetExceeded {
                pass,
                required: u64::MAX,
                maximum: MAX_REFERENCE_KERNEL_SAMPLES,
            }
        })?;
        let denominator = 2.0 * sigma * sigma;
        let mut taps = Vec::with_capacity((radius as usize).saturating_mul(2).saturating_add(1));
        let mut total = 0.0;
        for offset in -radius..=radius {
            let distance = f64::from(offset);
            let weight = exp(-(distance * distance) / denominator);
            taps.push((offset, weight));
            total += weight;
        }
        for (_, weight) in &mut taps {
            *weight /= total;
        }
        Ok(Self { taps })
    }
}

fn prepare_chroma_runtime(
    pass: ExecutionPassId,
    source_pipeline: &PlanSourcePipeline,
    bindings: &RenderBindings,
) -> Result<Option<ChromaRuntime>, ReferenceExecuteError> {
    source_pipeline
        .chroma_key
        .as_ref()
        .map(|effect| {
            let PreparedEffectKernel::ChromaKey {
                key_working_linear_rec2020,
                intensity,
                shadow,
                feather_sigma_device_px,
                edge_clean,
            } = effect.kernel
            else {
                return Err(ReferenceExecuteError::UnsupportedPass { pass });
            };
            let sigma = f64::from(dynamic_scalar_kind(
                bindings,
                feather_sigma_device_px,
                DynamicBindingKind::DeviceLength,
            )?);
            Ok(ChromaRuntime {
                key_working_linear_rec2020,
                intensity,
                shadow,
                edge_clean,
                gaussian: GaussianKernel::new(pass, sigma)?,
            })
        })
        .transpose()
}

fn prepare_import_backdrop_runtime(
    pass: ExecutionPassId,
    placement: ExternalPlacement,
    bindings: &RenderBindings,
) -> Result<ImportBackdropRuntime, ReferenceExecuteError> {
    match placement.backdrop {
        None => Ok(ImportBackdropRuntime::Transparent),
        Some(PreparedExternalBackdrop::Color {
            working_linear_rec2020_premul,
        }) => Ok(ImportBackdropRuntime::Color(
            PremulRgba32::from_premultiplied(working_linear_rec2020_premul)?,
        )),
        Some(PreparedExternalBackdrop::Blur {
            sigma_device_px,
            sample,
        }) => {
            let sigma = f64::from(dynamic_scalar_kind(
                bindings,
                sigma_device_px,
                DynamicBindingKind::DeviceLength,
            )?);
            Ok(ImportBackdropRuntime::Blur {
                sample,
                gaussian: GaussianKernel::new(pass, sigma)?,
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn raster_import(
    pass: ExecutionPassId,
    template: &RenderPlanTemplate,
    bound: &BoundExternalObjects<'_, ReferenceExternalObject>,
    bindings: &RenderBindings,
    external: PlanResourceId,
    source_pipeline: &PlanSourcePipeline,
    placement: ExternalPlacement,
    transform_binding: DynamicBindingId,
    bounds_binding: DynamicBindingId,
    extent: Extent2d,
) -> Result<ReferenceImage, ReferenceExecuteError> {
    let bounds = dynamic_bounds(bindings, bounds_binding)?
        .intersect(DeviceRect::full(extent.width(), extent.height()));
    if bounds.is_empty() {
        return ReferenceImage::transparent(extent).map_err(Into::into);
    }
    let source = external_visual(template, bound, external)?;
    let chroma = prepare_chroma_runtime(pass, source_pipeline, bindings)?;
    let transform = dynamic_transform(bindings, transform_binding)?;
    let inverse = invert_homography(transform.matrix()).ok_or(
        ReferenceExecuteError::NonInvertibleTransform {
            binding: transform_binding,
        },
    )?;
    let pixel_count = (extent.width() as usize)
        .checked_mul(extent.height() as usize)
        .ok_or(PixelError::ImageTooLarge)?;
    let mut pixels = vec![PremulRgba32::TRANSPARENT; pixel_count];
    let backdrop = prepare_import_backdrop_runtime(pass, placement, bindings)?;

    let start_x =
        u32::try_from(bounds.x).map_err(|_| ReferenceExecuteError::InvalidDynamicBinding {
            binding: bounds_binding,
        })?;
    let start_y =
        u32::try_from(bounds.y).map_err(|_| ReferenceExecuteError::InvalidDynamicBinding {
            binding: bounds_binding,
        })?;
    let end_x =
        start_x
            .checked_add(bounds.width)
            .ok_or(ReferenceExecuteError::InvalidDynamicBinding {
                binding: bounds_binding,
            })?;
    let end_y =
        start_y
            .checked_add(bounds.height)
            .ok_or(ReferenceExecuteError::InvalidDynamicBinding {
                binding: bounds_binding,
            })?;
    for y in start_y..end_y {
        for x in start_x..end_x {
            let device = [f64::from(x) + 0.5, f64::from(y) + 0.5];
            let local = project_homography(inverse, device)
                .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)?;
            if !contains_half_open(placement.clip_rect, local) {
                continue;
            }
            let mut pixel =
                sample_import_backdrop(source, placement.clip_rect, inverse, device, &backdrop)?;
            let original = sample_external_at_device(source, placement, inverse, device)?;
            let mut sampled = original;
            if let Some(effect) = &chroma
                && sampled.alpha() != 0.0
            {
                let mut coverage = 0.0_f64;
                for (offset_y, weight_y) in &effect.gaussian.taps {
                    for (offset_x, weight_x) in &effect.gaussian.taps {
                        let sample_x = (i64::from(x) + i64::from(*offset_x))
                            .clamp(0, i64::from(extent.width()) - 1);
                        let sample_y = (i64::from(y) + i64::from(*offset_y))
                            .clamp(0, i64::from(extent.height()) - 1);
                        let neighbor = sample_external_at_device(
                            source,
                            placement,
                            inverse,
                            [sample_x as f64 + 0.5, sample_y as f64 + 0.5],
                        )?;
                        let retained = chroma_key_coverage(
                            neighbor,
                            effect.key_working_linear_rec2020,
                            effect.intensity,
                            effect.shadow,
                        )?;
                        coverage += f64::from(retained) * weight_x * weight_y;
                    }
                }
                let coverage = (coverage as f32).clamp(0.0, 1.0);
                sampled = chroma_key_despill(
                    sampled,
                    effect.key_working_linear_rec2020,
                    effect.edge_clean,
                    1.0 - coverage,
                )?
                .scale_coverage(coverage)?;
            }
            pixel = sampled.source_over(pixel)?;
            let index = y as usize * extent.width() as usize + x as usize;
            pixels[index] = pixel;
        }
    }
    ReferenceImage::new(extent, pixels).map_err(Into::into)
}

fn sample_import_backdrop(
    image: &ReferenceImage,
    clip_rect: valle_draw::Rect,
    inverse: [f64; 9],
    device: [f64; 2],
    backdrop: &ImportBackdropRuntime,
) -> Result<PremulRgba32, ReferenceExecuteError> {
    match backdrop {
        ImportBackdropRuntime::Transparent => Ok(PremulRgba32::TRANSPARENT),
        ImportBackdropRuntime::Color(pixel) => Ok(*pixel),
        ImportBackdropRuntime::Blur { sample, gaussian } => {
            let mut channels = [0.0_f64; 4];
            for (offset_y, weight_y) in &gaussian.taps {
                for (offset_x, weight_x) in &gaussian.taps {
                    let sampled = sample_external_cover_at_device(
                        image,
                        *sample,
                        clip_rect,
                        inverse,
                        [
                            device[0] + f64::from(*offset_x),
                            device[1] + f64::from(*offset_y),
                        ],
                    )?;
                    let weight = weight_x * weight_y;
                    for (sum, value) in channels.iter_mut().zip(sampled.channels()) {
                        *sum += f64::from(value) * weight;
                    }
                }
            }
            premul_from_accumulated_channels(channels)
        }
    }
}

fn sample_external_cover_at_device(
    image: &ReferenceImage,
    sample: ExternalSample,
    clip_rect: valle_draw::Rect,
    inverse: [f64; 9],
    device: [f64; 2],
) -> Result<PremulRgba32, ReferenceExecuteError> {
    let local = project_homography(inverse, device)
        .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)?;
    let content = [
        (local[0].clamp(clip_rect.left(), clip_rect.right()) - clip_rect.x) / clip_rect.width,
        (local[1].clamp(clip_rect.top(), clip_rect.bottom()) - clip_rect.y) / clip_rect.height,
    ];
    sample_external_content(image, sample, content)
}

fn sample_external_at_device(
    image: &ReferenceImage,
    placement: ExternalPlacement,
    inverse: [f64; 9],
    device: [f64; 2],
) -> Result<PremulRgba32, ReferenceExecuteError> {
    let local = project_homography(inverse, device)
        .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)?;
    if !contains_half_open(placement.clip_rect, local)
        || !contains_half_open(placement.content_rect, local)
    {
        return Ok(PremulRgba32::TRANSPARENT);
    }
    let content = [
        (local[0] - placement.content_rect.x) / placement.content_rect.width,
        (local[1] - placement.content_rect.y) / placement.content_rect.height,
    ];
    sample_external_content(image, placement.sample, content)
}

fn sample_external_content(
    image: &ReferenceImage,
    sample: ExternalSample,
    content: [f64; 2],
) -> Result<PremulRgba32, ReferenceExecuteError> {
    let ExternalSample::Texture {
        texture_from_content,
        input_sample_bounds,
        ..
    } = sample
    else {
        return Ok(PremulRgba32::TRANSPARENT);
    };
    sample_bilinear_clamped(image, texture_from_content, input_sample_bounds, content)
}

fn sample_program_image_content(
    image: &ReferenceImage,
    sample: ExternalSample,
    sampling: SamplingMode,
    content: [f64; 2],
) -> Result<PremulRgba32, ReferenceExecuteError> {
    let ExternalSample::Texture {
        texture_from_content,
        input_sample_bounds,
        ..
    } = sample
    else {
        return Ok(PremulRgba32::TRANSPARENT);
    };
    match sampling {
        SamplingMode::NearestClamp => {
            sample_nearest_clamped(image, texture_from_content, input_sample_bounds, content)
        }
        SamplingMode::LinearClamp => {
            sample_bilinear_clamped(image, texture_from_content, input_sample_bounds, content)
        }
        SamplingMode::LinearDecal => {
            sample_bilinear_decal(image, texture_from_content, input_sample_bounds, content)
        }
        SamplingMode::CubicClamp => {
            sample_bilinear_clamped(image, texture_from_content, input_sample_bounds, content)
        }
    }
}

fn external_visual<'a>(
    template: &RenderPlanTemplate,
    bound: &BoundExternalObjects<'a, ReferenceExternalObject>,
    resource: PlanResourceId,
) -> Result<&'a ReferenceImage, ReferenceExecuteError> {
    let Some(plan_resource) = template.resources().get(resource.index()) else {
        return Err(ReferenceExecuteError::InvalidExternalResource { resource });
    };
    let PlanResourceKind::External { slot } = &plan_resource.kind else {
        return Err(ReferenceExecuteError::InvalidExternalResource { resource });
    };
    bound
        .get(*slot)
        .and_then(ReferenceExternalObject::visual_image)
        .ok_or(ReferenceExecuteError::InvalidExternalResource { resource })
}

fn dynamic_transform(
    bindings: &RenderBindings,
    id: DynamicBindingId,
) -> Result<DeviceTransform, ReferenceExecuteError> {
    let Some(binding) = bindings.dynamic().get(id) else {
        return Err(ReferenceExecuteError::InvalidDynamicBinding { binding: id });
    };
    if binding.binding_kind != DynamicBindingKind::DeviceTransform {
        return Err(ReferenceExecuteError::InvalidDynamicBinding { binding: id });
    }
    match &binding.value {
        DynamicValue::DeviceTransform(value) => Ok(*value),
        _ => Err(ReferenceExecuteError::InvalidDynamicBinding { binding: id }),
    }
}

fn dynamic_bounds(
    bindings: &RenderBindings,
    id: DynamicBindingId,
) -> Result<DeviceRect, ReferenceExecuteError> {
    dynamic_bounds_kind(bindings, id, DynamicBindingKind::Bounds)
}

fn dynamic_bounds_kind(
    bindings: &RenderBindings,
    id: DynamicBindingId,
    expected: DynamicBindingKind,
) -> Result<DeviceRect, ReferenceExecuteError> {
    let Some(binding) = bindings.dynamic().get(id) else {
        return Err(ReferenceExecuteError::InvalidDynamicBinding { binding: id });
    };
    if binding.binding_kind != expected {
        return Err(ReferenceExecuteError::InvalidDynamicBinding { binding: id });
    }
    match &binding.value {
        DynamicValue::Bounds(value) => Ok(*value),
        _ => Err(ReferenceExecuteError::InvalidDynamicBinding { binding: id }),
    }
}

fn dynamic_scalar(
    bindings: &RenderBindings,
    id: DynamicBindingId,
) -> Result<f32, ReferenceExecuteError> {
    dynamic_scalar_kind(bindings, id, DynamicBindingKind::Opacity)
}

fn dynamic_scalar_kind(
    bindings: &RenderBindings,
    id: DynamicBindingId,
    expected: DynamicBindingKind,
) -> Result<f32, ReferenceExecuteError> {
    let Some(binding) = bindings.dynamic().get(id) else {
        return Err(ReferenceExecuteError::InvalidDynamicBinding { binding: id });
    };
    if binding.binding_kind != expected {
        return Err(ReferenceExecuteError::InvalidDynamicBinding { binding: id });
    }
    match &binding.value {
        DynamicValue::Scalar(value) => Ok(*value as f32),
        _ => Err(ReferenceExecuteError::InvalidDynamicBinding { binding: id }),
    }
}

fn contains_half_open(rect: valle_draw::Rect, point: [f64; 2]) -> bool {
    point[0] >= rect.left()
        && point[1] >= rect.top()
        && point[0] < rect.right()
        && point[1] < rect.bottom()
}

fn multiply_homography(left: [f64; 9], right: [f64; 9]) -> [f64; 9] {
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
    if determinant == 0.0 || !determinant.is_finite() {
        return None;
    }
    let inverse = cofactors.map(|value| value / determinant);
    inverse
        .iter()
        .all(|value| value.is_finite())
        .then_some(inverse)
}

fn project_homography(matrix: [f64; 9], point: [f64; 2]) -> Option<[f64; 2]> {
    let denominator = matrix[6] * point[0] + matrix[7] * point[1] + matrix[8];
    if denominator == 0.0 || !denominator.is_finite() {
        return None;
    }
    let result = [
        (matrix[0] * point[0] + matrix[1] * point[1] + matrix[2]) / denominator,
        (matrix[3] * point[0] + matrix[4] * point[1] + matrix[5]) / denominator,
    ];
    result
        .iter()
        .all(|value| value.is_finite())
        .then_some(result)
}

fn sample_bilinear_clamped(
    image: &ReferenceImage,
    texture_from_content: [f64; 9],
    sample_bounds: valle_draw::Rect,
    content: [f64; 2],
) -> Result<PremulRgba32, ReferenceExecuteError> {
    let texture = project_homography(texture_from_content, content)
        .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)?;
    let x = sample_axis(
        texture[0],
        sample_bounds.left(),
        sample_bounds.right(),
        image.extent().width(),
    )?;
    let y = sample_axis(
        texture[1],
        sample_bounds.top(),
        sample_bounds.bottom(),
        image.extent().height(),
    )?;
    let x0_floor = x.coordinate.floor();
    let y0_floor = y.coordinate.floor();
    let tx = x.coordinate - x0_floor;
    let ty = y.coordinate - y0_floor;
    let x0 = (x0_floor as i64).clamp(x.first_texel, x.last_texel) as u32;
    let y0 = (y0_floor as i64).clamp(y.first_texel, y.last_texel) as u32;
    let x1 = (x0_floor as i64 + 1).clamp(x.first_texel, x.last_texel) as u32;
    let y1 = (y0_floor as i64 + 1).clamp(y.first_texel, y.last_texel) as u32;
    let samples = [
        image.pixel(x0, y0).expect("clamped sample is in bounds"),
        image.pixel(x1, y0).expect("clamped sample is in bounds"),
        image.pixel(x0, y1).expect("clamped sample is in bounds"),
        image.pixel(x1, y1).expect("clamped sample is in bounds"),
    ];
    let top = lerp_channels(samples[0].channels(), samples[1].channels(), tx);
    let bottom = lerp_channels(samples[2].channels(), samples[3].channels(), tx);
    PremulRgba32::from_premultiplied(lerp_channels(top, bottom, ty)).map_err(Into::into)
}

fn sample_nearest_clamped(
    image: &ReferenceImage,
    texture_from_content: [f64; 9],
    sample_bounds: valle_draw::Rect,
    content: [f64; 2],
) -> Result<PremulRgba32, ReferenceExecuteError> {
    let texture = project_homography(texture_from_content, content)
        .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)?;
    let x = sample_axis(
        texture[0],
        sample_bounds.left(),
        sample_bounds.right(),
        image.extent().width(),
    )?;
    let y = sample_axis(
        texture[1],
        sample_bounds.top(),
        sample_bounds.bottom(),
        image.extent().height(),
    )?;
    let x = x
        .coordinate
        .round()
        .clamp(x.first_texel as f64, x.last_texel as f64) as u32;
    let y = y
        .coordinate
        .round()
        .clamp(y.first_texel as f64, y.last_texel as f64) as u32;
    Ok(image.pixel(x, y).expect("clamped sample is in bounds"))
}

fn sample_bilinear_decal(
    image: &ReferenceImage,
    texture_from_content: [f64; 9],
    sample_bounds: valle_draw::Rect,
    content: [f64; 2],
) -> Result<PremulRgba32, ReferenceExecuteError> {
    let texture = project_homography(texture_from_content, content)
        .ok_or(ReferenceExecuteError::InvalidSampleCoordinate)?;
    if !texture.iter().all(|value| value.is_finite()) || sample_bounds.is_empty() {
        return Err(ReferenceExecuteError::InvalidSampleCoordinate);
    }
    let width = f64::from(image.extent().width());
    let height = f64::from(image.extent().height());
    let x = texture[0] * width - 0.5;
    let y = texture[1] * height - 0.5;
    if !x.is_finite() || !y.is_finite() {
        return Err(ReferenceExecuteError::InvalidSampleCoordinate);
    }
    let x0 = x.floor();
    let y0 = y.floor();
    let tx = x - x0;
    let ty = y - y0;
    let x0 = x0 as i64;
    let y0 = y0 as i64;
    let samples = [
        decal_texel(image, sample_bounds, x0, y0),
        decal_texel(image, sample_bounds, x0 + 1, y0),
        decal_texel(image, sample_bounds, x0, y0 + 1),
        decal_texel(image, sample_bounds, x0 + 1, y0 + 1),
    ];
    let top = lerp_channels(samples[0].channels(), samples[1].channels(), tx);
    let bottom = lerp_channels(samples[2].channels(), samples[3].channels(), tx);
    PremulRgba32::from_premultiplied(lerp_channels(top, bottom, ty)).map_err(Into::into)
}

fn decal_texel(
    image: &ReferenceImage,
    sample_bounds: valle_draw::Rect,
    x: i64,
    y: i64,
) -> PremulRgba32 {
    let Ok(x) = u32::try_from(x) else {
        return PremulRgba32::TRANSPARENT;
    };
    let Ok(y) = u32::try_from(y) else {
        return PremulRgba32::TRANSPARENT;
    };
    if x >= image.extent().width() || y >= image.extent().height() {
        return PremulRgba32::TRANSPARENT;
    }
    let center = [
        (f64::from(x) + 0.5) / f64::from(image.extent().width()),
        (f64::from(y) + 0.5) / f64::from(image.extent().height()),
    ];
    if !contains_half_open(sample_bounds, center) {
        return PremulRgba32::TRANSPARENT;
    }
    image
        .pixel(x, y)
        .expect("validated decal texel is in bounds")
}

fn sample_axis(
    value: f64,
    start: f64,
    end: f64,
    texels: u32,
) -> Result<SampleAxis, ReferenceExecuteError> {
    if ![value, start, end].iter().all(|value| value.is_finite()) || start >= end {
        return Err(ReferenceExecuteError::InvalidSampleCoordinate);
    }
    let texel_count = i64::from(texels);
    let texels = f64::from(texels);
    let lower = start + 0.5 / texels;
    let upper = end - 0.5 / texels;
    let center = if lower <= upper {
        value.clamp(lower, upper)
    } else {
        (start + end) * 0.5
    };
    let coordinate = center * texels - 0.5;
    // Clamp each raw linear-filter tap, not just the center coordinate. A sub-texel crop can have
    // no texel center of its own; deriving the second tap from an already-clamped first tap would
    // then reintroduce a neighbor whose texel cell is completely outside the declared domain.
    let first_texel = (start * texels).floor() as i64;
    let last_texel = (end * texels).ceil() as i64 - 1;
    let maximum = texel_count - 1;
    let first_texel = first_texel.clamp(0, maximum);
    let last_texel = last_texel.clamp(0, maximum);
    if !coordinate.is_finite() || first_texel > last_texel {
        return Err(ReferenceExecuteError::InvalidSampleCoordinate);
    }
    Ok(SampleAxis {
        coordinate,
        first_texel,
        last_texel,
    })
}

#[derive(Debug, Clone, Copy)]
struct SampleAxis {
    coordinate: f64,
    first_texel: i64,
    last_texel: i64,
}

fn lerp_channels(left: [f32; 4], right: [f32; 4], amount: f64) -> [f32; 4] {
    std::array::from_fn(|index| {
        (f64::from(left[index]) * (1.0 - amount) + f64::from(right[index]) * amount) as f32
    })
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ReferenceObjectError {
    #[error("reference visual object requires a visual ResourceKey")]
    VisualKeyRequired,
    #[error("reference font object requires a font-face ResourceKey")]
    FontKeyRequired,
    #[error("reference font bytes digest mismatch: expected {expected}, got {actual}")]
    FontDigestMismatch {
        expected: crate::resource::ContentDigest,
        actual: crate::resource::ContentDigest,
    },
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ReferenceExecuteError {
    #[error(transparent)]
    Bind(#[from] ExternalBindError),
    #[error(transparent)]
    Pixel(#[from] PixelError),
    #[error(transparent)]
    Output(#[from] OutputMathError),
    #[error(transparent)]
    Composite(#[from] CompositeError),
    #[error(transparent)]
    Blend(#[from] BlendError),
    #[error(transparent)]
    Transition(#[from] ReferenceTransitionError),
    #[error(transparent)]
    ProgramBinding(#[from] ProgramBindingError),
    #[error("reference target extent mismatch: expected {expected:?}, got {actual:?}")]
    TargetExtentMismatch {
        expected: Extent2d,
        actual: Extent2d,
    },
    #[error("reference target OutputSpec does not match the plan")]
    TargetSpecMismatch,
    #[error("render extent is invalid")]
    InvalidRenderExtent,
    #[error("bound external objects belong to another template")]
    BoundTemplateMismatch,
    #[error("reference executor does not support pass {pass:?}")]
    UnsupportedPass { pass: ExecutionPassId },
    #[error("extension implementation digest is not executable in pass {pass:?}")]
    ExtensionImplementationMismatch { pass: ExecutionPassId },
    #[error("reference kernel sample budget exceeded in pass {pass:?}: {required} > {maximum}")]
    KernelSampleBudgetExceeded {
        pass: ExecutionPassId,
        required: u64,
        maximum: u64,
    },
    #[error("reference raster sample budget exceeded in pass {pass:?}: {required} > {maximum}")]
    RasterSampleBudgetExceeded {
        pass: ExecutionPassId,
        required: u64,
        maximum: u64,
    },
    #[error(
        "reference raster geometry-test budget exceeded in pass {pass:?}: {required} > {maximum}"
    )]
    RasterGeometryBudgetExceeded {
        pass: ExecutionPassId,
        required: u64,
        maximum: u64,
    },
    #[error("reference outline segment budget exceeded in pass {pass:?}: {required} > {maximum}")]
    OutlineSegmentBudgetExceeded {
        pass: ExecutionPassId,
        required: u64,
        maximum: u64,
    },
    #[error("reference import resource {resource:?} is not a bound visual object")]
    InvalidExternalResource { resource: PlanResourceId },
    #[error("reference DrawProgram {program:?} is missing or invalid")]
    InvalidProgram { program: ProgramId },
    #[error("reference DrawProgram {program:?} has no prepared raster operation for node {node:?}")]
    InvalidProgramNode { program: ProgramId, node: NodeId },
    #[error("reference DrawProgram {program:?} has no prepared clip operation for node {node:?}")]
    InvalidProgramClip { program: ProgramId, node: NodeId },
    #[error(
        "reference DrawProgram {program:?} has no prepared filter {filter_index} for node {node:?}"
    )]
    InvalidProgramFilter {
        program: ProgramId,
        node: NodeId,
        filter_index: u32,
    },
    #[error("reference DrawProgram {program:?} has no prepared mask operation for node {node:?}")]
    InvalidProgramMask { program: ProgramId, node: NodeId },
    #[error("reference DrawProgram {program:?} has no bound schedule for pass {pass:?}")]
    InvalidProgramSchedule {
        program: ProgramId,
        pass: ExecutionPassId,
    },
    #[error("reference DrawProgram {program:?} has no valid visual texture binding for {key:?}")]
    InvalidProgramTexture { program: ProgramId, key: String },
    #[error("reference DrawProgram {program:?} texture slot {slot:?} is not a bound visual object")]
    InvalidProgramTextureSlot {
        program: ProgramId,
        slot: ExternalSlotId,
    },
    #[error("reference DrawProgram {program:?} has no matching font object for face {face_index}")]
    InvalidFontResource { program: ProgramId, face_index: u32 },
    #[error("reference DrawProgram {program:?} font face {face_index} is invalid")]
    InvalidFontFace { program: ProgramId, face_index: u32 },
    #[error("reference DrawProgram {program:?} contains invalid glyph id {glyph}")]
    InvalidGlyph { program: ProgramId, glyph: u32 },
    #[error(
        "reference DrawProgram {program:?} glyph {glyph} requires unsupported color/image semantics"
    )]
    UnsupportedColorGlyph { program: ProgramId, glyph: u32 },
    #[error("reference DrawProgram {program:?} glyph {glyph} has no valid bounded outline")]
    InvalidGlyphOutline { program: ProgramId, glyph: u32 },
    #[error(
        "reference DrawProgram {program:?} glyph outline exceeds its declared conservative bounds"
    )]
    GlyphBoundsMismatch { program: ProgramId },
    #[error("reference dynamic binding {binding:?} is missing or has the wrong value kind")]
    InvalidDynamicBinding { binding: DynamicBindingId },
    #[error("reference transform binding {binding:?} is non-invertible for a non-empty region")]
    NonInvertibleTransform { binding: DynamicBindingId },
    #[error("reference import produced a non-finite or undefined sample coordinate")]
    InvalidSampleCoordinate,
    #[error("reference effect bounds are outside the admitted root coordinate domain")]
    InvalidEffectBounds,
    #[error("reference mask payload is not executable")]
    InvalidMask,
    #[error("backdrop output binding {output:?} is not contained by sample binding {sample:?}")]
    InvalidBackdropBounds {
        sample: DynamicBindingId,
        output: DynamicBindingId,
    },
    #[error("reference resource {resource:?} was read before materialization")]
    MissingResource { resource: PlanResourceId },
    #[error("plan wrote non-terminal output resource {resource:?}")]
    WrongOutputResource { resource: PlanResourceId },
    #[error("reference execution completed without an output frame")]
    MissingOutput,
}

#[cfg(test)]
mod tests {
    use valle_draw::Rect;

    use super::*;

    #[test]
    fn extension_color_gain_rejects_an_unbound_implementation_digest() {
        let pass = ExecutionPassId::try_from(1_u32).unwrap();
        let kernel = PreparedEffectKernel::ExtensionColorGain {
            implementation_sha256: [2; 32],
            gain: 1.0,
            past_frames: 0,
            future_frames: 0,
        };
        let bindings = RenderBindings::new(
            valle_timeline::internal::RenderId::from_bytes([0; 32]),
            crate::resource::ContentDigest::from_bytes([0; 32]),
            crate::prepare::DynamicBindings::default(),
            crate::resource::ExternalGeneration::new(1).unwrap(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let effect = PlanEffect {
            semantic_path: "test.extensionColorGain".to_owned(),
            space: PreparedEffectSpace::Root,
            kernel,
        };
        assert!(matches!(
            prepare_reference_effect_runtime(pass, &effect, &bindings),
            Err(ReferenceExecuteError::ExtensionImplementationMismatch {
                pass: rejected_pass
            }) if rejected_pass == pass
        ));
    }

    #[test]
    fn font_object_rehashes_bytes_and_freezes_face_identity() {
        let bytes = b"deterministic reference font".to_vec();
        let digest = crate::resource::ContentDigest::of_bytes(bytes.as_slice());
        let key = ResourceKey::new(
            digest.clone(),
            ResourceInterpretation::FontFace { face_index: 3 },
        );
        let object = ReferenceExternalObject::font_bytes(key.clone(), bytes.clone()).unwrap();

        assert_eq!(object.key(), &key);
        assert_eq!(object.descriptor(), &ExternalResourceDesc::FontBytes);
        assert_eq!(object.font_data(), Some(bytes.as_slice()));
        assert!(object.visual_image().is_none());

        let wrong_kind = ResourceKey::new(
            digest.clone(),
            ResourceInterpretation::Scene3d {
                topology_digest: digest,
            },
        );
        assert_eq!(
            ReferenceExternalObject::font_bytes(wrong_kind, bytes.clone()).unwrap_err(),
            ReferenceObjectError::FontKeyRequired
        );
        let wrong_digest = ResourceKey::new(
            crate::resource::ContentDigest::from_bytes([0; 32]),
            ResourceInterpretation::FontFace { face_index: 3 },
        );
        assert!(matches!(
            ReferenceExternalObject::font_bytes(wrong_digest, bytes),
            Err(ReferenceObjectError::FontDigestMismatch { .. })
        ));
    }

    #[test]
    fn narrow_crop_clamps_each_linear_tap_to_the_declared_domain() {
        let extent = Extent2d::new(2, 1).unwrap();
        let red = PremulRgba32::from_straight([1.0, 0.0, 0.0], 1.0).unwrap();
        let green = PremulRgba32::from_straight([0.0, 1.0, 0.0], 1.0).unwrap();
        let image = ReferenceImage::new(extent, vec![red, green]).unwrap();

        let sampled = sample_bilinear_clamped(
            &image,
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            Rect::new(0.0, 0.0, 0.1, 1.0),
            [0.05, 0.5],
        )
        .unwrap();

        assert_eq!(sampled, red, "the excluded green texel must not bleed in");
    }

    #[test]
    fn resolve_copy_is_transparent_outside_the_declared_sample_roi() {
        let extent = Extent2d::new(3, 2).unwrap();
        let red = PremulRgba32::from_straight([1.0, 0.0, 0.0], 1.0).unwrap();
        let image = ReferenceImage::solid(extent, red).unwrap();

        let copied = copy_region(&image, DeviceRect::new(1, 0, 1, 2), extent).unwrap();

        assert_eq!(copied.pixel(1, 0), Some(red));
        assert_eq!(copied.pixel(1, 1), Some(red));
        assert_eq!(copied.pixel(0, 0), Some(PremulRgba32::TRANSPARENT));
        assert_eq!(copied.pixel(2, 1), Some(PremulRgba32::TRANSPARENT));
    }

    #[test]
    fn backdrop_rect_coverage_uses_the_program_transform_and_half_open_bounds() {
        let viewport = Rect::new(0.0, 0.0, 2.0, 1.0);
        let device_to_normalized = [0.5, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

        assert_eq!(
            program_rect_coverage(
                Rect::new(0.0, 0.0, 0.5, 1.0),
                viewport,
                device_to_normalized,
                0,
                0,
            )
            .unwrap(),
            0.5
        );
        assert_eq!(
            program_rect_coverage(
                Rect::new(0.0, 0.0, 1.0, 1.0),
                viewport,
                device_to_normalized,
                1,
                0,
            )
            .unwrap(),
            0.0
        );
    }

    #[test]
    fn truncated_gaussian_profile_is_symmetric_and_normalized() {
        let tail = normal_cdf(-MASK_GAUSSIAN_SUPPORT_SIGMAS);
        let normalize =
            |distance: f64| ((normal_cdf(-distance) - tail) / (1.0 - 2.0 * tail)).clamp(0.0, 1.0);

        assert!((normalize(0.0) - 0.5).abs() < 1.0e-12);
        assert!((normalize(-1.25) + normalize(1.25) - 1.0).abs() < 1.0e-12);
        assert_eq!(normalize(-MASK_GAUSSIAN_SUPPORT_SIGMAS), 1.0);
        assert_eq!(normalize(MASK_GAUSSIAN_SUPPORT_SIGMAS), 0.0);
    }

    #[test]
    fn raster_sample_budget_is_cumulative_and_overflow_closed() {
        let pass = ExecutionPassId::try_from(1).unwrap();
        let samples_per_pixel =
            u64::from(PROGRAM_COVERAGE_SAMPLES_PER_AXIS * PROGRAM_COVERAGE_SAMPLES_PER_AXIS);
        let mut samples = 0;
        reserve_raster_samples(
            pass,
            MAX_REFERENCE_RASTER_SAMPLES / samples_per_pixel,
            1,
            samples_per_pixel,
            &mut samples,
        )
        .unwrap();
        assert_eq!(samples, MAX_REFERENCE_RASTER_SAMPLES);
        assert!(matches!(
            reserve_raster_samples(pass, 1, 1, samples_per_pixel, &mut samples),
            Err(ReferenceExecuteError::RasterSampleBudgetExceeded { .. })
        ));
        let mut overflow = 0;
        assert!(matches!(
            reserve_raster_samples(pass, u64::MAX, 2, samples_per_pixel, &mut overflow),
            Err(ReferenceExecuteError::RasterSampleBudgetExceeded {
                required: u64::MAX,
                ..
            })
        ));
    }

    #[test]
    fn raster_tree_keeps_nested_coordinates_and_translucent_painter_order() {
        use valle_draw::program::{DrawProgramBuilder, Group, LinearColor, PathNode};
        let pass = ExecutionPassId::try_from(1).unwrap();
        let viewport = Rect::new(0.0, 0.0, 32.0, 32.0);
        let mut builder = DrawProgramBuilder::new(viewport);
        let mut children = Vec::new();
        for (x, color) in [
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
            let leaf = builder.push_node(Node::Path(PathNode {
                path,
                fill_rule: Default::default(),
                fill: Some(paint),
                stroke: None,
            }));
            let mut group = Group::plain(vec![leaf]);
            group.transform = Transform2d([1.0, 0.0, x, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
            children.push(builder.push_node(Node::Group(group)));
        }
        let mut outer = Group::plain(children);
        outer.transform = Transform2d([2.0, 0.0, 3.0, 0.0, 2.0, 5.0, 0.0, 0.0, 1.0]);
        let root = builder.push_node(Node::Group(outer));
        builder.add_root(root);
        let program = builder.finish().unwrap();
        let draws =
            collect_reference_raster_tree(&program, program.roots(), Transform2d::IDENTITY, pass)
                .unwrap();
        let operations = draws
            .iter()
            .filter_map(ReferenceTreeStep::draw)
            .map(|(node, transform)| {
                let Node::Path(node) = &program.nodes()[node.raw() as usize] else {
                    panic!("leaf")
                };
                let segments =
                    line_path_segments(&program.paths()[node.path.raw() as usize]).unwrap();
                let fill = ReferenceFill {
                    shapes: vec![ReferenceShape {
                        bounds: line_segments_bounds(&segments).unwrap(),
                        segments,
                    }],
                    color: solid_program_paint(&program, node.fill.unwrap(), pass).unwrap(),
                };
                (
                    fill,
                    normalized_program_matrices(transform.0, viewport)
                        .unwrap()
                        .1,
                )
            })
            .collect::<Vec<_>>();
        for (x, y, expected) in [
            (4, 6, [1.0, 0.0, 0.0, 1.0]),
            (12, 6, [0.5, 0.0, 0.5, 1.0]),
            (22, 6, [0.0, 0.0, 0.5, 0.5]),
            (2, 6, [0.0; 4]),
            (4, 22, [0.0; 4]),
        ] {
            let mut pixel = PremulRgba32::TRANSPARENT;
            for (fill, inverse) in &operations {
                pixel = ReferenceRasterOperation::Fill(fill)
                    .pixel(viewport, *inverse, x, y)
                    .unwrap()
                    .source_over(pixel)
                    .unwrap();
            }
            assert!(
                pixel.approx_eq(PremulRgba32::from_premultiplied(expected).unwrap(), 1e-6),
                "pixel ({x}, {y}) = {pixel:?}"
            );
        }
    }

    #[test]
    fn fused_raster_tree_roots_consume_the_shared_coverage_budget_individually() {
        let pass = ExecutionPassId::try_from(1).unwrap();
        let program = ReferenceProgramRuntime {
            id: ProgramId::try_from(1).unwrap(),
            viewport: Rect::new(0.0, 0.0, 1.0, 1.0),
            nodes: Vec::new(),
            raster_trees: BTreeMap::new(),
            clips: Vec::new(),
            filters: BTreeMap::new(),
            masks: BTreeMap::new(),
        };
        let operation = ReferenceProgramOperation::Fill(ReferenceFill {
            shapes: Vec::new(),
            color: PremulRgba32::TRANSPARENT,
        });
        let output_roi = DeviceRect::new(0, 0, 4096, 2048);
        let extent = Extent2d::new(4096, 2048).unwrap();
        let transform = DeviceTransform::from_affine([1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
        let samples_per_pixel =
            u64::from(PROGRAM_COVERAGE_SAMPLES_PER_AXIS * PROGRAM_COVERAGE_SAMPLES_PER_AXIS);
        let mut budget = ReferenceRasterBudget::default();

        preflight_program_raster_operation(
            &program,
            &operation,
            output_roi,
            transform,
            extent,
            pass,
            samples_per_pixel,
            &mut budget,
        )
        .unwrap();
        assert_eq!(budget.coverage_samples, MAX_REFERENCE_RASTER_SAMPLES);
        assert!(matches!(
            preflight_program_raster_operation(
                &program,
                &operation,
                output_roi,
                transform,
                extent,
                pass,
                samples_per_pixel,
                &mut budget,
            ),
            Err(ReferenceExecuteError::RasterSampleBudgetExceeded { .. })
        ));
    }

    #[test]
    fn raster_geometry_and_outline_budgets_are_cumulative_and_overflow_closed() {
        let pass = ExecutionPassId::try_from(1).unwrap();
        let samples_per_pixel =
            u64::from(PROGRAM_COVERAGE_SAMPLES_PER_AXIS * PROGRAM_COVERAGE_SAMPLES_PER_AXIS);
        let mut geometry = 0;
        reserve_raster_geometry_tests(
            pass,
            MAX_REFERENCE_RASTER_GEOMETRY_TESTS / samples_per_pixel,
            1,
            samples_per_pixel,
            &mut geometry,
        )
        .unwrap();
        assert_eq!(geometry, MAX_REFERENCE_RASTER_GEOMETRY_TESTS);
        assert!(matches!(
            reserve_raster_geometry_tests(pass, 1, 1, samples_per_pixel, &mut geometry),
            Err(ReferenceExecuteError::RasterGeometryBudgetExceeded { .. })
        ));

        let mut segments = MAX_REFERENCE_OUTLINE_SEGMENTS;
        assert!(matches!(
            reserve_outline_segments(pass, 1, &mut segments),
            Err(ReferenceExecuteError::OutlineSegmentBudgetExceeded { .. })
        ));
    }

    #[test]
    fn glyph_curve_flattening_preserves_collinear_reversals() {
        let mut builder = GlyphOutlineBuilder::new([0.0, 0.0], 1.0, 0.001, 4096);
        builder.move_to(0.0, 0.0);
        builder.curve_to(2.0, 0.0, -1.0, 0.0, 1.0, 0.0);
        builder.close();

        let segments = builder.finish().unwrap();
        assert!(
            segments.len() > 2,
            "controls outside the finite chord must not collapse to one edge"
        );
    }
}
