//! Cancelled jobs stop before model resolution and leave existing artifacts untouched.

#[cfg(any(
    feature = "tool-inpaint",
    feature = "tool-upscale",
    feature = "tool-interpolate",
    feature = "tool-separate"
))]
mod tests {
    use std::path::{Path, PathBuf};
    use valle_media::{
        models::{ModelManager, ModelSelection},
        tools::{CancellationToken, RunContext, ToolError, ToolErrorCode, ToolEvent, ToolPhase},
    };

    fn check(
        run: impl Fn(&Path, PathBuf, &mut RunContext<'_>) -> Result<(), ToolError>,
        output_is_file: bool,
    ) {
        for already_cancelled in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let input = root.path().join("input.mp4");
            let output = root.path().join("output.mp4");
            let mask = root.path().join("mask.mkv");
            std::fs::write(&input, b"original source").unwrap();
            std::fs::write(&mask, b"original mask").unwrap();
            if output_is_file {
                std::fs::write(&output, b"previous result").unwrap();
            }
            let manager = ModelManager::from_models_root(root.path().join("empty-model-cache"));
            let token = CancellationToken::new();
            if already_cancelled {
                token.cancel();
            }
            let mut phases = Vec::new();
            let mut progress = |event: ToolEvent| {
                phases.push(event.phase);
                if event.phase == ToolPhase::Resolving {
                    token.cancel();
                }
            };
            let mut context = RunContext::new(&manager, &mut progress);
            context.cancellation = token.clone();
            let error = run(&input, output.clone(), &mut context).unwrap_err();
            assert_eq!(error.code, ToolErrorCode::Cancelled, "{error}");
            assert!(!phases.contains(&ToolPhase::Loading));
            assert!(!phases.contains(&ToolPhase::Completed));
            assert_eq!(std::fs::read(&input).unwrap(), b"original source");
            assert_eq!(std::fs::read(&mask).unwrap(), b"original mask");
            if output_is_file {
                assert_eq!(std::fs::read(&output).unwrap(), b"previous result");
            } else {
                assert!(!output.exists());
            }
            assert!(!manager.models_root().exists());
            for entry in std::fs::read_dir(root.path()).unwrap() {
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

    #[cfg(feature = "tool-inpaint")]
    #[test]
    fn inpaint_cancels_before_model_resolution() {
        use valle_media::tools::inpaint::{InpaintRequest, run};
        check(
            |input, output, context| {
                run(
                    InpaintRequest {
                        input: input.into(),
                        mask: input.parent().unwrap().join("mask.mkv"),
                        output,
                        model: ModelSelection::pinned_default("lama"),
                        range: None,
                        overwrite: true,
                    },
                    context,
                )
                .map(|_| ())
            },
            true,
        );
    }

    #[cfg(feature = "tool-upscale")]
    #[test]
    fn upscale_cancels_before_model_resolution() {
        use valle_media::tools::upscale::{UpscaleRequest, run};
        check(
            |input, output, context| {
                run(
                    UpscaleRequest {
                        input: input.into(),
                        output,
                        model: ModelSelection::pinned_default("realesrgan"),
                        scale: 4,
                        range: None,
                        overwrite: true,
                    },
                    context,
                )
                .map(|_| ())
            },
            true,
        );
    }

    #[cfg(feature = "tool-interpolate")]
    #[test]
    fn interpolate_cancels_before_model_resolution() {
        use valle_media::tools::interpolate::{InterpolateRequest, run};
        check(
            |input, output, context| {
                run(
                    InterpolateRequest {
                        input: input.into(),
                        output,
                        model: ModelSelection::pinned_default("rife"),
                        fps: 60,
                        shots: None,
                        overwrite: true,
                    },
                    context,
                )
                .map(|_| ())
            },
            true,
        );
    }

    #[cfg(feature = "tool-separate")]
    #[test]
    fn separate_cancels_before_model_resolution() {
        use valle_media::tools::separate::{SeparateRequest, run};
        check(
            |input, output_dir, context| {
                run(
                    SeparateRequest {
                        input: input.into(),
                        output_dir,
                        model: ModelSelection::pinned_default("demucs"),
                    },
                    context,
                )
                .map(|_| ())
            },
            false,
        );
    }
}
