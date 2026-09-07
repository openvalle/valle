//! Positive/negative corpus for the compact full-document edit wire.

use serde_json::{Value, json};
use valle_timeline::wire::edit::*;

const REVISION_A: u64 = 42;
const REVISION_B: u64 = 43;

fn timeline() -> Value {
    json!({
        "canvas": { "width": 1920, "height": 1080, "fps": "30000/1001" },
        "resources": { "logo": "https://example.com/logo.png" },
        "tracks": {
            "visual": [{
                "clips": [{
                    "start": 0,
                    "duration": 2,
                    "kind": "image",
                    "src": "logo"
                }]
            }]
        }
    })
}

fn request(intent: Option<&str>) -> Value {
    let mut value = json!({
        "baseRevision": REVISION_A,
        "timeline": timeline()
    });
    if let Some(intent) = intent {
        value["intent"] = json!(intent);
    }
    value
}

#[test]
fn request_is_one_complete_sparse_document_replacement() {
    for intent in [None, Some("Replace the title with final copy")] {
        let value = request(intent);
        let decoded: EditTimelineRequestWire = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    }
}

#[test]
fn all_four_response_outcomes_are_closed_and_round_trip() {
    let responses = [
        json!({ "outcome": "committed", "revision": REVISION_B }),
        json!({ "outcome": "unchanged", "revision": REVISION_A }),
        json!({ "outcome": "staleBase", "revision": REVISION_B }),
        json!({
            "outcome": "rejected",
                "errors": [{
                    "code": "track_overlap",
                    "path": "/timeline/tracks/0/clips/1"
                }]
        }),
    ];

    for value in responses {
        let decoded: EditTimelineResponseWire = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    }
}

#[test]
fn unknown_fields_and_outcomes_fail_closed() {
    for field in ["contract", "diffSummary", "receipt"] {
        let mut response = json!({ "outcome": "committed", "revision": REVISION_B });
        response[field] = json!("unsupported");
        assert!(serde_json::from_value::<EditTimelineResponseWire>(response).is_err());
    }

    let unknown = json!({
        "outcome": "partiallyCommitted",
        "revision": REVISION_B
    });
    assert!(serde_json::from_value::<EditTimelineResponseWire>(unknown).is_err());
}

#[test]
fn revisions_are_positive_safe_integers_not_hashes() {
    for base_revision in [
        json!(0),
        json!(9_007_199_254_740_992u64),
        json!("sha256:unsupported"),
    ] {
        let mut value = request(None);
        value["baseRevision"] = base_revision;
        assert!(serde_json::from_value::<EditTimelineRequestWire>(value).is_err());
    }
}

#[test]
fn patch_and_primitive_command_vocabulary_is_not_a_second_write_path() {
    for forbidden in [
        "patch",
        "operations",
        "primitiveEdits",
        "editTransaction",
        "clientRequestId",
        "expectedBaseRevisionId",
        "clips",
    ] {
        let mut value = request(None);
        value.as_object_mut().unwrap().remove("timeline");
        value[forbidden] = json!([]);
        assert!(
            serde_json::from_value::<EditTimelineRequestWire>(value).is_err(),
            "{forbidden} must not replace the full timeline"
        );
    }
}

#[test]
fn request_and_timeline_are_closed() {
    let mut unknown = request(None);
    unknown["merge"] = json!(true);
    assert!(serde_json::from_value::<EditTimelineRequestWire>(unknown).is_err());

    let mut versioned = request(None);
    versioned["timeline"]["version"] = json!(2);
    assert!(serde_json::from_value::<EditTimelineRequestWire>(versioned).is_err());

    let mut tagged_track = request(None);
    tagged_track["timeline"]["tracks"] = json!([{
        "type": "visual",
        "clips": []
    }]);
    assert!(serde_json::from_value::<EditTimelineRequestWire>(tagged_track).is_err());

    for retired_slot in ["subtitle", "subtitles", "effect"] {
        let mut retired = request(None);
        retired["timeline"]["tracks"][retired_slot] = json!([]);
        assert!(
            serde_json::from_value::<EditTimelineRequestWire>(retired).is_err(),
            "retired track slot {retired_slot} must fail closed"
        );
    }

    let mut missing_tracks = request(None);
    missing_tracks["timeline"]
        .as_object_mut()
        .unwrap()
        .remove("tracks");
    assert!(serde_json::from_value::<EditTimelineRequestWire>(missing_tracks).is_err());

    let mut old_document = request(None);
    old_document["timeline"] = json!({
        "contract": "valle.timeline/document@2.0",
        "document": {}
    });
    assert!(serde_json::from_value::<EditTimelineRequestWire>(old_document).is_err());
}
