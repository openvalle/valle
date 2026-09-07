#![cfg(feature = "schema")]

use serde_json::{Value, json};
use valle_timeline::{schema::generated_schemas, wire::timeline::TimelineWire};

fn author() -> Value {
    json!({
        "canvas": { "width": 1920, "height": 1080, "fps": "30000/1001" },
        "resources": { "font": "https://example.test/font.ttf" },
        "tracks": {
            "visual": [{
                "clips": [{
                    "start": 0.133333333333,
                    "duration": 1,
                    "kind": "solid",
                    "color": "#ffffffff",
                    "opacity": {
                        "keyframes": [[0.266666666666, 0.9299999999999999]]
                    }
                }]
            }],
            "adjustment": [{
                "clips": [{
                    "start": 0,
                    "duration": 1,
                    "kind": "color-grade",
                    "temperature": 0.25
                }]
            }],
            "caption": [{
                "style": { "font": "font" },
                "clips": [{ "start": 0, "duration": 1, "text": "caption" }]
            }]
        }
    })
}

fn assert_author_agrees(instance: Value, expected: bool) {
    let schema = &generated_schemas()["timeline.schema.json"];
    jsonschema::draft202012::meta::validate(schema).expect("valid author schema");
    let validator = jsonschema::draft202012::new(schema).expect("compiled author schema");
    assert_eq!(validator.is_valid(&instance), expected, "schema result");
    assert_eq!(
        serde_json::from_value::<TimelineWire>(instance).is_ok(),
        expected,
        "wire result"
    );
}

#[test]
fn author_root_schema_matches_closed_wire() {
    assert_author_agrees(author(), true);

    let mut unknown_visual_source_field = author();
    unknown_visual_source_field["tracks"]["visual"][0]["clips"][0]["bogus"] = json!(true);
    assert_author_agrees(unknown_visual_source_field, false);

    let mut source_field_from_another_variant = author();
    source_field_from_another_variant["tracks"]["visual"][0]["clips"][0]["src"] = json!("hero");
    assert_author_agrees(source_field_from_another_variant, false);

    let mut unsupported_version = author();
    unsupported_version["version"] = json!(2);
    assert_author_agrees(unsupported_version, false);

    let mut unsupported_track_type = author();
    unsupported_track_type["tracks"]["visual"][0]["type"] = json!("visual");
    assert_author_agrees(unsupported_track_type, false);

    let mut unsupported_track_type_name = author();
    unsupported_track_type_name["tracks"]["visual"][0]["trackType"] = json!("visual");
    assert_author_agrees(unsupported_track_type_name, false);

    let mut unsupported_track_list = author();
    unsupported_track_list["tracks"] = json!([{ "type": "visual", "clips": [] }]);
    assert_author_agrees(unsupported_track_list, false);

    for unsupported_slot in ["subtitle", "subtitles", "effect"] {
        let mut unsupported = author();
        unsupported["tracks"][unsupported_slot] = json!([]);
        assert_author_agrees(unsupported, false);
    }

    let missing_tracks = json!({
        "canvas": { "width": 1920, "height": 1080, "fps": 30 }
    });
    assert_author_agrees(missing_tracks, false);

    let empty_tracks = json!({
        "canvas": { "width": 1920, "height": 1080, "fps": 30 },
        "tracks": {}
    });
    assert_author_agrees(empty_tracks.clone(), true);
    let decoded: TimelineWire = serde_json::from_value(empty_tracks).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap()["tracks"], json!({}));

    let mut zero_rational_fps = author();
    zero_rational_fps["canvas"]["fps"] = json!("0/1");
    assert_author_agrees(zero_rational_fps, false);
}
