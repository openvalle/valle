use super::{
    Inspector, Library,
    catalog::{entries, reindex},
};
use crate::{
    files::{FileStamp, Kind, files_in, folded},
    folder::digest_file,
};
use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

struct Observed {
    path: String,
    kind: Kind,
    stamp: String,
    identity: String,
    digest: String,
    info: Option<Value>,
}

pub(crate) fn scan(library: &mut Library, inspector: &dyn Inspector) -> Result<()> {
    let tx = library
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let old = entries(&tx)?;
    let by_path = old
        .iter()
        .map(|e| (e.path.as_str(), e))
        .collect::<BTreeMap<_, _>>();
    let mut observed = Vec::new();
    let mut keys = BTreeSet::new();
    for file in files_in(&library.root)? {
        let path = file
            .strip_prefix(&library.root)?
            .to_string_lossy()
            .replace('\\', "/");
        if !keys.insert(folded(&path)) {
            bail!(
                "library has filenames differing only in case: {path}; rename one before using the library"
            );
        }
        let stamp = FileStamp::read(&file)?;
        let serialized = serde_json::to_string(&stamp)?;
        let kind = Kind::detect(&file);
        let previous = by_path.get(path.as_str());
        let facts = previous
            .map(|e| {
                tx.query_row("SELECT info FROM facts WHERE asset_id=?", [e.id], |r| {
                    r.get::<_, String>(0)
                })
                .optional()
            })
            .transpose()?
            .flatten();
        let unchanged = previous.is_some_and(|e| e.stamp == serialized && e.kind == kind.as_str())
            && facts.is_some();
        let (digest, info) = if unchanged {
            (previous.unwrap().digest.clone(), None)
        } else {
            let digest = digest_file(&file)?.to_string();
            let info = inspector.inspect(&file, kind);
            if FileStamp::read(&file)? != stamp {
                bail!(
                    "file changed while scanning: {}; retry the command",
                    file.display()
                );
            }
            (digest, Some(info))
        };
        observed.push(Observed {
            path,
            kind,
            stamp: serialized,
            identity: stamp.identity,
            digest,
            info,
        });
    }
    let present = observed
        .iter()
        .map(|o| o.path.as_str())
        .collect::<BTreeSet<_>>();
    let missing = old
        .iter()
        .filter(|e| !present.contains(e.path.as_str()))
        .collect::<Vec<_>>();
    let mut new_digest_counts = BTreeMap::<&str, usize>::new();
    for file in &observed {
        if !by_path.contains_key(file.path.as_str()) {
            *new_digest_counts.entry(&file.digest).or_default() += 1;
        }
    }
    for entry in &missing {
        if !entry.missing {
            tx.execute("UPDATE assets SET missing=1 WHERE asset_id=?", [entry.id])?;
        }
    }
    let mut matched = BTreeSet::new();
    for file in &observed {
        let previous = by_path.get(file.path.as_str()).copied().or_else(|| {
            let candidates = missing
                .iter()
                .copied()
                .filter(|e| !matched.contains(&e.id) && e.digest == file.digest)
                .collect::<Vec<_>>();
            let identity = candidates
                .iter()
                .copied()
                .filter(|e| e.file_stamp().is_some_and(|s| s.identity == file.identity))
                .collect::<Vec<_>>();
            if identity.len() == 1 {
                Some(identity[0])
            } else if candidates.len() == 1
                && new_digest_counts.get(file.digest.as_str()) == Some(&1)
            {
                Some(candidates[0])
            } else {
                None
            }
        });
        let id = if let Some(entry) = previous {
            matched.insert(entry.id);
            if entry.path == file.path
                && entry.stamp == file.stamp
                && entry.kind == file.kind.as_str()
                && !entry.missing
                && file.info.is_none()
            {
                continue;
            }
            tx.execute("UPDATE assets SET path=?,path_key=?,kind=?,digest=?,stamp=?,missing=0 WHERE asset_id=?",params![file.path,folded(&file.path),file.kind.as_str(),file.digest,file.stamp,entry.id])?;
            entry.id
        } else {
            tx.execute(
                "INSERT INTO assets(path,path_key,kind,digest,stamp) VALUES(?,?,?,?,?)",
                params![
                    file.path,
                    folded(&file.path),
                    file.kind.as_str(),
                    file.digest,
                    file.stamp
                ],
            )?;
            tx.last_insert_rowid()
        };
        if let Some(info) = &file.info {
            tx.execute("INSERT INTO facts(asset_id,info) VALUES(?,?) ON CONFLICT(asset_id) DO UPDATE SET info=excluded.info",params![id,info.to_string()])?;
        }
        reindex(&tx, id)?;
    }
    tx.commit()?;
    Ok(())
}
