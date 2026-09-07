//! Analyzer contracts and cached result slots. Slots use
//! `analysis/<hash>/<analyzer>@<version>.json`; a parameter fingerprint determines cache hits, with
//! the latest parameters replacing the same slot. Forced analysis bypasses the cache. Results carry
//! optional time anchors and describe observed facts.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Value;

use crate::assets::clock;
use crate::assets::ctx::Ctx;
use crate::assets::db::Db;
use crate::assets::fsutil;
use crate::assets::home::Home;
use crate::assets::kind::AssetKind;
use crate::assets::report::{AssetsError, Result};

/// Analysis input with a resolved absolute path. The home directory provides dependency slots and
/// derived caches.
pub struct AnalyzerInput<'a> {
    pub hash: &'a str,
    pub path: &'a Path,
    pub kind: AssetKind,
    pub params: &'a Value,
    pub home: Home,
}

/// Analysis items with time anchors and cost in local milliseconds or API cents.
#[derive(Debug, Clone, Default)]
pub struct AnalyzerOutput {
    pub items: Vec<Value>,
    pub cost_ms: i64,
    pub cost_fen: i64,
}

/// Analyzer interface, implemented by in-process providers or an external-process adapter.
pub trait Analyzer {
    fn name(&self) -> &'static str;
    fn version(&self) -> u32;
    /// Whether the analyzer supports this asset kind.
    fn accepts(&self, kind: AssetKind) -> bool;
    /// Declared analyzer dependencies, scheduled before the dependent analysis.
    fn dependencies(&self) -> &'static [&'static str] {
        &[]
    }
    /// Default parameters, merged with caller overrides before fingerprinting.
    fn default_params(&self) -> Value {
        Value::Object(Default::default())
    }
    fn analyze(&self, input: &AnalyzerInput<'_>) -> Result<AnalyzerOutput>;
}

/// Authoritative analysis slot content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slot {
    pub analyzer: String,
    pub version: u32,
    pub params_fingerprint: ParamsFingerprint,
    pub params: Value,
    pub items: Vec<Value>,
    pub cost: SlotCost,
    pub finished_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotCost {
    pub ms: i64,
    pub fen: i64,
}

/// Private 64-bit cache discriminator for analyzer parameters. It is not a content digest or
/// integrity proof; a distinct type prevents mixing it with [`crate::ContentDigest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParamsFingerprint([u8; 8]);

impl ParamsFingerprint {
    fn from_bytes(bytes: [u8; 8]) -> Self {
        Self(bytes)
    }

    /// Stable 16-character lowercase hexadecimal representation for SQLite and slot JSON.
    pub fn to_storage(self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Display for ParamsFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_storage())
    }
}

impl Serialize for ParamsFingerprint {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_storage())
    }
}

impl<'de> Deserialize<'de> for ParamsFingerprint {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.len() != 16
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(de::Error::custom(
                "params fingerprint must be 16 lowercase hexadecimal digits",
            ));
        }
        let mut bytes = [0_u8; 8];
        hex::decode_to_slice(&value, &mut bytes).map_err(de::Error::custom)?;
        Ok(Self(bytes))
    }
}

/// Fingerprint canonical parameters using the first eight SHA-256 bytes.
pub fn params_fingerprint(params: &Value) -> ParamsFingerprint {
    use sha2::{Digest, Sha256};
    let canon = canonicalize(params);
    let bytes = serde_json::to_vec(&canon).unwrap_or_default();
    let digest = Sha256::digest(&bytes);
    let mut fingerprint = [0_u8; 8];
    fingerprint.copy_from_slice(&digest[..8]);
    ParamsFingerprint::from_bytes(fingerprint)
}

/// Sort object keys so their input order does not affect the fingerprint.
fn canonicalize(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let sorted: std::collections::BTreeMap<String, Value> = m
                .iter()
                .map(|(k, x)| (k.clone(), canonicalize(x)))
                .collect();
            serde_json::to_value(sorted).unwrap_or_default()
        }
        Value::Array(a) => Value::Array(a.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}

/// Path to an analysis slot.
pub fn slot_path(home: &Home, hash: &str, analyzer: &str, version: u32) -> std::path::PathBuf {
    home.analysis_dir(hash)
        .join(format!("{analyzer}@{version}.json"))
}

/// Read a slot: missing files return `None`, corrupt files return an error.
pub fn load_slot(home: &Home, hash: &str, analyzer: &str, version: u32) -> Result<Option<Slot>> {
    let p = slot_path(home, hash, analyzer, version);
    if !p.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&p)?;
    let slot = serde_json::from_slice(&bytes)
        .map_err(|e| AssetsError::io(format!("corrupt analysis slot {}: {e}", p.display())))?;
    Ok(Some(slot))
}

/// A cache hit requires the same slot and parameter fingerprint.
pub fn cache_hit(
    home: &Home,
    hash: &str,
    analyzer: &str,
    version: u32,
    params_fingerprint: ParamsFingerprint,
) -> Result<bool> {
    Ok(load_slot(home, hash, analyzer, version)?
        .map(|slot| slot.params_fingerprint == params_fingerprint)
        .unwrap_or(false))
}

/// Atomically replace the slot, upsert analysis, and refresh segment projections.
pub fn write_slot(
    ctx: &Ctx,
    hash: &str,
    analyzer: &str,
    version: u32,
    params: &Value,
    out: &AnalyzerOutput,
) -> Result<Slot> {
    let slot = Slot {
        analyzer: analyzer.to_owned(),
        version,
        params_fingerprint: params_fingerprint(params),
        params: params.clone(),
        items: out.items.clone(),
        cost: SlotCost {
            ms: out.cost_ms,
            fen: out.cost_fen,
        },
        finished_at: clock::iso8601(clock::now_millis()),
    };
    let bytes = serde_json::to_vec_pretty(&slot)
        .map_err(|e| AssetsError::io(format!("failed to serialize analysis slot: {e}")))?;
    fsutil::write_atomic(
        &slot_path(&ctx.home, hash, analyzer, version),
        &bytes,
        "analysis",
    )?;
    let db = Db::open(&ctx.home)?;
    project_slot(&db, hash, &slot)?;
    // Refresh semantic search units from the new analysis.
    crate::assets::units::rebuild_for_asset(&db, &ctx.home, hash)?;
    Ok(slot)
}

/// Project a slot into the analysis table and shot segments.
pub fn project_slot(db: &Db, hash: &str, slot: &Slot) -> Result<()> {
    db.conn.execute(
        "INSERT INTO analysis (hash, analyzer, version, params_fingerprint, cost_ms, cost_fen, item_count, finished_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(hash, analyzer, version) DO UPDATE SET
             params_fingerprint=?4, cost_ms=?5, cost_fen=?6, item_count=?7, finished_at=?8",
        rusqlite::params![
            hash,
            slot.analyzer,
            slot.version,
            slot.params_fingerprint.to_storage(),
            slot.cost.ms,
            slot.cost.fen,
            slot.items.len() as i64,
            slot.finished_at,
        ],
    )?;
    if slot.analyzer == "shots" {
        project_segments(db, hash, slot)?;
    }
    Ok(())
}

/// Project shot analysis into virtual segments at the shot level.
fn project_segments(db: &Db, hash: &str, slot: &Slot) -> Result<()> {
    db.conn
        .execute("DELETE FROM segments WHERE hash=?1", [hash])?;
    let source = format!("{}@{}", slot.analyzer, slot.version);
    for (i, item) in slot.items.iter().enumerate() {
        let (Some(s), Some(e)) = (
            item.get("start_ms").and_then(|v| v.as_i64()),
            item.get("end_ms").and_then(|v| v.as_i64()),
        ) else {
            continue;
        };
        db.conn.execute(
            "INSERT INTO segments (hash, id, level, start_ms, end_ms, source)
             VALUES (?1, ?2, 'shot', ?3, ?4, ?5)",
            rusqlite::params![hash, format!("s{}", i + 1), s, e, source],
        )?;
    }
    Ok(())
}

/// Load all analysis slots for an asset, for display or projection rebuilding.
pub fn load_all_slots(home: &Home, hash: &str) -> Result<Vec<Slot>> {
    let dir = home.analysis_dir(hash);
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    let mut files: Vec<_> = std::fs::read_dir(&dir)?.filter_map(|e| e.ok()).collect();
    files.sort_by_key(|e| e.file_name());
    for f in files {
        let p = f.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(slot) = serde_json::from_slice::<Slot>(&std::fs::read(&p)?) {
            out.push(slot);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_fingerprint_key_order_stable() {
        let a = serde_json::json!({"b": 1, "a": {"y": 2, "x": 3}});
        let b = serde_json::json!({"a": {"x": 3, "y": 2}, "b": 1});
        assert_eq!(params_fingerprint(&a), params_fingerprint(&b));
        assert_ne!(
            params_fingerprint(&a),
            params_fingerprint(&serde_json::json!({"b": 2}))
        );
    }

    #[test]
    fn params_fingerprint_has_a_distinct_typed_json_contract() {
        let fingerprint = params_fingerprint(&serde_json::json!({"threshold": 27}));
        let encoded = serde_json::to_string(&fingerprint).unwrap();
        assert_eq!(encoded.len(), 18, "quoted 16-digit storage form");
        assert_eq!(
            serde_json::from_str::<ParamsFingerprint>(&encoded).unwrap(),
            fingerprint
        );
        assert!(serde_json::from_str::<ParamsFingerprint>(r#""sha256:abcd""#).is_err());
    }
}
