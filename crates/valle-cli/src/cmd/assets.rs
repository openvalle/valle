//! Thin CLI and Studio adapters for the shared library operations.
use crate::AssetsAction;
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};
use valle_project::assets::{Command, Context, Inspector, Kind};

pub(crate) fn studio_context(root: PathBuf) -> Context {
    Context {
        root,
        inspector: Box::new(CliInspector),
    }
}

pub(crate) fn run(_json: bool, _events: bool, action: AssetsAction) -> Result<ExitCode> {
    let command = match action {
        AssetsAction::Add {
            paths,
            to,
            name,
            tags,
            note,
        } => Command::Add {
            paths,
            to,
            name,
            tags,
            note,
        },
        AssetsAction::List {
            directory,
            kind,
            tag,
            missing,
        } => Command::List {
            directory,
            kind,
            tag,
            missing,
        },
        AssetsAction::Search {
            query,
            kind,
            directory,
            tag,
            filter,
            limit,
        } => Command::Search {
            query,
            kind,
            directory,
            tag,
            filter,
            limit,
        },
        AssetsAction::Show { path } => Command::Show { path },
        AssetsAction::Edit { path, name, to } => Command::Edit { path, name, to },
        AssetsAction::Tag { path, add, rm } => Command::Tag { path, add, rm },
        AssetsAction::Annotate {
            path,
            text,
            at,
            range,
            id,
            rm,
        } => {
            let range = range
                .map(|v| {
                    v.try_into()
                        .map_err(|_| anyhow::anyhow!("--range requires START,END"))
                })
                .transpose()?;
            Command::Annotate {
                path,
                text,
                at,
                range,
                id,
                rm,
            }
        }
        AssetsAction::Remove { path, missing } => Command::Remove { path, missing },
        AssetsAction::Recover { backup } => Command::Recover { backup },
        AssetsAction::Sql { query } => Command::Sql { query },
    };
    let ctx = Context::resolve(Box::new(CliInspector))?;
    let report = ctx.execute(command);
    let success = report["status"] == "ok";
    if crate::output::machine() {
        crate::output::emit(report);
    } else {
        human(&report);
    }
    Ok(if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn human(report: &Value) {
    if let Some(items) = report["items"].as_array() {
        if items.is_empty() {
            println!("(no matches)");
        }
        for item in items {
            if item["status"] == "error" {
                eprintln!(
                    "error: {}: {}",
                    item["input"].as_str().unwrap_or(""),
                    item["error"]["message"].as_str().unwrap_or("import failed")
                );
            } else {
                println!(
                    "{}\t{}\t{}",
                    item["path"].as_str().unwrap_or(""),
                    item["kind"].as_str().unwrap_or(""),
                    item["absolutePath"].as_str().unwrap_or("")
                );
            }
        }
    } else if report["status"] == "error" {
        eprintln!(
            "error: {}",
            report["error"]["message"]
                .as_str()
                .unwrap_or("asset command failed")
        );
    } else {
        println!("{}", serde_json::to_string_pretty(report).unwrap());
    }
}

pub(crate) struct CliInspector;
impl Inspector for CliInspector {
    fn validate(&self, path: &Path) -> Result<()> {
        super::project::validate_component(path)
    }
    fn inspect(&self, path: &Path, kind: Kind) -> Value {
        if kind == Kind::Font {
            font_info(path).unwrap_or(Value::Null)
        } else {
            super::project::probe(path, kind)
        }
    }
}

fn font_info(path: &Path) -> Result<Value> {
    let bytes = std::fs::read(path)?;
    let face = ttf_parser::Face::parse(&bytes, 0).map_err(|e| anyhow::anyhow!("{e}"))?;
    let family = face
        .names()
        .into_iter()
        .filter(|n| n.name_id == ttf_parser::name_id::FAMILY && n.is_unicode())
        .find_map(|n| n.to_string());
    if family.is_none() {
        bail!("font has no family name");
    }
    Ok(
        json!({"family":family,"weight":face.weight().to_number(),"italic":face.is_italic(),"glyphs":face.number_of_glyphs(),"cjk":(['中','文','的','一'].iter().all(|c|face.glyph_index(*c).is_some()))}),
    )
}
