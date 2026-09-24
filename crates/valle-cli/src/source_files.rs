//! Studio source text access, scoped to the Motion files named by the active session.
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write as _,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use valle_motion::ContentDigest;

use crate::webhost::StudioHost;

const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WriteSourceRequest {
    path: String,
    #[serde(default)]
    base_digest: Option<ContentDigest>,
    text: String,
}

fn session_paths(state: &StudioHost) -> Result<BTreeSet<PathBuf>> {
    let mut paths = if let Some(project) = &state.project {
        let snapshot = project.store.get_timeline(&project.project_id, None)?;
        crate::cmd::timeline::motion_source_paths(snapshot.timeline(), Path::new("."))?
    } else if let Some(file) = &state.timeline_file {
        file.source_paths()?
    } else if let Some(motion) = &state.motion_source {
        let mut paths = crate::cmd::motion::motion_module_paths(&motion.input)
            .unwrap_or_else(|_| vec![motion.input.clone()])
            .into_iter()
            .collect::<BTreeSet<_>>();
        if let Some(data) = &motion.data {
            paths.insert(data.clone());
        }
        paths
    } else {
        bail!("Studio source session is unavailable");
    };
    let mut normalized = BTreeSet::new();
    for path in std::mem::take(&mut paths) {
        let path = path
            .canonicalize()
            .or_else(|_| std::path::absolute(&path))?;
        normalized.insert(path);
    }
    let mut confirmed = state
        .source_paths
        .lock()
        .map_err(|_| anyhow!("Studio source scope poisoned"))?;
    confirmed.extend(normalized);
    Ok(confirmed.clone())
}

pub(crate) fn list(state: &StudioHost) -> Result<Value> {
    list_paths(session_paths(state)?)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftManifestRequest {
    drafts: BTreeMap<String, String>,
}

pub(crate) fn list_with_drafts(state: &StudioHost, body: &str) -> Result<Value> {
    let request: DraftManifestRequest = serde_json::from_str(body)?;
    if request.drafts.len() > 128 {
        bail!("too many source drafts");
    }
    let mut paths = session_paths(state)?;
    for (name, text) in request.drafts {
        if text.len() as u64 > MAX_SOURCE_BYTES {
            bail!("source exceeds 4 MiB");
        }
        let entry = paths
            .iter()
            .find(|path| path.to_string_lossy() == name)
            .cloned()
            .ok_or_else(|| anyhow!("source path is outside this Studio session"))?;
        let root = entry
            .parent()
            .ok_or_else(|| anyhow!("source has no parent directory"))?;
        let filename = entry
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("source has no UTF-8 file name"))?;
        let overrides = BTreeMap::from([(filename.to_owned(), text)]);
        if let Ok(graph) = valle_compiler::motion::MotionModuleGraph::from_entry_path_with_overrides(
            &entry, &overrides,
        ) {
            for path in graph.modules.keys() {
                paths.insert(root.join(path).canonicalize()?);
            }
        }
    }
    state
        .source_paths
        .lock()
        .map_err(|_| anyhow!("Studio source scope poisoned"))?
        .extend(paths.iter().cloned());
    list_paths(paths)
}

fn list_paths(paths: BTreeSet<PathBuf>) -> Result<Value> {
    let files = paths
        .into_iter()
        .map(|path| {
            let result = read_text(&path);
            match result {
                Ok((text, digest)) => json!({
                    "path":path,
                    "status":"ok",
                    "text":text,
                    "baseDigest":digest,
                }),
                Err(error) => json!({
                    "path":path,
                    "status":"error",
                    "message":error.to_string(),
                    "baseDigest":null,
                }),
            }
        })
        .collect::<Vec<_>>();
    Ok(json!({"files":files}))
}

fn read_text(path: &Path) -> Result<(String, ContentDigest)> {
    let len = std::fs::metadata(path)
        .with_context(|| format!("reading {}", path.display()))?
        .len();
    if len > MAX_SOURCE_BYTES {
        bail!("source exceeds 4 MiB: {}", path.display());
    }
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let digest = ContentDigest::of_bytes(&bytes);
    let text = String::from_utf8(bytes).with_context(|| format!("decoding {}", path.display()))?;
    Ok((text, digest))
}

fn allowed_path(state: &StudioHost, input: &str) -> Result<PathBuf> {
    let allowed = session_paths(state)?;
    allowed
        .into_iter()
        .find(|path| path.to_string_lossy() == input)
        .ok_or_else(|| anyhow!("source path is outside this Studio session"))
}

pub(crate) fn write(state: &StudioHost, body: &str) -> Result<Value> {
    let request: WriteSourceRequest = serde_json::from_str(body)?;
    if request.text.len() as u64 > MAX_SOURCE_BYTES {
        bail!("source exceeds 4 MiB");
    }
    let path = allowed_path(state, &request.path)?;
    // Serialize writes within this local Host, including two browser requests for one file.
    static WRITES: OnceLock<Mutex<()>> = OnceLock::new();
    let _write_guard = WRITES
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| anyhow!("Studio source write lock poisoned"))?;
    let submitted = request.text.as_bytes();
    let submitted_digest = ContentDigest::of_bytes(submitted);
    let current = read_optional(&path)?;
    let current_digest = current.as_deref().map(ContentDigest::of_bytes);
    if current_digest == Some(submitted_digest) {
        return Ok(json!({"status":"unchanged","digest":submitted_digest}));
    }
    if current_digest != request.base_digest {
        return Ok(json!({"status":"conflict","currentDigest":current_digest}));
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("source has no parent directory"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    if let Ok(metadata) = std::fs::metadata(&path) {
        temp.as_file().set_permissions(metadata.permissions())?;
    }
    temp.write_all(submitted)?;
    temp.as_file().sync_all()?;
    // An external editor may still race with the rename, but do not overwrite a
    // change observed while the temporary file was being prepared.
    let before_replace = read_optional(&path)?;
    if before_replace.as_deref().map(ContentDigest::of_bytes) != request.base_digest {
        return Ok(json!({
            "status":"conflict",
            "currentDigest":before_replace.as_deref().map(ContentDigest::of_bytes),
        }));
    }
    if current.is_some() {
        temp.persist(&path).map_err(|error| error.error)?;
    } else {
        temp.persist_noclobber(&path).map_err(|error| error.error)?;
    }
    let after = std::fs::read(&path)?;
    let after_digest = ContentDigest::of_bytes(&after);
    if after_digest != submitted_digest {
        return Ok(json!({"status":"changedAfterWrite","digest":after_digest}));
    }
    Ok(json!({"status":"saved","digest":submitted_digest}))
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_SOURCE_BYTES => {
            bail!("source exceeds 4 MiB: {}", path.display());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    }
    Ok(Some(std::fs::read(path)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    fn motion_host(input: PathBuf) -> StudioHost {
        StudioHost {
            runtime_files: Default::default(),
            assets_dir: None,
            preview_files: Arc::new(Default::default()),
            motion_source: Some(crate::webhost::MotionSourceCtx {
                token: "test-token".into(),
                input,
                data: None,
            }),
            source_paths: Mutex::new(Default::default()),
            config_json: Arc::new(RwLock::new("{}".into())),
            sse: Default::default(),
            capture_dir: None,
            library: None,
            project: None,
            timeline_file: None,
            last_report: RwLock::new(None),
        }
    }

    #[test]
    fn source_digest_uses_exact_file_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("card.motion.tsx");
        std::fs::write(&path, b"line 1\r\nline 2\r\n").unwrap();
        let (text, digest) = read_text(&path).unwrap();
        assert_eq!(text, "line 1\r\nline 2\r\n");
        assert_eq!(digest, ContentDigest::of_bytes(b"line 1\r\nline 2\r\n"));
    }

    #[test]
    fn source_write_is_scoped_atomic_and_checks_the_opened_digest() {
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("card.motion.tsx");
        let imported = dir.path().join("theme.motion.ts");
        let unrelated = dir.path().join("unrelated.motion.tsx");
        std::fs::write(&entry, "import { tone } from './theme.motion'; export default function Card() { return <Scene />; }").unwrap();
        std::fs::write(&imported, "export const tone = 'blue';\r\n").unwrap();
        std::fs::write(&unrelated, "do not touch").unwrap();
        let host = motion_host(entry);
        let loaded = list(&host).unwrap();
        assert_eq!(loaded["files"].as_array().unwrap().len(), 2);
        let item = loaded["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["path"] == imported.canonicalize().unwrap().to_str().unwrap())
            .unwrap();
        let original_digest = item["baseDigest"].as_str().unwrap();
        let request = json!({"path":imported.canonicalize().unwrap(),"baseDigest":original_digest,"text":"export const tone = 'green';\n"});
        let saved = write(&host, &request.to_string()).unwrap();
        assert_eq!(saved["status"], "saved");
        assert_eq!(
            std::fs::read_to_string(&imported).unwrap(),
            "export const tone = 'green';\n"
        );
        assert_eq!(
            write(&host, &request.to_string()).unwrap()["status"],
            "unchanged"
        );
        let stale = json!({"path":imported.canonicalize().unwrap(),"baseDigest":original_digest,"text":"export const tone = 'red';\n"});
        assert_eq!(
            write(&host, &stale.to_string()).unwrap()["status"],
            "conflict"
        );
        let outside = json!({"path":unrelated,"baseDigest":null,"text":"overwrite"});
        assert!(
            write(&host, &outside.to_string())
                .unwrap_err()
                .to_string()
                .contains("outside this Studio session")
        );
        assert_eq!(std::fs::read_to_string(&unrelated).unwrap(), "do not touch");
    }

    #[test]
    fn draft_import_adds_existing_module_to_source_session() {
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("card.motion.tsx");
        let imported = dir.path().join("theme.motion.ts");
        std::fs::write(
            &entry,
            "export default function Card() { return <Scene />; }",
        )
        .unwrap();
        std::fs::write(&imported, "export const tone = 'blue';").unwrap();
        let host = motion_host(entry.clone());
        assert_eq!(list(&host).unwrap()["files"].as_array().unwrap().len(), 1);
        let draft = "import { tone } from './theme.motion'; export default function Card() { return <Scene />; }";
        let request = json!({"drafts": BTreeMap::from([(entry.canonicalize().unwrap().to_string_lossy().to_string(), draft)])});
        assert_eq!(
            list_with_drafts(&host, &request.to_string()).unwrap()["files"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(list(&host).unwrap()["files"].as_array().unwrap().len(), 2);
        let outside = json!({"drafts": BTreeMap::from([(dir.path().join("other.ts").to_string_lossy().to_string(), draft)])});
        assert!(list_with_drafts(&host, &outside.to_string()).is_err());
    }
}
