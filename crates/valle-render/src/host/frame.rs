use std::{sync::Arc, time::Instant};

use skia_safe::{Surface, surfaces};
use thiserror::Error;
use valle_engine::{
    compositor::{
        ExternalBindError, ExternalObjectTable,
        lower::{BackendCapabilities, RenderBindings, RenderPlanTemplate},
    },
    frame::{RenderSpec, RenderSpecError},
    product::{EngineRender, FrameCompiler, ProductEngineError},
    render::{FrameKey, RenderId},
    resource::{
        ContentDigest, DescriptorError, Extent2d, ExternalGeneration, ResourceContractError,
        ResourceRequest,
    },
};
use valle_media::frame::RgbaFrame;
#[cfg(all(target_os = "macos", feature = "native"))]
use valle_media::{SharedVideoFrame, SharedVideoFramePool};

#[cfg(all(target_os = "macos", feature = "native"))]
use crate::executor::skia::SubmittedSharedMetalFrame;
use crate::executor::skia::{
    SkiaBackendKind, SkiaExecuteError, SkiaExecutionProfile, SkiaExecutionReport, SkiaExecutor,
    SkiaExternalObject, SkiaObjectTable, SkiaTarget, SkiaTargetError, cpu_capabilities,
    rgba8_target_info,
};

/// A complete generation-local fulfillment result. The vector form makes ownership and duplicate
/// handle detection explicit when it is sealed into an object table.
pub type ResourceFulfillment = Vec<(valle_engine::resource::ExternalHandleId, SkiaExternalObject)>;

/// Host-owned decoder/font/shader/Scene3D provider. It receives only the immutable request and
/// render snapshot; authoring evaluation and render-plan structure are deliberately unavailable.
pub trait ResourceProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    fn begin_frame(&mut self) {}

    fn fulfill(&mut self, request: &ResourceRequest) -> Result<SkiaExternalObject, Self::Error>;

    fn finish_frame(&mut self, fulfilled_requests: &[ResourceRequest]) -> ResourceFrameReport {
        ResourceFrameReport::uncached(fulfilled_requests)
    }

    fn invalidate_generation(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Called exactly once before the runner is returned. Providers may select a platform decode
    /// transport, but cannot change Engine capabilities or compositor semantics.
    fn configure_backend(&mut self, _backend: SkiaBackendKind) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Per-frame host-object cache evidence. A miss means fulfillment created/rebuilt the requested
/// object; `bypasses` are misses deliberately not retained because an object exceeds configured
/// limits. Generation changes invalidate every backend object atomically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceCacheFrameReport {
    pub generation: u64,
    pub generation_invalidations: u64,
    pub requests: u64,
    pub hits: u64,
    pub misses: u64,
    pub insertions: u64,
    pub evictions: u64,
    pub bypasses: u64,
    pub resident_entries: usize,
    pub resident_bytes: u64,
}

impl ResourceCacheFrameReport {
    fn uncached(requests: usize) -> Self {
        let requests = u64::try_from(requests).unwrap_or(u64::MAX);
        Self {
            generation: 1,
            generation_invalidations: 0,
            requests,
            hits: 0,
            misses: requests,
            insertions: 0,
            evictions: 0,
            bypasses: requests,
            resident_entries: 0,
            resident_bytes: 0,
        }
    }
}

/// Scene3D fulfillment evidence for one frame. Prepare/raster/materialize are resource-provider
/// stages; composite is filled by the executor with the complete plan execution time for a frame
/// that consumes at least one Scene3D raster.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Scene3dFrameReport {
    pub requests: u64,
    pub prepared_cache_hits: u64,
    pub prepared_cache_misses: u64,
    pub prepare_us: u64,
    pub raster_us: u64,
    pub upload_us: u64,
    pub composite_us: u64,
}

/// One complete provider report. Cache behavior and typed resource-stage timings travel together
/// so a staged runner cannot lose worker-local evidence when `PreparedFrame` crosses a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceFrameReport {
    pub cache: ResourceCacheFrameReport,
    pub scene3d: Scene3dFrameReport,
}

impl ResourceFrameReport {
    fn uncached(requests: &[ResourceRequest]) -> Self {
        Self {
            cache: ResourceCacheFrameReport::uncached(requests.len()),
            scene3d: Scene3dFrameReport {
                requests: u64::try_from(
                    requests
                        .iter()
                        .filter(|request| {
                            matches!(
                                request.expected(),
                                valle_engine::resource::ExternalResourceDesc::Scene3d
                            )
                        })
                        .count(),
                )
                .unwrap_or(u64::MAX),
                ..Scene3dFrameReport::default()
            },
        }
    }
}

/// Machine-readable evidence returned by every host path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameEvidence {
    pub render_id: RenderId,
    pub frame: i64,
    pub profile: SkiaExecutionProfile,
    pub delivery: FrameDelivery,
    pub delivery_readback_bytes: u64,
    pub resource_requests: usize,
    pub resource_cache: ResourceCacheFrameReport,
    pub scene3d: Scene3dFrameReport,
    pub execution_passes: usize,
    pub template_hash: ContentDigest,
    pub template_cache_hit: bool,
    pub execution: SkiaExecutionReport,
    pub timings: FrameTimings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameDelivery {
    /// A caller-owned surface received the transactional terminal commit.
    Surface,
    /// The caller requested tightly packed CPU RGBA bytes.
    CpuReadback,
    /// An encoder-owned platform surface was rendered and submitted without CPU readback.
    SharedGpu,
}

/// Wall-clock evidence for the actual product stages. Fulfillment and lower intentionally overlap;
/// their individual durations may therefore sum to more than `fulfill_lower_wall_us`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTimings {
    pub evaluate_prepare_us: u64,
    pub fulfillment_us: u64,
    pub lower_us: u64,
    pub fulfill_lower_wall_us: u64,
    pub bind_us: u64,
    pub execute_us: u64,
    /// Present only for `render_rgba8`; caller-owned GPU/surface delivery has no CPU readback.
    pub delivery_readback_us: Option<u64>,
    /// Present only for shared-GPU delivery; queue residence is included in `total_us`, while this
    /// field measures only CPU time blocked waiting for Ganesh's completion callback.
    pub gpu_completion_wait_us: Option<u64>,
    pub total_us: u64,
}

pub(crate) struct PreparedFrame {
    started: Instant,
    render_id: RenderId,
    frame: i64,
    resource_requests: usize,
    resource_cache: ResourceCacheFrameReport,
    scene3d: Scene3dFrameReport,
    execution_passes: usize,
    template_hash: ContentDigest,
    template_cache_hit: bool,
    template: Arc<RenderPlanTemplate>,
    bindings: RenderBindings,
    objects: SkiaObjectTable,
    timings: FrameTimings,
}

#[cfg(all(target_os = "macos", feature = "native"))]
pub(crate) struct SubmittedSharedFrame {
    submitted: SubmittedSharedMetalFrame,
    evidence: FrameEvidence,
    started: Instant,
}

/// The one Native per-frame orchestration path.
pub struct FrameRunner<P> {
    render: EngineRender,
    compiler: FrameCompiler,
    executor: SkiaExecutor,
    provider: P,
    /// Reused CPU delivery target for preview, PNG and software encoders. The Product executor
    /// still renders transactionally into private plan surfaces; this host surface is overwritten
    /// only by the terminal commit and can therefore persist safely across frames.
    cpu_delivery_surface: Option<Surface>,
    next_generation: u64,
}

pub(crate) struct FramePreparationStage<P> {
    render: EngineRender,
    compiler: FrameCompiler,
    provider: P,
    capabilities: BackendCapabilities,
    next_generation: u64,
}

pub(crate) struct FrameExecutionStage {
    executor: SkiaExecutor,
    cpu_delivery_surface: Option<Surface>,
}

impl<P: ResourceProvider> FrameRunner<P> {
    /// Creates one runner for an explicit immutable physical backend. Product code never consults
    /// ambient process state, so preview, export and tests cannot silently select different paths.
    pub fn for_backend(
        render: EngineRender,
        provider: P,
        backend: SkiaBackendKind,
    ) -> Result<Self, FrameRenderError> {
        match backend {
            SkiaBackendKind::Raster => Self::cpu(render, provider),
            #[cfg(all(target_os = "macos", feature = "native"))]
            SkiaBackendKind::Metal => Self::metal(render, provider),
        }
    }

    pub fn cpu(render: EngineRender, provider: P) -> Result<Self, FrameRenderError> {
        let capabilities = cpu_capabilities(
            Extent2d::new(16_384, 16_384)?,
            2 * 1024 * 1024 * 1024,
            4 * 1024 * 1024 * 1024,
        )?;
        Self::new(render, provider, capabilities)
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub fn metal(render: EngineRender, provider: P) -> Result<Self, FrameRenderError> {
        let capabilities = cpu_capabilities(
            Extent2d::new(16_384, 16_384)?,
            2 * 1024 * 1024 * 1024,
            4 * 1024 * 1024 * 1024,
        )?;
        let compiler = render.frame_compiler();
        let mut runner = Self {
            render,
            compiler,
            executor: SkiaExecutor::metal(capabilities)?,
            provider,
            cpu_delivery_surface: None,
            next_generation: 1,
        };
        runner.configure_provider_backend()?;
        Ok(runner)
    }

    /// Builds a runner with an explicit, immutable backend contract. Product defaults use
    /// [`Self::cpu`], while pinned performance/capacity runners pass the exact same capability
    /// value through lower, bind and execution so admission evidence cannot drift between stages.
    pub fn new(
        render: EngineRender,
        provider: P,
        capabilities: valle_engine::compositor::lower::BackendCapabilities,
    ) -> Result<Self, FrameRenderError> {
        let compiler = render.frame_compiler();
        let mut runner = Self {
            render,
            compiler,
            executor: SkiaExecutor::new(capabilities)?,
            provider,
            cpu_delivery_surface: None,
            next_generation: 1,
        };
        runner.configure_provider_backend()?;
        Ok(runner)
    }

    pub const fn render(&self) -> &EngineRender {
        &self.render
    }

    pub const fn provider(&self) -> &P {
        &self.provider
    }

    pub fn provider_mut(&mut self) -> &mut P {
        &mut self.provider
    }

    pub(crate) fn into_stages(self) -> (FramePreparationStage<P>, FrameExecutionStage) {
        let capabilities = self.executor.capabilities().clone();
        (
            FramePreparationStage {
                render: self.render,
                compiler: self.compiler,
                provider: self.provider,
                capabilities,
                next_generation: self.next_generation,
            },
            FrameExecutionStage {
                executor: self.executor,
                cpu_delivery_surface: self.cpu_delivery_surface,
            },
        )
    }

    /// Invalidates backend-owned reusable surfaces after an explicit context/device loss. Normal
    /// extent and output-contract changes are detected by the executor automatically.
    pub fn invalidate_surface_generation(&mut self) -> Result<(), FrameRenderError> {
        self.provider.invalidate_generation().map_err(|error| {
            FrameRenderError::ResourceGenerationInvalidation {
                reason: error.to_string(),
            }
        })?;
        self.executor.invalidate_surface_generation();
        Ok(())
    }

    /// Render transactionally into a caller-owned target. Fulfillment completes before admission;
    /// executor failure therefore cannot leave a partially updated target.
    pub fn render_into(
        &mut self,
        frame: FrameKey,
        spec: RenderSpec,
        surface: &mut Surface,
    ) -> Result<FrameEvidence, FrameRenderError> {
        let prepared = self.prepare_frame(frame, spec)?;
        let mut target = SkiaTarget::new(surface, spec.output())?;
        self.execute_prepared(prepared, &mut target, FrameDelivery::Surface)
    }

    pub(crate) fn prepare_frame(
        &mut self,
        frame: FrameKey,
        spec: RenderSpec,
    ) -> Result<PreparedFrame, FrameRenderError> {
        let capabilities = self.executor.capabilities().clone();
        prepare_frame_with(
            &self.render,
            &mut self.compiler,
            &mut self.provider,
            &capabilities,
            &mut self.next_generation,
            frame,
            spec,
        )
    }

    fn execute_prepared(
        &mut self,
        prepared: PreparedFrame,
        target: &mut SkiaTarget<'_>,
        delivery: FrameDelivery,
    ) -> Result<FrameEvidence, FrameRenderError> {
        execute_prepared_with(&mut self.executor, prepared, target, delivery)
    }

    /// Record one frame directly into an encoder-owned VideoToolbox surface. The returned token
    /// keeps every CoreVideo/Metal/Skia owner alive until [`Self::finish_shared_frame`] observes
    /// Ganesh completion; callers can queue a small bounded number to overlap GPU work and encode.
    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn render_shared_frame(
        &mut self,
        frame: FrameKey,
        spec: RenderSpec,
        pool: &SharedVideoFramePool,
    ) -> Result<SubmittedSharedFrame, FrameRenderError> {
        if pool.dimensions() != (spec.width(), spec.height()) {
            return Err(FrameRenderError::SharedFrameExtent {
                expected_width: spec.width(),
                expected_height: spec.height(),
                actual_width: pool.dimensions().0,
                actual_height: pool.dimensions().1,
            });
        }
        if self.executor.backend_kind() != SkiaBackendKind::Metal {
            return Err(FrameRenderError::SharedFrameRequiresMetal);
        }
        let started = Instant::now();
        let output = spec.output();
        let prepared = self.prepare_frame(frame, spec)?;
        let mut shared = self.executor.acquire_shared_frame(pool, output)?;
        let surface = shared.surface_mut().map_err(SkiaExecuteError::from)?;
        let mut target = SkiaTarget::direct_gpu(surface, output)?;
        let mut evidence =
            self.execute_prepared(prepared, &mut target, FrameDelivery::SharedGpu)?;
        drop(target);
        let submitted = self.executor.submit_shared_frame(shared)?;
        evidence.timings.total_us = elapsed_us(started);
        Ok(SubmittedSharedFrame {
            submitted,
            evidence,
            started,
        })
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn finish_shared_frame(
        &mut self,
        submitted: SubmittedSharedFrame,
    ) -> Result<(SharedVideoFrame, FrameEvidence), FrameRenderError> {
        let wait_started = Instant::now();
        let frame = self.executor.finish_shared_frame(submitted.submitted)?;
        let wait_us = elapsed_us(wait_started);
        let mut evidence = submitted.evidence;
        evidence.timings.gpu_completion_wait_us = Some(wait_us);
        evidence.timings.total_us = elapsed_us(submitted.started);
        Ok((frame, evidence))
    }

    /// CPU delivery convenience used by preview/PNG/software encode. The same `render_into` path
    /// remains available to a GPU/shared-frame target, so this is a sink choice rather than a
    pub fn render_rgba8(
        &mut self,
        frame: FrameKey,
        spec: RenderSpec,
    ) -> Result<(RgbaFrame, FrameEvidence), FrameRenderError> {
        let prepared = self.prepare_frame(frame, spec)?;
        self.render_prepared_rgba8(prepared, spec)
    }

    pub(crate) fn render_prepared_rgba8(
        &mut self,
        prepared: PreparedFrame,
        spec: RenderSpec,
    ) -> Result<(RgbaFrame, FrameEvidence), FrameRenderError> {
        render_prepared_rgba8_with(
            &mut self.executor,
            &mut self.cpu_delivery_surface,
            prepared,
            spec,
        )
    }

    fn configure_provider_backend(&mut self) -> Result<(), FrameRenderError> {
        self.provider
            .configure_backend(self.executor.backend_kind())
            .map_err(|error| FrameRenderError::ResourceBackendConfiguration {
                reason: error.to_string(),
            })
    }
}

impl<P: ResourceProvider> FramePreparationStage<P> {
    pub(crate) fn prepare_frame(
        &mut self,
        frame: FrameKey,
        spec: RenderSpec,
    ) -> Result<PreparedFrame, FrameRenderError> {
        prepare_frame_with(
            &self.render,
            &mut self.compiler,
            &mut self.provider,
            &self.capabilities,
            &mut self.next_generation,
            frame,
            spec,
        )
    }
}

impl FrameExecutionStage {
    pub(crate) fn render_prepared_rgba8(
        &mut self,
        prepared: PreparedFrame,
        spec: RenderSpec,
    ) -> Result<(RgbaFrame, FrameEvidence), FrameRenderError> {
        render_prepared_rgba8_with(
            &mut self.executor,
            &mut self.cpu_delivery_surface,
            prepared,
            spec,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_frame_with<P: ResourceProvider>(
    render: &EngineRender,
    compiler: &mut FrameCompiler,
    provider: &mut P,
    capabilities: &BackendCapabilities,
    next_generation: &mut u64,
    frame: FrameKey,
    spec: RenderSpec,
) -> Result<PreparedFrame, FrameRenderError> {
    let frame_started = Instant::now();
    let stage_started = Instant::now();
    let render_id = render.render_id();
    let prepared = compiler.evaluate_prepare(render_id, frame, spec)?;
    let evaluate_prepare_us = elapsed_us(stage_started);
    let requests = prepared.prepared().resource_requests.clone();
    let generation = allocate_generation(next_generation)?;
    let overlap_started = Instant::now();
    provider.begin_frame();
    let ((fulfilled, fulfillment_us), (bound, lower_us)) = std::thread::scope(|scope| {
        let lower = scope.spawn(move || {
            let started = Instant::now();
            let result = prepared.lower_constructed(capabilities);
            (result, elapsed_us(started))
        });
        let started = Instant::now();
        let mut fulfilled = ResourceFulfillment::with_capacity(requests.requests().len());
        let fulfillment: Result<ResourceFulfillment, FrameRenderError> = (|| {
            for request in requests.requests() {
                let object =
                    provider
                        .fulfill(request)
                        .map_err(|error| FrameRenderError::Fulfillment {
                            handle: request.handle().get(),
                            reason: error.to_string(),
                        })?;
                fulfilled.push((request.handle(), object));
            }
            Ok(fulfilled)
        })();
        let fulfillment = (fulfillment, elapsed_us(started));
        let lower = lower
            .join()
            .map_err(|_| FrameRenderError::PlannerPanicked)?;
        Ok::<_, FrameRenderError>((fulfillment, lower))
    })?;
    let fulfilled = fulfilled?;
    let resource_report = provider.finish_frame(requests.requests());
    let resource_cache = resource_report.cache;
    let scene3d = resource_report.scene3d;
    let request_count = u64::try_from(requests.requests().len())
        .map_err(|_| FrameRenderError::InvalidResourceCacheEvidence)?;
    let expected_scene3d = u64::try_from(
        requests
            .requests()
            .iter()
            .filter(|request| {
                matches!(
                    request.expected(),
                    valle_engine::resource::ExternalResourceDesc::Scene3d
                )
            })
            .count(),
    )
    .map_err(|_| FrameRenderError::InvalidScene3dEvidence)?;
    if resource_cache.requests != request_count
        || resource_cache.hits.checked_add(resource_cache.misses) != Some(request_count)
        || resource_cache.bypasses > resource_cache.misses
    {
        return Err(FrameRenderError::InvalidResourceCacheEvidence);
    }
    if scene3d.requests != expected_scene3d
        || scene3d
            .prepared_cache_hits
            .checked_add(scene3d.prepared_cache_misses)
            .is_none_or(|prepared| prepared > scene3d.requests)
        || (scene3d.requests == 0
            && (scene3d.prepare_us != 0
                || scene3d.raster_us != 0
                || scene3d.upload_us != 0
                || scene3d.composite_us != 0))
        || scene3d.composite_us != 0
    {
        return Err(FrameRenderError::InvalidScene3dEvidence);
    }
    let bound = bound?;
    let fulfill_lower_wall_us = elapsed_us(overlap_started);
    let template_cache_hit = bound.template_cache_hit();
    let stage_started = Instant::now();
    let bound = bound.bind_constructed(generation)?;
    let (template, bindings) = bound.into_plan();
    let template_hash = template.template_hash()?;
    let execution_passes = template.passes().len();
    let objects: SkiaObjectTable = ExternalObjectTable::try_from_entries(generation, fulfilled)?;
    let bind_us = elapsed_us(stage_started);
    Ok(PreparedFrame {
        started: frame_started,
        render_id,
        frame: frame.index(),
        resource_requests: requests.requests().len(),
        resource_cache,
        scene3d,
        execution_passes,
        template_hash,
        template_cache_hit,
        template,
        bindings,
        objects,
        timings: FrameTimings {
            evaluate_prepare_us,
            fulfillment_us,
            lower_us,
            fulfill_lower_wall_us,
            bind_us,
            execute_us: 0,
            delivery_readback_us: None,
            gpu_completion_wait_us: None,
            total_us: 0,
        },
    })
}

fn allocate_generation(next_generation: &mut u64) -> Result<ExternalGeneration, FrameRenderError> {
    let value = *next_generation;
    *next_generation = value
        .checked_add(1)
        .ok_or(FrameRenderError::GenerationExhausted)?;
    ExternalGeneration::new(value).map_err(Into::into)
}

fn execute_prepared_with(
    executor: &mut SkiaExecutor,
    prepared: PreparedFrame,
    target: &mut SkiaTarget<'_>,
    delivery: FrameDelivery,
) -> Result<FrameEvidence, FrameRenderError> {
    let stage_started = Instant::now();
    let execution = executor.execute(
        &prepared.template,
        &prepared.bindings,
        &prepared.objects,
        target,
    )?;
    let execute_us = elapsed_us(stage_started);
    let mut timings = prepared.timings;
    timings.execute_us = execute_us;
    timings.total_us = elapsed_us(prepared.started);
    let mut scene3d = prepared.scene3d;
    if scene3d.requests > 0 {
        scene3d.composite_us = execute_us;
    }
    Ok(FrameEvidence {
        render_id: prepared.render_id,
        frame: prepared.frame,
        profile: executor.execution_profile(),
        delivery,
        delivery_readback_bytes: 0,
        resource_requests: prepared.resource_requests,
        resource_cache: prepared.resource_cache,
        scene3d,
        execution_passes: prepared.execution_passes,
        template_hash: prepared.template_hash,
        template_cache_hit: prepared.template_cache_hit,
        execution,
        timings,
    })
}

fn render_prepared_rgba8_with(
    executor: &mut SkiaExecutor,
    cpu_delivery_surface: &mut Option<Surface>,
    prepared: PreparedFrame,
    spec: RenderSpec,
) -> Result<(RgbaFrame, FrameEvidence), FrameRenderError> {
    let delivery_started = prepared.started;
    let extent = Extent2d::new(spec.width(), spec.height())?;
    let info = rgba8_target_info(spec.output(), extent)
        .map_err(|error| FrameRenderError::TargetFormat(error.to_string()))?;
    let row_bytes = usize::try_from(spec.width())
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or(FrameRenderError::TargetAllocation)?;
    let size = row_bytes
        .checked_mul(
            usize::try_from(spec.height()).map_err(|_| FrameRenderError::TargetAllocation)?,
        )
        .ok_or(FrameRenderError::TargetAllocation)?;
    let mut surface = match cpu_delivery_surface.take() {
        Some(surface) if surface.image_info() == info => surface,
        _ => surfaces::raster(&info, None, None).ok_or(FrameRenderError::TargetAllocation)?,
    };
    let rendered: Result<FrameEvidence, FrameRenderError> = (|| {
        let mut target = SkiaTarget::new(&mut surface, spec.output())?;
        execute_prepared_with(executor, prepared, &mut target, FrameDelivery::CpuReadback)
    })();
    let mut evidence = match rendered {
        Ok(evidence) => evidence,
        Err(error) => {
            *cpu_delivery_surface = Some(surface);
            return Err(error);
        }
    };
    let readback_started = Instant::now();
    let mut data = vec![0_u8; size];
    if !surface.read_pixels(&info, &mut data, row_bytes, (0, 0)) {
        *cpu_delivery_surface = Some(surface);
        return Err(FrameRenderError::TargetReadback);
    }
    *cpu_delivery_surface = Some(surface);
    evidence.timings.delivery_readback_us = Some(elapsed_us(readback_started));
    evidence.delivery = FrameDelivery::CpuReadback;
    evidence.delivery_readback_bytes = u64::try_from(size).unwrap_or(u64::MAX);
    evidence.timings.total_us = elapsed_us(delivery_started);
    Ok((
        RgbaFrame {
            width: spec.width(),
            height: spec.height(),
            data,
        },
        evidence,
    ))
}

#[derive(Debug, Error)]
pub enum FrameRenderError {
    #[error(transparent)]
    Engine(#[from] ProductEngineError),
    #[error(transparent)]
    Capability(#[from] valle_engine::compositor::lower::CapabilityContractError),
    #[error(transparent)]
    Resource(#[from] ResourceContractError),
    #[error(transparent)]
    Descriptor(#[from] DescriptorError),
    #[error(transparent)]
    Plan(#[from] valle_engine::compositor::lower::PlanValidationError),
    #[error(transparent)]
    Binding(#[from] ExternalBindError),
    #[error(transparent)]
    Executor(#[from] SkiaExecuteError),
    #[error(transparent)]
    Target(#[from] SkiaTargetError),
    #[error(transparent)]
    RenderSpec(#[from] RenderSpecError),
    #[error("resource handle {handle} could not be fulfilled: {reason}")]
    Fulfillment { handle: u32, reason: String },
    #[error("external generation id space exhausted")]
    GenerationExhausted,
    #[error("resource provider generation could not be invalidated: {reason}")]
    ResourceGenerationInvalidation { reason: String },
    #[error("resource provider could not select the compositor backend: {reason}")]
    ResourceBackendConfiguration { reason: String },
    #[error("resource provider returned inconsistent cache evidence")]
    InvalidResourceCacheEvidence,
    #[error("resource provider returned inconsistent Scene3D stage evidence")]
    InvalidScene3dEvidence,
    #[error("the pure RenderPlan lowering worker panicked")]
    PlannerPanicked,
    #[error("could not allocate the requested Skia target")]
    TargetAllocation,
    #[error("invalid CPU delivery target: {0}")]
    TargetFormat(String),
    #[error("could not read the completed CPU target")]
    TargetReadback,
    #[error("shared GPU delivery requires the Metal compositor backend")]
    SharedFrameRequiresMetal,
    #[error(
        "shared GPU pool extent {actual_width}x{actual_height} does not match render extent {expected_width}x{expected_height}"
    )]
    SharedFrameExtent {
        expected_width: u32,
        expected_height: u32,
        actual_width: u32,
        actual_height: u32,
    },
}

fn elapsed_us(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}
