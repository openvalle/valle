#[path = "../src/canonical_json.rs"]
mod canonical_json;

use canonical_json::{CanonicalJsonError, canonicalize, parse_strict, to_canonical_bytes};

#[test]
fn jcs_orders_object_keys_by_utf16_and_preserves_array_order() {
    let canonical =
        canonicalize(br#"{"\uE000":1,"\uD83D\uDE00":2,"array":["third","first","second"]}"#)
            .unwrap();

    assert_eq!(
        String::from_utf8(canonical).unwrap(),
        "{\"array\":[\"third\",\"first\",\"second\"],\"😀\":2,\"\":1}"
    );
}

#[test]
fn jcs_uses_ecmascript_number_formatting_and_normalizes_negative_zero() {
    let canonical =
        canonicalize(br#"[333333333.33333329,1E30,4.50,2e-3,0.000000000000000000000000001,-0]"#)
            .unwrap();

    assert_eq!(
        String::from_utf8(canonical).unwrap(),
        "[333333333.3333333,1e+30,4.5,0.002,1e-27,0]"
    );
}

#[test]
fn duplicate_keys_are_rejected_recursively_before_map_construction() {
    let error = parse_strict(br#"{"outer":{"a":1,"\u0061":2}}"#).unwrap_err();

    assert_eq!(
        error,
        CanonicalJsonError::DuplicateObjectKey {
            key: "a".to_owned()
        }
    );
}

#[test]
fn utf8_bom_is_rejected() {
    let error = parse_strict(b"\xef\xbb\xbf{}").unwrap_err();

    assert_eq!(error, CanonicalJsonError::Utf8Bom);
}

#[test]
fn invalid_utf8_and_unpaired_surrogates_are_rejected() {
    assert_eq!(
        parse_strict(&[b'"', 0xed, 0xa0, 0x80, b'"']).unwrap_err(),
        CanonicalJsonError::InvalidUnicode
    );

    for json in [
        br#""\uD800""#.as_slice(),
        br#""\uDC00""#.as_slice(),
        br#""\uD800\u0041""#.as_slice(),
    ] {
        assert_eq!(
            parse_strict(json).unwrap_err(),
            CanonicalJsonError::InvalidUnicode
        );
    }

    assert_eq!(
        parse_strict(br#""\uD834\uDD1E""#).unwrap(),
        serde_json::Value::String("𝄞".to_owned())
    );
}

#[test]
fn safe_integer_boundaries_are_enforced_at_every_depth() {
    parse_strict(br#"[-9007199254740991,0,9007199254740991,1e30]"#).unwrap();

    for json in [
        br#"{"n":9007199254740992}"#.as_slice(),
        br#"{"nested":[-9007199254740992]}"#.as_slice(),
        br#"184467440737095516160000"#.as_slice(),
    ] {
        assert!(matches!(
            parse_strict(json),
            Err(CanonicalJsonError::UnsafeInteger { .. })
        ));
    }
}

#[test]
fn typed_values_and_metadata_cannot_bypass_safe_integer_validation() {
    #[derive(serde::Serialize)]
    struct TrustedValue {
        count: u64,
        metadata: serde_json::Value,
    }

    let unsafe_field = TrustedValue {
        count: 9_007_199_254_740_992,
        metadata: serde_json::json!({"safe": 1}),
    };
    assert!(matches!(
        to_canonical_bytes(&unsafe_field),
        Err(CanonicalJsonError::UnsafeInteger { .. })
    ));

    let unsafe_metadata = TrustedValue {
        count: 1,
        metadata: serde_json::json!({"nested": [9_007_199_254_740_992_u64]}),
    };
    assert!(matches!(
        to_canonical_bytes(&unsafe_metadata),
        Err(CanonicalJsonError::UnsafeInteger { .. })
    ));
}

#[test]
fn non_finite_and_malformed_numbers_are_rejected() {
    for json in [
        b"NaN".as_slice(),
        b"Infinity".as_slice(),
        b"-Infinity".as_slice(),
    ] {
        assert!(parse_strict(json).is_err());
    }
    assert_eq!(
        parse_strict(b"1e400").unwrap_err(),
        CanonicalJsonError::NonFiniteNumber
    );

    assert_eq!(
        to_canonical_bytes(&f64::NAN).unwrap_err(),
        CanonicalJsonError::Encode
    );
}

#[test]
fn trailing_json_values_are_rejected() {
    assert_eq!(
        parse_strict(br#"{} []"#).unwrap_err(),
        CanonicalJsonError::MalformedJson
    );
}
