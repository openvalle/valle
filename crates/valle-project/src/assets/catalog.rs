use super::{Inspector, clock, fts};
use crate::{
    files::{FileStamp, folded},
    folder::absolute,
};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

pub struct Library {
    pub root: PathBuf,
    pub(crate) conn: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub id: i64,
    pub path: String,
    pub kind: String,
    pub digest: String,
    pub stamp: String,
    pub missing: bool,
}

impl Entry {
    pub fn file_stamp(&self) -> Option<FileStamp> {
        serde_json::from_str(&self.stamp).ok()
    }
}

const PRIMARY_SCHEMA: &str = "
CREATE TABLE assets (
 asset_id INTEGER PRIMARY KEY AUTOINCREMENT,
 path TEXT NOT NULL UNIQUE, path_key TEXT NOT NULL UNIQUE,
 kind TEXT NOT NULL, digest TEXT NOT NULL, stamp TEXT NOT NULL,
 missing INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE tags (asset_id INTEGER NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE, tag TEXT NOT NULL, PRIMARY KEY(asset_id,tag));
CREATE TABLE notes (note_id INTEGER PRIMARY KEY AUTOINCREMENT, asset_id INTEGER NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE, text TEXT NOT NULL, at REAL, start REAL, end REAL);
";

impl Library {
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        if !root.join(".valle/catalog.db").exists()
            && root.join("index.db").is_file()
            && Connection::open_with_flags(root.join("index.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)
                .and_then(|conn| conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name IN ('assets','objects'))", [], |row| row.get::<_, bool>(0)))
                .unwrap_or(false)
        {
            bail!("legacy asset library at {}; move it aside before creating a new library", root.display());
        }
        let management = root.join(".valle");
        fs::create_dir_all(&management)?;
        for path in [
            &management,
            &management.join("catalog.db"),
            &management.join("backups"),
            &management.join("tmp"),
        ] {
            if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
                bail!(
                    "asset management paths must not be symlinks: {}",
                    path.display()
                );
            }
        }
        let database = management.join("catalog.db");
        let new = !database.exists();
        if new
            && management.join("backups").is_dir()
            && fs::read_dir(management.join("backups"))?.next().is_some()
        {
            bail!("catalog.db is missing but backups exist; use valle assets recover");
        }
        if !new {
            let readonly =
                Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            readonly.busy_timeout(Duration::from_secs(15))?;
            validate(&readonly, false).context("catalog.db is damaged or has an unsupported primary schema; use valle assets recover")?;
        }
        let mut conn = Connection::open(&database)
            .context("open catalog.db; use valle assets recover if damaged")?;
        conn.busy_timeout(Duration::from_secs(15))?;
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;")?;
        if new {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let tables: i64 = tx.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='assets'",
                [],
                |r| r.get(0),
            )?;
            if tables == 0 {
                tx.execute_batch(PRIMARY_SCHEMA)?;
            }
            tx.commit()?;
        }
        validate(&conn, false).context(
            "catalog.db is damaged or has an unsupported primary schema; use valle assets recover",
        )?;
        let library = Self { root, conn };
        library.backup(false)?;
        library.ensure_derived()?;
        Ok(library)
    }

    fn ensure_derived(&self) -> Result<()> {
        let facts = columns(&self.conn, "facts")?;
        let search = columns(&self.conn, "asset_search")?;
        if facts == ["asset_id", "info"] && search == ["content"] {
            return Ok(());
        }
        // Only these derived tables may be rebuilt. Primary tables have already been validated.
        self.conn.execute_batch("BEGIN IMMEDIATE; DROP TABLE IF EXISTS facts; DROP TABLE IF EXISTS asset_search; CREATE TABLE facts(asset_id INTEGER PRIMARY KEY REFERENCES assets(asset_id) ON DELETE CASCADE, info TEXT NOT NULL); CREATE VIRTUAL TABLE asset_search USING fts5(content); COMMIT;")?;
        let ids = self
            .entries()?
            .into_iter()
            .map(|e| e.id)
            .collect::<Vec<_>>();
        let tx = self.conn.unchecked_transaction()?;
        for id in ids {
            reindex(&tx, id)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn entries(&self) -> Result<Vec<Entry>> {
        entries(&self.conn)
    }

    pub(crate) fn entry(&self, path: &str) -> Result<Entry> {
        entry(&self.conn, path)
    }

    pub(crate) fn describe(&self, entry: &Entry) -> Result<Value> {
        describe(&self.conn, &self.root, entry)
    }

    pub fn scan(&mut self, inspector: &dyn Inspector) -> Result<()> {
        super::scan::scan(self, inspector)
    }

    pub(crate) fn backup(&self, bulk: bool) -> Result<()> {
        let backups = self.root.join(".valle/backups");
        fs::create_dir_all(&backups)?;
        let timestamp = clock::now_millis();
        let date = clock::iso8601(timestamp)[..10].to_owned();
        let name = if bulk {
            format!("{date}-before-remove-{timestamp}.db")
        } else {
            format!("{date}.db")
        };
        let target = backups.join(name);
        if target.exists() {
            return Ok(());
        }
        let temporary = tempfile::NamedTempFile::new_in(self.root.join(".valle"))?;
        self.conn
            .backup(rusqlite::MAIN_DB, temporary.path(), None)?;
        temporary.as_file().sync_all()?;
        match temporary.persist_noclobber(&target) {
            Ok(_) => (),
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.error.into()),
        }
        crate::sync_directory(&backups)?;
        let mut files = fs::read_dir(&backups)?
            .collect::<std::io::Result<Vec<_>>>()?
            .into_iter()
            .filter(|e| {
                e.file_type().is_ok_and(|t| t.is_file())
                    && e.path().extension().is_some_and(|e| e == "db")
            })
            .map(|e| e.path())
            .collect::<Vec<_>>();
        files.sort_by_key(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            (
                name.get(..10).unwrap_or("").to_owned(),
                name.contains("-before-remove-"),
                name.into_owned(),
            )
        });
        let excess = files.len().saturating_sub(7);
        for old in files.into_iter().take(excess) {
            fs::remove_file(old)?;
        }
        Ok(())
    }
}

pub(crate) fn columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    Ok(conn
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |r| r.get(1))?
        .collect::<rusqlite::Result<_>>()?)
}

pub(crate) fn validate(conn: &Connection, full: bool) -> Result<()> {
    // quick_check also asks FTS to read its content and index. Pin one WAL snapshot across all
    // those reads and schema checks while other connections add tags and update search rows.
    let snapshot = conn.unchecked_transaction()?;
    let check: String = snapshot.query_row(
        if full {
            "PRAGMA integrity_check"
        } else {
            "PRAGMA quick_check"
        },
        [],
        |r| r.get(0),
    )?;
    if check != "ok" {
        bail!("database integrity check failed: {check}");
    }
    let normalize = |sql: &str| {
        sql.chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    for statement in PRIMARY_SCHEMA
        .split(';')
        .map(str::trim)
        .filter(|sql| !sql.is_empty())
    {
        let table = statement.split_whitespace().nth(2).unwrap();
        let stored: String = snapshot.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=?",
            [table],
            |row| row.get(0),
        )?;
        if normalize(&stored) != normalize(statement) {
            bail!("unsupported primary schema for {table}; user data was preserved");
        }
    }
    snapshot.commit()?;
    Ok(())
}

pub(crate) fn entries(conn: &Connection) -> Result<Vec<Entry>> {
    Ok(conn
        .prepare("SELECT asset_id,path,kind,digest,stamp,missing FROM assets ORDER BY path")?
        .query_map([], |r| {
            Ok(Entry {
                id: r.get(0)?,
                path: r.get(1)?,
                kind: r.get(2)?,
                digest: r.get(3)?,
                stamp: r.get(4)?,
                missing: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}

pub(crate) fn entry(conn: &Connection, path: &str) -> Result<Entry> {
    super::relative(path)?;
    entries(conn)?
        .into_iter()
        .find(|e| folded(&e.path) == folded(path))
        .with_context(|| {
            format!(
                "asset not found: {path}; use valle assets list or search to find its current path"
            )
        })
}

pub(crate) fn describe(conn: &Connection, root: &Path, entry: &Entry) -> Result<Value> {
    let tags = conn
        .prepare("SELECT tag FROM tags WHERE asset_id=? ORDER BY tag")?
        .query_map([entry.id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let notes=conn.prepare("SELECT note_id,text,at,start,end FROM notes WHERE asset_id=? ORDER BY COALESCE(at,start,0),note_id")?.query_map([entry.id],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"text":r.get::<_,String>(1)?,"at":r.get::<_,Option<f64>>(2)?,"range":r.get::<_,Option<f64>>(3)?.zip(r.get::<_,Option<f64>>(4)?).map(|(a,b)|[a,b])})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let info = conn
        .query_row("SELECT info FROM facts WHERE asset_id=?", [entry.id], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    Ok(
        json!({"path":entry.path,"absolutePath":absolute(&root.join(&entry.path))?,"kind":entry.kind,"missing":entry.missing,"info":info,"tags":tags,"notes":notes}),
    )
}

pub(crate) fn reindex(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM asset_search WHERE rowid=?", [id])?;
    let path = conn
        .query_row("SELECT path FROM assets WHERE asset_id=?", [id], |r| {
            r.get::<_, String>(0)
        })
        .optional()?;
    let Some(path) = path else {
        return Ok(());
    };
    let mut texts = vec![path];
    for sql in [
        "SELECT tag FROM tags WHERE asset_id=?",
        "SELECT text FROM notes WHERE asset_id=?",
        "SELECT COALESCE(json_extract(info,'$.family'),'') FROM facts WHERE asset_id=?",
    ] {
        texts.extend(
            conn.prepare(sql)?
                .query_map([id], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        );
    }
    let content = fts::index_texts(texts.iter().map(String::as_str));
    conn.execute(
        "INSERT INTO asset_search(rowid,content) VALUES (?,?)",
        rusqlite::params![id, content],
    )?;
    Ok(())
}
