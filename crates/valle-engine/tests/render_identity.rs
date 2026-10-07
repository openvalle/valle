use std::sync::Arc;

use serde_json::{Value, json};
use valle_compiler::motion::compile_motion_with_resources;
use valle_engine::fixed_package::{
    COMMON_PROFILE_KEY, FixedPackageOpenError, canonical_fixed_execution_profile,
    canonical_fixed_package_manifest, canonical_verified_binding_bundle, fixed_package_files,
    open_verified_fixed_package,
};
use valle_engine::{
    compositor::{
        graph::GraphCapability,
        lower::{BackendCapabilities, FramebufferFetchSemantics, RenderPlanTemplate},
    },
    resource::{Extent2d, ExternalPixelLayout, TextureFormat, TextureUsage},
};
use valle_engine::{
    product::EngineRender,
    render::{
        AUDIO_GAIN_EFFECT_ABI, AUDIO_GAIN_EFFECT_KIND, AudioFootprint, CAPTION_GLYPH_RUN_ABI,
        COMMON_AUDIO_ABI, Capabilities, CompiledAudioChannelMap, CompiledAudioItem,
        CompiledVisualItem, EXTENSION_COLOR_GAIN_ABI, EngineOpenDiagnosticCode, EngineOpenPhase,
        EngineOpenReport, EvaluatedVisualOperation, ExecutionLimits, ExecutionProfile,
        ExtensionKernelCapability, ResourceBinding, ResourceBindings, ResourceDependency,
        RuntimeFault, SampleRange, VerifiedHandleId, VerifiedResourceFacts, VisualFootprint,
        engine_owned_kernel_implementation_digest,
    },
};
use valle_motion::{ResourceRef, SceneArtifact};
use valle_timeline::internal::FrameKey;
use valle_timeline::internal::{
    ContentDigest, ResourceManifest, canonical_bytes, decode_canonical, decode_resource_manifest,
    wire::resource::ResourceEntryWire,
};

const IMAGE_DIGEST: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const AUDIO_DIGEST: &str =
    "sha256:1212121212121212121212121212121212121212121212121212121212121212";
const VIDEO_DIGEST: &str =
    "sha256:2323232323232323232323232323232323232323232323232323232323232323";
const MISMATCH_DIGEST: &str =
    "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";
const SCHEMA_DIGEST: &str =
    "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const KERNEL_DIGEST: &str =
    "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const OTHER_KERNEL_DIGEST: &str =
    "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
const FONT_DIGEST: &str = "sha256:d0332f52868370fd83ae7fa46470f90c8f2eab2fcf12bc4f88080b340c95a830";
const FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/katex/KaTeX_Main-Regular.ttf");

fn constant(value: Value) -> Value {
    json!({"type": "constant", "value": value})
}

fn layer(filters: Value) -> Value {
    json!({
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
    })
}

fn document_with_source(
    source: Value,
    filters: Value,
    track_id: &str,
    clip_id: &str,
    metadata: Value,
) -> Value {
    json!({
        "document": {
            "canvas": {
                "width": 1920,
                "height": 1080,
                "fps": "30/1",
                "sampleRate": 48000,
                "channelLayout": "stereo",
                "colorSpace": "srgb",
                "duration": "1/1"
            },
            "background": {"color": "#000000ff"},
            "visual": {
                "tracks": [{
                    "id": track_id,
                    "items": [{
                        "type": "clip",
                        "id": clip_id,
                        "duration": "1/1",
                        "layer": layer(filters),
                        "source": source
                    }]
                }]
            },
            "audio": {"tracks": []},
            "adjustments": [],
            "captions": {"tracks": []},
            "camera": null,
            "metadata": metadata
        }
    })
}

fn image_document(resource_id: &str) -> Value {
    document_with_source(
        json!({
            "type": "image",
            "resource": resource_id,
            "sampling": {"fit": "contain"}
        }),
        json!([]),
        "track:main",
        "clip:image",
        json!({}),
    )
}

fn motion_document(component: &str) -> Value {
    document_with_source(
        json!({
            "type": "motion",
            "component": component,
            "fit": "contain",
            "sourceStart": "0/1",
            "role": {"type":"clip"}, "sourceDuration": "1/1",
            "rate": "1/1",
            "endBehavior": "hold",
            "props": {"opacity": constant(json!(0.75))},
            "resources": {"logo": "asset:logo"},
        }),
        json!([]),
        "track:main",
        "clip:motion",
        json!({}),
    )
}

fn audio_document(resource_id: &str, duration: &str) -> Value {
    let mut document = document_with_source(
        json!({"type": "solid", "color": "#00000000"}),
        json!([]),
        "track:placeholder",
        "clip:placeholder",
        json!({}),
    );
    document["document"]["visual"]["tracks"] = json!([]);
    document["document"]["audio"]["tracks"] = json!([{
        "id": "audio:main",
        "items": [{
            "type": "clip",
            "id": "clip:audio",
            "duration": duration,
            "source": {
                "type": "media",
                "resource": resource_id,
                "sourceStart": "0/1",
                "rate": "1/1",
                "endBehavior": "error"
            },
            "gain": constant(json!(1.0)),
            "pan": constant(json!(0.0)),
            "effects": []
        }]
    }]);
    document
}

fn image_entry() -> Value {
    json!({
        "kind": "image",
        "digest": IMAGE_DIGEST,
        "descriptor": {
            "width": 1920,
            "height": 1080,
            "orientation": "identity",
            "color": {
                "primaries": "bt709",
                "transfer": "bt709",
                "matrix": "bt709",
                "fullRange": false
            }
        }
    })
}

fn audio_entry() -> Value {
    json!({
        "kind": "audio",
        "digest": AUDIO_DIGEST,
        "descriptor": {
            "duration": "1/1",
            "timeBase": "1/48000",
            "presentationIndexDigest": SCHEMA_DIGEST,
            "sampleRate": 48000,
            "channelLayout": "stereo",
            "audioStream": 0
        }
    })
}

fn video_entry(duration: &str) -> Value {
    json!({
        "kind": "video",
        "digest": VIDEO_DIGEST,
        "descriptor": {
            "duration": duration,
            "timeBase": "1/90000",
            "presentationIndexDigest": SCHEMA_DIGEST,
            "width": 1920,
            "height": 1080,
            "orientation": "identity",
            "color": {
                "primaries": "bt709",
                "transfer": "bt709",
                "matrix": "bt709",
                "fullRange": false
            },
            "videoStream": 0,
            "audioStream": null
        }
    })
}

fn motion_artifact() -> Arc<SceneArtifact> {
    Arc::new(
        compile_motion_with_resources(
            r#"
export const composition = { width: 1920, height: 1080, duration: 1 };
export const controls = ({
  props: { opacity: number({ default: 1, min: 0, max: 1 }) },
  assets: { logo: asset({ kind: "image", required: true }) },
});
export default function Title(ctx, props) {
  return <View style={{ width: 1920, height: 1080, opacity: props.opacity }} />;
}
"#,
            &[ResourceRef {
                control: "logo".to_owned(),
                content_hash: ContentDigest::parse(IMAGE_DIGEST).unwrap(),
            }],
        )
        .expect("Motion fixture must compile")
        .artifact,
    )
}

fn video_motion_artifact() -> Arc<SceneArtifact> {
    Arc::new(
        compile_motion_with_resources(
            r#"
export const composition = { width: 1920, height: 1080, duration: 1 };
export const controls = ({
  props: {
    opacity: number({ default: 1, min: 0, max: 1 }),
    linear: number({ default: 0, min: 0, max: 1 }),
  },
  assets: { footage: asset({ kind: "video", required: true }) },
});
export default function HeldVideo(ctx, props) {
  return (
    <View style={{ width: 1920, height: 1080 }}>
      <Video src="asset://footage" sourceStart={0.25} speed={0.75} style={{ display: "block", width: 1920, height: 1080, opacity: props.opacity }} />
      <Video src="asset://footage" sourceStart={0.1} speed={0.6} style={{ display: "block", width: 1920, height: 1080, opacity: props.opacity }} />
    </View>
  );
}
"#,
            &[ResourceRef {
                control: "footage".to_owned(),
                content_hash: ContentDigest::parse(VIDEO_DIGEST).unwrap(),
            }],
        )
        .expect("video Motion fixture must compile")
        .artifact,
    )
}

fn content_digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest::of_bytes(bytes)
}

#[cfg(feature = "text")]
fn two_face_font_collection(font: &[u8]) -> Vec<u8> {
    assert!(font.len() >= 12, "fixture must contain an sfnt header");
    let table_count = u16::from_be_bytes([font[4], font[5]]) as usize;
    let directory_end = 12 + table_count * 16;
    assert!(
        font.len() >= directory_end,
        "fixture table directory is truncated"
    );

    // TTC header (v1), with two collection entries sharing one table directory. Shared table
    // storage is explicitly permitted by the TTC format; offsets inside each sfnt directory are
    // absolute from the beginning of the collection.
    const TTC_HEADER_LEN: u32 = 20;
    let mut face = font.to_vec();
    for table_index in 0..table_count {
        let offset_index = 12 + table_index * 16 + 8;
        let offset = u32::from_be_bytes(face[offset_index..offset_index + 4].try_into().unwrap());
        face[offset_index..offset_index + 4]
            .copy_from_slice(&offset.checked_add(TTC_HEADER_LEN).unwrap().to_be_bytes());
    }

    let mut collection = Vec::with_capacity(TTC_HEADER_LEN as usize + face.len());
    collection.extend_from_slice(b"ttcf");
    collection.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    collection.extend_from_slice(&2_u32.to_be_bytes());
    collection.extend_from_slice(&TTC_HEADER_LEN.to_be_bytes());
    collection.extend_from_slice(&TTC_HEADER_LEN.to_be_bytes());
    collection.extend_from_slice(&face);
    assert!(ttf_parser::Face::parse(&collection, 1).is_ok());
    collection
}

fn motion_entry(artifact: &SceneArtifact) -> Value {
    let artifact_digest = content_digest(&valle_motion::canonical_bytes(artifact).unwrap());
    json!({
        "kind": "motion-artifact",
        "digest": artifact_digest.to_wire(),
        "abi": "valle.motion/artifact@1",
        "descriptor": {
            "readsDestination": artifact.reads_destination(),
            "boundarySampling": "left-limit"
        }
    })
}

fn font_entry() -> Value {
    json!({
        "kind": "font",
        "digest": FONT_DIGEST,
        "descriptor": {"faceIndex": 0, "variationAxes": {}}
    })
}

fn audio_entry_at(sample_rate: u32, duration: &str) -> Value {
    json!({
        "kind": "audio",
        "digest": AUDIO_DIGEST,
        "descriptor": {
            "duration": duration,
            "timeBase": format!("1/{sample_rate}"),
            "presentationIndexDigest": SCHEMA_DIGEST,
            "sampleRate": sample_rate,
            "channelLayout": "stereo",
            "audioStream": 0
        }
    })
}

fn solid_clip(id: &str, duration: &str, color: &str) -> Value {
    json!({
        "type": "clip",
        "id": id,
        "duration": duration,
        "layer": layer(json!([])),
        "source": {"type": "solid", "color": color}
    })
}

fn transition_document(transition_duration: &str) -> Value {
    let mut document = document_with_source(
        json!({"type": "solid", "color": "#000000ff"}),
        json!([]),
        "track:main",
        "clip:placeholder",
        json!({}),
    );
    document["document"]["canvas"]["fps"] = json!("4/1");
    document["document"]["canvas"]["duration"] = json!("2/1");
    document["document"]["visual"]["tracks"][0]["items"] = json!([
        solid_clip("clip:left", "1/1", "#ff0000ff"),
        {
            "type": "transition",
            "id": "transition:cut",
            "duration": transition_duration,
            "kernel": {"type": "fade"}
        },
        solid_clip("clip:right", "1/1", "#0000ffff")
    ]);
    document
}

fn audio_crossfade_document(crossfade_duration: &str) -> Value {
    let mut document = transition_document("1/4");
    document["document"]["visual"]["tracks"] = json!([]);
    document["document"]["canvas"]["sampleRate"] = json!(4);
    document["document"]["audio"]["tracks"] = json!([{
        "id": "audio:main",
        "items": [
            {
                "type": "clip",
                "id": "audio:left",
                "duration": "1/1",
                "source": {
                    "type": "media",
                    "resource": "audio:music",
                    "sourceStart": "0/1",
                    "rate": "1/1",
                    "endBehavior": "hold"
                },
                "gain": constant(json!(1.0)),
                "pan": constant(json!(0.0)),
                "effects": []
            },
            {
                "type": "crossfade",
                "id": "crossfade:cut",
                "duration": crossfade_duration
            },
            {
                "type": "clip",
                "id": "audio:right",
                "duration": "1/1",
                "source": {
                    "type": "media",
                    "resource": "audio:music",
                    "sourceStart": "0/1",
                    "rate": "1/1",
                    "endBehavior": "hold"
                },
                "gain": constant(json!(1.0)),
                "pan": constant(json!(0.0)),
                "effects": []
            }
        ]
    }]);
    document
}

fn manifest(entries: Value) -> ResourceManifest {
    decode_resource_manifest(
        &serde_json::to_vec(&json!({
            "entries": entries
        }))
        .unwrap(),
    )
    .unwrap()
}

fn timeline(value: &Value) -> valle_timeline::internal::CanonicalTimeline {
    decode_canonical(&serde_json::to_string(value).unwrap()).unwrap()
}

fn digest(value: &str) -> ContentDigest {
    ContentDigest::parse(value).unwrap()
}

fn verified_facts(entry: &ResourceEntryWire) -> VerifiedResourceFacts {
    match entry {
        ResourceEntryWire::Video { descriptor, .. } => VerifiedResourceFacts::Video {
            descriptor: descriptor.clone(),
            temporal_footprint: VisualFootprint::new(1, 1),
        },
        ResourceEntryWire::Audio { descriptor, .. } => VerifiedResourceFacts::Audio {
            descriptor: descriptor.clone(),
            temporal_footprint: AudioFootprint::new(8, 8).unwrap(),
        },
        ResourceEntryWire::Image { descriptor, .. } => VerifiedResourceFacts::Image {
            descriptor: descriptor.clone(),
            temporal_footprint: VisualFootprint::default(),
        },
        ResourceEntryWire::Font { descriptor, .. } => VerifiedResourceFacts::Font {
            descriptor: descriptor.clone(),
            bytes: Arc::from(FONT_BYTES),
        },
        ResourceEntryWire::MotionArtifact { .. } => {
            panic!("Motion bindings require an executable SceneArtifact")
        }
        ResourceEntryWire::Model3d { .. } | ResourceEntryWire::Environment { .. } => {
            panic!("Scene3D bindings require verified model or environment bytes")
        }
        ResourceEntryWire::Shader {
            abi, descriptor, ..
        } => VerifiedResourceFacts::Shader {
            abi: *abi,
            descriptor: descriptor.clone(),
            manifest_bytes: Arc::from(&b""[..]),
            source_bytes: Arc::from(&b""[..]),
        },
    }
}

fn motion_binding(
    manifest: &ResourceManifest,
    resource_id: &str,
    artifact: Arc<SceneArtifact>,
    handle: u64,
) -> ResourceBinding {
    let entry = manifest.entries().get(resource_id).unwrap();
    let ResourceEntryWire::MotionArtifact {
        abi,
        descriptor,
        digest,
    } = entry
    else {
        panic!("fixture must be a Motion artifact")
    };
    ResourceBinding::new(
        digest.clone(),
        VerifiedHandleId::new(handle).unwrap(),
        VerifiedResourceFacts::MotionArtifact {
            abi: *abi,
            descriptor: descriptor.clone(),
            artifact,
            temporal_footprint: VisualFootprint::new(2, 1),
        },
    )
    .with_dependency(ResourceDependency::new("logo", "asset:logo").unwrap())
}

fn custom_motion_binding(
    manifest: &ResourceManifest,
    resource_id: &str,
    artifact: Arc<SceneArtifact>,
    dependencies: &[(&str, &str)],
    handle: u64,
) -> ResourceBinding {
    let entry = manifest.entries().get(resource_id).unwrap();
    let ResourceEntryWire::MotionArtifact {
        abi,
        descriptor,
        digest,
    } = entry
    else {
        panic!("fixture must be a Motion artifact")
    };
    dependencies.iter().fold(
        ResourceBinding::new(
            digest.clone(),
            VerifiedHandleId::new(handle).unwrap(),
            VerifiedResourceFacts::MotionArtifact {
                abi: *abi,
                descriptor: descriptor.clone(),
                artifact,
                temporal_footprint: VisualFootprint::new(2, 1),
            },
        ),
        |binding, (role, target)| {
            binding.with_dependency(ResourceDependency::new(*role, *target).unwrap())
        },
    )
}

fn entry_digest(entry: &ResourceEntryWire) -> &ContentDigest {
    match entry {
        ResourceEntryWire::Video { digest, .. }
        | ResourceEntryWire::Audio { digest, .. }
        | ResourceEntryWire::Image { digest, .. }
        | ResourceEntryWire::Font { digest, .. }
        | ResourceEntryWire::MotionArtifact { digest, .. }
        | ResourceEntryWire::Model3d { digest, .. }
        | ResourceEntryWire::Environment { digest, .. }
        | ResourceEntryWire::Shader { digest, .. } => digest,
    }
}

fn binding(
    manifest: &ResourceManifest,
    resource_id: &str,
    digest_override: Option<&str>,
    handle: u64,
) -> ResourceBinding {
    let entry = manifest.entries().get(resource_id).unwrap();
    ResourceBinding::new(
        digest_override
            .map(digest)
            .unwrap_or_else(|| entry_digest(entry).clone()),
        VerifiedHandleId::new(handle).unwrap(),
        verified_facts(entry),
    )
}

fn kernel_capability(value: &str) -> ExtensionKernelCapability {
    ExtensionKernelCapability::new(EXTENSION_COLOR_GAIN_ABI, digest(value)).unwrap()
}

fn known_kernel_capability() -> ExtensionKernelCapability {
    ExtensionKernelCapability::new(
        EXTENSION_COLOR_GAIN_ABI,
        engine_owned_kernel_implementation_digest(EXTENSION_COLOR_GAIN_ABI).unwrap(),
    )
    .unwrap()
}

fn audio_gain_capabilities() -> Capabilities {
    Capabilities::new()
        .with_extension_kernel(
            AUDIO_GAIN_EFFECT_KIND,
            ExtensionKernelCapability::new(
                AUDIO_GAIN_EFFECT_ABI,
                engine_owned_kernel_implementation_digest(AUDIO_GAIN_EFFECT_ABI).unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
}

fn image_bindings(manifest: &ResourceManifest, resource_id: &str, handle: u64) -> ResourceBindings {
    ResourceBindings::new()
        .with_binding(resource_id, binding(manifest, resource_id, None, handle))
        .unwrap()
}

fn open(
    timeline: &valle_timeline::internal::CanonicalTimeline,
    manifest: &ResourceManifest,
    bindings: &ResourceBindings,
    capabilities: &Capabilities,
    profile: &ExecutionProfile,
) -> Result<EngineRender, EngineOpenReport> {
    match open_package(timeline, manifest, bindings, capabilities, profile) {
        Ok(render) => Ok(render),
        Err(FixedPackageOpenError::Engine(report)) => Err(report),
        Err(error) => panic!("test fixture failed before Engine admission: {error}"),
    }
}

fn open_package(
    timeline: &valle_timeline::internal::CanonicalTimeline,
    manifest: &ResourceManifest,
    bindings: &ResourceBindings,
    capabilities: &Capabilities,
    profile: &ExecutionProfile,
) -> Result<EngineRender, FixedPackageOpenError> {
    let timeline_json = String::from_utf8(canonical_bytes(timeline).unwrap()).unwrap();
    let manifest_json = std::str::from_utf8(manifest.canonical_bytes()).unwrap();
    let bundle_json = canonical_verified_binding_bundle(bindings, capabilities).unwrap();
    let profile_json = canonical_fixed_execution_profile(COMMON_PROFILE_KEY).unwrap();
    let baseline = baseline_profile();
    assert_eq!(
        profile, &baseline,
        "integration tests open only the public fixed profile"
    );
    let files = fixed_package_files(&timeline_json, manifest_json, &bundle_json, &profile_json);
    let package_manifest = canonical_fixed_package_manifest(&files).unwrap();
    open_verified_fixed_package(&package_manifest, &files).map(|opened| opened.engine_render())
}

fn profile(name: &str, numeric_abi: &str) -> ExecutionProfile {
    ExecutionProfile::new(
        name,
        numeric_abi,
        COMMON_AUDIO_ABI,
        "valle.motion/eval@1",
        CAPTION_GLYPH_RUN_ABI,
        ExecutionLimits::new(64, 8, 32).unwrap(),
    )
    .unwrap()
}

fn baseline_profile() -> ExecutionProfile {
    profile("common", "valle.numeric/common@1")
}

#[test]
fn open_freezes_resource_snapshot_and_evaluate_uses_only_compiled_handles() {
    let timeline = timeline(&image_document("asset:hero"));
    let manifest = manifest(json!({"asset:hero": image_entry()}));
    let bindings = image_bindings(&manifest, "asset:hero", 7);
    let profile = baseline_profile();
    let render = open(
        &timeline,
        &manifest,
        &bindings,
        &Capabilities::new(),
        &profile,
    )
    .unwrap();
    assert_eq!(render.resources().resource_count(), 1);
    assert_eq!(render.canvas().frame_count(), 30);
    assert_eq!(render.canvas().sample_count(), 48_000);

    drop(bindings);
    drop(manifest);
    drop(timeline);
    let frame = render.evaluate(FrameKey::new(0)).unwrap();
    assert_eq!(frame.render_id(), render.render_id());
    assert_eq!(frame.resources().len(), 1);
    assert_eq!(frame.resources()[0].handle().get(), 7);
    assert!(matches!(
        frame.resources()[0].facts(),
        VerifiedResourceFacts::Image {
            temporal_footprint,
            ..
        } if *temporal_footprint == VisualFootprint::default()
    ));
    assert!(matches!(
        render.evaluate(FrameKey::new(30)),
        Err(RuntimeFault::FrameOutOfRange { .. })
    ));
}

#[test]
fn unused_manifest_entries_and_verified_handles_do_not_change_render_identity() {
    let timeline = timeline(&image_document("asset:hero"));
    let base_manifest = manifest(json!({"asset:hero": image_entry()}));
    let extended_manifest = manifest(json!({
        "asset:hero": image_entry(),
        "font:unused": font_entry()
    }));
    let profile = baseline_profile();

    let first = open(
        &timeline,
        &base_manifest,
        &image_bindings(&base_manifest, "asset:hero", 1),
        &Capabilities::new(),
        &profile,
    )
    .unwrap();
    let second = open(
        &timeline,
        &extended_manifest,
        &image_bindings(&extended_manifest, "asset:hero", 999),
        &Capabilities::new(),
        &profile,
    )
    .unwrap();
    assert_eq!(first.render_id(), second.render_id());
    assert_ne!(
        first.evaluate(FrameKey::new(0)).unwrap().resources()[0].handle(),
        second.evaluate(FrameKey::new(0)).unwrap().resources()[0].handle()
    );
}

#[test]
fn metadata_authoring_ids_and_logical_resource_keys_do_not_change_render_identity() {
    let first_document = image_document("asset:first");
    let mut nonsemantic_document = image_document("asset:renamed");
    nonsemantic_document["document"]["metadata"] = json!({"review": "approved"});
    let mut author_renamed_document = nonsemantic_document.clone();
    author_renamed_document["document"]["visual"]["tracks"][0]["id"] = json!("track:renamed");
    author_renamed_document["document"]["visual"]["tracks"][0]["items"][0]["id"] =
        json!("clip:renamed");

    let first_manifest = manifest(json!({"asset:first": image_entry()}));
    let renamed_manifest = manifest(json!({"asset:renamed": image_entry()}));
    let profile = baseline_profile();
    let first = open(
        &timeline(&first_document),
        &first_manifest,
        &image_bindings(&first_manifest, "asset:first", 1),
        &Capabilities::new(),
        &profile,
    )
    .unwrap();
    let nonsemantic = open(
        &timeline(&nonsemantic_document),
        &renamed_manifest,
        &image_bindings(&renamed_manifest, "asset:renamed", 1),
        &Capabilities::new(),
        &profile,
    )
    .unwrap();
    let author_renamed = open(
        &timeline(&author_renamed_document),
        &renamed_manifest,
        &image_bindings(&renamed_manifest, "asset:renamed", 1),
        &Capabilities::new(),
        &profile,
    )
    .unwrap();

    assert_eq!(first.render_id(), nonsemantic.render_id());
    assert_eq!(nonsemantic.render_id(), author_renamed.render_id());
}

#[test]
fn resource_presence_and_kind_use_engine_diagnostics_while_digest_fails_package_verification() {
    let timeline = timeline(&image_document("asset:hero"));
    let profile = baseline_profile();

    let missing_manifest = manifest(json!({}));
    let report = open(
        &timeline,
        &missing_manifest,
        &ResourceBindings::new(),
        &Capabilities::new(),
        &profile,
    )
    .unwrap_err();
    assert_eq!(
        serde_json::to_string(report.diagnostics()).unwrap(),
        r#"[{"code":"missing_manifest_resource","path":"/document/visual/tracks/0/items/0/source/resource","phase":"resource-resolve","severity":"error","resourceId":"asset:hero","details":{}}]"#
    );
    assert!(report.contains(EngineOpenDiagnosticCode::MissingManifestResource));

    let image_manifest = manifest(json!({"asset:hero": image_entry()}));
    let report = open(
        &timeline,
        &image_manifest,
        &ResourceBindings::new(),
        &Capabilities::new(),
        &profile,
    )
    .unwrap_err();
    assert!(report.contains(EngineOpenDiagnosticCode::MissingResourceBinding));

    let wrong_kind = manifest(json!({"asset:hero": font_entry()}));
    let report = open(
        &timeline,
        &wrong_kind,
        &ResourceBindings::new()
            .with_binding("asset:hero", binding(&wrong_kind, "asset:hero", None, 1))
            .unwrap(),
        &Capabilities::new(),
        &profile,
    )
    .unwrap_err();
    assert!(report.contains(EngineOpenDiagnosticCode::ResourceKindMismatch));

    let error = open_package(
        &timeline,
        &image_manifest,
        &ResourceBindings::new()
            .with_binding(
                "asset:hero",
                binding(&image_manifest, "asset:hero", Some(MISMATCH_DIGEST), 1),
            )
            .unwrap(),
        &Capabilities::new(),
        &profile,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("[verified_binding_bundle] digest mismatch for \"asset:hero\"")
    );
}

#[test]
fn common_profile_rejects_mono_canvas_even_without_audio_tracks() {
    let mut value = document_with_source(
        json!({"type": "solid", "color": "#000000ff"}),
        json!([]),
        "track:main",
        "clip:solid",
        json!({}),
    );
    value["document"]["canvas"]["channelLayout"] = json!("mono");
    assert!(
        value["document"]["audio"]["tracks"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let report = open(
        &timeline(&value),
        &manifest(json!({})),
        &ResourceBindings::new(),
        &Capabilities::new(),
        &baseline_profile(),
    )
    .unwrap_err();
    let diagnostic = report
        .diagnostics()
        .iter()
        .find(|diagnostic| {
            diagnostic.code == EngineOpenDiagnosticCode::UnsupportedAudioChannelLayout
        })
        .expect("common must reject a mono canvas independently of audio tracks");
    assert_eq!(diagnostic.path, "/document/canvas/channelLayout");
    assert_eq!(diagnostic.phase, EngineOpenPhase::Admission);
    assert_eq!(
        diagnostic.severity,
        valle_timeline::internal::DiagnosticSeverity::Error
    );
    assert_eq!(
        diagnostic.details.get("reason").map(String::as_str),
        Some("common-canvas-layout")
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.phase != EngineOpenPhase::Compile)
    );
}

#[test]
fn verified_resource_facts_change_render_id_and_mismatches_fail() {
    let timeline = timeline(&image_document("asset:hero"));
    let manifest = manifest(json!({"asset:hero": image_entry()}));
    let profile = baseline_profile();
    let entry = manifest.entries().get("asset:hero").unwrap();
    let ResourceEntryWire::Image {
        digest: content_digest,
        descriptor,
    } = entry
    else {
        panic!("fixture must be an image")
    };

    let mut wrong_descriptor = descriptor.clone();
    wrong_descriptor.width += 1;
    let mismatched = ResourceBindings::new()
        .with_binding(
            "asset:hero",
            ResourceBinding::new(
                content_digest.clone(),
                VerifiedHandleId::new(1).unwrap(),
                VerifiedResourceFacts::Image {
                    descriptor: wrong_descriptor,
                    temporal_footprint: VisualFootprint::default(),
                },
            ),
        )
        .unwrap();
    let error = open_package(
        &timeline,
        &manifest,
        &mismatched,
        &Capabilities::new(),
        &profile,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("[verified_binding_bundle] facts mismatch for \"asset:hero\"")
    );

    let baseline = open(
        &timeline,
        &manifest,
        &image_bindings(&manifest, "asset:hero", 1),
        &Capabilities::new(),
        &profile,
    )
    .unwrap();
    let expanded = ResourceBindings::new()
        .with_binding(
            "asset:hero",
            ResourceBinding::new(
                content_digest.clone(),
                VerifiedHandleId::new(2).unwrap(),
                VerifiedResourceFacts::Image {
                    descriptor: descriptor.clone(),
                    temporal_footprint: VisualFootprint::new(2, 3),
                },
            ),
        )
        .unwrap();
    let expanded = open(
        &timeline,
        &manifest,
        &expanded,
        &Capabilities::new(),
        &profile,
    )
    .unwrap();
    assert_ne!(baseline.render_id(), expanded.render_id());
}

#[test]
fn positive_clip_durations_that_quantize_to_empty_ranges_are_rejected() {
    let mut visual_value = image_document("asset:hero");
    visual_value["document"]["visual"]["tracks"][0]["items"][0]["duration"] = json!("1/1000");
    let visual = timeline(&visual_value);
    let image_manifest = manifest(json!({"asset:hero": image_entry()}));
    let report = open(
        &visual,
        &image_manifest,
        &image_bindings(&image_manifest, "asset:hero", 1),
        &Capabilities::new(),
        &baseline_profile(),
    )
    .unwrap_err();
    assert!(report.contains(EngineOpenDiagnosticCode::PositiveDurationQuantizedToZeroFrame));
    assert_eq!(
        report.diagnostics()[0].path,
        "/document/visual/tracks/0/items/0/duration"
    );

    let audio = timeline(&audio_document("asset:voice", "1/100000"));
    let audio_manifest = manifest(json!({"asset:voice": audio_entry()}));
    let audio_bindings = ResourceBindings::new()
        .with_binding(
            "asset:voice",
            binding(&audio_manifest, "asset:voice", None, 2),
        )
        .unwrap();
    let report = open(
        &audio,
        &audio_manifest,
        &audio_bindings,
        &Capabilities::new(),
        &baseline_profile(),
    )
    .unwrap_err();
    assert!(report.contains(EngineOpenDiagnosticCode::PositiveDurationQuantizedToZeroSample));
    assert_eq!(
        report.diagnostics()[0].path,
        "/document/audio/tracks/0/items/0/duration"
    );

    let mut gap_value = image_document("asset:unused");
    gap_value["document"]["visual"]["tracks"][0]["items"][0] = json!({
        "type": "gap",
        "id": "gap:tiny",
        "duration": "1/1000"
    });
    let gap = timeline(&gap_value);
    open(
        &gap,
        &manifest(json!({})),
        &ResourceBindings::new(),
        &Capabilities::new(),
        &baseline_profile(),
    )
    .expect("a positive Gap may intentionally quantize to an empty execution range");
}

#[test]
fn extension_kernel_and_artifact_abi_are_admitted_before_compile() {
    let filtered = document_with_source(
        json!({
            "type": "image",
            "resource": "asset:hero",
            "sampling": {"fit": "contain"}
        }),
        json!([{
            "id": "filter:glow",
            "type": "example.visual/glow@1",
            "parameters": {"gain": 1.25}
        }]),
        "track:main",
        "clip:image",
        json!({}),
    );
    let canonical_timeline = timeline(&filtered);
    let image_manifest = manifest(json!({"asset:hero": image_entry()}));
    let profile = baseline_profile();
    let report = open(
        &canonical_timeline,
        &image_manifest,
        &image_bindings(&image_manifest, "asset:hero", 1),
        &Capabilities::new(),
        &profile,
    )
    .unwrap_err();
    assert!(report.contains(EngineOpenDiagnosticCode::UnsupportedExtensionKernel));

    let capabilities = Capabilities::new()
        .with_extension_kernel("example.visual/glow@1", known_kernel_capability())
        .unwrap();
    let admitted = open(
        &canonical_timeline,
        &image_manifest,
        &image_bindings(&image_manifest, "asset:hero", 1),
        &capabilities,
        &profile,
    )
    .unwrap();
    assert_eq!(admitted.admitted_kernel_count(), 1);

    let report = open(
        &canonical_timeline,
        &image_manifest,
        &image_bindings(&image_manifest, "asset:hero", 1),
        &Capabilities::new()
            .with_extension_kernel(
                "example.visual/glow@1",
                kernel_capability(OTHER_KERNEL_DIGEST),
            )
            .unwrap(),
        &profile,
    )
    .unwrap_err();
    assert!(report.contains(EngineOpenDiagnosticCode::UnsupportedExtensionKernel));

    let artifact = motion_artifact();
    let motion = timeline(&motion_document("component:title"));
    let motion_manifest = manifest(json!({
        "component:title": motion_entry(&artifact),
        "asset:logo": image_entry()
    }));
    let motion_bindings = ResourceBindings::new()
        .with_binding(
            "component:title",
            motion_binding(
                &motion_manifest,
                "component:title",
                Arc::clone(&artifact),
                2,
            ),
        )
        .unwrap()
        .with_binding(
            "asset:logo",
            binding(&motion_manifest, "asset:logo", None, 3),
        )
        .unwrap();
    let report = open(
        &motion,
        &motion_manifest,
        &motion_bindings,
        &Capabilities::new(),
        &profile,
    )
    .unwrap_err();
    assert!(report.contains(EngineOpenDiagnosticCode::UnsupportedArtifactAbi));

    let render = open(
        &motion,
        &motion_manifest,
        &motion_bindings,
        &Capabilities::new().with_artifact_abi("valle.motion/artifact@1"),
        &profile,
    )
    .unwrap();
    let first_address = render
        .motion_frame_address(render.render_id(), FrameKey::new(0), 0)
        .unwrap();
    assert_eq!(first_address.render_id(), render.render_id());
    assert_eq!(first_address.composition_frame(), FrameKey::new(0));
    assert_eq!(first_address.source_index(), 0);
    assert_eq!(first_address.source_frame(), 0);
    assert_eq!(
        render
            .motion_frame_address(render.render_id(), FrameKey::new(29), 0)
            .unwrap()
            .source_frame(),
        29
    );
    assert_eq!(
        render
            .motion_frame_address(admitted.render_id(), FrameKey::new(0), 0)
            .unwrap_err(),
        RuntimeFault::RenderMismatch
    );
    let compiled_motion = render.sources().source(0).unwrap().motion().unwrap();
    assert_eq!(compiled_motion.artifact(), artifact.as_ref());
    assert_ne!(
        compiled_motion.component_target(),
        compiled_motion.resources()["logo"]
    );
    let frame = render.evaluate(FrameKey::new(0)).unwrap();
    let EvaluatedVisualOperation::Clip(clip) = &frame.visual()[0] else {
        panic!("Motion fixture must evaluate as a visual clip")
    };
    assert_eq!(clip.clip_id(), "clip:motion");
    assert_eq!(clip.track_id(), "track:main");
    assert_eq!(
        clip.source().motion_props().unwrap()["opacity"],
        valle_engine::render::EvaluatedMotionValue::Scalar(0.75)
    );
    assert_eq!(
        clip.source().resource().unwrap().digest(),
        entry_digest(motion_manifest.entries().get("component:title").unwrap())
    );
    assert_eq!(clip.source().motion_resources()["logo"].handle().get(), 3);
    assert_eq!(clip.source().motion_artifact_dependencies().len(), 1);
    assert_eq!(
        clip.source().motion_artifact_dependencies()[0].digest(),
        entry_digest(motion_manifest.entries().get("asset:logo").unwrap())
    );

    let output = valle_engine::prepare::prepare_compiled_render_frame_cached(
        &render,
        &frame,
        &valle_engine::frame::RenderSpec::new(
            1920,
            1080,
            valle_engine::frame::RenderQuality::Preview,
            valle_engine::resource::OutputSpec::srgb_preview(
                valle_engine::resource::OutputBackground::opaque_srgb([0, 0, 0]),
            )
            .unwrap(),
        )
        .unwrap(),
        &mut valle_engine::prepare::ProductPrepareCaches::new(),
    )
    .unwrap();
    let valle_engine::prepare::PreparedVisualItem::Layer(layer) = &output.frame.visual[0] else {
        panic!("Motion fixture must prepare as a visual layer")
    };
    assert_eq!(layer.clip_id, "clip:motion");
    assert_eq!(layer.track_id, "track:main");
    assert_eq!(output.inspection.motion[0].clip_id, "clip:motion");
}

#[test]
fn motion_source_duration_is_resolved_and_validated_during_admission() {
    let artifact = motion_artifact();
    let motion_manifest = manifest(json!({
        "component:title": motion_entry(&artifact),
        "asset:logo": image_entry()
    }));
    let bindings = ResourceBindings::new()
        .with_binding(
            "component:title",
            motion_binding(
                &motion_manifest,
                "component:title",
                Arc::clone(&artifact),
                2,
            ),
        )
        .unwrap()
        .with_binding(
            "asset:logo",
            binding(&motion_manifest, "asset:logo", None, 3),
        )
        .unwrap();
    let capabilities = Capabilities::new().with_artifact_abi("valle.motion/artifact@1");

    let valid = motion_document("component:title");
    let render = open(
        &timeline(&valid),
        &motion_manifest,
        &bindings,
        &capabilities,
        &baseline_profile(),
    )
    .unwrap();
    assert_eq!(
        render
            .sources()
            .source(0)
            .unwrap()
            .motion()
            .unwrap()
            .artifact()
            .composition
            .as_ref()
            .unwrap()
            .duration()
            .unwrap()
            .to_string(),
        "1/1"
    );

    let mut zero_source_extent = motion_document("component:title");
    zero_source_extent["document"]["visual"]["tracks"][0]["items"][0]["source"]["sourceDuration"] =
        json!("1/1000");
    let report = open(
        &timeline(&zero_source_extent),
        &motion_manifest,
        &bindings,
        &capabilities,
        &baseline_profile(),
    )
    .unwrap_err();
    assert!(report.diagnostics().iter().any(|diagnostic| {
        diagnostic.code == EngineOpenDiagnosticCode::MotionTimingMismatch
            && diagnostic.details.get("reason").map(String::as_str)
                == Some("duration-quantized-to-zero-frames")
            && diagnostic.phase == EngineOpenPhase::Admission
    }));

    let mut mismatched = motion_document("component:title");
    mismatched["document"]["visual"]["tracks"][0]["items"][0]["source"]["sourceDuration"] =
        json!("2/1");
    let report = open(
        &timeline(&mismatched),
        &motion_manifest,
        &bindings,
        &capabilities,
        &baseline_profile(),
    )
    .unwrap_err();
    assert!(
        report.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == EngineOpenDiagnosticCode::MotionTimingMismatch
        })
    );
}

#[test]
fn hold_end_preserves_the_sentinel_and_uses_each_producers_abi_boundary() {
    let artifact = video_motion_artifact();
    let mut motion_value = motion_document("component:held-video");
    motion_value["document"]["canvas"]["fps"] = json!("3/1");
    motion_value["document"]["canvas"]["duration"] = json!("2/1");
    motion_value["document"]["visual"]["tracks"][0]["items"][0]["duration"] = json!("2/1");
    let motion_source = &mut motion_value["document"]["visual"]["tracks"][0]["items"][0]["source"];
    motion_source["resources"] = json!({"footage": "asset:footage"});
    motion_source["props"]["opacity"] = json!({
        "type": "curve",
        "id": "curve:opacity",
        "interpolation": "step",
        "keyframes": [
            {"id": "key:opacity:before", "time": "0/1", "value": 0.25, "outEasing": null},
            {"id": "key:opacity:left", "time": "5/6", "value": 0.5, "outEasing": null},
            {"id": "key:opacity:at-end", "time": "1/1", "value": 0.75, "outEasing": null}
        ],
        "extrapolation": "clamp"
    });
    motion_source["props"]["linear"] = json!({
        "type": "curve",
        "id": "curve:linear",
        "interpolation": "linear",
        "keyframes": [
            {"id": "key:linear:start", "time": "0/1", "value": 0.0, "outEasing": null},
            {"id": "key:linear:end", "time": "1/1", "value": 1.0, "outEasing": null}
        ],
        "extrapolation": "clamp"
    });
    let motion_manifest = manifest(json!({
        "component:held-video": motion_entry(&artifact),
        "asset:footage": video_entry("1/1")
    }));
    let motion_bindings = ResourceBindings::new()
        .with_binding(
            "component:held-video",
            custom_motion_binding(
                &motion_manifest,
                "component:held-video",
                Arc::clone(&artifact),
                &[("footage", "asset:footage")],
                41,
            ),
        )
        .unwrap()
        .with_binding(
            "asset:footage",
            binding(&motion_manifest, "asset:footage", None, 42),
        )
        .unwrap();
    let motion_render = open(
        &timeline(&motion_value),
        &motion_manifest,
        &motion_bindings,
        &Capabilities::new().with_artifact_abi("valle.motion/artifact@1"),
        &baseline_profile(),
    )
    .unwrap();

    let held_frame = FrameKey::new(3);
    let motion_frame = motion_render.evaluate(held_frame).unwrap();
    let EvaluatedVisualOperation::Clip(motion_clip) = &motion_frame.visual()[0] else {
        panic!("held Motion source must remain a visual clip")
    };
    assert_eq!(
        motion_clip.source().mapped_time(),
        valle_engine::render::MappedSourceTime::HoldEnd
    );
    let motion_frame_boundary = valle_timeline::RationalTime::new(2, 3).unwrap();
    assert_eq!(motion_clip.source().sample_time(), motion_frame_boundary);
    assert_eq!(
        motion_clip.source().motion_props().unwrap()["opacity"],
        valle_engine::render::EvaluatedMotionValue::Scalar(0.75),
        "holding the source must not hold a host-local prop curve"
    );
    assert_eq!(
        motion_clip.source().motion_props().unwrap()["linear"],
        valle_engine::render::EvaluatedMotionValue::Scalar(1.0),
        "the host-local curve clamps after its last keyframe"
    );
    assert_eq!(
        motion_render
            .motion_frame_address(motion_render.render_id(), held_frame, 0)
            .unwrap()
            .source_frame(),
        2
    );
    let motion_output = valle_engine::prepare::prepare_compiled_render_frame_cached(
        &motion_render,
        &motion_frame,
        &valle_engine::frame::RenderSpec::new(
            1920,
            1080,
            valle_engine::frame::RenderQuality::Preview,
            valle_engine::resource::OutputSpec::srgb_preview(
                valle_engine::resource::OutputBackground::opaque_srgb([0, 0, 0]),
            )
            .unwrap(),
        )
        .unwrap(),
        &mut valle_engine::prepare::ProductPrepareCaches::new(),
    )
    .unwrap();
    let embedded_video_time = valle_timeline::RationalTime::new(3, 4).unwrap();
    let second_embedded_video_time = valle_timeline::RationalTime::new(1, 2).unwrap();
    assert!(
        motion_output
            .resource_requests
            .requests()
            .iter()
            .any(|request| {
                request.sample()
                    == valle_engine::resource::ResourceSample::SourceTime(embedded_video_time)
            })
    );
    assert!(
        motion_output
            .resource_requests
            .requests()
            .iter()
            .any(|request| {
                request.sample()
                    == valle_engine::resource::ResourceSample::SourceTime(
                        second_embedded_video_time,
                    )
            })
    );
    assert!(
        motion_output
            .resource_requests
            .requests()
            .iter()
            .all(|request| {
                request.sample()
                    != valle_engine::resource::ResourceSample::SourceTime(motion_frame_boundary)
            })
    );
    assert!(
        motion_output
            .resource_requests
            .requests()
            .iter()
            .all(|request| {
                request.sample()
                    != valle_engine::resource::ResourceSample::SourceTime(
                        valle_timeline::RationalTime::ONE,
                    )
            })
    );
    let motion_program_index = motion_output
        .frame
        .programs
        .iter()
        .position(|program| program.kind == valle_engine::prepare::PreparedProgramKind::Motion)
        .unwrap();
    let motion_program = &motion_output.frame.programs[motion_program_index];
    assert_eq!(motion_program.resources.textures.len(), 2);
    assert_ne!(
        motion_program.resources.textures[0].handle, motion_program.resources.textures[1].handle,
        "the same asset at two producer-local samples needs two execution bindings"
    );
    assert_eq!(
        motion_program
            .requirements
            .external_textures
            .iter()
            .map(|texture| texture.sample_time_micros)
            .collect::<Vec<_>>(),
        vec![Some(500_000), Some(750_000)]
    );
    let mut tampered = motion_output.clone();
    let second_handle = tampered.frame.programs[motion_program_index]
        .resources
        .textures[1]
        .handle;
    tampered.frame.programs[motion_program_index]
        .resources
        .textures[0]
        .handle = second_handle;
    assert!(
        serde_json::from_value::<valle_engine::prepare::PrepareOutput>(
            serde_json::to_value(tampered).unwrap()
        )
        .is_err(),
        "PreparedFrame admission must compare the exact producer sample, not any SourceTime"
    );

    let graph = valle_engine::compositor::graph::build_render_graph(&motion_output.frame).unwrap();
    let backend = BackendCapabilities::new(
        Extent2d::new(4096, 4096).unwrap(),
        [TextureFormat::Rgba16Float, TextureFormat::Rgba32Float],
        [
            TextureUsage::Sampled,
            TextureUsage::StorageRead,
            TextureUsage::StorageWrite,
            TextureUsage::ColorAttachment,
            TextureUsage::CopySource,
            TextureUsage::CopyDestination,
        ],
        [1, 2, 4, 8],
        [
            ExternalPixelLayout::Rgba8,
            ExternalPixelLayout::Rgba16Float,
            ExternalPixelLayout::Nv12,
            ExternalPixelLayout::P010,
            ExternalPixelLayout::I420,
        ],
        [
            GraphCapability::Clear,
            GraphCapability::ExternalImport,
            GraphCapability::SourcePipeline,
            GraphCapability::DrawProgram,
            GraphCapability::BackdropRead,
            GraphCapability::Group,
            GraphCapability::Filter,
            GraphCapability::Mask,
            GraphCapability::Blend,
            GraphCapability::Transition,
            GraphCapability::AdjustmentEffect,
            GraphCapability::Caption,
            GraphCapability::OutputTransform,
        ],
        true,
        Some(FramebufferFetchSemantics::CoherentWorkingPremultiplied),
        512 * 1024 * 1024,
        1024 * 1024 * 1024,
    )
    .unwrap();
    let lowered = valle_engine::compositor::lower::lower_render_graph(
        &graph,
        &motion_output.dynamic,
        &backend,
    )
    .unwrap();
    let lowered_motion = lowered
        .programs()
        .iter()
        .find(|program| program.kind == valle_engine::prepare::PreparedProgramKind::Motion)
        .unwrap();
    assert_eq!(lowered_motion.requirements.external_textures.len(), 2);
    assert_eq!(lowered_motion.resources.textures.len(), 2);
    assert_eq!(
        lowered_motion
            .requirements
            .external_textures
            .iter()
            .map(|texture| (&texture.key, texture.sample_time_micros))
            .collect::<Vec<_>>(),
        vec![
            (&lowered_motion.resources.textures[0].key, Some(500_000)),
            (&lowered_motion.resources.textures[1].key, Some(750_000)),
        ],
        "lowering must preserve the positional full texture identity"
    );
    assert_ne!(
        lowered_motion.resources.textures[0].slot, lowered_motion.resources.textures[1].slot,
        "one logical asset at two samples needs two plan slots"
    );
    let lowered_roundtrip =
        RenderPlanTemplate::from_packed(&lowered.packed_bytes().unwrap()).unwrap();
    let roundtrip_motion = lowered_roundtrip
        .program_layouts()
        .iter()
        .find(|program| program.kind == valle_engine::prepare::PreparedProgramKind::Motion)
        .unwrap();
    assert_eq!(roundtrip_motion.resources.textures.len(), 2);
    assert_eq!(
        roundtrip_motion.resources.textures[0].key,
        roundtrip_motion.resources.textures[1].key
    );
    assert_ne!(
        roundtrip_motion.resources.textures[0].slot, roundtrip_motion.resources.textures[1].slot,
        "template wire roundtrip must retain both same-key texture slots"
    );
}

#[test]
fn motion_artifact_payload_is_fail_closed() {
    let artifact = motion_artifact();
    let timeline = timeline(&motion_document("component:title"));
    let manifest = manifest(json!({
        "component:title": motion_entry(&artifact),
        "asset:logo": image_entry()
    }));
    let entry = manifest.entries().get("component:title").unwrap();
    let ResourceEntryWire::MotionArtifact {
        abi,
        descriptor,
        digest,
    } = entry
    else {
        panic!("fixture must be a Motion artifact")
    };
    let image_binding = || binding(&manifest, "asset:logo", None, 3);
    let capabilities = Capabilities::new().with_artifact_abi("valle.motion/artifact@1");

    let mut tampered = artifact.as_ref().clone();
    tampered.component.push_str("Tampered");
    let tampered_binding = ResourceBinding::new(
        digest.clone(),
        VerifiedHandleId::new(2).unwrap(),
        VerifiedResourceFacts::MotionArtifact {
            abi: *abi,
            descriptor: descriptor.clone(),
            artifact: Arc::new(tampered),
            temporal_footprint: VisualFootprint::new(2, 1),
        },
    )
    .with_dependency(ResourceDependency::new("logo", "asset:logo").unwrap());
    let bindings = ResourceBindings::new()
        .with_binding("component:title", tampered_binding)
        .unwrap()
        .with_binding("asset:logo", image_binding())
        .unwrap();
    let report = open(
        &timeline,
        &manifest,
        &bindings,
        &capabilities,
        &baseline_profile(),
    )
    .unwrap_err();
    assert!(report.contains(EngineOpenDiagnosticCode::MotionArtifactPayloadMismatch));
}

#[test]
fn verified_transitive_dependencies_enter_render_id() {
    let timeline = timeline(&image_document("asset:hero"));
    let manifest = manifest(json!({
        "asset:hero": image_entry(),
        "font:sidecar": font_entry()
    }));
    let dependency = ResourceDependency::new("decoder-sidecar/font", "font:sidecar").unwrap();
    let bindings = ResourceBindings::new()
        .with_binding(
            "asset:hero",
            binding(&manifest, "asset:hero", None, 2).with_dependency(dependency),
        )
        .unwrap()
        .with_binding("font:sidecar", binding(&manifest, "font:sidecar", None, 3))
        .unwrap();
    let capabilities = Capabilities::new();
    let profile = baseline_profile();

    let render = open(&timeline, &manifest, &bindings, &capabilities, &profile).unwrap();
    assert_eq!(render.resources().resource_count(), 2);

    let missing_dependency = ResourceBindings::new()
        .with_binding(
            "asset:hero",
            binding(&manifest, "asset:hero", None, 2).with_dependency(
                ResourceDependency::new("decoder-sidecar/font", "font:sidecar").unwrap(),
            ),
        )
        .unwrap();
    let error = open_package(
        &timeline,
        &manifest,
        &missing_dependency,
        &capabilities,
        &profile,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("dependency \"font:sidecar\" for \"asset:hero\" has no binding")
    );

    let without_dependency = ResourceBindings::new()
        .with_binding("asset:hero", binding(&manifest, "asset:hero", None, 2))
        .unwrap();
    let no_dependency_render = open(
        &timeline,
        &manifest,
        &without_dependency,
        &capabilities,
        &profile,
    )
    .unwrap();
    assert_ne!(render.render_id(), no_dependency_render.render_id());
}

#[test]
fn visual_schedule_uses_cumulative_boundaries_and_preserves_band_order() {
    let mut value = transition_document("1/4");
    value["document"]["canvas"]["fps"] = json!("30/1");
    value["document"]["canvas"]["duration"] = json!("1/1");
    value["document"]["visual"]["tracks"] = json!([
        {
            "id": "track:bottom",
            "items": [
                solid_clip("clip:a", "1/30", "#ff0000ff"),
                {"type": "gap", "id": "gap:a", "duration": "1/60"},
                solid_clip("clip:b", "1/30", "#00ff00ff"),
                {"type": "gap", "id": "gap:b", "duration": "1/60"}
            ]
        },
        {
            "id": "track:top",
            "items": [solid_clip("clip:top", "1/1", "#0000ffff")]
        }
    ]);
    value["document"]["adjustments"] = json!([{
        "id": "global:warm",
        "start": "0/1",
        "duration": "1/1",
        "effect": {"type": "color-grade", "temperature": 0.5}
    }]);
    value["document"]["captions"]["tracks"] = json!([{
        "id": "caption:main",
        "items": [{
            "type": "clip",
            "id": "caption:hello",
            "duration": "1/1",
            "runs": [{
                "id": "run:hello",
                "text": "hello",
                "timing": null,
                "style": null
            }],
            "style": {"font": "font:main", "fontSize": 24.0, "color": "#ffffffff", "shadow": null},
            "layout": {"region": [0.0, 0.0, 1.0, 1.0], "align": "bottom-center"},
            "presentation": {
                "opacity": {"type": "constant", "value": 1.0},
                "translation": {"type": "constant", "value": [0.0, 0.0]},
                "scale": {"type": "constant", "value": 1.0},
                "rotation": {"type": "constant", "value": 0.0},
                "clipInset": {"type": "constant", "value": [0.0, 0.0, 0.0, 0.0]},
                "blurSigma": {"type": "constant", "value": 0.0}
            },
            "behavior": null
        }]
    }]);
    let timeline = timeline(&value);
    let manifest = manifest(json!({"font:main": font_entry()}));
    let bindings = ResourceBindings::new()
        .with_binding("font:main", binding(&manifest, "font:main", None, 9))
        .unwrap();
    let opened = open(
        &timeline,
        &manifest,
        &bindings,
        &Capabilities::new(),
        &baseline_profile(),
    );

    #[cfg(not(feature = "text"))]
    {
        let report = opened.unwrap_err();
        assert!(report.contains(EngineOpenDiagnosticCode::UnsupportedCaptionBackend));
    }

    #[cfg(feature = "text")]
    {
        let render = opened.unwrap();
        let compiled_visual = serde_json::to_value(render.visual()).unwrap();
        assert!(compiled_visual["tracks"][0].get("id").is_none());
        assert!(
            compiled_visual["tracks"][0]["items"][0]["clip"]
                .get("id")
                .is_none()
        );
        assert_eq!(render.visual().tracks()[0].id(), "track:bottom");
        let CompiledVisualItem::Clip { clip } = &render.visual().tracks()[0].items()[0] else {
            panic!("first visual item must be a clip")
        };
        assert_eq!(clip.id(), "clip:a");
        let ranges: Vec<_> = render.visual().tracks()[0]
            .items()
            .iter()
            .map(CompiledVisualItem::frame_range)
            .collect();
        assert_eq!((ranges[0].start(), ranges[0].end()), (0, 1));
        assert_eq!((ranges[1].start(), ranges[1].end()), (1, 2));
        assert_eq!((ranges[2].start(), ranges[2].end()), (2, 3));
        assert_eq!((ranges[3].start(), ranges[3].end()), (3, 3));

        let frame = render.evaluate(FrameKey::new(0)).unwrap();
        assert_eq!(frame.visual().len(), 2);
        assert_eq!(frame.visual()[0].track_order(), 0);
        assert_eq!(frame.visual()[1].track_order(), 1);
        let EvaluatedVisualOperation::Clip(bottom) = &frame.visual()[0] else {
            panic!("bottom track must evaluate a clip")
        };
        assert_eq!(bottom.clip_id(), "clip:a");
        assert_eq!(bottom.track_id(), "track:bottom");
        assert_eq!(frame.adjustments().len(), 1);
        assert_eq!(frame.captions().len(), 1);
        let valle_engine::render::EvaluatedCaption::Text(caption) = &frame.captions()[0] else {
            panic!("expected text caption")
        };
        assert_eq!(caption.track_order(), 0);
        assert_eq!(caption.runs(), &["hello".to_owned()]);
    }
}

#[cfg(feature = "text")]
#[test]
fn caption_rich_styles_and_presentation_use_caption_local_clock() {
    let mut value = image_document("asset:hero");
    value["document"]["canvas"]["fps"] = json!("10/1");
    value["document"]["captions"]["tracks"] = json!([{
        "id": "caption:main",
        "items": [
            {"type": "gap", "id": "gap:caption", "duration": "1/20"},
            {
                "type": "clip",
                "id": "caption:rich",
                "duration": "19/20",
                "runs": [
                    {"id": "run:base", "text": "hello ", "timing": null, "style": null},
                    {"id": "run:accent", "text": "world", "style": {
                        "fontSize": 48.0,
                        "color": "#fcfca2ff"
                    }, "timing": null}
                ],
                "style": {
                    "font": "font:main",
                    "fontSize": 24.0,
                    "color": "#ffffffff",
                    "shadow": {"color": "#000000e6", "offset": [2.0, 2.0], "blurSigma": 5.0}
                },
                "layout": {"region": [0.0, 0.0, 1.0, 1.0], "align": "bottom-center"},
                "presentation": {
                    "opacity": {
                        "type": "curve",
                        "id": "curve:caption:opacity",
                        "interpolation": "linear",
                        "keyframes": [
                            {"id": "keyframe:caption:opacity:0", "time": "0/1", "value": 0.0, "outEasing": null},
                            {"id": "keyframe:caption:opacity:1", "time": "1/10", "value": 1.0, "outEasing": null}
                        ],
                        "extrapolation": "clamp"
                    },
                    "translation": {"type": "constant", "value": [10.0, 20.0]},
                    "scale": {"type": "constant", "value": 1.25},
                    "rotation": {"type": "constant", "value": 15.0},
                    "clipInset": {"type": "constant", "value": [0.1, 0.2, 0.3, 0.0]},
                    "blurSigma": {"type": "constant", "value": 2.0}
                },
                "behavior": {"type": "scroll", "axis": "horizontal", "speed": 1.0}
            }
        ]
    }]);
    let manifest = manifest(json!({
        "asset:hero": image_entry(),
        "font:main": font_entry()
    }));
    let bindings = ResourceBindings::new()
        .with_binding("asset:hero", binding(&manifest, "asset:hero", None, 1))
        .unwrap()
        .with_binding("font:main", binding(&manifest, "font:main", None, 2))
        .unwrap();
    let render = open(
        &timeline(&value),
        &manifest,
        &bindings,
        &Capabilities::new(),
        &baseline_profile(),
    )
    .unwrap();

    assert!(
        render
            .evaluate(FrameKey::new(0))
            .unwrap()
            .captions()
            .is_empty()
    );
    // The exact authored start is 1/20s, halfway between frame-left samples at
    // 10fps. Admission assigns the caption to frame 1; its local clock must
    // still begin at exactly zero rather than composition 1/10 - exact 1/20.
    let at_start = render.evaluate(FrameKey::new(1)).unwrap();
    let valle_engine::render::EvaluatedCaption::Text(caption) = &at_start.captions()[0] else {
        panic!("expected text caption")
    };
    let resolved_face_weight = ttf_parser::Face::parse(FONT_BYTES, 0)
        .unwrap()
        .weight()
        .to_number();
    assert_eq!(caption.local_time(), valle_timeline::RationalTime::ZERO);
    assert_eq!(caption.run_styles()[0].font_size(), 24.0);
    assert_eq!(caption.run_styles()[0].font_weight(), resolved_face_weight);
    assert_eq!(caption.run_styles()[1].font_size(), 48.0);
    assert_eq!(caption.run_styles()[1].color(), "#fcfca2ff");
    assert_eq!(caption.run_styles()[1].font_weight(), resolved_face_weight);
    assert_eq!(caption.shadow().unwrap().blur_sigma(), 5.0);
    assert_eq!(caption.presentation().opacity(), 0.0);
    assert_eq!(caption.presentation().translation(), [10.0, 20.0]);
    assert_eq!(caption.presentation().scale(), 1.25);
    assert_eq!(caption.presentation().rotation(), 15.0);
    assert_eq!(caption.presentation().clip_inset(), [0.1, 0.2, 0.3, 0.0]);
    assert_eq!(caption.presentation().blur_sigma(), 2.0);

    let next_frame = render.evaluate(FrameKey::new(2)).unwrap();
    let valle_engine::render::EvaluatedCaption::Text(next_caption) = &next_frame.captions()[0]
    else {
        panic!("expected text caption")
    };
    assert_eq!(
        next_caption.local_time(),
        valle_timeline::RationalTime::new(1, 10).unwrap()
    );
    assert!((next_caption.presentation().opacity() - 1.0).abs() < 1e-12);

    // Preparing Scroll exercises the same EvaluatedCaption::local_time value;
    // Prepare must not reconstruct a second clock from the exact sequence start.
    let prepared = valle_engine::prepare::prepare_compiled_render_frame_cached(
        &render,
        &at_start,
        &valle_engine::frame::RenderSpec::new(
            1920,
            1080,
            valle_engine::frame::RenderQuality::Preview,
            valle_engine::resource::OutputSpec::srgb_preview(
                valle_engine::resource::OutputBackground::opaque_srgb([0, 0, 0]),
            )
            .unwrap(),
        )
        .unwrap(),
        &mut valle_engine::prepare::ProductPrepareCaches::new(),
    )
    .unwrap();
    assert_eq!(prepared.frame.captions.len(), 1);

    let mut aligned_value = value.clone();
    let mut aligned_caption =
        aligned_value["document"]["captions"]["tracks"][0]["items"][1].clone();
    aligned_caption["duration"] = json!("1/1");
    aligned_value["document"]["captions"]["tracks"][0]["items"] = json!([aligned_caption]);
    let aligned_render = open(
        &timeline(&aligned_value),
        &manifest,
        &bindings,
        &Capabilities::new(),
        &baseline_profile(),
    )
    .unwrap();
    let aligned_start = aligned_render.evaluate(FrameKey::new(0)).unwrap();
    let aligned_prepared = valle_engine::prepare::prepare_compiled_render_frame_cached(
        &aligned_render,
        &aligned_start,
        &valle_engine::frame::RenderSpec::new(
            1920,
            1080,
            valle_engine::frame::RenderQuality::Preview,
            valle_engine::resource::OutputSpec::srgb_preview(
                valle_engine::resource::OutputBackground::opaque_srgb([0, 0, 0]),
            )
            .unwrap(),
        )
        .unwrap(),
        &mut valle_engine::prepare::ProductPrepareCaches::new(),
    )
    .unwrap();
    let valle_engine::prepare::PreparedCaption::Text(fractional) = &prepared.frame.captions[0]
    else {
        panic!("expected text caption")
    };
    let valle_engine::prepare::PreparedCaption::Text(aligned) = &aligned_prepared.frame.captions[0]
    else {
        panic!("expected text caption")
    };
    let fractional_program = fractional.program.get() as usize;
    let aligned_program = aligned.program.get() as usize;
    assert_eq!(
        prepared.frame.programs[fractional_program - 1].packed,
        aligned_prepared.frame.programs[aligned_program - 1].packed
    );
}

#[test]
fn transition_windows_freeze_endpoint_progress_for_n1_and_n_greater_than_one() {
    let empty_manifest = manifest(json!({}));
    let profile = baseline_profile();
    let n3 = open(
        &timeline(&transition_document("3/4")),
        &empty_manifest,
        &ResourceBindings::new(),
        &Capabilities::new(),
        &profile,
    )
    .unwrap();
    let CompiledVisualItem::Transition { transition } = &n3.visual().tracks()[0].items()[1] else {
        panic!("middle item must be a transition")
    };
    assert_eq!(
        (transition.window().start(), transition.window().end()),
        (1, 4)
    );
    assert_eq!(transition.progress_at(1).unwrap().as_f64(), 0.0);
    assert_eq!(transition.progress_at(2).unwrap().as_f64(), 0.5);
    assert_eq!(transition.progress_at(3).unwrap().as_f64(), 1.0);
    for (frame, expected) in [(1, 0.0), (2, 0.5), (3, 1.0)] {
        let evaluated = n3.evaluate(FrameKey::new(frame)).unwrap();
        let EvaluatedVisualOperation::Transition(active) = &evaluated.visual()[0] else {
            panic!("transition must replace endpoint clips inside its window")
        };
        assert_eq!(active.track_id(), "track:main");
        assert_eq!(active.from_clip_id(), "clip:left");
        assert_eq!(active.to_clip_id(), "clip:right");
        assert_eq!(active.progress().as_f64(), expected);
        assert_eq!(active.from_weight(), 1.0 - expected);
        assert_eq!(active.to_weight(), expected);
    }

    let n1 = open(
        &timeline(&transition_document("1/4")),
        &empty_manifest,
        &ResourceBindings::new(),
        &Capabilities::new(),
        &profile,
    )
    .unwrap();
    let CompiledVisualItem::Transition { transition } = &n1.visual().tracks()[0].items()[1] else {
        panic!("middle item must be a transition")
    };
    assert_eq!(
        (transition.window().start(), transition.window().end()),
        (3, 4)
    );
    assert_eq!(transition.progress_at(3).unwrap().as_f64(), 0.5);
}

#[test]
fn camera_curves_sample_on_frame_left_boundaries() {
    let mut value = transition_document("1/4");
    value["document"]["visual"]["tracks"] = json!([]);
    value["document"]["canvas"]["duration"] = json!("1/1");
    value["document"]["camera"] = json!({
        "centerX": {
            "type": "curve",
            "id": "curve:camera-x",
            "interpolation": "linear",
            "keyframes": [
                {"id": "key:camera-x-0", "time": "0/1", "value": 0.0, "outEasing": "linear"},
                {"id": "key:camera-x-1", "time": "1/2", "value": 1.0, "outEasing": null}
            ],
            "extrapolation": "clamp"
        },
        "centerY": constant(json!(0.0)),
        "zoom": constant(json!(1.0)),
        "rotation": constant(json!(0.0))
    });
    let render = open(
        &timeline(&value),
        &manifest(json!({})),
        &ResourceBindings::new(),
        &Capabilities::new().with_camera(),
        &baseline_profile(),
    )
    .unwrap();
    assert_eq!(
        render
            .evaluate(FrameKey::new(0))
            .unwrap()
            .camera()
            .unwrap()
            .center_x(),
        0.0
    );
    assert_eq!(
        render
            .evaluate(FrameKey::new(1))
            .unwrap()
            .camera()
            .unwrap()
            .center_x(),
        0.5
    );
    assert_eq!(
        render
            .evaluate(FrameKey::new(2))
            .unwrap()
            .camera()
            .unwrap()
            .center_x(),
        1.0
    );
}

#[test]
fn audio_crossfade_uses_frozen_linear_gain_law_and_separate_sample_clock() {
    let stereo_manifest = manifest(json!({"audio:music": audio_entry_at(4, "2/1")}));
    let bindings = ResourceBindings::new()
        .with_binding(
            "audio:music",
            binding(&stereo_manifest, "audio:music", None, 11),
        )
        .unwrap();
    let profile = baseline_profile();
    let n3 = open(
        &timeline(&audio_crossfade_document("3/4")),
        &stereo_manifest,
        &bindings,
        &Capabilities::new(),
        &profile,
    )
    .unwrap();
    let CompiledAudioItem::Crossfade { crossfade } = &n3.audio().tracks()[0].items()[1] else {
        panic!("middle audio item must be a crossfade")
    };
    assert_eq!(
        (crossfade.window().start(), crossfade.window().end()),
        (3, 6)
    );
    for (sample, left, right, from_source_sample, to_source_sample) in [
        (3, 1.0, 0.0, 3, 0),
        (4, 0.5, 0.5, 4, 0),
        (5, 0.0, 1.0, 5, 1),
    ] {
        let evaluated = n3.audio().evaluate_sample(sample).unwrap();
        let endpoints = evaluated.tracks()[0].endpoints();
        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].crossfade_gain(), left);
        assert_eq!(endpoints[1].crossfade_gain(), right);
        assert_eq!(endpoints[0].left_gain(), left);
        assert_eq!(endpoints[1].right_gain(), right);
        assert_eq!(endpoints[0].source_sample_index(), from_source_sample);
        assert_eq!(endpoints[1].source_sample_index(), to_source_sample);
    }
    assert!(
        n3.evaluate(FrameKey::new(1))
            .unwrap()
            .resources()
            .is_empty()
    );

    let n1 = open(
        &timeline(&audio_crossfade_document("1/4")),
        &stereo_manifest,
        &bindings,
        &Capabilities::new(),
        &profile,
    )
    .unwrap();
    let evaluated = n1.audio().evaluate_sample(4).unwrap();
    assert_eq!(evaluated.tracks()[0].endpoints()[0].crossfade_gain(), 0.5);
    assert_eq!(evaluated.tracks()[0].endpoints()[1].crossfade_gain(), 0.5);

    let mut mono_entry = audio_entry_at(4, "2/1");
    mono_entry["descriptor"]["channelLayout"] = json!("mono");
    let mono_manifest = manifest(json!({"audio:music": mono_entry}));
    let mono_bindings = ResourceBindings::new()
        .with_binding(
            "audio:music",
            binding(&mono_manifest, "audio:music", None, 12),
        )
        .unwrap();
    let mono = open(
        &timeline(&audio_crossfade_document("1/4")),
        &mono_manifest,
        &mono_bindings,
        &Capabilities::new(),
        &profile,
    )
    .unwrap();
    assert_eq!(
        mono.sources().source(0).unwrap().audio_channel_map(),
        Some(CompiledAudioChannelMap::MonoToStereo)
    );
}

#[test]
fn audio_gain_effect_is_frozen_before_gain_pan_and_crossfade_and_is_seek_chunk_invariant() {
    let mut value = audio_crossfade_document("3/4");
    let left = &mut value["document"]["audio"]["tracks"][0]["items"][0];
    left["gain"] = constant(json!(0.7));
    left["pan"] = constant(json!(-0.25));
    left["effects"] = json!([
        {
            "id": "audio-effect:left-a",
            "type": AUDIO_GAIN_EFFECT_KIND,
            "parameters": {"multiplier": 0.1}
        },
        {
            "id": "audio-effect:left-b",
            "type": AUDIO_GAIN_EFFECT_KIND,
            "parameters": {"multiplier": 0.2}
        },
        {
            "id": "audio-effect:left-c",
            "type": AUDIO_GAIN_EFFECT_KIND,
            "parameters": {"multiplier": 0.3}
        }
    ]);
    let right = &mut value["document"]["audio"]["tracks"][0]["items"][2];
    right["gain"] = constant(json!(0.6));
    right["pan"] = constant(json!(0.4));
    right["effects"] = json!([{
        "id": "audio-effect:right",
        "type": AUDIO_GAIN_EFFECT_KIND,
        "parameters": {"multiplier": 0.3}
    }]);

    let stereo_manifest = manifest(json!({"audio:music": audio_entry_at(4, "2/1")}));
    let bindings = ResourceBindings::new()
        .with_binding(
            "audio:music",
            binding(&stereo_manifest, "audio:music", None, 11),
        )
        .unwrap();
    let capabilities = audio_gain_capabilities();
    let render = open(
        &timeline(&value),
        &stereo_manifest,
        &bindings,
        &capabilities,
        &baseline_profile(),
    )
    .unwrap();

    let evaluated = render.audio().evaluate_sample(4).unwrap();
    let endpoints = evaluated.tracks()[0].endpoints();
    assert_eq!(endpoints.len(), 2);
    assert_eq!(
        endpoints[0].effect_multiplier().to_bits(),
        0x3f78_9374_bc6a_7efb
    );
    assert_eq!(endpoints[0].gain(), 0.7);
    assert_eq!(endpoints[0].pan(), -0.25);
    assert_eq!(endpoints[0].crossfade_gain(), 0.5);
    assert_eq!(endpoints[0].left_gain().to_bits(), 0x3f61_3404_ea4a_8c16);
    assert_eq!(endpoints[0].right_gain().to_bits(), 0x3f59_ce07_5f6f_d221);
    assert_eq!(endpoints[1].effect_multiplier(), 0.3);
    assert_eq!(endpoints[1].gain(), 0.6);
    assert_eq!(endpoints[1].pan(), 0.4);
    assert_eq!(endpoints[1].crossfade_gain(), 0.5);
    assert_eq!(endpoints[1].left_gain().to_bits(), 0x3fab_a5e3_53f7_ced9);
    assert_eq!(endpoints[1].right_gain().to_bits(), 0x3fb7_0a3d_70a3_d70a);

    let whole = render
        .audio()
        .evaluate_block(SampleRange::new(0, 8).unwrap())
        .unwrap();
    let first = render
        .audio()
        .evaluate_block(SampleRange::new(0, 3).unwrap())
        .unwrap();
    let middle = render
        .audio()
        .evaluate_block(SampleRange::new(3, 5).unwrap())
        .unwrap();
    let last = render
        .audio()
        .evaluate_block(SampleRange::new(5, 8).unwrap())
        .unwrap();
    assert_eq!(&whole.samples()[0..3], first.samples());
    assert_eq!(&whole.samples()[3..5], middle.samples());
    assert_eq!(&whole.samples()[5..8], last.samples());
    for sample in whole.samples() {
        assert_eq!(
            sample,
            &render.audio().evaluate_sample(sample.sample()).unwrap()
        );
    }

    let mut changed_value = value.clone();
    changed_value["document"]["audio"]["tracks"][0]["items"][0]["effects"][1]["parameters"]["multiplier"] =
        json!(0.25);
    let changed = open(
        &timeline(&changed_value),
        &stereo_manifest,
        &bindings,
        &capabilities,
        &baseline_profile(),
    )
    .unwrap();
    assert_ne!(render.render_id(), changed.render_id());

    let mut reordered_value = value.clone();
    reordered_value["document"]["audio"]["tracks"][0]["items"][0]["effects"]
        .as_array_mut()
        .unwrap()
        .swap(1, 2);
    let reordered = open(
        &timeline(&reordered_value),
        &stereo_manifest,
        &bindings,
        &capabilities,
        &baseline_profile(),
    )
    .unwrap();
    assert_eq!(
        reordered.audio().evaluate_sample(4).unwrap().tracks()[0].endpoints()[0]
            .effect_multiplier()
            .to_bits(),
        0x3f78_9374_bc6a_7efa
    );
    assert_ne!(render.render_id(), reordered.render_id());
}

#[test]
fn audio_effect_admission_is_closed() {
    let mut value = audio_crossfade_document("1/4");
    value["document"]["audio"]["tracks"][0]["items"][0]["effects"] = json!([{
        "id": "audio-effect:unbound-implementation",
        "type": AUDIO_GAIN_EFFECT_KIND,
        "parameters": {"multiplier": 1.0}
    }]);
    let stereo_manifest = manifest(json!({"audio:music": audio_entry_at(4, "2/1")}));
    let bindings = ResourceBindings::new()
        .with_binding(
            "audio:music",
            binding(&stereo_manifest, "audio:music", None, 11),
        )
        .unwrap();
    let report = open(
        &timeline(&value),
        &stereo_manifest,
        &bindings,
        &Capabilities::new()
            .with_extension_kernel(
                AUDIO_GAIN_EFFECT_KIND,
                ExtensionKernelCapability::new(AUDIO_GAIN_EFFECT_ABI, digest(KERNEL_DIGEST))
                    .unwrap(),
            )
            .unwrap(),
        &baseline_profile(),
    )
    .unwrap_err();
    assert!(report.contains(EngineOpenDiagnosticCode::UnsupportedExtensionKernel));

    value["document"]["audio"]["tracks"][0]["items"][0]["effects"] = json!([{
        "id": "audio-effect:invalid",
        "type": AUDIO_GAIN_EFFECT_KIND,
        "parameters": {"multiplier": -0.1}
    }]);
    let report = open(
        &timeline(&value),
        &stereo_manifest,
        &bindings,
        &audio_gain_capabilities(),
        &baseline_profile(),
    )
    .unwrap_err();
    assert!(report.contains(EngineOpenDiagnosticCode::InvalidExtensionParameters));
    assert!(
        report
            .diagnostics()
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == EngineOpenDiagnosticCode::InvalidExtensionParameters
            })
            .all(|diagnostic| diagnostic.phase == EngineOpenPhase::Admission),
        "author parameter errors must close during admission: {report:?}"
    );
}

#[test]
fn author_semantic_errors_never_escape_as_compile_phase_diagnostics() {
    let mut audio = audio_crossfade_document("1/4");
    audio["document"]["audio"]["tracks"][0]["items"][0]["effects"] = json!([
        {
            "id": "audio-effect:first",
            "type": AUDIO_GAIN_EFFECT_KIND,
            "parameters": {"multiplier": 1.0e308}
        },
        {
            "id": "audio-effect:second",
            "type": AUDIO_GAIN_EFFECT_KIND,
            "parameters": {"multiplier": 1.0e308}
        }
    ]);
    let audio_manifest = manifest(json!({"audio:music": audio_entry_at(4, "2/1")}));
    let bindings = ResourceBindings::new()
        .with_binding(
            "audio:music",
            binding(&audio_manifest, "audio:music", None, 11),
        )
        .unwrap();
    let report = open(
        &timeline(&audio),
        &audio_manifest,
        &bindings,
        &audio_gain_capabilities(),
        &baseline_profile(),
    )
    .unwrap_err();
    assert!(report.contains(EngineOpenDiagnosticCode::InvalidExtensionParameters));
    assert!(
        report
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.phase != EngineOpenPhase::Compile)
    );
}

#[test]
fn kernel_temporal_footprint_overflow_is_closed_during_admission() {
    let filtered = document_with_source(
        json!({
            "type": "image",
            "resource": "asset:hero",
            "sampling": {"fit": "contain"}
        }),
        json!([
            {
                "id": "filter:large-a",
                "type": "example.visual/glow@1",
                "parameters": {"gain": 1.0}
            },
            {
                "id": "filter:large-b",
                "type": "example.visual/glow@1",
                "parameters": {"gain": 1.0}
            }
        ]),
        "track:main",
        "clip:image",
        json!({}),
    );
    let image_manifest = manifest(json!({"asset:hero": image_entry()}));
    let visual_capabilities = Capabilities::new()
        .with_extension_kernel(
            "example.visual/glow@1",
            known_kernel_capability().with_visual_footprint(VisualFootprint::new(u32::MAX, 0)),
        )
        .unwrap();
    let visual_report = open(
        &timeline(&filtered),
        &image_manifest,
        &image_bindings(&image_manifest, "asset:hero", 1),
        &visual_capabilities,
        &baseline_profile(),
    )
    .unwrap_err();
    let visual_diagnostic = visual_report
        .diagnostics()
        .iter()
        .find(|diagnostic| {
            diagnostic.code == EngineOpenDiagnosticCode::ExtensionKernelBudgetExceeded
                && diagnostic.details.get("reason").map(String::as_str)
                    == Some("visual-temporal-footprint-overflow")
        })
        .expect("visual footprint composition must fail during admission");
    assert_eq!(visual_diagnostic.phase, EngineOpenPhase::Admission);
    assert!(
        visual_report
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.phase != EngineOpenPhase::Compile)
    );

    let mut audio = audio_crossfade_document("1/4");
    audio["document"]["audio"]["tracks"][0]["items"][0]["effects"] = json!([
        {
            "id": "audio-effect:large-a",
            "type": AUDIO_GAIN_EFFECT_KIND,
            "parameters": {"multiplier": 1.0}
        },
        {
            "id": "audio-effect:large-b",
            "type": AUDIO_GAIN_EFFECT_KIND,
            "parameters": {"multiplier": 1.0}
        }
    ]);
    let audio_manifest = manifest(json!({"audio:music": audio_entry_at(4, "2/1")}));
    let audio_bindings = ResourceBindings::new()
        .with_binding(
            "audio:music",
            binding(&audio_manifest, "audio:music", None, 11),
        )
        .unwrap();
    let audio_capabilities = Capabilities::new()
        .with_extension_kernel(
            AUDIO_GAIN_EFFECT_KIND,
            ExtensionKernelCapability::new(
                AUDIO_GAIN_EFFECT_ABI,
                engine_owned_kernel_implementation_digest(AUDIO_GAIN_EFFECT_ABI).unwrap(),
            )
            .unwrap()
            .with_audio_footprint(AudioFootprint::new(9_007_199_254_740_991, 0).unwrap()),
        )
        .unwrap();
    let audio_report = open(
        &timeline(&audio),
        &audio_manifest,
        &audio_bindings,
        &audio_capabilities,
        &baseline_profile(),
    )
    .unwrap_err();
    let audio_diagnostic = audio_report
        .diagnostics()
        .iter()
        .find(|diagnostic| {
            diagnostic.code == EngineOpenDiagnosticCode::ExtensionKernelBudgetExceeded
                && diagnostic.details.get("reason").map(String::as_str)
                    == Some("audio-temporal-footprint-overflow")
        })
        .expect("audio footprint composition must fail during admission");
    assert_eq!(audio_diagnostic.phase, EngineOpenPhase::Admission);
    assert!(
        audio_report
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.phase != EngineOpenPhase::Compile)
    );
}

#[test]
fn audio_resource_sample_count_quantization_is_closed_during_admission() {
    let mut audio = audio_document("audio:music", "1/1");
    audio["document"]["canvas"]["sampleRate"] = json!(4);
    audio["document"]["audio"]["tracks"][0]["items"][0]["source"]["endBehavior"] = json!("hold");

    for (duration, expected_reason) in [
        ("1/100", "source-sample-count-quantized-to-zero"),
        ("9223372036854775807/1", "source-sample-count-overflow"),
    ] {
        let audio_manifest = manifest(json!({"audio:music": audio_entry_at(4, duration)}));
        let bindings = ResourceBindings::new()
            .with_binding(
                "audio:music",
                binding(&audio_manifest, "audio:music", None, 11),
            )
            .unwrap();
        let report = open(
            &timeline(&audio),
            &audio_manifest,
            &bindings,
            &Capabilities::new(),
            &baseline_profile(),
        )
        .unwrap_err();
        let diagnostic = report
            .diagnostics()
            .iter()
            .find(|diagnostic| {
                diagnostic.code == EngineOpenDiagnosticCode::InvalidIntervalQuantization
                    && diagnostic.details.get("reason").map(String::as_str) == Some(expected_reason)
            })
            .unwrap_or_else(|| {
                panic!("audio descriptor sample-count failure must be admitted: {report:?}")
            });
        assert_eq!(diagnostic.phase, EngineOpenPhase::Admission);
        assert!(
            report
                .diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.phase != EngineOpenPhase::Compile)
        );
    }
}

#[cfg(feature = "text")]
#[test]
fn caption_missing_glyph_is_closed_during_admission() {
    let mut value = image_document("asset:hero");
    value["document"]["captions"]["tracks"] = json!([{
        "id": "caption:main",
        "items": [{
            "type": "clip",
            "id": "caption:missing-glyph",
            "duration": "1/1",
            "runs": [{
                "id": "run:missing-glyph",
                "text": "中",
                "timing": null,
                "style": null
            }],
            "style": {"font": "font:main", "fontSize": 24.0, "color": "#ffffffff", "shadow": null},
            "layout": {"region": [0.0, 0.0, 1.0, 1.0], "align": "bottom-center"},
            "presentation": {
                "opacity": {"type": "constant", "value": 1.0},
                "translation": {"type": "constant", "value": [0.0, 0.0]},
                "scale": {"type": "constant", "value": 1.0},
                "rotation": {"type": "constant", "value": 0.0},
                "clipInset": {"type": "constant", "value": [0.0, 0.0, 0.0, 0.0]},
                "blurSigma": {"type": "constant", "value": 0.0}
            },
            "behavior": null
        }]
    }]);
    let manifest = manifest(json!({
        "asset:hero": image_entry(),
        "font:main": font_entry()
    }));
    let bindings = ResourceBindings::new()
        .with_binding("asset:hero", binding(&manifest, "asset:hero", None, 1))
        .unwrap()
        .with_binding("font:main", binding(&manifest, "font:main", None, 2))
        .unwrap();

    let report = open(
        &timeline(&value),
        &manifest,
        &bindings,
        &Capabilities::new(),
        &baseline_profile(),
    )
    .unwrap_err();
    let diagnostic = report
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code == EngineOpenDiagnosticCode::MissingFontGlyph)
        .expect("missing glyph must fail while opening the render");
    assert_eq!(diagnostic.phase, EngineOpenPhase::Admission);
    assert_eq!(
        diagnostic.details.get("codePoint").map(String::as_str),
        Some("U+4E2D")
    );
}

#[cfg(feature = "text")]
#[test]
fn common_profile_rejects_nonzero_collection_font_faces_during_admission() {
    let mut value = image_document("asset:hero");
    value["document"]["captions"]["tracks"] = json!([{
        "id": "caption:main",
        "items": [{
            "type": "clip",
            "id": "caption:hello",
            "duration": "1/1",
            "runs": [{
                "id": "run:hello",
                "text": "hello",
                "timing": null,
                "style": null
            }],
            "style": {"font": "font:collection", "fontSize": 24.0, "color": "#ffffffff", "shadow": null},
            "layout": {"region": [0.0, 0.0, 1.0, 1.0], "align": "bottom-center"},
            "presentation": {
                "opacity": {"type": "constant", "value": 1.0},
                "translation": {"type": "constant", "value": [0.0, 0.0]},
                "scale": {"type": "constant", "value": 1.0},
                "rotation": {"type": "constant", "value": 0.0},
                "clipInset": {"type": "constant", "value": [0.0, 0.0, 0.0, 0.0]},
                "blurSigma": {"type": "constant", "value": 0.0}
            },
            "behavior": null
        }]
    }]);

    let collection = two_face_font_collection(FONT_BYTES);
    let collection_digest = content_digest(&collection);
    let manifest = manifest(json!({
        "asset:hero": image_entry(),
        "font:collection": {
            "kind": "font",
            "digest": collection_digest.to_wire(),
            "descriptor": {"faceIndex": 1, "variationAxes": {}}
        }
    }));
    let ResourceEntryWire::Font { digest, descriptor } = &manifest.entries()["font:collection"]
    else {
        panic!("fixture must contain a font entry")
    };
    let bindings = ResourceBindings::new()
        .with_binding("asset:hero", binding(&manifest, "asset:hero", None, 1))
        .unwrap()
        .with_binding(
            "font:collection",
            ResourceBinding::new(
                digest.clone(),
                VerifiedHandleId::new(2).unwrap(),
                VerifiedResourceFacts::Font {
                    descriptor: descriptor.clone(),
                    bytes: Arc::from(collection),
                },
            ),
        )
        .unwrap();

    let report = open(
        &timeline(&value),
        &manifest,
        &bindings,
        &Capabilities::new(),
        &baseline_profile(),
    )
    .unwrap_err();
    let diagnostic = report
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code == EngineOpenDiagnosticCode::UnsupportedFontFaceIndex)
        .expect("non-zero collection face must fail while opening the render");
    assert_eq!(diagnostic.phase, EngineOpenPhase::Admission);
    assert_eq!(diagnostic.resource_id.as_deref(), Some("font:collection"));
    assert_eq!(
        diagnostic.details.get("actual").map(String::as_str),
        Some("1")
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.phase != EngineOpenPhase::Compile)
    );
}

#[cfg(feature = "text")]
#[test]
fn caption_inline_karaoke_timings_need_no_analysis_resource() {
    let mut value = image_document("asset:hero");
    value["document"]["captions"]["tracks"] = json!([{
        "id": "caption:main",
        "items": [{
            "type": "clip",
            "id": "caption:hello",
            "duration": "1/1",
            "runs": [{
                "id": "run:hello",
                "text": "hello",
                "timing": {"start": "0/1", "end": "1/1"},
                "style": null
            }],
            "style": {"font": "font:main", "fontSize": 24.0, "color": "#ffffffff", "shadow": null},
            "layout": {"region": [0.0, 0.0, 1.0, 1.0], "align": "bottom-center"},
            "presentation": {
                "opacity": {"type": "constant", "value": 1.0},
                "translation": {"type": "constant", "value": [0.0, 0.0]},
                "scale": {"type": "constant", "value": 1.0},
                "rotation": {"type": "constant", "value": 0.0},
                "clipInset": {"type": "constant", "value": [0.0, 0.0, 0.0, 0.0]},
                "blurSigma": {"type": "constant", "value": 0.0}
            },
            "behavior": {"type": "karaoke", "mode": "word"}
        }]
    }]);
    let manifest = manifest(json!({
        "asset:hero": image_entry(),
        "font:main": font_entry()
    }));
    let bindings = ResourceBindings::new()
        .with_binding("asset:hero", binding(&manifest, "asset:hero", None, 1))
        .unwrap()
        .with_binding("font:main", binding(&manifest, "font:main", None, 2))
        .unwrap();
    let compiled = open(
        &timeline(&value),
        &manifest,
        &bindings,
        &Capabilities::new(),
        &baseline_profile(),
    )
    .expect("inline run timings should fully admit karaoke without a Words resource");
    assert_eq!(compiled.captions().tracks().len(), 1);
}

#[test]
fn compiled_program_semantics_change_render_identity_but_execution_site_does_not() {
    let first_value = image_document("asset:hero");
    let mut changed_value = first_value.clone();
    changed_value["document"]["visual"]["tracks"][0]["items"][0]["layer"]["transform"]["position"] =
        constant(json!([0.25, 0.5]));
    let mut changed_background_value = first_value.clone();
    changed_background_value["document"]["background"]["color"] = json!("#112233ff");
    let manifest = manifest(json!({"asset:hero": image_entry()}));
    let profile = baseline_profile();
    let preview = open(
        &timeline(&first_value),
        &manifest,
        &image_bindings(&manifest, "asset:hero", 1),
        &Capabilities::new(),
        &profile,
    )
    .unwrap();
    let export = open(
        &timeline(&first_value),
        &manifest,
        &image_bindings(&manifest, "asset:hero", 99),
        &Capabilities::new(),
        &profile,
    )
    .unwrap();
    let changed = open(
        &timeline(&changed_value),
        &manifest,
        &image_bindings(&manifest, "asset:hero", 1),
        &Capabilities::new(),
        &profile,
    )
    .unwrap();
    let changed_background = open(
        &timeline(&changed_background_value),
        &manifest,
        &image_bindings(&manifest, "asset:hero", 1),
        &Capabilities::new(),
        &profile,
    )
    .unwrap();
    assert_eq!(preview.render_id(), export.render_id());
    assert_ne!(preview.render_id(), changed.render_id());
    assert_ne!(preview.render_id(), changed_background.render_id());
}

#[test]
fn motion_backdrop_fractional_projection_keeps_canonical_sample_bounds() {
    // Preserve the original TSX geometry and animation: frame 175 used to produce
    // different rounded sample bounds in preparation and strict binding validation.
    let artifact = Arc::new(
        valle_compiler::motion::compile_motion(
            r##"
export const composition = { width: 1920, height: 1080, duration: 10 };
export default function Demo(ctx) {
  const t = ctx.localFrame / 300;
  return <Scene style={{ width: 1920, height: 1080, backgroundColor: "#0891b2" }}>
    <View style={{ position: "absolute", left: 90, top: 190, width: 550, height: 350,
      backgroundColor: "#ffffff22", backdropFilter: `blur(${4 + 4 * sin(t * 6.283185307)}px)` }} />
  </Scene>;
}
"##,
        )
        .unwrap()
        .artifact,
    );
    let mut document = motion_document("component:backdrop");
    document["document"]["canvas"]["duration"] = json!("10/1");
    let clip = &mut document["document"]["visual"]["tracks"][0]["items"][0];
    clip["duration"] = json!("10/1");
    clip["source"]["sourceDuration"] = json!("10/1");
    clip["source"]["props"] = json!({});
    clip["source"]["resources"] = json!({});
    let manifest = manifest(json!({"component:backdrop": motion_entry(&artifact)}));
    let bindings = ResourceBindings::new()
        .with_binding(
            "component:backdrop",
            custom_motion_binding(&manifest, "component:backdrop", artifact, &[], 1),
        )
        .unwrap();
    let render = open(
        &timeline(&document),
        &manifest,
        &bindings,
        &Capabilities::new().with_artifact_abi("valle.motion/artifact@1"),
        &baseline_profile(),
    )
    .unwrap();
    let mut compiler = render.frame_compiler();
    for scale in [1, 2] {
        let spec = valle_engine::frame::RenderSpec::new(
            1920 * scale,
            1080 * scale,
            valle_engine::frame::RenderQuality::Preview,
            valle_engine::resource::OutputSpec::srgb_preview(
                valle_engine::resource::OutputBackground::opaque_srgb([0, 0, 0]),
            )
            .unwrap(),
        )
        .unwrap();
        for frame in [174, 175, 176, 225] {
            let prepared = compiler
                .evaluate_prepare(render.render_id(), FrameKey::new(frame), spec)
                .unwrap_or_else(|error| panic!("frame {frame}, scale {scale}: {error}"));
            let output = prepared.prepared();
            assert!(
                output
                    .frame
                    .programs
                    .iter()
                    .any(|program| !program.destination_uses.is_empty()),
                "fixture must exercise backdrop destination bindings"
            );
            output
                .validate()
                .unwrap_or_else(|error| panic!("frame {frame}, scale {scale}: {error}"));
        }
    }
}

#[test]
fn author_transition_preserves_video_source_and_layer_clocks_in_the_overlap() {
    let author = valle_timeline::decode_timeline(&json!({
        "canvas": {"width":640,"height":360,"fps":30},
        "resources": {"video":"https://example.test/video.mp4"},
        "tracks": {"visual":[{
            "clips":[
                {"kind":"video","src":"video","start":0,"duration":2.5,"trimStart":1,"rate":2,
                 "opacity":{"keyframes":[[0,0],[2.5,1]]}},
                {"kind":"video","src":"video","start":1.5,"duration":2.5,"trimStart":0.25,"rate":0.5,
                 "opacity":{"keyframes":[[0,0],[2.5,1]]}}
            ], "transitions":[{"from":0,"to":1,"kind":"circleOpen"}]
        }]}
    }).to_string()).unwrap();
    let canonical = valle_compiler::compile_timeline(author).unwrap();
    let mut video = video_entry("10/1");
    video["descriptor"]["audioStream"] = json!(0);
    let manifest =
        manifest(json!({"resource:video":video,"video:audio":audio_entry_at(48000,"10/1")}));
    let bindings = ResourceBindings::new()
        .with_binding(
            "resource:video",
            binding(&manifest, "resource:video", None, 1)
                .with_dependency(ResourceDependency::new("audio", "video:audio").unwrap()),
        )
        .unwrap()
        .with_binding("video:audio", binding(&manifest, "video:audio", None, 2))
        .unwrap();
    let render = open(
        &canonical,
        &manifest,
        &bindings,
        &Capabilities::new(),
        &baseline_profile(),
    )
    .unwrap();
    assert_eq!(render.canvas().frame_count(), 120);
    let audio = render.audio().evaluate_sample(96000).unwrap();
    assert_eq!(audio.tracks().len(), 2);
    assert_eq!(
        audio.tracks()[0].endpoints()[0].source().sample_time(),
        valle_timeline::RationalTime::new(5, 1).unwrap()
    );
    assert_eq!(
        audio.tracks()[1].endpoints()[0].source().sample_time(),
        valle_timeline::RationalTime::new(1, 2).unwrap()
    );
    for frame in [60, 45, 74, 60] {
        let evaluated = render.evaluate(FrameKey::new(frame)).unwrap();
        let EvaluatedVisualOperation::Transition(t) = &evaluated.visual()[0] else {
            panic!()
        };
        assert_eq!((t.window().start(), t.window().end()), (45, 75));
        assert_eq!(t.progress().as_f64(), (frame - 45) as f64 / 29.0);
        assert_eq!(
            t.from().sample_time(),
            valle_timeline::RationalTime::new(frame + 15, 15).unwrap()
        );
        assert_eq!(
            t.to().sample_time(),
            valle_timeline::RationalTime::new(frame - 30, 60).unwrap()
        );
        assert!((t.from_layer().opacity() - frame as f64 / 75.0).abs() < 1e-9);
        assert!((t.to_layer().opacity() - (frame - 45) as f64 / 75.0).abs() < 1e-9);
    }
    for (frame, id) in [
        (44, "timeline:visual-track:0:clip:0"),
        (75, "timeline:visual-track:0:clip:1"),
    ] {
        let evaluated = render.evaluate(FrameKey::new(frame)).unwrap();
        let EvaluatedVisualOperation::Clip(clip) = &evaluated.visual()[0] else {
            panic!()
        };
        assert_eq!(clip.clip_id(), id);
    }
}

#[test]
fn transition_quantizes_global_overlap_boundaries_instead_of_rounded_duration() {
    let author = valle_timeline::decode_timeline(
        &json!({
            "canvas":{"width":64,"height":64,"fps":1},
            "tracks":{"visual":[{
                "clips":[{"kind":"solid","color":"#000000","start":0,"duration":0.51},
                         {"kind":"solid","color":"#ffffff","start":0.49,"duration":1.51}],
                "transitions":[{"from":0,"to":1,"kind":"fade"}]
            }]}
        })
        .to_string(),
    )
    .unwrap();
    let canonical = valle_compiler::compile_timeline(author).unwrap();
    let render = open(
        &canonical,
        &manifest(json!({})),
        &ResourceBindings::new(),
        &Capabilities::new(),
        &baseline_profile(),
    )
    .unwrap();
    let CompiledVisualItem::Transition { transition } = &render.visual().tracks()[0].items()[1]
    else {
        panic!()
    };
    assert_eq!(
        (transition.window().start(), transition.window().end()),
        (0, 1)
    );
    assert_eq!(transition.progress_at(0).unwrap().as_f64(), 0.5);
}

#[test]
fn transition_parameters_participate_in_identity_and_survive_prepare() {
    use valle_draw::transition::TransitionKind;
    use valle_engine::prepare::{PreparedTransitionKernel, PreparedVisualItem};
    let manifest = manifest(json!({}));
    let bindings = ResourceBindings::new();
    let capabilities = Capabilities::new();
    let profile = baseline_profile();
    let spec = valle_engine::frame::RenderSpec::new(
        64,
        32,
        valle_engine::frame::RenderQuality::Preview,
        valle_engine::resource::OutputSpec::srgb_preview(
            valle_engine::resource::OutputBackground::opaque_srgb([0, 0, 0]),
        )
        .unwrap(),
    )
    .unwrap();
    let make = |params: Value| {
        let mut document = transition_document("3/4");
        document["document"]["visual"]["tracks"][0]["items"][1]["kernel"] =
            json!({"type":"circleOpen","params":params});
        open(
            &timeline(&document),
            &manifest,
            &bindings,
            &capabilities,
            &profile,
        )
        .unwrap()
    };
    let default = make(json!({}));
    let explicit = make(json!({"centerX":0.5,"centerY":0.5,"softness":0.5}));
    let changed = make(json!({"centerX":0.2,"softness":4}));
    assert_eq!(default.render_id(), explicit.render_id());
    assert_ne!(default.render_id(), changed.render_id());
    let prepare = |render: &EngineRender| {
        valle_engine::prepare::prepare_compiled_render_frame_cached(
            render,
            &render.evaluate(FrameKey::new(2)).unwrap(),
            &spec,
            &mut valle_engine::prepare::ProductPrepareCaches::new(),
        )
        .unwrap()
    };
    let output = prepare(&changed);
    let transition = output
        .frame
        .visual
        .iter()
        .find_map(|item| match item {
            PreparedVisualItem::Transition(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        transition.kernel,
        PreparedTransitionKernel::Builtin {
            kind: TransitionKind::CircleOpen,
            params: valle_draw::transition::TransitionValues([0.2, 0.5, 4.0, 0.0])
        }
    );
    let baseline = prepare(&default);
    assert_ne!(
        valle_engine::compositor::graph::build_render_graph(&baseline.frame)
            .unwrap()
            .semantic_hash()
            .unwrap(),
        valle_engine::compositor::graph::build_render_graph(&output.frame)
            .unwrap()
            .semantic_hash()
            .unwrap()
    );
    let mut invalid = serde_json::to_value(output).unwrap();
    // Prepared payload validation cannot trust even a correctly typed parameter array.
    let visual = invalid["frame"]["visual"].as_array_mut().unwrap();
    let item = visual
        .iter_mut()
        .find(|item| item["kind"] == "transition")
        .unwrap();
    item["kernel"]["params"][0] = json!(2.0);
    assert!(serde_json::from_value::<valle_engine::prepare::PrepareOutput>(invalid).is_err());
}

#[test]
fn motion_host_clock_and_props_ignore_source_retiming() {
    let artifact = motion_artifact();
    let resources =
        manifest(json!({"component:title":motion_entry(&artifact),"asset:logo":image_entry()}));
    let bindings = ResourceBindings::new()
        .with_binding(
            "component:title",
            motion_binding(&resources, "component:title", Arc::clone(&artifact), 2),
        )
        .unwrap()
        .with_binding("asset:logo", binding(&resources, "asset:logo", None, 3))
        .unwrap();
    for end in ["hold", "loop"] {
        let mut value = motion_document("component:title");
        value["document"]["canvas"]["fps"] = json!("30000/1001");
        value["document"]["canvas"]["duration"] = json!("3/1");
        let clip = &mut value["document"]["visual"]["tracks"][0]["items"][0];
        clip["duration"] = json!("2/1");
        clip["source"]["rate"] = json!("2/1");
        clip["source"]["endBehavior"] = json!(end);
        clip["source"]["props"]["opacity"] = json!({
            "type":"curve", "id":"curve:host", "interpolation":"linear",
            "keyframes":[
                {"id":"key:start","time":"0/1","value":0.0,"outEasing":null},
                {"id":"key:end","time":"2/1","value":1.0,"outEasing":null}
            ],"extrapolation":"clamp"
        });
        let items = value["document"]["visual"]["tracks"][0]["items"]
            .as_array_mut()
            .unwrap();
        items.insert(
            0,
            json!({"type":"gap","id":"gap:placement","duration":"1/1000"}),
        );
        let render = open(
            &timeline(&value),
            &resources,
            &bindings,
            &Capabilities::new().with_artifact_abi("valle.motion/artifact@1"),
            &baseline_profile(),
        )
        .unwrap();
        for frame in [59, 0, 30, 59, 1, 0] {
            let evaluated = render.evaluate(FrameKey::new(frame)).unwrap();
            let EvaluatedVisualOperation::Clip(clip) = &evaluated.visual()[0] else {
                panic!("expected clip")
            };
            let host = clip.source().motion_host().unwrap();
            let seconds = valle_timeline::RationalTime::new(frame * 1001 - 30, 30000).unwrap();
            assert_eq!(host.sample.composition(), seconds);
            assert_eq!(
                host.duration,
                valle_timeline::RationalTime::new(2, 1).unwrap()
            );
            assert_eq!(
                host.progress,
                valle_timeline::RationalTime::new(frame, 59).unwrap()
            );
            assert_eq!(
                clip.source().motion_props().unwrap()["opacity"],
                valle_engine::render::EvaluatedMotionValue::Scalar(
                    (seconds.as_f64() / 2.0).max(0.0)
                )
            );
        }
    }
}

#[test]
fn motion_host_progress_uses_both_absolute_quantized_boundaries() {
    let artifact = motion_artifact();
    let resources =
        manifest(json!({"component:title":motion_entry(&artifact),"asset:logo":image_entry()}));
    let bindings = ResourceBindings::new()
        .with_binding(
            "component:title",
            motion_binding(&resources, "component:title", Arc::clone(&artifact), 2),
        )
        .unwrap()
        .with_binding("asset:logo", binding(&resources, "asset:logo", None, 3))
        .unwrap();
    for (start, duration, expected_frames) in [
        ("0/1", "1/5", 1),
        ("0/1", "2/5", 1),
        ("3/20", "2/5", 2),
        ("1/5", "1/5", 0),
    ] {
        let mut value = motion_document("component:title");
        value["document"]["canvas"]["fps"] = json!("3/1");
        let clip = &mut value["document"]["visual"]["tracks"][0]["items"][0];
        clip["duration"] = json!(duration);
        if start != "0/1" {
            value["document"]["visual"]["tracks"][0]["items"]
                .as_array_mut()
                .unwrap()
                .insert(
                    0,
                    json!({"type":"gap","id":"gap:placement","duration":start}),
                );
        }
        let result = open(
            &timeline(&value),
            &resources,
            &bindings,
            &Capabilities::new().with_artifact_abi("valle.motion/artifact@1"),
            &baseline_profile(),
        );
        if expected_frames == 0 {
            assert!(result.is_err());
            continue;
        }
        let render = result.unwrap();
        for frame in 0..expected_frames {
            let evaluated = render.evaluate(FrameKey::new(frame)).unwrap();
            let EvaluatedVisualOperation::Clip(clip) = &evaluated.visual()[0] else {
                panic!("expected clip")
            };
            let host = clip.source().motion_host().unwrap();
            assert_eq!(
                host.progress,
                if expected_frames == 1 {
                    valle_timeline::RationalTime::new(1, 2).unwrap()
                } else {
                    valle_timeline::RationalTime::new(frame, expected_frames as u32 - 1).unwrap()
                }
            );
            assert_eq!(host.duration.to_string(), duration);
        }
    }
}

#[test]
fn overlay_host_time_and_viewport_are_exact_in_any_request_order() {
    use valle_timeline::RationalTime;
    for hold in ["once", "loop", "stretch"] {
        let artifact = Arc::new(valle_compiler::motion::compile_motion(&format!(r#"
export const composition={{width:64,height:32,fps:4,duration:2}};
export const role=overlay({{intro:seconds(0.5),outro:seconds(0.5),hold:"{hold}"}});
export default function Main(ctx) {{return <Scene><View key="bar" className="absolute"
 style={{{{left:ctx.seconds*12,top:ctx.host.progress*20,width:ctx.viewport.width/2,height:ctx.viewport.height,backgroundColor:"white"}}}}/></Scene>;}}
"#)).unwrap().artifact);
        let resources = manifest(json!({"component:title":motion_entry(&artifact)}));
        let bindings = ResourceBindings::new()
            .with_binding(
                "component:title",
                custom_motion_binding(&resources, "component:title", Arc::clone(&artifact), &[], 2),
            )
            .unwrap();
        for duration in ["1/1", "3/2", "4/1"] {
            let host =
                serde_json::from_value::<valle_timeline::internal::ExactRational>(json!(duration))
                    .unwrap();
            for size in [None, Some([31.5, 63.49])] {
                let mut value = motion_document("component:title");
                value["document"]["canvas"]["width"] = json!(64);
                value["document"]["canvas"]["height"] = json!(32);
                value["document"]["canvas"]["fps"] = json!("4/1");
                value["document"]["canvas"]["duration"] = json!(duration);
                let clip = &mut value["document"]["visual"]["tracks"][0]["items"][0];
                clip["duration"] = json!(duration);
                clip["source"]["sourceDuration"] = json!("2/1");
                clip["source"]["role"] = serde_json::to_value(artifact.role).unwrap();
                clip["source"]["resources"] = json!({});
                clip["source"]["props"] = json!({});
                if let Some(size) = size {
                    clip["layer"]["transform"]["size"] = constant(json!(size));
                }
                let render = open(
                    &timeline(&value),
                    &resources,
                    &bindings,
                    &Capabilities::new().with_artifact_abi("valle.motion/artifact@1"),
                    &baseline_profile(),
                )
                .unwrap();
                let spec = valle_engine::frame::RenderSpec::new(
                    64,
                    32,
                    valle_engine::frame::RenderQuality::Preview,
                    valle_engine::resource::OutputSpec::srgb_preview(
                        valle_engine::resource::OutputBackground::Transparent,
                    )
                    .unwrap(),
                )
                .unwrap();
                let mut caches = valle_engine::prepare::ProductPrepareCaches::new();
                let frames = (host.as_f64() * 4.0) as i64;
                let mut seen = std::collections::BTreeMap::new();
                for frame in [frames - 1, 0, frames / 2, frames - 1, 1, 0] {
                    let evaluated = render.evaluate(FrameKey::new(frame)).unwrap();
                    let EvaluatedVisualOperation::Clip(clip) = &evaluated.visual()[0] else {
                        panic!("clip")
                    };
                    let source = artifact
                        .role
                        .template_time(
                            RationalTime::new(frame, 4).unwrap(),
                            RationalTime::from_exact(host),
                            RationalTime::new(2, 1).unwrap(),
                        )
                        .unwrap();
                    assert_eq!(clip.source().sample_time(), source);
                    let output = valle_engine::prepare::prepare_compiled_render_frame_cached(
                        &render,
                        &evaluated,
                        &spec,
                        &mut caches,
                    )
                    .unwrap();
                    let motion = &output.inspection.motion[0];
                    let [w, h] = if size.is_some() {
                        [32.0, 63.0]
                    } else {
                        [64.0, 32.0]
                    };
                    assert_eq!(
                        motion.boxes["bar"],
                        [
                            ((source.as_f64() * 12.0).round()) as f32,
                            ((frame as f64 / (frames - 1) as f64 * 20.0).round()) as f32,
                            w / 2.0,
                            h
                        ]
                    );
                    let bytes = serde_json::to_vec(&output).unwrap();
                    if let Some(old) = seen.insert(frame, bytes.clone()) {
                        assert_eq!(old, bytes);
                    }
                }
            }
        }
    }
}

#[test]
fn overlay_host_clock_props_and_outer_curves_share_the_authored_start() {
    use valle_timeline::{MotionRole, OverlayHold, RationalTime};
    let mut artifact = (*motion_artifact()).clone();
    artifact.role = MotionRole::Overlay {
        intro: RationalTime::new(1, 4).unwrap(),
        outro: RationalTime::new(1, 4).unwrap(),
        hold: OverlayHold::Once,
    };
    let artifact = Arc::new(artifact);
    let resources =
        manifest(json!({"component:title":motion_entry(&artifact),"asset:logo":image_entry()}));
    let bindings = ResourceBindings::new()
        .with_binding(
            "component:title",
            motion_binding(&resources, "component:title", Arc::clone(&artifact), 2),
        )
        .unwrap()
        .with_binding("asset:logo", binding(&resources, "asset:logo", None, 3))
        .unwrap();
    for (fps, numerator, denominator) in [("4/1", 4, 1), ("30000/1001", 30000, 1001)] {
        for start in [
            RationalTime::new(1, 20).unwrap(),
            RationalTime::new(1, 8).unwrap(),
        ] {
            let mut value = motion_document("component:title");
            value["document"]["canvas"]["fps"] = json!(fps);
            value["document"]["canvas"]["duration"] = json!("3/1");
            let clip = &mut value["document"]["visual"]["tracks"][0]["items"][0];
            clip["duration"] = json!("2/1");
            clip["source"]["role"] = serde_json::to_value(artifact.role).unwrap();
            for (field, prefix) in [("source", "props"), ("layer", "outer")] {
                let curve = json!({"type":"curve","id":format!("curve:{prefix}"),"interpolation":"linear",
                    "keyframes":[{"id":format!("key:{prefix}:start"),"time":"0/1","value":0.0,"outEasing":null},
                        {"id":format!("key:{prefix}:end"),"time":"2/1","value":1.0,"outEasing":null}],"extrapolation":"clamp"});
                if field == "source" {
                    clip[field]["props"]["opacity"] = curve;
                } else {
                    clip[field]["opacity"] = curve;
                }
            }
            value["document"]["visual"]["tracks"][0]["items"]
                .as_array_mut()
                .unwrap()
                .insert(
                    0,
                    json!({"type":"gap","id":"gap:placement","duration":start.to_string()}),
                );
            let render = open(
                &timeline(&value),
                &resources,
                &bindings,
                &Capabilities::new().with_artifact_abi("valle.motion/artifact@1"),
                &baseline_profile(),
            )
            .unwrap();
            let first =
                (start.as_f64() * f64::from(numerator) / f64::from(denominator)).round() as i64;
            for frame in [first + 2, first, first + 1, first + 2, first] {
                let evaluated = render.evaluate(FrameKey::new(frame)).unwrap();
                let EvaluatedVisualOperation::Clip(clip) = &evaluated.visual()[0] else {
                    panic!("clip")
                };
                let seconds = RationalTime::new(frame * i64::from(denominator), numerator)
                    .unwrap()
                    .checked_sub(start)
                    .unwrap();
                assert_eq!(
                    clip.source().motion_host().unwrap().sample.composition(),
                    seconds
                );
                let valle_engine::render::EvaluatedMotionValue::Scalar(prop) =
                    clip.source().motion_props().unwrap()["opacity"]
                else {
                    panic!("scalar")
                };
                assert_eq!(prop, clip.layer().opacity());
                assert_eq!(prop, (seconds.as_f64() / 2.0).max(0.0));
                assert_eq!(
                    clip.source().sample_time(),
                    artifact
                        .role
                        .template_time(seconds, RationalTime::new(2, 1).unwrap(), RationalTime::ONE)
                        .unwrap()
                );
            }
        }
    }
}
