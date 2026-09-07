//! Asset maintenance: remove, verify, garbage-collect, and reindex.

use serde::Serialize;
use serde_json::Value;

use crate::ContentDigest;
use crate::assets::addr;
use crate::assets::clock;
use crate::assets::ctx::Ctx;
use crate::assets::db::{Db, walk_meta_tree};
use crate::assets::fsutil;
use crate::assets::lock;
use crate::assets::meta::{AssetMeta, LocationKind};
use crate::assets::report::{AssetsError, Result};
use crate::assets::resource_roots::CommittedResourceRoots;

// —— rm ——

#[derive(Debug, Serialize)]
pub struct RmOutcome {
    pub content_digest: ContentDigest,
    /// Removal retains knowledge; purging also deletes knowledge.
    pub action: &'static str,
}

/// Remove blobs and locations while retaining metadata and knowledge by default. Purging deletes
/// both, requiring force for human annotations. Self-contained projects remain independent.
pub fn rm(ctx: &Ctx, hash_or_prefix: &str, purge: bool, force: bool) -> Result<RmOutcome> {
    let _lock = lock::acquire(&ctx.home)?;
    // Keep package/lease root publication excluded until every possible byte
    // deletion has completed. Uncertain roots fail before any mutation.
    let committed_roots = ctx.committed_resource_roots()?;
    let hash = addr::find_hash(&ctx.home, hash_or_prefix)?;
    let meta_path = ctx.home.meta_path(&hash);
    let mut meta = AssetMeta::load(&meta_path)?;
    let content_digest = meta.content_digest;

    let ann_dir = ctx.home.annotations_dir(&hash);
    let has_annotations = ann_dir.exists()
        && std::fs::read_dir(&ann_dir)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);

    if purge {
        if has_annotations && !force {
            return Err(AssetsError::refused(format!(
                "asset {} has human annotations; purge refused",
                &hash[..12]
            ))
            .with_hint(
                "use --force to delete annotations too; omit --purge to remove only the bytes",
            ));
        }
        delete_bytes(ctx, &meta, &committed_roots);
        let db = Db::open(&ctx.home)?;
        crate::assets::units::delete_for_asset(&db, &hash)?;
        let _ = std::fs::remove_file(&meta_path);
        let _ = std::fs::remove_dir_all(ctx.home.analysis_dir(&hash));
        let _ = std::fs::remove_dir_all(&ann_dir);
        purge_db_rows(&db, &hash)?;
        return Ok(RmOutcome {
            content_digest,
            action: "purged",
        });
    }

    if meta.is_removed() {
        // Removing an already removed asset is idempotent.
        return Ok(RmOutcome {
            content_digest,
            action: "removed",
        });
    }
    delete_bytes(ctx, &meta, &committed_roots);
    meta.locations.clear();
    meta.removed_at = Some(clock::iso8601(clock::now_millis()));
    fsutil::write_atomic(&meta_path, &meta.to_bytes()?, "meta")?;
    let db = Db::open(&ctx.home)?;
    db.upsert_asset(&meta)?;
    // Removed assets must have no retrieval units or FTS entries.
    crate::assets::units::delete_for_asset(&db, &hash)?;
    Ok(RmOutcome {
        content_digest,
        action: "removed",
    })
}

/// Delete CAS blobs and companions; reference locations own no library bytes.
fn delete_bytes(ctx: &Ctx, meta: &AssetMeta, committed_roots: &CommittedResourceRoots) {
    if committed_roots.contains(&meta.content_digest) {
        return;
    }
    for loc in &meta.locations {
        if matches!(loc.kind, LocationKind::Cas) {
            let _ = std::fs::remove_file(ctx.home.assets_dir().join(&loc.path));
        }
    }
    if let Some(c) = &meta.companion {
        let _ = std::fs::remove_file(ctx.home.assets_dir().join(c));
    }
}

fn purge_db_rows(db: &Db, hash: &str) -> Result<()> {
    for table in [
        "assets",
        "analysis",
        "segments",
        "annotations",
        "annotation_entities",
        "retrieval_units",
    ] {
        db.conn
            .execute(&format!("DELETE FROM {table} WHERE hash=?1"), [hash])?;
    }
    Ok(())
}

// —— verify ——

#[derive(Debug, Serialize)]
pub struct Issue {
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyOutcome {
    pub checked: usize,
    pub issues: Vec<Issue>,
}

/// Verify references, missing and orphan blobs, orphan knowledge, temporary residue, and index
/// drift. Deep verification rehashes CAS contents.
pub fn verify(ctx: &Ctx, deep: bool) -> Result<VerifyOutcome> {
    let committed_roots = ctx.committed_resource_roots()?;
    let metas = walk_meta_tree(&ctx.home)?;
    let db = Db::open(&ctx.home)?;
    let mut issues = Vec::new();

    for meta in &metas {
        let hash = meta.content_digest.as_hex();
        for loc in &meta.locations {
            match loc.kind {
                LocationKind::Cas => {
                    let abs = ctx.home.assets_dir().join(&loc.path);
                    if !abs.exists() {
                        issues.push(Issue {
                            kind: "blob_missing",
                            hash: Some(hash.clone()),
                            detail: format!("blob missing: {}", loc.path),
                        });
                    } else if deep {
                        let actual = crate::assets::add::stream_sha256(&abs)?;
                        if actual != meta.content_digest {
                            issues.push(Issue {
                                kind: "hash_mismatch",
                                hash: Some(hash.clone()),
                                detail: format!(
                                    "blob content does not match its registered hash: {}",
                                    loc.path
                                ),
                            });
                        }
                    }
                }
                LocationKind::Reference => {
                    let stale = match std::fs::metadata(&loc.path) {
                        Ok(md) => {
                            let mtime_ms = md
                                .modified()
                                .ok()
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_millis() as i64);
                            loc.size != Some(md.len()) || loc.mtime_ms != mtime_ms
                        }
                        Err(_) => true,
                    };
                    db.conn.execute(
                        "UPDATE assets SET stale=?2 WHERE hash=?1",
                        rusqlite::params![hash, stale as i64],
                    )?;
                    if stale {
                        issues.push(Issue {
                            kind: "stale_reference",
                            hash: Some(hash.clone()),
                            detail: format!("reference source is stale: {}", loc.path),
                        });
                    }
                }
            }
        }
    }

    // Find blobs with no live CAS metadata or committed resource root.
    for (hash, path) in orphan_blobs(ctx, &metas, &committed_roots)? {
        issues.push(Issue {
            kind: "orphan_blob",
            hash: Some(hash),
            detail: format!("orphan blob eligible for GC: {}", path.display()),
        });
    }

    // Find analysis and annotation trees without metadata.
    let meta_hashes: std::collections::HashSet<String> = metas
        .iter()
        .map(|meta| meta.content_digest.as_hex())
        .collect();
    for tree in ["analysis", "annotations"] {
        for hash in tree_hashes(ctx, tree)? {
            if !meta_hashes.contains(hash.as_str()) {
                issues.push(Issue {
                    kind: "orphan_knowledge",
                    hash: Some(hash.clone()),
                    detail: format!(
                        "{tree}/ contains knowledge without metadata; reindex will skip it"
                    ),
                });
            }
        }
    }

    // Report temporary-file residue.
    for p in tmp_residue(ctx)? {
        issues.push(Issue {
            kind: "tmp_residue",
            hash: None,
            detail: format!("staging residue eligible for GC: {}", p.display()),
        });
    }

    // Detect differences between metadata hashes and indexed assets.
    let mut stmt = db.conn.prepare("SELECT hash FROM assets")?;
    let db_hashes: std::collections::HashSet<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();
    let truth: std::collections::HashSet<String> = metas
        .iter()
        .map(|meta| meta.content_digest.as_hex())
        .collect();
    if db_hashes != truth {
        issues.push(Issue {
            kind: "index_drift",
            hash: None,
            detail: format!(
                "index differs from metadata (metadata {} / index {}); run reindex",
                truth.len(),
                db_hashes.len()
            ),
        });
    }

    Ok(VerifyOutcome {
        checked: metas.len(),
        issues,
    })
}

/// Return orphan blob hashes and absolute paths, excluding live CAS metadata and committed roots.
fn orphan_blobs(
    ctx: &Ctx,
    metas: &[AssetMeta],
    committed_roots: &CommittedResourceRoots,
) -> Result<Vec<(String, std::path::PathBuf)>> {
    // Collect relative paths for live CAS locations and companions.
    let mut live: std::collections::HashSet<String> = std::collections::HashSet::new();
    for m in metas {
        for l in &m.locations {
            if matches!(l.kind, LocationKind::Cas) {
                live.insert(l.path.clone());
            }
        }
        if let Some(c) = &m.companion {
            live.insert(c.clone());
        }
    }
    let root = ctx.home.objects_dir();
    let mut out = Vec::new();
    if !real_directory_exists(&root, "CAS objects root")? {
        return Ok(out);
    }
    for shard in std::fs::read_dir(&root)? {
        let shard = shard?;
        let kind = shard.file_type()?;
        reject_symlink(&shard.path(), kind, "CAS objects tree")?;
        if !kind.is_dir() {
            continue;
        }
        let shard_name = shard.file_name().to_string_lossy().into_owned();
        for f in std::fs::read_dir(shard.path())? {
            let f = f?;
            let kind = f.file_type()?;
            reject_symlink(&f.path(), kind, "CAS objects tree")?;
            if !kind.is_file() {
                continue;
            }
            let name = f.file_name().to_string_lossy().into_owned();
            if is_atomic_staging_name(&name) {
                continue; // Report temporary residue separately.
            }
            let rel = format!("objects/{shard_name}/{name}");
            if !live.contains(&rel) {
                let stem = name.split('.').next().unwrap_or(&name);
                let hash = format!("{shard_name}{stem}");
                if !ContentDigest::from_hex(&hash)
                    .ok()
                    .is_some_and(|digest| committed_roots.contains(&digest))
                {
                    out.push((hash, f.path()));
                }
            }
        }
    }
    Ok(out)
}

/// Recover all content hashes from a sharded knowledge tree.
fn tree_hashes(ctx: &Ctx, tree: &str) -> Result<Vec<String>> {
    let root = ctx.home.assets_dir().join(tree);
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    for shard in std::fs::read_dir(&root)?.filter_map(|e| e.ok()) {
        if !shard.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let shard_name = shard.file_name().to_string_lossy().into_owned();
        for d in std::fs::read_dir(shard.path())?.filter_map(|e| e.ok()) {
            if d.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                out.push(format!("{shard_name}{}", d.file_name().to_string_lossy()));
            }
        }
    }
    Ok(out)
}

/// Find atomic-write staging residue under metadata and object directories.
fn tmp_residue(ctx: &Ctx) -> Result<Vec<std::path::PathBuf>> {
    let mut out = Vec::new();
    for root in [ctx.home.objects_dir(), ctx.home.meta_dir()] {
        if !real_directory_exists(&root, "atomic staging root")? {
            continue;
        }
        let mut stack = vec![root];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d)? {
                let e = entry?;
                let p = e.path();
                let kind = e.file_type()?;
                reject_symlink(&p, kind, "atomic staging tree")?;
                if kind.is_dir() {
                    stack.push(p);
                } else if kind.is_file()
                    && p.file_name()
                        .map(|n| is_atomic_staging_name(&n.to_string_lossy()))
                        .unwrap_or(false)
                {
                    out.push(p);
                }
            }
        }
    }
    Ok(out)
}

fn is_atomic_staging_name(name: &str) -> bool {
    name.starts_with('.') && name.contains(".tmp-")
}

fn real_directory_exists(path: &std::path::Path, subject: &str) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(true),
        Ok(_) => Err(AssetsError::io(format!(
            "{subject} must be a real directory: {}",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(AssetsError::from(error)),
    }
}

fn reject_symlink(path: &std::path::Path, kind: std::fs::FileType, subject: &str) -> Result<()> {
    if kind.is_symlink() {
        return Err(AssetsError::io(format!(
            "refusing {subject} traversal through symlink {}",
            path.display()
        )));
    }
    Ok(())
}

// —— gc ——

#[derive(Debug, Serialize)]
pub struct GcOutcome {
    pub removed_blobs: usize,
    pub removed_tmp: usize,
    pub removed_cache: usize,
}

/// Collect orphan blobs, temporary files, and orphan cache entries. Preserve registered
/// authoritative data and knowledge intentionally retained by removal.
pub fn gc(ctx: &Ctx) -> Result<GcOutcome> {
    let _lock = lock::acquire(&ctx.home)?;
    // This guard excludes package/lease root publication until all deletions finish.
    let committed_roots = ctx.committed_resource_roots()?;

    // Finish every fallible traversal before the first deletion. A malformed,
    // unreadable, or symlinked tree must fail closed without partially applying
    // an otherwise valid collection plan.
    let tmp_residue = tmp_residue(ctx)?;
    let metas = walk_meta_tree(&ctx.home)?;
    let orphan_blobs = orphan_blobs(ctx, &metas, &committed_roots)?;
    let live: std::collections::HashSet<String> = metas
        .iter()
        .filter(|m| !m.is_removed())
        .map(|m| m.content_digest.as_hex())
        .collect();
    let orphan_cache = crate::assets::cachefs::collect_gc_candidates(&ctx.home, &live)?;

    let mut removed_blobs = 0;
    for (_, path) in orphan_blobs {
        std::fs::remove_file(&path)?;
        removed_blobs += 1;
    }
    let mut removed_tmp = 0;
    for p in tmp_residue {
        std::fs::remove_file(&p)?;
        removed_tmp += 1;
    }
    // Cache entries are live only for metadata that is not removed.
    let removed_cache = crate::assets::cachefs::remove_gc_candidates(orphan_cache)?;
    Ok(GcOutcome {
        removed_blobs,
        removed_tmp,
        removed_cache,
    })
}

// —— reindex ——

/// Rebuild asset, entity, annotation, retrieval-unit, and FTS projections from authoritative files.
pub fn reindex(ctx: &Ctx) -> Result<crate::assets::units::RebuildStats> {
    let _lock = lock::acquire(&ctx.home)?;
    let mut db = Db::open(&ctx.home)?;
    crate::assets::units::full_rebuild(&mut db, &ctx.home)
}

/// Deterministic assets-table dump for comparing projection rebuilds.
pub fn dump_assets_table(db: &Db) -> Result<Value> {
    let mut stmt = db.conn.prepare(
        "SELECT hash, kind, subkind, size, original_name, title, tags, added_at, removed_at,
                location_kind, location_path, stale, duration_ms, width, height, fps
         FROM assets ORDER BY hash",
    )?;
    let rows: Vec<Value> = stmt
        .query_map([], |r| {
            Ok(serde_json::json!({
                "hash": r.get::<_, String>(0)?,
                "kind": r.get::<_, String>(1)?,
                "subkind": r.get::<_, Option<String>>(2)?,
                "size": r.get::<_, i64>(3)?,
                "original_name": r.get::<_, String>(4)?,
                "title": r.get::<_, Option<String>>(5)?,
                "tags": r.get::<_, String>(6)?,
                "added_at": r.get::<_, String>(7)?,
                "removed_at": r.get::<_, Option<String>>(8)?,
                "location_kind": r.get::<_, Option<String>>(9)?,
                "location_path": r.get::<_, Option<String>>(10)?,
                "stale": r.get::<_, i64>(11)?,
                "duration_ms": r.get::<_, Option<i64>>(12)?,
                "width": r.get::<_, Option<i64>>(13)?,
                "height": r.get::<_, Option<i64>>(14)?,
                "fps": r.get::<_, Option<f64>>(15)?,
            }))
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(Value::Array(rows))
}
