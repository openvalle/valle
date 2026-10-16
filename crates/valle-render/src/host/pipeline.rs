//! One Native frame loop for every delivery mode.

mod tasks;

#[cfg(target_os = "macos")]
use std::collections::VecDeque;
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use thiserror::Error;
use valle_engine::{
    frame::RenderSpec,
    render::{FrameKey, RenderId},
    resource::ContentDigest,
};
use valle_media::{SharedVideoFrame, SharedVideoFramePool};

use super::DeliveredSharedFrame;
#[cfg(target_os = "macos")]
use super::gpu_schedule::{AdaptiveQueues, QueueSample, metal_memory};
use super::{DeliveredFrame, FrameDelivery, FrameRenderError, FrameSink, NativeProject};
use crate::executor::skia::{SkiaBackendKind, SkiaExecutionProfile};

#[derive(Clone, Default)]
pub struct RenderControl {
    trace: Option<super::RenderTrace>,
    admission: Option<Arc<super::admission::FrameAdmission>>,
    cancelled: Arc<AtomicBool>,
    deadline: Option<Instant>,
    budget: Option<super::NativeRenderBudget>,
    residency_owners: super::budget::ResidencyOwners,
    maximum_gpu_contexts: usize,
}

impl RenderControl {
    /// Keep ordered frame roots on their controllers and share the pool between their kernels.
    /// A kernel join must not steal another complete frame and delay its own ordered delivery.
    fn compute<R: Send>(&self, work: impl FnOnce() -> R + Send) -> R {
        self.budget
            .as_ref()
            .expect("schedule resolves a host budget")
            .cpu
            .run(work)
    }

    pub fn with_budget(mut self, budget: super::NativeRenderBudget) -> Self {
        self.budget = Some(budget);
        self
    }

    fn execution_stage(
        &self,
        _project: &NativeProject,
        backend: SkiaBackendKind,
        spec: RenderSpec,
    ) -> Result<super::frame::FrameExecutionStage, PipelineError> {
        let budget = self.budget.as_ref().expect("resolved host budget");
        let limits = budget.owner_limits(self.residency_owners, spec)?;
        let capabilities = self.stage_capabilities()?;
        Ok(super::frame::FrameExecutionStage::for_backend(
            backend,
            capabilities,
            limits,
            budget.cpu.clone(),
        )?)
    }

    /// A frame's surfaces never exceed the whole budget; whether they fit beside other storage
    /// is admission's decision, which may evict optional retention to make room.
    fn stage_capabilities(
        &self,
    ) -> Result<valle_engine::compositor::lower::BackendCapabilities, PipelineError> {
        let budget = self.budget.as_ref().unwrap().memory_bytes();
        let capabilities = crate::executor::skia::cpu_capabilities(
            valle_engine::resource::Extent2d::new(16_384, 16_384).unwrap(),
            budget.map_or(2 * 1024 * 1024 * 1024, |b| b.min(2 * 1024 * 1024 * 1024)),
            budget.map_or(4 * 1024 * 1024 * 1024, |b| b.min(4 * 1024 * 1024 * 1024)),
        )
        .map_err(FrameRenderError::from)?;
        Ok(capabilities)
    }

    fn preparation_stage(
        &self,
        project: &NativeProject,
        backend: SkiaBackendKind,
        spec: RenderSpec,
    ) -> Result<
        super::frame::FramePreparationStage<super::shared_resource::SharedResourceProvider>,
        PipelineError,
    > {
        let budget = self.budget.as_ref().unwrap();
        let limits = budget.owner_limits(self.residency_owners, spec)?;
        let provider = super::NativeResourceProvider::with_cache_limits(
            Arc::clone(project.catalog()),
            limits.map_or_else(super::NativeResourceCacheLimits::default, |l| {
                super::NativeResourceCacheLimits::new(256, l.resource_cache).unwrap()
            }),
        )
        .with_render_resources(project.compiled());
        let mut preparer = super::frame::FramePreparationStage::new(
            project.render().clone(),
            provider,
            backend,
            self.stage_capabilities()?,
        )?
        .share_resources(backend)?;
        preparer.set_cpu_budget(budget.cpu.clone());
        preparer.set_trace(self.trace.clone());
        preparer.set_admission(self.admission.clone());
        Ok(preparer)
    }

    pub fn with_trace(mut self, trace: super::RenderTrace) -> Self {
        self.trace = Some(trace);
        self
    }

    pub fn with_deadline(deadline: Instant) -> Self {
        Self {
            deadline: Some(deadline),
            ..Self::default()
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn check(&self) -> Result<(), PipelineError> {
        if self.is_cancelled() {
            return Err(PipelineError::Cancelled);
        }
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(PipelineError::Deadline);
        }
        Ok(())
    }
}

fn budget_pressure(control: &RenderControl) -> bool {
    control
        .budget
        .as_ref()
        .is_some_and(|budget| budget.memory.under_pressure())
}

pub type ProgressCallback = Arc<dyn Fn(usize, usize) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineReport {
    pub render_id: RenderId,
    pub frames: usize,
    pub profile: Option<SkiaExecutionProfile>,
    pub delivery: Option<FrameDelivery>,
    pub resource_requests: usize,
    pub execution_passes: usize,
    pub unique_templates: usize,
    pub preparation_tasks: usize,
    pub maximum_in_flight_frames: usize,
    pub budget: Option<super::NativeBudgetReport>,
    pub template_cache_hits: usize,
    pub template_cache_misses: usize,
    pub resource_cache_hits: u64,
    pub resource_cache_misses: u64,
    pub resource_cache_insertions: u64,
    pub resource_cache_evictions: u64,
    pub resource_cache_bypasses: u64,
    pub resource_cache_generation_invalidations: u64,
    pub maximum_resource_cache_entries: usize,
    pub maximum_resource_cache_bytes: u64,
    pub scene3d_requests: u64,
    pub scene3d_prepared_cache_hits: u64,
    pub scene3d_prepared_cache_misses: u64,
    pub scene3d_prepare_us: u64,
    pub scene3d_raster_us: u64,
    pub scene3d_upload_us: u64,
    pub scene3d_composite_us: u64,
    pub program_cache_hits: u64,
    pub program_cache_misses: u64,
    pub raster_layer_cache_hits: u64,
    pub raster_layer_cache_misses: u64,
    pub maximum_raster_layer_cache_entries: usize,
    /// Largest per-worker retained ROI pixel estimate, not process or GPU allocation size.
    pub maximum_raster_layer_cache_bytes: usize,
    /// Largest per-worker cache observed; these are not summed across concurrent workers.
    pub maximum_program_cache_entries: usize,
    pub maximum_program_cache_cost_bytes: usize,
    pub font_cache_hits: u64,
    pub font_cache_misses: u64,
    pub shader_cache_hits: u64,
    pub shader_cache_misses: u64,
    pub built_in_kernel_cache_hits: u64,
    pub built_in_kernel_cache_misses: u64,
    pub external_gpu_imports: u64,
    pub cpu_upload_bytes: u64,
    pub surface_generation_invalidations: u64,
    pub plan_surface_backend_allocations: u64,
    pub plan_surface_backend_reuses: u64,
    pub total_surface_allocations: u64,
    pub unpooled_surface_allocations: u64,
    pub scratch_surface_allocations: u64,
    pub scratch_surface_reuses: u64,
    pub maximum_scratch_surfaces: usize,
    pub maximum_scratch_bytes: u64,
    pub maximum_scratch_pool_surfaces: usize,
    pub maximum_scratch_pool_bytes: u64,
    pub maximum_plan_surface_slots: usize,
    pub maximum_plan_logical_allocations: usize,
    pub maximum_plan_physical_bytes: u64,
    pub maximum_plan_logical_bytes: u64,
    pub maximum_plan_alias_saved_bytes: u64,
    pub maximum_plan_estimated_peak_bytes: u64,
    pub maximum_surface_pool_surfaces: usize,
    pub maximum_surface_pool_bytes: u64,
    pub surface_pool_evictions: u64,
    pub evaluate_prepare_us: u64,
    pub fulfillment_work_us: u64,
    pub lower_work_us: u64,
    pub fulfill_lower_wall_us: u64,
    pub bind_us: u64,
    pub execute_us: u64,
    pub delivery_readback_us: u64,
    pub delivery_readback_bytes: u64,
    pub gpu_completion_wait_us: u64,
    pub maximum_frame_us: u64,
}

/// Executes frame keys in caller order through the same staged Product Compositor runner.
pub fn render_schedule(
    project: &NativeProject,
    frames: &[FrameKey],
    spec: RenderSpec,
    sink: &mut dyn FrameSink,
    control: &RenderControl,
    progress: Option<&ProgressCallback>,
    backend: SkiaBackendKind,
) -> Result<PipelineReport, PipelineError> {
    let mut resolved = control.clone();
    let budget = resolved
        .budget
        .get_or_insert_with(super::NativeRenderBudget::default)
        .clone();
    if budget.cpu.is_current() {
        return Err(super::NativeBudgetError::ReentrantSession.into());
    }
    let workers =
        budget.frame_parallelism(budget.threads().min(4).min(frames.len()).max(1), spec)?;
    // Raster writes an encoder's shared frames on its usual pipeline; only GPU backends stage them.
    let shared_gpu = backend != SkiaBackendKind::Raster && sink.shared_frame_pool().is_some();
    let contexts = if shared_gpu { 3 } else { 1 };
    let owners = super::budget::ResidencyOwners {
        providers: 1,
        executors: if shared_gpu {
            contexts
        } else if backend == SkiaBackendKind::Raster {
            workers
        } else {
            1
        },
        gpu: backend != SkiaBackendKind::Raster,
    };
    let pool_bytes = budget
        .owner_limits(owners, spec)?
        .map_or(0, |limits| limits.caches.surface_pool);
    resolved.residency_owners = owners;
    resolved.maximum_gpu_contexts = contexts;
    resolved.admission = if budget.memory_bytes().is_some() {
        let delivery_bytes = (0..frames.len())
            .map(|sequence| sink.working_bytes(sequence))
            .collect::<anyhow::Result<Vec<_>>>()
            .map_err(|error| PipelineError::Sink(error.to_string()))?;
        Some(Arc::new(
            super::admission::FrameAdmission::new(
                budget.memory.clone(),
                resolved.cancelled.clone(),
                resolved.deadline,
                backend,
                pool_bytes,
            )
            .with_delivery_bytes(delivery_bytes),
        ))
    } else {
        None
    };
    let control = &resolved;
    let mut traced_sink = TraceSink {
        sink,
        trace: control.trace.as_ref(),
        cpu: &budget.cpu,
    };
    let sink: &mut dyn FrameSink = &mut traced_sink;
    let _pipeline_span = control.trace.as_ref().map(|t| {
        t.span(
            "pipeline",
            serde_json::json!({"frames": frames.len(), "backend": format!("{backend:?}")}),
        )
    });
    if let Some(trace) = &control.trace {
        trace.evidence("runtime.budget", serde_json::json!({"cpuThreads":budget.threads(), "memoryBudgetBytes":budget.memory_bytes(), "resourceProviders":control.residency_owners.providers,"executors":control.residency_owners.executors,
            "covered":"shared frame reservations, resource images, working and retained surfaces, conservative CPU filter/3D scratch and decoded media windows",
            "notCharged":"immutable compiled inputs, bounded plan/topology metadata, driver and codec implementation allocations; managed claims are not process RSS"}));
    }
    let result = match sink.shared_frame_pool() {
        Some(pool) if shared_gpu => {
            #[cfg(target_os = "macos")]
            {
                render_shared_staged(
                    project, frames, spec, sink, control, progress, backend, pool, workers,
                )
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = pool;
                Err(PipelineError::SharedGpuUnavailable)
            }
        }
        pool => tasks::render(
            project, frames, spec, sink, control, progress, backend, workers, pool,
        ),
    };
    if let Some(admission) = &control.admission {
        admission.stop();
    }
    let mut report = result?;
    report.budget = Some(budget.report(control.residency_owners, spec)?);
    if let Some(trace) = &control.trace {
        trace.evidence("runtime.budget.complete", serde_json::json!({"cpuThreads":budget.threads(),"peakCpuSlots":budget.cpu.peak_slots(),"capacityBytes":report.budget.unwrap().configured_capacity_bytes}));
    }
    Ok(report)
}

#[cfg(target_os = "macos")]
struct PendingSharedFrame {
    executor: usize,
    sequence: usize,
    key: FrameKey,
    submitted: super::SubmittedSharedFrame,
}

#[cfg(target_os = "macos")]
fn render_shared_staged(
    project: &NativeProject,
    frames: &[FrameKey],
    spec: RenderSpec,
    sink: &mut dyn FrameSink,
    control: &RenderControl,
    progress: Option<&ProgressCallback>,
    backend: SkiaBackendKind,
    pool: SharedVideoFramePool,
    workers: usize,
) -> Result<PipelineReport, PipelineError> {
    const MAXIMUM_PENDING_FRAMES: usize = 3;

    let first_preparer = control.preparation_stage(project, backend, spec)?;
    let executor = control.execution_stage(project, backend, spec)?;
    // CPU preparation remains parallel. Additional GPU contexts are allocated only
    // after sustained GPU completion waits justify a bounded throughput probe.
    let mut executors = vec![executor];
    let mut active_queues = 1;
    let mut queues = AdaptiveQueues::new(control.maximum_gpu_contexts);
    let mut ordered_shadows = false;
    if std::env::var_os("VALLE_PERF").is_some() {
        eprintln!(
            "[valle metal] shared-queues=1 adaptive=true pending-limit={MAXIMUM_PENDING_FRAMES}"
        );
    }
    let mut preparers = Vec::with_capacity(workers);
    for _ in 1..workers {
        preparers.push(first_preparer.fork());
    }
    preparers.insert(0, first_preparer);
    let abort = Arc::new(AtomicBool::new(false));
    let mut metrics = PipelineAccumulator::for_render_id(project.render_id());
    let mut pending = VecDeque::with_capacity(MAXIMUM_PENDING_FRAMES);
    let mut delivered = 0_usize;
    let mut first_error = None;

    std::thread::scope(|scope| {
        let mut receivers = Vec::with_capacity(workers);
        for (worker, mut preparer) in preparers.into_iter().enumerate().take(frames.len()) {
            // A rendezvous retains at most one prepared/in-progress frame per CPU worker.
            // GPU contexts and encoder surfaces stay on the ordered execution thread.
            let (sender, receiver) = crossbeam_channel::bounded(0);
            receivers.push(receiver);
            let worker_abort = Arc::clone(&abort);
            scope.spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> Result<(), PipelineError> {
                        for sequence in (worker..frames.len()).step_by(workers) {
                            if worker_abort.load(Ordering::Acquire) {
                                break;
                            }
                            control.check()?;
                            let key = frames[sequence];
                            let prepared = control.compute(|| {
                                preparer.prepare_frame_for_sequence(sequence, key, spec)
                            })?;
                            let _send_span = control.trace.as_ref().map(|t| {
                                t.span(
                                    "prepare.backpressure",
                                    serde_json::json!({"frame": key.index(), "sequence": sequence}),
                                )
                            });
                            if sender.send(Ok((sequence, key, prepared))).is_err() {
                                break;
                            }
                        }
                        Ok(())
                    },
                ))
                .unwrap_or(Err(PipelineError::StagePanicked));
                if let Err(error) = result {
                    if let Some(admission) = &control.admission {
                        admission.stop();
                    }
                    worker_abort.store(true, Ordering::Release);
                    let _ = sender.send(Err(error));
                }
            });
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), PipelineError> {
            for sequence in 0..frames.len() {
                control.check()?;
                let wait_span = control.trace.as_ref().map(|t| t.span("prepare.wait", serde_json::json!({"frame": frames[sequence].index(), "sequence": sequence})));
                let started = Instant::now();
                let (actual, key, prepared) = loop {
                    control.check()?;
                    if budget_pressure(control) && pending.is_empty() {
                        for executor in &mut executors { executor.trim_memory(); }
                    }
                    match receivers[sequence % workers].recv_timeout(std::time::Duration::from_millis(2)) {
                        Ok(packet) => break packet?,
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return Err(PipelineError::IncompleteEvidence),
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) if !pending.is_empty() => {
                            deliver_shared_head(&mut executors, &mut pending, sink, &mut metrics,
                                &mut delivered, frames.len(), progress)?;
                        },
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {},
                    }
                };
                drop(wait_span);
                let prepare_wait_us =
                    started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
                if actual != sequence {
                    return Err(PipelineError::OutOfOrder {
                        expected: sequence,
                        actual,
                    });
                }
                control.check()?;
                if !ordered_shadows && prepared.needs_ordered_shadow_context() {
                    // Drain with the owning contexts before reducing the queue
                    // count. This also preserves ordered delivery on cancellation
                    // or a sink error during the transition.
                    while !pending.is_empty() {
                        control.check()?;
                        deliver_shared_head(
                            &mut executors,
                            &mut pending,
                            sink,
                            &mut metrics,
                            &mut delivered,
                            frames.len(),
                            progress,
                        )?;
                    }
                    executors.truncate(1);
                    active_queues = 1;
                    ordered_shadows = true;
                    queues.restrict_to(1);
                    if std::env::var_os("VALLE_PERF").is_some() {
                        eprintln!(
                            "[valle metal] shared-queues=1 ordered-shadow-cache frame={sequence}"
                        );
                    }
                }
                let owner = sequence % active_queues;
                let submit_started = Instant::now();
                let submitted =
                    executors[owner].render_prepared_shared_frame(prepared, spec, &pool)?;
                let submit_us = submit_started
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64;
                pending.push_back(PendingSharedFrame {
                    executor: owner,
                    sequence,
                    key,
                    submitted,
                });
                if let Some(trace) = &control.trace {
                    trace.evidence("metal.queue", serde_json::json!({"frame": key.index(), "sequence": sequence,
                        "pending": pending.len(), "limit": MAXIMUM_PENDING_FRAMES, "activeContexts": active_queues, "retainedContexts": executors.len()}));
                    if sequence % 24 == 0 {
                        if let Some(memory) = metal_memory() {
                            trace.evidence("metal.memory", serde_json::json!({"frame": key.index(), "allocatedBytes": memory.allocated,
                                "optionalConcurrencyBudgetBytes": memory.budget, "scope": "Metal device allocations attributed to this process"}));
                        }
                    }
                }
                if pending.len() == MAXIMUM_PENDING_FRAMES {
                    let mut sample = deliver_shared_head(
                        &mut executors,
                        &mut pending,
                        sink,
                        &mut metrics,
                        &mut delivered,
                        frames.len(),
                        progress,
                    )?;
                    sample.wall_us = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
                    sample.prepare_wait_us = prepare_wait_us;
                    sample.submit_us = submit_us;
                    sample.shared_output_bytes = u64::from(spec.width())
                        .saturating_mul(u64::from(spec.height()))
                        .saturating_mul(4 * MAXIMUM_PENDING_FRAMES as u64);
                    if let Some(change) =
                        queues.observe(sample, frames.len() - sequence - 1, metal_memory)
                    {
                        // Explicit owners remain valid throughout draining and error cleanup.
                        // Reconfiguration happens only after every previous GPU user completes.
                        while !pending.is_empty() {
                            control.check()?;
                            deliver_shared_head(
                                &mut executors,
                                &mut pending,
                                sink,
                                &mut metrics,
                                &mut delivered,
                                frames.len(),
                                progress,
                            )?;
                        }
                        while executors.len() < change.retained_queues {
                            match control.execution_stage(project, backend, spec) {
                                Ok(executor) => executors.push(executor),
                                Err(error) => {
                                    // Optional concurrency must not make a render fail when its
                                    // already initialized contexts can complete the same work.
                                    queues.restrict_to(executors.len());
                                    if std::env::var_os("VALLE_PERF").is_some() {
                                        eprintln!("[valle metal] queue-probe unavailable: {error}");
                                    }
                                    break;
                                }
                            }
                        }
                        executors.truncate(change.retained_queues);
                        active_queues = change.queues.min(executors.len());
                        if let Some(trace) = &control.trace {
                            trace.evidence("metal.reconfigure", serde_json::json!({"frame": sequence + 1,
                                "activeContexts": active_queues, "retainedContexts": executors.len(), "reason": change.reason}));
                        }
                        if std::env::var_os("VALLE_PERF").is_some() {
                            eprintln!(
                                "[valle metal] shared-queues={} retained-queues={} reason={} frame={}",
                                active_queues,
                                executors.len(),
                                change.reason,
                                sequence + 1
                            );
                        }
                    }
                }
            }
            while !pending.is_empty() {
                control.check()?;
                deliver_shared_head(
                    &mut executors,
                    &mut pending,
                    sink,
                    &mut metrics,
                    &mut delivered,
                    frames.len(),
                    progress,
                )?;
            }
            Ok(())
        })).unwrap_or(Err(PipelineError::StagePanicked));
        if let Err(error) = result {
            first_error = Some(error);
            if let Some(admission) = &control.admission {
                admission.stop();
            }
            abort.store(true, Ordering::Release);
            drain_workers(&receivers, &mut first_error);
        }
    });

    if let Some(error) = first_error {
        // A cancelled/failed sink must not release encoder or decoder surfaces while GPU
        // commands still reference them. Complete outstanding work without delivering it.
        while let Some(frame) = pending.pop_front() {
            let _ = executors[frame.executor].finish_shared_frame(frame.submitted);
        }
        return Err(error);
    }
    control.check()?;
    sink.finish()
        .map_err(|error| PipelineError::Sink(error.to_string()))?;
    // Workers each own at most one preparing/waiting frame. Execution has at most three
    // additional frames (including the current handoff), independently of video duration.
    metrics.finish(
        delivered,
        workers.min(frames.len()),
        frames.len().min(workers + MAXIMUM_PENDING_FRAMES),
    )
}

#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
fn deliver_shared_head(
    executors: &mut [super::frame::FrameExecutionStage],
    pending: &mut VecDeque<PendingSharedFrame>,
    sink: &mut dyn FrameSink,
    metrics: &mut PipelineAccumulator,
    delivered: &mut usize,
    total: usize,
    progress: Option<&ProgressCallback>,
) -> Result<QueueSample, PipelineError> {
    let PendingSharedFrame {
        executor,
        sequence,
        key,
        submitted,
    } = pending
        .pop_front()
        .ok_or(PipelineError::IncompleteEvidence)?;
    if sequence != *delivered {
        return Err(PipelineError::OutOfOrder {
            expected: *delivered,
            actual: sequence,
        });
    }
    let memory = submitted.memory_credit.clone();
    let (frame, evidence) = executors[executor].finish_shared_frame(submitted)?;
    metrics.add(&evidence)?;
    let sink_started = Instant::now();
    let push = || {
        sink.push_shared(DeliveredSharedFrame {
            sequence,
            key,
            frame,
            evidence: &evidence,
        })
    };
    match memory {
        Some(credit) => credit.run(push),
        None => push(),
    }
    .map_err(|error| PipelineError::Sink(error.to_string()))?;
    let sink_us = sink_started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
    *delivered = delivered
        .checked_add(1)
        .ok_or(PipelineError::CounterOverflow)?;
    if let Some(progress) = progress {
        progress(*delivered, total);
    }
    let (opacity_cache_bytes, reuse_cache_pressure) =
        executors[executor].opacity_layer_cache_usage();
    Ok(QueueSample {
        gpu_wait_us: evidence.timings.gpu_completion_wait_us.unwrap_or(0),
        execute_us: evidence.timings.execute_us,
        // The aggregate pool already includes retained scratch and delivery surfaces.
        context_surface_bytes: evidence
            .execution
            .surfaces
            .pool_bytes
            .saturating_add(evidence.execution.raster_layer_cache_bytes as u64)
            .saturating_add(opacity_cache_bytes as u64),
        reuse_cache_pressure,
        sink_us,
        ..Default::default()
    })
}

struct CompletedFrame {
    memory: Option<valle_engine::motion::memory::MemoryCredit>,
    delivery_memory: Option<valle_engine::motion::memory::MemoryLease>,
    sequence: usize,
    key: FrameKey,
    pixels: CompletedPixels,
    evidence: super::FrameEvidence,
}

enum CompletedPixels {
    Rgba(valle_media::frame::RgbaFrame),
    /// An encoder-owned surface that Raster wrote in place.
    Shared(SharedVideoFrame),
}

#[derive(Default)]
struct PipelineAccumulator {
    render_id: Option<RenderId>,
    profile: Option<SkiaExecutionProfile>,
    delivery: Option<FrameDelivery>,
    requests: usize,
    passes: usize,
    templates: BTreeSet<ContentDigest>,
    template_cache_hits: usize,
    template_cache_misses: usize,
    resource_cache_hits: u64,
    resource_cache_misses: u64,
    resource_cache_insertions: u64,
    resource_cache_evictions: u64,
    resource_cache_bypasses: u64,
    resource_cache_generation_invalidations: u64,
    maximum_resource_cache_entries: usize,
    maximum_resource_cache_bytes: u64,
    scene3d_requests: u64,
    scene3d_prepared_cache_hits: u64,
    scene3d_prepared_cache_misses: u64,
    scene3d_prepare_us: u64,
    scene3d_raster_us: u64,
    scene3d_upload_us: u64,
    scene3d_composite_us: u64,
    program_cache_hits: u64,
    program_cache_misses: u64,
    raster_layer_cache_hits: u64,
    raster_layer_cache_misses: u64,
    maximum_raster_layer_cache_entries: usize,
    maximum_raster_layer_cache_bytes: usize,
    maximum_program_cache_entries: usize,
    maximum_program_cache_cost_bytes: usize,
    font_cache_hits: u64,
    font_cache_misses: u64,
    shader_cache_hits: u64,
    shader_cache_misses: u64,
    built_in_kernel_cache_hits: u64,
    built_in_kernel_cache_misses: u64,
    external_gpu_imports: u64,
    cpu_upload_bytes: u64,
    surface_generation_invalidations: u64,
    plan_surface_backend_allocations: u64,
    plan_surface_backend_reuses: u64,
    total_surface_allocations: u64,
    unpooled_surface_allocations: u64,
    scratch_surface_allocations: u64,
    scratch_surface_reuses: u64,
    maximum_scratch_surfaces: usize,
    maximum_scratch_bytes: u64,
    maximum_scratch_pool_surfaces: usize,
    maximum_scratch_pool_bytes: u64,
    maximum_plan_surface_slots: usize,
    maximum_plan_logical_allocations: usize,
    maximum_plan_physical_bytes: u64,
    maximum_plan_logical_bytes: u64,
    maximum_plan_alias_saved_bytes: u64,
    maximum_plan_estimated_peak_bytes: u64,
    maximum_surface_pool_surfaces: usize,
    maximum_surface_pool_bytes: u64,
    surface_pool_evictions: u64,
    evaluate_prepare_us: u64,
    fulfillment_work_us: u64,
    lower_work_us: u64,
    fulfill_lower_wall_us: u64,
    bind_us: u64,
    execute_us: u64,
    delivery_readback_us: u64,
    delivery_readback_bytes: u64,
    gpu_completion_wait_us: u64,
    maximum_frame_us: u64,
}

impl PipelineAccumulator {
    fn for_render_id(render_id: RenderId) -> Self {
        Self {
            render_id: Some(render_id),
            ..Self::default()
        }
    }

    fn add(&mut self, evidence: &super::FrameEvidence) -> Result<(), PipelineError> {
        // Opt-in per-frame diagnostics, outside all render identities and scheduling decisions.
        if std::env::var_os("VALLE_PERF_FRAMES").is_some() {
            let t = evidence.timings;
            eprintln!(
                "[valle frame] {}",
                serde_json::json!({
                    "frame": evidence.frame, "profile": format!("{:?}", evidence.profile),
                    "evaluatePrepareUs": t.evaluate_prepare_us, "fulfillLowerWallUs": t.fulfill_lower_wall_us,
                    "bindUs": t.bind_us, "executeUs": t.execute_us, "readbackUs": t.delivery_readback_us,
                    "totalUs": t.total_us,
                    "scene3dPrepareUs": evidence.scene3d.prepare_us,
                    "scene3dRasterUs": evidence.scene3d.raster_us,
                    "scene3dUploadUs": evidence.scene3d.upload_us,
                })
            );
        }
        match self.render_id {
            Some(render_id) if render_id != evidence.render_id => {
                return Err(PipelineError::MixedRenderIds);
            }
            None => self.render_id = Some(evidence.render_id),
            Some(_) => {}
        }
        match self.profile {
            Some(profile) if profile != evidence.profile => {
                return Err(PipelineError::MixedExecutionProfiles);
            }
            None => self.profile = Some(evidence.profile),
            Some(_) => {}
        }
        match self.delivery {
            Some(delivery) if delivery != evidence.delivery => {
                return Err(PipelineError::MixedDeliveryModes);
            }
            None => self.delivery = Some(evidence.delivery),
            Some(_) => {}
        }
        self.requests = add_usize(self.requests, evidence.resource_requests)?;
        self.passes = add_usize(self.passes, evidence.execution_passes)?;
        self.templates.insert(evidence.template_hash.clone());
        if evidence.template_cache_hit {
            self.template_cache_hits = add_usize(self.template_cache_hits, 1)?;
        } else {
            self.template_cache_misses = add_usize(self.template_cache_misses, 1)?;
        }
        let resources = evidence.resource_cache;
        self.resource_cache_hits = add_u64(self.resource_cache_hits, resources.hits)?;
        self.resource_cache_misses = add_u64(self.resource_cache_misses, resources.misses)?;
        self.resource_cache_insertions =
            add_u64(self.resource_cache_insertions, resources.insertions)?;
        self.resource_cache_evictions =
            add_u64(self.resource_cache_evictions, resources.evictions)?;
        self.resource_cache_bypasses = add_u64(self.resource_cache_bypasses, resources.bypasses)?;
        self.resource_cache_generation_invalidations = add_u64(
            self.resource_cache_generation_invalidations,
            resources.generation_invalidations,
        )?;
        self.maximum_resource_cache_entries = self
            .maximum_resource_cache_entries
            .max(resources.resident_entries);
        self.maximum_resource_cache_bytes = self
            .maximum_resource_cache_bytes
            .max(resources.resident_bytes);
        let scene3d = evidence.scene3d;
        self.scene3d_requests = add_u64(self.scene3d_requests, scene3d.requests)?;
        self.scene3d_prepared_cache_hits = add_u64(
            self.scene3d_prepared_cache_hits,
            scene3d.prepared_cache_hits,
        )?;
        self.scene3d_prepared_cache_misses = add_u64(
            self.scene3d_prepared_cache_misses,
            scene3d.prepared_cache_misses,
        )?;
        self.scene3d_prepare_us = add_u64(self.scene3d_prepare_us, scene3d.prepare_us)?;
        self.scene3d_raster_us = add_u64(self.scene3d_raster_us, scene3d.raster_us)?;
        self.scene3d_upload_us = add_u64(self.scene3d_upload_us, scene3d.upload_us)?;
        self.scene3d_composite_us = add_u64(self.scene3d_composite_us, scene3d.composite_us)?;
        let execution = evidence.execution;
        self.maximum_program_cache_entries = self
            .maximum_program_cache_entries
            .max(execution.program_cache_entries);
        self.maximum_program_cache_cost_bytes = self
            .maximum_program_cache_cost_bytes
            .max(execution.program_cache_cost_bytes);
        self.program_cache_hits = add_u64(self.program_cache_hits, execution.program_cache_hits)?;
        self.program_cache_misses =
            add_u64(self.program_cache_misses, execution.program_cache_misses)?;
        self.raster_layer_cache_hits = add_u64(
            self.raster_layer_cache_hits,
            execution.raster_layer_cache_hits,
        )?;
        self.raster_layer_cache_misses = add_u64(
            self.raster_layer_cache_misses,
            execution.raster_layer_cache_misses,
        )?;
        self.maximum_raster_layer_cache_entries = self
            .maximum_raster_layer_cache_entries
            .max(execution.raster_layer_cache_entries);
        self.maximum_raster_layer_cache_bytes = self
            .maximum_raster_layer_cache_bytes
            .max(execution.raster_layer_cache_bytes);
        self.font_cache_hits = add_u64(self.font_cache_hits, execution.font_cache_hits)?;
        self.font_cache_misses = add_u64(self.font_cache_misses, execution.font_cache_misses)?;
        self.shader_cache_hits = add_u64(self.shader_cache_hits, execution.shader_cache_hits)?;
        self.shader_cache_misses =
            add_u64(self.shader_cache_misses, execution.shader_cache_misses)?;
        self.built_in_kernel_cache_hits = add_u64(
            self.built_in_kernel_cache_hits,
            execution.built_in_kernel_cache_hits,
        )?;
        self.built_in_kernel_cache_misses = add_u64(
            self.built_in_kernel_cache_misses,
            execution.built_in_kernel_cache_misses,
        )?;
        self.external_gpu_imports =
            add_u64(self.external_gpu_imports, execution.external_gpu_imports)?;
        self.cpu_upload_bytes = add_u64(self.cpu_upload_bytes, execution.cpu_upload_bytes)?;
        let surfaces = execution.surfaces;
        self.surface_generation_invalidations = add_u64(
            self.surface_generation_invalidations,
            surfaces.generation_invalidations,
        )?;
        self.plan_surface_backend_allocations = add_u64(
            self.plan_surface_backend_allocations,
            surfaces.backend_allocations,
        )?;
        self.plan_surface_backend_reuses =
            add_u64(self.plan_surface_backend_reuses, surfaces.backend_reuses)?;
        self.total_surface_allocations = add_u64(
            self.total_surface_allocations,
            surfaces.total_surface_allocations,
        )?;
        self.unpooled_surface_allocations = add_u64(
            self.unpooled_surface_allocations,
            surfaces.unpooled_surface_allocations,
        )?;
        self.scratch_surface_allocations = add_u64(
            self.scratch_surface_allocations,
            surfaces.scratch_allocations,
        )?;
        self.scratch_surface_reuses =
            add_u64(self.scratch_surface_reuses, surfaces.scratch_reuses)?;
        self.maximum_scratch_surfaces = self
            .maximum_scratch_surfaces
            .max(surfaces.scratch_peak_surfaces);
        self.maximum_scratch_bytes = self.maximum_scratch_bytes.max(surfaces.scratch_peak_bytes);
        self.maximum_scratch_pool_surfaces = self
            .maximum_scratch_pool_surfaces
            .max(surfaces.scratch_pool_surfaces);
        self.maximum_scratch_pool_bytes = self
            .maximum_scratch_pool_bytes
            .max(surfaces.scratch_pool_bytes);
        self.maximum_plan_surface_slots = self.maximum_plan_surface_slots.max(surfaces.plan_slots);
        self.maximum_plan_logical_allocations = self
            .maximum_plan_logical_allocations
            .max(surfaces.logical_allocations);
        self.maximum_plan_physical_bytes = self
            .maximum_plan_physical_bytes
            .max(surfaces.physical_bytes);
        self.maximum_plan_logical_bytes =
            self.maximum_plan_logical_bytes.max(surfaces.logical_bytes);
        self.maximum_plan_alias_saved_bytes = self
            .maximum_plan_alias_saved_bytes
            .max(surfaces.alias_saved_bytes);
        self.maximum_plan_estimated_peak_bytes = self
            .maximum_plan_estimated_peak_bytes
            .max(surfaces.estimated_peak_bytes);
        self.maximum_surface_pool_surfaces = self
            .maximum_surface_pool_surfaces
            .max(surfaces.pool_surfaces);
        self.maximum_surface_pool_bytes = self.maximum_surface_pool_bytes.max(surfaces.pool_bytes);
        self.surface_pool_evictions =
            add_u64(self.surface_pool_evictions, surfaces.pool_evictions)?;
        let timings = evidence.timings;
        self.evaluate_prepare_us = add_u64(self.evaluate_prepare_us, timings.evaluate_prepare_us)?;
        self.fulfillment_work_us = add_u64(self.fulfillment_work_us, timings.fulfillment_us)?;
        self.lower_work_us = add_u64(self.lower_work_us, timings.lower_us)?;
        self.fulfill_lower_wall_us =
            add_u64(self.fulfill_lower_wall_us, timings.fulfill_lower_wall_us)?;
        self.bind_us = add_u64(self.bind_us, timings.bind_us)?;
        self.execute_us = add_u64(self.execute_us, timings.execute_us)?;
        match evidence.delivery {
            FrameDelivery::CpuReadback => {
                self.delivery_readback_us = add_u64(
                    self.delivery_readback_us,
                    timings
                        .delivery_readback_us
                        .ok_or(PipelineError::MissingDeliveryEvidence)?,
                )?;
                if timings.gpu_completion_wait_us.is_some() {
                    return Err(PipelineError::InvalidDeliveryEvidence);
                }
            }
            FrameDelivery::SharedGpu => {
                if timings.delivery_readback_us.is_some() || evidence.delivery_readback_bytes != 0 {
                    return Err(PipelineError::InvalidDeliveryEvidence);
                }
                self.gpu_completion_wait_us = add_u64(
                    self.gpu_completion_wait_us,
                    timings
                        .gpu_completion_wait_us
                        .ok_or(PipelineError::MissingDeliveryEvidence)?,
                )?;
            }
            FrameDelivery::SharedCpu => {
                if timings.delivery_readback_us.is_some()
                    || evidence.delivery_readback_bytes != 0
                    || timings.gpu_completion_wait_us.is_some()
                {
                    return Err(PipelineError::InvalidDeliveryEvidence);
                }
            }
            FrameDelivery::Surface => return Err(PipelineError::InvalidDeliveryEvidence),
        }
        self.delivery_readback_bytes = add_u64(
            self.delivery_readback_bytes,
            evidence.delivery_readback_bytes,
        )?;
        self.maximum_frame_us = self.maximum_frame_us.max(timings.total_us);
        Ok(())
    }

    fn finish(
        self,
        frames: usize,
        preparation_tasks: usize,
        maximum_in_flight_frames: usize,
    ) -> Result<PipelineReport, PipelineError> {
        if add_usize(self.template_cache_hits, self.template_cache_misses)? != frames {
            return Err(PipelineError::IncompleteEvidence);
        }
        if std::env::var_os("VALLE_PERF").is_some() {
            eprintln!(
                "[valle pipeline] frames={frames} preparation-tasks={preparation_tasks} evaluate-prepare={:.3}ms fulfill-lower-wall={:.3}ms bind={:.3}ms execute={:.3}ms readback={:.3}ms gpu-wait={:.3}ms max-frame={:.3}ms template-hits={} template-misses={} scratch-allocations={} scratch-reuses={} max-scratch-bytes={} max-program-cache-entries={} max-program-cache-cost-bytes={} raster-layer-hits={} raster-layer-misses={} max-raster-layer-bytes={}",
                self.evaluate_prepare_us as f64 / 1000.0,
                self.fulfill_lower_wall_us as f64 / 1000.0,
                self.bind_us as f64 / 1000.0,
                self.execute_us as f64 / 1000.0,
                self.delivery_readback_us as f64 / 1000.0,
                self.gpu_completion_wait_us as f64 / 1000.0,
                self.maximum_frame_us as f64 / 1000.0,
                self.template_cache_hits,
                self.template_cache_misses,
                self.scratch_surface_allocations,
                self.scratch_surface_reuses,
                self.maximum_scratch_bytes,
                self.maximum_program_cache_entries,
                self.maximum_program_cache_cost_bytes,
                self.raster_layer_cache_hits,
                self.raster_layer_cache_misses,
                self.maximum_raster_layer_cache_bytes,
            );
        }
        Ok(PipelineReport {
            render_id: self.render_id.ok_or(PipelineError::IncompleteEvidence)?,
            frames,
            profile: self.profile,
            delivery: self.delivery,
            resource_requests: self.requests,
            execution_passes: self.passes,
            unique_templates: self.templates.len(),
            preparation_tasks,
            maximum_in_flight_frames,
            budget: None,
            template_cache_hits: self.template_cache_hits,
            template_cache_misses: self.template_cache_misses,
            resource_cache_hits: self.resource_cache_hits,
            resource_cache_misses: self.resource_cache_misses,
            resource_cache_insertions: self.resource_cache_insertions,
            resource_cache_evictions: self.resource_cache_evictions,
            resource_cache_bypasses: self.resource_cache_bypasses,
            resource_cache_generation_invalidations: self.resource_cache_generation_invalidations,
            maximum_resource_cache_entries: self.maximum_resource_cache_entries,
            maximum_resource_cache_bytes: self.maximum_resource_cache_bytes,
            scene3d_requests: self.scene3d_requests,
            scene3d_prepared_cache_hits: self.scene3d_prepared_cache_hits,
            scene3d_prepared_cache_misses: self.scene3d_prepared_cache_misses,
            scene3d_prepare_us: self.scene3d_prepare_us,
            scene3d_raster_us: self.scene3d_raster_us,
            scene3d_upload_us: self.scene3d_upload_us,
            scene3d_composite_us: self.scene3d_composite_us,
            program_cache_hits: self.program_cache_hits,
            program_cache_misses: self.program_cache_misses,
            raster_layer_cache_hits: self.raster_layer_cache_hits,
            raster_layer_cache_misses: self.raster_layer_cache_misses,
            maximum_raster_layer_cache_entries: self.maximum_raster_layer_cache_entries,
            maximum_raster_layer_cache_bytes: self.maximum_raster_layer_cache_bytes,
            maximum_program_cache_entries: self.maximum_program_cache_entries,
            maximum_program_cache_cost_bytes: self.maximum_program_cache_cost_bytes,
            font_cache_hits: self.font_cache_hits,
            font_cache_misses: self.font_cache_misses,
            shader_cache_hits: self.shader_cache_hits,
            shader_cache_misses: self.shader_cache_misses,
            built_in_kernel_cache_hits: self.built_in_kernel_cache_hits,
            built_in_kernel_cache_misses: self.built_in_kernel_cache_misses,
            external_gpu_imports: self.external_gpu_imports,
            cpu_upload_bytes: self.cpu_upload_bytes,
            surface_generation_invalidations: self.surface_generation_invalidations,
            plan_surface_backend_allocations: self.plan_surface_backend_allocations,
            plan_surface_backend_reuses: self.plan_surface_backend_reuses,
            total_surface_allocations: self.total_surface_allocations,
            unpooled_surface_allocations: self.unpooled_surface_allocations,
            scratch_surface_allocations: self.scratch_surface_allocations,
            scratch_surface_reuses: self.scratch_surface_reuses,
            maximum_scratch_surfaces: self.maximum_scratch_surfaces,
            maximum_scratch_bytes: self.maximum_scratch_bytes,
            maximum_scratch_pool_surfaces: self.maximum_scratch_pool_surfaces,
            maximum_scratch_pool_bytes: self.maximum_scratch_pool_bytes,
            maximum_plan_surface_slots: self.maximum_plan_surface_slots,
            maximum_plan_logical_allocations: self.maximum_plan_logical_allocations,
            maximum_plan_physical_bytes: self.maximum_plan_physical_bytes,
            maximum_plan_logical_bytes: self.maximum_plan_logical_bytes,
            maximum_plan_alias_saved_bytes: self.maximum_plan_alias_saved_bytes,
            maximum_plan_estimated_peak_bytes: self.maximum_plan_estimated_peak_bytes,
            maximum_surface_pool_surfaces: self.maximum_surface_pool_surfaces,
            maximum_surface_pool_bytes: self.maximum_surface_pool_bytes,
            surface_pool_evictions: self.surface_pool_evictions,
            evaluate_prepare_us: self.evaluate_prepare_us,
            fulfillment_work_us: self.fulfillment_work_us,
            lower_work_us: self.lower_work_us,
            fulfill_lower_wall_us: self.fulfill_lower_wall_us,
            bind_us: self.bind_us,
            execute_us: self.execute_us,
            delivery_readback_us: self.delivery_readback_us,
            delivery_readback_bytes: self.delivery_readback_bytes,
            gpu_completion_wait_us: self.gpu_completion_wait_us,
            maximum_frame_us: self.maximum_frame_us,
        })
    }
}

fn add_usize(left: usize, right: usize) -> Result<usize, PipelineError> {
    left.checked_add(right)
        .ok_or(PipelineError::CounterOverflow)
}

fn add_u64(left: u64, right: u64) -> Result<u64, PipelineError> {
    left.checked_add(right)
        .ok_or(PipelineError::CounterOverflow)
}

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("render stage panicked")]
    StagePanicked,
    #[error(transparent)]
    Budget(#[from] super::NativeBudgetError),
    #[error(transparent)]
    Frame(#[from] FrameRenderError),
    #[error("render cancelled")]
    Cancelled,
    #[error("render deadline exceeded")]
    Deadline,
    #[error("delivery sink failed: {0}")]
    Sink(String),
    #[error("render metrics counter overflow")]
    CounterOverflow,
    #[error("CPU frame delivery did not report its readback stage")]
    MissingDeliveryEvidence,
    #[error("frame delivery evidence is inconsistent with its delivery mode")]
    InvalidDeliveryEvidence,
    #[error("one render schedule used more than one physical execution profile")]
    MixedExecutionProfiles,
    #[error("one render schedule mixed more than one RenderId")]
    MixedRenderIds,
    #[error("one render schedule used more than one frame delivery mode")]
    MixedDeliveryModes,
    #[error("shared GPU delivery is unavailable on this platform")]
    SharedGpuUnavailable,
    #[error("pipeline evidence does not cover every delivered frame")]
    IncompleteEvidence,
    #[error("raster workers produced frame {actual} while ordered delivery expected {expected}")]
    OutOfOrder { expected: usize, actual: usize },
}

/// Drain all producers before joining, retaining the real error if an aborted peer closed first.
fn drain_workers<T>(
    receivers: &[crossbeam_channel::Receiver<Result<T, PipelineError>>],
    first_error: &mut Option<PipelineError>,
) {
    for receiver in receivers {
        while let Ok(message) = receiver.recv() {
            if let Err(error) = message
                && matches!(first_error, Some(PipelineError::IncompleteEvidence))
            {
                *first_error = Some(error);
            }
        }
    }
}

#[cfg(test)]
mod worker_error_tests {
    use super::*;

    #[test]
    fn aborted_peer_closing_first_does_not_hide_producer_error() {
        let (peer, peer_rx) = crossbeam_channel::bounded::<Result<(), PipelineError>>(1);
        let (producer, producer_rx) = crossbeam_channel::bounded(1);
        drop(peer); // Earlier in frame order, this worker sees abort and exits.
        producer.send(Err(PipelineError::Deadline)).unwrap();
        drop(producer);
        assert!(peer_rx.recv().is_err());
        let mut error = Some(PipelineError::IncompleteEvidence);
        drain_workers(&[peer_rx, producer_rx], &mut error);
        assert!(matches!(error, Some(PipelineError::Deadline)));
    }

    #[test]
    fn draining_blocked_producers_preserves_the_original_sink_error() {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        std::thread::scope(|scope| {
            scope.spawn(move || {
                sender.send(Ok(())).unwrap();
                sender.send(Err(PipelineError::Cancelled)).unwrap();
            });
            let mut error = Some(PipelineError::Sink("disk full".into()));
            drain_workers(&[receiver], &mut error);
            assert!(matches!(error, Some(PipelineError::Sink(ref text)) if text == "disk full"));
        });
    }
}

struct TraceSink<'a> {
    sink: &'a mut dyn FrameSink,
    trace: Option<&'a super::RenderTrace>,
    cpu: &'a valle_engine::motion::cpu::CpuBudget,
}
impl FrameSink for TraceSink<'_> {
    fn working_bytes(&self, sequence: usize) -> anyhow::Result<u64> {
        self.sink.working_bytes(sequence)
    }

    fn push(&mut self, frame: DeliveredFrame<'_>) -> anyhow::Result<()> {
        let _span = self.trace.map(|t| t.span("encode.push", serde_json::json!({"frame": frame.key.index(), "sequence": frame.sequence, "readbackBytes": frame.evidence.delivery_readback_bytes})));
        self.cpu.run(|| self.sink.push(frame))
    }
    fn shared_frame_pool(&self) -> Option<valle_media::SharedVideoFramePool> {
        self.sink.shared_frame_pool()
    }
    fn push_shared(&mut self, frame: super::DeliveredSharedFrame<'_>) -> anyhow::Result<()> {
        let _span = self.trace.map(|t| t.span("encode.push", serde_json::json!({"frame": frame.key.index(), "sequence": frame.sequence, "readbackBytes": frame.evidence.delivery_readback_bytes})));
        self.cpu.run(|| self.sink.push_shared(frame))
    }
    fn finish(&mut self) -> anyhow::Result<()> {
        let _span = self
            .trace
            .map(|t| t.span("encode.drain_mux", serde_json::json!({})));
        self.cpu.run(|| self.sink.finish())
    }
}
