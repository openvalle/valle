//! Search-unit projection rebuild, removal, resurrection and orphan exclusion tests.

use std::path::Path;

use valle_project::assets::add::add;
use valle_project::assets::annotate::annotate;
use valle_project::assets::knowledge::{edit, tag};
use valle_project::assets::maintain::{reindex, rm};
use valle_project::assets::{AddMode, Ctx, Db, Home, entity};

fn setup(tmp: &Path) -> (Ctx, String) {
    // SAFETY: Environment setup is serialized within the test process.
    unsafe { std::env::set_var("VALLE_ASSETS_NOW", "1784197800123") };
    let c = Ctx::bare(Home::at(tmp.join("home")));
    let src = tmp.join("v.mp4");
    std::fs::write(&src, b"unit-bytes").unwrap();
    let hash = add(&c, &src, AddMode::Reflink, None, None, &[])
        .unwrap()
        .content_digest
        .as_hex();
    (c, hash)
}

/// Dump units deterministically, excluding auto-incremented IDs.
fn dump_units(db: &Db) -> String {
    let mut stmt = db
        .conn
        .prepare(
            "SELECT hash, unit_kind, start_ms, end_ms, source, title, user_text, user_tags,
                    entity_names, raw
             FROM retrieval_units ORDER BY hash, unit_kind, start_ms, source",
        )
        .unwrap();
    let rows: Vec<String> = stmt
        .query_map([], |r| {
            Ok(format!(
                "{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<String>>(9)?,
            ))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    rows.join("\n")
}

#[test]
fn full_rebuild_is_equivalent() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, hash) = setup(tmp.path());

    entity::add(&c, "Alex", Some("person"), &["Al".into()]).unwrap();
    edit(&c, &hash[..8], Some("Park footage"), None).unwrap();
    tag(&c, &hash[..8], &["Promo".into()], &[]).unwrap();
    annotate(
        &c,
        &hash[..8],
        None,
        None,
        Some("Fully licensed"),
        &[],
        &[],
        None,
        None,
    )
    .unwrap();
    annotate(
        &c,
        &hash[..8],
        None,
        Some([83.0, 102.0]),
        Some("Alex and Sam at Central Park"),
        &["Opening candidate".into()],
        &["e1".into()],
        None,
        None,
    )
    .unwrap();

    let before = dump_units(&Db::open(&c.home).unwrap());
    assert!(before.contains("annotation"), "annotation unit must exist");
    assert!(before.contains("asset"), "asset unit must exist");

    // Reindexing after database removal must reproduce the same bytes.
    std::fs::remove_file(c.home.index_db_path()).unwrap();
    let stats = reindex(&c).unwrap();
    assert_eq!(stats.assets, 1);
    assert_eq!(stats.entities, 1);
    assert_eq!(stats.annotations, 2);
    let after = dump_units(&Db::open(&c.home).unwrap());
    assert_eq!(before, after, "full projection rebuild must be equivalent");
}

#[test]
fn units_vanish_on_rm_and_revive_on_re_add() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, hash) = setup(tmp.path());
    edit(&c, &hash[..8], Some("Restorable footage"), None).unwrap();
    annotate(
        &c,
        &hash[..8],
        None,
        Some([1.0, 2.0]),
        Some("Highlight"),
        &[],
        &[],
        None,
        None,
    )
    .unwrap();

    let n_before: i64 = Db::open(&c.home)
        .unwrap()
        .conn
        .query_row(
            "SELECT count(*) FROM retrieval_units WHERE hash=?1",
            [&hash],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n_before, 2, "expect one asset unit and one annotation unit");

    rm(&c, &hash[..8], false, false).unwrap();
    let n_removed: i64 = Db::open(&c.home)
        .unwrap()
        .conn
        .query_row(
            "SELECT count(*) FROM retrieval_units WHERE hash=?1",
            [&hash],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n_removed, 0, "removed assets must not be projected");

    // Reimport restores search units from retained metadata.
    let src = tmp.path().join("v.mp4");
    add(&c, &src, AddMode::Reflink, None, None, &[]).unwrap();
    let n_back: i64 = Db::open(&c.home)
        .unwrap()
        .conn
        .query_row(
            "SELECT count(*) FROM retrieval_units WHERE hash=?1",
            [&hash],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n_back, 2, "reimport must restore units");

    // Orphan analysis data without metadata must not produce units.
    let kdir = c.home.annotations_dir(&"ee".repeat(32));
    std::fs::create_dir_all(&kdir).unwrap();
    std::fs::write(
        kdir.join("n1.json"),
        r#"{"id":"n1","text":"Orphan","author":"x","created_at":"","updated_at":""}"#.as_bytes(),
    )
    .unwrap();
    reindex(&c).unwrap();
    let n_orphan: i64 = Db::open(&c.home)
        .unwrap()
        .conn
        .query_row(
            "SELECT count(*) FROM retrieval_units WHERE hash=?1",
            [&"ee".repeat(32)],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n_orphan, 0, "reindex must skip orphan analysis data");
}
