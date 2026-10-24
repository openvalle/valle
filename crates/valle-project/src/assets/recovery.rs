use super::{Context as AssetsContext, Library, catalog::validate, clock};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn candidates(directory: &Path) -> Result<Vec<(String, PathBuf)>> {
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_file() && path.extension().is_some_and(|e| e == "db") {
            found.push((name, path));
        } else if entry.file_type()?.is_dir()
            && name.starts_with("replaced-")
            && path.join("catalog.db").is_file()
        {
            found.push((format!("{name}/catalog.db"), path.join("catalog.db")));
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(found)
}

fn read_backup(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    validate(&conn, true)?;
    Ok(conn)
}

pub(crate) fn recover(context: &AssetsContext, backup: Option<&str>) -> Result<Value> {
    let management = context.root.join(".valle");
    let backups = management.join("backups");
    for path in [&management, &backups] {
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!("asset management paths must not be symlinks");
        }
    }
    let found = candidates(&backups)?;
    let Some(name) = backup else {
        let items=found.into_iter().map(|(name,path)|match read_backup(&path) {
            Ok(conn)=>json!({"name":name,"date":name.get(..10),"assets":conn.query_row("SELECT count(*) FROM assets",[],|r|r.get::<_,i64>(0)).ok(),"notes":conn.query_row("SELECT count(*) FROM notes",[],|r|r.get::<_,i64>(0)).ok(),"corrupt":false}),
            Err(error)=>json!({"name":name,"corrupt":true,"error":format!("{error:#}")}),
        }).collect::<Vec<_>>();
        return Ok(
            json!({"status":"ok","backups":items,"hint":"Close Studio and other asset commands before restoring a backup."}),
        );
    };
    let source = found
        .into_iter()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, path)| path)
        .context("backup not found; run valle assets recover to list backups")?;
    // Validate and make a standalone SQLite snapshot before touching the current raw files.
    let source_conn =
        read_backup(&source).context("backup is corrupt; current catalog was preserved")?;
    fs::create_dir_all(&management)?;
    let temporary = tempfile::NamedTempFile::new_in(&management)?;
    source_conn.backup(rusqlite::MAIN_DB, temporary.path(), None)?;
    drop(source_conn);
    drop(read_backup(temporary.path())?);
    temporary.as_file().sync_all()?;
    let archive = tempfile::Builder::new()
        .prefix(&format!("replaced-{}-", clock::now_millis()))
        .tempdir_in(&backups)?
        .keep();
    for filename in ["catalog.db", "catalog.db-wal", "catalog.db-shm"] {
        let path = management.join(filename);
        if fs::symlink_metadata(&path).is_ok() {
            fs::rename(&path, archive.join(filename))?;
        }
    }
    crate::sync_directory(&archive)?;
    temporary
        .persist_noclobber(management.join("catalog.db"))
        .map_err(|e| e.error)?;
    crate::sync_directory(&management)?;
    let mut library = Library::open(&context.root)?;
    library.scan(context.inspector.as_ref())?;
    Ok(
        json!({"status":"ok","restored":name,"replaced":archive,"hint":"Files replaced at the same path since this backup can inherit older notes; review or remove them if necessary."}),
    )
}
