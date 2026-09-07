//! Resolve a full hash or prefix to an absolute path. Validate references by size and modification
//! time; removed assets return a revival hint. Do not silently rehash stale references.

use serde::Serialize;

use crate::ContentDigest;
use crate::assets::addr;
use crate::assets::ctx::Ctx;
use crate::assets::meta::{AssetMeta, LocationKind};
use crate::assets::report::{AssetsError, Result};

#[derive(Debug, Clone, Serialize)]
pub struct ResolveOutcome {
    pub content_digest: ContentDigest,
    pub path: String,
    pub location: &'static str,
}

pub fn resolve(ctx: &Ctx, hash_or_prefix: &str) -> Result<ResolveOutcome> {
    let hash = addr::find_hash(&ctx.home, hash_or_prefix)?;
    let meta = AssetMeta::load(&ctx.home.meta_path(&hash))?;
    let content_digest = meta.content_digest;
    if meta.is_removed() {
        return Err(
            AssetsError::not_found(format!("asset {} is removed", &hash[..12])).with_hint(
                "knowledge is retained; add a file with the same content to revive this asset",
            ),
        );
    }
    let loc = meta.locations.first().ok_or_else(|| {
        AssetsError::io(format!("metadata has no locations: {}", &hash[..12]))
            .with_hint("stored metadata is inconsistent; run `valle assets verify`")
    })?;
    match loc.kind {
        LocationKind::Cas => {
            let abs = ctx.home.assets_dir().join(&loc.path);
            if !abs.exists() {
                return Err(
                    AssetsError::io(format!("blob missing: {}", abs.display())).with_hint(
                        "run `valle assets verify`; add the same content again to repair the asset",
                    ),
                );
            }
            Ok(ResolveOutcome {
                content_digest,
                path: abs.to_string_lossy().into_owned(),
                location: "cas",
            })
        }
        LocationKind::Reference => {
            let p = std::path::Path::new(&loc.path);
            let md = std::fs::metadata(p).map_err(|_| {
                AssetsError::stale_reference(format!("reference source is missing: {}", loc.path))
                    .with_hint("the source was moved or deleted; add its new location to retain the same content identity")
            })?;
            let mtime_ms = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64);
            let unchanged = loc.size == Some(md.len()) && loc.mtime_ms == mtime_ms;
            if !unchanged {
                return Err(AssetsError::stale_reference(format!(
                    "reference source changed (mtime or size mismatch): {}",
                    loc.path
                ))
                .with_hint(
                    "the bytes may differ from the registered content; add the source again",
                ));
            }
            Ok(ResolveOutcome {
                content_digest,
                path: loc.path.clone(),
                location: "reference",
            })
        }
    }
}
