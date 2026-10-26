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
    std::collections::BTreeMap<String, valle_timeline::MotionSourceMetadata>,
)> {
    let (_, durations) = captured.prepare()?;
    let canonical =
        valle_compiler::compile_timeline_with_motion_sources(timeline.clone(), &durations)?;
    let canonical_value: Value =
        serde_json::from_slice(&valle_timeline::internal::canonical_bytes(&canonical)?)?;
    let author_value: Value = serde_json::from_slice(&timeline_bytes(timeline)?)?;
    let source_durations =
        super::cmd::timeline::motion_source_metadata(&canonical_value, &author_value)?;
    Ok((canonical, source_durations))
}

pub(crate) struct TimelineFile {
    pub input: PathBuf,
    pub token: String,
    pub project: Option<valle_project::folder::Project>,
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
            project: valle_project::folder::Project::discover(&input).ok(),
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

    /// Read the working document without capturing media or compiling Motion. Studio prepares
    /// its fixed package in the browser; the full projection is only needed on preparation errors.
    pub fn author_snapshot(&self) -> Result<Value> {
        let (revision, timeline) = self.read()?;
        let timeline_json = String::from_utf8(timeline_bytes(&timeline)?)?;
        let authored: Value = serde_json::from_str(&timeline_json)?;
        let empty = authored["tracks"]
            .as_object()
            .into_iter()
            .flat_map(|tracks| tracks.values())
            .flat_map(|tracks| tracks.as_array().into_iter().flatten())
            .all(|track| {
                track["clips"]
                    .as_array()
                    .is_none_or(|clips| clips.is_empty())
            });
        Ok(json!({
            "empty": empty,
            "timelineRevision": {"revision":revision,"parentRevision":null,"createdAt":"","actor":"file","cause":{"type":"genesis"},"intent":null},
            "timeline": authored, "timelineJson": timeline_json,
        }))
    }

    pub fn snapshot(&self) -> Result<Value> {
        let (revision, timeline) = self.read()?;
        let timeline_json = String::from_utf8(timeline_bytes(&timeline)?)?;
        let authored: Value = serde_json::from_str(&timeline_json)?;
        let empty = authored["tracks"]
            .as_object()
            .into_iter()
            .flat_map(|t| t.values())
            .flat_map(|tracks| tracks.as_array().into_iter().flatten())
            .all(|track| {
                track["clips"]
                    .as_array()
                    .is_none_or(|clips| clips.is_empty())
            });
        if empty {
            return Ok(
                json!({"empty":true,"timelineRevision":{"revision":revision,"parentRevision":null,"createdAt":"","actor":"studio","cause":{"type":"genesis"},"intent":null},"timelineJson":timeline_json,"timeline":authored,"inputDependencies":{},"assets":[]}),
            );
        }
        let prepared = (|| -> Result<_> {
            let captured = capture_file_motion_sources(&timeline, &self.input)?;
            let input_dependencies = captured.dependency_digests()?;
            let (canonical, durations) = bind_file_timeline_captured(&timeline, &captured)?;
            Ok((canonical, durations, input_dependencies))
        })();
        let (canonical, motion_source_metadata, input_dependencies, input_dependency_error) =
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
                            if clip["kind"] == "motion"
                                && let Some(component) = clip["component"].as_str()
                            {
                                durations.insert(
                                    component.to_owned(),
                                    valle_timeline::MotionSourceMetadata {
                                        duration: valle_timeline::RationalTime::new(86_400, 1)?,
                                        role: valle_timeline::MotionRole::Clip,
                                    },
                                );
                            }
                            for effect in clip["effects"].as_array().into_iter().flatten() {
                                if let Some(component) = effect["component"].as_str() {
                                    durations.insert(
                                        component.to_owned(),
                                        valle_timeline::MotionSourceMetadata {
                                            duration: valle_timeline::RationalTime::new(86_400, 1)?,
                                            role: valle_timeline::MotionRole::Effect,
                                        },
                                    );
                                }
                            }
                        }
                        for transition in track["transitions"].as_array().into_iter().flatten() {
                            if transition["kind"] == "motion"
                                && let Some(component) = transition["component"].as_str()
                            {
                                durations.insert(
                                    component.to_owned(),
                                    valle_timeline::MotionSourceMetadata {
                                        duration: valle_timeline::RationalTime::new(86_400, 1)?,
                                        role: valle_timeline::MotionRole::Transition,
                                    },
                                );
                            }
                        }
                    }
                    for clip in document["tracks"]["adjustment"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .flat_map(|track| track["clips"].as_array().into_iter().flatten())
                    {
                        if clip["kind"] == "motion"
                            && let Some(component) = clip["component"].as_str()
                        {
                            durations.insert(
                                component.to_owned(),
                                valle_timeline::MotionSourceMetadata {
                                    duration: valle_timeline::RationalTime::new(86_400, 1)?,
                                    role: valle_timeline::MotionRole::Effect,
                                },
                            );
                        }
                    }
                    for track in document["tracks"]["caption"]
                        .as_array()
                        .into_iter()
                        .flatten()
                    {
                        if let Some(component) = track["presenter"]["component"].as_str() {
                            durations.insert(
                                component.to_owned(),
                                valle_timeline::MotionSourceMetadata {
                                    duration: valle_timeline::RationalTime::new(86_400, 1)?,
                                    role: valle_timeline::MotionRole::CaptionPresenter {
                                        intro: valle_timeline::RationalTime::ZERO,
                                        outro: valle_timeline::RationalTime::ZERO,
                                    },
                                },
                            );
                        }
                    }
                    let projection = valle_compiler::compile_timeline_with_motion_sources(
                        timeline.clone(),
                        &durations,
                    )?;
                    let metadata = durations;
                    (
                        projection,
                        metadata,
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
            "motionSourceMetadata":motion_source_metadata,
            "inputDependencies":input_dependencies,
            "inputDependencyError":input_dependency_error,
            "render":{"timeline":serde_json::from_str::<Value>(&render_json)?,"timelineJson":render_json},
            "preview":{"status":"unavailable","code":"preview_not_prepared"},
        }))
    }

    pub fn save(&self, body: &str) -> Result<Value> {
        let _project_lock = self
            .project
            .as_ref()
            .map(|project| project.lock())
            .transpose()?;
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

/// Save a Motion arrangement into a new, self-contained project using the file save protocol.
pub(crate) fn save_motion_project(state: &crate::webhost::StudioHost, body: &str) -> Result<Value> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Request {
        directory: PathBuf,
        timeline: Value,
        expected_dependencies: BTreeMap<String, ContentDigest>,
    }
    let request: Request = serde_json::from_str(body)?;
    let motion = state
        .motion_source
        .as_ref()
        .context("Motion source session is unavailable")?;
    let config: Value = serde_json::from_str(
        &state
            .config_json
            .read()
            .map_err(|_| anyhow!("Studio config poisoned"))?,
    )?;
    if config["authorInputs"]["extraFonts"]
        .as_array()
        .is_some_and(|f| !f.is_empty())
    {
        anyhow::bail!(
            "extra --font stacks cannot be saved as a project; bind a declared font asset and preview again"
        );
    }
    let base = motion
        .input
        .parent()
        .context("Motion source has no parent")?;
    let mut document = request.timeline;
    let mut sources = Vec::new();
    for (alias, value) in document["resources"]
        .as_object_mut()
        .context("Timeline resources are missing")?
    {
        let locator = value.as_str().context("Timeline resource must be a path")?;
        if locator.contains("://") {
            anyhow::bail!("download remote resources before saving a self-contained project");
        }
        let path = if Path::new(locator).is_absolute() {
            PathBuf::from(locator)
        } else {
            std::path::absolute(locator)?
        };
        let path = path
            .canonicalize()
            .with_context(|| format!("opening resource {locator}"))?;
        crate::cmd::project::validate_component(&path)?;
        *value = json!(path);
        sources.push((alias.clone(), path));
    }
    let timeline = decode_timeline(&document.to_string())?;
    let captured = capture_file_motion_sources(&timeline, &motion.input)?;
    captured.verify_expected_dependencies(Some(&request.expected_dependencies))?;
    // Also verify the media facts that were admitted by the browser before copying.
    for (path, digest) in &request.expected_dependencies {
        if valle_project::folder::digest_file(Path::new(path))? != *digest {
            anyhow::bail!("input changed before saving project: {path}");
        }
    }
    let directory = if request.directory.is_absolute() {
        request.directory
    } else {
        base.join(request.directory)
    };
    let canvas = timeline.wire().canvas.clone();
    let fps = serde_json::to_value(&canvas)?["fps"].clone();
    let fps = fps
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| fps.to_string());
    let project =
        valle_project::folder::Project::create(&directory, canvas.width, canvas.height, &fps)?;
    {
        let _lock = project.lock()?;
        for (alias, path) in sources {
            let imported = project.import_file(&path, false)?;
            document["resources"][alias] = json!(imported.path);
        }
    }
    captured.verify_dependencies()?;
    let file = TimelineFile::open(&project.timeline_path())?;
    let copied = decode_timeline(&document.to_string())?;
    let copied_inputs = capture_file_motion_sources(&copied, &file.input)?.dependency_digests()?;
    let response=file.save(&json!({"baseRevision":file.revision()?,"timeline":document,"expectedDependencies":copied_inputs}).to_string())?;
    if response["outcome"] != "committed" && response["outcome"] != "unchanged" {
        anyhow::bail!(
            "project files were imported but the Timeline was not saved: {response}; inspect {}",
            project.root.display()
        );
    }
    let bytes = std::fs::read(&file.input)?;
    Ok(
        json!({"status":"saved","root":project.root,"target":file.input,"digest":ContentDigest::of_bytes(&bytes),"timeline":serde_json::from_slice::<Value>(&bytes)?}),
    )
}

pub(crate) fn studio_host(
    input: &Path,
    mut runtime_files: BTreeMap<String, crate::webruntime::HostedFile>,
) -> Result<Arc<crate::webhost::StudioHost>> {
    let file = Arc::new(TimelineFile::open(input)?);
    let project = file.project.as_ref().map(|project| json!({"name":project.config.name,"root":project.root,"timeline":project.config.timeline,"entry":project.timeline_path().canonicalize().ok().as_ref()==Some(&file.input)}));
    let config = json!({"input":file.input,"timelineFile":{"token":file.token},"project":project,"runtimeAssets":crate::webruntime::runtime_assets_json()});
    crate::cmd::motion::mount_motion_runtime_fonts(&mut runtime_files)?;
    let library = Some(crate::webhost::LibraryCtx {
        token: file.token.clone(),
        root: valle_project::folder::app_home()?.join("assets"),
    });
    let state = Arc::new(crate::webhost::StudioHost {
        runtime_files,
        assets_dir: None,
        preview_files: Arc::new(Default::default()),
        motion_source: None,
        config_json: Arc::new(RwLock::new(config.to_string())),
        sse: Default::default(),
        capture_dir: None,
        library,
        timeline_file: Some(file),
        source_paths: std::sync::Mutex::new(Default::default()),
        last_report: RwLock::new(None),
    });
    Ok(state)
}

pub(crate) fn serve(
    input: &Path,
    web_assets_dir: Option<&Path>,
    port: u16,
) -> Result<std::process::ExitCode> {
    let runtime = crate::webruntime::resolve(web_assets_dir)?;
    let state = studio_host(input, runtime.serving_map())?;
    let (server, addr) = crate::webhost::bind(port)?;
    let url = format!("http://{addr}/");
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
    fn author_snapshot_reads_no_motion_or_media_and_does_not_admit_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cut.json");
        let authored = json!({
            "canvas":{"width":64,"height":64,"fps":30},
            "resources":{"card":"missing.motion.tsx","hero":"missing.mov"},
            "tracks":{"visual":[{"clips":[{"kind":"motion","component":"card","start":0,"duration":1,"resources":{"hero":"hero"}}]}]}
        });
        std::fs::write(&path, authored.to_string()).unwrap();
        let file = TimelineFile::open(&path).unwrap();
        let trace = valle_compiler::motion::CompilationTrace::new(|_| {});
        let snapshot = file.author_snapshot().unwrap();
        assert_eq!(trace.metrics().compilations, 0);
        assert_eq!(snapshot["empty"], false);
        assert_eq!(snapshot["timeline"], authored);
        assert_eq!(snapshot["timelineRevision"]["revision"], 1);
        assert!(snapshot.get("render").is_none());
        assert!(snapshot.get("motionSourceMetadata").is_none());
        let saved = file
            .save(&json!({"baseRevision":1,"timeline":authored}).to_string())
            .unwrap();
        assert_eq!(saved["outcome"], "rejected");
        assert_eq!(file.revision().unwrap(), 1);
        assert_eq!(file.author_snapshot().unwrap()["timeline"], authored);
    }

    #[test]
    fn an_empty_project_is_an_editable_starting_point_without_a_fake_clip() {
        let directory = tempfile::tempdir().unwrap();
        let project =
            valle_project::folder::Project::create(&directory.path().join("promo"), 32, 32, "4")
                .unwrap();
        let file = TimelineFile::open(&project.timeline_path()).unwrap();
        let snapshot = file.snapshot().unwrap();
        assert_eq!(snapshot["empty"], true);
        assert_eq!(snapshot["timeline"]["tracks"], json!({}));
        assert_eq!(file.revision().unwrap(), 1);
    }

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
        assert_eq!(
            snapshot["motionSourceMetadata"]["card"],
            json!({
                "duration":"3/1", "role":{"type":"clip"}
            })
        );
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
    fn caption_edits_save_and_reopen_with_browser_confirmed_font_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("words.motion.tsx");
        std::fs::write(&source, "export const composition={width:160,height:48,duration:1}; export const role=captionPresenter({intro:0,outro:0}); export default function Words(props){ const caption = useCaption();return <Scene><Text style={{fontFamily:caption.style.font,fontSize:caption.style.fontSize}}>{caption.text}</Text></Scene>;}").unwrap();
        let font = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/fonts/noto/NotoSans-Regular.ttf")
            .canonicalize()
            .unwrap();
        let path = dir.path().join("captions.timeline.json");
        let mut authored = json!({"canvas":{"width":160,"height":48,"fps":4},
            "resources":{"words":"words.motion.tsx","font":font},
            "tracks":{"caption":[{"presenter":{"component":"words"},
                "style":{"font":"font","fontSize":20},"clips":[{"start":0.125,"duration":2,
                    "runs":[{"text":"Build","start":0,"end":1.75}]}]}]}});
        std::fs::write(&path, authored.to_string()).unwrap();
        let file = TimelineFile::open(&path).unwrap();
        let loaded = file.snapshot().unwrap();
        assert!(loaded["inputDependencyError"].is_null(), "{loaded}");
        authored["tracks"]["caption"][0]["clips"][0]["runs"][0] =
            json!({"text":"Create","start":0,"end":1.5});
        let mut cache = crate::preview_store::FrozenMediaCache::default();
        let (_, _, mut expected, _, _) = crate::cmd::timeline::prepare_timeline_media_facts(
            decode_timeline(&authored.to_string()).unwrap(),
            dir.path(),
            &mut cache,
        )
        .unwrap();
        // A successful browser preview replaces the loaded dependency set with
        // current media facts plus confirmed source files.
        let source_path = source.canonicalize().unwrap().to_string_lossy().to_string();
        let source_digest =
            serde_json::from_value(loaded["inputDependencies"][&source_path].clone()).unwrap();
        expected.insert(source_path, source_digest);
        let save = file
            .save(
                &json!({"baseRevision":1,"timeline":authored,
            "expectedDependencies":expected})
                .to_string(),
            )
            .unwrap();
        assert_eq!(save["outcome"], "committed", "{save}");
        let reopened = TimelineFile::open(&path).unwrap().snapshot().unwrap();
        assert!(reopened["inputDependencyError"].is_null(), "{reopened}");
        assert_eq!(reopened["timeline"], authored);
        let items = reopened["render"]["timeline"]["document"]["captions"]["tracks"][0]["items"]
            .as_array()
            .unwrap();
        assert_eq!(
            items.iter().filter(|item| item["type"] == "motion").count(),
            1
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
        let (_, _, media_dependencies, _, _) = crate::cmd::timeline::prepare_timeline_media_facts(
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

#[cfg(test)]
mod contract_tests {
    use super::*;

    fn host(input: PathBuf) -> crate::webhost::StudioHost {
        crate::webhost::StudioHost {
            runtime_files: Default::default(),
            assets_dir: None,
            preview_files: Arc::new(Default::default()),
            motion_source: Some(crate::webhost::MotionSourceCtx {
                token: "fixture".into(),
                input,
                data: None,
            }),
            config_json: Arc::new(RwLock::new(
                json!({"authorInputs":{"extraFonts":[]}}).to_string(),
            )),
            sse: Default::default(),
            capture_dir: None,
            library: None,
            timeline_file: None,
            source_paths: Mutex::new(Default::default()),
            last_report: RwLock::new(None),
        }
    }

    #[test]
    fn motion_save_project_copies_resources_and_rejects_changed_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("card.motion.tsx");
        std::fs::write(&source,"export const composition={width:32,height:32,fps:4,duration:1};export default function Card(){return <Scene/>;}").unwrap();
        let state = host(source.clone());
        let document = json!({"canvas":{"width":32,"height":32,"fps":4},"resources":{"card":source},"tracks":{"visual":[{"clips":[{"kind":"motion","component":"card","start":0,"duration":1}]}]}});
        let dependencies = json!({source.canonicalize().unwrap().to_string_lossy().to_string():ContentDigest::of_bytes(&std::fs::read(&source).unwrap())});
        let request =
            json!({"directory":"promo","timeline":document,"expectedDependencies":dependencies});
        let mut changed = request.clone();
        changed["expectedDependencies"][source.to_str().unwrap()] =
            json!(ContentDigest::of_bytes(b"old source"));
        assert!(save_motion_project(&state, &changed.to_string()).is_err());
        assert!(!dir.path().join("promo").exists());
        let saved = save_motion_project(&state, &request.to_string()).unwrap();
        assert_eq!(saved["status"], "saved");
        assert_eq!(
            saved["timeline"]["resources"]["card"],
            "components/card.motion.tsx"
        );
        assert!(save_motion_project(&state, &request.to_string()).is_err());
        let file = TimelineFile::open(Path::new(saved["target"].as_str().unwrap())).unwrap();
        assert!(file.project.is_some());
        assert_eq!(
            std::fs::read(dir.path().join("promo/components/card.motion.tsx")).unwrap(),
            std::fs::read(&source).unwrap()
        );
    }
}
