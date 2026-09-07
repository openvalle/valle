use std::{collections::BTreeMap, fmt, sync::Arc};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use valle_timeline::RationalTime;

use super::SignalLuminance;
use super::{
    ColorDescription, ContentDigest, InputAlphaMode, OperatorAlphaBehavior, OperatorColorDomain,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "Extent2dWire", into = "Extent2dWire")]
pub struct Extent2d {
    width: u32,
    height: u32,
}

impl Extent2d {
    pub fn new(width: u32, height: u32) -> Result<Self, DescriptorError> {
        if width == 0 || height == 0 {
            return Err(DescriptorError::EmptyExtent);
        }
        Ok(Self { width, height })
    }

    pub const fn width(self) -> u32 {
        self.width
    }

    pub const fn height(self) -> u32 {
        self.height
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Extent2dWire {
    width: u32,
    height: u32,
}

impl TryFrom<Extent2dWire> for Extent2d {
    type Error = DescriptorError;

    fn try_from(value: Extent2dWire) -> Result<Self, Self::Error> {
        Self::new(value.width, value.height)
    }
}

impl From<Extent2d> for Extent2dWire {
    fn from(value: Extent2d) -> Self {
        Self {
            width: value.width,
            height: value.height,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "RatioWire", into = "RatioWire")]
pub struct SampleAspectRatio {
    numerator: u32,
    denominator: u32,
}

impl SampleAspectRatio {
    pub const SQUARE: Self = Self {
        numerator: 1,
        denominator: 1,
    };

    pub fn new(numerator: u32, denominator: u32) -> Result<Self, DescriptorError> {
        if numerator == 0 || denominator == 0 {
            return Err(DescriptorError::InvalidRatio);
        }
        let divisor = gcd(numerator, denominator);
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    pub const fn denominator(self) -> u32 {
        self.denominator
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RatioWire {
    numerator: u32,
    denominator: u32,
}

impl TryFrom<RatioWire> for SampleAspectRatio {
    type Error = DescriptorError;

    fn try_from(value: RatioWire) -> Result<Self, Self::Error> {
        Self::new(value.numerator, value.denominator)
    }
}

impl From<SampleAspectRatio> for RatioWire {
    fn from(value: SampleAspectRatio) -> Self {
        Self {
            numerator: value.numerator,
            denominator: value.denominator,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PixelOrientation {
    Identity,
    MirrorHorizontal,
    Rotate180,
    MirrorVertical,
    MirrorHorizontalRotate270,
    Rotate90,
    MirrorHorizontalRotate90,
    Rotate270,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "UnitFractionWire", into = "UnitFractionWire")]
pub struct UnitFraction {
    numerator: u32,
    denominator: u32,
}

impl UnitFraction {
    pub const ZERO: Self = Self {
        numerator: 0,
        denominator: 1,
    };
    pub const ONE: Self = Self {
        numerator: 1,
        denominator: 1,
    };

    pub fn new(numerator: u32, denominator: u32) -> Result<Self, DescriptorError> {
        if denominator == 0 || numerator > denominator {
            return Err(DescriptorError::InvalidUnitFraction);
        }
        let divisor = gcd(numerator, denominator);
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    pub const fn denominator(self) -> u32 {
        self.denominator
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnitFractionWire {
    numerator: u32,
    denominator: u32,
}

impl TryFrom<UnitFractionWire> for UnitFraction {
    type Error = DescriptorError;

    fn try_from(value: UnitFractionWire) -> Result<Self, Self::Error> {
        Self::new(value.numerator, value.denominator)
    }
}

impl From<UnitFraction> for UnitFractionWire {
    fn from(value: UnitFraction) -> Self {
        Self {
            numerator: value.numerator,
            denominator: value.denominator,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "NormalizedCropWire", into = "NormalizedCropWire")]
pub struct NormalizedCrop {
    left: UnitFraction,
    top: UnitFraction,
    right: UnitFraction,
    bottom: UnitFraction,
}

impl NormalizedCrop {
    pub const FULL: Self = Self {
        left: UnitFraction::ZERO,
        top: UnitFraction::ZERO,
        right: UnitFraction::ONE,
        bottom: UnitFraction::ONE,
    };

    pub fn new(
        left: UnitFraction,
        top: UnitFraction,
        right: UnitFraction,
        bottom: UnitFraction,
    ) -> Result<Self, DescriptorError> {
        if !fraction_less(left, right) || !fraction_less(top, bottom) {
            return Err(DescriptorError::EmptyCrop);
        }
        Ok(Self {
            left,
            top,
            right,
            bottom,
        })
    }

    pub const fn left(self) -> UnitFraction {
        self.left
    }

    pub const fn top(self) -> UnitFraction {
        self.top
    }

    pub const fn right(self) -> UnitFraction {
        self.right
    }

    pub const fn bottom(self) -> UnitFraction {
        self.bottom
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NormalizedCropWire {
    left: UnitFraction,
    top: UnitFraction,
    right: UnitFraction,
    bottom: UnitFraction,
}

impl TryFrom<NormalizedCropWire> for NormalizedCrop {
    type Error = DescriptorError;

    fn try_from(value: NormalizedCropWire) -> Result<Self, Self::Error> {
        Self::new(value.left, value.top, value.right, value.bottom)
    }
}

impl From<NormalizedCrop> for NormalizedCropWire {
    fn from(value: NormalizedCrop) -> Self {
        Self {
            left: value.left,
            top: value.top,
            right: value.right,
            bottom: value.bottom,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualInterpretation {
    pub color: ColorDescription,
    pub luminance: SignalLuminance,
    pub alpha: InputAlphaMode,
    pub orientation: PixelOrientation,
    pub sample_aspect_ratio: SampleAspectRatio,
    pub crop: NormalizedCrop,
}

impl VisualInterpretation {
    pub const fn new(
        color: ColorDescription,
        luminance: SignalLuminance,
        alpha: InputAlphaMode,
    ) -> Self {
        Self {
            color,
            luminance,
            alpha,
            orientation: PixelOrientation::Identity,
            sample_aspect_ratio: SampleAspectRatio::SQUARE,
            crop: NormalizedCrop::FULL,
        }
    }

    pub const fn with_orientation(mut self, orientation: PixelOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    pub const fn with_sample_aspect_ratio(mut self, ratio: SampleAspectRatio) -> Self {
        self.sample_aspect_ratio = ratio;
        self
    }

    pub const fn with_crop(mut self, crop: NormalizedCrop) -> Self {
        self.crop = crop;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "MediaDescriptorWire", into = "MediaDescriptorWire")]
pub struct MediaDescriptor {
    extent: Option<Extent2d>,
    duration: Option<RationalTime>,
    visual: Option<VisualInterpretation>,
}

impl MediaDescriptor {
    pub fn visual(
        extent: Extent2d,
        duration: Option<RationalTime>,
        interpretation: VisualInterpretation,
    ) -> Result<Self, DescriptorError> {
        let descriptor = Self {
            extent: Some(extent),
            duration,
            visual: Some(interpretation),
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub fn image_srgb(
        width: u32,
        height: u32,
        alpha: InputAlphaMode,
    ) -> Result<Self, DescriptorError> {
        Self::visual(
            Extent2d::new(width, height)?,
            None,
            VisualInterpretation::new(ColorDescription::SRGB, SignalLuminance::SDR_100, alpha),
        )
    }

    pub fn video(
        width: u32,
        height: u32,
        duration: RationalTime,
        interpretation: VisualInterpretation,
    ) -> Result<Self, DescriptorError> {
        Self::visual(
            Extent2d::new(width, height)?,
            Some(duration),
            interpretation,
        )
    }

    pub fn audio(duration: RationalTime) -> Result<Self, DescriptorError> {
        let descriptor = Self {
            extent: None,
            duration: Some(duration),
            visual: None,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub const fn extent(&self) -> Option<Extent2d> {
        self.extent
    }

    pub const fn duration(&self) -> Option<RationalTime> {
        self.duration
    }

    pub const fn visual_interpretation(&self) -> Option<VisualInterpretation> {
        self.visual
    }

    fn validate(&self) -> Result<(), DescriptorError> {
        if self.extent.is_some() != self.visual.is_some() {
            return Err(DescriptorError::IncompleteVisualDescriptor);
        }
        if self.duration.is_some_and(RationalTime::is_non_positive) {
            return Err(DescriptorError::InvalidDuration);
        }
        Ok(())
    }

    fn validate_for(&self, kind: SemanticAssetKind) -> Result<(), DescriptorError> {
        self.validate()?;
        match kind {
            SemanticAssetKind::Video | SemanticAssetKind::Lottie => {
                if self.visual.is_none() || self.duration.is_none() {
                    return Err(DescriptorError::KindMismatch);
                }
            }
            SemanticAssetKind::Image => {
                if self.visual.is_none() || self.duration.is_some() {
                    return Err(DescriptorError::KindMismatch);
                }
            }
            SemanticAssetKind::Audio => {
                if self.visual.is_some() || self.duration.is_none() {
                    return Err(DescriptorError::KindMismatch);
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MediaDescriptorWire {
    extent: Option<Extent2d>,
    duration: Option<RationalTime>,
    visual: Option<VisualInterpretation>,
}

impl TryFrom<MediaDescriptorWire> for MediaDescriptor {
    type Error = DescriptorError;

    fn try_from(value: MediaDescriptorWire) -> Result<Self, Self::Error> {
        let descriptor = Self {
            extent: value.extent,
            duration: value.duration,
            visual: value.visual,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }
}

impl From<MediaDescriptor> for MediaDescriptorWire {
    fn from(value: MediaDescriptor) -> Self {
        Self {
            extent: value.extent,
            duration: value.duration,
            visual: value.visual,
        }
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorError {
    #[error("pixel extent must be non-empty")]
    EmptyExtent,
    #[error("sample aspect ratio must contain two positive integers")]
    InvalidRatio,
    #[error("unit fraction must be within [0, 1] and have a non-zero denominator")]
    InvalidUnitFraction,
    #[error("normalized crop must have positive width and height")]
    EmptyCrop,
    #[error("extent and visual interpretation must either both be present or both be absent")]
    IncompleteVisualDescriptor,
    #[error("media duration must be positive when present")]
    InvalidDuration,
    #[error("media descriptor does not match the semantic asset kind")]
    KindMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SemanticAssetKind {
    Video,
    Image,
    Audio,
    Lottie,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticAsset {
    pub id: String,
    pub kind: SemanticAssetKind,
    pub digest: ContentDigest,
    pub descriptor: MediaDescriptor,
}

/// Frozen semantic inputs referenced from one Motion instance.
///
/// The authored component binding name is retained because a validated `DrawProgram` refers to
/// that name, while the value is the immutable asset identity captured when the render input is
/// compiled. Prepare therefore never has to reopen the Timeline or consult a host registry.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionResourceSnapshot {
    pub assets: BTreeMap<String, SemanticAsset>,
    /// Font controls are semantic shaping inputs, not executor textures. Keeping their exact
    /// control-to-face binding here makes the compiled render independent from Timeline lookup
    /// while the emitted DrawProgram continues to use the content-derived family alias.
    pub fonts: BTreeMap<String, SemanticFont>,
}

impl SemanticAsset {
    pub fn new(
        id: impl Into<String>,
        kind: SemanticAssetKind,
        digest: ContentDigest,
        descriptor: MediaDescriptor,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            digest,
            descriptor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticFont {
    pub family: String,
    pub digest: ContentDigest,
    pub face_index: u32,
}

/// The deterministic family used when authored text does not name a font explicitly.
///
/// This belongs to the semantic resource contract rather than timeline compilation: Motion,
/// caption shaping and every host preflight must agree on the same family before a render is
/// opened.
pub const DEFAULT_FONT_FAMILY: &str = "Valle Default";

/// Render-local identity for a Motion Artifact specialized to one Timeline clip. The authored
/// component URI remains unchanged; compilation prefers this exact instance and falls back to a
/// component-wide structure only when no specialization was registered.
pub fn motion_instance_key(source: &str, clip_id: &str) -> String {
    format!("{source}#clip:{clip_id}")
}

/// Immutable bytes for one content-addressed font face. The descriptor remains the only value
/// serialized into frame/render identities; these bytes are render-owned semantic input used
/// by Motion layout and caption shaping before an executor is involved.
#[derive(Clone)]
pub struct SemanticFontPayload {
    bytes: Arc<Vec<u8>>,
}

impl fmt::Debug for SemanticFontPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SemanticFontPayload")
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

impl SemanticFontPayload {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn shared_bytes(&self) -> Arc<Vec<u8>> {
        Arc::clone(&self.bytes)
    }
}

impl SemanticFont {
    pub fn new(family: impl Into<String>, digest: ContentDigest, face_index: u32) -> Self {
        Self {
            family: family.into(),
            digest,
            face_index,
        }
    }
}

/// A flattened, ordered fallback chain. The primary family is not repeated in `fallbacks`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontFallbackChain {
    pub family: String,
    pub fallbacks: Vec<String>,
}

impl FontFallbackChain {
    pub fn new(
        family: impl Into<String>,
        fallbacks: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            family: family.into(),
            fallbacks: fallbacks.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StructureKind {
    Motion,
    Lottie,
    RuntimeShader,
    Scene3d,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StructureFootprint {
    Local,
    OutsetPixels { pixels: u32 },
    Unbounded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StructureDescriptor {
    Motion {
        topology_digest: ContentDigest,
        bounds: Extent2d,
    },
    Lottie {
        topology_digest: ContentDigest,
        bounds: Extent2d,
        duration: RationalTime,
    },
    RuntimeShader {
        abi_digest: ContentDigest,
        color_domain: OperatorColorDomain,
        alpha_behavior: OperatorAlphaBehavior,
        footprint: StructureFootprint,
    },
    Scene3d {
        topology_digest: ContentDigest,
        bounds_min: [f64; 3],
        bounds_max: [f64; 3],
    },
}

impl StructureDescriptor {
    pub const fn kind(&self) -> StructureKind {
        match self {
            Self::Motion { .. } => StructureKind::Motion,
            Self::Lottie { .. } => StructureKind::Lottie,
            Self::RuntimeShader { .. } => StructureKind::RuntimeShader,
            Self::Scene3d { .. } => StructureKind::Scene3d,
        }
    }

    fn validate(&self) -> Result<(), SnapshotError> {
        match self {
            Self::Lottie { duration, .. } if duration.is_non_positive() => {
                Err(SnapshotError::InvalidStructure {
                    key: String::new(),
                    reason: "lottie duration must be positive".to_owned(),
                })
            }
            Self::Scene3d {
                bounds_min,
                bounds_max,
                ..
            } if !bounds_min
                .iter()
                .chain(bounds_max)
                .all(|value| value.is_finite())
                || !(0..3).all(|axis| bounds_min[axis] < bounds_max[axis]) =>
            {
                Err(SnapshotError::InvalidStructure {
                    key: String::new(),
                    reason: "scene bounds must be finite and non-empty".to_owned(),
                })
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticStructure {
    pub key: String,
    pub digest: ContentDigest,
    pub descriptor: StructureDescriptor,
}

impl SemanticStructure {
    pub fn new(
        key: impl Into<String>,
        digest: ContentDigest,
        descriptor: StructureDescriptor,
    ) -> Self {
        Self {
            key: key.into(),
            digest,
            descriptor,
        }
    }

    pub const fn kind(&self) -> StructureKind {
        self.descriptor.kind()
    }
}

/// Validated Motion structure retained by a render. `PreparedScene` proves Artifact admission;
/// runtime shader packages are render-level semantic resources rather than duplicated payloads
/// owned by each Motion instance. Neither value contains a backend object.
#[derive(Clone)]
pub struct SemanticMotionPayload {
    prepared: valle_motion::PreparedScene,
}

impl fmt::Debug for SemanticMotionPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SemanticMotionPayload")
            .field("component", &self.prepared.artifact().component)
            .finish()
    }
}

impl SemanticMotionPayload {
    pub fn prepared(&self) -> &valle_motion::PreparedScene {
        &self.prepared
    }
}

#[derive(Debug, Clone)]
pub struct SemanticResourceSnapshot {
    assets: BTreeMap<String, SemanticAsset>,
    /// A CSS family owns an ordered set of immutable faces. Weight/style selection belongs to
    /// the shaper and must not be collapsed to one arbitrary digest during project admission.
    fonts: BTreeMap<String, Vec<SemanticFont>>,
    fallback_chains: BTreeMap<String, Vec<String>>,
    structures: BTreeMap<String, SemanticStructure>,
    font_payloads: BTreeMap<(ContentDigest, u32), SemanticFontPayload>,
    motion_payloads: BTreeMap<String, SemanticMotionPayload>,
    shaders: Arc<valle_motion::shader::ShaderRegistry>,
}

impl SemanticResourceSnapshot {
    pub fn asset(&self, id: &str) -> Option<&SemanticAsset> {
        self.assets.get(id)
    }

    pub fn assets(&self) -> impl ExactSizeIterator<Item = &SemanticAsset> {
        self.assets.values()
    }

    pub fn font(&self, family: &str) -> Option<&SemanticFont> {
        self.fonts.get(family).and_then(|faces| faces.first())
    }

    pub fn font_faces(&self, family: &str) -> Option<&[SemanticFont]> {
        self.fonts.get(family).map(Vec::as_slice)
    }

    pub fn fonts(&self) -> impl Iterator<Item = &SemanticFont> {
        self.fonts.values().flatten()
    }

    pub fn resolved_font_chain(&self, family: &str) -> Option<Vec<&SemanticFont>> {
        let primary = self.fonts.get(family)?;
        let mut result = primary.iter().collect::<Vec<_>>();
        if let Some(fallbacks) = self.fallback_chains.get(family) {
            result.extend(fallbacks.iter().flat_map(|fallback| {
                self.fonts
                    .get(fallback)
                    .expect("snapshot validation guarantees fallback faces")
            }));
        }
        Some(result)
    }

    pub fn structure(&self, key: &str) -> Option<&SemanticStructure> {
        self.structures.get(key)
    }

    pub fn structures(&self) -> impl ExactSizeIterator<Item = &SemanticStructure> {
        self.structures.values()
    }

    pub fn font_payload(
        &self,
        digest: &ContentDigest,
        face_index: u32,
    ) -> Option<&SemanticFontPayload> {
        self.font_payloads.get(&(digest.clone(), face_index))
    }

    pub fn motion_payload(&self, key: &str) -> Option<&SemanticMotionPayload> {
        self.motion_payloads.get(key)
    }

    /// Exact render-scoped shader package environment used to admit every Motion Artifact.
    pub fn shaders(&self) -> &valle_motion::shader::ShaderRegistry {
        &self.shaders
    }

    /// Runtime record produced by the render-wide shader admission that validated every owning
    /// Motion Artifact. Hosts materialize executor objects without re-parsing manifests or
    /// confusing the package content digest with the generated SkSL bytes.
    pub fn runtime_shader_record(
        &self,
        uri: &str,
        content_hash: &ContentDigest,
        abi_hash: &ContentDigest,
    ) -> Option<valle_motion::shader::ShaderRuntimeRecord> {
        let package = self.shaders.get(uri)?;
        (package.content_hash == *content_hash && package.manifest.abi_digest == *abi_hash)
            .then(|| package.runtime_record())
    }

    /// Resolve a runtime shader solely from the content-addressed execution identity carried by a
    /// [`ResourceRequest`](super::ResourceRequest). URI is an authoring address and intentionally
    /// does not enter host fulfillment; duplicate records with the same content/ABI are identical.
    pub fn runtime_shader_record_by_identity(
        &self,
        content_hash: &ContentDigest,
        abi_hash: &ContentDigest,
    ) -> Option<valle_motion::shader::ShaderRuntimeRecord> {
        self.shaders.packages().find_map(|package| {
            (package.content_hash == *content_hash && package.manifest.abi_digest == *abi_hash)
                .then(|| package.runtime_record())
        })
    }
}

#[derive(Debug, Default)]
pub struct SnapshotBuilder {
    assets: Vec<SemanticAsset>,
    fonts: Vec<SemanticFont>,
    fallback_chains: Vec<FontFallbackChain>,
    structures: Vec<SemanticStructure>,
    font_payloads: Vec<(ContentDigest, u32, Arc<[u8]>)>,
    motion_payloads: Vec<(String, valle_motion::SceneArtifact)>,
    shaders: valle_motion::shader::ShaderRegistry,
}

impl SnapshotBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn asset(mut self, asset: SemanticAsset) -> Self {
        self.assets.push(asset);
        self
    }

    pub fn font(mut self, font: SemanticFont) -> Self {
        self.fonts.push(font);
        self
    }

    /// Add one font descriptor and the exact bytes that make it usable for semantic shaping.
    pub fn font_face(mut self, font: SemanticFont, bytes: impl Into<Arc<[u8]>>) -> Self {
        let identity = (font.digest.clone(), font.face_index, bytes.into());
        self.fonts.push(font);
        self.font_payloads.push(identity);
        self
    }

    pub fn fallback_chain(mut self, chain: FontFallbackChain) -> Self {
        self.fallback_chains.push(chain);
        self
    }

    pub fn structure(mut self, structure: SemanticStructure) -> Self {
        self.structures.push(structure);
        self
    }

    /// Freeze one render-wide shader package environment. Package admission already proves the
    /// fixed Valle-SkSL v1 output and local sampling contracts; this builder materializes their
    /// content/ABI identities as ordinary semantic structures during `finish`.
    pub fn shader_registry(mut self, shaders: valle_motion::shader::ShaderRegistry) -> Self {
        self.shaders = shaders;
        self
    }

    /// Add a Motion descriptor and its already content-addressed, platform-free structure data.
    pub fn motion_structure(
        mut self,
        structure: SemanticStructure,
        artifact: valle_motion::SceneArtifact,
    ) -> Self {
        self.motion_payloads.push((structure.key.clone(), artifact));
        self.structures.push(structure);
        self
    }

    pub fn finish(self) -> Result<SemanticResourceSnapshot, SnapshotError> {
        let mut assets = BTreeMap::new();
        for asset in self.assets {
            validate_key("asset id", &asset.id)?;
            asset.descriptor.validate_for(asset.kind).map_err(|error| {
                SnapshotError::InvalidDescriptor {
                    owner: asset.id.clone(),
                    reason: error.to_string(),
                }
            })?;
            let id = asset.id.clone();
            if assets.insert(id.clone(), asset).is_some() {
                return Err(SnapshotError::DuplicateAsset { id });
            }
        }

        let mut fonts = BTreeMap::<String, Vec<SemanticFont>>::new();
        for font in self.fonts {
            validate_key("font family", &font.family)?;
            let family = font.family.clone();
            fonts.entry(family).or_default().push(font);
        }
        for faces in fonts.values_mut() {
            faces.sort_by(|left, right| {
                (&left.digest, left.face_index).cmp(&(&right.digest, right.face_index))
            });
            faces.dedup_by(|left, right| {
                left.digest == right.digest && left.face_index == right.face_index
            });
        }

        let mut fallback_chains = BTreeMap::new();
        for chain in self.fallback_chains {
            validate_key("font family", &chain.family)?;
            if !fonts.contains_key(&chain.family) {
                return Err(SnapshotError::MissingFallbackFont {
                    family: chain.family.clone(),
                });
            }
            let mut seen = BTreeMap::new();
            for fallback in &chain.fallbacks {
                validate_key("fallback font family", fallback)?;
                if fallback == &chain.family || seen.insert(fallback.clone(), ()).is_some() {
                    return Err(SnapshotError::InvalidFallbackChain {
                        family: chain.family.clone(),
                        reason: "fallback families must be unique and cannot repeat the primary"
                            .to_owned(),
                    });
                }
                if !fonts.contains_key(fallback) {
                    return Err(SnapshotError::MissingFallbackFont {
                        family: fallback.clone(),
                    });
                }
            }
            if fallback_chains
                .insert(chain.family.clone(), chain.fallbacks)
                .is_some()
            {
                return Err(SnapshotError::InvalidFallbackChain {
                    family: chain.family,
                    reason: "duplicate fallback chain".to_owned(),
                });
            }
        }

        let mut structures = BTreeMap::new();
        for package in self.shaders.packages() {
            let structure = semantic_runtime_shader(package);
            structures.insert(structure.key.clone(), structure);
        }
        for structure in self.structures {
            validate_key("structure key", &structure.key)?;
            structure
                .descriptor
                .validate()
                .map_err(|error| match error {
                    SnapshotError::InvalidStructure { reason, .. } => {
                        SnapshotError::InvalidStructure {
                            key: structure.key.clone(),
                            reason,
                        }
                    }
                    other => other,
                })?;
            let key = structure.key.clone();
            if structures.insert(key.clone(), structure).is_some() {
                return Err(SnapshotError::DuplicateStructure { key });
            }
        }

        let mut font_payloads = BTreeMap::<(ContentDigest, u32), SemanticFontPayload>::new();
        for (digest, face_index, bytes) in self.font_payloads {
            let actual = ContentDigest::of_bytes(&bytes);
            if actual != digest {
                return Err(SnapshotError::PayloadDigestMismatch {
                    owner: format!("font {digest}:{face_index}"),
                    expected: digest,
                    actual,
                });
            }
            ttf_parser::Face::parse(&bytes, face_index).map_err(|error| {
                SnapshotError::InvalidFontPayload {
                    digest: actual.clone(),
                    face_index,
                    reason: error.to_string(),
                }
            })?;
            let key = (actual, face_index);
            if let Some(existing) = font_payloads.get(&key) {
                if existing.bytes.as_ref() != bytes.as_ref() {
                    return Err(SnapshotError::ConflictingFontPayload {
                        digest: key.0,
                        face_index,
                    });
                }
                continue;
            }
            font_payloads.insert(
                key,
                SemanticFontPayload {
                    bytes: Arc::new(bytes.to_vec()),
                },
            );
        }

        let mut motion_payloads = BTreeMap::new();
        for (key, artifact) in self.motion_payloads {
            let structure = structures
                .get(&key)
                .ok_or_else(|| SnapshotError::MissingStructurePayloadOwner { key: key.clone() })?;
            let StructureDescriptor::Motion {
                topology_digest, ..
            } = &structure.descriptor
            else {
                return Err(SnapshotError::StructurePayloadKindMismatch { key });
            };
            artifact
                .validate_with_shaders(&self.shaders)
                .map_err(|errors| SnapshotError::InvalidStructure {
                    key: key.clone(),
                    reason: format!("Motion Artifact/shader admission failed: {errors:?}"),
                })?;
            let bytes = valle_motion::canonical_bytes(&artifact).map_err(|error| {
                SnapshotError::InvalidStructure {
                    key: key.clone(),
                    reason: format!("Motion Artifact canonicalization failed: {error}"),
                }
            })?;
            let actual = ContentDigest::of_bytes(&bytes);
            if actual != structure.digest {
                return Err(SnapshotError::PayloadDigestMismatch {
                    owner: key,
                    expected: structure.digest.clone(),
                    actual,
                });
            }
            // Until a narrower topology packet is introduced, the full admitted Artifact hash is
            // the conservative topology identity. It may reduce cache hits but can never reuse an
            // incompatible layout/program template.
            if topology_digest != &structure.digest {
                return Err(SnapshotError::InvalidStructure {
                    key: structure.key.clone(),
                    reason: "Motion topology digest must equal the admitted Artifact digest".into(),
                });
            }
            let prepared = valle_motion::prepare_owned_scene(artifact).map_err(|error| {
                SnapshotError::InvalidStructure {
                    key: structure.key.clone(),
                    reason: error.to_string(),
                }
            })?;
            if motion_payloads
                .insert(structure.key.clone(), SemanticMotionPayload { prepared })
                .is_some()
            {
                return Err(SnapshotError::DuplicateStructurePayload {
                    key: structure.key.clone(),
                });
            }
        }

        let mut motion_fonts = valle_motion::Fonts::default();
        for font in fonts.values().flatten() {
            let Some(payload) = font_payloads.get(&(font.digest.clone(), font.face_index)) else {
                continue;
            };
            let mut resource = valle_motion::FontResource::new(payload.bytes().to_vec())
                .override_info(valle_motion::FontOverride {
                    family_name: Some(Arc::<str>::from(font.family.as_str())),
                    ..Default::default()
                });
            if font.family == DEFAULT_FONT_FAMILY {
                resource = resource.generic_family(valle_motion::GenericFamily::SANS_SERIF);
            }
            motion_fonts
                .register(resource)
                .map_err(|error| SnapshotError::InvalidFontPayload {
                    digest: font.digest.clone(),
                    face_index: font.face_index,
                    reason: error.to_string(),
                })?;
        }

        Ok(SemanticResourceSnapshot {
            assets,
            fonts,
            fallback_chains,
            structures,
            font_payloads,
            motion_payloads,
            shaders: Arc::new(self.shaders),
        })
    }
}

fn semantic_runtime_shader(package: &valle_motion::shader::ShaderPackage) -> SemanticStructure {
    let digest = package.content_hash;
    let abi_digest = package.manifest.abi_digest;
    let footprint = match package.manifest.budget {
        valle_motion::shader::BudgetClass::Local => StructureFootprint::Local,
    };
    SemanticStructure::new(
        package.uri().to_string(),
        digest,
        StructureDescriptor::RuntimeShader {
            abi_digest,
            // Valle-SkSL v1 admits only sRGB author math and always writes a complete output
            // pixel (including coverage, or forced opaque coverage). These are package contract
            // facts, not executor guesses from generated SkSL text.
            color_domain: OperatorColorDomain::PerceptualSrgb,
            alpha_behavior: OperatorAlphaBehavior::RewritesCoverage,
            footprint,
        },
    )
}

fn validate_key(kind: &'static str, value: &str) -> Result<(), SnapshotError> {
    if value.is_empty() || value.chars().any(char::is_control) {
        Err(SnapshotError::InvalidKey {
            kind,
            value: value.to_owned(),
        })
    } else {
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum SnapshotError {
    #[error("invalid {kind}: {value:?}")]
    InvalidKey { kind: &'static str, value: String },
    #[error("duplicate semantic asset {id:?}")]
    DuplicateAsset { id: String },
    #[error("duplicate semantic structure {key:?}")]
    DuplicateStructure { key: String },
    #[error("duplicate semantic structure payload {key:?}")]
    DuplicateStructurePayload { key: String },
    #[error("semantic structure payload {key:?} has no descriptor")]
    MissingStructurePayloadOwner { key: String },
    #[error("semantic structure payload {key:?} does not match its descriptor kind")]
    StructurePayloadKindMismatch { key: String },
    #[error("payload digest mismatch for {owner}: expected {expected}, got {actual}")]
    PayloadDigestMismatch {
        owner: String,
        expected: ContentDigest,
        actual: ContentDigest,
    },
    #[error("font payload {digest}:{face_index} is invalid: {reason}")]
    InvalidFontPayload {
        digest: ContentDigest,
        face_index: u32,
        reason: String,
    },
    #[error("font payload {digest}:{face_index} has conflicting bytes")]
    ConflictingFontPayload {
        digest: ContentDigest,
        face_index: u32,
    },
    #[error("invalid media descriptor for {owner:?}: {reason}")]
    InvalidDescriptor { owner: String, reason: String },
    #[error("fallback chain references unavailable font family {family:?}")]
    MissingFallbackFont { family: String },
    #[error("invalid fallback chain for {family:?}: {reason}")]
    InvalidFallbackChain { family: String, reason: String },
    #[error("invalid semantic structure {key:?}: {reason}")]
    InvalidStructure { key: String, reason: String },
}

const fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

const fn fraction_less(left: UnitFraction, right: UnitFraction) -> bool {
    (left.numerator as u64) * (right.denominator as u64)
        < (right.numerator as u64) * (left.denominator as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY_SHADER: &[u8] = b"half4 valle_main(float2 uv) { return sampleContent(uv); }";

    fn digest(byte: char) -> ContentDigest {
        ContentDigest::from_hex(&byte.to_string().repeat(64)).unwrap()
    }

    fn identity_shader_registry() -> valle_motion::shader::ShaderRegistry {
        use valle_motion::shader::{
            BudgetClass, DIALECT_ID, DIALECT_VERSION, OutputContract, SHADER_MANIFEST_VERSION,
            ShaderManifest, ShaderPackage, ShaderRegistry,
        };

        let placeholder = ContentDigest::of_bytes(b"");
        let manifest = ShaderManifest {
            manifest_version: SHADER_MANIFEST_VERSION,
            name: "identity".into(),
            version: 1,
            dialect: DIALECT_ID.into(),
            dialect_version: DIALECT_VERSION,
            entry: "shader.vsksl".into(),
            inputs: Vec::new(),
            uniforms: Vec::new(),
            output: OutputContract::default(),
            budget: BudgetClass::Local,
            source_digest: placeholder,
            abi_digest: placeholder,
        }
        .seal(IDENTITY_SHADER)
        .unwrap();
        let package =
            ShaderPackage::admit(&serde_json::to_vec(&manifest).unwrap(), IDENTITY_SHADER).unwrap();
        let mut registry = ShaderRegistry::new();
        registry.register(package).unwrap();
        registry
    }

    #[test]
    fn ratios_and_crops_are_canonical_and_validated_on_wire() {
        assert_eq!(
            SampleAspectRatio::new(4, 2).unwrap(),
            SampleAspectRatio::new(2, 1).unwrap()
        );
        assert!(UnitFraction::new(2, 1).is_err());
        assert!(
            NormalizedCrop::new(
                UnitFraction::ONE,
                UnitFraction::ZERO,
                UnitFraction::ONE,
                UnitFraction::ONE
            )
            .is_err()
        );
        assert!(serde_json::from_str::<Extent2d>(r#"{"width":0,"height":10}"#).is_err());
    }

    #[test]
    fn snapshot_preserves_explicit_font_fallback_order() {
        let face = |family: &str, byte| SemanticFont::new(family, digest(byte), 0);
        let snapshot = SnapshotBuilder::new()
            .font(face("Brand", 'a'))
            .font(face("Han", 'b'))
            .font(face("Emoji", 'c'))
            .fallback_chain(FontFallbackChain::new("Brand", ["Han", "Emoji"]))
            .finish()
            .unwrap();
        assert_eq!(
            snapshot
                .resolved_font_chain("Brand")
                .unwrap()
                .iter()
                .map(|font| font.family.as_str())
                .collect::<Vec<_>>(),
            ["Brand", "Han", "Emoji"]
        );
    }

    #[test]
    fn one_family_preserves_all_faces_in_canonical_order() {
        let snapshot = SnapshotBuilder::new()
            .font(SemanticFont::new("Brand", digest('b'), 0))
            .font(SemanticFont::new("Brand", digest('a'), 0))
            .font(SemanticFont::new("Brand", digest('b'), 0))
            .finish()
            .unwrap();

        let faces = snapshot.font_faces("Brand").unwrap();
        assert_eq!(faces.len(), 2);
        assert_eq!(faces[0].digest, digest('a'));
        assert_eq!(faces[1].digest, digest('b'));
        assert_eq!(snapshot.resolved_font_chain("Brand").unwrap().len(), 2);
    }

    #[test]
    fn shader_registry_is_frozen_as_render_level_semantic_structures() {
        let registry = identity_shader_registry();
        let package = registry.get("shader://identity@1").unwrap();
        let content_hash = package.content_hash;
        let abi_hash = package.manifest.abi_digest;
        let snapshot = SnapshotBuilder::new()
            .shader_registry(registry)
            .finish()
            .unwrap();

        let structure = snapshot.structure("shader://identity@1").unwrap();
        assert_eq!(structure.digest, content_hash);
        assert!(matches!(
            &structure.descriptor,
            StructureDescriptor::RuntimeShader {
                abi_digest,
                color_domain: OperatorColorDomain::PerceptualSrgb,
                alpha_behavior: OperatorAlphaBehavior::RewritesCoverage,
                footprint: StructureFootprint::Local,
            } if *abi_digest == abi_hash
        ));
        assert_eq!(snapshot.shaders().len(), 1);
        assert!(
            snapshot
                .runtime_shader_record("shader://identity@1", &content_hash, &abi_hash)
                .is_some()
        );
        assert!(
            snapshot
                .runtime_shader_record_by_identity(&content_hash, &abi_hash)
                .is_some()
        );
    }
}
