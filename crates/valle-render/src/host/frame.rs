use std::{sync::Arc, time::Instant};
use valle_engine::motion::{
    cpu::CpuBudget,
    memory::{MemoryCredit, MemoryError},
};

use super::{RenderTrace, trace::TraceFrame};
use skia_safe::{Surface, surfaces};
use thiserror::Error;
#[cfg(feature = "native")]
use valle_engine::compositor::inspect::FrameCost;
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

use crate::executor::skia::{
    SkiaBackendKind, SkiaExecuteError, SkiaExecutionProfile, SkiaExecutionReport, SkiaExecutor,
    SkiaExternalObject, SkiaObjectTable, SkiaTarget, SkiaTargetError, cpu_capabilities,
    rgba8_target_info,
};
#[cfg(all(target_os = "macos", feature = "native"))]
use crate::executor::skia::{SubmittedSharedMetalFrame, draw_shared_raster_frame};

/// A complete generation-local fulfillment result. The vector form makes ownership and duplicate
/// handle detection explicit when it is sealed into an object table.
pub type ResourceFulfillment = Vec<(valle_engine::resource::ExternalHandleId, SkiaExternalObject)>;

/// Host-owned decoder/font/shader/Scene3D provider. It receives only the immutable request and
/// render snapshot; authoring evaluation and render-plan structure are deliberately unavailable.
pub trait ResourceProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    fn begin_frame(&mut self) {}

    /// Drop only optional retained data; already fulfilled immutable objects stay alive in
    /// prepared frames. This is called before frame admission, without a compute slot.
    fn trim_memory(&mut self) {}

    fn fulfill(&mut self, request: &ResourceRequest) -> Result<SkiaExternalObject, Self::Error>;

    /// Bytes fulfilling `request` keeps from its source when the descriptor states no extent (a
    /// font or runtime shader). Admission reserves them even for a cached object, which pressure
    /// can evict before this frame fulfils it.
    fn source_resident_bytes(&self, _request: &ResourceRequest) -> u64 {
        0
    }

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
    /// Raster wrote an encoder-owned platform surface in place through a CPU mapping.
    SharedCpu,
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
    pub(crate) memory_credit: Option<MemoryCredit>,
    pub(crate) delivery_memory_bytes: u64,
    trace: Option<TraceFrame>,
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
impl PreparedFrame {
    pub(crate) fn needs_ordered_shadow_context(&self) -> bool {
        // Ganesh's blurred shape masks can round differently after a different
        // cache history. Keep these frames on one persistent context, including
        // subsequent frames after a shadow disappears and later reappears.
        self.bindings.programs().iter().any(|program| {
            program.runtime_program().is_none_or(|program| {
                program.nodes().iter().any(|node| {
                    matches!(node, valle_draw::program::Node::Shadow(shadow)
                        if shadow.sigma_x > 0.0 || shadow.sigma_y > 0.0)
                })
            })
        })
    }
}

#[cfg(all(target_os = "macos", feature = "native"))]
pub(crate) struct SubmittedSharedFrame {
    pub(crate) memory_credit: Option<MemoryCredit>,
    trace: Option<TraceFrame>,
    submitted: SubmittedSharedMetalFrame,
    evidence: FrameEvidence,
    started: Instant,
}

/// The one Native per-frame orchestration path.
pub struct FrameRunner<P> {
    trace: Option<RenderTrace>,
    cpu_budget: Option<CpuBudget>,
    render: EngineRender,
    compiler: FrameCompiler,
    executor: SkiaExecutor,
    provider: P,
    /// Reused CPU delivery target for preview, PNG and software encoders. The Product executor
    /// still renders transactionally into private plan surfaces; this host surface is overwritten
    /// only by the terminal commit and can therefore persist safely across frames.
    cpu_delivery_surface: Option<(Surface, Option<valle_engine::motion::memory::MemoryLease>)>,
    next_generation: u64,
}

#[cfg(feature = "native")]
pub(crate) struct FramePreparationStage<P> {
    admission: Option<Arc<super::admission::FrameAdmission>>,
    trace: Option<RenderTrace>,
    cpu_budget: Option<CpuBudget>,
    render: EngineRender,
    compiler: FrameCompiler,
    provider: P,
    capabilities: BackendCapabilities,
    next_generation: u64,
}

#[cfg(feature = "native")]
pub(crate) struct FrameExecutionStage {
    cpu_budget: Option<CpuBudget>,
    executor: SkiaExecutor,
    cpu_delivery_surface: Option<(Surface, Option<valle_engine::motion::memory::MemoryLease>)>,
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
        Self::new(render, provider, delivery_capabilities()?)
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub fn metal(render: EngineRender, provider: P) -> Result<Self, FrameRenderError> {
        Self::metal_with_capabilities(render, provider, delivery_capabilities()?)
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    fn metal_with_capabilities(
        render: EngineRender,
        provider: P,
        capabilities: BackendCapabilities,
    ) -> Result<Self, FrameRenderError> {
        let compiler = render.frame_compiler();
        let mut runner = Self {
            trace: None,
            cpu_budget: None,
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
            trace: None,
            cpu_budget: None,
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
        let budget = self.cpu_budget.clone();
        budgeted(budget.as_ref(), || {
            prepare_frame_with(
                self.trace.as_ref(),
                &self.render,
                &mut self.compiler,
                &mut self.provider,
                &capabilities,
                &mut self.next_generation,
                frame,
                spec,
                None,
            )
        })
    }

    fn execute_prepared(
        &mut self,
        prepared: PreparedFrame,
        target: &mut SkiaTarget<'_>,
        delivery: FrameDelivery,
    ) -> Result<FrameEvidence, FrameRenderError> {
        let budget = self.cpu_budget.clone();
        budgeted(budget.as_ref(), || {
            execute_prepared_with(&mut self.executor, prepared, target, delivery)
        })
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
        let budget = self.cpu_budget.clone();
        budgeted(budget.as_ref(), || {
            render_prepared_rgba8_with(
                &mut self.executor,
                &mut self.cpu_delivery_surface,
                prepared,
                spec,
            )
        })
    }

    fn configure_provider_backend(&mut self) -> Result<(), FrameRenderError> {
        self.provider
            .configure_backend(self.executor.backend_kind())
            .map_err(|error| FrameRenderError::ResourceBackendConfiguration {
                reason: error.to_string(),
            })
    }
}

#[cfg(feature = "native")]
impl<P: ResourceProvider> FramePreparationStage<P> {
    pub(crate) fn set_admission(
        &mut self,
        admission: Option<Arc<super::admission::FrameAdmission>>,
    ) {
        self.admission = admission;
    }
    pub(crate) fn set_cpu_budget(&mut self, budget: CpuBudget) {
        self.cpu_budget = Some(budget);
    }
    pub(crate) fn set_trace(&mut self, trace: Option<RenderTrace>) {
        self.trace = trace;
    }
    pub(crate) fn new(
        render: EngineRender,
        mut provider: P,
        backend: SkiaBackendKind,
        capabilities: BackendCapabilities,
    ) -> Result<Self, FrameRenderError> {
        provider.configure_backend(backend).map_err(|error| {
            FrameRenderError::ResourceBackendConfiguration {
                reason: error.to_string(),
            }
        })?;
        Ok(Self {
            admission: None,
            trace: None,
            cpu_budget: None,
            compiler: render.frame_compiler(),
            render,
            provider,
            capabilities,
            next_generation: 1,
        })
    }

    /// Preparers feeding one executor share its generation namespace. Assign by delivery
    /// sequence, not worker-local counters or frame keys (which can repeat during seeks).
    pub(crate) fn prepare_frame_for_sequence(
        &mut self,
        sequence: usize,
        frame: FrameKey,
        spec: RenderSpec,
    ) -> Result<PreparedFrame, FrameRenderError> {
        self.next_generation = u64::try_from(sequence)
            .ok()
            .and_then(|sequence| sequence.checked_add(1))
            .ok_or(FrameRenderError::GenerationExhausted)?;
        self.prepare_frame(frame, spec)
    }

    pub(crate) fn prepare_frame(
        &mut self,
        frame: FrameKey,
        spec: RenderSpec,
    ) -> Result<PreparedFrame, FrameRenderError> {
        let budget = self.cpu_budget.clone();
        budgeted(budget.as_ref(), || {
            prepare_frame_with(
                self.trace.as_ref(),
                &self.render,
                &mut self.compiler,
                &mut self.provider,
                &self.capabilities,
                &mut self.next_generation,
                frame,
                spec,
                self.admission.as_deref(),
            )
        })
    }
}

#[cfg(feature = "native")]
impl FrameExecutionStage {
    pub(crate) fn trim_memory(&mut self) {
        self.cpu_delivery_surface = None;
        self.executor.invalidate_surface_generation();
    }
    pub(crate) fn for_backend(
        backend: SkiaBackendKind,
        capabilities: BackendCapabilities,
        limits: Option<super::budget::OwnerMemoryLimits>,
        cpu_budget: CpuBudget,
    ) -> Result<Self, FrameRenderError> {
        let mut executor = match backend {
            SkiaBackendKind::Raster => SkiaExecutor::new(capabilities)?,
            #[cfg(target_os = "macos")]
            SkiaBackendKind::Metal => SkiaExecutor::metal(capabilities)?,
        };
        if let Some(limits) = limits {
            executor.set_cache_limits(limits.caches);
        }
        Ok(Self {
            cpu_budget: Some(cpu_budget),
            executor,
            cpu_delivery_surface: None,
        })
    }

    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn opacity_layer_cache_usage(&self) -> (usize, bool) {
        self.executor.opacity_layer_cache_usage()
    }

    /// Record one frame directly into an encoder-owned VideoToolbox surface. The returned token
    /// keeps every CoreVideo/Metal/Skia owner alive until [`Self::finish_shared_frame`] observes
    /// Ganesh completion; callers can queue a small bounded number to overlap GPU work and encode.
    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn render_prepared_shared_frame(
        &mut self,
        prepared: PreparedFrame,
        spec: RenderSpec,
        pool: &SharedVideoFramePool,
    ) -> Result<SubmittedSharedFrame, FrameRenderError> {
        let memory_credit = prepared.memory_credit.clone();
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
        let started = prepared.started;
        let trace = prepared.trace.clone();
        let acquire_span = trace.as_ref().map(|t| t.span("delivery.surface_acquire"));
        let output = spec.output();
        let mut shared = drained(|| self.executor.acquire_shared_frame(pool, output))?;
        drop(acquire_span);
        let surface = shared.surface_mut().map_err(SkiaExecuteError::from)?;
        let mut target = SkiaTarget::direct_gpu(surface, output)?;
        let budget = self.cpu_budget.clone();
        let mut evidence = budgeted(budget.as_ref(), || {
            execute_prepared_with(
                &mut self.executor,
                prepared,
                &mut target,
                FrameDelivery::SharedGpu,
            )
        })?;
        drop(target);
        let submit_span = trace.as_ref().map(|t| t.span("execute.submit"));
        let submitted = drained(|| self.executor.submit_shared_frame(shared))?;
        drop(submit_span);
        evidence.timings.total_us = elapsed_us(started);
        Ok(SubmittedSharedFrame {
            memory_credit,
            trace,
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
        let wait_span = submitted
            .trace
            .as_ref()
            .map(|t| t.span("execute.gpu_completion_wait"));
        let wait_started = Instant::now();
        let frame = drained(|| self.executor.finish_shared_frame(submitted.submitted))?;
        let wait_us = elapsed_us(wait_started);
        drop(wait_span);
        let mut evidence = submitted.evidence;
        evidence.timings.gpu_completion_wait_us = Some(wait_us);
        evidence.timings.total_us = elapsed_us(submitted.started);
        Ok((frame, evidence))
    }

    /// Write one Raster frame straight into an encoder-owned VideoToolbox surface. The output
    /// commit lands in the surface's CPU mapping, so there is no readback, and the encoder rather
    /// than the CPU converts it to YUV.
    #[cfg(all(target_os = "macos", feature = "native"))]
    pub(crate) fn render_prepared_shared_raster(
        &mut self,
        prepared: PreparedFrame,
        spec: RenderSpec,
        pool: &SharedVideoFramePool,
    ) -> Result<(SharedVideoFrame, FrameEvidence), FrameRenderError> {
        if pool.dimensions() != (spec.width(), spec.height()) {
            return Err(FrameRenderError::SharedFrameExtent {
                expected_width: spec.width(),
                expected_height: spec.height(),
                actual_width: pool.dimensions().0,
                actual_height: pool.dimensions().1,
            });
        }
        let started = prepared.started;
        let extent = Extent2d::new(spec.width(), spec.height())?;
        let info = rgba8_target_info(spec.output(), extent)
            .map_err(|error| FrameRenderError::TargetFormat(error.to_string()))?
            .with_color_type(skia_safe::ColorType::BGRA8888);
        let budget = self.cpu_budget.clone();
        let executor = &mut self.executor;
        let (frame, rendered) = budgeted(budget.as_ref(), || {
            draw_shared_raster_frame(pool, &info, |surface| {
                let mut target = SkiaTarget::new(surface, spec.output())?;
                execute_prepared_with(executor, prepared, &mut target, FrameDelivery::SharedCpu)
            })
        })
        .map_err(SkiaExecuteError::from)?;
        let mut evidence = rendered?;
        evidence.timings.total_us = elapsed_us(started);
        Ok((frame, evidence))
    }

    pub(crate) fn render_prepared_rgba8(
        &mut self,
        prepared: PreparedFrame,
        spec: RenderSpec,
    ) -> Result<(RgbaFrame, FrameEvidence), FrameRenderError> {
        let budget = self.cpu_budget.clone();
        budgeted(budget.as_ref(), || {
            render_prepared_rgba8_with(
                &mut self.executor,
                &mut self.cpu_delivery_surface,
                prepared,
                spec,
            )
        })
    }
}

#[cfg(feature = "native")]
impl FramePreparationStage<super::NativeResourceProvider> {
    pub(crate) fn share_resources(
        self,
        backend: SkiaBackendKind,
    ) -> Result<
        FramePreparationStage<super::shared_resource::SharedResourceProvider>,
        FrameRenderError,
    > {
        let provider = super::shared_resource::SharedResourceProvider::new(self.provider, backend)
            .map_err(|error| FrameRenderError::ResourceBackendConfiguration {
                reason: error.to_string(),
            })?;
        Ok(FramePreparationStage {
            admission: self.admission,
            trace: self.trace,
            cpu_budget: self.cpu_budget,
            render: self.render,
            compiler: self.compiler,
            provider,
            capabilities: self.capabilities,
            next_generation: self.next_generation,
        })
    }
}

#[cfg(feature = "native")]
impl FramePreparationStage<super::shared_resource::SharedResourceProvider> {
    pub(crate) fn fork(&self) -> Self {
        Self {
            admission: self.admission.clone(),
            trace: self.trace.clone(),
            cpu_budget: self.cpu_budget.clone(),
            compiler: self.render.frame_compiler(),
            render: self.render.clone(),
            provider: self.provider.clone(),
            capabilities: self.capabilities.clone(),
            next_generation: 1,
        }
    }
}

/// Runs one frame stage under the CPU allowance, in its own autorelease pool.
fn budgeted<R>(budget: Option<&CpuBudget>, work: impl FnOnce() -> R) -> R {
    drained(|| match budget {
        Some(budget) => budget.run(work),
        None => work(),
    })
}

/// Metal hands back autoreleased objects (command encoders, blit contexts, transfer buffers), and
/// pipeline threads have no autorelease pool, so those objects lived until their thread exited: at
/// 4K, CPU readback grew GPU memory with every frame. Each frame stage drains its own pool.
#[cfg(all(target_os = "macos", feature = "native"))]
fn drained<R>(work: impl FnOnce() -> R) -> R {
    objc2::rc::autoreleasepool(|_| work())
}

#[cfg(not(all(target_os = "macos", feature = "native")))]
fn drained<R>(work: impl FnOnce() -> R) -> R {
    work()
}

#[allow(clippy::too_many_arguments)]
fn prepare_frame_with<P: ResourceProvider>(
    trace: Option<&RenderTrace>,
    render: &EngineRender,
    compiler: &mut FrameCompiler,
    provider: &mut P,
    capabilities: &BackendCapabilities,
    next_generation: &mut u64,
    frame: FrameKey,
    spec: RenderSpec,
    admission: Option<&super::admission::FrameAdmission>,
) -> Result<PreparedFrame, FrameRenderError> {
    let trace = trace.map(|t| t.frame(frame.index()));
    let evaluate_span = trace.as_ref().map(|t| t.span("prepare.evaluate"));
    let frame_started = Instant::now();
    let stage_started = Instant::now();
    let render_id = render.render_id();
    let prepared = compiler.evaluate_prepare(render_id, frame, spec)?;
    let evaluate_prepare_us = elapsed_us(stage_started);
    drop(evaluate_span);
    if let Some(trace) = &trace {
        for (owner, stats) in render.compiled().motion_instance_cache_stats() {
            trace.counters(
                "prepare.instance_cache",
                serde_json::json!({"owner":owner,"scope":"admitted-motion-lifetime","stats":stats}),
            );
        }
    }
    let requests = prepared.prepared().resource_requests.clone();
    let generation = allocate_generation(next_generation)?;
    let overlap_started = Instant::now();
    let lower_span = trace.as_ref().map(|t| t.span("lower"));
    let lower_started = Instant::now();
    let bound = prepared.lower_constructed(capabilities)?;
    let lower_us = elapsed_us(lower_started);
    drop(lower_span);
    let template_cache_hit = bound.template_cache_hit();
    let bind_span = trace.as_ref().map(|t| t.span("bind"));
    let bind_started = Instant::now();
    let (template, bindings) = bound.bind_constructed(generation)?.into_plan();
    let bind_us = elapsed_us(bind_started);
    drop(bind_span);
    let delivery_memory_bytes =
        admission.map_or(0, |a| a.delivery_bytes((generation.get() - 1) as usize));
    let admitted = admission
        .map(|admission| {
            let (bytes, execution_bytes) = frame_working_bytes(
                &template,
                &bindings,
                capabilities,
                requests.requests(),
                &*provider,
                spec,
                admission,
            )?;
            let bytes = super::admission::checked_add(bytes, delivery_memory_bytes)?;
            let execution_bytes =
                super::admission::checked_add(execution_bytes, delivery_memory_bytes)?;
            let credit = admission.admit((generation.get() - 1) as usize, bytes, || {
                provider.trim_memory()
            })?;
            Ok::<_, FrameRenderError>((credit, execution_bytes))
        })
        .transpose()?;
    let memory_credit = admitted.as_ref().map(|(credit, _)| credit.clone());
    provider.begin_frame();
    let fulfillment_span = trace.as_ref().map(|t| t.span("resource.fulfill"));
    let fulfillment_started = Instant::now();
    let mut fulfill = || -> Result<ResourceFulfillment, FrameRenderError> {
        let mut fulfilled = ResourceFulfillment::with_capacity(requests.requests().len());
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
    };
    let fulfilled = match &memory_credit {
        Some(credit) => credit.run(fulfill),
        None => fulfill(),
    }?;
    if let Some((credit, execution_bytes)) = admitted {
        credit.limit_remaining(execution_bytes);
    }
    let fulfillment_us = elapsed_us(fulfillment_started);
    drop(fulfillment_span);
    let resource_report = provider.finish_frame(requests.requests());
    let resource_cache = resource_report.cache;
    if let Some(trace) = &trace {
        trace.counters("resource.cache", serde_json::json!({
        "hits": resource_cache.hits, "misses": resource_cache.misses,
        "residentBytes": resource_cache.resident_bytes, "residentEntries": resource_cache.resident_entries,
        "scope": "this resource provider", "scene3dPrepareUs": resource_report.scene3d.prepare_us,
        "scene3dRasterUs": resource_report.scene3d.raster_us, "scene3dUploadUs": resource_report.scene3d.upload_us,
    }));
    }
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
    let fulfill_lower_wall_us = elapsed_us(overlap_started);
    let template_hash = template.template_hash()?;
    let execution_passes = template.passes().len();
    let objects: SkiaObjectTable = ExternalObjectTable::try_from_entries(generation, fulfilled)?;
    Ok(PreparedFrame {
        memory_credit,
        delivery_memory_bytes: super::admission::checked_add(
            delivery_memory_bytes,
            8 * 1024 * 1024,
        )?,
        trace,
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

/// The immutable contract both product backends lower, bind and execute against.
pub(crate) fn delivery_capabilities() -> Result<BackendCapabilities, FrameRenderError> {
    Ok(cpu_capabilities(
        Extent2d::new(16_384, 16_384)?,
        2 * 1024 * 1024 * 1024,
        4 * 1024 * 1024 * 1024,
    )?)
}

/// Plan one frame as delivery binds it, without fulfilling its resources or executing it.
#[cfg(feature = "native")]
pub(crate) fn plan_frame_cost(
    render: &EngineRender,
    compiler: &mut FrameCompiler,
    capabilities: &BackendCapabilities,
    frame: FrameKey,
    spec: RenderSpec,
) -> Result<FrameCost, FrameRenderError> {
    let (template, bindings) = compiler
        .evaluate_prepare(render.render_id(), frame, spec)?
        .lower_constructed(capabilities)?
        .bind_constructed(ExternalGeneration::new(1)?)?
        .into_plan();
    let schedules = template
        .bind_program_schedules(&bindings, capabilities)
        .map_err(SkiaExecuteError::from)?;
    Ok(FrameCost::of(&template, &bindings, &schedules))
}

fn allocate_generation(next_generation: &mut u64) -> Result<ExternalGeneration, FrameRenderError> {
    let value = *next_generation;
    *next_generation = value
        .checked_add(1)
        .ok_or(FrameRenderError::GenerationExhausted)?;
    ExternalGeneration::new(value).map_err(Into::into)
}

// Admission covers what this frame can charge to its credit: fulfillment's resources, then the
// executor's planned peak and delivery. Leases track owned storage and declared scratch claims;
// they are neither allocator instrumentation nor a measurement of whole-process memory.
#[allow(clippy::too_many_arguments)]
fn frame_working_bytes<P: ResourceProvider>(
    template: &RenderPlanTemplate,
    bindings: &RenderBindings,
    caps: &BackendCapabilities,
    requests: &[ResourceRequest],
    provider: &P,
    spec: RenderSpec,
    admission: &super::admission::FrameAdmission,
) -> Result<(u64, u64), FrameRenderError> {
    use super::admission::checked_add;
    use valle_engine::resource::{ExternalResourceDesc, ResourcePayload};
    let schedules = template
        .bind_program_schedules(bindings, caps)
        .map_err(SkiaExecuteError::from)?;
    let mut resources = 0_u64;
    let mut scene3d_working = 0_u64;
    let mut scene3d = Vec::new();
    for request in requests {
        match request.expected() {
            ExternalResourceDesc::VisualFrame { extent, .. }
            | ExternalResourceDesc::DataTexture { extent } => {
                let bytes = u64::from(extent.width()) * u64::from(extent.height()) * 32;
                resources = checked_add(resources, bytes)?;
            }
            ExternalResourceDesc::Scene3d => {
                let Some(ResourcePayload::Scene3dFrame(frame)) = request.payload() else {
                    continue;
                };
                let frame = frame.request();
                let pixels = u64::from(frame.width) * u64::from(frame.height);
                // Layers rasterize one at a time, each releasing its working set before the next
                // starts; every result (colour, plus depth for deferred depth of field) stays.
                resources = checked_add(resources, pixels * 12)?;
                scene3d_working = scene3d_working.max(pixels * 96);
                scene3d.push(Extent2d::new(frame.width, frame.height)?);
            }
            _ => resources = checked_add(resources, provider.source_resident_bytes(request))?,
        }
    }
    let execution_bytes = crate::executor::skia::planned_frame_bytes(
        template,
        bindings,
        &schedules,
        admission.backend(),
        admission.pool_bytes(),
        &scene3d,
    )?;
    // The RGBA readback and the delivery surfaces an executor keeps (CPU target, Metal output).
    let output = u64::from(spec.width()) * u64::from(spec.height()) * 4;
    let execution_bytes = checked_add(execution_bytes, output * 3)?;
    let bytes = checked_add(checked_add(execution_bytes, resources)?, scene3d_working)?;
    Ok((bytes, execution_bytes))
}

fn execute_prepared_with(
    executor: &mut SkiaExecutor,
    prepared: PreparedFrame,
    target: &mut SkiaTarget<'_>,
    delivery: FrameDelivery,
) -> Result<FrameEvidence, FrameRenderError> {
    let execute_span = prepared.trace.as_ref().map(|t| t.span("execute.record"));
    let stage_started = Instant::now();
    let mut execute = || {
        executor.execute(
            &prepared.template,
            &prepared.bindings,
            &prepared.objects,
            target,
        )
    };
    let execution = match &prepared.memory_credit {
        Some(credit) => credit.run(execute),
        None => execute(),
    }?;
    let execute_us = elapsed_us(stage_started);
    drop(execute_span);
    if let Some(trace) = &prepared.trace {
        trace.counters("execute.cache", serde_json::json!({
        "scope": "this executor", "programHits": execution.program_cache_hits,
        "programMisses": execution.program_cache_misses, "rasterLayerHits": execution.raster_layer_cache_hits,
        "rasterLayerBytes": execution.raster_layer_cache_bytes,
        "surfacePoolBytes": execution.surfaces.pool_bytes,
        "uploadBytes": execution.cpu_upload_bytes,
    }));
    }
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
    cpu_delivery_surface: &mut Option<(Surface, Option<valle_engine::motion::memory::MemoryLease>)>,
    prepared: PreparedFrame,
    spec: RenderSpec,
) -> Result<(RgbaFrame, FrameEvidence), FrameRenderError> {
    let delivery_started = prepared.started;
    let trace = prepared.trace.clone();
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
    let (mut surface, memory) = match cpu_delivery_surface.take() {
        Some((surface, memory)) if surface.image_info() == info => (surface, memory),
        _ => {
            let reserve = || {
                valle_engine::motion::memory::reserve(
                    valle_engine::motion::memory::MemoryKind::Delivery,
                    size as u64,
                )
            };
            let memory = match &prepared.memory_credit {
                Some(credit) => credit.run(reserve),
                None => reserve(),
            }?;
            (
                surfaces::raster(&info, None, None).ok_or(FrameRenderError::TargetAllocation)?,
                memory,
            )
        }
    };
    let rendered: Result<FrameEvidence, FrameRenderError> = (|| {
        let mut target = SkiaTarget::new(&mut surface, spec.output())?;
        execute_prepared_with(executor, prepared, &mut target, FrameDelivery::CpuReadback)
    })();
    let mut evidence = match rendered {
        Ok(evidence) => evidence,
        Err(error) => {
            *cpu_delivery_surface = Some((surface, memory));
            return Err(error);
        }
    };
    let readback_span = trace.as_ref().map(|t| t.span("delivery.readback"));
    let readback_started = Instant::now();
    let mut data = vec![0_u8; size];
    if !surface.read_pixels(&info, &mut data, row_bytes, (0, 0)) {
        *cpu_delivery_surface = Some((surface, memory));
        return Err(FrameRenderError::TargetReadback);
    }
    *cpu_delivery_surface = Some((surface, memory));
    evidence.timings.delivery_readback_us = Some(elapsed_us(readback_started));
    drop(readback_span);
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
    Memory(#[from] MemoryError),
    #[error("frame memory admission was cancelled or a sibling stage failed")]
    AdmissionStopped,
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

impl FrameRenderError {
    /// Whether the frame exceeds a render budget at this output size. These are the failures
    /// `motion check --cost` lists for every frame before rendering.
    pub fn exceeds_budget(&self) -> bool {
        match self {
            Self::Engine(error) => error.exceeds_budget(),
            Self::Executor(SkiaExecuteError::ProgramBinding(error)) => error.exceeds_budget(),
            _ => false,
        }
    }

    /// The semantic path of the visual, node or pass that failed, when the error names one.
    pub fn path(&self) -> Option<&str> {
        match self {
            Self::Engine(error) => error.path(),
            Self::Executor(SkiaExecuteError::ProgramBinding(error)) => error.path(),
            _ => None,
        }
    }
}

fn elapsed_us(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}
