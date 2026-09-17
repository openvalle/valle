#![cfg(feature = "motion")]

use std::collections::BTreeMap;
use valle_compiler::motion::compile_motion;
use valle_motion::layout::LayoutCache;
use valle_motion::{
    Fonts, LayoutOptions, ResolvedSignals, StyleCache, Viewport, build_tree, default_font_naming,
    emit, motion_context_at, phase_windows, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

fn source(attributes: &str) -> String {
    format!(
        r#"export default function Demo(ctx) {{return <Scene style={{{{width:320,height:180}}}}>
        <View key="card" {attributes}><View key="child" style={{{{position:'fixed',left:5,top:7,
        width:20,height:10,backgroundColor:'blue'}}}}/></View></Scene>;}}"#
    )
}
fn class_attr(classes: &str) -> String {
    format!("className={{{}}}", serde_json::to_string(classes).unwrap())
}

fn snapshots(source: &str, frames: &[u32]) -> Vec<(BTreeMap<String, [f32; 4]>, Vec<u8>)> {
    let artifact = compile_motion(source)
        .unwrap_or_else(|d| panic!("{source}: {d:#?}"))
        .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let mut fonts = Fonts::default();
    valle_motion::register_default_motion_fonts(&mut fonts).unwrap();
    // This named family is an explicit test asset, outside the default font pack.
    fonts
        .register(valle_motion::motion_font_resource(
            include_bytes!("../../../assets/fonts/katex/KaTeX_SansSerif-Regular.ttf").to_vec(),
        ))
        .unwrap();
    let styles = StyleCache::new();
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    let windows = phase_windows(&artifact.controls.phase_spec(), 90);
    frames
        .iter()
        .map(|&frame| {
            let ctx = motion_context_at(frame, &windows, FrameRate::new(30, 1).unwrap()).unwrap();
            let render = |styles| {
                let tree = build_tree(
                    &prepared,
                    &ctx,
                    &props,
                    &ResolvedSignals::default(),
                    &LayoutOptions {
                        viewport: Viewport::new((320, 180)),
                        fonts: &fonts,
                        styles,
                    },
                )
                .unwrap();
                let report = emit(&tree, &default_font_naming).unwrap();
                assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
                (
                    valle_motion::layout::layout_boxes(&artifact, &tree).unwrap(),
                    report.program.packed_bytes().unwrap(),
                )
            };
            let cold = render(None);
            assert_eq!(cold, render(Some(&styles)), "frame {frame}");
            cold
        })
        .collect()
}

#[test]
fn arbitrary_properties_share_css_geometry_paint_and_transform_semantics() {
    for (class, property, value) in [
        ("[width:123px]", "width", "123px"),
        ("[width:calc(100%-20px)]", "width", "calc(100% - 20px)"),
        ("[padding:3px_7px]", "padding", "3px 7px"),
        (
            "[margin-left:calc(2e-1*20px)]",
            "marginLeft",
            "calc(2e-1 * 20px)",
        ),
        ("[aspect-ratio:4/3]", "aspectRatio", "4/3"),
        ("[border:2px_solid_red]", "border", "2px solid red"),
        (
            "[background-image:linear-gradient(90deg,red,blue)]",
            "backgroundImage",
            "linear-gradient(90deg,red,blue)",
        ),
        (
            "[transform:scale(1.2)_translateX(4px)]",
            "transform",
            "scale(1.2) translateX(4px)",
        ),
        ("[translate:10%_20%]", "translate", "10% 20%"),
        ("[rotate:10deg]", "rotate", "10deg"),
        ("[scale:1]", "scale", "1"),
        ("[scale:120%_80%]", "scale", "120% 80%"),
        (
            "[filter:blur(2px)_brightness(1.2)]",
            "filter",
            "blur(2px) brightness(1.2)",
        ),
        ("[backdrop-filter:blur(2px)]", "backdropFilter", "blur(2px)"),
        ("[box-shadow:0_2px_3px_#000]", "boxShadow", "0 2px 3px #000"),
    ] {
        let base = "position:'absolute',left:30,top:20,height:80,backgroundColor:'red'";
        let authored = source(&format!("{} style={{{{{base}}}}}", class_attr(class)));
        let reference = source(&format!(
            "style={{{{{base},{property}:{}}}}}",
            serde_json::to_string(value).unwrap()
        ));
        assert_eq!(
            snapshots(&authored, &[60, 0, 30, 60]),
            snapshots(&reference, &[60, 0, 30, 60]),
            "{class}"
        );
    }
}

#[test]
fn arbitrary_conflicts_follow_deterministic_order_and_css_importance() {
    for (classes, inline, expected) in [
        ("[width:123px] w-40", "", "width:160"),
        ("[width:123px]! w-40", "width:90", "width:123"),
        ("[width:123px!important] w-40", "width:90", "width:123"),
        ("[width:123px] w-40", "width:90", "width:90"),
        (
            "[padding:12px] p-4 [padding-left:4px]",
            "",
            "padding:'16px 16px 16px 4px'",
        ),
        (
            // The longhand utility wins the width under the documented rule (a property outside the
            // canonical order contributes no key and sorts before the longhands), while the
            // arbitrary shorthand keeps style and colour.
            "[border:2px_solid_red] border-4 border-blue-500",
            "",
            "border:'4px solid red'",
        ),
        ("[scale:1]! scale-none", "scale:'none'", "scale:1"),
        ("[scale:none]! scale-100", "scale:1", "scale:'none'"),
    ] {
        let base = "position:'absolute',left:30,top:20,height:80,backgroundColor:'red'";
        let expected = snapshots(&source(&format!("style={{{{{base},{expected}}}}}")), &[0]);
        for classes in [
            classes.to_owned(),
            classes
                .split_whitespace()
                .rev()
                .collect::<Vec<_>>()
                .join(" "),
        ] {
            let actual = source(&format!(
                "{} style={{{{{base},{inline}}}}}",
                class_attr(&classes)
            ));
            assert_eq!(snapshots(&actual, &[0]), expected, "{classes}");
        }
    }
}

#[test]
fn finite_arbitrary_property_choices_seek_directly_and_control_geometry_reuse() {
    let choice = source(
        "className={ctx.localFrame<30?'[width:80px] [filter:blur(0px)]':'[width:140px] [filter:blur(2px)]'} style={{height:80,backgroundColor:'red'}}",
    );
    let direct = source(
        "style={{width:ctx.localFrame<30?80:140,height:80,backgroundColor:'red',filter:ctx.localFrame<30?'blur(0px)':'blur(2px)'}}",
    );
    assert_eq!(
        snapshots(&choice, &[60, 0, 29, 30, 89, 60, 0]),
        snapshots(&direct, &[60, 0, 29, 30, 89, 60, 0])
    );
    for (classes, reusable) in [
        ("[width:120px] [scale:1]", true),
        ("[filter:blur(0px)]", false),
        ("[transform:translateX(0px)]", false),
        ("[backdrop-filter:blur(2px)]", false),
    ] {
        let artifact = compile_motion(&source(&class_attr(classes)))
            .unwrap()
            .artifact;
        assert_eq!(
            artifact.reads_destination(),
            classes.contains("backdrop-filter")
        );
        let prepared = prepare_scene(&artifact).unwrap();
        let fonts = Fonts::default();
        let cache = LayoutCache::new(&prepared, &fonts);
        assert_eq!(cache.is_some(), reusable, "{classes}");
        let Some(cache) = cache else {
            continue;
        };
        let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
        let windows = phase_windows(&artifact.controls.phase_spec(), 90);
        let ctx = motion_context_at(0, &windows, FrameRate::new(30, 1).unwrap()).unwrap();
        for repeat in 0..2 {
            let (_, timing) = cache
                .build_tree_profiled(
                    &ctx,
                    &props,
                    &ResolvedSignals::default(),
                    Viewport::new((320, 180)),
                    None,
                )
                .unwrap();
            assert_eq!(timing.layout_reused, reusable && repeat > 0, "{classes}");
        }
    }
}

#[test]
fn arbitrary_values_cannot_bypass_shared_admission_in_unselected_branches_or_artifacts() {
    for class in [
        "[mask-size:cover]",
        "[unknown-property:1]",
        "[animation:spin_1s]",
        "[--brand:red]",
        "[width:var(--missing)]",
        "[width:12px;height:200px]",
        "[width:12px]junk",
        "[width:calc(100%-20px]",
        "[font-family:'broken]",
        "[grid-template-columns:1fr_nonsense]",
        "[background-image:url(https://example.com/a_b.png)]",
        "[background-image:linear-gradient(in_oklab,red,blue)]",
        "[background-image:linear-gradient(red,blue),url(a.png)]",
        "[filter:blur(-2px)]",
        "[motion-transform-3d-scale-x:2]",
        "[perspective:100px]",
        "[transform:matrix3d(1)]",
        "hover:[color:red]",
        "[&:hover]:[color:red]",
        "[opacity:]",
    ] {
        let conditional = source(&format!(
            "className={{ctx.localFrame<30?'w-20':{}}}",
            serde_json::to_string(class).unwrap()
        ));
        assert!(compile_motion(&conditional).is_err(), "{class}");
        let mut artifact = compile_motion(&source("")).unwrap().artifact;
        artifact.nodes[0].class_names.push(class.into());
        assert!(artifact.validate().is_err(), "{class}");
        assert!(prepare_scene(&artifact).is_err(), "{class}");
    }
}

#[test]
fn quoted_font_names_and_finite_font_choices_use_registered_fonts() {
    let text = |attribute: &str| {
        format!(
            r#"export default function Demo(ctx) {{return <Scene style={{{{width:320,height:180}}}}><Text key="label" {attribute} style={{{{fontSize:30,color:'red'}}}}>Motion<Span> AB</Span></Text></Scene>;}}"#
        )
    };
    let frames = [60, 0, 29, 30, 60, 0];
    let classes = r#"className={ctx.localFrame<30?"[font-family:'KaTeX\\_Main']":"[font-family:'KaTeX\\_SansSerif']"}"#;
    let actual = snapshots(&text(classes), &frames);
    for (i, &frame) in frames.iter().enumerate() {
        let family = if frame < 30 {
            "KaTeX_Main"
        } else {
            "KaTeX_SansSerif"
        };
        let reference =
            text("").replace("fontSize:30", &format!("fontFamily:'{family}',fontSize:30"));
        assert_eq!(actual[i], snapshots(&reference, &[frame])[0]);
    }
    assert_ne!(
        actual[0], actual[1],
        "different registered fonts must reach real glyph output"
    );
}

#[test]
fn generic_font_utilities_select_fixed_faces_and_follow_the_current_frame() {
    let source = |attributes: &str| {
        format!(
            r#"export default function Demo(ctx) {{return <Scene style={{{{width:320,height:180}}}}>
                <View {attributes}><Text key="label" style={{{{fontSize:28,color:'red'}}}}>WiMi 0123<Span> AB</Span></Text></View>
            </Scene>;}}"#
        )
    };
    let mut families = Vec::new();
    for (class, family) in [
        ("font-sans", "sans-serif"),
        ("font-serif", "serif"),
        ("font-mono", "monospace"),
    ] {
        let expected = snapshots(
            &source(&format!("style={{{{fontFamily:'{family}'}}}}")),
            &[0],
        );
        let actual = snapshots(&source(&class_attr(class)), &[0]);
        assert!(
            actual == expected,
            "{class}: actual {:?}, expected {:?}",
            actual[0].0,
            expected[0].0
        );
        families.push(expected);
    }
    assert_ne!(families[0], families[1]);
    assert_ne!(families[1], families[2]);
    assert_ne!(families[0], families[2]);

    for (class, family) in [
        ("font-mono font-serif", 1),
        ("font-serif font-mono", 1),
        ("font-serif!", 1),
        ("font-mono! font-serif", 2),
        ("font-mono! font-serif!", 1),
        ("[font-family:ui-serif,_serif]!", 1),
        ("[font-family:serif!important] font-mono", 1),
    ] {
        let inline = if class.ends_with('!') {
            " style={{fontFamily:'sans-serif'}}"
        } else {
            ""
        };
        assert_eq!(
            snapshots(&source(&format!("{}{inline}", class_attr(class))), &[0]),
            families[family],
            "{class}"
        );
    }
    let frames = [60, 0, 29, 30, 60, 0];
    let actual = snapshots(
        &source("className={ctx.localFrame<30?'font-serif!':'font-mono'}"),
        &frames,
    );
    for (index, &frame) in frames.iter().enumerate() {
        assert_eq!(actual[index], families[if frame < 30 { 1 } else { 2 }][0]);
    }
}

#[test]
fn arbitrary_inline_display_keeps_replaced_media_atomic_and_retains_rule_order() {
    let source = |classes: &str, style: &str| {
        format!(
            r#"
        export const controls=defineControls({{assets:{{dot:asset({{kind:'image'}})}}}});
        export default function Demo() {{return <Scene style={{{{width:320,height:180}}}}>
            <Text key="prefix" style={{{{display:'inline',fontSize:20}}}}>A</Text>
            <Image key="image" src="asset://dot" {} style={{{{width:40,height:24,{style}}}}}/>
            <View key="following" style={{{{width:12,height:12,backgroundColor:'red'}}}}/>
        </Scene>;}}
    "#,
            class_attr(classes)
        )
    };
    let reference = snapshots(&source("", "display:'inline'"), &[0]);
    assert_eq!(reference[0].0["image"][2], 40.0);
    for class in [
        "[display:inline]",
        "[display:inline]!",
        "[display:in/**/line]",
        "inline",
    ] {
        // A comment cannot split an identifier; use a separate token comment instead.
        if class.contains("in/**/line") {
            assert!(compile_motion(&source(class, "")).is_err());
            continue;
        }
        assert_eq!(snapshots(&source(class, ""), &[0]), reference, "{class}");
    }
    let block = snapshots(&source("", "display:'block'"), &[0]);
    for class in [
        "[display:inline] [display:block]",
        "[display:block] [display:inline]",
    ] {
        // The same-property arbitrary candidates use lexical rule order: inline wins.
        assert_eq!(snapshots(&source(class, ""), &[0]), reference, "{class}");
    }
    assert_ne!(reference, block);
}

#[test]
fn rich_inline_image_defaults_and_bounds_follow_current_frame_choices() {
    let source = |classes: &str, bound: &str| {
        format!(
            r#"
        export const controls=defineControls({{assets:{{dot:asset({{kind:'image'}})}}}});
        export default function Demo(ctx) {{return <Scene style={{{{width:320,height:180}}}}>
            <Text key="label" style={{{{fontSize:20,lineHeight:'30px'}}}}>A<Image key="image" src="asset://dot"
                className={{{classes}}} style={{{{width:40,height:24}}}}/>B</Text>
            <View key="follower" style={{{{position:'absolute',left:0,top:120,width:10,height:10,
                backgroundColor:'blue',transform:`translateX(${{{bound}}}px)`}}}}/>
        </Scene>;}}
    "#
        )
    };
    let requested = [60, 0, 29, 30, 60, 0];
    let actual = snapshots(
        &source(
            "ctx.localFrame<30?'[display:block]':''",
            "bounds('image').width",
        ),
        &requested,
    );
    for (index, &frame) in requested.iter().enumerate() {
        let class = if frame < 30 {
            "'[display:block]'"
        } else {
            "''"
        };
        let expected = snapshots(&source(class, "40"), &[frame]);
        assert_eq!(actual[index].0["image"][2], 40.0);
        assert_eq!(actual[index], expected[0], "frame {frame}");
    }
    assert_ne!(
        actual[0], actual[1],
        "inline and block images must produce different text flow"
    );
}
