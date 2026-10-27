//! One ordered GPU submission actor, bounded pending readbacks and optional context probing.
//! Software encoding stays on the delivery actor; a slow sink backpressures the whole window.

use super::*;
use std::{collections::VecDeque, sync::atomic::AtomicU64};
use crate::host::{
    frame::{FrameExecutionStage, PreparedFrame, SubmittedVulkanFrame},
    gpu_schedule::{AdaptiveQueues, QueueSample},
};

struct Pending {
    owner: usize,
    sequence: usize,
    key: FrameKey,
    frame: SubmittedVulkanFrame,
    memory: Option<valle_engine::motion::memory::MemoryCredit>,
    delivery_memory: Option<valle_engine::motion::memory::MemoryLease>,
    delivery_bytes: u64,
    submit_us: u64,
}

fn micros(started: Instant) -> u64 {
    started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    first: FrameExecutionStage,
    project: &NativeProject,
    spec: RenderSpec,
    control: &RenderControl,
    prepared: &Receiver<(usize, FrameKey, PreparedFrame)>,
    output: &Sender<Result<Outcome, PipelineError>>,
    abort: &AtomicBool,
    failed: &AtomicUsize,
    sink_us: &AtomicU64,
    total: usize,
    window: usize,
) -> Result<(), PipelineError> {
    let limit = window.min(3).max(1);
    // Pending tokens must drop before their owning Skia contexts on every exit.
    let mut executors = vec![first];
    let mut pending = VecDeque::with_capacity(limit);
    let mut ordered: BTreeMap<usize, (FrameKey, PreparedFrame)> = BTreeMap::new();
    let mut next = 0;
    let mut active = 1;
    let mut queues = AdaptiveQueues::new(control.maximum_gpu_contexts.min(limit));
    let mut ordered_shadows = false;
    let mut last_completion = Instant::now();
    let mut prepare_wait_us = 0;
    if std::env::var_os("VALLE_PERF").is_some() {
        eprintln!("[valle vulkan] asynchronous-readback=true pending-limit={limit} queues=1 adaptive=true");
    }
    loop {
        if abort.load(Ordering::Acquire) { break; }
        control.check()?;
        if next > failed.load(Ordering::Acquire) {
            // No more frames can be delivered after the earliest failed preparation.
            ordered.clear();
            if pending.is_empty() { break; }
        }
        let pressure = budget_pressure(control);
        let have_next = ordered.contains_key(&next);
        if !pending.is_empty() && (pending.len() >= limit || pressure || !have_next) {
            let ready = pending.front().map(|p: &Pending| p.frame.ready()).transpose()
                .map_err(|e| PipelineError::at_frame(project, pending.front().unwrap().key, spec, e))?
                .unwrap_or(false);
            if ready || pending.len() >= limit || pressure || next >= total {
                let mut sample = finish_head(project, spec, control, &mut executors, &mut pending,
                    output, abort, failed)?;
                sample.wall_us = micros(last_completion);
                last_completion = Instant::now();
                sample.prepare_wait_us = std::mem::take(&mut prepare_wait_us);
                sample.sink_us = sink_us.load(Ordering::Acquire);
                sample.shared_output_bytes = u64::from(spec.width()) * u64::from(spec.height()) * 8 * limit as u64;
                if let Some(change) = queues.observe(sample, total.saturating_sub(next),
                    || executors[0].vulkan_memory()) {
                    while !pending.is_empty() {
                        finish_head(project, spec, control, &mut executors, &mut pending,
                            output, abort, failed)?;
                    }
                    while executors.len() < change.retained_queues {
                        match control.execution_stage(project, SkiaBackendKind::Vulkan, spec) {
                            Ok(mut executor) => {
                                if !executor.supports_async_vulkan_output(spec) {
                                    queues.restrict_to(executors.len()); break;
                                }
                                executors.push(executor);
                            },
                            _ => { queues.restrict_to(executors.len()); break; },
                        }
                    }
                    executors.truncate(change.retained_queues);
                    active = change.queues.min(executors.len());
                    if let Some(trace) = &control.trace {
                        trace.evidence("vulkan.reconfigure", serde_json::json!({
                            "frame":next,"activeContexts":active,"retainedContexts":executors.len(),"reason":change.reason,
                        }));
                    }
                    if std::env::var_os("VALLE_PERF").is_some() {
                        eprintln!("[valle vulkan] queues={active} retained={} reason={}", executors.len(), change.reason);
                    }
                }
                continue;
            }
        }
        if pending.is_empty() && pressure {
            for executor in &mut executors { executor.trim_memory(); }
        }
        if let Some((key, packet)) = ordered.remove(&next) {
            if !ordered_shadows && packet.needs_ordered_shadow_context() {
                while !pending.is_empty() {
                    finish_head(project, spec, control, &mut executors, &mut pending,
                        output, abort, failed)?;
                }
                executors.truncate(1);
                active = 1;
                queues.restrict_to(1);
                ordered_shadows = true;
            }
            let sequence = next;
            next += 1;
            let memory = packet.memory_credit.clone();
            let delivery_bytes = packet.delivery_memory_bytes;
            let reserve = || valle_engine::motion::memory::reserve(
                valle_engine::motion::memory::MemoryKind::Delivery,
                u64::from(spec.width()) * u64::from(spec.height()) * 4,
            );
            let delivery_memory = match memory.as_ref() {
                Some(credit) => credit.run(reserve),
                None => reserve(),
            }.map_err(|e| PipelineError::at_frame(project, key, spec, e.into()))?;
            let owner = sequence % active;
            let started = Instant::now();
            match executors[owner].render_prepared_vulkan_frame(packet, spec) {
                Ok(frame) => pending.push_back(Pending {
                    owner, sequence, key, frame, memory, delivery_memory, delivery_bytes,
                    submit_us: micros(started),
                }),
                Err(error) => frame_failed(PipelineError::at_frame(project, key, spec, error),
                    sequence, failed, output, abort, control)?,
            }
            if let Some(trace) = &control.trace {
                trace.evidence("vulkan.queue", serde_json::json!({"frame":key.index(),"sequence":sequence,
                    "pending":pending.len(),"limit":limit,"activeContexts":active,"retainedContexts":executors.len(),
                    "delivery":"asynchronous CPU readback"}));
                if sequence % 24 == 0 && let Some(memory) = executors[0].vulkan_memory() {
                    trace.evidence("vulkan.memory", serde_json::json!({"frame":key.index(),
                        "allocatedBytes":memory.allocated,"optionalConcurrencyBudgetBytes":memory.budget,
                        "scope":"Vulkan device-local heaps including other processes"}));
                }
            }
            continue;
        }
        if next >= total && pending.is_empty() { break; }
        let started = Instant::now();
        match prepared.recv_timeout(Duration::from_millis(1)) {
            Ok((sequence, key, packet)) if sequence <= failed.load(Ordering::Acquire) => {
                if sequence < next || ordered.insert(sequence, (key, packet)).is_some() {
                    return Err(PipelineError::IncompleteEvidence);
                }
            },
            Ok(_) | Err(RecvTimeoutError::Timeout) => {},
            Err(RecvTimeoutError::Disconnected) => {
                while !pending.is_empty() {
                    finish_head(project, spec, control, &mut executors, &mut pending,
                        output, abort, failed)?;
                }
                break;
            },
        }
        prepare_wait_us += micros(started);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn finish_head(
    project: &NativeProject, spec: RenderSpec, control: &RenderControl,
    executors: &mut [FrameExecutionStage], pending: &mut VecDeque<Pending>,
    output: &Sender<Result<Outcome, PipelineError>>, abort: &AtomicBool, failed: &AtomicUsize,
) -> Result<QueueSample, PipelineError> {
    let frame = pending.pop_front().ok_or(PipelineError::IncompleteEvidence)?;
    let wait = Instant::now();
    while !frame.frame.ready().map_err(|e| PipelineError::at_frame(project, frame.key, spec, e))? {
        control.check()?;
        if abort.load(Ordering::Acquire) { return Ok(QueueSample::default()); }
        std::thread::sleep(Duration::from_micros(100));
    }
    let wait_us = micros(wait);
    let rendered = executors[frame.owner].finish_vulkan_frame(frame.frame, wait_us);
    let (pixels, evidence) = match rendered {
        Ok(value) => value,
        Err(error) => {
            frame_failed(PipelineError::at_frame(project, frame.key, spec, error),
                frame.sequence, failed, output, abort, control)?;
            return Ok(QueueSample::default());
        },
    };
    let (cache, reuse_cache_pressure) = executors[frame.owner].opacity_layer_cache_usage();
    let sample = QueueSample {
        gpu_wait_us: wait_us, submit_us: frame.submit_us,
        execute_us: evidence.timings.execute_us,
        context_surface_bytes: evidence.execution.surfaces.pool_bytes
            .saturating_add(evidence.execution.raster_layer_cache_bytes as u64)
            .saturating_add(cache as u64),
        reuse_cache_pressure, ..Default::default()
    };
    if let Some(credit) = &frame.memory { credit.limit_remaining(frame.delivery_bytes); }
    send(output, Ok(Outcome::Completed(CompletedFrame {
        memory: frame.memory, delivery_memory: frame.delivery_memory,
        sequence: frame.sequence, key: frame.key, pixels: CompletedPixels::Rgba(pixels), evidence,
    })), abort, control)?;
    Ok(sample)
}
