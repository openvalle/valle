use std::collections::BTreeMap;

use valle_timeline::{
    Timeline, TimelineValidationReport, decode_edit_request,
    wire::edit::{
        EditErrorWire, EditTimelineRequestWire, EditTimelineResponseWire, EditTimelineResultWire,
    },
};

use super::store::ProjectStore;
use super::{
    AuthenticatedContext, ProjectId, SnapshotWriteResult, StoreFault, validate_public_revision,
};

#[derive(Debug, thiserror::Error)]
pub enum EditTimelineServiceError {
    #[error("invalid editTimeline request: {0}")]
    Decode(#[from] valle_timeline::EditDecodeError),
    #[error(transparent)]
    Store(#[from] StoreFault),
}

impl ProjectStore {
    /// Strict transport entry point shared by CLI and Studio HTTP.
    pub fn edit_timeline_json(
        &self,
        project_id: &ProjectId,
        request_json: &str,
        auth: &AuthenticatedContext,
    ) -> Result<EditTimelineResponseWire, EditTimelineServiceError> {
        let request = decode_edit_request(request_json)?;
        Ok(self.edit_timeline(project_id, &request, auth)?)
    }

    /// Replace the complete sparse Timeline against one known HEAD.
    /// This is the only public write that accepts Timeline content.
    pub fn edit_timeline(
        &self,
        project_id: &ProjectId,
        request: &EditTimelineRequestWire,
        auth: &AuthenticatedContext,
    ) -> Result<EditTimelineResponseWire, StoreFault> {
        validate_public_revision(request.base_revision)?;
        let timeline = match Timeline::from_wire(request.timeline.clone()) {
            Ok(timeline) => timeline,
            Err(report) => return Ok(rejected(timeline_report(report))),
        };

        match self.write_timeline_snapshot(
            project_id,
            request.base_revision,
            &timeline,
            request.intent.as_deref(),
            auth,
        ) {
            Ok(result) => Ok(edit_response(result)),
            Err(StoreFault::InvalidIntent) => Ok(rejected(vec![EditErrorWire {
                code: "invalid_intent".to_owned(),
                path: "/intent".to_owned(),
                details: BTreeMap::new(),
            }])),
            Err(error) => Err(error),
        }
    }
}

fn timeline_report(report: TimelineValidationReport) -> Vec<EditErrorWire> {
    report
        .diagnostics
        .into_iter()
        .map(|diagnostic| EditErrorWire {
            code: diagnostic.code,
            path: timeline_request_path(&diagnostic.path),
            details: diagnostic.details,
        })
        .collect()
}

fn timeline_request_path(path: &str) -> String {
    if path.is_empty() {
        "/timeline".to_owned()
    } else if path.starts_with('/') {
        format!("/timeline{path}")
    } else {
        format!("/timeline/{path}")
    }
}

fn rejected(errors: Vec<EditErrorWire>) -> EditTimelineResponseWire {
    EditTimelineResponseWire {
        result: EditTimelineResultWire::Rejected { errors },
    }
}

fn edit_response(result: SnapshotWriteResult) -> EditTimelineResponseWire {
    let result = match result {
        SnapshotWriteResult::Committed { snapshot } => EditTimelineResultWire::Committed {
            revision: snapshot.revision().revision,
        },
        SnapshotWriteResult::Unchanged { snapshot } => EditTimelineResultWire::Unchanged {
            revision: snapshot.revision().revision,
        },
        SnapshotWriteResult::StaleBase { actual } => EditTimelineResultWire::StaleBase {
            revision: actual.revision().revision,
        },
        SnapshotWriteResult::Rejected { errors } => EditTimelineResultWire::Rejected { errors },
    };
    EditTimelineResponseWire { result }
}
