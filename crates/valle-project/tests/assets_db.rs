//! Idempotent and concurrent database initialization tests.

use std::process::Command;

use valle_project::assets::{Db, Home};

/// Child entry point selected with --ignored --exact.
#[test]
#[ignore = "child role: spawned by two_processes_create_db_concurrently"]
fn child_open_db() {
    let home = Home::at(std::env::var("VALLE_HOME").expect("child requires VALLE_HOME"));
    Db::open(&home).expect("child database initialization failed");
}

#[test]
fn two_processes_create_db_concurrently() {
    let tmp = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().unwrap();
    let spawn = || {
        Command::new(&exe)
            .args(["child_open_db", "--ignored", "--exact"])
            .env("VALLE_HOME", tmp.path())
            .spawn()
            .unwrap()
    };
    let mut a = spawn();
    let mut b = spawn();
    assert!(
        a.wait().unwrap().success(),
        "process A database initialization failed"
    );
    assert!(
        b.wait().unwrap().success(),
        "process B database initialization failed"
    );

    // Verify the initialized schema is usable.
    let home = Home::at(tmp.path());
    let db = Db::open(&home).unwrap();
    let tables: i64 = db
        .conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN
             ('assets','analysis','segments','annotations','entities','annotation_entities','retrieval_units','jobs','schema_version')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 9, "all nine tables must exist");
}
