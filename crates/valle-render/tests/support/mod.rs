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

#[allow(dead_code)] // Shared by integration test binaries.
pub fn opened_motion_render(source: &str, width: u32, height: u32) -> EngineRender {
    opened_motion_effect_render(source, None, width, height)
}

#[allow(dead_code)]
pub fn opened_motion_effect_render(
    source: &str,
    effect: Option<&str>,
    width: u32,
    height: u32,
) -> EngineRender {
    use std::sync::Arc;
    use valle_engine::resource::ContentDigest;
    let mut entries = serde_json::Map::new();
    let mut artifacts = Vec::new();
    for (name, code) in std::iter::once(("component:test", source))
        .chain(effect.map(|code| ("component:effect", code)))
    {
        let artifact = Arc::new(
            valle_compiler::motion::compile_motion(code)
                .unwrap()
                .artifact,
        );
        let digest = ContentDigest::of_bytes(&valle_motion::canonical_bytes(&artifact).unwrap());
        entries.insert(name.into(), json!({"kind":"motion-artifact", "digest":digest.to_wire(),
            "abi":"valle.motion/artifact@1", "descriptor":{"readsDestination":artifact.reads_destination(),"boundarySampling":"left-limit"}}));
        artifacts.push((name, digest, artifact));
    }
    let manifest =
        decode_resource_manifest(&serde_json::to_vec(&json!({"entries":entries})).unwrap())
            .unwrap();
    let mut bindings = ResourceBindings::new();
    for (i, (name, digest, artifact)) in artifacts.into_iter().enumerate() {
        let ResourceEntryWire::MotionArtifact {
            abi, descriptor, ..
        } = &manifest.entries()[name]
        else {
            unreachable!()
        };
        bindings = bindings
            .with_binding(
                name,
                ResourceBinding::new(
                    digest,
                    VerifiedHandleId::new(i as u64 + 1).unwrap(),
                    VerifiedResourceFacts::MotionArtifact {
                        abi: *abi,
                        descriptor: descriptor.clone(),
                        artifact,
                        temporal_footprint: VisualFootprint::default(),
                    },
                ),
            )
            .unwrap();
    }
    let effects = effect.map(|_| vec![json!({"component":"component:effect","role":{"type":"effect"},"sourceStart":"0/1","sourceDuration":"1/1","rate":"1/1","endBehavior":"hold","fit":"contain","props":{},"resources":{}})]).unwrap_or_default();
    let document = decode_canonical(&json!({"document":{
        "canvas":{"width":width,"height":height,"fps":"30/1","sampleRate":48000,"channelLayout":"stereo","colorSpace":"srgb","duration":"1/1"},
        "background":{"color":"#000000ff"},"visual":{"tracks":[{"id":"track:test","items":[{
            "type":"clip","id":"clip:test","duration":"1/1",
            "layer":{"transform":{"position":constant(json!([0.5,0.5])),"scale":constant(json!([1,1])),"rotation":constant(json!(0)),"anchor":[0.5,0.5]},"opacity":constant(json!(1)),"mask":null,"filters":[],"effects":effects,"blend":"normal"},
            "source":{"type":"motion","component":"component:test","sourceStart":"0/1","role":{"type":"clip"},"sourceDuration":"1/1","rate":"1/1","endBehavior":"hold","fit":"contain","props":{},"resources":{}}
        }]}]},"audio":{"tracks":[]},"adjustments":[],"captions":{"tracks":[]},"camera":null,"metadata":{}
    }}).to_string()).unwrap();
    let timeline = String::from_utf8(canonical_bytes(&document).unwrap()).unwrap();
    let bundle = canonical_verified_binding_bundle(
        &bindings,
        &Capabilities::new().with_artifact_abi("valle.motion/artifact@1"),
    )
    .unwrap();
    let profile = canonical_fixed_execution_profile(COMMON_PROFILE_KEY).unwrap();
    let files = fixed_package_files(
        &timeline,
        std::str::from_utf8(manifest.canonical_bytes()).unwrap(),
        &bundle.json,
        &profile,
        &bundle.resource_bytes,
    );
    let outer = canonical_fixed_package_manifest(&files).unwrap();
    if let Ok(directory) = std::env::var("VALLE_MOTION_PACKAGE_TEST_OUTPUT") {
        std::fs::write(
            std::path::Path::new(&directory).join(format!("motion-package-{width}.json")),
            serde_json::to_vec(
                &json!({"fixedPackageManifestJson":outer,"timelineJson":timeline,
                "resourceManifestJson":std::str::from_utf8(manifest.canonical_bytes()).unwrap(),
                "verifiedBindingBundleJson":bundle.json,"resourceBytes":bundle.resource_bytes}),
            )
            .unwrap(),
        )
        .unwrap();
    }
    open_verified_fixed_package(&outer, &files)
        .unwrap()
        .engine_render()
}

#[allow(dead_code)] // Shared by integration test binaries.
pub fn opened_image_render(digest: &str) -> EngineRender {
    opened_image_render_with_gain(digest, None, 2, 2)
}

#[allow(dead_code)] // This support module is compiled independently by each integration test.
pub fn opened_color_gain_render(digest: &str, gain: f64) -> EngineRender {
    opened_image_render_with_gain(digest, Some(gain), 2, 2)
}

#[allow(dead_code)]
pub fn opened_image_render_sized(digest: &str, width: u32, height: u32) -> EngineRender {
    opened_image_render_with_gain(digest, None, width, height)
}

fn opened_image_render_with_gain(
    digest: &str,
    extension_gain: Option<f64>,
    width: u32,
    height: u32,
) -> EngineRender {
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
                    "width": width,
                    "height": height,
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
                        "width": width,
                        "height": height,
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
    let files = fixed_package_files(
        &timeline_json,
        manifest_json,
        &bundle_json.json,
        &profile_json,
        &bundle_json.resource_bytes,
    );
    let package_manifest = canonical_fixed_package_manifest(&files).unwrap();
    if let Ok(directory) = std::env::var("VALLE_IMAGE_PACKAGE_TEST_OUTPUT") {
        std::fs::write(
            format!("{directory}/image-package-{width}.json"),
            serde_json::to_vec(&json!({
                "fixedPackageManifestJson":package_manifest,
                "timelineJson":timeline_json,
                "resourceManifestJson":manifest_json,
                "verifiedBindingBundleJson":bundle_json.json,
                "resourceBytes":bundle_json.resource_bytes,
            }))
            .unwrap(),
        )
        .unwrap();
    }
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
