#![cfg(feature = "schema")]

use std::{collections::BTreeSet, path::Path};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use valle_timeline::{
    internal::{
        schema::{
            generated_artifacts as generated_internal_artifacts,
            generated_schemas as generated_internal_schemas,
            generated_typescript as generated_internal_typescript,
        },
        wire::{document::TimelineDocumentEnvelopeWire, resource::ResourceManifestEnvelopeWire},
    },
    schema::{generated_artifacts, generated_schemas, generated_typescript, schema_bundle_pretty},
    wire::{
        edit::{EditTimelineRequestWire, EditTimelineResponseWire},
        timeline::TimelineWire,
    },
};

const EDIT_REVISION: u64 = 7;

fn document() -> Value {
    json!({
        "document": {
            "canvas": {
                "width": 1920,
                "height": 1080,
                "fps": "30/1",
                "sampleRate": 48000,
                "channelLayout": "stereo",
                "colorSpace": "srgb",
                "duration": "10/1"
            },
            "background": { "color": "#000000ff" },
            "visual": { "tracks": [] },
            "audio": { "tracks": [] },
            "adjustments": [],
            "captions": { "tracks": [] },
            "camera": null,
            "metadata": {}
        }
    })
}

fn manifest() -> Value {
    json!({
        "entries": {
            "asset:one": {
                "kind": "image",
                "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "descriptor": {
                    "width": 1,
                    "height": 1,
                    "orientation": "identity",
                    "color": {
                        "primaries": "srgb",
                        "transfer": "srgb",
                        "matrix": "identity",
                        "fullRange": true
                    }
                }
            }
        }
    })
}

fn edit_request() -> Value {
    json!({
        "baseRevision": EDIT_REVISION,
        "timeline": author_timeline(),
        "intent": "test edit"
    })
}

fn author_timeline() -> Value {
    json!({
        "canvas": { "width": 1920, "height": 1080, "fps": 30 },
        "tracks": {}
    })
}

fn edit_response() -> Value {
    json!({
        "outcome": "unchanged",
        "revision": EDIT_REVISION
    })
}

fn edit_committed_response() -> Value {
    json!({
        "outcome": "committed",
        "revision": EDIT_REVISION + 1
    })
}

fn edit_rejected_response() -> Value {
    json!({
        "outcome": "rejected",
        "errors": [{
                "code": "invalid_timeline",
                "path": "/timeline/canvas"
            }]
    })
}

fn assert_agree<T: DeserializeOwned>(schema_name: &str, instance: Value, expected: bool) {
    let schemas = generated_schemas()
        .into_iter()
        .chain(generated_internal_schemas())
        .collect::<std::collections::BTreeMap<_, _>>();
    let schema = &schemas[schema_name];
    jsonschema::draft202012::meta::validate(schema)
        .unwrap_or_else(|error| panic!("invalid generated schema {schema_name}: {error}"));
    let validator = jsonschema::draft202012::new(schema)
        .unwrap_or_else(|error| panic!("cannot compile {schema_name}: {error}"));
    let schema_accepts = validator.is_valid(&instance);
    let rust_accepts = serde_json::from_value::<T>(instance.clone()).is_ok();
    let errors = (!schema_accepts)
        .then(|| {
            validator
                .iter_errors(&instance)
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert_eq!(
        schema_accepts, expected,
        "schema result for {schema_name}: {errors:?}"
    );
    assert_eq!(
        rust_accepts, expected,
        "serde result for {schema_name}: {instance}"
    );
}

#[test]
fn generated_schema_and_rust_decode_share_the_checked_corpus() {
    assert_agree::<TimelineWire>("timeline.schema.json", author_timeline(), true);
    let mut explicit_empty_bands = author_timeline();
    explicit_empty_bands["tracks"] = json!({
        "visual": [],
        "audio": [],
        "caption": [],
        "adjustment": []
    });
    assert_agree::<TimelineWire>("timeline.schema.json", explicit_empty_bands, true);

    let mut with_version = author_timeline();
    with_version["version"] = json!(2);
    assert_agree::<TimelineWire>("timeline.schema.json", with_version, false);

    let mut array_tracks = author_timeline();
    array_tracks["tracks"] = json!([]);
    assert_agree::<TimelineWire>("timeline.schema.json", array_tracks, false);

    let mut missing_tracks = author_timeline();
    missing_tracks.as_object_mut().unwrap().remove("tracks");
    assert_agree::<TimelineWire>("timeline.schema.json", missing_tracks, false);

    for unsupported_band in ["subtitle", "subtitles", "effect"] {
        let mut timeline = author_timeline();
        timeline["tracks"][unsupported_band] = json!([]);
        assert_agree::<TimelineWire>("timeline.schema.json", timeline, false);
    }

    for unsupported_discriminant in ["type", "trackType"] {
        let mut timeline = author_timeline();
        let mut track = json!({ "clips": [] });
        track
            .as_object_mut()
            .unwrap()
            .insert(unsupported_discriminant.to_owned(), json!("visual"));
        timeline["tracks"]["visual"] = json!([track]);
        assert_agree::<TimelineWire>("timeline.schema.json", timeline, false);
    }

    assert_agree::<TimelineDocumentEnvelopeWire>("timeline-document.schema.json", document(), true);
    assert_agree::<EditTimelineRequestWire>(
        "timeline-edit-request.schema.json",
        edit_request(),
        true,
    );
    assert_agree::<EditTimelineResponseWire>(
        "timeline-edit-response.schema.json",
        edit_response(),
        true,
    );
    assert_agree::<EditTimelineResponseWire>(
        "timeline-edit-response.schema.json",
        edit_committed_response(),
        true,
    );
    assert_agree::<EditTimelineResponseWire>(
        "timeline-edit-response.schema.json",
        edit_rejected_response(),
        true,
    );
    assert_agree::<ResourceManifestEnvelopeWire>("resource-manifest.schema.json", manifest(), true);

    let mut missing_required_nullable = document();
    missing_required_nullable["document"]
        .as_object_mut()
        .unwrap()
        .remove("camera");
    assert_agree::<TimelineDocumentEnvelopeWire>(
        "timeline-document.schema.json",
        missing_required_nullable,
        false,
    );

    let mut missing_intent = edit_request();
    missing_intent.as_object_mut().unwrap().remove("intent");
    assert_agree::<EditTimelineRequestWire>(
        "timeline-edit-request.schema.json",
        missing_intent,
        true,
    );

    let mut unknown_outcome = edit_response();
    unknown_outcome["outcome"] = json!("merged");
    assert_agree::<EditTimelineResponseWire>(
        "timeline-edit-response.schema.json",
        unknown_outcome,
        false,
    );

    let mut wrong_edit_response_contract = edit_committed_response();
    wrong_edit_response_contract["contract"] = json!("valle.timeline/edit@1.0");
    assert_agree::<EditTimelineResponseWire>(
        "timeline-edit-response.schema.json",
        wrong_edit_response_contract,
        false,
    );

    let mut embedded_manifest_contract = manifest();
    embedded_manifest_contract["contract"] = json!("valle.resource/manifest@1.0");
    assert_agree::<ResourceManifestEnvelopeWire>(
        "resource-manifest.schema.json",
        embedded_manifest_contract,
        false,
    );

    let mut missing_image_width = manifest();
    missing_image_width["entries"]["asset:one"]["descriptor"]
        .as_object_mut()
        .unwrap()
        .remove("width");
    assert_agree::<ResourceManifestEnvelopeWire>(
        "resource-manifest.schema.json",
        missing_image_width,
        false,
    );
}

#[test]
fn checked_schema_artifacts_are_current() {
    let schema_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema");
    for (name, generated) in generated_internal_artifacts() {
        assert_eq!(
            std::fs::read_to_string(schema_dir.join(name)).unwrap(),
            generated,
            "checked internal artifact {name} drifted; run timeline schema-gen"
        );
    }
    for (name, generated) in generated_artifacts() {
        assert_eq!(
            std::fs::read_to_string(schema_dir.join(name)).unwrap(),
            generated,
            "checked public artifact {name} drifted; run timeline schema-gen"
        );
    }
    assert_eq!(
        generated_artifacts()
            .keys()
            .copied()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["timeline.schema-bundle.json", "timeline.generated.ts"])
    );
    assert_eq!(
        generated_internal_artifacts()
            .keys()
            .copied()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "timeline.internal.schema-bundle.json",
            "timeline.internal.generated.ts",
        ])
    );
}

#[test]
fn public_and_internal_artifacts_have_disjoint_roots() {
    assert_eq!(
        generated_schemas().keys().copied().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "timeline.schema.json",
            "timeline-edit-request.schema.json",
            "timeline-edit-response.schema.json",
        ])
    );
    assert_eq!(
        generated_internal_schemas()
            .keys()
            .copied()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "timeline-document.schema.json",
            "resource-manifest.schema.json",
        ])
    );

    let public_bundle = schema_bundle_pretty();
    let public_typescript = generated_typescript();
    for forbidden in [
        "\"contract\"",
        "updateResourceManifest",
        "TimelineDocument",
        "TimelineDraft",
    ] {
        assert!(
            !public_bundle.contains(forbidden),
            "public schema bundle leaked {forbidden}"
        );
        assert!(
            !public_typescript.contains(forbidden),
            "public TypeScript leaked {forbidden}"
        );
    }

    let internal_typescript = generated_internal_typescript();
    for forbidden in [
        "TimelineDraft",
        "RevisionCommittedEvent",
        "updateResourceManifest",
    ] {
        assert!(
            !internal_typescript.contains(forbidden),
            "internal TypeScript retained unused DTO {forbidden}"
        );
    }
}

#[test]
fn typescript_projections_keep_their_exact_wire_shapes() {
    let generated = generated_typescript();
    assert_eq!(
        generated,
        generated_typescript(),
        "projection is deterministic"
    );
    for literal in [
        "export type Timeline = TimelineSchema.Root",
        "export type EditTimelineRequest = EditTimelineRequestSchema.Root",
        "export type EditTimelineResponse = EditTimelineResponseSchema.Root",
        "\"outcome\": \"committed\"",
        "\"outcome\": \"staleBase\"",
        "export type TimelineVisualClipWire = ({ \"anchor\"?",
        "\"duration\": TimelineTimeWire",
        "\"start\": TimelineTimeWire",
        "export type TimelineAdjustmentClipWire = ({ \"duration\": TimelineTimeWire; \"start\": TimelineTimeWire } & { \"kind\": \"color-grade\"; \"temperature\": number })",
    ] {
        assert!(generated.contains(literal), "missing TS literal {literal}");
    }
    assert!(!generated.contains("TimelineVisualClipWire = (({"));
    assert!(!generated.contains("TimelineAdjustmentClipWire = ({ \"kind\""));
    assert!(!generated.contains("Record<string, unknown>"));

    let internal = generated_internal_typescript();
    assert_eq!(
        internal,
        generated_internal_typescript(),
        "internal projection is deterministic"
    );
    for literal in [
        "export type ContentDigest = string",
        "export type TimelineDocument = TimelineDocumentSchema.Root",
        "export type ResourceManifest = ResourceManifestSchema.Root",
    ] {
        assert!(
            internal.contains(literal),
            "missing internal TS literal {literal}"
        );
    }
    assert!(!internal.contains("\"contract\""));
    assert!(!internal.contains("valle.timeline/document@2.0"));
    assert!(!internal.contains("valle.resource/manifest@1.0"));
    assert!(!internal.contains("Sha256DigestWire"));
    assert!(!internal.contains("Record<string, unknown>"));
}
