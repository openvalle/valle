#![cfg(feature = "motion")]
//! Measured values are artifact constants with font bytes as explicit inputs. Missing measurement environments and frame-dependent inputs must fail.

#![cfg(feature = "motion")]

use valle_compiler::motion::{MeasureEnv, compile_motion, compile_motion_with_env};
use valle_motion::{Expr, MotionValue};

const FONT: &[u8] =
    include_bytes!("../../valle-motion/assets/fonts/noto/NotoSansCJKsc-Regular.otf");

fn env() -> MeasureEnv {
    MeasureEnv::new(&[FONT.to_vec()], (1920, 1080)).expect("font bundle builds a measure env")
}

/// Place a measured static text width in an opacity slot to isolate measurement behavior.
fn source(expr: &str) -> String {
    format!(
        "const M = measureText(\"全球业务网络\", {{ style: \"font-size: 48px\" }});\n\
         export default function Card(ctx) {{\n\
         \x20 return <Scene key=\"scene\" style={{{{ opacity: {expr} }}}} />;\n\
         }}\n"
    )
}

/// Collect both static styles and constant expressions when checking baked measurement values.
fn folded_numbers(artifact: &valle_motion::SceneArtifact) -> Vec<f64> {
    let from_exprs = artifact.exprs.iter().filter_map(|expr| match expr {
        Expr::Const {
            value: MotionValue::Number(value),
        } => Some(*value),
        _ => None,
    });
    let from_styles = artifact
        .nodes
        .iter()
        .flat_map(|node| &node.styles)
        .filter_map(|binding| match &binding.value {
            valle_motion::StyleValue::Static {
                value: MotionValue::Number(value),
            } => Some(*value),
            _ => None,
        });
    from_exprs.chain(from_styles).collect()
}

#[test]
fn measured_width_folds_into_the_artifact_as_a_constant() {
    let compiled = compile_motion_with_env(&source("M.width / 1000"), &[], Some(&env()))
        .expect("measure-backed source compiles when fonts are supplied");
    let numbers = folded_numbers(&compiled.artifact);
    // Six CJK characters at 48px should measure within 200-400px, allowing font-metric variation.
    let width = numbers
        .iter()
        .map(|value| value * 1000.0)
        .find(|value| (200.0..400.0).contains(value));
    assert!(
        width.is_some(),
        "no plausibly-measured width folded into the artifact: {numbers:?}"
    );
    compiled.artifact.validate().expect("artifact validates");
}

#[test]
fn measuring_without_a_font_bundle_fails_closed_and_points_at_the_flag() {
    // Missing fonts must prevent artifact production.
    let diagnostics =
        compile_motion(&source("M.width / 1000")).expect_err("no fonts must not compile");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("--font")),
        "diagnostic must point at the fix, got: {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| &diagnostic.message)
            .collect::<Vec<_>>()
    );
}

#[test]
fn an_empty_font_list_uses_the_canonical_default_font() {
    let fallback = MeasureEnv::new(&[], (1920, 1080)).expect("canonical fallback");
    let explicit = MeasureEnv::new(&[FONT.to_vec()], (1920, 1080)).expect("explicit default");
    let fallback_artifact =
        compile_motion_with_env(&source("M.width / 1000"), &[], Some(&fallback))
            .expect("fallback compile")
            .artifact;
    let explicit_artifact =
        compile_motion_with_env(&source("M.width / 1000"), &[], Some(&explicit))
            .expect("explicit compile")
            .artifact;
    assert_eq!(
        valle_motion::canonical_bytes(&fallback_artifact).unwrap(),
        valle_motion::canonical_bytes(&explicit_artifact).unwrap(),
        "implicit fallback and the canonical explicit font must measure identically"
    );
}

#[test]
fn frame_varying_measure_inputs_fail_closed() {
    // A ctx-dependent measureText argument must produce a preparation-time capability diagnostic.
    let source = "export default function Card(ctx) {\n  \
                  const m = measureText(\"x\", { maxWidth: ctx.enter.progress * 100 });\n  \
                  return <Scene key=\"scene\" style={{ opacity: m.width }} />;\n}\n";
    let diagnostics =
        compile_motion_with_env(source, &[], Some(&env())).expect_err("frame-varying measure");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("prepare-time capability")),
        "got: {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| &diagnostic.message)
            .collect::<Vec<_>>()
    );
}

#[test]
fn measurement_is_reproducible_across_compiles() {
    // Identical source and font bytes must produce identical artifacts.
    let first = compile_motion_with_env(&source("M.width / 1000"), &[], Some(&env()))
        .expect("first compile");
    let second = compile_motion_with_env(&source("M.width / 1000"), &[], Some(&env()))
        .expect("second compile");
    assert_eq!(
        valle_motion::canonical_bytes(&first.artifact).unwrap(),
        valle_motion::canonical_bytes(&second.artifact).unwrap(),
        "measure must not introduce compile-to-compile drift"
    );
}

#[test]
fn bad_measure_options_surface_as_authored_diagnostics_not_panics() {
    let source = "const M = measureText(\"x\", { maxWidth: -5 });\n\
                  export default function Card(ctx) {\n  \
                  return <Scene key=\"scene\" style={{ opacity: M.width }} />;\n}\n";
    let diagnostics =
        compile_motion_with_env(source, &[], Some(&env())).expect_err("negative maxWidth");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("maxWidth")),
        "got: {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| &diagnostic.message)
            .collect::<Vec<_>>()
    );
}

#[test]
fn measure_cannot_be_used_to_smuggle_nondeterminism_into_prepare() {
    // Installing measurement capabilities must preserve sandbox determinism restrictions.
    let source = "const M = measureText(String(Date.now()));\n\
                  export default function Card(ctx) {\n  \
                  return <Scene key=\"scene\" style={{ opacity: M.width }} />;\n}\n";
    let diagnostics = compile_motion_with_env(source, &[], Some(&env()))
        .expect_err("Date must stay forbidden with measure installed");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("Date")),
        "got: {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| &diagnostic.message)
            .collect::<Vec<_>>()
    );
}
