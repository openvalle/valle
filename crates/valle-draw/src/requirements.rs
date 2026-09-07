//! A validated [`DrawProgram`](crate::program::DrawProgram)'s complete execution contract.
//!
//! These values identify external data. They never contain a decoder, GPU object, browser
//! object, future, callback, or other platform state.

use serde::{Deserialize, Serialize};

use crate::{
    Rect,
    program::{BackdropScope, BlendMode, NodeId},
};

/// A validated 32-byte digest transported by the DrawProgram wire.
///
/// Draw stays independent from Timeline document semantics, so its internal
/// contract carries digest bytes rather than importing `ContentDigest`. The
/// serialized spelling remains lowercase bare hex because DrawProgram is a
/// closed binary/transport format, not a public content-identity API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DigestBytes([u8; 32]);

impl DigestBytes {
    pub fn from_hex(value: &str) -> Option<Self> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return None;
        }
        let mut bytes = [0_u8; 32];
        hex::decode_to_slice(value, &mut bytes).ok()?;
        Some(Self(bytes))
    }

    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub const fn into_bytes(self) -> [u8; 32] {
        self.0
    }

    pub fn as_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl Serialize for DigestBytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.as_hex())
    }
}

impl<'de> Deserialize<'de> for DigestBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_hex(&value).ok_or_else(|| {
            serde::de::Error::custom("digest must be 64 lowercase hexadecimal digits")
        })
    }
}

#[cfg(test)]
mod digest_tests {
    use super::DigestBytes;

    #[test]
    fn digest_bytes_wire_is_exact_lowercase_bare_hex() {
        let hex = "0123456789abcdef".repeat(4);
        let digest = DigestBytes::from_hex(&hex).expect("canonical digest");

        assert_eq!(
            serde_json::to_string(&digest).unwrap(),
            format!("\"{hex}\"")
        );
        assert_eq!(
            serde_json::from_str::<DigestBytes>(&format!("\"{hex}\"")).unwrap(),
            digest
        );

        for invalid in ["abc".to_owned(), "A".repeat(64), format!("sha256:{hex}")] {
            assert!(serde_json::from_str::<DigestBytes>(&format!("\"{invalid}\"")).is_err());
        }
    }
}

/// Canonical program-local bounds. Empty is distinct from a zero-sized rectangle at an arbitrary
/// coordinate so hashes and downstream diagnostics never have to guess intent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum LocalBounds {
    Empty,
    Finite { rect: Rect },
}

impl LocalBounds {
    pub const EMPTY: Self = Self::Empty;

    pub fn from_rect(rect: Rect) -> Self {
        if rect.is_empty() {
            Self::Empty
        } else {
            Self::Finite { rect }
        }
    }

    pub const fn rect(self) -> Option<Rect> {
        match self {
            Self::Empty => None,
            Self::Finite { rect } => Some(rect),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorDomain {
    /// Valle's only compositor working space.
    LinearRec2020,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AlphaMode {
    Opaque,
    Straight,
    Premultiplied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextureKind {
    Image,
    Video,
    Generated,
}

/// Semantic texture identity plus its interpretation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalTexture {
    pub key: String,
    pub kind: TextureKind,
    pub color_domain: ColorDomain,
    pub alpha: AlphaMode,
    /// Exact source timestamp for a frame-evaluated video texture, in microseconds.
    /// Static and host-generated textures use `None`.
    pub sample_time_micros: Option<i64>,
}

/// Stable font face identity. Glyph shaping is completed before execution.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontKey {
    pub face_hash: DigestBytes,
    pub face_index: u32,
}

/// Runtime shader identity and frozen ABI. Source code is deliberately absent.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeShaderKey {
    pub uri: String,
    pub content_hash: DigestBytes,
    pub abi_hash: DigestBytes,
}

/// Validated Scene3D payload identity. Platform scene objects live in the host table.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scene3dKey {
    /// Hash of the complete frame-local Scene3D request: static scene, evaluated frame state and
    /// target extent. Different times or sizes are different external resources.
    pub content_hash: DigestBytes,
    /// Hash of the static topology, allowing hosts to cache admitted geometry independently from
    /// the frame-local raster.
    pub topology_hash: DigestBytes,
}

/// Local-space conservative outset. Device-space resolution belongs to Engine prepare.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Insets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Insets {
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub const fn uniform(value: f32) -> Self {
        Self::new(value, value, value, value)
    }

    pub(crate) fn include(&mut self, other: Self) {
        self.left = self.left.max(other.left);
        self.top = self.top.max(other.top);
        self.right = self.right.max(other.right);
        self.bottom = self.bottom.max(other.bottom);
    }

    pub(crate) fn added(self, other: Self) -> Self {
        Self::new(
            self.left + other.left,
            self.top + other.top,
            self.right + other.right,
            self.bottom + other.bottom,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SamplingMode {
    NearestClamp,
    LinearClamp,
    LinearDecal,
    CubicClamp,
}

/// Why a DrawProgram node needs the visible destination at its exact execution point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum DestinationOperation {
    /// The destination becomes local source content, optionally with a spatial footprint.
    Backdrop {
        footprint: Insets,
        sampling: SamplingMode,
    },
    /// The effective source color is computed against the visible destination.
    Blend { mode: BlendMode },
}

impl DestinationOperation {
    pub const fn footprint(&self) -> Insets {
        match self {
            Self::Backdrop { footprint, .. } => *footprint,
            Self::Blend { .. } => Insets::new(0.0, 0.0, 0.0, 0.0),
        }
    }
}

/// One validator-derived read from an immutable composition version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DestinationUse {
    pub node: NodeId,
    pub scope: BackdropScope,
    /// Pixels changed by the destination-dependent operator.
    pub output_bounds: Rect,
    /// Local conservative read domain; Engine resolves it again after projective device mapping.
    pub sample_bounds: Rect,
    pub operation: DestinationOperation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DrawCapability {
    ExternalTexture,
    FontGlyphs,
    RuntimeShader,
    Scene3d,
    BackdropRead,
    TransformAffine,
    TransformProjective,
    ClipRect,
    ClipRoundRect,
    ClipPath,
    FilterBlur,
    FilterColorMatrix,
    FilterDropShadow,
    FilterNoiseDisplacement,
    FilterVelocityBlur,
    MaskAlpha,
    MaskLuminance,
    Blend,
    GradientConic,
    GradientSpread,
    StrokeDash,
    GeometryBatch,
    BoxShadow,
    GroupShader,
    MotionGlass,
}

/// This is output of validation, never author input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DrawRequirements {
    pub content_bounds: LocalBounds,
    pub output_bounds: LocalBounds,
    pub external_textures: Vec<ExternalTexture>,
    pub fonts: Vec<FontKey>,
    pub runtime_shaders: Vec<RuntimeShaderKey>,
    pub scene3d: Vec<Scene3dKey>,
    pub destination_uses: Vec<DestinationUse>,
    pub filter_footprint: Insets,
    pub color_domains: Vec<ColorDomain>,
    pub alpha_modes: Vec<AlphaMode>,
    pub capabilities: Vec<DrawCapability>,
    /// Largest single local intermediate implied by groups/filters/masks/backdrop operators.
    pub max_intermediate_pixels: u64,
}

impl Default for DrawRequirements {
    fn default() -> Self {
        Self {
            content_bounds: LocalBounds::Empty,
            output_bounds: LocalBounds::Empty,
            external_textures: Vec::new(),
            fonts: Vec::new(),
            runtime_shaders: Vec::new(),
            scene3d: Vec::new(),
            destination_uses: Vec::new(),
            filter_footprint: Insets::default(),
            // DrawProgram paint/composition semantics always use this internal contract.
            color_domains: vec![ColorDomain::LinearRec2020],
            alpha_modes: vec![AlphaMode::Premultiplied],
            capabilities: Vec::new(),
            max_intermediate_pixels: 0,
        }
    }
}
