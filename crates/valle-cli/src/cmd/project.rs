//! CLI and Studio operations for ordinary project folders.

mod render;
pub(crate) mod web;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};
use valle_project::{
    files::Kind,
    folder::{CapturedInput, Project, absolute, app_home, now_millis, write_json},
};
use valle_timeline::decode_timeline;

use crate::ProjectAction;

pub(crate) fn run(
    _json: bool,
    directory: Option<&Path>,
    action: ProjectAction,
) -> Result<ExitCode> {
    let start = absolute(directory.unwrap_or(Path::new(".")))?;
    let rendering = matches!(&action, ProjectAction::Render { .. });
    match run_inner(&start, action) {
        Ok((code, Some(report))) => {
            crate::output::emit(report);
            Ok(code)
        }
        Ok((code, None)) => Ok(code),
        Err(error) if rendering => Err(error),
        Err(error) => {
            crate::output::error_with_details(
                "project_failed",
                &format!("{error:#}"),
                Default::default(),
                Some("valle project check"),
            );
            Ok(ExitCode::FAILURE)
        }
    }
}

fn run_inner(start: &Path, action: ProjectAction) -> Result<(ExitCode, Option<Value>)> {
    if let ProjectAction::Create {
        directory,
        width,
        height,
        fps,
    } = &action
    {
        let project = Project::create(&start.join(directory), *width, *height, fps)?;
        remember(&project)?;
        return Ok((
            ExitCode::SUCCESS,
            Some(
                json!({"status":"ok","root":project.root,"files":["project.json","timeline.json","components/","media/","renders/",".valle/history/"]}),
            ),
        ));
    }
    if let ProjectAction::Studio {
        web_assets_dir,
        port,
    } = &action
    {
        if !start
            .ancestors()
            .any(|root| root.join("project.json").is_file())
        {
            return Ok((web::serve_home(web_assets_dir.as_deref(), *port)?, None));
        }
    }
    let project = Project::discover(start)?;
    remember(&project)?;
    let report = match action {
        ProjectAction::Create { .. } => unreachable!(),
        ProjectAction::Show => summary(&project)?,
        ProjectAction::Add { files, link } => {
            let report = add(&project, &files, link)?;
            return Ok((
                if report["status"] == "partial" {
                    ExitCode::FAILURE
                } else {
                    ExitCode::SUCCESS
                },
                Some(report),
            ));
        }
        ProjectAction::Check => check(&project)?,
        ProjectAction::Save { message } => save(&project, message.as_deref())?,
        ProjectAction::History => json!({"status":"ok","snapshots":project.history()?}),
        ProjectAction::Restore { snapshot } => restore(&project, snapshot)?,
        ProjectAction::Studio {
            web_assets_dir,
            port,
        } => {
            return Ok((
                crate::timeline_file::serve(
                    &project.timeline_path(),
                    web_assets_dir.as_deref(),
                    port,
                )?,
                None,
            ));
        }
        ProjectAction::Render { delivery, budget } => {
            render::render(&project, delivery.into(), budget)?
        }
    };
    Ok((
        if report["status"] == "error" {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        },
        Some(report),
    ))
}

pub(crate) fn save(project: &Project, message: Option<&str>) -> Result<Value> {
    let _lock = project.lock()?;
    let saved = project.save_captured(capture_inputs(project)?, message, "save")?;
    Ok(json!({"status":"ok","snapshot":saved.snapshot,"unchanged":saved.unchanged}))
}

pub(crate) fn restore(project: &Project, snapshot: u64) -> Result<Value> {
    let _lock = project.lock()?;
    let mut value = serde_json::to_value(
        project.restore(snapshot, capture_inputs(project).unwrap_or_default())?,
    )?;
    value["status"] = json!("ok");
    Ok(value)
}

pub(crate) fn remember(project: &Project) -> Result<()> {
    let path = app_home()?.join("recent-projects.json");
    let mut entries = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<Value>>(&bytes).ok())
        .unwrap_or_default();
    let root = project.root.to_string_lossy();
    entries.retain(|entry| entry["path"].as_str() != Some(root.as_ref()));
    entries.insert(
        0,
        json!({"path":project.root,"name":project.config.name,"lastOpened":now_millis()}),
    );
    entries.truncate(30);
    write_json(&path, &entries, true)
}

pub(crate) fn validate_component(path: &Path) -> Result<()> {
    if Kind::detect(path) != Kind::Component {
        return Ok(());
    }
    let graph = valle_compiler::motion::MotionModuleGraph::from_entry_path(path)
        .map_err(|diagnostics| anyhow::anyhow!("cannot read component imports: {diagnostics:?}"))?;
    if graph.modules.len() > 1 {
        bail!(
            "component imports other source files: {}; copy the component and these files together, preserving their relative paths, into components/",
            graph
                .modules
                .keys()
                .filter(|key| *key != &graph.entry)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(())
}

pub(crate) fn add(project: &Project, files: &[PathBuf], link: bool) -> Result<Value> {
    let _lock = project.lock()?;
    let mut items = Vec::new();
    let mut aliases = BTreeSet::new();
    for file in files {
        let result = (|| -> Result<Value> {
            validate_component(file)?;
            let mut imported = project.import_file(file, link)?;
            let base = imported.alias.clone();
            let mut number = 2;
            while !aliases.insert(imported.alias.clone()) {
                imported.alias = format!("{base}-{number}");
                number += 1;
            }
            let mut item = serde_json::to_value(&imported)?;
            item["status"] = json!("ok");
            item["resource"] = json!({imported.alias:imported.path});
            item["info"] = probe(file, imported.kind);
            Ok(item)
        })();
        items.push(match result {Ok(item)=>item,Err(error)=>json!({"status":"error","input":file,"error":{"code":"import_failed","message":format!("{error:#}"),"hint":"valle project show"}})});
    }
    let failed = items
        .iter()
        .filter(|item| item["status"] == "error")
        .count();
    Ok(json!({"status":if failed==0 {"ok"}else{"partial"},"items":items,"failed":failed}))
}

pub(crate) fn probe(path: &Path, kind: Kind) -> Value {
    match kind {
        Kind::Component => (|| -> Option<Value> {
            let graph=valle_compiler::motion::MotionModuleGraph::from_entry_path(path).ok()?;
            let compiled=valle_compiler::motion::compile_motion_modules(&graph).ok()?;
            let composition=compiled.artifact.composition?;
            Some(json!({"duration":composition.duration().ok()?.as_f64(),"width":composition.width,"height":composition.height,"fps":composition.fps}))
        })().unwrap_or(Value::Null),
        Kind::Video|Kind::Audio=>valle_media::codec::probe_av(path).ok().map(|info| {
            let fps=if kind==Kind::Video {valle_media::codec::decode::probe_video_presentation(path).ok().map(|probe|probe.display.nominal_fps)}else{None};
            json!({"duration":info.video.map(|(_,_,d)|d).or(info.audio),"width":info.video.map(|(w,_,_)|w),"height":info.video.map(|(_,h,_)|h),"fps":fps,"hasAudio":info.audio.is_some()})
        }).unwrap_or(Value::Null),
        Kind::Image=>fs::read(path).ok().and_then(|b|valle_render::host::probe_image_extent(&b).ok()).map(|(width,height)|json!({"width":width,"height":height})).unwrap_or(Value::Null),
        _=>Value::Null,
    }
}

pub(crate) fn capture_inputs(project: &Project) -> Result<Vec<CapturedInput>> {
    let bytes = fs::read(project.timeline_path())?;
    let timeline = decode_timeline(std::str::from_utf8(&bytes)?)?;
    let document: Value = serde_json::from_slice(&bytes)?;
    let base = project.timeline_path().parent().unwrap().to_path_buf();
    let motion = valle_compiler::motion_preparation_inputs(&timeline)?;
    let components = motion
        .iter()
        .map(|input| input.component.as_str())
        .collect::<BTreeSet<_>>();
    let mut inputs = Vec::new();
    for input in &motion {
        let entry = base.join(&input.locator);
        let graph = valle_compiler::motion::MotionModuleGraph::from_entry_path(&entry).map_err(
            |diagnostics| anyhow::anyhow!("cannot capture component imports: {diagnostics:?}"),
        )?;
        let root = entry.parent().context("component has no parent")?;
        for (module, source) in graph.modules {
            let path = root.join(module);
            let captured = fs::read(&path)?;
            if captured != source.as_bytes() {
                bail!("source changed while capturing: {}; retry", path.display());
            }
            inputs.push(CapturedInput::source(path, captured));
        }
    }
    let mut cache = valle_project::folder::DigestCache::open(project);
    let mut used = BTreeSet::new();
    super::timeline::collect_used_resources(&document["tracks"], &mut used);
    for alias in used {
        if components.contains(alias.as_str()) {
            continue;
        }
        let locator = document["resources"][&alias]
            .as_str()
            .with_context(|| format!("missing resource {alias}"))?;
        let path = if locator.starts_with("http://") || locator.starts_with("https://") {
            valle_render::AssetCache::new(valle_render::default_cache_dir())?
                .fetch(locator, "bin")?
        } else {
            base.join(locator)
        };
        if Kind::detect(&path).snapshot_source() {
            let (_, captured) = super::motion::capture_asset(&path)?;
            inputs.extend(captured);
        } else {
            inputs.push(
                cache
                    .capture(&path)
                    .with_context(|| format!("reading {alias}: {}", path.display()))?,
            );
        }
    }
    inputs.push(CapturedInput::source(project.timeline_path(), bytes));
    for input in &inputs {
        if let Some(bytes) = &input.bytes
            && fs::read(&input.path)? != *bytes
        {
            bail!(
                "source changed while capturing: {}; retry",
                input.path.display()
            );
        }
    }
    cache.persist()?;
    Ok(inputs)
}

pub(crate) fn summary(project: &Project) -> Result<Value> {
    let document: Value = serde_json::from_slice(&fs::read(project.timeline_path())?)?;
    let base = project.timeline_path().parent().unwrap().to_path_buf();
    let mut resources = Vec::new();
    let mut outside = Vec::new();
    let mut missing = Vec::new();
    for (alias, value) in document["resources"].as_object().into_iter().flatten() {
        let Some(locator) = value.as_str() else {
            continue;
        };
        let remote = locator.contains("://");
        let path = base.join(locator);
        let external = remote || absolute(&path)?.strip_prefix(&project.root).is_err();
        let entry = json!({"alias":alias,"path":if remote{locator.to_owned()}else{project.path_key(&path)?},"kind":Kind::detect(&path),"external":external,"missing":!remote&&!path.is_file()});
        if external {
            outside.push(entry.clone());
        }
        if !remote && !path.is_file() {
            missing.push(entry.clone());
        }
        resources.push(entry);
    }
    let history = project.history()?;
    let seconds = |value: &Value| -> f64 {
        value
            .as_f64()
            .or_else(|| {
                value.as_str().and_then(|s| {
                    s.split_once('/')
                        .and_then(|(a, b)| Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?))
                        .or_else(|| s.parse().ok())
                })
            })
            .unwrap_or(0.0)
    };
    let duration = document["tracks"]
        .as_object()
        .into_iter()
        .flat_map(|tracks| tracks.values())
        .flat_map(|tracks| tracks.as_array().into_iter().flatten())
        .flat_map(|track| track["clips"].as_array().into_iter().flatten())
        .map(|clip| seconds(&clip["start"]) + seconds(&clip["duration"]))
        .fold(0.0, f64::max);
    let mut exports = Vec::new();
    for snapshot in &history {
        let dir = project.snapshot_dir(snapshot.number).join("exports");
        if !dir.is_dir() {
            continue;
        }
        for file in fs::read_dir(dir)? {
            let path = file?.path();
            if path.extension().is_some_and(|e| e == "json") {
                exports.push(serde_json::from_slice::<Value>(&fs::read(path)?)?);
            }
        }
    }
    exports.sort_by_key(|entry| entry["createdAt"].as_u64().unwrap_or(0));
    Ok(
        json!({"status":"ok","root":project.root,"name":project.config.name,"timeline":project.config.timeline,"canvas":document["canvas"],"duration":duration,"tracks":document["tracks"],"resources":resources,"outsideReferences":outside,"missing":missing,"snapshots":history.len(),"lastExport":exports.last()}),
    )
}

pub(crate) fn check(project: &Project) -> Result<Value> {
    let report = summary(project)?;
    let timeline = decode_timeline(&fs::read_to_string(project.timeline_path())?)?;
    let base = project.timeline_path().parent().unwrap().to_path_buf();
    let mut result = json!({"status":"ok","outsideReferences":report["outsideReferences"],"missing":report["missing"],"warnings":[]});
    match super::timeline::prepare_timeline_package(timeline, &base) {
        Ok(prepared) => {
            result["warnings"] =
                serde_json::to_value(super::timeline::clock_warnings(prepared.opened.compiled())?)?;
        }
        Err(error) => {
            result["status"] = json!("error");
            result["error"] = json!({"code":"project_check_failed","message":format!("{error:#}"),"hint":"valle project show"});
        }
    }
    Ok(result)
}
