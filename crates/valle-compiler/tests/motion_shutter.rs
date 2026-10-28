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
    // Each subframe blurs over a quarter of a frame on each side of its own time.
    assert_eq!(window.past_frames, RationalTime::new(15, 32).unwrap());
    assert_eq!(window.future_frames, RationalTime::new(15, 32).unwrap());
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
fn temporal_layout_reuses_proven_invariant_samples_without_folding_exposure() {
    for (open, close, expected) in [
        (
            "<Shutter key=\"sample\" samples={16} angle={180}>",
            "</Shutter>",
            16,
        ),
        (
            "<Echo key=\"sample\" count={8} delay={0.02} decay={0.8}>",
            "</Echo>",
            8,
        ),
    ] {
        let source = format!(
            r##"
export const composition = {{ width: 96, height: 64, fps: 30, duration: 2 }};
export default function Demo() {{ const time = useCurrentTime();
  return <Scene style={{{{ width: 96, height: 64 }}}}>
    {open}<View key="content" style={{{{ opacity: 1, width: 48, height: 32, backgroundColor: "#88442280" }}}}>
      <View style={{{{ position: "absolute", left: 12, top: 4, width: 32, height: 16, backgroundColor: "#3355aa80" }}}} />
    </View>{close}
  </Scene>;
}}
"##
        );
        // Equal paint values with an explicit clock dependency force independent sample trees.
        // This is an independent full-layout oracle, not a product optimization toggle.
        let reference = source.replace("opacity: 1,", "opacity: 1 + time.seconds * 0,");
        let prepare = |source: &str| {
            let artifact = compile_motion(source).unwrap().artifact;
            let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
            (prepare_scene(&artifact).unwrap(), props)
        };
        let (prepared, props) = prepare(&source);
        let (full, full_props) = prepare(&reference);
        let fonts = Fonts::default();
        let options = LayoutOptions {
            viewport: Viewport::new((96, 64)),
            fonts: &fonts,
            styles: None,
        };
        for frame in [30, 0, 59, 15, 30] {
            let context =
                motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
            let (actual, stats) =
                build_tree_profiled(&prepared, &context, &props, &options).unwrap();
            let (reference, full_stats) =
                build_tree_profiled(&full, &context, &full_props, &options).unwrap();
            assert_eq!(stats.temporal_sample_trees, 1);
            // At clip boundaries the existing clock already coalesces clamped sample times.
            let times = reference
                .shutter_sample_times("sample")
                .or_else(|| reference.echo_sample_times("sample"))
                .unwrap();
            assert_eq!(times.len(), expected);
            let unique = times
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
                .len();
            assert_eq!(full_stats.temporal_sample_trees, unique);
            assert_eq!(
                actual.shutter_sample_times("sample"),
                reference.shutter_sample_times("sample")
            );
            assert_eq!(
                actual.echo_sample_times("sample"),
                reference.echo_sample_times("sample")
            );
            let actual = emit(&actual, &default_font_naming).unwrap();
            let reference = emit(&reference, &default_font_naming).unwrap();
            assert_eq!(actual.temporal_recordings.built, 1);
            assert_eq!(actual.temporal_recordings.reused, expected - 1);
            assert!(actual.temporal_recordings.commands_replayed > 0);
            assert_eq!(reference.temporal_recordings.built, expected);
            assert_eq!(reference.temporal_recordings.reused, 0);
            assert_eq!(
                actual.program.packed_bytes().unwrap(),
                reference.program.packed_bytes().unwrap()
            );
        }
    }
}

#[test]
fn temporal_recording_reuses_side_tables_but_preserves_grain_accounting() {
    let mut fonts = Fonts::default();
    let font = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/fonts/noto/NotoSans-Regular.ttf"),
    )
    .unwrap();
    fonts
        .register(valle_motion::FontResource::new(font.as_slice()))
        .unwrap();
    for (content, pure) in [
        (
            r##"<Text style={{fontFamily:"Noto Sans",fontSize:14,color:"#aabbcc80"}}>Sample glyphs</Text>"##,
            true,
        ),
        (
            r##"<View style={{width:40,height:24,borderRadius:4,border:"2px solid #aa3311",backgroundImage:"linear-gradient(90deg, #3377bb80, #ddaa3380)",filter:"blur(2px)",boxShadow:"1px 2px 3px #22334480"}} />"##,
            true,
        ),
        (
            r##"<View style={{width:48,height:32,backgroundColor:"#aabbcc",paperGrain:0.5}} />"##,
            false,
        ),
    ] {
        let source = format!(
            r##"
export const composition = {{width:160,height:90,fps:30,duration:2}};
export default function Demo() {{ const time = useCurrentTime();
  return <Scene style={{{{width:160,height:90,opacity:1}}}}>
    <View style={{{{position:"absolute",left:3.5,top:4,width:64,height:48,rotate:"8deg"}}}}>
      <Shutter samples={{4}} angle={{180}}>{content}</Shutter>
    </View>
    <View style={{{{position:"absolute",left:84,top:12,width:64,height:48}}}}>
      <Echo count={{4}} delay={{0.02}} decay={{0.8}}>{content}</Echo>
    </View>
  </Scene>;
}}
"##
        );
        let evaluate = |source: &str| {
            let artifact = compile_motion(source).unwrap().artifact;
            let prepared = prepare_scene(&artifact).unwrap();
            let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
            let context = motion_context_at_frame(20, 60, FrameRate::new(30, 1).unwrap()).unwrap();
            let tree = build_tree(
                &prepared,
                &context,
                &props,
                &LayoutOptions {
                    viewport: Viewport::new((160, 90)),
                    fonts: &fonts,
                    styles: None,
                },
            )
            .unwrap();
            emit(&tree, &default_font_naming).unwrap()
        };
        let actual = evaluate(&source);
        let reference = evaluate(&source.replace("opacity:1", "opacity:1+time.seconds*0"));
        assert_eq!(
            actual.temporal_recordings.built,
            if pure { 2 } else { 8 },
            "{content}"
        );
        assert_eq!(
            actual.temporal_recordings.reused,
            if pure { 6 } else { 0 },
            "{content}"
        );
        assert_eq!(
            actual.program.packed_bytes().unwrap(),
            reference.program.packed_bytes().unwrap(),
            "{content}"
        );
        assert_eq!(actual.unsupported, reference.unsupported);
    }
}

#[test]
fn temporal_layout_invariance_rejects_clock_values_even_when_only_paint_changes() {
    for value in ["time.seconds", "time.clipSeconds", "time.progress"] {
        let source = format!(
            r##"
export const composition = {{ width: 96, height: 64, fps: 30, duration: 2 }};
export default function Demo() {{ const time = useCurrentTime();
  return <Scene><Shutter key="sample" samples={{8}} angle={{180}}>
    <View style={{{{ width: 48, height: 32, opacity: {value}, backgroundColor: "#884422" }}}} />
  </Shutter></Scene>;
}}
"##
        );
        let artifact = compile_motion(&source).unwrap().artifact;
        let prepared = prepare_scene(&artifact).unwrap();
        let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
        let fonts = Fonts::default();
        let context = motion_context_at_frame(15, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let (_, stats) = build_tree_profiled(
            &prepared,
            &context,
            &props,
            &LayoutOptions {
                viewport: Viewport::new((96, 64)),
                fonts: &fonts,
                styles: None,
            },
        )
        .unwrap();
        assert_eq!(stats.temporal_sample_trees, 8, "{value}");
    }
}

#[test]
fn temporal_layout_sharing_never_retains_props_or_viewport_across_requests() {
    use std::collections::BTreeMap;
    let source = r##"
export const controls = ({props:{size:number({default:30,min:1,max:100})}});
export const composition = {width:96,height:64,fps:30,duration:2};
export default function Demo(props) { const time = useCurrentTime();
  return <Scene style={{width:"100%",height:"100%"}}><Shutter key="sample" samples={8} angle={180}>
    <View style={{width:props.size,height:24,opacity:1,backgroundColor:"#88442280"}} />
  </Shutter></Scene>;
}
"##;
    let artifact = compile_motion(source).unwrap().artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let full = prepare_scene(
        &compile_motion(&source.replace("opacity:1", "opacity:1+time.seconds*0"))
            .unwrap()
            .artifact,
    )
    .unwrap();
    let context = motion_context_at_frame(20, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let fonts = Fonts::default();
    let mut previous = None;
    for (size, width) in [(20., 96), (40., 192), (20., 96)] {
        let props = resolve_props(
            &artifact.controls,
            &BTreeMap::from([("size".into(), valle_motion::MotionValue::Number(size))]),
        )
        .unwrap();
        let options = LayoutOptions {
            viewport: Viewport::new((width, 64)),
            fonts: &fonts,
            styles: None,
        };
        let (actual, stats) = build_tree_profiled(&prepared, &context, &props, &options).unwrap();
        let reference = build_tree(&full, &context, &props, &options).unwrap();
        assert_eq!(stats.temporal_sample_trees, 1);
        let bytes = emit(&actual, &default_font_naming)
            .unwrap()
            .program
            .packed_bytes()
            .unwrap();
        assert_eq!(
            bytes,
            emit(&reference, &default_font_naming)
                .unwrap()
                .program
                .packed_bytes()
                .unwrap()
        );
        if let Some(previous) = previous {
            assert_ne!(bytes, previous);
        }
        previous = Some(bytes);
    }
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
fn shutter_prunes_unrelated_layout_beside_backdrop_readers_and_preserves_pixels() {
    let background = (0..400).map(|i| format!(r##"<View key="static-{i}" className="absolute" style={{{{left:{},top:{},width:6,height:6,backgroundColor:"#456",mixBlendMode:"{}"}}}}/>"##,i%40*16,i/40*16,if i%2==0 {"multiply"} else {"normal"})).collect::<String>();
    let source = format!(
        r##"export const composition={{width:640,height:360,fps:30,duration:2}};
        export default function SceneMain() {{ const time = useCurrentTime();return <Scene style={{{{width:640,height:360,backgroundColor:"#203040"}}}}>{background}
        <Shutter key="shutter" samples={{8}} angle={{180}}><View key="moving" style={{{{position:"absolute",left:time.seconds*100,top:80,width:24,height:24,backgroundColor:"white",mixBlendMode:"screen"}}}}/></Shutter></Scene>}}"##
    );
    let artifact = compile_motion(&source).unwrap().artifact;
    let ctx = motion_context_at_frame(20, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let opts = LayoutOptions {
        viewport: Viewport::new((640, 360)),
        fonts: &fonts,
        styles: None,
    };
    let (_, timing) =
        build_tree_profiled(&prepare_scene(&artifact).unwrap(), &ctx, &props, &opts).unwrap();
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
}

#[test]
fn shutter_prunes_unrelated_absolute_layout_and_preserves_pixels_and_bounds_readers() {
    let background = (0..1600).map(|i| format!(r##"<View key="static-{i}" className="absolute" style={{{{left:{},top:{},width:4,height:4,backgroundColor:"#456",opacity:0.5+time.progress*0.5}}}}/>"##,i%160*4,i/160*4)).collect::<String>();
    let source = format!(
        r##"export const composition={{width:640,height:360,fps:30,duration:2}};
        export default function SceneMain() {{ const time = useCurrentTime();return <Scene style={{{{width:640,height:360}}}}>{background}
        <Shutter key="shutter" samples={{16}} angle={{180}}><View key="moving" style={{{{position:"absolute",left:time.seconds*100,top:80,width:24,height:24,backgroundColor:"white"}}}}/></Shutter></Scene>}}"##
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
        "left:time.seconds*100,top:80",
        "left:0,top:0,translate:point(time.seconds*100+bounds(\"static-7\").width,80)",
    );
    let artifact = compile_motion(&with_reader).unwrap().artifact;
    let (_, timing) =
        build_tree_profiled(&prepare_scene(&artifact).unwrap(), &ctx, &props, &opts).unwrap();
    assert_eq!(timing.temporal_layout_nodes, 4);
}

#[test]
fn shutter_prunes_absolute_branches_with_other_classes_and_preserves_pixels() {
    // A flow layout inside an absolute container with ordinary classes stays out of the samples.
    let cells = (0..400).map(|i| format!(r##"<View key="cell-{i}" style={{{{width:6,height:6,margin:1,backgroundColor:"#456"}}}}/>"##)).collect::<String>();
    let source = format!(
        r##"export const composition={{width:640,height:360,fps:30,duration:2}};
        export default function SceneMain() {{ const time = useCurrentTime();return <Scene style={{{{width:640,height:360,backgroundColor:"#203040"}}}}>
        <View key="panel" className="absolute inset-0 flex flex-wrap" style={{{{padding:4}}}}>{cells}</View>
        <Shutter key="shutter" samples={{8}} angle={{180}}><View key="moving" style={{{{position:"absolute",left:time.seconds*100,top:80,width:24,height:24,backgroundColor:"white"}}}}/></Shutter></Scene>}}"##
    );
    let sampled_nodes = |source: &str| {
        let artifact = compile_motion(source).unwrap().artifact;
        let ctx = motion_context_at_frame(20, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
        let fonts = Fonts::default();
        let opts = LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        };
        build_tree_profiled(&prepare_scene(&artifact).unwrap(), &ctx, &props, &opts)
            .unwrap()
            .1
            .temporal_layout_nodes
    };
    assert_eq!(sampled_nodes(&source), 3);
    // A redundant position declaration keeps the panel without changing CSS, and so does a class
    // that could put it back in the flow.
    let kept = source.replace(
        "style={{padding:4}}",
        "style={{position:\"absolute\",padding:4}}",
    );
    assert_eq!(sampled_nodes(&kept), 404);
    assert_eq!(
        sampled_nodes(&source.replace("absolute inset-0", "absolute relative inset-0")),
        404
    );
    assert_eq!(
        program(&source, 20).packed_bytes().unwrap(),
        program(&kept, 20).packed_bytes().unwrap()
    );
}
