//! Asset ingestion: lock, stream hash, resolve existing or removed state, detect kind, probe,
//! publish blob, atomically commit metadata, then update the index. Repeated adds update explicit
//! titles and merge tags. Removed assets revive with rebuilt locations. Probe failures occur before
//! publishing any files.

use std::io::Read;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::ContentDigest;
use crate::assets::clock;
use crate::assets::ctx::Ctx;
use crate::assets::db::Db;
use crate::assets::fsutil;
use crate::assets::home::fanout;
use crate::assets::kind::AssetKind;
use crate::assets::lock;
use crate::assets::meta::{AssetMeta, Location, LocationKind};
use crate::assets::report::{AssetsError, Result};
use crate::assets::verb::AddMode;

/// Asset-add result for the response envelope.
#[derive(Debug, Clone, Serialize)]
pub struct AddOutcome {
    pub content_digest: ContentDigest,
    pub kind: AssetKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subkind: Option<String>,
    /// Whether the asset was already registered.
    pub existed: bool,
    /// Whether the asset was restored from removed state.
    pub revived: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Ingest one file. The caller expands batches and handles each file independently.
pub fn add(
    ctx: &Ctx,
    src: &Path,
    mode: AddMode,
    kind_override: Option<AssetKind>,
    title: Option<&str>,
    tags: &[String],
) -> Result<AddOutcome> {
    if !src.exists() {
        return Err(
            AssetsError::not_found(format!("file does not exist: {}", src.display()))
                .with_hint("check the path"),
        );
    }
    if !src.is_file() {
        return Err(AssetsError::unsupported_media(format!(
            "not a file: {}",
            src.display()
        )));
    }

    let _lock = lock::acquire(&ctx.home)?;
    let content_digest = stream_sha256(src)?;
    let hash = content_digest.as_hex();
    let mut warnings = Vec::new();

    let meta_path = ctx.home.meta_path(&hash);
    if meta_path.exists() {
        return re_add(ctx, src, content_digest, mode, title, tags, &mut warnings);
    }

    // Determine kind, preferring an explicit override over extension or structural detection.
    let ext = file_ext(src);
    let kind = match kind_override.or_else(|| detect_kind(src, ext.as_deref())) {
        Some(k) => k,
        None => {
            return Err(AssetsError::unsupported_media(format!(
                "cannot identify asset kind: {}",
                src.display()
            ))
            .with_hint(
                "specify --kind video|audio|image|font|model3d|lottie|component|other to override detection",
            ));
        }
    };

    // Probe before writing so failure leaves no published files.
    let outcome = ctx.prober.probe(src, kind)?;
    warnings.extend(outcome.warnings);
    let probe = outcome.probe;

    // Audio shorter than ten seconds is an SFX candidate.
    let subkind = match (kind, &probe) {
        (AssetKind::Audio, Some(p)) => p.duration_ms.map(|d| {
            if d < 10_000 {
                "sfx".to_owned()
            } else {
                "music".to_owned()
            }
        }),
        _ => None,
    };

    // Publish CAS blobs and companions, or record reference locations with size and modification
    // time.
    let (location, companion) = match mode {
        AddMode::Reference => (make_reference_location(src)?, None),
        _ => {
            let blob_rel = publish_blob(ctx, src, &hash, ext.as_deref(), mode)?;
            let companion = publish_companion(ctx, src, &hash, kind)?;
            (
                Location {
                    kind: LocationKind::Cas,
                    path: blob_rel,
                    verified_at: None,
                    mtime_ms: None,
                    size: None,
                },
                companion,
            )
        }
    };

    let size = std::fs::metadata(src)?.len();
    let meta = AssetMeta {
        content_digest,
        kind,
        subkind: subkind.clone(),
        size,
        original_name: file_name(src),
        added_at: clock::iso8601(clock::now_millis()),
        removed_at: None,
        title: title.map(|s| s.to_owned()),
        tags: tags.to_vec(),
        locations: vec![location],
        companion,
        probe,
    };
    commit_meta_and_index(ctx, &meta)?;
    Ok(AddOutcome {
        content_digest,
        kind,
        subkind,
        existed: false,
        revived: false,
        warnings,
    })
}

/// Handle an existing asset or revive a removed one.
fn re_add(
    ctx: &Ctx,
    src: &Path,
    content_digest: ContentDigest,
    mode: AddMode,
    title: Option<&str>,
    tags: &[String],
    warnings: &mut Vec<String>,
) -> Result<AddOutcome> {
    let hash = content_digest.as_hex();
    let meta_path = ctx.home.meta_path(&hash);
    let mut meta = AssetMeta::load(&meta_path)?;
    let revived = meta.is_removed();
    let mut dirty = false;

    if revived {
        // Restore locations and use the current source's original filename.
        let location = if matches!(mode, AddMode::Reference) {
            make_reference_location(src)?
        } else {
            let ext = file_ext(src);
            let blob_rel = publish_blob(ctx, src, &hash, ext.as_deref(), mode)?;
            Location {
                kind: LocationKind::Cas,
                path: blob_rel,
                verified_at: None,
                mtime_ms: None,
                size: None,
            }
        };
        meta.locations = vec![location];
        meta.removed_at = None;
        meta.original_name = file_name(src);
        meta.size = std::fs::metadata(src)?.len();
        warnings
            .push("asset revived; annotations, analysis, title, and tags are retained".to_owned());
        dirty = true;
    }
    // Update an explicit title, union supplied tags, and preserve omitted fields.
    if let Some(t) = title {
        if meta.title.as_deref() != Some(t) {
            meta.title = Some(t.to_owned());
            dirty = true;
        }
    }
    for t in tags {
        if !meta.tags.contains(t) {
            meta.tags.push(t.clone());
            dirty = true;
        }
    }
    if dirty {
        commit_meta_and_index(ctx, &meta)?;
    } else {
        // Repair a stale index even when ingestion is otherwise idempotent.
        Db::open(&ctx.home)?.upsert_asset(&meta)?;
    }
    Ok(AddOutcome {
        content_digest,
        kind: meta.kind,
        subkind: meta.subkind.clone(),
        existed: true,
        revived,
        warnings: std::mem::take(warnings),
    })
}

/// Atomically commit metadata, then update the index and search-unit projection.
fn commit_meta_and_index(ctx: &Ctx, meta: &AssetMeta) -> Result<()> {
    let hash = meta.content_digest.as_hex();
    fsutil::write_atomic(&ctx.home.meta_path(&hash), &meta.to_bytes()?, "meta")?;
    let db = Db::open(&ctx.home)?;
    db.upsert_asset(meta)?;
    crate::assets::units::rebuild_for_asset(&db, &ctx.home, &hash)?;
    Ok(())
}

/// Publish a blob through a temporary file and rename; skip existing content-addressed objects.
fn publish_blob(
    ctx: &Ctx,
    src: &Path,
    hash: &str,
    ext: Option<&str>,
    mode: AddMode,
) -> Result<String> {
    let dest = ctx.home.object_path(hash, ext);
    let (d, _) = fanout(hash);
    let rel = format!(
        "objects/{d}/{}",
        dest.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
    );
    if dest.exists() {
        return Ok(rel);
    }
    let dir = dest.parent().expect("object path has a parent");
    std::fs::create_dir_all(dir)?;
    fsutil::stage_atomic(&dest, "blob", |staged| {
        match mode {
            AddMode::Reflink => {
                reflink_copy::reflink_or_copy(src, staged)
                    .map_err(|e| AssetsError::io(format!("failed to copy blob: {e}")))?;
            }
            AddMode::Copy => {
                let mut source = std::fs::File::open(src)?;
                let mut destination = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(staged)?;
                std::io::copy(&mut source, &mut destination)?;
            }
            AddMode::Reference => unreachable!("reference mode does not publish blobs"),
        }
        Ok(())
    })?;
    Ok(rel)
}

/// Publish a component's same-stem `.keyframes.json` companion under the main content hash.
fn publish_companion(ctx: &Ctx, src: &Path, hash: &str, kind: AssetKind) -> Result<Option<String>> {
    if kind != AssetKind::Component {
        return Ok(None);
    }
    let comp = src.with_extension("keyframes.json");
    if !comp.is_file() {
        return Ok(None);
    }
    let dest = ctx.home.object_path(hash, Some("keyframes.json"));
    if !dest.exists() {
        std::fs::create_dir_all(dest.parent().expect("path has a parent"))?;
        fsutil::stage_atomic(&dest, "companion", |staged| {
            reflink_copy::reflink_or_copy(&comp, staged)
                .map_err(|e| AssetsError::io(format!("failed to copy companion: {e}")))?;
            Ok(())
        })?;
    }
    let (d, _) = fanout(hash);
    Ok(Some(format!(
        "objects/{d}/{}",
        dest.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
    )))
}

/// Record an absolute reference path and size/mtime baseline.
fn make_reference_location(src: &Path) -> Result<Location> {
    let abs = std::fs::canonicalize(src)?;
    let md = std::fs::metadata(&abs)?;
    let mtime_ms = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    Ok(Location {
        kind: LocationKind::Reference,
        path: abs.to_string_lossy().into_owned(),
        verified_at: None,
        mtime_ms,
        size: Some(md.len()),
    })
}

/// Ingest files independently so one failure does not stop the batch.
pub fn add_batch(
    ctx: &Ctx,
    paths: &[std::path::PathBuf],
    mode: AddMode,
    kind_override: Option<AssetKind>,
    title: Option<&str>,
    tags: &[String],
) -> Vec<BatchItem> {
    paths
        .iter()
        .map(|p| {
            let display = p.display().to_string();
            match add(ctx, p, mode, kind_override, title, tags) {
                Ok(outcome) => BatchItem {
                    path: display,
                    outcome: Some(outcome),
                    error: None,
                },
                Err(e) => BatchItem {
                    path: display,
                    outcome: None,
                    error: Some(crate::assets::report::ErrorBody {
                        code: e.code,
                        message: e.message,
                        hint: e.hint,
                    }),
                },
            }
        })
        .collect()
}

/// One batch result.
#[derive(Debug, Serialize)]
pub struct BatchItem {
    pub path: String,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<AddOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<crate::assets::report::ErrorBody>,
}

/// Compute SHA-256 with 64 KiB streaming reads.
pub fn stream_sha256(path: &Path) -> Result<ContentDigest> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(ContentDigest::from_bytes(hasher.finalize().into()))
}

fn file_ext(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_owned()
}

/// Detect kind primarily by extension; identify Lottie JSON by its structure.
fn detect_kind(path: &Path, ext: Option<&str>) -> Option<AssetKind> {
    match ext {
        Some("mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v") => Some(AssetKind::Video),
        Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tiff" | "tif" | "heic") => {
            Some(AssetKind::Image)
        }
        Some("mp3" | "wav" | "m4a" | "aac" | "flac" | "ogg" | "opus") => Some(AssetKind::Audio),
        Some("ttf" | "otf" | "woff" | "woff2") => Some(AssetKind::Font),
        Some("glb") => Some(AssetKind::Model3d),
        Some("jsx" | "html") => Some(AssetKind::Component),
        Some("json") => {
            if looks_like_lottie(path) {
                Some(AssetKind::Lottie)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Detect top-level `v` and `layers` fields, reading at most 1 MiB.
fn looks_like_lottie(path: &Path) -> bool {
    let Ok(f) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = Vec::new();
    if f.take(1024 * 1024).read_to_end(&mut buf).is_err() {
        return false;
    }
    match serde_json::from_slice::<serde_json::Value>(&buf) {
        Ok(v) => v.get("v").is_some() && v.get("layers").is_some(),
        Err(_) => false,
    }
}
