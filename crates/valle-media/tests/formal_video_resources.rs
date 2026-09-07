#![cfg(feature = "tool-shots")]

//! Explicit, ignored resource proof against an installed formal OmniShotCut artifact.
//!
//! The ordinary test suite remains hermetic. Run this test only with synthetic long-form video
//! and a verified local model store:
//!
//! ```text
//! VALLE_MODEL_CACHE=/path/to/models \
//! VALLE_LONG_VIDEO_FIXTURE=/path/to/synthetic-long-video.mp4 \
//! ORT_DYLIB_PATH=/path/to/libonnxruntime \
//! cargo test -p valle-media --features tool-shots \
//!   --test formal_video_resources -- --ignored --nocapture
//! ```

use std::{ffi::OsString, mem::size_of, path::PathBuf};

use valle_media::{
    models::{ModelManager, ModelSelection, RunBackendPreference},
    tools::{
        CancellationToken, NoopProgress, RunContext, ToolErrorCode, ToolEvent, ToolPhase,
        shots::{ShotsRequest, run},
    },
};

fn required_path(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} must name an explicit local test fixture"))
}

fn request(input: &std::path::Path, output: PathBuf) -> ShotsRequest {
    ShotsRequest {
        input: input.to_owned(),
        output,
        model: ModelSelection {
            id: "omnishotcut".to_owned(),
            version: None,
            backend: RunBackendPreference::Onnx,
        },
        overwrite: false,
    }
}

fn directory_entries(path: &std::path::Path) -> Vec<OsString> {
    std::fs::read_dir(path)
        .expect("read proof output directory")
        .map(|entry| entry.expect("read proof output entry").file_name())
        .collect()
}

#[test]
#[ignore = "requires a formal OmniShotCut artifact, fixed ORT bundle, and synthetic long video"]
fn formal_onnx_long_video_bounds_buffers_and_cleans_resource_failures() {
    let models_root = required_path("VALLE_MODEL_CACHE");
    let input = required_path("VALLE_LONG_VIDEO_FIXTURE");
    assert!(input.is_file(), "long-video fixture is missing");
    let manager = ModelManager::from_models_root(models_root);

    let completed_root = tempfile::tempdir().expect("create completed-run output directory");
    let completed_output = completed_root.path().join("shots.json");
    let mut progress = NoopProgress;
    let mut context = RunContext::new(&manager, &mut progress);
    context.resources.temporary_directory = completed_root.path().to_owned();
    let completed = run(request(&input, completed_output.clone()), &mut context)
        .expect("run the formal detector over synthetic long video");

    assert!(completed_output.is_file());
    assert!(
        completed.result.frames > completed.result.max_buffered_model_frames,
        "fixture must exceed the model's rolling window"
    );
    assert!(
        completed.result.max_buffered_model_frames
            <= valle_media::models::inference::omnishotcut::MAX_BUFFERED_FRAMES as u64,
        "the formal adapter exceeded its fixed rolling-frame bound"
    );
    assert!(completed.report.timing.total_seconds > 0.0);
    assert_eq!(completed.report.processed.frames, completed.result.frames);

    let budget_root = tempfile::tempdir().expect("create budget-failure output directory");
    let budget_output = budget_root.path().join("shots.json");
    let mut progress = NoopProgress;
    let mut context = RunContext::new(&manager, &mut progress);
    context.resources.temporary_directory = budget_root.path().to_owned();
    context.resources.temporary_disk_budget_bytes = (2 * size_of::<i64>()) as u64;
    let error = run(request(&input, budget_output.clone()), &mut context)
        .expect_err("the PTS spool must honor the explicit disk budget");
    assert_eq!(error.code, ToolErrorCode::ResourceBusy);
    assert!(!budget_output.exists());
    assert!(
        directory_entries(budget_root.path()).is_empty(),
        "resource failure left an output staging directory or PTS spool"
    );

    let cancel_root = tempfile::tempdir().expect("create cancellation output directory");
    let cancel_output = cancel_root.path().join("shots.json");
    let cancellation = CancellationToken::new();
    let cancellation_from_progress = cancellation.clone();
    let mut progress = move |event: ToolEvent| {
        if event.phase == ToolPhase::Inferencing && event.completed.unwrap_or_default() >= 2 {
            cancellation_from_progress.cancel();
        }
    };
    let mut context = RunContext::new(&manager, &mut progress);
    context.cancellation = cancellation;
    context.resources.temporary_directory = cancel_root.path().to_owned();
    let error = run(request(&input, cancel_output.clone()), &mut context)
        .expect_err("progress-triggered cancellation must stop the formal run");
    assert_eq!(error.code, ToolErrorCode::Cancelled);
    assert!(!cancel_output.exists());
    assert!(
        directory_entries(cancel_root.path()).is_empty(),
        "cancellation left an output staging directory or PTS spool"
    );
}
