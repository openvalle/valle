#![cfg(feature = "motion")]

use std::collections::BTreeMap;
use valle_compiler::motion::compile_motion;
use valle_motion::layout::LayoutCache;
use valle_motion::{
    Fonts, LayoutOptions, ResolvedSignals, StyleCache, Viewport, build_tree, default_font_naming,
    emit, motion_context_at, phase_windows, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

fn scene(class: &str, style: &str, position: &str) -> String {
    format!(
        r#"export default function Demo(ctx) {{ return <Scene style={{{{width:320,height:180}}}}>
        <View key="parent" {class} style={{{{marginLeft:70,marginTop:40,width:100,height:60,
            borderWidth:3,padding:8,backgroundColor:'red',{style}}}}}>
            <View key="child" style={{{{position:'{position}',left:5,top:7,width:20,height:10,backgroundColor:'blue'}}}}/>
        </View></Scene>; }}"#
    )
}

fn frames(source: &str, frames: &[u32]) -> Vec<(BTreeMap<String, [f32; 4]>, Vec<u8>)> {
    let artifact = compile_motion(source)
        .unwrap_or_else(|d| panic!("{source}: {d:#?}"))
        .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let fonts = Fonts::default();
    let styles = StyleCache::new();
    let geometry = LayoutCache::new(&prepared, &fonts);
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    let windows = phase_windows(&artifact.controls.phase_spec(), 90);
    frames
        .iter()
        .map(|&frame| {
            let ctx = motion_context_at(frame, &windows, FrameRate::new(30, 1).unwrap()).unwrap();
            let pack = |tree: &valle_motion::LayoutTree| {
                let report = emit(tree, &default_font_naming).unwrap();
                assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
                (
                    valle_motion::layout::layout_boxes(&artifact, tree).unwrap(),
                    report.program.packed_bytes().unwrap(),
                )
            };
            let cold = build_tree(
                &prepared,
                &ctx,
                &props,
                &ResolvedSignals::default(),
                &LayoutOptions {
                    viewport: Viewport::new((320, 180)),
                    fonts: &fonts,
                    styles: None,
                },
            )
            .unwrap();
            let warm = if let Some(geometry) = &geometry {
                geometry.build_tree(
                    &ctx,
                    &props,
                    &ResolvedSignals::default(),
                    Viewport::new((320, 180)),
                    Some(&styles),
                )
            } else {
                build_tree(
                    &prepared,
                    &ctx,
                    &props,
                    &ResolvedSignals::default(),
                    &LayoutOptions {
                        viewport: Viewport::new((320, 180)),
                        fonts: &fonts,
                        styles: Some(&styles),
                    },
                )
            }
            .unwrap();
            assert_eq!(pack(&cold), pack(&warm), "frame {frame}");
            pack(&cold)
        })
        .collect()
}

fn css_3d_scene(class: &str, inline: &str) -> String {
    format!(
        r#"export default function Demo(ctx){{return <Scene style={{{{width:320,height:180}}}}>
        <View key='red' {class} style={{{{position:'absolute',left:100,top:30,width:100,height:100,
            backgroundColor:'red',rotateY:45,{inline}}}}}/>
        <View key='blue' style={{{{position:'absolute',left:100,top:30,width:100,height:100,
            backgroundColor:'blue',rotateY:-45}}}}/>
    </Scene>}}"#
    )
}

#[test]
fn css_3d_depth_uses_the_effective_author_z_index() {
    for (class, inline) in [
        ("z-20", "zIndex:20"),
        ("z-0", "zIndex:0"),
        ("-z-10", "zIndex:-10"),
        ("[z-index:37]", "zIndex:37"),
    ] {
        assert_eq!(
            frames(&css_3d_scene(&format!("className='{class}'"), ""), &[0])[0],
            frames(&css_3d_scene("", inline), &[0])[0],
            "{class}"
        );
    }
    assert_ne!(
        frames(&css_3d_scene("className='z-20'", ""), &[0])[0].1,
        frames(&css_3d_scene("", ""), &[0])[0].1,
    );
    assert_eq!(
        frames(&css_3d_scene("className='z-20'", "zIndex:5"), &[0])[0],
        frames(&css_3d_scene("", "zIndex:5"), &[0])[0],
    );
    assert_eq!(
        frames(&css_3d_scene("className='z-20!'", "zIndex:5"), &[0])[0],
        frames(&css_3d_scene("", "zIndex:'20 !important'"), &[0])[0],
    );
    assert_eq!(
        frames(&css_3d_scene("", "zIndex:'auto'"), &[0])[0],
        frames(&css_3d_scene("", ""), &[0])[0],
    );
    assert_eq!(
        frames(&css_3d_scene("", "zIndex:'auto !important'"), &[0])[0],
        frames(&css_3d_scene("", ""), &[0])[0],
    );
}

#[test]
fn css_3d_origin_uses_the_effective_author_transform_origin() {
    for (class, inline) in [
        ("origin-top-left", "transformOrigin:'0% 0%'"),
        ("origin-center", "transformOrigin:'50% 50%'"),
        ("origin-bottom-right", "transformOrigin:'100% 100%'"),
        ("origin-[20%_40%]", "transformOrigin:'20% 40%'"),
        ("origin-[12px_8px]", "transformOrigin:'12px 8px'"),
    ] {
        assert_eq!(
            frames(&css_3d_scene(&format!("className='{class}'"), ""), &[0])[0],
            frames(&css_3d_scene("", inline), &[0])[0],
            "{class}"
        );
    }
    assert_ne!(
        frames(&css_3d_scene("className='origin-top-left'", ""), &[0])[0].1,
        frames(&css_3d_scene("", ""), &[0])[0].1,
    );
    assert_eq!(
        frames(
            &css_3d_scene("className='origin-top-left'", "transformOrigin:'100% 100%'"),
            &[0]
        )[0],
        frames(&css_3d_scene("", "transformOrigin:'100% 100%'"), &[0])[0],
    );
    assert_eq!(
        frames(
            &css_3d_scene(
                "className='origin-top-left!'",
                "transformOrigin:'100% 100%'"
            ),
            &[0]
        )[0],
        frames(
            &css_3d_scene("", "transformOrigin:'0% 0% !important'"),
            &[0]
        )[0],
    );
}

#[test]
fn conditional_3d_styles_and_rotation_seek_without_history() {
    let dynamic = css_3d_scene(
        "className={ctx.localFrame<30?'z-20 origin-top-left':''}",
        "",
    )
    .replace("rotateY:45,", "rotateY:ctx.localFrame,");
    let requested = [60, 0, 30, 60, 15, 0];
    let actual = frames(&dynamic, &requested);
    for (index, frame) in requested.into_iter().enumerate() {
        let class = if frame < 30 {
            "className='z-20 origin-top-left'"
        } else {
            ""
        };
        let expected = css_3d_scene(class, "").replace("rotateY:45,", &format!("rotateY:{frame},"));
        assert_eq!(actual[index], frames(&expected, &[0])[0], "frame {frame}");
    }
}

#[test]
fn identity_and_none_have_distinct_containing_blocks_in_every_entry() {
    for position in ["fixed", "absolute"] {
        let identity = frames(&scene("", "rotate:'0deg'", position), &[0]).remove(0);
        let none = frames(&scene("", "", position), &[0]).remove(0);
        assert_ne!(identity.1, none.1, "{position}");
        for (class, style) in [
            ("", "scale:1"),
            ("", "scale:point(1,1)"),
            ("", "scale:'100%'"),
            ("className=\"scale-100\"", ""),
            ("className=\"scale-x-100\"", ""),
            ("className=\"scale-[1]\"", ""),
            ("className=\"scale-[1_1]\"", ""),
            ("className=\"scale-100! scale-none\"", "scale:'none'"),
            ("", "translate:'0px 0px'"),
            ("className=\"translate-x-0\"", ""),
        ] {
            assert_eq!(
                frames(&scene(class, style, position), &[0])[0],
                identity,
                "{class} {style} / {position}"
            );
        }
        for (class, style) in [
            ("", "scale:'none'"),
            ("", "scale:'initial'"),
            ("", "translate:'none'"),
            ("className=\"scale-100 scale-none\"", ""),
            ("className=\"scale-100\"", "scale:'none'"),
            ("className=\"scale-100!\"", "scale:'none !important'"),
        ] {
            assert_eq!(
                frames(&scene(class, style, position), &[0])[0],
                none,
                "{class} {style} / {position}"
            );
        }
    }
}

#[test]
fn translate_single_value_defaults_y_to_zero() {
    for (single, pair) in [
        ("12px", "12px 0px"),
        ("25%", "25% 0px"),
        ("calc(25% + 5px)", "calc(25% + 5px) 0px"),
    ] {
        assert_eq!(
            frames(&scene("", &format!("translate:'{single}'"), "fixed"), &[0]),
            frames(&scene("", &format!("translate:'{pair}'"), "fixed"), &[0])
        );
    }
}

#[test]
fn animated_scale_keeps_geometry_at_identity_and_seeks_without_history() {
    let source = scene("", "scale:1+ctx.localFrame/90", "fixed");
    let requested = [0, 60, 0, 30, 89, 60, 0];
    let actual = frames(&source, &requested);
    for (i, frame) in requested.into_iter().enumerate() {
        assert_eq!(
            actual[i],
            frames(
                &scene(
                    "",
                    &format!("scale:{}", 1.0 + f64::from(frame) / 90.0),
                    "fixed"
                ),
                &[0]
            )[0]
        );
    }
    let class = scene(
        "className={ctx.localFrame < 30 ? 'scale-none' : 'scale-100'}",
        "",
        "fixed",
    );
    let actual = frames(&class, &requested);
    for (i, frame) in requested.into_iter().enumerate() {
        let style = if frame < 30 {
            "scale:'none'"
        } else {
            "scale:1"
        };
        assert_eq!(actual[i], frames(&scene("", style, "fixed"), &[0])[0]);
    }
}

#[test]
fn scale_presence_only_inherits_when_the_property_requests_inheritance() {
    let nested = |inner: &str| {
        format!(
            r#"export default function Demo() {{return <Scene style={{{{width:320,height:180}}}}>
        <View style={{{{scale:1,marginLeft:40,marginTop:20,width:200,height:120}}}}>
            <View key="parent" style={{{{marginLeft:30,marginTop:15,width:100,height:60,{inner}}}}}>
                <View key="child" style={{{{position:'fixed',left:5,top:7,width:20,height:10,backgroundColor:'red'}}}}/>
            </View></View></Scene>;}}"#
        )
    };
    assert_eq!(
        frames(&nested("scale:'inherit'"), &[0]),
        frames(&nested("scale:1"), &[0])
    );
    assert_ne!(frames(&nested(""), &[0]), frames(&nested("scale:1"), &[0]));
    assert_eq!(
        frames(&nested("scale:'unset'"), &[0]),
        frames(&nested(""), &[0])
    );
}

#[test]
fn ordered_lists_compose_with_individual_properties_and_preserve_reference_boxes() {
    for (style, equivalent) in [
        (
            "transform:'scale(2) translate(10px)'",
            "transform:'matrix(2,0,0,2,20,0)'",
        ),
        (
            "transform:'translate(10px) scale(2)'",
            "transform:'matrix(2,0,0,2,10,0)'",
        ),
        (
            "translate:'10px 20px',scale:point(2,3),transform:'translate(5px,7px) scale(0.5,2)'",
            "transform:'matrix(1,0,0,6,20,41)'",
        ),
        (
            "transform:'scaleX(-1) translateX(25%) translateY(50%)'",
            "transform:'matrix(-1,0,0,1,-25,30)'",
        ),
        (
            "transform:'translate(calc(25% + 5px))'",
            "transform:'translate(30px,0px)'",
        ),
        ("transform:'skew(0deg)'", "transform:'matrix(1,0,0,1,0,0)'"),
        (
            "transform:'skewX(45deg) skewY(45deg)'",
            "transform:'matrix(2,1,1,1,0,0)'",
        ),
    ] {
        for origin in ["0px 0px", "20% 40%"] {
            let extras =
                format!("transformOrigin:'{origin}',overflow:'hidden',filter:'blur(0px)',");
            assert_eq!(
                frames(&scene("", &(extras.clone() + style), "fixed"), &[0]),
                frames(&scene("", &(extras + equivalent), "fixed"), &[0]),
                "{style} / {origin}"
            );
        }
    }
    assert_ne!(
        frames(
            &scene("", "transform:'scale(2) translateX(10px)'", "fixed"),
            &[0]
        ),
        frames(
            &scene("", "transform:'translateX(10px) scale(2)'", "fixed"),
            &[0]
        )
    );
    assert_ne!(
        frames(&scene("", "transform:'matrix(1,0,0,1,0,0)'", "fixed"), &[0]),
        frames(&scene("", "transform:'none'", "fixed"), &[0])
    );
}

#[test]
fn dynamic_transform_lists_and_conditional_none_seek_directly() {
    let requested = [0, 60, 0, 30, 89, 60, 0];
    let actual = frames(
        &scene(
            "",
            "transform:`scale(${1+ctx.localFrame/90}) translateX(10px)`",
            "fixed",
        ),
        &requested,
    );
    for (i, frame) in requested.into_iter().enumerate() {
        assert_eq!(
            actual[i],
            frames(
                &scene(
                    "",
                    &format!(
                        "transform:'scale({}) translateX(10px)'",
                        1.0 + f64::from(frame) / 90.0
                    ),
                    "fixed"
                ),
                &[0]
            )[0]
        );
    }
    let actual = frames(
        &scene(
            "",
            "transform:ctx.localFrame < 30 ? 'none' : 'matrix(1,0,0,1,0,0)'",
            "fixed",
        ),
        &requested,
    );
    for (i, frame) in requested.into_iter().enumerate() {
        let value = if frame < 30 {
            "none"
        } else {
            "matrix(1,0,0,1,0,0)"
        };
        assert_eq!(
            actual[i],
            frames(&scene("", &format!("transform:'{value}'"), "fixed"), &[0])[0]
        );
    }
}

#[test]
fn singular_transforms_hide_the_whole_subtree_without_losing_layout() {
    let empty = frames(
        "export default function Demo(){return <Scene style={{width:320,height:180}}/>;}",
        &[0],
    )[0]
    .1
    .clone();
    for value in [
        "scale:0",
        "transform:'scale(0)'",
        "transform:'matrix(1,1,1,1,0,0)'",
    ] {
        let actual = frames(&scene("", value, "fixed"), &[0]);
        assert_eq!(actual[0].1, empty, "{value}");
        assert_eq!(actual[0].0["parent"][2..], [100.0, 60.0]);
    }
}

#[test]
fn transform_admission_is_shared_with_loaded_artifacts_and_presence_feedback_stays_closed() {
    let mut artifact = compile_motion(&scene("", "transform:'translate(10px)'", "fixed"))
        .unwrap()
        .artifact;
    let binding = artifact
        .nodes
        .iter_mut()
        .flat_map(|node| &mut node.styles)
        .find(|style| style.property == "transform")
        .unwrap();
    binding.value = valle_motion::StyleValue::Static {
        value: valle_motion::MotionValue::Str("translate(12px 0px)".into()),
    };
    assert!(artifact.validate().is_err());
    assert!(prepare_scene(&artifact).is_err());
    let diagnostics = compile_motion(&scene(
        "",
        "transform:bounds('child').width > 10 ? 'none' : 'translateX(10px)'",
        "fixed",
    ))
    .unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("post-layout")),
        "{diagnostics:?}"
    );
}

#[test]
fn finite_transform_arguments_cannot_overflow_into_a_successful_empty_frame() {
    let artifact = compile_motion(&scene("", "transform:'scale(1e30) scale(1e30)'", "fixed"))
        .unwrap()
        .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let fonts = Fonts::default();
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    let windows = phase_windows(&artifact.controls.phase_spec(), 90);
    let ctx = motion_context_at(0, &windows, FrameRate::new(30, 1).unwrap()).unwrap();
    let tree = build_tree(
        &prepared,
        &ctx,
        &props,
        &ResolvedSignals::default(),
        &LayoutOptions {
            viewport: Viewport::new((320, 180)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let error = emit(&tree, &default_font_naming)
        .err()
        .expect("overflow must fail");
    assert!(
        error
            .to_string()
            .contains("computed transform must be finite"),
        "{error}"
    );
}
