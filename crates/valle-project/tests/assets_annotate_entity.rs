//! Annotation states, persistence, purge protection, stable entity addressing and alias collision tests.

use std::path::Path;

use valle_project::assets::add::add;
use valle_project::assets::annotate::{annotate, load_all};
use valle_project::assets::maintain::rm;
use valle_project::assets::{AddMode, Ctx, ErrorCode, Home, entity};

fn setup(tmp: &Path) -> (Ctx, String) {
    // SAFETY: Environment setup is serialized within the test process.
    unsafe { std::env::set_var("VALLE_ASSETS_NOW", "1784197800123") };
    let c = Ctx::bare(Home::at(tmp.join("home")));
    let src = tmp.join("v.mp4");
    std::fs::write(&src, b"annotate-me").unwrap();
    let hash = add(&c, &src, AddMode::Reflink, None, None, &[])
        .unwrap()
        .content_digest
        .as_hex();
    (c, hash)
}

#[test]
fn three_states_and_persistence() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, hash) = setup(tmp.path());

    // Asset-level annotation without timestamps.
    annotate(
        &c,
        &hash[..8],
        None,
        None,
        Some("Entire clip usable"),
        &[],
        &[],
        None,
        None,
    )
    .unwrap();
    // Point marker with a start timestamp only.
    annotate(
        &c,
        &hash[..8],
        Some(12.5),
        None,
        Some("Highlight"),
        &[],
        &[],
        None,
        None,
    )
    .unwrap();
    // Range annotation with both timestamps.
    annotate(
        &c,
        &hash[..8],
        None,
        Some([83.0, 102.0]),
        Some("Alex and Sam at Central Park, suitable for an opening"),
        &["Opening candidate".into()],
        &[],
        None,
        None,
    )
    .unwrap();

    let all = load_all(&c.home, &hash).unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!((all[0].start_ms, all[0].end_ms), (None, None));
    assert_eq!((all[1].start_ms, all[1].end_ms), (Some(12500), None));
    assert_eq!(
        (all[2].start_ms, all[2].end_ms),
        (Some(83000), Some(102000))
    );
    assert_eq!(all[2].id, "n3");
    assert_eq!(all[2].author, "human-cli");

    // Reject conflicting timing arguments, reversed ranges and empty annotations.
    for bad in [
        annotate(
            &c,
            &hash[..8],
            Some(1.0),
            Some([2.0, 3.0]),
            Some("x"),
            &[],
            &[],
            None,
            None,
        ),
        annotate(
            &c,
            &hash[..8],
            None,
            Some([5.0, 2.0]),
            Some("x"),
            &[],
            &[],
            None,
            None,
        ),
        annotate(&c, &hash[..8], None, None, None, &[], &[], None, None),
    ] {
        assert_eq!(bad.unwrap_err().code, ErrorCode::BadQuery);
    }

    // Replacement preserves created_at and updates updated_at. SAFETY: Environment setup is serialized within the test process.
    unsafe { std::env::set_var("VALLE_ASSETS_NOW", "1784197900000") };
    annotate(
        &c,
        &hash[..8],
        None,
        Some([83.0, 105.0]),
        Some("Updated description"),
        &[],
        &[],
        Some("n3"),
        None,
    )
    .unwrap();
    let all = load_all(&c.home, &hash).unwrap();
    assert_eq!(all.len(), 3, "replacement must not add an annotation");
    assert_eq!(all[2].end_ms, Some(105_000));
    assert_eq!(all[2].created_at, "2026-07-16T10:30:00.123Z");
    assert_ne!(all[2].updated_at, all[2].created_at);

    // Delete the annotation.
    annotate(&c, &hash[..8], None, None, None, &[], &[], None, Some("n2")).unwrap();
    assert_eq!(load_all(&c.home, &hash).unwrap().len(), 2);
    // Annotation IDs increase monotonically and are not reused.
    let v = annotate(
        &c,
        &hash[..8],
        None,
        None,
        Some("New annotation"),
        &[],
        &[],
        None,
        None,
    )
    .unwrap();
    assert_eq!(v["annotation"]["id"], "n4");
}

#[test]
fn purge_blocked_when_annotated() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, hash) = setup(tmp.path());
    annotate(
        &c,
        &hash[..8],
        None,
        None,
        Some("Favorite"),
        &[],
        &[],
        None,
        None,
    )
    .unwrap();

    let err = rm(&c, &hash[..8], true, false).unwrap_err();
    assert_eq!(
        err.code,
        ErrorCode::Refused,
        "purging human annotations requires --force"
    );
    // Force permits the operation.
    assert_eq!(rm(&c, &hash[..8], true, true).unwrap().action, "purged");
    assert!(!c.home.annotations_dir(&hash).exists());
}

#[test]
fn entity_rename_keeps_annotations_and_collisions_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, hash) = setup(tmp.path());

    let e1 = entity::add(&c, "Alex", Some("person"), &["Alexander".into()]).unwrap();
    assert_eq!(e1["id"], "e1");
    // Attach the entity.
    annotate(
        &c,
        &hash[..8],
        None,
        Some([1.0, 2.0]),
        Some("Alex on screen"),
        &[],
        &["e1".into()],
        None,
        None,
    )
    .unwrap();
    // Unknown entities return not_found.
    let err = annotate(
        &c,
        &hash[..8],
        None,
        None,
        Some("x"),
        &[],
        &["e9".into()],
        None,
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);

    // Renaming and aliases preserve existing annotation links because they store entity IDs.
    entity::edit(
        &c,
        "e1",
        Some("Alex Smith"),
        Some(&["Alexander".into(), "Al".into()]),
    )
    .unwrap();
    let all = load_all(&c.home, &hash).unwrap();
    assert_eq!(all[0].entities[0].id, "e1");

    // Reject names that collide with existing names or aliases.
    for taken in ["Alex Smith", "Al"] {
        let err = entity::add(&c, taken, None, &[]).unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::Refused,
            "'{taken}' must cause a name collision"
        );
    }
    // Reject another entity's alias, allowing the entity's own alias.
    let e2 = entity::add(&c, "Sam", None, &[]).unwrap();
    assert_eq!(e2["id"], "e2");
    let err = entity::edit(&c, "e2", None, Some(&["Al".into()])).unwrap_err();
    assert_eq!(err.code, ErrorCode::Refused);
    entity::edit(&c, "e1", None, Some(&["Al".into()])).unwrap();
}
