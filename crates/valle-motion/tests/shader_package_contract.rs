use std::collections::BTreeMap;
use std::str::FromStr;

use valle_motion::shader::{
    ALPHA_MODE, BudgetClass, COLOR_SPACE, DiagnosticCode, InputSampling, OutputContract,
    ShaderInput, ShaderManifest, ShaderPackage, ShaderRegistry, ShaderUniform, ShaderUri,
    UniformBindings, UniformType, UniformValue,
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
    let changed = ShaderPackage::compile(manifest(), changed_source).unwrap();
    let changed_uri = changed.uri();
    registry.register_asset("effect", changed.clone()).unwrap();
    registry.register_asset("duplicate", changed).unwrap();
    assert_eq!(registry.len(), 2);
    assert_ne!(changed_uri, uri);
    assert_eq!(registry.asset("effect").unwrap().uri(), changed_uri);
    assert_eq!(registry.asset("duplicate").unwrap().uri(), changed_uri);
    assert!(registry.register_asset("effect", package()).is_err());
}

const SOURCE: &[u8] = include_bytes!(
    "../../valle-compiler/tests/fixtures/motion/shader-packages/local-dissolve/shader.vsksl"
);
const MANIFEST: &[u8] = include_bytes!(
    "../../valle-compiler/tests/fixtures/motion/shader-packages/local-dissolve/manifest.json"
);

#[test]
fn package_uri_manifest_digests_and_abi_are_canonical() {
    let descriptor = manifest();
    let wire = serde_json::to_value(&descriptor).unwrap();
    assert_eq!(wire.as_object().unwrap().len(), 6);
    assert_eq!(ShaderManifest::parse(MANIFEST).unwrap(), descriptor);
    let pretty = serde_json::to_vec_pretty(&descriptor).unwrap();
    let compact = serde_json::to_vec(&descriptor).unwrap();
    let pretty_package = ShaderPackage::admit(&pretty, SOURCE).unwrap();
    let compact_package = ShaderPackage::admit(&compact, SOURCE).unwrap();

    let uri = format!("shader://{}", pretty_package.content_hash.as_hex());
    assert_eq!(pretty_package.uri().to_string(), uri);
    assert_eq!(ShaderUri::from_str(&uri).unwrap(), pretty_package.uri());
    let uri_json = serde_json::to_string(&pretty_package.uri()).unwrap();
    assert_eq!(
        serde_json::from_str::<ShaderUri>(&uri_json).unwrap(),
        pretty_package.uri()
    );
    let frozen = pretty_package.frozen_bytes().unwrap();
    assert_eq!(
        valle_motion::ContentDigest::of_bytes(&frozen),
        pretty_package.content_hash
    );
    let restored = ShaderPackage::from_frozen(&frozen).unwrap();
    assert_eq!(restored.content_hash, pretty_package.content_hash);
    assert_eq!(restored.generated_sksl, pretty_package.generated_sksl);
    assert_eq!(
        pretty_package.canonical_manifest,
        compact_package.canonical_manifest
    );
    assert_eq!(pretty_package.content_hash, compact_package.content_hash);
    assert_eq!(
        pretty_package.abi.digest().unwrap(),
        pretty_package.abi_hash
    );
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
            ("valle_size_noise", 2, 2),
            ("progress", 4, 1),
            ("edgeWidth", 5, 1),
            ("edgeColor", 6, 4),
        ]
    );
    assert_eq!(pretty_package.abi.scalar_count, 10);
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
    assert!(lock.contains(&uri));
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
    let bytes = package
        .pack_uniforms([128.0, 64.0], &sizes(), &bindings)
        .unwrap();
    let values = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(&values[..6], &[128.0, 64.0, 8.0, 8.0, 0.52, 0.06]);
    for (actual, expected) in values[6..].iter().zip([0.1651322, 0.8069523, 1.0, 1.0]) {
        assert!((actual - expected).abs() < 1e-6);
    }
    let bindings = UniformBindings(BTreeMap::from([
        ("progress".into(), UniformValue::Float(0.0)),
        (
            "edgeColor".into(),
            UniformValue::Color([1.0, 0.0, 0.0, 0.0]),
        ),
    ]));
    let packed = package
        .pack_uniforms([1.0, 1.0], &sizes(), &bindings)
        .unwrap();
    let channels: Vec<f32> = packed[24..]
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect();
    assert_eq!(
        channels,
        [1.0, 0.0, 0.0, 0.0],
        "transparent color uniforms must retain RGB"
    );
}

#[test]
fn package_identity_changes_with_source_and_contract_and_uniforms_are_checked() {
    let original = package();
    let changed_source = [SOURCE, b"\n// changed"].concat();
    let changed = ShaderPackage::compile(manifest(), &changed_source).unwrap();
    assert_ne!(changed.source_hash, original.source_hash);
    assert_ne!(changed.content_hash, original.content_hash);
    assert_eq!(changed.abi_hash, original.abi_hash);
    let mut changed_contract = manifest();
    changed_contract.inputs[0].sampling = InputSampling::Linear;
    let changed = ShaderPackage::compile(changed_contract, SOURCE).unwrap();
    assert_ne!(changed.abi_hash, original.abi_hash);
    assert_ne!(changed.content_hash, original.content_hash);
    for uri in [
        "https://example.com/a",
        "shader://",
        "shader://Local",
        "shader://../local",
    ] {
        assert_eq!(
            ShaderUri::from_str(uri).unwrap_err().code,
            DiagnosticCode::InvalidUri
        );
    }
    let package = original;
    let missing = UniformBindings(BTreeMap::new());
    assert_eq!(
        package
            .pack_uniforms([128.0, 64.0], &sizes(), &missing)
            .unwrap_err()
            .code,
        DiagnosticCode::UniformMissing
    );
    let unknown = UniformBindings(BTreeMap::from([("clock".into(), UniformValue::Float(1.0))]));
    assert_eq!(
        package
            .pack_uniforms([128.0, 64.0], &sizes(), &unknown)
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
            .pack_uniforms([128.0, 64.0], &sizes(), &wrong)
            .unwrap_err()
            .code,
        DiagnosticCode::UniformType
    );
}

#[test]
fn controlled_dialect_rejects_loops_direct_textures_dynamic_index_and_unknown_calls() {
    for (needle, source) in [
        (
            "expected `int`",
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
            "expected `=`",
            "half4 valle_main(float2 uv) { float x[2]; return half4(1.0); }",
        ),
    ] {
        let error = ShaderPackage::compile(manifest(), source.as_bytes()).unwrap_err();
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
    let mut value = serde_json::to_value(manifest()).unwrap();
    value["hostCallback"] = serde_json::json!("https://example.com");
    assert_eq!(
        ShaderManifest::parse(&serde_json::to_vec(&value).unwrap())
            .unwrap_err()
            .code,
        DiagnosticCode::ManifestParse
    );

    let calls = core::iter::repeat_n(
        "sampleContent(uv)",
        valle_motion::shader::MAX_SAMPLES_PER_PIXEL + 1,
    )
    .collect::<Vec<_>>()
    .join(" + ");
    let too_many_samples = format!("half4 valle_main(float2 uv) {{ return {calls}; }}");
    assert_eq!(
        ShaderPackage::compile(manifest(), too_many_samples.as_bytes())
            .unwrap_err()
            .code,
        DiagnosticCode::BudgetExceeded
    );

    let oversized = format!(
        "half4 valle_main(float2 uv) {{ return half4(1.0); }}//{}",
        "x".repeat(valle_motion::shader::MAX_SOURCE_BYTES)
    );
    assert_eq!(
        ShaderPackage::compile(manifest(), oversized.as_bytes())
            .unwrap_err()
            .code,
        DiagnosticCode::BudgetExceeded
    );
}

#[test]
fn helpers_are_acyclic_and_transitive_sampling_stays_bounded() {
    let manifest = manifest();
    let source = b"half4 tap(float2 p) { return sampleContent(p); } half4 twice(float2 p) { return tap(p) + tap(p); } half4 valle_main(float2 uv) { return twice(uv) + twice(uv); }";
    let package = ShaderPackage::compile(manifest.clone(), source).unwrap();
    assert_eq!(package.dialect_report.samples_per_pixel, 4);
    for source in [
        "float recur(float p) { return recur(p); } half4 valle_main(float2 uv) { return half4(recur(uv.x)); }",
        "float first(float p) { return second(p); } float second(float p) { return first(p); } half4 valle_main(float2 uv) { return half4(first(uv.x)); }",
        "float hidden(float p) { while (p > 0.0) p -= 1.0; return p; } half4 valle_main(float2 uv) { return half4(hidden(uv.x)); }",
        "float resolution(float p) { return p; } half4 valle_main(float2 uv) { return half4(resolution(uv.x)); }",
        "float shared = 0.0; half4 valle_main(float2 uv) { shared += uv.x; return half4(shared); }",
        "float helper(float p) { return content.eval(float2(p)).x; } half4 valle_main(float2 uv) { return half4(helper(uv.x)); }",
    ] {
        assert_eq!(
            ShaderPackage::compile(manifest.clone(), source.as_bytes())
                .unwrap_err()
                .code,
            DiagnosticCode::DialectViolation,
            "{source}"
        );
    }
    let source = b"half4 tap(float2 p) { return sampleContent(p); } half4 valle_main(float2 uv) { float4 sum = float4(0.0); for (int i = 0; i < 65; i++) { sum += tap(uv); } return sum; }";
    assert_eq!(
        ShaderPackage::compile(manifest.clone(), source)
            .unwrap_err()
            .code,
        DiagnosticCode::BudgetExceeded
    );
    let mut expansion = "float a(float p) { return p; }".to_owned();
    for (name, previous) in [
        ('b', 'a'),
        ('c', 'b'),
        ('d', 'c'),
        ('e', 'd'),
        ('f', 'e'),
        ('g', 'f'),
        ('h', 'g'),
    ] {
        expansion.push_str(&format!(
            "float {name}(float p) {{ return {previous}(p) + {previous}(p); }}"
        ));
    }
    expansion.push_str("half4 valle_main(float2 uv) { return half4(h(uv.x) + h(uv.y)); }");
    assert_eq!(
        ShaderPackage::compile(manifest, expansion.as_bytes())
            .unwrap_err()
            .code,
        DiagnosticCode::BudgetExceeded
    );
}

fn package() -> ShaderPackage {
    ShaderPackage::compile(manifest(), SOURCE).unwrap()
}

fn manifest() -> ShaderManifest {
    ShaderManifest {
        name: "local-dissolve".into(),
        entry: "shader.vsksl".into(),
        inputs: vec![ShaderInput {
            kind: valle_motion::shader::InputKind::Color,
            name: "noise".into(),
            required: true,
            sampling: InputSampling::Nearest,
            wrap: valle_motion::shader::InputWrap::Clamp,
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
            padding: [0; 4],
            color_space: COLOR_SPACE.into(),
            alpha_mode: ALPHA_MODE.into(),
            allow_transparent: true,
        },
        budget: BudgetClass::Local,
    }
}

#[test]
fn typed_blocks_forward_calls_and_constant_loops_have_bounded_cost() {
    let source = br#"
        float4 valle_main(float2 uv) {
            const int radius = 2;
            float4 sum = float4(0.0);
            for (int y = -radius; y <= radius; ++y) {
                for (int x = -radius; x <= radius; x += 1) {
                    sum += tap(uv + float2(x, y) / resolution);
                }
            }
            return sum / 25.0;
        }
        float4 tap(float2 p) {
            if (p.x > 0.0) {
                float2x2 rotate = float2x2(0.0, -1.0, 1.0, 0.0);
                return sampleContent(rotate * p);
            } else {
                return sample_noise(p) + sampleContent(p);
            }
        }
    "#;
    let package = ShaderPackage::compile(manifest(), source).unwrap();
    assert_eq!(package.dialect_report.samples_per_pixel, 50);
    assert_eq!(package.dialect_report.calls_per_pixel, 25);
    assert!(
        package.generated_sksl.find("float4 tap(").unwrap()
            < package.generated_sksl.find("float4 valle_main(").unwrap()
    );
    assert!(
        package
            .generated_sksl
            .contains("valle_div(float4(sum), float4(25.0))")
    );
    assert!(package.dialect_report.operations_per_pixel > 25);
}

#[test]
fn shader_type_scope_return_and_loop_errors_have_source_locations() {
    for (body, message) in [
        ("float x = uv; return float4(x);", "expected float"),
        (
            "if (uv.x) return float4(1.0); else return float4(0.0);",
            "expected bool",
        ),
        (
            "if (uv.x > 0.0) return float4(1.0);",
            "every execution path",
        ),
        ("{ float x = 1.0; } return float4(x);", "unknown value"),
        (
            "const float x = 1.0; x = 2.0; return float4(x);",
            "cannot write",
        ),
        ("progress = 0.0; return float4(1.0);", "cannot write"),
        (
            "float2 x = uv; x.xx = uv; return float4(x, 0.0, 1.0);",
            "cannot repeat",
        ),
        ("return float4(uv.z);", "swizzle components"),
        (
            "return float4(uv[int(progress)]);",
            "compile-time constants",
        ),
        (
            "for (int i = 0; i < int(progress); i++) {} return float4(0.0);",
            "compile-time constants",
        ),
        (
            "for (int i = 0; i < 3; i++) { i = 1; } return float4(0.0);",
            "cannot write",
        ),
        (
            "for (int i = 0; i < 3; i++) { int i = 0; } return float4(0.0);",
            "cannot be shadowed",
        ),
        (
            "for (int i = 0; i < 3; i -= 1) {} return float4(0.0);",
            "progress toward",
        ),
    ] {
        let source = format!("float4 valle_main(float2 uv) {{\n{body}\n}}");
        let error = ShaderPackage::compile(manifest(), source.as_bytes()).unwrap_err();
        assert!(error.path.starts_with("source:"), "{error}");
        assert!(error.message.contains(message), "{body}: {error}");
    }
    let source = b"float4 valle_main(float2 uv) { float x = 0.0; for (int i = 0; i < 256; i++) { for (int j = 0; j < 256; j++) { x += sin(uv.x); } } return float4(x); }";
    assert_eq!(
        ShaderPackage::compile(manifest(), source).unwrap_err().code,
        DiagnosticCode::BudgetExceeded
    );
}

#[test]
fn vector_and_column_major_matrix_uniforms_preserve_their_numeric_values() {
    let uniforms = vec![
        ("offset", UniformValue::Float3([0.1, -0.2, 2.0])),
        ("weights", UniformValue::Float4([1.0, 2.0, 3.0, 4.0])),
        ("basis", UniformValue::Float2x2([0.0, 1.0, -1.0, 0.0])),
        (
            "transform",
            UniformValue::Float3x3([1.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0]),
        ),
        (
            "projection",
            UniformValue::Float4x4([
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ]),
        ),
    ];
    let mut descriptor = manifest();
    descriptor.inputs.clear();
    descriptor.uniforms = uniforms
        .iter()
        .map(|(name, value)| ShaderUniform {
            name: (*name).into(),
            uniform_type: value.uniform_type(),
            required: false,
            default: Some(value.clone()),
            min: Some(-4.0),
            max: Some(4.0),
        })
        .collect();
    let source = b"float4 valle_main(float2 uv) { float3 p = transform * float3(basis * uv, 1.0) + offset; return projection * float4(p, weights.w); }";
    let package = ShaderPackage::compile(descriptor, source).unwrap();
    let bytes = package
        .pack_uniforms([8.0, 4.0], &BTreeMap::new(), &UniformBindings::empty())
        .unwrap();
    let scalars: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect();
    assert_eq!(
        &scalars[..13],
        &[
            8.0, 4.0, 0.1, -0.2, 2.0, 1.0, 2.0, 3.0, 4.0, 0.0, 1.0, -1.0, 0.0
        ]
    );
    assert_eq!(scalars.len(), 38);
    let bindings = UniformBindings(BTreeMap::from([(
        "offset".into(),
        UniformValue::Float3([0.0, f32::INFINITY, 0.0]),
    )]));
    assert_eq!(
        package
            .pack_uniforms([8.0, 4.0], &BTreeMap::new(), &bindings)
            .unwrap_err()
            .code,
        DiagnosticCode::UniformRange
    );
}

fn sizes() -> BTreeMap<String, [u32; 2]> {
    BTreeMap::from([("noise".into(), [8, 8])])
}

#[test]
fn padding_is_static_and_counts_toward_layer_allocation() {
    let mut manifest = manifest();
    manifest.output.padding = [8, 4, 12, 6];
    let package = ShaderPackage::compile(
        manifest.clone(),
        b"float4 valle_main(float2 uv) { return sampleContent(uv); }",
    )
    .unwrap();
    assert_eq!(package.validate_layer_pixels(16, 16).unwrap(), 36 * 26);
    assert!(package.validate_layer_pixels(1920, 1080).is_err());
    manifest.output.padding = [u32::MAX; 4];
    assert!(
        ShaderPackage::compile(
            manifest,
            b"float4 valle_main(float2 uv) { return float4(0.0); }"
        )
        .is_err()
    );
}

#[test]
fn data_textures_preserve_raw_channels_and_admit_actual_storage_and_samples() {
    use std::io::Cursor;
    use valle_motion::shader::{
        InputKind, InputWrap, data_texture_storage_bytes, decode_data_texture,
    };
    let values = [32768_u16, 16385, 8193, 0, 32769, 65535, 0, 65535];
    let image = image::ImageBuffer::<image::Rgba<u16>, _>::from_raw(2, 1, values.to_vec()).unwrap();
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba16(image)
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    let packed = decode_data_texture(encoded.get_ref(), 2, 1).unwrap();
    for x in 0..2 {
        for channel in 0..4 {
            let offset = channel * 4 + x;
            assert_eq!(
                u16::from_be_bytes([packed[offset], packed[offset + 2]]),
                values[x * 4 + channel]
            );
        }
    }
    assert_eq!(packed.len(), 16);
    assert!(decode_data_texture(encoded.get_ref(), 1, 1).is_err());
    assert!(data_texture_storage_bytes(u32::MAX, u32::MAX).is_err());
    assert!(data_texture_storage_bytes(4096, 4096).is_err());
    let mut descriptor = manifest();
    descriptor.inputs[0].kind = InputKind::Data;
    descriptor.inputs[0].wrap = InputWrap::Mirror;
    descriptor.inputs[0].sampling = InputSampling::Linear;
    let source = b"float4 valle_main(float2 uv) { return sample_noise(uv); }";
    let data = ShaderPackage::compile(descriptor.clone(), source).unwrap();
    assert_eq!(data.dialect_report.samples_per_pixel, 32);
    descriptor.inputs[0].kind = InputKind::Color;
    let color = ShaderPackage::compile(descriptor, source).unwrap();
    assert_eq!(color.dialect_report.samples_per_pixel, 4);
    assert_ne!(color.abi_hash, data.abi_hash);
    assert_ne!(color.content_hash, data.content_hash);
}
