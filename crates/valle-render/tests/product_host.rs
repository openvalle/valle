mod support;

use std::{io::Cursor, sync::Arc};

use valle_engine::render::FrameKey;
use valle_engine::{
    frame::{RenderQuality, RenderSpec},
    resource::{ContentDigest, OutputBackground, OutputSpec},
};
use valle_render::{
    executor::skia::SkiaBackendKind,
    host::{
        FrameSchedule, NativeProject, NativeRenderOptions, NativeRenderer, NativeResourceCatalog,
    },
};

fn encoded_image() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut bytes), 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ])
            .unwrap();
    }
    bytes
}

fn project() -> NativeProject {
    let image = encoded_image();
    let digest = ContentDigest::of_bytes(&image);
    let digest_wire = digest.to_wire();
    let render = support::opened_image_render(&digest_wire);
    let mut catalog = NativeResourceCatalog::new();
    catalog
        .insert_bytes(digest, Arc::<[u8]>::from(image))
        .unwrap();
    NativeProject::from_render(render, Arc::new(catalog))
}

#[test]
fn native_project_schedule_and_preview_keep_one_render_id() {
    let project = project();
    let render = project.render_id();
    assert_eq!(project.render().render_id(), render);
    assert_eq!(project.compiled().render_id(), render);

    let frames = FrameSchedule::Range {
        from_seconds: 0.5,
        to_seconds: 0.6,
    }
    .frames(project.compiled().canvas())
    .unwrap();
    assert_eq!(
        frames.iter().map(|frame| frame.index()).collect::<Vec<_>>(),
        [15, 16, 17]
    );

    let output = OutputSpec::srgb_preview(OutputBackground::opaque_srgb([12, 34, 56])).unwrap();
    let spec = RenderSpec::new(2, 2, RenderQuality::Preview, output).unwrap();
    let mut runner = project.frame_runner(SkiaBackendKind::Raster).unwrap();
    let (frame, evidence) = runner.render_rgba8(FrameKey::new(0), spec).unwrap();
    assert_eq!(evidence.render_id, render);
    assert_eq!((frame.width, frame.height), (2, 2));
    assert!(frame.data.iter().any(|channel| *channel != 0));
}

#[test]
fn native_renderer_summary_reports_the_pinned_render() {
    let project = project();
    let render = project.render_id();
    let mut options = NativeRenderOptions::default();
    options.background = OutputBackground::opaque_srgb([12, 34, 56]);
    options.raster_workers = Some(1);
    let output = std::env::temp_dir().join(format!(
        "valle-native-fixed-render-{}-{}.png",
        std::process::id(),
        render
    ));
    let summary = NativeRenderer::new(project, options)
        .preview_frame_key(FrameKey::new(0), &output)
        .unwrap();
    assert_eq!(summary.render_id, render);
    assert_eq!(summary.pipeline.as_ref().unwrap().render_id, render);
    assert_eq!(summary.frames, 1);
    assert!(std::fs::metadata(&output).unwrap().len() > 0);
    std::fs::remove_file(output).unwrap();
}

#[test]
fn native_renderer_rejects_a_frame_outside_the_pinned_canvas() {
    let project = project();
    let frame_count = project.compiled().canvas().frame_count();
    let output = std::env::temp_dir().join(format!(
        "valle-native-out-of-range-{}-{frame_count}.png",
        std::process::id()
    ));
    let error = NativeRenderer::new(project, NativeRenderOptions::default())
        .preview_frame_key(FrameKey::new(frame_count), &output)
        .unwrap_err();
    assert!(matches!(
        error,
        valle_render::host::NativeRenderError::FrameOutOfRange {
            frame,
            frame_count: limit,
        } if frame == frame_count && limit == frame_count
    ));
    assert!(!output.exists());
}
