//! Offline search over FTS projections. Apply structured filters in SQL, expand functional query
//! aliases, and rank human knowledge above AI descriptions. Return evidence from original text
//! instead of bigram tokens.

use serde_json::{Value, json};

use crate::assets::ctx::Ctx;
use crate::assets::db::Db;
use crate::assets::fts;
use crate::assets::kind::AssetKind;
use crate::assets::report::{AssetsError, Result};

/// BM25 weights in FTS column order: title, user text, tags, entities, transcript, OCR, and AI
/// text.
const BM25_WEIGHTS: &str = "5.0, 6.0, 8.0, 10.0, 3.0, 3.0, 1.0";

/// Map functional search terms to structured SQL conditions.
fn functional_condition(term: &str) -> Option<&'static str> {
    match term {
        "空镜" => Some("u.empty_broll = 1"),
        "竖屏" => Some("a.height > a.width"),
        "横屏" => Some("a.width > a.height"),
        _ => None,
    }
}

/// Small synonym groups expanded with OR to reduce vocabulary mismatches. VLM output also supplies
/// synonyms for indexing.
const SYNONYM_GROUPS: &[&[&str]] = &[
    &["打草机", "割草机", "绿篱剪", "草皮修剪机", "除草机"],
    &["无人机", "航拍机"],
    &["手机", "移动电话", "智能机"],
    &["小孩", "孩子", "儿童", "小朋友"],
    &["狗", "小狗", "犬"],
    &["猫", "小猫", "猫咪"],
];

fn synonyms_of(term: &str) -> Option<&'static [&'static str]> {
    SYNONYM_GROUPS.iter().find(|g| g.contains(&term)).copied()
}

pub fn search(
    ctx: &Ctx,
    query: &str,
    kind: Option<AssetKind>,
    tag: Option<&str>,
    entity: Option<&str>,
    filter: Option<&str>,
    limit: Option<usize>,
) -> Result<Value> {
    let db = Db::open(&ctx.home)?;
    let limit = limit.unwrap_or(20).min(200);

    // Extract functional terms before full-text matching.
    let mut conds: Vec<String> = Vec::new();
    let mut text_terms: Vec<&str> = Vec::new();
    for term in query.split_whitespace() {
        match functional_condition(term) {
            Some(c) => conds.push(c.to_owned()),
            None => text_terms.push(term),
        }
    }
    // Expand dictionary terms into parenthesized synonym groups.
    let expr = {
        let mut parts = Vec::new();
        for term in &text_terms {
            match synonyms_of(term) {
                Some(group) => {
                    let alts: Vec<String> =
                        group.iter().filter_map(|g| fts::query_expr(g)).collect();
                    if !alts.is_empty() {
                        parts.push(format!("({})", alts.join(" OR ")));
                    }
                }
                None => {
                    if let Some(e) = fts::query_expr(term) {
                        parts.push(e);
                    }
                }
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" AND "))
        }
    };

    // Apply structured filters.
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(k) = kind {
        conds.push("a.kind = ?".into());
        params.push(Box::new(k.as_str().to_owned()));
    }
    if let Some(t) = tag {
        conds.push("EXISTS (SELECT 1 FROM json_each(a.tags) WHERE json_each.value = ?)".into());
        params.push(Box::new(t.to_owned()));
    }
    if let Some(e) = entity {
        conds.push("u.hash IN (SELECT hash FROM annotation_entities WHERE entity_id = ?)".into());
        params.push(Box::new(e.to_owned()));
    }
    if let Some(f) = filter {
        for pair in f.split(',').filter(|s| !s.trim().is_empty()) {
            let (k, v) = pair
                .split_once('=')
                .ok_or_else(|| AssetsError::bad_query(format!("filter is missing '=': {pair}")))?;
            match k.trim() {
                "orientation" => match v.trim() {
                    "portrait" => conds.push("a.height > a.width".into()),
                    "landscape" => conds.push("a.width > a.height".into()),
                    other => {
                        return Err(AssetsError::bad_query(format!("orientation '{other}'"))
                            .with_hint("portrait|landscape"));
                    }
                },
                "min-dur" => {
                    let s: f64 = v.trim().parse().map_err(|_| {
                        AssetsError::bad_query(format!("min-dur requires numeric seconds: {v}"))
                    })?;
                    conds.push("a.duration_ms >= ?".into());
                    params.push(Box::new((s * 1000.0) as i64));
                }
                "max-dur" => {
                    let s: f64 = v.trim().parse().map_err(|_| {
                        AssetsError::bad_query(format!("max-dur requires numeric seconds: {v}"))
                    })?;
                    conds.push("a.duration_ms <= ?".into());
                    params.push(Box::new((s * 1000.0) as i64));
                }
                other => {
                    return Err(
                        AssetsError::bad_query(format!("unknown filter key '{other}'"))
                            .with_hint("orientation|min-dur|max-dur"),
                    );
                }
            }
        }
    }

    let extra = if conds.is_empty() {
        String::new()
    } else {
        format!(" AND {}", conds.join(" AND "))
    };

    let (sql, has_match) = match &expr {
        Some(_) => (
            format!(
                "SELECT u.hash, u.unit_kind, u.start_ms, u.end_ms, u.source, u.rep_frame, u.raw,
                        bm25(fts, {BM25_WEIGHTS}) AS score,
                        a.kind, a.title, a.original_name, a.location_kind, a.location_path
                 FROM fts
                 JOIN retrieval_units u ON u.unit_id = fts.rowid
                 JOIN assets a ON a.hash = u.hash
                 WHERE fts MATCH ? AND a.removed_at IS NULL{extra}
                 ORDER BY score LIMIT {limit}"
            ),
            true,
        ),
        // When only filters remain, return asset-level results.
        None if !conds.is_empty() || kind.is_some() || tag.is_some() || entity.is_some() => (
            format!(
                "SELECT u.hash, u.unit_kind, u.start_ms, u.end_ms, u.source, u.rep_frame, u.raw,
                        0.0 AS score,
                        a.kind, a.title, a.original_name, a.location_kind, a.location_path
                 FROM retrieval_units u
                 JOIN assets a ON a.hash = u.hash
                 WHERE u.unit_kind = 'asset' AND a.removed_at IS NULL{extra}
                 ORDER BY a.added_at DESC LIMIT {limit}"
            ),
            false,
        ),
        None => {
            return Err(AssetsError::bad_query("empty query").with_hint(
                "provide search terms or at least one --kind, --tag, or --filter condition",
            ));
        }
    };

    let mut stmt = db.conn.prepare(&sql)?;
    let mut bind: Vec<&dyn rusqlite::types::ToSql> = Vec::new();
    let expr_s = expr.clone().unwrap_or_default();
    if has_match {
        bind.push(&expr_s);
    }
    bind.extend(params.iter().map(|b| b.as_ref()));

    let rows: Vec<Hit> = stmt
        .query_map(bind.as_slice(), |r| {
            Ok(Hit {
                hash: r.get(0)?,
                unit_kind: r.get(1)?,
                start_ms: r.get(2)?,
                end_ms: r.get(3)?,
                source: r.get(4)?,
                rep_frame: r.get(5)?,
                raw: r.get(6)?,
                score: r.get(7)?,
                asset_kind: r.get(8)?,
                title: r.get(9)?,
                original_name: r.get(10)?,
                location_kind: r.get(11)?,
                location_path: r.get(12)?,
                merged: 0,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    // Merge adjacent shot hits from the same asset.
    let rows = merge_adjacent(rows);
    let results: Vec<Value> = rows.iter().map(|h| h.to_value(ctx, &text_terms)).collect();
    Ok(json!({ "query": query, "results": results, "count": results.len() }))
}

/// Merge touching shot windows in a sorted linear pass, retaining the better score and combined
/// evidence.
fn merge_adjacent(rows: Vec<Hit>) -> Vec<Hit> {
    let mut out: Vec<Hit> = Vec::new();
    for h in rows {
        if h.unit_kind == "shot" {
            if let Some(prev) = out.iter_mut().find(|p| {
                p.unit_kind == "shot"
                    && p.hash == h.hash
                    && p.end_ms.is_some()
                    && p.end_ms == h.start_ms
            }) {
                prev.end_ms = h.end_ms;
                prev.score = prev.score.min(h.score);
                prev.merged += 1;
                continue;
            }
            if let Some(prev) = out.iter_mut().find(|p| {
                p.unit_kind == "shot"
                    && p.hash == h.hash
                    && p.start_ms.is_some()
                    && p.start_ms == h.end_ms
            }) {
                prev.start_ms = h.start_ms;
                prev.score = prev.score.min(h.score);
                prev.merged += 1;
                continue;
            }
        }
        out.push(h);
    }
    out.sort_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

struct Hit {
    hash: String,
    unit_kind: String,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
    source: Option<String>,
    rep_frame: Option<String>,
    raw: Option<String>,
    score: f64,
    asset_kind: String,
    title: Option<String>,
    original_name: Option<String>,
    location_kind: Option<String>,
    location_path: Option<String>,
    /// Number of additional units merged into this hit; zero means unmerged.
    merged: usize,
}

impl Hit {
    fn to_value(&self, ctx: &Ctx, terms: &[&str]) -> Value {
        // Find query terms in original text and select up to two evidence snippets by field
        // priority.
        let mut evidence = Vec::new();
        if let Some(raw) = self
            .raw
            .as_deref()
            .and_then(|r| serde_json::from_str::<Value>(r).ok())
        {
            for field in [
                "entity_names",
                "user_tags",
                "user_text",
                "title",
                "transcript",
                "ai_text",
            ] {
                if evidence.len() >= 2 {
                    break;
                }
                let texts: Vec<String> = match raw.get(field) {
                    Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
                    Some(Value::Array(a)) => a
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                        .collect(),
                    _ => continue,
                };
                for t in &texts {
                    if let Some(snip) = snippet(t, terms) {
                        evidence.push(json!({ "field": field, "snippet": snip }));
                        break;
                    }
                }
            }
        }
        // Time anchor in seconds.
        let range = match (self.start_ms, self.end_ms) {
            (Some(s), Some(e)) => json!([s as f64 / 1000.0, e as f64 / 1000.0]),
            (Some(s), None) => json!([s as f64 / 1000.0]),
            _ => Value::Null,
        };
        // Resolve CAS paths relative to the library; reference paths remain registered locations.
        let path = match (self.location_kind.as_deref(), self.location_path.as_deref()) {
            (Some("cas"), Some(rel)) => Some(
                ctx.home
                    .assets_dir()
                    .join(rel)
                    .to_string_lossy()
                    .into_owned(),
            ),
            (Some("reference"), Some(p)) => Some(p.to_owned()),
            _ => None,
        };
        json!({
            "asset": self.hash,
            "unit": self.unit_kind,
            "kind": self.asset_kind,
            "title": self.title.clone().or_else(|| self.original_name.clone()),
            "range": range,
            "source": self.source,
            "evidence": evidence,
            "frame": self.rep_frame,
            "path": path,
            "score": self.score,
        })
    }
}

/// Find the first query term in a 15-character context window and mark it with guillemets.
fn snippet(text: &str, terms: &[&str]) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let lower: String = text.to_lowercase();
    for term in terms {
        let t = term.trim_matches(['"', '「', '」', '“', '”']);
        if t.is_empty() {
            continue;
        }
        if let Some(byte_pos) = lower.find(&t.to_lowercase()) {
            let char_pos = text[..byte_pos].chars().count();
            let tlen = t.chars().count();
            let start = char_pos.saturating_sub(15);
            let end = (char_pos + tlen + 15).min(chars.len());
            let mut out = String::new();
            if start > 0 {
                out.push('…');
            }
            out.extend(&chars[start..char_pos]);
            out.push('«');
            out.extend(&chars[char_pos..char_pos + tlen]);
            out.push('»');
            out.extend(&chars[char_pos + tlen..end]);
            if end < chars.len() {
                out.push('…');
            }
            return Some(out);
        }
    }
    None
}
