use serde_json::{Value, json};
use valle_timeline::internal::{ResourceManifestError, decode_resource_manifest};

const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SEMANTIC_DIGEST: &str =
    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn color() -> Value {
    json!({
        "primaries": "bt709",
        "transfer": "bt709",
        "matrix": "bt709",
        "fullRange": false
    })
}

fn valid_manifest() -> Value {
    json!({
        "entries": {
            "asset:video": {
                "kind": "video",
                "digest": DIGEST,
                "descriptor": {
                    "duration": "60/1",
                    "timeBase": "1/90000",
                    "presentationIndexDigest": SEMANTIC_DIGEST,
                    "width": 3840,
                    "height": 2160,
                    "orientation": "identity",
                    "color": color(),
                    "videoStream": 0,
                    "audioStream": null
                }
            },
            "asset:audio": {
                "kind": "audio",
                "digest": DIGEST,
                "descriptor": {
                    "duration": "30/1",
                    "timeBase": "1/48000",
                    "presentationIndexDigest": SEMANTIC_DIGEST,
                    "sampleRate": 48000,
                    "channelLayout": "stereo",
                    "audioStream": 0
                }
            },
            "asset:image": {
                "kind": "image",
                "digest": DIGEST,
                "descriptor": {
                    "width": 1920,
                    "height": 1080,
                    "orientation": "identity",
                    "color": color()
                }
            },
            "asset:lottie": {
                "kind": "lottie",
                "digest": DIGEST,
                "abi": "valle.lottie/artifact@1",
                "descriptor": {
                    "duration": "5/1",
                    "timeBase": "1/60",
                    "width": 1920,
                    "height": 1080,
                    "boundarySampling": "left-limit"
                }
            },
            "font:inter": {
                "kind": "font",
                "digest": DIGEST,
                "descriptor": {
                    "faceIndex": 0,
                    "variationAxes": {
                        "wght": {"minimum": 100.0, "default": 400.0, "maximum": 900.0}
                    }
                }
            },
            "component:title": {
                "kind": "motion-artifact",
                "digest": DIGEST,
                "abi": "valle.motion/artifact@1",
                "descriptor": {
                    "readsDestination": true,
                    "boundarySampling": "left-limit"
                }
            },
            "shader:glass": {
                "kind": "shader",
                "digest": DIGEST,
                "abi": "valle.shader/artifact@1",
                "descriptor": {
                    "controlsSchemaDigest": SEMANTIC_DIGEST,
                    "readsDestination": true
                }
            }
        }
    })
}

fn encoded(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

#[test]
fn validated_manifest_owns_canonical_bytes() {
    let input = encoded(&valid_manifest());
    let manifest = decode_resource_manifest(&input).unwrap();

    assert_eq!(manifest.entries().len(), 7);
    assert_eq!(
        decode_resource_manifest(manifest.canonical_bytes())
            .unwrap()
            .canonical_bytes(),
        manifest.canonical_bytes()
    );
}

#[test]
fn object_order_and_whitespace_do_not_change_canonical_bytes() {
    let compact = encoded(&valid_manifest());
    let pretty = serde_json::to_vec_pretty(&valid_manifest()).unwrap();

    let compact = decode_resource_manifest(&compact).unwrap();
    let pretty = decode_resource_manifest(&pretty).unwrap();

    assert_eq!(compact.canonical_bytes(), pretty.canonical_bytes());
}

#[test]
fn strict_json_checks_run_before_wire_construction() {
    let duplicate = format!(
        r#"{{"entries":{{"asset:x":{{"kind":"image","digest":"{DIGEST}","descriptor":{{"width":1,"height":1,"orientation":"identity","color":{{"primaries":"srgb","transfer":"srgb","matrix":"identity","fullRange":true}}}}}},"asset:x":{{"kind":"image","digest":"{DIGEST}","descriptor":{{"width":1,"height":1,"orientation":"identity","color":{{"primaries":"srgb","transfer":"srgb","matrix":"identity","fullRange":true}}}}}}}}}}"#
    );
    assert!(matches!(
        decode_resource_manifest(duplicate.as_bytes()),
        Err(ResourceManifestError::DuplicateObjectKey { .. })
    ));

    let mut bom = vec![0xef, 0xbb, 0xbf];
    bom.extend_from_slice(br#"{"entries":{}}"#);
    assert_eq!(
        decode_resource_manifest(&bom).unwrap_err(),
        ResourceManifestError::Utf8Bom
    );

    let mut retired_contract = valid_manifest();
    retired_contract["contract"] = json!("valle.resource/manifest@1.0");
    assert_eq!(
        decode_resource_manifest(&encoded(&retired_contract)).unwrap_err(),
        ResourceManifestError::InvalidWireShape
    );
}

#[test]
fn empty_and_duplicate_resource_ids_are_rejected() {
    let mut empty = valid_manifest();
    let entry = empty["entries"]
        .as_object_mut()
        .unwrap()
        .remove("asset:image")
        .unwrap();
    empty["entries"][""] = entry;
    assert_eq!(
        decode_resource_manifest(&encoded(&empty)).unwrap_err(),
        ResourceManifestError::EmptyResourceId
    );

    let duplicate = format!(
        r#"{{"entries":{{"x":{{"kind":"image","digest":"{DIGEST}","descriptor":{{"width":1,"height":1,"orientation":"identity","color":{{"primaries":"srgb","transfer":"srgb","matrix":"identity","fullRange":true}}}}}},"x":{{"kind":"image","digest":"{DIGEST}","descriptor":{{"width":1,"height":1,"orientation":"identity","color":{{"primaries":"srgb","transfer":"srgb","matrix":"identity","fullRange":true}}}}}}}}}}"#
    );
    assert!(matches!(
        decode_resource_manifest(duplicate.as_bytes()),
        Err(ResourceManifestError::DuplicateObjectKey { key }) if key == "x"
    ));

    for invalid_id in [
        "plain",
        "asset:",
        ":image",
        "asset:has space",
        "https://host/x",
    ] {
        let mut manifest = valid_manifest();
        let entry = manifest["entries"]
            .as_object_mut()
            .unwrap()
            .remove("asset:image")
            .unwrap();
        manifest["entries"][invalid_id] = entry;
        assert!(matches!(
            decode_resource_manifest(&encoded(&manifest)),
            Err(ResourceManifestError::InvalidResourceId { resource_id })
                if resource_id == invalid_id
        ));
    }
}

#[test]
fn media_dimensions_times_and_sample_rate_must_be_positive() {
    for (resource, field, invalid) in [
        ("asset:video", "width", json!(0)),
        ("asset:video", "height", json!(0)),
        ("asset:video", "duration", json!("0/1")),
        ("asset:video", "timeBase", json!("-1/90000")),
        ("asset:audio", "sampleRate", json!(0)),
        ("asset:audio", "duration", json!("0/1")),
        ("asset:image", "width", json!(0)),
        ("asset:lottie", "height", json!(0)),
        ("asset:lottie", "duration", json!("0/1")),
    ] {
        let mut manifest = valid_manifest();
        manifest["entries"][resource]["descriptor"][field] = invalid;
        assert!(matches!(
            decode_resource_manifest(&encoded(&manifest)),
            Err(ResourceManifestError::InvalidDescriptor { resource_id, field: rejected })
                if resource_id == resource && rejected == field
        ));
    }
}

#[test]
fn font_axes_have_local_invariants() {
    let mut axis_order = valid_manifest();
    axis_order["entries"]["font:inter"]["descriptor"]["variationAxes"]["wght"]["default"] =
        json!(950.0);
    assert!(matches!(
        decode_resource_manifest(&encoded(&axis_order)),
        Err(ResourceManifestError::InvalidDescriptor {
            field: "variationAxes",
            ..
        })
    ));

    let mut axis_tag = valid_manifest();
    let axis = axis_tag["entries"]["font:inter"]["descriptor"]["variationAxes"]
        .as_object_mut()
        .unwrap()
        .remove("wght")
        .unwrap();
    axis_tag["entries"]["font:inter"]["descriptor"]["variationAxes"]["weight"] = axis;
    assert!(matches!(
        decode_resource_manifest(&encoded(&axis_tag)),
        Err(ResourceManifestError::InvalidDescriptor {
            field: "variationAxes",
            ..
        })
    ));
}

#[test]
fn malformed_wire_and_non_finite_numbers_fail_before_construction() {
    let mut malformed = valid_manifest();
    malformed["entries"]["asset:image"]["descriptor"]["locator"] = json!("forbidden");
    assert_eq!(
        decode_resource_manifest(&encoded(&malformed)).unwrap_err(),
        ResourceManifestError::InvalidWireShape
    );

    let non_finite = format!(
        r#"{{"entries":{{"font:x":{{"kind":"font","digest":"{DIGEST}","descriptor":{{"faceIndex":0,"variationAxes":{{"wght":{{"minimum":1e400,"default":400,"maximum":900}}}}}}}}}}}}"#
    );
    assert_eq!(
        decode_resource_manifest(non_finite.as_bytes()).unwrap_err(),
        ResourceManifestError::NonFiniteNumber
    );
}
