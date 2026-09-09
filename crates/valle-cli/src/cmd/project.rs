//! Timeline project CLI.
//!
//! Every authoring save carries one complete sparse document. There are no
//! clip/track patch commands in this adapter.

use std::{
    io::Write as _,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{Arc, RwLock},
};

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use valle_project::revision::{
    Actor, AuthenticatedContext, ProjectId, ProjectStore, ProjectTimelineSnapshot,
    SnapshotWriteResult,
};
use valle_timeline::{
    Timeline, decode_timeline, timeline_bytes, wire::edit::EditTimelineResultWire,
};

use crate::ProjectAction;

pub(crate) fn run(json_output: bool, action: ProjectAction) -> Result<ExitCode> {
    let store = ProjectStore::at(project_root()?);
    let auth = AuthenticatedContext::new(
        Actor::new("cli:local").map_err(|error| anyhow!(error.to_string()))?,
    );

    match action {
        ProjectAction::Create {
            project_id,
            timeline,
            intent,
        } => {
            let project_id = parse_project_id(project_id)?;
            let timeline = load_timeline(&timeline)?;
            let snapshot = store
                .create_project(&project_id, &timeline, intent.as_deref(), &auth)
                .context("creating project genesis")?;
            print_value(&snapshot_value(&snapshot)?, json_output);
            Ok(ExitCode::SUCCESS)
        }
        ProjectAction::GetTimeline {
            project_id,
            revision,
            output,
        } => {
            let project_id = parse_project_id(project_id)?;
            let snapshot = store
                .get_timeline(&project_id, revision)
                .context("reading project Timeline snapshot")?;
            if let Some(path) = output {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)?
                    .write_all(&timeline_bytes(snapshot.timeline())?)
                    .with_context(|| format!("writing {}", path.display()))?;
            }
            print_value(&snapshot_value(&snapshot)?, json_output);
            Ok(ExitCode::SUCCESS)
        }
        ProjectAction::ListTimelineRevisions {
            project_id,
            cursor,
            limit,
        } => {
            let project_id = parse_project_id(project_id)?;
            let page = store
                .list_revisions_with_limit(&project_id, cursor, limit)
                .context("listing project Timeline revisions")?;
            print_value(
                &json!({
                    "projectId": project_id,
                    "revisions": page.revisions,
                    "nextCursor": page.next_cursor,
                }),
                json_output,
            );
            Ok(ExitCode::SUCCESS)
        }
        ProjectAction::EditTimeline {
            project_id,
            base_revision,
            timeline,
            intent,
        } => {
            let project_id = parse_project_id(project_id)?;
            let request = load_edit_request(&timeline, base_revision, intent.as_deref())?;
            let response = store
                .edit_timeline_json(&project_id, &request, &auth)
                .context("submitting complete Timeline document")?;
            let success = matches!(
                response.result,
                EditTimelineResultWire::Committed { .. } | EditTimelineResultWire::Unchanged { .. }
            );
            print_serializable(&response, json_output)?;
            Ok(if success {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        ProjectAction::Studio {
            project_id,
            revision,
            web_assets_dir,
            port,
        } => {
            let project_id = parse_project_id(project_id)?;
            let snapshot = store
                .get_timeline(&project_id, revision)
                .context("reading Project Studio initial revision")?;
            let initial_revision = snapshot.revision().revision;
            let token = project_studio_token()?;
            let cache_root = crate::webruntime::default_cache_root()?;
            let runtime = crate::webruntime::resolve(web_assets_dir.as_deref(), &cache_root)?;
            let config = serde_json::json!({
                "runtimeAssets": crate::webruntime::runtime_assets_json()
            })
            .to_string();
            let state = Arc::new(crate::webhost::StudioHost {
                runtime_files: runtime.serving_map(),
                assets_dir: None,
                config_json: Arc::new(RwLock::new(config)),
                sse: crate::webhost::SseBroadcaster::default(),
                capture_dir: None,
                library: None,
                project: Some(crate::webhost::ProjectStudioCtx {
                    token,
                    project_id: project_id.clone(),
                    initial_revision,
                    store,
                    auth: AuthenticatedContext::new(
                        Actor::new(format!("studio:{project_id}"))
                            .map_err(|error| anyhow!(error.to_string()))?,
                    ),
                }),
                last_report: RwLock::new(None),
            });
            let (server, addr) = crate::webhost::bind(port)?;
            let url = format!("http://{addr}/studio?project={project_id}");
            if crate::output::events() {
                crate::events::emit(crate::events::EventKind::Ready {
                    url: url.clone(),
                    port: addr.port(),
                    runtime_version: runtime.manifest.runtime_version.clone(),
                    runtime_source: runtime.source.as_str().to_owned(),
                    project_id: Some(project_id.to_string()),
                    revision: Some(initial_revision),
                });
            } else if json_output {
                crate::output::emit(json!({
                    "status": "ready",
                    "url": url,
                    "port": addr.port(),
                    "projectId": project_id,
                    "revision": state.project.as_ref().map(|project| project.initial_revision),
                    "runtime_version": runtime.manifest.runtime_version,
                    "runtime_source": runtime.source.as_str(),
                }));
                std::io::stdout().flush()?;
            } else {
                eprintln!("project studio: {url} (Ctrl-C to stop)");
            }
            crate::webhost::serve_forever(server, state)?;
            Ok(ExitCode::SUCCESS)
        }
        ProjectAction::Render {
            project_id,
            revision,
            output,
            frame,
        } => {
            let id = parse_project_id(project_id)?;
            let snapshot = store.get_timeline(&id, revision)?;
            super::timeline::render_document(
                snapshot.timeline().clone(),
                &std::env::current_dir()?,
                &output,
                frame,
            )
        }
        ProjectAction::RestoreTimelineRevision {
            project_id,
            base_revision,
            source_revision,
            intent,
        } => {
            let project_id = parse_project_id(project_id)?;
            let result = store
                .restore_timeline_revision(
                    &project_id,
                    base_revision,
                    source_revision,
                    intent.as_deref(),
                    &auth,
                )
                .context("restoring Timeline revision")?;
            let success = matches!(
                result,
                SnapshotWriteResult::Committed { .. } | SnapshotWriteResult::Unchanged { .. }
            );
            print_value(&write_result_value(result), json_output);
            Ok(if success {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
    }
}

fn project_root() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("VALLE_HOME") {
        return Ok(PathBuf::from(root));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or_else(|| anyhow!("set VALLE_HOME because no user home directory is available"))?;
    Ok(PathBuf::from(home).join(".valle"))
}

fn parse_project_id(value: String) -> Result<ProjectId> {
    ProjectId::new(value).map_err(|error| anyhow!(error.to_string()))
}

fn load_timeline(path: &Path) -> Result<Timeline> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    decode_timeline(&normalize_resource_paths(&text, path)?)
        .with_context(|| format!("decoding Timeline {}", path.display()))
}

fn load_edit_request(path: &Path, base_revision: u64, intent: Option<&str>) -> Result<String> {
    let timeline =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let timeline = normalize_resource_paths(&timeline, path)?;
    let intent = serde_json::to_string(&intent)?;
    Ok(format!(
        "{{\"baseRevision\":{base_revision},\"timeline\":{timeline},\"intent\":{intent}}}"
    ))
}

fn normalize_resource_paths(text: &str, path: &Path) -> Result<String> {
    let mut value: Value = serde_json::from_str(text)?;
    let base = std::path::absolute(path)?.parent().unwrap().to_path_buf();
    if let Some(resources) = value.get_mut("resources").and_then(Value::as_object_mut) {
        for locator in resources.values_mut() {
            if let Some(s) = locator.as_str() {
                if !s.contains("://") && !Path::new(s).is_absolute() {
                    *locator = Value::String(base.join(s).to_string_lossy().into_owned());
                }
            }
        }
    }
    Ok(serde_json::to_string(&value)?)
}

fn project_studio_token() -> Result<String> {
    let mut token = [0_u8; 32];
    getrandom::fill(&mut token).context("generating local Studio authentication token")?;
    Ok(hex::encode(token))
}

fn snapshot_value(snapshot: &ProjectTimelineSnapshot) -> Result<Value> {
    let timeline: Value = serde_json::from_slice(&timeline_bytes(snapshot.timeline())?)?;
    Ok(json!({
        "projectId": snapshot.project_id(),
        "revision": snapshot.revision().revision,
        "timeline": timeline,
    }))
}

fn write_result_value(result: SnapshotWriteResult) -> Value {
    match result {
        SnapshotWriteResult::Committed { snapshot } => json!({
            "outcome": "committed",
            "revision": snapshot.revision().revision,
        }),
        SnapshotWriteResult::Unchanged { snapshot } => json!({
            "outcome": "unchanged",
            "revision": snapshot.revision().revision,
        }),
        SnapshotWriteResult::StaleBase { actual } => json!({
            "outcome": "staleBase",
            "revision": actual.revision().revision,
        }),
        SnapshotWriteResult::Rejected { errors } => json!({
            "outcome": "rejected",
            "errors": errors,
        }),
    }
}

fn print_serializable(value: &impl serde::Serialize, json_output: bool) -> Result<()> {
    let value = serde_json::to_value(value)?;
    print_value(&value, json_output);
    Ok(())
}

fn print_value(value: &Value, _json_output: bool) {
    crate::output::emit(value.clone());
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::project_studio_token;

    #[test]
    fn studio_token_has_fresh_256_bit_hex_encoding() {
        let tokens = (0..32)
            .map(|_| project_studio_token().unwrap())
            .collect::<Vec<_>>();
        assert!(tokens.iter().all(|token| {
            token.len() == 64
                && token
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        }));
        assert_eq!(tokens.iter().collect::<HashSet<_>>().len(), tokens.len());
    }
}
