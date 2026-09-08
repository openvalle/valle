#![cfg(feature = "motion")]
//! Reference examples cover typography, sequences, modifiers and procedural data scenes.
use valle_compiler::motion::{MeasureEnv, compile_motion, compile_motion_with_env};
use valle_motion::{
    ContentDigest, FONT_ASSET_CAPABILITY, MOTION_MATH_CAPABILITY, NUMBER_FORMAT_CAPABILITY,
    RICH_TEXT_CAPABILITY, ResourceRef,
};

const FONT: &[u8] =
    include_bytes!("../../../assets/fonts/noto/NotoSansCJKsc-Regular.otf");
const BRAND: &str = include_str!("fixtures/motion/authoring/project-font-brand.motion.tsx");
const TOUR: &str = include_str!("fixtures/motion/authoring/sequence-product-tour.motion.tsx");
const DASHBOARD: &str = include_str!("fixtures/motion/authoring/procedural-dashboard.motion.tsx");
const METRIC: &str = include_str!("fixtures/motion/authoring/rich-metric-title.motion.tsx");

fn compile_with_brand_font(source: &str) -> valle_motion::SceneArtifact {
    let hash = ContentDigest::of_bytes(FONT);
    let measure = MeasureEnv::new_with_aliases(
        &[],
        &[("asset://brandFont".into(), FONT.to_vec())],
        (1280, 720),
    )
    .expect("font environment");
    compile_motion_with_env(
        source,
        &[ResourceRef {
            control: "brandFont".into(),
            content_hash: hash,
        }],
        Some(&measure),
    )
    .expect("font-bound reference example compiles")
    .artifact
}

fn has(artifact: &valle_motion::SceneArtifact, capability: &str) -> bool {
    artifact
        .capability_set
        .names
        .iter()
        .any(|name| name == capability)
}

#[test]
fn typography_and_sequence_reference_examples_compile_from_the_public_surface() {
    let brand = compile_with_brand_font(BRAND);
    let tour = compile_motion(TOUR)
        .expect("Sequence tour compiles")
        .artifact;
    let dashboard = compile_motion(DASHBOARD)
        .expect("procedural dashboard compiles")
        .artifact;
    let metric = compile_with_brand_font(METRIC);

    for artifact in [&brand, &tour, &dashboard, &metric] {
        artifact
            .validate()
            .expect("admissible reference example artifact");
        assert!(
            artifact.nodes.len() >= 10,
            "reference film is not a toy probe"
        );
    }
    assert!(has(&brand, FONT_ASSET_CAPABILITY));
    assert!(has(&brand, RICH_TEXT_CAPABILITY));
    assert!(has(&tour, MOTION_MATH_CAPABILITY));
    assert!(has(&dashboard, MOTION_MATH_CAPABILITY));
    assert!(has(&dashboard, NUMBER_FORMAT_CAPABILITY));
    assert!(has(&metric, FONT_ASSET_CAPABILITY));
    assert!(has(&metric, RICH_TEXT_CAPABILITY));
    assert!(has(&metric, NUMBER_FORMAT_CAPABILITY));
}

#[test]
fn modifier_and_data_reference_examples_compile_from_the_public_surface() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in [
        "modifier-logo-echo",
        "path-formation",
        "dashboard-flip",
        "particle-data-stream",
    ] {
        let path = root.join(format!(
            "crates/valle-compiler/tests/fixtures/motion/animation/{name}.motion.tsx"
        ));
        let source = std::fs::read_to_string(&path).expect("read modifier reference example");
        compile_motion(&source).unwrap_or_else(|errors| panic!("{}: {errors:#?}", path.display()));
    }
}
