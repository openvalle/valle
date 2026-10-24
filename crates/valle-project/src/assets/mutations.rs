use super::{
    Inspector, Library,
    catalog::{describe, entry, reindex},
    filename, relative, safe_path,
};
use crate::{
    files::{FileStamp, Kind, copy_new, files_in, folded, occupied, rename_new, suffixed},
    folder::digest_file,
};
use anyhow::{Context, Result, bail};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

impl Library {
    pub fn add_batch(
        &mut self,
        paths: &[PathBuf],
        to: Option<&str>,
        name: Option<&str>,
        tags: &[String],
        note: Option<&str>,
        inspector: &dyn Inspector,
    ) -> Result<Value> {
        if paths.is_empty() {
            bail!("provide at least one file or directory");
        }
        if let Some(to) = to {
            relative(to)?;
        }
        if let Some(name) = name {
            filename(name)?;
            if paths.len() != 1 || !paths[0].is_file() {
                bail!("--name requires exactly one file");
            }
        }
        validate_tags(tags)?;
        if note.is_some_and(|n| n.trim().is_empty()) {
            bail!("note text must not be empty");
        }
        let mut items = Vec::new();
        for input in paths {
            let expanded = (|| -> Result<Vec<(PathBuf, PathBuf)>> {
                if input.is_dir() {
                    let base = PathBuf::from(input.file_name().context("directory has no name")?);
                    Ok(files_in(input)?
                        .into_iter()
                        .map(|p| {
                            let parent = p.strip_prefix(input).unwrap().parent().unwrap();
                            (p.clone(), base.join(parent))
                        })
                        .collect())
                } else {
                    Ok(vec![(input.clone(), PathBuf::new())])
                }
            })();
            match expanded {
                Ok(files)=>for (file,subdirectory) in files {
                    let result=self.add_one(&file,to,name,&subdirectory,tags,note,inspector);
                    items.push(match result {Ok(mut item)=>{item["status"]=json!("ok");item},Err(error)=>json!({"status":"error","input":file,"error":{"code":"import_failed","message":format!("{error:#}")}})});
                },
                Err(error)=>items.push(json!({"status":"error","input":input,"error":{"code":"import_failed","message":format!("{error:#}")}})),
            }
        }
        let failed = items.iter().filter(|i| i["status"] == "error").count();
        Ok(json!({"status":if failed==0{"ok"}else{"partial"},"items":items,"failed":failed}))
    }

    #[allow(clippy::too_many_arguments)]
    fn add_one(
        &mut self,
        input: &Path,
        to: Option<&str>,
        name: Option<&str>,
        subdirectory: &Path,
        tags: &[String],
        note: Option<&str>,
        inspector: &dyn Inspector,
    ) -> Result<Value> {
        let source = input
            .canonicalize()
            .with_context(|| format!("opening {}", input.display()))?;
        if !source.is_file() {
            bail!("not an ordinary file: {}", source.display());
        }
        inspector.validate(&source)?;
        let stamp = FileStamp::read(&source)?;
        let digest = digest_file(&source)?;
        let kind = Kind::detect(&source);
        let info = inspector.inspect(&source, kind);
        if FileStamp::read(&source)? != stamp {
            bail!("source changed during import; retry: {}", source.display());
        }
        let directory = PathBuf::from(to.unwrap_or(kind.directory())).join(subdirectory);
        let directory = directory.to_string_lossy().replace('\\', "/");

        let proposed = name.unwrap_or(
            source
                .file_name()
                .context("source has no filename")?
                .to_str()
                .context("filename must be UTF-8")?,
        );
        filename(proposed)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let dir = super::directory_path(&self.root, &directory)?;
        let directory = dir
            .strip_prefix(&self.root)?
            .to_string_lossy()
            .replace('\\', "/");
        let existing = tx
            .query_row(
                "SELECT path FROM assets WHERE digest=? AND missing=0 ORDER BY path LIMIT 1",
                [digest.to_string()],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        let (path, already_exists, renamed) = if let Some(path) = existing {
            let existing = entry(&tx, &path)?;
            let actual = safe_path(&self.root, &path)?;
            if FileStamp::read(&actual).ok() != existing.file_stamp() {
                bail!("existing asset changed during import; retry after assets show {path}");
            }
            (path, true, false)
        } else {
            fs::create_dir_all(&dir)?;
            let mut number = 1;
            let path = loop {
                let actual = suffixed(proposed, number);
                if occupied(&dir, &actual)? {
                    if name.is_some() {
                        bail!(
                            "filename already exists: {directory}/{actual}; choose another --name"
                        );
                    }
                    number += 1;
                    continue;
                }
                let target = dir.join(&actual);
                match copy_new(&source, &target, &self.root.join(".valle/tmp"), digest) {
                    Ok(()) => break target,
                    Err(error)
                        if error
                            .downcast_ref::<std::io::Error>()
                            .is_some_and(|e| e.kind() == std::io::ErrorKind::AlreadyExists)
                            && name.is_none() =>
                    {
                        number += 1;
                    }
                    Err(error) => return Err(error),
                }
            };
            let path = path
                .strip_prefix(&self.root)?
                .to_string_lossy()
                .replace('\\', "/");
            let stamp = FileStamp::read(&self.root.join(&path))?;
            // The file is already published. A failed transaction is recovered by the next scan.
            tx.execute(
                "INSERT INTO assets(path,path_key,kind,digest,stamp) VALUES(?,?,?,?,?)",
                params![
                    path,
                    folded(&path),
                    kind.as_str(),
                    digest.to_string(),
                    serde_json::to_string(&stamp)?
                ],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO facts(asset_id,info) VALUES(?,?)",
                params![id, info.to_string()],
            )?;
            (path, false, number > 1)
        };
        let entry = entry(&tx, &path)?;
        for tag in tags {
            tx.execute(
                "INSERT OR IGNORE INTO tags(asset_id,tag) VALUES(?,?)",
                params![entry.id, tag.trim()],
            )?;
        }
        if let Some(text) = note {
            add_note(&tx, entry.id, text, None, None, None)?;
        }
        reindex(&tx, entry.id)?;
        let mut item = describe(&tx, &self.root, &entry)?;
        item["alreadyExists"] = json!(already_exists);
        item["renamed"] = json!(renamed);
        tx.commit()?;
        Ok(item)
    }

    pub fn edit(
        &mut self,
        path: &str,
        name: Option<&str>,
        to: Option<&str>,
        inspector: &dyn Inspector,
    ) -> Result<Value> {
        if name.is_none() && to.is_none() {
            bail!("provide --name or --to");
        }
        if let Some(name) = name {
            filename(name)?;
        }
        if let Some(to) = to {
            relative(to)?;
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let entry = entry(&tx, path)?;
        if entry.missing {
            bail!("asset is missing: {}", entry.path);
        }
        let source = safe_path(&self.root, &entry.path)?;
        let old = Path::new(&entry.path);
        let directory = super::directory_path(
            &self.root,
            to.unwrap_or(old.parent().unwrap().to_str().unwrap()),
        )?;
        let destination =
            directory.join(name.unwrap_or(old.file_name().unwrap().to_str().unwrap()));
        let destination = destination.strip_prefix(&self.root)?;
        let destination = destination.to_string_lossy().replace('\\', "/");
        if destination == entry.path {
            return Ok(json!({"status":"ok","path":entry.path,"unchanged":true}));
        }
        let target = safe_path(&self.root, &destination)?;
        let same_file = target.exists() && target.canonicalize()? == source.canonicalize()?;
        let case_only = source.parent() == target.parent()
            && folded(&source.file_name().unwrap().to_string_lossy())
                == folded(&target.file_name().unwrap().to_string_lossy())
            && !target.exists();
        if occupied(
            target.parent().unwrap(),
            target.file_name().unwrap().to_str().unwrap(),
        )? && !same_file
            && !case_only
        {
            bail!("filename already exists: {destination}; choose another name or directory");
        }
        if FileStamp::read(&source).ok() != entry.file_stamp() {
            bail!("asset changed; inspect its current state with assets show before retrying edit");
        }
        fs::create_dir_all(target.parent().unwrap())?;
        if same_file {
            fs::rename(&source, &target)?;
        } else {
            rename_new(&source, &target)?;
        }
        let stamp = serde_json::to_string(&FileStamp::read(&target)?)?;
        let kind = Kind::detect(&target);
        tx.execute(
            "UPDATE assets SET path=?,path_key=?,stamp=?,kind=? WHERE asset_id=?",
            params![
                destination,
                folded(&destination),
                stamp,
                kind.as_str(),
                entry.id
            ],
        )?;
        if kind.as_str() != entry.kind {
            let info = inspector.inspect(&target, kind);
            tx.execute("INSERT INTO facts(asset_id,info) VALUES(?,?) ON CONFLICT(asset_id) DO UPDATE SET info=excluded.info",params![entry.id,info.to_string()])?;
        }
        reindex(&tx, entry.id)?;
        tx.commit()?;
        self.show(&destination)
    }

    pub fn tag(&mut self, path: &str, add: &[String], rm: &[String]) -> Result<Value> {
        validate_tags(add)?;
        validate_tags(rm)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let entry = entry(&tx, path)?;
        for tag in rm {
            tx.execute(
                "DELETE FROM tags WHERE asset_id=? AND tag=?",
                params![entry.id, tag.trim()],
            )?;
        }
        for tag in add {
            tx.execute(
                "INSERT OR IGNORE INTO tags(asset_id,tag) VALUES(?,?)",
                params![entry.id, tag.trim()],
            )?;
        }
        reindex(&tx, entry.id)?;
        tx.commit()?;
        self.show(path)
    }

    pub fn annotate(
        &mut self,
        path: &str,
        text: Option<&str>,
        at: Option<f64>,
        range: Option<[f64; 2]>,
        id: Option<i64>,
        rm: Option<i64>,
    ) -> Result<Value> {
        if rm.is_some() && (text.is_some() || id.is_some() || at.is_some() || range.is_some()) {
            bail!("--rm cannot be combined with note contents or --id");
        }
        if at.is_some() && range.is_some() {
            bail!("use either --at or --range");
        }
        if at
            .into_iter()
            .chain(range.into_iter().flatten())
            .any(|t| !t.is_finite() || t < 0.0)
            || range.is_some_and(|r| r[1] <= r[0])
        {
            bail!("note times must be nonnegative; a range needs END > START");
        }
        if rm.is_none() && text.is_none_or(|t| t.trim().is_empty()) {
            bail!("provide nonempty --text, or --rm ID");
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let entry = entry(&tx, path)?;
        let note_id = if let Some(rm) = rm {
            if tx.execute(
                "DELETE FROM notes WHERE note_id=? AND asset_id=?",
                params![rm, entry.id],
            )? == 0
            {
                bail!("note {rm} not found on {}", entry.path);
            }
            rm
        } else {
            add_note(&tx, entry.id, text.unwrap(), at, range, id)?
        };
        reindex(&tx, entry.id)?;
        tx.commit()?;
        let mut item = self.show(path)?;
        item["noteId"] = json!(note_id);
        Ok(item)
    }

    pub fn remove(&mut self, path: Option<&str>, missing: bool) -> Result<Value> {
        if path.is_some() == missing {
            bail!("provide PATH or --missing");
        }
        if missing {
            self.backup(true)?;
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if missing {
            let paths = super::catalog::entries(&tx)?
                .into_iter()
                .filter(|e| e.missing)
                .collect::<Vec<_>>();
            for entry in &paths {
                tx.execute("DELETE FROM assets WHERE asset_id=?", [entry.id])?;
                reindex(&tx, entry.id)?;
            }
            tx.commit()?;
            return Ok(
                json!({"status":"ok","removed":paths.into_iter().map(|e|e.path).collect::<Vec<_>>()}),
            );
        }
        let entry = entry(&tx, path.unwrap())?;
        let file = safe_path(&self.root, &entry.path)?;
        tx.execute("DELETE FROM assets WHERE asset_id=?", [entry.id])?;
        reindex(&tx, entry.id)?;
        tx.commit()?;
        // User metadata is gone before trashing. A leftover file becomes a fresh entry on scan.
        if file.exists() {
            if FileStamp::read(&file).ok() != entry.file_stamp() {
                bail!(
                    "file changed before trashing; metadata was removed; inspect assets show {} before retrying remove",
                    entry.path
                );
            }
            move_to_trash(&file).with_context(||format!("metadata removed but file could not enter Trash: {}; inspect assets show before retrying",file.display()))?;
        }
        Ok(json!({"status":"ok","removed":[entry.path]}))
    }
}

fn move_to_trash(path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut context = trash::TrashContext::new();
        context.set_delete_method(DeleteMethod::NsFileManager);
        context.delete(path)?;
    }
    #[cfg(not(target_os = "macos"))]
    trash::delete(path)?;
    Ok(())
}

fn validate_tags(tags: &[String]) -> Result<()> {
    if tags
        .iter()
        .any(|t| t.trim().is_empty() || t.chars().any(char::is_control))
    {
        bail!("tags must be nonempty and contain no control characters");
    }
    Ok(())
}

fn add_note(
    conn: &rusqlite::Connection,
    asset: i64,
    text: &str,
    at: Option<f64>,
    range: Option<[f64; 2]>,
    id: Option<i64>,
) -> Result<i64> {
    if let Some(id) = id {
        let existing = conn
            .query_row(
                "SELECT note_id FROM notes WHERE asset_id=? AND note_id=?",
                params![asset, id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        if existing.is_none() {
            bail!("note {id} not found on this asset");
        }
    }
    let start = range.map(|r| r[0]);
    let end = range.map(|r| r[1]);
    let existing=conn.query_row("SELECT note_id FROM notes WHERE asset_id=? AND text=? AND at IS ? AND start IS ? AND end IS ?",params![asset,text,at,start,end],|r|r.get::<_,i64>(0)).optional()?;
    if let Some(existing) = existing {
        if let Some(id) = id.filter(|id| *id != existing) {
            conn.execute("DELETE FROM notes WHERE note_id=?", [id])?;
        }
        return Ok(existing);
    }
    if let Some(id) = id {
        conn.execute(
            "UPDATE notes SET text=?,at=?,start=?,end=? WHERE note_id=? AND asset_id=?",
            params![text, at, start, end, id, asset],
        )?;
        Ok(id)
    } else {
        conn.execute(
            "INSERT INTO notes(asset_id,text,at,start,end) VALUES(?,?,?,?,?)",
            params![asset, text, at, start, end],
        )?;
        Ok(conn.last_insert_rowid())
    }
}
