#![cfg(feature = "motion")]
use valle_draw::program::{AuthorColor, Node, Paint};
use valle_motion::{
    Fonts, LayoutOptions, Viewport, build_tree, default_font_naming, emit, motion_context_at_frame,
    prepare_scene, resolve_props,
};

fn render(style: &str) -> valle_draw::program::DrawProgram {
    let source = format!(
        "export default function T(){{return <View style={{{{width:80,height:60,{style}}}}}/>}}"
    );
    let artifact = valle_compiler::motion::compile_motion(&source)
        .unwrap()
        .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let context =
        motion_context_at_frame(0, 30, valle_timeline::FrameRate::new(30, 1).unwrap()).unwrap();
    let fonts = Fonts::default();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((100, 100)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let report = emit(&tree, &default_font_naming).unwrap();
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    report.program
}

#[test]
fn css_border_shadow_and_gradient_keep_fractional_channels() {
    let color = "rgb(20.125 40.375 60.625 / 0.12345)";
    let expected = AuthorColor::parse(color).unwrap().to_working();
    let program = render(&format!(
        "border:'3px solid {color}',boxShadow:'2px 3px 4px {color}'"
    ));
    assert!(
        program
            .nodes()
            .iter()
            .any(|node| matches!(node, Node::Shadow(shadow) if shadow.color == expected))
    );
    assert!(
        program
            .paints()
            .iter()
            .any(|paint| matches!(paint, Paint::Solid(actual) if *actual == expected))
    );
    let program = render(&format!(
        "backgroundImage:'linear-gradient(in srgb, {color}, rgb(20.25 40.5 60.75 / 0.12346))'"
    ));
    let colors = program
        .paints()
        .iter()
        .flat_map(|paint| match paint {
            Paint::LinearGradient { stops, .. } => {
                stops.iter().map(|stop| stop.color).collect::<Vec<_>>()
            }
            _ => Vec::new(),
        })
        .collect::<Vec<_>>();
    assert!(!colors.is_empty());
    assert!(
        colors
            .iter()
            .any(|actual| *actual == AuthorColor::parse(color).unwrap().to_working())
    );
    assert!(colors.iter().any(|actual| {
        *actual
            == AuthorColor::parse("rgb(20.25 40.5 60.75 / 0.12346)")
                .unwrap()
                .to_working()
    }));
}
