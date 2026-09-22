#![cfg(feature = "motion")]

use std::collections::BTreeMap;
use valle_compiler::motion::compile_motion;
use valle_motion::diag::DiagClass;
use valle_motion::{
    Fonts, LayoutOptions, MotionValue, StyleBinding, StyleCache, StyleValue, Viewport, build_tree,
    default_font_naming, emit, motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

fn source(style: &str) -> String {
    format!(
        r#"export default function Demo(ctx) {{return <Scene style={{{{width:320,height:180}}}}>
    <View key="target" style={{{{position:'absolute',left:0,top:0,width:80+ctx.localFrame,height:20}}}}/>
    <View key="consumer" style={{{{position:'absolute',left:40,top:30,width:100,height:60,
        borderWidth:3,padding:8,backgroundColor:'red',{style}}}}}>
        <View key="child" style={{{{position:'fixed',left:5,top:7,width:20,height:10,backgroundColor:'blue'}}}}/>
    </View></Scene>;}}"#
    )
}

fn snapshots(source: &str, frames: &[u32]) -> Vec<(BTreeMap<String, [f32; 4]>, Vec<u8>)> {
    let artifact = compile_motion(source)
        .unwrap_or_else(|d| panic!("{source}: {d:#?}"))
        .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let mut fonts = Fonts::default();
    valle_motion::register_default_motion_fonts(&mut fonts).unwrap();
    let cache = StyleCache::new();
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    frames
        .iter()
        .map(|&frame| {
            let ctx = motion_context_at_frame(frame, 90, FrameRate::new(30, 1).unwrap()).unwrap();
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
                let report = emit(&tree, &default_font_naming).unwrap();
                assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
                (
                    valle_motion::layout::layout_boxes(&artifact, &tree).unwrap(),
                    report.program.packed_bytes().unwrap(),
                )
            };
            let cold = render(None);
            assert_eq!(cold, render(Some(&cache)), "frame {frame}");
            cold
        })
        .collect()
}

#[test]
fn unsupported_css_families_share_early_source_and_artifact_admission() {
    for property in [
        "clip-path",
        "mask-image",
        "mask-size",
        "mask-position",
        "mask-repeat",
        "mask-composite",
        "offset-path",
        "offset-distance",
        "offset-rotate",
        "offset-anchor",
        "border-image-source",
        "animation-duration",
        "transition-property",
    ] {
        let diagnostics = compile_motion(&source(&format!("'{property}':'initial'"))).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|d| d.class == DiagClass::Unsupported && d.message.contains(property)),
            "{diagnostics:?}"
        );
        let mut artifact = compile_motion(&source("")).unwrap().artifact;
        artifact.nodes[0].styles.push(StyleBinding {
            property: property.into(),
            value: StyleValue::Static {
                value: MotionValue::Str("initial".into()),
            },
        });
        assert!(
            artifact
                .validate()
                .unwrap_err()
                .iter()
                .any(|e| e.to_string().contains(property))
        );
        assert!(prepare_scene(&artifact).is_err());
    }
    let invalid = compile_motion(&source("unknownProperty:1")).unwrap_err();
    assert!(invalid.iter().any(|d| d.class == DiagClass::Illegal));
    assert!(compile_motion(&source("'motion-transform-3d-scale-x':2")).is_err());
}

#[test]
fn post_layout_css_uses_valid_probes_and_matches_direct_frame_expressions() {
    let requested = [60, 0, 29, 30, 89, 60, 0];
    for (bound, direct) in [
        (
            "transform:`translateX(${bounds('target').width/2}px)`",
            "transform:`translateX(${(80+ctx.localFrame)/2}px)`",
        ),
        (
            "filter:`blur(${bounds('target').width/40}px)`",
            "filter:`blur(${(80+ctx.localFrame)/40}px)`",
        ),
        (
            "backdropFilter:`blur(${bounds('target').width/40}px)`",
            "backdropFilter:`blur(${(80+ctx.localFrame)/40}px)`",
        ),
        (
            "backgroundColor:interpolate(bounds('target').width,[80,170],['red','blue'])",
            "backgroundColor:interpolate(80+ctx.localFrame,[80,170],['red','blue'])",
        ),
        (
            "boxShadow:`0px 0px ${bounds('target').width/20}px blue`",
            "boxShadow:`0px 0px ${(80+ctx.localFrame)/20}px blue`",
        ),
        (
            "backgroundImage:`linear-gradient(90deg, red 0px, blue ${bounds('target').width/2}px)`",
            "backgroundImage:`linear-gradient(90deg, red 0px, blue ${(80+ctx.localFrame)/2}px)`",
        ),
    ] {
        assert_eq!(
            snapshots(&source(bound), &requested),
            snapshots(&source(direct), &requested),
            "{bound}"
        );
    }
}

#[test]
fn post_layout_branches_must_preserve_containing_block_presence() {
    for property in ["transform", "filter", "backdropFilter"] {
        let list = if property == "transform" {
            "translateX(10px)"
        } else {
            "blur(1px)"
        };
        let other = if property == "transform" {
            "scale(1.2) translateX(4px)"
        } else {
            "brightness(0.8) blur(2px)"
        };
        for reset in ["none", "inherit", "initial"] {
            let diagnostics = compile_motion(&source(&format!(
                "{property}:bounds('target').width>100?'{reset}':'{list}'"
            )))
            .unwrap_err();
            assert!(
                diagnostics
                    .iter()
                    .any(|d| d.message.contains("post-layout")),
                "{diagnostics:?}"
            );
        }
        let bound = format!("{property}:bounds('target').width>100?'{list}':'{other}'");
        let direct = format!("{property}:80+ctx.localFrame>100?'{list}':'{other}'");
        assert_eq!(
            snapshots(&source(&bound), &[60, 0, 20, 21, 60, 0]),
            snapshots(&source(&direct), &[60, 0, 20, 21, 60, 0])
        );

        let bound = format!("{property}:bounds('target').width>100?'none':'initial'");
        assert_eq!(
            snapshots(&source(&bound), &[60, 0, 60]),
            snapshots(&source(&format!("{property}:'none'")), &[60, 0, 60])
        );
    }
}

#[test]
fn post_layout_css_checks_the_real_frame_after_a_valid_probe() {
    let artifact = compile_motion(&source("filter:`blur(${100-bounds('target').width}px)`"))
        .unwrap()
        .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let fonts = Fonts::default();
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    for frame in [0, 60, 20, 0] {
        let ctx = motion_context_at_frame(frame, 90, FrameRate::new(30, 1).unwrap()).unwrap();
        let tree = build_tree(
            &prepared,
            &ctx,
            &props,
            &LayoutOptions {
                viewport: Viewport::new((320, 180)),
                fonts: &fonts,
                styles: None,
            },
        );
        if frame == 60 {
            assert!(tree.is_err(), "negative blur must fail after a valid probe");
        } else {
            assert!(tree.is_ok(), "frame {frame}: {:?}", tree.err());
        }
    }
}

#[test]
fn post_layout_text_paint_reaches_real_glyphs_including_inherited_runs() {
    let text_source = |value: &str| {
        source("").replace("</Scene>", &format!(r#"<Text key="label" style={{{{position:'absolute',left:160,top:80,fontSize:20,color:{value}}}}}>A<Span>B</Span></Text></Scene>"#))
    };
    let requested = [60, 0, 30, 89, 0, 60];
    for (bound, direct) in [
        (
            "interpolate(bounds('target').width,[80,170],['red','blue'])",
            "interpolate(80+ctx.localFrame,[80,170],['red','blue'])",
        ),
        (
            "`rgb(${bounds('target').width} 20 40)`",
            "`rgb(${80+ctx.localFrame} 20 40)`",
        ),
    ] {
        assert_eq!(
            snapshots(&text_source(bound), &requested),
            snapshots(&text_source(direct), &requested)
        );
    }
    assert_ne!(
        snapshots(&text_source("'red'"), &[0]),
        snapshots(&text_source("'blue'"), &[0])
    );
}
