//! File-backed Timeline Studio. Revisions are session-local conflict counters, not project history.
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
};

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use valle_timeline::{Timeline, decode_timeline, timeline_bytes};

pub(crate) struct TimelineFile {
    pub input: PathBuf,
    pub token: String,
    observed: Mutex<ObservedFile>,
}

struct ObservedFile {
    bytes: Vec<u8>,
    revision: u64,
}

impl TimelineFile {
    pub fn open(input: &Path) -> Result<Self> {
        let input = input
            .canonicalize()
            .with_context(|| format!("opening {}", input.display()))?;
        let bytes = std::fs::read(&input)?;
        let timeline = decode_timeline(std::str::from_utf8(&bytes)?)?;
        valle_compiler::compile_timeline(timeline)?;
        let mut token = [0_u8; 32];
        getrandom::fill(&mut token).context("generating Timeline Studio token")?;
        Ok(Self {
            input,
            token: hex::encode(token),
            observed: Mutex::new(ObservedFile { bytes, revision: 1 }),
        })
    }

    fn refresh(&self, observed: &mut ObservedFile) -> Result<()> {
        let bytes = std::fs::read(&self.input)
            .with_context(|| format!("reading {}", self.input.display()))?;
        if bytes != observed.bytes {
            observed.bytes = bytes;
            observed.revision += 1;
        }
        Ok(())
    }

    pub fn revision(&self) -> Result<u64> {
        let mut observed = self
            .observed
            .lock()
            .map_err(|_| anyhow!("file state poisoned"))?;
        self.refresh(&mut observed)?;
        Ok(observed.revision)
    }

    pub fn read(&self) -> Result<(u64, Timeline)> {
        let mut observed = self
            .observed
            .lock()
            .map_err(|_| anyhow!("file state poisoned"))?;
        self.refresh(&mut observed)?;
        Ok((
            observed.revision,
            decode_timeline(std::str::from_utf8(&observed.bytes)?)?,
        ))
    }

    pub fn snapshot(&self) -> Result<Value> {
        let (revision, timeline) = self.read()?;
        let timeline_json = String::from_utf8(timeline_bytes(&timeline)?)?;
        let canonical = valle_compiler::compile_timeline(timeline)?;
        let render_json =
            String::from_utf8(valle_timeline::internal::canonical_bytes(&canonical)?)?;
        Ok(json!({
            "timelineRevision": {"revision":revision,"parentRevision":null,"createdAt":"","actor":"file","cause":{"type":"genesis"},"intent":null},
            "timeline":serde_json::from_str::<Value>(&timeline_json)?, "timelineJson":timeline_json,
            "render":{"timeline":serde_json::from_str::<Value>(&render_json)?,"timelineJson":render_json},
            "preview":{"status":"unavailable","code":"preview_not_prepared"},
        }))
    }

    pub fn save(&self, body: &str) -> Result<Value> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Edit {
            base_revision: u64,
            timeline: Value,
            #[serde(default)]
            intent: Option<String>,
        }
        let edit: Edit = serde_json::from_str(body)?;
        let _ = edit.intent;
        let timeline = decode_timeline(&edit.timeline.to_string())?;
        valle_compiler::compile_timeline(timeline.clone())?;
        let normalized: Value = serde_json::from_slice(&timeline_bytes(&timeline)?)?;
        let mut observed = self
            .observed
            .lock()
            .map_err(|_| anyhow!("file state poisoned"))?;
        self.refresh(&mut observed)?;
        if edit.base_revision != observed.revision {
            return Ok(json!({"outcome":"staleBase","revision":observed.revision}));
        }
        if serde_json::from_slice::<Value>(&observed.bytes)
            .ok()
            .as_ref()
            == Some(&normalized)
        {
            return Ok(json!({"outcome":"unchanged","revision":observed.revision}));
        }
        let bytes = format!("{}\n", serde_json::to_string_pretty(&normalized)?).into_bytes();
        let mut temp = tempfile::NamedTempFile::new_in(self.input.parent().unwrap())?;
        temp.as_file()
            .set_permissions(std::fs::metadata(&self.input)?.permissions())?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        // Check again immediately before replacing; no partial JSON is ever exposed to readers.
        self.refresh(&mut observed)?;
        if edit.base_revision != observed.revision {
            return Ok(json!({"outcome":"staleBase","revision":observed.revision}));
        }
        temp.persist(&self.input).map_err(|error| error.error)?;
        observed.bytes = bytes;
        observed.revision += 1;
        Ok(json!({"outcome":"committed","revision":observed.revision}))
    }
}

pub(crate) fn serve(
    input: &Path,
    web_assets_dir: Option<&Path>,
    port: u16,
) -> Result<std::process::ExitCode> {
    let file = Arc::new(TimelineFile::open(input)?);
    let runtime = crate::webruntime::resolve(web_assets_dir)?;
    let config = json!({"input":file.input,"timelineFile":{"token":file.token},"runtimeAssets":crate::webruntime::runtime_assets_json()});
    let state = Arc::new(crate::webhost::StudioHost {
        runtime_files: runtime.serving_map(),
        assets_dir: None,
        preview_files: Arc::new(RwLock::new(Default::default())),
        motion_preview: None,
        config_json: Arc::new(RwLock::new(config.to_string())),
        sse: Default::default(),
        capture_dir: None,
        library: None,
        project: None,
        timeline_file: Some(file),
        last_report: RwLock::new(None),
    });
    let (server, addr) = crate::webhost::bind(port)?;
    let url = format!("http://{addr}/studio");
    if crate::output::events() {
        crate::events::emit(crate::events::EventKind::Ready {
            url: url.clone(),
            port: addr.port(),
            runtime_version: runtime.manifest.runtime_version.clone(),
            runtime_source: runtime.source.as_str().to_owned(),
            project_id: None,
            revision: None,
        });
    } else if crate::output::machine() {
        crate::output::emit(
            json!({"status":"ready","url":url,"port":addr.port(),"input":input,"runtime_version":runtime.manifest.runtime_version,"runtime_source":runtime.source.as_str()}),
        );
        std::io::stdout().flush()?;
    } else {
        eprintln!(
            "timeline studio: {url} · {} (Save writes this file; Ctrl-C to stop)",
            input.display()
        );
    }
    crate::webhost::serve_forever(server, state)?;
    Ok(std::process::ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_saves_are_atomic_scoped_and_reject_external_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cut.json");
        let original = include_bytes!("../../../examples/timeline.json");
        std::fs::write(&path, original).unwrap();
        let file = TimelineFile::open(&path).unwrap();
        let snapshot = file.snapshot().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let mut timeline = snapshot["timeline"].clone();
        timeline["canvas"]["background"] = json!("#f97316ff");
        let request = json!({"baseRevision":1,"timeline":timeline}).to_string();
        assert_eq!(file.save(&request).unwrap()["outcome"], "committed");
        assert_eq!(file.snapshot().unwrap()["timeline"], timeline);
        assert_eq!(file.save(&request).unwrap()["outcome"], "staleBase");
        let revision = file.revision().unwrap();
        std::fs::write(&path, original).unwrap();
        assert_eq!(
            file.save(&json!({"baseRevision":revision,"timeline":timeline}).to_string())
                .unwrap()["outcome"],
            "staleBase"
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(
            file.save(&json!({"baseRevision":file.revision().unwrap(),"timeline":{}}).to_string())
                .is_err()
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
