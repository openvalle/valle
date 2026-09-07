use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Output, Stdio},
};

use fs2::FileExt;
use serde_json::json;
use valle_project::assets::{
    AddMode, AssetKind, Ctx, Home, ResourceRootStore,
    add::add,
    maintain::{gc, rm},
};

fn context(root: &Path) -> Ctx {
    Ctx::bare(Home::at(root))
}

#[test]
fn imported_render_resources_are_immutable_and_live_until_lease_release() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("home");
    let source = directory.path().join("source.bin");
    let bytes = b"render snapshot";
    std::fs::write(&source, bytes).unwrap();
    let digest = valle_project::ContentDigest::of_bytes(bytes);
    let store = ResourceRootStore::at(&root);
    let lease = store.import_and_lease([(digest, source.clone())]).unwrap();
    let blob = Home::at(&root).object_path(&digest.as_hex(), None);
    std::fs::write(source, b"changed after import").unwrap();
    assert_eq!(std::fs::read(&blob).unwrap(), bytes);
    assert_eq!(gc(&context(&root)).unwrap().removed_blobs, 0);
    lease.release().unwrap();
    assert_eq!(gc(&context(&root)).unwrap().removed_blobs, 1);
    assert!(!blob.exists());
}

#[test]
fn failed_import_cannot_publish_mismatched_content_or_an_active_lease() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("home");
    let source = directory.path().join("wrong.bin");
    std::fs::write(&source, b"wrong bytes").unwrap();
    let digest = valle_project::ContentDigest::of_bytes(b"expected bytes");
    let store = ResourceRootStore::at(&root);
    assert!(store.import_and_lease([(digest, source)]).is_err());
    assert!(!Home::at(&root).object_path(&digest.as_hex(), None).exists());
    assert_eq!(
        std::fs::read_dir(store.roots_dir().join("leases"))
            .unwrap()
            .count(),
        0
    );
    gc(&context(&root)).unwrap();
}

#[test]
fn unpin_does_not_release_a_running_render_and_job_failure_releases_its_lease() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("home");
    let source = directory.path().join("source.bin");
    std::fs::write(&source, b"render resource").unwrap();
    let digest = valle_project::ContentDigest::of_bytes(b"render resource");
    let store = ResourceRootStore::at(&root);
    let lease = store.import_and_lease([(digest, source)]).unwrap();
    let pin = store
        .publish_package_pin_unchecked(&fixed_package_manifest(&[digest.as_hex()]))
        .unwrap();
    store.remove_package_pin(*pin.package_digest()).unwrap();
    store.remove_package_pin(*pin.package_digest()).unwrap();
    assert_eq!(gc(&context(&root)).unwrap().removed_blobs, 0);
    // Error returns/unwinding use Drop, without requiring a successful explicit release.
    drop(lease);
    assert_eq!(gc(&context(&root)).unwrap().removed_blobs, 1);
    assert_eq!(
        std::fs::read_dir(store.roots_dir().join("leases"))
            .unwrap()
            .count(),
        0
    );
}

fn add_blob(ctx: &Ctx, directory: &Path, name: &str, bytes: &[u8]) -> (String, std::path::PathBuf) {
    let source = directory.join(name);
    std::fs::write(&source, bytes).unwrap();
    let asset = add(
        ctx,
        &source,
        AddMode::Copy,
        Some(AssetKind::Other),
        None,
        &[],
    )
    .unwrap();
    let digest = asset.content_digest.as_hex();
    let extension = Path::new(name).extension().and_then(|value| value.to_str());
    let blob = ctx.home.object_path(&digest, extension);
    (digest, blob)
}

fn fixed_package_manifest(digests: &[String]) -> String {
    let mut digests = digests
        .iter()
        .map(|digest| format!("sha256:{digest}"))
        .collect::<Vec<_>>();
    digests.sort();
    let member = |role: &str, path: &str| {
        json!({
            "role": role,
            "path": path,
            "bytes": 2,
            "digest": format!("sha256:{}", "0".repeat(64))
        })
    };
    let value = json!({
        "format": "valle.fixed-render-package@1",
        "members": [
            member("canonical-timeline", "canonical-timeline.json"),
            member("execution-profile", "execution-profile.json"),
            member("resource-manifest", "resource-manifest.json"),
            member("verified-binding-bundle", "verified-binding-bundle.json")
        ],
        "resourceDigests": digests
    });
    String::from_utf8(serde_jcs::to_vec(&value).unwrap()).unwrap()
}

const LEASE_CHILD: &str = "VALLE_RESOURCE_LEASE_CHILD";
const LEASE_ROOT: &str = "VALLE_RESOURCE_LEASE_ROOT";
const LEASE_DIGEST: &str = "VALLE_RESOURCE_LEASE_DIGEST";
const ROOT_PROCESS_CHILD: &str = "VALLE_RESOURCE_ROOT_PROCESS_CHILD";
const ROOT_PROCESS_ROOT: &str = "VALLE_RESOURCE_ROOT_PROCESS_ROOT";
const ROOT_PROCESS_DIGEST: &str = "VALLE_RESOURCE_ROOT_PROCESS_DIGEST";

#[test]
fn active_lease_process_helper() {
    if std::env::var_os(LEASE_CHILD).is_none() {
        return;
    }
    let root = std::env::var_os(LEASE_ROOT).unwrap();
    let digest = std::env::var(LEASE_DIGEST).unwrap();
    let lease = ResourceRootStore::at(root)
        .acquire_active_lease([digest])
        .unwrap();
    println!("LEASE_READY {}", lease.lease_id());
    std::io::stdout().flush().unwrap();
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input).unwrap();
    drop(lease);
}

#[test]
fn resource_root_process_helper() {
    let Ok(mode) = std::env::var(ROOT_PROCESS_CHILD) else {
        return;
    };
    let root = std::env::var_os(ROOT_PROCESS_ROOT).expect("resource-root child root");
    let store = ResourceRootStore::at(&root);
    match mode.as_str() {
        "format-crash" => {
            store
                .publish_package_pin_unchecked(&fixed_package_manifest(&[]))
                .unwrap();
        }
        "pin-crash" | "pin-race" => {
            let digest = std::env::var(ROOT_PROCESS_DIGEST).expect("resource-root child digest");
            let pin = store
                .publish_package_pin_unchecked(&fixed_package_manifest(&[digest]))
                .unwrap();
            if mode == "pin-race" {
                println!("PIN_DONE {}", pin.package_digest());
                std::io::stdout().flush().unwrap();
            }
        }
        "lease-crash" => {
            let digest = std::env::var(ROOT_PROCESS_DIGEST).expect("resource-root child digest");
            let _lease = store
                .acquire_active_lease([format!("sha256:{digest}")])
                .unwrap();
        }
        "gc-race" => {
            println!("GC_CHILD_READY");
            std::io::stdout().flush().unwrap();
            let mut release = [0_u8; 1];
            std::io::stdin().read_exact(&mut release).unwrap();
            println!("GC_CALLING");
            std::io::stdout().flush().unwrap();
            let outcome = gc(&context(Path::new(&root))).unwrap();
            println!("GC_DONE {}", outcome.removed_blobs);
            std::io::stdout().flush().unwrap();
        }
        other => panic!("unknown resource-root child mode {other}"),
    }
}

fn process_command(root: &Path, mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["resource_root_process_helper", "--exact", "--nocapture"])
        .env(ROOT_PROCESS_CHILD, mode)
        .env(ROOT_PROCESS_ROOT, root)
        .env_remove("VALLE_ASSETS_CRASH_AT")
        .env_remove("VALLE_RESOURCE_ROOT_BARRIER_AT")
        .env_remove("RUST_TEST_THREADS");
    command
}

fn spawn_crash(root: &Path, mode: &str, digest: Option<&str>, label: &str) -> Output {
    let mut command = process_command(root, mode);
    command.env("VALLE_ASSETS_CRASH_AT", label);
    if let Some(digest) = digest {
        command.env(ROOT_PROCESS_DIGEST, digest);
    }
    command.output().unwrap()
}

fn spawn_barrier_child(root: &Path, mode: &str, digest: Option<&str>) -> Child {
    let mut command = process_command(root, mode);
    if let Some(digest) = digest {
        command.env(ROOT_PROCESS_DIGEST, digest);
    }
    if mode == "pin-race" {
        command.env(
            "VALLE_RESOURCE_ROOT_BARRIER_AT",
            "resource-root-package-pin-after-assets-lock",
        );
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap()
}

fn wait_for_marker<R: Read>(reader: &mut BufReader<R>, marker: &str) {
    let mut line = String::new();
    loop {
        line.clear();
        assert_ne!(
            reader.read_line(&mut line).unwrap(),
            0,
            "child exited before marker {marker}"
        );
        if line.contains(marker) {
            return;
        }
    }
}

fn assert_no_staging_files(directory: &Path, prefix: &str) {
    assert!(
        std::fs::read_dir(directory)
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().starts_with(prefix)),
        "{} still contains {prefix} staging residue",
        directory.display()
    );
}

#[test]
fn every_format_publication_boundary_recovers_without_partial_truth() {
    for label in [
        "resource-root-format-after-write",
        "resource-root-format-after-file-fsync",
        "resource-root-format-after-rename",
        "resource-root-format-after-directory-fsync",
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("home");
        let output = spawn_crash(&root, "format-crash", None, label);
        assert!(
            !output.status.success(),
            "{label} did not abort the child: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let ctx = context(&root);
        assert_eq!(gc(&ctx).unwrap().removed_blobs, 0, "{label}");
        let roots = root.join("resource-roots");
        assert_eq!(
            std::fs::read(roots.join("FORMAT")).unwrap(),
            b"valle.resource-roots@1\n",
            "{label}"
        );
        assert_no_staging_files(&roots, ".root-");
        assert!(
            std::fs::read_dir(roots.join("packages"))
                .unwrap()
                .next()
                .is_none(),
            "FORMAT publication must not imply a package pin: {label}"
        );
        assert!(
            std::fs::read_dir(roots.join("leases"))
                .unwrap()
                .next()
                .is_none(),
            "FORMAT publication must not imply an active lease: {label}"
        );
    }
}

#[test]
fn every_package_pin_publication_boundary_recovers_to_one_commit_point() {
    for (label, committed) in [
        ("resource-root-package-pin-after-write", false),
        ("resource-root-package-pin-after-file-fsync", false),
        ("resource-root-package-pin-after-rename", true),
        ("resource-root-package-pin-after-directory-fsync", true),
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("home");
        let ctx = context(&root);
        let (digest, blob) = add_blob(&ctx, temporary.path(), "pin-crash.bin", b"pin-crash");
        assert_eq!(gc(&ctx).unwrap().removed_blobs, 0);
        let manifest = fixed_package_manifest(&[digest.clone()]);

        let output = spawn_crash(&root, "pin-crash", Some(&digest), label);
        assert!(
            !output.status.success(),
            "{label} did not abort the child: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        assert_eq!(gc(&ctx).unwrap().removed_blobs, 0, "{label}");
        let packages = root.join("resource-roots/packages");
        assert_no_staging_files(&packages, ".root-");
        let published = std::fs::read_dir(&packages)
            .unwrap()
            .filter_map(Result::ok)
            .count();
        assert_eq!(published, usize::from(committed), "{label}");

        let pin = ResourceRootStore::at(&root)
            .publish_package_pin_unchecked(&manifest)
            .unwrap();
        assert!(pin.path().is_file(), "{label}");
        assert_eq!(
            std::fs::read_dir(&packages)
                .unwrap()
                .filter_map(Result::ok)
                .count(),
            1,
            "retry must converge to exactly one immutable pin: {label}"
        );
        assert_eq!(rm(&ctx, &digest, true, true).unwrap().action, "purged");
        assert_eq!(gc(&ctx).unwrap().removed_blobs, 0, "{label}");
        assert!(
            blob.is_file(),
            "recovered pin must protect CAS bytes: {label}"
        );
    }
}

#[test]
fn every_active_lease_publication_boundary_recovers_as_stale() {
    for label in [
        "resource-root-active-lease-after-write",
        "resource-root-active-lease-after-file-fsync",
        "resource-root-active-lease-after-rename",
        "resource-root-active-lease-after-directory-fsync",
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("home");
        let ctx = context(&root);
        let (digest, blob) = add_blob(&ctx, temporary.path(), "lease-crash.bin", b"lease-crash");
        assert_eq!(gc(&ctx).unwrap().removed_blobs, 0);

        let output = spawn_crash(&root, "lease-crash", Some(&digest), label);
        assert!(
            !output.status.success(),
            "{label} did not abort the child: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        assert_eq!(gc(&ctx).unwrap().removed_blobs, 0, "{label}");
        let leases = root.join("resource-roots/leases");
        assert!(
            std::fs::read_dir(&leases).unwrap().next().is_none(),
            "crashed lease must be stale and reclaimable: {label}"
        );

        let lease = ResourceRootStore::at(&root)
            .acquire_active_lease([format!("sha256:{digest}")])
            .unwrap();
        assert_eq!(rm(&ctx, &digest, true, true).unwrap().action, "purged");
        assert!(
            blob.is_file(),
            "retry lease must protect CAS bytes: {label}"
        );
        lease.release().unwrap();
        assert_eq!(gc(&ctx).unwrap().removed_blobs, 1, "{label}");
        assert!(
            !blob.exists(),
            "released retry lease must be collectable: {label}"
        );
    }
}

#[test]
fn published_package_pin_is_the_only_persistent_gc_root() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("home");
    let ctx = context(&root);
    let (forgotten_hash, forgotten_blob) =
        add_blob(&ctx, temporary.path(), "forgotten.bin", b"forgotten");
    let (pinned_hash, pinned_blob) = add_blob(&ctx, temporary.path(), "pinned.bin", b"pinned");

    let store = ResourceRootStore::at(&root);
    let manifest = fixed_package_manifest(std::slice::from_ref(&pinned_hash));
    let pin = store.publish_package_pin_unchecked(&manifest).unwrap();
    assert!(pin.package_digest().to_wire().starts_with("sha256:"));
    assert!(pin.path().is_file());
    assert_eq!(
        std::fs::read(root.join("resource-roots/FORMAT")).unwrap(),
        b"valle.resource-roots@1\n"
    );

    assert_eq!(
        rm(&ctx, &forgotten_hash, true, true).unwrap().action,
        "purged"
    );
    assert!(!forgotten_blob.exists());
    assert_eq!(rm(&ctx, &pinned_hash, true, true).unwrap().action, "purged");
    assert!(pinned_blob.is_file());
    let outcome = gc(&ctx).unwrap();
    assert_eq!(outcome.removed_blobs, 0);
    assert!(pinned_blob.is_file());
}

#[test]
fn pinned_blob_with_a_tmp_source_extension_is_not_staging_residue() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("home");
    let ctx = context(&root);
    let (digest, blob) = add_blob(&ctx, temporary.path(), "legitimate.tmp", b"real-media");
    ResourceRootStore::at(&root)
        .publish_package_pin_unchecked(&fixed_package_manifest(&[digest.clone()]))
        .unwrap();
    assert_eq!(rm(&ctx, &digest, true, true).unwrap().action, "purged");

    assert_eq!(gc(&ctx).unwrap().removed_blobs, 0);
    assert!(
        blob.is_file(),
        "a source extension is not an atomic-write marker"
    );
}

#[test]
fn active_lease_protects_bytes_and_drop_makes_the_record_collectable() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("home");
    let ctx = context(&root);
    let (digest, blob) = add_blob(&ctx, temporary.path(), "leased.bin", b"leased");
    let store = ResourceRootStore::at(&root);
    let lease = store
        .acquire_active_lease([format!("sha256:{digest}")])
        .unwrap();
    assert!(lease.path().is_file());

    assert_eq!(rm(&ctx, &digest, true, true).unwrap().action, "purged");
    assert!(blob.is_file());
    assert_eq!(gc(&ctx).unwrap().removed_blobs, 0);
    assert!(blob.is_file());

    let lease_path = lease.path().to_path_buf();
    drop(lease);
    assert_eq!(gc(&ctx).unwrap().removed_blobs, 1);
    assert!(!blob.exists());
    assert!(!lease_path.exists());
}

#[test]
fn lease_owner_crash_releases_the_root_without_time_based_guessing() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("home");
    let ctx = context(&root);
    let (digest, blob) = add_blob(&ctx, temporary.path(), "crash.bin", b"crash-owned");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["active_lease_process_helper", "--exact", "--nocapture"])
        .env(LEASE_CHILD, "1")
        .env(LEASE_ROOT, &root)
        .env(LEASE_DIGEST, format!("sha256:{digest}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert_ne!(
            stdout.read_line(&mut line).unwrap(),
            0,
            "lease child exited before readiness"
        );
        if line.contains("LEASE_READY") {
            break;
        }
    }

    assert_eq!(rm(&ctx, &digest, true, true).unwrap().action, "purged");
    assert!(blob.is_file(), "live child lease must protect the blob");
    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert!(
        !status.success(),
        "test must simulate an unclean owner exit"
    );

    assert_eq!(gc(&ctx).unwrap().removed_blobs, 1);
    assert!(!blob.exists());
    assert!(
        std::fs::read_dir(root.join("resource-roots/leases"))
            .unwrap()
            .next()
            .is_none(),
        "the unlocked crash residue must be reclaimed"
    );
}

#[test]
fn corrupt_pin_aborts_before_any_orphan_is_deleted() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("home");
    let ctx = context(&root);
    let (digest, blob) = add_blob(&ctx, temporary.path(), "rooted.bin", b"rooted");
    let store = ResourceRootStore::at(&root);
    let pin = store
        .publish_package_pin_unchecked(&fixed_package_manifest(&[digest.clone()]))
        .unwrap();
    assert_eq!(rm(&ctx, &digest, true, true).unwrap().action, "purged");
    assert!(blob.is_file());
    std::fs::write(pin.path(), b"{}").unwrap();

    let error = gc(&ctx).unwrap_err();
    assert!(error.message.contains("digest mismatch"));
    assert!(
        blob.is_file(),
        "GC must fail before deletion when roots are uncertain"
    );
}

#[test]
fn nonempty_root_store_without_exact_format_fails_closed() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("home");
    let roots = root.join("resource-roots");
    std::fs::create_dir_all(&roots).unwrap();
    std::fs::write(roots.join("unknown"), b"state").unwrap();
    let ctx = context(&root);
    let error = gc(&ctx).unwrap_err();
    assert!(error.message.contains("missing exact FORMAT"));
    assert!(roots.join("unknown").is_file());
}

#[test]
fn initialized_store_missing_packages_directory_fails_closed() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("home");
    let ctx = context(&root);
    let (digest, blob) = add_blob(&ctx, temporary.path(), "rooted.bin", b"rooted");
    ResourceRootStore::at(&root)
        .publish_package_pin_unchecked(&fixed_package_manifest(&[digest.clone()]))
        .unwrap();
    assert_eq!(rm(&ctx, &digest, true, true).unwrap().action, "purged");
    std::fs::remove_dir_all(root.join("resource-roots/packages")).unwrap();

    let error = gc(&ctx).unwrap_err();
    assert!(error.message.contains("missing required directory"));
    assert!(
        blob.is_file(),
        "an uncertain persistent root set must block GC"
    );
    assert!(!root.join("resource-roots/packages").exists());
}

#[cfg(unix)]
#[test]
fn initialized_store_missing_live_leases_directory_fails_closed() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("home");
    let ctx = context(&root);
    let (digest, blob) = add_blob(&ctx, temporary.path(), "leased.bin", b"leased");
    let lease = ResourceRootStore::at(&root)
        .acquire_active_lease([format!("sha256:{digest}")])
        .unwrap();
    assert_eq!(rm(&ctx, &digest, true, true).unwrap().action, "purged");
    std::fs::remove_dir_all(root.join("resource-roots/leases")).unwrap();

    let error = gc(&ctx).unwrap_err();
    assert!(error.message.contains("missing required directory"));
    assert!(blob.is_file(), "a lost live-lease namespace must block GC");
    assert!(!root.join("resource-roots/leases").exists());
    drop(lease);
}

#[test]
fn empty_pre_format_directories_resume_initialization_safely() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("home");
    std::fs::create_dir_all(root.join("resource-roots/packages")).unwrap();
    let ctx = context(&root);

    let outcome = gc(&ctx).unwrap();
    assert_eq!(outcome.removed_blobs, 0);
    assert_eq!(
        std::fs::read(root.join("resource-roots/FORMAT")).unwrap(),
        b"valle.resource-roots@1\n"
    );
    assert!(root.join("resource-roots/packages").is_dir());
    assert!(root.join("resource-roots/leases").is_dir());
}

#[test]
fn companion_or_corrupt_bytes_cannot_satisfy_a_cas_digest() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("home");
    let ctx = context(&root);
    let (digest, blob) = add_blob(&ctx, temporary.path(), "source.bin", b"expected");
    let manifest = fixed_package_manifest(&[digest.clone()]);
    std::fs::write(&blob, b"wrong").unwrap();

    let error = ResourceRootStore::at(&root)
        .publish_package_pin_unchecked(&manifest)
        .unwrap_err();
    assert!(error.message.contains("before CAS blob"));

    std::fs::remove_file(&blob).unwrap();
    let companion = ctx.home.object_path(&digest, Some("keyframes.json"));
    std::fs::write(&companion, b"companion-not-the-source").unwrap();
    let error = ResourceRootStore::at(&root)
        .publish_package_pin_unchecked(&manifest)
        .unwrap_err();
    assert!(error.message.contains("before CAS blob"));
    assert!(
        std::fs::read_dir(root.join("resource-roots/packages"))
            .unwrap()
            .next()
            .is_none(),
        "failed CAS verification must not publish a pin"
    );
}

#[cfg(unix)]
#[test]
fn cas_shard_symlink_is_rejected_without_following_it() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("home");
    let ctx = context(&root);
    let bytes = b"outside-cas";
    let digest = {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(bytes))
    };
    let (shard, tail) = digest.split_at(2);
    std::fs::create_dir_all(ctx.home.objects_dir()).unwrap();
    let outside = temporary.path().join("outside-shard");
    std::fs::create_dir(&outside).unwrap();
    let outside_blob = outside.join(format!("{tail}.bin"));
    std::fs::write(&outside_blob, bytes).unwrap();
    symlink(&outside, ctx.home.objects_dir().join(shard)).unwrap();

    let error = ResourceRootStore::at(&root)
        .publish_package_pin_unchecked(&fixed_package_manifest(&[digest]))
        .unwrap_err();
    assert!(error.message.contains("real directory"));
    assert_eq!(std::fs::read(outside_blob).unwrap(), bytes);
}

#[cfg(unix)]
#[test]
fn resource_gc_lock_symlink_is_rejected_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("home");
    std::fs::create_dir(&root).unwrap();
    let outside = temporary.path().join("outside-gc-lock");
    std::fs::write(&outside, b"keep").unwrap();
    symlink(&outside, root.join(".resource-gc.lock")).unwrap();

    let error = ResourceRootStore::at(&root)
        .acquire_active_lease(std::iter::empty::<String>())
        .unwrap_err();
    assert!(error.message.contains("not a regular file"));
    assert_eq!(std::fs::read(outside).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn pin_symlink_is_rejected_without_following_it() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("home");
    let ctx = context(&root);
    let (digest, _) = add_blob(&ctx, temporary.path(), "rooted.bin", b"rooted");
    let store = ResourceRootStore::at(&root);
    let pin = store
        .publish_package_pin_unchecked(&fixed_package_manifest(&[digest]))
        .unwrap();
    let outside = temporary.path().join("outside");
    std::fs::write(&outside, b"keep").unwrap();
    std::fs::remove_file(pin.path()).unwrap();
    symlink(&outside, pin.path()).unwrap();

    let error = gc(&ctx).unwrap_err();
    assert!(error.message.contains("regular file"));
    assert_eq!(std::fs::read(outside).unwrap(), b"keep");
}

#[test]
fn cross_process_pin_publication_precedes_waiting_gc_without_a_dangling_root() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("home");
    let ctx = context(&root);
    let (digest, blob) = add_blob(&ctx, temporary.path(), "race.bin", b"race-root");
    assert_eq!(gc(&ctx).unwrap().removed_blobs, 0);
    std::fs::remove_file(ctx.home.meta_path(&digest)).unwrap();

    // Hold the inner lock first. The publisher can therefore prove it owns
    // assets/.lock at its stdout barrier while it cannot yet publish the pin.
    let gc_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join(".resource-gc.lock"))
        .unwrap();
    gc_lock.lock_exclusive().unwrap();

    let mut publisher = spawn_barrier_child(&root, "pin-race", Some(&digest));
    let mut publisher_stdout = BufReader::new(publisher.stdout.take().unwrap());
    wait_for_marker(
        &mut publisher_stdout,
        "RESOURCE_ROOT_BARRIER_READY resource-root-package-pin-after-assets-lock",
    );

    let mut collector = spawn_barrier_child(&root, "gc-race", None);
    let mut collector_stdout = BufReader::new(collector.stdout.take().unwrap());
    wait_for_marker(&mut collector_stdout, "GC_CHILD_READY");

    // Publisher now advances to the held GC lock while retaining assets/.lock.
    publisher.stdin.take().unwrap().write_all(b"p").unwrap();
    // Collector starts only after the publisher owns the outer lock, so it
    // must wait there and cannot inspect/delete CAS before pin publication.
    collector.stdin.take().unwrap().write_all(b"g").unwrap();
    wait_for_marker(&mut collector_stdout, "GC_CALLING");

    FileExt::unlock(&gc_lock).unwrap();
    wait_for_marker(&mut publisher_stdout, "PIN_DONE sha256:");
    assert!(publisher.wait().unwrap().success());
    wait_for_marker(&mut collector_stdout, "GC_DONE 0");
    assert!(collector.wait().unwrap().success());

    assert!(
        blob.is_file(),
        "published pin must protect the orphan CAS blob"
    );
    assert_eq!(
        std::fs::read_dir(root.join("resource-roots/packages"))
            .unwrap()
            .filter_map(Result::ok)
            .count(),
        1
    );
}
