//! Dispatch commands to operations and wrap results in a shared response envelope for CLI and
//! Studio.

use std::path::Path;

use serde_json::json;

use crate::assets::ctx::Ctx;
use crate::assets::report::Report;
use crate::assets::verb::EntityOp;
use crate::assets::verb::Verb;
use crate::assets::{add, annotate, entity, knowledge, maintain, read, resolve, search, sqlgate};

pub fn execute(ctx: &Ctx, verb: Verb) -> Report {
    run(ctx, verb).into()
}

fn run(ctx: &Ctx, verb: Verb) -> crate::assets::report::Result<Report> {
    Ok(match verb {
        Verb::Add {
            path,
            mode,
            kind,
            title,
            tags,
        } => {
            let out = add::add(ctx, Path::new(&path), mode, kind, title.as_deref(), &tags)?;
            let warnings = out.warnings.clone();
            Report::data(serde_json::to_value(&out).unwrap_or_default()).with_warnings(warnings)
        }
        Verb::Rm { hash, purge, force } => {
            let out = maintain::rm(ctx, &hash, purge, force)?;
            Report::data(serde_json::to_value(&out).unwrap_or_default())
        }
        Verb::List {
            kind,
            tag,
            stale,
            removed,
        } => Report::data(read::list(ctx, kind, tag.as_deref(), stale, removed)?),
        Verb::Show { hash } => Report::data(read::show(ctx, &hash)?),
        Verb::Transcript { hash, level } => Report::data(read::transcript(ctx, &hash, &level)?),
        Verb::Describe => Report::data(read::describe(ctx)?),
        Verb::Resolve { hash } => {
            let out = resolve::resolve(ctx, &hash)?;
            Report::data(serde_json::to_value(&out).unwrap_or_default())
        }
        Verb::Verify { deep } => {
            let out = maintain::verify(ctx, deep)?;
            Report::data(serde_json::to_value(&out).unwrap_or_default())
        }
        Verb::Gc => {
            let out = maintain::gc(ctx)?;
            Report::data(serde_json::to_value(&out).unwrap_or_default())
        }
        Verb::Reindex => {
            let out = maintain::reindex(ctx)?;
            Report::data(serde_json::to_value(&out).unwrap_or_default())
        }
        Verb::Sql { query } => {
            let out = sqlgate::sql(ctx, &query)?;
            Report::data(serde_json::to_value(&out).unwrap_or_default())
        }
        Verb::Edit {
            hash,
            title,
            subkind,
        } => Report::data(knowledge::edit(
            ctx,
            &hash,
            title.as_deref(),
            subkind.as_deref(),
        )?),
        Verb::Tag { hash, add, rm } => Report::data(knowledge::tag(ctx, &hash, &add, &rm)?),
        Verb::Annotate {
            hash,
            at,
            range,
            text,
            tags,
            entities,
            id,
            rm,
        } => Report::data(annotate::annotate(
            ctx,
            &hash,
            at,
            range,
            text.as_deref(),
            &tags,
            &entities,
            id.as_deref(),
            rm.as_deref(),
        )?),
        Verb::Entity { op } => match op {
            EntityOp::Add {
                name,
                kind,
                aliases,
            } => Report::data(entity::add(ctx, &name, kind.as_deref(), &aliases)?),
            EntityOp::List => Report::data(entity::list(ctx)?),
            EntityOp::Edit { id, name, aliases } => {
                Report::data(entity::edit(ctx, &id, name.as_deref(), aliases.as_deref())?)
            }
        },
        Verb::Analyze {
            hashes,
            all,
            kind,
            tag,
            with,
            budget,
            force,
        } => Report::data(crate::assets::analyze::analyze(
            ctx,
            &hashes,
            all,
            kind,
            tag.as_deref(),
            &with,
            budget,
            force,
        )?),
        Verb::Search {
            query,
            kind,
            tag,
            entity,
            filter,
            limit,
        } => Report::data(search::search(
            ctx,
            &query,
            kind,
            tag.as_deref(),
            entity.as_deref(),
            filter.as_deref(),
            limit,
        )?),
    })
}

/// Wrap batch ingestion results, retaining one result per file and continuing after individual
/// failures.
pub fn execute_add_batch(
    ctx: &Ctx,
    paths: &[std::path::PathBuf],
    mode: crate::assets::verb::AddMode,
    kind: Option<crate::assets::kind::AssetKind>,
    title: Option<&str>,
    tags: &[String],
) -> Report {
    let items = add::add_batch(ctx, paths, mode, kind, title, tags);
    let failed = items.iter().filter(|i| i.error.is_some()).count();
    let ok = failed == 0;
    let data = json!({
        "items": items.iter().map(|i| serde_json::to_value(i).unwrap_or_default()).collect::<Vec<_>>(),
        "total": items.len(),
        "failed": failed,
    });
    if ok {
        Report::data(data)
    } else {
        let mut r = Report::data(data);
        r.warnings.push(format!(
            "{failed} files failed ingestion; remaining files were processed"
        ));
        r
    }
}
