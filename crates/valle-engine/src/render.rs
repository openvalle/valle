//! Timeline resource admission and compiled-render vertical slice.
//!
//! This slice closes resource references, verified digest bindings,
//! transitive dependencies, extension kernels, exact frame/sample schedules,
//! source ranges, transition/crossfade handles, and typed property curves
//! before a render can be evaluated. Motion instances whose control-schema
//! values have not been compiled are rejected at open, as are audio sources
//! that would require an unproved channel mapper or resampler. Runtime code
//! therefore consumes only immutable programs and frozen resource facts.

mod admission;
mod audio_gain;
mod evaluate;

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use valle_motion::{AssetKind, ControlType, SceneArtifact, shader::ShaderPackage};
use valle_timeline::Color;
use valle_timeline::internal::quantize::{
    quantize_frame_boundary, quantize_frame_interval, quantize_sample_boundary,
    quantize_sample_interval,
};
use valle_timeline::internal::{
    CanonicalTimeline, ContentDigest, DiagnosticSeverity, FrameRate, RationalRate, RationalTime,
    ResourceManifest, SampleTime,
    document::*,
    wire::resource::{
        AudioChannelLayoutWire, AudioResourceDescriptorWire, ContinuousBoundarySamplingWire,
        FontResourceDescriptorWire, FontVariationAxisWire, ImageResourceDescriptorWire,
        LottieArtifactAbiWire, LottieResourceDescriptorWire, MotionArtifactAbiWire,
        MotionArtifactDescriptorWire, ResourceEntryWire, ShaderArtifactAbiWire,
        ShaderResourceDescriptorWire, VideoResourceDescriptorWire,
    },
};
pub use valle_timeline::internal::{FrameKey, RenderId};

#[cfg(test)]
use evaluate::evaluate_source_sample_index;
use evaluate::{
    crossfade_progress, cubic_bezier_progress, euclidean_source_modulo, evaluate_audio_sample,
    evaluate_camera, evaluate_caption_program, evaluate_visual_program, frame_sample_time,
    sample_time, transition_progress,
};

const MAX_SAFE_JSON_INTEGER: i64 = 9_007_199_254_740_991;

/// Engine-owned deterministic extension ABIs. A capability may bind a
/// project kernel name to one of these ABIs and a concrete implementation
/// digest; arbitrary ABI strings are not executable.
pub const EXTENSION_COLOR_GAIN_ABI: &str = "valle.compositor/color-gain@1";
pub const EXTENSION_CROSS_FADE_ABI: &str = "valle.compositor/cross-fade@1";
/// The only currently admitted audio execution profile. Its PCM conformance is bit-exact:
/// f64 endpoint/track accumulation, one terminal clamp, then one f32 conversion.
pub const COMMON_AUDIO_ABI: &str = "valle.audio/common@1";
/// Hash domain for profile-specific decoded PCM facts. The byte stream is:
/// this NUL-terminated domain, `sample_rate` as little-endian u32, `channels`
/// as little-endian u16, frame count as little-endian u64, then finite
/// interleaved samples as their IEEE-754 f32 bits in little-endian order.
pub const COMMON_AUDIO_PCM_DIGEST_DOMAIN: &[u8] =
    b"valle.audio/common@1/decoded-interleaved-f32le@1\0";
/// Author-facing namespaced kind for the first sample-local, stateless audio effect.
pub const AUDIO_GAIN_EFFECT_KIND: &str = "valle.audio/gain@1";
/// Executor ABI for [`AUDIO_GAIN_EFFECT_KIND`]. The closed parameter packet is
/// `{ "multiplier": finite f64 >= 0 }` and has zero latency/tail.
pub const AUDIO_GAIN_EFFECT_ABI: &str = "valle.audio/gain-multiplier@1";

// These are the executable artifacts behind the three engine-owned extension ABIs. Visual
// backends compile the exact embedded SkSL bytes below. The common audio implementation is kept
// in one deliberately small source module and those exact source bytes are embedded as its
// implementation artifact, so changing the executed algorithm necessarily rotates its digest.
const EXTENSION_COLOR_GAIN_IMPLEMENTATION_BYTES: &[u8] =
    include_bytes!("../../valle-draw/assets/shaders/extensioncolorgain.sksl");
const EXTENSION_CROSS_FADE_IMPLEMENTATION_BYTES: &[u8] =
    include_bytes!("../../valle-draw/assets/shaders/fade.sksl");
const AUDIO_GAIN_EFFECT_IMPLEMENTATION_BYTES: &[u8] = include_bytes!("render/audio_gain.rs");

fn engine_owned_kernel_implementation_bytes(abi: &str) -> Option<&'static [u8]> {
    match abi {
        EXTENSION_COLOR_GAIN_ABI => Some(EXTENSION_COLOR_GAIN_IMPLEMENTATION_BYTES),
        EXTENSION_CROSS_FADE_ABI => Some(EXTENSION_CROSS_FADE_IMPLEMENTATION_BYTES),
        AUDIO_GAIN_EFFECT_ABI => Some(AUDIO_GAIN_EFFECT_IMPLEMENTATION_BYTES),
        _ => None,
    }
}

/// Returns the only executable implementation identity for an engine-owned extension ABI.
///
/// Hosts advertise this value in [`ExtensionKernelCapability`]. Admission rejects every other
/// value, and prepared/executor boundaries compare the raw digest again before executing pixels.
pub fn engine_owned_kernel_implementation_digest(abi: &str) -> Option<ContentDigest> {
    engine_owned_kernel_implementation_bytes(abi).map(ContentDigest::of_bytes)
}

/// Raw form of [`engine_owned_kernel_implementation_digest`] carried by prepared kernel packets.
pub fn engine_owned_kernel_implementation_sha256(abi: &str) -> Option<[u8; 32]> {
    engine_owned_kernel_implementation_digest(abi).map(|digest| *digest.as_bytes())
}

/// Computes the semantic digest asserted by a verified common-profile audio
/// binding. Decoder/probe code calls this once during fulfillment admission;
/// render-time mixers trust the frozen fact and never rescan the source.
pub fn common_audio_pcm_digest(
    sample_rate: u32,
    channels: u16,
    interleaved_samples: &[f32],
) -> Result<ContentDigest, CommonAudioPcmDigestError> {
    if sample_rate == 0 || channels == 0 {
        return Err(CommonAudioPcmDigestError::InvalidFormat);
    }
    let channel_count = usize::from(channels);
    if !interleaved_samples.len().is_multiple_of(channel_count) {
        return Err(CommonAudioPcmDigestError::PartialFrame);
    }
    let frames = u64::try_from(interleaved_samples.len() / channel_count)
        .map_err(|_| CommonAudioPcmDigestError::TooManyFrames)?;
    let mut hasher = Sha256::new();
    hasher.update(COMMON_AUDIO_PCM_DIGEST_DOMAIN);
    hasher.update(sample_rate.to_le_bytes());
    hasher.update(channels.to_le_bytes());
    hasher.update(frames.to_le_bytes());
    for sample in interleaved_samples {
        if !sample.is_finite() {
            return Err(CommonAudioPcmDigestError::NonFiniteSample);
        }
        hasher.update(sample.to_bits().to_le_bytes());
    }
    Ok(ContentDigest::from_bytes(hasher.finalize().into()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommonAudioPcmDigestError {
    #[error("decoded PCM format must have a non-zero sample rate and channel count")]
    InvalidFormat,
    #[error("decoded PCM contains a partial interleaved frame")]
    PartialFrame,
    #[error("decoded PCM frame count exceeds u64")]
    TooManyFrames,
    #[error("decoded PCM contains a non-finite sample")]
    NonFiniteSample,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceKind {
    Video,
    Audio,
    Image,
    Lottie,
    Font,
    MotionArtifact,
    Shader,
}

/// Frozen visual dependency expansion contributed by a verified resource
/// implementation. Values are semantic, while the platform handle itself is
/// deliberately not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualFootprint {
    past_frames: u32,
    future_frames: u32,
}

impl VisualFootprint {
    pub const fn new(past_frames: u32, future_frames: u32) -> Self {
        Self {
            past_frames,
            future_frames,
        }
    }

    pub const fn past_frames(self) -> u32 {
        self.past_frames
    }

    pub const fn future_frames(self) -> u32 {
        self.future_frames
    }
}

/// Frozen audio dependency expansion contributed by a verified decoder or
/// artifact. The safe-integer bound keeps the semantic projection inside the
/// shared Valle JSON profile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioFootprint {
    past_samples: u64,
    future_samples: u64,
}

impl AudioFootprint {
    pub fn new(past_samples: u64, future_samples: u64) -> Result<Self, ResourceBindingBuildError> {
        if past_samples > MAX_SAFE_JSON_INTEGER as u64
            || future_samples > MAX_SAFE_JSON_INTEGER as u64
        {
            return Err(ResourceBindingBuildError::UnsafeAudioFootprint);
        }
        Ok(Self {
            past_samples,
            future_samples,
        })
    }

    pub const fn past_samples(self) -> u64 {
        self.past_samples
    }

    pub const fn future_samples(self) -> u64 {
        self.future_samples
    }
}

/// Facts re-probed from the content behind a verified handle. Admission
/// compares these values with the pinned manifest descriptor and freezes them
/// into the resource semantic projection. This prevents runtime code from
/// consulting mutable probe metadata after open.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum VerifiedResourceFacts {
    Video {
        descriptor: VideoResourceDescriptorWire,
        temporal_footprint: VisualFootprint,
    },
    Audio {
        descriptor: AudioResourceDescriptorWire,
        temporal_footprint: AudioFootprint,
        /// Profile-specific semantic digest of the verified decoded PCM. For
        /// [`COMMON_AUDIO_ABI`] this uses [`COMMON_AUDIO_PCM_DIGEST_DOMAIN`].
        decoded_pcm_digest: ContentDigest,
    },
    Image {
        descriptor: ImageResourceDescriptorWire,
        temporal_footprint: VisualFootprint,
    },
    Lottie {
        abi: LottieArtifactAbiWire,
        descriptor: LottieResourceDescriptorWire,
        temporal_footprint: VisualFootprint,
    },
    Font {
        descriptor: FontResourceDescriptorWire,
        /// Immutable bytes for the exact face pinned by the manifest digest.
        /// The digest and descriptor, rather than a duplicate byte encoding,
        /// participate in the semantic projection.
        #[serde(skip)]
        bytes: Arc<[u8]>,
    },
    MotionArtifact {
        abi: MotionArtifactAbiWire,
        descriptor: MotionArtifactDescriptorWire,
        #[serde(skip)]
        artifact: Arc<SceneArtifact>,
        temporal_footprint: VisualFootprint,
    },
    Shader {
        abi: ShaderArtifactAbiWire,
        descriptor: ShaderResourceDescriptorWire,
        #[serde(skip)]
        manifest_bytes: Arc<[u8]>,
        #[serde(skip)]
        source_bytes: Arc<[u8]>,
    },
}

fn artifact_asset_kind(kind: AssetKind) -> Option<ResourceKind> {
    match kind {
        AssetKind::Image => Some(ResourceKind::Image),
        AssetKind::Audio => Some(ResourceKind::Audio),
        AssetKind::Video => Some(ResourceKind::Video),
        AssetKind::Font => Some(ResourceKind::Font),
        // Timeline has no model-3d ResourceManifest kind.
        AssetKind::Model3d => None,
    }
}

impl VerifiedResourceFacts {
    pub const fn kind(&self) -> ResourceKind {
        match self {
            Self::Video { .. } => ResourceKind::Video,
            Self::Audio { .. } => ResourceKind::Audio,
            Self::Image { .. } => ResourceKind::Image,
            Self::Lottie { .. } => ResourceKind::Lottie,
            Self::Font { .. } => ResourceKind::Font,
            Self::MotionArtifact { .. } => ResourceKind::MotionArtifact,
            Self::Shader { .. } => ResourceKind::Shader,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VerifiedHandleId(u64);

impl VerifiedHandleId {
    pub fn new(value: u64) -> Result<Self, ResourceBindingBuildError> {
        if value == 0 {
            Err(ResourceBindingBuildError::InvalidHandle)
        } else {
            Ok(Self(value))
        }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceDependency {
    role: String,
    resource_id: String,
}

impl ResourceDependency {
    pub fn new(
        role: impl Into<String>,
        resource_id: impl Into<String>,
    ) -> Result<Self, ResourceBindingBuildError> {
        let role = role.into();
        let resource_id = resource_id.into();
        if role.trim().is_empty() {
            return Err(ResourceBindingBuildError::EmptyDependencyRole);
        }
        if resource_id.trim().is_empty() {
            return Err(ResourceBindingBuildError::EmptyResourceId);
        }
        Ok(Self { role, resource_id })
    }

    pub fn role(&self) -> &str {
        &self.role
    }

    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceBinding {
    digest: ContentDigest,
    handle: VerifiedHandleId,
    facts: VerifiedResourceFacts,
    dependencies: Vec<ResourceDependency>,
}

impl ResourceBinding {
    pub fn new(
        digest: ContentDigest,
        handle: VerifiedHandleId,
        facts: VerifiedResourceFacts,
    ) -> Self {
        Self {
            digest,
            handle,
            facts,
            dependencies: Vec::new(),
        }
    }

    pub fn with_dependency(mut self, dependency: ResourceDependency) -> Self {
        self.dependencies.push(dependency);
        self
    }

    pub fn digest(&self) -> &ContentDigest {
        &self.digest
    }

    pub const fn handle(&self) -> VerifiedHandleId {
        self.handle
    }

    pub fn facts(&self) -> &VerifiedResourceFacts {
        &self.facts
    }

    pub fn dependencies(&self) -> &[ResourceDependency] {
        &self.dependencies
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResourceBindings {
    entries: BTreeMap<String, ResourceBinding>,
}

impl ResourceBindings {
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub fn insert(
        &mut self,
        resource_id: impl Into<String>,
        binding: ResourceBinding,
    ) -> Result<(), ResourceBindingBuildError> {
        let resource_id = resource_id.into();
        if resource_id.trim().is_empty() {
            return Err(ResourceBindingBuildError::EmptyResourceId);
        }
        if self.entries.insert(resource_id.clone(), binding).is_some() {
            return Err(ResourceBindingBuildError::DuplicateResourceId { resource_id });
        }
        Ok(())
    }

    pub fn with_binding(
        mut self,
        resource_id: impl Into<String>,
        binding: ResourceBinding,
    ) -> Result<Self, ResourceBindingBuildError> {
        self.insert(resource_id, binding)?;
        Ok(self)
    }

    pub fn get(&self, resource_id: &str) -> Option<&ResourceBinding> {
        self.entries.get(resource_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &ResourceBinding)> {
        self.entries
            .iter()
            .map(|(resource_id, binding)| (resource_id.as_str(), binding))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResourceBindingBuildError {
    #[error("verified handle id must be non-zero")]
    InvalidHandle,
    #[error("resource id must not be empty")]
    EmptyResourceId,
    #[error("dependency role must not be empty")]
    EmptyDependencyRole,
    #[error("duplicate resource binding `{resource_id}`")]
    DuplicateResourceId { resource_id: String },
    #[error("audio temporal footprint exceeds the interoperable JSON integer range")]
    UnsafeAudioFootprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionKernelCapability {
    abi: String,
    implementation_digest: ContentDigest,
    visual_footprint: VisualFootprint,
    audio_footprint: AudioFootprint,
}

impl ExtensionKernelCapability {
    pub fn new(
        abi: impl Into<String>,
        implementation_digest: ContentDigest,
    ) -> Result<Self, CapabilityBuildError> {
        let abi = abi.into();
        if abi.trim().is_empty() {
            return Err(CapabilityBuildError::EmptyKernelAbi);
        }
        Ok(Self {
            abi,
            implementation_digest,
            visual_footprint: VisualFootprint::default(),
            audio_footprint: AudioFootprint::default(),
        })
    }

    pub const fn with_visual_footprint(mut self, footprint: VisualFootprint) -> Self {
        self.visual_footprint = footprint;
        self
    }

    pub const fn with_audio_footprint(mut self, footprint: AudioFootprint) -> Self {
        self.audio_footprint = footprint;
        self
    }

    pub fn abi(&self) -> &str {
        &self.abi
    }

    pub fn implementation_digest(&self) -> &ContentDigest {
        &self.implementation_digest
    }

    pub const fn visual_footprint(&self) -> VisualFootprint {
        self.visual_footprint
    }

    pub const fn audio_footprint(&self) -> AudioFootprint {
        self.audio_footprint
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    extension_kernels: BTreeMap<String, ExtensionKernelCapability>,
    artifact_abis: BTreeSet<String>,
    camera: bool,
}

impl Capabilities {
    pub const fn new() -> Self {
        Self {
            extension_kernels: BTreeMap::new(),
            artifact_abis: BTreeSet::new(),
            camera: false,
        }
    }

    pub fn with_extension_kernel(
        mut self,
        kernel: impl Into<String>,
        capability: ExtensionKernelCapability,
    ) -> Result<Self, CapabilityBuildError> {
        let kernel = kernel.into();
        if kernel.trim().is_empty() {
            return Err(CapabilityBuildError::EmptyKernelKind);
        }
        if self
            .extension_kernels
            .insert(kernel.clone(), capability)
            .is_some()
        {
            return Err(CapabilityBuildError::DuplicateKernel { kernel });
        }
        Ok(self)
    }

    pub fn with_artifact_abi(mut self, abi: impl Into<String>) -> Self {
        self.artifact_abis.insert(abi.into());
        self
    }

    pub const fn with_camera(mut self) -> Self {
        self.camera = true;
        self
    }

    pub fn extension_kernel(&self, kernel: &str) -> Option<&ExtensionKernelCapability> {
        self.extension_kernels.get(kernel)
    }

    pub fn supports_artifact_abi(&self, abi: &str) -> bool {
        self.artifact_abis.contains(abi)
    }

    pub const fn camera_enabled(&self) -> bool {
        self.camera
    }

    pub fn artifact_abis(&self) -> impl Iterator<Item = &str> {
        self.artifact_abis.iter().map(String::as_str)
    }

    pub fn extension_kernels(&self) -> impl Iterator<Item = (&str, &ExtensionKernelCapability)> {
        self.extension_kernels
            .iter()
            .map(|(kind, capability)| (kind.as_str(), capability))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CapabilityBuildError {
    #[error("extension kernel kind must not be empty")]
    EmptyKernelKind,
    #[error("extension kernel ABI must not be empty")]
    EmptyKernelAbi,
    #[error("duplicate extension kernel capability `{kernel}`")]
    DuplicateKernel { kernel: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionLimits {
    max_resources: u32,
    max_dependency_depth: u16,
    max_extension_kernels: u32,
}

impl ExecutionLimits {
    pub fn new(
        max_resources: u32,
        max_dependency_depth: u16,
        max_extension_kernels: u32,
    ) -> Result<Self, ExecutionProfileError> {
        if max_resources == 0 || max_dependency_depth == 0 || max_extension_kernels == 0 {
            return Err(ExecutionProfileError::ZeroLimit);
        }
        Ok(Self {
            max_resources,
            max_dependency_depth,
            max_extension_kernels,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ExecutionProfileProjection {
    name: String,
    numeric_abi: String,
    audio_abi: String,
    motion_abi: String,
    caption_abi: String,
    limits: ExecutionLimits,
}

pub const CAPTION_GLYPH_RUN_ABI: &str = "valle.caption/cosmic-text-glyph-run@1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionProfile {
    projection: ExecutionProfileProjection,
}

impl ExecutionProfile {
    pub fn new(
        name: impl Into<String>,
        numeric_abi: impl Into<String>,
        audio_abi: impl Into<String>,
        motion_abi: impl Into<String>,
        caption_abi: impl Into<String>,
        limits: ExecutionLimits,
    ) -> Result<Self, ExecutionProfileError> {
        Self::from_projection(ExecutionProfileProjection {
            name: name.into(),
            numeric_abi: numeric_abi.into(),
            audio_abi: audio_abi.into(),
            motion_abi: motion_abi.into(),
            caption_abi: caption_abi.into(),
            limits,
        })
    }

    pub(crate) fn from_projection(
        projection: ExecutionProfileProjection,
    ) -> Result<Self, ExecutionProfileError> {
        if projection.name.trim().is_empty()
            || projection.numeric_abi.trim().is_empty()
            || projection.audio_abi.trim().is_empty()
            || projection.motion_abi.trim().is_empty()
            || projection.caption_abi.trim().is_empty()
        {
            return Err(ExecutionProfileError::EmptyPolicyId);
        }
        execution_profile_projection_bytes(&projection)
            .map_err(|_| ExecutionProfileError::Canonical)?;
        Ok(Self { projection })
    }

    pub(crate) fn canonical_projection_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        execution_profile_projection_bytes(&self.projection)
    }

    pub const fn limits(&self) -> ExecutionLimits {
        self.projection.limits
    }

    pub fn caption_abi(&self) -> &str {
        &self.projection.caption_abi
    }

    pub fn audio_abi(&self) -> &str {
        &self.projection.audio_abi
    }
}

fn execution_profile_projection_bytes(
    projection: &ExecutionProfileProjection,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_jcs::to_vec(projection)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExecutionProfileError {
    #[error("execution profile policy identifiers must not be empty")]
    EmptyPolicyId,
    #[error("execution profile limits must be non-zero")]
    ZeroLimit,
    #[error("execution profile canonical encoding failed")]
    Canonical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EngineOpenPhase {
    ResourceResolve,
    Admission,
    Compile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineOpenDiagnosticCode {
    MissingManifestResource,
    ResourceKindMismatch,
    MissingResourceBinding,
    BindingDigestMismatch,
    BindingFactsMismatch,
    DependencyCycle,
    ResourceBudgetExceeded,
    DependencyDepthExceeded,
    UnsupportedExtensionKernel,
    UnsupportedArtifactAbi,
    UnsupportedCamera,
    UnsupportedCaptionBackend,
    UnsupportedAudioExecutionProfile,
    ExtensionKernelBudgetExceeded,
    InvalidCanvasQuantization,
    InvalidIntervalQuantization,
    PositiveDurationQuantizedToZeroFrame,
    PositiveDurationQuantizedToZeroSample,
    TransitionWindowOutOfCanvas,
    TransitionInsufficientHandle,
    TransitionEndpointLayerUnsupported,
    OverlappingTransitionEndpoint,
    CrossfadeWindowOutOfCanvas,
    CrossfadeInsufficientHandle,
    OverlappingCrossfadeEndpoint,
    SourceRangeOutOfBounds,
    UnsupportedAudioChannelLayout,
    UnprovenAudioResampler,
    MotionArtifactPayloadMismatch,
    MotionControlSchemaMismatch,
    MotionCueSchemaMismatch,
    MotionResourceSchemaMismatch,
    MotionTimingMismatch,
    FontPayloadMismatch,
    UnsupportedFontFaceIndex,
    MissingFontGlyph,
    ShaderPayloadMismatch,
    InvalidExtensionParameters,
    InvalidAdjustmentEffectParameters,
    InternalCompileFault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EngineOpenDiagnostic {
    pub code: EngineOpenDiagnosticCode,
    pub path: String,
    pub phase: EngineOpenPhase,
    pub severity: DiagnosticSeverity,
    pub resource_id: Option<String>,
    pub details: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("engine render admission failed")]
pub struct EngineOpenReport {
    diagnostics: Vec<EngineOpenDiagnostic>,
}

impl EngineOpenReport {
    pub fn diagnostics(&self) -> &[EngineOpenDiagnostic] {
        &self.diagnostics
    }

    pub fn contains(&self, code: EngineOpenDiagnosticCode) -> bool {
        self.diagnostics.iter().any(|item| item.code == code)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResolvedDependencyProjection {
    role: String,
    target: u32,
}

#[derive(Debug, Clone)]
struct ResolvedResource {
    resource_id: String,
    kind: ResourceKind,
    digest: ContentDigest,
    handle: VerifiedHandleId,
    facts: VerifiedResourceFacts,
    execution: ResolvedExecutionPayload,
    entry: ResourceEntryWire,
    dependencies: Vec<ResolvedDependencyProjection>,
}

#[derive(Debug, Clone, Default)]
enum ResolvedExecutionPayload {
    #[default]
    None,
    Shader(Arc<ShaderPackage>),
    Motion(Arc<valle_motion::PreparedScene>),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResolvedResourceProjection<'a> {
    kind: ResourceKind,
    digest: &'a ContentDigest,
    facts: &'a VerifiedResourceFacts,
    entry: &'a ResourceEntryWire,
    dependencies: &'a [ResolvedDependencyProjection],
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResolvedRootProjection {
    role: String,
    target: u32,
}

#[derive(Debug, Clone)]
pub struct ResolvedResourceSnapshot {
    resources: Vec<ResolvedResource>,
    roots: Vec<ResolvedRootProjection>,
}

impl ResolvedResourceSnapshot {
    pub fn resource_count(&self) -> usize {
        self.resources.len()
    }

    pub fn root_count(&self) -> usize {
        self.roots.len()
    }

    pub(crate) fn shader_package(&self, target: u32) -> Option<&ShaderPackage> {
        match &self.resources.get(target as usize)?.execution {
            ResolvedExecutionPayload::Shader(package) => Some(package),
            _ => None,
        }
    }

    pub(crate) fn motion_scene(&self, target: u32) -> Option<&valle_motion::PreparedScene> {
        match &self.resources.get(target as usize)?.execution {
            ResolvedExecutionPayload::Motion(scene) => Some(scene),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdmittedKernel {
    role: String,
    kind: String,
    domain: CompiledKernelDomain,
    abi: String,
    implementation_digest: ContentDigest,
    visual_footprint: VisualFootprint,
    audio_footprint: AudioFootprint,
}

#[derive(Debug, Clone)]
struct KernelRequirement {
    role: String,
    kind: String,
    domain: CompiledKernelDomain,
    parameters: JsonObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompiledKernelDomain {
    VisualFilter,
    VisualTransition,
    AdjustmentEffect,
    AudioEffect,
}

#[derive(Debug, Clone, Copy)]
pub struct AdmittedKernelRef<'a> {
    kernel: &'a AdmittedKernel,
}

impl AdmittedKernelRef<'_> {
    pub fn kind(&self) -> &str {
        &self.kernel.kind
    }

    pub const fn domain(&self) -> CompiledKernelDomain {
        self.kernel.domain
    }

    pub fn abi(&self) -> &str {
        &self.kernel.abi
    }

    pub fn implementation_digest(&self) -> &ContentDigest {
        &self.kernel.implementation_digest
    }

    pub const fn visual_footprint(&self) -> VisualFootprint {
        self.kernel.visual_footprint
    }

    pub const fn audio_footprint(&self) -> AudioFootprint {
        self.kernel.audio_footprint
    }
}

#[derive(Debug, Clone)]
struct ResourceUse {
    role: String,
    path: String,
    resource_id: String,
    expected_kind: Option<ResourceKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledCanvas {
    width: u32,
    height: u32,
    duration: RationalTime,
    frame_rate: FrameRate,
    sample_rate: u32,
    channel_layout: CompiledChannelLayout,
    color_space: CompiledColorSpace,
    background_rgba: [u8; 4],
    frame_count: i64,
    sample_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompiledChannelLayout {
    Mono,
    Stereo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompiledColorSpace {
    Srgb,
}

impl CompiledCanvas {
    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    pub const fn duration(&self) -> RationalTime {
        self.duration
    }

    pub const fn frame_rate(&self) -> FrameRate {
        self.frame_rate
    }

    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub const fn channel_layout(&self) -> CompiledChannelLayout {
        self.channel_layout
    }

    pub const fn color_space(&self) -> CompiledColorSpace {
        self.color_space
    }

    pub const fn background_rgba(&self) -> [u8; 4] {
        self.background_rgba
    }

    pub const fn frame_count(&self) -> i64 {
        self.frame_count
    }

    pub const fn sample_count(&self) -> i64 {
        self.sample_count
    }

    /// Quantizes UI seconds with the engine's common round-half-away clock
    /// and requires the result to name a frame in the half-open canvas.
    pub fn frame_at_seconds(self, seconds: f64) -> Result<FrameKey, CanvasClockError> {
        let units = self.frame_rate.numerator() as f64 / f64::from(self.frame_rate.denominator());
        quantize_seconds_clock(seconds, units, self.frame_count, false, "frame").map(FrameKey::new)
    }

    /// Quantizes a frame boundary. Unlike `frame_at_seconds`, the exact end
    /// boundary (`frame_count`) is accepted, which is useful for half-open
    /// range endpoints but can never be evaluated as a frame.
    pub fn frame_boundary_at_seconds(self, seconds: f64) -> Result<i64, CanvasClockError> {
        let units = self.frame_rate.numerator() as f64 / f64::from(self.frame_rate.denominator());
        quantize_seconds_clock(seconds, units, self.frame_count, true, "frame")
    }

    /// Quantizes UI seconds to an output sample in the half-open canvas.
    pub fn sample_at_seconds(self, seconds: f64) -> Result<i64, CanvasClockError> {
        quantize_seconds_clock(
            seconds,
            f64::from(self.sample_rate),
            self.sample_count,
            false,
            "sample",
        )
    }

    /// Quantizes an output-sample boundary and accepts `sample_count` as the
    /// one-past-end boundary.
    pub fn sample_boundary_at_seconds(self, seconds: f64) -> Result<i64, CanvasClockError> {
        quantize_seconds_clock(
            seconds,
            f64::from(self.sample_rate),
            self.sample_count,
            true,
            "sample",
        )
    }

    /// Converts an admitted frame boundary to the corresponding output-sample
    /// boundary with exact integer rational arithmetic. Both canvas end
    /// boundaries are accepted; frame identities themselves remain half-open.
    pub fn sample_boundary_at_frame(self, frame_boundary: i64) -> Result<i64, CanvasClockError> {
        if frame_boundary < 0 || frame_boundary > self.frame_count {
            return Err(CanvasClockError::OutOfRange {
                unit: "frame-boundary",
                index: frame_boundary,
                limit: self.frame_count,
                end: "]",
            });
        }
        // `frame_count` is itself a rounded Admission boundary and may lie
        // slightly after the exact canvas duration. The terminal export
        // boundary is canonically the separately admitted `sample_count`.
        if frame_boundary == self.frame_count {
            return Ok(self.sample_count);
        }
        let numerator = (frame_boundary as i128)
            .checked_mul(i128::from(self.frame_rate.denominator()))
            .and_then(|value| value.checked_mul(i128::from(self.sample_rate)))
            .ok_or(CanvasClockError::Overflow { unit: "sample" })?;
        let denominator = u128::try_from(self.frame_rate.numerator())
            .map_err(|_| CanvasClockError::Overflow { unit: "sample" })?;
        let magnitude = numerator.unsigned_abs();
        let quotient = magnitude / denominator;
        let remainder = magnitude % denominator;
        let rounded = quotient
            .checked_add(u128::from(remainder >= denominator - remainder))
            .and_then(|value| i64::try_from(value).ok())
            .ok_or(CanvasClockError::Overflow { unit: "sample" })?;
        if rounded > self.sample_count {
            return Err(CanvasClockError::OutOfRange {
                unit: "sample-boundary",
                index: rounded,
                limit: self.sample_count,
                end: "]",
            });
        }
        Ok(rounded)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CanvasClockError {
    #[error("{unit} clock seconds must be finite and non-negative")]
    InvalidSeconds { unit: &'static str },
    #[error("{unit} clock seconds overflow")]
    Overflow { unit: &'static str },
    #[error("quantized {unit} index {index} is outside [0, {limit}{end})")]
    OutOfRange {
        unit: &'static str,
        index: i64,
        limit: i64,
        end: &'static str,
    },
}

fn quantize_seconds_clock(
    seconds: f64,
    units_per_second: f64,
    count: i64,
    allow_end: bool,
    unit: &'static str,
) -> Result<i64, CanvasClockError> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(CanvasClockError::InvalidSeconds { unit });
    }
    let scaled = seconds * units_per_second;
    if !scaled.is_finite() || scaled > i64::MAX as f64 {
        return Err(CanvasClockError::Overflow { unit });
    }
    let index = scaled.round() as i64;
    let upper = if allow_end {
        count
            .checked_add(1)
            .ok_or(CanvasClockError::Overflow { unit })?
    } else {
        count
    };
    if index < 0 || index >= upper {
        return Err(CanvasClockError::OutOfRange {
            unit,
            index,
            limit: count,
            end: if allow_end { "]" } else { ")" },
        });
    }
    Ok(index)
}

/// Half-open frame interval materialized once during admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameRange {
    start: i64,
    end: i64,
}

impl FrameRange {
    const fn new(start: i64, end: i64) -> Self {
        Self { start, end }
    }

    pub const fn start(self) -> i64 {
        self.start
    }

    pub const fn end(self) -> i64 {
        self.end
    }

    pub const fn len(self) -> i64 {
        self.end - self.start
    }

    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub const fn contains(self, frame: i64) -> bool {
        frame >= self.start && frame < self.end
    }

    fn union(self, other: Self) -> Self {
        Self::new(self.start.min(other.start), self.end.max(other.end))
    }

    fn intersects(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// Half-open output-sample interval materialized once during admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SampleRange {
    start: i64,
    end: i64,
}

impl SampleRange {
    pub fn new(start: i64, end: i64) -> Result<Self, RuntimeFault> {
        if start < 0 || end < start {
            return Err(RuntimeFault::SampleRangeOutOfRange {
                start,
                end,
                limit: 0,
            });
        }
        Ok(Self { start, end })
    }

    const fn admitted(start: i64, end: i64) -> Self {
        Self { start, end }
    }

    pub const fn start(self) -> i64 {
        self.start
    }

    pub const fn end(self) -> i64 {
        self.end
    }

    pub const fn len(self) -> i64 {
        self.end - self.start
    }

    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub const fn contains(self, sample: i64) -> bool {
        sample >= self.start && sample < self.end
    }

    fn union(self, other: Self) -> Self {
        Self::admitted(self.start.min(other.start), self.end.max(other.end))
    }

    fn intersects(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum CompiledOwnerClock {
    CompositionGlobal,
    VisualClipLocal,
    AudioClipLocal,
    CaptionLocal,
    MotionSource,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompiledParam<T> {
    owner_clock: CompiledOwnerClock,
    value: CompiledParamValue<T>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum CompiledParamValue<T> {
    Constant { value: T },
    Curve { curve: CompiledCurve<T> },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompiledCurve<T> {
    interpolation: CompiledInterpolation,
    keyframes: Vec<CompiledKeyframe<T>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompiledKeyframe<T> {
    time: RationalTime,
    value: T,
    easing: CompiledEasing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum CompiledInterpolation {
    Step,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum CompiledEasing {
    Linear,
    CubicBezier { x1: f64, y1: f64, x2: f64, y2: f64 },
}

trait CompiledParamValueType: Clone {
    fn interpolate(&self, other: &Self, progress: f64) -> Self;
}

impl CompiledParamValueType for f64 {
    fn interpolate(&self, other: &Self, progress: f64) -> Self {
        self + (other - self) * progress
    }
}

impl CompiledParamValueType for [f64; 2] {
    fn interpolate(&self, other: &Self, progress: f64) -> Self {
        [
            self[0] + (other[0] - self[0]) * progress,
            self[1] + (other[1] - self[1]) * progress,
        ]
    }
}

impl CompiledParamValueType for [f64; 4] {
    fn interpolate(&self, other: &Self, progress: f64) -> Self {
        [
            self[0] + (other[0] - self[0]) * progress,
            self[1] + (other[1] - self[1]) * progress,
            self[2] + (other[2] - self[2]) * progress,
            self[3] + (other[3] - self[3]) * progress,
        ]
    }
}

impl<T: CompiledParamValueType> CompiledParam<T> {
    fn evaluate(&self, clocks: EvaluationClocks) -> T {
        let time = match self.owner_clock {
            CompiledOwnerClock::CompositionGlobal => clocks.composition,
            CompiledOwnerClock::VisualClipLocal
            | CompiledOwnerClock::AudioClipLocal
            | CompiledOwnerClock::CaptionLocal => clocks.clip,
            CompiledOwnerClock::MotionSource => clocks.motion,
        };
        match &self.value {
            CompiledParamValue::Constant { value } => value.clone(),
            CompiledParamValue::Curve { curve } => curve.evaluate(time),
        }
    }

    /// Evaluates a Motion-source-owned parameter without collapsing a held end into an arbitrary
    /// exact timestamp. A continuous curve has its ordinary value at the boundary, while a step
    /// curve excludes a keyframe exactly at that boundary.
    fn evaluate_motion_source(
        &self,
        mapped_time: MappedSourceTime,
        source_duration: RationalTime,
    ) -> T {
        assert_eq!(
            self.owner_clock,
            CompiledOwnerClock::MotionSource,
            "Motion controls must be compiled onto the Motion source clock"
        );
        match (&self.value, mapped_time) {
            (CompiledParamValue::Constant { value }, _) => value.clone(),
            (CompiledParamValue::Curve { curve }, MappedSourceTime::HoldEnd) => {
                curve.evaluate_left_limit(source_duration)
            }
            (CompiledParamValue::Curve { curve }, MappedSourceTime::Exact(time)) => {
                curve.evaluate(time)
            }
            (
                CompiledParamValue::Curve { curve },
                MappedSourceTime::Static | MappedSourceTime::HoldStart,
            ) => curve.evaluate(RationalTime::ZERO),
        }
    }
}

impl<T: CompiledParamValueType> CompiledCurve<T> {
    fn evaluate(&self, time: RationalTime) -> T {
        let first = &self.keyframes[0];
        if time <= first.time {
            return first.value.clone();
        }
        let last = self.keyframes.last().expect("admitted curve is non-empty");
        if time >= last.time {
            return last.value.clone();
        }
        let upper = self
            .keyframes
            .partition_point(|keyframe| keyframe.time <= time);
        let from = &self.keyframes[upper - 1];
        let to = &self.keyframes[upper];
        if self.interpolation == CompiledInterpolation::Step {
            return from.value.clone();
        }
        let elapsed = time
            .checked_sub(from.time)
            .expect("admitted curve time subtraction cannot overflow");
        let duration = to
            .time
            .checked_sub(from.time)
            .expect("admitted curve duration cannot overflow");
        let raw = elapsed
            .checked_div(duration)
            .expect("admitted curve segment duration is positive")
            .as_f64()
            .clamp(0.0, 1.0);
        from.value.interpolate(&to.value, from.easing.apply(raw))
    }

    fn evaluate_left_limit(&self, boundary: RationalTime) -> T {
        if self.interpolation != CompiledInterpolation::Step {
            // Linear and cubic-eased segments are continuous at their endpoint. Their exact
            // boundary value is therefore also the mathematical left limit.
            return self.evaluate(boundary);
        }
        let first = &self.keyframes[0];
        let upper = self
            .keyframes
            .partition_point(|keyframe| keyframe.time < boundary);
        if upper == 0 {
            first.value.clone()
        } else {
            self.keyframes[upper - 1].value.clone()
        }
    }
}

impl CompiledEasing {
    fn apply(self, progress: f64) -> f64 {
        match self {
            Self::Linear => progress,
            Self::CubicBezier { x1, y1, x2, y2 } => cubic_bezier_progress(progress, x1, y1, x2, y2),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct EvaluationClocks {
    composition: RationalTime,
    clip: RationalTime,
    motion: RationalTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompiledSourceKind {
    Video,
    Image,
    Lottie,
    Motion,
    Solid,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum CompiledEndBehavior {
    Static,
    Error,
    Hold,
    Loop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompiledRasterFit {
    Contain,
    Cover,
    Fill,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompiledAudioChannelMap {
    StereoIdentity,
    MonoToStereo,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum CompiledMotionParam {
    Scalar { param: CompiledParam<f64> },
    Vec2 { param: CompiledParam<[f64; 2]> },
    Vec4 { param: CompiledParam<[f64; 4]> },
    Boolean { value: bool },
    String { value: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CompiledMotionCue {
    SourceRange {
        start: RationalTime,
        end: RationalTime,
        enter_duration: RationalTime,
        exit_duration: RationalTime,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledMotionPhases {
    duration_frames: u32,
    enter_frames: u32,
    hold_frames: u32,
    exit_frames: u32,
    hold_cycle_frames: Option<u32>,
}

impl CompiledMotionPhases {
    pub const fn duration_frames(self) -> u32 {
        self.duration_frames
    }

    pub const fn enter_frames(self) -> u32 {
        self.enter_frames
    }

    pub const fn hold_frames(self) -> u32 {
        self.hold_frames
    }

    pub const fn exit_frames(self) -> u32 {
        self.exit_frames
    }

    pub const fn hold_cycle_frames(self) -> Option<u32> {
        self.hold_cycle_frames
    }

    pub const fn layout(self) -> valle_motion::PhaseLayout {
        valle_motion::PhaseLayout {
            duration_frames: self.duration_frames,
            enter_frames: self.enter_frames,
            hold_frames: self.hold_frames,
            exit_frames: self.exit_frames,
            hold_cycle_frames: self.hold_cycle_frames,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledMotionArtifactDependency {
    role: String,
    target: u32,
}

impl CompiledMotionArtifactDependency {
    pub fn role(&self) -> &str {
        &self.role
    }

    pub const fn target(&self) -> u32 {
        self.target
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledMotionInstance {
    component_target: u32,
    reads_destination: bool,
    props: BTreeMap<String, CompiledMotionParam>,
    cues: BTreeMap<String, CompiledMotionCue>,
    resources: BTreeMap<String, u32>,
    artifact_dependencies: Vec<CompiledMotionArtifactDependency>,
    phases: CompiledMotionPhases,
    #[serde(skip)]
    artifact: Arc<SceneArtifact>,
}

impl CompiledMotionInstance {
    pub const fn component_target(&self) -> u32 {
        self.component_target
    }

    pub const fn reads_destination(&self) -> bool {
        self.reads_destination
    }

    /// The admitted executable artifact. It is carried by the immutable
    /// render rather than looked up again by the renderer.
    pub fn artifact(&self) -> &SceneArtifact {
        self.artifact.as_ref()
    }

    pub fn cues(&self) -> &BTreeMap<String, CompiledMotionCue> {
        &self.cues
    }

    pub fn resources(&self) -> &BTreeMap<String, u32> {
        &self.resources
    }

    pub fn artifact_dependencies(&self) -> &[CompiledMotionArtifactDependency] {
        &self.artifact_dependencies
    }

    pub const fn phases(&self) -> CompiledMotionPhases {
        self.phases
    }

    fn evaluate_props(
        &self,
        mapped_time: MappedSourceTime,
        source_duration: RationalTime,
    ) -> BTreeMap<String, EvaluatedMotionValue> {
        self.props
            .iter()
            .map(|(name, param)| {
                let value = match param {
                    CompiledMotionParam::Scalar { param } => EvaluatedMotionValue::Scalar(
                        param.evaluate_motion_source(mapped_time, source_duration),
                    ),
                    CompiledMotionParam::Vec2 { param } => EvaluatedMotionValue::Vec2(
                        param.evaluate_motion_source(mapped_time, source_duration),
                    ),
                    CompiledMotionParam::Vec4 { param } => EvaluatedMotionValue::Vec4(
                        param.evaluate_motion_source(mapped_time, source_duration),
                    ),
                    CompiledMotionParam::Boolean { value } => EvaluatedMotionValue::Boolean(*value),
                    CompiledMotionParam::String { value } => {
                        EvaluatedMotionValue::String(value.clone())
                    }
                };
                (name.clone(), value)
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum CompiledSourcePayload {
    Video {
        fit: CompiledRasterFit,
    },
    Image {
        fit: CompiledRasterFit,
    },
    Lottie {
        fit: CompiledRasterFit,
    },
    Motion {
        instance: CompiledMotionInstance,
    },
    Solid {
        color: String,
    },
    Audio {
        channel_map: CompiledAudioChannelMap,
        source_sample_rate: u32,
        source_sample_count: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "unit", rename_all = "kebab-case", deny_unknown_fields)]
enum CompiledDependencyRange {
    Frames { range: FrameRange },
    Samples { range: SampleRange },
    Static,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledSource {
    kind: CompiledSourceKind,
    resource_target: Option<u32>,
    placement_start: RationalTime,
    source_start: RationalTime,
    source_duration: Option<RationalTime>,
    hold_end_time: Option<RationalTime>,
    rate: RationalRate,
    end_behavior: CompiledEndBehavior,
    dependency_range: CompiledDependencyRange,
    payload: CompiledSourcePayload,
}

impl CompiledSource {
    pub const fn kind(&self) -> CompiledSourceKind {
        self.kind
    }

    pub const fn resource_target(&self) -> Option<u32> {
        self.resource_target
    }

    pub const fn source_duration(&self) -> Option<RationalTime> {
        self.source_duration
    }

    pub fn motion(&self) -> Option<&CompiledMotionInstance> {
        match &self.payload {
            CompiledSourcePayload::Motion { instance } => Some(instance),
            _ => None,
        }
    }

    pub const fn raster_fit(&self) -> Option<CompiledRasterFit> {
        match &self.payload {
            CompiledSourcePayload::Video { fit }
            | CompiledSourcePayload::Image { fit }
            | CompiledSourcePayload::Lottie { fit } => Some(*fit),
            _ => None,
        }
    }

    pub const fn audio_channel_map(&self) -> Option<CompiledAudioChannelMap> {
        match &self.payload {
            CompiledSourcePayload::Audio { channel_map, .. } => Some(*channel_map),
            _ => None,
        }
    }

    pub const fn audio_source_sample_rate(&self) -> Option<u32> {
        match &self.payload {
            CompiledSourcePayload::Audio {
                source_sample_rate, ..
            } => Some(*source_sample_rate),
            _ => None,
        }
    }

    pub const fn audio_source_sample_count(&self) -> Option<i64> {
        match &self.payload {
            CompiledSourcePayload::Audio {
                source_sample_count,
                ..
            } => Some(*source_sample_count),
            _ => None,
        }
    }

    pub fn solid_color(&self) -> Option<&str> {
        match &self.payload {
            CompiledSourcePayload::Solid { color } => Some(color),
            _ => None,
        }
    }

    pub const fn placement_start(&self) -> RationalTime {
        self.placement_start
    }

    pub const fn source_start(&self) -> RationalTime {
        self.source_start
    }

    pub const fn rate(&self) -> RationalRate {
        self.rate
    }

    fn map_time(&self, composition_time: RationalTime) -> Result<MappedSourceTime, RuntimeFault> {
        if self.end_behavior == CompiledEndBehavior::Static {
            return Ok(MappedSourceTime::Static);
        }
        let clip_local = composition_time
            .checked_sub(self.placement_start)
            .map_err(|_| RuntimeFault::ExactTimeOverflow)?;
        let raw = self
            .source_start
            .checked_add(
                clip_local
                    .checked_scale(self.rate)
                    .map_err(|_| RuntimeFault::ExactTimeOverflow)?,
            )
            .map_err(|_| RuntimeFault::ExactTimeOverflow)?;
        let duration = self
            .source_duration
            .expect("finite admitted source has a duration");
        match self.end_behavior {
            CompiledEndBehavior::Static => Ok(MappedSourceTime::Static),
            CompiledEndBehavior::Error => Ok(MappedSourceTime::Exact(raw)),
            CompiledEndBehavior::Hold if raw.is_negative() => Ok(MappedSourceTime::HoldStart),
            CompiledEndBehavior::Hold if raw >= duration => Ok(MappedSourceTime::HoldEnd),
            CompiledEndBehavior::Hold => Ok(MappedSourceTime::Exact(raw)),
            CompiledEndBehavior::Loop => euclidean_source_modulo(raw, duration)
                .map(MappedSourceTime::Exact)
                .map_err(|_| RuntimeFault::ExactTimeOverflow),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledSourceCatalog {
    entries: Vec<CompiledSource>,
}

impl CompiledSourceCatalog {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn source(&self, index: u32) -> Option<&CompiledSource> {
        self.entries.get(index as usize)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompiledBlendMode {
    Normal,
    Screen,
    Lighten,
    ColorDodge,
    Multiply,
    Darken,
    ColorBurn,
    LinearBurn,
    Overlay,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledKernelCall {
    kernel: u32,
    parameters: JsonObject,
}

impl CompiledKernelCall {
    pub const fn kernel_index(&self) -> u32 {
        self.kernel
    }

    pub fn parameters(&self) -> &JsonObject {
        &self.parameters
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum CompiledMask {
    Rect {
        rect: CompiledParam<[f64; 4]>,
        feather: CompiledParam<f64>,
        invert: bool,
    },
    Ellipse {
        rect: CompiledParam<[f64; 4]>,
        feather: CompiledParam<f64>,
        invert: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompiledVisualLayer {
    position: CompiledParam<[f64; 2]>,
    scale: CompiledParam<[f64; 2]>,
    rotation: CompiledParam<f64>,
    anchor: [f64; 2],
    opacity: CompiledParam<f64>,
    mask: Option<CompiledMask>,
    filters: Vec<CompiledKernelCall>,
    blend: CompiledBlendMode,
    footprint: VisualFootprint,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledVisualClip {
    #[serde(skip)]
    id: String,
    range: FrameRange,
    exact_start: RationalTime,
    source: u32,
    layer: CompiledVisualLayer,
}

impl CompiledVisualClip {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub const fn range(&self) -> FrameRange {
        self.range
    }

    pub const fn source_index(&self) -> u32 {
        self.source
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledVisualGap {
    range: FrameRange,
}

impl CompiledVisualGap {
    pub const fn range(&self) -> FrameRange {
        self.range
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CompiledTransitionKernel {
    CrossFade,
    Extension { call: CompiledKernelCall },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledVisualTransition {
    window: FrameRange,
    cut_frame: i64,
    left_frames: i64,
    right_frames: i64,
    from_item: u32,
    to_item: u32,
    from_source: u32,
    to_source: u32,
    kernel: CompiledTransitionKernel,
    footprint: VisualFootprint,
}

impl CompiledVisualTransition {
    pub const fn window(&self) -> FrameRange {
        self.window
    }

    pub const fn cut_frame(&self) -> i64 {
        self.cut_frame
    }

    pub const fn left_frames(&self) -> i64 {
        self.left_frames
    }

    pub const fn right_frames(&self) -> i64 {
        self.right_frames
    }

    pub fn progress_at(&self, frame: i64) -> Option<RationalTime> {
        transition_progress(frame, self.window)
    }

    pub const fn kernel(&self) -> &CompiledTransitionKernel {
        &self.kernel
    }

    pub const fn footprint(&self) -> VisualFootprint {
        self.footprint
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CompiledVisualItem {
    Clip {
        clip: CompiledVisualClip,
    },
    Gap {
        gap: CompiledVisualGap,
    },
    Transition {
        transition: CompiledVisualTransition,
    },
}

impl CompiledVisualItem {
    pub fn frame_range(&self) -> FrameRange {
        match self {
            Self::Clip { clip } => clip.range,
            Self::Gap { gap } => gap.range,
            Self::Transition { transition } => transition.window,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledVisualTrack {
    #[serde(skip)]
    id: String,
    order: u32,
    items: Vec<CompiledVisualItem>,
}

impl CompiledVisualTrack {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub const fn order(&self) -> u32 {
        self.order
    }

    pub fn items(&self) -> &[CompiledVisualItem] {
        &self.items
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledVisualProgram {
    tracks: Vec<CompiledVisualTrack>,
}

impl CompiledVisualProgram {
    pub fn tracks(&self) -> &[CompiledVisualTrack] {
        &self.tracks
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CompiledAdjustmentEffect {
    ColorGrade { temperature: f64 },
    Extension { call: CompiledKernelCall },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledAdjustmentClip {
    range: FrameRange,
    effect: CompiledAdjustmentEffect,
}

impl CompiledAdjustmentClip {
    pub const fn range(&self) -> FrameRange {
        self.range
    }

    pub const fn effect(&self) -> &CompiledAdjustmentEffect {
        &self.effect
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledAdjustmentProgram {
    clips: Vec<CompiledAdjustmentClip>,
}

impl CompiledAdjustmentProgram {
    pub fn clips(&self) -> &[CompiledAdjustmentClip] {
        &self.clips
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompiledCaptionAlign {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CompiledCaptionBehavior {
    Scroll { horizontal: bool, speed: f64 },
    Karaoke { line_mode: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledCaptionRunTiming {
    start: RationalTime,
    end: RationalTime,
}

impl CompiledCaptionRunTiming {
    pub const fn start(self) -> RationalTime {
        self.start
    }

    pub const fn end(self) -> RationalTime {
        self.end
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledCaptionRunStyle {
    font_size: f64,
    color: String,
    font_weight: u16,
}

impl CompiledCaptionRunStyle {
    pub const fn font_size(&self) -> f64 {
        self.font_size
    }

    pub fn color(&self) -> &str {
        &self.color
    }

    pub const fn font_weight(&self) -> u16 {
        self.font_weight
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledCaptionShadow {
    color: String,
    offset: [f64; 2],
    blur_sigma: f64,
}

impl CompiledCaptionShadow {
    pub fn color(&self) -> &str {
        &self.color
    }

    pub const fn offset(&self) -> [f64; 2] {
        self.offset
    }

    pub const fn blur_sigma(&self) -> f64 {
        self.blur_sigma
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompiledCaptionPresentation {
    opacity: CompiledParam<f64>,
    translation: CompiledParam<[f64; 2]>,
    scale: CompiledParam<f64>,
    rotation: CompiledParam<f64>,
    clip_inset: CompiledParam<[f64; 4]>,
    blur_sigma: CompiledParam<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledCaptionClip {
    range: FrameRange,
    runs: Vec<String>,
    run_timings: Vec<Option<CompiledCaptionRunTiming>>,
    run_styles: Vec<CompiledCaptionRunStyle>,
    font_target: u32,
    shadow: Option<CompiledCaptionShadow>,
    region: [f64; 4],
    align: CompiledCaptionAlign,
    presentation: CompiledCaptionPresentation,
    behavior: Option<CompiledCaptionBehavior>,
}

impl CompiledCaptionClip {
    pub const fn range(&self) -> FrameRange {
        self.range
    }

    pub fn runs(&self) -> &[String] {
        &self.runs
    }

    pub fn run_timings(&self) -> &[Option<CompiledCaptionRunTiming>] {
        &self.run_timings
    }

    pub fn run_styles(&self) -> &[CompiledCaptionRunStyle] {
        &self.run_styles
    }

    pub const fn font_target(&self) -> u32 {
        self.font_target
    }

    pub const fn shadow(&self) -> Option<&CompiledCaptionShadow> {
        self.shadow.as_ref()
    }

    pub const fn region(&self) -> [f64; 4] {
        self.region
    }

    pub const fn align(&self) -> CompiledCaptionAlign {
        self.align
    }

    pub const fn behavior(&self) -> Option<&CompiledCaptionBehavior> {
        self.behavior.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledCaptionGap {
    range: FrameRange,
}

impl CompiledCaptionGap {
    pub const fn range(&self) -> FrameRange {
        self.range
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CompiledCaptionItem {
    Clip { clip: CompiledCaptionClip },
    Gap { gap: CompiledCaptionGap },
}

impl CompiledCaptionItem {
    pub const fn frame_range(&self) -> FrameRange {
        match self {
            Self::Clip { clip } => clip.range,
            Self::Gap { gap } => gap.range,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledCaptionTrack {
    order: u32,
    items: Vec<CompiledCaptionItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledCaptionProgram {
    tracks: Vec<CompiledCaptionTrack>,
}

impl CompiledCaptionProgram {
    pub fn tracks(&self) -> &[CompiledCaptionTrack] {
        &self.tracks
    }
}

impl CompiledCaptionTrack {
    pub const fn order(&self) -> u32 {
        self.order
    }

    pub fn items(&self) -> &[CompiledCaptionItem] {
        &self.items
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledAudioClip {
    range: SampleRange,
    exact_start: RationalTime,
    source: u32,
    effect_multiplier: f64,
    gain: CompiledParam<f64>,
    pan: CompiledParam<f64>,
    effects: Vec<CompiledKernelCall>,
    footprint: AudioFootprint,
}

impl CompiledAudioClip {
    pub const fn range(&self) -> SampleRange {
        self.range
    }

    pub const fn source_index(&self) -> u32 {
        self.source
    }

    /// Sample-local effect result frozen during admission. For the common
    /// audio ABI this is evaluated before the authored clip gain and pan.
    pub const fn effect_multiplier(&self) -> f64 {
        self.effect_multiplier
    }

    pub fn effects(&self) -> &[CompiledKernelCall] {
        &self.effects
    }

    pub const fn footprint(&self) -> AudioFootprint {
        self.footprint
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledAudioGap {
    range: SampleRange,
}

impl CompiledAudioGap {
    pub const fn range(&self) -> SampleRange {
        self.range
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledAudioCrossfade {
    window: SampleRange,
    cut_sample: i64,
    left_samples: i64,
    right_samples: i64,
    from_item: u32,
    to_item: u32,
    from_source: u32,
    to_source: u32,
}

impl CompiledAudioCrossfade {
    pub const fn window(&self) -> SampleRange {
        self.window
    }

    pub fn progress_at(&self, sample: i64) -> Option<RationalTime> {
        crossfade_progress(sample, self.window)
    }

    pub const fn cut_sample(&self) -> i64 {
        self.cut_sample
    }

    pub const fn left_samples(&self) -> i64 {
        self.left_samples
    }

    pub const fn right_samples(&self) -> i64 {
        self.right_samples
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CompiledAudioItem {
    Clip { clip: CompiledAudioClip },
    Gap { gap: CompiledAudioGap },
    Crossfade { crossfade: CompiledAudioCrossfade },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledAudioTrack {
    order: u32,
    items: Vec<CompiledAudioItem>,
}

impl CompiledAudioTrack {
    pub const fn order(&self) -> u32 {
        self.order
    }

    pub fn items(&self) -> &[CompiledAudioItem] {
        &self.items
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledAudioProgram {
    tracks: Vec<CompiledAudioTrack>,
    sample_rate: u32,
    sample_count: i64,
    #[serde(skip)]
    sources: Arc<CompiledSourceCatalog>,
    #[serde(skip)]
    resources: Arc<ResolvedResourceSnapshot>,
}

impl CompiledAudioProgram {
    pub fn tracks(&self) -> &[CompiledAudioTrack] {
        &self.tracks
    }

    pub fn evaluate_sample(&self, sample: i64) -> Result<EvaluatedAudioSample, RuntimeFault> {
        if sample < 0 || sample >= self.sample_count {
            return Err(RuntimeFault::SampleOutOfRange {
                sample,
                sample_count: self.sample_count,
            });
        }
        evaluate_audio_sample(self, sample)
    }

    pub fn evaluate_block(&self, range: SampleRange) -> Result<EvaluatedAudioBlock, RuntimeFault> {
        if range.start < 0 || range.end < range.start || range.end > self.sample_count {
            return Err(RuntimeFault::SampleRangeOutOfRange {
                start: range.start,
                end: range.end,
                limit: self.sample_count,
            });
        }
        let mut samples = Vec::with_capacity(range.len() as usize);
        for sample in range.start..range.end {
            samples.push(evaluate_audio_sample(self, sample)?);
        }
        Ok(EvaluatedAudioBlock { range, samples })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledCameraProgram {
    center_x: CompiledParam<f64>,
    center_y: CompiledParam<f64>,
    zoom: CompiledParam<f64>,
    rotation: CompiledParam<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MappedSourceTime {
    Static,
    Exact(RationalTime),
    HoldStart,
    HoldEnd,
}

#[derive(Debug, Clone)]
pub struct CompiledRender {
    render_id: RenderId,
    canvas: CompiledCanvas,
    sources: Arc<CompiledSourceCatalog>,
    visual: CompiledVisualProgram,
    adjustments: CompiledAdjustmentProgram,
    captions: CompiledCaptionProgram,
    audio: CompiledAudioProgram,
    camera: Option<CompiledCameraProgram>,
    kernels: Vec<AdmittedKernel>,
    resources: Arc<ResolvedResourceSnapshot>,
}

/// Exact composition-to-Motion frame mapping owned by one compiled render.
/// Hosts may inspect this address but never quantize source frames themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MotionFrameAddress {
    render_id: RenderId,
    composition_frame: FrameKey,
    source_index: u32,
    source_frame: u32,
}

impl MotionFrameAddress {
    pub const fn render_id(self) -> RenderId {
        self.render_id
    }

    pub const fn composition_frame(self) -> FrameKey {
        self.composition_frame
    }

    pub const fn source_index(self) -> u32 {
        self.source_index
    }

    pub const fn source_frame(self) -> u32 {
        self.source_frame
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompiledExecutionResourceKind {
    FontBytes,
    RuntimeShader,
}

#[derive(Debug, Clone, Copy)]
pub struct CompiledExecutionResourceRef<'a> {
    resource_id: &'a str,
    kind: CompiledExecutionResourceKind,
    content_digest: &'a ContentDigest,
    abi_digest: Option<&'a ContentDigest>,
    bytes: &'a [u8],
}

impl<'a> CompiledExecutionResourceRef<'a> {
    pub fn resource_id(self) -> &'a str {
        self.resource_id
    }

    pub const fn kind(self) -> CompiledExecutionResourceKind {
        self.kind
    }

    pub const fn content_digest(self) -> &'a ContentDigest {
        self.content_digest
    }

    pub const fn abi_digest(self) -> Option<&'a ContentDigest> {
        self.abi_digest
    }

    /// Immutable execution bytes. Runtime shaders expose the generated SkSL,
    /// never the Valle source that would require Host-side lowering.
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompiledExecutionResourceError {
    #[error("compiled execution resource query names a different RenderId")]
    RenderMismatch,
    #[error("compiled execution resource was not found")]
    NotFound,
    #[error("compiled runtime shader ABI digest does not match")]
    AbiMismatch,
}

impl CompiledRender {
    pub const fn render_id(&self) -> RenderId {
        self.render_id
    }

    pub const fn canvas(&self) -> CompiledCanvas {
        self.canvas
    }

    pub fn sources(&self) -> &CompiledSourceCatalog {
        &self.sources
    }

    /// Resolves a Motion source frame under this render's exact clock and
    /// source end-behavior. The explicit RenderId prevents stale inspector
    /// requests from crossing a replacement boundary.
    pub fn motion_frame_address(
        &self,
        render_id: RenderId,
        composition_frame: FrameKey,
        source_index: u32,
    ) -> Result<MotionFrameAddress, RuntimeFault> {
        if render_id != self.render_id {
            return Err(RuntimeFault::RenderMismatch);
        }
        if composition_frame.index() < 0 || composition_frame.index() >= self.canvas.frame_count {
            return Err(RuntimeFault::FrameOutOfRange {
                frame: composition_frame.index(),
                frame_count: self.canvas.frame_count,
            });
        }
        let source = self
            .sources
            .source(source_index)
            .ok_or(RuntimeFault::SourceOutOfRange {
                source_index,
                source_count: self.sources.len(),
            })?;
        if source.kind != CompiledSourceKind::Motion {
            return Err(RuntimeFault::MotionSourceRequired { source_index });
        }
        let duration = source
            .source_duration
            .ok_or(RuntimeFault::MotionSourceDurationMissing { source_index })?;
        let composition_time =
            frame_sample_time(composition_frame.index(), self.canvas.frame_rate)?;
        let mapped = source.map_time(composition_time)?;
        let source_frame =
            quantize_motion_source_frame(mapped, duration, self.canvas.frame_rate, source_index)?;
        Ok(MotionFrameAddress {
            render_id: self.render_id,
            composition_frame,
            source_index,
            source_frame,
        })
    }

    pub fn visual(&self) -> &CompiledVisualProgram {
        &self.visual
    }

    pub fn adjustments(&self) -> &CompiledAdjustmentProgram {
        &self.adjustments
    }

    pub fn captions(&self) -> &CompiledCaptionProgram {
        &self.captions
    }

    pub fn audio(&self) -> &CompiledAudioProgram {
        &self.audio
    }

    pub fn camera(&self) -> Option<&CompiledCameraProgram> {
        self.camera.as_ref()
    }

    pub fn resources(&self) -> &ResolvedResourceSnapshot {
        &self.resources
    }

    pub fn admitted_kernel_count(&self) -> usize {
        self.kernels.len()
    }

    pub fn admitted_kernel(&self, index: u32) -> Option<AdmittedKernelRef<'_>> {
        self.kernels
            .get(index as usize)
            .map(|kernel| AdmittedKernelRef { kernel })
    }

    /// Lists immutable payloads that a platform backend may fulfill without
    /// reopening, recompiling, or registering semantic state.
    pub fn execution_resources(&self) -> Vec<CompiledExecutionResourceRef<'_>> {
        self.resources
            .resources
            .iter()
            .filter_map(compiled_execution_resource_ref)
            .collect()
    }

    /// Looks up one backend payload under an explicitly pinned render.
    pub fn execution_resource(
        &self,
        render_id: RenderId,
        kind: CompiledExecutionResourceKind,
        content_digest: &ContentDigest,
        abi_digest: Option<&ContentDigest>,
    ) -> Result<CompiledExecutionResourceRef<'_>, CompiledExecutionResourceError> {
        if render_id != self.render_id {
            return Err(CompiledExecutionResourceError::RenderMismatch);
        }
        let mut content_match = None;
        for resource in &self.resources.resources {
            let Some(resource) = compiled_execution_resource_ref(resource) else {
                continue;
            };
            if resource.kind != kind || resource.content_digest != content_digest {
                continue;
            }
            content_match = Some(resource);
            if resource.abi_digest == abi_digest {
                return Ok(resource);
            }
        }
        match content_match {
            Some(_) => Err(CompiledExecutionResourceError::AbiMismatch),
            None => Err(CompiledExecutionResourceError::NotFound),
        }
    }

    pub(crate) fn shader_package(&self, resource: &EvaluatedResourceRef) -> Option<&ShaderPackage> {
        self.resources.shader_package(resource.target)
    }

    pub(crate) fn motion_scene(
        &self,
        instance: &CompiledMotionInstance,
    ) -> Option<&valle_motion::PreparedScene> {
        self.resources.motion_scene(instance.component_target)
    }

    /// Evaluates one visual frame entirely from immutable compiled schedules.
    /// Audio remains on [`CompiledAudioProgram`]'s sample clock.
    pub fn evaluate(&self, frame: FrameKey) -> Result<EvaluatedRenderFrame, RuntimeFault> {
        if frame.index() < 0 || frame.index() >= self.canvas.frame_count {
            return Err(RuntimeFault::FrameOutOfRange {
                frame: frame.index(),
                frame_count: self.canvas.frame_count,
            });
        }
        let composition_time = frame_sample_time(frame.index(), self.canvas.frame_rate)?;
        let sample_time = SampleTime::new(composition_time);
        let camera = self
            .camera
            .as_ref()
            .map(|camera| evaluate_camera(camera, composition_time));
        let mut resources = Vec::new();
        let visual = evaluate_visual_program(
            &self.visual,
            &self.sources,
            &self.resources,
            frame.index(),
            composition_time,
            &mut resources,
        )?;
        let adjustments = self
            .adjustments
            .clips
            .iter()
            .filter(|effect| effect.range.contains(frame.index()))
            .cloned()
            .collect();
        let captions = evaluate_caption_program(
            &self.captions,
            &self.resources,
            frame.index(),
            composition_time,
            self.canvas.frame_rate,
            &mut resources,
        )?;

        Ok(EvaluatedRenderFrame {
            render_id: self.render_id,
            frame,
            sample_time,
            camera,
            visual,
            adjustments,
            captions,
            resources,
        })
    }
}

fn quantize_motion_source_frame(
    mapped: MappedSourceTime,
    duration: RationalTime,
    frame_rate: FrameRate,
    source_index: u32,
) -> Result<u32, RuntimeFault> {
    let frame_count = u32::try_from(
        quantized_motion_source_frame_count(duration, frame_rate)
            .map_err(|_| RuntimeFault::MotionSourceDurationMissing { source_index })?,
    )
    .map_err(|_| RuntimeFault::ExactTimeOverflow)?;
    match mapped {
        MappedSourceTime::Static | MappedSourceTime::HoldStart => Ok(0),
        MappedSourceTime::HoldEnd => Ok(frame_count - 1),
        MappedSourceTime::Exact(time) => {
            let frame = quantize_frame_boundary(time, frame_rate)
                .map_err(|_| RuntimeFault::ExactTimeOverflow)?
                .clamp(0, i64::from(frame_count) - 1);
            u32::try_from(frame).map_err(|_| RuntimeFault::ExactTimeOverflow)
        }
    }
}

/// ABI-defined discrete extent of a Motion source on its output frame clock.
fn quantized_motion_source_frame_count(
    duration: RationalTime,
    frame_rate: FrameRate,
) -> Result<i64, ()> {
    let frame_count = quantize_frame_interval(RationalTime::ZERO, duration, frame_rate)
        .map_err(|_| ())?
        .duration_frames;
    (frame_count > 0).then_some(frame_count).ok_or(())
}

fn compiled_execution_resource_ref(
    resource: &ResolvedResource,
) -> Option<CompiledExecutionResourceRef<'_>> {
    match (&resource.facts, &resource.execution) {
        (VerifiedResourceFacts::Font { bytes, .. }, _) => Some(CompiledExecutionResourceRef {
            resource_id: &resource.resource_id,
            kind: CompiledExecutionResourceKind::FontBytes,
            content_digest: &resource.digest,
            abi_digest: None,
            bytes,
        }),
        (VerifiedResourceFacts::Shader { .. }, ResolvedExecutionPayload::Shader(package)) => {
            Some(CompiledExecutionResourceRef {
                resource_id: &resource.resource_id,
                kind: CompiledExecutionResourceKind::RuntimeShader,
                content_digest: &resource.digest,
                abi_digest: Some(&package.manifest.abi_digest),
                bytes: package.generated_sksl.as_bytes(),
            })
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatedResourceRef {
    target: u32,
    resource_id: String,
    role: String,
    kind: ResourceKind,
    digest: ContentDigest,
    handle: VerifiedHandleId,
    facts: VerifiedResourceFacts,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatedSourceRef {
    source_index: u32,
    kind: CompiledSourceKind,
    mapped_time: MappedSourceTime,
    sample_time: RationalTime,
    resource: Option<EvaluatedResourceRef>,
    motion_props: Option<BTreeMap<String, EvaluatedMotionValue>>,
    motion_resources: BTreeMap<String, EvaluatedResourceRef>,
    motion_artifact_dependencies: Vec<EvaluatedResourceRef>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EvaluatedMotionValue {
    Scalar(f64),
    Vec2([f64; 2]),
    Vec4([f64; 4]),
    Boolean(bool),
    String(String),
}

impl EvaluatedSourceRef {
    pub const fn source_index(&self) -> u32 {
        self.source_index
    }

    pub const fn kind(&self) -> CompiledSourceKind {
        self.kind
    }

    pub const fn mapped_time(&self) -> MappedSourceTime {
        self.mapped_time
    }

    /// Exact discrete producer/request clock after applying the source boundary policy. The
    /// original [`MappedSourceTime`] sentinel remains available separately: Motion continuous
    /// props evaluate `HoldEnd` as a mathematical left limit, while this clock addresses the
    /// final Motion ABI frame or Lottie descriptor tick.
    pub const fn sample_time(&self) -> RationalTime {
        self.sample_time
    }

    pub fn resource(&self) -> Option<&EvaluatedResourceRef> {
        self.resource.as_ref()
    }

    pub fn motion_props(&self) -> Option<&BTreeMap<String, EvaluatedMotionValue>> {
        self.motion_props.as_ref()
    }

    pub fn motion_resources(&self) -> &BTreeMap<String, EvaluatedResourceRef> {
        &self.motion_resources
    }

    pub fn motion_artifact_dependencies(&self) -> &[EvaluatedResourceRef] {
        &self.motion_artifact_dependencies
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatedVisualLayer {
    position: [f64; 2],
    scale: [f64; 2],
    rotation: f64,
    anchor: [f64; 2],
    opacity: f64,
    blend: CompiledBlendMode,
    mask: Option<EvaluatedVisualMask>,
    filters: Vec<CompiledKernelCall>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EvaluatedVisualMask {
    Rect {
        rect: [f64; 4],
        feather: f64,
        invert: bool,
    },
    Ellipse {
        rect: [f64; 4],
        feather: f64,
        invert: bool,
    },
}

impl EvaluatedVisualLayer {
    pub const fn position(&self) -> [f64; 2] {
        self.position
    }

    pub const fn scale(&self) -> [f64; 2] {
        self.scale
    }

    pub const fn rotation(&self) -> f64 {
        self.rotation
    }

    pub const fn opacity(&self) -> f64 {
        self.opacity
    }

    pub const fn anchor(&self) -> [f64; 2] {
        self.anchor
    }

    pub const fn blend(&self) -> CompiledBlendMode {
        self.blend
    }

    pub fn mask(&self) -> Option<&EvaluatedVisualMask> {
        self.mask.as_ref()
    }

    pub fn filters(&self) -> &[CompiledKernelCall] {
        &self.filters
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatedVisualClip {
    clip_id: String,
    track_id: String,
    track_order: u32,
    source: EvaluatedSourceRef,
    layer: EvaluatedVisualLayer,
}

impl EvaluatedVisualClip {
    pub fn clip_id(&self) -> &str {
        &self.clip_id
    }

    pub fn track_id(&self) -> &str {
        &self.track_id
    }

    pub const fn track_order(&self) -> u32 {
        self.track_order
    }

    pub fn source(&self) -> &EvaluatedSourceRef {
        &self.source
    }

    pub const fn layer(&self) -> &EvaluatedVisualLayer {
        &self.layer
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatedVisualTransition {
    track_id: String,
    from_clip_id: String,
    to_clip_id: String,
    track_order: u32,
    window: FrameRange,
    progress: RationalTime,
    from_weight: f64,
    to_weight: f64,
    from: EvaluatedSourceRef,
    to: EvaluatedSourceRef,
    from_layer: EvaluatedVisualLayer,
    to_layer: EvaluatedVisualLayer,
    kernel: CompiledTransitionKernel,
}

impl EvaluatedVisualTransition {
    pub fn track_id(&self) -> &str {
        &self.track_id
    }

    pub fn from_clip_id(&self) -> &str {
        &self.from_clip_id
    }

    pub fn to_clip_id(&self) -> &str {
        &self.to_clip_id
    }

    pub const fn track_order(&self) -> u32 {
        self.track_order
    }

    pub const fn window(&self) -> FrameRange {
        self.window
    }

    pub const fn progress(&self) -> RationalTime {
        self.progress
    }

    pub const fn from_weight(&self) -> f64 {
        self.from_weight
    }

    pub const fn to_weight(&self) -> f64 {
        self.to_weight
    }

    pub fn from(&self) -> &EvaluatedSourceRef {
        &self.from
    }

    pub fn to(&self) -> &EvaluatedSourceRef {
        &self.to
    }

    pub const fn from_layer(&self) -> &EvaluatedVisualLayer {
        &self.from_layer
    }

    pub const fn to_layer(&self) -> &EvaluatedVisualLayer {
        &self.to_layer
    }

    pub const fn kernel(&self) -> &CompiledTransitionKernel {
        &self.kernel
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EvaluatedVisualOperation {
    Clip(EvaluatedVisualClip),
    Transition(EvaluatedVisualTransition),
}

impl EvaluatedVisualOperation {
    pub const fn track_order(&self) -> u32 {
        match self {
            Self::Clip(clip) => clip.track_order,
            Self::Transition(transition) => transition.track_order,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatedCaption {
    track_order: u32,
    range: FrameRange,
    local_time: RationalTime,
    runs: Vec<String>,
    run_timings: Vec<Option<CompiledCaptionRunTiming>>,
    run_styles: Vec<CompiledCaptionRunStyle>,
    font: EvaluatedResourceRef,
    shadow: Option<CompiledCaptionShadow>,
    region: [f64; 4],
    align: CompiledCaptionAlign,
    presentation: EvaluatedCaptionPresentation,
    behavior: Option<CompiledCaptionBehavior>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EvaluatedCaptionPresentation {
    opacity: f64,
    translation: [f64; 2],
    scale: f64,
    rotation: f64,
    clip_inset: [f64; 4],
    blur_sigma: f64,
}

impl EvaluatedCaptionPresentation {
    pub const fn opacity(self) -> f64 {
        self.opacity
    }

    pub const fn translation(self) -> [f64; 2] {
        self.translation
    }

    pub const fn scale(self) -> f64 {
        self.scale
    }

    pub const fn rotation(self) -> f64 {
        self.rotation
    }

    pub const fn clip_inset(self) -> [f64; 4] {
        self.clip_inset
    }

    pub const fn blur_sigma(self) -> f64 {
        self.blur_sigma
    }
}

impl EvaluatedCaption {
    pub const fn track_order(&self) -> u32 {
        self.track_order
    }

    pub fn runs(&self) -> &[String] {
        &self.runs
    }

    pub fn run_timings(&self) -> &[Option<CompiledCaptionRunTiming>] {
        &self.run_timings
    }

    pub fn run_styles(&self) -> &[CompiledCaptionRunStyle] {
        &self.run_styles
    }

    pub fn font(&self) -> &EvaluatedResourceRef {
        &self.font
    }

    pub const fn range(&self) -> FrameRange {
        self.range
    }

    /// Caption-local frame-left time. The first admitted frame is always zero,
    /// including when the authored sequence start is not frame-aligned.
    pub const fn local_time(&self) -> RationalTime {
        self.local_time
    }

    pub const fn shadow(&self) -> Option<&CompiledCaptionShadow> {
        self.shadow.as_ref()
    }

    pub const fn region(&self) -> [f64; 4] {
        self.region
    }

    pub const fn align(&self) -> CompiledCaptionAlign {
        self.align
    }

    pub const fn presentation(&self) -> EvaluatedCaptionPresentation {
        self.presentation
    }

    pub const fn behavior(&self) -> Option<&CompiledCaptionBehavior> {
        self.behavior.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EvaluatedCamera {
    center_x: f64,
    center_y: f64,
    zoom: f64,
    rotation: f64,
}

impl EvaluatedCamera {
    pub const fn center_x(self) -> f64 {
        self.center_x
    }

    pub const fn center_y(self) -> f64 {
        self.center_y
    }

    pub const fn zoom(self) -> f64 {
        self.zoom
    }

    pub const fn rotation(self) -> f64 {
        self.rotation
    }
}

impl EvaluatedResourceRef {
    pub const fn target(&self) -> u32 {
        self.target
    }

    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }

    pub fn role(&self) -> &str {
        &self.role
    }

    pub const fn kind(&self) -> ResourceKind {
        self.kind
    }

    pub fn digest(&self) -> &ContentDigest {
        &self.digest
    }

    pub const fn handle(&self) -> VerifiedHandleId {
        self.handle
    }

    pub fn facts(&self) -> &VerifiedResourceFacts {
        &self.facts
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatedRenderFrame {
    render_id: RenderId,
    frame: FrameKey,
    sample_time: SampleTime,
    camera: Option<EvaluatedCamera>,
    visual: Vec<EvaluatedVisualOperation>,
    adjustments: Vec<CompiledAdjustmentClip>,
    captions: Vec<EvaluatedCaption>,
    resources: Vec<EvaluatedResourceRef>,
}

impl EvaluatedRenderFrame {
    pub const fn render_id(&self) -> RenderId {
        self.render_id
    }

    pub const fn frame(&self) -> FrameKey {
        self.frame
    }

    pub const fn sample_time(&self) -> SampleTime {
        self.sample_time
    }

    pub const fn camera(&self) -> Option<EvaluatedCamera> {
        self.camera
    }

    pub fn visual(&self) -> &[EvaluatedVisualOperation] {
        &self.visual
    }

    pub fn adjustments(&self) -> &[CompiledAdjustmentClip] {
        &self.adjustments
    }

    pub fn captions(&self) -> &[EvaluatedCaption] {
        &self.captions
    }

    pub fn resources(&self) -> &[EvaluatedResourceRef] {
        &self.resources
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatedAudioEndpoint {
    source: EvaluatedSourceRef,
    source_sample_index: i64,
    crossfade_gain: f64,
    effect_multiplier: f64,
    gain: f64,
    pan: f64,
    left_gain: f64,
    right_gain: f64,
}

impl EvaluatedAudioEndpoint {
    pub const fn source_index(&self) -> u32 {
        self.source.source_index
    }

    pub const fn mapped_time(&self) -> MappedSourceTime {
        self.source.mapped_time
    }

    pub fn source(&self) -> &EvaluatedSourceRef {
        &self.source
    }

    /// Engine-owned source sample selection after exact source-time mapping,
    /// hold/loop policy, and the admitted source clock have been applied.
    pub const fn source_sample_index(&self) -> i64 {
        self.source_sample_index
    }

    pub const fn crossfade_gain(&self) -> f64 {
        self.crossfade_gain
    }

    pub const fn effect_multiplier(&self) -> f64 {
        self.effect_multiplier
    }

    pub const fn gain(&self) -> f64 {
        self.gain
    }

    pub const fn pan(&self) -> f64 {
        self.pan
    }

    pub const fn left_gain(&self) -> f64 {
        self.left_gain
    }

    pub const fn right_gain(&self) -> f64 {
        self.right_gain
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatedAudioTrack {
    track_order: u32,
    endpoints: Vec<EvaluatedAudioEndpoint>,
}

impl EvaluatedAudioTrack {
    pub const fn track_order(&self) -> u32 {
        self.track_order
    }

    pub fn endpoints(&self) -> &[EvaluatedAudioEndpoint] {
        &self.endpoints
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatedAudioSample {
    sample: i64,
    sample_time: RationalTime,
    tracks: Vec<EvaluatedAudioTrack>,
}

impl EvaluatedAudioSample {
    pub const fn sample(&self) -> i64 {
        self.sample
    }

    pub const fn sample_time(&self) -> RationalTime {
        self.sample_time
    }

    pub fn tracks(&self) -> &[EvaluatedAudioTrack] {
        &self.tracks
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatedAudioBlock {
    range: SampleRange,
    samples: Vec<EvaluatedAudioSample>,
}

impl EvaluatedAudioBlock {
    pub const fn range(&self) -> SampleRange {
        self.range
    }

    pub fn samples(&self) -> &[EvaluatedAudioSample] {
        &self.samples
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeFault {
    #[error("runtime query names a different RenderId")]
    RenderMismatch,
    #[error("frame {frame} is outside [0, {frame_count})")]
    FrameOutOfRange { frame: i64, frame_count: i64 },
    #[error(
        "compiled source {source_index} is outside the source catalog of length {source_count}"
    )]
    SourceOutOfRange {
        source_index: u32,
        source_count: usize,
    },
    #[error("compiled source {source_index} is not a Motion source")]
    MotionSourceRequired { source_index: u32 },
    #[error("compiled Motion source {source_index} has no positive admitted duration")]
    MotionSourceDurationMissing { source_index: u32 },
    #[error("compiled visual source {source_index} has no admitted HoldEnd left-limit sample")]
    VisualSourceHoldEndMissing { source_index: u32 },
    #[error("sample {sample} is outside [0, {sample_count})")]
    SampleOutOfRange { sample: i64, sample_count: i64 },
    #[error("sample range [{start}, {end}) is outside [0, {limit})")]
    SampleRangeOutOfRange { start: i64, end: i64, limit: i64 },
    #[error("exact frame sample time overflow")]
    ExactTimeOverflow,
    #[error("source sample {sample} is outside [0, {sample_count})")]
    SourceSampleOutOfRange { sample: i64, sample_count: i64 },
}

/// Compiles one already-decoded Timeline/resource pair after the fixed-package
/// verifier has bound their exact bytes to the package manifest.
pub(crate) fn compile_render_input(
    timeline: &CanonicalTimeline,
    manifest: &ResourceManifest,
    bindings: &ResourceBindings,
    capabilities: &Capabilities,
    profile: &ExecutionProfile,
) -> Result<CompiledRender, EngineOpenReport> {
    admission::open_engine_render(timeline, manifest, bindings, capabilities, profile)
}

struct Resolver<'a> {
    manifest: &'a ResourceManifest,
    bindings: &'a ResourceBindings,
    capabilities: &'a Capabilities,
    limits: ExecutionLimits,
    resources: Vec<ResolvedResource>,
    index_by_id: BTreeMap<String, u32>,
    visiting: BTreeSet<String>,
    diagnostics: Vec<EngineOpenDiagnostic>,
}

impl<'a> Resolver<'a> {
    fn new(
        manifest: &'a ResourceManifest,
        bindings: &'a ResourceBindings,
        capabilities: &'a Capabilities,
        limits: ExecutionLimits,
    ) -> Self {
        Self {
            manifest,
            bindings,
            capabilities,
            limits,
            resources: Vec::new(),
            index_by_id: BTreeMap::new(),
            visiting: BTreeSet::new(),
            diagnostics: Vec::new(),
        }
    }

    fn resolve(
        &mut self,
        resource_id: &str,
        expected_kind: Option<ResourceKind>,
        path: &str,
        depth: u16,
    ) -> Option<u32> {
        if depth > self.limits.max_dependency_depth {
            self.diagnostics.push(diagnostic(
                EngineOpenDiagnosticCode::DependencyDepthExceeded,
                path,
                EngineOpenPhase::ResourceResolve,
                Some(resource_id),
                details([("limit", self.limits.max_dependency_depth.to_string())]),
            ));
            return None;
        }
        if let Some(index) = self.index_by_id.get(resource_id).copied() {
            let actual = self.resources[index as usize].kind;
            if expected_kind.is_some_and(|expected| expected != actual) {
                self.kind_mismatch(path, resource_id, expected_kind.unwrap(), actual);
                return None;
            }
            return Some(index);
        }
        if !self.visiting.insert(resource_id.to_owned()) {
            self.diagnostics.push(diagnostic(
                EngineOpenDiagnosticCode::DependencyCycle,
                path,
                EngineOpenPhase::ResourceResolve,
                Some(resource_id),
                BTreeMap::new(),
            ));
            return None;
        }
        if self.resources.len() + self.visiting.len() > self.limits.max_resources as usize {
            self.diagnostics.push(diagnostic(
                EngineOpenDiagnosticCode::ResourceBudgetExceeded,
                path,
                EngineOpenPhase::ResourceResolve,
                Some(resource_id),
                details([("limit", self.limits.max_resources.to_string())]),
            ));
            self.visiting.remove(resource_id);
            return None;
        }

        let Some(entry) = self.manifest.entries().get(resource_id) else {
            self.diagnostics.push(diagnostic(
                EngineOpenDiagnosticCode::MissingManifestResource,
                path,
                EngineOpenPhase::ResourceResolve,
                Some(resource_id),
                BTreeMap::new(),
            ));
            self.visiting.remove(resource_id);
            return None;
        };
        let actual_kind = resource_kind(entry);
        if expected_kind.is_some_and(|expected| expected != actual_kind) {
            self.kind_mismatch(path, resource_id, expected_kind.unwrap(), actual_kind);
            self.visiting.remove(resource_id);
            return None;
        }
        let Some(binding) = self.bindings.get(resource_id) else {
            self.diagnostics.push(diagnostic(
                EngineOpenDiagnosticCode::MissingResourceBinding,
                path,
                EngineOpenPhase::ResourceResolve,
                Some(resource_id),
                BTreeMap::new(),
            ));
            self.visiting.remove(resource_id);
            return None;
        };
        let digest = resource_digest(entry);
        if digest != binding.digest() {
            self.diagnostics.push(diagnostic(
                EngineOpenDiagnosticCode::BindingDigestMismatch,
                path,
                EngineOpenPhase::ResourceResolve,
                Some(resource_id),
                details([
                    ("expected", digest.to_wire()),
                    ("actual", binding.digest().to_wire()),
                ]),
            ));
            self.visiting.remove(resource_id);
            return None;
        }
        if let (
            ResourceEntryWire::MotionArtifact { .. },
            VerifiedResourceFacts::MotionArtifact { artifact, .. },
        ) = (entry, binding.facts())
        {
            let artifact_valid = artifact.validate().is_ok();
            let artifact_digest = valle_motion::canonical_bytes(artifact.as_ref())
                .ok()
                .map(|bytes| ContentDigest::of_bytes(&bytes));
            if !artifact_valid || artifact_digest.as_ref() != Some(digest) {
                self.diagnostics.push(diagnostic(
                    EngineOpenDiagnosticCode::MotionArtifactPayloadMismatch,
                    format!("{path}/verifiedFacts/artifact"),
                    EngineOpenPhase::ResourceResolve,
                    Some(resource_id),
                    details([("reason", "artifact-validation-or-digest".to_owned())]),
                ));
                self.visiting.remove(resource_id);
                return None;
            }
        }
        let execution = match verify_execution_payload(entry, binding.facts(), digest) {
            Ok(payload) => payload,
            Err((code, reason)) => {
                self.diagnostics.push(diagnostic(
                    code,
                    format!("{path}/verifiedFacts"),
                    EngineOpenPhase::ResourceResolve,
                    Some(resource_id),
                    details([("reason", reason)]),
                ));
                self.visiting.remove(resource_id);
                return None;
            }
        };
        if !binding_facts_match(entry, binding.facts()) {
            self.diagnostics.push(diagnostic(
                EngineOpenDiagnosticCode::BindingFactsMismatch,
                format!("{path}/verifiedFacts"),
                EngineOpenPhase::ResourceResolve,
                Some(resource_id),
                details([
                    ("expectedKind", format!("{actual_kind:?}")),
                    ("actualKind", format!("{:?}", binding.facts().kind())),
                ]),
            ));
            self.visiting.remove(resource_id);
            return None;
        }
        if let Some(abi) = resource_abi(entry)
            && !self.capabilities.supports_artifact_abi(abi)
        {
            self.diagnostics.push(diagnostic(
                EngineOpenDiagnosticCode::UnsupportedArtifactAbi,
                path,
                EngineOpenPhase::Admission,
                Some(resource_id),
                details([("abi", abi.to_owned())]),
            ));
            self.visiting.remove(resource_id);
            return None;
        }

        let mut dependencies = Vec::with_capacity(binding.dependencies().len());
        for dependency in binding.dependencies() {
            let dependency_path = format!("{path}/dependencies/{}", dependencies.len());
            if let Some(target) =
                self.resolve(dependency.resource_id(), None, &dependency_path, depth + 1)
            {
                dependencies.push(ResolvedDependencyProjection {
                    role: dependency.role().to_owned(),
                    target,
                });
            }
        }
        if let VerifiedResourceFacts::MotionArtifact { artifact, .. } = binding.facts() {
            for artifact_ref in &artifact.resource_refs {
                let matched = dependencies.iter().any(|dependency| {
                    if dependency.role != artifact_ref.control {
                        return false;
                    }
                    self.resources
                        .get(dependency.target as usize)
                        .is_some_and(|resource| resource.digest == artifact_ref.content_hash)
                });
                if !matched {
                    self.diagnostics.push(diagnostic(
                        EngineOpenDiagnosticCode::MotionResourceSchemaMismatch,
                        format!("{path}/dependencies/{}", artifact_ref.control),
                        EngineOpenPhase::ResourceResolve,
                        Some(resource_id),
                        details([("reason", "artifact-resource-ref-mismatch".to_owned())]),
                    ));
                }
            }
        }
        self.visiting.remove(resource_id);
        if dependencies.len() != binding.dependencies().len()
            || self.diagnostics.iter().any(|diagnostic| {
                diagnostic.resource_id.as_deref() == Some(resource_id)
                    && diagnostic.code == EngineOpenDiagnosticCode::MotionResourceSchemaMismatch
            })
        {
            return None;
        }

        let index = self.resources.len() as u32;
        self.resources.push(ResolvedResource {
            resource_id: resource_id.to_owned(),
            kind: actual_kind,
            digest: digest.clone(),
            handle: binding.handle(),
            facts: binding.facts().clone(),
            execution,
            entry: entry.clone(),
            dependencies,
        });
        self.index_by_id.insert(resource_id.to_owned(), index);
        Some(index)
    }

    fn kind_mismatch(
        &mut self,
        path: &str,
        resource_id: &str,
        expected: ResourceKind,
        actual: ResourceKind,
    ) {
        self.diagnostics.push(diagnostic(
            EngineOpenDiagnosticCode::ResourceKindMismatch,
            path,
            EngineOpenPhase::Admission,
            Some(resource_id),
            details([
                ("expected", format!("{expected:?}")),
                ("actual", format!("{actual:?}")),
            ]),
        ));
    }
}

fn collect_document_requirements(
    timeline: &CanonicalTimeline,
) -> (Vec<ResourceUse>, Vec<KernelRequirement>) {
    let document = timeline.document();
    let mut resources = Vec::new();
    let mut kernels = Vec::new();

    for (track_index, track) in document.visual.tracks.iter().enumerate() {
        for (item_index, item) in track.items.iter().enumerate() {
            let item_path = format!("/document/visual/tracks/{track_index}/items/{item_index}");
            match item {
                VisualItem::Clip(clip) => {
                    let (resource_id, expected_kind, source_name) = match &clip.source {
                        VisualSource::Video(source) => {
                            (Some(&source.resource), Some(ResourceKind::Video), "video")
                        }
                        VisualSource::Image(source) => {
                            (Some(&source.resource), Some(ResourceKind::Image), "image")
                        }
                        VisualSource::Lottie(source) => {
                            (Some(&source.resource), Some(ResourceKind::Lottie), "lottie")
                        }
                        VisualSource::Motion(source) => {
                            resources.push(ResourceUse {
                                role: format!("visual/{track_index}/{item_index}/motion-component"),
                                path: format!("{item_path}/source/component"),
                                resource_id: source.component.clone(),
                                expected_kind: Some(ResourceKind::MotionArtifact),
                            });
                            for (name, resource_id) in &source.resources {
                                resources.push(ResourceUse {
                                    role: format!(
                                        "visual/{track_index}/{item_index}/motion-resource/{name}"
                                    ),
                                    path: format!("{item_path}/source/resources/{name}"),
                                    resource_id: resource_id.clone(),
                                    expected_kind: None,
                                });
                            }
                            (None, None, "motion")
                        }
                        VisualSource::Solid(_) => (None, None, "solid"),
                    };
                    if let Some(resource_id) = resource_id {
                        resources.push(ResourceUse {
                            role: format!("visual/{track_index}/{item_index}/{source_name}"),
                            path: format!("{item_path}/source/resource"),
                            resource_id: resource_id.clone(),
                            expected_kind,
                        });
                    }
                    for (filter_index, filter) in clip.layer.filters.iter().enumerate() {
                        kernels.push(KernelRequirement {
                            role: format!("{item_path}/layer/filters/{filter_index}"),
                            kind: filter.kind.as_str().to_owned(),
                            domain: CompiledKernelDomain::VisualFilter,
                            parameters: filter.parameters.clone(),
                        });
                    }
                }
                VisualItem::Transition(transition) => {
                    if let TransitionKernel::Extension(extension) = &transition.kernel {
                        kernels.push(KernelRequirement {
                            role: format!("{item_path}/kernel"),
                            kind: extension.kind.as_str().to_owned(),
                            domain: CompiledKernelDomain::VisualTransition,
                            parameters: extension.parameters.clone(),
                        });
                    }
                }
                VisualItem::Gap(_) => {}
            }
        }
    }

    for (track_index, track) in document.audio.tracks.iter().enumerate() {
        for (item_index, item) in track.items.iter().enumerate() {
            if let AudioItem::Clip(clip) = item {
                let AudioSource::Media(source) = &clip.source;
                let item_path = format!("/document/audio/tracks/{track_index}/items/{item_index}");
                resources.push(ResourceUse {
                    role: format!("audio/{track_index}/{item_index}/media"),
                    path: format!("{item_path}/source/resource"),
                    resource_id: source.resource.clone(),
                    expected_kind: Some(ResourceKind::Audio),
                });
                for (effect_index, effect) in clip.effects.iter().enumerate() {
                    kernels.push(KernelRequirement {
                        role: format!("{item_path}/effects/{effect_index}"),
                        kind: effect.kind.as_str().to_owned(),
                        domain: CompiledKernelDomain::AudioEffect,
                        parameters: effect.parameters.clone(),
                    });
                }
            }
        }
    }

    for (effect_index, effect) in document.adjustments.iter().enumerate() {
        if let AdjustmentEffect::Extension(extension) = &effect.effect {
            kernels.push(KernelRequirement {
                role: format!("/document/adjustments/{effect_index}/effect"),
                kind: extension.kind.as_str().to_owned(),
                domain: CompiledKernelDomain::AdjustmentEffect,
                parameters: extension.parameters.clone(),
            });
        }
    }

    for (track_index, track) in document.captions.tracks.iter().enumerate() {
        for (item_index, item) in track.items.iter().enumerate() {
            if let CaptionItem::Clip(caption) = item {
                let item_path =
                    format!("/document/captions/tracks/{track_index}/items/{item_index}");
                resources.push(ResourceUse {
                    role: format!("caption/{track_index}/{item_index}/font"),
                    path: format!("{item_path}/style/font"),
                    resource_id: caption.style.font.clone(),
                    expected_kind: Some(ResourceKind::Font),
                });
            }
        }
    }

    (resources, kernels)
}

/// Exact logical resource roots referenced by the canonical document. Fixed
/// package decoding uses the same collector as Engine admission so unused
/// binding locators and payloads remain outside the verified closure.
pub(crate) fn document_resource_root_ids(timeline: &CanonicalTimeline) -> BTreeSet<String> {
    collect_document_requirements(timeline)
        .0
        .into_iter()
        .map(|resource_use| resource_use.resource_id)
        .collect()
}

fn admit_canvas(
    timeline: &CanonicalTimeline,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> Option<CompiledCanvas> {
    let canvas = &timeline.document().canvas;
    let frame_rate = canvas.fps;
    let duration = canvas.duration;
    let frame_count = match quantize_frame_boundary(duration, frame_rate) {
        Ok(value) if (1..=MAX_SAFE_JSON_INTEGER).contains(&value) => value,
        _ => {
            diagnostics.push(canvas_diagnostic("/document/canvas/duration"));
            return None;
        }
    };
    let sample_count = match quantize_sample_boundary(duration, canvas.sample_rate) {
        Ok(value) if (1..=MAX_SAFE_JSON_INTEGER).contains(&value) => value,
        _ => {
            diagnostics.push(canvas_diagnostic("/document/canvas/sampleRate"));
            return None;
        }
    };
    let background_rgba = match Color(timeline.document().background.color.clone()).parse_rgba() {
        Some(color) => color,
        None => {
            diagnostics.push(canvas_diagnostic("/document/background/color"));
            return None;
        }
    };
    Some(CompiledCanvas {
        width: canvas.width,
        height: canvas.height,
        duration,
        frame_rate,
        sample_rate: canvas.sample_rate,
        channel_layout: match canvas.channel_layout {
            ChannelLayout::Mono => CompiledChannelLayout::Mono,
            ChannelLayout::Stereo => CompiledChannelLayout::Stereo,
        },
        color_space: match canvas.color_space {
            ColorSpace::Srgb => CompiledColorSpace::Srgb,
        },
        background_rgba,
        frame_count,
        sample_count,
    })
}

fn validate_quantized_intervals(
    timeline: &CanonicalTimeline,
    canvas: CompiledCanvas,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) {
    let document = timeline.document();

    for (track_index, track) in document.visual.tracks.iter().enumerate() {
        let mut cursor = RationalTime::ZERO;
        for (item_index, item) in track.items.iter().enumerate() {
            let path = format!("/document/visual/tracks/{track_index}/items/{item_index}/duration");
            match item {
                VisualItem::Clip(clip) => {
                    cursor = validate_frame_interval(
                        cursor,
                        clip.duration,
                        canvas.frame_rate,
                        &path,
                        true,
                        diagnostics,
                    );
                }
                VisualItem::Gap(gap) => {
                    cursor = validate_frame_interval(
                        cursor,
                        gap.duration,
                        canvas.frame_rate,
                        &path,
                        false,
                        diagnostics,
                    );
                }
                VisualItem::Transition(transition) => {
                    validate_frame_interval(
                        RationalTime::ZERO,
                        transition.duration,
                        canvas.frame_rate,
                        &path,
                        true,
                        diagnostics,
                    );
                }
            }
        }
    }

    for (track_index, track) in document.audio.tracks.iter().enumerate() {
        let mut cursor = RationalTime::ZERO;
        for (item_index, item) in track.items.iter().enumerate() {
            let path = format!("/document/audio/tracks/{track_index}/items/{item_index}/duration");
            match item {
                AudioItem::Clip(clip) => {
                    cursor = validate_sample_interval(
                        cursor,
                        clip.duration,
                        canvas.sample_rate,
                        &path,
                        true,
                        diagnostics,
                    );
                }
                AudioItem::Gap(gap) => {
                    cursor = validate_sample_interval(
                        cursor,
                        gap.duration,
                        canvas.sample_rate,
                        &path,
                        false,
                        diagnostics,
                    );
                }
                AudioItem::Crossfade(crossfade) => {
                    validate_sample_interval(
                        RationalTime::ZERO,
                        crossfade.duration,
                        canvas.sample_rate,
                        &path,
                        true,
                        diagnostics,
                    );
                }
            }
        }
    }

    for (track_index, track) in document.captions.tracks.iter().enumerate() {
        let mut cursor = RationalTime::ZERO;
        for (item_index, item) in track.items.iter().enumerate() {
            let path =
                format!("/document/captions/tracks/{track_index}/items/{item_index}/duration");
            match item {
                CaptionItem::Clip(caption) => {
                    cursor = validate_frame_interval(
                        cursor,
                        caption.duration,
                        canvas.frame_rate,
                        &path,
                        true,
                        diagnostics,
                    );
                }
                CaptionItem::Gap(gap) => {
                    cursor = validate_frame_interval(
                        cursor,
                        gap.duration,
                        canvas.frame_rate,
                        &path,
                        false,
                        diagnostics,
                    );
                }
            }
        }
    }

    for (effect_index, effect) in document.adjustments.iter().enumerate() {
        validate_frame_interval(
            effect.start,
            effect.duration,
            canvas.frame_rate,
            &format!("/document/adjustments/{effect_index}/duration"),
            true,
            diagnostics,
        );
    }
}

fn validate_frame_interval(
    start: RationalTime,
    duration: RationalTime,
    frame_rate: FrameRate,
    path: &str,
    require_nonempty: bool,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> RationalTime {
    let end = match start.checked_add(duration) {
        Ok(end) => end,
        Err(_) => {
            diagnostics.push(interval_quantization_diagnostic(path));
            return start;
        }
    };
    match quantize_frame_interval(start, duration, frame_rate) {
        Ok(interval) if require_nonempty && interval.duration_frames <= 0 => {
            diagnostics.push(diagnostic(
                EngineOpenDiagnosticCode::PositiveDurationQuantizedToZeroFrame,
                path,
                EngineOpenPhase::Admission,
                None,
                details([
                    ("startFrame", interval.start_frame.to_string()),
                    ("endFrame", interval.end_frame.to_string()),
                ]),
            ));
        }
        Ok(_) => {}
        Err(_) => diagnostics.push(interval_quantization_diagnostic(path)),
    }
    end
}

fn validate_sample_interval(
    start: RationalTime,
    duration: RationalTime,
    sample_rate: u32,
    path: &str,
    require_nonempty: bool,
    diagnostics: &mut Vec<EngineOpenDiagnostic>,
) -> RationalTime {
    let end = match start.checked_add(duration) {
        Ok(end) => end,
        Err(_) => {
            diagnostics.push(interval_quantization_diagnostic(path));
            return start;
        }
    };
    match quantize_sample_interval(start, duration, sample_rate) {
        Ok(interval) if require_nonempty && interval.duration_samples <= 0 => {
            diagnostics.push(diagnostic(
                EngineOpenDiagnosticCode::PositiveDurationQuantizedToZeroSample,
                path,
                EngineOpenPhase::Admission,
                None,
                details([
                    ("startSample", interval.start_sample.to_string()),
                    ("endSample", interval.end_sample.to_string()),
                ]),
            ));
        }
        Ok(_) => {}
        Err(_) => diagnostics.push(interval_quantization_diagnostic(path)),
    }
    end
}

fn interval_quantization_diagnostic(path: &str) -> EngineOpenDiagnostic {
    diagnostic(
        EngineOpenDiagnosticCode::InvalidIntervalQuantization,
        path,
        EngineOpenPhase::Admission,
        None,
        BTreeMap::new(),
    )
}

fn resource_kind(entry: &ResourceEntryWire) -> ResourceKind {
    match entry {
        ResourceEntryWire::Video { .. } => ResourceKind::Video,
        ResourceEntryWire::Audio { .. } => ResourceKind::Audio,
        ResourceEntryWire::Image { .. } => ResourceKind::Image,
        ResourceEntryWire::Lottie { .. } => ResourceKind::Lottie,
        ResourceEntryWire::Font { .. } => ResourceKind::Font,
        ResourceEntryWire::MotionArtifact { .. } => ResourceKind::MotionArtifact,
        ResourceEntryWire::Shader { .. } => ResourceKind::Shader,
    }
}

fn resource_digest(entry: &ResourceEntryWire) -> &ContentDigest {
    match entry {
        ResourceEntryWire::Video { digest, .. }
        | ResourceEntryWire::Audio { digest, .. }
        | ResourceEntryWire::Image { digest, .. }
        | ResourceEntryWire::Lottie { digest, .. }
        | ResourceEntryWire::Font { digest, .. }
        | ResourceEntryWire::MotionArtifact { digest, .. }
        | ResourceEntryWire::Shader { digest, .. } => digest,
    }
}

fn resource_abi(entry: &ResourceEntryWire) -> Option<&'static str> {
    match entry {
        ResourceEntryWire::MotionArtifact { .. } => Some("valle.motion/artifact@1"),
        ResourceEntryWire::Lottie { .. } => Some("valle.lottie/artifact@1"),
        ResourceEntryWire::Shader { .. } => Some("valle.shader/artifact@1"),
        _ => None,
    }
}

fn binding_facts_match(entry: &ResourceEntryWire, facts: &VerifiedResourceFacts) -> bool {
    match (entry, facts) {
        (
            ResourceEntryWire::Video { descriptor, .. },
            VerifiedResourceFacts::Video {
                descriptor: verified,
                ..
            },
        ) => descriptor == verified,
        (
            ResourceEntryWire::Audio { descriptor, .. },
            VerifiedResourceFacts::Audio {
                descriptor: verified,
                ..
            },
        ) => descriptor == verified,
        (
            ResourceEntryWire::Image { descriptor, .. },
            VerifiedResourceFacts::Image {
                descriptor: verified,
                ..
            },
        ) => descriptor == verified,
        (
            ResourceEntryWire::Lottie {
                abi, descriptor, ..
            },
            VerifiedResourceFacts::Lottie {
                abi: verified_abi,
                descriptor: verified,
                ..
            },
        ) => abi == verified_abi && descriptor == verified,
        (
            ResourceEntryWire::Font { descriptor, .. },
            VerifiedResourceFacts::Font {
                descriptor: verified,
                ..
            },
        ) => descriptor == verified,
        (
            ResourceEntryWire::MotionArtifact {
                abi, descriptor, ..
            },
            VerifiedResourceFacts::MotionArtifact {
                abi: verified_abi,
                descriptor: verified,
                ..
            },
        ) => abi == verified_abi && descriptor == verified,
        (
            ResourceEntryWire::Shader {
                abi, descriptor, ..
            },
            VerifiedResourceFacts::Shader {
                abi: verified_abi,
                descriptor: verified,
                ..
            },
        ) => abi == verified_abi && descriptor == verified,
        _ => false,
    }
}

fn verify_execution_payload(
    entry: &ResourceEntryWire,
    facts: &VerifiedResourceFacts,
    digest: &ContentDigest,
) -> Result<ResolvedExecutionPayload, (EngineOpenDiagnosticCode, String)> {
    match (entry, facts) {
        (ResourceEntryWire::Audio { .. }, VerifiedResourceFacts::Audio { .. }) => {
            Ok(ResolvedExecutionPayload::None)
        }
        (ResourceEntryWire::Font { .. }, VerifiedResourceFacts::Font { descriptor, bytes }) => {
            if &ContentDigest::of_bytes(bytes) != digest {
                return Err((
                    EngineOpenDiagnosticCode::FontPayloadMismatch,
                    "content-digest".to_owned(),
                ));
            }
            let face = ttf_parser::Face::parse(bytes, descriptor.face_index).map_err(|_| {
                (
                    EngineOpenDiagnosticCode::FontPayloadMismatch,
                    "font-face-parse".to_owned(),
                )
            })?;
            let concrete_axes = face
                .variation_axes()
                .into_iter()
                .map(|axis| {
                    (
                        axis.tag.to_string(),
                        FontVariationAxisWire {
                            minimum: f64::from(axis.min_value),
                            default: f64::from(axis.def_value),
                            maximum: f64::from(axis.max_value),
                        },
                    )
                })
                .collect();
            if descriptor.variation_axes != concrete_axes {
                return Err((
                    EngineOpenDiagnosticCode::FontPayloadMismatch,
                    "variation-axes".to_owned(),
                ));
            }
            Ok(ResolvedExecutionPayload::None)
        }
        (
            ResourceEntryWire::MotionArtifact { descriptor, .. },
            VerifiedResourceFacts::MotionArtifact { artifact, .. },
        ) => {
            if descriptor.reads_destination != artifact.reads_destination() {
                return Err((
                    EngineOpenDiagnosticCode::MotionArtifactPayloadMismatch,
                    "reads-destination".to_owned(),
                ));
            }
            valle_motion::prepare_scene(artifact)
                .map(|scene| ResolvedExecutionPayload::Motion(Arc::new(scene)))
                .map_err(|error| {
                    (
                        EngineOpenDiagnosticCode::MotionArtifactPayloadMismatch,
                        format!("layout-admission:{error}"),
                    )
                })
        }
        (
            ResourceEntryWire::Shader { descriptor, .. },
            VerifiedResourceFacts::Shader {
                manifest_bytes,
                source_bytes,
                ..
            },
        ) => {
            let package = ShaderPackage::admit(manifest_bytes, source_bytes).map_err(|error| {
                (
                    EngineOpenDiagnosticCode::ShaderPayloadMismatch,
                    format!("package-admission:{:?}", error.code),
                )
            })?;
            let package_digest = package.content_hash;
            let abi_digest = package.manifest.abi_digest;
            if &package_digest != digest {
                return Err((
                    EngineOpenDiagnosticCode::ShaderPayloadMismatch,
                    "content-digest".to_owned(),
                ));
            }
            if abi_digest != descriptor.controls_schema_digest {
                return Err((
                    EngineOpenDiagnosticCode::ShaderPayloadMismatch,
                    "controls-schema-digest".to_owned(),
                ));
            }
            Ok(ResolvedExecutionPayload::Shader(Arc::new(package)))
        }
        _ => Ok(ResolvedExecutionPayload::None),
    }
}

fn diagnostic(
    code: EngineOpenDiagnosticCode,
    path: impl Into<String>,
    phase: EngineOpenPhase,
    resource_id: Option<&str>,
    details: BTreeMap<String, String>,
) -> EngineOpenDiagnostic {
    assert!(
        phase != EngineOpenPhase::Compile || code == EngineOpenDiagnosticCode::InternalCompileFault,
        "compile-phase diagnostics are reserved for internal compiler faults"
    );
    EngineOpenDiagnostic {
        code,
        path: path.into(),
        phase,
        severity: DiagnosticSeverity::Error,
        resource_id: resource_id.map(str::to_owned),
        details,
    }
}

fn canvas_diagnostic(path: &str) -> EngineOpenDiagnostic {
    diagnostic(
        EngineOpenDiagnosticCode::InvalidCanvasQuantization,
        path,
        EngineOpenPhase::Admission,
        None,
        BTreeMap::new(),
    )
}

fn admit_fault(path: &str, reason: &str) -> EngineOpenDiagnostic {
    diagnostic(
        EngineOpenDiagnosticCode::InternalCompileFault,
        path,
        EngineOpenPhase::Compile,
        None,
        details([("reason", reason.to_owned())]),
    )
}

fn internal_compile_report(path: &str, reason: &str) -> EngineOpenReport {
    EngineOpenReport {
        diagnostics: vec![admit_fault(path, reason)],
    }
}

fn details<const N: usize>(items: [(&str, String); N]) -> BTreeMap<String, String> {
    items
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

#[cfg(test)]
mod compiled_canvas_tests {
    use super::*;

    fn boundary_audio_source(end_behavior: CompiledEndBehavior) -> CompiledSource {
        CompiledSource {
            kind: CompiledSourceKind::Audio,
            resource_target: Some(0),
            placement_start: RationalTime::new(1, 4).unwrap(),
            source_start: RationalTime::ZERO,
            source_duration: Some(RationalTime::ONE),
            hold_end_time: None,
            rate: RationalRate::ONE,
            end_behavior,
            dependency_range: CompiledDependencyRange::Samples {
                range: SampleRange::admitted(0, 4),
            },
            payload: CompiledSourcePayload::Audio {
                channel_map: CompiledAudioChannelMap::StereoIdentity,
                source_sample_rate: 4,
                source_sample_count: 4,
            },
        }
    }

    fn non_aligned_canvas() -> CompiledCanvas {
        CompiledCanvas {
            width: 16,
            height: 16,
            duration: RationalTime::new(1, 10).unwrap(),
            frame_rate: FrameRate::new(24, 1).unwrap(),
            sample_rate: 48_000,
            channel_layout: CompiledChannelLayout::Stereo,
            color_space: CompiledColorSpace::Srgb,
            background_rgba: [0, 0, 0, 0],
            frame_count: 2,
            sample_count: 4_800,
        }
    }

    #[test]
    fn raw_kernel_digest_adapts_the_typed_implementation_digest() {
        for abi in [
            EXTENSION_COLOR_GAIN_ABI,
            EXTENSION_CROSS_FADE_ABI,
            AUDIO_GAIN_EFFECT_ABI,
        ] {
            let digest = engine_owned_kernel_implementation_digest(abi).unwrap();
            assert_eq!(
                engine_owned_kernel_implementation_sha256(abi),
                Some(*digest.as_bytes())
            );
        }
        assert_eq!(engine_owned_kernel_implementation_digest("unknown"), None);
        assert_eq!(engine_owned_kernel_implementation_sha256("unknown"), None);
    }

    #[test]
    fn seconds_clocks_are_half_away_and_explicit_about_end_boundaries() {
        let canvas = non_aligned_canvas();
        assert_eq!(canvas.frame_at_seconds(1.0 / 48.0).unwrap().index(), 1);
        assert!(canvas.frame_at_seconds(f64::NAN).is_err());
        assert!(canvas.frame_at_seconds(-0.1).is_err());
        assert_eq!(canvas.sample_at_seconds(0.05).unwrap(), 2_400);
        assert_eq!(canvas.sample_boundary_at_seconds(0.1).unwrap(), 4_800);
        assert!(canvas.sample_at_seconds(0.1).is_err());
    }

    #[test]
    fn terminal_frame_boundary_uses_independently_admitted_sample_end() {
        let canvas = non_aligned_canvas();
        assert_eq!(canvas.sample_boundary_at_frame(1).unwrap(), 2_000);
        // frame_count/fps is 1/12s, while the exact duration is 1/10s.
        assert_eq!(canvas.sample_boundary_at_frame(2).unwrap(), 4_800);
        assert!(canvas.sample_boundary_at_frame(3).is_err());
    }

    #[test]
    fn motion_source_frame_quantization_is_exact_and_clamps_end_sentinels() {
        let fps = FrameRate::new(30, 1).unwrap();
        let duration = RationalTime::new(1, 1).unwrap();
        assert_eq!(
            quantize_motion_source_frame(
                MappedSourceTime::Exact(RationalTime::new(7, 30).unwrap()),
                duration,
                fps,
                0,
            )
            .unwrap(),
            7
        );
        assert_eq!(
            quantize_motion_source_frame(MappedSourceTime::HoldStart, duration, fps, 0).unwrap(),
            0
        );
        assert_eq!(
            quantize_motion_source_frame(MappedSourceTime::HoldEnd, duration, fps, 0).unwrap(),
            29
        );
        // Exact duration is a boundary sentinel, never a source frame 30.
        assert_eq!(
            quantize_motion_source_frame(MappedSourceTime::Exact(duration), duration, fps, 0,)
                .unwrap(),
            29
        );
    }

    #[test]
    fn audio_source_boundaries_are_closed_for_error_hold_and_euclidean_loop() {
        let before_start = RationalTime::ZERO;
        let exact_end = RationalTime::new(5, 4).unwrap();

        let error = boundary_audio_source(CompiledEndBehavior::Error);
        let negative = error.map_time(before_start).unwrap();
        assert_eq!(
            negative,
            MappedSourceTime::Exact(RationalTime::new(-1, 4).unwrap())
        );
        assert!(matches!(
            evaluate_source_sample_index(&error, negative),
            Err(RuntimeFault::SourceSampleOutOfRange {
                sample: -1,
                sample_count: 4
            })
        ));
        let duration_sentinel = error.map_time(exact_end).unwrap();
        assert_eq!(
            duration_sentinel,
            MappedSourceTime::Exact(RationalTime::ONE)
        );
        assert!(matches!(
            evaluate_source_sample_index(&error, duration_sentinel),
            Err(RuntimeFault::SourceSampleOutOfRange {
                sample: 4,
                sample_count: 4
            })
        ));

        let hold = boundary_audio_source(CompiledEndBehavior::Hold);
        assert_eq!(
            hold.map_time(before_start).unwrap(),
            MappedSourceTime::HoldStart
        );
        assert_eq!(
            evaluate_source_sample_index(&hold, MappedSourceTime::HoldStart).unwrap(),
            0
        );
        assert_eq!(hold.map_time(exact_end).unwrap(), MappedSourceTime::HoldEnd);
        assert_eq!(
            evaluate_source_sample_index(&hold, MappedSourceTime::HoldEnd).unwrap(),
            3
        );

        let looping = boundary_audio_source(CompiledEndBehavior::Loop);
        let wrapped_negative = looping.map_time(before_start).unwrap();
        assert_eq!(
            wrapped_negative,
            MappedSourceTime::Exact(RationalTime::new(3, 4).unwrap())
        );
        assert_eq!(
            evaluate_source_sample_index(&looping, wrapped_negative).unwrap(),
            3
        );
        let wrapped_end = looping.map_time(exact_end).unwrap();
        assert_eq!(wrapped_end, MappedSourceTime::Exact(RationalTime::ZERO));
        assert_eq!(
            evaluate_source_sample_index(&looping, wrapped_end).unwrap(),
            0
        );
    }

    #[test]
    fn common_audio_pcm_digest_has_a_fixed_cross_platform_byte_domain() {
        assert_eq!(
            common_audio_pcm_digest(4, 1, &[0.1, 0.2])
                .unwrap()
                .to_wire(),
            "sha256:cfe5b3919c9c55851665cf86e0cc1c9409b64e94f9854e6ea8729ad2c9499960"
        );
        assert_eq!(
            common_audio_pcm_digest(4, 2, &[0.0]),
            Err(CommonAudioPcmDigestError::PartialFrame)
        );
        assert_eq!(
            common_audio_pcm_digest(4, 1, &[f32::NAN]),
            Err(CommonAudioPcmDigestError::NonFiniteSample)
        );
    }

    #[test]
    fn compiled_execution_resources_expose_frozen_font_bytes() {
        let font_bytes: Arc<[u8]> = Arc::from(&b"font fixture"[..]);
        let font_digest = content_digest(&font_bytes);
        let font_descriptor = FontResourceDescriptorWire {
            face_index: 0,
            variation_axes: BTreeMap::new(),
        };
        let font = ResolvedResource {
            resource_id: "font:fixture".to_owned(),
            kind: ResourceKind::Font,
            digest: font_digest.clone(),
            handle: VerifiedHandleId::new(1).unwrap(),
            facts: VerifiedResourceFacts::Font {
                descriptor: font_descriptor.clone(),
                bytes: Arc::clone(&font_bytes),
            },
            execution: ResolvedExecutionPayload::None,
            entry: ResourceEntryWire::Font {
                digest: font_digest,
                descriptor: font_descriptor,
            },
            dependencies: Vec::new(),
        };
        let font = compiled_execution_resource_ref(&font).unwrap();
        assert_eq!(font.kind(), CompiledExecutionResourceKind::FontBytes);
        assert_eq!(font.bytes(), &*font_bytes);
        assert_eq!(font.abi_digest(), None);
    }

    #[test]
    fn font_payload_axes_must_match_the_concrete_face() {
        let bytes: Arc<[u8]> = Arc::from(
            &include_bytes!("../../valle-motion/assets/fonts/katex/KaTeX_Main-Regular.ttf")[..],
        );
        let digest = content_digest(&bytes);
        let descriptor = FontResourceDescriptorWire {
            face_index: 0,
            variation_axes: BTreeMap::from([(
                "wght".to_owned(),
                FontVariationAxisWire {
                    minimum: 100.0,
                    default: 400.0,
                    maximum: 900.0,
                },
            )]),
        };
        let entry = ResourceEntryWire::Font {
            digest: digest.clone(),
            descriptor: descriptor.clone(),
        };
        let facts = VerifiedResourceFacts::Font { descriptor, bytes };

        let (code, reason) = verify_execution_payload(&entry, &facts, &digest).unwrap_err();
        assert_eq!(code, EngineOpenDiagnosticCode::FontPayloadMismatch);
        assert_eq!(reason, "variation-axes");
    }
}

#[cfg(test)]
fn content_digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest::of_bytes(bytes)
}
