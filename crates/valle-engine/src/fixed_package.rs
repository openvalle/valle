//! Platform-neutral fixed-package verification and closed admission for one
//! immutable Timeline render input.

pub use valle_timeline::internal::fixed_package::{
    CANONICAL_TIMELINE_MEMBER_PATH, EXECUTION_PROFILE_MEMBER_PATH, FIXED_PACKAGE_FORMAT,
    FixedPackageManifest, RESOURCE_MANIFEST_MEMBER_PATH, VERIFIED_BINDING_BUNDLE_MEMBER_PATH,
};
use valle_timeline::internal::fixed_package::{
    FixedPackageMember, FixedPackageMemberRole, is_normalized_package_relative_path,
    validate_fixed_package_manifest,
};

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashSet},
    fmt,
    sync::Arc,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use serde::{
    Deserialize, Serialize,
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Number, Value};
use valle_timeline::internal::{
    ContentDigest, RenderId, ResourceManifest,
    wire::resource::{
        AudioResourceDescriptorWire, FontResourceDescriptorWire, ImageResourceDescriptorWire,
        LottieArtifactAbiWire, LottieResourceDescriptorWire, MotionArtifactAbiWire,
        MotionArtifactDescriptorWire, ResourceEntryWire, ShaderArtifactAbiWire,
        ShaderResourceDescriptorWire, VideoResourceDescriptorWire,
    },
};

use crate::{
    product::EngineRender,
    render::{
        AudioFootprint, COMMON_AUDIO_ABI, Capabilities, CompiledRender, ExecutionLimits,
        ExecutionProfile, ExecutionProfileProjection, ExtensionKernelCapability, ResourceBinding,
        ResourceBindings, ResourceDependency, VerifiedHandleId, VerifiedResourceFacts,
        VisualFootprint,
    },
};

/// Failure while opening a package whose outer files are either still being
/// verified or whose decoded render input fails Engine admission.
#[derive(Debug)]
pub enum FixedPackageOpenError {
    InvalidPackage(String),
    Engine(crate::render::EngineOpenReport),
}

impl FixedPackageOpenError {
    pub fn engine_report(&self) -> Option<&crate::render::EngineOpenReport> {
        match self {
            Self::Engine(report) => Some(report),
            Self::InvalidPackage(_) => None,
        }
    }

    pub fn into_engine_report(self) -> Option<crate::render::EngineOpenReport> {
        match self {
            Self::Engine(report) => Some(report),
            Self::InvalidPackage(_) => None,
        }
    }
}

impl From<String> for FixedPackageOpenError {
    fn from(message: String) -> Self {
        Self::InvalidPackage(message)
    }
}

impl fmt::Display for FixedPackageOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPackage(message) => formatter.write_str(message),
            Self::Engine(report) => {
                let diagnostics =
                    serde_json::to_string(report.diagnostics()).unwrap_or_else(|_| "[]".to_owned());
                write!(formatter, "[engine_open] {diagnostics}")
            }
        }
    }
}

impl std::error::Error for FixedPackageOpenError {}

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub const COMMON_PROFILE_KEY: &str = "common";
/// One file presented to the fixed-package verifier.
///
/// Paths are package-relative names. The verifier owns all path normalization,
/// membership, length and digest checks; callers must not pre-trust these
/// values just because they came from a package directory.
#[derive(Debug, Clone, Copy)]
pub struct FixedPackageFile<'a> {
    path: &'a str,
    bytes: &'a [u8],
}

impl<'a> FixedPackageFile<'a> {
    pub const fn new(path: &'a str, bytes: &'a [u8]) -> Self {
        Self { path, bytes }
    }

    pub const fn path(&self) -> &'a str {
        self.path
    }

    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

/// A package whose outer manifest and every declared member have been
/// verified. Inner Timeline/resource/binding admission remains a separate,
/// explicit step through [`VerifiedFixedPackage::open`].
#[derive(Debug)]
pub struct VerifiedFixedPackage {
    manifest: FixedPackageManifest,
    canonical_manifest_bytes: Vec<u8>,
    package_digest: ContentDigest,
    members: BTreeMap<FixedPackageMemberRole, Vec<u8>>,
    execution_profile: ExecutionProfile,
}

impl VerifiedFixedPackage {
    pub const fn manifest(&self) -> &FixedPackageManifest {
        &self.manifest
    }

    pub fn canonical_manifest_bytes(&self) -> &[u8] {
        &self.canonical_manifest_bytes
    }

    pub const fn package_digest(&self) -> &ContentDigest {
        &self.package_digest
    }

    /// Perform the existing closed inner-wire verification and Engine
    /// admission, but only after the outer package has been verified.
    pub fn open(&self) -> Result<OpenedFixedPackage, FixedPackageOpenError> {
        let timeline_json = self.member_text(FixedPackageMemberRole::CanonicalTimeline)?;
        let manifest_json = self.member_text(FixedPackageMemberRole::ResourceManifest)?;
        let bundle_json = self.member_text(FixedPackageMemberRole::VerifiedBindingBundle)?;
        open_fixed_package_members_with_profile(
            timeline_json,
            manifest_json,
            bundle_json,
            &self.execution_profile,
        )
    }

    fn member_text(&self, role: FixedPackageMemberRole) -> Result<&str, String> {
        std::str::from_utf8(
            self.members
                .get(&role)
                .expect("verified package contains every closed member role"),
        )
        .map_err(|_| {
            format!(
                "[fixed_package] member {:?} is not valid UTF-8",
                role.path()
            )
        })
    }
}

#[derive(Debug)]
pub struct OpenedFixedPackage {
    render: EngineRender,
    receipt: RenderReceipt,
    receipt_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenderReceipt {
    render_id: RenderId,
    canvas_width: u32,
    canvas_height: u32,
    frame_rate: String,
    frame_count: i64,
    sample_rate: u32,
    sample_count: i64,
}

impl OpenedFixedPackage {
    /// The immutable render admitted from this exact fixed package.
    pub fn compiled(&self) -> &CompiledRender {
        self.render.compiled()
    }

    /// Clone the read-only shared render handle for a platform renderer.
    pub fn compiled_arc(&self) -> Arc<CompiledRender> {
        self.render.compiled_arc()
    }

    /// Clone the fully opened Product render without rebuilding it from a compiled payload.
    pub fn engine_render(&self) -> EngineRender {
        self.render.clone()
    }

    pub const fn receipt(&self) -> &RenderReceipt {
        &self.receipt
    }

    pub fn receipt_json(&self) -> &str {
        &self.receipt_json
    }

    pub fn into_parts(self) -> (EngineRender, RenderReceipt, String) {
        (self.render, self.receipt, self.receipt_json)
    }
}

impl RenderReceipt {
    pub const fn render_id(&self) -> RenderId {
        self.render_id
    }

    pub const fn canvas_width(&self) -> u32 {
        self.canvas_width
    }

    pub const fn canvas_height(&self) -> u32 {
        self.canvas_height
    }

    pub fn frame_rate(&self) -> &str {
        &self.frame_rate
    }

    pub const fn frame_count(&self) -> i64 {
        self.frame_count
    }

    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub const fn sample_count(&self) -> i64 {
        self.sample_count
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VerifiedBindingBundleWire {
    bindings: BTreeMap<String, ResourceBindingWire>,
    capabilities: CapabilitiesWire,
}

/// The bundle envelope is decoded independently from its binding values so an
/// unrelated locator/payload cannot become an eager prerequisite for opening a
/// render. Only bindings reached from the document roots (and their declared
/// dependencies) are subsequently decoded into [`ResourceBindingWire`].
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnresolvedBindingBundleWire {
    bindings: BTreeMap<String, Value>,
    capabilities: CapabilitiesWire,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResourceBindingWire {
    digest: ContentDigest,
    handle: u64,
    facts: VerifiedResourceFactsWire,
    dependencies: Vec<ResourceDependencyWire>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResourceDependencyWire {
    role: String,
    resource_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CapabilitiesWire {
    camera: bool,
    artifact_abis: Vec<String>,
    extension_kernels: BTreeMap<String, ExtensionKernelWire>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExtensionKernelWire {
    abi: String,
    implementation_digest: ContentDigest,
    visual_footprint: VisualFootprintWire,
    audio_footprint: AudioFootprintWire,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VisualFootprintWire {
    past_frames: u32,
    future_frames: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AudioFootprintWire {
    past_samples: u64,
    future_samples: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum VerifiedResourceFactsWire {
    Video {
        descriptor: VideoResourceDescriptorWire,
        temporal_footprint: VisualFootprintWire,
    },
    Audio {
        descriptor: AudioResourceDescriptorWire,
        temporal_footprint: AudioFootprintWire,
        decoded_pcm_digest: ContentDigest,
    },
    Image {
        descriptor: ImageResourceDescriptorWire,
        temporal_footprint: VisualFootprintWire,
    },
    Lottie {
        abi: LottieArtifactAbiWire,
        descriptor: LottieResourceDescriptorWire,
        temporal_footprint: VisualFootprintWire,
    },
    Font {
        descriptor: FontResourceDescriptorWire,
        bytes_base64: String,
    },
    MotionArtifact {
        abi: MotionArtifactAbiWire,
        descriptor: MotionArtifactDescriptorWire,
        artifact: valle_motion::SceneArtifact,
        temporal_footprint: VisualFootprintWire,
    },
    Shader {
        abi: ShaderArtifactAbiWire,
        descriptor: ShaderResourceDescriptorWire,
        manifest_bytes_base64: String,
        source_bytes_base64: String,
    },
}

/// Return the four closed files used by the fixed-package inner wire.
/// The outer manifest binds these exact bytes and paths.
pub fn fixed_package_files<'a>(
    timeline_json: &'a str,
    manifest_json: &'a str,
    verified_binding_bundle_json: &'a str,
    execution_profile_json: &'a str,
) -> [FixedPackageFile<'a>; 4] {
    [
        FixedPackageFile::new(CANONICAL_TIMELINE_MEMBER_PATH, timeline_json.as_bytes()),
        FixedPackageFile::new(RESOURCE_MANIFEST_MEMBER_PATH, manifest_json.as_bytes()),
        FixedPackageFile::new(
            VERIFIED_BINDING_BUNDLE_MEMBER_PATH,
            verified_binding_bundle_json.as_bytes(),
        ),
        FixedPackageFile::new(
            EXECUTION_PROFILE_MEMBER_PATH,
            execution_profile_json.as_bytes(),
        ),
    ]
}

/// Build the canonical outer manifest for a complete closed member set.
///
/// `resourceDigests` is derived from the ResourceManifest member and contains
/// only actual externally fulfilled content blobs. Descriptor fingerprints,
/// decoded-media facts, and Engine-owned implementation identities remain
/// bound by the four member digests but are not falsely advertised as CAS
/// objects to the package GC.
pub fn canonical_fixed_package_manifest(files: &[FixedPackageFile<'_>]) -> Result<String, String> {
    let files = collect_fixed_package_files(files)?;
    decode_fixed_execution_profile(
        files
            .get(&FixedPackageMemberRole::ExecutionProfile)
            .expect("closed member collection contains execution profile"),
    )?;
    let resource_digests = package_resource_digests(&files)?;
    let members = FixedPackageMemberRole::ALL
        .into_iter()
        .map(|role| {
            let bytes = files
                .get(&role)
                .expect("closed member collection contains every role");
            let byte_len = u64::try_from(bytes.len())
                .map_err(|_| "[fixed_package] member is too large".to_owned())?;
            if byte_len > MAX_SAFE_INTEGER {
                return Err(
                    "[fixed_package] member exceeds the interoperable JSON byte range".to_owned(),
                );
            }
            Ok(FixedPackageMember {
                role,
                path: role.path().to_owned(),
                bytes: byte_len,
                digest: content_digest(bytes),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let manifest = FixedPackageManifest {
        format: FIXED_PACKAGE_FORMAT.to_owned(),
        members,
        resource_digests,
    };
    let canonical = canonical_fixed_package_manifest_bytes(&manifest)?;
    String::from_utf8(canonical)
        .map_err(|error| format!("[fixed_package_manifest] JCS was not UTF-8: {error}"))
}

/// Verify a canonical outer manifest against the exact package files.
///
/// This step is intentionally separate from Engine admission. A value of this
/// type proves that roles, paths, byte lengths, member digests and the complete
/// resource digest set are mutually bound by `packageDigest`.
pub fn verify_fixed_package(
    manifest_json: &str,
    files: &[FixedPackageFile<'_>],
) -> Result<VerifiedFixedPackage, String> {
    let manifest_value = parse_strict_json(manifest_json, "fixed_package_manifest")?;
    let manifest: FixedPackageManifest = serde_json::from_value(manifest_value)
        .map_err(|error| format!("[fixed_package_manifest] invalid closed manifest: {error}"))?;
    let canonical_manifest_bytes = canonical_fixed_package_manifest_bytes(&manifest)?;
    if canonical_manifest_bytes != manifest_json.as_bytes() {
        return Err("[fixed_package_manifest] manifest must use canonical JCS bytes".to_owned());
    }
    validate_fixed_package_manifest(&manifest)?;

    let files = collect_fixed_package_files(files)?;
    let execution_profile = decode_fixed_execution_profile(
        files
            .get(&FixedPackageMemberRole::ExecutionProfile)
            .expect("closed member collection contains execution profile"),
    )?;
    for member in &manifest.members {
        let bytes = files
            .get(&member.role)
            .expect("manifest and actual files contain every closed member role");
        let actual_len = u64::try_from(bytes.len())
            .map_err(|_| "[fixed_package] member is too large".to_owned())?;
        if member.bytes != actual_len {
            return Err(format!(
                "[fixed_package] byte length mismatch for {:?}: expected {}, found {actual_len}",
                member.path, member.bytes
            ));
        }
        let actual_digest = content_digest(bytes);
        if member.digest != actual_digest {
            return Err(format!(
                "[fixed_package] digest mismatch for {:?}",
                member.path
            ));
        }
    }

    let expected_resource_digests = package_resource_digests(&files)?;
    if manifest.resource_digests != expected_resource_digests {
        return Err(
            "[fixed_package_manifest] resourceDigests is not the complete package blob set"
                .to_owned(),
        );
    }

    let package_digest = content_digest(&canonical_manifest_bytes);
    let members = files
        .into_iter()
        .map(|(role, bytes)| (role, bytes.to_vec()))
        .collect();
    Ok(VerifiedFixedPackage {
        manifest,
        canonical_manifest_bytes,
        package_digest,
        members,
        execution_profile,
    })
}

/// Verify the outer fixed package and then admit its closed inner wires.
pub fn open_verified_fixed_package(
    manifest_json: &str,
    files: &[FixedPackageFile<'_>],
) -> Result<OpenedFixedPackage, FixedPackageOpenError> {
    verify_fixed_package(manifest_json, files)
        .map_err(FixedPackageOpenError::InvalidPackage)?
        .open()
}

fn collect_fixed_package_files<'a>(
    files: &[FixedPackageFile<'a>],
) -> Result<BTreeMap<FixedPackageMemberRole, &'a [u8]>, String> {
    let mut by_role = BTreeMap::new();
    let mut paths = BTreeSet::new();
    for file in files {
        if !is_normalized_package_relative_path(file.path) {
            return Err(format!(
                "[fixed_package] member path {:?} is not a normalized package-relative path",
                file.path
            ));
        }
        if !paths.insert(file.path) {
            return Err(format!(
                "[fixed_package] duplicate member path {:?}",
                file.path
            ));
        }
        let role = FixedPackageMemberRole::from_path(file.path).ok_or_else(|| {
            format!(
                "[fixed_package] unexpected member path {:?}; the member set is closed",
                file.path
            )
        })?;
        if by_role.insert(role, file.bytes).is_some() {
            return Err(format!(
                "[fixed_package] duplicate member role {:?}",
                role.as_str()
            ));
        }
    }
    for role in FixedPackageMemberRole::ALL {
        if !by_role.contains_key(&role) {
            return Err(format!(
                "[fixed_package] missing member role {:?}",
                role.as_str()
            ));
        }
    }
    Ok(by_role)
}

fn canonical_fixed_package_manifest_bytes(
    manifest: &FixedPackageManifest,
) -> Result<Vec<u8>, String> {
    serde_jcs::to_vec(manifest)
        .map_err(|error| format!("[fixed_package_manifest] cannot encode JCS: {error}"))
}

fn package_resource_digests(
    files: &BTreeMap<FixedPackageMemberRole, &[u8]>,
) -> Result<Vec<ContentDigest>, String> {
    let manifest_bytes = files
        .get(&FixedPackageMemberRole::ResourceManifest)
        .expect("closed member collection contains resource manifest");
    let manifest = valle_timeline::internal::decode_resource_manifest(manifest_bytes)
        .map_err(|error| format!("[fixed_package_manifest.resourceDigests] {error}"))?;
    let mut digests = BTreeSet::new();
    for entry in manifest.entries().values() {
        digests.insert(resource_entry_digest(entry).clone());
    }
    Ok(digests.into_iter().collect())
}

fn content_digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest::of_bytes(bytes)
}

fn open_fixed_package_members_with_profile(
    timeline_json: &str,
    manifest_json: &str,
    verified_binding_bundle_json: &str,
    profile: &ExecutionProfile,
) -> Result<OpenedFixedPackage, FixedPackageOpenError> {
    let timeline = valle_timeline::internal::decode_canonical(timeline_json)
        .map_err(|error| format!("[timeline_document] {error}"))?;
    let manifest = valle_timeline::internal::decode_resource_manifest(manifest_json.as_bytes())
        .map_err(|error| format!("[resource_manifest] {error}"))?;
    // Keep strict JSON shape/duplicate-key checks for the whole envelope, but
    // defer interoperable-number and typed payload validation for individual
    // bindings until the Timeline dependency closure is known. An unused
    // platform locator is intentionally outside the open contract.
    let bundle_value =
        parse_strict_json_syntax(verified_binding_bundle_json, "verified_binding_bundle")?;
    if let Some(capabilities) = bundle_value.get("capabilities") {
        validate_safe_numbers(capabilities)
            .map_err(|error| format!("[verified_binding_bundle] {error}"))?;
    }
    let bundle: UnresolvedBindingBundleWire = serde_json::from_value(bundle_value)
        .map_err(|error| format!("[verified_binding_bundle] invalid closed bundle: {error}"))?;
    let bundle = bundle.decode_closure(&timeline)?;
    bundle.validate_manifest(&manifest)?;
    let (bindings, capabilities) = bundle.into_domain()?;
    let opened = EngineRender::open(&timeline, &manifest, &bindings, &capabilities, profile)
        .map_err(FixedPackageOpenError::Engine)?;
    let compiled = opened.compiled();
    let canvas = compiled.canvas();
    let frame_rate = canvas.frame_rate();
    if canvas.frame_count() < 0
        || canvas.frame_count() as u64 > MAX_SAFE_INTEGER
        || canvas.sample_count() < 0
        || canvas.sample_count() as u64 > MAX_SAFE_INTEGER
    {
        return Err(FixedPackageOpenError::InvalidPackage(
            "[render_receipt] frame/sample count exceeds the interoperable JSON range".to_owned(),
        ));
    }
    let receipt = RenderReceipt {
        render_id: compiled.render_id(),
        canvas_width: canvas.width(),
        canvas_height: canvas.height(),
        frame_rate: format!("{}/{}", frame_rate.numerator(), frame_rate.denominator()),
        frame_count: canvas.frame_count(),
        sample_rate: canvas.sample_rate(),
        sample_count: canvas.sample_count(),
    };
    let receipt_json =
        serde_json::to_string(&receipt).map_err(|error| format!("[render_receipt] {error}"))?;
    Ok(OpenedFixedPackage {
        render: opened,
        receipt,
        receipt_json,
    })
}

/// Serialize the exact closed binding bundle consumed by
/// [`VerifiedFixedPackage::open`]. Executable payload bytes are taken from
/// the verified domain facts, so producers cannot drift to a second mirror of
/// this wire contract.
pub fn canonical_verified_binding_bundle(
    bindings: &ResourceBindings,
    capabilities: &Capabilities,
) -> Result<String, String> {
    let mut wire_bindings = BTreeMap::new();
    for (resource_id, binding) in bindings.iter() {
        let dependencies = binding
            .dependencies()
            .iter()
            .map(|dependency| ResourceDependencyWire {
                role: dependency.role().to_owned(),
                resource_id: dependency.resource_id().to_owned(),
            })
            .collect();
        wire_bindings.insert(
            resource_id.to_owned(),
            ResourceBindingWire {
                digest: binding.digest().clone(),
                handle: binding.handle().get(),
                facts: VerifiedResourceFactsWire::from_domain(binding.facts())?,
                dependencies,
            },
        );
    }

    let mut extension_kernels = BTreeMap::new();
    for (kind, capability) in capabilities.extension_kernels() {
        extension_kernels.insert(
            kind.to_owned(),
            ExtensionKernelWire {
                abi: capability.abi().to_owned(),
                implementation_digest: capability.implementation_digest().clone(),
                visual_footprint: VisualFootprintWire::from_domain(capability.visual_footprint()),
                audio_footprint: AudioFootprintWire::from_domain(capability.audio_footprint()),
            },
        );
    }
    let bundle = VerifiedBindingBundleWire {
        bindings: wire_bindings,
        capabilities: CapabilitiesWire {
            camera: capabilities.camera_enabled(),
            artifact_abis: capabilities.artifact_abis().map(str::to_owned).collect(),
            extension_kernels,
        },
    };

    // Probe the typed graph first because serde_json represents non-finite
    // floats as null. JCS rejects them before that information can be lost.
    serde_jcs::to_vec(&bundle)
        .map_err(|error| format!("[verified_binding_bundle] cannot encode JCS: {error}"))?;
    let ordinary = serde_json::to_string(&bundle)
        .map_err(|error| format!("[verified_binding_bundle] cannot encode JSON: {error}"))?;
    let strict = parse_strict_json(&ordinary, "verified_binding_bundle")?;
    let canonical = serde_jcs::to_vec(&strict)
        .map_err(|error| format!("[verified_binding_bundle] cannot encode JCS: {error}"))?;
    String::from_utf8(canonical)
        .map_err(|error| format!("[verified_binding_bundle] JCS was not UTF-8: {error}"))
}

impl VerifiedResourceFactsWire {
    fn from_domain(facts: &VerifiedResourceFacts) -> Result<Self, String> {
        Ok(match facts {
            VerifiedResourceFacts::Video {
                descriptor,
                temporal_footprint,
            } => Self::Video {
                descriptor: descriptor.clone(),
                temporal_footprint: VisualFootprintWire::from_domain(*temporal_footprint),
            },
            VerifiedResourceFacts::Audio {
                descriptor,
                temporal_footprint,
                decoded_pcm_digest,
            } => Self::Audio {
                descriptor: descriptor.clone(),
                temporal_footprint: AudioFootprintWire::from_domain(*temporal_footprint),
                decoded_pcm_digest: decoded_pcm_digest.clone(),
            },
            VerifiedResourceFacts::Image {
                descriptor,
                temporal_footprint,
            } => Self::Image {
                descriptor: descriptor.clone(),
                temporal_footprint: VisualFootprintWire::from_domain(*temporal_footprint),
            },
            VerifiedResourceFacts::Lottie {
                abi,
                descriptor,
                temporal_footprint,
            } => Self::Lottie {
                abi: *abi,
                descriptor: descriptor.clone(),
                temporal_footprint: VisualFootprintWire::from_domain(*temporal_footprint),
            },
            VerifiedResourceFacts::Font { descriptor, bytes } => Self::Font {
                descriptor: descriptor.clone(),
                bytes_base64: BASE64_STANDARD.encode(bytes),
            },
            VerifiedResourceFacts::MotionArtifact {
                abi,
                descriptor,
                artifact,
                temporal_footprint,
            } => Self::MotionArtifact {
                abi: *abi,
                descriptor: descriptor.clone(),
                artifact: artifact.as_ref().clone(),
                temporal_footprint: VisualFootprintWire::from_domain(*temporal_footprint),
            },
            VerifiedResourceFacts::Shader {
                abi,
                descriptor,
                manifest_bytes,
                source_bytes,
            } => Self::Shader {
                abi: *abi,
                descriptor: descriptor.clone(),
                manifest_bytes_base64: BASE64_STANDARD.encode(manifest_bytes),
                source_bytes_base64: BASE64_STANDARD.encode(source_bytes),
            },
        })
    }
}

impl VisualFootprintWire {
    const fn from_domain(footprint: VisualFootprint) -> Self {
        Self {
            past_frames: footprint.past_frames(),
            future_frames: footprint.future_frames(),
        }
    }
}

impl AudioFootprintWire {
    const fn from_domain(footprint: AudioFootprint) -> Self {
        Self {
            past_samples: footprint.past_samples(),
            future_samples: footprint.future_samples(),
        }
    }
}

/// Canonical bytes for the complete execution policy embedded in a fixed
/// package. The short key is only a producer-side selection convenience; it
/// never crosses the package boundary by itself.
pub fn canonical_fixed_execution_profile(profile_key: &str) -> Result<String, String> {
    let profile = fixed_profile(profile_key)?;
    String::from_utf8(
        profile
            .canonical_projection_bytes()
            .map_err(|error| format!("[execution_profile] cannot encode JCS: {error}"))?,
    )
    .map_err(|error| format!("[execution_profile] JCS was not UTF-8: {error}"))
}

fn decode_fixed_execution_profile(bytes: &[u8]) -> Result<ExecutionProfile, String> {
    let json = std::str::from_utf8(bytes)
        .map_err(|_| "[execution_profile] member is not valid UTF-8".to_owned())?;
    let value = parse_strict_json(json, "execution_profile")?;
    let projection: ExecutionProfileProjection = serde_json::from_value(value)
        .map_err(|error| format!("[execution_profile] invalid closed profile: {error}"))?;
    let profile = ExecutionProfile::from_projection(projection)
        .map_err(|error| format!("[execution_profile] {error}"))?;
    let canonical = profile
        .canonical_projection_bytes()
        .map_err(|error| format!("[execution_profile] cannot encode JCS: {error}"))?;
    if canonical != bytes {
        return Err("[execution_profile] profile must use canonical JCS bytes".to_owned());
    }
    if profile != fixed_profile(COMMON_PROFILE_KEY)? {
        return Err("[execution_profile] unsupported fixed execution policy".to_owned());
    }
    Ok(profile)
}

fn fixed_profile(profile_key: &str) -> Result<ExecutionProfile, String> {
    if profile_key != COMMON_PROFILE_KEY {
        return Err(format!(
            "[execution_profile] unsupported profile key {profile_key:?}"
        ));
    }
    ExecutionProfile::new(
        COMMON_PROFILE_KEY,
        "valle.numeric/common@1",
        COMMON_AUDIO_ABI,
        "valle.motion/eval@1",
        crate::render::CAPTION_GLYPH_RUN_ABI,
        ExecutionLimits::new(64, 8, 32).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("[execution_profile] {error}"))
}

impl VerifiedBindingBundleWire {
    fn validate_manifest(&self, manifest: &ResourceManifest) -> Result<(), String> {
        for (resource_id, binding) in &self.bindings {
            let entry = manifest.entries().get(resource_id).ok_or_else(|| {
                format!(
                    "[verified_binding_bundle] binding {resource_id:?} is absent from the pinned manifest"
                )
            })?;
            if resource_entry_digest(entry) != &binding.digest {
                return Err(format!(
                    "[verified_binding_bundle] digest mismatch for {resource_id:?}"
                ));
            }
            if !binding.facts.matches_manifest_entry(entry) {
                return Err(format!(
                    "[verified_binding_bundle] facts mismatch for {resource_id:?}"
                ));
            }
        }
        Ok(())
    }

    fn into_domain(self) -> Result<(ResourceBindings, Capabilities), String> {
        for (resource_id, binding) in &self.bindings {
            if resource_id.trim().is_empty() {
                return Err("[verified_binding_bundle] resource id must not be empty".to_owned());
            }
            let mut roles = BTreeSet::new();
            for dependency in &binding.dependencies {
                if !roles.insert(dependency.role.as_str()) {
                    return Err(format!(
                        "[verified_binding_bundle] duplicate dependency role {:?} for {resource_id:?}",
                        dependency.role
                    ));
                }
                if !self.bindings.contains_key(&dependency.resource_id) {
                    return Err(format!(
                        "[verified_binding_bundle] dependency {:?} for {resource_id:?} has no binding",
                        dependency.resource_id
                    ));
                }
            }
        }

        let mut bindings = ResourceBindings::new();
        for (resource_id, binding) in self.bindings {
            if binding.handle == 0 || binding.handle > MAX_SAFE_INTEGER {
                return Err(format!(
                    "[verified_binding_bundle] invalid interoperable handle for {resource_id:?}"
                ));
            }
            let mut domain = ResourceBinding::new(
                binding.digest,
                VerifiedHandleId::new(binding.handle).map_err(|error| error.to_string())?,
                binding.facts.into_domain()?,
            );
            for dependency in binding.dependencies {
                domain = domain.with_dependency(
                    ResourceDependency::new(dependency.role, dependency.resource_id)
                        .map_err(|error| error.to_string())?,
                );
            }
            bindings
                .insert(resource_id, domain)
                .map_err(|error| format!("[verified_binding_bundle] {error}"))?;
        }

        let mut capabilities = Capabilities::new();
        if self.capabilities.camera {
            capabilities = capabilities.with_camera();
        }
        let mut artifact_abis = BTreeSet::new();
        for abi in self.capabilities.artifact_abis {
            if abi.trim().is_empty() || !artifact_abis.insert(abi.clone()) {
                return Err(
                    "[verified_binding_bundle] artifact ABI must be non-empty and unique"
                        .to_owned(),
                );
            }
            capabilities = capabilities.with_artifact_abi(abi);
        }
        for (kind, kernel) in self.capabilities.extension_kernels {
            let capability =
                ExtensionKernelCapability::new(kernel.abi, kernel.implementation_digest)
                    .map_err(|error| format!("[verified_binding_bundle] {error}"))?
                    .with_visual_footprint(kernel.visual_footprint.into_domain())
                    .with_audio_footprint(kernel.audio_footprint.into_domain()?);
            capabilities = capabilities
                .with_extension_kernel(kind, capability)
                .map_err(|error| format!("[verified_binding_bundle] {error}"))?;
        }
        Ok((bindings, capabilities))
    }
}

impl UnresolvedBindingBundleWire {
    fn decode_closure(
        self,
        timeline: &valle_timeline::internal::CanonicalTimeline,
    ) -> Result<VerifiedBindingBundleWire, String> {
        let Self {
            bindings: unresolved,
            capabilities,
        } = self;
        let mut pending: Vec<_> = crate::render::document_resource_root_ids(timeline)
            .into_iter()
            .collect();
        let mut visited = BTreeSet::new();
        let mut bindings = BTreeMap::new();

        while let Some(resource_id) = pending.pop() {
            if !visited.insert(resource_id.clone()) {
                continue;
            }
            let Some(raw_binding) = unresolved.get(&resource_id) else {
                // Preserve the Engine's normal missing-binding diagnostic. The
                // closure decoder only proves supplied binding values.
                continue;
            };
            validate_safe_numbers(raw_binding).map_err(|error| {
                format!("[verified_binding_bundle] {error} for {resource_id:?}")
            })?;
            let binding: ResourceBindingWire = serde_json::from_value(raw_binding.clone())
                .map_err(|error| {
                    format!(
                        "[verified_binding_bundle] invalid closed binding {resource_id:?}: {error}"
                    )
                })?;
            pending.extend(
                binding
                    .dependencies
                    .iter()
                    .map(|dependency| dependency.resource_id.clone()),
            );
            bindings.insert(resource_id, binding);
        }

        Ok(VerifiedBindingBundleWire {
            bindings,
            capabilities,
        })
    }
}

impl VerifiedResourceFactsWire {
    fn matches_manifest_entry(&self, entry: &ResourceEntryWire) -> bool {
        match (self, entry) {
            (
                Self::Video { descriptor, .. },
                ResourceEntryWire::Video {
                    descriptor: expected,
                    ..
                },
            ) => descriptor == expected,
            (
                Self::Audio { descriptor, .. },
                ResourceEntryWire::Audio {
                    descriptor: expected,
                    ..
                },
            ) => descriptor == expected,
            (
                Self::Image { descriptor, .. },
                ResourceEntryWire::Image {
                    descriptor: expected,
                    ..
                },
            ) => descriptor == expected,
            (
                Self::Lottie {
                    abi, descriptor, ..
                },
                ResourceEntryWire::Lottie {
                    abi: expected_abi,
                    descriptor: expected,
                    ..
                },
            ) => abi == expected_abi && descriptor == expected,
            (
                Self::Font { descriptor, .. },
                ResourceEntryWire::Font {
                    descriptor: expected,
                    ..
                },
            ) => descriptor == expected,
            (
                Self::MotionArtifact {
                    abi, descriptor, ..
                },
                ResourceEntryWire::MotionArtifact {
                    abi: expected_abi,
                    descriptor: expected,
                    ..
                },
            ) => abi == expected_abi && descriptor == expected,
            (
                Self::Shader {
                    abi, descriptor, ..
                },
                ResourceEntryWire::Shader {
                    abi: expected_abi,
                    descriptor: expected,
                    ..
                },
            ) => abi == expected_abi && descriptor == expected,
            _ => false,
        }
    }

    fn into_domain(self) -> Result<VerifiedResourceFacts, String> {
        Ok(match self {
            Self::Video {
                descriptor,
                temporal_footprint,
            } => VerifiedResourceFacts::Video {
                descriptor,
                temporal_footprint: temporal_footprint.into_domain(),
            },
            Self::Audio {
                descriptor,
                temporal_footprint,
                decoded_pcm_digest,
            } => VerifiedResourceFacts::Audio {
                descriptor,
                temporal_footprint: temporal_footprint.into_domain()?,
                decoded_pcm_digest,
            },
            Self::Image {
                descriptor,
                temporal_footprint,
            } => VerifiedResourceFacts::Image {
                descriptor,
                temporal_footprint: temporal_footprint.into_domain(),
            },
            Self::Lottie {
                abi,
                descriptor,
                temporal_footprint,
            } => VerifiedResourceFacts::Lottie {
                abi,
                descriptor,
                temporal_footprint: temporal_footprint.into_domain(),
            },
            Self::Font {
                descriptor,
                bytes_base64,
            } => VerifiedResourceFacts::Font {
                descriptor,
                bytes: decode_base64_payload(bytes_base64, "font.bytesBase64")?,
            },
            Self::MotionArtifact {
                abi,
                descriptor,
                artifact,
                temporal_footprint,
            } => {
                artifact.validate().map_err(|errors| {
                    format!("[verified_binding_bundle] invalid Motion artifact: {errors:?}")
                })?;
                VerifiedResourceFacts::MotionArtifact {
                    abi,
                    descriptor,
                    artifact: Arc::new(artifact),
                    temporal_footprint: temporal_footprint.into_domain(),
                }
            }
            Self::Shader {
                abi,
                descriptor,
                manifest_bytes_base64,
                source_bytes_base64,
            } => VerifiedResourceFacts::Shader {
                abi,
                descriptor,
                manifest_bytes: decode_base64_payload(
                    manifest_bytes_base64,
                    "shader.manifestBytesBase64",
                )?,
                source_bytes: decode_base64_payload(
                    source_bytes_base64,
                    "shader.sourceBytesBase64",
                )?,
            },
        })
    }
}

fn decode_base64_payload(encoded: String, field: &str) -> Result<Arc<[u8]>, String> {
    let bytes = BASE64_STANDARD
        .decode(encoded.as_bytes())
        .map_err(|error| format!("[verified_binding_bundle] invalid {field}: {error}"))?;
    if BASE64_STANDARD.encode(&bytes) != encoded {
        return Err(format!(
            "[verified_binding_bundle] {field} must use canonical padded base64"
        ));
    }
    Ok(Arc::from(bytes))
}

fn resource_entry_digest(entry: &ResourceEntryWire) -> &ContentDigest {
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

impl VisualFootprintWire {
    const fn into_domain(self) -> VisualFootprint {
        VisualFootprint::new(self.past_frames, self.future_frames)
    }
}

impl AudioFootprintWire {
    fn into_domain(self) -> Result<AudioFootprint, String> {
        AudioFootprint::new(self.past_samples, self.future_samples)
            .map_err(|error| format!("[verified_binding_bundle] {error}"))
    }
}

fn parse_strict_json(input: &str, label: &str) -> Result<Value, String> {
    let value = parse_strict_json_syntax(input, label)?;
    validate_safe_numbers(&value).map_err(|error| format!("[{label}] {error}"))?;
    Ok(value)
}

fn parse_strict_json_syntax(input: &str, label: &str) -> Result<Value, String> {
    if input.as_bytes().starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(format!("[{label}] UTF-8 BOM is not allowed"));
    }
    let failure = RefCell::new(None);
    let seed = StrictValueSeed { failure: &failure };
    let mut deserializer = serde_json::Deserializer::from_str(input);
    let value = seed.deserialize(&mut deserializer).map_err(|_| {
        failure
            .borrow()
            .clone()
            .unwrap_or_else(|| "invalid JSON".to_owned())
    })?;
    deserializer
        .end()
        .map_err(|_| format!("[{label}] trailing or malformed JSON"))?;
    Ok(value)
}

fn validate_safe_numbers(value: &Value) -> Result<(), String> {
    match value {
        Value::Number(number) => {
            if number
                .as_u64()
                .is_some_and(|value| value > MAX_SAFE_INTEGER)
                || number
                    .as_i64()
                    .is_some_and(|value| value.unsigned_abs() > MAX_SAFE_INTEGER)
                || number.as_f64().is_some_and(|value| {
                    !value.is_finite()
                        || (value.fract() == 0.0 && value.abs() > MAX_SAFE_INTEGER as f64)
                })
            {
                return Err("number is outside the interoperable JSON range".to_owned());
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_safe_numbers(value)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                validate_safe_numbers(value)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct StrictValueSeed<'a> {
    failure: &'a RefCell<Option<String>>,
}

impl<'de> DeserializeSeed<'de> for StrictValueSeed<'_> {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor {
            failure: self.failure,
        })
    }
}

struct StrictValueVisitor<'a> {
    failure: &'a RefCell<Option<String>>,
}

impl StrictValueVisitor<'_> {
    fn reject<E: de::Error>(&self, message: impl Into<String>) -> E {
        if self.failure.borrow().is_none() {
            *self.failure.borrow_mut() = Some(message.into());
        }
        E::custom("strict JSON rejected")
    }
}

impl<'de> Visitor<'de> for StrictValueVisitor<'_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a strict JSON value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| self.reject("non-finite number"))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(StrictValueSeed {
            failure: self.failure,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut values = Map::new();
        let mut keys = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(self.reject(format!("duplicate object key {key:?}")));
            }
            let value = map.next_value_seed(StrictValueSeed {
                failure: self.failure,
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

#[cfg(test)]
pub(super) fn empty_fixed_package_fixtures() -> (String, String, String, String) {
    use serde_json::json;

    let timeline_json = json!({
        "document": {
            "canvas": {"width": 320, "height": 180, "fps": "30/1", "sampleRate": 48000, "channelLayout": "stereo", "colorSpace": "srgb", "duration": "1/1"},
            "background": {"color": "#000000ff"},
            "visual": {"tracks": []},
            "audio": {"tracks": []},
            "adjustments": [],
            "captions": {"tracks": []},
            "camera": null,
            "metadata": {}
        }
    })
    .to_string();
    let manifest_json = json!({
        "entries": {}
    })
    .to_string();
    let bundle_json = json!({
        "bindings": {},
        "capabilities": {"camera": false, "artifactAbis": [], "extensionKernels": {}}
    })
    .to_string();
    let profile_json = canonical_fixed_execution_profile(COMMON_PROFILE_KEY).unwrap();
    (timeline_json, manifest_json, bundle_json, profile_json)
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use serde_json::{Value, json};

    use super::{
        CANONICAL_TIMELINE_MEMBER_PATH, COMMON_PROFILE_KEY, FIXED_PACKAGE_FORMAT, FixedPackageFile,
        FixedPackageManifest, RESOURCE_MANIFEST_MEMBER_PATH, VerifiedResourceFactsWire,
        canonical_fixed_execution_profile, canonical_fixed_package_manifest,
        canonical_fixed_package_manifest_bytes, canonical_verified_binding_bundle, content_digest,
        decode_base64_payload, empty_fixed_package_fixtures, fixed_package_files,
        open_verified_fixed_package, parse_strict_json, verify_fixed_package,
    };
    use crate::render::{Capabilities, ResourceBindings};

    const IMAGE_DIGEST: &str =
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const UNUSED_FONT_DIGEST: &str =
        "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";

    fn open_test_fixed_package(
        timeline_json: &str,
        manifest_json: &str,
        verified_binding_bundle_json: &str,
        profile_key: &str,
    ) -> Result<super::OpenedFixedPackage, String> {
        let execution_profile_json = canonical_fixed_execution_profile(profile_key)?;
        let files = fixed_package_files(
            timeline_json,
            manifest_json,
            verified_binding_bundle_json,
            &execution_profile_json,
        );
        let fixed_package_manifest_json = canonical_fixed_package_manifest(&files)?;
        open_verified_fixed_package(&fixed_package_manifest_json, &files)
            .map_err(|error| error.to_string())
    }

    fn image_package_fixtures() -> (String, String, Value) {
        let timeline = json!({
            "document": {
                "canvas": {"width": 320, "height": 180, "fps": "30/1", "sampleRate": 48000, "channelLayout": "stereo", "colorSpace": "srgb", "duration": "1/1"},
                "background": {"color": "#000000ff"},
                "visual": {
                    "tracks": [{
                        "id": "track:main",
                        "items": [{
                            "type": "clip",
                            "id": "clip:hero",
                            "duration": "1/1",
                            "layer": {
                                "transform": {
                                    "position": {"type": "constant", "value": [0.5, 0.5]},
                                    "scale": {"type": "constant", "value": [1.0, 1.0]},
                                    "rotation": {"type": "constant", "value": 0.0},
                                    "anchor": [0.5, 0.5]
                                },
                                "opacity": {"type": "constant", "value": 1.0},
                                "mask": null,
                                "filters": [],
                                "blend": "normal"
                            },
                            "source": {"type": "image", "resource": "asset:hero", "sampling": {"fit": "contain"}}
                        }]
                    }]
                },
                "audio": {"tracks": []},
                "adjustments": [],
                "captions": {"tracks": []},
                "camera": null,
                "metadata": {}
            }
        })
        .to_string();
        let image_descriptor = json!({
            "width": 320,
            "height": 180,
            "orientation": "identity",
            "color": {"primaries": "bt709", "transfer": "bt709", "matrix": "bt709", "fullRange": false}
        });
        let manifest = json!({
            "entries": {
                "asset:hero": {"kind": "image", "digest": IMAGE_DIGEST, "descriptor": image_descriptor.clone()},
                "font:unused": {
                    "kind": "font",
                    "digest": UNUSED_FONT_DIGEST,
                    "descriptor": {"faceIndex": 0, "variationAxes": {}}
                }
            }
        })
        .to_string();
        let bundle = json!({
            "bindings": {
                "asset:hero": {
                    "digest": IMAGE_DIGEST,
                    "handle": 1,
                    "facts": {"kind": "image", "descriptor": image_descriptor, "temporalFootprint": {"pastFrames": 0, "futureFrames": 0}},
                    "dependencies": []
                }
            },
            "capabilities": {"camera": false, "artifactAbis": [], "extensionKernels": {}}
        });
        (timeline, manifest, bundle)
    }

    fn malformed_binding_with_locator_and_payload() -> Value {
        json!({
            "digest": "not-a-digest",
            "handle": 9_007_199_254_740_992_u64,
            "facts": {
                "kind": "font",
                "descriptor": {"faceIndex": 0, "variationAxes": {}},
                "bytesBase64": "not canonical base64!"
            },
            "dependencies": [],
            "locator": {"url": 42}
        })
    }

    fn rewrite_manifest(manifest: &FixedPackageManifest) -> String {
        String::from_utf8(canonical_fixed_package_manifest_bytes(manifest).unwrap()).unwrap()
    }

    fn common_profile_json() -> &'static str {
        static PROFILE: OnceLock<String> = OnceLock::new();
        PROFILE
            .get_or_init(|| canonical_fixed_execution_profile(COMMON_PROFILE_KEY).unwrap())
            .as_str()
    }

    #[test]
    fn canonical_fixed_package_manifest_verifies_and_opens() {
        let (timeline, resource_manifest, bundle, profile) = empty_fixed_package_fixtures();
        let files = fixed_package_files(&timeline, &resource_manifest, &bundle, &profile);
        let manifest_json = canonical_fixed_package_manifest(&files).unwrap();
        let verified = verify_fixed_package(&manifest_json, &files).unwrap();

        assert_eq!(verified.manifest().format(), FIXED_PACKAGE_FORMAT);
        assert!(verified.manifest().resource_digests().is_empty());
        assert_eq!(
            verified.package_digest(),
            &content_digest(manifest_json.as_bytes())
        );
        assert_eq!(
            verified.canonical_manifest_bytes(),
            manifest_json.as_bytes()
        );
        assert!(verified.open().is_ok());
        assert!(open_verified_fixed_package(&manifest_json, &files).is_ok());
    }

    #[test]
    fn fixed_package_embeds_the_complete_canonical_execution_profile() {
        let (timeline, resource_manifest, bundle, _) = empty_fixed_package_fixtures();
        let profile_json = common_profile_json();
        let files = fixed_package_files(&timeline, &resource_manifest, &bundle, profile_json);
        let manifest_json = canonical_fixed_package_manifest(&files).unwrap();
        let manifest: FixedPackageManifest = serde_json::from_str(&manifest_json).unwrap();
        let profile_member = manifest
            .members
            .iter()
            .find(|member| member.path == super::EXECUTION_PROFILE_MEMBER_PATH)
            .unwrap();
        assert_eq!(
            profile_member.digest,
            content_digest(profile_json.as_bytes())
        );
        assert!(profile_json.contains("\"numericAbi\":\"valle.numeric/common@1\""));
        assert!(profile_json.contains("\"maxResources\":64"));

        let mut changed: Value = serde_json::from_str(profile_json).unwrap();
        changed["motionAbi"] = json!("valle.motion/eval@2");
        let changed = String::from_utf8(serde_jcs::to_vec(&changed).unwrap()).unwrap();
        let changed_files = fixed_package_files(&timeline, &resource_manifest, &bundle, &changed);
        let error = canonical_fixed_package_manifest(&changed_files).unwrap_err();
        assert!(
            error.contains("unsupported fixed execution policy"),
            "{error}"
        );
    }

    #[test]
    fn fixed_package_member_swap_is_rejected() {
        let (timeline, resource_manifest, bundle, profile) = empty_fixed_package_fixtures();
        let files = fixed_package_files(&timeline, &resource_manifest, &bundle, &profile);
        let manifest_json = canonical_fixed_package_manifest(&files).unwrap();
        let swapped = [
            FixedPackageFile::new(CANONICAL_TIMELINE_MEMBER_PATH, resource_manifest.as_bytes()),
            FixedPackageFile::new(RESOURCE_MANIFEST_MEMBER_PATH, timeline.as_bytes()),
            files[2],
            files[3],
        ];

        let error = verify_fixed_package(&manifest_json, &swapped).unwrap_err();
        assert!(
            error.contains("mismatch for \"canonical-timeline.json\""),
            "{error}"
        );
    }

    #[test]
    fn fixed_package_missing_and_extra_members_are_rejected() {
        let (timeline, resource_manifest, bundle, profile) = empty_fixed_package_fixtures();
        let files = fixed_package_files(&timeline, &resource_manifest, &bundle, &profile);
        let manifest_json = canonical_fixed_package_manifest(&files).unwrap();

        let missing = verify_fixed_package(&manifest_json, &files[..3]).unwrap_err();
        assert!(missing.contains("missing member role"), "{missing}");

        let mut extra = files.to_vec();
        extra.push(FixedPackageFile::new("extra.json", b"{}"));
        let extra_error = verify_fixed_package(&manifest_json, &extra).unwrap_err();
        assert!(
            extra_error.contains("unexpected member path"),
            "{extra_error}"
        );
    }

    #[test]
    fn fixed_package_length_digest_and_format_errors_are_rejected() {
        let (timeline, resource_manifest, bundle, profile) = empty_fixed_package_fixtures();
        let files = fixed_package_files(&timeline, &resource_manifest, &bundle, &profile);
        let manifest_json = canonical_fixed_package_manifest(&files).unwrap();
        let baseline: FixedPackageManifest = serde_json::from_str(&manifest_json).unwrap();

        let mut wrong_length = baseline.clone();
        wrong_length.members[0].bytes += 1;
        let error = verify_fixed_package(&rewrite_manifest(&wrong_length), &files).unwrap_err();
        assert!(error.contains("byte length mismatch"), "{error}");

        let mut wrong_digest = baseline.clone();
        wrong_digest.members[0].digest =
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .parse()
                .unwrap();
        let error = verify_fixed_package(&rewrite_manifest(&wrong_digest), &files).unwrap_err();
        assert!(error.contains("digest mismatch"), "{error}");

        let mut wrong_format = baseline;
        wrong_format.format = "valle.fixed-render-package@2".to_owned();
        let error = verify_fixed_package(&rewrite_manifest(&wrong_format), &files).unwrap_err();
        assert!(error.contains("unsupported format"), "{error}");
    }

    #[test]
    fn fixed_package_manifest_is_closed_canonical_and_path_normalized() {
        let (timeline, resource_manifest, bundle, profile) = empty_fixed_package_fixtures();
        let files = fixed_package_files(&timeline, &resource_manifest, &bundle, &profile);
        let manifest_json = canonical_fixed_package_manifest(&files).unwrap();
        let baseline: FixedPackageManifest = serde_json::from_str(&manifest_json).unwrap();

        let noncanonical = format!("{manifest_json}\n");
        let error = verify_fixed_package(&noncanonical, &files).unwrap_err();
        assert!(error.contains("canonical JCS"), "{error}");

        let trailing = format!("{manifest_json}x");
        let error = verify_fixed_package(&trailing, &files).unwrap_err();
        assert!(error.contains("trailing or malformed JSON"), "{error}");

        let mut duplicate_role = baseline.clone();
        duplicate_role.members[1].role = duplicate_role.members[0].role;
        duplicate_role.members[1].path = duplicate_role.members[0].path.clone();
        let error = verify_fixed_package(&rewrite_manifest(&duplicate_role), &files).unwrap_err();
        assert!(error.contains("duplicate member role"), "{error}");

        let mut traversal = baseline;
        traversal.members[0].path = "../canonical-timeline.json".to_owned();
        let error = verify_fixed_package(&rewrite_manifest(&traversal), &files).unwrap_err();
        assert!(error.contains("normalized package-relative"), "{error}");

        let mut unknown_role: Value = serde_json::from_str(&manifest_json).unwrap();
        unknown_role["members"][0]["role"] = json!("unknown-canonical-timeline");
        let unknown_role = String::from_utf8(serde_jcs::to_vec(&unknown_role).unwrap()).unwrap();
        let error = verify_fixed_package(&unknown_role, &files).unwrap_err();
        assert!(error.contains("invalid closed manifest"), "{error}");
    }

    #[test]
    fn resource_digests_are_exactly_the_sorted_external_resource_entry_blobs() {
        const VIDEO_DIGEST: &str =
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        const INDEX_DIGEST: &str =
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        const SHADER_DIGEST: &str =
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        const SCHEMA_DIGEST: &str =
            "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
        const IMPLEMENTATION_DIGEST: &str =
            "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
        let (timeline, _, _, profile) = empty_fixed_package_fixtures();
        let bundle = json!({
            "bindings": {},
            "capabilities": {
                "camera": false,
                "artifactAbis": [],
                "extensionKernels": {
                    "vendor/kernel": {
                        "abi": "vendor/kernel@1",
                        "implementationDigest": IMPLEMENTATION_DIGEST,
                        "visualFootprint": {"pastFrames": 0, "futureFrames": 0},
                        "audioFootprint": {"pastSamples": 0, "futureSamples": 0}
                    }
                }
            }
        })
        .to_string();
        let resource_manifest = json!({
            "entries": {
                "asset:video": {
                    "kind": "video",
                    "digest": VIDEO_DIGEST,
                    "descriptor": {
                        "duration": "1/1",
                        "timeBase": "1/90000",
                        "presentationIndexDigest": INDEX_DIGEST,
                        "width": 320,
                        "height": 180,
                        "orientation": "identity",
                        "color": {"primaries": "bt709", "transfer": "bt709", "matrix": "bt709", "fullRange": false},
                        "videoStream": 0,
                        "audioStream": null
                    }
                },
                "shader:glass": {
                    "kind": "shader",
                    "digest": SHADER_DIGEST,
                    "abi": "valle.shader/artifact@1",
                    "descriptor": {"controlsSchemaDigest": SCHEMA_DIGEST, "readsDestination": true}
                }
            }
        })
        .to_string();
        let files = fixed_package_files(&timeline, &resource_manifest, &bundle, &profile);
        let manifest_json = canonical_fixed_package_manifest(&files).unwrap();
        let verified = verify_fixed_package(&manifest_json, &files).unwrap();
        let actual = verified
            .manifest()
            .resource_digests()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert_eq!(actual, [VIDEO_DIGEST, SHADER_DIGEST]);

        let mut incomplete: FixedPackageManifest = serde_json::from_str(&manifest_json).unwrap();
        incomplete.resource_digests.pop();
        let error = verify_fixed_package(&rewrite_manifest(&incomplete), &files).unwrap_err();
        assert!(error.contains("complete package blob set"), "{error}");
    }

    #[test]
    fn derived_audio_facts_are_bound_by_members_but_are_not_cas_roots() {
        const ENCODED_DIGEST: &str =
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        const INDEX_DIGEST: &str =
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        const PCM_DIGEST: &str =
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let (_, _, _, profile) = empty_fixed_package_fixtures();
        let timeline = json!({
            "document": {
                "canvas": {"width": 320, "height": 180, "fps": "30/1", "sampleRate": 48000, "channelLayout": "stereo", "colorSpace": "srgb", "duration": "1/1"},
                "background": {"color": "#000000ff"},
                "visual": {"tracks": []},
                "audio": {"tracks": [{
                    "id": "audio:main",
                    "items": [{
                        "type": "clip",
                        "id": "audio:clip",
                        "duration": "1/1",
                        "source": {"type": "media", "resource": "audio:source", "sourceStart": "0/1", "rate": "1/1", "endBehavior": "hold"},
                        "gain": {"type": "constant", "value": 1.0},
                        "pan": {"type": "constant", "value": 0.0},
                        "effects": []
                    }]
                }]},
                "adjustments": [],
                "captions": {"tracks": []},
                "camera": null,
                "metadata": {}
            }
        })
        .to_string();
        let descriptor = json!({
            "duration": "1/1",
            "timeBase": "1/48000",
            "presentationIndexDigest": INDEX_DIGEST,
            "sampleRate": 48000,
            "channelLayout": "stereo",
            "audioStream": 0
        });
        let resource_manifest = json!({
            "entries": {
                "audio:source": {"kind": "audio", "digest": ENCODED_DIGEST, "descriptor": descriptor.clone()}
            }
        })
        .to_string();
        let bundle = json!({
            "bindings": {
                "audio:source": {
                    "digest": ENCODED_DIGEST,
                    "handle": 1,
                    "facts": {
                        "kind": "audio",
                        "descriptor": descriptor,
                        "temporalFootprint": {"pastSamples": 0, "futureSamples": 0},
                        "decodedPcmDigest": PCM_DIGEST
                    },
                    "dependencies": []
                }
            },
            "capabilities": {"camera": false, "artifactAbis": [], "extensionKernels": {}}
        })
        .to_string();
        let files = fixed_package_files(&timeline, &resource_manifest, &bundle, &profile);
        let manifest_json = canonical_fixed_package_manifest(&files).unwrap();
        let verified = verify_fixed_package(&manifest_json, &files).unwrap();
        let actual = verified
            .manifest()
            .resource_digests()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert_eq!(actual, [ENCODED_DIGEST]);
    }

    #[test]
    fn fixed_package_open_returns_only_render_id_and_result_facts() {
        let (timeline, manifest, bundle, _) = empty_fixed_package_fixtures();
        let opened =
            open_test_fixed_package(&timeline, &manifest, &bundle, COMMON_PROFILE_KEY).unwrap();
        let receipt: serde_json::Value = serde_json::from_str(&opened.receipt_json).unwrap();
        assert_eq!(
            receipt["renderId"],
            opened.compiled().render_id().to_string()
        );
        assert_eq!(receipt["canvasWidth"], 320);
        assert_eq!(receipt["canvasHeight"], 180);
        assert_eq!(receipt["frameRate"], "30/1");
        assert_eq!(receipt["frameCount"], 30);
        assert_eq!(receipt["sampleRate"], 48_000);
        assert_eq!(receipt["sampleCount"], 48_000);
        assert_eq!(receipt.as_object().unwrap().len(), 7);
    }

    #[test]
    fn binding_bundle_and_profile_are_closed_and_fail_closed() {
        let (timeline, manifest, bundle, _) = empty_fixed_package_fixtures();
        let unknown = bundle.replacen("\"bindings\":{}", "\"bindings\":{},\"unsupported\":true", 1);
        assert!(
            open_test_fixed_package(&timeline, &manifest, &unknown, COMMON_PROFILE_KEY)
                .unwrap_err()
                .contains("closed bundle")
        );

        let duplicate = bundle.replacen("\"camera\":false", "\"camera\":false,\"camera\":false", 1);
        assert!(
            open_test_fixed_package(&timeline, &manifest, &duplicate, COMMON_PROFILE_KEY)
                .unwrap_err()
                .contains("duplicate object key")
        );

        assert!(
            open_test_fixed_package(&timeline, &manifest, &bundle, "custom-profile",)
                .unwrap_err()
                .contains("unsupported profile")
        );
    }

    #[test]
    fn domain_bundle_serializer_round_trips_through_the_same_closed_opener() {
        let (timeline, manifest, _, _) = empty_fixed_package_fixtures();
        let bundle =
            canonical_verified_binding_bundle(&ResourceBindings::new(), &Capabilities::new())
                .unwrap();
        let opened =
            open_test_fixed_package(&timeline, &manifest, &bundle, COMMON_PROFILE_KEY).unwrap();
        assert_eq!(opened.receipt().render_id(), opened.compiled().render_id());
    }

    #[test]
    fn unused_malformed_binding_locator_and_payload_do_not_affect_open_or_render_identity() {
        let (timeline, manifest, baseline_bundle) = image_package_fixtures();
        let baseline = open_test_fixed_package(
            &timeline,
            &manifest,
            &baseline_bundle.to_string(),
            COMMON_PROFILE_KEY,
        )
        .unwrap();

        let mut poisoned_bundle = baseline_bundle;
        poisoned_bundle["bindings"]["font:unused"] = malformed_binding_with_locator_and_payload();
        let opened = open_test_fixed_package(
            &timeline,
            &manifest,
            &poisoned_bundle.to_string(),
            COMMON_PROFILE_KEY,
        )
        .unwrap();

        assert_eq!(baseline.receipt().render_id(), opened.receipt().render_id());
    }

    #[test]
    fn malformed_binding_is_rejected_when_directly_or_transitively_referenced() {
        let (timeline, manifest, baseline_bundle) = image_package_fixtures();

        let mut direct = baseline_bundle.clone();
        direct["bindings"]["asset:hero"] = malformed_binding_with_locator_and_payload();
        let direct_error = open_test_fixed_package(
            &timeline,
            &manifest,
            &direct.to_string(),
            COMMON_PROFILE_KEY,
        )
        .unwrap_err();
        assert!(direct_error.contains("asset:hero"), "{direct_error}");

        let mut transitive = baseline_bundle;
        transitive["bindings"]["asset:hero"]["dependencies"] =
            json!([{"role": "payload", "resourceId": "font:unused"}]);
        transitive["bindings"]["font:unused"] = malformed_binding_with_locator_and_payload();
        let transitive_error = open_test_fixed_package(
            &timeline,
            &manifest,
            &transitive.to_string(),
            COMMON_PROFILE_KEY,
        )
        .unwrap_err();
        assert!(
            transitive_error.contains("font:unused"),
            "{transitive_error}"
        );
    }

    #[test]
    fn bundle_preflight_rejects_unsafe_integer_spellings_and_noncanonical_base64() {
        assert!(
            parse_strict_json(r#"{"handle":9007199254740992.0}"#, "bundle")
                .unwrap_err()
                .contains("interoperable JSON range")
        );
        assert_eq!(
            decode_base64_payload("e30=".to_owned(), "data.bytesBase64")
                .unwrap()
                .as_ref(),
            b"{}"
        );
        assert!(decode_base64_payload("e30".to_owned(), "font.bytesBase64").is_err());
    }

    #[test]
    fn audio_decoded_pcm_digest_is_required_in_the_closed_bundle() {
        let facts = serde_json::json!({
            "kind": "audio",
            "descriptor": {
                "duration": "1/1",
                "timeBase": "1/48000",
                "presentationIndexDigest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "sampleRate": 48000,
                "channelLayout": "mono",
                "audioStream": 0
            },
            "temporalFootprint": {"pastSamples": 0, "futureSamples": 0}
        });
        assert!(
            serde_json::from_value::<VerifiedResourceFactsWire>(facts.clone()).is_err(),
            "decodedPcmDigest must never default or be inferred from the encoded resource digest"
        );
        let mut complete = facts;
        complete["decodedPcmDigest"] = serde_json::json!(
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
        assert!(serde_json::from_value::<VerifiedResourceFactsWire>(complete).is_ok());
    }
}
