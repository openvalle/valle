use super::*;
use crate::{
    codec::write_gray8_png,
    frame::{Gray8Frame, RgbaFrame},
    models::ModelManager,
    tools::{
        CancellationToken, NoopProgress, ProgressSink, model_session::test_support::candidates,
    },
};

const PREVIOUS: &[u8] = b"previous successful result";

struct Fixture {
    root: tempfile::TempDir,
    request: InpaintRequest,
    source: RgbaFrame,
}

impl Fixture {
    fn new(mask: &Gray8Frame, overwrite: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = RgbaFrame {
            width: 2,
            height: 2,
            data: vec![
                255, 0, 0, 255, 0, 255, 0, 127, 0, 0, 255, 0, 31, 63, 95, 200,
            ],
        };
        let request = InpaintRequest {
            input: root.path().join("input.png"),
            mask: root.path().join("mask.png"),
            output: root.path().join("output.png"),
            model: ModelSelection::pinned_default(MODEL_ID),
            range: None,
            overwrite,
        };
        write_rgba_png(&request.input, &source).unwrap();
        write_gray8_png(&request.mask, mask).unwrap();
        if overwrite {
            std::fs::write(&request.output, PREVIOUS).unwrap();
        }
        Self {
            root,
            request,
            source,
        }
    }

    fn run(&self, progress: &mut dyn ProgressSink) -> Result<ToolRun<InpaintResult>, ToolError> {
        let manager = ModelManager::from_models_root(self.root.path().join("models"));
        let mut context = RunContext::new(&manager, progress);
        self.run_with_context(&mut context)
    }

    fn run_with_context(
        &self,
        context: &mut RunContext<'_>,
    ) -> Result<ToolRun<InpaintResult>, ToolError> {
        run_png(
            self.request.clone(),
            FileOutputTransaction::new(&self.request.output, self.request.overwrite).unwrap(),
            candidates(MODEL_ID, &self.root.path().join("absent-weights")),
            Instant::now(),
            context,
        )
    }

    fn assert_rollback(&self) {
        assert_eq!(std::fs::read(&self.request.output).unwrap(), PREVIOUS);
        assert_eq!(read_rgba_png(&self.request.input).unwrap(), self.source);
        self.assert_no_staging();
    }

    fn assert_no_staging(&self) {
        for entry in std::fs::read_dir(self.root.path()).unwrap() {
            assert!(
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".valle-staging-")
            );
        }
    }
}

#[test]
fn empty_mask_publishes_an_exact_rgba_copy_without_opening_weights() {
    for overwrite in [false, true] {
        let fixture = Fixture::new(&Gray8Frame::new(2, 2), overwrite);
        let mut phases = Vec::new();
        let run = fixture
            .run(&mut |event: ToolEvent| phases.push(event.phase))
            .unwrap();
        assert_eq!(
            read_rgba_png(&fixture.request.output).unwrap(),
            fixture.source
        );
        assert_eq!(run.result.frames, 1);
        assert_eq!((run.result.tasks, run.result.scaled_tasks), (0, 0));
        assert_eq!(run.report.processed.frames, 1);
        assert_eq!(run.report.models[0].id, MODEL_ID);
        assert_eq!(run.report.timing.load_seconds, 0.0);
        assert_eq!(run.report.timing.inference_seconds, 0.0);
        assert_eq!(run.result.outputs[0].path, fixture.request.output);
        assert_eq!(phases.last(), Some(&ToolPhase::Completed));
        assert!(!phases.contains(&ToolPhase::Loading));
        assert!(!phases.contains(&ToolPhase::Inferencing));
        fixture.assert_no_staging();
    }
}

#[test]
fn invalid_png_input_or_mask_rolls_back_the_prepared_output() {
    for corrupt_mask in [false, true] {
        let fixture = Fixture::new(&Gray8Frame::new(2, 2), true);
        let path = if corrupt_mask {
            &fixture.request.mask
        } else {
            &fixture.request.input
        };
        std::fs::write(path, b"not a PNG").unwrap();
        let error = fixture.run(&mut NoopProgress).unwrap_err();
        assert_eq!(error.code, ToolErrorCode::InvalidInput);
        assert_eq!(std::fs::read(&fixture.request.output).unwrap(), PREVIOUS);
        fixture.assert_no_staging();
    }
}

#[test]
fn mismatched_and_soft_masks_fail_before_loading_and_preserve_the_original() {
    for mask in [
        Gray8Frame::new(1, 2),
        Gray8Frame::from_data(2, 2, vec![0, 127, 0, 255]).unwrap(),
    ] {
        let fixture = Fixture::new(&mask, true);
        let mut phases = Vec::new();
        let error = fixture
            .run(&mut |event: ToolEvent| phases.push(event.phase))
            .unwrap_err();
        assert_eq!(error.code, ToolErrorCode::InvalidInput);
        assert!(!phases.contains(&ToolPhase::Loading));
        fixture.assert_rollback();
    }
}

#[test]
fn cancellation_during_decode_plan_or_encode_discards_staging_and_keeps_the_previous_file() {
    for cancel_phase in [
        ToolPhase::Decoding,
        ToolPhase::Preprocessing,
        ToolPhase::Encoding,
    ] {
        let fixture = Fixture::new(&Gray8Frame::new(2, 2), true);
        let manager = ModelManager::from_models_root(fixture.root.path().join("models"));
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
        let error = fixture.run_with_context(&mut context).unwrap_err();
        assert_eq!(
            error.code,
            ToolErrorCode::Cancelled,
            "{cancel_phase:?}: {error}"
        );
        assert!(phases.contains(&cancel_phase));
        assert!(!phases.contains(&ToolPhase::Completed));
        fixture.assert_rollback();
    }
}

#[test]
fn cancellation_on_load_does_not_open_weights_or_replace_the_output() {
    let mask = Gray8Frame::from_data(2, 2, vec![255, 0, 0, 0]).unwrap();
    let fixture = Fixture::new(&mask, true);
    let manager = ModelManager::from_models_root(fixture.root.path().join("models"));
    let token = CancellationToken::new();
    let mut phases = Vec::new();
    let mut progress = |event: ToolEvent| {
        phases.push(event.phase);
        if event.phase == ToolPhase::Loading {
            token.cancel();
        }
    };
    let mut context = RunContext::new(&manager, &mut progress);
    context.cancellation = token.clone();
    let error = fixture.run_with_context(&mut context).unwrap_err();
    assert_eq!(error.code, ToolErrorCode::Cancelled, "{error}");
    assert!(phases.contains(&ToolPhase::Loading));
    assert!(!phases.contains(&ToolPhase::Inferencing));
    fixture.assert_rollback();
}
