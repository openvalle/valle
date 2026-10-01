#![cfg(feature = "motion")]
use valle_compiler::motion::compile_motion;
use valle_motion::{
    DiagCode, Fonts, LayoutOptions, Viewport, build_tree, motion_context_at_frame, prepare_scene,
    resolve_props,
};
use valle_timeline::FrameRate;

fn filter(amount: &str) -> String {
    format!(
        r##"export const composition={{width:320,height:180,fps:30,duration:2}};
export default function Main(ctx) {{
 const amount = {amount};
 return <Scene><View key="blur" style={{{{width:320,height:180,
 filter:`radial-blur(160px 90px ${{amount}}px)`}}}} /></Scene>;
}}"##
    )
}
const BOUNCY: &str = "spring({elapsedFrames:ctx.localFrame,fps:ctx.fps,preset:'bouncy'}) * 120";

#[test]
fn spring_overshoot_is_reported_before_render_without_changing_values() {
    let source = filter(BOUNCY);
    let compiled = compile_motion(&source).unwrap();
    let warning = compiled
        .warnings
        .iter()
        .find(|w| w.code == DiagCode::ParameterRange)
        .expect("overshoot warning");
    assert!(warning.message.contains("radial-blur amount"));
    assert!(warning.message.contains("128"));
    assert!(warning.span.line > 1);
    let prepared = prepare_scene(&compiled.artifact).unwrap();
    let props = resolve_props(&compiled.artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let ctx = motion_context_at_frame(10, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let error = match build_tree(
        &prepared,
        &ctx,
        &props,
        &LayoutOptions {
            fonts: &fonts,
            viewport: Viewport::new((320, 180)),
            styles: None,
        },
    ) {
        Ok(_) => panic!("overshoot must remain a strict runtime error"),
        Err(e) => e.to_string(),
    };
    assert!(error.contains("radial-blur amount ="), "{error}");
    assert!(error.contains("expression "), "{error}");
    assert!(
        !error.contains("width: 320"),
        "whole CSS block must not bury the parameter: {error}"
    );
    let clamped = compile_motion(&filter(&format!("clamp({BOUNCY},0,128)"))).unwrap();
    assert!(
        clamped
            .warnings
            .iter()
            .all(|w| w.code != DiagCode::ParameterRange),
        "{:?}",
        clamped.warnings
    );
}

#[test]
fn known_bad_ranges_fail_and_unknown_inputs_do_not_forge_proofs() {
    let errors = compile_motion(&filter("200 + ctx.progress * 2")).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("radial-blur amount range")),
        "{errors:?}"
    );
    assert!(compile_motion(&filter("clamp(200 + ctx.progress * 2,0,128)")).is_ok());
    // Source frames are host-defined; the compiler must not invent a 30fps limit.
    assert!(compile_motion(&filter("ctx.localFrame")).is_ok());
}

#[test]
fn transitions_share_strict_range_diagnostics() {
    let source = r##"export const composition={width:320,height:180,fps:30,duration:2};
    export default function Main(ctx) {return <Scene><Transition kind="circleOpen" progress={0.5}
        params={{centerX:ctx.progress*2,softness:3}}><View/><View/></Transition></Scene>; }"##;
    let compiled = compile_motion(source).unwrap();
    assert!(
        compiled
            .warnings
            .iter()
            .any(|w| w.code == DiagCode::ParameterRange && w.message.contains("centerX"))
    );
    let clamped =
        compile_motion(&source.replace("ctx.progress*2", "clamp(ctx.progress*2,0,1)")).unwrap();
    assert!(
        clamped
            .warnings
            .iter()
            .all(|w| w.code != DiagCode::ParameterRange)
    );
}
