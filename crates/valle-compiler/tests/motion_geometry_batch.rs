#![cfg(feature = "motion")]
//! Explicit geometry batches and independent styles of ordinary repeated nodes.
use valle_compiler::motion::compile_motion;
use valle_motion::{MotionValue, NodeKind, StyleValue};

#[test]
fn geometry_batch_lowers_to_one_explicit_leaf() {
    let compiled = compile_motion(
        r##"
const PTS = [point(10, 10), point(20, 20)];
export default function P(ctx) {
  return <GeometryBatch key="dots" geometry="circle" positions={PTS} sizes={4} fills="#fff" />;
}
"##,
    )
    .expect("GeometryBatch compiles");
    assert!(
        compiled
            .artifact
            .nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::GeometryBatch { .. }))
    );
}

#[test]
fn ordinary_static_nodes_keep_independent_styles_not_a_hidden_batch() {
    let compiled = compile_motion(
        r##"
const PTS = [{x:10,y:10},{x:20,y:20}];
export default function P(ctx) {
  return <View>{PTS.map((p, i) => <View key={`p-${i}`} style={{position:"absolute",left:p.x,top:p.y,width:4,height:4}} />)}</View>;
}
"##,
    )
    .expect("ordinary points compile");
    let lefts = compiled
        .artifact
        .nodes
        .iter()
        .flat_map(|node| &node.styles)
        .filter(|style| style.property == "left")
        .filter_map(|style| match &style.value {
            StyleValue::Static {
                value: MotionValue::Number(value),
            } => Some(*value),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(lefts, vec![10.0, 20.0]);
}
