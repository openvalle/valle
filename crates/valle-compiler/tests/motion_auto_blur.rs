#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_draw::program::{Filter, Node};
use valle_motion::{
    Fonts, LayoutOptions, MotionValue, StyleValue, Viewport, build_tree, default_font_naming, emit,
    motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::{FrameRate, RationalTime};

fn program(source: &str, frame: u32) -> valle_draw::program::DrawProgram {
    let artifact = compile_motion(source).unwrap().artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    emit(&tree, &default_font_naming).unwrap().program
}

fn velocity(program: &valle_draw::program::DrawProgram) -> Option<[f32; 2]> {
    program.nodes().iter().find_map(|node| match node {
        Node::Group(group) => group.filters.iter().find_map(|filter| match filter {
            Filter::VelocityBlur { velocity, .. } => Some(*velocity),
            _ => None,
        }),
        _ => None,
    })
}

fn profiled_program(source: &str, frame: u32) -> (valle_draw::program::DrawProgram, u8) {
    let artifact = compile_motion(source).unwrap().artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let (tree, timings) = valle_motion::layout::build_tree_profiled(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    (
        emit(&tree, &default_font_naming).unwrap().program,
        timings.auto_blur_neighbor_layouts,
    )
}

#[test]
fn auto_blur_tracks_motion_and_stops_when_stationary() {
    let source = include_str!("fixtures/motion/composition/auto-motion-blur.motion.tsx");
    let artifact = compile_motion(source).unwrap().artifact;
    let window = prepare_scene(&artifact)
        .unwrap()
        .dependencies()
        .temporal_window();
    assert_eq!(window.past_frames, RationalTime::new(1, 2).unwrap());
    assert_eq!(window.future_frames, RationalTime::new(1, 2).unwrap());
    let moving = program(source, 10);
    let measured = velocity(&moving).expect("moving bar has directional blur");
    assert!((measured[0] - 20.0).abs() < 0.5, "{measured:?}");
    assert!(measured[1].abs() < 0.01, "{measured:?}");

    let stopped = program(source, 45);
    assert!(velocity(&stopped).is_none(), "stationary bar is sharp");
    assert_eq!(
        stopped.packed_bytes().unwrap(),
        program(&source.replace("motionBlur: \"auto\"", ""), 45)
            .packed_bytes()
            .unwrap()
    );

    let mut invalid = compile_motion(source).unwrap().artifact;
    let auto = invalid
        .nodes
        .iter_mut()
        .flat_map(|node| &mut node.styles)
        .find(|style| style.property == "motion-velocity-blur-auto")
        .unwrap();
    auto.value = StyleValue::Static {
        value: MotionValue::Bool(false),
    };
    assert!(invalid.validate().is_err());
    assert!(compile_motion(&source.replace("\"auto\"", "\"automatic\"")).is_err());
}

#[test]
fn auto_blur_includes_layout_and_ancestor_motion() {
    let source = r##"
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Flow(ctx) {
  return <Scene className="flex" style={{ width:640,height:360,backgroundColor:"#101010" }}>
    <View style={{ width:100+ctx.seconds*60,height:40,backgroundColor:"#101010" }}/>
    <View key="follower" style={{ width:40,height:40,backgroundColor:"#ffffff",motionBlur:"auto" }}/>
  </Scene>;
}
"##;
    let flow = velocity(&program(source, 10)).expect("flow shifts the follower");
    assert!((flow[0] - 2.0).abs() < 0.5, "{flow:?}");
    assert!(flow[1].abs() < 0.01, "{flow:?}");

    let ancestor = r##"
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function ParentMotion(ctx) {
  return <Scene style={{width:640,height:360,backgroundColor:"#101010"}}>
    <View key="parent" className="absolute"
      style={{left:0,top:80,width:40,height:40,translate:point(ctx.seconds*60,0)}}>
      <View key="follower" style={{width:40,height:40,backgroundColor:"#ffffff",motionBlur:"auto"}} />
    </View>
  </Scene>;
}
"##;
    let translated = velocity(&program(ancestor, 10)).expect("ancestor shifts the follower");
    assert!((translated[0] - 2.0).abs() < 0.5, "{translated:?}");
}

#[test]
fn auto_blur_suppresses_a_position_cut() {
    let source = r##"
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Cut(ctx) {
  return <Scene style={{width:640,height:360,backgroundColor:"#101010"}}>
    <View key="bar" className="absolute" style={{left:40,top:120,width:40,height:120,
      backgroundColor:"#ffffff",translate:point(ctx.seconds < 0.5 ? 0 : 100,0),motionBlur:"auto"}}/>
  </Scene>;
}
"##;
    assert!(velocity(&program(source, 15)).is_none());
}

#[test]
fn affine_translation_uses_screen_space_chain_rule_without_neighbor_layouts() {
    let source = r##"
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Affine(ctx) {
  return <Scene style={{width:640,height:360}}>
    <View key="parent" className="absolute" style={{
      left:100,top:100,width:40,height:40,rotate:"90deg",
      translate:point(ctx.seconds*60,0) }}>
      <View key="bar" style={{width:40,height:40,backgroundColor:"#ffffff",
        translate:point(0,ctx.seconds*30),motionBlur:"auto"}} />
    </View>
  </Scene>;
}
"##;
    let (affine_program, neighbors) = profiled_program(source, 10);
    assert_eq!(
        neighbors, 0,
        "affine translation should avoid neighbor layouts"
    );
    let measured = velocity(&affine_program).expect("combined parent and child translation");
    assert!((measured[0] - 1.0).abs() < 0.05, "{measured:?}");
    assert!(measured[1].abs() < 0.05, "{measured:?}");

    // Enabling geometry reuse for blur must preserve the complete program on random seeks.
    let artifact = compile_motion(source).unwrap().artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let fonts = Fonts::default();
    let cache = valle_motion::layout::LayoutCache::new(&prepared, &fonts)
        .expect("static geometry with auto blur can be reused");
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    for frame in [10, 20, 10] {
        let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let reused = cache
            .build_tree(&context, &props, Viewport::new((640, 360)), None)
            .unwrap();
        assert_eq!(
            emit(&reused, &default_font_naming)
                .unwrap()
                .program
                .packed_bytes()
                .unwrap(),
            program(source, frame).packed_bytes().unwrap(),
            "cached frame {frame} differs"
        );
    }

    let (nonlinear, neighbors) = profiled_program(
        include_str!("fixtures/motion/composition/auto-motion-blur.motion.tsx"),
        10,
    );
    assert_eq!(neighbors, 2, "nonlinear motion needs neighboring samples");
    assert!((velocity(&nonlinear).unwrap()[0] - 20.0).abs() < 0.5);
}
