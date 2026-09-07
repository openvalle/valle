//! Rebuildable SQLite projections for assets, full-text search, and analysis. Removed assets have
//! no retrieval units; orphan knowledge without metadata is skipped. Jobs hold resumable
//! coordination state rather than a source projection.

use rusqlite::Connection;

use crate::assets::home::Home;
use crate::assets::meta::{AssetMeta, LocationKind};
use crate::assets::report::{AssetsError, Result};

/// Schema version. A mismatch rebuilds the disposable database instead of migrating it.
pub const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);

-- Metadata projection, retaining removed rows for explicit filtering.
CREATE TABLE IF NOT EXISTS assets (
  hash          TEXT PRIMARY KEY,
  kind          TEXT NOT NULL,
  subkind       TEXT,
  size          INTEGER NOT NULL,
  original_name TEXT NOT NULL DEFAULT '',
  title         TEXT,
  tags          TEXT NOT NULL DEFAULT '[]',   -- JSON array filtered with json_each.
  added_at      TEXT NOT NULL,
  removed_at    TEXT,
  location_kind TEXT,                          -- cas | reference | NULL(removed)
  location_path TEXT,
  stale         INTEGER NOT NULL DEFAULT 0,    -- Reference validation state, followed by structured probe filters.
  duration_ms   INTEGER,
  width         INTEGER,
  height        INTEGER,
  fps           REAL
);

-- Analysis coverage and cost projection; slot files are authoritative.
CREATE TABLE IF NOT EXISTS analysis (
  hash        TEXT NOT NULL,
  analyzer    TEXT NOT NULL,
  version     INTEGER NOT NULL,
  params_fingerprint TEXT NOT NULL DEFAULT '',
  cost_ms     INTEGER NOT NULL DEFAULT 0,
  cost_fen    INTEGER NOT NULL DEFAULT 0,
  item_count  INTEGER NOT NULL DEFAULT 0,
  finished_at TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (hash, analyzer, version)
);

-- Virtual shot segments projected from shot analysis.
CREATE TABLE IF NOT EXISTS segments (
  hash     TEXT NOT NULL,
  id       TEXT NOT NULL,
  level    TEXT NOT NULL,
  start_ms INTEGER NOT NULL,
  end_ms   INTEGER NOT NULL,
  source   TEXT NOT NULL,
  PRIMARY KEY (hash, id)
);

-- Annotation projection.
CREATE TABLE IF NOT EXISTS annotations (
  hash       TEXT NOT NULL,
  id         TEXT NOT NULL,
  start_ms   INTEGER,
  end_ms     INTEGER,
  text       TEXT,
  tags       TEXT NOT NULL DEFAULT '[]',
  author     TEXT,
  created_at TEXT,
  updated_at TEXT,
  PRIMARY KEY (hash, id)
);

-- Entity projection.
CREATE TABLE IF NOT EXISTS entities (
  id      TEXT PRIMARY KEY,
  kind    TEXT,
  name    TEXT NOT NULL,
  aliases TEXT NOT NULL DEFAULT '[]'
);

CREATE TABLE IF NOT EXISTS annotation_entities (
  hash          TEXT NOT NULL,
  annotation_id TEXT NOT NULL,
  entity_id     TEXT NOT NULL,
  role          TEXT,
  PRIMARY KEY (hash, annotation_id, entity_id)
);

-- Asset, shot, and annotation retrieval units used as FTS external content.
CREATE TABLE IF NOT EXISTS retrieval_units (
  unit_id      INTEGER PRIMARY KEY,
  hash         TEXT NOT NULL,
  unit_kind    TEXT NOT NULL,
  start_ms     INTEGER,
  end_ms       INTEGER,
  rep_frame    TEXT,
  source       TEXT,
  -- Seven bigram token columns; original display text is stored in raw.
  title        TEXT,
  user_text    TEXT,
  user_tags    TEXT,
  entity_names TEXT,
  transcript   TEXT,
  ocr          TEXT,
  ai_text      TEXT,
  -- Original JSON text used for evidence snippets.
  raw          TEXT,
  -- Structured AI filter fields.
  shot_scale     TEXT,
  empty_broll    INTEGER,
  negative_space INTEGER,
  dominant_color TEXT,
  motion_dir     TEXT
);
CREATE INDEX IF NOT EXISTS idx_units_hash ON retrieval_units(hash);

CREATE VIRTUAL TABLE IF NOT EXISTS fts USING fts5(
  title, user_text, user_tags, entity_names, transcript, ocr, ai_text,
  content='retrieval_units', content_rowid='unit_id',
  tokenize='unicode61'
);

-- Resumable analysis coordination state.
CREATE TABLE IF NOT EXISTS jobs (
  hash        TEXT NOT NULL,
  analyzer    TEXT NOT NULL,
  status      TEXT NOT NULL,        -- queued | running | done | failed
  started_at  TEXT,
  finished_at TEXT,
  error       TEXT,
  cost_ms     INTEGER NOT NULL DEFAULT 0,
  cost_fen    INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (hash, analyzer)
);
"#;

/// Open or create index.db with WAL, a busy timeout, and idempotent schema setup.
pub struct Db {
    pub conn: Connection,
}

impl Db {
    pub fn open(home: &Home) -> Result<Db> {
        let path = home.index_db_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Serialize database creation and WAL setup to prevent concurrent first-open races. The
        // initialization lock is separate from the write lock.
        let _init = crate::assets::lock::acquire_path(
            &home.init_lock_path(),
            "index.db initialization lock",
        )?;
        let conn = Connection::open(&path)?;
        let nuked = init_conn(&conn)?;
        let mut db = Db { conn };
        if nuked {
            // Immediately restore projections from authoritative files after a schema rebuild.
            crate::assets::units::full_rebuild(&mut db, home)?;
        }
        Ok(db)
    }

    /// Open an existing database read-only for SQL queries.
    pub fn open_readonly(home: &Home) -> Result<Db> {
        use rusqlite::OpenFlags;
        let path = home.index_db_path();
        if !path.exists() {
            return Err(
                AssetsError::io(format!("index does not exist: {}", path.display()))
                    .with_hint("create the index with `valle assets reindex`"),
            );
        }
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        Ok(Db { conn })
    }

    /// Shared metadata-to-assets upsert for writes and reindexing.
    pub fn upsert_asset(&self, meta: &AssetMeta) -> Result<()> {
        let hash = meta.content_digest.as_hex();
        let tags = serde_json::to_string(&meta.tags)
            .map_err(|e| AssetsError::io(format!("failed to serialize tags: {e}")))?;
        // Live local assets usually have one location; removed assets have none.
        let (loc_kind, loc_path) = match meta.locations.first() {
            Some(l) => (
                Some(match l.kind {
                    LocationKind::Cas => "cas",
                    LocationKind::Reference => "reference",
                }),
                Some(l.path.as_str()),
            ),
            None => (None, None),
        };
        let (dur, w, h, fps) = match &meta.probe {
            Some(p) => (p.duration_ms, p.width, p.height, p.fps),
            None => (None, None, None, None),
        };
        self.conn.execute(
            "INSERT INTO assets (hash, kind, subkind, size, original_name, title, tags,
                                 added_at, removed_at, location_kind, location_path, stale,
                                 duration_ms, width, height, fps)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, ?12, ?13, ?14, ?15)
             ON CONFLICT(hash) DO UPDATE SET
                 kind=?2, subkind=?3, size=?4, original_name=?5, title=?6, tags=?7,
                 added_at=?8, removed_at=?9, location_kind=?10, location_path=?11,
                 duration_ms=?12, width=?13, height=?14, fps=?15",
            rusqlite::params![
                hash,
                meta.kind.as_str(),
                meta.subkind,
                meta.size as i64,
                meta.original_name,
                meta.title,
                tags,
                meta.added_at,
                meta.removed_at,
                loc_kind,
                loc_path,
                dur,
                w,
                h,
                fps,
            ],
        )?;
        Ok(())
    }

    /// Rebuild the assets table from metadata.
    pub fn reindex_assets(&mut self, home: &Home) -> Result<usize> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM assets", [])?;
        tx.commit()?;
        let mut n = 0;
        for meta in walk_meta_tree(home)? {
            self.upsert_asset(&meta)?;
            n += 1;
        }
        Ok(n)
    }
}

/// Initialize pragmas and schema; return true when a version mismatch requires full projection
/// recovery.
fn init_conn(conn: &Connection) -> Result<bool> {
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    let mut nuked = false;
    let has_version: bool = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='schema_version'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)?;
    if has_version {
        let v: Option<i64> = conn
            .query_row("SELECT version FROM schema_version", [], |r| r.get(0))
            .ok();
        if v != Some(SCHEMA_VERSION) {
            for t in [
                "schema_version",
                "assets",
                "analysis",
                "segments",
                "annotations",
                "entities",
                "annotation_entities",
                "jobs",
                "fts",
                "retrieval_units",
            ] {
                conn.execute_batch(&format!("DROP TABLE IF EXISTS {t}"))?;
            }
            nuked = true;
        }
    }
    conn.execute_batch(SCHEMA)?;
    let count: i64 = conn.query_row("SELECT count(*) FROM schema_version", [], |r| r.get(0))?;
    if count == 0 {
        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [SCHEMA_VERSION],
        )?;
    }
    Ok(nuked)
}

/// Read authoritative metadata, rejecting corruption so reindexing or GC cannot mistake live bytes
/// for orphans. Knowledge trees without metadata are excluded.
pub fn walk_meta_tree(home: &Home) -> Result<Vec<AssetMeta>> {
    let root = home.meta_dir();
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    let mut shards: Vec<_> = std::fs::read_dir(&root)?.filter_map(|e| e.ok()).collect();
    shards.sort_by_key(|e| e.file_name());
    for shard in shards {
        if !shard.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let mut files: Vec<_> = std::fs::read_dir(shard.path())?
            .filter_map(|e| e.ok())
            .collect();
        files.sort_by_key(|e| e.file_name());
        for f in files {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            out.push(AssetMeta::load(&p)?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::kind::AssetKind;
    use crate::assets::meta::Location;

    fn sample(hash: &str) -> AssetMeta {
        let content_digest = crate::ContentDigest::from_hex(hash).unwrap();
        AssetMeta {
            content_digest,
            kind: AssetKind::Video,
            subkind: None,
            size: 100,
            original_name: "a.mp4".into(),
            added_at: "2026-07-17T00:00:00.000Z".into(),
            removed_at: None,
            title: None,
            tags: vec![],
            locations: vec![Location {
                kind: LocationKind::Cas,
                path: format!("objects/{}/{}.mp4", &hash[..2], &hash[2..]),
                verified_at: None,
                mtime_ms: None,
                size: None,
            }],
            companion: None,
            probe: None,
        }
    }

    #[test]
    fn open_is_idempotent_and_fts_available() {
        let tmp = tempfile::tempdir().unwrap();
        let home = Home::at(tmp.path());
        {
            let db = Db::open(&home).unwrap();
            db.conn
                .execute(
                    "INSERT INTO retrieval_units (hash, unit_kind, title) VALUES ('h', 'asset', 'x')",
                    [],
                )
                .unwrap();
        }
        // Reopening preserves data and supports idempotent schema setup and FTS5.
        let db = Db::open(&home).unwrap();
        let n: i64 = db
            .conn
            .query_row("SELECT count(*) FROM retrieval_units", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let v: i64 = db
            .conn
            .query_row("SELECT version FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        db.conn
            .execute("INSERT INTO fts(fts) VALUES('integrity-check')", [])
            .unwrap();
    }

    #[test]
    fn analysis_projection_names_the_cache_fingerprint_explicitly() {
        let tmp = tempfile::tempdir().unwrap();
        let home = Home::at(tmp.path());
        let db = Db::open(&home).unwrap();
        let mut columns = db.conn.prepare("PRAGMA table_info(analysis)").unwrap();
        let columns = columns
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert!(columns.iter().any(|column| column == "params_fingerprint"));
        assert!(!columns.iter().any(|column| column == "params_hash"));
    }

    #[test]
    fn upsert_then_reindex_from_meta_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let home = Home::at(tmp.path());
        let h1 = "a1".repeat(32);
        let h2 = "b2".repeat(32);
        for h in [&h1, &h2] {
            let p = home.meta_path(h);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, sample(h).to_bytes().unwrap()).unwrap();
        }
        let mut db = Db::open(&home).unwrap();
        let n = db.reindex_assets(&home).unwrap();
        assert_eq!(n, 2);
        // Reindexing is idempotent.
        let n = db.reindex_assets(&home).unwrap();
        assert_eq!(n, 2);
        let cnt: i64 = db
            .conn
            .query_row("SELECT count(*) FROM assets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cnt, 2);
    }

    #[test]
    fn empty_reindex_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let home = Home::at(tmp.path());
        let mut db = Db::open(&home).unwrap();
        assert_eq!(db.reindex_assets(&home).unwrap(), 0);
    }
}
