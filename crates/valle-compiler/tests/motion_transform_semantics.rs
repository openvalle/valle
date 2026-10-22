#![cfg(feature = "motion")]

use std::collections::BTreeMap;
use valle_compiler::motion::compile_motion;
use valle_draw::program::{Node, Paint, Transform2d};
use valle_motion::layout::LayoutCache;
use valle_motion::{
    Fonts, LayoutOptions, StyleCache, Viewport, build_tree, default_font_naming, emit,
    motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

fn scene(class: &str, style: &str, position: &str) -> String {
    format!(
        r#"export default function Demo() {{ const time = useCurrentTime(); const frame = Math.floor(time.seconds * 30); return <Scene style={{{{width:320,height:180}}}}>
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
    frames
        .iter()
        .map(|&frame| {
            let ctx = motion_context_at_frame(frame, 90, FrameRate::new(30, 1).unwrap()).unwrap();
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
                &LayoutOptions {
                    viewport: Viewport::new((320, 180)),
                    fonts: &fonts,
                    styles: None,
                },
            )
            .unwrap();
            let warm = if let Some(geometry) = &geometry {
                geometry.build_tree(&ctx, &props, Viewport::new((320, 180)), Some(&styles))
            } else {
                build_tree(
                    &prepared,
                    &ctx,
                    &props,
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
        r#"export default function Demo(){{ const time = useCurrentTime(); const frame = Math.floor(time.seconds * 30);return <Scene style={{{{width:320,height:180}}}}>
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
    let dynamic = css_3d_scene("className={frame<30?'z-20 origin-top-left':''}", "")
        .replace("rotateY:45,", "rotateY:frame,");
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
fn css_3d_lists_draw_the_matrices_css_defines_for_perspective_matrix3d_skew_and_matrix() {
    let tan = |degrees: f64| valle_draw::math::tan(degrees.to_radians());
    let list = |value: &str| css_3d_scene("", &format!("transform:'{value}'"));
    for (functions, matrices) in [
        (
            "rotateX(20deg) skew(20deg, 10deg)".to_owned(),
            format!(
                "rotateX(20deg) matrix(1, {}, {}, 1, 0, 0)",
                tan(10.0),
                tan(20.0)
            ),
        ),
        (
            "rotateX(20deg) skewX(20deg) skewY(10deg)".to_owned(),
            format!(
                "rotateX(20deg) matrix(1, 0, {}, 1, 0, 0) matrix(1, {}, 0, 1, 0, 0)",
                tan(20.0),
                tan(10.0)
            ),
        ),
        (
            "perspective(400px) rotateX(20deg)".to_owned(),
            format!(
                "matrix3d(1,0,0,0, 0,1,0,0, 0,0,1,{}, 0,0,0,1) rotateX(20deg)",
                -1.0 / 400.0
            ),
        ),
        (
            "translate3d(12px, 8px, 30px) rotateX(20deg)".to_owned(),
            "matrix3d(1,0,0,0, 0,1,0,0, 0,0,1,0, 12,8,30,1) rotateX(20deg)".to_owned(),
        ),
        // A depth below 1px is 1px, and `none` is the identity.
        (
            "perspective(0.5px) translateZ(-10px)".to_owned(),
            "perspective(1px) translateZ(-10px)".to_owned(),
        ),
        (
            "perspective(none) translateZ(-10px)".to_owned(),
            "translateZ(-10px)".to_owned(),
        ),
    ] {
        assert_eq!(
            frames(&list(&functions), &[0])[0],
            frames(&list(&matrices), &[0])[0],
            "{functions}"
        );
    }
    assert_ne!(
        frames(&list("perspective(400px) rotateX(20deg)"), &[0])[0].1,
        frames(&list("rotateX(20deg)"), &[0])[0].1,
    );
    assert_ne!(
        frames(&list("rotateX(20deg) skew(20deg, 10deg)"), &[0])[0].1,
        frames(&list("rotateX(20deg) skewX(20deg) skewY(10deg)"), &[0])[0].1,
    );
    let requested = [60, 0, 30];
    let dynamic = css_3d_scene(
        "",
        "transform:`perspective(${400 + frame}px) rotateX(${frame}deg)`",
    );
    let actual = frames(&dynamic, &requested);
    for (index, frame) in requested.into_iter().enumerate() {
        assert_eq!(
            actual[index],
            frames(
                &list(&format!(
                    "perspective({}px) rotateX({frame}deg)",
                    400 + frame
                )),
                &[0]
            )[0],
            "frame {frame}"
        );
    }
}

fn stage_scene(transform_style: &str, backface: &str) -> String {
    format!(
        r#"export default function Demo(){{ const time = useCurrentTime(); const frame = Math.floor(time.seconds * 30);return <Scene style={{{{width:320,height:180}}}}>
        <View key='stage' style={{{{position:'absolute',left:60,top:20,width:200,height:140,
            perspective:400,rotateY:50,transformStyle:{transform_style}}}}}>
            <View key='card' style={{{{position:'absolute',left:50,top:20,width:100,height:100,
                backgroundColor:'red',rotateY:50,backfaceVisibility:{backface}}}}}/>
        </View>
    </Scene>}}"#
    )
}

#[test]
fn transform_style_and_backface_visibility_can_change_per_frame() {
    // Preserved, the card turns 100° in all and shows its back; flattened, it turns 50°.
    let preserved_hidden = frames(&stage_scene("'preserve-3d'", "'hidden'"), &[0]).remove(0);
    assert_ne!(
        preserved_hidden.1,
        frames(&stage_scene("'preserve-3d'", "'visible'"), &[0])[0].1
    );
    assert_ne!(
        preserved_hidden.1,
        frames(&stage_scene("'flat'", "'hidden'"), &[0])[0].1
    );
    let dynamic = stage_scene(
        "frame<30?'preserve-3d':'flat'",
        "frame<15||frame>=45?'hidden':'visible'",
    );
    let requested = [60, 0, 30, 15, 45, 0];
    let actual = frames(&dynamic, &requested);
    for (index, frame) in requested.into_iter().enumerate() {
        let expected = stage_scene(
            if frame < 30 {
                "'preserve-3d'"
            } else {
                "'flat'"
            },
            if !(15..45).contains(&frame) {
                "'hidden'"
            } else {
                "'visible'"
            },
        );
        assert_eq!(actual[index], frames(&expected, &[0])[0], "frame {frame}");
    }

    // A keyword that only the frame knows is checked on the frame that draws it.
    let mut artifact = compile_motion(&dynamic).unwrap().artifact;
    for expr in &mut artifact.exprs {
        if let valle_motion::Expr::Const {
            value: valle_motion::MotionValue::Str(text),
        } = expr
            && text == "flat"
        {
            *text = "flatten".into();
        }
    }
    artifact.validate().unwrap();
    let prepared = prepare_scene(&artifact).unwrap();
    let fonts = Fonts::default();
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    let build = |frame| {
        build_tree(
            &prepared,
            &motion_context_at_frame(frame, 90, FrameRate::new(30, 1).unwrap()).unwrap(),
            &props,
            &LayoutOptions {
                viewport: Viewport::new((320, 180)),
                fonts: &fonts,
                styles: None,
            },
        )
    };
    build(0).unwrap();
    let error = build(30)
        .err()
        .expect("frame 30 draws `flatten`")
        .to_string();
    assert!(
        error.contains("transformStyle takes `flat` or `preserve-3d`, not `flatten`"),
        "{error}"
    );
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
fn dynamic_transform_values_match_literal_css_and_preserve_geometry_reuse() {
    let requested = [60, 0, 30, 60, 15, 0];
    for (dynamic, reference) in [
        ("translate:`${frame}px 10%`", "translate:'FRAMEpx 10%'"),
        ("translate:`${frame}px`", "translate:'FRAMEpx'"),
        ("translate:point(frame,5)", "translate:'FRAMEpx 5px'"),
        ("rotate:`${frame}deg`", "rotate:'FRAMEdeg'"),
        ("scale:`${1+frame/90} 1`", "scale:'SCALE 1'"),
        ("scale:`${100+frame}%`", "scale:'PERCENT%'"),
        (
            "transformOrigin:`${frame}% 20%`,rotate:'20deg'",
            "transformOrigin:'FRAME% 20%',rotate:'20deg'",
        ),
    ] {
        let authored = scene("", dynamic, "fixed");
        let artifact = compile_motion(&authored).unwrap().artifact;
        if !dynamic.starts_with("transformOrigin") {
            assert!(
                prepare_scene(&artifact).unwrap().can_reuse_layout(),
                "{dynamic}"
            );
        }
        let actual = frames(&authored, &requested);
        for (index, frame) in requested.into_iter().enumerate() {
            let literal = reference
                .replace("FRAME", &frame.to_string())
                .replace("SCALE", &(1.0 + frame as f64 / 90.0).to_string())
                .replace("PERCENT", &(100 + frame).to_string());
            assert_eq!(
                actual[index],
                frames(&scene("", &literal, "fixed"), &[0])[0],
                "{dynamic}, frame {frame}"
            );
        }
    }
}

#[test]
fn dynamic_transform_presence_switches_rebuild_positioned_descendants() {
    let requested = [60, 0, 29, 30, 60, 0];
    for (property, identity) in [
        ("translate", "0px 0px"),
        ("rotate", "0deg"),
        ("scale", "1 1"),
    ] {
        for position in ["absolute", "fixed"] {
            let dynamic = format!("{property}:frame<30?'none':'{identity}'");
            let authored = scene("", &dynamic, position);
            let artifact = compile_motion(&authored).unwrap().artifact;
            assert!(!prepare_scene(&artifact).unwrap().can_reuse_layout());
            let actual = frames(&authored, &requested);
            assert_ne!(actual[0], actual[1]);
            for (index, frame) in requested.into_iter().enumerate() {
                let value = if frame < 30 { "none" } else { identity };
                assert_eq!(
                    actual[index],
                    frames(&scene("", &format!("{property}:'{value}'"), position), &[0])[0]
                );
            }
        }
    }
}

#[test]
fn transform_templates_can_be_reused_through_constants_helpers_and_components() {
    let source = r#"function angle(t) { return `${t}deg`; }
    function Item(props) { return <View key='parent' style={{width:80,height:60,backgroundColor:'red',
        translate:props.move,rotate:props.turn,scale:props.zoom,transform:props.skew}}/>; }
    export default function Demo() { const time = useCurrentTime(); const frame = Math.floor(time.seconds * 30);
        const move = `${frame}px 5px`;
        const turn = angle(frame);
        const zoom = `${1+frame/90} 1`;
        const skew = frame<30 ? `skew(${frame}deg)` : `skew(10deg,${frame}deg)`;
        return <Scene style={{width:320,height:180}}><Item move={move} turn={turn} zoom={zoom} skew={skew}/></Scene>;
    }"#;
    let actual = frames(source, &[60, 0, 15, 60, 0]);
    assert_ne!(actual[0], actual[1]);
    assert_ne!(actual[1], actual[2]);
    assert_eq!(actual[0], actual[3]);
    assert_eq!(actual[1], actual[4]);
}

#[test]
fn css_template_choices_keep_literal_keywords_through_local_bindings() {
    for (property, value) in [
        ("translate", "${frame}px"),
        ("rotate", "${frame}deg"),
        ("scale", "${1+frame/90}"),
    ] {
        let dynamic = scene("", &format!("{property}:chosen"), "fixed")
            .replace("* 30); return <Scene", &format!("* 30); const reset = 'none'; const chosen = frame<30 ? reset : `{value}`; return <Scene"));
        let inline = scene(
            "",
            &format!("{property}:frame<30?'none':`{value}`"),
            "fixed",
        );
        assert_eq!(
            frames(&dynamic, &[60, 0, 15, 30, 60, 0]),
            frames(&inline, &[60, 0, 15, 30, 60, 0]),
            "{property}"
        );
    }
}

#[test]
fn css_choices_can_mix_typed_scalars_and_literal_strings() {
    let dynamic = scene(
        "",
        "rotate:frame<30?'15deg':interpolate(frame,[0,90],['0deg','90deg'])",
        "fixed",
    );
    let inline = scene("", "rotate:frame<30?'15deg':`${frame}deg`", "fixed");
    assert_eq!(
        frames(&dynamic, &[60, 0, 15, 30, 60, 0]),
        frames(&inline, &[60, 0, 15, 30, 60, 0])
    );
    let scale = scene("", "scale:frame<30?'none':1+frame/90", "fixed");
    let inline = scene("", "scale:frame<30?'none':`${1+frame/90}`", "fixed");
    assert_eq!(
        frames(&scale, &[60, 0, 15, 30, 60, 0]),
        frames(&inline, &[60, 0, 15, 30, 60, 0])
    );
}

#[test]
fn dynamic_priority_must_not_reuse_a_different_containing_block() {
    for (property, identity, class) in [
        ("translate", "0px", "[translate:none]!"),
        ("rotate", "0deg", "[rotate:none]!"),
        ("scale", "1", "scale-none!"),
    ] {
        let class = format!("className='{class}'");
        let dynamic = scene(
            &class,
            &format!("{property}:frame<30?'{identity}':'{identity} !important'"),
            "fixed",
        );
        let artifact = compile_motion(&dynamic).unwrap().artifact;
        assert!(
            !prepare_scene(&artifact).unwrap().can_reuse_layout(),
            "{property}"
        );
        let actual = frames(&dynamic, &[60, 0, 60, 0]);
        assert_ne!(actual[0], actual[1], "{property}");
        assert_eq!(
            actual[0],
            frames(
                &scene(
                    &class,
                    &format!("{property}:'{identity} !important'"),
                    "fixed"
                ),
                &[0]
            )[0]
        );
        assert_eq!(
            actual[1],
            frames(
                &scene(&class, &format!("{property}:'{identity}'"), "fixed"),
                &[0]
            )[0]
        );
    }
}

#[test]
fn animated_scale_keeps_geometry_at_identity_and_seeks_without_history() {
    let source = scene("", "scale:1+frame/90", "fixed");
    let requested = [0, 60, 0, 30, 89, 60, 0];
    let actual = frames(&source, &requested);
    for (i, frame) in requested.into_iter().enumerate() {
        // The last frame shows the end of the scene.
        let shown = if frame == 89 { 90 } else { frame };
        assert_eq!(
            actual[i],
            frames(
                &scene(
                    "",
                    &format!("scale:{}", 1.0 + f64::from(shown) / 90.0),
                    "fixed"
                ),
                &[0]
            )[0]
        );
    }
    let class = scene(
        "className={frame < 30 ? 'scale-none' : 'scale-100'}",
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
            "transform:`scale(${1+frame/90}) translateX(10px)`",
            "fixed",
        ),
        &requested,
    );
    for (i, frame) in requested.into_iter().enumerate() {
        // The last frame shows the end of the scene.
        let shown = if frame == 89 { 90 } else { frame };
        assert_eq!(
            actual[i],
            frames(
                &scene(
                    "",
                    &format!(
                        "transform:'scale({}) translateX(10px)'",
                        1.0 + f64::from(shown) / 90.0
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
            "transform:frame < 30 ? 'none' : 'matrix(1,0,0,1,0,0)'",
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
    let ctx = motion_context_at_frame(0, 90, FrameRate::new(30, 1).unwrap()).unwrap();
    let tree = build_tree(
        &prepared,
        &ctx,
        &props,
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

/// Where the frame draws the corners of its one red box, in output pixels: the program's
/// transforms composed down to the box's path, as an executor draws it.
fn red_corners(source: &str) -> [[f64; 2]; 4] {
    // Cached geometry draws what a cold layout draws.
    frames(source, &[0]);
    let artifact = compile_motion(source)
        .unwrap_or_else(|d| panic!("{source}: {d:#?}"))
        .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let fonts = Fonts::default();
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    let tree = build_tree(
        &prepared,
        &motion_context_at_frame(0, 90, FrameRate::new(30, 1).unwrap()).unwrap(),
        &props,
        &LayoutOptions {
            viewport: Viewport::new((320, 180)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let program = emit(&tree, &default_font_naming).unwrap().program;
    // Programs hold colors in the linear Rec.2020 working space.
    let red = valle_draw::program::LinearColor::from_srgb8(valle_draw::Rgba::rgb(255, 0, 0));
    let mut corners = Vec::new();
    let mut pending = program
        .roots()
        .iter()
        .map(|&node| (node, Transform2d::IDENTITY))
        .collect::<Vec<_>>();
    while let Some((node, outer)) = pending.pop() {
        match &program.nodes()[node.raw() as usize] {
            Node::Group(group) => {
                let inner = group.transform.then(outer);
                pending.extend(group.children.iter().map(|&child| (child, inner)));
            }
            Node::Path(path)
                if path.fill.is_some_and(|paint| {
                    matches!(&program.paints()[paint.raw() as usize], Paint::Solid(color)
                        if (color.red - red.red).abs() < 1e-4
                            && (color.green - red.green).abs() < 1e-4
                            && (color.blue - red.blue).abs() < 1e-4)
                }) =>
            {
                let [x0, y0, x1, y1] = program.paths()[path.path.raw() as usize]
                    .points
                    .iter()
                    .fold(
                        [f64::MAX, f64::MAX, f64::MIN, f64::MIN],
                        |[x0, y0, x1, y1], &[x, y]| [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
                    );
                corners.push([[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(|[x, y]| {
                    let point = outer.try_apply(valle_draw::Point::new(x, y)).unwrap();
                    [point.x, point.y]
                }));
            }
            _ => {}
        }
    }
    assert_eq!(corners.len(), 1, "{source}");
    corners[0]
}

fn assert_corners(source: &str, expected: [[f64; 2]; 4]) {
    let actual = red_corners(source);
    assert!(
        actual
            .iter()
            .zip(&expected)
            .all(|(a, e)| (a[0] - e[0]).abs() < 1e-3 && (a[1] - e[1]).abs() < 1e-3),
        "drawn {actual:?}, expected {expected:?}\n{source}"
    );
}

fn plane_scene(camera: &str, body: &str) -> String {
    format!(
        r#"export default function Demo(){{return <Scene style={{{{width:320,height:180}}}} {camera}>{body}</Scene>}}"#
    )
}

/// A red 100×60 box at (110, 60), its origin at (160, 90).
fn card(style: &str) -> String {
    format!(
        r#"<View key='card' style={{{{position:'absolute',left:110,top:60,width:100,height:60,
            backgroundColor:'red',{style}}}}}/>"#
    )
}

// The card turned 60° about y and seen without perspective is 50 wide; turned 90° in 2D about
// its center, it then stands 50 wide and 100 tall around (160, 90).
const TURNED_CARD: [[f64; 2]; 4] = [[190.0, 65.0], [190.0, 115.0], [130.0, 115.0], [130.0, 65.0]];

#[test]
fn a_plane_draws_where_its_flat_ancestors_and_the_camera_put_it() {
    assert_corners(
        &plane_scene(
            "",
            r#"<View key='parent' style={{position:'absolute',left:110,top:60,width:100,height:60,
                rotate:'90deg'}}>
                <View key='card' style={{position:'absolute',left:0,top:0,width:100,height:60,
                    backgroundColor:'red',transform:'rotateY(60deg)'}}/>
            </View>"#,
        ),
        TURNED_CARD,
    );
    // The camera draws world point (160, 90) at the output's center, zoomed 2× and turned 90°.
    assert_corners(
        &plane_scene(
            "camera={{center:point(160,90),zoom:2,rotation:90}}",
            &format!("<World>{}</World>", card("transform:'rotateY(60deg)'")),
        ),
        [[220.0, 40.0], [220.0, 140.0], [100.0, 140.0], [100.0, 40.0]],
    );
}

#[test]
fn a_plane_keeps_its_own_2d_transforms_and_so_does_each_plane_of_a_3d_context() {
    assert_corners(
        &plane_scene("", &card("rotate:'90deg',transform:'rotateY(60deg)'")),
        TURNED_CARD,
    );
    let stage = |stage: &str, style: &str| {
        plane_scene(
            "",
            &format!(
                r#"<View key='stage' style={{{{position:'absolute',left:60,top:20,width:200,
                    height:140,transformStyle:'preserve-3d',{stage}}}}}>
                    <View key='card' style={{{{position:'absolute',left:50,top:40,width:100,
                        height:60,backgroundColor:'red',{style}}}}}/>
                </View>"#
            ),
        )
    };
    let upright = [[190.0, 40.0], [190.0, 140.0], [130.0, 140.0], [130.0, 40.0]];
    assert_corners(&stage("", "rotate:'90deg'"), upright);
    assert_corners(&stage("", "transform:'rotate(90deg)'"), upright);
    // The context's root turns its planes too.
    assert_corners(&stage("rotate:'90deg'", ""), upright);
    assert_corners(
        &stage("", "translate:'30px 10px'"),
        [[140.0, 70.0], [240.0, 70.0], [240.0, 130.0], [140.0, 130.0]],
    );
}

#[test]
fn an_empty_box_is_a_plane_for_its_overflowing_descendants() {
    assert_corners(
        &plane_scene(
            "",
            r#"<View key='hinge' style={{position:'absolute',left:160,top:90,width:0,height:0,
                transform:'rotateY(60deg)'}}>
                <View key='card' style={{position:'absolute',left:-50,top:-30,width:100,height:60,
                    backgroundColor:'red'}}/>
            </View>"#,
        ),
        [[135.0, 60.0], [185.0, 60.0], [185.0, 120.0], [135.0, 120.0]],
    );
}

#[test]
fn a_camera_moving_in_depth_draws_world_planes_in_css_perspective() {
    let (sin, cos) = valle_draw::math::sin_cos(30_f64.to_radians());
    // CSS perspective 400px toward the camera's center (160, 90).
    let project = |[x, y, z]: [f64; 3]| {
        let scale = 400.0 / (400.0 - z);
        [160.0 + (x - 160.0) * scale, 90.0 + (y - 90.0) * scale]
    };
    let corners = |point: &dyn Fn(f64, f64) -> [f64; 3]| {
        [[110.0, 60.0], [210.0, 60.0], [210.0, 120.0], [110.0, 120.0]]
            .map(|[x, y]| project(point(x, y)))
    };
    let camera = |fields: &str| {
        plane_scene(
            &format!("camera={{{{center:point(160,90),zoom:1,perspective:400,{fields}}}}}"),
            &format!("<World>{}</World>", card("")),
        )
    };
    // Orbiting right brings the card's right edge closer, and orbiting up its top edge.
    assert_corners(
        &camera("rotateY:30"),
        corners(&|x, y| [160.0 + (x - 160.0) * cos, y, (x - 160.0) * sin]),
    );
    assert_corners(
        &camera("rotateX:30"),
        corners(&|x, y| [x, 90.0 + (y - 90.0) * cos, -(y - 90.0) * sin]),
    );
    // Moving the focus 100px into the scene brings the card 100px closer.
    assert_corners(&camera("z:-100"), corners(&|x, y| [x, y, 100.0]));
    assert_corners(&camera(""), corners(&|x, y| [x, y, 0.0]));
    // Screen content stays out of the camera's space, and its own planes are drawn flat.
    assert_corners(
        &plane_scene(
            "camera={{center:point(100,50),zoom:2,perspective:400,z:-100,rotateY:30}}",
            &format!(
                "<World><View key='floor' style={{{{width:10,height:10}}}}/></World><Screen>{}</Screen>",
                card("transform:'rotateY(60deg)'")
            ),
        ),
        [[135.0, 60.0], [185.0, 60.0], [185.0, 120.0], [135.0, 120.0]],
    );

    // The camera is the CSS a World would otherwise write by hand: the perspective toward the
    // center and the inverse of the orbit and depth around it, inside the 2D camera.
    let planes = card("transform:'rotateY(20deg)'");
    let by_hand = red_corners(&plane_scene(
        "camera={{center:point(150,80),zoom:1.5,rotation:10}}",
        &format!(
            r#"<World><View key='lens' style={{{{position:'absolute',left:0,top:0,width:320,height:180,
                perspective:'500px',perspectiveOrigin:'150px 80px'}}}}>
                <View key='orbit' style={{{{position:'absolute',left:0,top:0,width:320,height:180,
                    transformStyle:'preserve-3d',transformOrigin:'150px 80px',
                    transform:'rotateX(-15deg) rotateY(25deg) translateZ(40px)'}}}}>{planes}</View>
            </View></World>"#
        ),
    ));
    assert_corners(
        &plane_scene(
            "camera={{center:point(150,80),zoom:1.5,rotation:10,perspective:500,z:-40,rotateX:15,rotateY:-25}}",
            &format!("<World>{planes}</World>"),
        ),
        by_hand,
    );
}
