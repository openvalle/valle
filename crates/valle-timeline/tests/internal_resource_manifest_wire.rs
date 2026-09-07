use serde_json::{Value, json};
use valle_timeline::internal::{
    ContentDigest,
    wire::resource::{ResourceEntryWire, ResourceManifestEnvelopeWire},
};

const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SCHEMA_DIGEST: &str =
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
                    "presentationIndexDigest": SCHEMA_DIGEST,
                    "width": 3840,
                    "height": 2160,
                    "orientation": "identity",
                    "color": color(),
                    "videoStream": 0,
                    "audioStream": 1
                }
            },
            "asset:audio": {
                "kind": "audio",
                "digest": DIGEST,
                "descriptor": {
                    "duration": "30/1",
                    "timeBase": "1/48000",
                    "presentationIndexDigest": SCHEMA_DIGEST,
                    "sampleRate": 48000,
                    "channelLayout": "mono",
                    "audioStream": 0
                }
            },
            "asset:image": {
                "kind": "image",
                "digest": DIGEST,
                "descriptor": {
                    "width": 1920,
                    "height": 1080,
                    "orientation": "rotate-90",
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
                    "controlsSchemaDigest": SCHEMA_DIGEST,
                    "readsDestination": true
                }
            }
        }
    })
}

#[test]
fn all_closed_resource_kinds_decode_and_round_trip() {
    let manifest: ResourceManifestEnvelopeWire = serde_json::from_value(valid_manifest()).unwrap();

    assert_eq!(manifest.entries.len(), 7);
    assert!(matches!(
        manifest.entries["asset:video"],
        ResourceEntryWire::Video { .. }
    ));
    assert!(matches!(
        manifest.entries["component:title"],
        ResourceEntryWire::MotionArtifact { .. }
    ));

    let round_trip = serde_json::to_value(&manifest).unwrap();
    assert_eq!(round_trip, valid_manifest());
}

#[test]
fn unknown_envelope_entry_and_descriptor_fields_are_rejected() {
    let mut envelope = valid_manifest();
    envelope["unexpected"] = json!(true);
    assert!(serde_json::from_value::<ResourceManifestEnvelopeWire>(envelope).is_err());

    let mut entry = valid_manifest();
    entry["entries"]["asset:image"]["unexpected"] = json!(true);
    assert!(serde_json::from_value::<ResourceManifestEnvelopeWire>(entry).is_err());

    let mut descriptor = valid_manifest();
    descriptor["entries"]["asset:image"]["descriptor"]["unexpected"] = json!(true);
    assert!(serde_json::from_value::<ResourceManifestEnvelopeWire>(descriptor).is_err());
}

#[test]
fn unknown_kind_and_abi_are_rejected() {
    let mut kind = valid_manifest();
    kind["entries"]["asset:image"]["kind"] = json!("document");
    assert!(serde_json::from_value::<ResourceManifestEnvelopeWire>(kind).is_err());

    let mut retired_words = valid_manifest();
    retired_words["entries"]["asset:image"]["kind"] = json!("words");
    assert!(serde_json::from_value::<ResourceManifestEnvelopeWire>(retired_words).is_err());

    let mut retired_data = valid_manifest();
    retired_data["entries"]["asset:image"]["kind"] = json!("data");
    assert!(serde_json::from_value::<ResourceManifestEnvelopeWire>(retired_data).is_err());

    let mut abi = valid_manifest();
    abi["entries"]["component:title"]["abi"] = json!("valle.motion/artifact@6");
    assert!(serde_json::from_value::<ResourceManifestEnvelopeWire>(abi).is_err());
}

#[test]
fn binding_information_cannot_be_injected() {
    for field in ["locator", "url", "path", "token"] {
        let mut at_entry = valid_manifest();
        at_entry["entries"]["asset:video"][field] = json!("forbidden");
        assert!(
            serde_json::from_value::<ResourceManifestEnvelopeWire>(at_entry).is_err(),
            "entry accepted {field}"
        );

        let mut at_descriptor = valid_manifest();
        at_descriptor["entries"]["asset:video"]["descriptor"][field] = json!("forbidden");
        assert!(
            serde_json::from_value::<ResourceManifestEnvelopeWire>(at_descriptor).is_err(),
            "descriptor accepted {field}"
        );
    }
}

#[test]
fn descriptor_fields_cannot_cross_resource_kinds() {
    let mut video_with_audio_descriptor = valid_manifest();
    let audio_descriptor =
        video_with_audio_descriptor["entries"]["asset:audio"]["descriptor"].take();
    video_with_audio_descriptor["entries"]["asset:video"]["descriptor"] = audio_descriptor;
    assert!(
        serde_json::from_value::<ResourceManifestEnvelopeWire>(video_with_audio_descriptor)
            .is_err()
    );

    let mut font_with_motion_descriptor = valid_manifest();
    let motion_descriptor =
        font_with_motion_descriptor["entries"]["component:title"]["descriptor"].take();
    font_with_motion_descriptor["entries"]["font:inter"]["descriptor"] = motion_descriptor;
    assert!(
        serde_json::from_value::<ResourceManifestEnvelopeWire>(font_with_motion_descriptor)
            .is_err()
    );
}

#[test]
fn digest_is_required_lowercase_sha256() {
    for digest in [
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "sha256:aaaaaaaa",
        "sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "sha512:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        let mut manifest = valid_manifest();
        manifest["entries"]["asset:image"]["digest"] = json!(digest);
        assert!(serde_json::from_value::<ResourceManifestEnvelopeWire>(manifest).is_err());
    }

    let mut missing = valid_manifest();
    missing["entries"]["asset:image"]
        .as_object_mut()
        .unwrap()
        .remove("digest");
    assert!(serde_json::from_value::<ResourceManifestEnvelopeWire>(missing).is_err());
}

#[test]
fn content_digest_has_one_wire_spelling_and_explicit_path_hex() {
    let digest = ContentDigest::of_bytes(b"abc");
    let hex = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let wire = format!("sha256:{hex}");

    assert_eq!(digest.as_hex(), hex);
    assert_eq!(digest.to_wire(), wire);
    assert_eq!(digest.to_string(), wire);
    assert_eq!(digest.as_bytes().len(), 32);
    assert_eq!(ContentDigest::from_hex(hex).unwrap(), digest);
    assert_eq!(ContentDigest::from_bytes(*digest.as_bytes()), digest);
    assert_eq!(serde_json::to_value(digest).unwrap(), json!(wire));
    assert_eq!(
        serde_json::from_value::<ContentDigest>(json!(wire)).unwrap(),
        digest
    );

    for rejected in [
        hex,
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015a",
        "sha256:BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD",
        "sha512:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    ] {
        assert!(
            ContentDigest::parse(rejected).is_err(),
            "accepted {rejected}"
        );
        assert!(
            serde_json::from_value::<ContentDigest>(json!(rejected)).is_err(),
            "serde accepted {rejected}"
        );
    }

    for rejected_hex in [
        wire.as_str(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015a",
        "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD",
    ] {
        assert!(
            ContentDigest::from_hex(rejected_hex).is_err(),
            "from_hex accepted {rejected_hex}"
        );
    }
}

#[test]
fn embedded_contract_is_rejected_and_canonical_rational_shape_is_required() {
    let mut retired_contract = valid_manifest();
    retired_contract["contract"] = json!("valle.resource/manifest@1.0");
    assert!(serde_json::from_value::<ResourceManifestEnvelopeWire>(retired_contract).is_err());

    let mut time = valid_manifest();
    time["entries"]["asset:video"]["descriptor"]["duration"] = json!(60.0);
    assert!(serde_json::from_value::<ResourceManifestEnvelopeWire>(time).is_err());
}
