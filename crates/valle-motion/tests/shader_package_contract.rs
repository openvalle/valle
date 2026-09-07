use std::collections::BTreeMap;
use std::str::FromStr;

use valle_motion::shader::{
    ALPHA_MODE, BudgetClass, COLOR_SPACE, ContentDigest, DIALECT_ID, DIALECT_VERSION,
    DiagnosticCode, InputSampling, OutputContract, SHADER_MANIFEST_VERSION, ShaderInput,
    ShaderManifest, ShaderPackage, ShaderRegistry, ShaderUniform, ShaderUri, UniformBindings,
    UniformType, UniformValue,
};

#[test]
fn registry_is_exact_uri_and_content_addressed() {
    let first = package();
    let uri = first.uri();
    let mut registry = ShaderRegistry::new();
    registry.register(first.clone()).unwrap();
    registry
        .register(first)
        .expect("identical registration is idempotent");
    assert_eq!(registry.len(), 1);
    assert_eq!(registry.resolve(&uri).unwrap().uri(), uri);

    let changed_source = b"half4 valle_main(float2 uv) { return sampleContent(uv) * 0.5; }";
    let changed_manifest = manifest().seal(changed_source).unwrap();
    let changed = ShaderPackage::admit(
        &serde_json::to_vec(&changed_manifest).unwrap(),
        changed_source,
    )
    .unwrap();
    let error = registry
        .register(changed)
        .expect_err("one URI cannot resolve to two package byte sets");
    assert!(error.message.contains("already pinned"));
}

const SOURCE: &[u8] = include_bytes!(
    "../../valle-compiler/tests/fixtures/motion/shader-packages/local-dissolve/shader.vsksl"
);
const MANIFEST: &[u8] = include_bytes!(
    "../../valle-compiler/tests/fixtures/motion/shader-packages/local-dissolve/manifest.json"
);

#[test]
fn package_uri_manifest_digests_and_abi_are_canonical() {
    let sealed = manifest().seal(SOURCE).unwrap();
    let sealed_wire = serde_json::to_value(&sealed).unwrap();
    assert_eq!(sealed_wire["sourceDigest"], sealed.source_digest.to_wire());
    assert_eq!(sealed_wire["abiDigest"], sealed.abi_digest.to_wire());
    for field in ["sourceDigest", "abiDigest"] {
        let mut bare = sealed_wire.clone();
        let digest = if field == "sourceDigest" {
            sealed.source_digest.as_hex()
        } else {
            sealed.abi_digest.as_hex()
        };
        bare[field] = serde_json::json!(digest);
        assert_eq!(
            ShaderManifest::parse(&serde_json::to_vec(&bare).unwrap())
                .unwrap_err()
                .code,
            DiagnosticCode::ManifestParse,
            "accepted bare {field}"
        );
    }
    assert_eq!(ShaderManifest::parse(MANIFEST).unwrap(), sealed);
    let pretty = serde_json::to_vec_pretty(&sealed).unwrap();
    let compact = serde_json::to_vec(&sealed).unwrap();
    let pretty_package = ShaderPackage::admit(&pretty, SOURCE).unwrap();
    let compact_package = ShaderPackage::admit(&compact, SOURCE).unwrap();

    assert_eq!(
        pretty_package.uri().to_string(),
        "shader://local-dissolve@1"
    );
    assert_eq!(
        ShaderUri::from_str("shader://local-dissolve@1").unwrap(),
        pretty_package.uri()
    );
    assert_eq!(
        serde_json::from_str::<ShaderUri>(r#""shader://local-dissolve@1""#).unwrap(),
        pretty_package.uri()
    );
    assert_eq!(
        serde_json::to_string(&pretty_package.uri()).unwrap(),
        r#""shader://local-dissolve@1""#
    );
    assert_eq!(
        pretty_package.canonical_manifest,
        compact_package.canonical_manifest
    );
    assert_eq!(pretty_package.content_hash, compact_package.content_hash);
    assert_eq!(pretty_package.abi.digest().unwrap(), sealed.abi_digest);
    assert_eq!(
        pretty_package
            .abi
            .children
            .iter()
            .map(|slot| slot.name.as_str())
            .collect::<Vec<_>>(),
        ["content", "noise"]
    );
    assert_eq!(
        pretty_package
            .abi
            .uniforms
            .iter()
            .map(|slot| (slot.name.as_str(), slot.scalar_offset, slot.scalar_width))
            .collect::<Vec<_>>(),
        [
            ("resolution", 0, 2),
            ("progress", 2, 1),
            ("edgeWidth", 3, 1),
            ("edgeColor", 4, 4),
        ]
    );
    assert_eq!(pretty_package.abi.scalar_count, 8);
    assert_eq!(pretty_package.dialect_report.samples_per_pixel, 2);
    assert!(
        pretty_package
            .generated_sksl
            .contains("uniform shader content;")
    );
    assert!(
        pretty_package
            .generated_sksl
            .contains("uniform shader noise;")
    );
    assert!(
        pretty_package.generated_sksl.contains("straight.rgb")
            && pretty_package.generated_sksl.contains("* a")
    );

    let lock = String::from_utf8(pretty_package.lock_record().unwrap()).unwrap();
    assert!(lock.contains("shader://local-dissolve@1"));
    assert!(lock.contains(&pretty_package.content_hash.to_wire()));
}

#[test]
fn typed_uniform_packing_uses_manifest_order_and_defaults() {
    let package = package();
    let bindings = UniformBindings(BTreeMap::from([
        ("progress".into(), UniformValue::Float(0.52)),
        (
            "edgeColor".into(),
            UniformValue::Color([113.0 / 255.0, 232.0 / 255.0, 1.0, 1.0]),
        ),
    ]));
    let bytes = package.pack_uniforms([128.0, 64.0], &bindings).unwrap();
    let values = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        values,
        [
            128.0,
            64.0,
            0.52,
            0.06,
            113.0 / 255.0,
            232.0 / 255.0,
            1.0,
            1.0,
        ]
    );
}

#[test]
fn package_admission_fails_closed_for_digest_abi_uri_and_uniform_drift() {
    let sealed = manifest().seal(SOURCE).unwrap();
    let bytes = serde_json::to_vec(&sealed).unwrap();
    let changed_source = [SOURCE, b"\n// changed"].concat();
    assert_eq!(
        ShaderPackage::admit(&bytes, &changed_source)
            .unwrap_err()
            .code,
        DiagnosticCode::SourceDigest
    );

    let mut bad_abi = sealed.clone();
    bad_abi.abi_digest = ContentDigest::of_bytes(b"wrong ABI");
    assert_eq!(
        ShaderPackage::admit(&serde_json::to_vec(&bad_abi).unwrap(), SOURCE)
            .unwrap_err()
            .code,
        DiagnosticCode::AbiDigest
    );
    for uri in [
        "https://example.com/a",
        "shader://local-dissolve",
        "shader://Local@1",
        "shader://local@0",
        "shader://local@01",
    ] {
        assert_eq!(
            ShaderUri::from_str(uri).unwrap_err().code,
            DiagnosticCode::InvalidUri,
            "{uri}"
        );
    }

    let package = ShaderPackage::admit(&bytes, SOURCE).unwrap();
    let missing = UniformBindings(BTreeMap::new());
    assert_eq!(
        package
            .pack_uniforms([128.0, 64.0], &missing)
            .unwrap_err()
            .code,
        DiagnosticCode::UniformMissing
    );
    let unknown = UniformBindings(BTreeMap::from([("clock".into(), UniformValue::Float(1.0))]));
    assert_eq!(
        package
            .pack_uniforms([128.0, 64.0], &unknown)
            .unwrap_err()
            .code,
        DiagnosticCode::UniformUnknown
    );
    let wrong = UniformBindings(BTreeMap::from([
        ("progress".into(), UniformValue::Bool(true)),
        (
            "edgeColor".into(),
            UniformValue::Color([0.0, 1.0, 1.0, 1.0]),
        ),
    ]));
    assert_eq!(
        package
            .pack_uniforms([128.0, 64.0], &wrong)
            .unwrap_err()
            .code,
        DiagnosticCode::UniformType
    );
}

#[test]
fn controlled_dialect_rejects_loops_direct_textures_dynamic_index_and_unknown_calls() {
    for (needle, source) in [
        (
            "forbidden",
            "half4 valle_main(float2 uv) { for (;;) { } return half4(1.0); }",
        ),
        (
            "opaque",
            "half4 valle_main(float2 uv) { return content.eval(uv); }",
        ),
        (
            "outside",
            "half4 valle_main(float2 uv) { return random(uv); }",
        ),
        (
            "character",
            "half4 valle_main(float2 uv) { float x[2]; return half4(1.0); }",
        ),
    ] {
        let error = manifest().seal(source.as_bytes()).unwrap_err();
        assert_eq!(error.code, DiagnosticCode::DialectViolation);
        assert!(error.message.contains(needle), "{error}");
    }
}

#[test]
fn manifest_shape_and_layer_area_are_bounded() {
    let mut duplicate = manifest();
    duplicate.uniforms[0].name = "noise".into();
    assert_eq!(
        duplicate.validate_shape().unwrap_err().code,
        DiagnosticCode::DuplicateName
    );

    for reserved in ["for", "main", "sk_FragCoord", "valle_main", "sample_noise"] {
        let mut invalid_name = manifest();
        invalid_name.uniforms[0].name = reserved.into();
        assert_eq!(
            invalid_name.validate_shape().unwrap_err().code,
            DiagnosticCode::Identifier,
            "{reserved}"
        );
    }

    let mut invalid_default = manifest();
    invalid_default.uniforms[0].default = Some(UniformValue::Float(0.5));
    assert_eq!(
        invalid_default.validate_shape().unwrap_err().code,
        DiagnosticCode::UniformDefault
    );

    let package = package();
    assert_eq!(
        package.validate_layer_pixels(1920, 1080).unwrap(),
        2_073_600
    );
    assert_eq!(
        package.validate_layer_pixels(1921, 1080).unwrap_err().code,
        DiagnosticCode::LayerBounds
    );
    assert_eq!(
        package.validate_layer_pixels(0, 720).unwrap_err().code,
        DiagnosticCode::LayerBounds
    );
}

#[test]
fn strict_manifest_and_static_budgets_reject_expansion() {
    let sealed = manifest().seal(SOURCE).unwrap();
    let mut value = serde_json::to_value(&sealed).unwrap();
    value["hostCallback"] = serde_json::json!("https://example.com");
    assert_eq!(
        ShaderManifest::parse(&serde_json::to_vec(&value).unwrap())
            .unwrap_err()
            .code,
        DiagnosticCode::ManifestParse
    );

    let calls = core::iter::repeat_n("sampleContent(uv)", 9)
        .collect::<Vec<_>>()
        .join(" + ");
    let too_many_samples = format!("half4 valle_main(float2 uv) {{ return {calls}; }}");
    assert_eq!(
        manifest()
            .seal(too_many_samples.as_bytes())
            .unwrap_err()
            .code,
        DiagnosticCode::BudgetExceeded
    );

    let oversized = format!(
        "half4 valle_main(float2 uv) {{ return half4(1.0); }}//{}",
        "x".repeat(valle_motion::shader::MAX_SOURCE_BYTES)
    );
    assert_eq!(
        manifest().seal(oversized.as_bytes()).unwrap_err().code,
        DiagnosticCode::BudgetExceeded
    );
}

fn package() -> ShaderPackage {
    let sealed = manifest().seal(SOURCE).unwrap();
    ShaderPackage::admit(&serde_json::to_vec(&sealed).unwrap(), SOURCE).unwrap()
}

fn manifest() -> ShaderManifest {
    ShaderManifest {
        manifest_version: SHADER_MANIFEST_VERSION,
        name: "local-dissolve".into(),
        version: 1,
        dialect: DIALECT_ID.into(),
        dialect_version: DIALECT_VERSION,
        entry: "shader.vsksl".into(),
        inputs: vec![ShaderInput {
            name: "noise".into(),
            required: true,
            sampling: InputSampling::Nearest,
        }],
        uniforms: vec![
            ShaderUniform {
                name: "progress".into(),
                uniform_type: UniformType::Float,
                required: true,
                default: None,
                min: Some(0.0),
                max: Some(1.0),
            },
            ShaderUniform {
                name: "edgeWidth".into(),
                uniform_type: UniformType::Float,
                required: false,
                default: Some(UniformValue::Float(0.06)),
                min: Some(0.001),
                max: Some(0.25),
            },
            ShaderUniform {
                name: "edgeColor".into(),
                uniform_type: UniformType::Color,
                required: true,
                default: None,
                min: None,
                max: None,
            },
        ],
        output: OutputContract {
            color_space: COLOR_SPACE.into(),
            alpha_mode: ALPHA_MODE.into(),
            allow_transparent: true,
        },
        budget: BudgetClass::Local,
        source_digest: ContentDigest::of_bytes(b"unsealed"),
        abi_digest: ContentDigest::of_bytes(b"unsealed"),
    }
}
