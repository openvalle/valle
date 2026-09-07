//! Asset import, deduplication, kind detection, rollback and copy fallback tests.

use std::path::Path;

use valle_project::assets::add::{add, stream_sha256};
use valle_project::assets::{
    AddMode, AssetKind, AssetMeta, Ctx, Db, Home, Probe, ProbeOutcome, Prober,
};

/// Use a fixed clock for reproducible imports.
fn ctx(root: &Path) -> Ctx {
    // SAFETY: Environment setup is serialized within the test process.
    unsafe { std::env::set_var("VALLE_ASSETS_NOW", "1784197800123") };
    Ctx::bare(Home::at(root))
}

struct FakeProber(Probe);
impl Prober for FakeProber {
    fn probe(&self, _p: &Path, _k: AssetKind) -> valle_project::assets::Result<ProbeOutcome> {
        Ok(ProbeOutcome {
            probe: Some(self.0.clone()),
            warnings: vec![],
        })
    }
}

struct FailProber;
impl Prober for FailProber {
    fn probe(&self, p: &Path, _k: AssetKind) -> valle_project::assets::Result<ProbeOutcome> {
        Err(valle_project::assets::AssetsError::unsupported_media(
            format!("invalid file: {}", p.display()),
        ))
    }
}

fn write_sample(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

#[test]
fn import_writes_meta_blob_index() {
    let tmp = tempfile::tempdir().unwrap();
    let home_dir = tmp.path().join("home");
    let src = write_sample(tmp.path(), "clip.mp4", b"fake-mp4-bytes");
    let mut c = ctx(&home_dir);
    c.prober = Box::new(FakeProber(Probe {
        duration_ms: Some(5000),
        width: Some(1920),
        height: Some(1080),
        ..Default::default()
    }));

    let out = add(
        &c,
        &src,
        AddMode::Reflink,
        None,
        Some("Test clip"),
        &["preview".into()],
    )
    .unwrap();
    assert_eq!(out.content_digest, stream_sha256(&src).unwrap());
    assert_eq!(out.kind, AssetKind::Video);
    assert!(!out.existed);
    let serialized = serde_json::to_value(&out).unwrap();
    assert_eq!(serialized["content_digest"], out.content_digest.to_wire());
    assert!(serialized.get("hash").is_none());

    let hash = out.content_digest.as_hex();

    // Verify persisted metadata.
    let meta = AssetMeta::load(&c.home.meta_path(&hash)).unwrap();
    assert_eq!(meta.content_digest, out.content_digest);
    assert_eq!(meta.original_name, "clip.mp4");
    assert_eq!(meta.title.as_deref(), Some("Test clip"));
    assert_eq!(meta.tags, vec!["preview".to_owned()]);
    assert_eq!(meta.added_at, "2026-07-16T10:30:00.123Z");
    assert_eq!(meta.probe.as_ref().unwrap().width, Some(1920));

    // Verify content-addressed blob placement and bytes.
    let blob = c.home.object_path(&hash, Some("mp4"));
    assert_eq!(std::fs::read(&blob).unwrap(), b"fake-mp4-bytes");

    // Verify index rows.
    let db = Db::open(&c.home).unwrap();
    let (kind, w): (String, i64) = db
        .conn
        .query_row(
            "SELECT kind, width FROM assets WHERE hash=?1",
            [&hash],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(kind, "video");
    assert_eq!(w, 1920);
}

#[test]
fn re_add_is_idempotent_and_merges() {
    let tmp = tempfile::tempdir().unwrap();
    let home_dir = tmp.path().join("home");
    let c = ctx(&home_dir);
    let src = write_sample(tmp.path(), "a.mp3", b"same-bytes");

    let first = add(&c, &src, AddMode::Reflink, None, None, &["t1".into()]).unwrap();
    // Identical content retains its ID; an explicit title replaces the title and tags are merged.
    let src2 = write_sample(tmp.path(), "b.mp3", b"same-bytes");
    let second = add(
        &c,
        &src2,
        AddMode::Reflink,
        None,
        Some("New title"),
        &["t1".into(), "t2".into()],
    )
    .unwrap();
    assert_eq!(first.content_digest, second.content_digest);
    assert!(second.existed);
    assert!(!second.revived);

    let meta = AssetMeta::load(&c.home.meta_path(&first.content_digest.as_hex())).unwrap();
    assert_eq!(meta.title.as_deref(), Some("New title"));
    assert_eq!(meta.tags, vec!["t1".to_owned(), "t2".to_owned()]);
    // Preserve the first registered original filename.
    assert_eq!(meta.original_name, "a.mp3");
    // Store only one blob.
    let objects = c.home.objects_dir();
    let count = walk_files(&objects).len();
    assert_eq!(count, 1);
}

#[test]
fn kind_detection_ttf_glb_jsx_lottie() {
    let tmp = tempfile::tempdir().unwrap();
    let home_dir = tmp.path().join("home");
    let c = ctx(&home_dir);

    let ttf = write_sample(tmp.path(), "font.ttf", b"\x00\x01\x00\x00fontdata");
    assert_eq!(
        add(&c, &ttf, AddMode::Reflink, None, None, &[])
            .unwrap()
            .kind,
        AssetKind::Font
    );

    let glb = write_sample(
        tmp.path(),
        "product.glb",
        b"fake bytes accepted by FakeProber",
    );
    assert_eq!(
        add(&c, &glb, AddMode::Reflink, None, None, &[])
            .unwrap()
            .kind,
        AssetKind::Model3d
    );

    // Component with companion keyframes.
    let jsx = write_sample(tmp.path(), "title.jsx", b"export default () => <h1/>;");
    write_sample(tmp.path(), "title.keyframes.json", b"{\"k\":[]}");
    let out = add(&c, &jsx, AddMode::Reflink, None, None, &[]).unwrap();
    assert_eq!(out.kind, AssetKind::Component);
    let hash = out.content_digest.as_hex();
    let meta = AssetMeta::load(&c.home.meta_path(&hash)).unwrap();
    assert!(
        meta.companion.is_some(),
        "metadata must record the companion file"
    );
    assert!(c.home.object_path(&hash, Some("keyframes.json")).exists());

    // Detect Lottie by structure.
    let lottie = write_sample(
        tmp.path(),
        "confetti.json",
        br#"{"v":"5.7","layers":[],"op":60}"#,
    );
    assert_eq!(
        add(&c, &lottie, AddMode::Reflink, None, None, &[])
            .unwrap()
            .kind,
        AssetKind::Lottie
    );

    // Reject ordinary JSON unless kind is explicitly overridden.
    let plain = write_sample(tmp.path(), "data.json", br#"{"a":1}"#);
    let err = add(&c, &plain, AddMode::Reflink, None, None, &[]).unwrap_err();
    assert_eq!(err.code, valle_project::assets::ErrorCode::UnsupportedMedia);
    let forced = add(
        &c,
        &plain,
        AddMode::Reflink,
        Some(AssetKind::Other),
        None,
        &[],
    )
    .unwrap();
    assert_eq!(forced.kind, AssetKind::Other);

    // Reject unknown extensions with a hint.
    let bin = write_sample(tmp.path(), "blob.xyz", b"???");
    let err = add(&c, &bin, AddMode::Reflink, None, None, &[]).unwrap_err();
    assert!(err.hint.unwrap().contains("--kind"));
}

#[test]
fn probe_failure_rejects_without_residue() {
    let tmp = tempfile::tempdir().unwrap();
    let home_dir = tmp.path().join("home");
    let mut c = ctx(&home_dir);
    c.prober = Box::new(FailProber);
    let src = write_sample(tmp.path(), "corrupt.mp4", b"not-really-mp4");

    let err = add(&c, &src, AddMode::Reflink, None, None, &[]).unwrap_err();
    assert_eq!(err.code, valle_project::assets::ErrorCode::UnsupportedMedia);
    // Leave no blob or metadata after failure.
    let h = stream_sha256(&src).unwrap();
    assert!(!c.home.meta_path(&h.as_hex()).exists());
    assert!(walk_files(&c.home.objects_dir()).is_empty());
}

#[test]
fn copy_mode_plain_copy_path() {
    let tmp = tempfile::tempdir().unwrap();
    let home_dir = tmp.path().join("home");
    let c = ctx(&home_dir);
    let src = write_sample(tmp.path(), "pic.png", b"png-bytes");
    let out = add(&c, &src, AddMode::Copy, None, None, &[]).unwrap();
    assert_eq!(out.kind, AssetKind::Image);
    let blob = c
        .home
        .object_path(&out.content_digest.as_hex(), Some("png"));
    assert_eq!(std::fs::read(blob).unwrap(), b"png-bytes");
}

fn walk_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if !root.exists() {
        return out;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}
