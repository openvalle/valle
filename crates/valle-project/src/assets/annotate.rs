//! Human annotations at asset, point, or range scope. Store one JSON file per annotation with
//! monotonically increasing per-asset IDs. Commands accept seconds; storage uses integer
//! milliseconds.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::assets::addr;
use crate::assets::clock;
use crate::assets::ctx::Ctx;
use crate::assets::db::Db;
use crate::assets::entity;
use crate::assets::fsutil;
use crate::assets::lock;
use crate::assets::report::{AssetsError, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// Authoritative annotation. No bounds means asset scope, start alone means a marker, and both
/// bounds mean a range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<EntityRef>,
    pub author: String,
    pub created_at: String,
    pub updated_at: String,
}

impl Annotation {
    pub fn load(path: &std::path::Path) -> Result<Annotation> {
        let bytes = std::fs::read(path)?;
        serde_json::from_slice(&bytes)
            .map_err(|e| AssetsError::io(format!("corrupt annotation {}: {e}", path.display())))
    }
}

/// Load an asset's annotations in numeric ID order.
pub fn load_all(home: &crate::assets::home::Home, hash: &str) -> Result<Vec<Annotation>> {
    let dir = home.annotations_dir(hash);
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    let mut files: Vec<_> = std::fs::read_dir(&dir)?.filter_map(|e| e.ok()).collect();
    files.sort_by_key(|e| ann_num(&e.file_name().to_string_lossy()).unwrap_or(u64::MAX));
    for f in files {
        if f.path().extension().and_then(|e| e.to_str()) == Some("json") {
            out.push(Annotation::load(&f.path())?);
        }
    }
    Ok(out)
}

fn ann_num(name: &str) -> Option<u64> {
    name.strip_prefix('n')?.strip_suffix(".json")?.parse().ok()
}

/// Create with no ID, replace with an ID, or delete with `rm`.
#[allow(clippy::too_many_arguments)]
pub fn annotate(
    ctx: &Ctx,
    hash_or_prefix: &str,
    at: Option<f64>,
    range: Option<[f64; 2]>,
    text: Option<&str>,
    tags: &[String],
    entities: &[String],
    id: Option<&str>,
    rm: Option<&str>,
) -> Result<Value> {
    let _lock = lock::acquire(&ctx.home)?;
    let hash = addr::find_hash(&ctx.home, hash_or_prefix)?;
    let dir = ctx.home.annotations_dir(&hash);

    if let Some(n) = rm {
        let path = dir.join(format!("{n}.json"));
        if !path.exists() {
            return Err(AssetsError::not_found(format!(
                "annotation not found: {}/{n}",
                &hash[..8]
            )));
        }
        std::fs::remove_file(&path)?;
        let db = Db::open(&ctx.home)?;
        db.conn.execute(
            "DELETE FROM annotations WHERE hash=?1 AND id=?2",
            [&hash, &n.to_owned()],
        )?;
        db.conn.execute(
            "DELETE FROM annotation_entities WHERE hash=?1 AND annotation_id=?2",
            [&hash, &n.to_owned()],
        )?;
        crate::assets::units::rebuild_for_asset(&db, &ctx.home, &hash)?;
        return Ok(json!({ "hash": hash, "removed": n }));
    }

    // Determine annotation scope.
    let (start_ms, end_ms) = match (at, range) {
        (Some(_), Some(_)) => {
            return Err(AssetsError::bad_query(
                "--at and --range are mutually exclusive",
            ));
        }
        (Some(t), None) => (Some((t * 1000.0).round() as i64), None),
        (None, Some([s, e])) => {
            if e <= s || s < 0.0 {
                return Err(AssetsError::bad_query(format!("invalid range [{s}, {e}]"))
                    .with_hint("require 0 <= start < end, in seconds"));
            }
            (
                Some((s * 1000.0).round() as i64),
                Some((e * 1000.0).round() as i64),
            )
        }
        (None, None) => (None, None), // Asset-level annotation.
    };

    if text.is_none() && tags.is_empty() && entities.is_empty() {
        return Err(AssetsError::bad_query(
            "empty annotation: provide --text, --tag, or --entity",
        ));
    }
    // Entity links are optional, but every supplied entity must exist.
    for eid in entities {
        if !entity::exists(&ctx.home, eid)? {
            return Err(
                AssetsError::not_found(format!("entity not found: {eid}")).with_hint(
                    "create an entity with `valle assets entity add --name ...`, or omit --entity",
                ),
            );
        }
    }
    let entity_refs: Vec<EntityRef> = entities
        .iter()
        .map(|e| EntityRef {
            id: e.clone(),
            role: None,
        })
        .collect();

    let now = clock::iso8601(clock::now_millis());
    let ann = match id {
        // Replace values while preserving creation time and author.
        Some(n) => {
            let path = dir.join(format!("{n}.json"));
            if !path.exists() {
                return Err(AssetsError::not_found(format!(
                    "annotation not found: {}/{n}",
                    &hash[..8]
                ))
                .with_hint(
                    "omit --id to create an annotation; --id updates an existing annotation",
                ));
            }
            let old = Annotation::load(&path)?;
            Annotation {
                id: n.to_owned(),
                start_ms,
                end_ms,
                text: text.map(|s| s.to_owned()),
                tags: tags.to_vec(),
                entities: entity_refs,
                author: old.author,
                created_at: old.created_at,
                updated_at: now,
            }
        }
        None => {
            let max = if dir.exists() {
                std::fs::read_dir(&dir)?
                    .filter_map(|e| e.ok())
                    .filter_map(|e| ann_num(&e.file_name().to_string_lossy()))
                    .max()
                    .unwrap_or(0)
            } else {
                0
            };
            Annotation {
                id: format!("n{}", max + 1),
                start_ms,
                end_ms,
                text: text.map(|s| s.to_owned()),
                tags: tags.to_vec(),
                entities: entity_refs,
                author: ctx.actor.clone(),
                created_at: now.clone(),
                updated_at: now,
            }
        }
    };

    let bytes = serde_json::to_vec_pretty(&ann)
        .map_err(|e| AssetsError::io(format!("failed to serialize annotation: {e}")))?;
    fsutil::write_atomic(&dir.join(format!("{}.json", ann.id)), &bytes, "annotation")?;
    let db = Db::open(&ctx.home)?;
    project_annotation(&db, &hash, &ann)?;
    crate::assets::units::rebuild_for_asset(&db, &ctx.home, &hash)?;
    Ok(json!({ "hash": hash, "annotation": serde_json::to_value(&ann).unwrap_or_default() }))
}

/// Project one annotation and its entity links.
pub fn project_annotation(db: &Db, hash: &str, ann: &Annotation) -> Result<()> {
    db.conn.execute(
        "INSERT INTO annotations (hash, id, start_ms, end_ms, text, tags, author, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(hash, id) DO UPDATE SET
             start_ms=?3, end_ms=?4, text=?5, tags=?6, author=?7, created_at=?8, updated_at=?9",
        rusqlite::params![
            hash,
            ann.id,
            ann.start_ms,
            ann.end_ms,
            ann.text,
            serde_json::to_string(&ann.tags).unwrap_or_else(|_| "[]".into()),
            ann.author,
            ann.created_at,
            ann.updated_at,
        ],
    )?;
    db.conn.execute(
        "DELETE FROM annotation_entities WHERE hash=?1 AND annotation_id=?2",
        [hash, &ann.id],
    )?;
    for e in &ann.entities {
        db.conn.execute(
            "INSERT OR REPLACE INTO annotation_entities (hash, annotation_id, entity_id, role)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![hash, ann.id, e.id, e.role],
        )?;
    }
    Ok(())
}
