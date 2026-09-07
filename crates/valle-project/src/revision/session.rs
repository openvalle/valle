//! Studio's local full-document save state machine.
//!
//! Gestures may produce any locally valid working copy, but persistence is
//! deliberately narrow: a save yields exactly one editTimeline request
//! containing the complete sparse Timeline.

use valle_timeline::{
    Timeline,
    wire::edit::{
        EditErrorWire, EditTimelineRequestWire, EditTimelineResponseWire, EditTimelineResultWire,
    },
};

use super::{ProjectId, ProjectTimelineSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StudioSaveToken(u64);

#[derive(Debug, Clone, PartialEq)]
pub struct StudioSaveRequest {
    token: StudioSaveToken,
    project_id: ProjectId,
    request: EditTimelineRequestWire,
}

impl StudioSaveRequest {
    pub const fn token(&self) -> StudioSaveToken {
        self.token
    }

    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    pub fn request(&self) -> &EditTimelineRequestWire {
        &self.request
    }

    pub fn into_parts(self) -> (StudioSaveToken, ProjectId, EditTimelineRequestWire) {
        (self.token, self.project_id, self.request)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StudioSaveOutcome {
    Saved {
        revision: u64,
        has_pending_changes: bool,
    },
    StaleBase {
        revision: u64,
    },
    Rejected {
        errors: Vec<EditErrorWire>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StudioSessionError {
    #[error("a Studio save is already in flight")]
    SaveInFlight,
    #[error("Studio must reload HEAD before saving after staleBase")]
    ReloadRequired,
    #[error("save response does not match the in-flight request")]
    MismatchedResponse,
    #[error("Studio session snapshot belongs to a different project")]
    WrongProject,
    #[error(
        "local Timeline is based on revision {expected_revision}, but HEAD is {actual_revision}"
    )]
    StaleWorkingCopy {
        expected_revision: u64,
        actual_revision: u64,
    },
}

/// One Studio tab's local working-copy state.
#[derive(Debug, Clone)]
pub struct StudioDocumentSession {
    project_id: ProjectId,
    base_revision: u64,
    working_copy: Timeline,
    generation: u64,
    acknowledged_generation: u64,
    next_token: u64,
    in_flight: Option<SaveFlight>,
    reload_required: bool,
}

#[derive(Debug, Clone)]
struct SaveFlight {
    token: StudioSaveToken,
    generation: u64,
}

impl StudioDocumentSession {
    pub fn open(snapshot: &ProjectTimelineSnapshot) -> Self {
        Self {
            project_id: snapshot.project_id().clone(),
            base_revision: snapshot.revision().revision,
            working_copy: snapshot.timeline().clone(),
            generation: 0,
            acknowledged_generation: 0,
            next_token: 1,
            in_flight: None,
            reload_required: false,
        }
    }

    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    pub const fn base_revision(&self) -> u64 {
        self.base_revision
    }

    pub fn working_copy(&self) -> &Timeline {
        &self.working_copy
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.generation != self.acknowledged_generation
    }

    pub const fn save_in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    pub const fn reload_required(&self) -> bool {
        self.reload_required
    }

    /// Replace the local working copy after any number of local gestures.
    pub fn replace_working_copy(&mut self, timeline: Timeline) {
        if self.working_copy != timeline {
            self.working_copy = timeline;
            self.generation = self.generation.saturating_add(1);
        }
    }

    /// Start one full-document save. Edits made while it is in flight remain
    /// in the working copy and become the next full save.
    pub fn begin_save(
        &mut self,
        intent: Option<String>,
    ) -> Result<StudioSaveRequest, StudioSessionError> {
        if self.in_flight.is_some() {
            return Err(StudioSessionError::SaveInFlight);
        }
        if self.reload_required {
            return Err(StudioSessionError::ReloadRequired);
        }
        let token = StudioSaveToken(self.next_token);
        self.next_token = self.next_token.saturating_add(1);
        self.in_flight = Some(SaveFlight {
            token,
            generation: self.generation,
        });
        Ok(StudioSaveRequest {
            token,
            project_id: self.project_id.clone(),
            request: EditTimelineRequestWire {
                base_revision: self.base_revision,
                timeline: self.working_copy.to_wire(),
                intent,
            },
        })
    }

    pub fn finish_save(
        &mut self,
        token: StudioSaveToken,
        response: EditTimelineResponseWire,
    ) -> Result<StudioSaveOutcome, StudioSessionError> {
        let Some(flight) = self.in_flight.take() else {
            return Err(StudioSessionError::MismatchedResponse);
        };
        if flight.token != token {
            self.in_flight = Some(flight);
            return Err(StudioSessionError::MismatchedResponse);
        }

        match response.result {
            EditTimelineResultWire::Committed { revision }
            | EditTimelineResultWire::Unchanged { revision } => {
                self.base_revision = revision;
                self.acknowledged_generation = flight.generation;
                Ok(StudioSaveOutcome::Saved {
                    revision,
                    has_pending_changes: self.has_unsaved_changes(),
                })
            }
            EditTimelineResultWire::StaleBase { revision } => {
                self.reload_required = true;
                Ok(StudioSaveOutcome::StaleBase { revision })
            }
            EditTimelineResultWire::Rejected { errors } => {
                Ok(StudioSaveOutcome::Rejected { errors })
            }
        }
    }

    /// Install a freshly loaded HEAD only when the working copy is clean.
    ///
    /// A dirty working copy is never merged or rebased automatically. If HEAD
    /// moved, the complete local document and its base are preserved so the
    /// caller can explicitly reopen, discard, or reconstruct the edit.
    pub fn reload(&mut self, snapshot: &ProjectTimelineSnapshot) -> Result<(), StudioSessionError> {
        if self.in_flight.is_some() {
            return Err(StudioSessionError::SaveInFlight);
        }
        if snapshot.project_id() != &self.project_id {
            return Err(StudioSessionError::WrongProject);
        }

        let actual_revision = snapshot.revision().revision;
        if self.has_unsaved_changes() && actual_revision != self.base_revision {
            self.reload_required = true;
            return Err(StudioSessionError::StaleWorkingCopy {
                expected_revision: self.base_revision,
                actual_revision,
            });
        }

        if self.has_unsaved_changes() {
            self.reload_required = false;
            return Ok(());
        }

        if self.working_copy != *snapshot.timeline() {
            self.generation = self.generation.saturating_add(1);
        }
        self.base_revision = snapshot.revision().revision;
        self.working_copy = snapshot.timeline().clone();
        self.acknowledged_generation = self.generation;
        self.reload_required = false;
        Ok(())
    }
}
