use valle_timeline::internal::identity::{IdentityParseError, RENDER_ID_DOMAIN, RenderId};

#[test]
fn render_id_wire_has_one_exact_spelling() {
    let render_id = RenderId::from_canonical_bytes(br#"{"a":1}"#);
    let wire = render_id.to_string();
    assert!(wire.starts_with("sha256:"));
    assert_eq!(wire.len(), 71);
    assert_eq!(RenderId::parse(&wire).unwrap(), render_id);
    assert_eq!(
        serde_json::to_string(&render_id).unwrap(),
        format!(r#""{wire}""#)
    );
    assert_eq!(
        serde_json::from_str::<RenderId>(&format!(r#""{wire}""#)).unwrap(),
        render_id
    );

    let bare = wire.trim_start_matches("sha256:");
    assert_eq!(
        RenderId::parse(bare),
        Err(IdentityParseError::MissingSha256Prefix)
    );
    assert_eq!(
        RenderId::parse(&wire.to_uppercase()),
        Err(IdentityParseError::MissingSha256Prefix)
    );
    assert_eq!(
        RenderId::parse("sha256:abc"),
        Err(IdentityParseError::InvalidDigest)
    );
}

#[test]
fn render_id_domain_tag_has_exact_spelling() {
    assert_eq!(RENDER_ID_DOMAIN, b"valle.render/1\0");
}
