#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_draw::program::{BlendMode, DrawProgram, Node};
use valle_motion::layout::build_tree_profiled;
use valle_motion::{
    Fonts, LayoutOptions, SHUTTER_CAPABILITY, Viewport, build_tree, default_font_naming, emit,
    motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::{FrameRate, RationalTime};

const SHUTTER_SAMPLING: &str =
    include_str!("fixtures/motion/composition/shutter-sampling.motion.tsx");
const SHUTTER_INSTANCES: &str =
    include_str!("fixtures/motion/composition/shutter-instances.motion.tsx");

fn program(source: &str, frame: u32) -> DrawProgram {
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

#[test]
fn shutter_accumulates_independent_subframes() {
    let artifact = compile_motion(SHUTTER_SAMPLING).unwrap().artifact;
    assert!(
        artifact
            .capability_set
            .names
            .iter()
            .any(|name| name == SHUTTER_CAPABILITY)
    );
    let frame20 = program(SHUTTER_SAMPLING, 20);
    let frame45 = program(SHUTTER_SAMPLING, 45);
    let exposure =
        |program: &DrawProgram| {
            let plus = program.nodes().iter().filter(|node| {
            matches!(node, Node::Group(group) if group.internal_blend == BlendMode::Plus)
        }).count();
            let weighted = program.nodes().iter().filter(|node| {
            matches!(node, Node::Group(group) if (group.opacity - 0.125).abs() < 1e-6)
        }).count();
            (plus, weighted)
        };
    assert_eq!(exposure(&frame20), (8, 8));
    assert_eq!(exposure(&frame45), (8, 8));
    assert_eq!(
        frame20.packed_bytes().unwrap(),
        program(SHUTTER_SAMPLING, 20).packed_bytes().unwrap()
    );
    let no_shutter = SHUTTER_SAMPLING
        .replace(
            "<Shutter key=\"shutter\" samples={8} angle={180}>",
            "<Group>",
        )
        .replace("</Shutter>", "</Group>");
    assert_ne!(
        frame20.packed_bytes().unwrap(),
        program(&no_shutter, 20).packed_bytes().unwrap()
    );
}

#[test]
fn shutter_samples_are_exact_output_times() {
    let artifact = compile_motion(SHUTTER_SAMPLING).unwrap().artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let window = prepared.dependencies().temporal_window();
    assert_eq!(window.past_frames, RationalTime::new(7, 32).unwrap());
    assert_eq!(window.future_frames, RationalTime::new(7, 32).unwrap());
    let with_auto_blur = SHUTTER_SAMPLING.replace(
        "backgroundColor: \"#ffffff\",",
        "backgroundColor: \"#ffffff\", motionBlur: \"auto\",",
    );
    let combined = compile_motion(&with_auto_blur).unwrap().artifact;
    let window = prepare_scene(&combined)
        .unwrap()
        .dependencies()
        .temporal_window();
    assert_eq!(window.past_frames, RationalTime::new(23, 32).unwrap());
    assert_eq!(window.future_frames, RationalTime::new(23, 32).unwrap());
    let context = motion_context_at_frame(20, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let (tree, timings) = build_tree_profiled(
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
    assert!(timings.sample_static_computed > 0);
    assert!(timings.sample_static_reused > timings.sample_static_computed);
    let times = tree.shutter_sample_times("shutter").unwrap();
    assert_eq!(times.len(), 8);
    for (index, time) in times.iter().enumerate() {
        assert_eq!(
            time.composition(),
            RationalTime::new(633 + 2 * index as i64, 960).unwrap()
        );
    }
}

#[test]
fn shutter_reuses_static_instance_template_values() {
    let artifact = compile_motion(SHUTTER_INSTANCES).unwrap().artifact;
    assert_eq!(artifact.instance_groups.len(), 1);
    let expanded = SHUTTER_INSTANCES.replace("className=\"absolute\"", "className=\"absolute \"");
    assert!(
        compile_motion(&expanded)
            .unwrap()
            .artifact
            .instance_groups
            .is_empty()
    );
    let prepared = prepare_scene(&artifact).unwrap();
    let context = motion_context_at_frame(20, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let options = LayoutOptions {
        viewport: Viewport::new((320, 120)),
        fonts: &fonts,
        styles: None,
    };
    let (tree, timings) = build_tree_profiled(&prepared, &context, &props, &options).unwrap();
    assert!(timings.instance_static_computed > 0);
    assert!(timings.instance_static_reused > 0);
    assert_eq!(tree.shutter_sample_times("shutter").unwrap().len(), 4);
    let later = motion_context_at_frame(21, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    build_tree(&prepared, &later, &props, &options).unwrap();
    let repeated = build_tree(&prepared, &context, &props, &options).unwrap();
    assert_eq!(
        emit(&tree, &default_font_naming)
            .unwrap()
            .program
            .packed_bytes()
            .unwrap(),
        emit(&repeated, &default_font_naming)
            .unwrap()
            .program
            .packed_bytes()
            .unwrap()
    );
}

#[test]
fn shutter_admission_rejects_invalid_samples_and_angle() {
    assert!(compile_motion(&SHUTTER_SAMPLING.replace("samples={8}", "samples={0}")).is_err());
    assert!(compile_motion(&SHUTTER_SAMPLING.replace("samples={8}", "samples={33}")).is_err());
    assert!(compile_motion(&SHUTTER_SAMPLING.replace("angle={180}", "angle={361}")).is_err());
    let mut invalid = compile_motion(SHUTTER_SAMPLING).unwrap().artifact;
    let shutter = invalid
        .nodes
        .iter_mut()
        .find(|node| node.key == "shutter")
        .unwrap();
    shutter.kind = valle_motion::NodeKind::Shutter {
        samples: 0,
        angle_degrees: 180,
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn shutter_prunes_unrelated_absolute_layout_and_preserves_pixels_and_bounds_readers() {
    let background = (0..1600).map(|i| format!(r##"<View key="static-{i}" className="absolute" style={{{{left:{},top:{},width:4,height:4,backgroundColor:"#456",opacity:0.5+ctx.progress*0.5}}}}/>"##,i%160*4,i/160*4)).collect::<String>();
    let source = format!(
        r##"export const composition={{width:640,height:360,fps:30,duration:2}};
        export default function SceneMain(ctx) {{return <Scene style={{{{width:640,height:360}}}}>{background}
        <Shutter key="shutter" samples={{16}} angle={{180}}><View key="moving" style={{{{position:"absolute",left:ctx.seconds*100,top:80,width:24,height:24,backgroundColor:"white"}}}}/></Shutter></Scene>}}"##
    );
    let artifact = compile_motion(&source).unwrap().artifact;
    let scene = prepare_scene(&artifact).unwrap();
    let ctx = motion_context_at_frame(20, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let opts = LayoutOptions {
        viewport: Viewport::new((640, 360)),
        fonts: &fonts,
        styles: None,
    };
    let (_, timing) = build_tree_profiled(&scene, &ctx, &props, &opts).unwrap();
    assert_eq!(timing.temporal_layout_nodes, 3);
    // The redundant position declaration conservatively disables pruning without changing CSS.
    let complete = source.replace(
        "className=\"absolute\" style={{",
        "className=\"absolute\" style={{position:\"absolute\",",
    );
    assert_eq!(
        program(&source, 20).packed_bytes().unwrap(),
        program(&complete, 20).packed_bytes().unwrap()
    );
    let with_reader = source.replace(
        "left:ctx.seconds*100,top:80",
        "left:0,top:0,translate:point(ctx.seconds*100+bounds(\"static-7\").width,80)",
    );
    let artifact = compile_motion(&with_reader).unwrap().artifact;
    let (_, timing) =
        build_tree_profiled(&prepare_scene(&artifact).unwrap(), &ctx, &props, &opts).unwrap();
    assert_eq!(timing.temporal_layout_nodes, 4);
}
