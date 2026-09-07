use serde_json::Value;
use valle_web_schema_gen::{MotionContextWire, StudioBootWire};

fn fixed_motion_context() -> Value {
    serde_json::json!({
      "status": "ok",
      "protocolVersion": 1,
      "generation": 3,
      "input": "components/Card.tsx",
      "artifactDigest": format!("sha256:{}", "a".repeat(64)),
      "artifact": {
        "formatVersion": 1,
        "component": "Card",
        "controls": {
          "props": {},
          "data": {},
          "timing": {
            "enterFrames": {"default": 0, "min": 0, "max": null},
            "holdCycleFrames": {"default": null, "min": 1, "max": null},
            "exitFrames": {"default": 0, "min": 0, "max": null}
          },
          "cues": {},
          "assets": {},
          "camera": {"values": {}}
        }
      },
      "preparedData": {},
      "dataSource": null,
      "timing": {"enterFrames": 0, "exitFrames": 0},
      "cueBindings": {
        "beat": {
          "type": "sourceRange",
          "startFrame": 0,
          "endFrame": 20,
          "enterFrames": 2,
          "exitFrames": 3
        },
        "title": {
          "type": "sourceRange",
          "startFrame": 5,
          "endFrame": 18,
          "enterFrames": 1,
          "exitFrames": 1
        }
      },
      "sourceMap": {
        "version": 1,
        "component": "Card",
        "entry": "components/Card.tsx",
        "closureDigest": format!("sha256:{}", "c".repeat(64)),
        "modules": [],
        "nodes": [],
        "exprs": [],
        "objects": []
      },
      "assets": [{"name": "hero", "kind": "image", "url": "/motion-assets/hero"}],
      "resourceLocators": [{"id": "asset:hero", "url": "/motion-assets/hero"}],
      "shaders": [],
      "durationFrames": 30,
      "fps": {"num": 30, "den": 1},
      "viewport": {"width": 1920, "height": 1080},
      "diagnostics": [],
      "runtimeBaseUrl": "/",
      "runtimeAssets": {},
      "fixedPackageManifestJson": "{\"format\":\"valle.fixed-render-package@1\"}",
      "timelineJson": "{\"document\":{}}",
      "timeline": {"document": {}},
      "resourceManifestJson": "{\"entries\":{}}",
      "resourceManifest": {"entries": {}},
      "verifiedBindingBundleJson": "{\"bindings\":{},\"capabilities\":{}}"
    })
}

#[test]
fn non_timeline_host_protocols_remain_closed() {
    let boot = r#"{
      "protocolVersion":1,
      "session":{"kind":"timeline-file","input":"timeline.json"},
      "capabilities":{
        "saveTimeline":false,
        "editProject":false,
        "editMotionProps":false,
        "writeMotionSource":false
      },
      "runtime":{"assetBaseUrl":"/assets/","assetUrls":{}}
    }"#;
    serde_json::from_str::<StudioBootWire>(boot).expect("generated Studio boot accepts its DTO");

    let with_unknown = boot.replace(
        "\"protocolVersion\":1",
        "\"protocolVersion\":1,\"unsupportedTimeline\":{}",
    );
    assert!(serde_json::from_str::<StudioBootWire>(&with_unknown).is_err());

    let motion_error = r#"{
      "status":"error",
      "protocolVersion":1,
      "generation":2,
      "input":"components/Card.tsx",
      "diagnostics":[]
    }"#;
    let decoded: MotionContextWire = serde_json::from_str(motion_error).expect("motion error DTO");
    assert_eq!(
        serde_json::to_value(decoded).expect("serialize motion error")["status"],
        Value::String("error".to_owned()),
    );
}

#[test]
fn motion_compiler_error_span_round_trips_the_real_producer_shape() {
    let error = r#"{
          "status":"error",
          "protocolVersion":1,
          "generation":2,
          "input":"components/Card.tsx",
          "diagnostics":[{
            "class":"illegal",
            "code":"motion-parse",
            "span":{"start":4,"end":9,"line":1,"column":5},
            "sourcePath":"components/Card.tsx",
            "message":"invalid expression"
          }]
        }"#;
    let decoded: MotionContextWire = serde_json::from_str(&error).expect("compiler error DTO");
    let encoded = serde_json::to_value(decoded).expect("serialize compiler error");
    assert_eq!(encoded["diagnostics"][0]["span"]["start"], 4);
    assert_eq!(encoded["diagnostics"][0]["span"]["end"], 9);
}

#[test]
fn checked_generation_has_a_real_negative_drift_probe() {
    let checked = valle_web_schema_gen::render_types(false);
    let simulated_rust_change = valle_web_schema_gen::render_types(true);
    assert_ne!(checked, simulated_rust_change);
    assert!(simulated_rust_change.contains("changedField"));
}

#[test]
fn motion_fixed_package_fields_reference_authoritative_timeline_types() {
    let generated = valle_web_schema_gen::render_types(false);
    assert!(generated.contains("export type ContentDigest = string;"));
    assert!(!generated.contains("Sha256DigestWire"));
    for field in ["artifactDigest", "closureDigest"] {
        assert!(
            generated.contains(&format!("{field}: ContentDigest")),
            "generated protocol must keep `{field}` on the shared ContentDigest alias"
        );
    }
    assert!(generated.contains(
        "import type { ResourceManifest, TimelineDocument } from \"../internal-timeline.ts\";"
    ));
    for (field, expected_type) in [
        ("timeline", "TimelineDocument"),
        ("resourceManifest", "ResourceManifest"),
    ] {
        assert!(
            generated.contains(&format!("{field}: {expected_type}")),
            "generated MotionContext must type `{field}` as `{expected_type}`"
        );
        assert!(
            !generated.contains(&format!("{field}: Record<string, unknown>")),
            "generated MotionContext must not erase `{field}` to an open record"
        );
    }
}

#[test]
fn motion_success_requires_the_closed_fixed_package() {
    let value = fixed_motion_context();
    serde_json::from_value::<MotionContextWire>(value.clone())
        .expect("complete fixed Motion context");

    for required in [
        "fixedPackageManifestJson",
        "timelineJson",
        "timeline",
        "resourceManifestJson",
        "resourceManifest",
        "verifiedBindingBundleJson",
        "resourceLocators",
    ] {
        let mut missing = value.clone();
        missing
            .as_object_mut()
            .expect("fixture object")
            .remove(required);
        assert!(
            serde_json::from_value::<MotionContextWire>(missing).is_err(),
            "success DTO admitted missing `{required}`"
        );
    }
}

#[test]
fn motion_success_rejects_the_unsupported_controls_mirror() {
    let mut value = fixed_motion_context();
    value["controls"] = serde_json::json!({});
    assert!(serde_json::from_value::<MotionContextWire>(value).is_err());
}

#[test]
fn motion_digests_have_one_exact_wire_spelling() {
    let value = fixed_motion_context();
    for invalid in [
        "abc".to_owned(),
        format!("sha256:{}", "A".repeat(64)),
        format!("sha256:{}", "a".repeat(63)),
    ] {
        let mut bad_artifact = value.clone();
        bad_artifact["artifactDigest"] = Value::String(invalid.clone());
        assert!(serde_json::from_value::<MotionContextWire>(bad_artifact).is_err());

        let mut bad_source_map = value.clone();
        bad_source_map["sourceMap"]["closureDigest"] = Value::String(invalid);
        assert!(serde_json::from_value::<MotionContextWire>(bad_source_map).is_err());
    }
}

#[test]
fn motion_cues_require_an_explicit_closed_discriminator() {
    let value = fixed_motion_context();
    serde_json::from_value::<MotionContextWire>(value.clone())
        .expect("canonical Motion source-range cues");

    let mut missing_type = value.clone();
    missing_type["cueBindings"]["beat"]
        .as_object_mut()
        .expect("cue object")
        .remove("type");
    assert!(serde_json::from_value::<MotionContextWire>(missing_type).is_err());

    let mut unknown_type = value;
    unknown_type["cueBindings"]["title"]["type"] = Value::String("unsupported".to_owned());
    assert!(serde_json::from_value::<MotionContextWire>(unknown_type).is_err());

    for required in ["startFrame", "endFrame"] {
        let mut missing_resolved_bound = fixed_motion_context();
        missing_resolved_bound["cueBindings"]["title"]
            .as_object_mut()
            .expect("source-range cue object")
            .remove(required);
        assert!(
            serde_json::from_value::<MotionContextWire>(missing_resolved_bound).is_err(),
            "source-range cue admitted missing resolved `{required}`"
        );
    }
}

#[test]
fn runtime_locators_cannot_smuggle_probe_or_unsupported_motion_side_channels() {
    let mut locator_with_probe = fixed_motion_context();
    locator_with_probe["resourceLocators"][0]["digest"] = Value::String("sha256:abc".to_owned());
    assert!(serde_json::from_value::<MotionContextWire>(locator_with_probe).is_err());

    let mut unsupported_audio = fixed_motion_context();
    unsupported_audio["audioEnvelopes"] = serde_json::json!({});
    assert!(serde_json::from_value::<MotionContextWire>(unsupported_audio).is_err());
}
