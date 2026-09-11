//! Shared standalone Motion fixed-package construction for Studio and Native export.
//!
//! This is deliberately separate from the HTTP/UI projection. A successful package is a complete
//! Timeline render input whose resource proof is opened by the Rust Product engine before
//! any JSON is published to Studio.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use valle_engine::fixed_package::{
    COMMON_PROFILE_KEY, canonical_fixed_execution_profile, canonical_fixed_package_manifest,
    canonical_verified_binding_bundle, fixed_package_files, open_verified_fixed_package,
};
use valle_engine::render::{
    AudioFootprint, Capabilities, ResourceBinding, ResourceBindings, ResourceDependency,
    VerifiedHandleId, VerifiedResourceFacts, VisualFootprint,
};
use valle_motion::{
    AssetKind, ControlType, CueWindow, MotionValue, NodeKind, SceneArtifact,
    shader::{ShaderPackage, ShaderRegistry},
    value::{AngleUnit, LengthUnit},
};
use valle_timeline::internal::{
    CanonicalTimeline, ContentDigest, ResourceManifest, canonical_bytes, decode_canonical,
    wire::resource::{
        AudioChannelLayoutWire, AudioResourceDescriptorWire, ColorMatrixWire, ColorPrimariesWire,
        ColorTransferWire, ContinuousBoundarySamplingWire, FontResourceDescriptorWire,
        FontVariationAxisWire, ImageResourceDescriptorWire, MediaColorDescriptorWire,
        MediaOrientationWire, MotionArtifactAbiWire, MotionArtifactDescriptorWire,
        ResourceEntryWire, ResourceManifestEnvelopeWire, ShaderArtifactAbiWire,
        ShaderResourceDescriptorWire,
    },
};
use valle_timeline::{FrameRate, RationalTime, time::ExactRational};

use super::motion::BoundAsset;

const COMPONENT_RESOURCE_ID: &str = "component:standalone-motion";

pub(super) struct StandaloneMotionPackageInput<'a> {
    pub artifact: &'a SceneArtifact,
    pub assets: &'a BTreeMap<String, BoundAsset>,
    pub font_blobs: &'a [Vec<u8>],
    pub shaders: &'a ShaderRegistry,
    pub cue_bindings: &'a BTreeMap<String, CueWindow>,
    pub prop_bindings: &'a BTreeMap<String, Value>,
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
    pub execution_profile_json: String,
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
        let resource_id = format!("font:standalone-{index}");
        resources.add_font(&resource_id, bytes)?;
        component_dependencies.push(FixedResourceDependency {
            role: format!("font:{index}"),
            resource_id,
        });
    }

    let used_shaders = used_shader_uris(input.artifact);
    for package in input
        .shaders
        .packages()
        .filter(|package| used_shaders.contains(&package.uri().to_string()))
    {
        let resource_id = shader_resource_id(package);
        resources.add_shader(&resource_id, package)?;
        component_dependencies.push(FixedResourceDependency {
            role: format!("shader:{}", package.uri()),
            resource_id,
        });
    }

    resources.add(
        COMPONENT_RESOURCE_ID,
        ResourceEntryWire::MotionArtifact {
            digest: artifact_digest.clone(),
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
    let verified_binding_bundle_json =
        canonical_verified_binding_bundle(&resources.domain_bindings, &capabilities)
            .map_err(|error| anyhow!(error))?;
    let execution_profile_json =
        canonical_fixed_execution_profile(COMMON_PROFILE_KEY).map_err(|error| anyhow!(error))?;
    let files = fixed_package_files(
        &timeline_json,
        &manifest_json,
        &verified_binding_bundle_json,
        &execution_profile_json,
    );
    let fixed_package_manifest_json =
        canonical_fixed_package_manifest(&files).map_err(|error| anyhow!(error))?;
    let opened =
        open_verified_fixed_package(&fixed_package_manifest_json, &files).map_err(|error| {
            anyhow!("standalone Motion fixed package failed closed-package admission: {error}")
        })?;
    if opened.compiled().canvas().frame_rate() != input.frame_rate {
        bail!("standalone Motion fixed package self-check returned a different frame rate");
    }

    let timeline_value: Value = serde_json::from_str(&timeline_json)?;
    let manifest_value: Value = serde_json::from_str(&manifest_json)?;

    Ok(StandaloneMotionPackage {
        fixed_package_manifest_json,
        timeline_json,
        timeline: timeline_value,
        resource_manifest_json: manifest_json,
        resource_manifest: manifest_value,
        verified_binding_bundle_json,
        execution_profile_json,
    })
}

fn build_timeline(
    input: &StandaloneMotionPackageInput<'_>,
    resource_ids: &BTreeMap<String, String>,
    artifact_digest: &ContentDigest,
) -> Result<CanonicalTimeline> {
    let props = authored_default_props(input.artifact, input.prop_bindings)?;
    let cues = input
        .cue_bindings
        .iter()
        .map(|(name, cue)| {
            Ok((
                name.clone(),
                json!({
                    "type": "source-range",
                    "start": frame_time(cue.start_frame, input.frame_rate)?,
                    "end": frame_time(cue.end_frame, input.frame_rate)?,
                    "enterDuration": frame_time(cue.enter_frames, input.frame_rate)?,
                    "exitDuration": frame_time(cue.exit_frames, input.frame_rate)?,
                }),
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let phase = input.artifact.controls.phase_spec();
    let document = json!({
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
            "background": {"color": "#000000ff"},
            "visual": {
                "tracks": [{
                    "id": "visual:motion",
                    "items": [{
                        "type": "clip",
                        "id": "clip:motion",
                        "duration": input.duration,
                        "layer": standalone_motion_layer(),
                        "source": {
                            "type": "motion",
                            "component": COMPONENT_RESOURCE_ID,
                            "sourceStart": "0/1",
                            "sourceDuration": input.duration,
                            "rate": "1/1",
                            "endBehavior": "hold",
                            "props": props,
                            "cues": cues,
                            "resources": resource_ids,
                            "phases": {
                                "enterDuration": frame_time(phase.enter_frames, input.frame_rate)?,
                                "exitDuration": frame_time(phase.exit_frames, input.frame_rate)?,
                            },
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
    decode_canonical(&serde_json::to_string(&document)?).map_err(Into::into)
}

fn standalone_motion_layer() -> Value {
    json!({
        "transform": {
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
            json!([value.r, value.g, value.b, value.a])
        }
        (MotionValue::Rect(value), ControlType::Rect) => {
            json!([value.x, value.y, value.width, value.height])
        }
        (MotionValue::Bool(value), ControlType::Bool) => json!(value),
        (MotionValue::Str(value), ControlType::String | ControlType::NodeTarget) => json!(value),
        (MotionValue::Enum(value), ControlType::Select { values }) if values.contains(value) => {
            json!(value)
        }
        (MotionValue::PathData(_), ControlType::PathData) => {
            bail!("PathData Motion props are not admitted by Timeline")
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

fn frame_time(frame: u32, frame_rate: FrameRate) -> Result<RationalTime> {
    let numerator = i64::from(frame)
        .checked_mul(i64::from(frame_rate.denominator()))
        .ok_or_else(|| anyhow!("Motion frame time overflow"))?;
    let denominator = u32::try_from(frame_rate.numerator())
        .map_err(|_| anyhow!("Motion frame rate exceeds u32"))?;
    Ok(RationalTime::new(numerator, denominator)?)
}

fn motion_digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest::of_bytes(bytes)
}

fn used_shader_uris(artifact: &SceneArtifact) -> BTreeSet<String> {
    artifact
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::ShaderLayer { program, .. } => Some(program.uri.clone()),
            _ => None,
        })
        .collect()
}

fn shader_resource_id(package: &ShaderPackage) -> String {
    format!(
        "shader:{}-v{}",
        package.manifest.name, package.manifest.version
    )
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
            AssetKind::Font => self.add_font_with_digest(resource_id, digest, &asset.bytes),
            AssetKind::Audio => {
                let decoded = decode_audio_corpus(asset)?;
                let descriptor = analyzed_audio_descriptor(&decoded)?;
                self.add(
                    resource_id,
                    ResourceEntryWire::Audio {
                        digest,
                        descriptor: descriptor.clone(),
                    },
                    VerifiedResourceFacts::Audio {
                        descriptor: descriptor.clone(),
                        temporal_footprint: AudioFootprint::default(),
                        decoded_pcm_digest: decoded.digest,
                    },
                    Vec::new(),
                )
            }
            AssetKind::Video => bail!(
                "Video Motion assets require a verified presentation-index descriptor not yet produced by standalone Studio"
            ),
            AssetKind::Model3d => {
                bail!("Model3D Motion assets have no Timeline ResourceManifest kind")
            }
        }
    }

    pub(super) fn add_font(&mut self, resource_id: &str, bytes: &[u8]) -> Result<()> {
        self.add_font_with_digest(resource_id, motion_digest(bytes), bytes)
    }

    fn add_font_with_digest(
        &mut self,
        resource_id: &str,
        digest: ContentDigest,
        bytes: &[u8],
    ) -> Result<()> {
        let descriptor = font_descriptor(bytes)?;
        self.add(
            resource_id,
            ResourceEntryWire::Font {
                digest,
                descriptor: descriptor.clone(),
            },
            VerifiedResourceFacts::Font {
                descriptor: descriptor.clone(),
                bytes: Arc::from(bytes),
            },
            Vec::new(),
        )
    }

    pub(super) fn add_shader(&mut self, resource_id: &str, package: &ShaderPackage) -> Result<()> {
        self.has_shader = true;
        let digest = package.content_hash;
        let controls_schema_digest = package.manifest.abi_digest;
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
        let digest = entry_digest(&entry).clone();
        let handle = VerifiedHandleId::new(self.next_handle)?;
        self.next_handle += 1;
        let mut binding = ResourceBinding::new(digest.clone(), handle, facts);
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
        let mut capabilities = Capabilities::new()
            .with_artifact_abi("valle.motion/artifact@1")
            .with_artifact_abi("valle.lottie/artifact@1");
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
    digest: ContentDigest,
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
    let mut decoder =
        valle_media::codec::LibavAudioStream::open(&asset.path, 48_000, info.channels)?;
    let mut samples = Vec::new();
    loop {
        let block = decoder.read(48_000)?;
        if block.samples.is_empty() {
            break;
        }
        samples.extend_from_slice(&block.samples);
    }
    let sample_count = i64::try_from(samples.len() / usize::from(info.channels))
        .map_err(|_| anyhow!("decoded common-profile PCM exceeds the exact time domain"))?;
    if sample_count == 0 {
        bail!("decoded common-profile PCM must contain at least one sample");
    }
    let digest = valle_engine::render::common_audio_pcm_digest(48_000, info.channels, &samples)
        .map_err(|error| anyhow!("hash common-profile decoded PCM: {error}"))?;
    Ok(DecodedAudioCorpus {
        channels: info.channels,
        stream: info.stream,
        sample_count,
        digest,
    })
}

fn entry_digest(entry: &ResourceEntryWire) -> &ContentDigest {
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

#[cfg(test)]
mod tests {
    use super::*;
    use valle_engine::render::common_audio_pcm_digest;

    #[test]
    fn standalone_timeline_centers_the_full_canvas_motion_source() {
        let layer = standalone_motion_layer();

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
                        decoded_pcm_digest: common_audio_pcm_digest(48_000, 1, &[0.0]).unwrap(),
                    },
                ),
            )
            .unwrap();
        serde_json::from_str(
            &canonical_verified_binding_bundle(&bindings, &Capabilities::new()).unwrap(),
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
        assert!(
            value["bindings"]["audio:test"]["facts"]["decodedPcmDigest"]
                .as_str()
                .is_some_and(|digest| digest.starts_with("sha256:"))
        );
    }

    #[test]
    fn audio_descriptor_duration_is_the_exact_decoded_pcm_sample_count() {
        let decoded = DecodedAudioCorpus {
            channels: 1,
            stream: 0,
            sample_count: 48_001,
            digest: common_audio_pcm_digest(48_000, 1, &vec![0.0; 48_001]).unwrap(),
        };
        let descriptor = analyzed_audio_descriptor(&decoded).unwrap();
        assert_eq!(
            descriptor.duration,
            RationalTime::new(48_001, 48_000).unwrap()
        );
        assert_eq!(descriptor.time_base, ExactRational::new(1, 48_000).unwrap());
        assert_eq!(descriptor.sample_rate, 48_000);
        assert_eq!(descriptor.channel_layout, AudioChannelLayoutWire::Mono);
        assert_ne!(descriptor.presentation_index_digest, decoded.digest);

        let next = analyzed_audio_descriptor(&DecodedAudioCorpus {
            channels: 1,
            stream: 0,
            sample_count: 48_002,
            digest: decoded.digest,
        })
        .unwrap();
        assert_ne!(
            descriptor.presentation_index_digest,
            next.presentation_index_digest
        );
    }
}
