mod common;

use std::sync::{Arc, Barrier};

use serde_json::json;
use valle_project::revision::{
    Actor, AuthenticatedContext, ProjectId, ProjectStore, RevisionCause, SnapshotWriteResult,
    StoreFault,
};
use valle_timeline::{decode_timeline, timeline_bytes, wire::edit::*};

use common::{fixture, solid_timeline, timeline};

fn edit(
    store: &ProjectStore,
    project_id: &ProjectId,
    base_revision: u64,
    timeline: &valle_timeline::Timeline,
    auth: &AuthenticatedContext,
) -> Result<EditTimelineResponseWire, StoreFault> {
    store.edit_timeline(
        project_id,
        &EditTimelineRequestWire {
            base_revision,
            timeline: timeline.to_wire(),
            intent: None,
        },
        auth,
    )
}

#[test]
fn genesis_persists_sparse_author_truth_and_exposes_compiled_internal_view() {
    let (_temporary, store, id, auth) = fixture();
    let authored = timeline(0);
    let created = store
        .create_project(&id, &authored, Some("genesis"), &auth)
        .unwrap();

    assert_eq!(created.revision().revision, 1);
    assert_eq!(created.revision().parent_revision, None);
    assert_eq!(created.revision().cause, RevisionCause::Genesis);
    assert_eq!(created.revision().actor.as_str(), "agent:test");
    assert_eq!(created.timeline(), &authored);
    assert_eq!(
        created.canonical().document().canvas.duration.to_string(),
        "2/1"
    );
    assert_eq!(
        std::fs::read(store.root().join("FORMAT")).unwrap(),
        b"valle.project-store@1\n"
    );

    let revision_dir = store
        .project_dir(&id)
        .join("revisions")
        .join(created.revision().revision.to_string());
    assert_eq!(
        std::fs::read(revision_dir.join("timeline.json")).unwrap(),
        timeline_bytes(&authored).unwrap()
    );
    for file in ["timeline.json", "revision.json"] {
        assert!(revision_dir.join(file).is_file());
    }
    assert!(!revision_dir.join("resources.json").exists());
    assert!(!revision_dir.join("diff.json").exists());

    let public: serde_json::Value =
        serde_json::from_slice(&std::fs::read(revision_dir.join("timeline.json")).unwrap())
            .unwrap();
    assert!(public.get("version").is_none());
    assert!(public.get("contract").is_none());
    assert!(public.get("document").is_none());
    assert!(public.get("camera").is_none());

    let revision_bytes = std::fs::read(revision_dir.join("revision.json")).unwrap();
    let revision: serde_json::Value = serde_json::from_slice(&revision_bytes).unwrap();
    assert_eq!(
        revision
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        [
            "actor",
            "cause",
            "createdAt",
            "intent",
            "parentRevision",
            "revision",
            "snapshotDigest",
        ]
    );
    let head: serde_json::Value =
        serde_json::from_slice(&std::fs::read(store.project_dir(&id).join("HEAD")).unwrap())
            .unwrap();
    assert_eq!(
        head.as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["revision", "snapshotDigest"]
    );
}

#[test]
fn genesis_reports_invalid_timeline_input_instead_of_storage_corruption() {
    let (_temporary, store, id, auth) = fixture();
    let empty = decode_timeline(
        &json!({
            "canvas": {"width": 1920, "height": 1080, "fps": 30},
            "tracks": {}
        })
        .to_string(),
    )
    .unwrap();

    let result = store.create_project(&id, &empty, None, &auth);
    assert!(matches!(
        result,
        Err(StoreFault::InvalidTimelineInput(error))
            if error.to_string().contains("at least one positive-duration clip")
    ));
    assert!(!store.project_dir(&id).exists());
}

#[test]
fn authored_bytes_not_compiled_equivalence_decide_revisions() {
    let (_temporary, store, id, auth) = fixture();
    let implicit_black = solid_timeline(None, "#ffffffff");
    let explicit_black = solid_timeline(Some("#000000ff"), "#ffffffff");
    let genesis = store
        .create_project(&id, &implicit_black, None, &auth)
        .unwrap();

    let compiled_explicit = valle_compiler::compile_timeline(explicit_black.clone()).unwrap();
    assert_eq!(genesis.canonical(), &compiled_explicit);
    assert_ne!(
        timeline_bytes(&implicit_black).unwrap(),
        timeline_bytes(&explicit_black).unwrap()
    );

    let committed = edit(
        &store,
        &id,
        genesis.revision().revision,
        &explicit_black,
        &auth,
    )
    .unwrap();
    let committed_id = match committed.result {
        EditTimelineResultWire::Committed { revision: 2 } => 2,
        other => panic!("unexpected result: {other:?}"),
    };
    let head = store.get_timeline(&id, None).unwrap();
    assert_eq!(head.timeline(), &explicit_black);

    let stale_retry = edit(
        &store,
        &id,
        genesis.revision().revision,
        &explicit_black,
        &auth,
    )
    .unwrap();
    assert!(matches!(
        stale_retry.result,
        EditTimelineResultWire::StaleBase { revision } if revision == committed_id
    ));

    let current_retry = edit(&store, &id, committed_id, &explicit_black, &auth).unwrap();
    assert!(matches!(
        current_retry.result,
        EditTimelineResultWire::Unchanged { revision } if revision == committed_id
    ));

    let stale = edit(
        &store,
        &id,
        genesis.revision().revision,
        &timeline(2),
        &auth,
    )
    .unwrap();
    assert!(matches!(
        stale.result,
        EditTimelineResultWire::StaleBase { revision }
            if revision == committed_id
    ));
}

#[test]
fn runtime_fulfillment_cannot_be_smuggled_into_project_history() {
    let (_temporary, store, id, auth) = fixture();
    let genesis = store
        .create_project(&id, &timeline(0), None, &auth)
        .unwrap();
    let resources = store
        .project_dir(&id)
        .join("revisions")
        .join(genesis.revision().revision.to_string())
        .join("resources.json");
    std::fs::write(&resources, b"{}").unwrap();
    assert!(matches!(
        store.get_timeline(&id, None),
        Err(StoreFault::Corrupt("revision directory has unknown member"))
    ));
}

#[test]
fn restore_copies_only_authored_state() {
    let (_temporary, store, id, auth) = fixture();
    let genesis = store
        .create_project(&id, &timeline(0), None, &auth)
        .unwrap();
    let edited_id = match edit(
        &store,
        &id,
        genesis.revision().revision,
        &timeline(1),
        &auth,
    )
    .unwrap()
    .result
    {
        EditTimelineResultWire::Committed { revision } => revision,
        other => panic!("unexpected edit: {other:?}"),
    };

    let stale_identical = store
        .restore_timeline_revision(
            &id,
            genesis.revision().revision,
            edited_id,
            Some("stale restore"),
            &auth,
        )
        .unwrap();
    assert!(matches!(
        stale_identical,
        SnapshotWriteResult::StaleBase { actual }
            if actual.revision().revision == edited_id
    ));

    let restored = match store
        .restore_timeline_revision(
            &id,
            edited_id,
            genesis.revision().revision,
            Some("undo"),
            &auth,
        )
        .unwrap()
    {
        SnapshotWriteResult::Committed { snapshot } => snapshot,
        other => panic!("unexpected restore: {other:?}"),
    };
    assert_eq!(restored.revision().revision, 3);
    assert_eq!(restored.timeline(), genesis.timeline());
    assert_eq!(
        restored.revision().cause,
        RevisionCause::Restore {
            source_revision: genesis.revision().revision,
        }
    );
}

#[test]
fn revision_pagination_is_an_exclusive_linear_cursor_without_diff_payloads() {
    let (_temporary, store, id, auth) = fixture();
    let genesis = store
        .create_project(&id, &timeline(0), None, &auth)
        .unwrap();
    let head_id = match edit(
        &store,
        &id,
        genesis.revision().revision,
        &timeline(1),
        &auth,
    )
    .unwrap()
    .result
    {
        EditTimelineResultWire::Committed { revision } => revision,
        other => panic!("unexpected edit: {other:?}"),
    };

    assert!(matches!(
        store.list_revisions_with_limit(&id, None, 0),
        Err(StoreFault::InvalidPageLimit)
    ));
    let first = store.list_revisions_with_limit(&id, None, 1).unwrap();
    assert_eq!(first.revisions[0].revision, head_id);
    assert_eq!(first.next_cursor, Some(head_id));
    let serialized = serde_json::to_value(&first).unwrap();
    assert!(serialized["revisions"][0].get("diffSummary").is_none());
    assert!(serialized["revisions"][0].get("contract").is_none());
    assert!(serialized["revisions"][0].get("documentHash").is_none());

    let second = store
        .list_revisions_with_limit(&id, first.next_cursor, 1)
        .unwrap();
    assert_eq!(second.revisions[0].revision, genesis.revision().revision);
    assert_eq!(second.next_cursor, None);
}

#[test]
fn revision_pagination_requires_the_cursor_revision_to_exist() {
    let (_temporary, store, id, auth) = fixture();
    let genesis = store
        .create_project(&id, &timeline(0), None, &auth)
        .unwrap();
    assert!(matches!(
        edit(
            &store,
            &id,
            genesis.revision().revision,
            &timeline(1),
            &auth,
        )
        .unwrap()
        .result,
        EditTimelineResultWire::Committed { revision: 2 }
    ));
    let revision_dir = store
        .project_dir(&id)
        .join("revisions")
        .join(genesis.revision().revision.to_string());
    std::fs::rename(&revision_dir, revision_dir.with_extension("missing")).unwrap();

    assert!(matches!(
        store.list_revisions_with_limit(&id, Some(genesis.revision().revision), 1),
        Err(StoreFault::NotFound)
    ));
}

#[test]
fn public_revision_inputs_reject_zero_and_unsafe_json_integers() {
    let (_temporary, store, id, auth) = fixture();
    let genesis = store
        .create_project(&id, &timeline(0), None, &auth)
        .unwrap();

    for invalid in [0, 9_007_199_254_740_992] {
        assert!(matches!(
            store.get_timeline(&id, Some(invalid)),
            Err(StoreFault::InvalidRevision(_))
        ));
        assert!(matches!(
            store.list_revisions_with_limit(&id, Some(invalid), 1),
            Err(StoreFault::InvalidRevision(_))
        ));
        assert!(matches!(
            store
                .restore_timeline_revision(&id, invalid, genesis.revision().revision, None, &auth,),
            Err(StoreFault::InvalidRevision(_))
        ));
        assert!(matches!(
            store
                .restore_timeline_revision(&id, genesis.revision().revision, invalid, None, &auth,),
            Err(StoreFault::InvalidRevision(_))
        ));
        assert!(matches!(
            edit(&store, &id, invalid, &timeline(1), &auth),
            Err(StoreFault::InvalidRevision(_))
        ));
    }
}

#[test]
fn explicit_revision_reads_reject_revisions_not_reachable_from_head() {
    let (_temporary, store, id, auth) = fixture();
    let genesis = store
        .create_project(&id, &timeline(0), None, &auth)
        .unwrap();
    let head_path = store.project_dir(&id).join("HEAD");
    let genesis_head = std::fs::read(&head_path).unwrap();
    let committed_id = match edit(
        &store,
        &id,
        genesis.revision().revision,
        &timeline(1),
        &auth,
    )
    .unwrap()
    .result
    {
        EditTimelineResultWire::Committed { revision } => revision,
        other => panic!("unexpected edit: {other:?}"),
    };

    // Simulate a previously published directory that is no longer part of
    // the authoritative linear history after HEAD is rolled back.
    std::fs::write(&head_path, genesis_head).unwrap();
    assert!(matches!(
        store.get_timeline(&id, Some(committed_id)),
        Err(StoreFault::NotFound)
    ));
    assert_eq!(
        store
            .get_timeline(&id, Some(genesis.revision().revision))
            .unwrap()
            .revision()
            .revision,
        genesis.revision().revision
    );
}

#[test]
fn author_bytes_must_remain_canonical_before_digest_verification() {
    let (_temporary, store, id, auth) = fixture();
    let genesis = store
        .create_project(&id, &timeline(0), None, &auth)
        .unwrap();
    let timeline_path = store
        .project_dir(&id)
        .join("revisions")
        .join(genesis.revision().revision.to_string())
        .join("timeline.json");
    let mut bytes = std::fs::read(&timeline_path).unwrap();
    bytes.push(b' ');
    std::fs::write(timeline_path, bytes).unwrap();

    assert!(matches!(
        store.get_timeline(&id, None),
        Err(StoreFault::Corrupt("timeline.json is not canonical"))
    ));
}

#[test]
fn snapshot_digest_rejects_a_valid_canonical_timeline_replacement() {
    let (_temporary, store, id, auth) = fixture();
    let genesis = store
        .create_project(&id, &timeline(0), None, &auth)
        .unwrap();
    let timeline_path = store
        .project_dir(&id)
        .join("revisions")
        .join(genesis.revision().revision.to_string())
        .join("timeline.json");
    std::fs::write(timeline_path, timeline_bytes(&timeline(1)).unwrap()).unwrap();

    assert!(matches!(
        store.get_timeline(&id, None),
        Err(StoreFault::Corrupt("snapshot digest mismatch"))
    ));
}

#[test]
fn head_digest_rejects_a_valid_canonical_pointer_replacement() {
    let (_temporary, store, id, auth) = fixture();
    let genesis = store
        .create_project(&id, &timeline(0), None, &auth)
        .unwrap();
    let committed = edit(
        &store,
        &id,
        genesis.revision().revision,
        &timeline(1),
        &auth,
    )
    .unwrap();
    assert!(matches!(
        committed.result,
        EditTimelineResultWire::Committed { revision: 2 }
    ));

    let head_path = store.project_dir(&id).join("HEAD");
    let mut head: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&head_path).unwrap()).unwrap();
    head["revision"] = json!(genesis.revision().revision);
    std::fs::write(&head_path, serde_jcs::to_vec(&head).unwrap()).unwrap();

    assert!(matches!(
        store.get_timeline(&id, None),
        Err(StoreFault::Corrupt(
            "HEAD snapshot digest does not match revision"
        ))
    ));
}

#[test]
fn store_format_is_exact_and_missing_committed_format_fails_closed() {
    for (format, expected) in [
        (
            Some(b"valle.project-store@2\n".as_slice()),
            "unsupported project store FORMAT",
        ),
        (None, "project store FORMAT is missing"),
    ] {
        let (_temporary, store, id, auth) = fixture();
        store
            .create_project(&id, &timeline(0), None, &auth)
            .unwrap();
        let format_path = store.root().join("FORMAT");
        match format {
            Some(bytes) => std::fs::write(format_path, bytes).unwrap(),
            None => std::fs::remove_file(format_path).unwrap(),
        }

        assert!(matches!(
            store.get_timeline(&id, None),
            Err(StoreFault::Corrupt(message)) if message == expected
        ));
    }
}

#[cfg(unix)]
#[test]
fn replace_writes_never_follow_preplanted_temporary_symlinks() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("store");
    std::fs::create_dir(&root).unwrap();
    let format_target = temporary.path().join("format-target");
    std::fs::write(&format_target, b"format sentinel").unwrap();
    symlink(&format_target, root.join("FORMAT.tmp")).unwrap();
    symlink(&format_target, root.join(".FORMAT.tmp-attacker")).unwrap();

    let store = ProjectStore::at(&root);
    let id = ProjectId::new("p1").unwrap();
    let auth = AuthenticatedContext::new(Actor::new("agent:test").unwrap());
    let genesis = store
        .create_project(&id, &timeline(0), None, &auth)
        .unwrap();
    assert_eq!(std::fs::read(&format_target).unwrap(), b"format sentinel");

    let project_dir = store.project_dir(&id);
    let head_path = project_dir.join("HEAD");
    let old_head = std::fs::read(&head_path).unwrap();

    let head_temporary_target = temporary.path().join("head-temporary-target");
    std::fs::write(&head_temporary_target, b"head sentinel").unwrap();
    symlink(&head_temporary_target, project_dir.join("HEAD.tmp")).unwrap();
    symlink(
        &head_temporary_target,
        project_dir.join(".HEAD.tmp-attacker"),
    )
    .unwrap();

    let result = edit(
        &store,
        &id,
        genesis.revision().revision,
        &timeline(1),
        &auth,
    )
    .unwrap();
    assert!(matches!(
        result.result,
        EditTimelineResultWire::Committed { .. }
    ));
    assert_eq!(
        std::fs::read(&head_temporary_target).unwrap(),
        b"head sentinel"
    );
    assert_ne!(std::fs::read(&head_path).unwrap(), old_head);
    assert!(
        !std::fs::symlink_metadata(head_path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn store_format_initialization_is_safe_across_independent_stores() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().to_owned();
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();

    for project in ["p1", "p2"] {
        let root = root.clone();
        let barrier = Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            let store = ProjectStore::at(root);
            let id = ProjectId::new(project).unwrap();
            let auth = AuthenticatedContext::new(Actor::new("agent:test").unwrap());
            barrier.wait();
            store.create_project(&id, &timeline(0), None, &auth)
        }));
    }

    barrier.wait();
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
    assert_eq!(
        std::fs::read(temporary.path().join("FORMAT")).unwrap(),
        b"valle.project-store@1\n"
    );
    assert!(
        std::fs::read_dir(temporary.path())
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry
                .file_name()
                .to_string_lossy()
                .starts_with(".FORMAT.tmp-"))
    );
}

#[test]
fn missing_format_never_reclassifies_project_state_as_unpublished() {
    let temporary = tempfile::tempdir().unwrap();
    let store = ProjectStore::at(temporary.path());
    let id = ProjectId::new("p1").unwrap();
    let auth = AuthenticatedContext::new(Actor::new("agent:test").unwrap());
    let marker = store.project_dir(&id).join("revisions/1/existing");
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    std::fs::write(&marker, b"must not be swept").unwrap();

    assert!(matches!(
        store.create_project(&id, &timeline(0), None, &auth),
        Err(StoreFault::Corrupt("project store FORMAT is missing"))
    ));
    assert!(marker.is_file());
    assert!(!store.root().join("FORMAT").exists());
}

#[test]
fn one_owner_serializes_competing_complete_documents() {
    let (temporary, store, id, auth) = fixture();
    let genesis = store
        .create_project(&id, &timeline(0), None, &auth)
        .unwrap();

    let competing_store = ProjectStore::at(temporary.path());
    assert!(matches!(
        edit(
            &competing_store,
            &id,
            genesis.revision().revision,
            &timeline(1),
            &auth,
        ),
        Err(StoreFault::OwnerUnavailable)
    ));

    let store = Arc::new(store);
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for candidate in [timeline(1), timeline(2)] {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        let id = id.clone();
        let auth = auth.clone();
        let base = genesis.revision().revision;
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            edit(&store, &id, base, &candidate, &auth).unwrap()
        }));
    }
    barrier.wait();
    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        results
            .iter()
            .filter(|response| matches!(response.result, EditTimelineResultWire::Committed { .. }))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|response| matches!(response.result, EditTimelineResultWire::StaleBase { .. }))
            .count(),
        1
    );
}

#[test]
fn pre_head_artifacts_are_reclaimed_and_genesis_can_be_retried() {
    let temporary = tempfile::tempdir().unwrap();
    let store = ProjectStore::at(temporary.path());
    let id = ProjectId::new("p1").unwrap();
    let auth = AuthenticatedContext::new(Actor::new("agent:test").unwrap());
    std::fs::write(temporary.path().join("FORMAT"), b"valle.project-store@1\n").unwrap();
    let project_dir = store.project_dir(&id);
    let revisions = project_dir.join("revisions");
    std::fs::create_dir_all(&revisions).unwrap();
    let orphan = revisions.join("1");
    std::fs::create_dir(&orphan).unwrap();
    let orphan_marker = orphan.join("unpublished");
    std::fs::write(&orphan_marker, b"partial").unwrap();
    std::fs::create_dir(revisions.join(".tmp-2")).unwrap();
    std::fs::write(project_dir.join("HEAD.tmp"), b"partial").unwrap();

    let genesis = store
        .create_project(&id, &timeline(0), None, &auth)
        .unwrap();
    assert_eq!(genesis.revision().revision, 1);
    // Numeric revisions deliberately reuse `revisions/1` for the successful
    // genesis. The crashed directory must be replaced, not left alongside it.
    assert!(!orphan_marker.exists());
    assert!(orphan.join("revision.json").is_file());
    assert!(!project_dir.join("HEAD.tmp").exists());

    let uncommitted = revisions.join("2");
    std::fs::create_dir(&uncommitted).unwrap();
    let outcome = edit(
        &store,
        &id,
        genesis.revision().revision,
        genesis.timeline(),
        &auth,
    )
    .unwrap();
    assert!(matches!(
        outcome.result,
        EditTimelineResultWire::Unchanged { .. }
    ));
    assert!(!uncommitted.exists());
}

#[cfg(unix)]
#[test]
fn project_directory_symlink_is_rejected_without_sweeping_its_target() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().unwrap();
    let store = ProjectStore::at(temporary.path());
    let id = ProjectId::new("p1").unwrap();
    let auth = AuthenticatedContext::new(Actor::new("agent:test").unwrap());
    std::fs::write(temporary.path().join("FORMAT"), b"valle.project-store@1\n").unwrap();
    std::fs::create_dir(temporary.path().join("projects")).unwrap();
    let outside = temporary.path().join("outside-project");
    let sentinel = outside.join("revisions/1/sentinel");
    std::fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
    std::fs::write(&sentinel, b"keep").unwrap();
    symlink(&outside, store.project_dir(&id)).unwrap();

    assert!(matches!(
        store.create_project(&id, &timeline(0), None, &auth),
        Err(StoreFault::Corrupt(
            "project store directory member is not a real directory"
        ))
    ));
    assert_eq!(std::fs::read(sentinel).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn revisions_directory_symlink_is_rejected_without_sweeping_its_target() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().unwrap();
    let store = ProjectStore::at(temporary.path());
    let id = ProjectId::new("p1").unwrap();
    let auth = AuthenticatedContext::new(Actor::new("agent:test").unwrap());
    std::fs::write(temporary.path().join("FORMAT"), b"valle.project-store@1\n").unwrap();
    std::fs::create_dir_all(store.project_dir(&id)).unwrap();
    let outside = temporary.path().join("outside-revisions");
    let sentinel = outside.join("1/sentinel");
    std::fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
    std::fs::write(&sentinel, b"keep").unwrap();
    symlink(&outside, store.project_dir(&id).join("revisions")).unwrap();

    assert!(matches!(
        store.create_project(&id, &timeline(0), None, &auth),
        Err(StoreFault::Corrupt(
            "project store directory member is not a real directory"
        ))
    ));
    assert_eq!(std::fs::read(sentinel).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn stable_store_files_must_not_be_symlinks() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().unwrap();
    let outside = temporary.path().join("outside-format");
    std::fs::write(&outside, b"valle.project-store@1\n").unwrap();
    symlink(&outside, temporary.path().join("FORMAT")).unwrap();
    let store = ProjectStore::at(temporary.path());
    let id = ProjectId::new("p1").unwrap();
    let auth = AuthenticatedContext::new(Actor::new("agent:test").unwrap());

    assert!(matches!(
        store.create_project(&id, &timeline(0), None, &auth),
        Err(StoreFault::Corrupt(
            "project store FORMAT is not a regular file"
        ))
    ));
    assert_eq!(std::fs::read(outside).unwrap(), b"valle.project-store@1\n");
}

#[cfg(unix)]
#[test]
fn head_symlink_is_rejected_without_following_it() {
    use std::os::unix::fs::symlink;

    let (_temporary, store, id, auth) = fixture();
    store
        .create_project(&id, &timeline(0), None, &auth)
        .unwrap();
    let head = store.project_dir(&id).join("HEAD");
    let outside = store.root().join("outside-head");
    let bytes = std::fs::read(&head).unwrap();
    std::fs::write(&outside, &bytes).unwrap();
    std::fs::remove_file(&head).unwrap();
    symlink(&outside, &head).unwrap();

    assert!(matches!(
        store.get_timeline(&id, None),
        Err(StoreFault::Corrupt("HEAD is not a regular file"))
    ));
    assert_eq!(std::fs::read(outside).unwrap(), bytes);
}
