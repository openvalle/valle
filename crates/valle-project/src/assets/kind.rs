//! Storage kinds determine decoding, preview, and ingestion. Audio subkinds such as music and SFX
//! belong in metadata.

use serde::{Deserialize, Serialize};

use crate::assets::report::{AssetsError, Result};

/// Supported storage kinds. Unknown input requires an explicit kind or fails as unsupported media.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Video,
    Audio,
    Image,
    Font,
    /// Motion Scene3D's locked GLB subset. Timeline continues to carry it as generic `data`.
    Model3d,
    Lottie,
    /// JSX or HTML component with an optional same-stem keyframe companion.
    Component,
    Other,
}

impl AssetKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AssetKind::Video => "video",
            AssetKind::Audio => "audio",
            AssetKind::Image => "image",
            AssetKind::Font => "font",
            AssetKind::Model3d => "model3d",
            AssetKind::Lottie => "lottie",
            AssetKind::Component => "component",
            AssetKind::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Result<AssetKind> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "video" => AssetKind::Video,
            "audio" => AssetKind::Audio,
            "image" => AssetKind::Image,
            "font" => AssetKind::Font,
            "model3d" => AssetKind::Model3d,
            "lottie" => AssetKind::Lottie,
            "component" => AssetKind::Component,
            "other" => AssetKind::Other,
            other => {
                return Err(AssetsError::unsupported_media(format!(
                    "unknown asset kind '{other}'"
                ))
                .with_hint("video|audio|image|font|model3d|lottie|component|other"));
            }
        })
    }
}

impl std::fmt::Display for AssetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_lowercase_roundtrip() {
        for k in [
            AssetKind::Video,
            AssetKind::Audio,
            AssetKind::Image,
            AssetKind::Font,
            AssetKind::Model3d,
            AssetKind::Lottie,
            AssetKind::Component,
            AssetKind::Other,
        ] {
            let v = serde_json::to_value(k).unwrap();
            assert_eq!(v, serde_json::json!(k.as_str()));
            assert_eq!(AssetKind::parse(k.as_str()).unwrap(), k);
        }
        assert!(AssetKind::parse("psd").is_err());
    }
}
