#![cfg(feature = "motion")]

use std::{collections::BTreeMap, sync::Arc};
use valle_compiler::motion::compile_motion;
use valle_motion::layout::PaintProgramCache;
use valle_motion::{FontResource, Fonts, LayoutOptions, MotionValue, StyleCache, Viewport};
use valle_timeline::FrameRate;

fn fonts() -> Fonts {
    let mut fonts = Fonts::default();
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/fonts/noto/NotoSans-Regular.ttf"),
    )
    .unwrap();
    fonts.register(FontResource::new(bytes.as_slice())).unwrap();
    fonts
}

fn source(color: &str, extra: &str) -> String {
    format!(
        r##"
export const controls=({{props:{{size:number({{default:80,min:1,max:300}})}}}});
export default function Demo(props) {{const t=useCurrentTime();return <Scene style={{{{width:640,height:360,backgroundColor:'#111827',color:'#eeeeee'}}}}>
<View key="dynamic" style={{{{width:props.size,height:70,backgroundColor:{color},borderRadius:8}}}} />
{extra}
<Text style={{{{fontSize:18}}}}>Fixed geometry</Text>
</Scene>}}
"##
    )
}

fn check(source: &str, require_hit: bool) {
    let artifact = compile_motion(source).unwrap().artifact;
    let scene = valle_motion::prepare_scene(&artifact).unwrap();
    let fonts = fonts();
    let styles = StyleCache::new();
    let mut cache = PaintProgramCache::new(&scene, &fonts).expect("admitted scene");
    let catalog = |_: &str| None;
    let mut hits = 0;
    // Change layout inputs, seek backwards, clear state, and cross color/alpha intern guards.
    for (i, frame) in [0, 30, 59, 5, 30, 0, 59, 17, 3, 59, 30, 0]
        .into_iter()
        .enumerate()
    {
        if i == 7 {
            cache.clear();
        }
        let props = valle_motion::resolve_props(
            &artifact.controls,
            &BTreeMap::from([(
                "size".to_owned(),
                MotionValue::Number(if i < 9 { 80. } else { 95. }),
            )]),
        )
        .unwrap();
        let viewport =
            Viewport::new((640, 360)).with_device_pixel_ratio(if i == 6 { 2. } else { 1. });
        let ctx = valle_motion::motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap())
            .unwrap();
        let density = valle_motion::emit::RasterDensity {
            scale: if i == 8 { 0.5 } else { 1. },
            ..Default::default()
        };
        let clip = (i == 5).then(|| valle_draw::Rect::new(0., 0., 200., 200.));
        for compact in [false, true] {
            valle_draw::program::with_instance_compaction(compact, || {
                // Two calls also verify exact repeated-frame reuse with the current build policy.
                for _ in 0..2 {
                    let actual = cache
                        .prepare(
                            &ctx,
                            &props,
                            viewport,
                            Some(&styles),
                            None,
                            &catalog,
                            clip,
                            density,
                        )
                        .unwrap();
                    hits += usize::from(actual.stats.reused_template);
                    let tree = valle_motion::build_tree(
                        &scene,
                        &ctx,
                        &props,
                        &LayoutOptions {
                            viewport,
                            fonts: &fonts,
                            styles: Some(&styles),
                        },
                    )
                    .unwrap();
                    let expected = valle_motion::emit::emit_program_at_density(
                        &tree,
                        &valle_motion::default_font_naming,
                        None,
                        &catalog,
                        clip,
                        density,
                    )
                    .unwrap();
                    assert_eq!(
                        actual.program.packed_bytes().unwrap(),
                        expected.program.packed_bytes().unwrap(),
                        "frame={frame} compact={compact}"
                    );
                    assert_eq!(
                        *actual.layout_boxes,
                        valle_motion::layout_boxes(&artifact, &tree).unwrap()
                    );
                    actual.program.validate().unwrap();
                }
            });
        }
    }
    if require_hit {
        assert!(hits > 0);
    }
}

#[test]
fn template_matches_full_preparation_across_seeks_and_layout_inputs() {
    check(
        &source(
            r##"interpolate(t.seconds,[0,2],["#aa3520","#207bbb"])"##,
            "",
        ),
        true,
    );
}

#[test]
fn paint_sharing_and_zero_alpha_select_the_exact_canonical_program() {
    for color in [
        r##"interpolate(t.seconds,[0,1,2],["#111827","#207bbb","#111827"])"##,
        r##"interpolate(t.seconds,[0,1,2],["#aa352000","#207bbb","#aa352000"])"##,
        r##"interpolate(t.seconds,[0,2],["#ff000001","#ff000000"])"##,
    ] {
        check(
            &source(
                color,
                r##"<View style={{width:20,height:20,backgroundColor:'#207bbb'}}/>"##,
            ),
            true,
        );
    }
}

#[test]
fn partial_paint_updates_reuse_geometry_without_visiting_static_nodes() {
    let mut counts = Vec::new();
    for count in [20, 200] {
        let static_nodes = (0..count).map(|i| format!(r##"<View key="fixed{i}" style={{{{position:'absolute',left:{},top:200,width:2,height:2,backgroundColor:'#{:06x}'}}}}/>"##, i % 600, 0x404040 + i * 977)).collect::<String>();
        let artifact = compile_motion(&source(
            r##"interpolate(t.seconds,[0,2],["#aa3520","#207bbb"])"##,
            &static_nodes,
        ))
        .unwrap()
        .artifact;
        let scene = valle_motion::prepare_scene(&artifact).unwrap();
        let fonts = fonts();
        let props = valle_motion::resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
        let mut cache = PaintProgramCache::new(&scene, &fonts).unwrap();
        let catalog = |_: &str| None;
        let mut previous = None;
        for frame in [0, 30, 59, 17, 30] {
            let ctx =
                valle_motion::motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap())
                    .unwrap();
            let result = cache
                .prepare(
                    &ctx,
                    &props,
                    Viewport::new((640, 360)),
                    None,
                    None,
                    &catalog,
                    None,
                    Default::default(),
                )
                .unwrap();
            if let Some(old) = previous {
                assert!(
                    result.stats.reused_template,
                    "frame {frame} must bind, not re-emit"
                );
                assert!(!result.stats.emitted_program);
                assert!(Arc::ptr_eq(&old, &result.layout_boxes));
                counts.push((
                    result.stats.evaluated_expressions,
                    result.stats.parameter_slots,
                ));
            }
            previous = Some(result.layout_boxes);
        }
    }
    assert!(counts.iter().all(|count| *count == counts[0]));
}

#[test]
fn dynamic_inherited_color_transform_and_filters_are_not_misclassified_as_background_slots() {
    for style in [
        "color:interpolate(t.seconds,[0,2],['red','blue'])",
        "translate:point(t.seconds,0)",
        "filter:`blur(${t.seconds}px)`",
    ] {
        let text = format!(
            r#"export default function Demo() {{const t=useCurrentTime();return <Scene><View style={{{{width:80,height:70,{style}}}}}><Text>Inherited</Text></View></Scene>}}"#
        );
        let artifact = compile_motion(&text).unwrap().artifact;
        let scene = valle_motion::prepare_scene(&artifact).unwrap();
        assert!(
            PaintProgramCache::new(&scene, &fonts()).is_none(),
            "{style}"
        );
    }
}

#[test]
fn host_clock_inputs_are_evaluated_and_host_duration_invalidates_static_paint() {
    use valle_timeline::{ClipClock, ClipMap, RationalRate, RationalTime, SourceEnd};
    let source = r##"export default function Demo(){const t=useCurrentTime();const comp=useComposition();return <Scene style={{width:160,height:90}}><View style={{width:80,height:60,opacity:comp.clipDuration/4,backgroundColor:interpolate(t.clipSeconds,[0,3],['#8a3520','#207bbb'])}}/></Scene>}"##;
    for input in [
        source.to_owned(),
        source.replace(
            "interpolate(t.clipSeconds,[0,3],['#8a3520','#207bbb'])",
            "'#8a3520'",
        ),
        source.replace("comp.clipDuration/4", "1"),
    ] {
        let artifact = compile_motion(&input).unwrap().artifact;
        let scene = valle_motion::prepare_scene(&artifact).unwrap();
        let fonts = Fonts::default();
        let props = valle_motion::resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
        let mut cache = PaintProgramCache::new(&scene, &fonts).unwrap();
        for duration in [2, 3, 2] {
            let clock = ClipClock::new(
                FrameRate::new(30, 1).unwrap(),
                RationalTime::ZERO,
                RationalTime::new(duration, 1).unwrap(),
                ClipMap::Clip {
                    trim: RationalTime::ZERO,
                    rate: RationalRate::ONE,
                    end: SourceEnd::Hold,
                },
                RationalTime::new(1, 1).unwrap(),
                false,
            )
            .unwrap();
            for frame in [59, 0, 30, 59, 1, 0] {
                let ctx = valle_motion::MotionContext::new(clock, frame).unwrap();
                let viewport = Viewport::new((160, 90));
                let actual = cache
                    .prepare(
                        &ctx,
                        &props,
                        viewport,
                        None,
                        None,
                        &|_: &str| None,
                        None,
                        Default::default(),
                    )
                    .unwrap();
                let tree = valle_motion::build_tree(
                    &scene,
                    &ctx,
                    &props,
                    &LayoutOptions {
                        viewport,
                        fonts: &fonts,
                        styles: None,
                    },
                )
                .unwrap();
                let expected =
                    valle_motion::emit(&tree, &valle_motion::default_font_naming).unwrap();
                assert_eq!(
                    actual.program.packed_bytes().unwrap(),
                    expected.program.packed_bytes().unwrap()
                );
            }
        }
    }
}

#[test]
fn competing_background_utilities_require_cascade_provenance() {
    let text = source(
        r##"interpolate(t.seconds,[0,2],["#aa3520","#207bbb"])"##,
        "",
    )
    .replace(
        "key=\"dynamic\"",
        "key=\"dynamic\" className=\"bg-blue-500\"",
    );
    let artifact = compile_motion(&text).unwrap().artifact;
    let scene = valle_motion::prepare_scene(&artifact).unwrap();
    assert!(PaintProgramCache::new(&scene, &fonts()).is_none());
}

#[test]
fn signed_zero_props_and_clips_preserve_canonical_bytes() {
    use valle_draw::{Rect, program::AuthorColor};
    let source = r##"export const controls=({props:{tint:color({default:'#000000'})}});export default function Demo(props){return <Scene style={{width:160,height:90,color:props.tint}}><Text style={{fontSize:20}}>Zero</Text><View style={{width:20,height:20,backgroundColor:props.tint}}/></Scene>}"##;
    for input in [
        source.replace("backgroundColor:props.tint", "backgroundColor:'#ffffff'"),
        source.replace("color:props.tint", "color:'#ffffff'"),
    ] {
        let artifact = compile_motion(&input).unwrap().artifact;
        let scene = valle_motion::prepare_scene(&artifact).unwrap();
        let fonts = fonts();
        let mut cache = PaintProgramCache::new(&scene, &fonts).unwrap();
        let ctx =
            valle_motion::motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let viewport = Viewport::new((160, 90));
        for (i, zero) in [0.0, 0.0, -0.0, -0.0, 0.0].into_iter().enumerate() {
            let props = valle_motion::resolve_props(
                &artifact.controls,
                &BTreeMap::from([(
                    "tint".into(),
                    MotionValue::Color(AuthorColor {
                        red: zero,
                        green: zero,
                        blue: zero,
                        alpha: 1.,
                    }),
                )]),
            )
            .unwrap();
            let clip = Some(Rect::new(zero, zero, 160., 90.));
            let actual = cache
                .prepare(
                    &ctx,
                    &props,
                    viewport,
                    None,
                    None,
                    &|_: &str| None,
                    clip,
                    Default::default(),
                )
                .unwrap();
            assert_eq!(actual.stats.reused_template, i == 1 || i == 3);
            let tree = valle_motion::build_tree(
                &scene,
                &ctx,
                &props,
                &LayoutOptions {
                    viewport,
                    fonts: &fonts,
                    styles: None,
                },
            )
            .unwrap();
            let expected = valle_motion::emit::emit_program_at_density(
                &tree,
                &valle_motion::default_font_naming,
                None,
                &|_: &str| None,
                clip,
                Default::default(),
            )
            .unwrap();
            assert_eq!(
                actual.program.packed_bytes().unwrap(),
                expected.program.packed_bytes().unwrap()
            );
        }
    }
    assert!(!MotionValue::Number(0.0).bitwise_eq(&MotionValue::Number(-0.0)));
    assert!(
        !MotionValue::Rect(Rect::new(0., 0., 1., 1.))
            .bitwise_eq(&MotionValue::Rect(Rect::new(-0., 0., 1., 1.)))
    );
}

/// Compare canonical bytes and template identity with a fresh cache at the same input. This
/// catches history-dependent variant selection even when two variants draw identical pixels.
fn guarded_sequence(
    source: &str,
    sequence: &[u32],
    check_cold: bool,
) -> Vec<valle_motion::layout::PaintPreparationStats> {
    let artifact = compile_motion(source).unwrap().artifact;
    let scene = valle_motion::prepare_scene(&artifact).unwrap();
    let fonts = fonts();
    let props = valle_motion::resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    let viewport = Viewport::new((640, 360));
    let mut cache = PaintProgramCache::new(&scene, &fonts).unwrap();
    let mut stats = Vec::new();
    for frame in sequence {
        let ctx = valle_motion::motion_context_at_frame(*frame, 64, FrameRate::new(30, 1).unwrap())
            .unwrap();
        let prepare = |cache: &mut PaintProgramCache| {
            cache
                .prepare(
                    &ctx,
                    &props,
                    viewport,
                    None,
                    None,
                    &|_: &str| None,
                    None,
                    Default::default(),
                )
                .unwrap()
        };
        let actual = prepare(&mut cache);
        let tree = valle_motion::build_tree(
            &scene,
            &ctx,
            &props,
            &LayoutOptions {
                viewport,
                fonts: &fonts,
                styles: None,
            },
        )
        .unwrap();
        let expected = valle_motion::emit(&tree, &valle_motion::default_font_naming).unwrap();
        assert_eq!(
            actual.program.packed_bytes().unwrap(),
            expected.program.packed_bytes().unwrap(),
            "frame {frame}"
        );
        if check_cold {
            let cold = prepare(&mut PaintProgramCache::new(&scene, &fonts).unwrap());
            let identity = |result: &valle_motion::layout::PreparedPaintFrame| {
                result
                    .paint_frame
                    .as_ref()
                    .map(|f| (f.template().digest().unwrap(), f.parameter_bytes()))
            };
            assert_eq!(
                identity(&actual),
                identity(&cold),
                "cold identity frame {frame}"
            );
        }
        assert!(actual.stats.resident_variants <= 16);
        stats.push(actual.stats);
    }
    stats
}

#[test]
fn guarded_variants_reuse_shared_static_dynamic_and_omitted_paints() {
    let source = r##"export default function Demo(){const t=useCurrentTime(); return <Scene style={{width:640,height:360,backgroundColor:'red'}}>
    <View style={{width:80,height:70,backgroundColor:interpolate(t.seconds,[0,1,2],['red','blue','red'])}}/>
    <View style={{width:80,height:70,backgroundColor:interpolate(t.seconds,[0,1,2],['red','blue','green'])}}/>
    <View style={{width:80,height:70,backgroundColor:interpolate(t.seconds,[0,1,2],['#bb224400','#bb2244ff','#bb224400'])}}/>
    </Scene>}"##;
    let stats = guarded_sequence(source, &[0, 0, 15, 30, 45, 59, 0, 45, 30, 15, 0], true);
    assert!(
        stats[1].reused_template,
        "omitted + shared static paint must be reusable"
    );
    assert!(
        stats[3].reused_template,
        "two dynamic sources may share one changing paint"
    );
    assert!(stats[5..].iter().all(|s| s.reused_template));
    assert_eq!(stats.last().unwrap().resident_variants, 3);
}

#[test]
fn more_than_sixteen_guard_variants_keep_cold_hot_reverse_and_evicted_identity() {
    let nodes = (0..5).map(|i| format!(r##"<View key="color{i}" style={{{{width:30,height:20,backgroundColor:interpolate(Math.floor((t.seconds*30+0.1)/{})%2,[0,1],['#{:06x}00','#{:06x}ff'])}}}}/>"##, 1 << i, 0x224466 + i * 31, 0x224466 + i * 31)).collect::<String>();
    let source = format!(
        r#"export default function Demo(){{const t=useCurrentTime();return <Scene style={{{{width:640,height:360}}}}>{nodes}</Scene>}}"#
    );
    let order: Vec<_> = (0..32)
        .chain((0..32).rev())
        .chain((0..32).map(|i| (i * 13) % 32))
        .chain([0, 0, 31, 31])
        .collect();
    let stats = guarded_sequence(&source, &order, true);
    assert!(
        stats[..32]
            .iter()
            .all(|s| s.emitted_program && !s.reused_template)
    );
    assert!(stats[32..48].iter().all(|s| s.reused_template));
    assert!(stats[48..64].iter().all(|s| s.emitted_program));
    assert_eq!(stats[31].resident_variants, 16);
    assert!(stats[97].reused_template);
    assert!(stats[99].reused_template);
}

#[test]
fn static_opacity_siblings_use_explicit_pre_fold_paint_interning_dependencies() {
    let source = r##"export default function Demo(){const t=useCurrentTime();return <Scene style={{width:640,height:360}}>
    <View style={{width:80,height:70,backgroundColor:interpolate(t.seconds,[0,1,2],['red','blue','red'])}}/>
    <View style={{width:80,height:70,opacity:0.5,backgroundColor:'red'}}/>
    <View style={{width:80,height:70,backgroundColor:'blue'}}/>
    </Scene>}"##;
    let stats = guarded_sequence(source, &[15, 0, 30, 59, 0, 15], true);
    assert!(stats[4..].iter().all(|s| s.reused_template));
}

#[test]
fn rectangular_opacity_parameters_preserve_zero_one_and_interning_guards() {
    let text = r##"export default function Demo(){const t=useCurrentTime();return <Scene style={{width:640,height:360}}>
    <View style={{width:80,height:60,backgroundColor:'red',opacity:interpolate(t.seconds,[0,1,2],[0,0.5,1])}}/>
    <View style={{width:80,height:60,backgroundColor:'red',opacity:interpolate(t.seconds,[0,1,2],[1,0.5,0])}}/>
    <View style={{width:80,height:60,backgroundColor:'red',opacity:0.5}}/>
    <View style={{width:80,height:60,backgroundColor:'#ff000080'}}/>
    </Scene>}"##;
    let stats = guarded_sequence(text, &[0, 0, 15, 30, 45, 59, 60, 60, 0, 30, 15, 45], true);
    assert!(stats[1].reused_template, "zero-opacity omission");
    assert!(
        stats[4].reused_template,
        "different fold coefficients preserve the variant"
    );
    assert!(stats[5].reused_template);
    assert!(stats[7..].iter().all(|s| s.reused_template));
}

#[test]
fn animated_color_and_opacity_bind_without_repeating_static_geometry() {
    let text = r##"export default function Demo(){const t=useCurrentTime();return <Scene style={{width:640,height:360}}>
    <View style={{width:80,height:60,backgroundColor:interpolate(t.seconds,[0,2],['#aa3520','#207bbb']),opacity:interpolate(t.seconds,[0,2],[0.2,0.8])}}/>
    </Scene>}"##;
    let stats = guarded_sequence(text, &[0, 15, 30, 59, 0, 45, 30, 0], true);
    assert!(
        stats[1..]
            .iter()
            .all(|s| s.reused_template && !s.emitted_program)
    );
    assert!(stats.iter().all(|s| s.parameter_slots == 1));
}

#[test]
fn unsupported_opacity_consumers_do_not_enter_rectangular_paint_binding() {
    for content in [
        "<View style={{width:80,height:60,opacity:t.progress}}><View style={{width:10,height:10,backgroundColor:'red'}}/></View>",
        "<View style={{width:80,height:60,opacity:t.progress,backgroundColor:'red',borderRadius:10}}/>",
        "<Text style={{opacity:t.progress}}>Text</Text>",
    ] {
        let text = format!(
            "export default function Demo(){{const t=useCurrentTime();return <Scene>{content}</Scene>}}"
        );
        let artifact = compile_motion(&text).unwrap().artifact;
        let scene = valle_motion::prepare_scene(&artifact).unwrap();
        assert!(PaintProgramCache::new(&scene, &fonts()).is_none());
    }
}

#[test]
fn static_temporal_scopes_bind_only_unrelated_current_frame_paint() {
    for (open, close) in [
        ("<Shutter samples={16} angle={180}>", "</Shutter>"),
        ("<Echo count={8} delay={0.05} decay={0.7}>", "</Echo>"),
    ] {
        let text = format!(
            r##"export const controls=({{props:{{size:number({{default:80,min:1,max:300}})}}}});
            export default function Demo(props){{const t=useCurrentTime();return <Scene style={{{{width:640,height:360}}}}>
            <View style={{{{width:200,height:100,rotate:'8deg',opacity:0.7}}}}>{open}
            <View style={{{{width:props.size,height:40,backgroundColor:interpolate(props.size,[1,300],['#ffffff80','#33445580']),borderRadius:5,boxShadow:'1px 2px 3px #22334480',filter:'blur(1px)'}}}}/>
            <Text style={{{{fontSize:18,color:'#ffffff80'}}}}>Sampled text</Text>{close}</View>
            <View style={{{{position:'absolute',left:10,top:200,width:80,height:40,backgroundColor:interpolate(t.seconds,[0,2],['#113322','#ddccaa'])}}}}/>
            </Scene>}}"##
        );
        check(&text, true);
        let stats = guarded_sequence(&text, &[0, 15, 63, 30, 1, 0, 59, 30], true);
        assert!(stats.iter().all(|s| s.parameter_slots == 1));
        assert!(
            stats[1..]
                .iter()
                .all(|s| s.reused_template && !s.emitted_program)
        );
    }
}

#[test]
fn temporal_templates_guard_paints_shared_with_static_exposures() {
    let text = r##"export default function Demo(){const t=useCurrentTime();return <Scene style={{width:640,height:360}}>
        <Shutter samples={8} angle={180}><View style={{width:80,height:40,backgroundColor:'red'}}/></Shutter>
        <Echo count={8} delay={0.05} decay={0.7}><View style={{width:80,height:40,backgroundColor:'red'}}/></Echo>
        <View style={{position:'absolute',top:100,width:80,height:40,backgroundColor:interpolate(t.seconds,[0,1,2],['red','blue','red'])}}/>
        </Scene>}"##;
    let stats = guarded_sequence(text, &[0, 15, 30, 63, 0, 30, 15, 63], true);
    assert!(stats[4..].iter().all(|s| s.reused_template));
}

#[test]
fn temporal_templates_reject_time_dependent_sample_contents_and_ancestors() {
    for (open, close) in [
        ("<Shutter samples={8} angle={180}>", "</Shutter>"),
        ("<Echo count={8} delay={0.05} decay={0.7}>", "</Echo>"),
    ] {
        for (parent, child) in [
            (
                "",
                "backgroundColor:interpolate(t.seconds,[0,2],['red','blue'])",
            ),
            ("", "opacity:t.progress"),
            (
                "color:interpolate(t.seconds,[0,2],['red','blue'])",
                "backgroundColor:'white'",
            ),
            ("", "opacity:t.clipSeconds"),
        ] {
            let text = format!(
                "export default function Demo(){{const t=useCurrentTime();return <Scene><View style={{{{{parent}}}}}>{open}<View style={{{{width:80,height:40,{child}}}}}/><Text>Inherited</Text>{close}</View></Scene>}}"
            );
            let artifact = compile_motion(&text).unwrap().artifact;
            let scene = valle_motion::prepare_scene(&artifact).unwrap();
            assert!(PaintProgramCache::new(&scene, &fonts()).is_none(), "{text}");
        }
    }
}

#[test]
fn inactive_temporal_scopes_keep_current_frame_parameter_binding() {
    for (open, close) in [
        ("<Shutter samples={1} angle={0}>", "</Shutter>"),
        ("<Echo count={8} delay={0.05} decay={0}>", "</Echo>"),
    ] {
        let text = format!(
            "export default function Demo(){{const t=useCurrentTime();return <Scene>{open}<View style={{{{width:80,height:40,backgroundColor:interpolate(t.seconds,[0,2],['red','blue'])}}}}/>{close}</Scene>}}"
        );
        let stats = guarded_sequence(&text, &[0, 15, 63, 30, 1, 0], true);
        assert!(stats.iter().all(|s| s.parameter_slots == 1));
        assert!(stats[1..].iter().all(|s| s.reused_template));
    }
}

#[test]
fn temporal_template_hits_preserve_sample_clock_errors_and_hidden_scope_pruning() {
    for (open, close, at) in [
        (
            "<Shutter key=\"sample\" samples={8} angle={180}>",
            "</Shutter>",
            i64::MAX,
        ),
        (
            "<Echo key=\"sample\" count={8} delay={0.05} decay={0.7}>",
            "</Echo>",
            i64::MIN,
        ),
    ] {
        for hidden in [false, true] {
            let text = format!(
                "export default function Demo(){{return <Scene><View style={{{{opacity:{}}}}}>{open}<View style={{{{width:80,height:40,backgroundColor:'red'}}}}/>{close}</View></Scene>}}",
                if hidden { 0 } else { 1 }
            );
            let artifact = compile_motion(&text).unwrap().artifact;
            let scene = valle_motion::prepare_scene(&artifact).unwrap();
            let fonts = Fonts::default();
            let props = valle_motion::resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
            let mut cache = PaintProgramCache::new(&scene, &fonts).unwrap();
            let mut ctx =
                valle_motion::motion_context_at_frame(30, 60, FrameRate::new(30, 1).unwrap())
                    .unwrap();
            let viewport = Viewport::new((640, 360));
            let prepare = |cache: &mut PaintProgramCache, ctx: &valle_motion::MotionContext| {
                cache.prepare(
                    ctx,
                    &props,
                    viewport,
                    None,
                    None,
                    &|_: &str| None,
                    None,
                    Default::default(),
                )
            };
            prepare(&mut cache, &ctx).unwrap();
            assert!(prepare(&mut cache, &ctx).unwrap().stats.reused_template);
            // Public contexts can carry a sample output time whose subframe arithmetic fails.
            // A previously built template must preserve the full path's validation behavior.
            ctx.at = valle_timeline::RationalTime::new(at, 1).unwrap();
            let hot = prepare(&mut cache, &ctx);
            let cold = prepare(&mut PaintProgramCache::new(&scene, &fonts).unwrap(), &ctx);
            if hidden {
                assert!(hot.unwrap().stats.reused_template);
                assert!(cold.is_ok());
            } else {
                assert_eq!(
                    hot.err().unwrap().to_string(),
                    cold.err().unwrap().to_string()
                );
            }
        }
    }
}

#[test]
#[ignore = "isolated preparation benchmark; run optimized and without concurrent builds"]
fn preparation_template_benchmark() {
    use std::{hint::black_box, time::Instant};
    for (static_count, consumers) in [(200, 1), (2000, 1), (2000, 10)] {
        let fixed = (0..static_count).map(|i| format!(r##"<View key="fixed{i}" style={{{{position:'absolute',left:{},top:200,width:2,height:2,backgroundColor:'#{:06x}'}}}}/>"##, i % 600, 0x404040 + i * 977)).collect::<String>();
        let dynamic = (1..consumers).map(|i| format!(r##"<View key="changing{i}" style={{{{position:'absolute',left:{},top:250,width:2,height:2,backgroundColor:interpolate(t.seconds,[0,2],['#aa3520','#207bbb'])}}}}/>"##, i * 4)).collect::<String>();
        let artifact = compile_motion(&source(
            r##"interpolate(t.seconds,[0,2],["#aa3520","#207bbb"])"##,
            &(fixed + &dynamic),
        ))
        .unwrap()
        .artifact;
        let scene = valle_motion::prepare_scene(&artifact).unwrap();
        let fonts = fonts();
        let props = valle_motion::resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
        let styles = StyleCache::new();
        let faces = valle_motion::FaceCache::new();
        let viewport = Viewport::new((640, 360));
        let catalog = |_: &str| None;
        let frames: Vec<_> = (0..120)
            .map(|frame| {
                valle_motion::motion_context_at_frame(
                    frame % 60,
                    60,
                    FrameRate::new(30, 1).unwrap(),
                )
                .unwrap()
            })
            .collect();
        let mut medians = [Vec::new(), Vec::new()];
        let mut counts = None;
        // Fresh caches per run, one untimed warmup, then alternating old/new passes.
        for repeat in 0..5 {
            for version in if repeat % 2 == 0 { [0, 1] } else { [1, 0] } {
                let geometry = valle_motion::layout::LayoutCache::new(&scene, &fonts).unwrap();
                let mut template = PaintProgramCache::new(&scene, &fonts).unwrap();
                let mut run = |ctx: &valle_motion::MotionContext| {
                    if version == 0 {
                        let tree = geometry
                            .build_tree(ctx, &props, viewport, Some(&styles))
                            .unwrap();
                        let boxes = valle_motion::layout_boxes(&artifact, &tree).unwrap();
                        let report = valle_motion::emit::emit_program_at_density(
                            &tree,
                            &valle_motion::default_font_naming,
                            Some(&faces),
                            &catalog,
                            None,
                            Default::default(),
                        )
                        .unwrap();
                        black_box((report.program, boxes));
                    } else {
                        let result = template
                            .prepare(
                                ctx,
                                &props,
                                viewport,
                                Some(&styles),
                                Some(&faces),
                                &catalog,
                                None,
                                Default::default(),
                            )
                            .unwrap();
                        counts = Some((
                            result.stats.evaluated_expressions,
                            result.stats.parameter_slots,
                        ));
                        black_box(result);
                    }
                };
                run(&frames[0]);
                let started = Instant::now();
                for frame in &frames {
                    run(frame);
                }
                medians[version]
                    .push(started.elapsed().as_secs_f64() * 1000. / frames.len() as f64);
            }
        }
        for values in &mut medians {
            values.sort_by(f64::total_cmp);
        }
        eprintln!(
            "TEMPLATE_BENCH static={static_count} consumers={consumers} expressions={} slots={} old_ms={:.6} new_ms={:.6}",
            counts.unwrap().0,
            counts.unwrap().1,
            medians[0][2],
            medians[1][2]
        );
    }
}
