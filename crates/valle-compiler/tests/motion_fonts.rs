#![cfg(feature = "motion")]
//! Content-addressed author fonts and missing-resource diagnostics.
use valle_compiler::motion::{MeasureEnv, compile_motion, compile_motion_with_env};
use valle_motion::{ContentDigest, FONT_ASSET_CAPABILITY, MotionValue, ResourceRef, StyleValue};
const FONT: &[u8] =
    include_bytes!("../../../assets/fonts/noto/NotoSansCJKsc-Regular.otf");

fn diagnostic_messages(source: &str) -> Vec<String> {
    compile_motion(source)
        .expect_err("invalid font binding must fail closed")
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

fn has_message(messages: &[String], needles: &[&str]) -> bool {
    messages
        .iter()
        .any(|message| needles.iter().all(|needle| message.contains(needle)))
}

#[test]
fn font_control_lowers_to_a_content_addressed_family() {
    let hash = ContentDigest::of_bytes(FONT);
    let measure = MeasureEnv::new_with_aliases(
        &[],
        &[("asset://brandFont".into(), FONT.to_vec())],
        (320, 180),
    )
    .expect("aliased measure font");
    let compiled = compile_motion_with_env(
        r##"
export const controls = defineControls({
  assets: { brandFont: asset({ kind: "font", required: true }) },
});
export default function Card(ctx) {
  return <Text style={{ fontFamily: "asset://brandFont" }}>VALLE</Text>;
}
"##,
        &[ResourceRef {
            control: "brandFont".into(),
            content_hash: hash.clone(),
        }],
        Some(&measure),
    )
    .expect("bound font control compiles");
    assert!(
        compiled
            .artifact
            .capability_set
            .names
            .iter()
            .any(|name| name == FONT_ASSET_CAPABILITY)
    );
    assert_eq!(compiled.artifact.resource_refs.len(), 1);
    let family = compiled
        .artifact
        .nodes
        .iter()
        .flat_map(|node| &node.styles)
        .find(|style| style.property == "font-family")
        .and_then(|style| match &style.value {
            StyleValue::Static {
                value: MotionValue::Str(value),
            } => Some(value.as_str()),
            _ => None,
        });
    let expected_family = valle_motion::font_family_alias(&hash);
    assert_eq!(family, Some(expected_family.as_str()));
}

#[test]
fn used_font_control_without_a_binding_still_fails_closed() {
    let messages = diagnostic_messages(
        r##"
export const controls = defineControls({
  assets: { brandFont: asset({ kind: "font", required: true }) },
});
export default function Card(ctx) {
  return <Text style={{ fontFamily: "asset://brandFont" }}>VALLE</Text>;
}
"##,
    );
    assert!(has_message(&messages, &["brandFont", "no bound resource"]));
}
