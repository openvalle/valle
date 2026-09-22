#![cfg(feature = "motion")]

use std::collections::BTreeMap;
use valle_compiler::motion::compile_motion;
use valle_motion::{
    Fonts, LayoutOptions, MotionValue, StyleBinding, StyleCache, StyleValue, Viewport, build_tree,
    default_font_naming, emit, motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

fn scene(attributes: &str) -> String {
    format!(
        r#"export default function Demo(ctx) {{ return <Scene style={{{{width:320,height:180}}}}>
        <View key="grid" {attributes}><View key="first" style={{{{height:20,backgroundColor:"red"}}}} />
        <View key="second" style={{{{height:20,backgroundColor:"blue"}}}} /></View></Scene>; }}"#
    )
}

#[test]
fn unsupported_intrinsic_utilities_and_inactive_css_animations_fail_early() {
    for class in [
        "w-min",
        "w-max",
        "w-fit",
        "h-fit",
        "min-w-min",
        "max-h-max",
        "basis-fit",
        "size-fit",
        "auto-cols-min",
        "auto-rows-max",
        "overflow-auto",
        "overflow-scroll",
    ] {
        assert!(
            compile_motion(&scene(&format!("className=\"{class}\""))).is_err(),
            "{class}"
        );
    }
    for style in [
        r#"width: "min-content""#,
        r#"animation: "spin 1s linear infinite""#,
        r#"animationDuration: "1s""#,
    ] {
        assert!(
            compile_motion(&scene(&format!("style={{{{{style}}}}}"))).is_err(),
            "{style}"
        );
    }
}

#[test]
fn compiler_and_direct_artifacts_reject_invalid_css_values() {
    for (property, value) in [
        ("grid-template-columns", "subgrid"),
        ("grid-template-columns", "1fr nonsense"),
        ("grid-auto-rows", "10px nonsense"),
        ("width", "100px; height: 200px"),
        ("unknown-property", "1"),
    ] {
        let style = format!(
            "style={{{{\"{property}\":{}}}}}",
            serde_json::to_string(value).unwrap()
        );
        assert!(
            compile_motion(&scene(&style)).is_err(),
            "{property}: {value}"
        );
        let mut artifact = compile_motion(&scene("")).unwrap().artifact;
        artifact
            .nodes
            .iter_mut()
            .find(|node| node.key == "grid")
            .unwrap()
            .styles
            .push(StyleBinding {
                property: property.into(),
                value: StyleValue::Static {
                    value: MotionValue::Str(value.into()),
                },
            });
        let errors = artifact
            .validate()
            .expect_err("direct artifact must share admission");
        assert!(
            errors
                .iter()
                .any(|error| error.to_string().contains(property))
        );
        assert!(prepare_scene(&artifact).is_err());
    }
    assert!(
        compile_motion(&scene(
            r#"style={{gridTemplateColumns: ctx.localFrame < 30 ? "1fr 2fr" : "subgrid"}}"#
        ))
        .is_err()
    );
}

#[test]
fn grid_geometry_and_programs_are_identical_for_cold_warm_and_reverse_seeks() {
    let artifact = compile_motion(&scene(
        r#"style={{display:"grid",gridTemplateColumns:"1fr 2fr",gap:20,
        width:320,translate:point(ctx.localFrame,0)}}"#,
    ))
    .unwrap()
    .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let fonts = Fonts::default();
    let cache = StyleCache::new();
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    for frame in [0, 30, 12, 59, 0, 30] {
        let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let render = |styles| {
            let tree = build_tree(
                &prepared,
                &ctx,
                &props,
                &LayoutOptions {
                    viewport: Viewport::new((320, 180)),
                    fonts: &fonts,
                    styles,
                },
            )
            .unwrap();
            let boxes = valle_motion::layout::layout_boxes(&artifact, &tree).unwrap();
            let first = boxes["first"];
            let second = boxes["second"];
            assert!((first[2] - 100.0).abs() < 0.001, "{first:?}");
            assert!((second[2] - 200.0).abs() < 0.001, "{second:?}");
            emit(&tree, &default_font_naming)
                .unwrap()
                .program
                .packed_bytes()
                .unwrap()
        };
        assert_eq!(render(None), render(Some(&cache)), "frame {frame}");
    }
    assert!(cache.stats().0 > 0);
}

fn snapshot(attributes: &str) -> (BTreeMap<String, [f32; 4]>, Vec<u8>) {
    let artifact = compile_motion(&scene(attributes))
        .unwrap_or_else(|d| panic!("{attributes}: {d:#?}"))
        .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let fonts = Fonts::default();
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    let ctx = motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
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
    (
        valle_motion::layout::layout_boxes(&artifact, &tree).unwrap(),
        emit(&tree, &default_font_naming)
            .unwrap()
            .program
            .packed_bytes()
            .unwrap(),
    )
}

#[test]
fn fraction_and_arbitrary_classes_match_inline_geometry_and_programs() {
    for (classes, style) in [
        ("w-1/2 h-[100px]", r#"width:"50%",height:100"#),
        ("w-[137px] h-1/2", r#"width:137,height:"50%""#),
        (
            "grid w-full grid-cols-[1fr_2fr] gap-[20px]",
            r#"display:"grid",width:"100%",gridTemplateColumns:"1fr 2fr",gap:20"#,
        ),
        (
            "w-[calc(100%_-_40px)] bg-[#123456]",
            r##"width:"calc(100% - 40px)",backgroundColor:"#123456""##,
        ),
        ("w-[calc(100%/2)]", r#"width:"calc(100%/2)""#),
        ("w-full max-w-3xs", r#"width:"100%",maxWidth:"16rem""#),
        ("size-[72px]", "width:72,height:72"),
        (
            "w-[160px] px-[13px] py-[7px]",
            "width:160,paddingInline:13,paddingBlock:7",
        ),
        (
            "w-[160px] ps-[17px] pe-[9px]",
            "width:160,paddingInlineStart:17,paddingInlineEnd:9",
        ),
        (
            "relative w-[80px] -left-[20px] -mt-[10px]",
            "position:'relative',width:80,left:-20,marginTop:-10",
        ),
        (
            "absolute inset-x-[12px] top-[10px] h-[40px]",
            "position:'absolute',insetInline:12,top:10,height:40",
        ),
        (
            "grid w-full grid-cols-2 gap-x-[12px] gap-y-[8px]",
            "display:'grid',width:'100%',gridTemplateColumns:'repeat(2,minmax(0,1fr))',columnGap:12,rowGap:8",
        ),
    ] {
        assert_eq!(
            snapshot(&format!("className=\"{classes}\"")),
            snapshot(&format!("style={{{{{style}}}}}")),
            "{classes}"
        );
    }
    for class in [
        "w-[137px_junk]",
        "grid-cols-[1fr_junk]",
        "w-1/0",
        "w-[100px;height:20px]",
        "w-[var(--missing)]",
        "-p-[12px]",
        "-w-[12px]",
        "-bg-[red]",
        "-z-auto",
        "order--2",
    ] {
        assert!(
            compile_motion(&scene(&format!("className=\"{class}\""))).is_err(),
            "{class}"
        );
    }
}

#[test]
fn utility_order_is_stable_and_important_beats_normal_inline() {
    let a = snapshot(r#"className="w-40 w-20""#);
    let b = snapshot(r#"className="w-20 w-40""#);
    assert_eq!(a, b);
    assert_eq!(a.0["grid"][2], 160.0);
    assert_eq!(
        snapshot(r#"className="w-40" style={{width:120}}"#).0["grid"][2],
        120.0
    );
    assert_eq!(
        snapshot(r#"className="w-40!" style={{width:120}}"#).0["grid"][2],
        160.0
    );
    assert_eq!(snapshot(r#"className="w-20! w-40""#).0["grid"][2], 80.0);
    assert_eq!(
        snapshot(r#"className="p-8 px-4 pl-2""#),
        snapshot(r#"className="pl-2 p-8 px-4""#)
    );
}

#[test]
fn default_border_paints_and_box_sizing_can_be_overridden() {
    let implicit = snapshot(r#"className="w-40 border-2 border-white p-4""#);
    let explicit =
        snapshot(r#"className="w-40 border-2 border-white p-4 border-solid box-border""#);
    assert_eq!(implicit, explicit);
    assert_eq!(implicit.0["grid"][2], 160.0);
    assert_ne!(
        implicit.1,
        snapshot(r#"className="w-40 border-2 border-white p-4 border-none""#).1
    );
    assert_eq!(
        snapshot(r#"className="w-40 border-2 border-white p-4 box-content""#).0["grid"][2],
        196.0
    );
}

#[test]
fn numeric_flex_and_order_classes_change_real_layout() {
    let source = |classes: bool| {
        format!(
            r#"export default function Demo() {{ return <Scene style={{{{display:"flex",width:300,height:80}}}}>
      <View key="a" {} /><View key="b" {} /></Scene>; }}"#,
            if classes {
                r#"className="flex-1 order-2 bg-red-500""#
            } else {
                r#"className="bg-red-500" style={{flex:"1",order:2}}"#
            },
            if classes {
                r#"className="flex-2 order-1 bg-blue-500""#
            } else {
                r#"className="bg-blue-500" style={{flex:"2",order:1}}"#
            }
        )
    };
    let render = |classes| {
        let artifact = compile_motion(&source(classes)).unwrap().artifact;
        let prepared = prepare_scene(&artifact).unwrap();
        let fonts = Fonts::default();
        let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
        let ctx = motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let tree = build_tree(
            &prepared,
            &ctx,
            &props,
            &LayoutOptions {
                viewport: Viewport::new((300, 80)),
                fonts: &fonts,
                styles: None,
            },
        )
        .unwrap();
        let boxes = valle_motion::layout::layout_boxes(&artifact, &tree).unwrap();
        assert_eq!(boxes["a"], [200.0, 0.0, 100.0, 80.0]);
        assert_eq!(boxes["b"], [0.0, 0.0, 200.0, 80.0]);
        emit(&tree, &default_font_naming)
            .unwrap()
            .program
            .packed_bytes()
            .unwrap()
    };
    assert_eq!(render(true), render(false));
}

#[test]
fn automatic_self_alignment_uses_parent_alignment() {
    let run = |attribute: &str| {
        let source = format!(
            r#"export default function Demo() {{ return <Scene style={{{{display:"flex",flexDirection:"column",alignItems:"center",width:300,height:100}}}}>
            <View key="item" {attribute} style={{{{width:80,height:20,backgroundColor:"red"}}}} /></Scene>; }}"#
        );
        let artifact = compile_motion(&source).unwrap().artifact;
        let prepared = prepare_scene(&artifact).unwrap();
        let fonts = Fonts::default();
        let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
        let ctx = motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let tree = build_tree(
            &prepared,
            &ctx,
            &props,
            &LayoutOptions {
                viewport: Viewport::new((300, 100)),
                fonts: &fonts,
                styles: None,
            },
        )
        .unwrap();
        valle_motion::layout::layout_boxes(&artifact, &tree).unwrap()["item"]
    };
    assert_eq!(run(r#"className="self-auto""#), [110.0, 0.0, 80.0, 20.0]);
    assert_eq!(run(r#"className="self-start""#), [0.0, 0.0, 80.0, 20.0]);
}
