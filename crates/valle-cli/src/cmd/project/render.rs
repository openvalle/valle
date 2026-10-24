//! Project delivery reuses Timeline preparation and captures the same inputs in source history.

use super::super::render_delivery::{DeliveryObservation, DeliveryPlan};
use crate::{RenderBudgetArgs, RenderOutputArgs};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use valle_project::{
    files::{occupied, suffixed},
    folder::{CapturedInput, Project, digest_file, now_millis, write_json},
};
use valle_render::host::{NativeProject, NativeRenderOptions, NativeRenderer};

pub(super) fn render(
    project: &Project,
    mut delivery: RenderOutputArgs,
    budget: RenderBudgetArgs,
) -> Result<Value> {
    let complete =
        delivery.frame.is_none() && delivery.frames.is_empty() && delivery.storyboard.is_none();
    let mut random = [0; 8];
    getrandom::fill(&mut random)?;
    let id = format!("{}-{}", now_millis(), hex::encode(random));
    let observation = DeliveryObservation::new()?;
    let lock = project.lock()?;
    let bytes = fs::read(project.timeline_path())?;
    let timeline = valle_timeline::decode_timeline(std::str::from_utf8(&bytes)?)?;
    let canvas = serde_json::from_slice::<Value>(&bytes)?["canvas"].clone();
    let base = project.timeline_path().parent().unwrap().to_path_buf();
    let mut prepared = super::super::timeline::prepare_timeline_package(timeline, &base)?;
    prepared
        .snapshot_inputs
        .push(CapturedInput::source(project.timeline_path(), bytes));
    let saved = if complete {
        Some(project.save_captured(prepared.snapshot_inputs.clone(), None, "render")?)
    } else {
        None
    };
    drop(lock);
    let automatic = delivery.output.is_none() && delivery.storyboard.is_none();
    if automatic {
        let filename = if let Some(saved) = &saved {
            format!(
                "{}-{}.mp4",
                project.root.file_name().unwrap().to_string_lossy(),
                saved.snapshot
            )
        } else {
            format!("preview-{id}.png")
        };
        delivery.output = Some(project.root.join("renders").join(filename));
    } else {
        delivery.output = delivery.output.map(|path| project.root.join(path));
        delivery.storyboard = delivery.storyboard.map(|path| project.root.join(path));
    }
    let prefix = format!(".valle-part-{id}-");
    let stage = |path: &Path| -> Result<PathBuf> {
        let parent = path.parent().context("output has no parent")?;
        fs::create_dir_all(parent)?;
        Ok(parent.join(format!(
            "{prefix}{}",
            path.file_name().unwrap().to_string_lossy()
        )))
    };
    let mut staging = delivery.clone();
    staging.output = delivery.output.as_deref().map(stage).transpose()?;
    staging.storyboard = delivery.storyboard.as_deref().map(stage).transpose()?;
    let plan = DeliveryPlan::new(staging.clone())?;
    let renderer = NativeRenderer::new(
        NativeProject::from_render(
            prepared.opened.engine_render(),
            Arc::new(std::mem::replace(&mut prepared.catalog, Default::default())),
        ),
        NativeRenderOptions {
            backend: valle_render::executor::skia::SkiaBackendKind::Raster,
            budget: budget.native()?,
            background: valle_engine::resource::OutputBackground::AuthorSrgbStraight {
                color: valle_engine::resource::AuthorSrgbStraight(
                    prepared.opened.engine_render().canvas().background_rgba(),
                ),
            },
            progress: crate::output::render_progress(),
            ..Default::default()
        },
    );
    let (summary, details) = plan.execute(&renderer)?;
    let mut staged = details["outputs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(PathBuf::from))
        .collect::<Vec<_>>();
    if let Some(path) = &staging.storyboard {
        staged.push(path.clone());
    }
    if plan.codec.is_some() {
        staged.push(staging.output.clone().unwrap());
    }
    let _lock = project.lock()?;
    let mut outputs = Vec::new();
    for path in &staged {
        let filename = path.file_name().unwrap().to_string_lossy();
        let name = filename
            .strip_prefix(&prefix)
            .context("invalid staged output")?;
        let parent = path.parent().unwrap();
        let mut name = name.to_owned();
        if automatic {
            let base = name.clone();
            let mut number = 2;
            while occupied(parent, &name)? {
                name = suffixed(&base, number);
                number += 1;
            }
        }
        outputs.push(
            json!({"path":project.path_key(&parent.join(name))?,"digest":digest_file(path)?}),
        );
    }
    let mut warnings = Vec::new();
    for input in &prepared.snapshot_inputs {
        if input.bytes.is_none()
            && (input.stamp.is_none()
                || valle_project::files::FileStamp::read(&input.path).ok() != input.stamp)
            && digest_file(&input.path).ok() != Some(input.digest)
        {
            warnings.push(json!({"code":"media_changed","path":project.path_key(&input.path)?,"message":"Media changed during export; this output uses the captured original and may no longer be reproducible from the project."}));
        }
    }
    let record = json!({"id":id,"createdAt":now_millis(),"snapshot":saved.as_ref().map(|s|s.snapshot),"outputs":outputs,
        "inputs":prepared.snapshot_inputs.iter().map(|input|Ok(json!({"path":project.path_key(&input.path)?,"digest":input.digest,"fileIdentity":input.stamp}))).collect::<Result<Vec<_>>>()?,
        "receipt":serde_json::from_str::<Value>(prepared.opened.receipt_json())?,"parameters":{"frame":delivery.frame,"frames":delivery.frames,"storyboard":delivery.storyboard,"codec":details["codec"],"canvas":canvas},"valleVersion":env!("CARGO_PKG_VERSION"),"warnings":warnings});
    let record_path = saved.as_ref().map(|s| {
        project
            .snapshot_dir(s.snapshot)
            .join("exports")
            .join(format!("{id}.json"))
    });
    if let Some(path) = &record_path {
        write_json(path, &record, false)?;
    }
    let mut published = Vec::new();
    let publish = (|| -> Result<()> {
        for (source, output) in staged.iter().zip(&outputs) {
            let target = project.root.join(output["path"].as_str().unwrap());
            let file = fs::OpenOptions::new().read(true).write(true).open(source)?;
            let temp = tempfile::NamedTempFile::from_parts(
                file,
                tempfile::TempPath::try_from_path(source.clone())?,
            );
            temp.persist_noclobber(&target).map_err(|e| e.error)?;
            published.push(target);
        }
        Ok(())
    })();
    if let Err(error) = publish {
        for path in published {
            let _ = fs::remove_file(path);
        }
        if let Some(path) = record_path {
            let _ = fs::remove_file(path);
        }
        return Err(error);
    }
    let mut report = super::super::delivery_report::delivery_report(
        &prepared.opened,
        if complete { "export" } else { "preview" },
        &project.root.join(
            outputs
                .first()
                .and_then(|o| o["path"].as_str())
                .unwrap_or("renders"),
        ),
        &summary,
        None,
    )?;
    report["snapshot"] = json!(saved.as_ref().map(|s| s.snapshot));
    report["exportId"] = json!(if complete { Some(id) } else { None });
    report["outputs"] = json!(outputs);
    report["warnings"] = json!(warnings);
    report["compilations"] = json!(observation.trace.metrics().compilations);
    Ok(report)
}
