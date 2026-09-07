//! Read operations for listing, asset details, and statistics. Readers use projections without
//! locks; reindexing repairs stale projections.

use serde_json::{Value, json};

use crate::assets::addr;
use crate::assets::ctx::Ctx;
use crate::assets::db::Db;
use crate::assets::kind::AssetKind;
use crate::assets::meta::AssetMeta;
use crate::assets::report::Result;

/// List live assets by default, or select removed assets and stale references through filters.
pub fn list(
    ctx: &Ctx,
    kind: Option<AssetKind>,
    tag: Option<&str>,
    stale: bool,
    removed: bool,
) -> Result<Value> {
    let db = Db::open(&ctx.home)?;
    let mut sql = String::from(
        "SELECT hash, kind, subkind, title, original_name, tags, added_at, removed_at,
                location_kind, stale, duration_ms, width, height
         FROM assets WHERE 1=1",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if removed {
        sql.push_str(" AND removed_at IS NOT NULL");
    } else {
        sql.push_str(" AND removed_at IS NULL");
    }
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
    if stale {
        sql.push_str(" AND stale = 1");
    }
    sql.push_str(" ORDER BY added_at DESC, hash");

    let mut stmt = db.conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let rows: Vec<Value> = stmt
        .query_map(refs.as_slice(), |r| {
            let tags_json: String = r.get(5)?;
            Ok(json!({
                "hash": r.get::<_, String>(0)?,
                "kind": r.get::<_, String>(1)?,
                "subkind": r.get::<_, Option<String>>(2)?,
                "title": r.get::<_, Option<String>>(3)?,
                "original_name": r.get::<_, String>(4)?,
                "tags": serde_json::from_str::<Value>(&tags_json).unwrap_or(json!([])),
                "added_at": r.get::<_, String>(6)?,
                "removed_at": r.get::<_, Option<String>>(7)?,
                "location": r.get::<_, Option<String>>(8)?,
                "stale": r.get::<_, i64>(9)? == 1,
                "duration_ms": r.get::<_, Option<i64>>(10)?,
                "width": r.get::<_, Option<i64>>(11)?,
                "height": r.get::<_, Option<i64>>(12)?,
            }))
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(json!({ "assets": rows, "count": rows.len() }))
}

/// Derive word or sentence transcripts from the stored word-level ASR slot without rerunning a
/// model or writing new data.
pub fn transcript(ctx: &Ctx, hash_or_prefix: &str, level: &str) -> Result<Value> {
    if !matches!(level, "word" | "sentence") {
        return Err(
            crate::assets::AssetsError::bad_query(format!("unknown level {level:?}"))
                .with_hint("use --level word or sentence"),
        );
    }
    let hash = addr::find_hash(&ctx.home, hash_or_prefix)?;
    let slot =
        crate::assets::analysis::load_slot(&ctx.home, &hash, "asr", 1)?.ok_or_else(|| {
            crate::assets::AssetsError::not_found(format!("{hash} has no ASR analysis slot"))
                .with_hint(
                    "this command reads existing asr@1 slots; create new transcripts with `valle media transcribe -i <input> -o <output.words.json>`",
                )
        })?;

    let segs: Vec<Value> = if level == "sentence" {
        let words: Vec<crate::assets::asrseg::WordTuple> = slot
            .items
            .iter()
            .filter_map(|it| {
                Some((
                    it.get("text").and_then(|v| v.as_str())?.to_owned(),
                    it.get("start_ms").and_then(|v| v.as_i64())? as f64 / 1000.0,
                    it.get("end_ms").and_then(|v| v.as_i64())? as f64 / 1000.0,
                ))
            })
            .collect();
        crate::assets::asrseg::sentences(&words, 15.0)
            .iter()
            .map(|s| {
                json!({
                    "text": s.get("text").and_then(|v| v.as_str()).unwrap_or(""),
                    "start": s.get("start_ms").and_then(|v| v.as_i64()).unwrap_or(0) as f64 / 1000.0,
                    "end": s.get("end_ms").and_then(|v| v.as_i64()).unwrap_or(0) as f64 / 1000.0,
                })
            })
            .collect()
    } else {
        slot.items
            .iter()
            .map(|it| {
                json!({
                    "text": it.get("text").and_then(|v| v.as_str()).unwrap_or(""),
                    "start": it.get("start_ms").and_then(|v| v.as_i64()).unwrap_or(0) as f64 / 1000.0,
                    "end": it.get("end_ms").and_then(|v| v.as_i64()).unwrap_or(0) as f64 / 1000.0,
                })
            })
            .collect()
    };

    // Join sentences with newlines and word-level text directly.
    let joiner = if level == "sentence" { "\n" } else { "" };
    let text: String = segs
        .iter()
        .filter_map(|s| s.get("text").and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join(joiner);

    Ok(json!({
        "hash": hash,
        "level": level,
        "count": segs.len(),
        "segments": segs,
        "text": text,
    }))
}

/// Asset knowledge card: metadata, analysis slots, and annotations.
pub fn show(ctx: &Ctx, hash_or_prefix: &str) -> Result<Value> {
    let hash = addr::find_hash(&ctx.home, hash_or_prefix)?;
    let meta = AssetMeta::load(&ctx.home.meta_path(&hash))?;
    let db = Db::open(&ctx.home)?;

    // List indexed analysis status; slot files remain authoritative.
    let mut stmt = db.conn.prepare(
        "SELECT analyzer, version, item_count, cost_ms, cost_fen, finished_at
         FROM analysis WHERE hash=?1 ORDER BY analyzer, version",
    )?;
    let analysis: Vec<Value> = stmt
        .query_map([&hash], |r| {
            Ok(json!({
                "analyzer": r.get::<_, String>(0)?,
                "version": r.get::<_, i64>(1)?,
                "items": r.get::<_, i64>(2)?,
                "cost_ms": r.get::<_, i64>(3)?,
                "cost_fen": r.get::<_, i64>(4)?,
                "finished_at": r.get::<_, String>(5)?,
            }))
        })?
        .filter_map(|r| r.ok())
        .collect();

    // Read complete annotations directly from their files.
    let mut annotations = Vec::new();
    let ann_dir = ctx.home.annotations_dir(&hash);
    if ann_dir.exists() {
        let mut files: Vec<_> = std::fs::read_dir(&ann_dir)?
            .filter_map(|e| e.ok())
            .collect();
        files.sort_by_key(|e| e.file_name());
        for f in files {
            if let Ok(bytes) = std::fs::read(f.path()) {
                if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                    annotations.push(v);
                }
            }
        }
    }

    Ok(json!({
        "meta": serde_json::to_value(&meta).unwrap_or_default(),
        "analysis": analysis,
        "annotations": annotations,
    }))
}

/// Library statistics: counts, kinds, removed and stale assets, analysis coverage, and accumulated
/// cost.
pub fn describe(ctx: &Ctx) -> Result<Value> {
    let db = Db::open(&ctx.home)?;
    let total: i64 = db.conn.query_row(
        "SELECT count(*) FROM assets WHERE removed_at IS NULL",
        [],
        |r| r.get(0),
    )?;
    let removed: i64 = db.conn.query_row(
        "SELECT count(*) FROM assets WHERE removed_at IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    let stale: i64 = db
        .conn
        .query_row("SELECT count(*) FROM assets WHERE stale=1", [], |r| {
            r.get(0)
        })?;

    let mut stmt = db.conn.prepare(
        "SELECT kind, count(*) FROM assets WHERE removed_at IS NULL GROUP BY kind ORDER BY kind",
    )?;
    let by_kind: serde_json::Map<String, Value> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .filter_map(|r| r.ok())
        .map(|(k, n)| (k, Value::from(n)))
        .collect();

    // Coverage and accumulated cost by analyzer.
    let mut stmt = db.conn.prepare(
        "SELECT analyzer, count(DISTINCT hash), sum(cost_ms), sum(cost_fen)
         FROM analysis GROUP BY analyzer ORDER BY analyzer",
    )?;
    let analyzers: Vec<Value> = stmt
        .query_map([], |r| {
            Ok(json!({
                "analyzer": r.get::<_, String>(0)?,
                "assets": r.get::<_, i64>(1)?,
                "cost_ms": r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                "cost_fen": r.get::<_, Option<i64>>(3)?.unwrap_or(0),
            }))
        })?
        .filter_map(|r| r.ok())
        .collect();

    let entities: i64 = db
        .conn
        .query_row("SELECT count(*) FROM entities", [], |r| r.get(0))?;
    let annotations: i64 = db
        .conn
        .query_row("SELECT count(*) FROM annotations", [], |r| r.get(0))?;

    Ok(json!({
        "assets": total,
        "removed": removed,
        "stale": stale,
        "by_kind": by_kind,
        "analyzers": analyzers,
        "entities": entities,
        "annotations": annotations,
        "home": ctx.home.assets_dir().to_string_lossy(),
    }))
}
