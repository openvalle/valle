//! Metadata edits, tagging, idempotence and knowledge-write crash recovery tests.

use std::path::Path;
use std::process::Command;

use valle_project::assets::add::add;
use valle_project::assets::knowledge::{edit, tag};
use valle_project::assets::read::show;
use valle_project::assets::{AddMode, AssetMeta, Ctx, Db, ErrorCode, Home};

fn ctx(root: &Path) -> Ctx {
    // SAFETY: Environment setup is serialized within the test process.
    unsafe { std::env::set_var("VALLE_ASSETS_NOW", "1784197800123") };
    Ctx::bare(Home::at(root))
}

fn setup(tmp: &Path, name: &str, bytes: &[u8]) -> (Ctx, String) {
    let c = ctx(&tmp.join("home"));
    let src = tmp.join(name);
    std::fs::write(&src, bytes).unwrap();
    let hash = add(&c, &src, AddMode::Reflink, None, None, &[])
        .unwrap()
        .content_digest
        .as_hex();
    (c, hash)
}

#[test]
fn edit_and_tag_reflect_in_show_and_are_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, hash) = setup(tmp.path(), "bgm.mp3", b"music-bytes");

    let edited = edit(&c, &hash[..8], Some("Upbeat BGM"), Some("music")).unwrap();
    assert_eq!(edited["content_digest"], format!("sha256:{hash}"));
    assert!(edited.get("hash").is_none());
    let tagged = tag(
        &c,
        &hash[..8],
        &["beat-sync".into(), "licensed".into()],
        &[],
    )
    .unwrap();
    assert_eq!(tagged["content_digest"], format!("sha256:{hash}"));
    assert!(tagged.get("hash").is_none());

    let card = show(&c, &hash[..8]).unwrap();
    assert_eq!(card["meta"]["title"], "Upbeat BGM");
    assert_eq!(card["meta"]["subkind"], "music");
    assert_eq!(
        card["meta"]["tags"],
        serde_json::json!(["beat-sync", "licensed"])
    );

    // Repeated identical edits preserve metadata bytes.
    let before = std::fs::read(c.home.meta_path(&hash)).unwrap();
    edit(&c, &hash[..8], Some("Upbeat BGM"), None).unwrap();
    tag(&c, &hash[..8], &["beat-sync".into()], &[]).unwrap();
    let after = std::fs::read(c.home.meta_path(&hash)).unwrap();
    assert_eq!(before, after, "identical edits must not rewrite metadata");

    // Tag removal updates the projection.
    tag(&c, &hash[..8], &[], &["licensed".into()]).unwrap();
    let db = Db::open(&c.home).unwrap();
    let tags: String = db
        .conn
        .query_row("SELECT tags FROM assets WHERE hash=?1", [&hash], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(tags, r#"["beat-sync"]"#);
}

#[test]
fn subkind_guard() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, hash) = setup(tmp.path(), "v.mp4", b"video-bytes");
    // Reject audio subkind on non-audio assets.
    let err = edit(&c, &hash[..8], None, Some("music")).unwrap_err();
    assert_eq!(err.code, ErrorCode::Refused);
    // Invalid values return bad_query.
    let tmp2 = tempfile::tempdir().unwrap();
    let (c2, h2) = setup(tmp2.path(), "a.mp3", b"audio-bytes");
    let err = edit(&c2, &h2[..8], None, Some("jingle")).unwrap_err();
    assert_eq!(err.code, ErrorCode::BadQuery);
}

/// Child entry point for injected knowledge-write crashes.
#[test]
#[ignore = "child role: spawned by knowledge crash matrix"]
fn child_edit_crash() {
    let home = Home::at(std::env::var("VALLE_HOME").expect("VALLE_HOME"));
    let hash = std::env::var("EDIT_HASH").expect("EDIT_HASH");
    let ctx = Ctx::bare(home);
    let _ = edit(&ctx, &hash, Some("Crash title"), None);
}

#[test]
fn knowledge_write_crash_matrix() {
    for label in ["meta-before-rename", "meta-after-rename"] {
        let tmp = tempfile::tempdir().unwrap();
        let (c, hash) = setup(tmp.path(), "k.mp3", b"crash-knowledge");
        edit(&c, &hash[..8], Some("Original title"), None).unwrap();

        let exe = std::env::current_exe().unwrap();
        let status = Command::new(&exe)
            .args(["child_edit_crash", "--ignored", "--exact"])
            .env("VALLE_HOME", tmp.path().join("home"))
            .env("EDIT_HASH", &hash)
            .env("VALLE_ASSETS_CRASH_AT", label)
            .status()
            .unwrap();
        assert!(!status.success(), "{label}: child process must abort");

        let meta = AssetMeta::load(&c.home.meta_path(&hash)).unwrap();
        match label {
            // Before rename, preserve the old metadata.
            "meta-before-rename" => assert_eq!(meta.title.as_deref(), Some("Original title")),
            // After rename, new metadata is committed and a repeated edit can repair the index.
            "meta-after-rename" => assert_eq!(meta.title.as_deref(), Some("Crash title")),
            _ => unreachable!(),
        }
        // Repeating the edit restores consistency.
        edit(&c, &hash[..8], Some("Crash title"), None).unwrap();
        let db = Db::open(&c.home).unwrap();
        let t: String = db
            .conn
            .query_row("SELECT title FROM assets WHERE hash=?1", [&hash], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(t, "Crash title", "{label}: projection must converge");
    }
}
