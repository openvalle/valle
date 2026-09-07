//! Domain-level full-document edit decoding tests.

use serde_json::{Value, json};
use valle_timeline::{EditDecodeError, EditJsonIssue, decode_edit_request};

const BASE_REVISION: u64 = 42;

fn request(timeline: Value) -> Value {
    json!({
        "baseRevision": BASE_REVISION,
        "timeline": timeline,
    })
}

fn sparse_timeline() -> Value {
    json!({
        "canvas": { "width": 1920, "height": 1080, "fps": 30 },
        "tracks": {}
    })
}

#[test]
fn edit_decode_is_strict_and_shape_only() {
    let mut locally_invalid = sparse_timeline();
    locally_invalid["canvas"]["width"] = json!(0);
    let wire = decode_edit_request(&serde_json::to_string(&request(locally_invalid)).unwrap())
        .expect("shape decode must not run Timeline local invariants");
    assert_eq!(wire.timeline.canvas.width, 0);

    let duplicate = format!(
        r#"{{"baseRevision":{BASE_REVISION},"timeline":{{}},"intent":null,"intent":null}}"#
    );
    assert!(matches!(
        decode_edit_request(&duplicate),
        Err(EditDecodeError::CanonicalJson {
            issue: EditJsonIssue::DuplicateObjectKey,
            ..
        })
    ));

    let mut unknown_field = request(sparse_timeline());
    unknown_field["patch"] = json!([]);
    assert_eq!(
        decode_edit_request(&serde_json::to_string(&unknown_field).unwrap()),
        Err(EditDecodeError::InvalidShape)
    );

    let malformed = decode_edit_request("{").unwrap_err();
    assert!(matches!(
        malformed,
        EditDecodeError::CanonicalJson {
            issue: EditJsonIssue::MalformedJson,
            ..
        }
    ));
    assert!(!malformed.to_string().contains("line"));
    assert!(!malformed.to_string().contains("column"));
}

#[test]
fn edit_decode_rejects_noncanonical_json_before_wire_shape() {
    let unsafe_integer =
        format!(r#"{{"baseRevision":{BASE_REVISION},"timeline":{{}},"extra":9007199254740992}}"#);
    assert!(matches!(
        decode_edit_request(&unsafe_integer),
        Err(EditDecodeError::CanonicalJson {
            issue: EditJsonIssue::UnsafeInteger,
            ..
        })
    ));

    let missing_timeline = format!(r#"{{"baseRevision":{BASE_REVISION}}}"#);
    assert_eq!(
        decode_edit_request(&missing_timeline),
        Err(EditDecodeError::InvalidShape)
    );
}
