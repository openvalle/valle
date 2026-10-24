//! Studio adapters. Project operations reuse the CLI functions and file save protocols.
use crate::{
    RenderBudgetArgs, RenderOutputArgs, RenderVideoCodec,
    webhost::{LibraryCtx, StudioHost},
    webruntime::HostedFile,
};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{Arc, Mutex, RwLock},
};
use valle_project::{
    files::{Kind, files_in},
    folder::{Project, app_home},
};

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum Command {
    Recent,
    Create {
        directory: PathBuf,
        width: Option<u32>,
        height: Option<u32>,
        fps: Option<String>,
    },
    Open {
        directory: PathBuf,
    },
    Show,
    Add {
        files: Vec<PathBuf>,
        #[serde(default)]
        link: bool,
    },
    Check,
    Save {
        message: Option<String>,
    },
    History,
    Restore {
        snapshot: u64,
    },
    Exports,
    Render {
        output: Option<PathBuf>,
        codec: Option<String>,
    },
}

pub(crate) fn project(state: &StudioHost) -> Result<Project> {
    let input = state
        .timeline_file
        .as_ref()
        .map(|f| &f.input)
        .or_else(|| state.motion_source.as_ref().map(|m| &m.input))
        .context("no project is open")?;
    Project::discover(input)
}

fn require_entry(state: &StudioHost, project: &Project) -> Result<()> {
    if state
        .timeline_file
        .as_ref()
        .is_none_or(|f| project.timeline_path().canonicalize().ok().as_ref() != Some(&f.input))
    {
        bail!(
            "history and project export apply only to the entry Timeline; open it with project studio"
        );
    }
    Ok(())
}

pub(crate) fn execute(state: &StudioHost, body: &str) -> Result<Value> {
    let command: Command = serde_json::from_str(body)?;
    match command {
        Command::Recent => return recent(),
        Command::Create {
            directory,
            width,
            height,
            fps,
        } => {
            let project = Project::create(
                &directory,
                width.unwrap_or(1920),
                height.unwrap_or(1080),
                fps.as_deref().unwrap_or("30"),
            )?;
            super::remember(&project)?;
            return open(&project, state.runtime_files.clone());
        }
        Command::Open { directory } => {
            let project = Project::discover(&directory)?;
            super::remember(&project)?;
            return open(&project, state.runtime_files.clone());
        }
        _ => (),
    }
    let project = project(state)?;
    match command {
        Command::Show => {
            let mut report = super::summary(&project)?;
            let mut files = Vec::new();
            for directory in ["media", "components"] {
                for path in files_in(&project.root.join(directory))? {
                    files.push(json!({"path":project.path_key(&path)?,"absolutePath":path,"kind":Kind::detect(&path),"info":super::probe(&path,Kind::detect(&path))}));
                }
            }
            report["files"] = json!(files);
            Ok(report)
        }
        Command::Add { files, link } => super::add(&project, &files, link),
        Command::Check => super::check(&project),
        Command::Save { message } => {
            require_entry(state, &project)?;
            super::save(&project, message.as_deref())
        }
        Command::History => {
            require_entry(state, &project)?;
            Ok(json!({"status":"ok","snapshots":project.history()?}))
        }
        Command::Restore { snapshot } => {
            require_entry(state, &project)?;
            super::restore(&project, snapshot)
        }
        Command::Exports => {
            require_entry(state, &project)?;
            let records = export_records(&project)?;
            let files = files_in(&project.root.join("renders"))?
                .iter()
                .map(|p| Ok(json!({"path":project.path_key(p)?,"bytes":fs::metadata(p)?.len()})))
                .collect::<Result<Vec<_>>>()?;
            Ok(json!({"status":"ok","records":records,"files":files}))
        }
        Command::Render { output, codec } => {
            require_entry(state, &project)?;
            let codec = codec
                .map(|c| match c.as_str() {
                    "h264" => Ok(RenderVideoCodec::H264),
                    "hevc" => Ok(RenderVideoCodec::Hevc),
                    "qtrle" => Ok(RenderVideoCodec::Qtrle),
                    "prores4444" => Ok(RenderVideoCodec::Prores4444),
                    _ => bail!("unknown video codec: {c}"),
                })
                .transpose()?;
            super::render::render(
                &project,
                RenderOutputArgs {
                    output,
                    codec,
                    ..Default::default()
                },
                RenderBudgetArgs::default(),
            )
        }
        Command::Recent | Command::Create { .. } | Command::Open { .. } => unreachable!(),
    }
}

fn export_records(project: &Project) -> Result<Vec<Value>> {
    let mut records = Vec::new();
    for snapshot in project.history()? {
        let directory = project.snapshot_dir(snapshot.number).join("exports");
        if !directory.is_dir() {
            continue;
        }
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "json") {
                records.push(serde_json::from_slice::<Value>(&fs::read(path)?)?);
            }
        }
    }
    records.sort_by_key(|v| std::cmp::Reverse(v["createdAt"].as_u64().unwrap_or(0)));
    Ok(records)
}

pub(crate) fn recent() -> Result<Value> {
    let path = app_home()?.join("recent-projects.json");
    let mut projects = if path.is_file() {
        serde_json::from_slice::<Vec<Value>>(&fs::read(path)?)?
    } else {
        Vec::new()
    };
    for item in &mut projects {
        item["missing"] = json!(
            !Path::new(item["path"].as_str().unwrap_or(""))
                .join("project.json")
                .is_file()
        );
    }
    Ok(json!({"status":"ok","projects":projects,"defaultDirectory":std::env::current_dir()?}))
}

pub(crate) fn open(
    project: &Project,
    runtime_files: BTreeMap<String, HostedFile>,
) -> Result<Value> {
    let host = crate::timeline_file::studio_host(&project.timeline_path(), runtime_files)?;
    let (server, address) = crate::webhost::bind(0)?;
    std::thread::spawn(move || {
        if let Err(error) = crate::webhost::serve_forever(server, host) {
            eprintln!("project Studio stopped: {error:#}");
        }
    });
    Ok(json!({"status":"ok","root":project.root,"url":format!("http://{address}/")}))
}

pub(crate) fn serve_home(web_assets_dir: Option<&Path>, port: u16) -> Result<ExitCode> {
    let runtime = crate::webruntime::resolve(web_assets_dir)?;
    let mut token = [0; 32];
    getrandom::fill(&mut token)?;
    let token = hex::encode(token);
    let config = json!({"home":{"token":token}});
    let state = Arc::new(StudioHost {
        runtime_files: runtime.serving_map(),
        assets_dir: None,
        preview_files: Arc::new(Default::default()),
        motion_source: None,
        source_paths: Mutex::new(Default::default()),
        config_json: Arc::new(RwLock::new(config.to_string())),
        sse: Default::default(),
        capture_dir: None,
        library: Some(LibraryCtx {
            token,
            root: app_home()?.join("assets"),
        }),
        timeline_file: None,
        last_report: RwLock::new(None),
    });
    let (server, address) = crate::webhost::bind(port)?;
    let url = format!("http://{address}/");
    if crate::output::events() {
        crate::events::emit(crate::events::EventKind::Ready {
            url: url.clone(),
            port: address.port(),
            runtime_version: runtime.manifest.runtime_version.clone(),
            runtime_source: runtime.source.as_str().into(),
            project_id: None,
            revision: None,
        });
    } else if crate::output::machine() {
        crate::output::emit(
            json!({"status":"ready","url":url,"port":address.port(),"runtime_version":runtime.manifest.runtime_version,"runtime_source":runtime.source.as_str()}),
        );
        std::io::stdout().flush()?;
    } else {
        eprintln!("project studio: {url} · recent projects (Ctrl-C to stop)");
    }
    crate::webhost::serve_forever(server, state)?;
    Ok(ExitCode::SUCCESS)
}
