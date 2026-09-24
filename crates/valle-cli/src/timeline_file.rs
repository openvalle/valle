//! File-backed Timeline Studio. Revisions are session-local conflict counters, not project history.
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
};

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use valle_motion::ContentDigest;
use valle_timeline::{Timeline, decode_timeline, timeline_bytes};

fn capture_file_motion_sources(
    timeline: &Timeline,
    path: &Path,
) -> Result<super::cmd::timeline::CapturedMotionSources> {
    let mut document: Value = serde_json::from_slice(&timeline_bytes(timeline)?)?;
    super::cmd::timeline::prepare_motion_instances(&mut document)?;
    super::cmd::timeline::capture_motion_sources(
        &document,
        path.parent()
            .ok_or_else(|| anyhow!("Timeline file has no parent directory"))?,
    )
}

fn bind_file_timeline_captured(
    timeline: &Timeline,
    captured: &super::cmd::timeline::CapturedMotionSources,
) -> Result<(
    valle_timeline::internal::CanonicalTimeline,
    std::collections::BTreeMap<String, f64>,
)> {
    let (_, durations) = captured.prepare()?;
    let canonical =
        valle_compiler::compile_timeline_with_motion_sources(timeline.clone(), &durations)?;
    let canonical_value: Value =
        serde_json::from_slice(&valle_timeline::internal::canonical_bytes(&canonical)?)?;
    let author_value: Value = serde_json::from_slice(&timeline_bytes(timeline)?)?;
    let source_durations =
        super::cmd::timeline::motion_source_durations(&canonical_value, &author_value)?;
    Ok((canonical, source_durations))
}

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
        decode_timeline(std::str::from_utf8(&bytes)?)?;
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

    pub(crate) fn source_paths(&self) -> Result<std::collections::BTreeSet<PathBuf>> {
        let (_, timeline) = self.read()?;
        super::cmd::timeline::motion_source_paths(
            &timeline,
            self.input
                .parent()
                .ok_or_else(|| anyhow!("Timeline file has no parent directory"))?,
        )
    }

    pub fn snapshot(&self) -> Result<Value> {
        let (revision, timeline) = self.read()?;
        let timeline_json = String::from_utf8(timeline_bytes(&timeline)?)?;
        let prepared = (|| -> Result<_> {
            let captured = capture_file_motion_sources(&timeline, &self.input)?;
            let input_dependencies = captured.dependency_digests()?;
            let (canonical, durations) = bind_file_timeline_captured(&timeline, &captured)?;
            Ok((canonical, durations, input_dependencies))
        })();
        let (canonical, motion_source_durations, input_dependencies, input_dependency_error) =
            match prepared {
                Ok((canonical, durations, dependencies)) => {
                    (canonical, durations, dependencies, None)
                }
                Err(error) => {
                    // The authored file stays editable while a referenced JSX module is broken.
                    // This projection only supports the timeline UI until browser preparation succeeds.
                    let mut durations = BTreeMap::new();
                    let document: Value = serde_json::from_str(&timeline_json)?;
                    for track in document["tracks"]["visual"]
                        .as_array()
                        .into_iter()
                        .flatten()
                    {
                        for clip in track["clips"].as_array().into_iter().flatten() {
                            if clip["kind"] == "motion" {
                                if let Some(component) = clip["component"].as_str() {
                                    durations.insert(
                                        component.to_owned(),
                                        valle_timeline::RationalTime::new(86_400, 1)?,
                                    );
                                }
                            }
                        }
                    }
                    let projection = valle_compiler::compile_timeline_with_motion_sources(
                        timeline.clone(),
                        &durations,
                    )?;
                    let duration_numbers = durations
                        .into_iter()
                        .map(|(key, duration)| (key, duration.as_f64()))
                        .collect();
                    (
                        projection,
                        duration_numbers,
                        BTreeMap::new(),
                        Some(format!("{error:#}")),
                    )
                }
            };
        let render_json =
            String::from_utf8(valle_timeline::internal::canonical_bytes(&canonical)?)?;
        Ok(json!({
            "timelineRevision": {"revision":revision,"parentRevision":null,"createdAt":"","actor":"file","cause":{"type":"genesis"},"intent":null},
            "timeline":serde_json::from_str::<Value>(&timeline_json)?, "timelineJson":timeline_json,
            "motionSourceDurations":motion_source_durations,
            "inputDependencies":input_dependencies,
            "inputDependencyError":input_dependency_error,
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
            #[serde(default)]
            expected_dependencies: Option<BTreeMap<String, ContentDigest>>,
        }
        let edit: Edit = serde_json::from_str(body)?;
        let _ = edit.intent;
        let timeline = decode_timeline(&edit.timeline.to_string())?;
        let normalized: Value = serde_json::from_slice(&timeline_bytes(&timeline)?)?;
        let mut observed = self
            .observed
            .lock()
            .map_err(|_| anyhow!("file state poisoned"))?;
        self.refresh(&mut observed)?;
        if edit.base_revision != observed.revision {
            return Ok(json!({"outcome":"staleBase","revision":observed.revision}));
        }
        let captured = match capture_file_motion_sources(&timeline, &self.input) {
            Ok(captured) => captured,
            Err(error) => return Ok(motion_save_rejected(&error)),
        };
        if let Err(error) =
            captured.verify_expected_dependencies(edit.expected_dependencies.as_ref())
        {
            return Ok(motion_save_rejected(&error));
        }
        if let Err(error) = bind_file_timeline_captured(&timeline, &captured) {
            return Ok(motion_save_rejected(&error));
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
        if let Err(error) = captured.verify_dependencies() {
            return Ok(motion_save_rejected(&error));
        }
        temp.persist(&self.input).map_err(|error| error.error)?;
        observed.bytes = bytes;
        observed.revision += 1;
        Ok(json!({"outcome":"committed","revision":observed.revision}))
    }
}

fn motion_save_rejected(error: &anyhow::Error) -> Value {
    json!({
        "outcome":"rejected",
        "errors":[{"code":"motion_preparation","path":"/timeline","details":{"message":error.to_string()}}]
    })
}

/// Convert a Motion Studio draft into a normal Timeline file after source files were saved.
/// The target and every dependency are checked again before the atomic replacement.
pub(crate) fn save_motion_timeline_as(
    state: &crate::webhost::StudioHost,
    body: &str,
) -> Result<Value> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Request {
        target: PathBuf,
        base_digest: Option<ContentDigest>,
        timeline: Value,
        expected_dependencies: BTreeMap<String, ContentDigest>,
    }
    let request: Request = serde_json::from_str(body)?;
    let motion = state
        .motion_source
        .as_ref()
        .ok_or_else(|| anyhow!("Motion source session is unavailable"))?;
    let config: Value = serde_json::from_str(
        &state
            .config_json
            .read()
            .map_err(|_| anyhow!("Studio config poisoned"))?,
    )?;
    if config["authorInputs"]["extraFonts"]
        .as_array()
        .is_some_and(|fonts| !fonts.is_empty())
    {
        anyhow::bail!(
            "extra --font stacks cannot be saved as Timeline; bind a declared font asset and preview again"
        );
    }
    let source_dir = motion
        .input
        .parent()
        .ok_or_else(|| anyhow!("Motion input has no parent"))?;
    let target = if request.target.is_absolute() {
        request.target.clone()
    } else {
        source_dir.join(&request.target)
    };
    if target
        .extension()
        .is_none_or(|extension| extension != "json")
    {
        anyhow::bail!("Timeline save target must end in .json");
    }
    let parent = target
        .parent()
        .ok_or_else(|| anyhow!("Timeline target has no parent"))?
        .canonicalize()
        .with_context(|| format!("opening Timeline target directory {}", target.display()))?;
    let file_name = target
        .file_name()
        .ok_or_else(|| anyhow!("Timeline target needs a file name"))?;
    let target = parent.join(file_name);
    let current = read_existing_timeline(&target)?;
    let current_digest = current.as_deref().map(ContentDigest::of_bytes);
    if current_digest != request.base_digest {
        return Ok(json!({"status":"conflict","currentDigest":current_digest}));
    }
    let mut authored = request.timeline;
    let resources = authored["resources"]
        .as_object_mut()
        .ok_or_else(|| anyhow!("Timeline resources are missing"))?;
    for locator in resources.values_mut() {
        let raw = locator
            .as_str()
            .ok_or_else(|| anyhow!("Timeline resource locator must be a string"))?;
        if raw.contains("://") {
            continue;
        }
        let path = Path::new(raw);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else if request.base_digest.is_some() {
            parent.join(path)
        } else {
            std::path::absolute(path)?
        };
        let resolved = resolved
            .canonicalize()
            .with_context(|| format!("resolving Timeline resource {raw}"))?;
        *locator = Value::String(
            relative_path(&parent, &resolved)
                .to_string_lossy()
                .into_owned(),
        );
    }
    let timeline = decode_timeline(&authored.to_string())?;
    let normalized: Value = serde_json::from_slice(&timeline_bytes(&timeline)?)?;
    let captured = capture_file_motion_sources(&timeline, &target)?;
    captured.verify_expected_dependencies(Some(&request.expected_dependencies))?;
    bind_file_timeline_captured(&timeline, &captured)?;
    let bytes = format!("{}\n", serde_json::to_string_pretty(&normalized)?).into_bytes();
    let mut temp = tempfile::NamedTempFile::new_in(&parent)?;
    if let Ok(metadata) = std::fs::metadata(&target) {
        temp.as_file().set_permissions(metadata.permissions())?;
    }
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    let before_replace = read_existing_timeline(&target)?;
    let before_digest = before_replace.as_deref().map(ContentDigest::of_bytes);
    if before_digest != request.base_digest {
        return Ok(json!({"status":"conflict","currentDigest":before_digest}));
    }
    captured.verify_dependencies()?;
    if request.base_digest.is_some() {
        temp.persist(&target).map_err(|error| error.error)?;
    } else {
        temp.persist_noclobber(&target)
            .map_err(|error| error.error)?;
    }
    let written = std::fs::read(&target)?;
    let digest = ContentDigest::of_bytes(&written);
    if written != bytes {
        anyhow::bail!("Timeline target changed immediately after save");
    }
    Ok(json!({"status":"saved","target":target,"digest":digest,"timeline":normalized}))
}

pub(crate) fn load_saved_motion_timeline(
    state: &crate::webhost::StudioHost,
    body: &str,
) -> Result<Value> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        target: PathBuf,
    }
    let request: Request = serde_json::from_str(body)?;
    let motion = state
        .motion_source
        .as_ref()
        .ok_or_else(|| anyhow!("Motion source session is unavailable"))?;
    let source_dir = motion
        .input
        .parent()
        .ok_or_else(|| anyhow!("Motion input has no parent"))?;
    let target = if request.target.is_absolute() {
        request.target
    } else {
        source_dir.join(request.target)
    };
    let bytes = std::fs::read(&target)
        .with_context(|| format!("reading Timeline target {}", target.display()))?;
    if bytes.len() > 4 * 1024 * 1024 {
        anyhow::bail!("Timeline target exceeds 4 MiB");
    }
    let timeline = decode_timeline(std::str::from_utf8(&bytes)?)?;
    Ok(json!({
        "target":target.canonicalize()?,
        "digest":ContentDigest::of_bytes(&bytes),
        "timeline":serde_json::from_slice::<Value>(&timeline_bytes(&timeline)?)?,
    }))
}

fn relative_path(base: &Path, target: &Path) -> PathBuf {
    let base_parts = base.components().collect::<Vec<_>>();
    let target_parts = target.components().collect::<Vec<_>>();
    let common = base_parts
        .iter()
        .zip(&target_parts)
        .take_while(|(left, right)| left == right)
        .count();
    let mut relative = PathBuf::new();
    for _ in common..base_parts.len() {
        relative.push("..");
    }
    for component in &target_parts[common..] {
        relative.push(component.as_os_str());
    }
    relative
}

fn read_existing_timeline(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("reading Timeline target {}", path.display()))
        }
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
    let mut runtime_files = runtime.serving_map();
    crate::cmd::motion::mount_motion_runtime_fonts(&mut runtime_files)?;
    let state = Arc::new(crate::webhost::StudioHost {
        runtime_files,
        assets_dir: None,
        preview_files: Arc::new(Default::default()),
        motion_source: None,
        config_json: Arc::new(RwLock::new(config.to_string())),
        sse: Default::default(),
        capture_dir: None,
        library: None,
        project: None,
        timeline_file: Some(file),
        source_paths: std::sync::Mutex::new(Default::default()),
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
    fn broken_motion_source_keeps_file_timeline_editable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("card.motion.tsx"),
            "export default function Card() { return <Scene>",
        )
        .unwrap();
        let path = dir.path().join("cut.json");
        std::fs::write(&path, json!({
            "canvas":{"width":64,"height":64,"fps":30},
            "resources":{"card":"card.motion.tsx"},
            "tracks":{"visual":[{"clips":[{"kind":"motion","component":"card","start":0,"duration":1}]}]}
        }).to_string()).unwrap();
        let file = TimelineFile::open(&path).unwrap();
        let snapshot = file.snapshot().unwrap();
        assert!(
            snapshot["inputDependencyError"]
                .as_str()
                .unwrap()
                .contains("Motion")
        );
        assert_eq!(
            snapshot["timeline"]["tracks"]["visual"][0]["clips"][0]["component"],
            "card"
        );
        assert_eq!(snapshot["preview"]["status"], "unavailable");
    }

    #[test]
    fn file_save_inputs_reject_source_change_after_capture() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("card.motion.tsx");
        std::fs::write(&source, "export const composition = { width: 64, height: 64, duration: 1 }; export default function Card() { return <Scene />; }").unwrap();
        let path = dir.path().join("cut.json");
        let timeline = decode_timeline(
            &json!({
                "canvas":{"width":64,"height":64,"fps":30},
                "resources":{"card":"card.motion.tsx"},
                "tracks":{"visual":[{"clips":[{"kind":"motion","component":"card","start":0,"duration":1}]}]}
            })
            .to_string(),
        )
        .unwrap();
        let captured = capture_file_motion_sources(&timeline, &path).unwrap();
        bind_file_timeline_captured(&timeline, &captured).unwrap();
        std::fs::write(&source, "export const composition = { width: 64, height: 64, duration: 2 }; export default function Card() { return <Scene />; }").unwrap();
        let error = bind_file_timeline_captured(&timeline, &captured).unwrap_err();
        assert!(error.to_string().contains("changed during save"));
    }
    #[test]
    fn file_studio_resolves_motion_duration_without_persisting_the_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("card.motion.tsx"), "export const composition = { width: 64, height: 64, duration: 3 }; export default function Card() { return <Scene />; }").unwrap();
        let path = dir.path().join("cut.json");
        let authored = json!({
            "canvas":{"width":64,"height":64,"fps":30},
            "resources":{"card":"card.motion.tsx"},
            "tracks":{"visual":[{"clips":[{"kind":"motion","component":"card","start":0,"duration":1}]}]}
        });
        std::fs::write(&path, authored.to_string()).unwrap();
        let file = TimelineFile::open(&path).unwrap();
        let snapshot = file.snapshot().unwrap();
        assert_eq!(snapshot["inputDependencies"].as_object().unwrap().len(), 1);
        assert_eq!(snapshot["motionSourceDurations"]["card"], 3.0);
        assert!(
            snapshot["timeline"]["tracks"]["visual"][0]["clips"][0]
                .get("sourceDuration")
                .is_none()
        );
        assert_eq!(
            snapshot["render"]["timeline"]["document"]["visual"]["tracks"][0]["items"][0]["source"]
                ["sourceDuration"],
            "3/1"
        );
    }

    #[test]
    fn file_save_rejects_a_dependency_changed_since_load() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("card.motion.tsx");
        std::fs::write(&source, "export const composition = { width: 64, height: 64, duration: 1 }; export default function Card() { return <Scene />; }").unwrap();
        let path = dir.path().join("cut.json");
        let authored = json!({
            "canvas":{"width":64,"height":64,"fps":30},
            "resources":{"card":"card.motion.tsx"},
            "tracks":{"visual":[{"clips":[{"kind":"motion","component":"card","start":0,"duration":1}]}]}
        });
        std::fs::write(&path, authored.to_string()).unwrap();
        let file = TimelineFile::open(&path).unwrap();
        let loaded = file.snapshot().unwrap();
        std::fs::write(&source, "export const composition = { width: 64, height: 64, duration: 2 }; export default function Card() { return <Scene />; }").unwrap();
        let mut edited = authored.clone();
        edited["canvas"]["background"] = json!("#123456ff");
        let report = file
            .save(
                &json!({
                    "baseRevision":1,
                    "timeline":edited,
                    "expectedDependencies":loaded["inputDependencies"],
                })
                .to_string(),
            )
            .unwrap();
        assert_eq!(report["outcome"], "rejected");
        assert!(
            report["errors"][0]["details"]["message"]
                .as_str()
                .unwrap()
                .contains("changed since it was loaded")
        );
        assert_eq!(file.revision().unwrap(), 1);
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),
            authored
        );
    }

    #[test]
    fn switching_a_motion_asset_saves_with_the_previewed_asset_digest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("card.motion.tsx"),
            "export const composition = { width: 64, height: 64, duration: 1 }; export const controls = { assets: { image: asset({ kind: 'image' }) } }; export default function Card() { return <Scene><Image src=\"asset://image\" style={{ width: 64, height: 64 }} /></Scene>; }").unwrap();
        let image =
            include_bytes!("../../valle-compiler/tests/fixtures/motion/modules/assets/dot.png");
        std::fs::write(dir.path().join("first.png"), image).unwrap();
        std::fs::write(dir.path().join("second.png"), image).unwrap();
        let path = dir.path().join("cut.json");
        let authored = json!({
            "canvas":{"width":64,"height":64,"fps":30},
            "resources":{"card":"card.motion.tsx","first":"first.png","second":"second.png"},
            "tracks":{"visual":[{"clips":[{"kind":"motion","component":"card","start":0,"duration":1,
                "resources":{"image":"first"}}]}]}
        });
        std::fs::write(&path, authored.to_string()).unwrap();
        let file = TimelineFile::open(&path).unwrap();
        let loaded = file.snapshot().unwrap();
        assert!(loaded["inputDependencyError"].is_null(), "{loaded}");
        let mut edited = authored.clone();
        edited["tracks"]["visual"][0]["clips"][0]["resources"]["image"] = json!("second");
        let stale = file
            .save(
                &json!({"baseRevision":1,"timeline":edited,
            "expectedDependencies":loaded["inputDependencies"]})
                .to_string(),
            )
            .unwrap();
        assert_eq!(stale["outcome"], "rejected");

        let mut cache = crate::preview_store::FrozenMediaCache::default();
        let (_, _, media_dependencies) = crate::cmd::timeline::prepare_timeline_media_facts(
            decode_timeline(&edited.to_string()).unwrap(),
            dir.path(),
            &mut cache,
        )
        .unwrap();
        let mut expected = loaded["inputDependencies"].as_object().unwrap().clone();
        expected.extend(
            serde_json::to_value(media_dependencies)
                .unwrap()
                .as_object()
                .unwrap()
                .clone(),
        );
        let saved = file
            .save(
                &json!({"baseRevision":1,"timeline":edited,
            "expectedDependencies":expected})
                .to_string(),
            )
            .unwrap();
        assert_eq!(saved["outcome"], "committed", "{saved}");
        assert_eq!(
            file.snapshot().unwrap()["timeline"]["tracks"]["visual"][0]["clips"][0]["resources"]["image"],
            "second"
        );
    }

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
