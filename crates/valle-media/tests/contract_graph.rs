#![cfg(feature = "media-tools")]
//! Real ORT + codecs + publication using small graphs with explicit numerical oracles.
//! Run with the pinned ORT_DYLIB_PATH and `--ignored --test-threads=1`.

#[path = "../test_support/models.rs"]
mod models;

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use valle_media::{
    codec::{
        TransparentVideoMuxer, decode_rgba_frames, read_rgba_png, write_gray8_png, write_rgba_png,
    },
    frame::{Gray8Frame, RgbaFrame},
    models::{ModelManager, ModelSelection, RunBackendPreference},
    tools::{
        CancellationToken, NoopProgress, RunContext, TimeRange, ToolErrorCode, ToolEvent, ToolPhase,
    },
};

fn selection(id: &str) -> ModelSelection {
    ModelSelection {
        id: id.into(),
        version: Some(models::VERSION.into()),
        backend: RunBackendPreference::Onnx,
    }
}

fn manager(root: &Path, id: &str) -> ModelManager {
    let cache = root.join("models");
    models::install(&cache, id);
    ModelManager::from_models_root(cache)
}

fn frame(width: u32, height: u32, color: [u8; 4]) -> RgbaFrame {
    let mut frame = RgbaFrame::new(width, height);
    frame.fill(color);
    frame
}

fn video(root: &Path, count: u32) -> PathBuf {
    let path = root.join("source.mov");
    let mut writer = TransparentVideoMuxer::open(&path, 64, 48, 10, 1).unwrap();
    for index in 0..count {
        writer
            .encode_video(&frame(64, 48, [40 + index as u8 * 10, 100, 150, 255]))
            .unwrap();
    }
    writer.finish().unwrap();
    path
}

fn mask_video(root: &Path, count: usize) -> PathBuf {
    let path = root.join("mask.mkv");
    let mut child = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "rawvideo",
            "-pixel_format",
            "gray",
            "-video_size",
            "64x48",
            "-framerate",
            "10",
            "-i",
            "pipe:0",
            "-c:v",
            "ffv1",
        ])
        .arg(&path)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&vec![255; 64 * 48 * count])
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    path
}

fn wav(path: &Path, rate: u32, channels: u16, samples: usize) {
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for index in 0..samples {
        for _ in 0..channels {
            writer
                .write_sample(if index % 2 == 0 { 0.25_f32 } else { -0.25_f32 })
                .unwrap();
        }
    }
    writer.finalize().unwrap();
}

fn no_staging(root: &Path) {
    for entry in fs::read_dir(root).unwrap() {
        assert!(
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".valle-staging-")
        );
    }
}

#[test]
#[ignore = "requires the pinned ORT_DYLIB_PATH bundle"]
fn inpaint_png_contract_graph_only_replaces_masked_rgb_and_keeps_alpha() {
    use valle_media::tools::inpaint::{self, InpaintRequest};
    let root = tempfile::tempdir().unwrap();
    let manager = manager(root.path(), "lama");
    let input = root.path().join("source.png");
    let mask = root.path().join("mask.png");
    let output = root.path().join("result.png");
    let source = RgbaFrame {
        width: 2,
        height: 2,
        data: vec![
            10, 20, 30, 0, 40, 50, 60, 90, 70, 80, 90, 150, 100, 110, 120, 255,
        ],
    };
    write_rgba_png(&input, &source).unwrap();
    write_gray8_png(
        &mask,
        &Gray8Frame::from_data(2, 2, vec![0, 255, 0, 255]).unwrap(),
    )
    .unwrap();
    let mut phases = Vec::new();
    let mut progress = |event: ToolEvent| phases.push(event.phase);
    let mut context = RunContext::new(&manager, &mut progress);
    context.resources.cpu_threads = 1;
    let run = inpaint::run(
        InpaintRequest {
            input,
            mask,
            output: output.clone(),
            model: selection("lama"),
            range: None,
            overwrite: false,
        },
        &mut context,
    )
    .unwrap();
    assert_eq!(run.result.frames, 1);
    assert_eq!(run.result.tasks, 1);
    assert_eq!(run.report.models[0].version, models::VERSION);
    let actual = read_rgba_png(&output).unwrap();
    for index in 0..4 {
        let start = index * 4;
        assert_eq!(actual.data[start + 3], source.data[start + 3]);
        // LaMa's u8 bridge truncates 0.25 * 255 to 63.
        assert_eq!(
            &actual.data[start..start + 3],
            if index % 2 == 0 {
                &source.data[start..start + 3]
            } else {
                &[63, 63, 63]
            }
        );
    }
    assert!(phases.contains(&ToolPhase::Inferencing));
    assert_eq!(phases.last(), Some(&ToolPhase::Completed));
    no_staging(root.path());
}

#[test]
#[ignore = "requires the pinned ORT_DYLIB_PATH bundle and FFmpeg"]
fn inpaint_video_contract_graph_keeps_frame_clock_for_full_and_range_jobs() {
    use valle_media::tools::inpaint::{self, InpaintRequest};
    for ranged in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let manager = manager(root.path(), "lama");
        let input = video(root.path(), 4);
        let mask = mask_video(root.path(), if ranged { 2 } else { 4 });
        let output = root.path().join("result.mp4");
        let mut progress = NoopProgress;
        let mut context = RunContext::new(&manager, &mut progress);
        context.resources.cpu_threads = 1;
        let run = inpaint::run(
            InpaintRequest {
                input,
                mask,
                output: output.clone(),
                model: selection("lama"),
                range: if ranged {
                    Some(TimeRange::new(0.1, Some(0.3)).unwrap())
                } else {
                    None
                },
                overwrite: false,
            },
            &mut context,
        )
        .unwrap();
        assert_eq!(run.result.frames, if ranged { 2 } else { 4 });
        let frames = decode_rgba_frames(&output, None).unwrap();
        assert_eq!(frames.len() as u64, run.result.frames);
        for actual in frames {
            for channel in &actual.data[..3] {
                assert!(channel.abs_diff(64) <= 3);
            }
        }
        assert!(run.result.pipeline.is_some());
        no_staging(root.path());
    }
}

#[test]
#[ignore = "requires the pinned ORT_DYLIB_PATH bundle"]
fn upscale_png_contract_graph_repeats_each_pixel_four_times_and_preserves_alpha() {
    use valle_media::tools::upscale::{self, UpscaleRequest};
    let root = tempfile::tempdir().unwrap();
    let manager = manager(root.path(), "realesrgan");
    let source = RgbaFrame {
        width: 2,
        height: 2,
        data: vec![
            10, 20, 30, 0, 40, 50, 60, 90, 70, 80, 90, 150, 100, 110, 120, 255,
        ],
    };
    let input = root.path().join("source.png");
    let output = root.path().join("result.png");
    write_rgba_png(&input, &source).unwrap();
    let mut progress = NoopProgress;
    let mut context = RunContext::new(&manager, &mut progress);
    context.resources.cpu_threads = 1;
    let run = upscale::run(
        UpscaleRequest {
            input,
            output: output.clone(),
            model: selection("realesrgan"),
            scale: 4,
            range: None,
            overwrite: false,
        },
        &mut context,
    )
    .unwrap();
    assert_eq!((run.result.output_width, run.result.output_height), (8, 8));
    let actual = read_rgba_png(&output).unwrap();
    for (x, y) in [(0, 0), (7, 0), (0, 7), (7, 7)] {
        let pixel = actual.pixel(x, y).unwrap();
        assert_eq!(&pixel[..3], &source.pixel(x / 4, y / 4).unwrap()[..3]);
    }
    assert_eq!(actual.pixel(0, 0).unwrap()[3], 0);
    assert_eq!(actual.pixel(7, 7).unwrap()[3], 255);
    assert!(run.result.tiles > 0);
    no_staging(root.path());
}

#[test]
#[ignore = "requires the pinned ORT_DYLIB_PATH bundle and FFmpeg"]
fn upscale_video_contract_graph_preserves_selection_and_source_cadence() {
    use valle_media::tools::upscale::{self, UpscaleRequest};
    for ranged in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let manager = manager(root.path(), "realesrgan");
        let input = video(root.path(), 4);
        let output = root.path().join("result.mp4");
        let mut progress = NoopProgress;
        let mut context = RunContext::new(&manager, &mut progress);
        context.resources.cpu_threads = 1;
        let run = upscale::run(
            UpscaleRequest {
                input,
                output: output.clone(),
                model: selection("realesrgan"),
                scale: 4,
                range: if ranged {
                    Some(TimeRange::new(0.1, Some(0.3)).unwrap())
                } else {
                    None
                },
                overwrite: false,
            },
            &mut context,
        )
        .unwrap();
        assert_eq!(run.result.frames, if ranged { 2 } else { 4 });
        assert_eq!(
            (run.result.output_width, run.result.output_height),
            (256, 192)
        );
        let frames = decode_rgba_frames(&output, None).unwrap();
        assert_eq!(frames.len() as u64, run.result.frames);
        assert!(frames[0].pixel(128, 96).unwrap()[0].abs_diff(if ranged { 50 } else { 40 }) <= 4);
        assert!(run.result.pipeline.is_some());
        no_staging(root.path());
    }
}

#[test]
#[ignore = "requires the pinned ORT_DYLIB_PATH bundle and FFmpeg"]
fn interpolation_contract_graph_generates_an_ordered_high_fps_video() {
    use valle_media::tools::interpolate::{self, InterpolateRequest};
    let root = tempfile::tempdir().unwrap();
    let manager = manager(root.path(), "rife");
    let input = video(root.path(), 4);
    let output = root.path().join("result.mp4");
    let mut progress = NoopProgress;
    let mut context = RunContext::new(&manager, &mut progress);
    context.resources.cpu_threads = 1;
    let run = interpolate::run(
        InterpolateRequest {
            input,
            output: output.clone(),
            model: selection("rife"),
            fps: 30,
            shots: None,
            overwrite: false,
        },
        &mut context,
    )
    .unwrap();
    assert_eq!(run.result.source_frames, 4);
    assert_eq!(run.result.output_frames, 12);
    assert!(run.result.generated_frames >= 6);
    let frames = decode_rgba_frames(&output, None).unwrap();
    assert_eq!(frames.len() as u64, run.result.output_frames);
    let reds = frames
        .iter()
        .map(|frame| frame.pixel(32, 24).unwrap()[0])
        .collect::<Vec<_>>();
    assert!(
        reds.windows(2).all(|pair| pair[1] + 3 >= pair[0]),
        "{reds:?}"
    );
    no_staging(root.path());
}

#[test]
#[ignore = "requires the pinned ORT_DYLIB_PATH bundle and FFmpeg"]
fn matte_contract_graph_multiplies_source_alpha_for_png_and_video() {
    use valle_media::tools::matte::{self, MatteRequest};
    for id in ["birefnet", "modnet"] {
        let root = tempfile::tempdir().unwrap();
        let manager = manager(root.path(), id);
        let source = frame(2, 2, [80, 100, 120, 128]);
        let input = root.path().join("source.png");
        let output = root.path().join("result.png");
        write_rgba_png(&input, &source).unwrap();
        let mut progress = NoopProgress;
        let mut context = RunContext::new(&manager, &mut progress);
        context.resources.cpu_threads = 1;
        let run = matte::run(
            MatteRequest {
                input,
                output: output.clone(),
                model: selection(id),
                range: None,
                sample_fps: None,
                overwrite: false,
            },
            &mut context,
        )
        .unwrap();
        assert_eq!(run.result.frames, 1);
        assert_eq!(
            read_rgba_png(&output).unwrap().pixel(0, 0).unwrap(),
            [80, 100, 120, 64]
        );
        let input = video(root.path(), 4);
        let output = root.path().join("result.mov");
        let run = matte::run(
            MatteRequest {
                input,
                output: output.clone(),
                model: selection(id),
                range: Some(TimeRange::new(0.1, Some(0.3)).unwrap()),
                sample_fps: Some(10.0),
                overwrite: false,
            },
            &mut context,
        )
        .unwrap();
        assert_eq!(run.result.frames, 2);
        let frames = decode_rgba_frames(&output, None).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].pixel(32, 24).unwrap()[3], 128);
        no_staging(root.path());
    }
}

#[test]
#[ignore = "requires the pinned ORT_DYLIB_PATH bundle and FFmpeg"]
fn enhancement_contract_graph_publishes_exact_length_wav_and_flac() {
    use valle_media::tools::enhance::{self, EnhanceRequest};
    for extension in ["wav", "flac"] {
        let root = tempfile::tempdir().unwrap();
        let manager = manager(root.path(), "dpdfnet");
        let input = root.path().join("source.wav");
        let output = root.path().join(format!("result.{extension}"));
        wav(&input, 48_000, 1, 5_003);
        let mut progress = NoopProgress;
        let mut context = RunContext::new(&manager, &mut progress);
        context.resources.cpu_threads = 1;
        context.resources.temporary_directory = root.path().into();
        let run = enhance::run(
            EnhanceRequest {
                input,
                output: output.clone(),
                model: selection("dpdfnet"),
                overwrite: false,
            },
            &mut context,
        )
        .unwrap();
        assert_eq!(run.result.input_samples, 5_003);
        assert_eq!(run.result.output_samples, 5_003);
        assert!(run.result.stream_working_set_high_watermark_bytes > 0);
        assert!(output.is_file());
        no_staging(root.path());
    }
}

#[test]
#[ignore = "requires the pinned ORT_DYLIB_PATH bundle and FFmpeg"]
fn separation_contract_graph_publishes_both_stems_with_channel_order_and_exact_length() {
    use valle_media::tools::separate::{self, SeparateRequest};
    let root = tempfile::tempdir().unwrap();
    let manager = manager(root.path(), "demucs");
    let input = root.path().join("source.wav");
    wav(&input, 44_100, 2, 5_000);
    let mut progress = NoopProgress;
    let mut context = RunContext::new(&manager, &mut progress);
    context.resources.cpu_threads = 1;
    context.resources.temporary_directory = root.path().into();
    let run = separate::run(
        SeparateRequest {
            input,
            output_dir: root.path().join("stems"),
            model: selection("demucs"),
        },
        &mut context,
    )
    .unwrap();
    assert_eq!(run.result.samples, 5_000);
    assert_eq!(run.result.outputs.len(), 2);
    assert_eq!(run.result.segments, 1);
    for artifact in &run.result.outputs {
        let mut reader = hound::WavReader::open(&artifact.path).unwrap();
        assert_eq!(reader.spec().channels, 2);
        let samples = reader
            .samples::<f32>()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        assert_eq!(samples.len(), 10_000);
        assert!((samples[0] - 0.125).abs() < 1e-5);
        assert!((samples[2] + 0.125).abs() < 1e-5);
    }
    no_staging(root.path());
}

#[test]
#[ignore = "requires the pinned ORT_DYLIB_PATH bundle and FFmpeg"]
fn shot_detection_contract_graph_uses_source_pts_and_publishes_canonical_payload() {
    use valle_media::tools::shots::{self, ShotsRequest};
    for id in ["omnishotcut", "transnetv2"] {
        let root = tempfile::tempdir().unwrap();
        let manager = manager(root.path(), id);
        let input = video(root.path(), 4);
        let output = root.path().join("shots.json");
        let mut progress = NoopProgress;
        let mut context = RunContext::new(&manager, &mut progress);
        context.resources.cpu_threads = 1;
        context.resources.temporary_directory = root.path().into();
        let run = shots::run(
            ShotsRequest {
                input,
                output: output.clone(),
                model: selection(id),
                overwrite: false,
            },
            &mut context,
        )
        .unwrap();
        assert_eq!(run.result.frames, 4);
        assert_eq!(run.result.shots.shots.len(), 1);
        assert_eq!(run.result.model_windows, 1);
        let value: serde_json::Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        assert_eq!(value, serde_json::to_value(run.result.shots).unwrap());
        no_staging(root.path());
    }
}

#[test]
#[ignore = "requires the pinned ORT_DYLIB_PATH bundle"]
fn cancellation_after_real_inference_rolls_back_the_previous_output() {
    use valle_media::tools::upscale::{self, UpscaleRequest};
    let root = tempfile::tempdir().unwrap();
    let manager = manager(root.path(), "realesrgan");
    let input = root.path().join("source.png");
    let output = root.path().join("result.png");
    write_rgba_png(&input, &frame(2, 2, [80, 100, 120, 255])).unwrap();
    fs::write(&output, b"previous result").unwrap();
    let token = CancellationToken::new();
    let mut progress = |event: ToolEvent| {
        if event.phase == ToolPhase::Validating {
            token.cancel();
        }
    };
    let mut context = RunContext::new(&manager, &mut progress);
    context.resources.cpu_threads = 1;
    context.cancellation = token.clone();
    let error = upscale::run(
        UpscaleRequest {
            input,
            output: output.clone(),
            model: selection("realesrgan"),
            scale: 4,
            range: None,
            overwrite: true,
        },
        &mut context,
    )
    .unwrap_err();
    assert_eq!(error.code, ToolErrorCode::Cancelled);
    assert_eq!(fs::read(output).unwrap(), b"previous result");
    no_staging(root.path());
}

#[test]
#[ignore = "requires the pinned ORT_DYLIB_PATH bundle"]
fn typed_runtime_contract_graph_preserves_output_order_types_and_owned_input_validation() {
    use valle_media::models::runtime::{
        TensorInput,
        onnx::{OnnxSession, OnnxTensorInput},
    };
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("typed.onnx");
    fs::write(&path, include_bytes!("fixtures/models/typed.onnx")).unwrap();
    let mut session = OnnxSession::load(&path).unwrap();
    let floats = [1.5, -2.5];
    let ints = [3, -4];
    let bytes = [5, 6];
    let inputs = [
        OnnxTensorInput::borrowed_f32("float_in", [2], &floats),
        OnnxTensorInput::borrowed_i32("int_in", [2], &ints),
        OnnxTensorInput::borrowed_u8("byte_in", [2], &bytes),
    ];
    assert_eq!(
        session.run_typed(&inputs, &["sum"]).unwrap()[0].data,
        [9.5, -0.5]
    );
    assert_eq!(
        session.run_typed_i64(&inputs, &["indices"]).unwrap()[0].data,
        [3, -4]
    );
    assert!(session.run_typed(&inputs, &["indices"]).is_err());
    assert!(session.run_typed_i64(&inputs, &["sum"]).is_err());
    assert!(
        session
            .run_f32(TensorInput::owned("float_in", [2], floats.to_vec()), "sum")
            .is_err()
    );
}

#[test]
#[ignore = "requires the pinned ORT_DYLIB_PATH bundle"]
fn omnishot_predictor_convenience_open_reuses_its_session_and_validates_threads() {
    use valle_media::models::inference::omnishotcut::{
        MODEL_WINDOW_BYTES, OrtSessionOptions, OrtWindowPredictor, WindowPredictor,
    };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/models/omnishotcut.onnx");
    let mut predictor = OrtWindowPredictor::open(&path).unwrap();
    for value in [0u8, 128] {
        let prediction = predictor
            .predict_window(&vec![value; MODEL_WINDOW_BYTES])
            .unwrap();
        assert_eq!(prediction.intra, vec![0; 24]);
        assert_eq!(prediction.inter, vec![0; 24]);
        assert_eq!(&prediction.ranges[..2], &[0, 100]);
    }
    assert!(predictor.predict_window(&[0]).is_err());
    assert!(
        OrtWindowPredictor::open_with_options(
            path,
            OrtSessionOptions {
                intra_threads: Some(0),
                ..Default::default()
            }
        )
        .is_err()
    );
}
