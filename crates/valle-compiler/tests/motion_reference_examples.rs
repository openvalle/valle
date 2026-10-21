#![cfg(feature = "motion")]
//! Reference examples cover typography, sequences, modifiers and procedural data scenes.
use valle_compiler::motion::{
    MeasureEnv, PrepareDataBinding, compile_motion, compile_motion_with_env,
    compile_motion_with_full_env_and_data,
};
use valle_motion::{
    ContentDigest, FONT_ASSET_CAPABILITY, MOTION_MATH_CAPABILITY, NUMBER_FORMAT_CAPABILITY,
    RICH_TEXT_CAPABILITY, ResourceRef,
};

const FONT: &[u8] = include_bytes!("../../../assets/fonts/noto/NotoSansCJKsc-Regular.otf");
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

/// The guide's fenced blocks: the line of the opening fence, the language and the body.
fn fenced_blocks(guide: &str) -> Vec<(usize, &str, &str)> {
    let mut blocks = Vec::new();
    let mut open = None;
    let mut offset = 0;
    for (index, line) in guide.split_inclusive('\n').enumerate() {
        if let Some(fence) = line.trim_end().strip_prefix("```") {
            match open.take() {
                None => {
                    let language = fence.split_whitespace().next().unwrap_or("");
                    open = Some((index + 1, language, offset + line.len()));
                }
                Some((at, language, start)) => blocks.push((at, language, &guide[start..offset])),
            }
        }
        offset += line.len();
    }
    blocks
}

/// Every complete example in the Motion guide, a `tsx` block with a default export, compiles as
/// written. One that reads `useData()` compiles with the `json` block that follows it; the asset
/// controls the guide's examples declare are bound as the guide tells: `noise` to an image and
/// `effect` to the local-dissolve descriptor. A caption presenter compiles against its host's
/// caption layout, which motion_caption_presenter.rs supplies for the guide's example.
#[test]
fn every_complete_guide_example_compiles() {
    let guide = include_str!("../../../docs/motion.md");
    let blocks = fenced_blocks(guide);
    let measure = MeasureEnv::new_with_aliases(&[], &[], (1280, 720)).expect("measure env");
    let effect = valle_motion::shader::ShaderPackage::admit(
        include_bytes!("fixtures/motion/shader-packages/local-dissolve/manifest.json"),
        include_bytes!("fixtures/motion/shader-packages/local-dissolve/shader.vsksl"),
    )
    .expect("the guide's descriptor admits");
    let mut examples = 0;
    let mut failures = Vec::new();
    for (index, (line, language, body)) in blocks.iter().enumerate() {
        if *language != "tsx" || !body.contains("export default") {
            continue;
        }
        examples += 1;
        if body.contains("captionPresenter(") {
            continue;
        }
        let mut resources = Vec::new();
        let mut shaders = valle_motion::shader::ShaderRegistry::new();
        if body.contains("noise: asset(") {
            resources.push(ResourceRef {
                control: "noise".into(),
                content_hash: ContentDigest::of_bytes(b"noise.png"),
            });
        }
        if body.contains("effect: asset(") {
            shaders
                .register_asset("effect", effect.clone())
                .expect("bind the descriptor");
            resources.push(ResourceRef {
                control: "effect".into(),
                content_hash: effect.content_hash,
            });
        }
        let data = body.contains("useData()").then(|| {
            let (_, _, json) = blocks[index + 1..]
                .iter()
                .find(|(_, language, _)| *language == "json")
                .expect("a data example shows its data");
            PrepareDataBinding {
                source: "guide.json".into(),
                value: serde_json::from_str(json).expect("the guide's data parses"),
            }
        });
        if let Err(diagnostics) = compile_motion_with_full_env_and_data(
            body,
            &resources,
            Some(&measure),
            Some(&shaders),
            data.as_ref(),
        ) {
            let messages = diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>();
            failures.push(format!("docs/motion.md:{line}: {}", messages.join("; ")));
        }
    }
    assert!(examples >= 28, "the guide has {examples} complete examples");
    assert!(
        failures.is_empty(),
        "guide examples that do not compile:\n{}",
        failures.join("\n")
    );
}

/// The Motion examples of the agent skill and the READMEs compile as written.
#[test]
fn the_skill_and_readme_examples_compile() {
    let measure = MeasureEnv::new_with_aliases(&[], &[], (1920, 1080)).expect("measure env");
    for (file, text) in [
        (
            "skills/valle/SKILL.md",
            include_str!("../../../skills/valle/SKILL.md"),
        ),
        ("README.md", include_str!("../../../README.md")),
        ("README.zh-CN.md", include_str!("../../../README.zh-CN.md")),
    ] {
        let examples = fenced_blocks(text)
            .into_iter()
            .filter(|(_, language, _)| *language == "tsx")
            .collect::<Vec<_>>();
        assert_eq!(examples.len(), 1, "{file} shows one Motion example");
        let (line, _, body) = examples[0];
        if let Err(diagnostics) = compile_motion_with_env(body, &[], Some(&measure)) {
            let messages = diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>();
            panic!("{file}:{line}: {}", messages.join("; "));
        }
    }
}
