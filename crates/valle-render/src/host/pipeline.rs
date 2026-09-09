//! One Native frame loop for every delivery mode.

use std::{
    collections::{BTreeSet, VecDeque},
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
#[cfg(target_os = "macos")]
use valle_media::SharedVideoFramePool;

use super::{
    DeliveredFrame, DeliveredSharedFrame, FrameDelivery, FrameRenderError, FrameSink, NativeProject,
};
use crate::executor::skia::{SkiaBackendKind, SkiaExecutionProfile};

#[derive(Clone, Default)]
pub struct RenderControl {
    cancelled: Arc<AtomicBool>,
    deadline: Option<Instant>,
}

impl RenderControl {
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
    pub raster_workers: usize,
    pub maximum_in_flight_frames: usize,
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

/// Executes a sorted frame schedule through the same staged Product Compositor runner.
pub fn render_schedule(
    project: &NativeProject,
    frames: &[FrameKey],
    spec: RenderSpec,
    sink: &mut dyn FrameSink,
    control: &RenderControl,
    progress: Option<&ProgressCallback>,
    backend: SkiaBackendKind,
    requested_workers: Option<usize>,
) -> Result<PipelineReport, PipelineError> {
    if let Some(pool) = sink.shared_frame_pool() {
        #[cfg(target_os = "macos")]
        {
            return render_shared_serial(
                project, frames, spec, sink, control, progress, backend, pool,
            );
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = pool;
            return Err(PipelineError::SharedGpuUnavailable);
        }
    }
    let workers = raster_worker_count(
        frames.len(),
        backend_default_workers(backend, requested_workers),
    )?;
    if workers == 1 && frames.len() <= 1 {
        return render_serial(project, frames, spec, sink, control, progress, backend);
    }
    if workers == 1 {
        return render_staged_serial(project, frames, spec, sink, control, progress, backend);
    }
    render_parallel(
        project, frames, spec, sink, control, progress, backend, workers,
    )
}

#[cfg(target_os = "macos")]
fn render_shared_serial(
    project: &NativeProject,
    frames: &[FrameKey],
    spec: RenderSpec,
    sink: &mut dyn FrameSink,
    control: &RenderControl,
    progress: Option<&ProgressCallback>,
    backend: SkiaBackendKind,
    pool: SharedVideoFramePool,
) -> Result<PipelineReport, PipelineError> {
    const MAXIMUM_PENDING_FRAMES: usize = 3;

    let mut runner = project.frame_runner(backend)?;
    let mut metrics = PipelineAccumulator::for_render_id(project.render_id());
    let mut pending = VecDeque::with_capacity(MAXIMUM_PENDING_FRAMES);
    let mut delivered = 0_usize;
    let mut maximum_pending = 0_usize;

    for (sequence, key) in frames.iter().copied().enumerate() {
        control.check()?;
        let submitted = runner.render_shared_frame(key, spec, &pool)?;
        pending.push_back((sequence, key, submitted));
        maximum_pending = maximum_pending.max(pending.len());
        if pending.len() == MAXIMUM_PENDING_FRAMES {
            deliver_shared_head(
                &mut runner,
                &mut pending,
                sink,
                &mut metrics,
                &mut delivered,
                frames.len(),
                progress,
            )?;
        }
    }
    while !pending.is_empty() {
        control.check()?;
        deliver_shared_head(
            &mut runner,
            &mut pending,
            sink,
            &mut metrics,
            &mut delivered,
            frames.len(),
            progress,
        )?;
    }
    control.check()?;
    sink.finish()
        .map_err(|error| PipelineError::Sink(error.to_string()))?;
    metrics.finish(delivered, usize::from(!frames.is_empty()), maximum_pending)
}

#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
fn deliver_shared_head(
    runner: &mut super::FrameRunner<super::NativeResourceProvider>,
    pending: &mut VecDeque<(usize, FrameKey, super::SubmittedSharedFrame)>,
    sink: &mut dyn FrameSink,
    metrics: &mut PipelineAccumulator,
    delivered: &mut usize,
    total: usize,
    progress: Option<&ProgressCallback>,
) -> Result<(), PipelineError> {
    let (sequence, key, submitted) = pending
        .pop_front()
        .ok_or(PipelineError::IncompleteEvidence)?;
    if sequence != *delivered {
        return Err(PipelineError::OutOfOrder {
            expected: *delivered,
            actual: sequence,
        });
    }
    let (frame, evidence) = runner.finish_shared_frame(submitted)?;
    metrics.add(&evidence)?;
    sink.push_shared(DeliveredSharedFrame {
        sequence,
        key,
        frame,
        evidence: &evidence,
    })
    .map_err(|error| PipelineError::Sink(error.to_string()))?;
    *delivered = delivered
        .checked_add(1)
        .ok_or(PipelineError::CounterOverflow)?;
    if let Some(progress) = progress {
        progress(*delivered, total);
    }
    Ok(())
}

fn render_serial(
    project: &NativeProject,
    frames: &[FrameKey],
    spec: RenderSpec,
    sink: &mut dyn FrameSink,
    control: &RenderControl,
    progress: Option<&ProgressCallback>,
    backend: SkiaBackendKind,
) -> Result<PipelineReport, PipelineError> {
    let mut runner = project.frame_runner(backend)?;
    let mut metrics = PipelineAccumulator::for_render_id(project.render_id());
    for (sequence, key) in frames.iter().copied().enumerate() {
        control.check()?;
        let (pixels, evidence) = runner.render_rgba8(key, spec)?;
        metrics.add(&evidence)?;
        sink.push(DeliveredFrame {
            sequence,
            key,
            pixels: &pixels,
            evidence: &evidence,
        })
        .map_err(|error| PipelineError::Sink(error.to_string()))?;
        if let Some(progress) = progress {
            progress(sequence + 1, frames.len());
        }
    }
    control.check()?;
    sink.finish()
        .map_err(|error| PipelineError::Sink(error.to_string()))?;
    metrics.finish(
        frames.len(),
        usize::from(!frames.is_empty()),
        usize::from(!frames.is_empty()),
    )
}

/// One ordered CPU preparer stays one frame ahead of one persistent physical executor. This is
/// the Metal CPU-delivery default, but it also gives explicitly single-worker Raster schedules the
/// same bounded stage overlap. The channel contains at most one `PreparedFrame`; no completed RGBA
/// frame waits outside the executor/sink, and template/resource caches remain single-owner and
/// strictly ordered.
fn render_staged_serial(
    project: &NativeProject,
    frames: &[FrameKey],
    spec: RenderSpec,
    sink: &mut dyn FrameSink,
    control: &RenderControl,
    progress: Option<&ProgressCallback>,
    backend: SkiaBackendKind,
) -> Result<PipelineReport, PipelineError> {
    let abort = Arc::new(AtomicBool::new(false));
    let runner = project.frame_runner(backend)?;
    let (mut preparer, mut executor) = runner.into_stages();
    let mut metrics = PipelineAccumulator::for_render_id(project.render_id());
    let mut delivered = 0_usize;
    let mut first_error = None;

    std::thread::scope(|scope| {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let control = control.clone();
        let worker_abort = Arc::clone(&abort);
        scope.spawn(move || {
            let result = (|| -> Result<(), PipelineError> {
                for (sequence, key) in frames.iter().copied().enumerate() {
                    if worker_abort.load(Ordering::Acquire) {
                        break;
                    }
                    control.check()?;
                    let prepared = preparer.prepare_frame(key, spec)?;
                    if sender.send(Ok((sequence, key, prepared))).is_err() {
                        break;
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                worker_abort.store(true, Ordering::Release);
                let _ = sender.send(Err(error));
            }
        });

        for sequence in 0..frames.len() {
            let (actual, key, prepared) = match receiver.recv() {
                Ok(Ok(frame)) => frame,
                Ok(Err(error)) => {
                    first_error = Some(error);
                    break;
                }
                Err(_) => {
                    first_error = Some(PipelineError::IncompleteEvidence);
                    break;
                }
            };
            if actual != sequence {
                first_error = Some(PipelineError::OutOfOrder {
                    expected: sequence,
                    actual,
                });
                break;
            }
            let (pixels, evidence) = match executor.render_prepared_rgba8(prepared, spec) {
                Ok(rendered) => rendered,
                Err(error) => {
                    first_error = Some(error.into());
                    break;
                }
            };
            if let Err(error) = metrics.add(&evidence) {
                first_error = Some(error);
                break;
            }
            if let Err(error) = sink.push(DeliveredFrame {
                sequence,
                key,
                pixels: &pixels,
                evidence: &evidence,
            }) {
                first_error = Some(PipelineError::Sink(error.to_string()));
                break;
            }
            delivered += 1;
            if let Some(progress) = progress {
                progress(delivered, frames.len());
            }
        }
        if first_error.is_some() {
            abort.store(true, Ordering::Release);
            while receiver.recv().is_ok() {}
        }
    });

    if let Some(error) = first_error {
        return Err(error);
    }
    control.check()?;
    sink.finish()
        .map_err(|error| PipelineError::Sink(error.to_string()))?;
    metrics.finish(delivered, 1, frames.len().min(2))
}

struct CompletedFrame {
    sequence: usize,
    key: FrameKey,
    pixels: valle_media::frame::RgbaFrame,
    evidence: super::FrameEvidence,
}

fn render_parallel(
    project: &NativeProject,
    frames: &[FrameKey],
    spec: RenderSpec,
    sink: &mut dyn FrameSink,
    control: &RenderControl,
    progress: Option<&ProgressCallback>,
    backend: SkiaBackendKind,
    workers: usize,
) -> Result<PipelineReport, PipelineError> {
    let abort = Arc::new(AtomicBool::new(false));
    let mut metrics = PipelineAccumulator::for_render_id(project.render_id());
    let mut delivered = 0_usize;
    let mut first_error = None;

    std::thread::scope(|scope| {
        let mut receivers = Vec::with_capacity(workers);
        for worker in 0..workers {
            if worker >= frames.len() {
                break;
            }
            // Interleave canonical frame numbers across workers. Each channel holds one completed
            // frame, so all workers remain active while the sink consumes 0,1,2... in exact order;
            // the in-flight memory bound is still one RGBA frame per worker and needs no reorder
            // map whose size could grow with timeline length.
            let (sender, receiver) = crossbeam_channel::bounded(1);
            receivers.push(receiver);
            let project = project.clone();
            let control = control.clone();
            let abort = Arc::clone(&abort);
            scope.spawn(move || {
                let result = (|| -> Result<(), PipelineError> {
                    let mut runner = project.frame_runner(backend)?;
                    for sequence in (worker..frames.len()).step_by(workers) {
                        if abort.load(Ordering::Acquire) {
                            break;
                        }
                        control.check()?;
                        let key = frames[sequence];
                        let (pixels, evidence) = runner.render_rgba8(key, spec)?;
                        if sender
                            .send(Ok(CompletedFrame {
                                sequence,
                                key,
                                pixels,
                                evidence,
                            }))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    abort.store(true, Ordering::Release);
                    let _ = sender.send(Err(error));
                }
            });
        }

        let active_workers = receivers.len();
        for sequence in 0..frames.len() {
            let message = receivers[sequence % active_workers].recv();
            let frame = match message {
                Ok(Ok(frame)) => frame,
                Ok(Err(error)) => {
                    first_error = Some(error);
                    break;
                }
                Err(_) => {
                    first_error = Some(PipelineError::IncompleteEvidence);
                    break;
                }
            };
            if frame.sequence != sequence {
                first_error = Some(PipelineError::OutOfOrder {
                    expected: sequence,
                    actual: frame.sequence,
                });
                break;
            }
            if let Err(error) = metrics.add(&frame.evidence) {
                first_error = Some(error);
                break;
            }
            if let Err(error) = sink.push(DeliveredFrame {
                sequence: frame.sequence,
                key: frame.key,
                pixels: &frame.pixels,
                evidence: &frame.evidence,
            }) {
                first_error = Some(PipelineError::Sink(error.to_string()));
                break;
            }
            delivered += 1;
            if let Some(progress) = progress {
                progress(delivered, frames.len());
            }
        }
        if first_error.is_some() {
            abort.store(true, Ordering::Release);
            // A worker may already be blocked on its one-slot channel. Drain every channel before
            // the scoped threads join so cancellation cannot deadlock the error path.
            drain_workers(&receivers, &mut first_error);
        }
    });

    if let Some(error) = first_error {
        return Err(error);
    }
    control.check()?;
    sink.finish()
        .map_err(|error| PipelineError::Sink(error.to_string()))?;
    metrics.finish(delivered, workers, workers)
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
        raster_workers: usize,
        maximum_in_flight_frames: usize,
    ) -> Result<PipelineReport, PipelineError> {
        if add_usize(self.template_cache_hits, self.template_cache_misses)? != frames {
            return Err(PipelineError::IncompleteEvidence);
        }
        if std::env::var_os("VALLE_PERF").is_some() {
            eprintln!(
                "[valle pipeline] frames={frames} workers={raster_workers} evaluate-prepare={:.3}ms fulfill-lower-wall={:.3}ms bind={:.3}ms execute={:.3}ms readback={:.3}ms gpu-wait={:.3}ms max-frame={:.3}ms template-hits={} template-misses={} scratch-allocations={} scratch-reuses={} max-scratch-bytes={} max-program-cache-entries={} max-program-cache-cost-bytes={}",
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
            raster_workers,
            maximum_in_flight_frames,
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

fn raster_worker_count(
    frame_count: usize,
    requested_workers: Option<usize>,
) -> Result<usize, PipelineError> {
    if frame_count == 0 {
        return Ok(1);
    }
    if requested_workers == Some(0) {
        return Err(PipelineError::InvalidWorkerCount);
    }
    let available = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    Ok(requested_workers
        .unwrap_or_else(|| available.min(2))
        .min(8)
        .min(frame_count)
        .max(1))
}

/// One Metal device/context is the physical renderer for CPU delivery. Multiple independent
/// contexts contend for the same GPU and make export slower; one ordered CPU preparer instead
/// stays one frame ahead of the single persistent executor, while that executor overlaps the
/// sink/encoder. Raster remains CPU-parallel, and an explicit caller override is always honored.
fn backend_default_workers(
    backend: SkiaBackendKind,
    requested_workers: Option<usize>,
) -> Option<usize> {
    #[cfg(all(target_os = "macos", feature = "native"))]
    if backend == SkiaBackendKind::Metal && requested_workers.is_none() {
        return Some(1);
    }
    let _ = backend;
    requested_workers
}

#[derive(Debug, Error)]
pub enum PipelineError {
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
    #[error("raster worker count must be positive")]
    InvalidWorkerCount,
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
