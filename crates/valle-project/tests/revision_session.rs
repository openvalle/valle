mod common;

use valle_project::revision::{StudioDocumentSession, StudioSaveOutcome, StudioSessionError};
use valle_timeline::wire::edit::EditTimelineRequestWire;

use common::{fixture, timeline};

#[test]
fn edits_made_during_save_become_the_next_complete_author_save() {
    let (_root, store, project_id, auth) = fixture();
    let genesis = store
        .create_project(&project_id, &timeline(0), None, &auth)
        .unwrap();
    let mut session = StudioDocumentSession::open(&genesis);
    session.replace_working_copy(timeline(1));

    let first = session.begin_save(Some("first".to_owned())).unwrap();
    session.replace_working_copy(timeline(2));
    let (token, project, request) = first.into_parts();
    let response = store.edit_timeline(&project, &request, &auth).unwrap();
    assert!(matches!(
        session.finish_save(token, response).unwrap(),
        StudioSaveOutcome::Saved {
            has_pending_changes: true,
            ..
        }
    ));

    let second = session.begin_save(Some("second".to_owned())).unwrap();
    let (token, project, request) = second.into_parts();
    let response = store.edit_timeline(&project, &request, &auth).unwrap();
    assert!(matches!(
        session.finish_save(token, response).unwrap(),
        StudioSaveOutcome::Saved {
            has_pending_changes: false,
            ..
        }
    ));
    assert_eq!(
        store.get_timeline(&project_id, None).unwrap().timeline(),
        &timeline(2)
    );
}

#[test]
fn stale_reload_preserves_dirty_author_document_and_original_base() {
    let (_root, store, project_id, auth) = fixture();
    let genesis = store
        .create_project(&project_id, &timeline(0), None, &auth)
        .unwrap();
    let mut session = StudioDocumentSession::open(&genesis);
    session.replace_working_copy(timeline(1));
    let pending = session.begin_save(None).unwrap();

    store
        .edit_timeline(
            &project_id,
            &EditTimelineRequestWire {
                base_revision: genesis.revision().revision,
                timeline: timeline(2).into_wire(),
                intent: None,
            },
            &auth,
        )
        .unwrap();
    let (token, project, request) = pending.into_parts();
    let response = store.edit_timeline(&project, &request, &auth).unwrap();
    assert!(matches!(
        session.finish_save(token, response).unwrap(),
        StudioSaveOutcome::StaleBase { .. }
    ));

    let head = store.get_timeline(&project_id, None).unwrap();
    assert_eq!(
        session.reload(&head),
        Err(StudioSessionError::StaleWorkingCopy {
            expected_revision: genesis.revision().revision,
            actual_revision: head.revision().revision,
        })
    );
    assert!(session.reload_required());
    assert!(session.has_unsaved_changes());
    assert_eq!(session.base_revision(), genesis.revision().revision);
    assert_eq!(session.working_copy(), &timeline(1));
    assert_eq!(
        session.begin_save(None),
        Err(StudioSessionError::ReloadRequired)
    );
}

#[test]
fn clean_working_copy_reload_fast_forwards_to_head() {
    let (_root, store, project_id, auth) = fixture();
    let genesis = store
        .create_project(&project_id, &timeline(0), None, &auth)
        .unwrap();
    let mut session = StudioDocumentSession::open(&genesis);
    store
        .edit_timeline(
            &project_id,
            &EditTimelineRequestWire {
                base_revision: genesis.revision().revision,
                timeline: timeline(1).into_wire(),
                intent: None,
            },
            &auth,
        )
        .unwrap();

    let head = store.get_timeline(&project_id, None).unwrap();
    session.reload(&head).unwrap();
    assert_eq!(session.base_revision(), head.revision().revision);
    assert_eq!(session.working_copy(), head.timeline());
    assert!(!session.has_unsaved_changes());
    assert!(!session.reload_required());
}

#[test]
fn dirty_reload_at_the_same_revision_keeps_local_author_state() {
    let (_root, store, project_id, auth) = fixture();
    let genesis = store
        .create_project(&project_id, &timeline(0), None, &auth)
        .unwrap();
    let mut session = StudioDocumentSession::open(&genesis);
    session.replace_working_copy(timeline(1));

    session.reload(&genesis).unwrap();
    assert_eq!(session.working_copy(), &timeline(1));
    assert!(session.has_unsaved_changes());
    assert!(!session.reload_required());
}
