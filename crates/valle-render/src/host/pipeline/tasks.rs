//! Bounded preparation and drawing stages. Resources have one session owner; physical drawing
//! contexts consume prepared packets independently of the task that prepared them.

use super::*;
use crossbeam_channel::{Receiver, RecvTimeoutError, SendTimeoutError, Sender};
use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
    time::Duration,
};

const CHECK_INTERVAL: Duration = Duration::from_millis(10);

fn receive<T>(
    rx: &Receiver<T>,
    abort: &AtomicBool,
    control: &RenderControl,
    mut idle: impl FnMut(),
) -> Result<Option<T>, PipelineError> {
    loop {
        if abort.load(Ordering::Acquire) {
            return Ok(None);
        }
        control.check()?;
        match rx.recv_timeout(CHECK_INTERVAL) {
            Ok(value) => return Ok(Some(value)),
            Err(RecvTimeoutError::Timeout) => idle(),
            Err(RecvTimeoutError::Disconnected) => return Ok(None),
        }
    }
}

fn send<T>(
    tx: &Sender<T>,
    mut value: T,
    abort: &AtomicBool,
    control: &RenderControl,
) -> Result<(), PipelineError> {
    loop {
        if abort.load(Ordering::Acquire) {
            return Ok(());
        }
        control.check()?;
        match tx.send_timeout(value, CHECK_INTERVAL) {
            Ok(()) => return Ok(()),
            Err(SendTimeoutError::Timeout(returned)) => value = returned,
            Err(SendTimeoutError::Disconnected(_)) => return Ok(()),
        }
    }
}

fn actor(
    abort: &AtomicBool,
    output: &Sender<Result<CompletedFrame, PipelineError>>,
    control: &RenderControl,
    work: impl FnOnce() -> Result<(), PipelineError>,
) {
    let result =
        catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|_| Err(PipelineError::StagePanicked));
    if let Err(error) = result {
        // The failed job has no completed frame, so its window slot is available in this queue.
        let _ = output.try_send(Err(error));
        if let Some(admission) = &control.admission {
            admission.stop();
        }
        abort.store(true, Ordering::Release);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    project: &NativeProject,
    frames: &[FrameKey],
    spec: RenderSpec,
    sink: &mut dyn FrameSink,
    control: &RenderControl,
    progress: Option<&ProgressCallback>,
    backend: SkiaBackendKind,
    concurrency: usize,
) -> Result<PipelineReport, PipelineError> {
    let window = concurrency.min(frames.len()).max(1);
    let first = control.preparation_stage(project, backend, spec)?;
    let preparers = (0..window).map(|_| first.fork()).collect::<Vec<_>>();
    drop(first);
    let abort = AtomicBool::new(false);
    let mut metrics = PipelineAccumulator::for_render_id(project.render_id());
    let mut delivered = 0;
    let result = std::thread::scope(|scope| {
        let (jobs_tx, jobs_rx) = crossbeam_channel::bounded::<(usize, FrameKey)>(window);
        let (prepared_tx, prepared_rx) = crossbeam_channel::bounded(window);
        let (output_tx, output_rx) = crossbeam_channel::bounded(window);
        for mut preparer in preparers {
            let jobs = jobs_rx.clone();
            let prepared = prepared_tx.clone();
            let output = output_tx.clone();
            let abort = &abort;
            scope.spawn(move || {
                actor(abort, &output, control, || {
                    while let Some((sequence, key)) = receive(&jobs, abort, control, || {})? {
                        let packet = preparer.prepare_frame_for_sequence(sequence, key, spec)?;
                        send(&prepared, (sequence, key, packet), abort, control)?;
                    }
                    Ok(())
                })
            });
        }
        drop(jobs_rx);
        drop(prepared_tx);
        let execution_slots = if backend == SkiaBackendKind::Raster {
            window
        } else {
            1
        };
        for _ in 0..execution_slots {
            let prepared = prepared_rx.clone();
            let output = output_tx.clone();
            let abort = &abort;
            scope.spawn(move || {
                actor(abort, &output, control, || {
                    let mut executor = control.execution_stage(project, backend, spec)?;
                    while let Some((sequence, key, packet)) =
                        receive(&prepared, abort, control, || {
                            if budget_pressure(control) {
                                executor.trim_memory();
                            }
                        })?
                    {
                        let memory = packet.memory_credit.clone();
                        let delivery_memory_bytes = packet.delivery_memory_bytes;
                        let reserve = || {
                            valle_engine::motion::memory::reserve(
                                valle_engine::motion::memory::MemoryKind::Delivery,
                                u64::from(spec.width()) * u64::from(spec.height()) * 4,
                            )
                        };
                        let delivery_memory = control
                            .compute(|| match &memory {
                                Some(credit) => credit.run(reserve),
                                None => reserve(),
                            })
                            .map_err(FrameRenderError::from)?;
                        let (pixels, evidence) = executor.render_prepared_rgba8(packet, spec)?;
                        if let Some(credit) = &memory {
                            credit.limit_remaining(delivery_memory_bytes);
                        }
                        send(
                            &output,
                            Ok(CompletedFrame {
                                _memory: memory,
                                _delivery_memory: delivery_memory,
                                sequence,
                                key,
                                pixels,
                                evidence,
                            }),
                            abort,
                            control,
                        )?;
                    }
                    Ok(())
                })
            });
        }
        drop(prepared_rx);
        drop(output_tx);
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut next = 0;
            while next < window.min(frames.len()) {
                jobs_tx
                    .send((next, frames[next]))
                    .map_err(|_| PipelineError::IncompleteEvidence)?;
                next += 1;
            }
            let mut pending = BTreeMap::new();
            for sequence in 0..frames.len() {
                control.check()?;
                while !pending.contains_key(&sequence) {
                    control.check()?;
                    match output_rx.recv_timeout(CHECK_INTERVAL) {
                        Ok(Ok(frame)) => {
                            if frame.sequence < sequence
                                || pending.insert(frame.sequence, frame).is_some()
                            {
                                return Err(PipelineError::IncompleteEvidence);
                            }
                        }
                        Ok(Err(error)) => return Err(error),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => {
                            return Err(PipelineError::IncompleteEvidence);
                        }
                    }
                }
                let frame = pending.remove(&sequence).unwrap();
                metrics.add(&frame.evidence)?;
                let mut push = || {
                    sink.push(DeliveredFrame {
                        sequence,
                        key: frame.key,
                        pixels: &frame.pixels,
                        evidence: &frame.evidence,
                    })
                };
                match &frame._memory {
                    Some(credit) => credit.run(push),
                    None => push(),
                }
                .map_err(|e| PipelineError::Sink(e.to_string()))?;
                drop(frame);
                delivered += 1;
                if let Some(progress) = progress {
                    progress(delivered, frames.len());
                }
                control.check()?;
                // A new job can enter only after ordered delivery retires the preceding job.
                if next < frames.len() {
                    jobs_tx
                        .send((next, frames[next]))
                        .map_err(|_| PipelineError::IncompleteEvidence)?;
                    next += 1;
                }
            }
            Ok(())
        }))
        .unwrap_or(Err(PipelineError::StagePanicked));
        if let Some(admission) = &control.admission {
            admission.stop();
        }
        abort.store(true, Ordering::Release);
        drop(jobs_tx);
        drop(output_rx);
        result
    });
    result?;
    control.check()?;
    sink.finish()
        .map_err(|e| PipelineError::Sink(e.to_string()))?;
    metrics.finish(delivered, concurrency, window)
}
