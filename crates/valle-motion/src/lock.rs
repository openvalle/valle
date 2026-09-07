use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::artifact::{SceneArtifact, ValidationError};
use super::canonical::{CanonicalError, canonical_bytes};
use super::controls::AssetKind;
use super::domain::MotionViewport;
use crate::ContentDigest;

pub const MOTION_BUNDLE_FORMAT_VERSION: u32 = 1;

/// Digest of canonical [`ArtifactEnvelope`] bytes.
///
/// This semantic wrapper still stores the shared byte-backed
/// [`ContentDigest`]. Its JSON representation is therefore the same strict
/// project-wide wire. Content-addressed paths must opt into bare hexadecimal
/// through [`Self::as_hex`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EnvelopeDigest(ContentDigest);

impl EnvelopeDigest {
    fn of_bytes(bytes: &[u8]) -> Self {
        Self(ContentDigest::of_bytes(bytes))
    }

    pub fn as_hex(&self) -> String {
        self.0.as_hex()
    }

    pub fn to_wire(&self) -> String {
        self.0.to_wire()
    }
}

impl core::fmt::Display for EnvelopeDigest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundledAsset {
    pub kind: AssetKind,
    pub content_hash: ContentDigest,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundledFont {
    pub content_hash: ContentDigest,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundledSource {
    pub content_hash: ContentDigest,
    pub path: String,
}

/// Manifest for the self-checking author export emitted by `valle motion build`.
///
/// Every member path is bundle-relative and its meaningful bytes are pinned. This is compiler
/// output for inspection and reproducible handoff, not renderer/runtime admission; production
/// rendering remains governed by Fixed Package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionBundleManifest {
    pub format_version: u32,
    /// Logical authoring Canvas used to compile and bake layout into the Artifact.
    pub canvas_size: MotionViewport,
    pub component: String,
    pub envelope_digest: EnvelopeDigest,
    pub closure_digest: ContentDigest,
    pub entry: String,
    pub source_path: String,
    pub modules: BTreeMap<String, BundledSource>,
    pub artifact_path: String,
    pub source_map_path: String,
    pub source_map_digest: ContentDigest,
    /// Frozen prepare input, when the component declares and consumes `controls.data`.
    pub data_path: Option<String>,
    pub data_source: Option<String>,
    pub data_digest: Option<ContentDigest>,
    pub assets: BTreeMap<String, BundledAsset>,
    pub fonts: Vec<BundledFont>,
}

impl MotionBundleManifest {
    pub fn validate(&self) -> Result<(), LockError> {
        if self.format_version != MOTION_BUNDLE_FORMAT_VERSION {
            return Err(LockError::Manifest(format!(
                "bundle format {} does not match {}",
                self.format_version, MOTION_BUNDLE_FORMAT_VERSION
            )));
        }
        if self.component.is_empty() || self.fonts.is_empty() {
            return Err(LockError::Manifest(
                "component and at least one deterministic font are required".into(),
            ));
        }
        if !canvas_size_is_valid(self.canvas_size) {
            return Err(LockError::Manifest(
                "canvasSize must be positive and fit Skia's i32 domain".into(),
            ));
        }
        let expected_artifact = format!("artifacts/{}.json", self.envelope_digest.as_hex());
        let expected_source = format!("sources/{}", self.entry);
        for (actual, expected) in [
            (self.source_path.as_str(), expected_source.as_str()),
            (self.artifact_path.as_str(), expected_artifact.as_str()),
            (self.source_map_path.as_str(), "source-map.json"),
        ] {
            if actual != expected {
                return Err(LockError::Manifest(format!(
                    "bundle path `{actual}` must be `{expected}`"
                )));
            }
        }
        match (&self.data_path, &self.data_source, &self.data_digest) {
            (None, None, None) => {}
            (Some(path), Some(source), Some(_))
                if path == "data.json" && !source.trim().is_empty() => {}
            _ => {
                return Err(LockError::Manifest(
                    "dataPath/dataSource/dataDigest must be absent together or identify data.json"
                        .into(),
                ));
            }
        }
        if self.entry.is_empty()
            || self.modules.is_empty()
            || !self.modules.contains_key(&self.entry)
        {
            return Err(LockError::Manifest(
                "entry must identify one module in the non-empty source closure".into(),
            ));
        }
        for (module, source) in &self.modules {
            if module.is_empty()
                || module.starts_with('/')
                || module.contains('\\')
                || module
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
                || source.path != format!("sources/{module}")
            {
                return Err(LockError::Manifest(format!(
                    "module `{module}` must use a normalized project-relative name and path `sources/{module}`"
                )));
            }
        }
        for (name, asset) in &self.assets {
            if name.is_empty() || asset.path != format!("resources/{}", asset.content_hash.as_hex())
            {
                return Err(LockError::Manifest(format!(
                    "asset `{name}` must use its content-addressed resource path"
                )));
            }
        }
        let mut fonts = BTreeSet::new();
        for font in &self.fonts {
            if font.path != format!("fonts/{}", font.content_hash.as_hex())
                || !fonts.insert(font.content_hash)
            {
                return Err(LockError::Manifest(
                    "fonts must be unique and content-addressed".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BuildFingerprint {
    pub compiler_version: String,
    pub math_engine: String,
    pub normalized_ast_digest: ContentDigest,
    pub prepared_data_digest: ContentDigest,
    pub assets_digest: ContentDigest,
    /// Logical authoring Canvas is a compile input because layout values are baked into Artifact.
    pub canvas_size: MotionViewport,
    pub layout_engine: String,
    /// Independent of [`Self::math_engine`] (`valle_draw::math`). Empty when the
    /// artifact has no `MathFormula` node.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub formula_layout_engine: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactEnvelope {
    pub build_fingerprint: BuildFingerprint,
    pub artifact: SceneArtifact,
}

impl ArtifactEnvelope {
    pub fn validate(&self) -> Result<(), LockError> {
        self.artifact.validate().map_err(LockError::Artifact)?;
        let fingerprint = &self.build_fingerprint;
        if fingerprint.compiler_version.is_empty() || fingerprint.layout_engine.is_empty() {
            return Err(LockError::Fingerprint(
                "compilerVersion and layoutEngine must be non-empty".into(),
            ));
        }
        if !canvas_size_is_valid(fingerprint.canvas_size) {
            return Err(LockError::Fingerprint(
                "canvasSize must be positive and fit Skia's i32 domain".into(),
            ));
        }
        if fingerprint.math_engine != valle_draw::math::ENGINE_ID {
            return Err(LockError::Fingerprint(format!(
                "mathEngine must exactly match {}",
                valle_draw::math::ENGINE_ID
            )));
        }
        let uses_formula = self
            .artifact
            .nodes
            .iter()
            .any(|node| matches!(node.kind, crate::NodeKind::MathFormula { .. }));
        if uses_formula {
            if fingerprint.formula_layout_engine.is_empty() {
                return Err(LockError::Fingerprint(
                    "formulaLayoutEngine must be set when MathFormula is present".into(),
                ));
            }
        } else if !fingerprint.formula_layout_engine.is_empty() {
            return Err(LockError::Fingerprint(
                "formulaLayoutEngine must be empty when the scene has no MathFormula".into(),
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, LockError> {
        self.validate()?;
        canonical_bytes(self).map_err(LockError::Canonical)
    }

    pub fn envelope_digest(&self) -> Result<EnvelopeDigest, LockError> {
        Ok(EnvelopeDigest::of_bytes(&self.canonical_bytes()?))
    }
}

fn canvas_size_is_valid(size: MotionViewport) -> bool {
    size.is_valid_pixel_extent()
}

#[derive(Debug, Clone, PartialEq)]
pub enum LockError {
    Artifact(Vec<ValidationError>),
    Canonical(CanonicalError),
    Fingerprint(String),
    Manifest(String),
}

impl core::fmt::Display for LockError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for LockError {}
