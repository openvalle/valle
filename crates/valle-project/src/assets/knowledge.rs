//! Metadata editing and tag operations. All metadata changes use `update_meta`: lock, load, detect
//! changes, atomically publish, and update projections. Absolute-value updates are idempotent.

use serde_json::{Value, json};

use crate::assets::addr;
use crate::assets::ctx::Ctx;
use crate::assets::db::Db;
use crate::assets::fsutil;
use crate::assets::kind::AssetKind;
use crate::assets::lock;
use crate::assets::meta::AssetMeta;
use crate::assets::report::{AssetsError, Result};

/// Update authoritative metadata and its projection under a lock. The callback reports changes;
/// unchanged metadata still repairs a stale index.
pub fn update_meta(
    ctx: &Ctx,
    hash_or_prefix: &str,
    mutate: impl FnOnce(&mut AssetMeta) -> Result<bool>,
) -> Result<AssetMeta> {
    let _lock = lock::acquire(&ctx.home)?;
    let hash = addr::find_hash(&ctx.home, hash_or_prefix)?;
    let meta_path = ctx.home.meta_path(&hash);
    let mut meta = AssetMeta::load(&meta_path)?;
    let dirty = mutate(&mut meta)?;
    if dirty {
        fsutil::write_atomic(&meta_path, &meta.to_bytes()?, "meta")?;
    }
    let db = Db::open(&ctx.home)?;
    db.upsert_asset(&meta)?;
    crate::assets::units::rebuild_for_asset(&db, &ctx.home, &meta.content_digest.as_hex())?;
    Ok(meta)
}

/// Update title or subkind; tags use the separate tag operation.
pub fn edit(ctx: &Ctx, hash: &str, title: Option<&str>, subkind: Option<&str>) -> Result<Value> {
    if title.is_none() && subkind.is_none() {
        return Err(AssetsError::bad_query("edit requires --title or --subkind"));
    }
    let meta = update_meta(ctx, hash, |m| {
        let mut dirty = false;
        if let Some(t) = title {
            if m.title.as_deref() != Some(t) {
                m.title = Some(t.to_owned());
                dirty = true;
            }
        }
        if let Some(s) = subkind {
            if m.kind != AssetKind::Audio {
                return Err(AssetsError::refused(format!(
                    "subkind only applies to audio assets (this asset is {})",
                    m.kind
                ))
                .with_hint(
                    "audio supports music and sfx subkinds; other asset kinds have no subkind",
                ));
            }
            if s != "music" && s != "sfx" {
                return Err(
                    AssetsError::bad_query(format!("unknown subkind '{s}'")).with_hint("music|sfx")
                );
            }
            if m.subkind.as_deref() != Some(s) {
                m.subkind = Some(s.to_owned());
                dirty = true;
            }
        }
        Ok(dirty)
    })?;
    Ok(json!({
        "content_digest": meta.content_digest,
        "title": meta.title,
        "subkind": meta.subkind
    }))
}

/// Add or remove tags with idempotent set semantics.
pub fn tag(ctx: &Ctx, hash: &str, add: &[String], rm: &[String]) -> Result<Value> {
    if add.is_empty() && rm.is_empty() {
        return Err(AssetsError::bad_query("tag requires --add or --rm"));
    }
    let meta = update_meta(ctx, hash, |m| {
        let mut dirty = false;
        for t in add {
            if !m.tags.contains(t) {
                m.tags.push(t.clone());
                dirty = true;
            }
        }
        for t in rm {
            let before = m.tags.len();
            m.tags.retain(|x| x != t);
            dirty |= m.tags.len() != before;
        }
        Ok(dirty)
    })?;
    Ok(json!({ "content_digest": meta.content_digest, "tags": meta.tags }))
}
