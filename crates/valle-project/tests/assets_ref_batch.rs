//! Batch failure isolation, reference resolution and stale-source detection tests.

use std::path::Path;

use valle_project::assets::add::add_batch;
use valle_project::assets::resolve::resolve;
use valle_project::assets::{AddMode, Ctx, ErrorCode, Home};

fn ctx(root: &Path) -> Ctx {
    // SAFETY: Environment setup is serialized within the test process.
    unsafe { std::env::set_var("VALLE_ASSETS_NOW", "1784197800123") };
    Ctx::bare(Home::at(root))
}

#[test]
fn mixed_batch_continues_past_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let c = ctx(&tmp.path().join("home"));
    let mk = |name: &str, bytes: &[u8]| {
        let p = tmp.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    };
    let paths = vec![
        mk("v.mp4", b"vv"),
        mk("i.png", b"ii"),
        mk("garbage.xyz", b"??"),
        mk("a.mp3", b"aa"),
    ];
    let items = add_batch(&c, &paths, AddMode::Reflink, None, None, &[]);
    assert_eq!(items.len(), 4);
    let oks: Vec<_> = items.iter().filter(|i| i.outcome.is_some()).collect();
    let errs: Vec<_> = items.iter().filter(|i| i.error.is_some()).collect();
    assert_eq!(oks.len(), 3, "MP4, PNG and MP3 imports must succeed");
    assert_eq!(
        errs.len(),
        1,
        "invalid input must fail without interrupting the batch"
    );
    assert!(errs[0].path.ends_with("garbage.xyz"));
    // An intermediate failure must not prevent the later MP3 import.
    assert!(items[3].outcome.is_some());
}

#[test]
fn reference_registers_without_blob_and_resolves() {
    let tmp = tempfile::tempdir().unwrap();
    let home_dir = tmp.path().join("home");
    let c = ctx(&home_dir);
    let src = tmp.path().join("footage.mp4");
    std::fs::write(&src, b"reference-bytes").unwrap();

    let items = add_batch(&c, &[src.clone()], AddMode::Reference, None, None, &[]);
    let out = items[0]
        .outcome
        .as_ref()
        .expect("reference registration must succeed");
    let hash = out.content_digest.as_hex();
    // Reference assets have no stored blob.
    assert!(!c.home.objects_dir().exists() || walk(&c.home.objects_dir()).is_empty());
    // Resolve to the source's absolute path.
    let r = resolve(&c, &hash[..8]).unwrap();
    assert_eq!(r.location, "reference");
    assert_eq!(r.content_digest, out.content_digest);
    assert_eq!(
        std::fs::canonicalize(&src).unwrap().to_string_lossy(),
        r.path
    );

    // Changed source bytes produce stale_reference.
    std::thread::sleep(std::time::Duration::from_millis(10));
    std::fs::write(&src, b"changed-bytes-longer").unwrap();
    let err = resolve(&c, &hash[..8]).unwrap_err();
    assert_eq!(err.code, ErrorCode::StaleReference);

    // A missing source also produces stale_reference.
    std::fs::remove_file(&src).unwrap();
    let err = resolve(&c, &hash[..8]).unwrap_err();
    assert_eq!(err.code, ErrorCode::StaleReference);
}

fn walk(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.filter_map(|e| e.ok()) {
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
