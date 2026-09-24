//! File and project Timeline delivery through the fixed-package admission path.
use crate::{
    TimelineAction,
    preview_store::{FrozenMediaCache, PreviewFile},
};
use anyhow::{Context, Result, anyhow, bail};
use std::{path::Path, sync::Arc};
use valle_engine::fixed_package::*;
use valle_motion::{AssetKind, ContentDigest};
use valle_render::host::{
    NativeProject, NativeRenderOptions, NativeRenderer, NativeResourceCatalog,
};
use valle_timeline::internal::{
    ResourceManifest, encode_canonical, wire::resource::ResourceManifestEnvelopeWire,
};
use valle_timeline::{Timeline, decode_timeline, timeline_bytes};

pub fn run(action: TimelineAction) -> Result<std::process::ExitCode> {
    match action {
        TimelineAction::Studio {
            input,
            web_assets_dir,
            port,
        } => crate::timeline_file::serve(&input, web_assets_dir.as_deref(), port),
        TimelineAction::Check { input } => {
            let timeline = decode_timeline(&super::read(&input)?)?;
            let temp = tempfile::tempdir()?;
            render_document_impl(
                timeline,
                input.parent().unwrap_or(Path::new(".")),
                &temp.path().join("check.png"),
                Some(0),
                true,
            )?;
            crate::output::emit(serde_json::json!({"status":"ok", "input":input}));
            Ok(std::process::ExitCode::SUCCESS)
        }
        TimelineAction::Render {
            input,
            output,
            frame,
        } => {
            let timeline = decode_timeline(&super::read(&input)?)?;
            render_document(
                timeline,
                input.parent().unwrap_or(Path::new(".")),
                &output,
                frame,
            )
        }
    }
}

pub(super) fn render_document(
    timeline: Timeline,
    base: &Path,
    output: &Path,
    frame: Option<i64>,
) -> Result<std::process::ExitCode> {
    render_document_impl(timeline, base, output, frame, false)
}

fn render_document_impl(
    timeline: Timeline,
    base: &Path,
    output: &Path,
    frame: Option<i64>,
    checking: bool,
) -> Result<std::process::ExitCode> {
    super::fixed_render::require_new_output(output)?;
    let extension = if frame.is_some() { "png" } else { "mp4" };
    if !output
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(extension))
    {
        bail!("output must have an .{extension} extension");
    }
    let prepared = prepare_timeline_package(timeline, base)?;
    let profile = canonical_fixed_execution_profile(COMMON_PROFILE_KEY).map_err(|e| anyhow!(e))?;
    let files = fixed_package_files(
        &prepared.canonical_timeline_json,
        &prepared.resource_manifest_json,
        &prepared.verified_binding_bundle_json,
        &profile,
    );
    let opened = open_verified_fixed_package(&prepared.fixed_package_manifest_json, &files)
        .map_err(|e| anyhow!(e))?;
    let catalog = prepared.catalog;
    let renderer = NativeRenderer::new(
        NativeProject::from_render(opened.engine_render(), Arc::new(catalog)),
        NativeRenderOptions {
            backend: valle_render::executor::skia::SkiaBackendKind::Raster,
            background: valle_engine::resource::OutputBackground::AuthorSrgbStraight {
                color: valle_engine::resource::AuthorSrgbStraight(
                    opened.engine_render().canvas().background_rgba(),
                ),
            },
            progress: crate::output::render_progress(),
            ..NativeRenderOptions::default()
        },
    );
    if checking {
        return Ok(std::process::ExitCode::SUCCESS);
    }
    let summary = match frame {
        Some(f) => renderer.preview_frame_key(valle_engine::render::FrameKey::new(f), output)?,
        None => renderer.export_mp4(output)?,
    };
    super::fixed_render::print_delivery_report(
        &opened,
        if frame.is_some() { "preview" } else { "export" },
        output,
        &summary,
        None,
    )?;
    Ok(std::process::ExitCode::SUCCESS)
}

/// One frozen preparation shared by Native delivery and browser previews.
pub(crate) struct TimelinePreviewPackage {
    pub timeline_json: String,
    pub canonical_timeline_json: String,
    pub fixed_package_manifest_json: String,
    pub resource_manifest_json: String,
    pub verified_binding_bundle_json: String,
    pub motion: serde_json::Value,
    pub blobs: std::collections::BTreeMap<String, PreviewFile>,
    pub input_dependencies: std::collections::BTreeMap<String, ContentDigest>,
    catalog: NativeResourceCatalog,
}

pub(crate) fn prepare_timeline_package(
    timeline: Timeline,
    base: &Path,
) -> Result<TimelinePreviewPackage> {
    prepare_timeline_package_impl(timeline, base, None)
}

pub(crate) fn prepare_timeline_preview(
    timeline: Timeline,
    base: &Path,
    media: &mut FrozenMediaCache,
) -> Result<TimelinePreviewPackage> {
    prepare_timeline_package_impl(timeline, base, Some(media))
}

/// Capture only the media inputs consumed by browser preparation. Motion source
/// evaluation stays in the Studio Worker, including when a clip binds assets.
pub(crate) fn prepare_timeline_media_facts(
    timeline: Timeline,
    base: &Path,
    media: &mut FrozenMediaCache,
) -> Result<(
    Vec<serde_json::Value>,
    std::collections::BTreeMap<String, PreviewFile>,
    std::collections::BTreeMap<String, ContentDigest>,
)> {
    use std::collections::{BTreeMap, BTreeSet};

    let original_doc: serde_json::Value = serde_json::from_slice(&timeline_bytes(&timeline)?)?;
    let mut doc = original_doc.clone();
    let mut motion_assets = BTreeSet::new();
    for track in doc["tracks"]["visual"].as_array_mut().into_iter().flatten() {
        if let Some(clips) = track["clips"].as_array_mut() {
            clips.retain(|clip| {
                if clip["kind"] != "motion" {
                    return true;
                }
                if let Some(bindings) = clip["resources"].as_object() {
                    motion_assets.extend(
                        bindings
                            .values()
                            .filter_map(|value| value.as_str().map(str::to_owned)),
                    );
                }
                false
            });
        }
    }
    let mut used_by_media = BTreeSet::new();
    collect_used_resources(&doc["tracks"], &mut used_by_media);
    let mut inputs = Vec::new();
    let mut blobs = BTreeMap::new();
    let mut input_dependencies = BTreeMap::new();
    let mut record_dependency = |path: &Path, digest: ContentDigest| -> Result<()> {
        let key = path.canonicalize()?.to_string_lossy().into_owned();
        if input_dependencies
            .insert(key.clone(), digest)
            .is_some_and(|old| old != digest)
        {
            bail!("Motion asset changed during preview: {key}");
        }
        Ok(())
    };
    if !used_by_media.is_empty() {
        let package = prepare_timeline_preview(decode_timeline(&doc.to_string())?, base, media)?;
        let manifest: serde_json::Value = serde_json::from_str(&package.resource_manifest_json)?;
        let bundle: serde_json::Value =
            serde_json::from_str(&package.verified_binding_bundle_json)?;
        let entries = manifest["entries"]
            .as_object()
            .context("media entries missing")?;
        let bindings = bundle["bindings"]
            .as_object()
            .context("media bindings missing")?;
        for (id, entry) in entries {
            let binding = bindings
                .get(id)
                .with_context(|| format!("media binding {id} missing"))?;
            inputs.push(serde_json::json!({
                "id": id, "entry": entry, "facts": binding["facts"],
                "dependencies": binding["dependencies"],
            }));
        }
        blobs = package.blobs;
    }

    let frozen = Arc::new(tempfile::tempdir().context("freezing Motion preview assets")?);
    let mut resources = super::motion_package::FixedResources::new();
    for alias in motion_assets {
        let id = format!("resource:{alias}");
        let locator = original_doc["resources"][&alias]
            .as_str()
            .with_context(|| format!("missing Motion resource {alias}"))?;
        if locator.contains("://") {
            bail!("Motion resource {alias}: use a local resource locator");
        }
        let path = base.join(locator);
        // Probe only media containers. FFmpeg probing an arbitrary image or
        // font can block the preview request.
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let video_candidate = matches!(
            extension.as_str(),
            "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v"
        );
        let audio_candidate = video_candidate
            || matches!(
                extension.as_str(),
                "wav" | "mp3" | "m4a" | "aac" | "flac" | "ogg" | "opus"
            );
        if inputs.iter().any(|input| input["id"] == id) {
            if audio_candidate {
                let (digest, _) = media.get(&path)?;
                record_dependency(&path, digest)?;
            } else {
                let (_, dependencies) = super::motion::load_asset_with_dependencies(&path)?;
                for (path, digest) in dependencies {
                    record_dependency(&path, digest)?;
                }
            }
            continue;
        }
        let video =
            video_candidate && valle_media::codec::decode::probe_video_presentation(&path).is_ok();
        let audio = !video
            && audio_candidate
            && matches!(
                valle_media::codec::audio::probe_audio_stream(&path),
                Ok(Some(_))
            );
        if video || audio {
            let (hash, file) = media.get(&path)?;
            record_dependency(&path, hash)?;
            let frozen_path = match &file {
                PreviewFile::File { path, .. } => path.clone(),
                PreviewFile::Bytes(_) => unreachable!(),
            };
            blobs.entry(hash.as_hex().to_owned()).or_insert(file);
            if video {
                add_video_resource(
                    &mut resources,
                    &id,
                    hash,
                    &frozen_path,
                    has_motion_resource(&original_doc["tracks"], &alias),
                )?;
            } else {
                let asset = super::motion::BoundAsset {
                    path: frozen_path,
                    bytes: Arc::from([]),
                    hash,
                };
                resources.add_asset(&id, AssetKind::Audio, &asset)?;
            }
            continue;
        }
        let (mut asset, dependencies) = super::motion::load_asset_with_dependencies(&path)?;
        for (path, digest) in dependencies {
            record_dependency(&path, digest)?;
        }
        let kind = if locator.ends_with(".shader.json") {
            AssetKind::Shader
        } else if ttf_parser::Face::parse(&asset.bytes, 0).is_ok() {
            AssetKind::Font
        } else if valle_render::host::probe_image_extent(&asset.bytes).is_ok() {
            AssetKind::Image
        } else if valle_motion::scene3d::admit_glb(&asset.bytes).is_ok() {
            AssetKind::Model3d
        } else {
            let environment = valle_motion::scene3d::EnvironmentAsset::from_encoded(
                &asset.bytes,
                Default::default(),
            )
            .with_context(|| format!("unsupported Motion resource {alias}"))?;
            asset.bytes = environment.frozen_bytes()?.into();
            AssetKind::Environment
        };
        asset.hash = ContentDigest::of_bytes(&asset.bytes);
        let frozen_path = frozen.path().join(asset.hash.as_hex());
        std::fs::write(&frozen_path, &asset.bytes)?;
        asset.path = frozen_path.clone();
        blobs
            .entry(asset.hash.as_hex().to_owned())
            .or_insert(PreviewFile::File {
                path: frozen_path,
                _directory: Arc::clone(&frozen),
            });
        resources.add_asset(&id, kind, &asset)?;
    }
    let capabilities = resources.capabilities();
    let bindings = canonical_verified_binding_bundle(&resources.domain_bindings, &capabilities)
        .map_err(|error| anyhow!(error))?;
    let bindings: serde_json::Value = serde_json::from_str(&bindings)?;
    for (id, entry) in resources.entries {
        let binding = &bindings["bindings"][&id];
        inputs.push(serde_json::json!({
            "id": id, "entry": entry, "facts": binding["facts"],
            "dependencies": binding["dependencies"],
        }));
    }
    Ok((inputs, blobs, input_dependencies))
}

fn add_video_resource(
    resources: &mut super::motion_package::FixedResources,
    id: &str,
    hash: ContentDigest,
    path: &Path,
    include_audio: bool,
) -> Result<()> {
    use valle_timeline::internal::wire::resource::*;
    let probe = valle_media::codec::decode::probe_video_presentation(path)?;
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
        dependencies.push(super::motion_package::FixedResourceDependency {
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

fn prepare_timeline_package_impl(
    timeline: Timeline,
    base: &Path,
    media: Option<&mut FrozenMediaCache>,
) -> Result<TimelinePreviewPackage> {
    // The browser consumes HTTP snapshots, so it needs no native catalog rehash.
    let native = media.is_none();
    let mut fresh_media = FrozenMediaCache::default();
    let media = media.unwrap_or(&mut fresh_media);
    let timeline_json = String::from_utf8(timeline_bytes(&timeline)?)?;
    let mut doc: serde_json::Value = serde_json::from_slice(&timeline_bytes(&timeline)?)?;
    prepare_motion_instances(&mut doc)?;
    let captured_motions = capture_motion_sources(&doc, base)?;
    let input_dependencies = captured_motions.dependency_digests()?;
    let (mut prepared_motions, motion_sources) = captured_motions.prepare()?;
    let compiled = valle_compiler::compile_timeline_with_motion_sources(timeline, &motion_sources)?;
    let canonical = encode_canonical(&compiled)?;
    let frozen = Arc::new(tempfile::tempdir().context("creating immutable render resources")?);
    let mut resources = super::motion_package::FixedResources::new();
    let mut catalog = NativeResourceCatalog::new();
    let mut blobs = std::collections::BTreeMap::new();
    let mut structures = Vec::new();
    if let Some(locators) = doc["resources"].as_object() {
        let mut used = std::collections::BTreeSet::new();
        collect_used_resources(&doc["tracks"], &mut used);
        let mut ordered: Vec<_> = locators
            .iter()
            .filter(|(name, _)| used.contains(name.as_str()))
            .collect();
        ordered.sort_by_key(|(name, _)| motion_component(&doc, name).is_none());
        let mut motion_asset_kinds = std::collections::BTreeMap::new();
        let mut motion_environments = std::collections::BTreeMap::new();
        for (name, locator) in ordered {
            let locator = locator
                .as_str()
                .ok_or_else(|| anyhow!("invalid locator for {name}"))?;
            let path = if locator.starts_with("https://") || locator.starts_with("http://") {
                valle_render::AssetCache::new(valle_render::default_cache_dir())?
                    .fetch(locator, "bin")?
            } else {
                base.join(locator)
            };
            let id = format!("resource:{name}");
            if motion_component(&doc, name).is_some() {
                let clip = motion_component(&doc, name).unwrap();
                let mut deps = Vec::new();
                if let Some(bindings) = clip["resources"].as_object() {
                    for (control, resource) in bindings {
                        let key = resource
                            .as_str()
                            .ok_or_else(|| anyhow!("invalid Motion resource binding"))?;
                        let locator = locators
                            .get(key)
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("missing Motion resource {key}"))?;
                        if locator.contains("://") {
                            bail!("Motion resource {key}: use a local resource locator");
                        }
                        deps.push(super::motion_package::FixedResourceDependency {
                            role: control.clone(),
                            resource_id: format!("resource:{key}"),
                        });
                    }
                }
                let prepared = prepared_motions
                    .remove(name)
                    .ok_or_else(|| anyhow!("missing prepared Motion instance {name}"))?;
                let mut source_map = serde_json::to_value(&prepared.compiled.source_map)?;
                source_map["entry"] = serde_json::json!(path);
                let artifact = prepared.compiled.artifact;
                let compiled_json: serde_json::Value = serde_json::from_str(&canonical)?;
                let frame_rate: valle_timeline::FrameRate =
                    serde_json::from_value(compiled_json["document"]["canvas"]["fps"].clone())?;
                let total_frames = artifact
                    .composition
                    .as_ref()
                    .ok_or_else(|| anyhow!("Motion {name} needs composition.duration"))?
                    .duration_frames(frame_rate)?;
                for track in compiled_json["document"]["visual"]["tracks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    for item in track["items"].as_array().into_iter().flatten() {
                        if item["source"]["component"].as_str() == Some(&id) {
                            structures.push(serde_json::json!({
                                "clipId": item["id"],
                                "authoring": {"sourceMap": source_map, "totalFrames": total_frames}
                            }));
                        }
                    }
                }
                if let Some(bindings) = clip["resources"].as_object() {
                    for (control, resource) in bindings {
                        let key = resource
                            .as_str()
                            .ok_or_else(|| anyhow!("invalid Motion resource binding"))?;
                        let kind = artifact
                            .controls
                            .assets
                            .get(control)
                            .ok_or_else(|| anyhow!("unknown Motion asset control {control}"))?
                            .kind;
                        if kind == AssetKind::Environment {
                            motion_environments
                                .insert(key.to_owned(), prepared.assets[control].clone());
                        }
                        if motion_asset_kinds
                            .insert(key.to_owned(), kind)
                            .is_some_and(|old| old != kind)
                        {
                            bail!("resource {key} is used with conflicting Motion asset kinds");
                        }
                    }
                }
                let font_blobs = super::motion::fixed_package_font_blobs(&artifact, &[])?;
                for (i, bytes) in font_blobs.iter().enumerate() {
                    let font_id = resources.intern_font(bytes)?;
                    deps.push(super::motion_package::FixedResourceDependency {
                        role: super::motion_package::font_dependency_role(&artifact, bytes, i),
                        resource_id: font_id,
                    });
                    let digest = ContentDigest::of_bytes(bytes);
                    let path = frozen.path().join(digest.as_hex());
                    std::fs::write(&path, bytes)?;
                    blobs.insert(
                        digest.as_hex().to_owned(),
                        PreviewFile::File {
                            path,
                            _directory: Arc::clone(&frozen),
                        },
                    );
                    catalog.insert_bytes(ContentDigest::of_bytes(bytes), bytes.to_vec())?;
                }
                use valle_timeline::internal::wire::resource::*;
                let descriptor = MotionArtifactDescriptorWire {
                    reads_destination: artifact.reads_destination(),
                    boundary_sampling: ContinuousBoundarySamplingWire::LeftLimit,
                };
                let digest = ContentDigest::of_bytes(&valle_motion::canonical_bytes(&artifact)?);
                resources.add(
                    &id,
                    ResourceEntryWire::MotionArtifact {
                        digest,
                        abi: MotionArtifactAbiWire::Canonical,
                        descriptor: descriptor.clone(),
                    },
                    valle_engine::render::VerifiedResourceFacts::MotionArtifact {
                        abi: MotionArtifactAbiWire::Canonical,
                        descriptor,
                        artifact: Arc::new(artifact),
                        temporal_footprint: Default::default(),
                    },
                    deps,
                )?;
                continue;
            }
            let declared_kind =
                resource_kind(&doc, name).or_else(|| motion_asset_kinds.get(name).copied());
            // Media metadata/decoders consume paths, never a full in-memory file.
            let (bytes, hash, file) =
                if matches!(declared_kind, Some(AssetKind::Video | AssetKind::Audio)) {
                    let (hash, file) = media.get(&path)?;
                    (Vec::new(), hash, file)
                } else {
                    let asset = if let Some(asset) = motion_environments.remove(name) {
                        asset
                    } else {
                        super::motion::load_asset(&path).with_context(|| {
                            format!("reading resource {name}: {}", path.display())
                        })?
                    };
                    let path = frozen.path().join(asset.hash.as_hex());
                    std::fs::write(&path, &asset.bytes)?;
                    (
                        asset.bytes.to_vec(),
                        asset.hash,
                        PreviewFile::File {
                            path,
                            _directory: Arc::clone(&frozen),
                        },
                    )
                };
            // Identical bytes share one owner/path, including in the native catalog.
            let file = blobs.entry(hash.as_hex().to_owned()).or_insert(file);
            let path = match file {
                PreviewFile::File { path, .. } => path.clone(),
                PreviewFile::Bytes(_) => unreachable!(),
            };
            let id = format!("resource:{name}");
            if has_visual_source(&doc["tracks"], name, "lottie") {
                let value: serde_json::Value = serde_json::from_slice(&bytes)?;
                if value["assets"].as_array().is_some_and(|assets| {
                    assets
                        .iter()
                        .any(|a| a["p"].as_str().is_some_and(|p| !p.starts_with("data:")))
                }) || value["fonts"]["list"]
                    .as_array()
                    .is_some_and(|fonts| !fonts.is_empty())
                {
                    bail!(
                        "Lottie {name}: external images and fonts must be embedded or converted to shapes before rendering"
                    );
                }
                use valle_timeline::internal::wire::resource::*;
                use valle_timeline::{
                    RationalTime, time::ExactRational, wire::timeline::TimelineTimeWire,
                };
                let number = |key: &str| -> Result<ExactRational> {
                    Ok(TimelineTimeWire::new(value[key].to_string())?.to_exact())
                };
                let fps = number("fr")?;
                if !fps.is_positive() {
                    bail!("Lottie frame rate must be positive");
                }
                let descriptor = LottieResourceDescriptorWire {
                    duration: RationalTime::from_exact(
                        number("op")?.checked_sub(number("ip")?)?.checked_div(fps)?,
                    ),
                    time_base: ExactRational::ONE.checked_div(fps)?,
                    width: value["w"]
                        .as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or_else(|| anyhow!("invalid Lottie width"))?,
                    height: value["h"]
                        .as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or_else(|| anyhow!("invalid Lottie height"))?,
                    boundary_sampling: ContinuousBoundarySamplingWire::LeftLimit,
                };
                resources.add(
                    &id,
                    ResourceEntryWire::Lottie {
                        digest: hash,
                        abi: LottieArtifactAbiWire::Canonical,
                        descriptor: descriptor.clone(),
                    },
                    valle_engine::render::VerifiedResourceFacts::Lottie {
                        abi: LottieArtifactAbiWire::Canonical,
                        descriptor,
                        temporal_footprint: Default::default(),
                    },
                    Vec::new(),
                )?;
                if native {
                    catalog.insert_file(hash, path)?;
                }
                continue;
            }
            let kind = declared_kind
                .or_else(|| {
                    if ttf_parser::Face::parse(&bytes, 0).is_ok() {
                        Some(AssetKind::Font)
                    } else if valle_render::host::probe_image_extent(&bytes).is_ok() {
                        Some(AssetKind::Image)
                    } else {
                        None
                    }
                })
                .ok_or_else(|| anyhow!("resource {name} has no supported Timeline consumer"))?;
            let asset = super::motion::BoundAsset {
                path: path.clone(),
                bytes: bytes.into(),
                hash,
            };
            if kind == AssetKind::Video {
                add_video_resource(
                    &mut resources,
                    &id,
                    hash,
                    &path,
                    video_audio_used(&doc, name),
                )?;
            } else {
                resources.add_asset(&id, kind, &asset)?;
            }
            if native {
                catalog.insert_file(hash, path)?;
            }
        }
    }
    let capabilities = resources.capabilities();
    let bindings = canonical_verified_binding_bundle(&resources.domain_bindings, &capabilities)
        .map_err(|e| anyhow!(e))?;
    let manifest = ResourceManifest::try_from_wire(ResourceManifestEnvelopeWire {
        entries: resources.entries,
    })?;
    let manifest = std::str::from_utf8(manifest.canonical_bytes())?;
    let profile = canonical_fixed_execution_profile(COMMON_PROFILE_KEY).map_err(|e| anyhow!(e))?;
    let files = fixed_package_files(&canonical, manifest, &bindings, &profile);
    let package = canonical_fixed_package_manifest(&files).map_err(|e| anyhow!(e))?;
    open_verified_fixed_package(&package, &files).map_err(|e| anyhow!(e))?;
    captured_motions.verify_dependencies()?;
    Ok(TimelinePreviewPackage {
        timeline_json,
        canonical_timeline_json: canonical,
        fixed_package_manifest_json: package,
        resource_manifest_json: manifest.to_owned(),
        verified_binding_bundle_json: bindings,
        motion: serde_json::json!({"structures": structures, "problems": [], "shaders": []}),
        blobs,
        input_dependencies,
        catalog,
    })
}

fn video_audio_used(doc: &serde_json::Value, name: &str) -> bool {
    fn scan(value: &serde_json::Value, name: &str) -> bool {
        match value {
            serde_json::Value::Object(map) => {
                if map.get("src").and_then(|v| v.as_str()) == Some(name) {
                    return map.get("gain").and_then(|v| v.as_f64()) != Some(0.0);
                }
                map.values().any(|value| scan(value, name))
            }
            serde_json::Value::Array(values) => values.iter().any(|value| scan(value, name)),
            _ => false,
        }
    }
    // Motion controls may consume the video dynamically; preserve their dependencies.
    scan(&doc["tracks"], name) || has_motion_resource(&doc["tracks"], name)
}

fn has_motion_resource(value: &serde_json::Value, name: &str) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            map.get("resources")
                .and_then(|v| v.as_object())
                .is_some_and(|bindings| bindings.values().any(|v| v.as_str() == Some(name)))
                || map.values().any(|value| has_motion_resource(value, name))
        }
        serde_json::Value::Array(values) => {
            values.iter().any(|value| has_motion_resource(value, name))
        }
        _ => false,
    }
}

fn resource_kind(doc: &serde_json::Value, name: &str) -> Option<AssetKind> {
    fn scan(v: &serde_json::Value, name: &str, audio: bool) -> Option<AssetKind> {
        match v {
            serde_json::Value::Object(map) => {
                if map.get("font").and_then(|x| x.as_str()) == Some(name) {
                    return Some(AssetKind::Font);
                }
                if map.get("src").and_then(|x| x.as_str()) == Some(name) {
                    return match map.get("kind").and_then(|x| x.as_str()) {
                        Some("image") => Some(AssetKind::Image),
                        Some("video") => Some(AssetKind::Video),
                        _ if audio => Some(AssetKind::Audio),
                        _ => None,
                    };
                }
                resource_fields(map).find_map(|(k, v)| scan(v, name, audio || k == "audio"))
            }
            serde_json::Value::Array(a) => a.iter().find_map(|v| scan(v, name, audio)),
            _ => None,
        }
    }
    scan(&doc["tracks"], name, false)
}

fn motion_component<'a>(doc: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
    match doc {
        serde_json::Value::Object(map) => {
            if map.get("kind").and_then(|v| v.as_str()) == Some("motion")
                && map.get("component").and_then(|v| v.as_str()) == Some(name)
            {
                Some(doc)
            } else {
                resource_fields(map).find_map(|(_, v)| motion_component(v, name))
            }
        }
        serde_json::Value::Array(a) => a.iter().find_map(|v| motion_component(v, name)),
        _ => None,
    }
}

fn has_visual_source(value: &serde_json::Value, name: &str, kind: &str) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            (map.get("kind").and_then(|v| v.as_str()) == Some(kind)
                && map.get("src").and_then(|v| v.as_str()) == Some(name))
                || resource_fields(map).any(|(_, v)| has_visual_source(v, name, kind))
        }
        serde_json::Value::Array(values) => values.iter().any(|v| has_visual_source(v, name, kind)),
        _ => false,
    }
}

/// Compilation captures bound resource facts; vary the artifact by its actual preparation inputs.
pub(crate) fn prepare_motion_instances(doc: &mut serde_json::Value) -> Result<()> {
    let mut locators = doc["resources"].as_object().cloned().unwrap_or_default();
    let original = locators.clone();
    if let Some(tracks) = doc["tracks"]["visual"].as_array_mut() {
        for track in tracks {
            if let Some(clips) = track["clips"].as_array_mut() {
                for clip in clips {
                    if clip["kind"] != "motion" {
                        continue;
                    }
                    let name = clip["component"]
                        .as_str()
                        .ok_or_else(|| anyhow!("missing Motion component"))?;
                    let locator = original
                        .get(name)
                        .ok_or_else(|| anyhow!("missing Motion component {name}"))?;
                    let resources = clip
                        .get("resources")
                        .filter(|value| value.as_object().is_some_and(|map| !map.is_empty()))
                        .unwrap_or(&serde_json::Value::Null)
                        .clone();
                    let data = clip
                        .get("data")
                        .filter(|value| value.as_object().is_some_and(|map| !map.is_empty()))
                        .unwrap_or(&serde_json::Value::Null)
                        .clone();
                    let key = valle_compiler::motion_instance_key(
                        locator
                            .as_str()
                            .ok_or_else(|| anyhow!("invalid Motion locator"))?,
                        &resources,
                        &data,
                    );
                    if original.contains_key(&key) {
                        bail!("resource alias {key} collides with a prepared Motion instance");
                    }
                    locators.insert(key.clone(), locator.clone());
                    clip["component"] = key.into();
                }
            }
        }
    }
    doc["resources"] = locators.into();
    Ok(())
}

/// Prepare each bound Motion instance once and carry its validated composition into normalization.
pub(crate) struct CapturedMotionSources {
    instances: std::collections::BTreeMap<String, super::motion::CapturedTimelineComponent>,
}

impl CapturedMotionSources {
    pub(crate) fn dependency_digests(
        &self,
    ) -> Result<std::collections::BTreeMap<String, ContentDigest>> {
        let mut digests = std::collections::BTreeMap::new();
        for instance in self.instances.values() {
            for (path, digest) in instance.dependency_digests() {
                let key = path.canonicalize()?.to_string_lossy().into_owned();
                if digests
                    .insert(key.clone(), digest)
                    .is_some_and(|old| old != digest)
                {
                    bail!("Motion dependency changed during capture: {key}");
                }
            }
        }
        Ok(digests)
    }

    pub(crate) fn verify_expected_dependencies(
        &self,
        expected: Option<&std::collections::BTreeMap<String, ContentDigest>>,
    ) -> Result<()> {
        let actual = self.dependency_digests()?;
        if actual.is_empty() {
            return Ok(());
        }
        let expected = expected.ok_or_else(|| {
            anyhow!("Motion dependency digests are required; reload the Studio inputs")
        })?;
        for (path, digest) in actual {
            match expected.get(&path) {
                Some(confirmed) if *confirmed == digest => {}
                Some(_) => bail!("Motion dependency changed since it was loaded: {path}"),
                None => bail!("Motion dependency was not confirmed by Studio: {path}"),
            }
        }
        Ok(())
    }

    pub(crate) fn verify_dependencies(&self) -> Result<()> {
        for instance in self.instances.values() {
            instance.verify_dependencies()?;
        }
        Ok(())
    }

    pub(crate) fn prepare(
        &self,
    ) -> Result<(
        std::collections::BTreeMap<String, super::motion::PreparedInput>,
        std::collections::BTreeMap<String, valle_timeline::RationalTime>,
    )> {
        self.verify_dependencies()?;
        let mut prepared = std::collections::BTreeMap::new();
        let mut durations = std::collections::BTreeMap::new();
        for (name, instance) in &self.instances {
            let input = super::motion::compile_captured_timeline_component(instance)?;
            let composition = input
                .compiled
                .artifact
                .composition
                .as_ref()
                .ok_or_else(|| anyhow!("Motion {name} needs composition.duration"))?;
            durations.insert(name.clone(), composition.duration()?);
            prepared.insert(name.clone(), input);
        }
        self.verify_dependencies()?;
        Ok((prepared, durations))
    }
}

pub(crate) fn capture_motion_sources(
    doc: &serde_json::Value,
    base: &Path,
) -> Result<CapturedMotionSources> {
    let locators = doc["resources"].as_object().cloned().unwrap_or_default();
    let mut instances = std::collections::BTreeMap::new();
    for track in doc["tracks"]["visual"].as_array().into_iter().flatten() {
        for clip in track["clips"].as_array().into_iter().flatten() {
            if clip["kind"] != "motion" {
                continue;
            }
            let name = clip["component"]
                .as_str()
                .ok_or_else(|| anyhow!("missing Motion component"))?;
            if instances.contains_key(name) {
                continue;
            }
            let locator = locators
                .get(name)
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow!("missing Motion resource {name}"))?;
            let mut assets = Vec::new();
            for (control, alias) in clip["resources"].as_object().into_iter().flatten() {
                let alias = alias
                    .as_str()
                    .ok_or_else(|| anyhow!("invalid Motion asset alias"))?;
                let asset = locators
                    .get(alias)
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| anyhow!("missing Motion resource {alias}"))?;
                assets.push(format!("{control}={}", base.join(asset).display()));
            }
            let input = super::motion::capture_timeline_component(
                &base.join(locator),
                &assets,
                clip.get("data"),
            )?;
            instances.insert(name.to_owned(), input);
        }
    }
    Ok(CapturedMotionSources { instances })
}

/// Files in the current author's Motion module closure. Keep the entry available
/// even when its imports cannot be resolved, so a broken source remains editable.
pub(crate) fn motion_source_paths(
    timeline: &Timeline,
    base: &Path,
) -> Result<std::collections::BTreeSet<std::path::PathBuf>> {
    let document: serde_json::Value = serde_json::from_slice(&timeline_bytes(timeline)?)?;
    let locators = document["resources"].as_object();
    let mut paths = std::collections::BTreeSet::new();
    for track in document["tracks"]["visual"]
        .as_array()
        .into_iter()
        .flatten()
    {
        for clip in track["clips"].as_array().into_iter().flatten() {
            if clip["kind"] != "motion" {
                continue;
            }
            let name = clip["component"]
                .as_str()
                .ok_or_else(|| anyhow!("missing Motion component"))?;
            let locator = locators
                .and_then(|locators| locators.get(name))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow!("missing Motion resource {name}"))?;
            if locator.contains("://") {
                bail!("Motion source {name} must be a local file");
            }
            let entry = base.join(locator);
            paths
                .extend(super::motion::motion_module_paths(&entry).unwrap_or_else(|_| vec![entry]));
        }
    }
    Ok(paths)
}

pub(crate) fn motion_source_durations(
    canonical: &serde_json::Value,
    author: &serde_json::Value,
) -> Result<std::collections::BTreeMap<String, f64>> {
    let mut durations = std::collections::BTreeMap::new();
    for track in canonical["document"]["visual"]["tracks"]
        .as_array()
        .into_iter()
        .flatten()
    {
        for item in track["items"].as_array().into_iter().flatten() {
            let source = &item["source"];
            if source["type"] != "motion" {
                continue;
            }
            let alias = source["component"]
                .as_str()
                .and_then(|id| id.strip_prefix("resource:"))
                .ok_or_else(|| anyhow!("invalid prepared Motion component"))?;
            let duration: valle_timeline::RationalTime =
                serde_json::from_value(source["sourceDuration"].clone())?;
            durations.insert(alias.to_owned(), duration.as_f64());
        }
    }
    for (track_index, track) in author["tracks"]["visual"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        for (clip_index, clip) in track["clips"].as_array().into_iter().flatten().enumerate() {
            if clip["kind"] != "motion" {
                continue;
            }
            let id = format!("timeline:visual-track:{track_index}:clip:{clip_index}");
            let source = canonical["document"]["visual"]["tracks"][track_index]["items"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|item| item["id"] == id)
                .ok_or_else(|| anyhow!("prepared Motion clip {id} is missing"))?;
            let duration: valle_timeline::RationalTime =
                serde_json::from_value(source["source"]["sourceDuration"].clone())?;
            if let Some(alias) = clip["component"].as_str() {
                durations.insert(alias.to_owned(), duration.as_f64());
            }
        }
    }
    Ok(durations)
}

fn collect_used_resources(
    value: &serde_json::Value,
    used: &mut std::collections::BTreeSet<String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for field in ["src", "component", "font"] {
                if let Some(name) = map.get(field).and_then(|v| v.as_str()) {
                    used.insert(name.to_owned());
                }
            }
            if map.get("kind").and_then(|v| v.as_str()) == Some("motion") {
                if let Some(bindings) = map.get("resources").and_then(|v| v.as_object()) {
                    used.extend(
                        bindings
                            .values()
                            .filter_map(|v| v.as_str().map(str::to_owned)),
                    );
                }
            }
            for (_, child) in resource_fields(map) {
                collect_used_resources(child, used);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                collect_used_resources(child, used);
            }
        }
        _ => {}
    }
}

/// Data and props are authored values, not Timeline resource declarations.
fn resource_fields(
    map: &serde_json::Map<String, serde_json::Value>,
) -> impl Iterator<Item = (&str, &serde_json::Value)> {
    map.iter()
        .filter(|(key, _)| !matches!(key.as_str(), "data" | "props"))
        .map(|(key, value)| (key.as_str(), value))
}

#[cfg(test)]
mod motion_source_metadata_tests {
    use super::*;

    #[test]
    fn prepared_timeline_projects_composition_duration() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("phase.motion.tsx"),
            "export const composition = { width: 64, height: 64, fps: 24, duration: 4.6 };\nexport default function Phase() { return <Scene />; }",
        )
        .unwrap();
        let timeline = serde_json::json!({
            "canvas": {"width": 64, "height": 64, "fps": 24},
            "resources": {"phase": "phase.motion.tsx"},
            "tracks": {"visual": [{"clips": [{
                "kind": "motion", "component": "phase", "start": 0, "duration": 4.6,
                "data": {}, "resources": {}
            }]}]}
        });
        let timeline = decode_timeline(&timeline.to_string()).unwrap();
        let prepared = prepare_timeline_package(timeline, dir.path()).unwrap();
        let authoring = &prepared.motion["structures"][0]["authoring"];
        assert_eq!(authoring["totalFrames"], 110);
        assert!(authoring.get("timing").is_none());
        let canonical: serde_json::Value =
            serde_json::from_str(&prepared.canonical_timeline_json).unwrap();
        assert_eq!(
            canonical["document"]["visual"]["tracks"][0]["items"][0]["source"]["sourceDuration"],
            "23/5"
        );
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;

    #[test]
    fn media_facts_for_motion_asset_do_not_compile_the_component() {
        let dir = tempfile::tempdir().unwrap();
        let image =
            include_bytes!("../../../valle-compiler/tests/fixtures/motion/modules/assets/dot.png");
        std::fs::write(dir.path().join("poster.png"), image).unwrap();
        // The component does not exist. The browser Worker owns its compilation.
        let timeline = serde_json::json!({
            "canvas": {"width": 64, "height": 64, "fps": 24},
            "resources": {"component": "missing.motion.tsx", "poster": "poster.png"},
            "tracks": {"visual": [{"clips": [{
                "kind": "motion", "component": "component", "start": 0, "duration": 1,
                "resources": {"image": "poster"}
            }]}]}
        });
        let mut cache = FrozenMediaCache::default();
        let (facts, blobs, input_dependencies) = prepare_timeline_media_facts(
            decode_timeline(&timeline.to_string()).unwrap(),
            dir.path(),
            &mut cache,
        )
        .unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0]["id"], "resource:poster");
        assert_eq!(facts[0]["entry"]["kind"], "image");
        assert!(blobs.contains_key(&ContentDigest::of_bytes(image).as_hex()));
        assert_eq!(
            input_dependencies[&dir
                .path()
                .join("poster.png")
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .to_string()],
            ContentDigest::of_bytes(image),
        );
    }

    #[test]
    fn duplicate_media_keeps_the_native_catalog_file_alive() {
        let dir = tempfile::tempdir().unwrap();
        // One second of mono PCM16 silence, authored under two resource paths.
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&96_036_u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&48_000_u32.to_le_bytes());
        wav.extend_from_slice(&96_000_u32.to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&96_000_u32.to_le_bytes());
        wav.resize(96_044, 0);
        std::fs::write(dir.path().join("a.wav"), &wav).unwrap();
        std::fs::write(dir.path().join("b.wav"), &wav).unwrap();
        let timeline = serde_json::json!({
            "canvas": {"width": 64, "height": 64, "fps": 24},
            "resources": {"a": "a.wav", "b": "b.wav"},
            "tracks": {"audio": [{"clips": [
                {"src": "a", "start": 0, "duration": 1},
                {"src": "b", "start": 1, "duration": 1}
            ]}]}
        });
        let prepared =
            prepare_timeline_package(decode_timeline(&timeline.to_string()).unwrap(), dir.path())
                .unwrap();
        let digest = ContentDigest::of_bytes(&wav);
        assert_eq!(prepared.blobs.len(), 1);
        let Some(valle_render::host::NativeResourceSource::File(path)) =
            prepared.catalog.source(&digest)
        else {
            panic!("missing native media source");
        };
        assert_eq!(std::fs::read(path).unwrap(), wav);
    }

    #[test]
    fn muted_video_dependencies_are_pruned_only_when_all_uses_are_silent() {
        let mut doc = serde_json::json!({"tracks":{"visual":[{"clips":[
            {"kind":"video","src":"camera","gain":0},
            {"kind":"video","src":"camera","gain":0}
        ]}]}});
        assert!(!video_audio_used(&doc, "camera"));
        doc["tracks"]["visual"][0]["clips"][1]["gain"] = serde_json::json!(1);
        assert!(video_audio_used(&doc, "camera"));
        doc["tracks"]["visual"][0]["clips"][1] =
            serde_json::json!({"kind":"motion","resources":{"video":"camera"}});
        assert!(video_audio_used(&doc, "camera"));
    }
}
