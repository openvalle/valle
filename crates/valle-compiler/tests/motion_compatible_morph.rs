#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::{
    EvalInputs, Expr, MotionValue, SceneArtifact, eval_all, motion_context_at_frame, resolve_props,
};
use valle_timeline::FrameRate;

const PAIR: &str = r##"
export const composition = { width: 160, height: 120, fps: 30, duration: 2 };
const A = path("M 12 12 L 60 12 L 60 60 L 36 36 L 12 60 Z");
const B = path("M 68 10 L 142 10 L 130 48 L 142 100 L 68 100 Z");
export default function Compatible(ctx) {
  return <Scene style={{ width: 160, height: 120 }}>
    <Path d={morph(A, B, ctx.progress, { method: "compatible" })} fill="#ffffff" />
  </Scene>;
}
"##;

const SEQUENCE: &str = r##"
export const composition = { width: 160, height: 120, fps: 30, duration: 2 };
const A = path("M 12 12 L 60 12 L 60 60 L 36 36 L 12 60 Z");
const B = path("M 68 10 L 142 10 L 130 48 L 142 100 L 68 100 Z");
const C = path("M 10 10 L 140 10 L 140 100 L 75 54 L 10 100 Z");
export default function CompatibleSequence(ctx) {
  return <Scene style={{ width: 160, height: 120 }}>
    <Path d={morphSequence([A, B, C], [0, 0.5, 1], ctx.progress, { method: "compatible" })} fill="#ffffff" />
  </Scene>;
}
"##;

const AUTO_FALLBACK: &str = r##"
export const composition = { width: 160, height: 64, fps: 30, duration: 2 };
const A = path("M 26 6 L 5 18 L 23 17 L 14 15 L 28 16 Z");
const B = path("M 107 5 L 119 2 L 128 20 L 128 28 L 115 24 Z");
export default function AutoFallback(ctx) {
  return <Scene style={{width:160,height:64}}>
    <Path d={morph(A, B, ctx.progress)} fill="#ffffff" />
  </Scene>;
}
"##;

fn evaluated_path(
    artifact: &SceneArtifact,
    expr_index: usize,
    frame: u32,
) -> valle_motion::geometry::PathData {
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let values = eval_all(
        artifact,
        EvalInputs {
            ctx: &ctx,
            props: &props,
            unit: None,
            viewport: None,
        },
    )
    .unwrap();
    match &values[expr_index] {
        MotionValue::PathData(path) => path.clone(),
        other => panic!("expected path, got {other:?}"),
    }
}

#[test]
fn author_compatible_pair_serializes_and_evaluates_the_prepared_mesh() {
    let artifact = compile_motion(PAIR)
        .expect("compatible pair compiles")
        .artifact;
    artifact.validate().unwrap();
    let encoded = serde_json::to_vec(&artifact).unwrap();
    let decoded: SceneArtifact = serde_json::from_slice(&encoded).unwrap();
    decoded.validate().unwrap();
    let (index, prepared) = decoded
        .exprs
        .iter()
        .enumerate()
        .find_map(|(index, expr)| match expr {
            Expr::PathCompatibleMorph { prepared, .. } => Some((index, prepared)),
            _ => None,
        })
        .expect("prepared compatible expression");
    for frame in [0, 9, 18, 27, 36, 45, 59] {
        let path = evaluated_path(&decoded, index, frame);
        assert_eq!(path.points.len(), 5);
        assert_eq!(path, prepared.outline_path(frame as f64 / 60.0).unwrap());
    }
    let unsupported = PAIR.replace(
        "method: \"compatible\"",
        "method: \"compatible\", anchors: [[point(12, 12), point(68, 10)]]",
    );
    assert!(compile_motion(&unsupported).is_err());
}

#[test]
fn author_compatible_sequence_reaches_shared_key_shape() {
    let artifact = compile_motion(SEQUENCE)
        .expect("compatible sequence compiles")
        .artifact;
    artifact.validate().unwrap();
    let (index, segments) = artifact
        .exprs
        .iter()
        .enumerate()
        .find_map(|(index, expr)| match expr {
            Expr::PathCompatibleMorphSequence { segments, .. } => Some((index, segments)),
            _ => None,
        })
        .expect("prepared compatible sequence");
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].outline_path(1.0), segments[1].outline_path(0.0));
    assert_eq!(
        evaluated_path(&artifact, index, 30),
        segments[1].outline_path(0.0).unwrap()
    );
    for frame in [0, 15, 30, 45, 59] {
        assert_eq!(evaluated_path(&artifact, index, frame).points.len(), 5);
    }
}

#[test]
fn auto_retries_a_rejected_linear_correspondence_with_barycentric_morph() {
    let artifact = compile_motion(AUTO_FALLBACK)
        .expect("auto should use a guaranteed morph when arc length fails")
        .artifact;
    artifact.validate().unwrap();
    let index = artifact
        .exprs
        .iter()
        .position(|expr| matches!(expr, Expr::PathCompatibleMorph { .. }))
        .expect("compatible fallback");
    for frame in [0, 11, 27, 40, 59] {
        assert_eq!(evaluated_path(&artifact, index, frame).points.len(), 5);
    }
}

#[test]
fn authored_triangle_and_square_keep_their_boundaries_after_count_equalization() {
    let source = r##"
export const composition = { width: 120, height: 120, fps: 30, duration: 2 };
const A = path("M 10 10 L 50 10 L 50 50 L 10 50 Z");
const B = path("M 60 10 L 110 10 L 60 60 Z");
export default function DifferentCounts(ctx) {
  return <Scene style={{width:120,height:120}}>
    <Path d={morph(A, B, ctx.progress, { method: "compatible" })} fill="#fff" />
  </Scene>;
}
"##;
    let artifact = compile_motion(source)
        .expect("different vertex counts compile")
        .artifact;
    artifact.validate().unwrap();
    let index = artifact
        .exprs
        .iter()
        .position(|expr| matches!(expr, Expr::PathCompatibleMorph { .. }))
        .unwrap();
    assert_eq!(evaluated_path(&artifact, index, 0).points.len(), 4);
    assert_eq!(evaluated_path(&artifact, index, 30).points.len(), 4);
}

#[test]
fn compatible_sequence_uses_one_output_vertex_count_across_unequal_keys() {
    let source = r##"
export const composition = { width: 160, height: 120, fps: 30, duration: 2 };
const A = path("M 10 10 L 50 10 L 50 50 L 10 50 Z");
const B = path("M 60 10 L 110 10 L 60 60 Z");
const C = path("M 70 10 L 140 10 L 130 50 L 140 100 L 70 100 Z");
export default function DifferentSequence(ctx) {
  return <Scene style={{width:160,height:120}}>
    <Path d={morphSequence([A, B, C], [0, 0.5, 1], ctx.progress, { method: "compatible" })} fill="#fff" />
  </Scene>;
}
"##;
    let artifact = compile_motion(source)
        .expect("unequal key contours compile")
        .artifact;
    artifact.validate().unwrap();
    let (index, segments) = artifact
        .exprs
        .iter()
        .enumerate()
        .find_map(|(index, expr)| match expr {
            Expr::PathCompatibleMorphSequence { segments, .. } => Some((index, segments)),
            _ => None,
        })
        .unwrap();
    assert_eq!(segments[0].outline_path(1.0), segments[1].outline_path(0.0));
    for frame in [0, 15, 30, 45, 59] {
        assert_eq!(evaluated_path(&artifact, index, frame).points.len(), 5);
    }
}
