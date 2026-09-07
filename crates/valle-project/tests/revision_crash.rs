//! Process-level crash matrix for the Timeline revision commit point.

mod common;

use std::{path::Path, process::Command};

use valle_project::revision::{Actor, AuthenticatedContext, ProjectId, ProjectStore};
use valle_timeline::wire::edit::{EditTimelineRequestWire, EditTimelineResultWire};

use common::timeline;

const CHILD_ENV: &str = "VALLE_REVISION_CRASH_CHILD";
const ROOT_ENV: &str = "VALLE_REVISION_CRASH_ROOT";
const MODE_ENV: &str = "VALLE_REVISION_CRASH_MODE";

fn auth() -> AuthenticatedContext {
    AuthenticatedContext::new(Actor::new("test:crash-matrix").unwrap())
}

fn edit(
    store: &ProjectStore,
    project_id: &ProjectId,
    base_revision: u64,
) -> Result<valle_timeline::wire::edit::EditTimelineResponseWire, valle_project::revision::StoreFault>
{
    store.edit_timeline(
        project_id,
        &EditTimelineRequestWire {
            base_revision,
            timeline: timeline(1).into_wire(),
            intent: Some("crash matrix candidate".to_owned()),
        },
        &auth(),
    )
}

#[test]
fn revision_crash_child_worker() {
    let Ok(root) = std::env::var(ROOT_ENV) else {
        return;
    };
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }

    let store = ProjectStore::at(root);
    let project_id = ProjectId::new("crash-matrix").unwrap();
    if std::env::var_os(MODE_ENV).is_some_and(|mode| mode == "create") {
        let _ = store.create_project(&project_id, &timeline(0), None, &auth());
        panic!("crash injection label did not abort the worker");
    }
    let head = store.get_timeline(&project_id, None).unwrap();
    let _ = edit(&store, &project_id, head.revision().revision);
    panic!("crash injection label did not abort the worker");
}

fn spawn_worker(root: &Path, label: &str) -> std::process::ExitStatus {
    Command::new(std::env::current_exe().unwrap())
        .args(["revision_crash_child_worker", "--exact", "--quiet"])
        .env(CHILD_ENV, "1")
        .env(ROOT_ENV, root)
        .env("VALLE_PROJECT_CRASH_AT", label)
        .env("VALLE_PROJECT_NOW", "1784197800123")
        .env_remove("RUST_TEST_THREADS")
        .output()
        .expect("spawn crash worker")
        .status
}

fn spawn_create_worker(root: &Path, label: &str) -> std::process::ExitStatus {
    Command::new(std::env::current_exe().unwrap())
        .args(["revision_crash_child_worker", "--exact", "--quiet"])
        .env(CHILD_ENV, "1")
        .env(ROOT_ENV, root)
        .env(MODE_ENV, "create")
        .env("VALLE_PROJECT_CRASH_AT", label)
        .env("VALLE_PROJECT_NOW", "1784197800123")
        .env_remove("RUST_TEST_THREADS")
        .output()
        .expect("spawn FORMAT crash worker")
        .status
}

#[test]
fn every_store_format_publish_boundary_can_retry_genesis() {
    for label in [
        "after-store-format-write",
        "after-store-format-fsync",
        "after-store-format-rename",
        "after-store-root-fsync",
    ] {
        let root = tempfile::tempdir().unwrap();
        let status = spawn_create_worker(root.path(), label);
        assert!(!status.success(), "{label} must abort the worker");

        let store = ProjectStore::at(root.path());
        let project_id = ProjectId::new("crash-matrix").unwrap();
        let snapshot = store
            .create_project(&project_id, &timeline(0), None, &auth())
            .unwrap();
        assert_eq!(snapshot.revision().revision, 1, "{label}");
        assert_eq!(
            std::fs::read(root.path().join("FORMAT")).unwrap(),
            b"valle.project-store@1\n",
            "{label}"
        );
        assert!(!root.path().join("FORMAT.tmp").exists(), "{label}");
    }
}

#[test]
fn every_revision_publish_boundary_recovers_to_one_commit_point() {
    let cases = [
        ("after-timeline-write", false),
        ("after-timeline-fsync", false),
        ("after-revision-metadata-write", false),
        ("after-revision-metadata-fsync", false),
        ("after-revision-directory-fsync", false),
        ("after-revision-rename", false),
        ("after-revisions-directory-fsync", false),
        ("after-head-file-write", false),
        ("after-head-file-fsync", false),
        ("after-head-rename", true),
        ("after-project-directory-fsync", true),
    ];

    for (label, published) in cases {
        let root = tempfile::tempdir().unwrap();
        let project_id = ProjectId::new("crash-matrix").unwrap();
        let genesis = {
            let store = ProjectStore::at(root.path());
            store
                .create_project(&project_id, &timeline(0), None, &auth())
                .unwrap()
        };

        let status = spawn_worker(root.path(), label);
        assert!(!status.success(), "{label} must abort the worker");

        let store = ProjectStore::at(root.path());
        let recovered = store.get_timeline(&project_id, None).unwrap();
        assert_eq!(
            recovered.revision().revision,
            if published { 2 } else { 1 },
            "{label} exposed the wrong HEAD"
        );

        let retry = edit(&store, &project_id, genesis.revision().revision).unwrap();
        if published {
            assert!(matches!(
                retry.result,
                EditTimelineResultWire::StaleBase { .. }
            ));
        } else {
            assert!(matches!(
                retry.result,
                EditTimelineResultWire::Committed { .. }
            ));
        }

        let final_snapshot = store.get_timeline(&project_id, None).unwrap();
        assert_eq!(final_snapshot.revision().revision, 2, "{label}");
        assert_eq!(
            store
                .list_revisions(&project_id, None)
                .unwrap()
                .revisions
                .len(),
            2
        );

        let project_dir = store.project_dir(&project_id);
        assert!(!project_dir.join("HEAD.tmp").exists(), "{label}");
        assert!(
            std::fs::read_dir(project_dir.join("revisions"))
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().starts_with(".tmp-")),
            "{label} left an unpublished temporary revision"
        );
    }
}
