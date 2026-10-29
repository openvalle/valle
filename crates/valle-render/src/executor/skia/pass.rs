use std::collections::BTreeMap;

use skia_safe::{BlendMode, Color4f, IRect, Image, Paint, Surface};
use thiserror::Error;
use valle_engine::{
    compositor::{
        BoundExternalObjects, ExternalBindError, ExternalObject, bind_external_objects,
        lower::{
            BackendCapabilities, BoundExecutionPass, BoundProgramSchedule, BoundProgramSchedules,
            CompositeMode, CopyOperation, ExecutionPassId, ExecutionPassKind, KernelInvocation,
            PlanFrameInput, PlanProgram, PlanResourceId, PlanResourceKind, ProgramBindingError,
            ProgramPassKind, RenderBindings, RenderPlanTemplate, SurfaceSlotId,
        },
    },
    prepare::{
        DeviceRect, DeviceTransform, DynamicBindingId, DynamicBindingKind, DynamicValue, ProgramId,
    },
    resource::{ContentDigest, Extent2d, ExternalResourceDesc},
};

use super::{
    SkiaExternalObject, SkiaObjectTable, SkiaTarget,
    blend::BlendRuntime,
    cache::{BackendCacheCounters, BackendCaches, RasterLayerCache, RasterLayerKey},
    draw::{DrawError, ProgramRuntime, ProgramTerminal, erase_raster},
    effect::{EffectCacheCounters, EffectRuntime, apply_prepared_mask, set_working_color},
    import::render_import,
    output::{
        prefers_cached_output, stage_cpu_sdr_output_into, stage_output_into, stage_sdr_output_into,
        validate_target,
    },
    surface::{
        PlanImage, SkiaBackendKind, SurfaceAllocationScope, SurfaceArena, SurfaceError,
        SurfaceFrame, SurfaceFrameReport, working_info,
    },
};

/// Sealed executor instance. Capability identity is immutable for its lifetime and is rechecked
/// against every template during admission.
#[derive(Debug)]
pub struct SkiaExecutor {
    capabilities: BackendCapabilities,
    effects: EffectRuntime,
    caches: BackendCaches,
    raster_layers: RasterLayerCache,
    surfaces: SurfaceArena,
    capture_working: bool,
    working_capture: Option<super::WorkingF16Capture>,
}

impl SkiaExecutor {
    #[cfg(feature = "native")]
    pub(crate) fn set_cache_limits(&mut self, limits: super::cache::SkiaCacheLimits) {
        self.caches.set_limits(limits);
        self.raster_layers.set_byte_limit(limits.raster_layers);
        self.surfaces.set_pool_byte_limit(limits.surface_pool);
        self.surfaces
            .set_gpu_resource_cache_limit(limits.gpu_resources);
    }
    pub fn new(capabilities: BackendCapabilities) -> Result<Self, SkiaExecuteError> {
        // Selects Skia's CPU-specific raster pipeline (e.g. AVX2/F16C); idempotent.
        skia_safe::graphics::init();
        capabilities.validate()?;
        Ok(Self {
            capabilities,
            effects: EffectRuntime::new(),
            caches: BackendCaches::new(),
            raster_layers: RasterLayerCache::new(),
            surfaces: SurfaceArena::new(),
            capture_working: false,
            working_capture: None,
        })
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub fn metal(capabilities: BackendCapabilities) -> Result<Self, SkiaExecuteError> {
        capabilities.validate()?;
        Ok(Self {
            capabilities,
            effects: EffectRuntime::new(),
            caches: BackendCaches::new(),
            raster_layers: RasterLayerCache::new(),
            surfaces: SurfaceArena::metal()?,
            capture_working: false,
            working_capture: None,
        })
    }

    #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
    pub fn vulkan(capabilities: BackendCapabilities) -> Result<Self, SkiaExecuteError> {
        capabilities.validate()?;
        Ok(Self {
            capabilities,
            effects: EffectRuntime::new(),
            caches: BackendCaches::new(),
            raster_layers: RasterLayerCache::new(),
            surfaces: SurfaceArena::vulkan()?,
            capture_working: false,
            working_capture: None,
        })
    }

    pub const fn capabilities(&self) -> &BackendCapabilities {
        &self.capabilities
    }

    /// Opt-in diagnostics only: the next admitted execution captures working F16 before
    /// delivery. Default rendering allocates nothing and performs no additional readback.
    pub fn capture_next_working_frame(&mut self) {
        self.working_capture = None;
        self.capture_working = true;
    }

    pub fn take_working_capture(&mut self) -> Option<super::WorkingF16Capture> {
        self.working_capture.take()
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
        self.raster_layers.clear();
        self.caches
            .opacity_layers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.caches
            .noise_fields
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
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
            self.surfaces.backend_kind() != super::surface::SkiaBackendKind::Raster,
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
        let capture =
            std::mem::take(&mut self.capture_working).then_some(&mut self.working_capture);
        let result = admitted.execute_with_arena(
            target,
            &mut self.surfaces,
            &mut self.raster_layers,
            capture,
        );
        if result.is_err() {
            self.working_capture = None;
        }
        result
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

    #[cfg(all(
        feature = "native",
        any(target_os = "macos", target_os = "linux", target_os = "windows")
    ))]
    pub(crate) fn opacity_layer_cache_usage(&self) -> (usize, bool) {
        let cache = self
            .caches
            .opacity_layers
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        (cache.bytes(), cache.under_pressure())
    }

    #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
    pub(crate) fn supports_async_vulkan_output(&mut self) -> bool {
        self.surfaces
            .vulkan_context_mut()
            .is_some_and(|c| c.shared_device().is_some())
    }

    #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
    pub(crate) fn vulkan_memory(&mut self) -> Option<(u64, u64)> {
        self.surfaces.vulkan_context_mut()?.memory_budget()
    }

    #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
    pub(crate) fn acquire_vulkan_output(
        &mut self,
        width: u32,
        height: u32,
        output: valle_engine::resource::OutputSpec,
    ) -> Result<super::vulkan_delivery::VulkanOutput, SkiaExecuteError> {
        let context = self.surfaces.vulkan_context_mut().ok_or_else(|| {
            super::surface::SurfaceError::VulkanUnavailable("output requires Vulkan".into())
        })?;
        Ok(super::vulkan_delivery::VulkanOutput::new(
            context, width, height, output,
        )?)
    }

    #[cfg(all(feature = "native", any(target_os = "linux", target_os = "windows")))]
    pub(crate) fn submit_vulkan_output(
        &mut self,
        output: super::vulkan_delivery::VulkanOutput,
    ) -> Result<super::vulkan_delivery::SubmittedVulkanOutput, SkiaExecuteError> {
        Ok(self.surfaces.submit_vulkan_output(output)?)
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
        false,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "Keep the execution stage bindings, surfaces, geometry, and control inputs explicit."
)]
fn admit_skia_frame_with_effects<'a>(
    template: &'a RenderPlanTemplate,
    bindings: &'a RenderBindings,
    capabilities: &BackendCapabilities,
    objects: &'a SkiaObjectTable,
    target: &SkiaTarget<'_>,
    effects: EffectRuntime,
    caches: &mut BackendCaches,
    effect_cache_counters: EffectCacheCounters,
    gpu: bool,
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
        let runtime = ProgramRuntime::admit(program, &bound, caches, gpu)?;
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
        max_frame_bytes: capabilities.max_frame_bytes()
            - template.binding_layout().data_texture_bytes(),
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
    pub raster_layer_cache_hits: u64,
    pub raster_layer_cache_misses: u64,
    pub raster_layer_cache_entries: usize,
    /// Retained ROI pixels × 8 bytes; an estimate, not actual Skia allocation size.
    pub raster_layer_cache_bytes: usize,
    pub font_cache_hits: u64,
    pub font_cache_misses: u64,
    pub shader_cache_hits: u64,
    pub shader_cache_misses: u64,
    pub built_in_kernel_cache_hits: u64,
    pub built_in_kernel_cache_misses: u64,
    pub external_gpu_imports: u64,
    /// Scene3D layers rasterized from geometry on this executor's GPU.
    pub scene3d_gpu_rasters: u64,
    /// Scene3D layers using CPU rasterization, including unsupported GPU materials.
    pub scene3d_cpu_fallbacks: u64,
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
        let mut raster_layers = RasterLayerCache::new();
        self.execute_with_arena(target, &mut arena, &mut raster_layers, None)
    }

    fn execute_with_arena(
        mut self,
        target: &mut SkiaTarget<'_>,
        arena: &mut SurfaceArena,
        raster_layers: &mut RasterLayerCache,
        capture: Option<&mut Option<super::WorkingF16Capture>>,
    ) -> Result<SkiaExecutionReport, SkiaExecuteError> {
        preflight_target(self.template, target)?;
        raster_layers.begin_frame();
        let extent = render_extent(self.template)?;
        let info = working_info(extent)?;
        let render_spec = ContentDigest::of_bytes(
            &serde_json::to_vec(&self.template.render_spec())
                .expect("validated render spec is serializable"),
        );
        let root = DeviceRect::full(extent.width(), extent.height());
        let mut surfaces = BTreeMap::<PlanResourceId, PlanImage>::new();
        let allocation_scope = SurfaceAllocationScope::begin()?;
        let mut surface_frame = arena.begin_frame(
            self.template,
            &self.schedules,
            self.max_surface_bytes,
            self.max_frame_bytes,
        )?;
        let (layer_hits_before, layer_misses_before, _, _) = raster_layers.counters();
        let external_images = materialize_external_visuals(&self.bound, &mut surface_frame)?;
        for program in self.programs.values_mut() {
            program
                .resolve_deferred_visuals(&external_images.images)
                .map_err(|error| SkiaExecuteError::Program(error.to_string()))?;
        }

        // Deep per-pass tracing is intentionally separate from the stable aggregate VALLE_PERF
        // report: printing from parallel raster workers perturbs the wall-clock gate it diagnoses.
        let trace_passes = std::env::var_os("VALLE_COMPOSITOR_TRACE_PASSES").is_some();
        for pass in self.template.passes() {
            let pass_started = trace_passes.then(std::time::Instant::now);
            let bound_pass = self
                .schedules
                .passes()
                .get(pass.id.get() as usize - 1)
                .expect("frame admission binds every canonical execution pass");
            debug_assert_eq!(bound_pass.pass(), pass.id);
            let output = bound_pass.output();
            let output_roi = bound_pass.device_roi();
            let output_slot = bound_pass.surface_slot();
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
                ExecutionPassKind::FrameRegion { layer, .. } => {
                    let layer = match layer {
                        Some(layer) => Some((
                            surface(&surfaces, layer.layer)?,
                            dynamic_scalar(self.bindings, layer.opacity)?,
                        )),
                        None => None,
                    };
                    (
                        render_plan_surface(
                            &mut surface_frame,
                            required_output_slot(output, output_slot)?,
                            output_roi,
                            |target| {
                                target.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
                                if let Some((source, opacity)) = layer {
                                    draw_plan_image(
                                        target,
                                        source,
                                        output_roi,
                                        BlendMode::SrcOver,
                                        opacity,
                                    );
                                }
                                Ok(())
                            },
                        )?,
                        true,
                    )
                }
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
                            external_picture(self.template, &self.bound, *external),
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
                    external_inputs,
                    destination_inputs,
                    frame_inputs,
                    transform,
                    ..
                } => {
                    let output_slot = required_output_slot(output, output_slot)?;
                    let schedule = program_schedule(&self.schedules, *program, pass.id)?;
                    let plan = plan_program(self.bindings, *program)?;
                    let transform = dynamic_transform(self.bindings, *transform)?;
                    if !frame_inputs.is_empty() || !plan.requirements.frames.is_empty() {
                        let frames = frame_images(
                            self.template,
                            &mut surface_frame,
                            &surfaces,
                            plan,
                            frame_inputs,
                            transform,
                            extent,
                        )?;
                        self.programs
                            .get_mut(program)
                            .ok_or(SkiaExecuteError::InvalidProgram {
                                program: program.get(),
                            })?
                            .bind_frames(frames);
                    }
                    let cache_key = (external_inputs.is_empty()
                        && destination_inputs.is_empty()
                        && frame_inputs.is_empty()
                        && plan.resources.textures.is_empty()
                        && plan.resources.fonts.is_empty()
                        && plan.resources.runtime_shaders.is_empty()
                        && plan.resources.scenes.is_empty()
                        && plan.destination_uses.is_empty()
                        && plan.local_plan().passes().iter().all(|pass| {
                            !matches!(
                                &pass.kind,
                                ProgramPassKind::ReadDestination { .. }
                                    | ProgramPassKind::Backdrop { .. }
                            )
                        })
                        && !output_roi.is_empty())
                    .then(|| raster_layer_key(plan, transform, output_roi, self.template));
                    if let Some(image) = cache_key.as_ref().and_then(|key| raster_layers.get(key)) {
                        (image, false)
                    } else {
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
                        let image = {
                            let roi_copies =
                                program_uses_roi_copies(plan_program(self.bindings, *program)?);
                            let mut scratch = if roi_copies {
                                surface_frame.scratch_program(&local_extents)?
                            } else {
                                surface_frame.scratch(&local_extents)?
                            };
                            self.program(*program)?.execute(
                                plan,
                                schedule,
                                transform,
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
                                raster_layers,
                                render_spec,
                            )?
                        };
                        if let Some(key) = cache_key {
                            if plan.paint_frame().is_some() {
                                // Animated full-frame snapshots must not evict the reusable
                                // local passes merely because their first evaluation finished.
                                let _ = raster_layers.retain(key, image.clone(), true);
                            } else {
                                raster_layers.insert(key, image.clone());
                            }
                        }
                        (image, true)
                    }
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
                            raster_layers,
                            render_spec,
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
                            apply_prepared_mask(
                                &mut scratch,
                                output_slot,
                                &input,
                                *mask,
                                output_roi,
                            )?;
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
                    let opacity = dynamic_scalar(self.bindings, *opacity)?;
                    let normal = matches!(
                        mode,
                        CompositeMode::SourceOver {}
                            | CompositeMode::Blend {
                                mode: valle_draw::program::BlendMode::Normal,
                            }
                    );
                    if normal
                        && let Some(backdrop_slot) = in_place_backdrop(
                            self.template,
                            bound_pass,
                            &surfaces,
                            *backdrop,
                            *layer,
                            output_roi,
                        )
                        && surface_frame.swap_raster_slots(
                            backdrop_slot,
                            required_output_slot(output, output_slot)?,
                        )?
                    {
                        // The backdrop dies here, so Raster draws the layer over its pixels
                        // instead of over a copy of them; the two surfaces trade slots so the
                        // result is in the planned one. Dropping the backdrop's image first keeps
                        // the pixels unshared, which spares a copy-on-write.
                        surfaces.remove(backdrop);
                        let source = surface(&surfaces, *layer)?;
                        (
                            render_plan_surface(
                                &mut surface_frame,
                                required_output_slot(output, output_slot)?,
                                output_roi,
                                |target| {
                                    draw_plan_image(
                                        target,
                                        source,
                                        output_roi,
                                        BlendMode::SrcOver,
                                        opacity,
                                    );
                                    Ok(())
                                },
                            )?,
                            true,
                        )
                    } else {
                        let source = surface(&surfaces, *layer)?;
                        let destination = surface(&surfaces, *backdrop)?;
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
                                let destination = materialize_root_image(
                                    &mut surface_frame,
                                    destination,
                                    extent,
                                )?;
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
                                                valle_draw::program::BlendSpace::Srgb,
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
            for resource in bound_pass.retire_after() {
                surfaces.remove(resource);
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
        if let Some(capture) = capture {
            *capture = Some(super::working_capture::capture(
                &output,
                extent,
                &mut surface_frame,
            )?);
        }
        commit_output(&output, target, &mut self.effects, &mut surface_frame)?;
        if let Some(started) = commit_started {
            eprintln!(
                "[valle compositor commit] {:.3}ms",
                started.elapsed().as_secs_f64() * 1_000.0,
            );
        }
        #[cfg(all(
            feature = "native",
            any(target_os = "macos", target_os = "linux", target_os = "windows")
        ))]
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
        let (layer_hits, layer_misses, layer_entries, layer_bytes) = raster_layers.counters();
        Ok(SkiaExecutionReport {
            passes: self.template.passes().len(),
            programs: self.programs.len(),
            program_cache_hits: self.cache_counters.program_hits,
            program_cache_misses: self.cache_counters.program_misses,
            program_cache_entries: self.cache_counters.program_entries,
            program_cache_cost_bytes: self.cache_counters.program_cost_bytes,
            raster_layer_cache_hits: layer_hits.saturating_sub(layer_hits_before),
            raster_layer_cache_misses: layer_misses.saturating_sub(layer_misses_before),
            raster_layer_cache_entries: layer_entries,
            raster_layer_cache_bytes: layer_bytes,
            font_cache_hits: self.cache_counters.font_hits,
            font_cache_misses: self.cache_counters.font_misses,
            shader_cache_hits: self.cache_counters.shader_hits,
            shader_cache_misses: self.cache_counters.shader_misses,
            built_in_kernel_cache_hits: self.effect_cache_counters.hits,
            built_in_kernel_cache_misses: self.effect_cache_counters.misses,
            external_gpu_imports: external_images.gpu_imports,
            scene3d_gpu_rasters: external_images.scene3d_gpu_rasters,
            scene3d_cpu_fallbacks: external_images.scene3d_cpu_fallbacks,
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

/// The planned slot of a composite backdrop that can take the layer in place: no later pass
/// reads it and its image covers exactly the output region. A surface resource's image is its
/// slot's pixels from the origin: each pass draws its output there or copies a reused image
/// there, and no other pass writes the slot while the resource is live.
fn in_place_backdrop(
    template: &RenderPlanTemplate,
    pass: &BoundExecutionPass,
    surfaces: &BTreeMap<PlanResourceId, PlanImage>,
    backdrop: PlanResourceId,
    layer: PlanResourceId,
    output_roi: DeviceRect,
) -> Option<SurfaceSlotId> {
    if backdrop == layer || !pass.retire_after().contains(&backdrop) {
        return None;
    }
    let image = surfaces.get(&backdrop)?;
    if image.image().is_none() || image.roi() != output_roi {
        return None;
    }
    match template
        .resources()
        .get(backdrop.get() as usize - 1)
        .filter(|candidate| candidate.id == backdrop)?
        .kind
    {
        PlanResourceKind::Surface { slot } => Some(slot),
        _ => None,
    }
}

/// The host frames one pass hands its program: each frame's pixels under the program's
/// viewport, cut from the full-frame picture its frame region drew.
fn frame_images(
    template: &RenderPlanTemplate,
    frame: &mut SurfaceFrame<'_>,
    surfaces: &BTreeMap<PlanResourceId, PlanImage>,
    plan: &PlanProgram,
    inputs: &[PlanFrameInput],
    transform: DeviceTransform,
    extent: Extent2d,
) -> Result<Vec<(valle_draw::requirements::HostFrame, Image)>, SkiaExecuteError> {
    if plan
        .requirements
        .frames
        .iter()
        .any(|key| !inputs.iter().any(|input| input.frame == *key))
    {
        return Err(SkiaExecuteError::Program(
            "a program reads a host frame its pass does not supply".into(),
        ));
    }
    let region = transform
        .frame_region(extent.width(), extent.height())
        .map_err(|error| SkiaExecuteError::Program(error.to_string()))?;
    let root = DeviceRect::full(extent.width(), extent.height());
    inputs
        .iter()
        .map(|input| {
            let picture = surface(surfaces, input.input)?;
            // Under a viewport that covers the frame, as when the canvas fills the output, the
            // program reads the picture as it is.
            if region == root
                && picture.roi() == root
                && let Some(image) = picture.image()
            {
                return Ok((input.frame, image.clone()));
            }
            // Otherwise it reads the pixels the frame region left in its surface, which stays
            // live until this pass.
            let slot = match template
                .resources()
                .get(input.input.get() as usize - 1)
                .filter(|resource| resource.id == input.input)
                .map(|resource| &resource.kind)
            {
                Some(PlanResourceKind::Surface { slot }) => *slot,
                _ => {
                    return Err(SkiaExecuteError::MissingResource {
                        resource: input.input.get(),
                    });
                }
            };
            let image = frame
                .surface_mut(slot)?
                .image_snapshot_with_bounds(IRect::from_xywh(
                    region.x,
                    region.y,
                    region.width as i32,
                    region.height as i32,
                ))
                .ok_or(SurfaceError::Allocation)?;
            Ok((input.frame, image))
        })
        .collect()
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

fn raster_layer_key(
    program: &PlanProgram,
    transform: DeviceTransform,
    roi: DeviceRect,
    template: &RenderPlanTemplate,
) -> RasterLayerKey {
    // The frame hash covers the admitted local pass graph and schedule as well as viewport;
    // the content hash covers evaluated geometry, paints and shader uniforms.
    let render_spec =
        serde_json::to_vec(&template.render_spec()).expect("validated render spec is serializable");
    RasterLayerKey {
        program: *program.content_hash(),
        frame: *program.frame_hash(),
        render_spec: ContentDigest::of_bytes(&render_spec),
        transform: transform.matrix().map(f64::to_bits),
        roi: (roi.x, roi.y, roi.width, roi.height),
    }
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
        let terminal_bytes = match &pass.kind {
            ExecutionPassKind::RasterCaption { .. } => full_surface_bytes,
            // Each host frame is cut from its surface into an image of its own.
            ExecutionPassKind::RasterProgram { frame_inputs, .. } => full_surface_bytes
                .checked_mul(frame_inputs.len() as u64)
                .ok_or(SurfaceError::ByteOverflow)?,
            _ => 0,
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
    let peak = peak
        .checked_add(template.binding_layout().data_texture_bytes())
        .ok_or(SurfaceError::ByteOverflow)?;
    if peak > capabilities.max_frame_bytes() {
        return Err(SurfaceError::FrameBudgetExceeded {
            required_bytes: peak,
            max_bytes: capabilities.max_frame_bytes(),
        }
        .into());
    }
    Ok(())
}

/// Bytes one frame can charge to its admission credit while `backend` executes it.
///
/// Every outer slot stays allocated for the whole frame. A pass also checks out scratch
/// surfaces (program slots and helpers, kernel and import scratch), copies inputs into
/// full-frame images, and a software F16 filter reserves its CPU working set; the largest pass
/// sets the peak. Each GPU 3D layer retains either its raster color/depth plus a Skia copy,
/// or depth-of-field scratch and its result. Scratch a pass returns waits in the
/// reusable pool still charged to this frame while later passes run; the pool keeps at most
/// `pool_bytes`. Caches, and surfaces pooled by earlier frames, are charged to the shared
/// budget instead.
pub(crate) fn planned_frame_bytes(
    template: &RenderPlanTemplate,
    bindings: &RenderBindings,
    schedules: &BoundProgramSchedules,
    backend: SkiaBackendKind,
    pool_bytes: u64,
    scene3d: &[Extent2d],
) -> Result<u64, SkiaExecuteError> {
    let add = |a: u64, b: u64| a.checked_add(b).ok_or(SurfaceError::ByteOverflow);
    let full = surface_bytes(render_extent(template)?)?;
    let mut outer = 0;
    for slot in schedules.surface_slots() {
        outer = add(outer, slot.estimated_bytes())?;
    }
    let mut returned = 0_u64;
    let mut peak = 0_u64;
    let mut run = |checked_out: u64, working: u64| -> Result<(), SurfaceError> {
        peak = peak.max(add(add(returned.min(pool_bytes), checked_out)?, working)?);
        returned = add(returned, checked_out)?;
        Ok(())
    };
    if backend != SkiaBackendKind::Raster {
        for extent in scene3d {
            let bytes = surface_bytes(*extent)?;
            outer = add(outer, bytes)?;
            run(2 * bytes, 0)?;
        }
    }
    for pass in template.passes() {
        // Scratch surfaces the pass checks out, the full-frame copies of its inputs (each made
        // in its own scratch surface) and a software filter's working set.
        let (scratch, copies, working) = match &pass.kind {
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
                (2 * full, 0, 0)
            }
            ExecutionPassKind::DispatchKernel { invocation } => match invocation {
                KernelInvocation::Mask { mask, .. } => (
                    (1 + u64::from(mask.feather_sigma_device_px() > 0.0)) * full,
                    full,
                    0,
                ),
                KernelInvocation::Transition { .. } => (3 * full, 3 * full, 0),
                KernelInvocation::Filter { .. } | KernelInvocation::AdjustmentEffect { .. } => {
                    (0, full, 0)
                }
                _ => (0, 0, 0),
            },
            ExecutionPassKind::CompositeRegion {
                mode: CompositeMode::Blend { mode },
                ..
            } if *mode != valle_draw::program::BlendMode::Normal => (0, 2 * full, 0),
            ExecutionPassKind::RasterProgram {
                program,
                destination_inputs,
                frame_inputs,
                transform,
                ..
            } => {
                let destinations = destination_inputs
                    .iter()
                    .map(|input| {
                        schedules
                            .resources()
                            .iter()
                            .find(|resource| resource.resource() == *input)
                            .map(|resource| resource.device_roi())
                            .ok_or(SkiaExecuteError::Program(format!(
                                "destination resource {} is not bound",
                                input.get()
                            )))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let (scratch, working) = program_working_bytes(
                    bindings,
                    schedules,
                    pass.id,
                    *program,
                    dynamic_transform(bindings, *transform)?,
                    &destinations,
                    None,
                    backend,
                )?;
                // Each host frame is cut from its surface into an image of its own.
                (scratch, frame_inputs.len() as u64 * full, working)
            }
            ExecutionPassKind::RasterCaption {
                program, transform, ..
            } => {
                let (scratch, working) = program_working_bytes(
                    bindings,
                    schedules,
                    pass.id,
                    *program,
                    dynamic_transform(bindings, *transform)?,
                    &[],
                    Some(render_extent(template)?),
                    backend,
                )?;
                (scratch, 0, working)
            }
            _ => (0, 0, 0),
        };
        run(add(scratch, copies)?, working)?;
    }
    Ok(add(outer, peak.max(returned.min(pool_bytes)))?)
}

/// Scratch surfaces one program checks out and the largest software F16 filter working set it
/// reserves meanwhile.
#[allow(clippy::too_many_arguments)]
fn program_working_bytes(
    bindings: &RenderBindings,
    schedules: &BoundProgramSchedules,
    pass: ExecutionPassId,
    program: ProgramId,
    transform: DeviceTransform,
    destinations: &[DeviceRect],
    terminal: Option<Extent2d>,
    backend: SkiaBackendKind,
) -> Result<(u64, u64), SkiaExecuteError> {
    let plan = plan_program(bindings, program)?;
    let schedule = program_schedule(schedules, program, pass)?;
    let mut extents = schedule
        .surface_slots()
        .iter()
        .filter_map(|slot| slot.extent())
        .collect::<Vec<_>>();
    extents.extend(program_helper_extent(plan, schedule)?);
    extents.extend(terminal);
    // Raster ROI copies round their capacity up to 32-pixel steps (`scratch_program`).
    let step = if backend == SkiaBackendKind::Raster && program_uses_roi_copies(plan) {
        32
    } else {
        1
    };
    let mut scratch = 0_u64;
    for extent in extents {
        let [width, height] =
            [extent.width(), extent.height()].map(|n| u64::from(n.div_ceil(step) * step));
        scratch = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(8))
            .and_then(|bytes| bytes.checked_add(scratch))
            .ok_or(SurfaceError::ByteOverflow)?;
    }
    let cpu =
        super::draw::program_f16_filter_bytes(plan, schedule, transform, destinations, backend)?;
    Ok((scratch, cpu))
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
    scene3d_gpu_rasters: u64,
    scene3d_cpu_fallbacks: u64,
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
    #[cfg(feature = "native")]
    let mut scene3d_gpu_rasters = 0_u64;
    #[cfg(not(feature = "native"))]
    let scene3d_gpu_rasters = 0_u64;
    let mut scene3d_cpu_fallbacks = 0_u64;
    for binding in bound.objects() {
        let object = binding.object();
        #[cfg(feature = "native")]
        if object.deferred_scene3d_gpu().is_some() {
            #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
            let rendered = surfaces.render_scene3d_gpu(object)?;
            #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
            let rendered: Option<(Image, u64)> = None;
            let image = if let Some((image, upload_bytes)) = rendered {
                scene3d_gpu_rasters = scene3d_gpu_rasters
                    .checked_add(1)
                    .ok_or(SkiaExecuteError::ExternalTransferAccounting)?;
                gpu_imports = gpu_imports
                    .checked_add(1)
                    .ok_or(SkiaExecuteError::ExternalTransferAccounting)?;
                cpu_upload_bytes = cpu_upload_bytes
                    .checked_add(upload_bytes)
                    .ok_or(SkiaExecuteError::ExternalTransferAccounting)?;
                image
            } else {
                let fallback = object
                    .scene3d_gpu_fallback()
                    .map_err(|error| SkiaExecuteError::Program(error.to_string()))?;
                scene3d_cpu_fallbacks = scene3d_cpu_fallbacks
                    .checked_add(1)
                    .ok_or(SkiaExecuteError::ExternalTransferAccounting)?;
                cpu_upload_bytes = cpu_upload_bytes
                    .checked_add(
                        fallback
                            .resident_bytes()
                            .ok_or(SkiaExecuteError::ExternalTransferAccounting)?,
                    )
                    .ok_or(SkiaExecuteError::ExternalTransferAccounting)?;
                fallback
                    .visual_image()
                    .ok_or(SkiaExecuteError::ExternalTransferAccounting)?
                    .clone()
            };
            images.insert(binding.slot(), image);
            continue;
        }
        if matches!(object.descriptor(), ExternalResourceDesc::Scene3d) {
            scene3d_cpu_fallbacks = scene3d_cpu_fallbacks
                .checked_add(1)
                .ok_or(SkiaExecuteError::ExternalTransferAccounting)?;
        }
        let image = if let Some(input) = object.deferred_scene3d_dof() {
            #[cfg(feature = "native")]
            {
                cpu_upload_bytes = cpu_upload_bytes
                    .checked_add(input.resident_bytes())
                    .ok_or(SkiaExecuteError::ExternalTransferAccounting)?;
            }
            Some(
                super::scene3d_dof::render(input, surfaces)
                    .map_err(|error| SkiaExecuteError::Program(error.to_string()))?,
            )
        } else if let Some(image) = object.visual_image() {
            #[cfg(feature = "native")]
            if surfaces.backend_kind() != super::surface::SkiaBackendKind::Raster {
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
        scene3d_gpu_rasters,
        scene3d_cpu_fallbacks,
    })
}

fn external_picture<'a>(
    template: &RenderPlanTemplate,
    bound: &'a BoundExternalObjects<'_, SkiaExternalObject>,
    resource: PlanResourceId,
) -> Option<&'a skia_safe::Picture> {
    let plan = template.resources().get(resource.get() as usize - 1)?;
    let PlanResourceKind::External { slot } = plan.kind else {
        return None;
    };
    bound.get(slot)?.svg_picture()
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
    if erase_raster(surface, color, None) {
        return Ok(());
    }
    let mut paint = Paint::default();
    paint.set_blend_mode(BlendMode::Src);
    set_working_color(
        &mut paint,
        valle_draw::program::LinearColor {
            red: color[0],
            green: color[1],
            blue: color[2],
            alpha: color[3],
        },
    )?;
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
    // A backdrop covering the target rewrites every pixel under Src, so on Raster the clear is
    // a redundant full write. GPU targets keep it: it can become the render pass's load action.
    let covered =
        destination.image().is_some() && destination.roi().intersect(target_roi) == target_roi;
    if !covered || surface.direct_context().is_some() {
        surface.canvas().clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    }
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
    effects: &mut EffectRuntime,
    surface_frame: &mut SurfaceFrame<'_>,
) -> Result<(), SkiaExecuteError> {
    let image = image.image().ok_or(SkiaExecuteError::MissingOutput)?;
    let spec = target.output();
    if target.is_direct_gpu() {
        let shader = effects.output_shader(image, spec, false)?;
        if !target.commit_shader(shader) {
            return Err(SkiaExecuteError::TargetCommit);
        }
        return Ok(());
    }
    if surface_frame.backend_kind() == super::surface::SkiaBackendKind::Raster {
        let staging = surface_frame.output_staging_mut();
        if let Some(row_bytes) =
            stage_cpu_sdr_output_into(image, spec, &target.image_info(), staging)?
        {
            if !target.commit_pixels(staging.pixels(), row_bytes) {
                return Err(SkiaExecuteError::TargetCommit);
            }
            return Ok(());
        }
    }
    if surface_frame.backend_kind() == super::surface::SkiaBackendKind::Raster
        // Use the canonical delivery conversion on x86: its integer bytes must not depend on
        // the platform Skia runtime shader's float evaluation/rounding path.
        && cfg!(target_arch = "aarch64")
        && spec.dither() != valle_engine::resource::Dither::None
        && spec.bit_depth() == valle_engine::resource::OutputBitDepth::Eight
    {
        if !target.commit_shader(effects.output_shader(image, spec, false)?) {
            return Err(SkiaExecuteError::TargetCommit);
        }
        return Ok(());
    }
    #[cfg(all(
        feature = "native",
        any(target_os = "macos", target_os = "linux", target_os = "windows")
    ))]
    if surface_frame.backend_kind() != super::surface::SkiaBackendKind::Raster
        && spec.bit_depth() == valle_engine::resource::OutputBitDepth::Eight
    {
        let shader = effects.output_shader(image, spec, true)?;
        let straight = spec.alpha() == valle_engine::resource::OutputAlphaMode::StraightCoverage;
        // Ganesh cannot allocate an unassociated-alpha render target. Store the terminal
        // codes in a premultiplied *transport* surface instead: Src writes every byte,
        // and its raw readback must never be drawn or unpremultiplied by Skia again.
        let info = if straight {
            target
                .image_info()
                .with_alpha_type(skia_safe::AlphaType::Premul)
        } else {
            target.image_info()
        };
        let raster = surface_frame.render_output_shader_to_raster(shader, &info)?;
        let committed = if straight {
            let pixels = raster.peek_pixels().ok_or(SkiaExecuteError::TargetCommit)?;
            target.commit_pixels(
                pixels.bytes().ok_or(SkiaExecuteError::TargetCommit)?,
                pixels.row_bytes(),
            )
        } else {
            target.commit_image(&raster)
        };
        if !committed {
            return Err(SkiaExecuteError::TargetCommit);
        }
        return Ok(());
    }
    if surface_frame.backend_kind() == super::surface::SkiaBackendKind::Raster
        && spec.dither() == valle_engine::resource::Dither::None
        && spec.bit_depth() == valle_engine::resource::OutputBitDepth::Eight
        && !prefers_cached_output(image)
    {
        let staging = surface_frame.output_staging_mut();
        if let Some(row_bytes) = stage_sdr_output_into(image, spec, &target.image_info(), staging)?
        {
            if !target.commit_pixels(staging.pixels(), row_bytes) {
                return Err(SkiaExecuteError::TargetCommit);
            }
        } else if !target.commit_shader(effects.output_shader(image, spec, false)?) {
            return Err(SkiaExecuteError::TargetCommit);
        }
        return Ok(());
    }
    let image = surface_frame.prepare_cpu_image(image)?;
    let target_info = target.image_info();
    let staging = surface_frame.output_staging_mut();
    let row_bytes = stage_output_into(&image, spec, &target_info, staging)?;
    if !target.commit_pixels(staging.pixels(), row_bytes) {
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

#[cfg(test)]
mod tests {
    use valle_draw::program::LinearColor;

    use super::*;
    use crate::executor::skia::surface::raster_surface;

    fn pixels(surface: &mut Surface) -> Vec<u8> {
        surface.peek_pixels().unwrap().bytes().unwrap().to_vec()
    }

    #[test]
    fn raster_erase_stores_the_bytes_of_the_working_color_shader() {
        let info = working_info(Extent2d::new(7, 3).unwrap()).unwrap();
        // Extended-range, negative and half-float subnormal channels convert like in-range ones.
        for color in [
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            [0.214_041_14, 0.033_104_766, 0.006_048_833, 1.0],
            [1.0 / 3.0, 2.0 / 3.0, 0.1, 1.0],
            [1.7, -0.25, 1e-6, 1.0],
        ] {
            let mut shaded = raster_surface(&info).unwrap();
            let mut paint = Paint::default();
            paint.set_blend_mode(BlendMode::Src);
            set_working_color(
                &mut paint,
                LinearColor {
                    red: color[0],
                    green: color[1],
                    blue: color[2],
                    alpha: color[3],
                },
            )
            .unwrap();
            shaded.canvas().draw_paint(&paint);
            let mut erased = raster_surface(&info).unwrap();
            assert!(erase_raster(&mut erased, color, None));
            assert_eq!(pixels(&mut erased), pixels(&mut shaded), "{color:?}");
        }
        let mut surface = raster_surface(&info).unwrap();
        assert!(!erase_raster(&mut surface, [0.25, 0.25, 0.25, 0.5], None));
        assert!(!erase_raster(&mut surface, [f32::NAN, 0.0, 0.0, 1.0], None));
    }

    #[test]
    fn raster_erase_leaves_a_shared_snapshot_intact() {
        let info = working_info(Extent2d::new(4, 4).unwrap()).unwrap();
        let mut surface = raster_surface(&info).unwrap();
        assert!(erase_raster(&mut surface, [1.0, 0.0, 0.0, 1.0], None));
        let red = pixels(&mut surface);
        let snapshot = surface.image_snapshot();
        assert!(erase_raster(&mut surface, [0.0, 0.0, 1.0, 1.0], None));
        assert_ne!(pixels(&mut surface), red);
        let kept = snapshot.peek_pixels().unwrap().bytes().unwrap().to_vec();
        assert_eq!(kept, red);
    }
}
