//! Analysis orchestration: asset selection, dependency scheduling, resumable jobs, and budget
//! limits. Hold short locks for state updates, never during analyzer execution.

use serde_json::{Value, json};

use crate::assets::addr;
use crate::assets::analysis::{self, Analyzer, ParamsFingerprint};
use crate::assets::clock;
use crate::assets::ctx::Ctx;
use crate::assets::db::Db;
use crate::assets::kind::AssetKind;
use crate::assets::lock;
use crate::assets::report::{AssetsError, Result};
use crate::assets::resolve;

/// Recover stale running jobs by timeout on the local machine.
const RUNNING_TIMEOUT_MS: i64 = 30 * 60 * 1000;

struct Job {
    hash: String,
    analyzer_idx: usize,
    params: Value,
    params_fingerprint: ParamsFingerprint,
}

#[allow(clippy::too_many_arguments)]
pub fn analyze(
    ctx: &Ctx,
    hashes: &[String],
    all: bool,
    kind: Option<AssetKind>,
    tag: Option<&str>,
    with: &[String],
    budget_yuan: Option<f64>,
    force: bool,
) -> Result<Value> {
    if with.is_empty() {
        return Err(AssetsError::bad_query(
            "--with requires at least one analyzer, such as shots or vlm",
        ));
    }
    // Select live assets.
    let targets = select_assets(ctx, hashes, all, kind, tag)?;
    if targets.is_empty() {
        return Err(AssetsError::not_found("no assets match the selectors")
            .with_hint("use `valle assets list` to inspect assets; select with --all, --kind, --tag, or a hash"));
    }

    // Build a deduplicated analyzer plan with registered dependencies first.
    let plan = plan_analyzers(&ctx.analyzers, with)?;

    // Expand jobs, filtering unsupported kinds and cache hits.
    let mut jobs: Vec<Job> = Vec::new();
    let mut cached = 0usize;
    let mut skipped_kind = 0usize;
    for (hash, akind) in &targets {
        for &idx in &plan {
            let a = &ctx.analyzers[idx];
            if !a.accepts(*akind) {
                skipped_kind += 1;
                continue;
            }
            let params = a.default_params();
            let params_fingerprint = analysis::params_fingerprint(&params);
            if !force
                && analysis::cache_hit(&ctx.home, hash, a.name(), a.version(), params_fingerprint)?
            {
                cached += 1;
                continue;
            }
            jobs.push(Job {
                hash: hash.clone(),
                analyzer_idx: idx,
                params,
                params_fingerprint,
            });
        }
    }

    // Register jobs under a short lock, skipping fresh running jobs owned by another invocation.
    let mut queued: Vec<Job> = Vec::new();
    let mut running_elsewhere = 0usize;
    {
        let _lock = lock::acquire(&ctx.home)?;
        let db = Db::open(&ctx.home)?;
        let now = clock::now_millis();
        let stale_cutoff = clock::iso8601(now - RUNNING_TIMEOUT_MS);
        for job in jobs {
            let a = &ctx.analyzers[job.analyzer_idx];
            let fresh_running: bool = db
                .conn
                .query_row(
                    "SELECT count(*) FROM jobs WHERE hash=?1 AND analyzer=?2
                     AND status='running' AND started_at > ?3",
                    rusqlite::params![job.hash, a.name(), stale_cutoff],
                    |r| r.get::<_, i64>(0),
                )
                .map(|n| n > 0)?;
            if fresh_running {
                running_elsewhere += 1;
                continue;
            }
            db.conn.execute(
                "INSERT OR REPLACE INTO jobs (hash, analyzer, status, started_at) VALUES (?1, ?2, 'queued', NULL)",
                rusqlite::params![job.hash, a.name()],
            )?;
            queued.push(job);
        }
    }

    // Execute without the lock, then briefly lock to commit each result.
    let budget_fen = budget_yuan.map(|y| (y * 100.0).round() as i64);
    let mut spent_fen = 0i64;
    let mut spent_ms = 0i64;
    let mut done = 0usize;
    let mut failed: Vec<Value> = Vec::new();
    let mut failed_deps: std::collections::HashSet<(String, String)> = Default::default();
    let total = queued.len();
    let mut over_budget = false;

    for (i, job) in queued.iter().enumerate() {
        let a = &ctx.analyzers[job.analyzer_idx];
        if over_budget {
            mark_job(
                ctx,
                &job.hash,
                a.name(),
                "failed",
                Some("stopped: budget_exceeded"),
            )?;
            continue;
        }
        // Skip dependents when a dependency fails.
        if a.dependencies()
            .iter()
            .any(|d| failed_deps.contains(&(job.hash.clone(), d.to_string())))
        {
            mark_job(
                ctx,
                &job.hash,
                a.name(),
                "failed",
                Some("skipped after dependency failure"),
            )?;
            failed_deps.insert((job.hash.clone(), a.name().to_owned()));
            failed.push(
                json!({"hash": &job.hash[..12], "analyzer": a.name(), "error": "skipped after dependency failure"}),
            );
            continue;
        }
        ctx.report_progress(&format!(
            "[{}/{}] {} ← {}",
            i + 1,
            total,
            a.name(),
            &job.hash[..12]
        ));
        // Mark the job running under a short lock.
        mark_job(ctx, &job.hash, a.name(), "running", None)?;
        // Resolve the path and execute without holding the lock.
        let outcome = resolve::resolve(ctx, &job.hash).and_then(|r| {
            let meta_kind = targets
                .iter()
                .find(|(h, _)| h == &job.hash)
                .map(|(_, k)| *k)
                .expect("asset is in the target set");
            a.analyze(&analysis::AnalyzerInput {
                hash: &job.hash,
                path: std::path::Path::new(&r.path),
                kind: meta_kind,
                params: &job.params,
                home: ctx.home.clone(),
            })
        });
        match outcome {
            Ok(out) => {
                let _lock = lock::acquire(&ctx.home)?;
                debug_assert_eq!(
                    analysis::params_fingerprint(&job.params),
                    job.params_fingerprint
                );
                analysis::write_slot(ctx, &job.hash, a.name(), a.version(), &job.params, &out)?;
                spent_ms += out.cost_ms;
                spent_fen += out.cost_fen;
                done += 1;
                set_job_done(ctx, &job.hash, a.name(), out.cost_ms, out.cost_fen)?;
                if let Some(limit) = budget_fen {
                    if spent_fen > limit {
                        over_budget = true;
                    }
                }
            }
            Err(e) => {
                mark_job(ctx, &job.hash, a.name(), "failed", Some(&e.message))?;
                failed_deps.insert((job.hash.clone(), a.name().to_owned()));
                failed.push(
                    json!({"hash": &job.hash[..12], "analyzer": a.name(), "error": e.message}),
                );
            }
        }
    }

    let summary = json!({
        "assets": targets.len(),
        "done": done,
        "cached": cached,
        "failed": failed,
        "skipped_kind": skipped_kind,
        "running_elsewhere": running_elsewhere,
        "cost": { "ms": spent_ms, "fen": spent_fen },
    });
    if over_budget {
        return Err(AssetsError::budget_exceeded(format!(
            "budget of CNY {:.2} exhausted (spent CNY {:.2}); {done} completed, remaining jobs stopped",
            budget_yuan.unwrap_or(0.0),
            spent_fen as f64 / 100.0
        ))
        .with_hint("completed results are retained; raise the budget or narrow the selection and rerun to reuse cached results"));
    }
    Ok(summary)
}

/// Resolve selectors to asset hashes and kinds.
fn select_assets(
    ctx: &Ctx,
    hashes: &[String],
    all: bool,
    kind: Option<AssetKind>,
    tag: Option<&str>,
) -> Result<Vec<(String, AssetKind)>> {
    if !hashes.is_empty() {
        let mut out = Vec::new();
        for h in hashes {
            let full = addr::find_hash(&ctx.home, h)?;
            let meta = crate::assets::meta::AssetMeta::load(&ctx.home.meta_path(&full))?;
            if meta.is_removed() {
                return Err(
                    AssetsError::not_found(format!("asset {} is removed", &full[..12]))
                        .with_hint("removed assets have no bytes to analyze; add the content again before analysis"),
                );
            }
            out.push((full, meta.kind));
        }
        return Ok(out);
    }
    if !all && kind.is_none() && tag.is_none() {
        return Err(AssetsError::bad_query("no asset selector provided")
            .with_hint("provide a hash, --all, --kind video, or --tag promo"));
    }
    let db = Db::open(&ctx.home)?;
    let mut sql = String::from("SELECT hash, kind FROM assets WHERE removed_at IS NULL");
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(k) = kind {
        sql.push_str(" AND kind = ?");
        params.push(Box::new(k.as_str().to_owned()));
    }
    if let Some(t) = tag {
        sql.push_str(
            " AND EXISTS (SELECT 1 FROM json_each(assets.tags) WHERE json_each.value = ?)",
        );
        params.push(Box::new(t.to_owned()));
    }
    sql.push_str(" ORDER BY added_at, hash");
    let mut stmt = db.conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let rows: Vec<(String, AssetKind)> = stmt
        .query_map(refs.as_slice(), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .filter_map(|r| r.ok())
        .filter_map(|(h, k)| AssetKind::parse(&k).ok().map(|k| (h, k)))
        .collect();
    Ok(rows)
}

/// Plan requested analyzers in dependency order without duplicates.
fn plan_analyzers(analyzers: &[Box<dyn Analyzer>], with: &[String]) -> Result<Vec<usize>> {
    let find = |name: &str| analyzers.iter().position(|a| a.name() == name);
    let mut order: Vec<usize> = Vec::new();
    let visit = |name: &str, order: &mut Vec<usize>| -> Result<()> {
        let idx = find(name).ok_or_else(|| {
            let known: Vec<&str> = analyzers.iter().map(|a| a.name()).collect();
            AssetsError::analyzer_failed(format!("analyzer is not registered: {name}"))
                .with_hint(format!("available: {}", known.join(", ")))
        })?;
        // Schedule registered dependencies first; missing unregistered dependencies fail during
        // execution.
        for dep in analyzers[idx].dependencies() {
            if let Some(di) = find(dep) {
                if !order.contains(&di) {
                    order.push(di);
                }
            }
        }
        if !order.contains(&idx) {
            order.push(idx);
        }
        Ok(())
    };
    for name in with {
        visit(name, &mut order)?;
    }
    Ok(order)
}

/// Persist job status under a short lock.
fn mark_job(
    ctx: &Ctx,
    hash: &str,
    analyzer: &str,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    let _lock = lock::acquire(&ctx.home)?;
    let db = Db::open(&ctx.home)?;
    let now = clock::iso8601(clock::now_millis());
    db.conn.execute(
        "INSERT OR REPLACE INTO jobs (hash, analyzer, status, started_at, finished_at, error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            hash,
            analyzer,
            status,
            if status == "running" {
                Some(now.clone())
            } else {
                None::<String>
            },
            if status == "running" {
                None::<String>
            } else {
                Some(now.clone())
            },
            error
        ],
    )?;
    Ok(())
}

fn set_job_done(ctx: &Ctx, hash: &str, analyzer: &str, cost_ms: i64, cost_fen: i64) -> Result<()> {
    let db = Db::open(&ctx.home)?;
    let now = clock::iso8601(clock::now_millis());
    db.conn.execute(
        "INSERT OR REPLACE INTO jobs (hash, analyzer, status, finished_at, cost_ms, cost_fen)
         VALUES (?1, ?2, 'done', ?3, ?4, ?5)",
        rusqlite::params![hash, analyzer, now, cost_ms, cost_fen],
    )?;
    Ok(())
}
