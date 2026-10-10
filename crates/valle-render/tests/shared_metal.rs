#![cfg(all(target_os = "macos", feature = "native"))]

mod support;

use std::{ffi::c_void, sync::Arc};

use valle_engine::{
    frame::{RenderQuality, RenderSpec},
    render::FrameKey,
    resource::{ContentDigest, OutputBackground, OutputSpec},
};
use valle_media::{
    Muxer, SharedVideoFrame, SharedVideoFrameHandle, SharedVideoFramePool, VideoFrameTransport,
};
use valle_render::{
    executor::skia::SkiaBackendKind,
    host::{
        DeliveredFrame, DeliveredSharedFrame, FrameSink, NativeProject, NativeResourceCatalog,
        RenderControl, render_schedule,
    },
};

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVPixelBufferLockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(buffer: *mut c_void) -> *const u8;
    fn CVPixelBufferGetBytesPerRow(buffer: *mut c_void) -> usize;
}

fn bgra(frame: &SharedVideoFrame) -> Vec<u8> {
    let SharedVideoFrameHandle::VideoToolbox(buffer) = frame.handle() else {
        panic!("expected VideoToolbox")
    };
    let (width, height) = frame.dimensions();
    // SAFETY: The delivered frame owns this buffer and GPU completion has been
    // observed. Keep the read-only lock for the full row copy, excluding padding.
    unsafe {
        assert_eq!(CVPixelBufferLockBaseAddress(buffer, 1), 0);
        let base = CVPixelBufferGetBaseAddress(buffer);
        let stride = CVPixelBufferGetBytesPerRow(buffer);
        assert!(!base.is_null() && stride >= width as usize * 4);
        let mut bytes = Vec::new();
        for y in 0..height as usize {
            bytes.extend_from_slice(std::slice::from_raw_parts(
                base.add(y * stride),
                width as usize * 4,
            ));
        }
        assert_eq!(CVPixelBufferUnlockBaseAddress(buffer, 1), 0);
        bytes
    }
}

struct ProbeSink {
    pool: SharedVideoFramePool,
    frames: Vec<(FrameKey, SharedVideoFrame)>,
    finished: bool,
    fail_at: Option<usize>,
    cancel: Option<RenderControl>,
}

impl FrameSink for ProbeSink {
    fn push(&mut self, _: DeliveredFrame<'_>) -> anyhow::Result<()> {
        anyhow::bail!("unexpected CPU delivery")
    }
    fn shared_frame_pool(&self) -> Option<SharedVideoFramePool> {
        Some(self.pool.clone())
    }
    fn push_shared(&mut self, frame: DeliveredSharedFrame<'_>) -> anyhow::Result<()> {
        assert_eq!(frame.sequence, self.frames.len());
        assert_eq!(frame.evidence.delivery_readback_bytes, 0);
        if self.fail_at == Some(frame.sequence) {
            anyhow::bail!("intentional sink failure")
        }
        self.frames.push((frame.key, frame.frame));
        if self.frames.len() == 2
            && let Some(control) = &self.cancel
        {
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
#[ignore = "requires real Metal and a VideoToolbox frame pool"]
fn shared_queues_preserve_order_pixels_and_complete_on_sink_failure_or_cancel() {
    let mut image = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut image, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ])
            .unwrap();
    }
    let digest = ContentDigest::of_bytes(&image);
    let render = support::opened_image_render(&digest.to_wire());
    let mut catalog = NativeResourceCatalog::new();
    catalog.insert_bytes(digest, Arc::<[u8]>::from(image));
    let project = NativeProject::from_render(render, Arc::new(catalog));
    let directory = tempfile::tempdir().unwrap();
    let muxer = Muxer::open_ext_with_transport(
        &directory.path().join("pool.mp4"),
        64,
        48,
        30,
        None,
        true,
        None,
        VideoFrameTransport::SharedGpu,
    )
    .unwrap();
    let pool = muxer.shared_frame_pool().unwrap();
    let spec = RenderSpec::new(
        64,
        48,
        RenderQuality::Preview,
        OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
    )
    .unwrap();
    let frames = [0, 7, 2, 7, 23, 29, 1].map(FrameKey::new);
    let mut expected: Option<Vec<Vec<u8>>> = None;
    for (workers, fail, cancel) in [
        (1, false, false),
        (2, false, false),
        (4, false, false),
        (8, false, false),
        (4, true, false),
        (4, false, true),
    ] {
        let control = RenderControl::default();
        let mut sink = ProbeSink {
            pool: pool.clone(),
            frames: Vec::new(),
            finished: false,
            fail_at: fail.then_some(2),
            cancel: cancel.then(|| control.clone()),
        };
        let result = render_schedule(
            &project,
            &frames,
            spec,
            &mut sink,
            &control,
            None,
            SkiaBackendKind::Metal,
            Some(workers),
        );
        if fail || cancel {
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains(if fail {
                    "intentional sink failure"
                } else {
                    "cancel"
                }),
                "{error}"
            );
            assert!(!sink.finished);
            assert_eq!(sink.frames.len(), 2);
        } else {
            assert_eq!(result.unwrap().frames, frames.len());
            assert!(sink.finished);
        }
        // Read after all executors have been dropped, including the error paths.
        let actual = sink
            .frames
            .iter()
            .enumerate()
            .map(|(sequence, (key, frame))| {
                assert_eq!(*key, frames[sequence]);
                let bytes = bgra(frame);
                assert!(bytes.chunks_exact(4).any(|p| p[0] != p[1]));
                bytes
            })
            .collect::<Vec<_>>();
        if let Some(expected) = &expected {
            assert_eq!(actual, expected[..actual.len()]);
        } else {
            expected = Some(actual);
        }
    }
}
