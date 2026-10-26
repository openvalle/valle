//! Shared standalone Motion fixed-package construction for Studio and Native export.
//!
//! This is deliberately separate from the HTTP/UI projection. A successful package is a complete
//! Timeline render input whose resource proof is opened by the Rust Product engine before
//! any JSON is published to Studio.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use valle_engine::fixed_package::{
    COMMON_PROFILE_KEY, OpenedFixedPackage, build_fixed_package, canonical_fixed_execution_profile,
    canonical_verified_binding_bundle, fixed_package_files,
};
use valle_engine::render::{
    AudioFootprint, Capabilities, ResourceBinding, ResourceBindings, ResourceDependency,
    VerifiedHandleId, VerifiedResourceFacts, VisualFootprint,
};
use valle_motion::{
    AssetKind, ControlType, MotionValue, SceneArtifact,
    shader::ShaderPackage,
    value::{AngleUnit, LengthUnit},
};
use valle_timeline::internal::{
    CanonicalTimeline, ContentDigest, ResourceManifest, canonical_bytes, decode_canonical,
    wire::resource::{
        AudioChannelLayoutWire, AudioResourceDescriptorWire, ColorMatrixWire, ColorPrimariesWire,
        ColorTransferWire, ContinuousBoundarySamplingWire, FontResourceDescriptorWire,
        FontVariationAxisWire, ImageResourceDescriptorWire, MediaColorDescriptorWire,
        MediaOrientationWire, Model3dResourceDescriptorWire, MotionArtifactAbiWire,
        MotionArtifactDescriptorWire, ResourceEntryWire, ResourceManifestEnvelopeWire,
        ShaderArtifactAbiWire, ShaderResourceDescriptorWire,
    },
};
use valle_timeline::{FrameRate, RationalTime, time::ExactRational};

use super::motion::BoundAsset;

const COMPONENT_RESOURCE_ID: &str = "component:standalone-motion";

pub(super) struct StandaloneMotionPackageInput<'a> {
    pub artifact: &'a SceneArtifact,
    pub assets: &'a BTreeMap<String, BoundAsset>,
    pub font_blobs: &'a [std::sync::Arc<[u8]>],
    pub prop_bindings: &'a BTreeMap<String, Value>,
    /// The caption a captionPresenter draws.
    pub caption: Option<&'a valle_motion::caption::CaptionPresenterData>,
    pub duration: RationalTime,
    pub frame_rate: FrameRate,
    pub canvas: (u32, u32),
}

#[derive(Debug)]
pub(super) struct StandaloneMotionPackage {
    pub fixed_package_manifest_json: String,
    pub timeline_json: String,
    pub timeline: Value,
    pub resource_manifest_json: String,
    pub resource_manifest: Value,
    pub verified_binding_bundle_json: String,
    pub resource_bytes: Vec<u8>,
    /// The package opened by the closed-package admission check above, so a native render reuses
    /// it instead of parsing, decoding and hashing the same members a second time.
    pub opened: OpenedFixedPackage,
}

pub(super) fn font_dependency_role(artifact: &SceneArtifact, bytes: &[u8], index: usize) -> String {
    let has_formula = artifact
        .nodes
        .iter()
        .any(|node| matches!(node.kind, valle_motion::NodeKind::MathFormula { .. }));
    let is_formula = has_formula
        && valle_motion::math_formula::formula_font_pack().any(|(_, font)| font == bytes);
    format!(
        "{}:{index}",
        if is_formula { "formula-font" } else { "font" }
    )
}

pub(super) fn build_standalone_motion_package(
    input: StandaloneMotionPackageInput<'_>,
) -> Result<StandaloneMotionPackage> {
    let artifact_digest = motion_digest(&valle_motion::canonical_bytes(input.artifact)?);
    let resource_ids = asset_resource_ids(input.artifact, input.assets)?;
    let timeline = build_timeline(&input, &resource_ids, &artifact_digest)?;

    let motion_descriptor = MotionArtifactDescriptorWire {
        reads_destination: input.artifact.reads_destination(),
        boundary_sampling: ContinuousBoundarySamplingWire::LeftLimit,
    };

    let mut resources = FixedResources::new();
    let mut component_dependencies = Vec::new();
    for (control, resource_id) in &resource_ids {
        let asset = input
            .assets
            .get(control)
            .expect("resource ids were built from bound assets");
        let kind = input.artifact.controls.assets[control].kind;
        resources.add_asset(resource_id, kind, asset)?;
        component_dependencies.push(FixedResourceDependency {
            role: control.clone(),
            resource_id: resource_id.clone(),
        });
    }

    for (index, bytes) in input.font_blobs.iter().enumerate() {
        let digest = resources.intern_shared_font(bytes.clone())?;
        component_dependencies.push(FixedResourceDependency {
            role: font_dependency_role(input.artifact, bytes, index),
            resource_id: font_resource_id(&digest),
        });
    }

    resources.add(
        COMPONENT_RESOURCE_ID,
        ResourceEntryWire::MotionArtifact {
            digest: artifact_digest,
            abi: MotionArtifactAbiWire::Canonical,
            descriptor: motion_descriptor.clone(),
        },
        VerifiedResourceFacts::MotionArtifact {
            abi: MotionArtifactAbiWire::Canonical,
            descriptor: motion_descriptor.clone(),
            artifact: Arc::new(input.artifact.clone()),
            temporal_footprint: VisualFootprint::default(),
        },
        component_dependencies,
    )?;

    let capabilities = resources.capabilities();
    let manifest = ResourceManifest::try_from_wire(ResourceManifestEnvelopeWire {
        entries: resources.entries,
    })?;
    let timeline_json = String::from_utf8(canonical_bytes(&timeline)?)
        .context("canonical Timeline is not UTF-8")?;
    let manifest_json = String::from_utf8(manifest.canonical_bytes().to_vec())
        .context("canonical ResourceManifest is not UTF-8")?;
    let bundle = canonical_verified_binding_bundle(&resources.domain_bindings, &capabilities)
        .map_err(|error| anyhow!(error))?;
    let verified_binding_bundle_json = bundle.json;
    let resource_bytes = bundle.resource_bytes;
    let execution_profile_json =
        canonical_fixed_execution_profile(COMMON_PROFILE_KEY).map_err(|error| anyhow!(error))?;
    let files = fixed_package_files(
        &timeline_json,
        &manifest_json,
        &verified_binding_bundle_json,
        &execution_profile_json,
        &resource_bytes,
    );
    let (fixed_package_manifest_json, opened) = build_fixed_package(&files).map_err(|error| {
        anyhow!("standalone Motion fixed package failed closed-package admission: {error}")
    })?;

    let timeline_value: Value = serde_json::from_str(&timeline_json)?;
    let manifest_value: Value = serde_json::from_str(&manifest_json)?;

    Ok(StandaloneMotionPackage {
        fixed_package_manifest_json,
        timeline_json,
        timeline: timeline_value,
        resource_manifest_json: manifest_json,
        resource_manifest: manifest_value,
        verified_binding_bundle_json,
        resource_bytes,
        opened,
    })
}

fn build_timeline(
    input: &StandaloneMotionPackageInput<'_>,
    resource_ids: &BTreeMap<String, String>,
    artifact_digest: &ContentDigest,
) -> Result<CanonicalTimeline> {
    let props = authored_default_props(input.artifact, input.prop_bindings)?;
    let composition = input
        .artifact
        .composition
        .as_ref()
        .context("standalone Motion needs a composition")?;
    // The clip holds a one-shot scene and loops a looping one, as its standalone clock does.
    let end_behavior = if composition.r#loop { "loop" } else { "hold" };
    let mut document = json!({
        "document": {
            "canvas": {
                "width": input.canvas.0,
                "height": input.canvas.1,
                "fps": input.frame_rate,
                "sampleRate": 48_000,
                "channelLayout": "stereo",
                "colorSpace": "srgb",
                "duration": input.duration,
            },
            "background": {"color": "#00000000"},
            "visual": {
                "tracks": [{
                    "id": "visual:motion",
                    "items": [{
                        "type": "clip",
                        "id": "clip:motion",
                        "duration": input.duration,
                        "layer": standalone_motion_layer(input.canvas),
                        "source": {
                            "type": "motion",
                            "component": COMPONENT_RESOURCE_ID,
                            "fit": "contain",
                            "sourceStart": "0/1",
                            "role": input.artifact.role,
                            "sourceDuration": composition.duration().map_err(|error| anyhow!(error))?,
                            "rate": "1/1",
                            "endBehavior": end_behavior,
                            "props": props,
                            "resources": resource_ids,
                        },
                    }],
                }],
            },
            "audio": {"tracks": []},
            "adjustments": [],
            "captions": {"tracks": []},
            "camera": null,
            "metadata": {"standaloneMotionArtifactDigest": artifact_digest},
        },
    });
    if matches!(
        input.artifact.role,
        valle_timeline::MotionRole::CaptionPresenter { .. }
    ) {
        let mut source = document["document"]["visual"]["tracks"][0]["items"][0]["source"].clone();
        source
            .as_object_mut()
            .expect("source object")
            .remove("type");
        let caption = input
            .caption
            .context("a captionPresenter draws a caption; give one with --data")?;
        let runs = caption_runs(caption, input.duration)?;
        document["document"]["visual"]["tracks"] = json!([]);
        document["document"]["captions"]["tracks"] = json!([{ "id": "caption:motion", "items": [{ "type": "motion", "id": "clip:motion", "duration": input.duration, "source": source, "runs": runs }] }]);
    }
    decode_canonical(&serde_json::to_string(&document)?).map_err(Into::into)
}

/// The runs of a caption as a canonical caption item holds them: each with its times, size and
/// color, an untimed run spanning the caption.
fn caption_runs(
    caption: &valle_motion::caption::CaptionPresenterData,
    duration: RationalTime,
) -> Result<Value> {
    let time = |seconds: f64| -> Result<RationalTime> {
        let wire = valle_timeline::wire::timeline::TimelineTimeWire::new(seconds.to_string())?;
        Ok(RationalTime::from_exact(wire.to_exact()))
    };
    let runs = caption
        .runs
        .iter()
        .enumerate()
        .map(|(index, run)| {
            let (start, end) = match (run.start, run.end) {
                (Some(start), Some(end)) => (time(start)?, time(end)?),
                _ => (RationalTime::ZERO, duration),
            };
            Ok(json!({
                "id": format!("clip:motion:run:{index}"),
                "text": run.text,
                "timing": {"start": start, "end": end},
                "style": {
                    "fontSize": run.font_size.unwrap_or(caption.style.font_size),
                    "color": run.color.as_deref().unwrap_or(&caption.style.color),
                },
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::Array(runs))
}

fn standalone_motion_layer(canvas: (u32, u32)) -> Value {
    json!({
        "transform": {
            "size": {"type":"constant","value":[canvas.0,canvas.1]},
            "position": {"type": "constant", "value": [0.5, 0.5]},
            "scale": {"type": "constant", "value": [1.0, 1.0]},
            "rotation": {"type": "constant", "value": 0.0},
            "anchor": [0.5, 0.5],
        },
        "opacity": {"type": "constant", "value": 1.0},
        "mask": null,
        "filters": [],
        "blend": "normal",
    })
}

fn authored_default_props(
    artifact: &SceneArtifact,
    bindings: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>> {
    for name in bindings.keys() {
        if !artifact.controls.props.contains_key(name) {
            bail!("unknown Motion prop `{name}`");
        }
    }
    let mut props = BTreeMap::new();
    for (name, control) in &artifact.controls.props {
        if let Some(value) = bindings.get(name) {
            props.insert(name.clone(), json!({"type":"constant", "value":value}));
            continue;
        }
        let Some(default) = &control.default else {
            if control.required {
                bail!("required Motion prop `{name}` has no binding or default");
            }
            continue;
        };
        props.insert(
            name.clone(),
            json!({"type": "constant", "value": motion_value(default, &control.control)?}),
        );
    }
    Ok(props)
}

fn motion_value(value: &MotionValue, control: &ControlType) -> Result<Value> {
    let output = match (value, control) {
        (MotionValue::Number(value), ControlType::Number { .. }) => json!(value),
        (MotionValue::Length(value), ControlType::Length) if value.unit == LengthUnit::Px => {
            json!(value.value)
        }
        (MotionValue::Angle(value), ControlType::Angle) if value.unit == AngleUnit::Deg => {
            json!(value.value)
        }
        (MotionValue::Point(value), ControlType::Point) => json!([value.x, value.y]),
        (MotionValue::Color(value), ControlType::Color) => {
            let value = value.to_srgb8();
            json!([value.r, value.g, value.b, value.a])
        }
        (MotionValue::Rect(value), ControlType::Rect) => {
            json!([value.x, value.y, value.width, value.height])
        }
        (MotionValue::Bool(value), ControlType::Bool) => json!(value),
        (MotionValue::Str(value), ControlType::String) => json!(value),
        (MotionValue::Enum(value), ControlType::Select { values }) if values.contains(value) => {
            json!(value)
        }
        _ => bail!("Motion prop default does not match its fixed-package control type"),
    };
    Ok(output)
}

fn asset_resource_ids(
    artifact: &SceneArtifact,
    assets: &BTreeMap<String, BoundAsset>,
) -> Result<BTreeMap<String, String>> {
    let mut ids = BTreeMap::new();
    for control in assets.keys() {
        if !artifact.controls.assets.contains_key(control) {
            bail!("asset binding `{control}` is absent from the compiled Motion controls schema");
        }
        if control
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace() || ch == '/')
        {
            bail!("asset control `{control}` cannot form a Timeline ResourceId");
        }
        ids.insert(control.clone(), format!("asset:{control}"));
    }
    Ok(ids)
}

pub(super) fn font_resource_id(digest: &ContentDigest) -> String {
    format!("font:{}:0", digest.as_hex())
}

fn motion_digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest::of_bytes(bytes)
}

pub(super) struct FixedResources {
    pub(super) entries: BTreeMap<String, ResourceEntryWire>,
    pub(super) domain_bindings: ResourceBindings,
    next_handle: u64,
    has_shader: bool,
}

pub(super) struct FixedResourceDependency {
    pub(super) role: String,
    pub(super) resource_id: String,
}

impl FixedResources {
    pub(super) fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            domain_bindings: ResourceBindings::new(),
            next_handle: 1,
            has_shader: false,
        }
    }

    pub(super) fn add_asset(
        &mut self,
        resource_id: &str,
        kind: AssetKind,
        asset: &BoundAsset,
    ) -> Result<()> {
        let digest = asset.hash;
        match kind {
            AssetKind::Image => {
                let descriptor = image_descriptor(&asset.bytes)
                    .with_context(|| format!("probing image asset `{resource_id}`"))?;
                self.add(
                    resource_id,
                    ResourceEntryWire::Image {
                        digest,
                        descriptor: descriptor.clone(),
                    },
                    VerifiedResourceFacts::Image {
                        descriptor: descriptor.clone(),
                        temporal_footprint: VisualFootprint::default(),
                    },
                    Vec::new(),
                )
            }
            AssetKind::Font => self.add_font_with_digest(resource_id, digest, asset.bytes.clone()),
            AssetKind::Audio => {
                let decoded = decode_audio_corpus(asset)?;
                let descriptor = analyzed_audio_descriptor(&decoded)?;
                self.add_audio(resource_id, digest, descriptor)
            }
            AssetKind::Video => add_video_resource(self, resource_id, digest, &asset.path, false)
                .with_context(|| {
                    format!(
                        "probing Motion video asset `{resource_id}` ({})",
                        asset.path.display()
                    )
                }),
            AssetKind::Model3d => {
                let model = valle_motion::scene3d::admit_glb(&asset.bytes)
                    .with_context(|| format!("admitting GLB asset `{resource_id}`"))?;
                let descriptor = Model3dResourceDescriptorWire {
                    byte_length: u32::try_from(model.source_bytes())?,
                    vertex_count: model.vertex_count(),
                    triangle_count: model.triangle_count(),
                };
                self.add(
                    resource_id,
                    ResourceEntryWire::Model3d {
                        digest,
                        descriptor: descriptor.clone(),
                    },
                    VerifiedResourceFacts::Model3d {
                        descriptor,
                        bytes: asset.bytes.clone(),
                    },
                    Vec::new(),
                )
            }
            AssetKind::Shader => {
                let package = ShaderPackage::from_frozen(&asset.bytes)?;
                self.add_shader(resource_id, &package)
            }
            AssetKind::Environment => {
                let environment =
                    valle_motion::scene3d::EnvironmentAsset::from_frozen(&asset.bytes)?;
                let descriptor = environment.descriptor();
                self.add(
                    resource_id,
                    ResourceEntryWire::Environment {
                        digest: environment.content_digest(),
                        descriptor: descriptor.clone(),
                    },
                    VerifiedResourceFacts::Environment {
                        descriptor,
                        bytes: environment.frozen_bytes()?.into(),
                    },
                    Vec::new(),
                )
            }
        }
    }

    pub(super) fn add_audio(
        &mut self,
        id: &str,
        digest: ContentDigest,
        descriptor: AudioResourceDescriptorWire,
    ) -> Result<()> {
        self.add(
            id,
            ResourceEntryWire::Audio {
                digest,
                descriptor: descriptor.clone(),
            },
            VerifiedResourceFacts::Audio {
                descriptor,
                temporal_footprint: AudioFootprint::default(),
            },
            Vec::new(),
        )
    }

    /// Share dependency fonts across Motion components without changing each component's
    /// ordered font roles, returning the font's digest. Font descriptors currently admit face
    /// index zero only.
    pub(super) fn intern_font(&mut self, bytes: &[u8]) -> Result<ContentDigest> {
        let digest = motion_digest(bytes);
        let resource_id = font_resource_id(&digest);
        if !self.entries.contains_key(&resource_id) {
            self.add_font_with_digest(&resource_id, digest, Arc::from(bytes))?;
        }
        Ok(digest)
    }

    fn intern_shared_font(&mut self, bytes: Arc<[u8]>) -> Result<ContentDigest> {
        let digest = motion_digest(&bytes);
        let resource_id = font_resource_id(&digest);
        if !self.entries.contains_key(&resource_id) {
            self.add_font_with_digest(&resource_id, digest, bytes)?;
        }
        Ok(digest)
    }

    fn add_font_with_digest(
        &mut self,
        resource_id: &str,
        digest: ContentDigest,
        bytes: Arc<[u8]>,
    ) -> Result<()> {
        let descriptor = font_descriptor(&bytes)?;
        self.add(
            resource_id,
            ResourceEntryWire::Font {
                digest,
                descriptor: descriptor.clone(),
            },
            VerifiedResourceFacts::Font {
                descriptor: descriptor.clone(),
                bytes,
            },
            Vec::new(),
        )
    }

    pub(super) fn add_shader(&mut self, resource_id: &str, package: &ShaderPackage) -> Result<()> {
        self.has_shader = true;
        let digest = package.content_hash;
        let controls_schema_digest = package.abi_hash;
        let descriptor = ShaderResourceDescriptorWire {
            controls_schema_digest,
            reads_destination: false,
        };
        self.add(
            resource_id,
            ResourceEntryWire::Shader {
                digest,
                abi: ShaderArtifactAbiWire::Canonical,
                descriptor: descriptor.clone(),
            },
            VerifiedResourceFacts::Shader {
                abi: ShaderArtifactAbiWire::Canonical,
                descriptor: descriptor.clone(),
                manifest_bytes: Arc::from(package.canonical_manifest.as_slice()),
                source_bytes: Arc::from(package.source.as_bytes()),
            },
            Vec::new(),
        )
    }

    pub(super) fn add(
        &mut self,
        resource_id: &str,
        entry: ResourceEntryWire,
        facts: VerifiedResourceFacts,
        dependencies: Vec<FixedResourceDependency>,
    ) -> Result<()> {
        if self
            .entries
            .insert(resource_id.to_owned(), entry.clone())
            .is_some()
        {
            bail!("duplicate standalone resource `{resource_id}`");
        }
        let digest = *entry_digest(&entry);
        let handle = VerifiedHandleId::new(self.next_handle)?;
        self.next_handle += 1;
        let mut binding = ResourceBinding::new(digest, handle, facts);
        for dependency in &dependencies {
            binding = binding.with_dependency(ResourceDependency::new(
                dependency.role.clone(),
                dependency.resource_id.clone(),
            )?);
        }
        self.domain_bindings.insert(resource_id, binding)?;
        Ok(())
    }

    pub(super) fn capabilities(&self) -> Capabilities {
        let mut capabilities = Capabilities::new().with_artifact_abi("valle.motion/artifact@1");
        if self.has_shader {
            capabilities = capabilities.with_artifact_abi("valle.shader/artifact@1");
        }
        capabilities
    }
}

fn image_descriptor(bytes: &[u8]) -> Result<ImageResourceDescriptorWire> {
    let (width, height) = valle_render::host::probe_image_extent(bytes)?;
    Ok(ImageResourceDescriptorWire {
        width,
        height,
        orientation: MediaOrientationWire::Identity,
        color: MediaColorDescriptorWire {
            primaries: ColorPrimariesWire::Srgb,
            transfer: ColorTransferWire::Srgb,
            matrix: ColorMatrixWire::Identity,
            full_range: true,
        },
    })
}

fn font_descriptor(bytes: &[u8]) -> Result<FontResourceDescriptorWire> {
    let face = ttf_parser::Face::parse(bytes, 0).map_err(|_| anyhow!("invalid font face"))?;
    let variation_axes = face
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
    Ok(FontResourceDescriptorWire {
        face_index: 0,
        variation_axes,
    })
}

struct DecodedAudioCorpus {
    channels: u16,
    stream: u32,
    sample_count: i64,
}

const STANDALONE_AUDIO_PRESENTATION_INDEX_DOMAIN: &[u8] =
    b"valle.audio/uniform-pcm-presentation-index@1\0";

fn analyzed_audio_descriptor(decoded: &DecodedAudioCorpus) -> Result<AudioResourceDescriptorWire> {
    let duration = RationalTime::new(decoded.sample_count, 48_000)?;
    let mut index_fact = Vec::with_capacity(
        STANDALONE_AUDIO_PRESENTATION_INDEX_DOMAIN.len()
            + core::mem::size_of::<u32>() * 2
            + core::mem::size_of::<u16>()
            + core::mem::size_of::<i64>(),
    );
    index_fact.extend_from_slice(STANDALONE_AUDIO_PRESENTATION_INDEX_DOMAIN);
    index_fact.extend_from_slice(&48_000_u32.to_le_bytes());
    index_fact.extend_from_slice(&decoded.channels.to_le_bytes());
    index_fact.extend_from_slice(&decoded.stream.to_le_bytes());
    index_fact.extend_from_slice(&decoded.sample_count.to_le_bytes());
    Ok(AudioResourceDescriptorWire {
        duration,
        time_base: ExactRational::new(1, 48_000)?,
        presentation_index_digest: motion_digest(&index_fact),
        sample_rate: 48_000,
        channel_layout: if decoded.channels == 1 {
            AudioChannelLayoutWire::Mono
        } else {
            AudioChannelLayoutWire::Stereo
        },
        audio_stream: decoded.stream,
    })
}

fn decode_audio_corpus(asset: &BoundAsset) -> Result<DecodedAudioCorpus> {
    let info = valle_media::codec::audio::probe_audio_stream(&asset.path)?
        .ok_or_else(|| anyhow!("no audio stream in {}", asset.path.display()))?;
    if !matches!(info.channels, 1 | 2) {
        bail!(
            "audio requires mono or stereo; {} has {} channels",
            asset.path.display(),
            info.channels
        );
    }
    // The descriptor duration is the exact decoded sample count, which container metadata
    // does not guarantee.
    let mut decoder =
        valle_media::codec::LibavAudioStream::open(&asset.path, 48_000, info.channels)?;
    let mut sample_count = 0_i64;
    loop {
        let block = decoder.read(48_000)?;
        if block.samples.is_empty() {
            break;
        }
        sample_count += i64::try_from(block.samples.len() / usize::from(info.channels))?;
    }
    if sample_count == 0 {
        bail!("decoded common-profile PCM must contain at least one sample");
    }
    Ok(DecodedAudioCorpus {
        channels: info.channels,
        stream: info.stream,
        sample_count,
    })
}

pub(crate) fn preview_audio(path: &Path) -> Result<(AudioResourceDescriptorWire, Arc<[u8]>)> {
    let info = valle_media::codec::audio::probe_audio_stream(path)?
        .ok_or_else(|| anyhow!("no audio stream in {}", path.display()))?;
    if !matches!(info.channels, 1 | 2) {
        bail!(
            "audio requires mono or stereo; {} has {} channels",
            path.display(),
            info.channels
        );
    }
    let samples = valle_media::codec::decode_audio_mono_f32(path, 48_000)?;
    if samples.is_empty() {
        bail!("decoded common-profile PCM must contain at least one sample");
    }
    let descriptor = analyzed_audio_descriptor(&DecodedAudioCorpus {
        channels: info.channels,
        stream: info.stream,
        sample_count: i64::try_from(samples.len())?,
    })?;
    let bytes = samples
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>()
        .into();
    Ok((descriptor, bytes))
}

fn entry_digest(entry: &ResourceEntryWire) -> &ContentDigest {
    match entry {
        ResourceEntryWire::Video { digest, .. }
        | ResourceEntryWire::Audio { digest, .. }
        | ResourceEntryWire::Image { digest, .. }
        | ResourceEntryWire::Font { digest, .. }
        | ResourceEntryWire::Model3d { digest, .. }
        | ResourceEntryWire::Environment { digest, .. }
        | ResourceEntryWire::MotionArtifact { digest, .. }
        | ResourceEntryWire::Shader { digest, .. } => digest,
    }
}

pub(super) fn add_video_resource(
    resources: &mut FixedResources,
    id: &str,
    hash: ContentDigest,
    path: &Path,
    include_audio: bool,
) -> Result<()> {
    let probe = valle_media::codec::decode::probe_video_presentation(path)?;
    add_probed_video_resource(resources, id, hash, path, include_audio, probe)
}

pub(super) fn add_probed_video_resource(
    resources: &mut FixedResources,
    id: &str,
    hash: ContentDigest,
    path: &Path,
    include_audio: bool,
    probe: valle_media::codec::decode::VideoPresentationProbe,
) -> Result<()> {
    use valle_timeline::internal::wire::resource::*;
    let audio = valle_media::codec::audio::probe_audio_stream(path)?;
    let mut dependencies = Vec::new();
    if audio.is_some() && include_audio {
        let audio_id = format!("{id}:audio");
        resources.add_asset(
            &audio_id,
            AssetKind::Audio,
            &super::motion::BoundAsset {
                path: path.to_path_buf(),
                bytes: Arc::from([]),
                hash,
            },
        )?;
        dependencies.push(FixedResourceDependency {
            role: "audio".into(),
            resource_id: audio_id,
        });
    }
    let descriptor = VideoResourceDescriptorWire {
        duration: valle_timeline::RationalTime::new(
            probe
                .duration_ticks
                .checked_mul(i64::from(probe.time_base.0))
                .ok_or_else(|| anyhow!("video duration overflow"))?,
            probe.time_base.1 as u32,
        )?,
        time_base: valle_timeline::time::ExactRational::new(
            i64::from(probe.time_base.0),
            probe.time_base.1 as u32,
        )?,
        presentation_index_digest: ContentDigest::of_bytes(&serde_json::to_vec(
            &probe.presentation,
        )?),
        width: probe.display.width,
        height: probe.display.height,
        has_alpha: probe.has_alpha,
        orientation: MediaOrientationWire::Identity,
        color: MediaColorDescriptorWire {
            primaries: ColorPrimariesWire::Srgb,
            transfer: ColorTransferWire::Srgb,
            matrix: ColorMatrixWire::Identity,
            full_range: true,
        },
        video_stream: probe.stream,
        audio_stream: audio.map(|audio| audio.stream),
    };
    resources.add(
        id,
        ResourceEntryWire::Video {
            digest: hash,
            descriptor: descriptor.clone(),
        },
        valle_engine::render::VerifiedResourceFacts::Video {
            descriptor,
            temporal_footprint: Default::default(),
        },
        dependencies,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use valle_engine::fixed_package::open_verified_fixed_package;

    #[test]
    fn audio_corpus_counts_exact_decoded_samples() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&42_u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&48_000_u32.to_le_bytes());
        bytes.extend_from_slice(&96_000_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&6_u32.to_le_bytes());
        for sample in [-32768_i16, 0, 32767] {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(file.path(), &bytes).unwrap();
        let corpus = decode_audio_corpus(&BoundAsset {
            path: file.path().to_path_buf(),
            hash: ContentDigest::of_bytes(&bytes),
            bytes: Vec::new().into(),
        })
        .unwrap();
        assert_eq!(corpus.sample_count, 3);
        assert_eq!(corpus.channels, 1);
        let (descriptor, pcm) = preview_audio(file.path()).unwrap();
        assert_eq!(descriptor, analyzed_audio_descriptor(&corpus).unwrap());
        assert_eq!(pcm.len(), 3 * 4);
    }

    #[test]
    fn shared_analysis_decode_preserves_exact_mono_and_stereo_resource_descriptors() {
        use valle_media::{codec::FloatWavWriter, frame::AudioBuffer};
        let directory = tempfile::tempdir().unwrap();
        for rate in [16_000, 44_100] {
            for channels in [1, 2] {
                let path = directory.path().join(format!("{rate}-{channels}.wav"));
                let mut writer = FloatWavWriter::create(&path, rate, channels).unwrap();
                writer
                    .write(&AudioBuffer {
                        samples: (0..2_137 * usize::from(channels))
                            .map(|index| (index % 31) as f32 / 31.0 - 0.5)
                            .collect(),
                        sample_rate: rate,
                        channels,
                    })
                    .unwrap();
                writer.finish().unwrap();
                let asset = BoundAsset {
                    path: path.clone(),
                    hash: ContentDigest::of_bytes(&std::fs::read(&path).unwrap()),
                    bytes: Arc::from([]),
                };
                let expected =
                    analyzed_audio_descriptor(&decode_audio_corpus(&asset).unwrap()).unwrap();
                let (actual, pcm) = preview_audio(&path).unwrap();
                assert_eq!(actual, expected, "{rate} Hz / {channels} channels");
                assert_eq!(pcm.len() % 4, 0);
            }
        }
    }

    #[test]
    fn dependency_fonts_share_one_resource_and_preserve_distinct_faces() {
        let mut resources = FixedResources::new();
        let regular = valle_motion::DEFAULT_MOTION_FONT_WEIGHTS[0];
        let other = valle_motion::DEFAULT_MOTION_FONT_WEIGHTS[1];
        let first = resources.intern_font(regular).unwrap();
        for _ in 0..32 {
            assert_eq!(resources.intern_font(regular).unwrap(), first);
        }
        assert_eq!(resources.entries.len(), 1);
        let second = resources.intern_font(other).unwrap();
        assert_ne!(first, second);
        assert_eq!(resources.entries.len(), 2);
        let mut reverse = FixedResources::new();
        assert_eq!(reverse.intern_font(other).unwrap(), second);
        assert_eq!(reverse.intern_font(regular).unwrap(), first);
        assert_eq!(resources.entries, reverse.entries);
    }

    #[test]
    fn font_closure_includes_later_formulas_but_not_unused_formula_faces() {
        let plain = valle_compiler::motion::compile_motion(
            r#"export default function T(){return <Text>Hello</Text>; }"#,
        )
        .unwrap()
        .artifact;
        let later = valle_compiler::motion::compile_motion(
            r##"export default function T(){ const time = useCurrentTime();return <Scene>
                <MathFormula latex="\\frac{1}{2}" style={{opacity:time.seconds > 1 ? 1 : 0}} />
            </Scene>; }"##,
        )
        .unwrap()
        .artifact;
        let plain_fonts =
            super::super::motion::fixed_package_font_blobs(&plain, &[], None, &[]).unwrap();
        let formula_fonts =
            super::super::motion::fixed_package_font_blobs(&later, &[], None, &[]).unwrap();
        let all_formula_count = valle_motion::math_formula::formula_font_pack().count();
        assert!(formula_fonts.len() > plain_fonts.len());
        assert!(formula_fonts.len() < plain_fonts.len() + all_formula_count);
        assert_eq!(plain_fonts.len(), 1);
        assert!(formula_fonts.iter().all(|font| {
            valle_motion::math_formula::formula_font_pack().any(|(_, bytes)| font.as_ref() == bytes)
        }));
    }

    fn text_package(source: &str) -> StandaloneMotionPackage {
        let artifact = valle_compiler::motion::compile_motion(source)
            .unwrap()
            .artifact;
        let fonts =
            super::super::motion::fixed_package_font_blobs(&artifact, &[], None, &[]).unwrap();
        build_standalone_motion_package(StandaloneMotionPackageInput {
            artifact: &artifact,
            assets: &BTreeMap::new(),
            font_blobs: &fonts,
            prop_bindings: &BTreeMap::new(),
            caption: None,
            duration: RationalTime::ONE,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            canvas: (320, 180),
        })
        .unwrap()
    }

    #[test]
    fn host_fonts_replace_text_stack_and_preserve_formula_resources() {
        let package = text_package(
            r#"export const composition = { width: 320, height: 180, duration: 1 };
export default function T(){return <Scene><Text>Hello</Text><MathFormula latex="x^2" /></Scene>}"#,
        );
        let font = valle_motion::DEFAULT_MOTION_FONT_FILES
            .iter()
            .zip(valle_motion::DEFAULT_MOTION_FONT_WEIGHTS)
            .find(|(name, _)| **name == "NotoSansMono-Regular.ttf")
            .unwrap()
            .1;
        let selected = valle_engine::fixed_package::with_motion_fonts(
            &package.fixed_package_manifest_json,
            &package.timeline_json,
            &package.resource_manifest_json,
            &package.verified_binding_bundle_json,
            &package.resource_bytes,
            &[valle_engine::fixed_package::MotionFontBytes {
                bytes: font.to_vec(),
            }],
        )
        .unwrap();
        let bundle: Value = serde_json::from_str(&selected.verified_binding_bundle_json).unwrap();
        let dependencies = bundle["bindings"][COMPONENT_RESOURCE_ID]["dependencies"]
            .as_array()
            .unwrap();
        assert_eq!(
            dependencies
                .iter()
                .filter(|dep| dep["role"].as_str().unwrap().starts_with("font:"))
                .count(),
            1
        );
        assert!(
            dependencies
                .iter()
                .any(|dep| dep["role"].as_str().unwrap().starts_with("formula-font:"))
        );
        let profile = canonical_fixed_execution_profile(COMMON_PROFILE_KEY).unwrap();
        let files = fixed_package_files(
            &selected.timeline_json,
            &selected.resource_manifest_json,
            &selected.verified_binding_bundle_json,
            &profile,
            &selected.resource_bytes,
        );
        let render = open_verified_fixed_package(&selected.fixed_package_manifest_json, &files)
            .unwrap()
            .engine_render();
        let spec = valle_engine::frame::RenderSpec::new(
            320,
            180,
            valle_engine::frame::RenderQuality::Preview,
            valle_engine::resource::OutputSpec::srgb_preview(
                valle_engine::resource::OutputBackground::opaque_srgb([0, 0, 0]),
            )
            .unwrap(),
        )
        .unwrap();
        let mut compiler = render.frame_compiler();
        let frame = compiler
            .evaluate_prepare(
                render.render_id(),
                valle_timeline::internal::FrameKey::new(0),
                spec,
            )
            .unwrap();
        let requests = serde_json::to_string(&frame.prepared().resource_requests).unwrap();
        assert!(requests.contains(&ContentDigest::of_bytes(font).as_hex().to_owned()));
        assert!(
            !requests.contains(
                &ContentDigest::of_bytes(valle_motion::DEFAULT_MOTION_FONT)
                    .as_hex()
                    .to_owned()
            )
        );
        // Reapplying the same selection is stable, without accumulating resources or handles.
        let repeated = valle_engine::fixed_package::with_motion_fonts(
            &selected.fixed_package_manifest_json,
            &selected.timeline_json,
            &selected.resource_manifest_json,
            &selected.verified_binding_bundle_json,
            &selected.resource_bytes,
            &[valle_engine::fixed_package::MotionFontBytes {
                bytes: font.to_vec(),
            }],
        )
        .unwrap();
        assert_eq!(
            selected.fixed_package_manifest_json,
            repeated.fixed_package_manifest_json
        );
    }

    #[test]
    fn standalone_timeline_centers_the_full_canvas_motion_source() {
        let layer = standalone_motion_layer((64, 32));

        assert_eq!(
            layer["transform"]["position"],
            json!({"type": "constant", "value": [0.5, 0.5]})
        );
    }

    fn serialized_audio_bundle() -> Value {
        let descriptor = AudioResourceDescriptorWire {
            duration: RationalTime::ONE,
            time_base: ExactRational::new(1, 48_000).unwrap(),
            presentation_index_digest: motion_digest(b"index"),
            sample_rate: 48_000,
            channel_layout: AudioChannelLayoutWire::Mono,
            audio_stream: 0,
        };
        let mut bindings = ResourceBindings::new();
        bindings
            .insert(
                "audio:test",
                ResourceBinding::new(
                    motion_digest(b"audio"),
                    VerifiedHandleId::new(1).unwrap(),
                    VerifiedResourceFacts::Audio {
                        descriptor,
                        temporal_footprint: AudioFootprint::default(),
                    },
                ),
            )
            .unwrap();
        serde_json::from_str(
            &canonical_verified_binding_bundle(&bindings, &Capabilities::new())
                .unwrap()
                .json,
        )
        .unwrap()
    }

    #[test]
    fn engine_bundle_serializer_keeps_audio_decode_facts_without_analysis_sidecars() {
        let value = serialized_audio_bundle();
        assert!(
            value["bindings"]["audio:test"]["facts"]
                .get("envelope")
                .is_none()
        );
    }

    #[test]
    fn audio_descriptor_duration_is_the_exact_decoded_pcm_sample_count() {
        let decoded = DecodedAudioCorpus {
            channels: 1,
            stream: 0,
            sample_count: 48_001,
        };
        let descriptor = analyzed_audio_descriptor(&decoded).unwrap();
        assert_eq!(
            descriptor.duration,
            RationalTime::new(48_001, 48_000).unwrap()
        );
        assert_eq!(descriptor.time_base, ExactRational::new(1, 48_000).unwrap());
        assert_eq!(descriptor.sample_rate, 48_000);
        assert_eq!(descriptor.channel_layout, AudioChannelLayoutWire::Mono);

        let next = analyzed_audio_descriptor(&DecodedAudioCorpus {
            channels: 1,
            stream: 0,
            sample_count: 48_002,
        })
        .unwrap();
        assert_ne!(
            descriptor.presentation_index_digest,
            next.presentation_index_digest
        );
    }
}
