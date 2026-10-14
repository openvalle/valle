#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_draw::program::{DrawProgram, Node};
use valle_motion::layout::build_tree_profiled;
use valle_motion::{
    ECHO_CAPABILITY, Fonts, LayoutOptions, NodeKind, Viewport, default_font_naming, emit,
    motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::{FrameRate, RationalTime};

const ECHO_TRAILS: &str = include_str!("fixtures/motion/composition/echo-trails.motion.tsx");

fn tree(source: &str, frame: u32) -> valle_motion::LayoutTree {
    let (tree, timings, has_echo) = profiled_tree(source, frame);
    if has_echo {
        assert!(timings.sample_static_computed > 0);
        assert!(timings.sample_static_reused > 0);
    }
    tree
}

fn profiled_tree(
    source: &str,
    frame: u32,
) -> (
    valle_motion::LayoutTree,
    valle_motion::layout::LayoutTimings,
    bool,
) {
    let artifact = compile_motion(source).unwrap().artifact;
    let has_echo = artifact
        .nodes
        .iter()
        .any(|node| matches!(node.kind, NodeKind::Echo { .. }));
    let prepared = prepare_scene(&artifact).unwrap();
    let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
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
    (tree, timings, has_echo)
}

fn program(source: &str, frame: u32) -> DrawProgram {
    emit(&tree(source, frame), &default_font_naming)
        .unwrap()
        .program
}

#[test]
fn echo_samples_past_frames_and_decays() {
    let artifact = compile_motion(ECHO_TRAILS).unwrap().artifact;
    let window = prepare_scene(&artifact)
        .unwrap()
        .dependencies()
        .temporal_window();
    assert_eq!(window.past_frames, RationalTime::new(12, 1).unwrap());
    assert_eq!(window.future_frames, RationalTime::ZERO);
    assert!(
        artifact
            .capability_set
            .names
            .iter()
            .any(|name| name == ECHO_CAPABILITY)
    );
    let frame30 = tree(ECHO_TRAILS, 30);
    let times = frame30.echo_sample_times("echo").unwrap();
    assert_eq!(times.len(), 4);
    for (time, frame) in times.iter().zip([18, 21, 24, 27]) {
        assert_eq!(time.composition(), RationalTime::new(frame, 30).unwrap());
    }
    let p30 = emit(&frame30, &default_font_naming).unwrap().program;
    let opacities = p30
        .nodes()
        .iter()
        .filter_map(|node| match node {
            Node::Group(group) if group.opacity < 1.0 => Some(group.opacity),
            _ => None,
        })
        .collect::<Vec<_>>();
    for opacity in [0.0625, 0.125, 0.25, 0.5] {
        assert!(
            opacities
                .iter()
                .any(|actual| (actual - opacity).abs() < 1e-6)
        );
    }
    assert_eq!(
        p30.packed_bytes().unwrap(),
        program(ECHO_TRAILS, 30).packed_bytes().unwrap()
    );
    let frame50 = tree(ECHO_TRAILS, 50);
    let times = frame50.echo_sample_times("echo").unwrap();
    for (time, frame) in times.iter().zip([38, 41, 44, 47]) {
        assert_eq!(time.composition(), RationalTime::new(frame, 30).unwrap());
    }
}

#[test]
fn echo_admission_rejects_invalid_parameters_and_nested_sampling() {
    assert!(compile_motion(&ECHO_TRAILS.replace("count={4}", "count={0}")).is_err());
    assert!(compile_motion(&ECHO_TRAILS.replace("interval={3}", "interval={0}")).is_err());
    assert!(compile_motion(&ECHO_TRAILS.replace("decay={0.5}", "decay={1.5}")).is_err());
    assert!(
        compile_motion(&ECHO_TRAILS.replace(
            "<Echo key=\"echo\"",
            "<Echo key=\"echo\" className=\"relative\""
        ))
        .is_err()
    );
    let mut invalid = compile_motion(ECHO_TRAILS).unwrap().artifact;
    let echo = invalid
        .nodes
        .iter_mut()
        .find(|node| node.key == "echo")
        .unwrap();
    echo.kind = valle_motion::NodeKind::Echo {
        count: 4,
        interval_frames: 0,
        decay: 0.5,
    };
    assert!(invalid.validate().is_err());
    let nested = ECHO_TRAILS
        .replace(
            "<View key=\"dot\"",
            "<Shutter key=\"nested\" samples={8} angle={180}><View key=\"dot\"",
        )
        .replace("</Echo>", "</Shutter></Echo>");
    assert!(compile_motion(&nested).is_err());
}

/// Stacking contexts drop the subtree of an invisible node, so an Echo below a hidden ancestor
/// paints no sample. The frame skips their layouts unless batches and paper grain coexist, where
/// skipped samples would change paper grain's share of the display-instance budget.
#[test]
fn echo_prunes_unrelated_layout_beside_backdrop_readers_and_preserves_pixels() {
    let background = (0..400)
        .map(|i| {
            format!(
                r##"<View key="static-{i}" className="absolute" style={{{{left:{},top:{},width:6,height:6,backgroundColor:"#456",mixBlendMode:"{}"}}}}/>"##,
                i % 40 * 16,
                i / 40 * 16,
                if i % 2 == 0 { "multiply" } else { "normal" }
            )
        })
        .collect::<String>();
    let source = |extra: &str| {
        format!(
            r##"export const composition={{width:640,height:360,fps:30,duration:2}};
export default function SceneMain(ctx) {{return <Scene style={{{{width:640,height:360,backgroundColor:"#203040"}}}}>{background}{extra}
<Echo key="echo" count={{4}} interval={{2}} decay={{0.6}}>
  <View key="moving" style={{{{position:"absolute",left:ctx.seconds*100,top:80,width:24,height:24,backgroundColor:"#f80"}}}}/>
  <View key="screen" style={{{{position:"absolute",left:ctx.seconds*120,top:90,width:30,height:12,backgroundColor:"#0af",mixBlendMode:"screen"}}}}/>
</Echo></Scene>}}"##
        )
    };
    // The redundant position declaration conservatively disables pruning without changing CSS.
    let complete = |source: String| {
        source.replace(
            "className=\"absolute\" style={{",
            "className=\"absolute\" style={{position:\"absolute\",",
        )
    };
    let pruned = source("");
    let (_, timings, _) = profiled_tree(&pruned, 20);
    assert_eq!(timings.temporal_layout_nodes, 4);
    let (_, timings, _) = profiled_tree(&complete(pruned.clone()), 20);
    assert!(timings.temporal_layout_nodes > 400);
    assert_eq!(
        program(&pruned, 20).packed_bytes().unwrap(),
        program(&complete(pruned), 20).packed_bytes().unwrap()
    );

    // Batches with paper grain take the display remainder the sample trees leave, so their
    // scenes keep full sample trees around backdrop readers.
    let reserved = source(
        r##"<GeometryBatch key="batch" geometry="circle" positions={[point(10, 10), point(30, 10)]}
        sizes={4} fills="#ff0000" style={{ position: "absolute", left: 0, top: 0, width: 640, height: 360 }} />
      <View key="paper" className="absolute" style={{ left: 0, top: 0, width: 8, height: 8, paperGrain: 0.5 }} />"##,
    );
    let (_, timings, _) = profiled_tree(&reserved, 20);
    assert!(timings.temporal_layout_nodes > 400);
}

#[test]
fn echo_below_a_hidden_ancestor_skips_its_unpainted_samples() {
    let source = |extra: &str| {
        format!(
            r##"export const composition = {{ width: 640, height: 360, fps: 30, duration: 2 }};
export default function HiddenEcho(ctx) {{
  return (
    <Scene className="relative h-full w-full" style={{{{ backgroundColor: "#101010" }}}}>
      <View key="gate" className="absolute" visible={{ctx.localFrame < 20}}
        style={{{{ left: 0, top: 0, width: 640, height: 360 }}}}>
        <Echo key="echo" count={{4}} interval={{3}} decay={{0.5}}>
          <View key="dot" className="absolute" style={{{{ left: 50, top: 170, width: 20, height: 20,
            backgroundColor: "#ffffff", translate: point(ctx.localFrame * 10, 0) }}}} />
        </Echo>
      </View>
      {extra}
    </Scene>
  );
}}"##
        )
    };
    let echo_layers = |program: &DrawProgram| {
        program
            .nodes()
            .iter()
            .filter(|node| matches!(node, Node::Group(group) if group.opacity < 1.0))
            .count()
    };
    let plain = source("");
    let (visible, _, _) = profiled_tree(&plain, 19);
    assert_eq!(visible.echo_sample_times("echo").unwrap().len(), 4);
    let (hidden, _, _) = profiled_tree(&plain, 40);
    assert!(hidden.echo_sample_times("echo").is_none());
    let empty = source("").replace(
        r#"<Echo key="echo" count={4} interval={3} decay={0.5}>"#,
        r#"<View key="echo" className="absolute" style={{ left: 0, top: 0, width: 640, height: 360 }}>"#,
    ).replace("</Echo>", "</View>");
    assert_eq!(
        emit(&hidden, &default_font_naming)
            .unwrap()
            .program
            .packed_bytes()
            .unwrap(),
        program(&empty, 40).packed_bytes().unwrap()
    );

    // With both a batch and paper grain the samples are still resolved, and still never painted.
    let reserved = source(
        r##"<GeometryBatch key="batch" geometry="circle" positions={[point(10, 10), point(30, 10)]}
        sizes={4} fills="#ff0000" style={{ position: "absolute", left: 0, top: 0, width: 640, height: 360 }} />
      <View key="paper" className="absolute" style={{ left: 0, top: 0, width: 8, height: 8, paperGrain: 0 }} />"##,
    );
    let (visible, _, _) = profiled_tree(&reserved, 19);
    assert!(echo_layers(&emit(&visible, &default_font_naming).unwrap().program) >= 4);
    let (hidden, _, _) = profiled_tree(&reserved, 40);
    assert_eq!(hidden.echo_sample_times("echo").unwrap().len(), 4);
    assert_eq!(
        echo_layers(&emit(&hidden, &default_font_naming).unwrap().program),
        0
    );
}
