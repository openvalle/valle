use super::*;
use crate::{
    models::ModelManager,
    tools::{CancellationToken, NoopProgress, model_session::test_support::candidates},
};

const PREVIOUS: &[u8] = b"previous successful upscale";

fn request(root: &Path) -> UpscaleRequest {
    let request = UpscaleRequest {
        input: root.join("input.png"),
        output: root.join("output.png"),
        model: ModelSelection::pinned_default(MODEL_ID),
        scale: PUBLISHED_SCALE,
        range: None,
        overwrite: true,
    };
    write_rgba_png(&request.input, &RgbaFrame::new(2, 2)).unwrap();
    std::fs::write(&request.output, PREVIOUS).unwrap();
    request
}

fn assert_rollback(root: &Path, request: &UpscaleRequest) {
    assert_eq!(std::fs::read(&request.output).unwrap(), PREVIOUS);
    for entry in std::fs::read_dir(root).unwrap() {
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
fn corrupt_png_rolls_back_the_prepared_output_before_loading() {
    let root = tempfile::tempdir().unwrap();
    let request = request(root.path());
    std::fs::write(&request.input, b"not a PNG").unwrap();
    let manager = ModelManager::from_models_root(root.path().join("models"));
    let mut progress = NoopProgress;
    let mut context = RunContext::new(&manager, &mut progress);
    let error = run_png(
        request.clone(),
        FileOutputTransaction::new(&request.output, true).unwrap(),
        candidates(MODEL_ID, &root.path().join("absent-weights")),
        Instant::now(),
        &mut context,
    )
    .unwrap_err();
    assert_eq!(error.code, ToolErrorCode::InvalidInput);
    assert_rollback(root.path(), &request);
}

#[test]
fn cancellation_on_decode_or_load_does_not_open_weights_or_replace_the_output() {
    for cancel_phase in [ToolPhase::Decoding, ToolPhase::Loading] {
        let root = tempfile::tempdir().unwrap();
        let request = request(root.path());
        let manager = ModelManager::from_models_root(root.path().join("models"));
        let token = CancellationToken::new();
        let mut phases = Vec::new();
        let mut progress = |event: ToolEvent| {
            phases.push(event.phase);
            if event.phase == cancel_phase {
                token.cancel();
            }
        };
        let mut context = RunContext::new(&manager, &mut progress);
        context.cancellation = token.clone();
        let error = run_png(
            request.clone(),
            FileOutputTransaction::new(&request.output, true).unwrap(),
            candidates(MODEL_ID, &root.path().join("absent-weights")),
            Instant::now(),
            &mut context,
        )
        .unwrap_err();
        assert_eq!(
            error.code,
            ToolErrorCode::Cancelled,
            "{cancel_phase:?}: {error}"
        );
        assert!(phases.contains(&cancel_phase));
        assert!(!phases.contains(&ToolPhase::Inferencing));
        assert_rollback(root.path(), &request);
    }
}
