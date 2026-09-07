#![cfg(feature = "motion")]
//! Rich text spans lower to ordinary text nodes with dynamic number formatting.
use valle_compiler::motion::compile_motion;
use valle_motion::RICH_TEXT_CAPABILITY;

#[test]
fn span_children_lower_to_the_existing_inline_text_path() {
    let compiled = compile_motion(
        r##"
export const controls = defineControls({ props: { value: number({ default: 42 }) } });
export default function Card(ctx, props) {
  return (
    <Text style={{ fontSize: 48 }}>
      {"Revenue "}
      <Span style={{ color: "#67e8f9", fontWeight: 700 }}>
        {formatNumber(props.value, { decimals: 1 })}
      </Span>
    </Text>
  );
}
"##,
    )
    .expect("fixed rich-text runs compile");
    assert!(
        compiled
            .artifact
            .capability_set
            .names
            .iter()
            .any(|name| name == RICH_TEXT_CAPABILITY)
    );
    assert_eq!(
        compiled
            .artifact
            .nodes
            .iter()
            .filter(|node| matches!(node.kind, valle_motion::NodeKind::Text { .. }))
            .count(),
        2,
        "Span syntax lowers to adjacent existing Text leaves"
    );
}
