#![cfg(feature = "motion")]
//! Structured data binding, prepare topology and content invalidation.
use std::path::PathBuf;

use serde_json::json;
use valle_compiler::motion::{
    MotionModuleGraph, PrepareDataBinding, compile_motion_modules_with_full_env_and_data,
    compile_motion_with_full_env_and_data,
};
use valle_motion::NodeKind;

const SOURCE: &str = r#"
export const controls = defineControls({
  data: {
    rows: array(
      record({ id: string(), label: string(), value: number({ min: 0 }) }),
      { minItems: 1, maxItems: 16, key: "id" },
    ),
    origin: point(),
    palette: tuple([color(), color()]),
  },
});

export default function DataBars(ctx, props, signals, data) {
  const visible = data.rows.filter((item) => item.value > 0);
  const total = data.rows.reduce((sum, item) => sum + item.value, 0);
  return <Group key="root">
    {visible.map((item, index) => {
      const width = item.value / total * 100;
      const x = data.origin[0] + index * 12;
      return <View key={item.id} style={{ width, left: x }} />;
    })}
  </Group>;
}
"#;

fn binding(rows: serde_json::Value) -> PrepareDataBinding {
    PrepareDataBinding {
        source: "fixtures/dashboard.json".into(),
        value: json!({
            "rows": rows,
            "origin": [8, 12],
            "palette": ["#67e8f9", "#f472b6"],
        }),
    }
}

#[test]
fn structured_data_drives_prepare_topology_and_block_map_locals() {
    let compiled = compile_motion_with_full_env_and_data(
        SOURCE,
        &[],
        None,
        None,
        Some(&binding(json!([
            {"id": "a", "label": "A", "value": 2},
            {"id": "b", "label": "B", "value": 3},
        ]))),
    )
    .unwrap();

    assert_eq!(compiled.artifact.controls.data.len(), 3);
    assert_eq!(compiled.artifact.nodes.len(), 3);
    assert_eq!(compiled.artifact.nodes[1].key, "a");
    assert_eq!(compiled.artifact.nodes[2].key, "b");
}

#[test]
fn data_content_and_cardinality_invalidate_prepare_without_changing_source_hash() {
    let one = compile_motion_with_full_env_and_data(
        SOURCE,
        &[],
        None,
        None,
        Some(&binding(json!([{"id": "a", "label": "A", "value": 2}]))),
    )
    .unwrap();
    let two = compile_motion_with_full_env_and_data(
        SOURCE,
        &[],
        None,
        None,
        Some(&binding(json!([
            {"id": "a", "label": "A", "value": 2},
            {"id": "b", "label": "B", "value": 3},
        ]))),
    )
    .unwrap();

    assert_eq!(one.normalized_ast_digest, two.normalized_ast_digest);
    assert_ne!(one.prepared_data_digest, two.prepared_data_digest);
    assert_ne!(one.artifact.nodes.len(), two.artifact.nodes.len());
}

#[test]
fn data_binding_is_required_and_reports_json_pointer_against_its_source() {
    let missing = valle_compiler::motion::compile_motion(SOURCE).unwrap_err();
    assert!(
        missing
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("no data binding was supplied") })
    );

    let duplicate = compile_motion_with_full_env_and_data(
        SOURCE,
        &[],
        None,
        None,
        Some(&binding(json!([
            {"id": "same", "label": "A", "value": 2},
            {"id": "same", "label": "B", "value": 3},
        ]))),
    )
    .unwrap_err();
    assert!(duplicate.iter().any(|diagnostic| {
        diagnostic.source_path.as_deref() == Some("fixtures/dashboard.json")
            && diagnostic.message.contains("/data/rows/1/id")
            && diagnostic.message.contains("duplicates item 0")
    }));
}

#[test]
fn frame_values_still_cannot_choose_prepare_cardinality() {
    let source = r#"
export default function IllegalTopology(ctx) {
  return <Group>{Array.from({ length: ctx.frame }, (_, i) => i).map((i) => <View key={i} />)}</Group>;
}
"#;
    let errors = valle_compiler::motion::compile_motion(source).unwrap_err();
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.message.contains("prepare-time static")
            || diagnostic.message.contains("frame-time")
    }));
}

#[test]
fn bounded_for_of_expands_an_explicit_jsx_accumulator() {
    let source = r#"
export const controls = defineControls({
  data: {
    rows: array(record({ id: string(), value: number() }), { maxItems: 8, key: "id" }),
  },
});
export default function ForOfBars(ctx, props, signals, data) {
  const bars = [];
  for (const item of data.rows) {
    const width = item.value * 10;
    bars.push(<View key={item.id} style={{ width }} />);
  }
  return <Group key="root">{bars}</Group>;
}
"#;
    let compiled = compile_motion_with_full_env_and_data(
        source,
        &[],
        None,
        None,
        Some(&PrepareDataBinding {
            source: "for-of.json".into(),
            value: json!({
                "rows": [
                    {"id": "a", "value": 2},
                    {"id": "b", "value": 4},
                ],
            }),
        }),
    )
    .unwrap();
    assert_eq!(compiled.artifact.nodes.len(), 3);
    assert_eq!(compiled.artifact.nodes[1].key, "a");
    assert_eq!(compiled.artifact.nodes[2].key, "b");
}

#[test]
fn external_data_drives_the_multifile_chart_map_diagram_reference_without_domain_ir() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let example =
        workspace.join("crates/valle-compiler/tests/fixtures/motion/modules/data-dashboard");
    let entry = example.join("dashboard.motion.tsx");
    let graph =
        MotionModuleGraph::from_entry_path(&entry).expect("load data-dashboard module closure");
    let binding = PrepareDataBinding {
        source:
            "crates/valle-compiler/tests/fixtures/motion/modules/data-dashboard/dashboard.data.json"
                .into(),
        value: serde_json::from_slice(
            &std::fs::read(example.join("dashboard.data.json")).expect("read dashboard data"),
        )
        .expect("parse dashboard data"),
    };
    let compiled =
        compile_motion_modules_with_full_env_and_data(&graph, &[], None, None, Some(&binding))
            .unwrap_or_else(|diagnostics| {
                panic!("data-dashboard reference must compile: {diagnostics:#?}")
            });

    assert_eq!(compiled.artifact.component, "DataDashboard");
    assert_eq!(compiled.source_map.modules.len(), 5);
    assert_eq!(compiled.artifact.controls.data.len(), 4);
    assert!(
        compiled.artifact.nodes.iter().all(|node| matches!(
            node.kind,
            NodeKind::Group | NodeKind::Box | NodeKind::Text { .. } | NodeKind::Path { .. }
        )),
        "the reference must lower to ordinary Motion nodes"
    );
    for key in ["chart", "map", "diagram"] {
        assert!(
            compiled
                .source_map
                .nodes
                .iter()
                .any(|mapping| mapping.key.starts_with(key)),
            "{key} panel remains source-addressable"
        );
    }
}
