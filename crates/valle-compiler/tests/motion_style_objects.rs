#![cfg(feature = "motion")]

use std::collections::BTreeMap;
use valle_compiler::motion::compile_motion;
use valle_motion::{
    Fonts, LayoutOptions, ResolvedSignals, StyleCache, Viewport, build_tree, default_font_naming,
    emit, motion_context_at, phase_windows, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

fn frames(source: &str) -> Vec<(BTreeMap<String, [f32; 4]>, Vec<u8>)> {
    let artifact = compile_motion(source)
        .unwrap_or_else(|d| panic!("{d:#?}"))
        .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let fonts = Fonts::default();
    let cache = StyleCache::new();
    let props = resolve_props(&artifact.controls, &BTreeMap::new()).unwrap();
    let windows = phase_windows(&artifact.controls.phase_spec(), 90);
    [60, 0, 30, 0, 60]
        .into_iter()
        .map(|frame| {
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
                (
                    valle_motion::layout::layout_boxes(&artifact, &tree).unwrap(),
                    emit(&tree, &default_font_naming)
                        .unwrap()
                        .program
                        .packed_bytes()
                        .unwrap(),
                )
            };
            let cold = render(None);
            assert_eq!(cold, render(Some(&cache)), "frame {frame}");
            cold
        })
        .collect()
}

#[test]
fn object_reuse_preserves_js_key_order_lexical_scope_and_frame_values() {
    let reused = r#"
        const unit = 20;
        const base = { width: unit * 8, paddingLeft: 4, padding: 8, height: unit * 2 };
        const alias = base;
        export default function Demo(ctx) {
            const unit = 2;
            const dimension = 'width';
            const width = 120 + ctx.localFrame;
            const animated = { ...alias, [dimension]: width, backgroundColor: 'red' } as const;
            const reused = animated;
            return <Scene style={{ width: 320, height: 180 }}>
                <View key="target" style={{ ...reused, paddingLeft: 12 }}>
                    <View key="child" style={{ width: 10, height: 10, backgroundColor: 'blue' }} />
                </View>
            </Scene>;
        }
    "#;
    let explicit = r#"
        export default function Demo(ctx) {
            return <Scene style={{ width: 320, height: 180 }}>
                <View key="target" style={{ width: 120 + ctx.localFrame, paddingLeft: 12, padding: 8,
                    height: 40, backgroundColor: 'red' }}>
                    <View key="child" style={{ width: 10, height: 10, backgroundColor: 'blue' }} />
                </View>
            </Scene>;
        }
    "#;
    let actual = frames(reused);
    assert_eq!(actual, frames(explicit));
    assert_eq!(actual[0].0["target"][2], 180.0);
    assert_eq!(actual[0].0["target"][3], 40.0);
    assert_eq!(actual[0].0["child"][0], 8.0);
    assert_eq!(actual[0], actual[4]);
    assert_eq!(actual[1], actual[3]);
}

#[test]
fn duplicate_keys_replace_values_without_moving_declaration_positions() {
    let authored = r#"export default function Demo(ctx) {
        return <Scene style={{width:320,height:180}}><View key="target"
            style={{width:160,height:80,paddingLeft:2,padding:10,paddingLeft:20,backgroundColor:'red'}}>
            <View key="child" style={{width:10,height:10,backgroundColor:'blue'}} />
        </View></Scene>;
    }"#;
    let reference = authored.replace(
        "paddingLeft:2,padding:10,paddingLeft:20",
        "paddingLeft:20,padding:10",
    );
    assert_eq!(frames(authored), frames(&reference));
}

#[test]
fn reused_styles_keep_typed_transforms_and_span_admission() {
    let source = r#"const textStyle = {color:'red',fontSize:24};
        export default function Demo(ctx) {
            const movement = {translate:point(ctx.localFrame, 4),rotate:interpolate(ctx.localFrame,[0,60],["0deg","60deg"])};
            return <Scene><View style={movement}/><Text><Span style={{...textStyle,fontWeight:700}}>Hello</Span></Text></Scene>;
        }"#;
    compile_motion(source).unwrap();
    assert!(compile_motion(&source.replace("fontSize:24", "padding:24")).is_err());
}

#[test]
fn local_object_capture_survives_map_shadowing_and_new_iterations() {
    let source = r#"export default function Demo(ctx) {
        const width = 30 + ctx.localFrame;
        const shared = {width, height:20, backgroundColor:'red'};
        return <Scene style={{width:320,height:180}}>
            {[0,1].map(width => <View key={`item-${width}`} style={shared}/>)}
        </Scene>;
    }"#;
    let reference = source.replace(
        "style={shared}",
        "style={{width:30 + ctx.localFrame,height:20,backgroundColor:'red'}}",
    );
    assert_eq!(frames(source), frames(&reference));
}

#[test]
fn shape_changes_getters_and_shadowed_nonobjects_fail_at_compile_time() {
    for body in [
        "const style = {get width(){return 10;}}; return <Scene style={style}/>;",
        "const style = {[ctx.localFrame > 0 ? 'width' : 'height']:10}; return <Scene style={style}/>;",
        "const style = ctx.localFrame > 0 ? {width:10} : {height:20}; return <Scene style={style}/>;",
        "const style = {width:10}; return <Scene style={{...style,...(ctx.localFrame > 0 ? {height:20} : {})}}/>;",
        "const base = 42; return <Scene style={base}/>;",
        "const base = ctx.localFrame; return <Scene style={base}/>;",
    ] {
        let source =
            format!("const base = {{width:10}}; export default function Demo(ctx) {{ {body} }}");
        assert!(compile_motion(&source).is_err(), "{body}");
    }
}
