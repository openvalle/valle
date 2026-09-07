mod common;

use serde_json::json;
use valle_project::revision::{RevisionCause, SnapshotWriteResult};
use valle_timeline::{
    decode_timeline,
    wire::edit::{EditTimelineRequestWire, EditTimelineResultWire},
};

use common::{fixture, timeline};

#[test]
fn edit_timeline_accepts_one_sparse_document_and_maps_all_outcomes() {
    let (_temporary, store, project_id, auth) = fixture();
    let genesis = store
        .create_project(&project_id, &timeline(0), None, &auth)
        .unwrap();

    let request = EditTimelineRequestWire {
        base_revision: genesis.revision().revision,
        timeline: timeline(1).into_wire(),
        intent: Some("complete rewrite".to_owned()),
    };
    let committed = store.edit_timeline(&project_id, &request, &auth).unwrap();
    let committed_revision = match committed.result {
        EditTimelineResultWire::Committed { revision: 2 } => 2,
        other => panic!("unexpected response: {other:?}"),
    };
    let public = serde_json::to_value(&committed).unwrap();
    assert_eq!(public["outcome"], "committed");
    for removed in [
        "contract",
        "documentHash",
        "resourceManifestHash",
        "diffSummary",
    ] {
        assert!(public.get(removed).is_none(), "{removed} leaked publicly");
    }

    let stale_identical = store.edit_timeline(&project_id, &request, &auth).unwrap();
    assert!(matches!(
        stale_identical.result,
        EditTimelineResultWire::StaleBase { revision } if revision == committed_revision
    ));

    let unchanged = store
        .edit_timeline(
            &project_id,
            &EditTimelineRequestWire {
                base_revision: committed_revision,
                timeline: timeline(1).into_wire(),
                intent: None,
            },
            &auth,
        )
        .unwrap();
    assert!(matches!(
        unchanged.result,
        EditTimelineResultWire::Unchanged { revision } if revision == committed_revision
    ));

    let stale = store
        .edit_timeline(
            &project_id,
            &EditTimelineRequestWire {
                timeline: timeline(2).into_wire(),
                ..request
            },
            &auth,
        )
        .unwrap();
    assert!(matches!(
        stale.result,
        EditTimelineResultWire::StaleBase { revision }
            if revision == committed_revision
    ));
}

#[test]
fn invalid_author_document_is_rejected_without_writing() {
    let (_temporary, store, project_id, auth) = fixture();
    let genesis = store
        .create_project(&project_id, &timeline(0), None, &auth)
        .unwrap();
    let mut invalid = timeline(1).into_wire();
    invalid.canvas.width = 0;

    let response = store
        .edit_timeline(
            &project_id,
            &EditTimelineRequestWire {
                base_revision: genesis.revision().revision,
                timeline: invalid,
                intent: None,
            },
            &auth,
        )
        .unwrap();
    let errors = match response.result {
        EditTimelineResultWire::Rejected { errors } => errors,
        other => panic!("unexpected response: {other:?}"),
    };
    assert!(errors.iter().any(|error| {
        error.code == "invalid_canvas_width" && error.path == "/timeline/canvas/width"
    }));
    let public = serde_json::to_value(EditTimelineResultWire::Rejected { errors }).unwrap();
    assert!(public.get("errors").is_some());
    assert!(public.get("report").is_none());
    assert!(public.get("warnings").is_none());
    assert_eq!(
        store
            .get_timeline(&project_id, None)
            .unwrap()
            .revision()
            .revision,
        1
    );
}

#[test]
fn request_metadata_errors_use_request_root_paths() {
    let (_temporary, store, project_id, auth) = fixture();
    let genesis = store
        .create_project(&project_id, &timeline(0), None, &auth)
        .unwrap();

    let response = store
        .edit_timeline(
            &project_id,
            &EditTimelineRequestWire {
                base_revision: genesis.revision().revision,
                timeline: timeline(1).into_wire(),
                intent: Some("invalid\0intent".to_owned()),
            },
            &auth,
        )
        .unwrap();
    assert!(matches!(
        response.result,
        EditTimelineResultWire::Rejected { errors }
            if errors.len() == 1
                && errors[0].code == "invalid_intent"
                && errors[0].path == "/intent"
    ));
}

#[test]
fn compiler_errors_keep_specific_codes_and_request_root_paths() {
    let (_temporary, store, project_id, auth) = fixture();
    let genesis = store
        .create_project(&project_id, &timeline(0), None, &auth)
        .unwrap();
    let overlapping = decode_timeline(
        &json!({
            "canvas": {"width": 1920, "height": 1080, "fps": 30},
            "tracks": {
                "adjustment": [{
                    "clips": [
                        {"start": 0, "duration": 2, "kind": "color-grade", "temperature": 0.1},
                        {"start": 1, "duration": 1, "kind": "color-grade", "temperature": 0.2}
                    ]
                }]
            }
        })
        .to_string(),
    )
    .unwrap();

    let response = store
        .edit_timeline(
            &project_id,
            &EditTimelineRequestWire {
                base_revision: genesis.revision().revision,
                timeline: overlapping.into_wire(),
                intent: None,
            },
            &auth,
        )
        .unwrap();
    assert!(matches!(
        response.result,
        EditTimelineResultWire::Rejected { errors }
            if errors.len() == 1
                && errors[0].code == "track_overlap"
                && errors[0].path == "/timeline/tracks/adjustment/0/clips/1"
                && errors[0].details.contains_key("start")
                && errors[0].details.contains_key("previousEnd")
    ));
}

#[test]
fn restore_appends_only_the_selected_author_document() {
    let (_temporary, store, project_id, auth) = fixture();
    let genesis = store
        .create_project(&project_id, &timeline(0), None, &auth)
        .unwrap();
    let response = store
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
    let edited_revision = match response.result {
        EditTimelineResultWire::Committed { revision } => revision,
        other => panic!("unexpected response: {other:?}"),
    };

    let restored = match store
        .restore_timeline_revision(
            &project_id,
            edited_revision,
            genesis.revision().revision,
            Some("restore genesis"),
            &auth,
        )
        .unwrap()
    {
        SnapshotWriteResult::Committed { snapshot } => snapshot,
        other => panic!("unexpected restore: {other:?}"),
    };
    assert_eq!(restored.timeline(), genesis.timeline());
    assert_eq!(
        restored.revision().cause,
        RevisionCause::Restore {
            source_revision: genesis.revision().revision,
        }
    );

    let raw = json!({
        "baseRevision": restored.revision().revision,
        "timeline": restored.timeline().to_wire()
    });
    assert!(raw.get("resourceManifest").is_none());
}
