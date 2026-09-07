//! Crash recovery across import commit boundaries; repeated add and reindex repair stale indexes.

use std::path::Path;
use std::process::Command;

use valle_project::assets::add::{add, stream_sha256};
use valle_project::assets::{AddMode, AssetMeta, Ctx, Db, Home};

/// Child process aborts at the requested injection point.
#[test]
#[ignore = "child role: spawned by crash matrix"]
fn child_add_crash() {
    let home = Home::at(std::env::var("VALLE_HOME").expect("VALLE_HOME"));
    let src = std::env::var("ADD_SRC").expect("ADD_SRC");
    let ctx = Ctx::bare(home);
    // An injected abort never returns; a normal exit means the point was not reached.
    let _ = add(&ctx, Path::new(&src), AddMode::Reflink, None, None, &[]);
}

fn spawn_crash(home: &Path, src: &Path, label: &str) -> std::process::ExitStatus {
    let exe = std::env::current_exe().unwrap();
    Command::new(exe)
        .args(["child_add_crash", "--ignored", "--exact"])
        .env("VALLE_HOME", home)
        .env("ADD_SRC", src)
        .env("VALLE_ASSETS_CRASH_AT", label)
        .status()
        .unwrap()
}

#[test]
fn crash_matrix_no_partial_truth_and_self_heal() {
    let labels = [
        "blob-before-rename",
        "blob-after-rename",
        "meta-before-rename",
        "meta-after-rename",
    ];
    for label in labels {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("home");
        let src = tmp.path().join("v.mp4");
        std::fs::write(&src, format!("bytes-{label}")).unwrap();
        let hash = stream_sha256(&src).unwrap().as_hex();
        let home = Home::at(&home_dir);

        let status = spawn_crash(&home_dir, &src, label);
        assert!(!status.success(), "{label}: child process must abort");

        let meta_exists = home.meta_path(&hash).exists();
        match label {
            // Before metadata rename, the asset remains unregistered.
            "blob-before-rename" | "blob-after-rename" | "meta-before-rename" => {
                assert!(
                    !meta_exists,
                    "{label}: metadata must not exist before commit"
                );
            }
            // After metadata commit, the index may lag behind.
            "meta-after-rename" => {
                assert!(meta_exists, "{label}: metadata must exist after commit");
                let db = Db::open(&home).unwrap();
                let n: i64 = db
                    .conn
                    .query_row("SELECT count(*) FROM assets WHERE hash=?1", [&hash], |r| {
                        r.get(0)
                    })
                    .unwrap();
                assert_eq!(n, 0, "{label}: index must lag before upsert");
            }
            _ => unreachable!(),
        }

        // Queries tolerate an index awaiting repair.
        {
            let db = Db::open(&home).unwrap();
            let _: i64 = db
                .conn
                .query_row("SELECT count(*) FROM assets", [], |r| r.get(0))
                .unwrap();
        }

        // Repeating the import repairs the index idempotently.
        let ctx = Ctx::bare(Home::at(&home_dir));
        let out = add(&ctx, &src, AddMode::Reflink, None, None, &[]).unwrap();
        assert_eq!(out.content_digest.as_hex(), hash);
        assert!(home.meta_path(&hash).exists());
        let db = Db::open(&home).unwrap();
        let n: i64 = db
            .conn
            .query_row("SELECT count(*) FROM assets WHERE hash=?1", [&hash], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 1, "{label}: retry must repair the index");

        // Reindexing recreates the index from persisted metadata.
        std::fs::remove_file(home.index_db_path()).unwrap();
        let mut db = Db::open(&home).unwrap();
        let rebuilt = db.reindex_assets(&home).unwrap();
        assert_eq!(rebuilt, 1, "{label}: reindex must agree with metadata");
        let meta = AssetMeta::load(&home.meta_path(&hash)).unwrap();
        assert_eq!(meta.content_digest.as_hex(), hash);
    }
}
