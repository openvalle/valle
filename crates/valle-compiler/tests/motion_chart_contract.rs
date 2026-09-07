#![cfg(feature = "motion")]
//! Chart public contract tests.

use valle_compiler::motion::compile_motion;
use valle_motion::{ARTIFACT_FORMAT_VERSION, NodeKind};

const CONTRACT: &str = include_str!("fixtures/motion/charts/chart-contract.motion.tsx");
const LINE_CHART: &str = include_str!("fixtures/motion/charts/line-chart.motion.tsx");
const STACKED_BARS: &str = include_str!("fixtures/motion/charts/stacked-bars.motion.tsx");

#[test]
fn chart_source_package_examples_compile_from_the_public_surface() {
    for (name, source) in [
        ("contract", CONTRACT),
        ("line-chart", LINE_CHART),
        ("stacked-bars", STACKED_BARS),
    ] {
        let compiled = compile_motion(source)
            .unwrap_or_else(|diagnostics| panic!("{name} must compile: {diagnostics:#?}"));
        compiled
            .artifact
            .validate()
            .expect("valid package artifact");
        assert!(
            compiled.artifact.nodes.len() >= 20,
            "{name} is only a toy probe"
        );
    }
}

#[test]
fn chart_contract_expands_to_ordinary_nodes_and_one_artifact_format() {
    let compiled = compile_motion(CONTRACT).expect("Chart contract source compiles");
    compiled
        .artifact
        .validate()
        .expect("Chart artifact validates");
    assert_eq!(compiled.artifact.format_version, ARTIFACT_FORMAT_VERSION);
    assert_eq!(ARTIFACT_FORMAT_VERSION, 1);
    assert!(compiled.artifact.nodes.iter().all(|node| {
        matches!(
            node.kind,
            NodeKind::Group | NodeKind::Box | NodeKind::Text { .. } | NodeKind::Path { .. }
        )
    }));
}

#[test]
fn animated_domain_keeps_both_prepared_tick_subtrees_and_stable_keys() {
    let first = compile_motion(CONTRACT).expect("first compile");
    let second = compile_motion(CONTRACT).expect("second compile");
    let keys = first
        .artifact
        .nodes
        .iter()
        .map(|node| node.key.as_str())
        .collect::<Vec<_>>();
    assert!(
        keys.iter()
            .any(|key| key.contains("from-axis") && key.contains("grid"))
    );
    assert!(
        keys.iter()
            .any(|key| key.contains("to-axis") && key.contains("grid"))
    );
    assert!(
        keys.iter()
            .any(|key| key.contains("from-axis") && key.contains("tick"))
    );
    assert!(
        keys.iter()
            .any(|key| key.contains("to-axis") && key.contains("tick"))
    );
    assert_eq!(
        valle_motion::canonical_bytes(&first.artifact).unwrap(),
        valle_motion::canonical_bytes(&second.artifact).unwrap()
    );
    assert_eq!(first.source_map, second.source_map);
}

#[test]
fn theme_controls_are_explicit_typed_scalars_and_instance_local() {
    let compiled = compile_motion(CONTRACT).expect("contract compiles");
    for name in ["accent", "grid", "label"] {
        let control = compiled
            .artifact
            .controls
            .props
            .get(name)
            .unwrap_or_else(|| panic!("{name} control exists"));
        assert!(matches!(control.control, valle_motion::ControlType::Color));
    }
    assert!(!CONTRACT.contains("ThemeProvider"));
    assert!(!CONTRACT.contains("useTheme"));
}

#[test]
fn ordinary_marks_keep_item_addresses_and_do_not_silently_batch() {
    let compiled = compile_motion(CONTRACT).expect("contract compiles");
    let keys = compiled
        .artifact
        .nodes
        .iter()
        .map(|node| node.key.as_str())
        .collect::<Vec<_>>();
    for index in 0..4 {
        let key = format!("main/bar-{index}/bar");
        assert!(
            compiled.artifact.nodes.iter().any(|node| node.key == key),
            "missing {key}; got {keys:?}"
        );
        assert!(
            compiled
                .source_map
                .nodes
                .iter()
                .any(|mapping| mapping.key == key)
        );
    }
    assert!(
        compiled
            .artifact
            .nodes
            .iter()
            .all(|node| { !matches!(node.kind, NodeKind::GeometryBatch { .. }) })
    );
}

#[test]
fn duplicate_datum_ids_fail_at_the_existing_stable_key_gate() {
    let duplicate = CONTRACT.replace(
        "const LABELS = [\"Q1\", \"Q2\", \"Q3\", \"Q4\"]",
        "const LABELS = [\"Q1\", \"Q1\", \"Q3\", \"Q4\"]",
    );
    // The contract keys by prepared index today, so duplicate display labels are legal. This test
    // instead proves why the documented Datum.id must become the call key in concrete data shells.
    compile_motion(&duplicate).expect("labels are content, not semantic ids");

    let bad = r#"
const DATA = [{id:"same"},{id:"same"}];
function Mark(ctx,{datum}) { return <View key="mark" style={{width:10,height:10}}/>; }
export default function Chart(){return <Scene key="scene">{DATA.map((datum)=><Mark key={datum.id} datum={datum}/>)}</Scene>;}
"#;
    let diagnostics = compile_motion(bad).expect_err("duplicate stable ids must fail");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("duplicate node key"))
    );
}

#[test]
fn runtime_scale_and_runtime_list_cardinality_remain_illegal() {
    for source in [
        include_str!("fixtures/motion/invalid-inputs/chart.negative.motion.tsx"),
        include_str!("fixtures/motion/invalid-inputs/data-story.negative.motion.tsx"),
    ] {
        compile_motion(source).expect_err("Chart structure and compute must stay prepare-time");
    }
}

#[test]
fn ordinary_part_and_shell_names_compile_away() {
    let renamed = CONTRACT
        .replace("function Axis(", "function CustomAxis(")
        .replace("<Axis key=", "<CustomAxis key=")
        .replace("function Bar(", "function CustomBar(")
        .replace("<Bar key=", "<CustomBar key=")
        .replace("function BarChart(", "function CustomChart(")
        .replace("<BarChart key=", "<CustomChart key=");
    let original = compile_motion(CONTRACT).expect("original");
    let renamed = compile_motion(&renamed).expect("renamed ordinary parts");
    assert_eq!(
        valle_motion::canonical_bytes(&original.artifact).unwrap(),
        valle_motion::canonical_bytes(&renamed.artifact).unwrap(),
        "local component vocabulary is source organization, not domain IR"
    );
    let wire = serde_json::to_string(&original.artifact).unwrap();
    for source_only_name in ["BarChart", "AxisLabel", "GridLine", "Legend"] {
        assert!(!wire.contains(source_only_name));
    }
}

#[test]
fn ai_style_data_and_part_edits_stay_inside_the_frozen_contract() {
    let edited = CONTRACT
        .replace("default: \"#38bdf8\"", "default: \"#f97316\"")
        .replace("const TO = [42, 31, 72, 88]", "const TO = [36, 64, 58, 94]")
        .replace("borderRadius: 5", "borderRadius: 9");
    let before = compile_motion(CONTRACT).expect("before");
    let after = compile_motion(&edited).expect("AI-style edit compiles");
    assert_eq!(before.artifact.nodes.len(), after.artifact.nodes.len());
    assert_eq!(
        before
            .artifact
            .nodes
            .iter()
            .map(|node| &node.key)
            .collect::<Vec<_>>(),
        after
            .artifact
            .nodes
            .iter()
            .map(|node| &node.key)
            .collect::<Vec<_>>()
    );
    assert_ne!(
        valle_motion::canonical_bytes(&before.artifact).unwrap(),
        valle_motion::canonical_bytes(&after.artifact).unwrap(),
        "content/theme edit must change the artifact while retaining the public topology"
    );
}
