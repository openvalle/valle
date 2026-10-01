#![cfg(feature = "motion")]

use std::collections::BTreeMap;

use valle_compiler::motion::compile_motion;
use valle_motion::{
    Expr, Fonts, LayoutOptions, Viewport, build_tree, default_font_naming, emit,
    motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

const SOURCE: &str = r##"
export const composition = { width: 320, height: 180, fps: 30, duration: 2 };
const OUTLINE = path("M 20 20 L 80 20 L 80 80 L 20 80 Z");
const PREPARED = resamplePath(OUTLINE, 32);
export default function Resampled(ctx) {
  return <Scene style={{ width: 320, height: 180, backgroundColor: "#101010" }}>
    <Path key="prepared" d={PREPARED} fill="#ffffff" />
    <Path key="animated" d={resamplePath(
      arc(point(180 + ctx.progress * 10, 90), 40, 0, TAU), 64
    )} fill="#f472b6" />
  </Scene>;
}
"##;

#[test]
fn resample_path_works_in_preparation_and_frame_evaluation() {
    let compiled = compile_motion(SOURCE).expect("both resampling phases compile");
    let artifact = &compiled.artifact;
    assert!(
        artifact
            .exprs
            .iter()
            .any(|expr| matches!(expr, Expr::PathResample { count: 64, .. }))
    );
    let prepared = prepare_scene(artifact).expect("prepare");
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).expect("props");
    let fonts = Fonts::default();
    for frame in [0, 30, 59] {
        let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let tree = build_tree(
            &prepared,
            &ctx,
            &props,
            &LayoutOptions {
                viewport: Viewport::new((320, 180)),
                fonts: &fonts,
                styles: None,
            },
        )
        .expect("layout");
        let report = emit(&tree, &default_font_naming).expect("emit");
        assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    }
}

#[test]
fn resample_path_rejects_dynamic_counts_and_tampered_budgets() {
    assert!(compile_motion(&SOURCE.replace("), 64", "), ctx.localFrame + 64")).is_err());
    assert!(compile_motion(&SOURCE.replace("), 64", "), 64.5")).is_err());
    let mut artifact = compile_motion(SOURCE).unwrap().artifact;
    let count = artifact
        .exprs
        .iter_mut()
        .find_map(|expr| match expr {
            Expr::PathResample { count, .. } => Some(count),
            _ => None,
        })
        .unwrap();
    *count = 0;
    assert!(artifact.validate().is_err());
}
