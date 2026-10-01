#![cfg(feature = "motion")]

use valle_compiler::motion::{MotionModuleGraph, compile_motion, compile_motion_modules};
use valle_draw::program::{InstanceShape, Node};
use valle_motion::{
    DiagCode, EvalInputs, FontResource, Fonts, InstanceColumnValues, InstanceKeys, LayoutOptions,
    StyleValue, Viewport, build_tree, default_font_naming, emit, eval_all, eval_instances,
    motion_context_at_frame, prepare_scene, resolve_props,
};
use valle_timeline::FrameRate;

fn flow_rows(count: usize) -> String {
    include_str!("fixtures/motion/composition/flow-instances.motion.tsx")
        .replace("length: 4", &format!("length: {count}"))
}

fn card_rows(count: usize) -> String {
    include_str!("fixtures/motion/composition/repeated-cards.motion.tsx")
        .replace("length: 4", &format!("length: {count}"))
}

fn component_card_rows(count: usize) -> String {
    include_str!("fixtures/motion/composition/repeated-card-component.motion.tsx")
        .replace("length: 4", &format!("length: {count}"))
}

fn compile_card_module(source: &str) -> valle_motion::SceneArtifact {
    let graph = MotionModuleGraph::new(
        "scene.motion.tsx",
        std::collections::BTreeMap::from([("scene.motion.tsx".into(), source.into())]),
    )
    .unwrap();
    compile_motion_modules(&graph).unwrap().artifact
}

#[test]
fn component_card_map_shares_one_template_and_preserves_component_keys() {
    let source = component_card_rows(4);
    let compiled = compile_motion(&source).unwrap();
    assert!(
        compiled
            .warnings
            .iter()
            .all(|warning| warning.code != DiagCode::InstanceFallback)
    );
    let compact = compiled.artifact;
    let large = compile_motion(&component_card_rows(40)).unwrap().artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    assert_eq!(large.instance_groups.len(), 1);
    assert_eq!(compact.nodes.len(), large.nodes.len());
    assert_eq!(
        compact.instance_groups[0].exprs.len(),
        large.instance_groups[0].exprs.len()
    );
    let group = &compact.instance_groups[0];
    assert_eq!(group.template_node_count(), 4);
    assert_eq!(group.columns.len(), 3);
    assert!(group.columns.iter().all(|column| column.name != "id"));
    assert_ne!(group.template.key, group.template_key_prefix);
    assert_eq!(
        group.key_for_node(0, &group.template.key).as_deref(),
        Some("card0/body")
    );
    assert_eq!(
        group
            .key_for_node(0, &group.template_children[0].node.key)
            .as_deref(),
        Some("card0/root.1")
    );
    compact.validate().unwrap();
    let mut invalid = compact.clone();
    invalid.instance_groups[0].template_key_prefix = "unrelated".into();
    assert!(invalid.validate().is_err());
    let expanded = compile_motion(&source.replace("key={card.id}", "key={`${card.id}`}"))
        .unwrap()
        .artifact;
    assert!(expanded.instance_groups.is_empty());
    let module_compact = compile_card_module(&source);
    assert_eq!(module_compact.instance_groups.len(), 1);
    let card_start = source.find("function Card(ctx, { item })").unwrap();
    let entry_start = source.find("export default function Cards(ctx)").unwrap();
    let entry = format!(
        "{}import {{ Card }} from \"./card.motion\";\n{}",
        &source[..card_start],
        &source[entry_start..]
    );
    let card_module = format!("export {}", &source[card_start..entry_start]);
    let graph = MotionModuleGraph::new(
        "scene.motion.tsx",
        std::collections::BTreeMap::from([
            ("scene.motion.tsx".into(), entry),
            ("card.motion.tsx".into(), card_module),
        ]),
    )
    .unwrap();
    let imported = compile_motion_modules(&graph).unwrap().artifact;
    assert_eq!(imported.instance_groups.len(), 1);
    assert_eq!(imported.instance_groups[0].template_node_count(), 4);
    let mut fonts = Fonts::default();
    fonts
        .register(FontResource::new(
            include_bytes!("../../../assets/fonts/noto/NotoSans-Variable.ttf").to_vec(),
        ))
        .unwrap();
    let opts = LayoutOptions {
        viewport: Viewport::new((160, 180)),
        fonts: &fonts,
        styles: None,
    };
    for frame in [0, 30] {
        let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let props = resolve_props(&compact.controls, &Default::default()).unwrap();
        let compact_tree =
            build_tree(&prepare_scene(&compact).unwrap(), &context, &props, &opts).unwrap();
        let expanded_tree =
            build_tree(&prepare_scene(&expanded).unwrap(), &context, &props, &opts).unwrap();
        assert_eq!(
            valle_motion::layout_boxes(&compact, &compact_tree).unwrap(),
            valle_motion::layout_boxes(&expanded, &expanded_tree).unwrap(),
            "frame {frame}"
        );
        assert_eq!(compact_tree.node_texts, expanded_tree.node_texts);
        assert_eq!(
            emit(&compact_tree, &default_font_naming).unwrap().program,
            emit(&expanded_tree, &default_font_naming).unwrap().program,
            "frame {frame}"
        );
    }
}

#[test]
fn component_card_layout_location_and_text_sources_match_expansion() {
    let source = component_card_rows(4);
    let compact = compile_motion(&source).unwrap().artifact;
    let expanded = compile_motion(&source.replace("key={card.id}", "key={`${card.id}`}"))
        .unwrap()
        .artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    let mut fonts = Fonts::default();
    fonts
        .register(FontResource::new(
            include_bytes!("../../../assets/fonts/noto/NotoSans-Variable.ttf").to_vec(),
        ))
        .unwrap();
    let opts = LayoutOptions {
        viewport: Viewport::new((160, 180)),
        fonts: &fonts,
        styles: None,
    };
    for frame in [0, 30] {
        let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let props = resolve_props(&compact.controls, &Default::default()).unwrap();
        let compact_tree =
            build_tree(&prepare_scene(&compact).unwrap(), &context, &props, &opts).unwrap();
        let expanded_tree =
            build_tree(&prepare_scene(&expanded).unwrap(), &context, &props, &opts).unwrap();
        for index in 0..4 {
            let body = format!("card{index}/body");
            let child = format!("card{index}/root.1");
            let group = format!("card{index}/root.3");
            let boxes = valle_motion::layout_boxes(&compact, &compact_tree).unwrap();
            let at_body = [1.0, boxes[&body][1] + 1.0];
            let at_child = [boxes[&child][0] + 1.0, boxes[&child][1] + 1.0];
            let at_group = [boxes[&group][0] + 1.0, boxes[&group][1] + 1.0];
            for (point, expected) in [(at_body, &body), (at_child, &child), (at_group, &group)] {
                assert_eq!(
                    valle_motion::layout_box_at_point(&compact_tree, point).unwrap(),
                    Some(expected.clone()),
                    "compact frame {frame} point {point:?}"
                );
                assert_eq!(
                    valle_motion::layout_box_at_point(&expanded_tree, point).unwrap(),
                    Some(expected.clone()),
                    "expanded frame {frame} point {point:?}"
                );
            }
        }
        assert_eq!(
            valle_motion::layout_box_at_point(&compact_tree, [159.0, 179.0]).unwrap(),
            Some("root".into())
        );
        assert_eq!(
            valle_motion::layout_box_at_point(&compact_tree, [160.0, 179.0]).unwrap(),
            None
        );
        assert_eq!(
            valle_motion::layout_box_at_point(&compact_tree, [f32::NAN, 0.0]).unwrap(),
            None
        );

        let text_keys = |tree| {
            emit(tree, &default_font_naming)
                .unwrap()
                .program
                .nodes()
                .iter()
                .filter_map(|node| match node {
                    Node::GlyphRun(run) => run.source_node.clone(),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let expected = (0..4)
            .map(|index| format!("card{index}/root.3.1"))
            .collect::<Vec<_>>();
        assert_eq!(text_keys(&compact_tree), expected);
        assert_eq!(text_keys(&expanded_tree), expected);
    }
}

#[test]
fn layout_location_follows_stacking_transform_visibility_and_overflow() {
    let source = r##"
export const composition = { width: 160, height: 120, fps: 30, duration: 1 };
export default function Locate() {
  return <Scene style={{ width: 160, height: 120, backgroundColor: "#111111" }}>
    <View key="back" style={{ position: "absolute", left: 10, top: 10,
      width: 60, height: 60, backgroundColor: "#333333" }} />
    <View key="front" style={{ position: "absolute", left: 20, top: 20,
      width: 40, height: 40, zIndex: 2, backgroundColor: "#aaaaaa" }} />
    <View key="moved" style={{ position: "absolute", left: 70, top: 10,
      width: 10, height: 10, transform: "translate(10px, 0px)",
      backgroundColor: "#eeeeee" }} />
    <View key="clip" style={{ position: "absolute", left: 10, top: 80,
      width: 20, height: 20, overflow: "hidden", backgroundColor: "#555555" }}>
      <View key="outside" style={{ position: "absolute", left: 15, top: 0,
        width: 20, height: 20, backgroundColor: "#eeeeee" }} />
    </View>
    <View key="hidden" style={{ position: "absolute", left: 110, top: 10,
      width: 20, height: 20, opacity: 0, backgroundColor: "#ffffff" }} />
  </Scene>;
}
"##;
    let artifact = compile_motion(source).unwrap().artifact;
    let context = motion_context_at_frame(0, 30, FrameRate::new(30, 1).unwrap()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepare_scene(&artifact).unwrap(),
        &context,
        &resolve_props(&artifact.controls, &Default::default()).unwrap(),
        &LayoutOptions {
            viewport: Viewport::new((160, 120)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    for (point, expected) in [
        ([15.0, 15.0], Some("back")),
        ([25.0, 25.0], Some("front")),
        ([75.0, 15.0], Some("root")),
        ([85.0, 15.0], Some("moved")),
        ([27.0, 85.0], Some("outside")),
        ([35.0, 85.0], Some("root")),
        ([115.0, 15.0], Some("root")),
        ([160.0, 15.0], None),
    ] {
        assert_eq!(
            valle_motion::layout_box_at_point(&tree, point)
                .unwrap()
                .as_deref(),
            expected,
            "point {point:?}"
        );
    }
}

#[test]
fn direct_and_component_group_roots_share_layout_templates() {
    let component_group = |count| {
        component_card_rows(count)
            .replace("return <View key=\"body\"", "return <Group key=\"body\"")
            .replace("\n  </View>;", "\n  </Group>;")
    };
    let direct_group = |count| {
        card_rows(count)
            .replace("<View key={card.id}", "<Group key={card.id}")
            .replace("    </View>)}", "    </Group>)}")
    };
    let mut fonts = Fonts::default();
    fonts
        .register(FontResource::new(
            include_bytes!("../../../assets/fonts/noto/NotoSans-Variable.ttf").to_vec(),
        ))
        .unwrap();
    let opts = LayoutOptions {
        viewport: Viewport::new((160, 180)),
        fonts: &fonts,
        styles: None,
    };
    for (source, large_source) in [
        (component_group(4), component_group(40)),
        (direct_group(4), direct_group(40)),
    ] {
        let compact = compile_motion(&source).unwrap().artifact;
        let large = compile_motion(&large_source).unwrap().artifact;
        assert_eq!(compact.instance_groups.len(), 1);
        assert_eq!(large.instance_groups.len(), 1);
        assert!(matches!(
            compact.instance_groups[0].template.kind,
            valle_motion::NodeKind::Group
        ));
        assert_eq!(
            compact.instance_groups[0].exprs.len(),
            large.instance_groups[0].exprs.len()
        );
        assert_eq!(compact.nodes.len(), large.nodes.len());
        compact.validate().unwrap();
        let expanded = compile_motion(&source.replace("key={card.id}", "key={`${card.id}`}"))
            .unwrap()
            .artifact;
        assert!(expanded.instance_groups.is_empty());
        let compact_prepared = prepare_scene(&compact).unwrap();
        let expanded_prepared = prepare_scene(&expanded).unwrap();
        for frame in [0, 30] {
            let context =
                motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
            let compact_tree = build_tree(
                &compact_prepared,
                &context,
                &resolve_props(&compact.controls, &Default::default()).unwrap(),
                &opts,
            )
            .unwrap();
            let expanded_tree = build_tree(
                &expanded_prepared,
                &context,
                &resolve_props(&expanded.controls, &Default::default()).unwrap(),
                &opts,
            )
            .unwrap();
            assert_eq!(
                valle_motion::layout_boxes(&compact, &compact_tree).unwrap(),
                valle_motion::layout_boxes(&expanded, &expanded_tree).unwrap(),
                "frame {frame}"
            );
            assert_eq!(compact_tree.node_texts, expanded_tree.node_texts);
            assert_eq!(
                emit(&compact_tree, &default_font_naming).unwrap().program,
                emit(&expanded_tree, &default_font_naming).unwrap().program,
                "frame {frame}"
            );
        }
    }
}

#[test]
fn direct_and_component_text_roots_share_layout_and_glyph_sources() {
    let source = include_str!("fixtures/motion/composition/text-root-instances.motion.tsx");
    let direct = source.replace(
        "<Label key={row.id} item={row} />",
        "<Text key={row.id} style={{ fontSize: 14, color: row.color }}>{row.label}</Text>",
    );
    let mut fonts = Fonts::default();
    fonts
        .register(FontResource::new(
            include_bytes!("../../../assets/fonts/noto/NotoSans-Variable.ttf").to_vec(),
        ))
        .unwrap();
    let opts = LayoutOptions {
        viewport: Viewport::new((160, 180)),
        fonts: &fonts,
        styles: None,
    };
    for source in [source.to_string(), direct] {
        let compact = compile_motion(&source).unwrap().artifact;
        let large = compile_motion(&source.replace("length: 4", "length: 40"))
            .unwrap()
            .artifact;
        assert_eq!(compact.instance_groups.len(), 1);
        assert_eq!(large.instance_groups.len(), 1);
        assert!(matches!(
            compact.instance_groups[0].template.kind,
            valle_motion::NodeKind::Text { .. }
        ));
        assert_eq!(
            compact.instance_groups[0].exprs.len(),
            large.instance_groups[0].exprs.len()
        );
        assert_eq!(compact.nodes.len(), large.nodes.len());
        compact.validate().unwrap();
        let expanded = compile_motion(&source.replace("key={row.id}", "key={`${row.id}`}"))
            .unwrap()
            .artifact;
        assert!(expanded.instance_groups.is_empty());
        let compact_prepared = prepare_scene(&compact).unwrap();
        let expanded_prepared = prepare_scene(&expanded).unwrap();
        for frame in [0, 30] {
            let context =
                motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
            let compact_tree = build_tree(
                &compact_prepared,
                &context,
                &resolve_props(&compact.controls, &Default::default()).unwrap(),
                &opts,
            )
            .unwrap();
            let expanded_tree = build_tree(
                &expanded_prepared,
                &context,
                &resolve_props(&expanded.controls, &Default::default()).unwrap(),
                &opts,
            )
            .unwrap();
            assert_eq!(
                valle_motion::layout_boxes(&compact, &compact_tree).unwrap(),
                valle_motion::layout_boxes(&expanded, &expanded_tree).unwrap(),
                "frame {frame}"
            );
            assert_eq!(compact_tree.node_texts, expanded_tree.node_texts);
            assert_eq!(
                emit(&compact_tree, &default_font_naming).unwrap().program,
                emit(&expanded_tree, &default_font_naming).unwrap().program,
                "frame {frame}"
            );
        }
        let review = valle_motion::review_motion(
            &compact_prepared,
            &resolve_props(&compact.controls, &Default::default()).unwrap(),
            &opts,
            FrameRate::new(30, 1).unwrap(),
            4,
            valle_timeline::RationalTime::new(2, 15).unwrap(),
            4,
            false,
        )
        .unwrap();
        let first_key = compact.instance_groups[0]
            .key_for_node(0, &compact.instance_groups[0].template.key)
            .unwrap();
        assert!(
            review
                .issues
                .iter()
                .any(|issue| issue.code == "text_readability" && issue.node == first_key)
        );
    }
}

#[test]
fn component_instance_ignores_unread_heterogeneous_item_fields() {
    let source = component_card_rows(4).replace(
        "  color: index % 2 === 0 ? \"#3154ad\" : \"#ac5732\",",
        "  color: index % 2 === 0 ? \"#3154ad\" : \"#ac5732\",\n  metadata: { tags: [index, index + 1] },\n  unused: index % 2 === 0 ? 7 : \"later\",",
    );
    let compact = compile_motion(&source).unwrap().artifact;
    let expanded = compile_motion(&source.replace("key={card.id}", "key={`${card.id}`}"))
        .unwrap()
        .artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    assert_eq!(compact.instance_groups[0].columns.len(), 3);
    assert!(expanded.instance_groups.is_empty());
    let mut fonts = Fonts::default();
    fonts
        .register(FontResource::new(
            include_bytes!("../../../assets/fonts/noto/NotoSans-Variable.ttf").to_vec(),
        ))
        .unwrap();
    let opts = LayoutOptions {
        viewport: Viewport::new((160, 180)),
        fonts: &fonts,
        styles: None,
    };
    let context = motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let compact_tree = build_tree(
        &prepare_scene(&compact).unwrap(),
        &context,
        &resolve_props(&compact.controls, &Default::default()).unwrap(),
        &opts,
    )
    .unwrap();
    let expanded_tree = build_tree(
        &prepare_scene(&expanded).unwrap(),
        &context,
        &resolve_props(&expanded.controls, &Default::default()).unwrap(),
        &opts,
    )
    .unwrap();
    assert_eq!(
        valle_motion::layout_boxes(&compact, &compact_tree).unwrap(),
        valle_motion::layout_boxes(&expanded, &expanded_tree).unwrap()
    );
    assert_eq!(
        emit(&compact_tree, &default_font_naming).unwrap().program,
        emit(&expanded_tree, &default_font_naming).unwrap().program
    );
}

#[test]
fn component_instance_item_supports_props_paths_and_local_destructuring() {
    let source = component_card_rows(4);
    let props_path = source
        .replace(
            "function Card(ctx, { item }) {",
            "function Card(ctx, props) {",
        )
        .replace(
            "const height = item.height;",
            "const height = props.item.height;",
        )
        .replace(
            "const color = item.color;",
            "const color = props.item.color;",
        )
        .replace(
            "const label = item.label;",
            "const label = props.item.label;",
        );
    let destructured = source.replace(
        "const height = item.height;\n  const color = item.color;\n  const label = item.label;",
        "const { height, color, label } = item;",
    );
    for variant in [props_path, destructured] {
        let compact = compile_motion(&variant).unwrap().artifact;
        let expanded = compile_motion(&variant.replace("key={card.id}", "key={`${card.id}`}"))
            .unwrap()
            .artifact;
        assert_eq!(compact.instance_groups.len(), 1);
        assert_eq!(compact.instance_groups[0].columns.len(), 3);
        assert!(expanded.instance_groups.is_empty());
        let mut fonts = Fonts::default();
        fonts
            .register(FontResource::new(
                include_bytes!("../../../assets/fonts/noto/NotoSans-Variable.ttf").to_vec(),
            ))
            .unwrap();
        let opts = LayoutOptions {
            viewport: Viewport::new((160, 180)),
            fonts: &fonts,
            styles: None,
        };
        let context = motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let compact_tree = build_tree(
            &prepare_scene(&compact).unwrap(),
            &context,
            &resolve_props(&compact.controls, &Default::default()).unwrap(),
            &opts,
        )
        .unwrap();
        let expanded_tree = build_tree(
            &prepare_scene(&expanded).unwrap(),
            &context,
            &resolve_props(&expanded.controls, &Default::default()).unwrap(),
            &opts,
        )
        .unwrap();
        assert_eq!(
            emit(&compact_tree, &default_font_naming).unwrap().program,
            emit(&expanded_tree, &default_font_naming).unwrap().program
        );
    }
}

#[test]
fn nested_component_instance_forwards_item_and_keeps_nested_keys() {
    let source = component_card_rows(4)
        .replace("function Card(ctx, { item }) {", "function CardBody(ctx, { item }) {")
        .replace(
            "export default function Cards(ctx) {",
            "function Card(ctx, { item }) { return <CardBody key=\"inner\" item={item} />; }\n\nexport default function Cards(ctx) {",
        );
    let compact = compile_motion(&source).unwrap().artifact;
    let expanded = compile_motion(&source.replace("key={card.id}", "key={`${card.id}`}"))
        .unwrap()
        .artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    let group = &compact.instance_groups[0];
    assert_eq!(
        group.key_for_node(0, &group.template.key).as_deref(),
        Some("card0/inner/body")
    );
    assert!(expanded.instance_groups.is_empty());
    let mut fonts = Fonts::default();
    fonts
        .register(FontResource::new(
            include_bytes!("../../../assets/fonts/noto/NotoSans-Variable.ttf").to_vec(),
        ))
        .unwrap();
    let opts = LayoutOptions {
        viewport: Viewport::new((160, 180)),
        fonts: &fonts,
        styles: None,
    };
    let context = motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let compact_tree = build_tree(
        &prepare_scene(&compact).unwrap(),
        &context,
        &resolve_props(&compact.controls, &Default::default()).unwrap(),
        &opts,
    )
    .unwrap();
    let expanded_tree = build_tree(
        &prepare_scene(&expanded).unwrap(),
        &context,
        &resolve_props(&expanded.controls, &Default::default()).unwrap(),
        &opts,
    )
    .unwrap();
    assert_eq!(
        valle_motion::layout_boxes(&compact, &compact_tree).unwrap(),
        valle_motion::layout_boxes(&expanded, &expanded_tree).unwrap()
    );
    assert_eq!(
        emit(&compact_tree, &default_font_naming).unwrap().program,
        emit(&expanded_tree, &default_font_naming).unwrap().program
    );
}

#[test]
fn flow_card_subtrees_share_one_template_and_preserve_descendant_layout() {
    let source = card_rows(4);
    let compact = compile_motion(&source).unwrap().artifact;
    let large = compile_motion(&card_rows(40)).unwrap().artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    assert_eq!(large.instance_groups.len(), 1);
    assert_eq!(compact.instance_groups[0].template_children.len(), 2);
    assert_eq!(compact.instance_groups[0].template_node_count(), 4);
    assert_eq!(
        compact.instance_groups[0].exprs.len(),
        large.instance_groups[0].exprs.len()
    );
    assert_eq!(compact.nodes.len(), large.nodes.len());
    compact.validate().unwrap();
    let mut foreign_key = compact.clone();
    foreign_key.instance_groups[0].template_children[1].children[0]
        .node
        .key = "reader".into();
    assert!(foreign_key.validate().is_err());
    let mut conflicting_descendant = compact.clone();
    conflicting_descendant.instance_groups[0].keys = InstanceKeys::Explicit(vec![
        "card0".into(),
        "card0.1".into(),
        "card2".into(),
        "card3".into(),
    ]);
    assert!(conflicting_descendant.validate().is_err());
    let mut unsupported_style = compact.clone();
    unsupported_style.instance_groups[0].template_children[0]
        .node
        .styles[0]
        .property = "filter".into();
    assert!(unsupported_style.validate().is_err());

    let expanded = compile_motion(&source.replace("key={card.id}", "key={`${card.id}`}"))
        .unwrap()
        .artifact;
    assert!(expanded.instance_groups.is_empty());
    let module_compact = compile_card_module(&source);
    let module_expanded =
        compile_card_module(&source.replace("key={card.id}", "key={`${card.id}`}"));
    assert_eq!(module_compact.instance_groups.len(), 1);
    let mut fonts = Fonts::default();
    fonts
        .register(FontResource::new(
            include_bytes!("../../../assets/fonts/noto/NotoSans-Variable.ttf").to_vec(),
        ))
        .unwrap();
    let opts = LayoutOptions {
        viewport: Viewport::new((160, 180)),
        fonts: &fonts,
        styles: None,
    };
    for frame in [0, 30] {
        let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let stats = prepare_scene(&compact)
            .unwrap()
            .frame_evaluation_stats(&context);
        assert_eq!(stats.layout_nodes, stats.active_nodes + 4 * 4);
        let compact_tree = build_tree(
            &prepare_scene(&compact).unwrap(),
            &context,
            &resolve_props(&compact.controls, &Default::default()).unwrap(),
            &opts,
        )
        .unwrap();
        let expanded_tree = build_tree(
            &prepare_scene(&expanded).unwrap(),
            &context,
            &resolve_props(&expanded.controls, &Default::default()).unwrap(),
            &opts,
        )
        .unwrap();
        let compact_boxes = valle_motion::layout_boxes(&compact, &compact_tree).unwrap();
        let expanded_boxes = valle_motion::layout_boxes(&expanded, &expanded_tree).unwrap();
        assert_eq!(compact_boxes, expanded_boxes, "frame {frame}");
        assert_eq!(compact_tree.node_texts, expanded_tree.node_texts);
        for index in 0..4 {
            for suffix in ["", ".1", ".3"] {
                let key = format!("card{index}{suffix}");
                assert!(compact_boxes.contains_key(&key), "missing {key}");
                assert_eq!(
                    compact_boxes.get(&key),
                    expanded_boxes.get(&key),
                    "{key} at frame {frame}"
                );
            }
        }
        let compact_program = emit(&compact_tree, &default_font_naming).unwrap().program;
        let expanded_program = emit(&expanded_tree, &default_font_naming).unwrap().program;
        assert_eq!(compact_program, expanded_program, "frame {frame}");
        assert!(
            compact_program
                .nodes()
                .iter()
                .any(|node| matches!(node, Node::GlyphRun(_)))
        );
        let module_programs = [&module_compact, &module_expanded].map(|artifact| {
            let tree = build_tree(
                &prepare_scene(artifact).unwrap(),
                &context,
                &resolve_props(&artifact.controls, &Default::default()).unwrap(),
                &opts,
            )
            .unwrap();
            emit(&tree, &default_font_naming).unwrap().program
        });
        assert_eq!(
            module_programs[0], module_programs[1],
            "module frame {frame}"
        );
        assert!(
            module_programs[0]
                .nodes()
                .iter()
                .any(|node| matches!(node, Node::GlyphRun(_)))
        );
        for (artifact, expected) in [(&compact, &compact_program), (&expanded, &expanded_program)] {
            let prepared = prepare_scene(artifact).unwrap();
            if let Some(cache) = valle_motion::layout::LayoutCache::new(&prepared, &fonts) {
                let tree = cache
                    .build_tree(
                        &context,
                        &resolve_props(&artifact.controls, &Default::default()).unwrap(),
                        opts.viewport,
                        None,
                    )
                    .unwrap();
                assert_eq!(
                    &emit(&tree, &default_font_naming).unwrap().program,
                    expected
                );
            }
        }
    }
}

#[test]
fn unsupported_card_descendant_falls_back_to_expanded_nodes() {
    let source = card_rows(4).replace(
        "<Group style={{ width: 106, height: 16 }}>",
        "<Group style={{ width: 106, height: 16, filter: \"blur(2px)\" }}>",
    );
    let compiled = compile_motion(&source).unwrap();
    assert!(compiled.artifact.instance_groups.is_empty());
    assert!(
        compiled
            .warnings
            .iter()
            .all(|warning| warning.code != DiagCode::InstanceFallback)
    );
    let large = compile_motion(&source.replace("length: 4", "length: 64")).unwrap();
    assert!(large.warnings.iter().any(|warning| {
        warning.code == DiagCode::InstanceFallback && warning.message.contains("template children")
    }));
}

#[test]
fn layout_instance_classes_match_expanded_static_and_row_conditional_styles() {
    let source = card_rows(4)
        .replace(
            "<View key={card.id} style=",
            "<View key={card.id} className={card.height > 36 ? \"flex flex-col rounded-lg opacity-50\" : \"flex flex-col rounded-lg\"} style=",
        )
        .replace(
            "<Group style={{ width: 106, height: 16 }}>",
            "<Group className=\"relative\" style={{ width: 106, height: 16 }}>",
        )
        .replace(
            "<Text visible={ctx.localFrame < card.height - 6}",
            "<Text className={card.height > 36 ? \"font-bold\" : \"font-normal\"} visible={ctx.localFrame < card.height - 6}",
        );
    let compact = compile_motion(&source).unwrap().artifact;
    let expanded = compile_motion(&source.replace("key={card.id}", "key={`${card.id}`}"))
        .unwrap()
        .artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    assert!(expanded.instance_groups.is_empty());
    assert_eq!(compact.instance_groups[0].template_node_count(), 4);
    assert_eq!(compact.instance_groups[0].template.class_names.len(), 4);
    assert!(
        !compact.instance_groups[0]
            .template
            .class_conditions
            .is_empty()
    );
    assert_eq!(
        compact.instance_groups[0].template_children[1].children[0]
            .node
            .class_conditions
            .len(),
        2
    );
    compact.validate().unwrap();
    let mut invalid = compact.clone();
    invalid.instance_groups[0].template_children[0]
        .node
        .class_names
        .push("unknown-instance-class".into());
    assert!(invalid.validate().is_err());
    let mut backdrop = compact.clone();
    backdrop.instance_groups[0].template_children[0]
        .node
        .class_names
        .push("backdrop-blur-sm".into());
    backdrop.validate().unwrap();
    assert!(backdrop.reads_destination());
    let mut fonts = Fonts::default();
    fonts
        .register(FontResource::new(
            include_bytes!("../../../assets/fonts/noto/NotoSans-Variable.ttf").to_vec(),
        ))
        .unwrap();
    let opts = LayoutOptions {
        viewport: Viewport::new((160, 180)),
        fonts: &fonts,
        styles: None,
    };
    let compact_prepared = prepare_scene(&compact).unwrap();
    let expanded_prepared = prepare_scene(&expanded).unwrap();
    for frame in [0, 30] {
        let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let props = resolve_props(&compact.controls, &Default::default()).unwrap();
        let compact_tree = build_tree(&compact_prepared, &context, &props, &opts).unwrap();
        let expanded_tree = build_tree(&expanded_prepared, &context, &props, &opts).unwrap();
        assert_eq!(
            valle_motion::layout_boxes(&compact, &compact_tree).unwrap(),
            valle_motion::layout_boxes(&expanded, &expanded_tree).unwrap(),
            "frame {frame}"
        );
        assert_eq!(
            emit(&compact_tree, &default_font_naming).unwrap().program,
            emit(&expanded_tree, &default_font_naming).unwrap().program,
            "frame {frame}"
        );
    }
}

#[test]
fn class_only_view_subtrees_share_one_layout_template() {
    let source = r##"
export const composition = { width: 64, height: 64, fps: 30, duration: 1 };
const ROWS = [{ id: "a" }, { id: "b" }, { id: "c" }];
export default function ClassCards() {
  return <Scene className="flex flex-col h-full w-full">
    {ROWS.map((row) => <View key={row.id}
      className={row.id === "a" ? "w-10 h-6 bg-blue-500" : "w-10 h-6 bg-red-500"}>
      <View className="w-4 h-2 bg-yellow-300" />
    </View>)}
  </Scene>;
}
"##;
    let compact = compile_motion(source).unwrap().artifact;
    let expanded = compile_motion(&source.replace("key={row.id}", "key={`${row.id}`}"))
        .unwrap()
        .artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    assert_eq!(compact.instance_groups[0].template_node_count(), 2);
    assert!(compact.instance_groups[0].template.styles.is_empty());
    assert!(
        !compact.instance_groups[0]
            .template
            .class_conditions
            .is_empty()
    );
    assert!(expanded.instance_groups.is_empty());
    let fonts = Fonts::default();
    let opts = LayoutOptions {
        viewport: Viewport::new((64, 64)),
        fonts: &fonts,
        styles: None,
    };
    let context = motion_context_at_frame(0, 30, FrameRate::new(30, 1).unwrap()).unwrap();
    let compact_tree = build_tree(
        &prepare_scene(&compact).unwrap(),
        &context,
        &resolve_props(&compact.controls, &Default::default()).unwrap(),
        &opts,
    )
    .unwrap();
    let expanded_tree = build_tree(
        &prepare_scene(&expanded).unwrap(),
        &context,
        &resolve_props(&expanded.controls, &Default::default()).unwrap(),
        &opts,
    )
    .unwrap();
    assert_eq!(
        valle_motion::layout_boxes(&compact, &compact_tree).unwrap(),
        valle_motion::layout_boxes(&expanded, &expanded_tree).unwrap()
    );
    assert_eq!(
        emit(&compact_tree, &default_font_naming).unwrap().program,
        emit(&expanded_tree, &default_font_naming).unwrap().program
    );
}

#[test]
fn flow_view_instances_share_expressions_and_keep_individual_layout_boxes() {
    let compiled = compile_motion(&flow_rows(4)).unwrap();
    assert!(
        compiled
            .warnings
            .iter()
            .all(|warning| warning.code != DiagCode::InstanceFallback)
    );
    let small = compiled.artifact;
    let large = compile_motion(&flow_rows(40)).unwrap().artifact;
    assert_eq!(small.instance_groups.len(), 1);
    assert_eq!(large.instance_groups.len(), 1);
    assert_eq!(
        small.instance_groups[0].exprs.len(),
        large.instance_groups[0].exprs.len()
    );
    assert!(matches!(
        small
            .nodes
            .iter()
            .find(|node| matches!(node.kind, valle_motion::NodeKind::InstanceLayout { .. }))
            .map(|node| &node.kind),
        Some(valle_motion::NodeKind::InstanceLayout { .. })
    ));
    small.validate().unwrap();
    let mut invalid = small.clone();
    invalid.instance_groups[0]
        .template
        .class_names
        .push("unknown-instance-class".into());
    assert!(invalid.validate().is_err());
    let mut conflicting_keys = small.clone();
    conflicting_keys.instance_groups[0].keys = InstanceKeys::Explicit(vec![
        "row0".into(),
        "row1".into(),
        "row2".into(),
        "reader".into(),
    ]);
    assert!(conflicting_keys.validate().is_err());

    let expanded_source = flow_rows(4).replace("key={row.id}", "key={`${row.id}`}");
    let expanded = compile_motion(&expanded_source).unwrap().artifact;
    assert!(expanded.instance_groups.is_empty());
    let fonts = Fonts::default();
    for frame in [0, 30] {
        let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let stats = prepare_scene(&small)
            .unwrap()
            .frame_evaluation_stats(&context);
        assert_eq!(stats.layout_nodes, stats.active_nodes + 4);
        let opts = LayoutOptions {
            viewport: Viewport::new((160, 180)),
            fonts: &fonts,
            styles: None,
        };
        let compact_tree = build_tree(
            &prepare_scene(&small).unwrap(),
            &context,
            &resolve_props(&small.controls, &Default::default()).unwrap(),
            &opts,
        )
        .unwrap();
        let expanded_tree = build_tree(
            &prepare_scene(&expanded).unwrap(),
            &context,
            &resolve_props(&expanded.controls, &Default::default()).unwrap(),
            &opts,
        )
        .unwrap();
        let compact_boxes = valle_motion::layout_boxes(&small, &compact_tree).unwrap();
        let expanded_boxes = valle_motion::layout_boxes(&expanded, &expanded_tree).unwrap();
        for index in 0..4 {
            let key = format!("row{index}");
            assert_eq!(
                compact_boxes[&key], expanded_boxes[&key],
                "{key} at frame {frame}"
            );
        }
        assert_eq!(compact_boxes["reader"], expanded_boxes["reader"]);
        assert_eq!(compact_boxes["row0"][3], 8.0);
        assert_eq!(compact_boxes["row1"][1], 10.0);
        let compact_program = emit(&compact_tree, &default_font_naming).unwrap().program;
        assert!(
            compact_program
                .nodes()
                .iter()
                .all(|node| !matches!(node, Node::InstanceBatch(_)))
        );
    }
}

#[test]
fn flow_instances_are_individual_flex_items() {
    let source = flow_rows(4).replace("relative h-full w-full", "relative flex h-full w-full");
    let expanded_source = source.replace("key={row.id}", "key={`${row.id}`}");
    let compact = compile_motion(&source).unwrap().artifact;
    let expanded = compile_motion(&expanded_source).unwrap().artifact;
    let context = motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let fonts = Fonts::default();
    let opts = LayoutOptions {
        viewport: Viewport::new((160, 180)),
        fonts: &fonts,
        styles: None,
    };
    let compact_tree = build_tree(
        &prepare_scene(&compact).unwrap(),
        &context,
        &resolve_props(&compact.controls, &Default::default()).unwrap(),
        &opts,
    )
    .unwrap();
    let expanded_tree = build_tree(
        &prepare_scene(&expanded).unwrap(),
        &context,
        &resolve_props(&expanded.controls, &Default::default()).unwrap(),
        &opts,
    )
    .unwrap();
    let compact_boxes = valle_motion::layout_boxes(&compact, &compact_tree).unwrap();
    let expanded_boxes = valle_motion::layout_boxes(&expanded, &expanded_tree).unwrap();
    for index in 0..4 {
        let key = format!("row{index}");
        assert_eq!(compact_boxes[&key], expanded_boxes[&key]);
    }
    assert!(compact_boxes["row1"][0] > compact_boxes["row0"][0]);
}

#[test]
fn flow_instances_feed_glass_foreground_bounds_and_auto_tone() {
    let source = r##"
export const composition = { width: 160, height: 180, fps: 30, duration: 2 };
const ROWS = [{ id: "row0", h: 12 }, { id: "row1", h: 20 }];
export default function FlowGlass(ctx) {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    <Glass key="lens" surfaceId="lens" shape={{ kind: "continuousRect", radius: 12 }}
      material={{ clarity: 0.8, depth: 0.4, tint: "#b9d7ff18" }}
      foreground={{ tone: "auto", protection: 0.7 }}
      style={{ width: 100, height: 100 }}>
      {ROWS.map((row) => <View key={row.id} style={{ width: 40, height: row.h,
        backgroundColor: interpolate(ctx.progress, [0, 1], ["#2140ff", "#ffd000"],
          { colorSpace: "oklch" }) }} />)}
    </Glass>
  </Scene>;
}

"##;
    let compact = compile_motion(source).unwrap().artifact;
    let expanded = compile_motion(&source.replace("key={row.id}", "key={`${row.id}`}"))
        .unwrap()
        .artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    let context = motion_context_at_frame(15, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let fonts = Fonts::default();
    let opts = LayoutOptions {
        viewport: Viewport::new((160, 180)),
        fonts: &fonts,
        styles: None,
    };
    let compact_tree = build_tree(
        &prepare_scene(&compact).unwrap(),
        &context,
        &resolve_props(&compact.controls, &Default::default()).unwrap(),
        &opts,
    )
    .unwrap();
    let expanded_tree = build_tree(
        &prepare_scene(&expanded).unwrap(),
        &context,
        &resolve_props(&expanded.controls, &Default::default()).unwrap(),
        &opts,
    )
    .unwrap();
    assert_eq!(
        compact_tree.glass.surfaces[0].foreground,
        expanded_tree.glass.surfaces[0].foreground
    );
    assert!(compact_tree.glass.surfaces[0].foreground.bounds.is_some());
    assert!(compact_tree.glass.surfaces[0].foreground.luma.is_some());
}

#[test]
fn flow_subtree_text_contributes_to_glass_foreground() {
    let source = r##"
export const composition = { width: 160, height: 180, fps: 30, duration: 2 };
const ROWS = [{ id: "row0", label: "One", color: "#ffffff" },
  { id: "row1", label: "Two", color: "#ff0000" }];
export default function FlowGlass() {
  return <Scene className="relative h-full w-full">
    <Glass key="lens" surfaceId="lens" shape={{ kind: "continuousRect", radius: 12 }}
      material={{ clarity: 0.8, depth: 0.4, tint: "#b9d7ff18" }}
      foreground={{ tone: "auto", protection: 0.7 }}
      style={{ width: 100, height: 100 }}>
      {ROWS.map((row) => <View key={row.id} style={{ width: 40, height: 20,
        backgroundColor: "#000000" }}>
        <Text style={{ fontSize: 12, color: row.color }}>{row.label}</Text>
      </View>)}
    </Glass>
  </Scene>;
}
"##;
    let compact = compile_motion(source).unwrap().artifact;
    let expanded = compile_motion(&source.replace("key={row.id}", "key={`${row.id}`}"))
        .unwrap()
        .artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    let context = motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let fonts = Fonts::default();
    let opts = LayoutOptions {
        viewport: Viewport::new((160, 180)),
        fonts: &fonts,
        styles: None,
    };
    let compact_tree = build_tree(
        &prepare_scene(&compact).unwrap(),
        &context,
        &resolve_props(&compact.controls, &Default::default()).unwrap(),
        &opts,
    )
    .unwrap();
    let expanded_tree = build_tree(
        &prepare_scene(&expanded).unwrap(),
        &context,
        &resolve_props(&expanded.controls, &Default::default()).unwrap(),
        &opts,
    )
    .unwrap();
    assert_eq!(
        compact_tree.glass.surfaces[0].foreground,
        expanded_tree.glass.surfaces[0].foreground
    );
    assert!(compact_tree.glass.surfaces[0].foreground.bounds.is_some());
    assert!(compact_tree.glass.surfaces[0].foreground.luma.unwrap() > 0.0);
}

fn grid(cols: usize) -> String {
    include_str!("fixtures/motion/composition/grid-instances.motion.tsx")
        .replace("const COLS = 8;", &format!("const COLS = {cols};"))
}

fn circle_grid(cols: usize) -> String {
    include_str!("fixtures/motion/composition/circle-instances.motion.tsx")
        .replace("const COLS = 8;", &format!("const COLS = {cols};"))
}

fn path_grid(cols: usize) -> String {
    include_str!("fixtures/motion/composition/path-instances.motion.tsx")
        .replace("const COLS = 8;", &format!("const COLS = {cols};"))
}

#[test]
fn static_path_template_shares_geometry_and_keeps_row_values_in_columns() {
    let small = compile_motion(&path_grid(8)).unwrap().artifact;
    let large = compile_motion(&path_grid(64)).unwrap().artifact;
    for (artifact, rows) in [(&small, 64), (&large, 4096)] {
        assert_eq!(artifact.instance_groups.len(), 1);
        assert_eq!(artifact.instance_groups[0].rows(), rows);
        assert!(artifact.instance_groups[0].static_path_template().is_some());
        artifact.validate().unwrap();
    }
    assert_eq!(
        small.instance_groups[0].exprs.len(),
        large.instance_groups[0].exprs.len()
    );
    let expanded = compile_motion(&path_grid(8).replace("<Path ", "<path "))
        .unwrap()
        .artifact;
    assert!(expanded.instance_groups.is_empty());

    let prepared = prepare_scene(&small).unwrap();
    let context = motion_context_at_frame(30, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&small.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let program = emit(&tree, &default_font_naming).unwrap().program;
    let batches = program
        .nodes()
        .iter()
        .filter_map(|node| match node {
            Node::InstanceBatch(batch) => Some(batch),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].instances.len(), 64);
    assert!(matches!(batches[0].shape, InstanceShape::Path(_)));
}

#[test]
fn static_path_template_carries_row_and_time_opacity() {
    let source = path_grid(8).replace(
        "translate: point(dot.x + ctx.seconds * 3, dot.y)",
        "translate: point(dot.x + ctx.seconds * 3, dot.y), opacity: 0.4 + dot.delay * 0.4 + ctx.progress * 0.1",
    );
    let compact = compile_motion(&source).unwrap().artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    assert!(compact.instance_groups[0].static_path_template().is_some());
    let mut invalid = compact.clone();
    invalid.instance_groups[0]
        .template
        .styles
        .iter_mut()
        .find(|style| style.property == "opacity")
        .unwrap()
        .value = StyleValue::Static {
        value: valle_motion::MotionValue::Number(1.5),
    };
    assert!(invalid.validate().is_err());
    let expanded = compile_motion(&source.replace("<Path ", "<path "))
        .unwrap()
        .artifact;
    assert!(expanded.instance_groups.is_empty());
    for frame in [0, 30] {
        let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let props = resolve_props(&compact.controls, &Default::default()).unwrap();
        let fonts = Fonts::default();
        let opts = LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        };
        let tree = build_tree(&prepare_scene(&compact).unwrap(), &context, &props, &opts).unwrap();
        let program = emit(&tree, &default_font_naming).unwrap().program;
        let batch = program
            .nodes()
            .iter()
            .find_map(|node| match node {
                Node::InstanceBatch(batch) => Some(batch),
                _ => None,
            })
            .unwrap();
        assert_eq!(batch.instances.len(), 64);
        assert!(batch.instances.opacities[0] >= 0.4);
        assert!(batch.instances.opacities[63] > batch.instances.opacities[0]);
        assert!(batch.instances.opacities[63] < 1.0);
    }
}

#[test]
fn static_path_template_rotates_each_row_around_explicit_zero_origin() {
    let source = path_grid(8).replace(
        "translate: point(dot.x + ctx.seconds * 3, dot.y)",
        "translate: point(dot.x + ctx.seconds * 3, dot.y), rotate: `${dot.delay * 180 + ctx.seconds * 30}deg`, transformOrigin: point(0, 0)",
    );
    let compact = compile_motion(&source).unwrap().artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    assert!(compact.instance_groups[0].static_path_template().is_some());
    let expanded = compile_motion(&source.replace("<Path ", "<path "))
        .unwrap()
        .artifact;
    assert!(expanded.instance_groups.is_empty());
    let context = motion_context_at_frame(30, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&compact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepare_scene(&compact).unwrap(),
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let program = emit(&tree, &default_font_naming).unwrap().program;
    let batch = program
        .nodes()
        .iter()
        .find_map(|node| match node {
            Node::InstanceBatch(batch) => Some(batch),
            _ => None,
        })
        .unwrap();
    assert_eq!(batch.instances.len(), 64);
    assert!(batch.instances.stroke_colors.is_empty());
    assert!(batch.instances.transforms[0].0[1] > 0.0);
    assert!(batch.instances.transforms[1].0[1] > batch.instances.transforms[0].0[1]);

    let unsupported = source.replace(
        "transformOrigin: point(0, 0)",
        "transformOrigin: point(1, 0)",
    );
    assert!(
        compile_motion(&unsupported)
            .unwrap()
            .artifact
            .instance_groups
            .is_empty()
    );
    let mut invalid = compact.clone();
    invalid.instance_groups[0]
        .template
        .styles
        .iter_mut()
        .find(|style| style.property == "transform-origin")
        .unwrap()
        .value = StyleValue::Static {
        value: valle_motion::MotionValue::Point(valle_draw::Point::new(1.0, 0.0)),
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn static_path_template_scales_each_row_around_explicit_zero_origin() {
    let source = path_grid(8).replace(
        "translate: point(dot.x + ctx.seconds * 3, dot.y)",
        "translate: point(dot.x + ctx.seconds * 3, dot.y), scale: point(0.6 + dot.delay * 0.4, 0.7 + ctx.progress * 0.2), transformOrigin: point(0, 0)",
    );
    let large_source = source.replace("const COLS = 8;", "const COLS = 64;");
    let compact = compile_motion(&source).unwrap().artifact;
    let large = compile_motion(&large_source).unwrap().artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    assert_eq!(large.instance_groups.len(), 1);
    assert_eq!(compact.instance_groups[0].rows(), 64);
    assert_eq!(large.instance_groups[0].rows(), 4096);
    assert_eq!(
        compact.instance_groups[0].exprs.len(),
        large.instance_groups[0].exprs.len()
    );
    compact.validate().unwrap();
    let expanded = compile_motion(&source.replace("<Path ", "<path "))
        .unwrap()
        .artifact;
    assert!(expanded.instance_groups.is_empty());
    for frame in [0, 30] {
        let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let props = resolve_props(&compact.controls, &Default::default()).unwrap();
        let fonts = Fonts::default();
        let tree = build_tree(
            &prepare_scene(&compact).unwrap(),
            &context,
            &props,
            &LayoutOptions {
                viewport: Viewport::new((640, 360)),
                fonts: &fonts,
                styles: None,
            },
        )
        .unwrap();
        let program = emit(&tree, &default_font_naming).unwrap().program;
        let batch = program
            .nodes()
            .iter()
            .find_map(|node| match node {
                Node::InstanceBatch(batch) => Some(batch),
                _ => None,
            })
            .unwrap();
        assert_eq!(batch.instances.len(), 64);
        assert!((batch.instances.transforms[0].0[0] - 0.6).abs() < 1.0e-6);
        assert!(
            (batch.instances.transforms[0].0[3] - (0.7 + f64::from(frame) / 60.0 * 0.2)).abs()
                < 1.0e-6
        );
        assert!(batch.instances.transforms[63].0[0] > batch.instances.transforms[0].0[0]);
    }

    let unsupported = source.replace(
        "transformOrigin: point(0, 0)",
        "transformOrigin: point(1, 0)",
    );
    assert!(
        compile_motion(&unsupported)
            .unwrap()
            .artifact
            .instance_groups
            .is_empty()
    );
    let zero = source.replace(
        "scale: point(0.6 + dot.delay * 0.4, 0.7 + ctx.progress * 0.2)",
        "scale: 0",
    );
    assert!(
        compile_motion(&zero)
            .unwrap()
            .artifact
            .instance_groups
            .is_empty()
    );
    let mut invalid = compact.clone();
    invalid.instance_groups[0]
        .template
        .styles
        .iter_mut()
        .find(|style| style.property == "scale")
        .unwrap()
        .value = StyleValue::Static {
        value: valle_motion::MotionValue::Number(0.0),
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn static_path_template_skews_each_row_around_explicit_zero_origin() {
    let source = path_grid(8).replace(
        "translate: point(dot.x + ctx.seconds * 3, dot.y)",
        "translate: point(dot.x + ctx.seconds * 3, dot.y), transform: `skewX(${dot.delay * 40 + ctx.seconds * 10 - 20}deg)`, transformOrigin: point(0, 0)",
    );
    let large_source = source.replace("const COLS = 8;", "const COLS = 64;");
    let compact = compile_motion(&source).unwrap().artifact;
    let large = compile_motion(&large_source).unwrap().artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    assert_eq!(large.instance_groups.len(), 1);
    assert_eq!(compact.instance_groups[0].rows(), 64);
    assert_eq!(large.instance_groups[0].rows(), 4096);
    assert_eq!(
        compact.instance_groups[0].exprs.len(),
        large.instance_groups[0].exprs.len()
    );
    compact.validate().unwrap();
    large.validate().unwrap();

    let static_skew = source.replace(
        "`skewX(${dot.delay * 40 + ctx.seconds * 10 - 20}deg)`",
        "\"skewX(-15deg)\"",
    );
    assert_eq!(
        compile_motion(&static_skew)
            .unwrap()
            .artifact
            .instance_groups
            .len(),
        1
    );
    for unsupported in [
        source.replace(
            "transformOrigin: point(0, 0)",
            "transformOrigin: point(1, 0)",
        ),
        static_skew.replace("skewX(-15deg)", "skewY(-15deg)"),
    ] {
        assert!(
            compile_motion(&unsupported)
                .unwrap()
                .artifact
                .instance_groups
                .is_empty()
        );
    }
    let mut invalid = compile_motion(&static_skew).unwrap().artifact;
    invalid.instance_groups[0]
        .template
        .styles
        .iter_mut()
        .find(|style| style.property == "transform")
        .unwrap()
        .value = StyleValue::Static {
        value: valle_motion::MotionValue::Str("rotate(15deg)".into()),
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn static_path_scale_rotation_matrices_match_expanded_paths() {
    use valle_draw::program::{DrawProgram, NodeId, Transform2d};
    fn matrices(
        program: &DrawProgram,
        id: NodeId,
        parent: Transform2d,
        output: &mut Vec<[f64; 9]>,
    ) {
        match &program.nodes()[id.raw() as usize] {
            Node::Group(group) => {
                let transform = parent.then(group.transform);
                for child in &group.children {
                    matrices(program, *child, transform, output);
                }
            }
            Node::Path(_) => output.push(parent.0),
            Node::InstanceBatch(batch) => {
                for transform in &batch.instances.transforms {
                    output.push(parent.then(Transform2d::from_affine(*transform)).0);
                }
            }
            _ => {}
        }
    }
    let source = path_grid(8).replace(
        "translate: point(dot.x + ctx.seconds * 3, dot.y)",
        "translate: point(dot.x + ctx.seconds * 3, dot.y), scale: point(0.6 - dot.delay, 0.7 + ctx.progress * 0.2), rotate: `${dot.delay * 180 + ctx.seconds * 30 - 40}deg`, transform: `skewX(${dot.delay * 40 + ctx.seconds * 10 - 20}deg)`, transformOrigin: point(0, 0), opacity: 0.4 + dot.delay * 0.4 + ctx.progress * 0.1",
    );
    let zero_row = source.replace(
        "scale: point(0.6 - dot.delay, 0.7 + ctx.progress * 0.2)",
        "scale: dot.delay === 0.5 ? 0 : 1",
    );
    let skew_only = path_grid(8).replace(
        "translate: point(dot.x + ctx.seconds * 3, dot.y)",
        "translate: point(dot.x + ctx.seconds * 3, dot.y), transform: `skewX(${dot.delay * 40 + ctx.seconds * 10 - 20}deg)`, transformOrigin: point(0, 0)",
    );
    let stroked = source.replace(
        "fill={dot.color}",
        "fill={dot.color} stroke={dot.color} strokeWidth={1.5}",
    );
    let fonts = Fonts::default();
    let opts = LayoutOptions {
        viewport: Viewport::new((640, 360)),
        fonts: &fonts,
        styles: None,
    };
    for (source, expected_paths) in [(source, 65), (zero_row, 64), (skew_only, 65), (stroked, 65)] {
        let compact = compile_motion(&source).unwrap().artifact;
        let expanded = compile_motion(&source.replace("<Path ", "<path "))
            .unwrap()
            .artifact;
        assert_eq!(compact.instance_groups.len(), 1);
        assert!(expanded.instance_groups.is_empty());
        for frame in [0, 15, 30] {
            let ctx = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
            let mut programs = Vec::new();
            for artifact in [&compact, &expanded] {
                let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
                let tree =
                    build_tree(&prepare_scene(artifact).unwrap(), &ctx, &props, &opts).unwrap();
                let program = emit(&tree, &default_font_naming).unwrap().program;
                let mut output = Vec::new();
                for root in program.roots() {
                    matrices(&program, *root, Transform2d::IDENTITY, &mut output);
                }
                programs.push(output);
            }
            assert_eq!(programs[0].len(), expected_paths);
            for (index, (compact, expanded)) in
                programs[0].iter().zip(programs[1].iter()).enumerate()
            {
                for (component, (&actual, &expected)) in
                    compact.iter().zip(expanded.iter()).enumerate()
                {
                    if matches!(component, 2 | 5 | 6 | 7 | 8) {
                        assert_eq!(
                            actual, expected,
                            "frame {frame}, path {index}, component {component}"
                        );
                    } else {
                        // The shared path uses pinned libm; Takumi resolves expanded CSS
                        // rotation/skew with platform f32 trig, which can differ by a few ULPs.
                        assert!(
                            (actual - expected).abs() <= f64::from(f32::EPSILON),
                            "frame {frame}, path {index}, component {component}: {actual} vs {expected}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn static_path_template_shares_same_color_dynamic_stroke() {
    let source = path_grid(8)
        .replace(
            "fill={dot.color}",
            "fill={dot.color} stroke={dot.color} strokeWidth={1.5}",
        )
        .replace(
            "translate: point(dot.x + ctx.seconds * 3, dot.y)",
            "translate: point(dot.x + ctx.seconds * 3, dot.y), opacity: 0.4 + dot.delay * 0.4",
        );
    let large_source = source.replace("const COLS = 8;", "const COLS = 64;");
    let compact = compile_motion(&source).unwrap().artifact;
    let large = compile_motion(&large_source).unwrap().artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    assert_eq!(large.instance_groups.len(), 1);
    assert_eq!(compact.instance_groups[0].rows(), 64);
    assert_eq!(large.instance_groups[0].rows(), 4096);
    assert_eq!(
        compact.instance_groups[0].exprs.len(),
        large.instance_groups[0].exprs.len()
    );
    compact.validate().unwrap();
    let valle_motion::NodeKind::Path {
        fill: Some(fill),
        stroke: Some(stroke),
        ..
    } = &compact.instance_groups[0].template.kind
    else {
        panic!("expected shared stroked Path");
    };
    assert_eq!(fill, &stroke.paint);

    let context = motion_context_at_frame(15, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&compact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepare_scene(&compact).unwrap(),
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let program = emit(&tree, &default_font_naming).unwrap().program;
    let batch = program
        .nodes()
        .iter()
        .find_map(|node| match node {
            Node::InstanceBatch(batch) => Some(batch),
            _ => None,
        })
        .unwrap();
    assert_eq!(batch.instances.len(), 64);
    assert!(
        batch
            .instances
            .stroke_widths
            .iter()
            .all(|width| *width == 1.5)
    );

    for accepted in [
        source.replace("strokeWidth={1.5}", ""),
        source.replace(
            "strokeWidth={1.5}",
            "strokeWidth={1.5} strokeLinecap=\"round\"",
        ),
    ] {
        let artifact = compile_motion(&accepted).unwrap().artifact;
        assert_eq!(artifact.instance_groups.len(), 1);
        artifact.validate().unwrap();
    }
    let dynamic = source.replace(
        "strokeWidth={1.5}",
        "strokeWidth={0.25 + dot.delay + ctx.progress}",
    );
    let dynamic_artifact = compile_motion(&dynamic).unwrap().artifact;
    let dynamic_large = compile_motion(&dynamic.replace("const COLS = 8;", "const COLS = 64;"))
        .unwrap()
        .artifact;
    assert_eq!(dynamic_artifact.instance_groups.len(), 1);
    assert_eq!(dynamic_large.instance_groups.len(), 1);
    assert_eq!(dynamic_large.instance_groups[0].rows(), 4096);
    assert_eq!(
        dynamic_artifact.instance_groups[0].exprs.len(),
        dynamic_large.instance_groups[0].exprs.len()
    );
    for frame in [0, 15] {
        let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let props = resolve_props(&dynamic_artifact.controls, &Default::default()).unwrap();
        let tree = build_tree(
            &prepare_scene(&dynamic_artifact).unwrap(),
            &context,
            &props,
            &LayoutOptions {
                viewport: Viewport::new((640, 360)),
                fonts: &fonts,
                styles: None,
            },
        )
        .unwrap();
        let program = emit(&tree, &default_font_naming).unwrap().program;
        let batch = program
            .nodes()
            .iter()
            .find_map(|node| match node {
                Node::InstanceBatch(batch) => Some(batch),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            batch.instances.stroke_widths[0],
            if frame == 0 { 0.25 } else { 0.5 }
        );
        assert!(batch.instances.stroke_widths[63] > batch.instances.stroke_widths[0]);
    }
    let zero = source.replace("strokeWidth={1.5}", "strokeWidth={0}");
    assert!(
        compile_motion(&zero)
            .unwrap()
            .artifact
            .instance_groups
            .is_empty()
    );
    let dynamic_zero = source.replace(
        "strokeWidth={1.5}",
        "strokeWidth={dot.delay + ctx.progress}",
    );
    let zero_artifact = compile_motion(&dynamic_zero).unwrap().artifact;
    assert_eq!(zero_artifact.instance_groups.len(), 1);
    let zero_context = motion_context_at_frame(0, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let zero_props = resolve_props(&zero_artifact.controls, &Default::default()).unwrap();
    assert!(
        build_tree(
            &prepare_scene(&zero_artifact).unwrap(),
            &zero_context,
            &zero_props,
            &LayoutOptions {
                viewport: Viewport::new((640, 360)),
                fonts: &fonts,
                styles: None,
            },
        )
        .is_err()
    );
    let expanded_zero = compile_motion(&dynamic_zero.replace("<Path ", "<path "))
        .unwrap()
        .artifact;
    let expanded_props = resolve_props(&expanded_zero.controls, &Default::default()).unwrap();
    let expanded_tree = build_tree(
        &prepare_scene(&expanded_zero).unwrap(),
        &zero_context,
        &expanded_props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    assert!(emit(&expanded_tree, &default_font_naming).is_err());
}

#[test]
fn static_path_template_keeps_independent_row_and_time_stroke_colors() {
    use valle_draw::program::{DrawProgram, LinearColor};

    let source = path_grid(8).replace(
        "fill={dot.color}",
        "fill={dot.color} stroke={dot.delay < ctx.progress ? '#ff0000' : '#00ff00'} strokeWidth={0.25 + dot.delay + ctx.progress}",
    );
    let compact = compile_motion(&source).unwrap().artifact;
    let expanded = compile_motion(&source.replace("<Path ", "<path "))
        .unwrap()
        .artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    assert!(expanded.instance_groups.is_empty());
    compact.validate().unwrap();
    let fonts = Fonts::default();
    for frame in [0, 15, 30] {
        let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let programs = [&compact, &expanded].map(|artifact| {
            let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
            let tree = build_tree(
                &prepare_scene(artifact).unwrap(),
                &context,
                &props,
                &LayoutOptions {
                    viewport: Viewport::new((640, 360)),
                    fonts: &fonts,
                    styles: None,
                },
            )
            .unwrap();
            emit(&tree, &default_font_naming).unwrap().program
        });
        let batch = programs[0]
            .nodes()
            .iter()
            .find_map(|node| match node {
                Node::InstanceBatch(batch) => Some(batch),
                _ => None,
            })
            .unwrap();
        assert_eq!(batch.instances.len(), 64);
        assert_eq!(batch.instances.stroke_colors.len(), 64);
        assert_eq!(batch.instances.stroke_widths[0], 0.25 + frame as f32 / 60.0);
        assert!(batch.instances.stroke_widths[63] > batch.instances.stroke_widths[0]);
        let green = LinearColor::from_srgb8(valle_draw::Rgba::rgb(0, 255, 0));
        let red = LinearColor::from_srgb8(valle_draw::Rgba::rgb(255, 0, 0));
        assert_eq!(
            batch.instances.stroke_colors[0],
            if frame == 0 { green } else { red }
        );
        assert_eq!(batch.instances.stroke_colors[63], green);
        let packed = programs[0].packed_bytes().unwrap();
        assert_eq!(DrawProgram::from_packed(&packed).unwrap(), programs[0]);
    }
}

#[test]
fn stroke_only_path_instances_share_caps_joins_and_dashes_with_dynamic_phase() {
    use valle_draw::program::{DrawProgram, StrokeCap, StrokeJoin};

    let source = path_grid(8).replace(
        "fill={dot.color}",
        "fill=\"none\" stroke={dot.color} strokeWidth={1.5} strokeLinecap=\"round\" strokeLinejoin=\"bevel\" strokeMiterlimit=\"6\" strokeDasharray=\"3 2\" strokeDashoffset={dot.delay * 5 + ctx.progress}",
    );
    let compact = compile_motion(&source).unwrap().artifact;
    let expanded = compile_motion(&source.replace("<Path ", "<path "))
        .unwrap()
        .artifact;
    assert_eq!(compact.instance_groups.len(), 1);
    assert!(expanded.instance_groups.is_empty());
    compact.validate().unwrap();
    let fonts = Fonts::default();
    for frame in [0, 15, 30] {
        let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let props = resolve_props(&compact.controls, &Default::default()).unwrap();
        let tree = build_tree(
            &prepare_scene(&compact).unwrap(),
            &context,
            &props,
            &LayoutOptions {
                viewport: Viewport::new((640, 360)),
                fonts: &fonts,
                styles: None,
            },
        )
        .unwrap();
        let program = emit(&tree, &default_font_naming).unwrap().program;
        let batch = program
            .nodes()
            .iter()
            .find_map(|node| match node {
                Node::InstanceBatch(batch) => Some(batch),
                _ => None,
            })
            .unwrap();
        let style = batch.path_style.as_ref().unwrap();
        assert!(!style.fill);
        assert_eq!(style.cap, StrokeCap::Round);
        assert_eq!(style.join, StrokeJoin::Bevel);
        assert_eq!(style.miter_limit, 6.0);
        assert_eq!(style.dash, vec![3.0, 2.0]);
        assert_eq!(batch.instances.stroke_colors.len(), 0);
        assert_eq!(batch.instances.dash_offsets.len(), 64);
        assert_eq!(batch.instances.dash_offsets[0], frame as f32 / 60.0);
        assert_eq!(
            batch.instances.dash_offsets[63],
            63.0 / 64.0 * 5.0 + frame as f32 / 60.0
        );
        let expanded_props = resolve_props(&expanded.controls, &Default::default()).unwrap();
        let expanded_tree = build_tree(
            &prepare_scene(&expanded).unwrap(),
            &context,
            &expanded_props,
            &LayoutOptions {
                viewport: Viewport::new((640, 360)),
                fonts: &fonts,
                styles: None,
            },
        )
        .unwrap();
        let expanded_program = emit(&expanded_tree, &default_font_naming).unwrap().program;
        let expanded_offsets = expanded_program
            .nodes()
            .iter()
            .filter_map(|node| match node {
                Node::Path(path) => path.stroke.as_ref().map(|stroke| stroke.dash_offset),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(batch.instances.dash_offsets, expanded_offsets);
        let packed = program.packed_bytes().unwrap();
        assert_eq!(DrawProgram::from_packed(&packed).unwrap(), program);
    }
}

#[test]
fn static_path_template_rejects_unsupported_paint() {
    let source = path_grid(8);
    for variant in [source.replace("d={STAR}", "d={STAR} trimStart={0.25}")] {
        let compiled = compile_motion(&variant).unwrap();
        assert!(compiled.artifact.instance_groups.is_empty());
    }
}

#[test]
fn expanded_maps_explain_why_instancing_was_skipped() {
    let cases = [
        (
            circle_grid(8).replace("fill={dot.color}", "fill={dot.color} stroke=\"#fff\""),
            "Circle stroke",
        ),
        (
            grid(8).replace("className=\"absolute\"", "className=\"absolute \""),
            "exact className",
        ),
        (
            grid(8).replace("left: cell.x", "left: \"10%\""),
            "View has a style value type",
        ),
    ];
    for (source, reason) in cases {
        let compiled = compile_motion(&source).unwrap();
        assert!(compiled.artifact.instance_groups.is_empty());
        let warning = compiled
            .warnings
            .iter()
            .find(|warning| warning.code == DiagCode::InstanceFallback)
            .expect("static map fallback should carry a diagnostic");
        assert!(warning.message.contains(reason), "{}", warning.message);
        assert!(warning.span.line > 0);
    }
    assert!(
        compile_motion(&path_grid(8))
            .unwrap()
            .warnings
            .iter()
            .all(|warning| warning.code != DiagCode::InstanceFallback)
    );
}

#[test]
fn circle_template_is_shared_and_records_exact_arc_paths() {
    let small = compile_motion(&circle_grid(8)).unwrap().artifact;
    let large = compile_motion(&circle_grid(64)).unwrap().artifact;
    for (artifact, rows) in [(&small, 64), (&large, 4096)] {
        assert_eq!(artifact.instance_groups.len(), 1);
        assert_eq!(artifact.instance_groups[0].rows(), rows);
        assert!(
            artifact.instance_groups[0]
                .circle_template_parameters()
                .is_some()
        );
        artifact.validate().unwrap();
    }
    assert_eq!(
        small.instance_groups[0].exprs.len(),
        large.instance_groups[0].exprs.len()
    );
    assert!(compile_motion(&circle_grid(128)).is_err());
    let expanded = compile_motion(&circle_grid(8).replace("<Circle ", "<circle "))
        .unwrap()
        .artifact;
    assert!(expanded.instance_groups.is_empty());
    let prepared = prepare_scene(&small).unwrap();
    let context = motion_context_at_frame(30, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&small.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let program = emit(&tree, &default_font_naming).unwrap().program;
    assert_eq!(
        program
            .nodes()
            .iter()
            .filter(|node| matches!(node, Node::Path(_)))
            .count(),
        65 // 64 circles plus the Scene background
    );
    assert!(
        !program
            .nodes()
            .iter()
            .any(|node| matches!(node, Node::InstanceBatch(_)))
    );
}

#[test]
fn circle_template_validation_rejects_a_non_circle_path() {
    let mut artifact = compile_motion(&circle_grid(8)).unwrap().artifact;
    let group = &mut artifact.instance_groups[0];
    let start_angle = group
        .exprs
        .iter()
        .find_map(|expr| match expr {
            valle_motion::expr::Expr::PathArc { start_angle, .. } => Some(*start_angle),
            _ => None,
        })
        .unwrap();
    group.exprs[start_angle.0 as usize] = valle_motion::expr::Expr::Const {
        value: valle_motion::MotionValue::Number(0.1),
    };
    assert!(artifact.validate().is_err());
}

#[test]
fn circle_template_admits_visibility_and_expands_stroked_shapes() {
    let visible = circle_grid(8).replace(
        "fill={dot.color}",
        "fill={dot.color} visible={dot.radius > 3.5}",
    );
    let artifact = compile_motion(&visible).unwrap().artifact;
    assert_eq!(artifact.instance_groups.len(), 1);
    artifact.validate().unwrap();
    let stroked = circle_grid(8).replace(
        "fill={dot.color}",
        "fill={dot.color} stroke=\"#fff\" strokeWidth={1}",
    );
    assert!(
        compile_motion(&stroked)
            .unwrap()
            .artifact
            .instance_groups
            .is_empty()
    );
}

#[test]
fn indexed_keys_preserve_scoped_ids_and_irregular_ids_stay_explicit() {
    let original = vec!["scope0/t0".into(), "scope0/t1".into(), "scope0/t2".into()];
    let keys = InstanceKeys::from_values(original.clone());
    assert!(matches!(keys, InstanceKeys::Indexed { .. }));
    assert_eq!(
        (0..original.len())
            .map(|index| keys.key_at(index).unwrap())
            .collect::<Vec<_>>(),
        original
    );
    assert!(keys.key_at(original.len()).is_none());
    assert!(matches!(
        InstanceKeys::from_values(vec!["a".into(), "z".into()]),
        InstanceKeys::Explicit(_)
    ));
}

#[test]
fn template_expression_count_is_independent_of_rows() {
    let small = compile_motion(&grid(8)).unwrap().artifact;
    let expression_count = |artifact: &valle_motion::SceneArtifact| {
        artifact.exprs.len()
            + artifact
                .instance_groups
                .iter()
                .map(|group| group.exprs.len())
                .sum::<usize>()
    };
    assert_eq!(small.instance_groups.len(), 1);
    assert_eq!(small.instance_groups[0].rows(), 64);
    assert!(matches!(
        small.instance_groups[0].keys,
        InstanceKeys::Indexed { .. }
    ));
    assert_eq!(
        small.instance_groups[0]
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["d", "x", "y"],
    );
    assert_eq!(expression_count(&small), 21);
    assert!(
        small.instance_groups[0]
            .columns
            .iter()
            .all(|column| matches!(&column.values, InstanceColumnValues::Formula { .. }))
    );
    let small_bytes = serde_json::to_vec(&small.instance_groups[0].columns)
        .unwrap()
        .len();
    let small_key_bytes = serde_json::to_vec(&small.instance_groups[0].keys)
        .unwrap()
        .len();
    for (cols, rows) in [(32, 1024), (64, 4096), (128, 16_384)] {
        let artifact = compile_motion(&grid(cols)).unwrap().artifact;
        assert_eq!(artifact.instance_groups.len(), 1);
        assert_eq!(artifact.instance_groups[0].rows(), rows);
        assert_eq!(expression_count(&artifact), expression_count(&small));
        assert!(
            artifact.instance_groups[0]
                .columns
                .iter()
                .all(|column| matches!(&column.values, InstanceColumnValues::Formula { .. }))
        );
        let bytes = serde_json::to_vec(&artifact.instance_groups[0].columns).unwrap();
        assert!(
            bytes.len() < small_bytes + 100,
            "programmatic numeric columns grew from {small_bytes} to {} bytes",
            bytes.len()
        );
        let key_bytes = serde_json::to_vec(&artifact.instance_groups[0].keys).unwrap();
        assert!(matches!(
            artifact.instance_groups[0].keys,
            InstanceKeys::Indexed { .. }
        ));
        assert!(key_bytes.len() < small_key_bytes + 10);
        assert!(key_bytes.len() < 100);
        for index in [0, rows / 2, rows - 1] {
            assert_eq!(
                artifact.instance_groups[0].keys.key_at(index).unwrap(),
                format!("t{index}")
            );
        }
    }
}

#[test]
fn template_lowers_one_batch_per_frame() {
    let compiled = compile_motion(&grid(8)).unwrap();
    let prepared = prepare_scene(&compiled.artifact).unwrap();
    let context = motion_context_at_frame(30, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&compiled.artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((640, 360)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let program = emit(&tree, &default_font_naming).unwrap().program;
    let batches = program
        .nodes()
        .iter()
        .filter_map(|node| match node {
            Node::InstanceBatch(batch) => Some(batch),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].instances.len(), 64);
    let transform = batches[0].instances.transforms[0].0;
    assert!((transform[0].hypot(transform[1]) - 4.0).abs() < 1e-5);
    assert!((transform[2].hypot(transform[3]) - 4.0).abs() < 1e-5);
}

#[test]
fn ineligible_map_keeps_the_expanded_scene() {
    let source = grid(8).replace("className=\"absolute\"", "className=\"absolute \"");
    let compiled = compile_motion(&source).unwrap();
    assert!(compiled.artifact.instance_groups.is_empty());
    assert_eq!(compiled.artifact.nodes.len(), 65);
    assert_eq!(compiled.artifact.exprs.len(), 1088);
    let percent_position = grid(8).replace("left: cell.x", "left: \"10%\"");
    let compiled = compile_motion(&percent_position).unwrap();
    assert!(compiled.artifact.instance_groups.is_empty());
    assert_eq!(compiled.artifact.nodes.len(), 65);
}

#[test]
fn unrecognized_numeric_generator_keeps_authored_column() {
    let source = grid(8).replace("8 + c * (624 / COLS)", "8 + Math.sin(i) * 4");
    let artifact = compile_motion(&source).unwrap().artifact;
    let columns = &artifact.instance_groups[0].columns;
    assert!(matches!(
        columns
            .iter()
            .find(|column| column.name == "x")
            .unwrap()
            .values,
        InstanceColumnValues::Numbers(_)
    ));
    assert!(matches!(
        columns
            .iter()
            .find(|column| column.name == "y")
            .unwrap()
            .values,
        InstanceColumnValues::Formula { .. }
    ));
}

#[test]
fn instance_table_rejects_tampered_rows_and_references() {
    let mut artifact = compile_motion(&grid(8)).unwrap().artifact;
    let mut keys = (0..64)
        .map(|index| artifact.instance_groups[0].keys.key_at(index).unwrap())
        .collect::<Vec<_>>();
    keys[1] = keys[0].clone();
    artifact.instance_groups[0].keys = InstanceKeys::Explicit(keys);
    assert!(artifact.validate().is_err());
    let mut artifact = compile_motion(&grid(8)).unwrap().artifact;
    if let InstanceKeys::Indexed { rows, .. } = &mut artifact.instance_groups[0].keys {
        *rows -= 1;
    } else {
        panic!("instances must have indexed keys");
    }
    assert!(artifact.validate().is_err());
    let mut artifact = compile_motion(&grid(8)).unwrap().artifact;
    if let InstanceColumnValues::Formula { expression, .. } =
        &mut artifact.instance_groups[0].columns[0].values
    {
        *expression = valle_motion::IndexFormula::Constant(f64::NAN);
    } else {
        panic!("instances must use programmatic numeric columns");
    }
    assert!(artifact.validate().is_err());
    let mut artifact = compile_motion(&grid(8)).unwrap().artifact;
    let mut deep = valle_motion::IndexFormula::Index;
    for _ in 0..33 {
        deep = valle_motion::IndexFormula::Neg(Box::new(deep));
    }
    artifact.instance_groups[0].columns[0].values = InstanceColumnValues::Formula {
        rows: 64,
        expression: deep,
    };
    assert!(artifact.validate().is_err());
    let mut artifact = compile_motion(&grid(8)).unwrap().artifact;
    match &mut artifact.instance_groups[0].columns[0].values {
        InstanceColumnValues::Numbers(values) => {
            values.pop();
        }
        InstanceColumnValues::Formula { rows, .. } => {
            *rows -= 1;
        }
        _ => panic!("instances numeric column lost its numeric representation"),
    }
    assert!(artifact.validate().is_err());
    let mut artifact = compile_motion(&grid(8)).unwrap().artifact;
    let column = &mut artifact.instance_groups[0].columns[0];
    let values = (0..column.values.len())
        .map(|index| column.values.get(index).unwrap())
        .collect();
    column.values = InstanceColumnValues::Other(values);
    assert!(
        artifact.validate().is_err(),
        "numeric values have one canonical column encoding"
    );
    let mut artifact = compile_motion(&grid(8)).unwrap().artifact;
    let node = artifact
        .nodes
        .iter_mut()
        .find(|node| matches!(node.kind, valle_motion::NodeKind::InstanceBatch { .. }))
        .unwrap();
    node.kind = valle_motion::NodeKind::InstanceBatch { group: 99 };
    assert!(artifact.validate().is_err());
}

#[test]
fn template_style_values_match_expanded_rows() {
    let instanced = compile_motion(&grid(8)).unwrap().artifact;
    let expanded =
        compile_motion(&grid(8).replace("className=\"absolute\"", "className=\"absolute \""))
            .unwrap()
            .artifact;
    for frame in [0, 15, 30] {
        let context = motion_context_at_frame(frame, 60, FrameRate::new(30, 1).unwrap()).unwrap();
        let props = resolve_props(&instanced.controls, &Default::default()).unwrap();
        let inputs = EvalInputs {
            ctx: &context,
            props: &props,
            unit: None,
            viewport: Some((640.0, 360.0)),
        };
        let rows = eval_instances(&instanced.instance_groups[0], inputs).unwrap();
        let values = eval_all(&expanded, inputs).unwrap();
        for (index, row) in rows.iter().enumerate() {
            let template = &instanced.instance_groups[0].template;
            let node = &expanded.nodes[index + 1];
            for property in [
                "left",
                "top",
                "width",
                "height",
                "background-color",
                "opacity",
                "transform",
            ] {
                let find = |node: &valle_motion::SceneNode,
                            values: &[valle_motion::MotionValue]| {
                    node.styles
                        .iter()
                        .find(|style| style.property == property)
                        .map(|style| match &style.value {
                            StyleValue::Static { value } => value.clone(),
                            StyleValue::Expr { expr } => values[expr.0 as usize].clone(),
                        })
                };
                assert_eq!(
                    find(template, row),
                    find(node, &values),
                    "frame={frame} row={index} {property}"
                );
            }
        }
    }
}

#[test]
fn index_and_count_bind_to_each_instance_row() {
    let source = grid(8)
        .replace("CELLS.map((cell) =>", "CELLS.map((cell, i) =>")
        .replace(
            "clamp(ctx.seconds * 2 - cell.d, 0, 1)",
            "clamp(ctx.seconds * 2 - cell.d + i / ctx.instance.count, 0, 1)",
        );
    let instanced = compile_motion(&source).unwrap().artifact;
    let group = &instanced.instance_groups[0];
    assert!(
        group
            .exprs
            .iter()
            .any(|expr| matches!(expr, valle_motion::Expr::InstanceIndex))
    );
    assert!(
        group
            .exprs
            .iter()
            .any(|expr| matches!(expr, valle_motion::Expr::InstanceCount))
    );
    let expanded = compile_motion(
        &source
            .replace("className=\"absolute\"", "className=\"absolute \"")
            .replace("ctx.instance.count", "64"),
    )
    .unwrap()
    .artifact;
    let context = motion_context_at_frame(15, 60, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&instanced.controls, &Default::default()).unwrap();
    let inputs = EvalInputs {
        ctx: &context,
        props: &props,
        unit: None,
        viewport: Some((640.0, 360.0)),
    };
    let rows = eval_instances(group, inputs).unwrap();
    let expanded_values = eval_all(&expanded, inputs).unwrap();
    let opacity = |node: &valle_motion::SceneNode, values: &[valle_motion::MotionValue]| {
        node.styles
            .iter()
            .find(|style| style.property == "opacity")
            .map(|style| match &style.value {
                StyleValue::Static { value } => value.clone(),
                StyleValue::Expr { expr } => values[expr.0 as usize].clone(),
            })
    };
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(
            opacity(&group.template, row),
            opacity(&expanded.nodes[index + 1], &expanded_values),
            "row {index}",
        );
    }
}

#[test]
fn color_columns_keep_per_row_paint_after_packing() {
    let source = r##"
export const composition = { width: 64, height: 32, fps: 30, duration: 1 };
const ITEMS = [{ key: "red", x: 4, color: "#ff0000" }, { key: "green", x: 36, color: "#00ff00" }];
export default function Colors() {
  return <Scene style={{width:64,height:32}}>
    {ITEMS.map((item) => <View key={item.key} className="absolute"
      style={{left:item.x,top:4,width:20,height:20,backgroundColor:item.color}} />)}
  </Scene>;
}
"##;
    let artifact = compile_motion(source).unwrap().artifact;
    assert_eq!(artifact.instance_groups.len(), 1);
    assert!(matches!(
        artifact.instance_groups[0].keys,
        InstanceKeys::Explicit(_)
    ));
    let columns = &artifact.instance_groups[0].columns;
    assert!(
        columns
            .iter()
            .any(|column| matches!(&column.values, InstanceColumnValues::Numbers(_)))
    );
    assert!(
        columns
            .iter()
            .any(|column| matches!(&column.values, InstanceColumnValues::Colors(_)))
    );

    let prepared = prepare_scene(&artifact).unwrap();
    let context = motion_context_at_frame(0, 30, FrameRate::new(30, 1).unwrap()).unwrap();
    let props = resolve_props(&artifact.controls, &Default::default()).unwrap();
    let fonts = Fonts::default();
    let tree = build_tree(
        &prepared,
        &context,
        &props,
        &LayoutOptions {
            viewport: Viewport::new((64, 32)),
            fonts: &fonts,
            styles: None,
        },
    )
    .unwrap();
    let program = emit(&tree, &default_font_naming).unwrap().program;
    let packed = program.packed_bytes().unwrap();
    let decoded = valle_draw::program::DrawProgram::from_packed(&packed).unwrap();
    let batch = decoded
        .nodes()
        .iter()
        .find_map(|node| match node {
            Node::InstanceBatch(batch) => Some(batch),
            _ => None,
        })
        .unwrap();
    assert_eq!(batch.instances.len(), 2);
    assert_ne!(batch.instances.colors[0], batch.instances.colors[1]);
}
