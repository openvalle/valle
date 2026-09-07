use serde_json::{Value, json};
use valle_compiler::timeline_contract::{
    ResourceEntryWire, ResourceManifest, canonical_bytes, decode_canonical,
    decode_resource_manifest,
};
use valle_engine::render::{
    Capabilities, EXTENSION_COLOR_GAIN_ABI, ExtensionKernelCapability, ResourceBinding,
    ResourceBindings, VerifiedHandleId, VerifiedResourceFacts, VisualFootprint,
    engine_owned_kernel_implementation_digest,
};
use valle_engine::{
    fixed_package::{
        COMMON_PROFILE_KEY, canonical_fixed_execution_profile, canonical_fixed_package_manifest,
        canonical_verified_binding_bundle, fixed_package_files, open_verified_fixed_package,
    },
    product::EngineRender,
};

#[allow(dead_code)] // This support module is compiled independently by each integration test.
pub const DEFAULT_DIGEST: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const COLOR_GAIN_KIND: &str = "example.visual/color-gain@1";

fn constant(value: Value) -> Value {
    json!({"type": "constant", "value": value})
}

pub fn opened_image_render(digest: &str) -> EngineRender {
    opened_image_render_with_gain(digest, None)
}

#[allow(dead_code)] // This support module is compiled independently by each integration test.
pub fn opened_color_gain_render(digest: &str, gain: f64) -> EngineRender {
    opened_image_render_with_gain(digest, Some(gain))
}

fn opened_image_render_with_gain(digest: &str, extension_gain: Option<f64>) -> EngineRender {
    let filters = extension_gain
        .map(|gain| {
            vec![json!({
                "id": "filter:color-gain",
                "type": COLOR_GAIN_KIND,
                "parameters": {"gain": gain}
            })]
        })
        .unwrap_or_default();
    let document = decode_canonical(
        &serde_json::to_string(&json!({
            "document": {
                "canvas": {
                    "width": 2,
                    "height": 2,
                    "fps": "30/1",
                    "sampleRate": 48000,
                    "channelLayout": "stereo",
                    "colorSpace": "srgb",
                    "duration": "1/1"
                },
                "background": {"color": "#0c2238ff"},
                "visual": {
                    "tracks": [{
                        "id": "track:main",
                        "items": [{
                            "type": "clip",
                            "id": "clip:hero",
                            "duration": "1/1",
                            "layer": {
                                "transform": {
                                    "position": constant(json!([0.5, 0.5])),
                                    "scale": constant(json!([1.0, 1.0])),
                                    "rotation": constant(json!(0.0)),
                                    "anchor": [0.5, 0.5]
                                },
                                "opacity": constant(json!(1.0)),
                                "mask": null,
                                "filters": filters,
                                "blend": "normal"
                            },
                            "source": {
                                "type": "image",
                                "resource": "asset:hero",
                                "sampling": {"fit": "fill"}
                            }
                        }]
                    }]
                },
                "audio": {"tracks": []},
                "adjustments": [],
                "captions": {"tracks": []},
                "camera": null,
                "metadata": {}
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let manifest = decode_resource_manifest(
        &serde_json::to_vec(&json!({
            "entries": {
                "asset:hero": {
                    "kind": "image",
                    "digest": digest,
                    "descriptor": {
                        "width": 2,
                        "height": 2,
                        "orientation": "identity",
                        "color": {
                            "primaries": "bt709",
                            "transfer": "bt709",
                            "matrix": "bt709",
                            "fullRange": false
                        }
                    }
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let binding = image_binding(&manifest);
    let mut capabilities = Capabilities::new();
    if extension_gain.is_some() {
        capabilities = capabilities
            .with_extension_kernel(
                COLOR_GAIN_KIND,
                ExtensionKernelCapability::new(
                    EXTENSION_COLOR_GAIN_ABI,
                    engine_owned_kernel_implementation_digest(EXTENSION_COLOR_GAIN_ABI).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
    }
    let timeline_json = String::from_utf8(canonical_bytes(&document).unwrap()).unwrap();
    let manifest_json = std::str::from_utf8(manifest.canonical_bytes()).unwrap();
    let bundle_json = canonical_verified_binding_bundle(&binding, &capabilities).unwrap();
    let profile_json = canonical_fixed_execution_profile(COMMON_PROFILE_KEY).unwrap();
    let files = fixed_package_files(&timeline_json, manifest_json, &bundle_json, &profile_json);
    let package_manifest = canonical_fixed_package_manifest(&files).unwrap();
    open_verified_fixed_package(&package_manifest, &files)
        .unwrap()
        .engine_render()
}

fn image_binding(manifest: &ResourceManifest) -> ResourceBindings {
    let ResourceEntryWire::Image {
        digest, descriptor, ..
    } = manifest.entries().get("asset:hero").unwrap()
    else {
        panic!("fixture entry must be an image")
    };
    ResourceBindings::new()
        .with_binding(
            "asset:hero",
            ResourceBinding::new(
                digest.clone(),
                VerifiedHandleId::new(7).unwrap(),
                VerifiedResourceFacts::Image {
                    descriptor: descriptor.clone(),
                    temporal_footprint: VisualFootprint::default(),
                },
            ),
        )
        .unwrap()
}
