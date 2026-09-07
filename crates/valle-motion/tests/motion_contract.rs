use std::collections::BTreeMap;

use valle_motion::{
    ARTIFACT_FORMAT_VERSION, ArtifactEnvelope, AssetControl, AssetKind, BuildFingerprint,
    BundledAsset, BundledFont, BundledSource, CameraControls, CapabilitySet, ChildRange, CompareOp,
    ContentDigest, ContextInput, ControlType, ControlsSchema, CueControl, CueKind, Expr, ExprId,
    Extrapolation, FrameControl, InterpolateStop, LockError, MOTION_BUNDLE_FORMAT_VERSION,
    MotionBundleManifest, MotionEasing, MotionValue, MotionViewport, NodeId, NodeKind, NumberValue,
    OptionalFrameControl, PropControl, ResourceRef, SceneArtifact, SceneNode, SemanticMeta,
    StyleBinding, StyleValue, TemplatePart, TextValue, TimingControls, canonical_bytes,
    motion_context_at, phase_windows,
};
use valle_timeline::FrameRate;

const ENVELOPE_SHA256: &str = "673aa103e492d0598ed50d6c8a65b693779f6c3f28a185390ed340d93e041365";

fn prop(control: ControlType, default: MotionValue) -> PropControl {
    PropControl {
        control,
        default: Some(default),
        required: false,
        label: None,
    }
}

fn envelope() -> ArtifactEnvelope {
    let capability_set = CapabilitySet::base();
    let controls = ControlsSchema {
        props: BTreeMap::from([
            (
                "intensity".into(),
                prop(
                    ControlType::Number {
                        min: Some(0.0),
                        max: Some(1.0),
                        step: Some(0.1),
                    },
                    MotionValue::Number(-0.0),
                ),
            ),
            (
                "label".into(),
                prop(ControlType::String, MotionValue::Str("Valle".into())),
            ),
        ]),
        data: BTreeMap::new(),
        timing: TimingControls {
            enter_frames: FrameControl {
                default: 10,
                min: 0,
                max: Some(120),
            },
            hold_cycle_frames: OptionalFrameControl {
                default: Some(12),
                min: 1,
                max: Some(120),
            },
            exit_frames: FrameControl {
                default: 10,
                min: 0,
                max: Some(120),
            },
        },
        cues: BTreeMap::from([(
            "narration".into(),
            CueControl {
                kind: CueKind::Span,
                required: false,
            },
        )]),
        assets: BTreeMap::from([(
            "hero".into(),
            AssetControl {
                kind: AssetKind::Image,
                required: true,
            },
        )]),
        camera: CameraControls {
            values: BTreeMap::from([(
                "zoom".into(),
                prop(
                    ControlType::Number {
                        min: Some(0.1),
                        max: Some(8.0),
                        step: Some(0.1),
                    },
                    MotionValue::Number(1.0),
                ),
            )]),
        },
    };

    let exprs = vec![
        Expr::Const {
            value: MotionValue::Number(2.0),
        },
        Expr::Const {
            value: MotionValue::Number(4.0),
        },
        Expr::Add {
            lhs: ExprId(0),
            rhs: ExprId(1),
        },
        Expr::Sub {
            lhs: ExprId(1),
            rhs: ExprId(0),
        },
        Expr::Mul {
            lhs: ExprId(2),
            rhs: ExprId(3),
        },
        Expr::Div {
            lhs: ExprId(4),
            rhs: ExprId(1),
        },
        Expr::Neg { input: ExprId(0) },
        Expr::Compare {
            op: CompareOp::Gt,
            lhs: ExprId(5),
            rhs: ExprId(6),
        },
        Expr::Prop {
            name: "label".into(),
        },
        Expr::Const {
            value: MotionValue::Str("fallback".into()),
        },
        Expr::Select {
            condition: ExprId(7),
            when_true: ExprId(8),
            when_false: ExprId(9),
        },
        Expr::Context {
            input: ContextInput::PhaseProgress {
                phase: valle_motion::PhaseKind::Enter,
            },
        },
        Expr::Interpolate {
            input: ExprId(11),
            stops: vec![
                InterpolateStop {
                    input: 0.0,
                    output: MotionValue::Number(0.0),
                },
                InterpolateStop {
                    input: 1.0,
                    output: MotionValue::Number(1.0),
                },
            ],
            easings: vec![MotionEasing::EaseOut],
            extrapolate_left: Extrapolation::Clamp,
            extrapolate_right: Extrapolation::Clamp,
        },
    ];

    let artifact = SceneArtifact {
        camera: None,
        format_version: ARTIFACT_FORMAT_VERSION,
        capability_set: capability_set.clone(),
        component: "ContractProbe".into(),
        controls,
        resource_refs: vec![ResourceRef {
            control: "hero".into(),
            content_hash: ContentDigest::of_bytes(b"hero.png"),
        }],
        exprs,
        nodes: vec![
            SceneNode {
                key: "root".into(),
                kind: NodeKind::Group,
                space: None,
                class_names: vec!["relative".into()],
                styles: vec![],
                visibility: None,
                children: ChildRange { start: 0, end: 2 },
                semantic: Some(SemanticMeta {
                    role: Some("scene".into()),
                    label: None,
                    tags: vec!["probe".into()],
                }),
            },
            SceneNode {
                key: "panel".into(),
                kind: NodeKind::Box,
                space: None,
                class_names: vec!["rounded-xl".into()],
                styles: vec![StyleBinding {
                    property: "opacity".into(),
                    value: StyleValue::Expr { expr: ExprId(12) },
                }],
                visibility: Some(ExprId(7)),
                children: ChildRange { start: 2, end: 2 },
                semantic: None,
            },
            SceneNode {
                key: "title".into(),
                kind: NodeKind::Text {
                    text: TextValue::Expr { expr: ExprId(10) },
                    per_unit: None,
                    path: None,
                },
                space: None,
                class_names: vec!["text-white".into()],
                styles: vec![],
                visibility: None,
                children: ChildRange { start: 2, end: 2 },
                semantic: None,
            },
        ],
        node_children: vec![NodeId(1), NodeId(2)],
        root: NodeId(0),
    };

    ArtifactEnvelope {
        build_fingerprint: BuildFingerprint {
            compiler_version: "valle-compiler@mj1.3".into(),
            math_engine: valle_draw::math::ENGINE_ID.into(),
            normalized_ast_digest: ContentDigest::of_bytes(b"normalized-ast"),
            prepared_data_digest: ContentDigest::of_bytes(b"prepared-data"),
            assets_digest: ContentDigest::of_bytes(b"assets"),
            canvas_size: MotionViewport::DEFAULT,
            // Keep the layout engine identity in sync; it distinguishes artifacts produced by different engines.
            layout_engine: "takumi-core@0.23.1".into(),
            formula_layout_engine: String::new(),
        },
        artifact,
    }
}

#[test]
fn complete_envelope_is_frozen_by_its_canonical_hash() {
    let envelope = envelope();
    envelope.validate().unwrap();

    let bytes = envelope.canonical_bytes().unwrap();
    let actual = String::from_utf8(bytes).unwrap();
    let digest = envelope.envelope_digest().unwrap();
    eprintln!("canonical={actual}");
    eprintln!("sha256={digest}");

    let wire: serde_json::Value = serde_json::from_str(&actual).unwrap();
    assert_eq!(wire["artifact"]["formatVersion"], ARTIFACT_FORMAT_VERSION);
    assert!(wire["artifact"].get("artifactVersion").is_none());
    assert!(wire["artifact"].get("sceneIrVersion").is_none());
    assert!(wire["artifact"].get("displayListVersion").is_none());
    assert_eq!(
        wire["artifact"]["capabilitySet"],
        serde_json::json!({"names": envelope.artifact.capability_set.names})
    );
    assert!(wire["buildFingerprint"].get("displayListVersion").is_none());
    assert_eq!(
        wire["buildFingerprint"]["canvasSize"],
        serde_json::json!({"width": 1920, "height": 1080})
    );
    assert_eq!(digest.as_hex(), ENVELOPE_SHA256);
}

#[test]
fn logical_canvas_is_part_of_the_artifact_identity() {
    let base = envelope();
    let base_digest = base.envelope_digest().unwrap();
    let mut changed = base;
    changed.build_fingerprint.canvas_size = MotionViewport::new(1280, 720);
    assert_ne!(changed.envelope_digest().unwrap(), base_digest);
}

/// Unit context inputs are valid only in per-unit evaluation; admission rejects other uses with a field path.
#[test]
fn unit_context_outside_a_per_unit_style_is_refused_at_admission() {
    let mut artifact = envelope().artifact;
    artifact.exprs.push(Expr::Context {
        input: ContextInput::UnitIndex,
    });
    let unit_expr = ExprId(artifact.exprs.len() as u32 - 1);
    artifact.nodes[1].styles.push(StyleBinding {
        property: "opacity".into(),
        value: StyleValue::Expr { expr: unit_expr },
    });

    let errors = artifact.validate().unwrap_err();
    assert!(
        errors.iter().any(|error| {
            error.path.ends_with("/styles/1/value") && error.message.contains("ctx.unit.*")
        }),
        "{errors:?}"
    );
}

#[test]
fn versions_and_capabilities_are_exact_admission_gates() {
    let mut changed = envelope();
    changed.artifact.format_version += 1;
    let error = changed.validate().unwrap_err();
    assert!(matches!(error, LockError::Artifact(_)));

    let mut changed = envelope();
    changed.artifact.capability_set = CapabilitySet::new(["box", "group", "text", "video"]);
    let error = changed.validate().unwrap_err();
    assert!(matches!(error, LockError::Artifact(_)));

    let mut changed = envelope();
    changed.build_fingerprint.math_engine = "platform-libm".into();
    assert!(matches!(changed.validate(), Err(LockError::Fingerprint(_))));
}

#[test]
fn tailwind_catalog_rejects_unknown_and_frame_impure_classes_in_artifacts() {
    let mut changed = envelope();
    changed.artifact.nodes[1].class_names = vec!["gird".into(), "sm:grid".into()];
    let errors = changed.artifact.validate().unwrap_err();
    assert!(errors.iter().any(|error| {
        error.path == "/nodes/1/classNames/0" && error.message.contains("tailwind")
    }));
    assert!(errors.iter().any(|error| {
        error.path == "/nodes/1/classNames/1" && error.message.contains("forbidden")
    }));
}

#[test]
fn controls_cover_six_namespaces_and_drive_hold_cycles() {
    let controls = &envelope().artifact.controls;
    assert!(controls.props.contains_key("label"));
    assert!(controls.data.is_empty());
    assert_eq!(controls.timing.hold_cycle_frames.default, Some(12));
    assert!(controls.cues.contains_key("narration"));
    assert!(controls.assets.contains_key("hero"));
    assert!(controls.camera.values.contains_key("zoom"));

    let layout = phase_windows(&controls.phase_spec(), 60);
    let ctx = motion_context_at(22, &layout, FrameRate::new(30, 1).unwrap()).unwrap();
    assert_eq!(ctx.hold.frame, 12);
    assert_eq!(ctx.hold.iteration, 1);
    assert_eq!(ctx.hold.cycle_frame, 0);
    assert_eq!(ctx.hold.cycle_progress.to_bits(), 0.0f64.to_bits());

    let overridden = controls
        .phase_spec_with_overrides(Some(20), Some(30))
        .unwrap();
    assert_eq!((overridden.enter_frames, overridden.exit_frames), (20, 30));
    let error = controls
        .phase_spec_with_overrides(Some(121), None)
        .expect_err("override must honor the declared max");
    assert_eq!(error.field, "enterFrames");
    assert_eq!(error.max, Some(120));
}

/// Video nodes are leaves and must reject child content during artifact admission.
#[test]
fn video_nodes_are_leaves_and_children_are_rejected() {
    let mut changed = envelope();
    changed.artifact.nodes[1].kind = NodeKind::Video {
        source: "asset://hero".into(),
        source_start: NumberValue::Static { value: 0.0 },
        speed: NumberValue::Static { value: 1.0 },
    };
    changed.artifact.nodes[1].children = ChildRange { start: 0, end: 1 };
    let errors = changed.artifact.validate().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| { error.path == "/nodes/1/children" && error.message.contains("leaf") }),
        "{errors:?}"
    );
}

#[test]
fn expression_types_and_tree_topology_fail_closed() {
    let mut changed = envelope();
    changed.artifact.exprs[2] = Expr::Add {
        lhs: ExprId(99),
        rhs: ExprId(1),
    };
    assert!(changed.artifact.validate().is_err());

    let mut changed = envelope();
    changed.artifact.nodes.push(SceneNode {
        key: "orphan".into(),
        kind: NodeKind::Group,
        space: None,
        class_names: vec![],
        styles: vec![],
        visibility: None,
        children: ChildRange { start: 2, end: 2 },
        semantic: None,
    });
    let errors = changed.artifact.validate().unwrap_err();
    assert!(errors.iter().any(|error| {
        error.path == "/nodes/3" && error.message.contains("reachable from root")
    }));

    let mut changed = envelope();
    changed.artifact.nodes[1].children = ChildRange { start: 0, end: 1 };
    let errors = changed.artifact.validate().unwrap_err();
    assert!(
        errors.iter().any(|error| {
            error.path == "/nodeChildren/0" && error.message.contains("exactly one")
        })
    );

    let mut changed = envelope();
    changed.artifact.resource_refs.clear();
    assert!(
        changed.artifact.validate().is_ok(),
        "component Artifacts may leave required controls for instance-time binding"
    );
}

#[test]
fn template_expressions_reject_non_scalar_substitutions_and_budget_overflow() {
    let mut changed = envelope();
    let rect = changed.artifact.exprs.len() as u32;
    changed.artifact.exprs.push(Expr::Const {
        value: MotionValue::Rect(valle_draw::Rect::new(0.0, 0.0, 10.0, 10.0)),
    });
    changed.artifact.exprs.push(Expr::Template {
        parts: vec![TemplatePart::Expr { expr: ExprId(rect) }],
    });
    let errors = changed.artifact.validate().unwrap_err();
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("template substitutions must be scalar CSS-token values")
    }));

    let mut changed = envelope();
    changed.artifact.exprs.push(Expr::Template {
        parts: vec![TemplatePart::Text {
            value: "x".repeat(valle_motion::MAX_TEMPLATE_STATIC_BYTES + 1),
        }],
    });
    let errors = changed.artifact.validate().unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("at most 8192 static UTF-8 bytes"))
    );
}

#[test]
fn canonical_and_artifact_validation_reject_every_non_finite_path() {
    assert!(canonical_bytes(&[f64::NAN]).is_err());

    let mut changed = envelope();
    if let Expr::Interpolate { easings, .. } = &mut changed.artifact.exprs[12] {
        easings[0] = MotionEasing::CubicBezier {
            p: [0.0, f64::INFINITY, 1.0, 1.0],
        };
    } else {
        panic!("fixture expression 12 must be interpolate");
    }
    assert!(changed.artifact.validate().is_err());
}

#[test]
fn motion_bundle_manifest_freezes_content_addressed_relative_paths() {
    let envelope_digest = envelope().envelope_digest().unwrap();
    let asset_hash = ContentDigest::of_bytes(b"image");
    let font_hash = ContentDigest::of_bytes(b"font");
    let manifest = MotionBundleManifest {
        format_version: MOTION_BUNDLE_FORMAT_VERSION,
        canvas_size: MotionViewport::DEFAULT,
        component: "ContractProbe".into(),
        envelope_digest,
        closure_digest: ContentDigest::of_bytes(b"closure"),
        entry: "main.motion.tsx".into(),
        source_path: "sources/main.motion.tsx".into(),
        modules: BTreeMap::from([(
            "main.motion.tsx".into(),
            BundledSource {
                content_hash: ContentDigest::of_bytes(b"source"),
                path: "sources/main.motion.tsx".into(),
            },
        )]),
        artifact_path: format!("artifacts/{}.json", envelope_digest.as_hex()),
        source_map_path: "source-map.json".into(),
        source_map_digest: ContentDigest::of_bytes(b"source-map"),
        data_path: None,
        data_source: None,
        data_digest: None,
        assets: BTreeMap::from([(
            "hero".into(),
            BundledAsset {
                kind: AssetKind::Image,
                content_hash: asset_hash,
                path: format!("resources/{}", asset_hash.as_hex()),
            },
        )]),
        fonts: vec![BundledFont {
            content_hash: font_hash,
            path: format!("fonts/{}", font_hash.as_hex()),
        }],
    };
    manifest.validate().unwrap();
    let manifest_json = serde_json::to_value(&manifest).unwrap();
    assert!(manifest_json.get("controlsPath").is_none());
    assert_eq!(manifest_json["envelopeDigest"], envelope_digest.to_wire());
    assert!(manifest_json.get("artifactHash").is_none());
    for pointer in [
        "/closureDigest",
        "/sourceMapDigest",
        "/modules/main.motion.tsx/contentHash",
        "/assets/hero/contentHash",
        "/fonts/0/contentHash",
    ] {
        let mut bare = manifest_json.clone();
        let wire = bare
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .expect("fixture digest wire");
        let hex = wire
            .strip_prefix("sha256:")
            .expect("fixture digest uses strict wire")
            .to_owned();
        *bare.pointer_mut(pointer).expect("fixture digest path") = serde_json::json!(hex);
        assert!(
            serde_json::from_value::<MotionBundleManifest>(bare).is_err(),
            "accepted bare digest at {pointer}"
        );
    }
    let mut bare_digest = manifest_json.clone();
    bare_digest["envelopeDigest"] = serde_json::json!(envelope_digest.as_hex());
    assert!(serde_json::from_value::<MotionBundleManifest>(bare_digest).is_err());
    let mut unsupported_name = manifest_json;
    let unsupported_digest = unsupported_name["envelopeDigest"].take();
    unsupported_name["artifactHash"] = unsupported_digest;
    unsupported_name
        .as_object_mut()
        .unwrap()
        .remove("envelopeDigest");
    assert!(serde_json::from_value::<MotionBundleManifest>(unsupported_name).is_err());

    let mut escaped = manifest.clone();
    escaped.assets.get_mut("hero").unwrap().path = "../image".into();
    assert!(matches!(escaped.validate(), Err(LockError::Manifest(_))));

    let mut wrong_artifact = manifest;
    wrong_artifact.artifact_path = "artifacts/latest.json".into();
    assert!(matches!(
        wrong_artifact.validate(),
        Err(LockError::Manifest(_))
    ));
}

#[test]
fn unknown_fields_are_rejected_at_top_level_and_nested_unions() {
    let value = serde_json::to_value(envelope()).unwrap();

    let mut changed = value.clone();
    changed["artifact"]["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ArtifactEnvelope>(changed).is_err());

    let mut changed = value.clone();
    changed["artifact"]["capabilitySet"]["hash"] = serde_json::json!(
        "sha256:0000000000000000000000000000000000000000000000000000000000000000"
    );
    assert!(serde_json::from_value::<ArtifactEnvelope>(changed).is_err());

    let mut changed = value;
    changed["artifact"]["exprs"][11]["input"]["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ArtifactEnvelope>(changed).is_err());
}
