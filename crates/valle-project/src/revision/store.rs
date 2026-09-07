use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use valle_compiler::{CompileTimelineError, compile_timeline};
use valle_timeline::internal::{CanonicalTimeline, ContentDigest, DiagnosticSeverity};
use valle_timeline::{Timeline, decode_timeline, timeline_bytes, wire::edit::EditErrorWire};

use super::{
    AuthenticatedContext, ListedTimelineRevision, MAX_PUBLIC_REVISION, ProjectId,
    ProjectTimelineSnapshot, RevisionCause, RevisionPage, SnapshotWriteResult, StoreFault,
    TimelineRevision, Timestamp, validate_public_revision,
};

const HEAD_FILE: &str = "HEAD";
const STORE_FORMAT_FILE: &str = "FORMAT";
const STORE_FORMAT_LOCK_FILE: &str = ".project-store-format.lock";
const STORE_FORMAT_TMP_FILE: &str = "FORMAT.tmp";
const STORE_FORMAT_TMP_PREFIX: &str = ".FORMAT.tmp-";
const STORE_FORMAT: &[u8] = b"valle.project-store@1\n";
const REVISIONS_DIR: &str = "revisions";
const TIMELINE_FILE: &str = "timeline.json";
const REVISION_FILE: &str = "revision.json";
const OWNER_LOCK_FILE: &str = ".owner.lock";
const INTENT_MAX_BYTES: usize = 2_048;
const DEFAULT_PAGE_SIZE: usize = 50;
const STORED_SNAPSHOT_DIGEST_DOMAIN: &[u8] = b"valle.project-snapshot/1\0";

/// Filesystem-backed single-owner Project revision store.
#[derive(Debug, Clone)]
pub struct ProjectStore {
    root: PathBuf,
    owners: Arc<Mutex<BTreeMap<ProjectId, Arc<ProjectOwner>>>>,
}

impl ProjectStore {
    /// `root` contains the store's `projects/` directory.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            owners: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.root.join("projects")
    }

    pub fn project_dir(&self, project_id: &ProjectId) -> PathBuf {
        self.projects_dir().join(project_id.as_str())
    }

    fn initialize_or_validate_store_format(&self) -> Result<(), StoreFault> {
        std::fs::create_dir_all(&self.root)?;
        require_real_directory(&self.root)?;
        let lock_path = self.root.join(STORE_FORMAT_LOCK_FILE);
        if path_entry_exists(&lock_path)? {
            require_real_file(
                &lock_path,
                "project store format lock is not a regular file",
            )?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)?;
        require_real_file(
            &lock_path,
            "project store format lock is not a regular file",
        )?;
        lock.lock_exclusive()?;

        sweep_replace_temporary_files(
            &self.root,
            STORE_FORMAT_TMP_PREFIX,
            Some(STORE_FORMAT_TMP_FILE),
        )?;

        let format_path = self.root.join(STORE_FORMAT_FILE);
        if path_entry_exists(&format_path)? {
            return validate_store_format_bytes(&format_path);
        }
        if project_store_has_entries(&self.root)? {
            return Err(StoreFault::Corrupt("project store FORMAT is missing"));
        }

        write_atomic_replace_synced(
            &self.root,
            STORE_FORMAT_TMP_PREFIX,
            &format_path,
            STORE_FORMAT,
            "after-store-format-write",
            "after-store-format-fsync",
        )?;
        super::crash::maybe_crash("after-store-format-rename");
        sync_dir(&self.root)?;
        super::crash::maybe_crash("after-store-root-fsync");
        Ok(())
    }

    fn validate_store_format(&self) -> Result<(), StoreFault> {
        if !path_entry_exists(&self.root)? {
            return Err(StoreFault::NotFound);
        }
        require_real_directory(&self.root)?;
        let format_path = self.root.join(STORE_FORMAT_FILE);
        match std::fs::symlink_metadata(&format_path) {
            Ok(_) => return validate_store_format_bytes(&format_path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(StoreFault::Io(error)),
        }
        if project_store_has_entries(&self.root)? {
            Err(StoreFault::Corrupt("project store FORMAT is missing"))
        } else {
            Err(StoreFault::NotFound)
        }
    }

    pub fn create_project(
        &self,
        project_id: &ProjectId,
        timeline: &Timeline,
        intent: Option<&str>,
        auth: &AuthenticatedContext,
    ) -> Result<ProjectTimelineSnapshot, StoreFault> {
        validate_intent(intent)?;
        let canonical =
            compile_timeline_document(timeline).map_err(StoreFault::InvalidTimelineInput)?;
        self.initialize_or_validate_store_format()?;
        create_or_validate_directory(&self.projects_dir())?;
        let project_dir = self.project_dir(project_id);
        let created = match std::fs::create_dir(&project_dir) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                require_real_directory(&project_dir)?;
                false
            }
            Err(error) => return Err(error.into()),
        };
        if created {
            std::fs::create_dir(project_dir.join(REVISIONS_DIR))?;
        } else {
            create_or_validate_directory(&project_dir.join(REVISIONS_DIR))?;
        }

        let owner = self.acquire_owner(project_id, &project_dir)?;
        let _writer = owner
            .writer
            .lock()
            .map_err(|_| StoreFault::OwnerUnavailable)?;
        if path_entry_exists(&project_dir.join(HEAD_FILE))? {
            return Err(StoreFault::AlreadyExists);
        }
        // A previous genesis may have crashed before publishing HEAD. With no
        // commit point, every revision directory is an orphan and can be
        // reclaimed before the caller retries genesis.
        sweep_temporary_revisions(&project_dir)?;
        sweep_all_unpublished_revisions(&project_dir)?;
        let revision = self.build_revision(None, 1, RevisionCause::Genesis, intent, auth)?;
        let snapshot = close_snapshot(project_id.clone(), revision, timeline.clone(), canonical);
        self.persist_revision(project_id, snapshot.revision(), snapshot.timeline())?;
        Ok(snapshot)
    }

    pub fn get_timeline(
        &self,
        project_id: &ProjectId,
        revision: Option<u64>,
    ) -> Result<ProjectTimelineSnapshot, StoreFault> {
        if let Some(revision) = revision {
            validate_public_revision(revision)?;
        }
        self.validate_store_format()?;
        let project_dir = self.project_dir(project_id);
        if !path_entry_exists(&project_dir)? {
            return Err(StoreFault::NotFound);
        }
        require_real_directory(&project_dir)?;
        require_real_directory(&project_dir.join(REVISIONS_DIR))?;
        let head = read_head(&project_dir)?;
        let revision = revision.unwrap_or(head.revision);
        if revision == 0 || revision > head.revision {
            return Err(StoreFault::NotFound);
        }
        let loaded = self.read_snapshot(project_id, revision)?;
        if revision == head.revision && loaded.digest != head.snapshot_digest {
            return Err(StoreFault::Corrupt(
                "HEAD snapshot digest does not match revision",
            ));
        }
        Ok(loaded.snapshot)
    }

    pub fn list_revisions(
        &self,
        project_id: &ProjectId,
        cursor: Option<u64>,
    ) -> Result<RevisionPage, StoreFault> {
        self.list_revisions_with_limit(project_id, cursor, DEFAULT_PAGE_SIZE)
    }

    pub fn list_revisions_with_limit(
        &self,
        project_id: &ProjectId,
        cursor: Option<u64>,
        limit: usize,
    ) -> Result<RevisionPage, StoreFault> {
        if let Some(cursor) = cursor {
            validate_public_revision(cursor)?;
        }
        if limit == 0 {
            return Err(StoreFault::InvalidPageLimit);
        }
        let head = self.get_timeline(project_id, None)?;
        let mut current = match cursor {
            Some(cursor) => {
                self.get_timeline(project_id, Some(cursor))?
                    .revision
                    .parent_revision
            }
            None => Some(head.revision.revision),
        };

        let mut revisions = Vec::new();
        while let Some(revision) = current {
            if revisions.len() == limit {
                break;
            }
            let snapshot = self.read_snapshot(project_id, revision)?.snapshot;
            current = snapshot.revision.parent_revision;
            revisions.push(ListedTimelineRevision::from(snapshot.revision));
        }
        let next_cursor = if current.is_some() {
            revisions.last().map(|entry| entry.revision)
        } else {
            None
        };
        Ok(RevisionPage {
            revisions,
            next_cursor,
        })
    }

    pub(super) fn write_timeline_snapshot(
        &self,
        project_id: &ProjectId,
        base_revision: u64,
        timeline: &Timeline,
        intent: Option<&str>,
        auth: &AuthenticatedContext,
    ) -> Result<SnapshotWriteResult, StoreFault> {
        self.mutate(
            project_id,
            base_revision,
            SnapshotMutation::TimelineEdit {
                timeline: timeline.clone(),
            },
            intent,
            auth,
        )
    }

    pub fn restore_timeline_revision(
        &self,
        project_id: &ProjectId,
        base_revision: u64,
        source_revision: u64,
        intent: Option<&str>,
        auth: &AuthenticatedContext,
    ) -> Result<SnapshotWriteResult, StoreFault> {
        self.mutate(
            project_id,
            base_revision,
            SnapshotMutation::Restore { source_revision },
            intent,
            auth,
        )
    }

    fn mutate(
        &self,
        project_id: &ProjectId,
        base_revision: u64,
        mutation: SnapshotMutation,
        intent: Option<&str>,
        auth: &AuthenticatedContext,
    ) -> Result<SnapshotWriteResult, StoreFault> {
        validate_public_revision(base_revision)?;
        if let SnapshotMutation::Restore { source_revision } = &mutation {
            validate_public_revision(*source_revision)?;
        }
        validate_intent(intent)?;
        self.validate_store_format()?;
        let project_dir = self.project_dir(project_id);
        if !path_entry_exists(&project_dir)? {
            return Err(StoreFault::NotFound);
        }
        require_real_directory(&project_dir)?;
        require_real_directory(&project_dir.join(REVISIONS_DIR))?;
        let owner = self.acquire_owner(project_id, &project_dir)?;
        let _writer = owner
            .writer
            .lock()
            .map_err(|_| StoreFault::OwnerUnavailable)?;
        sweep_temporary_revisions(&project_dir)?;

        let head_pointer = read_head(&project_dir)?;
        let loaded_head = self.read_snapshot(project_id, head_pointer.revision)?;
        if loaded_head.snapshot.revision.revision != head_pointer.revision
            || loaded_head.digest != head_pointer.snapshot_digest
        {
            return Err(StoreFault::Corrupt("HEAD does not match revision"));
        }
        let head = loaded_head.snapshot;
        self.gc_orphan_revisions(project_id, head.revision.revision)?;

        if base_revision != head.revision.revision {
            return Ok(SnapshotWriteResult::StaleBase { actual: head });
        }

        let (timeline, canonical, cause) = match mutation {
            SnapshotMutation::TimelineEdit { timeline } => {
                let canonical = match compile_timeline_document(&timeline) {
                    Ok(canonical) => canonical,
                    Err(error) => {
                        return Ok(SnapshotWriteResult::Rejected {
                            errors: compile_timeline_errors(error),
                        });
                    }
                };
                (timeline, canonical, RevisionCause::TimelineEdit)
            }
            SnapshotMutation::Restore { source_revision } => {
                if source_revision > head.revision.revision {
                    return Err(StoreFault::NotFound);
                }
                let source = self.read_snapshot(project_id, source_revision)?.snapshot;
                (
                    source.timeline().clone(),
                    source.canonical().clone(),
                    RevisionCause::Restore { source_revision },
                )
            }
        };

        if timeline == *head.timeline() {
            return Ok(SnapshotWriteResult::Unchanged { snapshot: head });
        }
        let revision = head
            .revision
            .revision
            .checked_add(1)
            .ok_or(StoreFault::Corrupt("revision overflow"))?;
        let revision = self.build_revision(Some(&head.revision), revision, cause, intent, auth)?;
        let snapshot = close_snapshot(project_id.clone(), revision, timeline, canonical);
        self.persist_revision(project_id, snapshot.revision(), snapshot.timeline())?;
        Ok(SnapshotWriteResult::Committed { snapshot })
    }

    fn build_revision(
        &self,
        parent: Option<&TimelineRevision>,
        revision: u64,
        cause: RevisionCause,
        intent: Option<&str>,
        auth: &AuthenticatedContext,
    ) -> Result<TimelineRevision, StoreFault> {
        if revision == 0 || revision > MAX_PUBLIC_REVISION {
            return Err(StoreFault::Corrupt("revision exceeds valle-json range"));
        }
        let created_at =
            Timestamp::server_issued(super::clock::iso8601(super::clock::now_millis()));
        let actor = auth.actor().clone();
        let intent = intent.map(str::to_owned);
        let parent_revision = parent.map(|parent| parent.revision);
        Ok(TimelineRevision {
            revision,
            parent_revision,
            created_at,
            actor,
            cause,
            intent,
        })
    }

    fn persist_revision(
        &self,
        project_id: &ProjectId,
        revision: &TimelineRevision,
        timeline: &Timeline,
    ) -> Result<(), StoreFault> {
        let project_dir = self.project_dir(project_id);
        let revisions_dir = project_dir.join(REVISIONS_DIR);
        let revision_name = revision.revision.to_string();
        let temporary_dir = revisions_dir.join(format!(".tmp-{revision_name}"));
        let final_dir = revisions_dir.join(&revision_name);
        if path_entry_exists(&temporary_dir)? || path_entry_exists(&final_dir)? {
            return Err(StoreFault::Corrupt("revision number collision"));
        }
        std::fs::create_dir(&temporary_dir)?;

        let timeline_bytes = timeline_bytes(timeline).map_err(|_| StoreFault::Canonicalization)?;
        let snapshot_digest = snapshot_digest_for_revision(revision, &timeline_bytes)?;

        write_new_synced(
            &temporary_dir.join(TIMELINE_FILE),
            &timeline_bytes,
            "after-timeline-write",
            "after-timeline-fsync",
        )?;
        write_new_synced(
            &temporary_dir.join(REVISION_FILE),
            &canonical_json(&StoredTimelineRevision::new(revision, snapshot_digest))?,
            "after-revision-metadata-write",
            "after-revision-metadata-fsync",
        )?;
        sync_dir(&temporary_dir)?;
        super::crash::maybe_crash("after-revision-directory-fsync");

        std::fs::rename(&temporary_dir, &final_dir)?;
        super::crash::maybe_crash("after-revision-rename");
        sync_dir(&revisions_dir)?;
        super::crash::maybe_crash("after-revisions-directory-fsync");

        let head = HeadPointer {
            revision: revision.revision,
            snapshot_digest,
        };
        let head_bytes = canonical_json(&head)?;
        write_atomic_replace_synced(
            &project_dir,
            ".HEAD.tmp-",
            &project_dir.join(HEAD_FILE),
            &head_bytes,
            "after-head-file-write",
            "after-head-file-fsync",
        )?;
        super::crash::maybe_crash("after-head-rename");
        sync_dir(&project_dir)?;
        super::crash::maybe_crash("after-project-directory-fsync");
        Ok(())
    }

    fn read_snapshot(
        &self,
        project_id: &ProjectId,
        revision: u64,
    ) -> Result<LoadedSnapshot, StoreFault> {
        let revision_dir = self
            .project_dir(project_id)
            .join(REVISIONS_DIR)
            .join(revision.to_string());
        let Ok(metadata) = std::fs::symlink_metadata(&revision_dir) else {
            return Err(StoreFault::NotFound);
        };
        if !metadata.file_type().is_dir() {
            return Err(StoreFault::Corrupt("revision path is not a directory"));
        }
        validate_revision_directory(&revision_dir)?;

        let revision_bytes = std::fs::read(revision_dir.join(REVISION_FILE))?;
        let stored: StoredTimelineRevision = serde_json::from_slice(&revision_bytes)
            .map_err(|_| StoreFault::Corrupt("invalid revision.json"))?;
        if stored.revision != revision {
            return Err(StoreFault::Corrupt("revision number mismatch"));
        }
        if stored.revision == 0
            || stored.revision > MAX_PUBLIC_REVISION
            || validate_intent(stored.intent.as_deref()).is_err()
            || matches!(&stored.cause, RevisionCause::Genesis)
                != (stored.revision == 1 && stored.parent_revision.is_none())
            || (!matches!(&stored.cause, RevisionCause::Genesis)
                && stored.parent_revision != stored.revision.checked_sub(1))
        {
            return Err(StoreFault::Corrupt("revision metadata invariant failed"));
        }
        if canonical_json(&stored)? != revision_bytes {
            return Err(StoreFault::Corrupt("revision.json is not canonical"));
        }

        let stored_timeline_bytes = std::fs::read(revision_dir.join(TIMELINE_FILE))?;
        let timeline_text = std::str::from_utf8(&stored_timeline_bytes)
            .map_err(|_| StoreFault::Corrupt("timeline.json is not UTF-8"))?;
        let timeline = decode_timeline(timeline_text)
            .map_err(|_| StoreFault::Corrupt("invalid timeline.json"))?;
        if timeline_bytes(&timeline).map_err(|_| StoreFault::Canonicalization)?
            != stored_timeline_bytes
        {
            return Err(StoreFault::Corrupt("timeline.json is not canonical"));
        }
        let expected_digest = snapshot_digest_for_stored(&stored, &stored_timeline_bytes)?;
        if expected_digest != stored.snapshot_digest {
            return Err(StoreFault::Corrupt("snapshot digest mismatch"));
        }
        let canonical = compile_timeline_document(&timeline)
            .map_err(|_| StoreFault::Corrupt("stored Timeline failed to compile"))?;

        let digest = stored.snapshot_digest;
        let snapshot = close_snapshot(project_id.clone(), stored.into(), timeline, canonical);
        Ok(LoadedSnapshot { snapshot, digest })
    }

    fn acquire_owner(
        &self,
        project_id: &ProjectId,
        project_dir: &Path,
    ) -> Result<Arc<ProjectOwner>, StoreFault> {
        let mut owners = self
            .owners
            .lock()
            .map_err(|_| StoreFault::OwnerUnavailable)?;
        if let Some(owner) = owners.get(project_id) {
            return Ok(Arc::clone(owner));
        }
        let lock_path = project_dir.join(OWNER_LOCK_FILE);
        if path_entry_exists(&lock_path)? {
            require_real_file(&lock_path, "project owner lock is not a regular file")?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        require_real_file(&lock_path, "project owner lock is not a regular file")?;
        file.try_lock_exclusive()
            .map_err(|_| StoreFault::OwnerUnavailable)?;
        let owner = Arc::new(ProjectOwner {
            _file: file,
            writer: Mutex::new(()),
        });
        owners.insert(project_id.clone(), Arc::clone(&owner));
        Ok(owner)
    }

    fn gc_orphan_revisions(&self, project_id: &ProjectId, head: u64) -> Result<(), StoreFault> {
        let revisions = self.project_dir(project_id).join(REVISIONS_DIR);
        for entry in std::fs::read_dir(&revisions)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(revision) = name.parse::<u64>() else {
                continue;
            };
            if revision == 0 || revision > head {
                std::fs::remove_dir_all(entry.path())?;
            }
        }
        sync_dir(&revisions)?;
        Ok(())
    }
}

fn close_snapshot(
    project_id: ProjectId,
    revision: TimelineRevision,
    timeline: Timeline,
    canonical: CanonicalTimeline,
) -> ProjectTimelineSnapshot {
    ProjectTimelineSnapshot::close(project_id, revision, timeline, canonical)
}

#[derive(Debug)]
enum SnapshotMutation {
    TimelineEdit { timeline: Timeline },
    Restore { source_revision: u64 },
}

#[derive(Debug)]
struct ProjectOwner {
    _file: File,
    writer: Mutex<()>,
}

#[derive(Debug)]
struct LoadedSnapshot {
    snapshot: ProjectTimelineSnapshot,
    digest: ContentDigest,
}

/// Private integrity digest for one persisted Project snapshot.
///
/// The exact preimage is `valle.project-snapshot/1\0`, followed by the
/// big-endian `u64` byte length and bytes of canonical revision metadata
/// without `snapshotDigest`, then the big-endian `u64` byte length and exact
/// persisted `timeline.json` bytes. Runtime fulfillment is deliberately not a
/// Project revision member.
fn snapshot_digest_for_revision(
    revision: &TimelineRevision,
    timeline_bytes: &[u8],
) -> Result<ContentDigest, StoreFault> {
    let metadata = StoredSnapshotMetadata {
        revision: revision.revision,
        parent_revision: revision.parent_revision,
        created_at: &revision.created_at,
        actor: &revision.actor,
        cause: &revision.cause,
        intent: revision.intent.as_deref(),
    };
    snapshot_digest_from_parts(&canonical_json(&metadata)?, timeline_bytes)
}

fn snapshot_digest_for_stored(
    revision: &StoredTimelineRevision,
    timeline_bytes: &[u8],
) -> Result<ContentDigest, StoreFault> {
    let metadata = StoredSnapshotMetadata {
        revision: revision.revision,
        parent_revision: revision.parent_revision,
        created_at: &revision.created_at,
        actor: &revision.actor,
        cause: &revision.cause,
        intent: revision.intent.as_deref(),
    };
    snapshot_digest_from_parts(&canonical_json(&metadata)?, timeline_bytes)
}

fn snapshot_digest_from_parts(
    metadata: &[u8],
    timeline: &[u8],
) -> Result<ContentDigest, StoreFault> {
    let mut hasher = Sha256::new();
    hasher.update(STORED_SNAPSHOT_DIGEST_DOMAIN);
    update_length_framed(&mut hasher, metadata)?;
    update_length_framed(&mut hasher, timeline)?;
    Ok(ContentDigest::from_bytes(hasher.finalize().into()))
}

fn update_length_framed(hasher: &mut Sha256, bytes: &[u8]) -> Result<(), StoreFault> {
    let length = u64::try_from(bytes.len()).map_err(|_| StoreFault::Canonicalization)?;
    hasher.update(length.to_be_bytes());
    hasher.update(bytes);
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredSnapshotMetadata<'a> {
    revision: u64,
    parent_revision: Option<u64>,
    created_at: &'a Timestamp,
    actor: &'a super::Actor,
    cause: &'a RevisionCause,
    intent: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredTimelineRevision {
    snapshot_digest: ContentDigest,
    revision: u64,
    #[serde(deserialize_with = "required_option")]
    parent_revision: Option<u64>,
    created_at: Timestamp,
    actor: super::Actor,
    cause: RevisionCause,
    #[serde(deserialize_with = "required_option")]
    intent: Option<String>,
}

impl StoredTimelineRevision {
    fn new(revision: &TimelineRevision, snapshot_digest: ContentDigest) -> Self {
        Self {
            snapshot_digest,
            revision: revision.revision,
            parent_revision: revision.parent_revision,
            created_at: revision.created_at.clone(),
            actor: revision.actor.clone(),
            cause: revision.cause.clone(),
            intent: revision.intent.clone(),
        }
    }
}

impl From<StoredTimelineRevision> for TimelineRevision {
    fn from(revision: StoredTimelineRevision) -> Self {
        Self {
            revision: revision.revision,
            parent_revision: revision.parent_revision,
            created_at: revision.created_at,
            actor: revision.actor,
            cause: revision.cause,
            intent: revision.intent,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HeadPointer {
    revision: u64,
    snapshot_digest: ContentDigest,
}

fn read_head(project_dir: &Path) -> Result<HeadPointer, StoreFault> {
    let path = project_dir.join(HEAD_FILE);
    require_real_file(&path, "HEAD is not a regular file").map_err(|error| match error {
        StoreFault::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
            StoreFault::NotFound
        }
        error => error,
    })?;
    let bytes = std::fs::read(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            StoreFault::NotFound
        } else {
            StoreFault::Io(error)
        }
    })?;
    let head: HeadPointer =
        serde_json::from_slice(&bytes).map_err(|_| StoreFault::Corrupt("invalid HEAD"))?;
    if canonical_json(&head)? != bytes {
        return Err(StoreFault::Corrupt("HEAD is not canonical"));
    }
    if head.revision == 0 || head.revision > MAX_PUBLIC_REVISION {
        return Err(StoreFault::Corrupt("HEAD revision is invalid"));
    }
    Ok(head)
}

fn validate_intent(intent: Option<&str>) -> Result<(), StoreFault> {
    if intent.is_some_and(|value| {
        value.len() > INTENT_MAX_BYTES || value.chars().any(|character| character == '\0')
    }) {
        return Err(StoreFault::InvalidIntent);
    }
    Ok(())
}

fn canonical_json(value: &impl Serialize) -> Result<Vec<u8>, StoreFault> {
    serde_jcs::to_vec(value).map_err(|_| StoreFault::Canonicalization)
}

fn validate_store_format_bytes(path: &Path) -> Result<(), StoreFault> {
    require_real_file(path, "project store FORMAT is not a regular file")?;
    if std::fs::read(path)? == STORE_FORMAT {
        Ok(())
    } else {
        Err(StoreFault::Corrupt("unsupported project store FORMAT"))
    }
}

fn project_store_has_entries(root: &Path) -> Result<bool, StoreFault> {
    let projects = root.join("projects");
    match std::fs::symlink_metadata(&projects) {
        Ok(metadata) if !metadata.file_type().is_dir() => {
            return Err(StoreFault::Corrupt(
                "project store directory member is not a real directory",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(StoreFault::Io(error)),
    }
    match std::fs::read_dir(projects) {
        Ok(mut entries) => Ok(entries.next().transpose()?.is_some()),
        Err(error) => Err(error.into()),
    }
}

fn write_new_synced(
    path: &Path,
    bytes: &[u8],
    write_crash_point: &str,
    fsync_crash_point: &str,
) -> Result<(), StoreFault> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    super::crash::maybe_crash(write_crash_point);
    file.sync_all()?;
    super::crash::maybe_crash(fsync_crash_point);
    Ok(())
}

fn write_atomic_replace_synced(
    parent: &Path,
    temporary_prefix: &str,
    destination: &Path,
    bytes: &[u8],
    write_crash_point: &str,
    fsync_crash_point: &str,
) -> Result<(), StoreFault> {
    // NamedTempFile uses create_new semantics. Keeping the staging file in the
    // destination directory makes persist an atomic rename on one filesystem,
    // while a pre-planted symlink at a predictable name is never opened.
    let mut temporary = tempfile::Builder::new()
        .prefix(temporary_prefix)
        .tempfile_in(parent)?;
    temporary.as_file_mut().write_all(bytes)?;
    super::crash::maybe_crash(write_crash_point);
    temporary.as_file().sync_all()?;
    super::crash::maybe_crash(fsync_crash_point);
    temporary
        .persist(destination)
        .map_err(|error| StoreFault::Io(error.error))?;
    Ok(())
}

fn sync_dir(path: &Path) -> Result<(), StoreFault> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn path_entry_exists(path: &Path) -> Result<bool, StoreFault> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn require_real_directory(path: &Path) -> Result<(), StoreFault> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir() {
        return Err(StoreFault::Corrupt(
            "project store directory member is not a real directory",
        ));
    }
    Ok(())
}

fn require_real_file(path: &Path, message: &'static str) -> Result<(), StoreFault> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(StoreFault::Corrupt(message));
    }
    Ok(())
}

fn create_or_validate_directory(path: &Path) -> Result<(), StoreFault> {
    match std::fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            require_real_directory(path)
        }
        Err(error) => Err(error.into()),
    }
}

fn sweep_temporary_revisions(project_dir: &Path) -> Result<(), StoreFault> {
    let revisions = project_dir.join(REVISIONS_DIR);
    for entry in std::fs::read_dir(revisions)? {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && entry.file_name().to_str().is_some_and(|name| {
                name.strip_prefix(".tmp-")
                    .is_some_and(|revision| revision.parse::<u64>().is_ok())
            })
        {
            std::fs::remove_dir_all(entry.path())?;
        }
    }
    sweep_replace_temporary_files(project_dir, ".HEAD.tmp-", Some("HEAD.tmp"))?;
    Ok(())
}

fn sweep_replace_temporary_files(
    parent: &Path,
    temporary_prefix: &str,
    legacy_name: Option<&str>,
) -> Result<(), StoreFault> {
    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(temporary_prefix) && legacy_name != Some(name) {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_file() || file_type.is_symlink() {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn sweep_all_unpublished_revisions(project_dir: &Path) -> Result<(), StoreFault> {
    let revisions = project_dir.join(REVISIONS_DIR);
    for entry in std::fs::read_dir(&revisions)? {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.parse::<u64>().is_ok_and(|revision| revision > 0))
        {
            std::fs::remove_dir_all(entry.path())?;
        }
    }
    sync_dir(&revisions)?;
    Ok(())
}

fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

fn compile_timeline_document(
    timeline: &Timeline,
) -> Result<CanonicalTimeline, CompileTimelineError> {
    compile_timeline(timeline.clone())
}

fn compile_timeline_errors(error: CompileTimelineError) -> Vec<EditErrorWire> {
    let one = |code: &str, path: &str, details| {
        vec![EditErrorWire {
            code: code.to_owned(),
            path: timeline_request_path(path),
            details,
        }]
    };
    match error {
        CompileTimelineError::InvalidResourceAlias { alias, path } => one(
            "invalid_resource_alias",
            &path,
            BTreeMap::from([("alias".to_owned(), json!(alias))]),
        ),
        CompileTimelineError::EmptyResourceLocator { alias } => one(
            "empty_resource_locator",
            &format!("/resources/{alias}"),
            BTreeMap::new(),
        ),
        CompileTimelineError::UnknownResourceAlias { alias, path } => one(
            "unknown_resource_alias",
            &path,
            BTreeMap::from([("alias".to_owned(), json!(alias))]),
        ),
        CompileTimelineError::TrackOverlap {
            path,
            start,
            previous_end,
        } => one(
            "track_overlap",
            &path,
            BTreeMap::from([
                ("start".to_owned(), json!(start.to_string())),
                ("previousEnd".to_owned(), json!(previous_end.to_string())),
            ]),
        ),
        CompileTimelineError::TimeOverflow { path } => one("time_overflow", &path, BTreeMap::new()),
        CompileTimelineError::EmptyTimeline => one("empty_timeline", "/tracks", BTreeMap::new()),
        CompileTimelineError::InvalidCaptionContent { path } => {
            one("invalid_caption_content", &path, BTreeMap::new())
        }
        CompileTimelineError::PresetWithCustomPresentation { path } => {
            one("preset_with_custom_presentation", &path, BTreeMap::new())
        }
        CompileTimelineError::InvalidFrameRate(source) => one(
            "invalid_frame_rate",
            "/canvas/fps",
            BTreeMap::from([("reason".to_owned(), json!(source.to_string()))]),
        ),
        CompileTimelineError::CaptionPreset { path, source } => one(
            "invalid_caption_preset",
            &path,
            BTreeMap::from([("reason".to_owned(), json!(source.to_string()))]),
        ),
        CompileTimelineError::InvalidCanonical(report) => report
            .diagnostics
            .into_iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
            .map(|diagnostic| EditErrorWire {
                code: diagnostic.code,
                // Canonical paths name the private normalized document and are not pointers into
                // the submitted request. Keep the public pointer truthful until the compiler can
                // project each invariant back to its author path.
                path: "/timeline".to_owned(),
                details: diagnostic.details,
            })
            .collect(),
    }
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

fn validate_revision_directory(revision_dir: &Path) -> Result<(), StoreFault> {
    let mut members = BTreeMap::new();
    for entry in std::fs::read_dir(revision_dir)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            return Err(StoreFault::Corrupt("revision member name is not UTF-8"));
        };
        if name != TIMELINE_FILE && name != REVISION_FILE {
            return Err(StoreFault::Corrupt("revision directory has unknown member"));
        }
        if !entry.file_type()?.is_file() || members.insert(name, ()).is_some() {
            return Err(StoreFault::Corrupt("revision member is not a regular file"));
        }
    }
    if !members.contains_key(TIMELINE_FILE) || !members.contains_key(REVISION_FILE) {
        return Err(StoreFault::Corrupt("revision directory is incomplete"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::snapshot_digest_from_parts;

    #[test]
    fn snapshot_digest_preimage_framing_is_stable() {
        let digest = snapshot_digest_from_parts(br#"{"revision":1}"#, br#"{"tracks":{}}"#).unwrap();
        assert_eq!(
            digest.to_string(),
            "sha256:f7c726218b06aeaf7383f8398cf51cf1abebe682d546c8921b7b27dc3fb9cca9"
        );
    }
}
