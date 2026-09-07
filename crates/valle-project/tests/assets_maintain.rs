//! Reindex equivalence, asset resurrection, stale references, garbage collection and read-only SQL tests.

use std::path::Path;

use valle_project::assets::add::{add, add_batch};
use valle_project::assets::maintain::{dump_assets_table, gc, reindex, rm, verify};
use valle_project::assets::sqlgate::sql;
use valle_project::assets::{AddMode, AssetMeta, Ctx, Db, ErrorCode, Home};

fn ctx(root: &Path) -> Ctx {
    // SAFETY: Environment setup is serialized within the test process.
    unsafe { std::env::set_var("VALLE_ASSETS_NOW", "1784197800123") };
    Ctx::bare(Home::at(root))
}

fn mk(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

#[test]
fn rebuild_gate_byte_identical() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(&tmp.path().join("home"));
    let paths = vec![
        mk(tmp.path(), "a.mp4", b"aa"),
        mk(tmp.path(), "b.png", b"bb"),
        mk(tmp.path(), "c.mp3", b"cc"),
    ];
    let items = add_batch(
        &c,
        &paths,
        AddMode::Reflink,
        None,
        Some("Batch"),
        &["t".into()],
    );
    assert!(items.iter().all(|i| i.outcome.is_some()));

    let before =
        serde_json::to_string(&dump_assets_table(&Db::open(&c.home).unwrap()).unwrap()).unwrap();
    std::fs::remove_file(c.home.index_db_path()).unwrap();
    reindex(&c).unwrap();
    let after =
        serde_json::to_string(&dump_assets_table(&Db::open(&c.home).unwrap()).unwrap()).unwrap();
    assert_eq!(
        before, after,
        "reindex must reproduce the same bytes after index.db removal"
    );
}

#[test]
fn rm_then_re_add_revives_knowledge() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(&tmp.path().join("home"));
    let src = mk(tmp.path(), "v.mp4", b"revive-me");
    let out = add(
        &c,
        &src,
        AddMode::Reflink,
        None,
        Some("Favorite footage"),
        &["licensed".into()],
    )
    .unwrap();
    let hash = out.content_digest.as_hex();

    let r = rm(&c, &hash[..8], false, false).unwrap();
    assert_eq!(r.action, "removed");
    assert_eq!(r.content_digest, out.content_digest);
    let wire = serde_json::to_value(&r).unwrap();
    assert_eq!(wire["content_digest"], out.content_digest.to_wire());
    assert!(wire.get("hash").is_none());
    // Remove blob bytes while retaining removal metadata, title and tags.
    assert!(!c.home.object_path(&hash, Some("mp4")).exists());
    let meta = AssetMeta::load(&c.home.meta_path(&hash)).unwrap();
    assert!(meta.is_removed());
    assert_eq!(meta.title.as_deref(), Some("Favorite footage"));
    assert_eq!(meta.tags, vec!["licensed".to_owned()]);
    // Removal is idempotent.
    assert_eq!(rm(&c, &hash[..8], false, false).unwrap().action, "removed");

    // Reimport restores bytes and clears removal status while preserving metadata.
    let back = add(&c, &src, AddMode::Reflink, None, None, &[]).unwrap();
    assert_eq!(back.content_digest, out.content_digest);
    assert!(back.revived);
    let meta = AssetMeta::load(&c.home.meta_path(&hash)).unwrap();
    assert!(!meta.is_removed());
    assert_eq!(meta.title.as_deref(), Some("Favorite footage"));
    assert_eq!(meta.tags, vec!["licensed".to_owned()]);
    assert!(c.home.object_path(&hash, Some("mp4")).exists());
}

#[test]
fn purge_deletes_everything() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(&tmp.path().join("home"));
    let src = mk(tmp.path(), "x.mp4", b"purge-me");
    let out = add(&c, &src, AddMode::Reflink, None, None, &[]).unwrap();
    let hash = out.content_digest.as_hex();
    // Create analysis data to verify purge removes it as well.
    let adir = c.home.analysis_dir(&hash);
    std::fs::create_dir_all(&adir).unwrap();
    std::fs::write(adir.join("shots@1.json"), b"{}").unwrap();

    let r = rm(&c, &hash[..8], true, false).unwrap();
    assert_eq!(r.action, "purged");
    assert!(!c.home.meta_path(&hash).exists());
    assert!(!adir.exists());
    let db = Db::open(&c.home).unwrap();
    let n: i64 = db
        .conn
        .query_row("SELECT count(*) FROM assets", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn verify_reports_stale_orphan_drift() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(&tmp.path().join("home"));
    // Modify a reference asset's source.
    let refsrc = mk(tmp.path(), "ref.mp4", b"ref-bytes");
    let items = add_batch(&c, &[refsrc.clone()], AddMode::Reference, None, None, &[]);
    let rhash = items[0].outcome.as_ref().unwrap().content_digest.as_hex();
    std::thread::sleep(std::time::Duration::from_millis(10));
    std::fs::write(&refsrc, b"tampered-longer").unwrap();

    // Create an orphan blob without metadata.
    let orphan_dir = c.home.objects_dir().join("ff");
    std::fs::create_dir_all(&orphan_dir).unwrap();
    std::fs::write(orphan_dir.join("deadbeef.bin"), b"orphan").unwrap();

    // Create orphan analysis data.
    let kdir = c
        .home
        .assets_dir()
        .join("annotations")
        .join("ee")
        .join("11");
    std::fs::create_dir_all(&kdir).unwrap();
    std::fs::write(kdir.join("n1.json"), b"{}").unwrap();

    let v = verify(&c, false).unwrap();
    let kinds: Vec<&str> = v.issues.iter().map(|i| i.kind).collect();
    assert!(
        kinds.contains(&"stale_reference"),
        "verify must report stale references: {kinds:?}"
    );
    assert!(
        kinds.contains(&"orphan_blob"),
        "verify must report orphan blobs: {kinds:?}"
    );
    assert!(
        kinds.contains(&"orphan_knowledge"),
        "verify must report orphan analysis data: {kinds:?}"
    );
    // Persist stale status in the query index.
    let db = Db::open(&c.home).unwrap();
    let stale: i64 = db
        .conn
        .query_row("SELECT stale FROM assets WHERE hash=?1", [&rhash], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(stale, 1);
}

#[test]
fn gc_removes_orphans_keeps_live() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(&tmp.path().join("home"));
    let src = mk(tmp.path(), "live.mp4", b"live");
    let out = add(&c, &src, AddMode::Reflink, None, None, &[]).unwrap();
    let hash = out.content_digest.as_hex();

    let orphan_dir = c.home.objects_dir().join("ff");
    std::fs::create_dir_all(&orphan_dir).unwrap();
    let orphan = orphan_dir.join("deadbeef.bin");
    std::fs::write(&orphan, b"orphan").unwrap();
    // Temporary-file residue.
    let tmp_res = c.home.objects_dir().join("ff").join(".x.tmp-999");
    std::fs::write(&tmp_res, b"tmp").unwrap();

    let g = gc(&c).unwrap();
    assert_eq!(g.removed_blobs, 1);
    assert_eq!(g.removed_tmp, 1);
    assert!(!orphan.exists());
    assert!(!tmp_res.exists());
    assert!(
        c.home.object_path(&hash, Some("mp4")).exists(),
        "live blobs must remain"
    );
}

#[test]
fn gc_fails_closed_when_meta_content_digest_disagrees_with_its_path() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(&tmp.path().join("home"));
    let src = mk(tmp.path(), "live.mp4", b"live-with-corrupt-meta");
    let out = add(&c, &src, AddMode::Reflink, None, None, &[]).unwrap();
    let hash = out.content_digest.as_hex();
    let blob = c.home.object_path(&hash, Some("mp4"));
    let meta_path = c.home.meta_path(&hash);
    let mut meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
    meta["content_digest"] = serde_json::json!(format!("sha256:{}", "f".repeat(64)));
    std::fs::write(&meta_path, serde_json::to_vec_pretty(&meta).unwrap()).unwrap();

    let error = gc(&c).unwrap_err();
    assert!(
        error
            .message
            .contains("content identity does not match path")
    );
    assert!(
        blob.is_file(),
        "invalid metadata must not cause GC to delete its blob as an orphan"
    );
}

#[test]
fn gc_fails_closed_when_meta_content_paths_are_not_bound_to_the_digest() {
    for corrupt_field in ["location", "companion"] {
        let tmp = tempfile::tempdir().unwrap();
        let c = ctx(&tmp.path().join(format!("home-{corrupt_field}")));
        let src = mk(
            tmp.path(),
            "live.mp4",
            format!("live-with-corrupt-{corrupt_field}").as_bytes(),
        );
        let out = add(&c, &src, AddMode::Reflink, None, None, &[]).unwrap();
        let hash = out.content_digest.as_hex();
        let blob = c.home.object_path(&hash, Some("mp4"));
        let meta_path = c.home.meta_path(&hash);
        let orphan_dir = c.home.objects_dir().join("ff");
        std::fs::create_dir_all(&orphan_dir).unwrap();
        let orphan = orphan_dir.join("deadbeef.bin");
        std::fs::write(&orphan, b"must survive rejected GC").unwrap();

        let mut meta: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
        match corrupt_field {
            "location" => {
                meta["locations"][0]["path"] = serde_json::json!("objects/ff/not-this-content.mp4");
            }
            "companion" => {
                meta["companion"] = serde_json::json!("objects/ff/not-this-content.keyframes.json");
            }
            _ => unreachable!(),
        }
        std::fs::write(&meta_path, serde_json::to_vec_pretty(&meta).unwrap()).unwrap();

        let error = gc(&c).unwrap_err();
        assert!(
            error.message.contains("does not match content identity"),
            "unexpected {corrupt_field} error: {}",
            error.message
        );
        assert!(
            blob.is_file(),
            "invalid metadata must not cause GC to delete a live blob"
        );
        assert!(
            orphan.is_file(),
            "GC must fail closed before deleting anything"
        );
    }
}

#[cfg(unix)]
#[test]
fn gc_never_follows_a_directory_symlink_while_sweeping_tmp_files() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(&tmp.path().join("home"));
    let orphan_dir = c.home.objects_dir().join("ff");
    std::fs::create_dir_all(&orphan_dir).unwrap();
    let local_orphan = orphan_dir.join("deadbeef.bin");
    std::fs::write(&local_orphan, b"keep until traversal is proven safe").unwrap();
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let victim = outside.join(".valuable.tmp-attacker");
    std::fs::write(&victim, b"keep").unwrap();
    symlink(&outside, c.home.objects_dir().join("linked-outside")).unwrap();

    let error = gc(&c).unwrap_err();
    assert!(error.message.contains("through symlink"));
    assert_eq!(std::fs::read(victim).unwrap(), b"keep");
    assert!(
        local_orphan.is_file(),
        "a rejected traversal must fail before deleting a valid candidate"
    );
}

#[cfg(unix)]
#[test]
fn gc_rejects_symlinked_destructive_roots_without_deleting_outside_or_local_files() {
    use std::os::unix::fs::symlink;

    for root_name in ["objects", "meta", "cache"] {
        let tmp = tempfile::tempdir().unwrap();
        let c = ctx(&tmp.path().join(format!("home-{root_name}")));
        std::fs::create_dir_all(c.home.assets_dir()).unwrap();

        let outside = tmp.path().join(format!("outside-{root_name}"));
        std::fs::create_dir_all(&outside).unwrap();
        let outside_victim = outside.join(".valuable.tmp-attacker");
        std::fs::write(&outside_victim, b"outside must survive").unwrap();

        let root = match root_name {
            "objects" => c.home.objects_dir(),
            "meta" => c.home.meta_dir(),
            "cache" => c.home.cache_dir(),
            _ => unreachable!(),
        };
        symlink(&outside, &root).unwrap();

        let local_orphan = if root_name == "objects" {
            None
        } else {
            let directory = c.home.objects_dir().join("ff");
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join("deadbeef.bin");
            std::fs::write(&path, b"local candidate must survive").unwrap();
            Some(path)
        };

        let error = gc(&c).unwrap_err();
        assert!(
            error.message.contains("must be a real directory"),
            "unexpected {root_name} error: {}",
            error.message
        );
        assert_eq!(
            std::fs::read(&outside_victim).unwrap(),
            b"outside must survive",
            "GC followed its {root_name} root symlink"
        );
        if let Some(local_orphan) = local_orphan {
            assert!(
                local_orphan.is_file(),
                "{root_name} validation failed after an earlier deletion"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn asset_write_lock_symlink_is_rejected_without_truncating_its_target() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(&tmp.path().join("home"));
    std::fs::create_dir_all(c.home.assets_dir()).unwrap();
    let outside = tmp.path().join("outside-lock");
    std::fs::write(&outside, b"keep").unwrap();
    symlink(&outside, c.home.lock_path()).unwrap();

    let error = gc(&c).unwrap_err();
    assert!(error.message.contains("regular file"));
    assert_eq!(std::fs::read(outside).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn gc_rejects_cache_descendant_symlink_before_applying_other_deletions() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(&tmp.path().join("home"));
    let orphan_dir = c.home.objects_dir().join("ff");
    std::fs::create_dir_all(&orphan_dir).unwrap();
    let local_orphan = orphan_dir.join("deadbeef.bin");
    std::fs::write(&local_orphan, b"keep until cache traversal is proven safe").unwrap();

    let outside = tmp.path().join("outside-cache");
    std::fs::create_dir_all(&outside).unwrap();
    let outside_victim = outside.join("cache.bin");
    std::fs::write(&outside_victim, b"outside must survive").unwrap();
    std::fs::create_dir_all(c.home.cache_dir()).unwrap();
    symlink(&outside, c.home.cache_dir().join("linked-area")).unwrap();

    let error = gc(&c).unwrap_err();
    assert!(error.message.contains("cache GC traversal through symlink"));
    assert_eq!(
        std::fs::read(outside_victim).unwrap(),
        b"outside must survive"
    );
    assert!(
        local_orphan.is_file(),
        "cache validation must finish before object deletion begins"
    );
}

#[test]
fn sql_readonly_gate() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(&tmp.path().join("home"));
    let src = mk(tmp.path(), "q.mp4", b"query-me");
    add(&c, &src, AddMode::Reflink, None, Some("Title"), &[]).unwrap();

    // SELECT succeeds.
    let out = sql(&c, "SELECT hash, title FROM assets").unwrap();
    assert_eq!(out.columns, vec!["hash", "title"]);
    assert_eq!(out.rows.len(), 1);
    assert_eq!(out.rows[0][1], serde_json::json!("Title"));

    // The read-only connection and authorizer reject writes.
    for bad in [
        "UPDATE assets SET title='x'",
        "DROP TABLE assets",
        "DELETE FROM assets",
    ] {
        let err = sql(&c, bad).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadQuery, "{bad} must be rejected");
    }
    // Leading SQL comments cannot bypass write protection.
    let err = sql(&c, "/* select */ INSERT INTO assets (hash) VALUES ('h')").unwrap_err();
    assert_eq!(err.code, ErrorCode::BadQuery);
}
