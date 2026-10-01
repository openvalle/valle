#![cfg(feature = "motion")]

use valle_compiler::motion::{MeasureEnv, compile_motion, compile_motion_with_env};
use valle_draw::program::{DrawProgram, Node};
use valle_motion::{
    Fonts, LayoutOptions, SceneArtifact, Viewport, build_tree, default_font_naming, emit,
    motion_context_at_frame, prepare_scene, register_default_motion_fonts, resolve_props,
};
use valle_timeline::FrameRate;

const VARIABLE_FONT_WEIGHT: &str =
    include_str!("fixtures/motion/composition/variable-font-weight.motion.tsx");

fn fonts() -> Fonts {
    let mut fonts = Fonts::default();
    register_default_motion_fonts(&mut fonts).unwrap();
    fonts
}

fn program(
    scene: &valle_motion::PreparedScene,
    artifact: &SceneArtifact,
    fonts: &Fonts,
    frame: u32,
) -> DrawProgram {
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    // This 17-frame test spans exactly 100..900, at 50-unit increments.
    let ctx = motion_context_at_frame(frame, 17, FrameRate::new(16, 1).unwrap()).unwrap();
    let tree = build_tree(
        scene,
        &ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts,
            styles: None,
        },
    )
    .unwrap();
    emit(&tree, &default_font_naming).unwrap().program
}

fn ink_width(program: &DrawProgram) -> f64 {
    let runs: Vec<_> = program
        .nodes()
        .iter()
        .filter_map(|n| match n {
            Node::GlyphRun(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(runs.len(), 1);
    assert!(
        runs[0].outline.is_some(),
        "variable glyphs must freeze the shaped outline for both backends"
    );
    runs[0].bounds.width
}

#[test]
fn default_pack_instances_continuous_weight_and_reuses_no_stale_glyphs() {
    let source = VARIABLE_FONT_WEIGHT.replace("ctx.progress", "ctx.seconds");
    let weight = compile_motion(&source).unwrap().artifact;
    let axis_source = source.replace(
        "fontWeight: 100 + ctx.seconds * 800",
        "fontVariationSettings: `\"wght\" ${100 + ctx.seconds * 800}`",
    );
    let axes = compile_motion(&axis_source).unwrap().artifact;
    let weight_scene = prepare_scene(&weight).unwrap();
    let axes_scene = prepare_scene(&axes).unwrap();
    let fonts = fonts();
    let outputs: Vec<_> = (0..17)
        .map(|frame| {
            let a = program(&weight_scene, &weight, &fonts, frame);
            let b = program(&axes_scene, &axes, &fonts, frame);
            assert_eq!(
                a.packed_bytes().unwrap(),
                b.packed_bytes().unwrap(),
                "weight vs explicit axis at {frame}"
            );
            a
        })
        .collect();
    let widths: Vec<_> = outputs.iter().map(ink_width).collect();
    assert!(widths.windows(2).all(|w| w[0] < w[1]), "{widths:?}");
    // Animated axes change glyph geometry, so whole-scene layout reuse must remain disabled.
    assert!(valle_motion::layout::LayoutCache::new(&weight_scene, &fonts).is_none());
    assert!(valle_motion::layout::LayoutCache::new(&axes_scene, &fonts).is_none());
    for frame in [16, 0, 8, 3, 15, 1, 8] {
        let warm = program(&weight_scene, &weight, &fonts, frame);
        let cold_scene = prepare_scene(&weight).unwrap();
        let mut cold_fonts = Fonts::default();
        register_default_motion_fonts(&mut cold_fonts).unwrap();
        let cold = program(&cold_scene, &weight, &cold_fonts, frame);
        assert_eq!(warm.packed_bytes().unwrap(), cold.packed_bytes().unwrap());
        assert_eq!(
            warm.packed_bytes().unwrap(),
            outputs[frame as usize].packed_bytes().unwrap()
        );
    }
}

#[test]
fn axes_work_in_rich_text_and_measurement_and_preserve_fractional_weights() {
    let fonts = fonts();
    let render = |body: &str| {
        let source = format!("export default function T(ctx) {{return {body};}}");
        let artifact = compile_motion(&source).unwrap().artifact;
        program(&prepare_scene(&artifact).unwrap(), &artifact, &fonts, 8)
    };
    let a = render(r#"<Text style={{fontSize:80,fontWeight:403.25}}>WEIGHT</Text>"#);
    let b = render(r#"<Text style={{fontSize:80,fontWeight:403.75}}>WEIGHT</Text>"#);
    assert_ne!(a.packed_bytes().unwrap(), b.packed_bytes().unwrap());
    let span = render(
        r#"<Text style={{fontSize:80}}><Span style={{fontVariationSettings:`"wght" ${100+ctx.seconds*800}, "wdth" 75`}}>WEIGHT</Span></Text>"#,
    );
    let direct = render(
        r#"<Text style={{fontSize:80,fontVariationSettings:'"wght" 500, "wdth" 75'}}>WEIGHT</Text>"#,
    );
    let overridden = render(
        r#"<Text style={{fontSize:80,fontWeight:900,fontVariationSettings:'"wght" 500, "wdth" 75'}}>WEIGHT</Text>"#,
    );
    assert_eq!(
        direct.packed_bytes().unwrap(),
        overridden.packed_bytes().unwrap()
    );
    assert_eq!(ink_width(&span), ink_width(&direct));
    assert!(ink_width(&direct) < ink_width(&a));

    let env = MeasureEnv::new(&[], (640, 360)).unwrap();
    let measured = compile_motion_with_env(r#"const m=measureText("WEIGHT", {fontSize:80,fontVariationSettings:'"wght" 500, "wdth" 75'});
        export default function T(){return <Text style={{fontSize:80,width:m.width,fontVariationSettings:'"wght" 500, "wdth" 75'}}>WEIGHT</Text>; }"#,&[],Some(&env)).unwrap().artifact;
    let actual = program(&prepare_scene(&measured).unwrap(), &measured, &fonts, 8);
    assert_eq!(ink_width(&actual), ink_width(&direct));
    for settings in ["42", "'invalid'", "'\"weight\" 500'", "'\"wght\" 1e999'"] {
        let source = format!(
            "const m=measureText('A',{{fontSize:80,fontVariationSettings:{settings}}}); export default function T(){{return <View style={{{{width:m.width}}}}/>;}}"
        );
        assert!(
            compile_motion_with_env(&source, &[], Some(&env)).is_err(),
            "{settings}"
        );
    }
}

#[test]
fn default_chinese_fallback_has_real_continuous_bold_outlines() {
    let source = "export default function Main(ctx) { return <Scene><Text style={{fontSize:64,fontWeight:400+ctx.seconds*400}}>中文</Text></Scene>; }";
    let artifact = compile_motion(source).unwrap().artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let fonts = fonts();
    let regular = program(&prepared, &artifact, &fonts, 0);
    let medium = program(&prepared, &artifact, &fonts, 8);
    let bold = program(&prepared, &artifact, &fonts, 16);
    for output in [&regular, &medium, &bold] {
        assert!(
            output
                .nodes()
                .iter()
                .any(|node| matches!(node,Node::GlyphRun(run) if run.outline.is_some()))
        );
    }
    assert_ne!(regular.paths(), medium.paths());
    assert_ne!(medium.paths(), bold.paths());
    assert_eq!(regular, program(&prepared, &artifact, &fonts, 0));
}
