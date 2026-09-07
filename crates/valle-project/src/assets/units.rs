//! Rebuildable retrieval units for assets, annotations, and semantic segments. Synchronize FTS
//! external content on every insert and delete. Knowledge writes rebuild one asset; reindexing
//! rebuilds all units. Removed assets and orphan knowledge have no projection.

use serde_json::{Value, json};

use crate::assets::analysis::{self, Slot};
use crate::assets::annotate::{self, Annotation};
use crate::assets::db::{Db, walk_meta_tree};
use crate::assets::entity::{self, Entity};
use crate::assets::fts;
use crate::assets::home::Home;
use crate::assets::meta::AssetMeta;
use crate::assets::report::Result;
use crate::assets::vlm::window_transcript;

/// Replace one asset's retrieval units and synchronize FTS.
pub fn rebuild_for_asset(db: &Db, home: &Home, hash: &str) -> Result<()> {
    delete_for_asset(db, hash)?;
    let meta_path = home.meta_path(hash);
    if !meta_path.exists() {
        return Ok(()); // Do not project orphan knowledge left after purging.
    }
    let meta = AssetMeta::load(&meta_path)?;
    if meta.is_removed() {
        return Ok(()); // Removed assets have no retrieval units.
    }
    let annotations = annotate::load_all(home, hash)?;
    let entities = entity::load(home)?;
    let slots = analysis::load_all_slots(home, hash)?;
    insert_units(db, home, &meta, &annotations, &entities, &slots)
}

/// Delete FTS entries before their external-content rows.
pub fn delete_for_asset(db: &Db, hash: &str) -> Result<()> {
    let mut stmt = db.conn.prepare(
        "SELECT unit_id, title, user_text, user_tags, entity_names, transcript, ocr, ai_text
         FROM retrieval_units WHERE hash=?1",
    )?;
    let rows: Vec<(i64, [Option<String>; 7])> = stmt
        .query_map([hash], |r| {
            Ok((
                r.get(0)?,
                [
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ],
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();
    for (rowid, cols) in rows {
        db.conn.execute(
            "INSERT INTO fts(fts, rowid, title, user_text, user_tags, entity_names, transcript, ocr, ai_text)
             VALUES ('delete', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                rowid, cols[0], cols[1], cols[2], cols[3], cols[4], cols[5], cols[6]
            ],
        )?;
    }
    db.conn
        .execute("DELETE FROM retrieval_units WHERE hash=?1", [hash])?;
    Ok(())
}

/// Resolve entity IDs to names and aliases for indexing.
fn entity_name_strings(entities: &[Entity], ids: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for id in ids {
        if let Some(e) = entities.iter().find(|e| &e.id == id) {
            out.push(e.name.clone());
            out.extend(e.aliases.iter().cloned());
        }
    }
    out
}

fn insert_units(
    db: &Db,
    home: &Home,
    meta: &AssetMeta,
    annotations: &[Annotation],
    entities: &[Entity],
    slots: &[Slot],
) -> Result<()> {
    let hash = meta.content_digest.as_hex();
    // Asset unit: title, tags, asset-level annotations, and linked entities.
    let asset_level: Vec<&Annotation> = annotations
        .iter()
        .filter(|a| a.start_ms.is_none() && a.end_ms.is_none())
        .collect();
    let asset_texts: Vec<&str> = asset_level
        .iter()
        .filter_map(|a| a.text.as_deref())
        .collect();
    let mut asset_tags: Vec<String> = meta.tags.clone();
    for a in &asset_level {
        asset_tags.extend(a.tags.iter().cloned());
    }
    let asset_entity_ids: Vec<String> = asset_level
        .iter()
        .flat_map(|a| a.entities.iter().map(|e| e.id.clone()))
        .collect();
    let asset_entity_names = entity_name_strings(entities, &asset_entity_ids);
    insert_unit(
        db,
        &hash,
        "asset",
        None,
        None,
        None,
        meta.title.as_deref(),
        &asset_texts.join("\n"),
        &asset_tags,
        &asset_entity_names,
    )?;

    // Create one unit for each time-anchored annotation.
    for a in annotations {
        if a.start_ms.is_none() && a.end_ms.is_none() {
            continue;
        }
        let ids: Vec<String> = a.entities.iter().map(|e| e.id.clone()).collect();
        let names = entity_name_strings(entities, &ids);
        insert_unit(
            db,
            &hash,
            "annotation",
            a.start_ms,
            a.end_ms,
            Some(&a.id),
            None,
            a.text.as_deref().unwrap_or(""),
            &a.tags,
            &names,
        )?;
    }

    // Use shot windows for transcripts and visual descriptions, or ASR sentence windows when shots
    // are unavailable.
    insert_semantic_units(db, home, meta, slots)?;
    Ok(())
}

/// Combine shot, ASR, and VLM slots into semantic units; audio and unsegmented video use ASR
/// sentences.
fn insert_semantic_units(db: &Db, home: &Home, meta: &AssetMeta, slots: &[Slot]) -> Result<()> {
    let hash = meta.content_digest.as_hex();
    let find = |name: &str| slots.iter().find(|s| s.analyzer == name);
    let shots = find("shots");
    let asr = find("asr");
    let vlm = find("vlm");

    if let Some(shots) = shots {
        let source = format!("shots@{}", shots.version);
        for (i, it) in shots.items.iter().enumerate() {
            let (Some(s), Some(e)) = (
                it.get("start_ms").and_then(|v| v.as_i64()),
                it.get("end_ms").and_then(|v| v.as_i64()),
            ) else {
                continue;
            };
            let transcript = asr
                .map(|slot| window_transcript(&slot.items, s, e))
                .unwrap_or_default();
            // Match VLM items by equal start time or overlapping windows.
            let vitem = vlm.and_then(|slot| {
                slot.items.iter().find(|v| {
                    v.get("start_ms")
                        .and_then(|x| x.as_i64())
                        .map(|vs| vs == s)
                        .unwrap_or(false)
                        || (v
                            .get("start_ms")
                            .and_then(|x| x.as_i64())
                            .unwrap_or(i64::MAX)
                            < e
                            && v.get("end_ms").and_then(|x| x.as_i64()).unwrap_or(i64::MIN) > s)
                })
            });
            let ai_text = vitem.map(vlm_item_text).unwrap_or_default();
            // Attach a representative frame only when it exists in the cache.
            let mid = (s + e) / 2;
            let frame =
                crate::assets::cachefs::cache_path(home, "frames", &hash, &mid.to_string(), "png");
            let rep_frame = frame.exists().then(|| frame.to_string_lossy().into_owned());
            insert_shot_unit(
                db,
                &hash,
                s,
                e,
                &format!("{source}/s{}", i + 1),
                &transcript,
                &ai_text,
                rep_frame.as_deref(),
                vitem,
            )?;
        }
    } else if let Some(asr) = asr {
        // Derive sentence units from word-level ASR for audio or video without shot windows.
        let source = format!("asr@{}", asr.version);
        let words: Vec<crate::assets::asrseg::WordTuple> = asr
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
        let segments = crate::assets::asrseg::sentences(&words, 15.0);
        for (i, it) in segments.iter().enumerate() {
            let (Some(s), Some(e)) = (
                it.get("start_ms").and_then(|v| v.as_i64()),
                it.get("end_ms").and_then(|v| v.as_i64()),
            ) else {
                continue;
            };
            let text = it.get("text").and_then(|v| v.as_str()).unwrap_or("");
            insert_shot_unit(
                db,
                &hash,
                s,
                e,
                &format!("{source}/g{}", i + 1),
                text,
                "",
                None,
                None,
            )?;
        }
    }
    Ok(())
}

/// Combine VLM subjects, events, scenes, summaries, and synonyms into original AI text.
fn vlm_item_text(item: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    for key in ["subjects", "synonyms"] {
        if let Some(arr) = item.get(key).and_then(|v| v.as_array()) {
            parts.extend(arr.iter().filter_map(|x| x.as_str().map(|s| s.to_owned())));
        }
    }
    for key in [
        "event",
        "scene",
        "time_of_day",
        "shot_scale",
        "dominant_color",
        "summary",
    ] {
        if let Some(s) = item.get(key).and_then(|v| v.as_str()) {
            parts.push(s.to_owned());
        }
    }
    parts.join(" ")
}

/// Insert a shot or sentence unit with transcript, AI text, and structured AI filters.
#[allow(clippy::too_many_arguments)]
fn insert_shot_unit(
    db: &Db,
    hash: &str,
    start_ms: i64,
    end_ms: i64,
    source: &str,
    transcript: &str,
    ai_text: &str,
    rep_frame: Option<&str>,
    vlm_item: Option<&Value>,
) -> Result<()> {
    if transcript.is_empty() && ai_text.is_empty() {
        return Ok(()); // Skip empty units.
    }
    let raw = json!({ "transcript": transcript, "ai_text": ai_text });
    let f_transcript = fts::index_text(transcript);
    let f_ai = fts::index_text(ai_text);
    let get_s = |k: &str| vlm_item.and_then(|v| v.get(k)).and_then(|x| x.as_str());
    let get_b = |k: &str| vlm_item.and_then(|v| v.get(k)).and_then(|x| x.as_bool());
    db.conn.execute(
        "INSERT INTO retrieval_units
             (hash, unit_kind, start_ms, end_ms, source, rep_frame,
              transcript, ai_text, raw, shot_scale, empty_broll, negative_space,
              dominant_color, motion_dir)
         VALUES (?1, 'shot', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        rusqlite::params![
            hash,
            start_ms,
            end_ms,
            source,
            rep_frame,
            f_transcript,
            f_ai,
            raw.to_string(),
            get_s("shot_scale"),
            get_b("empty_broll").map(|b| b as i64),
            get_b("negative_space").map(|b| b as i64),
            get_s("dominant_color"),
            get_s("motion_dir"),
        ],
    )?;
    let rowid = db.conn.last_insert_rowid();
    db.conn.execute(
        "INSERT INTO fts(rowid, title, user_text, user_tags, entity_names, transcript, ocr, ai_text)
         VALUES (?1, NULL, NULL, NULL, NULL, ?2, NULL, ?3)",
        rusqlite::params![rowid, f_transcript, f_ai],
    )?;
    Ok(())
}

/// Insert content tokens, original text, and the corresponding FTS entry.
#[allow(clippy::too_many_arguments)]
fn insert_unit(
    db: &Db,
    hash: &str,
    unit_kind: &str,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
    source: Option<&str>,
    title: Option<&str>,
    user_text: &str,
    user_tags: &[String],
    entity_names: &[String],
) -> Result<()> {
    // Skip entirely empty text units. Structured asset-only queries use the assets table.
    let empty = title.map(|t| t.is_empty()).unwrap_or(true)
        && user_text.is_empty()
        && user_tags.is_empty()
        && entity_names.is_empty();
    if empty {
        return Ok(());
    }
    let raw = json!({
        "title": title,
        "user_text": user_text,
        "user_tags": user_tags,
        "entity_names": entity_names,
    });
    let f_title = title.map(fts::index_text);
    let f_text = fts::index_text(user_text);
    let f_tags = fts::index_texts(user_tags.iter().map(|s| s.as_str()));
    let f_names = fts::index_texts(entity_names.iter().map(|s| s.as_str()));
    db.conn.execute(
        "INSERT INTO retrieval_units
             (hash, unit_kind, start_ms, end_ms, source, title, user_text, user_tags,
              entity_names, transcript, ocr, ai_text, raw)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, NULL, NULL, ?10)",
        rusqlite::params![
            hash,
            unit_kind,
            start_ms,
            end_ms,
            source,
            f_title,
            f_text,
            f_tags,
            f_names,
            raw.to_string()
        ],
    )?;
    let rowid = db.conn.last_insert_rowid();
    db.conn.execute(
        "INSERT INTO fts(rowid, title, user_text, user_tags, entity_names, transcript, ocr, ai_text)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![rowid, f_title, f_text, f_tags, f_names, None::<String>, None::<String>, None::<String>],
    )?;
    Ok(())
}

/// Rebuild all projections from authoritative files, including recovery after a schema-version
/// change.
pub fn full_rebuild(db: &mut Db, home: &Home) -> Result<RebuildStats> {
    // Clear retrieval units and FTS.
    db.conn
        .execute("INSERT INTO fts(fts) VALUES('delete-all')", [])?;
    db.conn.execute("DELETE FROM retrieval_units", [])?;
    // Rebuild base tables.
    let assets = db.reindex_assets(home)?;
    let entities = entity::load(home)?;
    entity::project(db, &entities)?;
    db.conn.execute("DELETE FROM annotations", [])?;
    db.conn.execute("DELETE FROM annotation_entities", [])?;
    // Start from metadata so orphan knowledge is excluded.
    let mut units = 0usize;
    let mut annotations_n = 0usize;
    for meta in walk_meta_tree(home)? {
        let hash = meta.content_digest.as_hex();
        let anns = annotate::load_all(home, &hash)?;
        for a in &anns {
            annotate::project_annotation(db, &hash, a)?;
        }
        annotations_n += anns.len();
        // Project analysis slots into analysis and segment tables.
        let slots = analysis::load_all_slots(home, &hash)?;
        for s in &slots {
            analysis::project_slot(db, &hash, s)?;
        }
        if !meta.is_removed() {
            insert_units(db, home, &meta, &anns, &entities, &slots)?;
        }
        units += 1;
    }
    Ok(RebuildStats {
        assets,
        entities: entities.len(),
        annotations: annotations_n,
        scanned: units,
    })
}

#[derive(Debug, serde::Serialize)]
pub struct RebuildStats {
    pub assets: usize,
    pub entities: usize,
    pub annotations: usize,
    /// Number of metadata files scanned, including removed assets.
    pub scanned: usize,
}
