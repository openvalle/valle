#![cfg(feature = "native")]

mod support;

use std::{
    io::Cursor,
    sync::Arc,
    time::{Duration, Instant},
};
use valle_engine::{
    frame::{RenderQuality, RenderSpec},
    render::FrameKey,
    resource::{ContentDigest, OutputBackground, OutputSpec},
};
use valle_render::{
    executor::skia::SkiaBackendKind,
    host::{
        DeliveredFrame, FrameSink, NativeProject, NativeRenderBudget, NativeResourceCatalog,
        PipelineError, RenderControl, render_schedule,
    },
};

fn project() -> NativeProject {
    let mut image = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut image), 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[
                255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 255, 255, 255, 255, 64,
            ])
            .unwrap();
    }
    let digest = ContentDigest::of_bytes(&image);
    let render = support::opened_image_render(&digest.to_wire());
    let mut catalog = NativeResourceCatalog::new();
    catalog.insert_bytes(digest, Arc::<[u8]>::from(image));
    NativeProject::from_render(render, Arc::new(catalog))
}

fn spec() -> RenderSpec {
    RenderSpec::new(
        320,
        180,
        RenderQuality::Preview,
        OutputSpec::srgb_preview(OutputBackground::Transparent).unwrap(),
    )
    .unwrap()
}

#[derive(Default)]
struct Capture {
    frames: Vec<(i64, Vec<u8>)>,
    finished: bool,
    cancel: Option<RenderControl>,
    fail: bool,
}
impl FrameSink for Capture {
    fn push(&mut self, frame: DeliveredFrame<'_>) -> anyhow::Result<()> {
        // Exercise a slow downstream with strict ordered delivery and duplicates.
        std::thread::sleep(Duration::from_millis(1));
        if self.fail {
            anyhow::bail!("test sink failure");
        }
        assert_eq!(frame.sequence, self.frames.len());
        self.frames
            .push((frame.key.index(), frame.pixels.data.clone()));
        if let Some(control) = &self.cancel {
            control.cancel();
        }
        Ok(())
    }
    fn finish(&mut self) -> anyhow::Result<()> {
        self.finished = true;
        Ok(())
    }
}

#[test]
fn constrained_single_and_parallel_schedules_preserve_random_access_pixels() {
    let project = project();
    let frames = [
        0, 15, 3, 0, 29, 10, 2, 20, 8, 14, 29, 7, 11, 0, 19, 27, 3, 29,
    ]
    .map(FrameKey::new);
    let mut reference = Capture::default();
    render_schedule(
        &project,
        &frames,
        spec(),
        &mut reference,
        &RenderControl::default(),
        None,
        SkiaBackendKind::Raster,
    )
    .unwrap();
    for (threads, memory_mib) in [(1, 64), (4, 64), (8, 64), (16, 64), (16, 12)] {
        let memory_bytes = memory_mib * 1024 * 1024;
        let budget = NativeRenderBudget::new(threads, Some(memory_bytes)).unwrap();
        let control = RenderControl::with_deadline(Instant::now() + Duration::from_secs(10))
            .with_budget(budget);
        let mut sink = Capture::default();
        let report = render_schedule(
            &project,
            &frames,
            spec(),
            &mut sink,
            &control,
            None,
            SkiaBackendKind::Raster,
        )
        .unwrap();
        assert_eq!(sink.frames, reference.frames);
        assert!(sink.finished);
        assert!(report.preparation_tasks > 0 && report.preparation_tasks <= threads);
        assert_eq!(report.budget.unwrap().executors, report.preparation_tasks);
        assert!(report.maximum_in_flight_frames <= threads + 1);
        // One owner shares the image across tasks. Low-memory admission can evict it;
        // those extra decodes must preserve pixels and release their leases too.
        assert_eq!(
            report.resource_cache_misses + report.resource_cache_hits,
            frames.len() as u64
        );
        if memory_mib == 64 {
            assert_eq!(report.resource_cache_misses, 1);
        }
        let budget = report.budget.unwrap();
        assert_eq!(budget.cpu_threads, threads);
        assert_eq!(budget.resource_providers, 1);
        assert!(budget.peak_cpu_slots <= threads);
        assert!(budget.configured_capacity_bytes.unwrap() <= memory_bytes);
        assert!(budget.memory.peak_bytes <= memory_bytes);
        assert!(budget.memory.peak_live_bytes > 0);
        assert_eq!(budget.memory.used_bytes, 0);
    }
}

#[test]
fn shared_small_byte_budget_makes_progress_and_returns_every_lease() {
    let project = project();
    let budget = NativeRenderBudget::new(1, Some(12 * 1024 * 1024)).unwrap();
    std::thread::scope(|scope| {
        for preview in [false, true] {
            let project = &project;
            let budget = if preview {
                budget.for_preview()
            } else {
                budget.clone()
            };
            scope.spawn(move || {
                let control =
                    RenderControl::with_deadline(Instant::now() + Duration::from_secs(15))
                        .with_budget(budget);
                let frames = (0..120).map(|i| FrameKey::new(i % 30)).collect::<Vec<_>>();
                let mut sink = Capture::default();
                let report = render_schedule(
                    project,
                    &frames,
                    spec(),
                    &mut sink,
                    &control,
                    None,
                    SkiaBackendKind::Raster,
                )
                .unwrap();
                assert!(sink.finished);
                assert_eq!(report.frames, frames.len());
                assert!(report.budget.unwrap().memory.peak_bytes <= 12 * 1024 * 1024);
            });
        }
    });
    assert_eq!(budget.memory_report().used_bytes, 0);
}

#[test]
fn a_panicking_sink_drains_every_stage_and_releases_shared_memory() {
    struct PanickingSink;
    impl FrameSink for PanickingSink {
        fn push(&mut self, _: DeliveredFrame<'_>) -> anyhow::Result<()> {
            panic!("sink panic");
        }
        fn finish(&mut self) -> anyhow::Result<()> {
            panic!("must not finish");
        }
    }
    for threads in [1, 4] {
        let budget = NativeRenderBudget::new(threads, Some(64 * 1024 * 1024)).unwrap();
        let control = RenderControl::with_deadline(Instant::now() + Duration::from_secs(5))
            .with_budget(budget.clone());
        let frames = (0..30).map(FrameKey::new).collect::<Vec<_>>();
        let result = render_schedule(
            &project(),
            &frames,
            spec(),
            &mut PanickingSink,
            &control,
            None,
            SkiaBackendKind::Raster,
        );
        assert!(matches!(result, Err(PipelineError::StagePanicked)));
        assert_eq!(budget.memory_report().used_bytes, 0);
    }
}

#[test]
fn nested_synchronous_delivery_rejects_reusing_its_own_allowance() {
    struct ReentrantSink<'a> {
        project: &'a NativeProject,
        control: RenderControl,
        calls: usize,
    }
    impl FrameSink for ReentrantSink<'_> {
        fn push(&mut self, _: DeliveredFrame<'_>) -> anyhow::Result<()> {
            let error = render_schedule(
                self.project,
                &[FrameKey::new(0)],
                spec(),
                &mut Capture::default(),
                &self.control,
                None,
                SkiaBackendKind::Raster,
            )
            .unwrap_err();
            assert!(matches!(error, PipelineError::Budget(_)));
            assert!(error.to_string().contains("nested"));
            self.calls += 1;
            Ok(())
        }
        fn finish(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
    }
    let project = project();
    let budget = NativeRenderBudget::new(1, Some(32 * 1024 * 1024)).unwrap();
    let control = RenderControl::default().with_budget(budget.clone());
    let mut sink = ReentrantSink {
        project: &project,
        control: control.clone(),
        calls: 0,
    };
    render_schedule(
        &project,
        &[FrameKey::new(0)],
        spec(),
        &mut sink,
        &control,
        None,
        SkiaBackendKind::Raster,
    )
    .unwrap();
    assert_eq!(sink.calls, 1);
    assert_eq!(budget.memory_report().used_bytes, 0);
}

#[test]
fn cancellation_and_sink_failure_drain_rendezvous_producers() {
    let project = project();
    let frames = (0..30).map(FrameKey::new).collect::<Vec<_>>();
    for threads in [1, 4] {
        for fail in [false, true] {
            let control = RenderControl::with_deadline(Instant::now() + Duration::from_secs(10))
                .with_budget(NativeRenderBudget::new(threads, Some(64 * 1024 * 1024)).unwrap());
            let mut sink = Capture {
                fail,
                cancel: (!fail).then(|| control.clone()),
                ..Default::default()
            };
            let result = render_schedule(
                &project,
                &frames,
                spec(),
                &mut sink,
                &control,
                None,
                SkiaBackendKind::Raster,
            );
            assert!(!sink.finished);
            if fail {
                assert!(
                    matches!(result, Err(PipelineError::Sink(ref message)) if message.contains("test sink failure"))
                );
            } else {
                assert!(matches!(result, Err(PipelineError::Cancelled)));
                assert_eq!(sink.frames.len(), 1);
            }
        }
    }
}

#[test]
fn an_impossible_budget_fails_before_delivering_pixels() {
    let mut sink = Capture::default();
    let control =
        RenderControl::default().with_budget(NativeRenderBudget::new(4, Some(1024)).unwrap());
    let error = render_schedule(
        &project(),
        &[FrameKey::new(0)],
        spec(),
        &mut sink,
        &control,
        None,
        SkiaBackendKind::Raster,
    )
    .unwrap_err();
    assert!(matches!(error, PipelineError::Budget(_)));
    assert!(sink.frames.is_empty());
    assert!(!sink.finished);
}

#[test]
fn large_delivery_working_sets_are_admitted_with_each_frame_and_released() {
    use valle_engine::motion::memory::{self, MemoryKind};
    struct MediaSink {
        frames: usize,
    }
    impl FrameSink for MediaSink {
        fn working_bytes(&self, _: usize) -> anyhow::Result<u64> {
            Ok(20 * 1024 * 1024)
        }
        fn push(&mut self, _: DeliveredFrame<'_>) -> anyhow::Result<()> {
            // A multi-track audio delivery can need more than the old fixed 8 MiB reserve.
            let _pcm = memory::reserve(MemoryKind::Media, 20 * 1024 * 1024)?;
            self.frames += 1;
            Ok(())
        }
        fn finish(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
    }
    for threads in [1, 4] {
        let budget = NativeRenderBudget::new(threads, Some(64 * 1024 * 1024)).unwrap();
        let control = RenderControl::with_deadline(Instant::now() + Duration::from_secs(10))
            .with_budget(budget.clone());
        let mut sink = MediaSink { frames: 0 };
        render_schedule(
            &project(),
            &(0..60).map(|i| FrameKey::new(i % 30)).collect::<Vec<_>>(),
            spec(),
            &mut sink,
            &control,
            None,
            SkiaBackendKind::Raster,
        )
        .unwrap();
        assert_eq!(sink.frames, 60);
        assert!(budget.memory_report().peak_bytes <= 64 * 1024 * 1024);
        assert_eq!(budget.memory_report().used_bytes, 0);
    }
}
