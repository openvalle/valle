use std::collections::BTreeMap;

use skia_safe::{BlendMode, Color4f, Image, Paint, Surface};
use thiserror::Error;
use valle_engine::{
    compositor::{
        BoundExternalObjects, ExternalBindError, bind_external_objects,
        lower::{
            BackendCapabilities, BoundProgramSchedule, BoundProgramSchedules, CompositeMode,
            CopyOperation, ExecutionPassId, ExecutionPassKind, KernelInvocation, PlanProgram,
            PlanResourceId, PlanResourceKind, ProgramBindingError, ProgramPassKind, RenderBindings,
            RenderPlanTemplate, SurfaceSlotId,
        },
    },
    prepare::{
        DeviceRect, DeviceTransform, DynamicBindingId, DynamicBindingKind, DynamicValue, ProgramId,
    },
    resource::Extent2d,
};

use super::{
    SkiaExternalObject, SkiaObjectTable, SkiaTarget,
    blend::BlendRuntime,
    cache::{BackendCacheCounters, BackendCaches},
    draw::{DrawError, ProgramRuntime, ProgramTerminal},
    effect::{EffectCacheCounters, EffectRuntime, apply_prepared_mask, straight_color},
    import::render_import,
    output::{prefers_cached_output, stage_output, stage_sdr_output, validate_target},
    surface::{
        PlanImage, SurfaceAllocationScope, SurfaceArena, SurfaceError, SurfaceFrame,
        SurfaceFrameReport, working_color_space, working_info,
    },
};

/// Sealed executor instance. Capability identity is immutable for its lifetime and is rechecked
/// against every template during admission.
#[derive(Debug)]
pub struct SkiaExecutor {
    capabilities: BackendCapabilities,
    effects: EffectRuntime,
    caches: BackendCaches,
    surfaces: SurfaceArena,
}

impl SkiaExecutor {
    pub fn new(capabilities: BackendCapabilities) -> Result<Self, SkiaExecuteError> {
        capabilities.validate()?;
        Ok(Self {
            capabilities,
            effects: EffectRuntime::new(),
            caches: BackendCaches::new(),
            surfaces: SurfaceArena::new(),
        })
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub fn metal(capabilities: BackendCapabilities) -> Result<Self, SkiaExecuteError> {
        capabilities.validate()?;
        Ok(Self {
            capabilities,
            effects: EffectRuntime::new(),
            caches: BackendCaches::new(),
            surfaces: SurfaceArena::metal()?,
        })
    }

    pub const fn capabilities(&self) -> &BackendCapabilities {
        &self.capabilities
    }

    pub const fn backend_kind(&self) -> super::surface::SkiaBackendKind {
        self.surfaces.backend_kind()
    }

    pub const fn execution_profile(&self) -> super::surface::SkiaExecutionProfile {
        super::surface::SkiaExecutionProfile::native(self.backend_kind())
    }

    /// Drops every reusable backend surface. CPU hosts call this for an explicit device/context
    /// loss; extent or output-contract changes invalidate the generation automatically.
    pub fn invalidate_surface_generation(&mut self) {
        self.surfaces.invalidate();
    }

    pub fn admit<'a>(
        &mut self,
        template: &'a RenderPlanTemplate,
        bindings: &'a RenderBindings,
        objects: &'a SkiaObjectTable,
        target: &SkiaTarget<'_>,
    ) -> Result<AdmittedSkiaFrame<'a>, SkiaExecuteError> {
        let effect_before = self.effects.counters();
        self.effects.admit_template(template, bindings)?;
        admit_skia_frame_with_effects(
            template,
            bindings,
            &self.capabilities,
            objects,
            target,
            self.effects.clone(),
            &mut self.caches,
            self.effects.counters().since(effect_before),
        )
    }

    pub fn execute(
        &mut self,
        template: &RenderPlanTemplate,
        bindings: &RenderBindings,
        objects: &SkiaObjectTable,
        target: &mut SkiaTarget<'_>,
    ) -> Result<SkiaExecutionReport, SkiaExecuteError> {
        let admitted = self.admit(template, bindings, objects, target)?;
        admitted.execute_with_arena(target, &mut self.surfaces)
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn acquire_shared_frame(
        &mut self,
        pool: &valle_media::SharedVideoFramePool,
        output: valle_engine::resource::OutputSpec,
    ) -> Result<super::surface::SharedMetalFrame, SkiaExecuteError> {
        Ok(self.surfaces.acquire_shared_frame(pool, output)?)
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn submit_shared_frame(
        &mut self,
        target: super::surface::SharedMetalFrame,
    ) -> Result<super::surface::SubmittedSharedMetalFrame, SkiaExecuteError> {
        Ok(self.surfaces.submit_shared_frame(target)?)
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn finish_shared_frame(
        &mut self,
        submitted: super::surface::SubmittedSharedMetalFrame,
    ) -> Result<valle_media::SharedVideoFrame, SkiaExecuteError> {
        Ok(self.surfaces.finish_shared_frame(submitted)?)
    }
}

pub fn execute_skia(
    template: &RenderPlanTemplate,
    bindings: &RenderBindings,
    capabilities: &BackendCapabilities,
    objects: &SkiaObjectTable,
    target: &mut SkiaTarget<'_>,
) -> Result<SkiaExecutionReport, SkiaExecuteError> {
    admit_skia_frame(template, bindings, capabilities, objects, target)?.execute(target)
}

/// Complete immutable admission proof. Construction performs every check that may fail before
/// scratch allocation; execution owns private surfaces and commits exactly once.
#[derive(Debug)]
pub struct AdmittedSkiaFrame<'a> {
    template: &'a RenderPlanTemplate,
    bindings: &'a RenderBindings,
    bound: BoundExternalObjects<'a, SkiaExternalObject>,
    schedules: BoundProgramSchedules,
    programs: BTreeMap<ProgramId, ProgramRuntime>,
    effects: EffectRuntime,
    blend: Option<BlendRuntime>,
    cache_counters: BackendCacheCounters,
    effect_cache_counters: EffectCacheCounters,
    max_surface_bytes: u64,
    max_frame_bytes: u64,
}

pub fn admit_skia_frame<'a>(
    template: &'a RenderPlanTemplate,
    bindings: &'a RenderBindings,
    capabilities: &BackendCapabilities,
    objects: &'a SkiaObjectTable,
    target: &SkiaTarget<'_>,
) -> Result<AdmittedSkiaFrame<'a>, SkiaExecuteError> {
    let effects = EffectRuntime::admit(template, bindings)?;
    let effect_cache_counters = effects.counters();
    let mut caches = BackendCaches::new();
    admit_skia_frame_with_effects(
        template,
        bindings,
        capabilities,
        objects,
        target,
        effects,
        &mut caches,
        effect_cache_counters,
    )
}

fn admit_skia_frame_with_effects<'a>(
    template: &'a RenderPlanTemplate,
    bindings: &'a RenderBindings,
    capabilities: &BackendCapabilities,
    objects: &'a SkiaObjectTable,
    target: &SkiaTarget<'_>,
    effects: EffectRuntime,
    caches: &mut BackendCaches,
    effect_cache_counters: EffectCacheCounters,
) -> Result<AdmittedSkiaFrame<'a>, SkiaExecuteError> {
    preflight_target(template, target)?;
    let schedules = template.bind_program_schedules(bindings, capabilities)?;
    preflight_executor_surface_budget(template, bindings, &schedules, capabilities)?;
    let bound = bind_external_objects(template, bindings, objects)?;
    let expected_hash = template.template_hash().map_err(ExternalBindError::from)?;
    if bound.template_hash() != &expected_hash {
        return Err(SkiaExecuteError::BoundTemplateMismatch);
    }
    let blend = template
        .passes()
        .iter()
        .any(|pass| {
            matches!(
                pass.kind,
                ExecutionPassKind::CompositeRegion {
                    mode: CompositeMode::Blend { mode },
                    ..
                } if mode != valle_draw::program::BlendMode::Normal
            )
        })
        .then(BlendRuntime::admit)
        .transpose()?;
    let mut programs = BTreeMap::new();
    let cache_before = caches.counters();
    for program in bindings.programs() {
        let runtime = ProgramRuntime::admit(program, &bound, caches)?;
        if programs.insert(program.id, runtime).is_some() {
            return Err(SkiaExecuteError::DuplicateProgram {
                program: program.id.get(),
            });
        }
    }
    Ok(AdmittedSkiaFrame {
        template,
        bindings,
        bound,
        schedules,
        programs,
        effects,
        blend,
        cache_counters: caches.counters().since(cache_before),
        effect_cache_counters,
        max_surface_bytes: capabilities.max_surface_bytes(),
        max_frame_bytes: capabilities.max_frame_bytes(),
    })
}

/// Actual work performed by one admitted Skia frame. Cache counters are deltas for this frame,
/// never process-lifetime totals, so preview/export aggregation cannot double count warm hits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkiaExecutionReport {
    pub passes: usize,
    pub programs: usize,
    pub program_cache_hits: u64,
    pub program_cache_misses: u64,
    /// Current retained-program gauges for this executor, not frame deltas or process RSS.
    pub program_cache_entries: usize,
    pub program_cache_cost_bytes: usize,
    pub font_cache_hits: u64,
    pub font_cache_misses: u64,
    pub shader_cache_hits: u64,
    pub shader_cache_misses: u64,
    pub built_in_kernel_cache_hits: u64,
    pub built_in_kernel_cache_misses: u64,
    pub external_gpu_imports: u64,
    pub cpu_upload_bytes: u64,
    pub surfaces: SkiaSurfaceReport,
    pub committed: bool,
}

/// Per-frame physical plan-surface evidence. These counters deliberately describe only storage
/// declared by `RenderPlanTemplate::surface_slots`; program-local and kernel helper scratch are
/// reported separately once admitted by their own schedules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SkiaSurfaceReport {
    pub generation: u64,
    pub generation_invalidations: u64,
    pub plan_slots: usize,
    pub logical_allocations: usize,
    pub backend_allocations: u64,
    pub backend_reuses: u64,
    pub total_surface_allocations: u64,
    pub unpooled_surface_allocations: u64,
    pub scratch_allocations: u64,
    pub scratch_reuses: u64,
    pub scratch_peak_surfaces: usize,
    pub scratch_peak_bytes: u64,
    pub scratch_pool_surfaces: usize,
    pub scratch_pool_bytes: u64,
    pub physical_bytes: u64,
    pub logical_bytes: u64,
    pub alias_saved_bytes: u64,
    pub estimated_peak_bytes: u64,
    pub pool_surfaces: usize,
    pub pool_bytes: u64,
    pub pool_evictions: u64,
}

impl From<SurfaceFrameReport> for SkiaSurfaceReport {
    fn from(value: SurfaceFrameReport) -> Self {
        Self {
            generation: value.generation,
            generation_invalidations: value.generation_invalidations,
            plan_slots: value.plan_slots,
            logical_allocations: value.logical_allocations,
            backend_allocations: value.allocations,
            backend_reuses: value.reuses,
            total_surface_allocations: 0,
            unpooled_surface_allocations: 0,
            scratch_allocations: value.scratch_allocations,
            scratch_reuses: value.scratch_reuses,
            scratch_peak_surfaces: value.scratch_peak_surfaces,
            scratch_peak_bytes: value.scratch_peak_bytes,
            scratch_pool_surfaces: value.scratch_pool_surfaces,
            scratch_pool_bytes: value.scratch_pool_bytes,
            physical_bytes: value.physical_bytes,
            logical_bytes: value.logical_bytes,
            alias_saved_bytes: value.alias_saved_bytes,
            estimated_peak_bytes: value.estimated_peak_bytes,
            pool_surfaces: value.pool_surfaces,
            pool_bytes: value.pool_bytes,
            pool_evictions: value.pool_evictions,
        }
    }
}

impl AdmittedSkiaFrame<'_> {
    /// Executes into executor-private Linear Rec.2020 RGBA16F images. The target is revalidated,
    /// then receives one terminal commit only after the complete closed pass list succeeds.
    pub fn execute(
        self,
        target: &mut SkiaTarget<'_>,
    ) -> Result<SkiaExecutionReport, SkiaExecuteError> {
        let mut arena = SurfaceArena::new();
        self.execute_with_arena(target, &mut arena)
    }

    fn execute_with_arena(
        self,
        target: &mut SkiaTarget<'_>,
        arena: &mut SurfaceArena,
    ) -> Result<SkiaExecutionReport, SkiaExecuteError> {
        preflight_target(self.template, target)?;
        let extent = render_extent(self.template)?;
        let info = working_info(extent)?;
        let root = DeviceRect::full(extent.width(), extent.height());
        let mut surfaces = BTreeMap::<PlanResourceId, PlanImage>::new();
        let allocation_scope = SurfaceAllocationScope::begin()?;
        let mut surface_frame = arena.begin_frame(
            self.template,
            &self.schedules,
            self.max_surface_bytes,
            self.max_frame_bytes,
        )?;
        let external_images = materialize_external_visuals(&self.bound, &mut surface_frame)?;

        // Deep per-pass tracing is intentionally separate from the stable aggregate VALLE_PERF
        // report: printing from parallel raster workers perturbs the wall-clock gate it diagnoses.
        let trace_passes = std::env::var_os("VALLE_COMPOSITOR_TRACE_PASSES").is_some();
        for pass in self.template.passes() {
            let pass_started = trace_passes.then(std::time::Instant::now);
            let output = pass.kind.output();
            let output_roi = bound_resource_roi(&self.schedules, output)?;
            let output_slot = plan_surface_slot(self.template, output)?;
            let (image, already_materialized) = match &pass.kind {
                ExecutionPassKind::ClearRegion {
                    working_linear_rec2020_premul,
                    ..
                } => (
                    render_plan_surface(
                        &mut surface_frame,
                        required_output_slot(output, output_slot)?,
                        output_roi,
                        |surface| draw_solid(surface, *working_linear_rec2020_premul),
                    )?,
                    true,
                ),
                ExecutionPassKind::ImportRegion {
                    external,
                    source_pipeline,
                    placement,
                    transform,
                    bounds,
                    ..
                } => {
                    let source =
                        external_visual(self.template, &external_images.images, *external)?;
                    let transform = dynamic_transform(self.bindings, *transform)?;
                    let bounds = dynamic_bounds(self.bindings, *bounds)?.intersect(root);
                    let scratch_count = if source_pipeline.chroma_key.is_some()
                        || matches!(
                            placement.backdrop,
                            Some(valle_engine::prepare::PreparedExternalBackdrop::Blur { .. })
                        ) {
                        2
                    } else {
                        0
                    };
                    let output_slot = required_output_slot(output, output_slot)?;
                    let image = {
                        let mut scratch = surface_frame.scratch(&vec![extent; scratch_count])?;
                        render_import(
                            &mut scratch,
                            output_slot,
                            source,
                            *placement,
                            transform,
                            bounds,
                            source_pipeline.chroma_key.as_ref(),
                            |id, kind| {
                                dynamic_scalar_kind(self.bindings, id, kind)
                                    .map_err(|error| DrawError::Internal(error.to_string()))
                            },
                            &self.effects,
                            self.bindings,
                            &info,
                            [output_roi.x, output_roi.y],
                        )?;
                        scratch.snapshot_output(output_slot, output_roi)?
                    };
                    (image, true)
                }
                ExecutionPassKind::RasterProgram {
                    program,
                    destination_inputs,
                    transform,
                    ..
                } => {
                    let output_slot = required_output_slot(output, output_slot)?;
                    let schedule = program_schedule(&self.schedules, *program, pass.id)?;
                    let mut local_extents = schedule
                        .surface_slots()
                        .iter()
                        .filter_map(|slot| slot.extent())
                        .collect::<Vec<_>>();
                    let helper_index =
                        program_helper_extent(plan_program(self.bindings, *program)?, schedule)?
                            .map(|helper| {
                                let index = local_extents.len();
                                local_extents.push(helper);
                                index
                            });
                    let image = {
                        let roi_copies =
                            program_uses_roi_copies(plan_program(self.bindings, *program)?);
                        let mut scratch = if roi_copies {
                            surface_frame.scratch_program(&local_extents)?
                        } else {
                            surface_frame.scratch(&local_extents)?
                        };
                        self.program(*program)?.execute(
                            plan_program(self.bindings, *program)?,
                            schedule,
                            dynamic_transform(self.bindings, *transform)?,
                            destination_inputs,
                            &surfaces,
                            extent,
                            &info,
                            &mut scratch,
                            ProgramTerminal::Plan {
                                slot: output_slot,
                                roi: output_roi,
                            },
                            helper_index,
                        )?
                    };
                    (image, true)
                }
                ExecutionPassKind::RasterCaption {
                    program,
                    destination,
                    transform,
                    opacity,
                    ..
                } => {
                    let source = {
                        let schedule = program_schedule(&self.schedules, *program, pass.id)?;
                        let mut local_extents = schedule
                            .surface_slots()
                            .iter()
                            .filter_map(|slot| slot.extent())
                            .collect::<Vec<_>>();
                        let helper_index = program_helper_extent(
                            plan_program(self.bindings, *program)?,
                            schedule,
                        )?
                        .map(|helper| {
                            let index = local_extents.len();
                            local_extents.push(helper);
                            index
                        });
                        let terminal_index = local_extents.len();
                        local_extents.push(extent);
                        let roi_copies =
                            program_uses_roi_copies(plan_program(self.bindings, *program)?);
                        let mut scratch = if roi_copies {
                            surface_frame.scratch_program(&local_extents)?
                        } else {
                            surface_frame.scratch(&local_extents)?
                        };
                        self.program(*program)?.execute(
                            plan_program(self.bindings, *program)?,
                            schedule,
                            dynamic_transform(self.bindings, *transform)?,
                            &[],
                            &surfaces,
                            extent,
                            &info,
                            &mut scratch,
                            ProgramTerminal::Scratch(terminal_index),
                            helper_index,
                        )?
                    };
                    let destination = surface(&surfaces, *destination)?;
                    let opacity = dynamic_scalar(self.bindings, *opacity)?;
                    (
                        render_plan_surface(
                            &mut surface_frame,
                            required_output_slot(output, output_slot)?,
                            output_roi,
                            |target| {
                                draw_composite(
                                    target,
                                    &source,
                                    destination,
                                    output_roi,
                                    BlendMode::SrcOver,
                                    opacity,
                                )
                            },
                        )?,
                        true,
                    )
                }
                ExecutionPassKind::BindBackdropView { input, .. }
                | ExecutionPassKind::AliasResource { input, .. } => {
                    (surface(&surfaces, *input)?.clone(), false)
                }
                ExecutionPassKind::ResolveRegion {
                    input,
                    sample_bounds,
                    output_bounds,
                    ..
                } => {
                    let (sample, _) =
                        backdrop_bounds(self.bindings, *sample_bounds, *output_bounds, root)?;
                    (
                        render_plan_surface(
                            &mut surface_frame,
                            required_output_slot(output, output_slot)?,
                            output_roi,
                            |target| {
                                draw_copy_region(
                                    target,
                                    surface(&surfaces, *input)?,
                                    sample,
                                    output_roi,
                                )
                            },
                        )?,
                        true,
                    )
                }
                ExecutionPassKind::DispatchKernel { invocation } => match invocation {
                    KernelInvocation::Group { input, .. } => (
                        render_plan_surface(
                            &mut surface_frame,
                            required_output_slot(output, output_slot)?,
                            output_roi,
                            |target| draw_copy(target, surface(&surfaces, *input)?, output_roi),
                        )?,
                        true,
                    ),
                    KernelInvocation::Mask { input, mask, .. } => {
                        let input = materialize_root_image(
                            &mut surface_frame,
                            surface(&surfaces, *input)?,
                            extent,
                        )?;
                        let output_slot = required_output_slot(output, output_slot)?;
                        let scratch_count = if mask.feather_sigma_device_px() > 0.0 {
                            2
                        } else {
                            1
                        };
                        let image = {
                            let mut scratch =
                                surface_frame.scratch(&vec![extent; scratch_count])?;
                            apply_prepared_mask(&mut scratch, output_slot, &input, *mask)?;
                            scratch.snapshot_output(output_slot, output_roi)?
                        };
                        (image, true)
                    }
                    KernelInvocation::Transition {
                        backdrop,
                        from,
                        to,
                        kernel,
                        progress,
                        from_opacity,
                        to_opacity,
                        ..
                    } => {
                        let backdrop = materialize_root_image(
                            &mut surface_frame,
                            surface(&surfaces, *backdrop)?,
                            extent,
                        )?;
                        let from = materialize_root_image(
                            &mut surface_frame,
                            surface(&surfaces, *from)?,
                            extent,
                        )?;
                        let to = materialize_root_image(
                            &mut surface_frame,
                            surface(&surfaces, *to)?,
                            extent,
                        )?;
                        let progress = dynamic_scalar_kind(
                            self.bindings,
                            *progress,
                            DynamicBindingKind::TransitionProgress,
                        )?;
                        let from_opacity = dynamic_scalar(self.bindings, *from_opacity)?;
                        let to_opacity = dynamic_scalar(self.bindings, *to_opacity)?;
                        let output_slot = required_output_slot(output, output_slot)?;
                        let image = {
                            let mut scratch = surface_frame.scratch(&[extent; 3])?;
                            self.effects.transition(
                                &mut scratch,
                                output_slot,
                                &backdrop,
                                &from,
                                &to,
                                *kernel,
                                progress,
                                from_opacity,
                                to_opacity,
                                &info,
                            )?;
                            scratch.snapshot_output(output_slot, output_roi)?
                        };
                        (image, true)
                    }
                    KernelInvocation::Filter { input, effect, .. }
                    | KernelInvocation::AdjustmentEffect { input, effect, .. } => {
                        let input = materialize_root_image(
                            &mut surface_frame,
                            surface(&surfaces, *input)?,
                            extent,
                        )?;
                        (
                            render_plan_surface(
                                &mut surface_frame,
                                required_output_slot(output, output_slot)?,
                                output_roi,
                                |target| {
                                    self.effects.apply_prepared_into(
                                        target,
                                        &input,
                                        effect,
                                        self.bindings,
                                        &info,
                                    )?;
                                    Ok(())
                                },
                            )?,
                            true,
                        )
                    }
                },
                ExecutionPassKind::CompositeRegion {
                    backdrop,
                    layer,
                    opacity,
                    mode,
                    ..
                } => {
                    let source = surface(&surfaces, *layer)?;
                    let destination = surface(&surfaces, *backdrop)?;
                    let opacity = dynamic_scalar(self.bindings, *opacity)?;
                    match mode {
                        CompositeMode::SourceOver {}
                        | CompositeMode::Blend {
                            mode: valle_draw::program::BlendMode::Normal,
                        } => (
                            render_plan_surface(
                                &mut surface_frame,
                                required_output_slot(output, output_slot)?,
                                output_roi,
                                |target| {
                                    draw_composite(
                                        target,
                                        source,
                                        destination,
                                        output_roi,
                                        BlendMode::SrcOver,
                                        opacity,
                                    )
                                },
                            )?,
                            true,
                        ),
                        CompositeMode::Blend { mode } => {
                            let blend = self.blend.as_ref().ok_or_else(|| {
                                SkiaExecuteError::Program(
                                    "creative blend kernel was not admitted".into(),
                                )
                            })?;
                            let source =
                                materialize_root_image(&mut surface_frame, source, extent)?;
                            let destination =
                                materialize_root_image(&mut surface_frame, destination, extent)?;
                            (
                                render_plan_surface(
                                    &mut surface_frame,
                                    required_output_slot(output, output_slot)?,
                                    output_roi,
                                    |target| {
                                        blend.composite_into(
                                            target,
                                            &source,
                                            &destination,
                                            *mode,
                                            opacity,
                                        )?;
                                        Ok(())
                                    },
                                )?,
                                true,
                            )
                        }
                    }
                }
                ExecutionPassKind::CopyConvert {
                    input,
                    output,
                    operation,
                } => {
                    if let CopyOperation::OutputTransform { spec } = operation {
                        if *output != self.template.output() {
                            return Err(SkiaExecuteError::WrongOutputResource {
                                resource: output.get(),
                            });
                        }
                        if *spec != self.template.render_spec().output() {
                            return Err(SkiaExecuteError::TargetSpecMismatch);
                        }
                    }
                    (surface(&surfaces, *input)?.clone(), false)
                }
            };
            let image = if already_materialized {
                image
            } else {
                materialize_plan_surface(
                    self.template,
                    output,
                    output_roi,
                    image,
                    &mut surface_frame,
                )?
            };
            if surfaces.insert(output, image).is_some() {
                return Err(SkiaExecuteError::DuplicateResource {
                    resource: output.get(),
                });
            }
            // The plan's inclusive liveness intervals are executable ownership, not inspector
            // decoration. Retire dead logical images immediately so Skia can release their pixel
            // storage (or make the physical slot reusable) before the next pass.
            for resource in self
                .template
                .surface_slots()
                .iter()
                .flat_map(|slot| &slot.allocations)
                .filter(|allocation| allocation.interval.last == pass.id)
                .map(|allocation| allocation.resource)
            {
                if resource != self.template.output() {
                    surfaces.remove(&resource);
                }
            }
            if let Some(started) = pass_started {
                eprintln!(
                    "[valle compositor pass] {} {:.3}ms",
                    pass.semantic_path,
                    started.elapsed().as_secs_f64() * 1_000.0,
                );
            }
        }

        let commit_started = trace_passes.then(std::time::Instant::now);
        let output = surfaces
            .get(&self.template.output())
            .cloned()
            .ok_or(SkiaExecuteError::MissingOutput)?;
        commit_output(&output, target, &self.effects, &mut surface_frame)?;
        if let Some(started) = commit_started {
            eprintln!(
                "[valle compositor commit] {:.3}ms",
                started.elapsed().as_secs_f64() * 1_000.0,
            );
        }
        #[cfg(all(target_os = "macos", feature = "native"))]
        if !target.is_direct_gpu() {
            surface_frame.release_completed_external_imports();
        }
        let mut surface_report: SkiaSurfaceReport = surface_frame.report().into();
        surface_report.total_surface_allocations = allocation_scope.finish();
        let pooled_allocations = surface_report
            .backend_allocations
            .checked_add(surface_report.scratch_allocations)
            .ok_or(SkiaExecuteError::SurfaceAllocationAccounting)?;
        surface_report.unpooled_surface_allocations = surface_report
            .total_surface_allocations
            .checked_sub(pooled_allocations)
            .ok_or(SkiaExecuteError::SurfaceAllocationAccounting)?;
        Ok(SkiaExecutionReport {
            passes: self.template.passes().len(),
            programs: self.programs.len(),
            program_cache_hits: self.cache_counters.program_hits,
            program_cache_misses: self.cache_counters.program_misses,
            program_cache_entries: self.cache_counters.program_entries,
            program_cache_cost_bytes: self.cache_counters.program_cost_bytes,
            font_cache_hits: self.cache_counters.font_hits,
            font_cache_misses: self.cache_counters.font_misses,
            shader_cache_hits: self.cache_counters.shader_hits,
            shader_cache_misses: self.cache_counters.shader_misses,
            built_in_kernel_cache_hits: self.effect_cache_counters.hits,
            built_in_kernel_cache_misses: self.effect_cache_counters.misses,
            external_gpu_imports: external_images.gpu_imports,
            cpu_upload_bytes: external_images.cpu_upload_bytes,
            surfaces: surface_report,
            committed: true,
        })
    }

    fn program(&self, id: ProgramId) -> Result<&ProgramRuntime, SkiaExecuteError> {
        self.programs
            .get(&id)
            .ok_or(SkiaExecuteError::InvalidProgram { program: id.get() })
    }
}

fn materialize_plan_surface(
    template: &RenderPlanTemplate,
    resource: PlanResourceId,
    roi: DeviceRect,
    image: PlanImage,
    frame: &mut SurfaceFrame<'_>,
) -> Result<PlanImage, SkiaExecuteError> {
    let plan = template
        .resources()
        .get(resource.get() as usize - 1)
        .filter(|candidate| candidate.id == resource)
        .ok_or(SkiaExecuteError::InvalidPassOutput {
            resource: resource.get(),
        })?;
    match plan.kind {
        PlanResourceKind::Surface { slot } => {
            render_plan_surface(frame, slot, roi, |target| draw_copy(target, &image, roi))
        }
        PlanResourceKind::Alias { .. } | PlanResourceKind::OutputTarget {} => Ok(image),
        PlanResourceKind::External { .. } => Err(SkiaExecuteError::InvalidPassOutput {
            resource: resource.get(),
        }),
    }
}

fn plan_surface_slot(
    template: &RenderPlanTemplate,
    resource: PlanResourceId,
) -> Result<Option<SurfaceSlotId>, SkiaExecuteError> {
    let plan = template
        .resources()
        .get(resource.get() as usize - 1)
        .filter(|candidate| candidate.id == resource)
        .ok_or(SkiaExecuteError::InvalidPassOutput {
            resource: resource.get(),
        })?;
    match plan.kind {
        PlanResourceKind::Surface { slot } => Ok(Some(slot)),
        PlanResourceKind::Alias { .. } | PlanResourceKind::OutputTarget {} => Ok(None),
        PlanResourceKind::External { .. } => Err(SkiaExecuteError::InvalidPassOutput {
            resource: resource.get(),
        }),
    }
}

fn required_output_slot(
    resource: PlanResourceId,
    slot: Option<SurfaceSlotId>,
) -> Result<SurfaceSlotId, SkiaExecuteError> {
    slot.ok_or(SkiaExecuteError::InvalidPassOutput {
        resource: resource.get(),
    })
}

fn render_plan_surface(
    frame: &mut SurfaceFrame<'_>,
    slot: SurfaceSlotId,
    roi: DeviceRect,
    draw: impl FnOnce(&mut Surface) -> Result<(), SkiaExecuteError>,
) -> Result<PlanImage, SkiaExecuteError> {
    if roi.is_empty() {
        return Ok(PlanImage::transparent());
    }
    let surface = frame.surface_mut(slot)?;
    draw(surface)?;
    Ok(frame.snapshot(slot, roi)?)
}

fn materialize_root_image(
    frame: &mut SurfaceFrame<'_>,
    image: &PlanImage,
    extent: Extent2d,
) -> Result<Image, SkiaExecuteError> {
    let root = DeviceRect::full(extent.width(), extent.height());
    if image.roi() == root
        && let Some(pixels) = image.image()
    {
        return Ok(pixels.clone());
    }
    let mut scratch = frame.scratch(&[extent])?;
    let target = scratch.surface_mut(0)?;
    target.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    draw_plan_image(target, image, root, BlendMode::Src, 1.0);
    Ok(target.image_snapshot())
}

fn preflight_target(
    template: &RenderPlanTemplate,
    target: &SkiaTarget<'_>,
) -> Result<(), SkiaExecuteError> {
    let expected = render_extent(template)?;
    if target.extent() != expected {
        return Err(SkiaExecuteError::TargetExtentMismatch {
            expected,
            actual: target.extent(),
        });
    }
    if target.output() != template.render_spec().output() {
        return Err(SkiaExecuteError::TargetSpecMismatch);
    }
    validate_target(target.output(), &target.image_info())?;
    Ok(())
}

fn render_extent(template: &RenderPlanTemplate) -> Result<Extent2d, SkiaExecuteError> {
    Extent2d::new(
        template.render_spec().width(),
        template.render_spec().height(),
    )
    .map_err(|_| SkiaExecuteError::InvalidRenderExtent)
}

fn preflight_executor_surface_budget(
    template: &RenderPlanTemplate,
    bindings: &RenderBindings,
    schedules: &BoundProgramSchedules,
    capabilities: &BackendCapabilities,
) -> Result<(), SkiaExecuteError> {
    let extent = render_extent(template)?;
    let full_surface_bytes = surface_bytes(extent)?;
    if full_surface_bytes > capabilities.max_surface_bytes() {
        return Err(SurfaceError::SurfaceBudgetExceeded {
            required_bytes: full_surface_bytes,
            max_bytes: capabilities.max_surface_bytes(),
        }
        .into());
    }
    let outer_physical_bytes =
        schedules
            .surface_slots()
            .iter()
            .try_fold(0_u64, |bytes, slot| {
                bytes
                    .checked_add(slot.estimated_bytes())
                    .ok_or(SurfaceError::ByteOverflow)
            })?;
    let mut peak = outer_physical_bytes;
    for pass in template.passes() {
        let kernel_scratch_surfaces = match &pass.kind {
            ExecutionPassKind::ImportRegion {
                source_pipeline,
                placement,
                ..
            } if source_pipeline.chroma_key.is_some()
                || matches!(
                    placement.backdrop,
                    Some(valle_engine::prepare::PreparedExternalBackdrop::Blur { .. })
                ) =>
            {
                2
            }
            ExecutionPassKind::DispatchKernel {
                invocation: KernelInvocation::Mask { mask, .. },
            } => u64::from(mask.feather_sigma_device_px() > 0.0) + 1,
            ExecutionPassKind::DispatchKernel {
                invocation: KernelInvocation::Transition { .. },
            } => 3,
            _ => 0,
        };
        let schedule = schedules
            .programs()
            .iter()
            .find(|schedule| schedule.execution_pass() == pass.id);
        let program_bytes = schedule.map_or(0, BoundProgramSchedule::estimated_surface_bytes);
        let program_helper_bytes = match (&pass.kind, schedule) {
            (
                ExecutionPassKind::RasterProgram { program, .. }
                | ExecutionPassKind::RasterCaption { program, .. },
                Some(schedule),
            ) => program_helper_extent(plan_program(bindings, *program)?, schedule)?
                .map(surface_bytes)
                .transpose()?
                .unwrap_or(0),
            _ => 0,
        };
        let terminal_bytes = if matches!(&pass.kind, ExecutionPassKind::RasterCaption { .. }) {
            full_surface_bytes
        } else {
            0
        };
        let kernel_scratch_bytes = full_surface_bytes
            .checked_mul(kernel_scratch_surfaces)
            .ok_or(SurfaceError::ByteOverflow)?;
        let required = outer_physical_bytes
            .checked_add(program_bytes)
            .and_then(|bytes| bytes.checked_add(program_helper_bytes))
            .and_then(|bytes| bytes.checked_add(terminal_bytes))
            .and_then(|bytes| bytes.checked_add(kernel_scratch_bytes))
            .ok_or(SurfaceError::ByteOverflow)?;
        peak = peak.max(required);
    }
    if peak > capabilities.max_frame_bytes() {
        return Err(SurfaceError::FrameBudgetExceeded {
            required_bytes: peak,
            max_bytes: capabilities.max_frame_bytes(),
        }
        .into());
    }
    Ok(())
}

fn surface_bytes(extent: Extent2d) -> Result<u64, SurfaceError> {
    u64::from(extent.width())
        .checked_mul(u64::from(extent.height()))
        .and_then(|pixels| pixels.checked_mul(8))
        .ok_or(SurfaceError::ByteOverflow)
}

fn plan_program(
    bindings: &RenderBindings,
    id: ProgramId,
) -> Result<&PlanProgram, SkiaExecuteError> {
    bindings
        .programs()
        .get(id.get() as usize - 1)
        .filter(|program| program.id == id)
        .ok_or(SkiaExecuteError::InvalidProgram { program: id.get() })
}

fn program_schedule(
    schedules: &BoundProgramSchedules,
    id: ProgramId,
    pass: ExecutionPassId,
) -> Result<&BoundProgramSchedule, SkiaExecuteError> {
    schedules
        .programs()
        .iter()
        .find(|schedule| schedule.program() == id && schedule.execution_pass() == pass)
        .ok_or(SkiaExecuteError::InvalidProgramSchedule {
            program: id.get(),
            pass: pass.get(),
        })
}

fn bound_resource_roi(
    schedules: &BoundProgramSchedules,
    resource: PlanResourceId,
) -> Result<DeviceRect, SkiaExecuteError> {
    schedules
        .resources()
        .get(resource.get() as usize - 1)
        .filter(|candidate| candidate.resource() == resource)
        .map(|resource| resource.device_roi())
        .ok_or(SkiaExecuteError::InvalidPassOutput {
            resource: resource.get(),
        })
}

// These operations sample strict logical ROIs; shaders and image kernels keep exact
// scratch extents because their sampling contracts can depend on image dimensions.
fn program_uses_roi_copies(program: &PlanProgram) -> bool {
    program.local_plan().passes().iter().all(|pass| {
        matches!(
            pass.kind,
            ProgramPassKind::Clear { .. }
                | ProgramPassKind::RasterNode { .. }
                | ProgramPassKind::RasterTree { .. }
                | ProgramPassKind::SourceOver { .. }
                | ProgramPassKind::ApplyClip { .. }
                | ProgramPassKind::ApplyOpacity { .. }
                | ProgramPassKind::ApplyTransform { .. }
        )
    })
}

fn program_helper_extent(
    program: &PlanProgram,
    schedule: &BoundProgramSchedule,
) -> Result<Option<Extent2d>, SkiaExecuteError> {
    let mut width = 0_u32;
    let mut height = 0_u32;
    for pass in program.local_plan().passes() {
        let ProgramPassKind::Backdrop { filters, .. } = &pass.kind else {
            continue;
        };
        if filters.is_empty() {
            continue;
        }
        let output = pass.kind.output();
        let roi = schedule
            .resources()
            .iter()
            .find(|resource| resource.resource() == output)
            .map(|resource| resource.device_roi())
            .ok_or_else(|| {
                SkiaExecuteError::Program(format!(
                    "DrawProgram {} has no bound ROI for local resource {}",
                    program.id.get(),
                    output.get()
                ))
            })?;
        width = width.max(roi.width);
        height = height.max(roi.height);
    }
    if width == 0 || height == 0 {
        Ok(None)
    } else {
        Extent2d::new(width, height)
            .map(Some)
            .map_err(|error| SkiaExecuteError::Program(error.to_string()))
    }
}

struct MaterializedExternalVisuals {
    images: BTreeMap<valle_engine::compositor::lower::ExternalSlotId, Image>,
    gpu_imports: u64,
    cpu_upload_bytes: u64,
}

fn materialize_external_visuals(
    bound: &BoundExternalObjects<'_, SkiaExternalObject>,
    surfaces: &mut SurfaceFrame<'_>,
) -> Result<MaterializedExternalVisuals, SkiaExecuteError> {
    #[cfg(not(feature = "native"))]
    let _ = surfaces;
    let mut images = BTreeMap::new();
    #[cfg(feature = "native")]
    let mut gpu_imports = 0_u64;
    #[cfg(not(feature = "native"))]
    let gpu_imports = 0_u64;
    #[cfg(feature = "native")]
    let mut cpu_upload_bytes = 0_u64;
    #[cfg(not(feature = "native"))]
    let cpu_upload_bytes = 0_u64;
    for binding in bound.objects() {
        let object = binding.object();
        let image = if let Some(image) = object.visual_image() {
            #[cfg(all(feature = "native", target_os = "macos"))]
            if surfaces.backend_kind() == super::surface::SkiaBackendKind::Metal {
                cpu_upload_bytes = cpu_upload_bytes
                    .checked_add(
                        object
                            .resident_bytes()
                            .ok_or(SkiaExecuteError::ExternalTransferAccounting)?,
                    )
                    .ok_or(SkiaExecuteError::ExternalTransferAccounting)?;
            }
            Some(image.clone())
        } else {
            #[cfg(feature = "native")]
            if object.decoded_video().is_some() {
                Some(match surfaces.import_decoded_video(object) {
                    Ok(image) => {
                        gpu_imports = gpu_imports
                            .checked_add(1)
                            .ok_or(SkiaExecuteError::ExternalTransferAccounting)?;
                        image
                    }
                    Err(_) => {
                        cpu_upload_bytes = cpu_upload_bytes
                            .checked_add(
                                object
                                    .resident_bytes()
                                    .ok_or(SkiaExecuteError::ExternalTransferAccounting)?,
                            )
                            .ok_or(SkiaExecuteError::ExternalTransferAccounting)?;
                        object.decoded_raster_image()?
                    }
                })
            } else {
                None
            }
            #[cfg(not(feature = "native"))]
            {
                None
            }
        };
        if let Some(image) = image {
            images.insert(binding.slot(), image);
        }
    }
    Ok(MaterializedExternalVisuals {
        images,
        gpu_imports,
        cpu_upload_bytes,
    })
}

fn external_visual<'a>(
    template: &RenderPlanTemplate,
    images: &'a BTreeMap<valle_engine::compositor::lower::ExternalSlotId, Image>,
    resource: PlanResourceId,
) -> Result<&'a Image, SkiaExecuteError> {
    let plan = template
        .resources()
        .get(resource.get() as usize - 1)
        .filter(|candidate| candidate.id == resource)
        .ok_or(SkiaExecuteError::InvalidExternalResource {
            resource: resource.get(),
        })?;
    let PlanResourceKind::External { slot } = plan.kind else {
        return Err(SkiaExecuteError::InvalidExternalResource {
            resource: resource.get(),
        });
    };
    images
        .get(&slot)
        .ok_or(SkiaExecuteError::InvalidExternalResource {
            resource: resource.get(),
        })
}

fn surface(
    surfaces: &BTreeMap<PlanResourceId, PlanImage>,
    resource: PlanResourceId,
) -> Result<&PlanImage, SkiaExecuteError> {
    surfaces
        .get(&resource)
        .ok_or(SkiaExecuteError::MissingResource {
            resource: resource.get(),
        })
}

fn dynamic_transform(
    bindings: &RenderBindings,
    id: DynamicBindingId,
) -> Result<DeviceTransform, SkiaExecuteError> {
    let binding = dynamic_binding(bindings, id, DynamicBindingKind::DeviceTransform)?;
    match binding.value {
        DynamicValue::DeviceTransform(value) => Ok(value),
        _ => Err(SkiaExecuteError::InvalidDynamicBinding { binding: id.get() }),
    }
}

fn dynamic_bounds(
    bindings: &RenderBindings,
    id: DynamicBindingId,
) -> Result<DeviceRect, SkiaExecuteError> {
    dynamic_bounds_kind(bindings, id, DynamicBindingKind::Bounds)
}

fn dynamic_bounds_kind(
    bindings: &RenderBindings,
    id: DynamicBindingId,
    kind: DynamicBindingKind,
) -> Result<DeviceRect, SkiaExecuteError> {
    let binding = dynamic_binding(bindings, id, kind)?;
    match binding.value {
        DynamicValue::Bounds(value) => Ok(value),
        _ => Err(SkiaExecuteError::InvalidDynamicBinding { binding: id.get() }),
    }
}

fn dynamic_scalar(
    bindings: &RenderBindings,
    id: DynamicBindingId,
) -> Result<f32, SkiaExecuteError> {
    dynamic_scalar_kind(bindings, id, DynamicBindingKind::Opacity)
}

fn dynamic_scalar_kind(
    bindings: &RenderBindings,
    id: DynamicBindingId,
    kind: DynamicBindingKind,
) -> Result<f32, SkiaExecuteError> {
    let binding = dynamic_binding(bindings, id, kind)?;
    match binding.value {
        DynamicValue::Scalar(value) => Ok(value as f32),
        _ => Err(SkiaExecuteError::InvalidDynamicBinding { binding: id.get() }),
    }
}

fn dynamic_binding(
    bindings: &RenderBindings,
    id: DynamicBindingId,
    kind: DynamicBindingKind,
) -> Result<&valle_engine::prepare::DynamicBinding, SkiaExecuteError> {
    bindings
        .dynamic()
        .get(id)
        .filter(|binding| binding.binding_kind == kind)
        .ok_or(SkiaExecuteError::InvalidDynamicBinding { binding: id.get() })
}

fn backdrop_bounds(
    bindings: &RenderBindings,
    sample: DynamicBindingId,
    output: DynamicBindingId,
    root: DeviceRect,
) -> Result<(DeviceRect, DeviceRect), SkiaExecuteError> {
    let sample_rect =
        dynamic_bounds_kind(bindings, sample, DynamicBindingKind::BackdropSampleBounds)?;
    let output_rect =
        dynamic_bounds_kind(bindings, output, DynamicBindingKind::BackdropOutputBounds)?;
    if !output_rect.is_empty() && sample_rect.intersect(output_rect) != output_rect {
        return Err(SkiaExecuteError::InvalidBackdropBounds {
            sample: sample.get(),
            output: output.get(),
        });
    }
    Ok((sample_rect.intersect(root), output_rect.intersect(root)))
}

fn draw_solid(surface: &mut Surface, color: [f32; 4]) -> Result<(), SkiaExecuteError> {
    let mut paint = Paint::default();
    paint.set_blend_mode(BlendMode::Src);
    let space = working_color_space()?;
    paint.set_color4f(
        straight_color(valle_draw::program::LinearColor {
            red: color[0],
            green: color[1],
            blue: color[2],
            alpha: color[3],
        }),
        &space,
    );
    surface.canvas().draw_paint(&paint);
    Ok(())
}

fn draw_copy(
    surface: &mut Surface,
    image: &PlanImage,
    target_roi: DeviceRect,
) -> Result<(), SkiaExecuteError> {
    surface.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    draw_plan_image(surface, image, target_roi, BlendMode::Src, 1.0);
    Ok(())
}

fn draw_copy_region(
    surface: &mut Surface,
    image: &PlanImage,
    bounds: DeviceRect,
    target_roi: DeviceRect,
) -> Result<(), SkiaExecuteError> {
    surface.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    if bounds.is_empty() {
        return Ok(());
    }
    let canvas = surface.canvas();
    canvas.save();
    canvas.clip_rect(
        skia_safe::Rect::from_xywh(
            (bounds.x - target_roi.x) as f32,
            (bounds.y - target_roi.y) as f32,
            bounds.width as f32,
            bounds.height as f32,
        ),
        skia_safe::ClipOp::Intersect,
        false,
    );
    if let Some(pixels) = image.image() {
        let roi = image.roi();
        let mut paint = Paint::default();
        paint.set_blend_mode(BlendMode::Src);
        canvas.draw_image(
            pixels,
            ((roi.x - target_roi.x) as f32, (roi.y - target_roi.y) as f32),
            Some(&paint),
        );
    }
    canvas.restore();
    Ok(())
}

fn draw_composite(
    surface: &mut Surface,
    source: &PlanImage,
    destination: &PlanImage,
    target_roi: DeviceRect,
    mode: BlendMode,
    opacity: f32,
) -> Result<(), SkiaExecuteError> {
    surface.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    draw_plan_image(surface, destination, target_roi, BlendMode::Src, 1.0);
    draw_plan_image(surface, source, target_roi, mode, opacity);
    Ok(())
}

fn draw_plan_image(
    surface: &mut Surface,
    image: &PlanImage,
    target_roi: DeviceRect,
    mode: BlendMode,
    opacity: f32,
) {
    let Some(pixels) = image.image() else {
        return;
    };
    let roi = image.roi();
    let mut paint = Paint::default();
    paint.set_blend_mode(mode);
    paint.set_alpha_f(opacity);
    surface.canvas().draw_image(
        pixels,
        ((roi.x - target_roi.x) as f32, (roi.y - target_roi.y) as f32),
        Some(&paint),
    );
}

fn commit_output(
    image: &PlanImage,
    target: &mut SkiaTarget<'_>,
    effects: &EffectRuntime,
    surface_frame: &mut SurfaceFrame<'_>,
) -> Result<(), SkiaExecuteError> {
    let image = image.image().ok_or(SkiaExecuteError::MissingOutput)?;
    let spec = target.output();
    if target.is_direct_gpu() {
        let shader = effects.output_shader(image, spec)?;
        if !target.commit_shader(shader) {
            return Err(SkiaExecuteError::TargetCommit);
        }
        return Ok(());
    }
    #[cfg(all(target_os = "macos", feature = "native"))]
    if surface_frame.backend_kind() == super::surface::SkiaBackendKind::Metal
        && spec.dither() == valle_engine::resource::Dither::None
        && spec.bit_depth() == valle_engine::resource::OutputBitDepth::Eight
        // Metal cannot allocate a straight-alpha render target.
        && spec.alpha() != valle_engine::resource::OutputAlphaMode::StraightCoverage
    {
        let shader = effects.output_shader(image, spec)?;
        let raster = surface_frame.render_output_shader_to_raster(shader, &target.image_info())?;
        if !target.commit_image(&raster) {
            return Err(SkiaExecuteError::TargetCommit);
        }
        return Ok(());
    }
    if surface_frame.backend_kind() == super::surface::SkiaBackendKind::Raster
        && spec.dither() == valle_engine::resource::Dither::None
        && spec.bit_depth() == valle_engine::resource::OutputBitDepth::Eight
        && !prefers_cached_output(image)
    {
        if let Some(staging) = stage_sdr_output(image, spec, &target.image_info())? {
            if !target.commit_pixels(staging.pixels(), staging.row_bytes()) {
                return Err(SkiaExecuteError::TargetCommit);
            }
        } else if !target.commit_shader(effects.output_shader(image, spec)?) {
            return Err(SkiaExecuteError::TargetCommit);
        }
        return Ok(());
    }
    let image = surface_frame.prepare_cpu_image(image)?;
    let target_info = target.image_info();
    let staging = stage_output(&image, spec, &target_info)?;
    if !target.commit_pixels(staging.pixels(), staging.row_bytes()) {
        return Err(SkiaExecuteError::TargetCommit);
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum SkiaExecuteError {
    #[error(transparent)]
    Capability(#[from] valle_engine::compositor::lower::CapabilityContractError),
    #[error(transparent)]
    ProgramBinding(#[from] ProgramBindingError),
    #[error(transparent)]
    ExternalBind(#[from] ExternalBindError),
    #[error(transparent)]
    Surface(#[from] SurfaceError),
    #[error(transparent)]
    Object(#[from] super::SkiaObjectError),
    #[error("Skia program execution failed: {0}")]
    Program(String),
    #[error("render plan has an invalid output extent")]
    InvalidRenderExtent,
    #[error("target extent mismatch: expected {expected:?}, got {actual:?}")]
    TargetExtentMismatch {
        expected: Extent2d,
        actual: Extent2d,
    },
    #[error("target output contract does not match the render plan")]
    TargetSpecMismatch,
    #[error("bound object projection does not match the admitted template")]
    BoundTemplateMismatch,
    #[error("DrawProgram id {program} occurs more than once")]
    DuplicateProgram { program: u32 },
    #[error("DrawProgram {program} is missing from the admitted frame")]
    InvalidProgram { program: u32 },
    #[error("DrawProgram {program} has no bound schedule for execution pass {pass}")]
    InvalidProgramSchedule { program: u32, pass: u32 },
    #[error("external plan resource {resource} is invalid or has no visual object")]
    InvalidExternalResource { resource: u32 },
    #[error("plan resource {resource} has not been produced")]
    MissingResource { resource: u32 },
    #[error("plan resource {resource} is produced more than once")]
    DuplicateResource { resource: u32 },
    #[error("execution pass writes invalid plan resource {resource}")]
    InvalidPassOutput { resource: u32 },
    #[error("dynamic binding {binding} has the wrong kind or value")]
    InvalidDynamicBinding { binding: u32 },
    #[error("backdrop bindings {sample}/{output} do not form a valid containment pair")]
    InvalidBackdropBounds { sample: u32, output: u32 },
    #[error("output transform wrote non-terminal resource {resource}")]
    WrongOutputResource { resource: u32 },
    #[error("render plan did not produce its terminal output")]
    MissingOutput,
    #[error("the terminal output could not be committed to the caller target")]
    TargetCommit,
    #[error("surface allocation evidence is inconsistent with physical pool growth")]
    SurfaceAllocationAccounting,
    #[error("external GPU import/upload accounting overflowed")]
    ExternalTransferAccounting,
}

impl From<DrawError> for SkiaExecuteError {
    fn from(value: DrawError) -> Self {
        Self::Program(value.to_string())
    }
}
