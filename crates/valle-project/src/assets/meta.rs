//! Authoritative asset metadata, one JSON file per content hash. SQLite is a projection. A removal
//! timestamp means bytes are gone but knowledge remains; adding the content again revives it.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ContentDigest;
use crate::assets::ctx::Probe;
use crate::assets::kind::AssetKind;
use crate::assets::report::{AssetsError, Result};

/// Location kind: managed CAS blob or external reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocationKind {
    Cas,
    Reference,
}

/// CAS paths are relative to the asset root; reference paths are absolute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub kind: LocationKind,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<String>,
    /// Reference validation baseline: modification time in milliseconds and byte size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// Authoritative asset metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetMeta {
    pub content_digest: ContentDigest,
    pub kind: AssetKind,
    /// Audio subkind, such as music or SFX, suggested heuristically and editable by users.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subkind: Option<String>,
    pub size: u64,
    pub original_name: String,
    pub added_at: String,
    /// Removal timestamp; adding the asset again clears it and restores locations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<Location>,
    /// Optional same-stem keyframe companion for a component.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub companion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<Probe>,
}

impl AssetMeta {
    /// Load an authoritative metadata file.
    pub fn load(path: &Path) -> Result<AssetMeta> {
        let bytes = std::fs::read(path).map_err(|e| {
            AssetsError::io(format!("failed to read metadata {}: {e}", path.display()))
        })?;
        let meta: AssetMeta = serde_json::from_slice(&bytes).map_err(|e| {
            AssetsError::io(format!("corrupt metadata {}: {e}", path.display()))
                .with_hint("avoid editing authoritative metadata directly; add the asset again to repair corrupt data")
        })?;
        let path_digest = content_digest_from_meta_path(path)?;
        if meta.content_digest != path_digest {
            return Err(AssetsError::io(format!(
                "metadata content identity does not match path {}: path {}, content {}",
                path.display(),
                path_digest,
                meta.content_digest
            ))
            .with_hint("add writes both the metadata path and content_digest; add the asset again to repair this mismatch"));
        }
        validate_content_paths(&meta, path)?;
        Ok(meta)
    }

    /// Serialize as readable, formatted JSON.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec_pretty(self)
            .map_err(|e| AssetsError::io(format!("failed to serialize metadata: {e}")))
    }

    pub fn is_removed(&self) -> bool {
        self.removed_at.is_some()
    }
}

fn content_digest_from_meta_path(path: &Path) -> Result<ContentDigest> {
    let shard = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str());
    let stem = path.file_stem().and_then(|value| value.to_str());
    let Some((shard, stem)) = shard.zip(stem) else {
        return Err(AssetsError::io(format!(
            "cannot derive content identity from metadata path: {}",
            path.display()
        )));
    };
    if path.extension().and_then(|value| value.to_str()) != Some("json")
        || shard.len() != 2
        || stem.len() != 62
    {
        return Err(AssetsError::io(format!(
            "metadata path is not a canonical content address: {}",
            path.display()
        ))
        .with_hint("metadata must be stored at meta/<2 lowercase hex>/<62 lowercase hex>.json"));
    }
    ContentDigest::from_hex(&format!("{shard}{stem}")).map_err(|_| {
        AssetsError::io(format!(
            "metadata path is not a canonical content address: {}",
            path.display()
        ))
        .with_hint("metadata must be stored at meta/<2 lowercase hex>/<62 lowercase hex>.json")
    })
}

fn validate_content_paths(meta: &AssetMeta, meta_path: &Path) -> Result<()> {
    let content_hex = meta.content_digest.as_hex();
    let (shard, stem) = content_hex.split_at(2);
    let object_prefix = format!("objects/{shard}/{stem}");

    for location in &meta.locations {
        match location.kind {
            LocationKind::Cas => {
                let valid = location.path == object_prefix
                    || location
                        .path
                        .strip_prefix(&format!("{object_prefix}."))
                        .is_some_and(|extension| {
                            !extension.is_empty()
                                && !extension.contains('/')
                                && !extension.contains('\\')
                        });
                if !valid {
                    return Err(AssetsError::io(format!(
                        "metadata CAS location does not match content identity {}: {} does not belong to {}",
                        meta_path.display(),
                        location.path,
                        meta.content_digest
                    ))
                    .with_hint("CAS locations must use objects/<digest fanout>[.<extension>]"));
                }
            }
            LocationKind::Reference => {
                if !Path::new(&location.path).is_absolute() {
                    return Err(AssetsError::io(format!(
                        "metadata reference location is not absolute {}: {}",
                        meta_path.display(),
                        location.path
                    ))
                    .with_hint(
                        "reference locations must be absolute paths canonicalized by the producer",
                    ));
                }
            }
        }
    }

    if let Some(companion) = &meta.companion {
        let expected = format!("{object_prefix}.keyframes.json");
        if companion != &expected {
            return Err(AssetsError::io(format!(
                "metadata companion does not match content identity {}: {} should be {}",
                meta_path.display(),
                companion,
                expected
            ))
            .with_hint("component companions must share the main content's digest path"));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_roundtrip_and_optional_fields_absent() {
        let content_digest = ContentDigest::from_hex(&"3f".repeat(32)).unwrap();
        let content_hex = content_digest.as_hex();
        let m = AssetMeta {
            content_digest,
            kind: AssetKind::Video,
            subkind: None,
            size: 812,
            original_name: "IMG_2031.MOV".into(),
            added_at: "2026-07-17T10:30:00.000Z".into(),
            removed_at: None,
            title: Some("Interview".into()),
            tags: vec!["Promo".into()],
            locations: vec![Location {
                kind: LocationKind::Cas,
                path: format!("objects/{}/{}.mp4", &content_hex[..2], &content_hex[2..]),
                verified_at: None,
                mtime_ms: None,
                size: None,
            }],
            companion: None,
            probe: None,
        };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["content_digest"], format!("sha256:{}", "3f".repeat(32)));
        assert!(v.get("hash").is_none());
        assert!(
            v.get("removed_at").is_none(),
            "omit None fields from storage"
        );
        assert_eq!(v["locations"][0]["kind"], "cas");
        let back: AssetMeta = serde_json::from_value(v).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn meta_rejects_unsupported_or_invalid_content_identity() {
        let mut value = serde_json::to_value(AssetMeta {
            content_digest: ContentDigest::from_hex(&"3f".repeat(32)).unwrap(),
            kind: AssetKind::Video,
            subkind: None,
            size: 1,
            original_name: "a.mp4".into(),
            added_at: "2026-07-17T10:30:00.000Z".into(),
            removed_at: None,
            title: None,
            tags: vec![],
            locations: vec![],
            companion: None,
            probe: None,
        })
        .unwrap();
        value.as_object_mut().unwrap().remove("content_digest");
        value["hash"] = serde_json::json!("3f".repeat(32));
        assert!(serde_json::from_value::<AssetMeta>(value).is_err());

        let invalid = serde_json::json!({
            "content_digest": format!("sha256:{}", "A".repeat(64)),
            "kind": "video",
            "size": 1,
            "original_name": "a.mp4",
            "added_at": "2026-07-17T10:30:00.000Z"
        });
        assert!(serde_json::from_value::<AssetMeta>(invalid).is_err());
    }

    #[test]
    fn load_rejects_digest_that_disagrees_with_meta_path() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp
            .path()
            .join("aa")
            .join(format!("{}.json", "0".repeat(62)));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let meta = AssetMeta {
            content_digest: ContentDigest::from_hex(&"bb".repeat(32)).unwrap(),
            kind: AssetKind::Video,
            subkind: None,
            size: 1,
            original_name: "a.mp4".into(),
            added_at: "2026-07-17T10:30:00.000Z".into(),
            removed_at: None,
            title: None,
            tags: vec![],
            locations: vec![],
            companion: None,
            probe: None,
        };
        std::fs::write(&path, meta.to_bytes().unwrap()).unwrap();
        assert!(
            AssetMeta::load(&path)
                .unwrap_err()
                .message
                .contains("content identity does not match path")
        );
    }

    #[test]
    fn load_rejects_noncanonical_meta_fanout_or_extension() {
        let temp = tempfile::tempdir().unwrap();
        let digest = ContentDigest::from_hex(&"ab".repeat(32)).unwrap();
        let meta = AssetMeta {
            content_digest: digest,
            kind: AssetKind::Video,
            subkind: None,
            size: 1,
            original_name: "a.mp4".into(),
            added_at: "2026-07-17T10:30:00.000Z".into(),
            removed_at: None,
            title: None,
            tags: vec![],
            locations: vec![],
            companion: None,
            probe: None,
        };
        let content_hex = digest.as_hex();

        let wrong_fanout = temp
            .path()
            .join(&content_hex[..1])
            .join(format!("{}.json", &content_hex[1..]));
        std::fs::create_dir_all(wrong_fanout.parent().unwrap()).unwrap();
        std::fs::write(&wrong_fanout, meta.to_bytes().unwrap()).unwrap();
        assert!(
            AssetMeta::load(&wrong_fanout)
                .unwrap_err()
                .message
                .contains("is not a canonical content address")
        );

        let wrong_extension = temp
            .path()
            .join(&content_hex[..2])
            .join(format!("{}.JSON", &content_hex[2..]));
        std::fs::create_dir_all(wrong_extension.parent().unwrap()).unwrap();
        std::fs::write(&wrong_extension, meta.to_bytes().unwrap()).unwrap();
        assert!(
            AssetMeta::load(&wrong_extension)
                .unwrap_err()
                .message
                .contains("is not a canonical content address")
        );
    }

    #[test]
    fn load_rejects_paths_that_violate_location_contract() {
        let temp = tempfile::tempdir().unwrap();
        let digest = ContentDigest::from_hex(&"ab".repeat(32)).unwrap();
        let content_hex = digest.as_hex();
        let path = temp
            .path()
            .join(&content_hex[..2])
            .join(format!("{}.json", &content_hex[2..]));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let canonical_object = format!("objects/{}/{}.mp4", &content_hex[..2], &content_hex[2..]);
        let mut meta = AssetMeta {
            content_digest: digest,
            kind: AssetKind::Video,
            subkind: None,
            size: 1,
            original_name: "a.mp4".into(),
            added_at: "2026-07-17T10:30:00.000Z".into(),
            removed_at: None,
            title: None,
            tags: vec![],
            locations: vec![Location {
                kind: LocationKind::Cas,
                path: "objects/ff/not-this-content.mp4".into(),
                verified_at: None,
                mtime_ms: None,
                size: None,
            }],
            companion: None,
            probe: None,
        };

        std::fs::write(&path, meta.to_bytes().unwrap()).unwrap();
        assert!(
            AssetMeta::load(&path)
                .unwrap_err()
                .message
                .contains("CAS location does not match content identity")
        );

        meta.locations[0].path = canonical_object;
        meta.companion = Some("objects/ff/not-this-content.keyframes.json".into());
        std::fs::write(&path, meta.to_bytes().unwrap()).unwrap();
        assert!(
            AssetMeta::load(&path)
                .unwrap_err()
                .message
                .contains("companion does not match content identity")
        );

        meta.companion = None;
        meta.locations[0].kind = LocationKind::Reference;
        meta.locations[0].path = "relative/a.mp4".into();
        std::fs::write(&path, meta.to_bytes().unwrap()).unwrap();
        assert!(
            AssetMeta::load(&path)
                .unwrap_err()
                .message
                .contains("reference location is not absolute")
        );
    }
}
