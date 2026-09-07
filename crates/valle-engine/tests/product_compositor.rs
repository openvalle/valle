use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use valle_engine::{
    compositor::{
        ExternalObjectTable,
        graph::GraphCapability,
        lower::{
            BackendCapabilities, FramebufferFetchSemantics, RenderBindings, RenderPlanTemplate,
        },
        reference::{
            ReferenceExternalObject, ReferenceImage, ReferenceTarget, decode_author_srgb,
            execute_reference,
        },
    },
    fixed_package::{
        COMMON_PROFILE_KEY, canonical_fixed_execution_profile, canonical_fixed_package_manifest,
        canonical_verified_binding_bundle, fixed_package_files, open_verified_fixed_package,
    },
    frame::{RenderQuality, RenderSpec},
    product::{EngineRender, ProductEngineError},
    render::{
        Capabilities, ResourceBinding, ResourceBindings, VerifiedHandleId, VerifiedResourceFacts,
        VisualFootprint,
    },
    resource::{
        AuthorSrgbStraight, Extent2d, ExternalGeneration, ExternalPixelLayout,
        ExternalResourceDesc, OutputBackground, OutputSpec, ResourceRequestSet, TextureFormat,
        TextureUsage,
    },
};
use valle_timeline::internal::FrameKey;
use valle_timeline::internal::{
    RenderId, ResourceManifest, canonical_bytes, decode_canonical, decode_resource_manifest,
    wire::resource::ResourceEntryWire,
};

const IMAGE_DIGEST: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn constant(value: Value) -> Value {
    json!({"type": "constant", "value": value})
}

fn image_document() -> valle_timeline::internal::CanonicalTimeline {
    let value = json!({
        "document": {
            "canvas": {
                "width": 2,
                "height": 1,
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
                            "filters": [],
                            "blend": "normal"
                        },
                        "source": {
                            "type": "image",
                            "resource": "asset:hero",
                            "sampling": {"fit": "contain"}
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
    });
    decode_canonical(&serde_json::to_string(&value).unwrap()).unwrap()
}

fn image_manifest() -> ResourceManifest {
    decode_resource_manifest(
        &serde_json::to_vec(&json!({
            "entries": {
                "asset:hero": {
                    "kind": "image",
                    "digest": IMAGE_DIGEST,
                    "descriptor": {
                        "width": 2,
                        "height": 1,
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
    .unwrap()
}

fn compiled_render() -> EngineRender {
    let timeline = image_document();
    let manifest = image_manifest();
    let ResourceEntryWire::Image {
        digest, descriptor, ..
    } = manifest.entries().get("asset:hero").unwrap()
    else {
        panic!("fixture resource must be an image")
    };
    let bindings = ResourceBindings::new()
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
        .unwrap();
    let timeline_json = String::from_utf8(canonical_bytes(&timeline).unwrap()).unwrap();
    let manifest_json = std::str::from_utf8(manifest.canonical_bytes()).unwrap();
    let bundle_json = canonical_verified_binding_bundle(&bindings, &Capabilities::new()).unwrap();
    let profile_json = canonical_fixed_execution_profile(COMMON_PROFILE_KEY).unwrap();
    let files = fixed_package_files(&timeline_json, manifest_json, &bundle_json, &profile_json);
    let package_manifest = canonical_fixed_package_manifest(&files).unwrap();
    open_verified_fixed_package(&package_manifest, &files)
        .unwrap()
        .engine_render()
}

fn capabilities() -> BackendCapabilities {
    BackendCapabilities::new(
        Extent2d::new(4096, 4096).unwrap(),
        [TextureFormat::Rgba32Float, TextureFormat::Rgba16Float],
        [
            TextureUsage::Sampled,
            TextureUsage::StorageRead,
            TextureUsage::StorageWrite,
            TextureUsage::ColorAttachment,
        ],
        [1],
        [ExternalPixelLayout::Rgba8],
        [
            GraphCapability::Blend,
            GraphCapability::Clear,
            GraphCapability::ExternalImport,
            GraphCapability::OutputTransform,
        ],
        true,
        Some(FramebufferFetchSemantics::CoherentWorkingPremultiplied),
        32 * 1024 * 1024,
        64 * 1024 * 1024,
    )
    .unwrap()
}

fn render_spec(width: u32) -> RenderSpec {
    RenderSpec::new(
        width,
        1,
        RenderQuality::Preview,
        OutputSpec::srgb_preview(OutputBackground::opaque_srgb([12, 34, 56])).unwrap(),
    )
    .unwrap()
}

#[test]
fn staged_product_api_pins_one_compiled_render_through_execution() {
    let render = compiled_render();
    let render_id = render.render_id();
    let spec = render_spec(2);
    let mut compiler = render.frame_compiler();

    let wrong_render = RenderId::from_bytes([0x55; 32]);
    assert!(matches!(
        compiler.evaluate_prepare(wrong_render, FrameKey::new(0), spec),
        Err(ProductEngineError::RenderMismatch { .. })
    ));

    let prepared = compiler
        .evaluate_prepare(render_id, FrameKey::new(0), spec)
        .unwrap();
    assert_eq!(prepared.evaluated().render_id(), render_id);
    assert_eq!(prepared.prepared().frame.render_id, render_id);
    assert!(
        serde_json::to_value(&prepared.prepared().frame)
            .unwrap()
            .get("header")
            .is_none()
    );
    let request_bytes = prepared
        .prepared()
        .resource_requests
        .packed_bytes()
        .unwrap();
    assert_eq!(
        serde_json::to_string(&prepared.prepared().resource_requests).unwrap(),
        r#"[{"handle":1,"key":{"content":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","interpretation":{"kind":"visual","interpretation":{"color":{"primaries":"rec709","transfer":"rec709","matrix":"bt709","range":"limited"},"luminance":{"referenceWhite":100,"peak":100},"alpha":"straightCoverage","orientation":"identity","sampleAspectRatio":{"numerator":1,"denominator":1},"crop":{"left":{"numerator":0,"denominator":1},"top":{"numerator":0,"denominator":1},"right":{"numerator":1,"denominator":1},"bottom":{"numerator":1,"denominator":1}}}}},"sample":{"kind":"static"},"expected":{"kind":"visualFrame","extent":{"width":2,"height":1},"pixel_layout":"rgba8"},"payload":null}]"#
    );
    let packed_digest = Sha256::digest(&request_bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        packed_digest,
        "fd5a942092a3dae7f0a70bcc1987718e6026a60448bf78242045298a38c78d8c"
    );
    assert_eq!(
        ResourceRequestSet::from_packed(&request_bytes)
            .unwrap()
            .requests()
            .len(),
        1
    );

    let generation = ExternalGeneration::new(7).unwrap();
    let bound = prepared
        .lower(&capabilities())
        .unwrap()
        .bind(generation)
        .unwrap();
    assert_eq!(bound.graph().render_id, render_id);
    assert!(
        serde_json::to_value(bound.graph())
            .unwrap()
            .get("header")
            .is_none()
    );
    assert_eq!(*bound.template().render_id(), render_id);
    assert_eq!(*bound.bindings().render_id(), render_id);

    let plan = RenderPlanTemplate::from_packed(&bound.template().packed_bytes().unwrap()).unwrap();
    let bindings = RenderBindings::from_packed(&bound.bindings().packed_bytes().unwrap()).unwrap();
    assert_eq!(*plan.render_id(), render_id);
    assert_eq!(*bindings.render_id(), render_id);
    plan.validate_bindings(&bindings).unwrap();

    let slot = &plan.binding_layout().external_slots()[0];
    let ExternalResourceDesc::VisualFrame { pixel_layout, .. } = slot.expected else {
        panic!("image fixture must lower to a visual external slot")
    };
    let pixel = decode_author_srgb(AuthorSrgbStraight([255, 0, 0, 255])).unwrap();
    let image = ReferenceImage::solid(Extent2d::new(2, 1).unwrap(), pixel).unwrap();
    let object = ReferenceExternalObject::visual(slot.key.clone(), pixel_layout, image).unwrap();
    let objects =
        ExternalObjectTable::try_from_entries(generation, [(bindings.external_ids()[0], object)])
            .unwrap();
    let mut target = ReferenceTarget::new(Extent2d::new(2, 1).unwrap(), spec.output());
    execute_reference(&plan, &bindings, &capabilities(), &objects, &mut target).unwrap();
    assert!(target.frame().is_some());
}

#[test]
fn product_plan_cache_is_render_spec_and_mode_scoped() {
    let render = compiled_render();
    let render_id = render.render_id();
    let backend = capabilities();
    let spec = render_spec(2);

    let mut first_worker = render.frame_compiler();
    let first = first_worker
        .evaluate_prepare(render_id, FrameKey::new(0), spec)
        .unwrap()
        .lower(&backend)
        .unwrap();
    assert!(!first.template_cache_hit());

    let mut second_worker = render.frame_compiler();
    let second = second_worker
        .evaluate_prepare(render_id, FrameKey::new(1), spec)
        .unwrap()
        .lower(&backend)
        .unwrap();
    assert!(second.template_cache_hit());
    assert_eq!(first.template(), second.template());

    let different_spec = second_worker
        .evaluate_prepare(render_id, FrameKey::new(0), render_spec(3))
        .unwrap()
        .lower(&backend)
        .unwrap();
    assert!(!different_spec.template_cache_hit());

    let native = first_worker
        .evaluate_prepare(render_id, FrameKey::new(1), spec)
        .unwrap()
        .lower_constructed(&backend)
        .unwrap();
    assert!(!native.template_cache_hit());
    assert!(native.template().packed_bytes().is_err());
    let native = native
        .bind_constructed(ExternalGeneration::new(9).unwrap())
        .unwrap();
    assert!(native.bindings().packed_bytes().is_err());
}

#[test]
fn product_frame_range_is_checked_by_the_pinned_compiled_render() {
    let render = compiled_render();
    let mut compiler = render.frame_compiler();
    let error = compiler
        .evaluate_prepare(render.render_id(), FrameKey::new(30), render_spec(2))
        .unwrap_err();
    assert!(matches!(error, ProductEngineError::Evaluate(_)));
}
