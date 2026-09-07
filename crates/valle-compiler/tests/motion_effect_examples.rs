#![cfg(feature = "motion")]
//! Local filters and advanced node-effect examples compile through the public author surface.
use valle_compiler::motion::compile_motion;

#[test]
fn filter_and_node_effect_examples_compile_from_the_public_surface() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for relative in [
        "crates/valle-compiler/tests/fixtures/motion/effects/local-filter-isolation.motion.tsx",
        "crates/valle-compiler/tests/fixtures/motion/effects/vector-turbulence-proxy.motion.tsx",
        "crates/valle-compiler/tests/fixtures/motion/effects/temporal-trail-proxy.motion.tsx",
        "crates/valle-compiler/tests/fixtures/motion/effects/components/layer-probe.tsx",
        "crates/valle-compiler/tests/fixtures/motion/effects/advanced-node-effects.motion.tsx",
    ] {
        let path = root.join(relative);
        let source = std::fs::read_to_string(&path).expect("read effect reference component");
        compile_motion(&source).unwrap_or_else(|errors| panic!("{}: {errors:#?}", path.display()));
    }
}
