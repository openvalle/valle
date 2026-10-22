#![cfg(feature = "motion")]
//! Rich text spans lower to ordinary text nodes with dynamic number formatting.
use valle_compiler::motion::compile_motion;
use valle_motion::RICH_TEXT_CAPABILITY;

#[test]
fn span_children_lower_to_the_existing_inline_text_path() {
    let compiled = compile_motion(
        r##"
export const controls = ({ props: { value: number({ default: 42 }) } });
export default function Card(props) {
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

#[test]
fn a_prepared_list_maps_to_the_spans_of_one_paragraph() {
    let compiled = compile_motion(
        r##"
const WORDS = [{ text: "Build ", color: "#ffffff" }, { text: "your ", color: "#facc15" }, { text: "story", color: "#ffffff" }];
export default function Card() {
  return <Text style={{ fontSize: 48 }}>{WORDS.map((word) => <Span style={{ color: word.color }}>{word.text}</Span>)}</Text>;
}
"##,
    )
    .expect("a prepared map draws Spans");
    let runs = compiled
        .artifact
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            valle_motion::NodeKind::Text {
                text: valle_motion::TextValue::Static { value },
                ..
            } => Some(value.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(runs, ["Build ", "your ", "story"]);
}
