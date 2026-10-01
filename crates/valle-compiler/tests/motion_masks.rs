#![cfg(feature = "motion")]

use std::collections::BTreeMap;
use valle_compiler::motion::compile_motion;
use valle_draw::program::{DrawProgram, MaskMode, Node};
use valle_motion::{
    Fonts, LayoutOptions, Viewport, build_tree, default_font_naming, emit, motion_context_at_frame,
    prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

fn program(body: &str) -> DrawProgram {
    let source = format!(
        r#"export default function MaskTest(ctx) {{
        return <Scene style={{{{width:64,height:64}}}}>{body}</Scene>;
    }}"#
    );
    let artifact = compile_motion(&source)
        .unwrap_or_else(|e| panic!("{e:#?}"))
        .artifact;
    let scene = prepare_scene(&artifact).unwrap();
    let fonts = Fonts::default();
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    let ctx = motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let tree = build_tree(
        &scene,
        &ctx,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((64, 64)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    emit(&tree, &default_font_naming).unwrap().program
}

#[test]
fn subtree_masks_are_independent_of_source_order_and_support_all_channels() {
    let source = r##"<MaskSource key="source"><Circle key="circle" cx={32} cy={32} r={16} fill="#fff" /></MaskSource>"##;
    let content = r##"<View key="content" style={{width:64,height:64,backgroundColor:"#00f"}}/>"##;
    for (channel, invert, expected) in [
        ("alpha", false, MaskMode::Alpha),
        ("luminance", false, MaskMode::Luminance),
        ("alpha", true, MaskMode::AlphaInverted),
        ("luminance", true, MaskMode::LuminanceInverted),
    ] {
        let render = |body: String| {
            program(&format!(
                r#"<Mask key="mask" rect={{rect(0,0,64,64)}} mode="{channel}" invert={{{invert}}}
                style={{{{width:64,height:64}}}}>{body}</Mask>"#
            ))
        };
        let first = render(format!("{source}{content}"));
        let last = render(format!("{content}{source}"));
        assert_eq!(first.packed_bytes().unwrap(), last.packed_bytes().unwrap());
        let masks: Vec<_> = first
            .nodes()
            .iter()
            .filter_map(|node| match node {
                Node::Group(group) => group.mask.as_ref(),
                _ => None,
            })
            .collect();
        assert_eq!(masks.len(), 1);
        assert_eq!(masks[0].mode, expected);
        assert!(matches!(
            &first.nodes()[masks[0].source.raw() as usize],
            Node::Group(_)
        ));
        first.validate().unwrap();
    }
}

#[test]
fn nested_and_absent_mask_sources_remain_structured() {
    let nested = program(
        r##"<Mask key="outer" rect={rect(0,0,64,64)}>
        <View style={{width:64,height:64,backgroundColor:"#00f"}}/>
        <MaskSource key="outer-source"><Mask key="inner" rect={rect(0,0,64,64)} mode="luminance">
            <Circle cx={32} cy={32} r={24} fill="#fff"/>
            <MaskSource key="inner-source"><Circle cx={32} cy={32} r={16} fill="#fff"/></MaskSource>
        </Mask></MaskSource></Mask>"##,
    );
    assert_eq!(
        nested
            .nodes()
            .iter()
            .filter(|node| matches!(node, Node::Group(g) if g.mask.is_some()))
            .count(),
        2
    );
    for visibility in [r#"style={{display:"none"}}"#, "visible={false}", ""] {
        program(&format!(
            r##"<Mask key="mask" rect={{rect(0,0,64,64)}}>
            <View style={{{{width:64,height:64,backgroundColor:"#00f"}}}}/>
            <MaskSource key="source" {visibility}/></Mask>"##
        ));
    }
}
