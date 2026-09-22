#![cfg(feature = "motion")]

use std::collections::BTreeMap;
use valle_compiler::motion::compile_motion;
use valle_motion::{
    Expr, ExprId, Fonts, LayoutOptions, MotionValue, StyleCache, Viewport, build_tree,
    default_font_naming, emit, motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

fn source(class: &str) -> String {
    format!(
        r#"export default function Demo(ctx) {{
        return <Scene style={{{{width:320,height:180}}}}>
            <View key="target" className={class} style={{{{height:40}}}} />
        </Scene>;
    }}"#
    )
}

fn snapshots(source: &str, frames: &[u32]) -> Vec<(BTreeMap<String, [f32; 4]>, Vec<u8>)> {
    let artifact = compile_motion(source)
        .unwrap_or_else(|d| panic!("{source}: {d:#?}"))
        .artifact;
    let prepared = prepare_scene(&artifact).unwrap();
    let fonts = Fonts::default();
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
fn independent_conditions_and_token_fragments_select_the_right_frame_styles() {
    let dynamic = source(
        r#"{`w-20 ${ctx.localFrame >= 30 ? 'w-40' : ''} bg-${ctx.localFrame >= 50 ? 'blue-500' : 'red-500'}`}"#,
    );
    let frames = [60, 0, 30, 49, 50, 0, 60];
    let actual = snapshots(&dynamic, &frames);
    for (i, frame) in frames.into_iter().enumerate() {
        let classes = format!(
            "\"w-20 {} bg-{}-500\"",
            if frame >= 30 { "w-40" } else { "" },
            if frame >= 50 { "blue" } else { "red" }
        );
        assert_eq!(actual[i], snapshots(&source(&classes), &[frame])[0]);
    }
    assert_eq!(actual[0], actual[6]);
    assert_eq!(actual[1], actual[5]);
    let artifact = compile_motion(&dynamic).unwrap().artifact;
    assert!(!prepare_scene(&artifact).unwrap().can_reuse_layout());
}

#[test]
fn reused_class_choices_and_pure_helpers_preserve_important_and_inline_precedence() {
    let authored = r#"function color(active) { return active ? 'bg-blue-500' : 'bg-red-500'; }
        export default function Demo(ctx) {
            const width = ctx.localFrame >= 30 ? 'w-40' : 'w-20!';
            return <Scene style={{width:320,height:180}}>
                <View key="target" className={`${width} ${color(ctx.localFrame >= 50)}`} style={{height:40,width:100}}/>
            </Scene>;
        }"#;
    let actual = snapshots(authored, &[60, 0, 30]);
    assert_eq!(actual[0].0["target"][2], 100.0);
    assert_eq!(actual[1].0["target"][2], 80.0);
    assert_eq!(actual[2].0["target"][2], 100.0);
    let static_classes = authored.replace(
        "`${width} ${color(ctx.localFrame >= 50)}`",
        "'w-20! bg-red-500'",
    );
    assert_eq!(actual[1], snapshots(&static_classes, &[0])[0]);
}

#[test]
fn repeated_candidates_merge_conditions_and_unconditional_classes_win() {
    let authored = source(
        r#"{`${ctx.localFrame > 10 ? 'w-40 bg-red-500' : 'w-20'} ${ctx.localFrame < 30 ? 'bg-red-500' : ''} w-20`}"#,
    );
    let actual = snapshots(&authored, &[0, 20, 60]);
    for (i, class) in [
        "\"w-20 bg-red-500\"",
        "\"w-20 w-40 bg-red-500\"",
        "\"w-20 w-40 bg-red-500\"",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(actual[i], snapshots(&source(class), &[0])[0]);
    }
}

#[test]
fn independent_class_conditions_do_not_expand_complete_style_combinations() {
    let holes = (0..20)
        .map(|n| format!("${{ctx.localFrame > {n} ? 'w-{}' : ''}}", n + 1))
        .collect::<Vec<_>>()
        .join(" ");
    let artifact = compile_motion(&source(&format!("{{`{holes}`}}")))
        .unwrap()
        .artifact;
    let node = artifact
        .nodes
        .iter()
        .find(|node| node.key == "target")
        .unwrap();
    assert_eq!(node.class_names.len(), 20);
    assert_eq!(node.class_conditions.len(), 20);
    assert!(artifact.exprs.len() < 200, "{}", artifact.exprs.len());

    let mut aliases = "const c0 = ctx.localFrame > 30 ? 'w-20' : 'w-40';".to_owned();
    for n in 1..25 {
        aliases.push_str(&format!(
            "const c{n} = ctx.localFrame > {n} ? c{} : c{};",
            n - 1,
            n - 1
        ));
    }
    let shared = format!(
        "export default function Demo(ctx) {{ {aliases} return <Scene className={{c24}}/>; }}"
    );
    let artifact = compile_motion(&shared).unwrap().artifact;
    assert_eq!(artifact.nodes[0].class_names.len(), 2);
}

#[test]
fn all_choices_and_direct_artifact_conditions_are_validated() {
    for class in [
        "{ctx.localFrame > 30 ? 'w-20' : 'unknown-class'}",
        "{ctx.localFrame > 30 ? 'w-20' : 'animate-spin'}",
        "{`w-${ctx.localFrame}`}",
        "{ctx.localFrame}",
        "{bounds('target').width > 0 ? 'w-20' : 'w-40'}",
        "{unit.index > 0 ? 'w-20' : 'w-40'}",
    ] {
        assert!(compile_motion(&source(class)).is_err(), "{class}");
    }
    let mut artifact = compile_motion(&source("\"w-20\"")).unwrap().artifact;
    let id = ExprId(artifact.exprs.len() as u32);
    artifact.exprs.push(Expr::Const {
        value: MotionValue::Number(1.0),
    });
    artifact.nodes[1].class_conditions.insert("w-20".into(), id);
    assert!(
        artifact
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.message.contains("bool expression"))
    );
    artifact.exprs[id.0 as usize] = Expr::Const {
        value: MotionValue::Bool(true),
    };
    artifact.nodes[1].class_conditions.insert("w-40".into(), id);
    assert!(
        artifact
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.message.contains("present in classNames"))
    );
}
