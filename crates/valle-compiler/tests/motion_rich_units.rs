#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::layout::ResolvedUnit;
use valle_motion::{
    Fonts, LayoutOptions, NodeKind, Viewport, build_tree, motion_context_at_frame, prepare_scene,
    register_default_motion_fonts, resolve_props,
};
use valle_timeline::FrameRate;

const RICH_TEXT_UNITS: &str =
    include_str!("fixtures/motion/composition/rich-text-units.motion.tsx");

fn units(source: &str, frame: u32) -> (Vec<ResolvedUnit>, Vec<ResolvedUnit>) {
    let artifact = compile_motion(source).unwrap().artifact;
    artifact.validate().unwrap();
    let scene = prepare_scene(&artifact).unwrap();
    let mut fonts = Fonts::default();
    register_default_motion_fonts(&mut fonts).unwrap();
    let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let tree = build_tree(
        &scene,
        &ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    (
        tree.units["t/__run_0__"].clone(),
        tree.units["t/__run_1__"].clone(),
    )
}

#[test]
fn span_units_keep_a_single_character_clock() {
    let artifact = compile_motion(RICH_TEXT_UNITS).unwrap().artifact;
    artifact.validate().unwrap();
    let grouped = artifact
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Text {
                per_unit: Some(binding),
                ..
            } => Some(binding.group_key.as_deref()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(grouped, vec![Some("t"), Some("t")]);

    let (white, red) = units(RICH_TEXT_UNITS, 10);
    assert_eq!((white.len(), red.len()), (6, 5));
    assert_eq!((red[0].start, red[0].end), (0, 1));
    for (index, unit) in red.iter().enumerate() {
        let expected = ((index as f64 + 6.0) * 0.8 + 2.0).sin() * 10.0;
        assert!((unit.translate.unwrap().y - expected).abs() < 1e-6);
    }
    assert_eq!((white[5].start, white[5].end), (5, 6));
    let (_, later) = units(RICH_TEXT_UNITS, 20);
    let displacements = red
        .iter()
        .zip(&later)
        .map(|(before, after)| after.translate.unwrap().y - before.translate.unwrap().y)
        .collect::<Vec<_>>();
    let spread = displacements
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max)
        - displacements.iter().copied().fold(f64::INFINITY, f64::min);
    assert!(
        spread >= 4.0,
        "red letters need distinct phases: {displacements:?}"
    );
}

#[test]
fn rich_units_keep_global_utf8_offsets_and_words_cross_span_boundaries() {
    let chars = r##"export const composition={width:640,height:360,fps:30,duration:2};
export default function Units(ctx) { return <Scene><Text key="t" split="char"
  perUnit={{opacity:ctx.unit.start / 10}}>{"Aé"}<Span style={{color:"#ff0000"}}>中B</Span></Text></Scene>; }"##;
    let (plain, span) = units(chars, 0);
    assert_eq!(plain.len(), 2);
    assert_eq!(span.len(), 2);
    assert_eq!(span[0].opacity, Some(0.3));
    assert_eq!(span[1].opacity, Some(0.6));
    assert_eq!((span[0].start, span[0].end), (0, 3));
    assert_eq!((span[1].start, span[1].end), (3, 4));

    let words = r##"export const composition={width:640,height:360,fps:30,duration:2};
export default function Units(ctx) { return <Scene><Text key="t" split="word"
  perUnit={{opacity:ctx.unit.end / 10}}>{"hel"}<Span style={{color:"#ff0000"}}>lo</Span></Text></Scene>; }"##;
    let (plain, span) = units(words, 0);
    assert_eq!(plain.len(), 1);
    assert_eq!(span.len(), 1);
    assert_eq!((plain[0].start, plain[0].end), (0, 3));
    assert_eq!((span[0].start, span[0].end), (0, 2));
    assert_eq!(plain[0].opacity, Some(0.5));
    assert_eq!(span[0].opacity, Some(0.5));
}

#[test]
fn common_dynamic_interpolation_stops_shift_the_input_per_unit() {
    let source = r##"export const composition={width:640,height:360,fps:30,duration:2};
export default function Units(ctx) { return <Scene><Text key="t" split="char"
  perUnit={{opacity:interpolate(ctx.seconds,
    [ctx.unit.index * 0.1, 1 + ctx.unit.index * 0.1], [0, 1])}}>
  {"AB"}<Span style={{color:"#ff0000"}}>CD</Span></Text></Scene>; }"##;
    let direct = source.replace(
        "interpolate(ctx.seconds,\n    [ctx.unit.index * 0.1, 1 + ctx.unit.index * 0.1]",
        "interpolate(ctx.seconds - ctx.unit.index * 0.1, [0, 1]",
    );
    assert_ne!(source, direct);
    for frame in [0, 10, 20, 45] {
        assert_eq!(units(source, frame), units(&direct, frame), "frame {frame}");
    }
    let (plain, span) = units(source, 10);
    assert!(plain[0].opacity.unwrap() > plain[1].opacity.unwrap());
    assert!(span[0].opacity.unwrap() > span[1].opacity.unwrap());

    let unequal = source.replace("1 + ctx.unit.index * 0.1", "1 + ctx.unit.index * 0.2");
    let diagnostics = compile_motion(&unequal).unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("shared shift on every stop") }),
        "{diagnostics:?}"
    );
}

#[test]
fn dynamic_span_text_recomputes_the_shared_unit_count_per_frame() {
    let source = r##"export const composition={width:640,height:360,fps:30,duration:2};
export default function Units(ctx) { return <Scene><Text key="t" split="char"
  perUnit={{opacity:ctx.unit.count / 10}}>{"A"}<Span style={{color:"#ff0000"}}>
  {ctx.seconds < 0.5 ? "B" : "BC"}</Span></Text></Scene>; }"##;
    let (plain, span) = units(source, 0);
    assert_eq!((plain.len(), span.len()), (1, 1));
    assert_eq!(plain[0].opacity, Some(0.2));
    assert_eq!(span[0].opacity, Some(0.2));

    let (plain, span) = units(source, 30);
    assert_eq!((plain.len(), span.len()), (1, 2));
    assert_eq!(plain[0].opacity, Some(0.3));
    assert_eq!(span[0].opacity, Some(0.3));
    assert_eq!(span[1].opacity, Some(0.3));
}

#[test]
fn rich_unit_group_must_match_its_direct_styled_runs() {
    let mut artifact = compile_motion(RICH_TEXT_UNITS).unwrap().artifact;
    let run = artifact
        .nodes
        .iter_mut()
        .find(|node| node.key == "t/__run_0__")
        .unwrap();
    let NodeKind::Text {
        per_unit: Some(binding),
        ..
    } = &mut run.kind
    else {
        unreachable!()
    };
    binding.group_key = Some("missing".into());
    assert!(artifact.validate().is_err());

    let mut artifact = compile_motion(RICH_TEXT_UNITS).unwrap().artifact;
    let run = artifact
        .nodes
        .iter_mut()
        .find(|node| node.key == "t/__run_1__")
        .unwrap();
    let NodeKind::Text {
        per_unit: Some(binding),
        ..
    } = &mut run.kind
    else {
        unreachable!()
    };
    binding.group_key = None;
    assert!(artifact.validate().is_err());

    let mut artifact = compile_motion(RICH_TEXT_UNITS).unwrap().artifact;
    let group = artifact
        .nodes
        .iter_mut()
        .find(|node| node.key == "t")
        .unwrap();
    group.children.start = group.children.end + 1;
    assert!(artifact.validate().is_err());
}
