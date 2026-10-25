use serde_json::{Value, json};
use std::{fs, path::Path};
use valle_project::assets::{Context, NoInspector};

fn context(root: &Path) -> Context {
    Context {
        root: root.to_path_buf(),
        inspector: Box::new(NoInspector),
    }
}
fn run(ctx: &Context, command: Value) -> Value {
    let result = ctx.execute(serde_json::from_value(command).unwrap());
    assert_eq!(result["status"], "ok", "{result}");
    result
}
fn list(ctx: &Context) -> Value {
    run(ctx, json!({"op":"list"}))
}

#[test]
fn missing_names_are_reserved_and_case_only_replacements_keep_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(&temp.path().join("library"));
    fs::create_dir_all(ctx.root.join("pack")).unwrap();
    fs::write(ctx.root.join("pack/A.bin"), b"old").unwrap();
    run(&ctx, json!({"op":"tag","path":"pack/A.bin","add":["keep"]}));
    fs::remove_file(ctx.root.join("pack/A.bin")).unwrap();
    assert_eq!(list(&ctx)["total"], 0);
    let source = temp.path().join("A.bin");
    fs::write(&source, b"new").unwrap();
    let added = run(&ctx, json!({"op":"add","paths":[source],"to":"pack"}));
    assert_eq!(added["items"][0]["path"], "pack/A-2.bin");
    let rejected = ctx.execute(
        serde_json::from_value(json!({"op":"edit","path":"pack/A-2.bin","name":"a.bin"})).unwrap(),
    );
    assert_eq!(rejected["status"], "error");
    assert!(ctx.root.join("pack/A-2.bin").exists());
    fs::write(
        ctx.root.join("pack/a.bin"),
        b"replacement with different bytes",
    )
    .unwrap();
    let shown = run(&ctx, json!({"op":"show","path":"pack/a.bin"}));
    assert_eq!(shown["tags"], json!(["keep"]));
    assert_eq!(list(&ctx)["total"], 2);
}

#[test]
fn ordinary_legacy_named_directories_and_index_files_are_allowed() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(&temp.path().join("library"));
    fs::create_dir_all(ctx.root.join("objects")).unwrap();
    fs::create_dir_all(ctx.root.join("meta")).unwrap();
    fs::write(
        ctx.root.join("index.db"),
        b"ordinary asset, not an old catalog",
    )
    .unwrap();
    fs::write(ctx.root.join("objects/item.bin"), b"asset").unwrap();
    assert_eq!(list(&ctx)["total"], 2);
    assert_eq!(list(&ctx)["total"], 2);
}

#[test]
fn media_inspection_does_not_hold_the_catalog_write_lock() {
    struct LockProbe;
    impl valle_project::assets::Inspector for LockProbe {
        fn inspect(&self, path: &Path, _kind: valle_project::files::Kind) -> Value {
            let conn = rusqlite::Connection::open(path.parent().unwrap().join(".valle/catalog.db"))
                .unwrap();
            conn.busy_timeout(std::time::Duration::ZERO).unwrap();
            conn.execute_batch("BEGIN IMMEDIATE; ROLLBACK;")
                .expect("scanning must not hold a write transaction during inspection");
            Value::Null
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let ctx = Context {
        root: temp.path().join("library"),
        inspector: Box::new(LockProbe),
    };
    fs::create_dir_all(&ctx.root).unwrap();
    fs::write(ctx.root.join("new.bin"), b"new").unwrap();
    assert_eq!(list(&ctx)["total"], 1);
}

#[test]
fn case_only_renames_and_directory_spelling_preserve_one_asset_namespace() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(&temp.path().join("library"));
    let a = temp.path().join("one.bin");
    let b = temp.path().join("two.bin");
    fs::write(&a, b"one").unwrap();
    fs::write(&b, b"two").unwrap();
    run(
        &ctx,
        json!({"op":"add","paths":[a],"to":"Brand/Acme","tags":["keep"]}),
    );
    let second = run(&ctx, json!({"op":"add","paths":[b],"to":"brand/acme"}));
    assert_eq!(second["items"][0]["path"], "Brand/Acme/two.bin");
    run(
        &ctx,
        json!({"op":"edit","path":"brand/acme/ONE.BIN","name":"ONE.bin"}),
    );
    let shown = run(&ctx, json!({"op":"show","path":"Brand/Acme/ONE.bin"}));
    assert_eq!(shown["tags"], json!(["keep"]));
    assert_eq!(
        run(&ctx, json!({"op":"list","directory":"brand/acme"}))["total"],
        2
    );
    run(
        &ctx,
        json!({"op":"edit","path":"Brand/Acme/ONE.bin","name":"one.csv"}),
    );
    assert_eq!(
        run(&ctx, json!({"op":"show","path":"Brand/Acme/one.csv"}))["kind"],
        "data"
    );
    let shader = temp.path().join("effect.shader.json");
    fs::write(&shader, b"{}").unwrap();
    let imported = run(&ctx, json!({"op":"add","paths":[shader]}));
    assert_eq!(imported["items"][0]["kind"], "shader");
    assert_eq!(imported["items"][0]["path"], "shaders/effect.shader.json");
}

#[test]
fn primary_constraints_cannot_be_rebuilt_as_if_they_were_derived_data() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(&temp.path().join("library"));
    fs::create_dir_all(&ctx.root).unwrap();
    fs::write(ctx.root.join("take.txt"), b"take").unwrap();
    run(
        &ctx,
        json!({"op":"tag","path":"take.txt","add":["preserve"]}),
    );
    let conn = rusqlite::Connection::open(ctx.root.join(".valle/catalog.db")).unwrap();
    conn.execute_batch("ALTER TABLE tags RENAME TO old_tags; CREATE TABLE tags(asset_id INTEGER NOT NULL,tag TEXT NOT NULL,PRIMARY KEY(asset_id,tag)); INSERT INTO tags SELECT * FROM old_tags; DROP TABLE old_tags;").unwrap();
    drop(conn);
    let report = ctx.execute(serde_json::from_value(json!({"op":"list"})).unwrap());
    assert_eq!(report["status"], "error");
    let conn = rusqlite::Connection::open(ctx.root.join(".valle/catalog.db")).unwrap();
    assert_eq!(
        conn.query_row("SELECT tag FROM tags", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "preserve"
    );
}

#[test]
fn imports_are_independent_deduplicated_and_notes_merge_without_exposing_identity() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(&temp.path().join("library"));
    let source = temp.path().join(if cfg!(windows) {
        "first file take.bin"
    } else {
        "first file: take.bin"
    });
    fs::write(&source, b"original").unwrap();
    let first = run(
        &ctx,
        json!({"op":"add","paths":[source],"tags":["品牌"],"note":"原始素材"}),
    );
    let path = first["items"][0]["path"].as_str().unwrap();
    assert_eq!(
        path,
        if cfg!(windows) {
            "other/first file take.bin"
        } else {
            "other/first file: take.bin"
        }
    );
    assert!(first["items"][0].get("digest").is_none());
    assert!(first["items"][0].get("asset_id").is_none());
    let duplicate = temp.path().join("other.bin");
    fs::copy(&source, &duplicate).unwrap();
    let again = run(
        &ctx,
        json!({"op":"add","paths":[duplicate],"tags":["收藏"],"note":"原始素材"}),
    );
    assert_eq!(again["items"][0]["path"], path);
    assert_eq!(again["items"][0]["alreadyExists"], true);
    let show = run(&ctx, json!({"op":"show","path":path}));
    assert_eq!(show["tags"].as_array().unwrap().len(), 2);
    assert_eq!(show["notes"].as_array().unwrap().len(), 1);
    fs::write(&source, b"changed!").unwrap();
    assert_eq!(fs::read(ctx.root.join(path)).unwrap(), b"original");
    let search = run(&ctx, json!({"op":"search","query":"原始素材"}));
    assert_eq!(search["total"], 1);
}

#[test]
fn finder_moves_keep_the_right_notes_when_duplicate_files_exist() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(&temp.path().join("library"));
    fs::create_dir_all(ctx.root.join("pack")).unwrap();
    for name in ["a.bin", "b.bin"] {
        fs::write(ctx.root.join("pack").join(name), b"same bytes").unwrap();
    }
    assert_eq!(list(&ctx)["total"], 2);
    run(
        &ctx,
        json!({"op":"tag","path":"pack/a.bin","add":["belongs-to-a"]}),
    );
    run(
        &ctx,
        json!({"op":"annotate","path":"pack/a.bin","at":45,"text":"进高潮"}),
    );
    fs::rename(ctx.root.join("pack/a.bin"), ctx.root.join("pack/moved.bin")).unwrap();
    let moved = run(&ctx, json!({"op":"show","path":"pack/moved.bin"}));
    assert_eq!(moved["tags"], json!(["belongs-to-a"]));
    assert_eq!(moved["notes"][0]["at"].as_f64(), Some(45.0));
    let other = run(&ctx, json!({"op":"show","path":"pack/b.bin"}));
    assert_eq!(other["tags"], json!([]));
    fs::copy(
        ctx.root.join("pack/moved.bin"),
        ctx.root.join("pack/copy.bin"),
    )
    .unwrap();
    assert_eq!(list(&ctx)["total"], 3);
    let copy = run(&ctx, json!({"op":"show","path":"pack/copy.bin"}));
    assert_eq!(copy["tags"], json!([]));
    fs::remove_file(ctx.root.join("pack/moved.bin")).unwrap();
    assert_eq!(list(&ctx)["total"], 2);
    let missing = run(&ctx, json!({"op":"list","missing":true}));
    assert_eq!(missing["items"][0]["notes"][0]["text"], "进高潮");
    run(&ctx, json!({"op":"remove","missing":true}));
    assert_eq!(run(&ctx, json!({"op":"list","missing":true}))["total"], 0);
}

#[test]
fn unique_digest_can_reattach_a_moved_copy_but_ambiguous_duplicates_stay_missing() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(&temp.path().join("library"));
    fs::create_dir_all(&ctx.root).unwrap();
    fs::write(ctx.root.join("old.txt"), b"unique").unwrap();
    run(&ctx, json!({"op":"tag","path":"old.txt","add":["keep"]}));
    fs::copy(ctx.root.join("old.txt"), ctx.root.join("new.txt")).unwrap();
    fs::remove_file(ctx.root.join("old.txt")).unwrap();
    assert_eq!(
        run(&ctx, json!({"op":"show","path":"new.txt"}))["tags"],
        json!(["keep"])
    );
    fs::copy(ctx.root.join("new.txt"), ctx.root.join("one.txt")).unwrap();
    fs::copy(ctx.root.join("new.txt"), ctx.root.join("two.txt")).unwrap();
    fs::remove_file(ctx.root.join("new.txt")).unwrap();
    assert_eq!(list(&ctx)["total"], 2);
    assert_eq!(
        run(&ctx, json!({"op":"show","path":"one.txt"}))["tags"],
        json!([])
    );
    assert_eq!(
        run(&ctx, json!({"op":"list","missing":true}))["items"][0]["tags"],
        json!(["keep"])
    );
}

#[test]
fn rename_collisions_partial_imports_and_directory_structure_are_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(&temp.path().join("library"));
    let a = temp.path().join("Clip.bin");
    let b = temp.path().join("clip.bin");
    fs::write(&a, b"a").unwrap();
    // Separate parent also works on a case-insensitive filesystem.
    fs::create_dir(temp.path().join("second")).unwrap();
    let b = if a == b {
        b
    } else {
        temp.path().join("second/clip.bin")
    };
    fs::write(&b, b"b").unwrap();
    run(&ctx, json!({"op":"add","paths":[a,b]}));
    let items = list(&ctx);
    assert_eq!(items["total"], 2);
    assert!(items.to_string().contains("clip-2.bin"));
    let collision = ctx.execute(
        serde_json::from_value(json!({"op":"edit","path":"other/Clip.bin","name":"clip-2.bin"}))
            .unwrap(),
    );
    assert_eq!(collision["status"], "error");
    run(
        &ctx,
        json!({"op":"edit","path":"other/Clip.bin","to":"brand/acme","name":"Hero.bin"}),
    );
    assert!(ctx.root.join("brand/acme/Hero.bin").is_file());
    let pack = temp.path().join("sounds");
    fs::create_dir_all(pack.join("whoosh")).unwrap();
    fs::write(pack.join("whoosh/wind.bin"), b"wind").unwrap();
    let result = ctx.execute(
        serde_json::from_value(
            json!({"op":"add","paths":[pack,temp.path().join("not-found")],"to":"sfx"}),
        )
        .unwrap(),
    );
    assert_eq!(result["status"], "partial");
    assert!(ctx.root.join("sfx/sounds/whoosh/wind.bin").is_file());
}

#[test]
fn corrupt_primary_database_never_rebuilds_and_recovery_preserves_raw_files() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(&temp.path().join("library"));
    fs::create_dir_all(&ctx.root).unwrap();
    fs::write(ctx.root.join("take.txt"), b"take").unwrap();
    run(
        &ctx,
        json!({"op":"annotate","path":"take.txt","text":"must survive"}),
    );
    fs::remove_file(ctx.root.join("take.txt")).unwrap();
    run(&ctx, json!({"op":"remove","missing":true}));
    let backups = run(&ctx, json!({"op":"recover"}));
    let backup = backups["backups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"].as_str().unwrap().contains("before-remove"))
        .unwrap()["name"]
        .as_str()
        .unwrap()
        .to_owned();
    let db = ctx.root.join(".valle/catalog.db");
    fs::write(&db, b"corrupt database bytes").unwrap();
    fs::write(ctx.root.join(".valle/catalog.db-wal"), b"old wal").unwrap();
    fs::write(ctx.root.join(".valle/catalog.db-shm"), b"old shm").unwrap();
    let broken = ctx.execute(serde_json::from_value(json!({"op":"list"})).unwrap());
    assert_eq!(broken["status"], "error");
    assert_eq!(fs::read(&db).unwrap(), b"corrupt database bytes");
    let recovered = run(&ctx, json!({"op":"recover","backup":backup}));
    let archive = Path::new(recovered["replaced"].as_str().unwrap());
    assert_eq!(
        fs::read(archive.join("catalog.db")).unwrap(),
        b"corrupt database bytes"
    );
    assert_eq!(
        fs::read(archive.join("catalog.db-wal")).unwrap(),
        b"old wal"
    );
    let missing = run(&ctx, json!({"op":"list","missing":true}));
    assert_eq!(missing["items"][0]["notes"][0]["text"], "must survive");
    // A bad backup is rejected before archiving the healthy current catalog.
    fs::write(ctx.root.join(".valle/backups/bad.db"), b"broken").unwrap();
    let bad =
        ctx.execute(serde_json::from_value(json!({"op":"recover","backup":"bad.db"})).unwrap());
    assert_eq!(bad["status"], "error");
    assert_eq!(run(&ctx, json!({"op":"list","missing":true}))["total"], 1);
}

#[test]
fn derived_tables_rebuild_but_sql_and_hidden_paths_cannot_mutate_primary_data() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(&temp.path().join("library"));
    fs::create_dir_all(&ctx.root).unwrap();
    fs::write(ctx.root.join("music.txt"), b"music").unwrap();
    run(
        &ctx,
        json!({"op":"tag","path":"music.txt","add":["轻快音乐"]}),
    );
    let db = rusqlite::Connection::open(ctx.root.join(".valle/catalog.db")).unwrap();
    db.execute_batch("DROP TABLE facts; DROP TABLE asset_search;")
        .unwrap();
    drop(db);
    assert_eq!(
        run(&ctx, json!({"op":"search","query":"轻快音乐"}))["total"],
        1
    );
    for query in [
        "DELETE FROM tags",
        "PRAGMA writable_schema=ON",
        "ATTACH DATABASE ':memory:' AS other",
    ] {
        let result =
            ctx.execute(serde_json::from_value(json!({"op":"sql","query":query})).unwrap());
        assert_eq!(result["status"], "error", "{result}");
    }
    assert_eq!(
        run(
            &ctx,
            json!({"op":"sql","query":"SELECT count(*) FROM tags"})
        )["rows"][0][0],
        1
    );
    for path in ["../music.txt", ".valle/catalog.db"] {
        assert!(ctx.file(path).is_err());
    }
    assert_eq!(list(&ctx)["total"], 1);
}

#[test]
fn concurrent_tags_are_serialized_without_a_library_filesystem_lock() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("clip.txt"), b"clip").unwrap();
    list(&context(&root));
    std::thread::scope(|scope| {
        let mut tasks = Vec::new();
        for number in 0..8 {
            let root = &root;
            tasks.push(scope.spawn(move || {
                run(
                    &context(root),
                    json!({"op":"tag","path":"clip.txt","add":[format!("tag-{number}")]}),
                )
            }));
        }
        for task in tasks {
            task.join().unwrap();
        }
    });
    assert_eq!(
        run(&context(&root), json!({"op":"show","path":"clip.txt"}))["tags"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert!(!root.join(".valle/lock").exists());
}
