#![cfg(feature = "motion")]

use std::collections::BTreeMap;
use valle_compiler::motion::compile_motion;
use valle_draw::program::{DrawProgram, Filter, Node};
use valle_motion::{
    Fonts, LayoutOptions, MotionValue, ResolvedSignals, StyleBinding, StyleCache, StyleValue,
    Viewport, build_tree, default_font_naming, emit, motion_context_at, phase_windows,
    prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

fn scene(attributes: &str, styles: &str) -> String {
    format!(
        r##"export default function Demo(ctx) {{ return <Scene style={{{{width:320,height:180}}}}>
        <View key="card" {attributes} style={{{{width:120,height:80,borderWidth:4,padding:5,
            backgroundColor:"#168ddd",borderColor:"#fff",{styles}}}}}>
            <View key="child" style={{{{width:20,height:10,backgroundColor:"red"}}}} />
        </View></Scene>; }}"##
    )
}

fn samples(source: &str, frames: &[u32]) -> Vec<DrawProgram> {
    let artifact = compile_motion(source)
        .unwrap_or_else(|d| panic!("{source}: {d:#?}"))
        .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let fonts = Fonts::default();
    let cache = StyleCache::new();
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
                report.program
            };
            let cold = render(None);
            assert_eq!(
                cold.packed_bytes().unwrap(),
                render(Some(&cache)).packed_bytes().unwrap(),
                "frame {frame}"
            );
            cold
        })
        .collect()
}

fn snapshot(classes: &str, style: &str) -> DrawProgram {
    samples(&scene(&format!("className=\"{classes}\""), style), &[0]).remove(0)
}

fn filters(program: &DrawProgram) -> Vec<Filter> {
    program
        .nodes()
        .iter()
        .flat_map(|node| match node {
            Node::Group(group) => group.filters.clone(),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn visual_utilities_match_inline_properties_and_class_permutations() {
    for (classes, style) in [
        ("translate-x-4 -translate-y-2", "translate:'16px -8px'"),
        ("-translate-x-1/2 translate-y-[25%]", "translate:'-50% 25%'"),
        ("translate-4 translate-x-2", "translate:'8px 16px'"),
        (
            "-translate-x-[calc(50%_-_10px)]",
            "translate:'calc(10px - 50%) 0px'",
        ),
        (
            "rotate-45 scale-x-50 scale-y-150 origin-top-left",
            "rotate:'45deg',scale:point(0.5,1.5),transformOrigin:'0% 0%'",
        ),
        (
            "-rotate-[0.25turn] -scale-x-50 origin-[20%_40%]",
            "rotate:'-90deg',scale:point(-0.5,1),transformOrigin:'20% 40%'",
        ),
        ("scale-110 scale-x-50", "scale:point(0.5,1.1)"),
        ("scale-x-50 scale-[1.25]", "scale:1.25"),
        ("scale-[1.25_0.5]", "scale:point(1.25,0.5)"),
        ("-scale-[1.25]", "scale:-1.25"),
        (
            "blur-sm brightness-125 contrast-75 grayscale-20 -hue-rotate-30 invert-10 saturate-150 sepia-20",
            "filter:'blur(8px) brightness(125%) contrast(75%) grayscale(20%) hue-rotate(-30deg) invert(10%) saturate(150%) sepia(20%)'",
        ),
        (
            "drop-shadow-sm",
            "filter:'drop-shadow(0 1px 2px rgb(0 0 0 / 0.15))'",
        ),
        (
            "drop-shadow-[2px_4px_3px_#000] blur-[3px]",
            "filter:'blur(3px) drop-shadow(2px 4px 3px #000)'",
        ),
        (
            "filter-[brightness(1.25)_blur(3px)]",
            "filter:'brightness(1.25) blur(3px)'",
        ),
        (
            "backdrop-blur-md backdrop-brightness-125 backdrop-opacity-50 backdrop-saturate-150",
            "backdropFilter:'blur(12px) brightness(125%) opacity(50%) saturate(150%)'",
        ),
        (
            "backdrop-filter-[contrast(0.75)_blur(3px)]",
            "backdropFilter:'contrast(0.75) blur(3px)'",
        ),
    ] {
        let actual = snapshot(classes, "");
        assert_eq!(actual, snapshot("", style), "{classes}");
        let reverse = classes
            .split_whitespace()
            .rev()
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(actual, snapshot(&reverse, ""), "permutation {classes}");
    }
}

#[test]
fn resets_and_importance_resolve_before_effect_emission() {
    for (classes, style, expected) in [
        ("blur-sm", "filter:'none'", ""),
        ("blur-sm", "filter:'blur(0px)'", "filter:'blur(0px)'"),
        ("blur-sm filter-none", "", ""),
        (
            "blur-[3px] blur-none brightness-125",
            "",
            "filter:'brightness(125%)'",
        ),
        ("drop-shadow-sm drop-shadow-none", "", ""),
        (
            "blur-sm! filter-none",
            "filter:'none'",
            "filter:'blur(8px)'",
        ),
        (
            "blur-sm! blur-lg brightness-125",
            "",
            "filter:'blur(8px) brightness(125%)'",
        ),
        ("blur-sm! blur-none!", "", "filter:'blur(8px)'"),
        ("blur-[3px]! blur-none!", "", ""),
        ("blur-sm filter-none!", "", ""),
        ("backdrop-blur-md", "backdropFilter:'none'", ""),
        (
            "backdrop-blur-[3px] backdrop-blur-none backdrop-brightness-125",
            "",
            "backdropFilter:'brightness(125%)'",
        ),
        ("backdrop-blur-md backdrop-filter-none", "", ""),
        ("translate-x-4 translate-none", "", ""),
        ("scale-x-50 scale-none", "", ""),
        ("rotate-45 rotate-none", "", ""),
        (
            "translate-x-4!",
            "translate:'0px 0px'",
            "translate:'16px 0px'",
        ),
        ("scale-x-50!", "scale:1", "scale:point(0.5,1)"),
        ("scale-150 scale-x-50!", "", "scale:point(0.5,1.5)"),
    ] {
        assert_eq!(
            snapshot(classes, style),
            snapshot("", expected),
            "{classes} / {style}"
        );
    }
}

#[test]
fn identity_filters_keep_containing_blocks_and_composition_slots_do_not_inherit() {
    let positioned = |style: &str| {
        scene("", style)
            .replace(
                "width:120,height:80",
                "marginLeft:40,marginTop:20,width:120,height:80",
            )
            .replace(
                "width:20,height:10",
                "position:'fixed',left:0,top:0,width:20,height:10",
            )
    };
    let zero = samples(&positioned("filter:'blur(0px)'"), &[0]).remove(0);
    let none = samples(&positioned("filter:'none'"), &[0]).remove(0);
    assert_ne!(zero.packed_bytes().unwrap(), none.packed_bytes().unwrap());
    // Child effect slots start empty; the parent's blur is applied only to its subtree group.
    let with_class = scene("className=\"blur-sm\"", "").replace(
        "key=\"child\"",
        "key=\"child\" className=\"brightness-125\"",
    );
    let with_style = scene("", "filter:'blur(8px)'").replace(
        "width:20,height:10",
        "filter:'brightness(125%)',width:20,height:10",
    );
    assert_eq!(samples(&with_class, &[0]), samples(&with_style, &[0]));
    let transform_class = scene("className=\"translate-x-4 scale-x-50\"", "").replace(
        "key=\"child\"",
        "key=\"child\" className=\"translate-y-2 scale-y-150\"",
    );
    let transform_style = scene("", "translate:'16px 0px',scale:point(0.5,1)").replace(
        "width:20,height:10",
        "translate:'0px 8px',scale:point(1,1.5),width:20,height:10",
    );
    assert_eq!(
        samples(&transform_class, &[0]),
        samples(&transform_style, &[0])
    );
}

#[test]
fn computed_negative_blur_fails_at_the_requested_frame() {
    // Mixed-unit calc has a valid structure but its sign depends on the actual viewport.
    let artifact = compile_motion(&scene("", "filter:'blur(calc(1vw - 4px))'"))
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
        .expect("negative resolved blur must fail");
    assert!(error.to_string().contains("sigma"), "{error}");
}

#[test]
fn css_filter_lengths_are_sigma_and_large_bounded_amounts_clamp() {
    let actual = snapshot("blur-[6px] drop-shadow-[2px_4px_3px_#000]", "");
    let filters = filters(&actual);
    assert!(
        filters
            .iter()
            .any(|filter| matches!(filter, Filter::Blur { sigma_x, sigma_y } if *sigma_x == 6.0 && *sigma_y == 6.0))
    );
    assert!(
        filters
            .iter()
            .any(|filter| matches!(filter, Filter::DropShadow { sigma_x, sigma_y, .. } if *sigma_x == 3.0 && *sigma_y == 3.0))
    );
    assert_eq!(
        snapshot("grayscale-200 invert-200 sepia-200", ""),
        snapshot("grayscale invert sepia", "")
    );
    let translated = snapshot("-translate-x-1/2 translate-y-[25%]", "");
    assert!(
        translated.nodes().iter().any(|node| matches!(node,
            Node::Group(group) if group.transform.0 == [1.0,0.0,-60.0,0.0,1.0,20.0,0.0,0.0,1.0]
        )),
        "translation must use the 120 by 80 border box, not its content or viewport"
    );
}

#[test]
fn finite_effect_choices_seek_without_state_and_mark_destination_reads() {
    let source = scene(
        r#"className={ctx.localFrame < 30 ? 'translate-x-4 blur-sm' : '-translate-y-1/2 backdrop-blur-md'}"#,
        "",
    );
    let frames = [60, 0, 29, 30, 12, 60, 0];
    let actual = samples(&source, &frames);
    for (i, frame) in frames.into_iter().enumerate() {
        let class = if frame < 30 {
            "translate-x-4 blur-sm"
        } else {
            "-translate-y-1/2 backdrop-blur-md"
        };
        assert_eq!(actual[i], snapshot(class, ""));
    }
    let artifact = compile_motion(&source).unwrap().artifact;
    assert!(artifact.reads_destination());
    assert!(!prepare_scene(&artifact).unwrap().can_reuse_layout());
    let foreground = compile_motion(&scene("className=\"blur-sm\"", ""))
        .unwrap()
        .artifact;
    assert!(!foreground.reads_destination());
    assert!(!prepare_scene(&foreground).unwrap().can_reuse_layout());
    assert!(
        prepare_scene(
            &compile_motion(&scene("className=\"translate-x-4\"", ""))
                .unwrap()
                .artifact
        )
        .unwrap()
        .can_reuse_layout()
    );
}

#[test]
fn invalid_effects_reject_in_source_classes_and_direct_artifacts() {
    for class in [
        "blur-bogus",
        "blur-[-2px]",
        "blur-[10%]",
        "blur-[auto]",
        "blur-[var(--missing)]",
        "brightness-[-1]",
        "drop-shadow-[0_0_-2px_red]",
        "filter-[blur(3px)_junk]",
        "filter-[blur(3px);opacity:0]",
        "filter-[url(#bad)]",
        "-blur-sm",
        "scale-110.5",
        "rotate-12.5",
        "translate-x-[1px_2px]",
        "scale-[NaN]",
        "scale-[1e100]",
        "backdrop-invert-[-1]",
        "rotate-x-45",
    ] {
        assert!(
            compile_motion(&scene(&format!("className=\"{class}\""), "")).is_err(),
            "{class}"
        );
    }
    for value in [
        "",
        "blur(-2px)",
        "blur(10%)",
        "blur(auto)",
        "brightness(-1)",
        "blur(1e100px)",
        "drop-shadow(red 0 0 -2px)",
        "blur(calc(10% + 2px))",
    ] {
        assert!(
            compile_motion(&scene(
                "",
                &format!("filter:{}", serde_json::to_string(value).unwrap())
            ))
            .is_err(),
            "{value}"
        );
        let mut artifact = compile_motion(&scene("", "")).unwrap().artifact;
        artifact.nodes[1].styles.push(StyleBinding {
            property: "filter".into(),
            value: StyleValue::Static {
                value: MotionValue::Str(value.into()),
            },
        });
        assert!(artifact.validate().is_err(), "direct artifact {value}");
    }
}
