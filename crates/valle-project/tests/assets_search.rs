//! Model-free CJK search covers descriptions, entities, tags, aliases, character prefixes, filters and removed assets.

use std::path::Path;

use valle_project::assets::add::add;
use valle_project::assets::annotate::annotate;
use valle_project::assets::knowledge::{edit, tag};
use valle_project::assets::maintain::rm;
use valle_project::assets::search::search;
use valle_project::assets::{AddMode, AssetKind, Ctx, Home, Probe, ProbeOutcome, Prober, entity};

struct FakeProber(i64);
impl Prober for FakeProber {
    fn probe(&self, _p: &Path, _k: AssetKind) -> valle_project::assets::Result<ProbeOutcome> {
        Ok(ProbeOutcome {
            probe: Some(Probe {
                duration_ms: Some(self.0),
                width: Some(1080),
                height: Some(1920),
                ..Default::default()
            }),
            warnings: vec![],
        })
    }
}

fn ctx(root: &Path, dur_ms: i64) -> Ctx {
    // SAFETY: Environment setup is serialized within the test process.
    unsafe { std::env::set_var("VALLE_ASSETS_NOW", "1784197800123") };
    let mut c = Ctx::bare(Home::at(root));
    c.prober = Box::new(FakeProber(dur_ms));
    c
}

fn mk(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

/// Two assets match the same CJK query through description text or entity/tag metadata.
fn stage(tmp: &Path) -> (Ctx, String, String) {
    let c = ctx(&tmp.join("home"), 30_000);
    let a = add(
        &c,
        &mk(tmp, "a.mp4", b"asset-a"),
        AddMode::Reflink,
        None,
        None,
        &[],
    )
    .unwrap()
    .content_digest
    .as_hex();
    let b = add(
        &c,
        &mk(tmp, "b.mp4", b"asset-b"),
        AddMode::Reflink,
        None,
        None,
        &[],
    )
    .unwrap()
    .content_digest
    .as_hex();
    entity::add(&c, "小明", Some("person"), &["明哥".into()]).unwrap();
    // Asset A matches through annotation text.
    annotate(
        &c,
        &a[..8],
        None,
        Some([83.0, 102.0]),
        Some("小明和小红在世纪公园散步,适合开场"),
        &[],
        &[],
        None,
        None,
    )
    .unwrap();
    // Asset B matches through entity names and tags.
    annotate(
        &c,
        &b[..8],
        None,
        Some([5.0, 9.0]),
        Some("主角出镜"),
        &["公园".into()],
        &["e1".into()],
        None,
        None,
    )
    .unwrap();
    (c, a, b)
}

#[test]
fn three_way_hits_reproducible() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, a, b) = stage(tmp.path());

    let out = search(&c, "小明 公园", None, None, None, None, None).unwrap();
    let results = out["results"].as_array().unwrap();
    assert_eq!(results.len(), 2, "both assets must match: {out}");

    // Evidence fields identify the matching description, entity or tag.
    let fields: Vec<String> = results
        .iter()
        .flat_map(|r| r["evidence"].as_array().unwrap().iter())
        .map(|e| e["field"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        fields.contains(&"user_text".to_owned()),
        "description evidence: {fields:?}"
    );
    assert!(
        fields.contains(&"entity_names".to_owned()) || fields.contains(&"user_tags".to_owned()),
        "entity/tag evidence: {fields:?}"
    );

    // Results include time anchors and resource paths usable by a Timeline.
    let ra = results
        .iter()
        .find(|r| r["asset"] == serde_json::json!(a))
        .unwrap();
    assert_eq!(ra["range"], serde_json::json!([83.0, 102.0]));
    assert!(ra["path"].as_str().unwrap().contains("objects"));
    assert!(ra.get("use").is_none());

    // Entity matches rank above description matches.
    assert_eq!(
        results[0]["asset"],
        serde_json::json!(b),
        "entity matches must rank first: {out}"
    );
}

#[test]
fn alias_and_single_char_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, _a, b) = stage(tmp.path());

    // Entity aliases are searchable.
    let out = search(&c, "明哥", None, None, None, None, None).unwrap();
    let results = out["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["asset"], serde_json::json!(b));

    // A single CJK character matches description and entity prefixes.
    let out = search(&c, "明", None, None, None, None, None).unwrap();
    assert_eq!(out["results"].as_array().unwrap().len(), 2, "{out}");
}

#[test]
fn filters_and_removed_exclusion() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, a, b) = stage(tmp.path());
    edit(&c, &a[..8], Some("公园素材甲"), None).unwrap();
    tag(&c, &a[..8], &["已授权".into()], &[]).unwrap();

    // Apply tag filters before searching; every result must belong to asset A.
    let out = search(&c, "公园", None, Some("已授权"), None, None, None).unwrap();
    let results = out["results"].as_array().unwrap();
    assert!(!results.is_empty());
    assert!(
        results.iter().all(|r| r["asset"] == serde_json::json!(a)),
        "only asset A remains after filtering: {out}"
    );

    // A 60-second minimum excludes the 30-second fake assets.
    let out = search(&c, "公园", None, None, None, Some("min-dur=60"), None).unwrap();
    assert_eq!(out["results"].as_array().unwrap().len(), 0);
    // Translate the portrait-orientation query term into a structural filter and search the remaining terms.
    let out = search(&c, "竖屏 公园", None, None, None, None, None).unwrap();
    let distinct: std::collections::HashSet<&str> = out["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["asset"].as_str().unwrap())
        .collect();
    assert_eq!(distinct.len(), 2, "{out}");

    // Removed assets are excluded.
    rm(&c, &b[..8], false, false).unwrap();
    let out = search(&c, "公园", None, None, None, None, None).unwrap();
    let hashes: Vec<&str> = out["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["asset"].as_str().unwrap())
        .collect();
    assert!(
        !hashes.contains(&b.as_str()),
        "removed assets must not appear"
    );

    // A filter-only query lists matching asset units.
    let out = search(&c, "", None, Some("已授权"), None, None, None).unwrap();
    assert_eq!(out["results"].as_array().unwrap().len(), 1);
}
