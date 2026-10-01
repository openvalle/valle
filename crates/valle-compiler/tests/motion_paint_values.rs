#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::{
    Fonts, LayoutOptions, StyleCache, Viewport, build_tree, default_font_naming, emit,
    motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

fn program(source: &str, frame: u32) -> Vec<u8> {
    let artifact = compile_motion(source)
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:?}"))
        .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let ctx = motion_context_at_frame(frame, 30, FrameRate::new(30, 1).unwrap()).unwrap();
    let fonts = Fonts::default();
    let styles = StyleCache::new();
    let tree = build_tree(
        &prepared,
        &ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((64, 64)),
            fonts: &fonts,
            styles: Some(&styles),
        },
    )
    .unwrap();
    emit(&tree, &default_font_naming)
        .unwrap()
        .program
        .packed_bytes()
        .unwrap()
}

#[test]
fn frame_time_gradient_survives_local_alias_and_component_prop() {
    let header = r##"export const composition={width:64,height:64,fps:30,duration:1};
function Shape(ctx, {paint}) {
  return <Path d="M 0 0 L 64 0 L 64 64 L 0 64 Z" fill={paint}/>;
}
function DirectShape(ctx, props) {
  return <Path d="M 0 0 L 64 0 L 64 64 L 0 64 Z" fill={props.paint}/>;
}
const shape = (paint) => <Path d="M 0 0 L 64 0 L 64 64 L 0 64 Z" fill={paint}/>;
"##;
    let gradient = r##"linearGradient(point(0,0),point(64,0),[
      gradientStop(0,interpolate(ctx.seconds,[0,1],["#ff0000","#00ff00"])),
      gradientStop(1,"#0000ff")])"##;
    let direct = format!(
        "{header}export default function Demo(ctx) {{return <Scene><Path d=\"M 0 0 L 64 0 L 64 64 L 0 64 Z\" fill={{{gradient}}}/></Scene>;}}"
    );
    let local = format!(
        "{header}export default function Demo(ctx) {{const paint={gradient}; const alias=paint; return <Scene><Path d=\"M 0 0 L 64 0 L 64 64 L 0 64 Z\" fill={{alias}}/></Scene>;}}"
    );
    let passed = format!(
        "{header}export default function Demo(ctx) {{const paint={gradient}; return <Scene><Shape paint={{paint}}/></Scene>;}}"
    );
    let passed_direct = format!(
        "{header}export default function Demo(ctx) {{const paint={gradient}; return <Scene><DirectShape paint={{paint}}/></Scene>;}}"
    );
    let passed_helper = format!(
        "{header}export default function Demo(ctx) {{const paint={gradient}; return <Scene>{{shape(paint)}}</Scene>;}}"
    );
    for frame in [0, 15, 29] {
        let expected = program(&direct, frame);
        assert_eq!(program(&local, frame), expected, "local frame {frame}");
        assert_eq!(program(&passed, frame), expected, "prop frame {frame}");
        assert_eq!(
            program(&passed_direct, frame),
            expected,
            "direct prop frame {frame}"
        );
        assert_eq!(
            program(&passed_helper, frame),
            expected,
            "helper frame {frame}"
        );
    }
    assert_ne!(program(&direct, 0), program(&direct, 29));
}

#[test]
fn frame_time_paint_cannot_be_used_as_a_scalar() {
    let source = r##"export default function Demo(ctx) {
      const paint=linearGradient(point(0,0),point(64,0),[
        gradientStop(0,interpolate(ctx.seconds,[0,1],["red","blue"])),
        gradientStop(1,"green")]);
      return <Scene><View style={{opacity:paint}}/></Scene>;
    }"##;
    let diagnostics = compile_motion(source).unwrap_err();
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("is a paint value; use it as Path fill/stroke or Mask paint")
        }),
        "{diagnostics:?}"
    );
}
