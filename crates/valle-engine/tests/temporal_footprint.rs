use std::sync::Arc;

use serde_json::{Value, json};
use valle_compiler::motion::compile_motion;
use valle_engine::{
    fixed_package::{
        COMMON_PROFILE_KEY, FixedPackageOpenError, canonical_fixed_execution_profile,
        canonical_fixed_package_manifest, canonical_verified_binding_bundle, fixed_package_files,
        open_verified_fixed_package,
    },
    product::EngineRender,
    render::{
        Capabilities, CompiledTransitionKernel, CompiledVisualItem, EXTENSION_COLOR_GAIN_ABI,
        EXTENSION_CROSS_FADE_ABI, EngineOpenDiagnosticCode, EngineOpenReport,
        ExtensionKernelCapability, ResourceBinding, ResourceBindings, VerifiedHandleId,
        VerifiedResourceFacts, VisualFootprint, engine_owned_kernel_implementation_digest,
    },
};
use valle_motion::SceneArtifact;
use valle_timeline::internal::{
    ContentDigest, ResourceManifest, canonical_bytes, decode_canonical, decode_resource_manifest,
    wire::resource::ResourceEntryWire,
};

const FILTER_KIND: &str = "example.visual/history@1";
const TRANSITION_KIND: &str = "example.transition/history@1";

fn constant(value: Value) -> Value {
    json!({"type": "constant", "value": value})
}

fn layer(filter_id: &str) -> Value {
    json!({
        "transform": {
            "position": constant(json!([0.5, 0.5])),
            "scale": constant(json!([1.0, 1.0])),
            "rotation": constant(json!(0.0)),
            "anchor": [0.5, 0.5]
        },
        "opacity": constant(json!(1.0)),
        "mask": null,
        "filters": [{
            "id": filter_id,
            "type": FILTER_KIND,
            "parameters": {"gain": 1.0}
        }],
        "blend": "normal"
    })
}

fn motion_clip(id: &str, filter_id: &str) -> Value {
    json!({
        "type": "clip",
        "id": id,
        "duration": "1/1",
        "layer": layer(filter_id),
        "source": {
            "type": "motion",
            "component": "component:glass",
            "sourceStart": "0/1",
            "sourceDuration": "1/1",
            "rate": "1/1",
            "endBehavior": "hold",
            "props": {},
            "cues": {},
            "resources": {},
            "phases": {"enterDuration": null, "exitDuration": null}
        }
    })
}

fn timeline() -> valle_timeline::internal::CanonicalTimeline {
    let value = json!({
        "document": {
            "canvas": {
                "width": 1920,
                "height": 1080,
                "fps": "4/1",
                "sampleRate": 48000,
                "channelLayout": "stereo",
                "colorSpace": "srgb",
                "duration": "2/1"
            },
            "background": {"color": "#000000ff"},
            "visual": {
                "tracks": [{
                    "id": "track:main",
                    "items": [
                        motion_clip("clip:left", "filter:left-history"),
                        {
                            "type": "transition",
                            "id": "transition:history",
                            "duration": "3/4",
                            "kernel": {
                                "type": TRANSITION_KIND,
                                "parameters": {}
                            }
                        },
                        motion_clip("clip:right", "filter:right-history")
                    ]
                }]
            },
            "audio": {"tracks": []},
            "adjustments": [],
            "captions": {"tracks": []},
            "camera": null,
            "metadata": {}
        }
    });
    decode_canonical(&serde_json::to_string(&value).unwrap()).unwrap()
}

fn glass_artifact() -> Arc<SceneArtifact> {
    let artifact = compile_motion(
        r#"
export const component = "glass-history";
export default function Scene() {
  return <Glass surfaceId="history-lens" shape={{ kind: "circle" }} />;
}
"#,
    )
    .expect("the production Motion compiler must compile <Glass>")
    .artifact;
    assert!(
        artifact.reads_destination(),
        "a real <Glass> artifact must declare destination reads"
    );
    Arc::new(artifact)
}

fn content_digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest::of_bytes(bytes)
}

fn manifest(artifact: &SceneArtifact) -> ResourceManifest {
    let artifact_digest = content_digest(&valle_motion::canonical_bytes(artifact).unwrap());
    decode_resource_manifest(
        &serde_json::to_vec(&json!({
            "entries": {
                "component:glass": {
                    "kind": "motion-artifact",
                    "digest": artifact_digest.to_wire(),
                    "abi": "valle.motion/artifact@1",
                    "descriptor": {
                        "readsDestination": true,
                        "boundarySampling": "left-limit"
                    }
                }
            }
        }))
        .unwrap(),
    )
    .unwrap()
}

fn open_render(
    timeline: &valle_timeline::internal::CanonicalTimeline,
    manifest: &ResourceManifest,
    bindings: &ResourceBindings,
    capabilities: &Capabilities,
) -> Result<EngineRender, EngineOpenReport> {
    let timeline_json = String::from_utf8(canonical_bytes(timeline).unwrap()).unwrap();
    let manifest_json = std::str::from_utf8(manifest.canonical_bytes()).unwrap();
    let bundle_json = canonical_verified_binding_bundle(bindings, capabilities).unwrap();
    let profile_json = canonical_fixed_execution_profile(COMMON_PROFILE_KEY).unwrap();
    let files = fixed_package_files(&timeline_json, manifest_json, &bundle_json, &profile_json);
    let package_manifest = canonical_fixed_package_manifest(&files).unwrap();
    match open_verified_fixed_package(&package_manifest, &files) {
        Ok(opened) => Ok(opened.engine_render()),
        Err(FixedPackageOpenError::Engine(report)) => Err(report),
        Err(error) => panic!("test fixture failed before Engine admission: {error}"),
    }
}

#[test]
fn transition_filter_and_motion_footprints_form_one_serial_dependency_range() {
    let timeline = timeline();
    let artifact = glass_artifact();
    let manifest = manifest(&artifact);
    let ResourceEntryWire::MotionArtifact {
        abi,
        descriptor,
        digest,
    } = manifest.entries().get("component:glass").unwrap()
    else {
        panic!("fixture must contain the Motion artifact")
    };
    let bindings = ResourceBindings::new()
        .with_binding(
            "component:glass",
            ResourceBinding::new(
                digest.clone(),
                VerifiedHandleId::new(1).unwrap(),
                VerifiedResourceFacts::MotionArtifact {
                    abi: *abi,
                    descriptor: descriptor.clone(),
                    artifact: Arc::clone(&artifact),
                    temporal_footprint: VisualFootprint::new(2, 1),
                },
            ),
        )
        .unwrap();
    let capabilities = Capabilities::new()
        .with_artifact_abi("valle.motion/artifact@1")
        .with_extension_kernel(
            FILTER_KIND,
            ExtensionKernelCapability::new(
                EXTENSION_COLOR_GAIN_ABI,
                engine_owned_kernel_implementation_digest(EXTENSION_COLOR_GAIN_ABI).unwrap(),
            )
            .unwrap()
            .with_visual_footprint(VisualFootprint::new(3, 4)),
        )
        .unwrap()
        .with_extension_kernel(
            TRANSITION_KIND,
            ExtensionKernelCapability::new(
                EXTENSION_CROSS_FADE_ABI,
                engine_owned_kernel_implementation_digest(EXTENSION_CROSS_FADE_ABI).unwrap(),
            )
            .unwrap()
            .with_visual_footprint(VisualFootprint::new(5, 6)),
        )
        .unwrap();

    let unbound_transition = Capabilities::new()
        .with_artifact_abi("valle.motion/artifact@1")
        .with_extension_kernel(
            FILTER_KIND,
            ExtensionKernelCapability::new(
                EXTENSION_COLOR_GAIN_ABI,
                engine_owned_kernel_implementation_digest(EXTENSION_COLOR_GAIN_ABI).unwrap(),
            )
            .unwrap()
            .with_visual_footprint(VisualFootprint::new(3, 4)),
        )
        .unwrap()
        .with_extension_kernel(
            TRANSITION_KIND,
            ExtensionKernelCapability::new(
                EXTENSION_CROSS_FADE_ABI,
                ContentDigest::parse(
                    "sha256:2222222222222222222222222222222222222222222222222222222222222222",
                )
                .unwrap(),
            )
            .unwrap()
            .with_visual_footprint(VisualFootprint::new(5, 6)),
        )
        .unwrap();
    let report = open_render(&timeline, &manifest, &bindings, &unbound_transition).unwrap_err();
    assert!(report.contains(EngineOpenDiagnosticCode::UnsupportedExtensionKernel));

    let render = open_render(&timeline, &manifest, &bindings, &capabilities).unwrap();
    let items = render.visual().tracks()[0].items();
    let CompiledVisualItem::Clip { clip: left } = &items[0] else {
        panic!("first item must be the left Motion clip")
    };
    let CompiledVisualItem::Transition { transition } = &items[1] else {
        panic!("middle item must be the extension transition")
    };
    let CompiledVisualItem::Clip { clip: right } = &items[2] else {
        panic!("last item must be the right Motion clip")
    };

    assert_eq!(
        (transition.window().start(), transition.window().end()),
        (3, 6)
    );
    assert!(matches!(
        transition.kernel(),
        CompiledTransitionKernel::Extension { .. }
    ));
    assert_eq!(transition.footprint(), VisualFootprint::new(5, 6));
    assert_ne!(left.source_index(), right.source_index());

    let expected_dependency_range = json!({
        "unit": "frames",
        "range": {"start": -7, "end": 17}
    });
    for source_index in [left.source_index(), right.source_index()] {
        let source = render.sources().source(source_index).unwrap();
        let compiled_motion = source.motion().expect("both endpoints must stay Motion");
        assert!(compiled_motion.reads_destination());
        assert!(compiled_motion.artifact().reads_destination());
        assert_eq!(
            serde_json::to_value(source).unwrap()["dependencyRange"],
            expected_dependency_range,
            "filter F=(3,4), transition T=(5,6), and Motion M=(2,1) must compose serially"
        );
    }
}
