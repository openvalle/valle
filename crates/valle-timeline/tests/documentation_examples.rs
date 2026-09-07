use valle_timeline::{
    decode_timeline,
    wire::{edit::EditTimelineResponseWire, timeline::TimelineWire},
};

#[cfg(feature = "schema")]
use valle_timeline::schema::schema_bundle_pretty;

fn json_example(name: &str) -> &'static str {
    match name {
        "timeline" => include_str!("fixtures/timeline.json"),
        "edit-request" => include_str!("fixtures/edit-request.json"),
        "edit-response" => include_str!("fixtures/edit-response.json"),
        _ => panic!("unknown contract fixture `{name}`"),
    }
}

#[test]
fn contract_fixtures_decode_with_their_closed_contracts() {
    serde_json::from_str::<TimelineWire>(json_example("timeline"))
        .expect("the fixture Timeline must pass the public wire decoder");
    decode_timeline(json_example("timeline"))
        .expect("the fixture Timeline must pass the production author decoder");
    serde_json::from_str::<EditTimelineResponseWire>(json_example("edit-response"))
        .expect("the fixture edit response must pass the public wire decoder");
}

#[cfg(feature = "schema")]
#[test]
fn contract_fixtures_pass_the_generated_json_schemas() {
    // The checked bundle is an artifact envelope with generator metadata, not one root JSON
    // Schema. Compile the exact per-contract roots nested under `schemas`; schema_contract.rs
    // separately proves that this generated string is byte-identical to the checked artifact.
    let bundle: serde_json::Value =
        serde_json::from_str(&schema_bundle_pretty()).expect("generated schema bundle is JSON");
    let schemas = bundle
        .get("schemas")
        .and_then(serde_json::Value::as_object)
        .expect("generated schema bundle contains root schemas");

    for (fixture_name, schema_name) in [
        ("timeline", "timeline.schema.json"),
        ("edit-request", "timeline-edit-request.schema.json"),
        ("edit-response", "timeline-edit-response.schema.json"),
    ] {
        let schema = schemas
            .get(schema_name)
            .unwrap_or_else(|| panic!("generated schema bundle is missing `{schema_name}`"));
        jsonschema::draft202012::meta::validate(schema)
            .unwrap_or_else(|error| panic!("invalid generated schema `{schema_name}`: {error}"));
        let validator = jsonschema::draft202012::new(schema)
            .unwrap_or_else(|error| panic!("cannot compile `{schema_name}`: {error}"));
        let instance: serde_json::Value = serde_json::from_str(json_example(fixture_name))
            .unwrap_or_else(|error| {
                panic!("contract fixture `{fixture_name}` is not JSON: {error}")
            });
        let errors = validator
            .iter_errors(&instance)
            .map(|error| error.to_string())
            .collect::<Vec<_>>();
        assert!(
            errors.is_empty(),
            "contract fixture `{fixture_name}` failed `{schema_name}`: {errors:?}"
        );
    }
}
