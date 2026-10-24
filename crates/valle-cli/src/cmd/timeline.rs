//! File and project Timeline delivery through the fixed-package admission path.
use super::render_delivery::{DeliveryObservation, DeliveryPlan};
use crate::{
    RenderBudgetArgs, RenderOutputArgs, TimelineAction,
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
            let mut warnings = Vec::new();
            render_document_impl(
                timeline,
                input.parent().unwrap_or(Path::new(".")),
                RenderOutputArgs {
                    output: Some(temp.path().join("check.png")),
                    frame: Some(0),
                    ..Default::default()
                },
                Some(&mut warnings),
                RenderBudgetArgs::default(),
            )?;
            for warning in &warnings {
                eprintln!(
                    "{}: [warning:{:?}] {}",
                    input.display(),
                    warning.code,
                    warning.message
                );
            }
            crate::output::emit(
                serde_json::json!({"status":"ok", "input":input, "warnings":warnings}),
            );
            Ok(std::process::ExitCode::SUCCESS)
        }
        TimelineAction::Render {
            input,
            delivery,
            budget,
        } => {
            let timeline = decode_timeline(&super::read(&input)?)?;
            render_document_impl(
                timeline,
                input.parent().unwrap_or(Path::new(".")),
                delivery,
                None,
                budget,
            )
        }
    }
}

/// Render a Timeline document, or with `check` prepare it to render and collect what its Motion
/// hosts' clocks leave out.
fn render_document_impl(
    timeline: Timeline,
    base: &Path,
    delivery: RenderOutputArgs,
    check: Option<&mut Vec<valle_motion::MotionDiagnostic>>,
    budget: RenderBudgetArgs,
) -> Result<std::process::ExitCode> {
    let plan = DeliveryPlan::new(delivery)?;
    let observation = DeliveryObservation::new()?;
    let prepared = prepare_timeline_package(timeline, base)?;
    // Package preparation already admitted and opened the closed package; reuse it.
    let opened = prepared.opened;
    let catalog = prepared.catalog;
    let renderer = NativeRenderer::new(
        NativeProject::from_render(opened.engine_render(), Arc::new(catalog)),
        NativeRenderOptions {
            backend: valle_render::executor::skia::SkiaBackendKind::Raster,
            budget: budget.native()?,
            background: valle_engine::resource::OutputBackground::AuthorSrgbStraight {
                color: valle_engine::resource::AuthorSrgbStraight(
                    opened.engine_render().canvas().background_rgba(),
                ),
            },
            control: observation
                .performance
                .as_ref()
                .map(|(trace, _)| {
                    valle_render::host::RenderControl::default().with_trace(trace.clone())
                })
                .unwrap_or_default(),
            progress: crate::output::render_progress(),
            ..NativeRenderOptions::default()
        },
    );
    if let Some(warnings) = check {
        warnings.extend(clock_warnings(opened.compiled())?);
        return Ok(std::process::ExitCode::SUCCESS);
    }
    plan.deliver_and_report(&renderer, &opened, &observation, None)?;
    Ok(std::process::ExitCode::SUCCESS)
}

/// What the clocks of the Motion hosts leave out: an end no frame shows, and onsets shorter than
/// a frame of output. A warning names the visual clip that places the source by its path in the
/// document, such as `/tracks/visual/0/clips/1`.
pub(super) fn clock_warnings(
    render: &valle_engine::render::CompiledRender,
) -> Result<Vec<valle_motion::MotionDiagnostic>> {
    use valle_engine::render::CompiledVisualItem;
    let mut clips = std::collections::BTreeMap::new();
    for track in render.visual().tracks() {
        for item in track.items() {
            if let CompiledVisualItem::Clip { clip } = item {
                clips.insert(clip.source_index(), document_path(clip.id()));
            }
        }
    }
    let mut warnings = Vec::new();
    for index in 0..u32::try_from(render.sources().len())? {
        let Some(motion) = render
            .sources()
            .source(index)
            .and_then(valle_engine::render::CompiledSource::motion)
        else {
            continue;
        };
        let path = clips.get(&index).cloned().unwrap_or_default();
        for (code, message) in
            super::motion::clock_warnings(&motion.clock(), motion.artifact(), true)?
        {
            warnings.push(valle_motion::MotionDiagnostic::new(
                code,
                path.clone(),
                message,
            ));
        }
    }
    Ok(warnings)
}

/// The path in the document of a prepared clip, `timeline:visual-track:0:clip:1` as
/// `/tracks/visual/0/clips/1`.
fn document_path(id: &str) -> String {
    id.strip_prefix("timeline:")
        .and_then(|id| id.split_once("-track:"))
        .and_then(|(band, id)| {
            let (track, clip) = id.split_once(":clip:")?;
            Some(format!("/tracks/{band}/{track}/clips/{clip}"))
        })
        .unwrap_or_else(|| id.to_owned())
}

/// One frozen preparation shared by Native delivery and browser previews.
pub(crate) struct TimelinePreviewPackage {
    pub snapshot_inputs: Vec<valle_project::folder::CapturedInput>,
    pub timeline_json: String,
    pub canonical_timeline_json: String,
    pub fixed_package_manifest_json: String,
    pub resource_manifest_json: String,
    pub verified_binding_bundle_json: String,
    pub motion: serde_json::Value,
    pub blobs: std::collections::BTreeMap<String, PreviewFile>,
    pub input_dependencies: std::collections::BTreeMap<String, ContentDigest>,
    pub(super) catalog: NativeResourceCatalog,
    /// The package opened by the closed-package admission check, reused by native renders.
    pub(super) opened: OpenedFixedPackage,
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
    for track in doc
        .pointer_mut("/tracks/visual")
        .and_then(serde_json::Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if let Some(clips) = track["clips"].as_array_mut() {
            for clip in clips {
                // Effect templates are Motion too; only the assets they bind are media facts.
                if let Some(effects) = clip.as_object_mut().and_then(|clip| clip.remove("effects"))
                {
                    for effect in effects.as_array().into_iter().flatten() {
                        motion_assets.extend(
                            effect["resources"]
                                .as_object()
                                .into_iter()
                                .flatten()
                                .filter_map(|(_, value)| value.as_str().map(str::to_owned)),
                        );
                    }
                }
                if clip["kind"] != "motion" {
                    continue;
                }
                if let Some(bindings) = clip["resources"].as_object() {
                    motion_assets.extend(
                        bindings
                            .values()
                            .filter_map(|value| value.as_str().map(str::to_owned)),
                    );
                }
                // Preserve endpoint indices and overlap clocks while only collecting media.
                *clip = serde_json::json!({
                    "kind": "solid", "color": "#00000000",
                    "start": clip["start"], "duration": clip["duration"],
                });
            }
        }
        for transition in track
            .get_mut("transitions")
            .and_then(serde_json::Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if transition["kind"] != "motion" {
                continue;
            }
            if let Some(bindings) = transition["resources"].as_object() {
                motion_assets.extend(
                    bindings
                        .values()
                        .filter_map(|value| value.as_str().map(str::to_owned)),
                );
            }
            *transition = serde_json::json!({
                "from": transition["from"], "to": transition["to"], "kind": "fade",
            });
        }
    }
    for track in doc
        .pointer_mut("/tracks/adjustment")
        .and_then(serde_json::Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        for clip in track["clips"].as_array_mut().into_iter().flatten() {
            if clip["kind"] != "motion" {
                continue;
            }
            if let Some(bindings) = clip["resources"].as_object() {
                motion_assets.extend(
                    bindings
                        .values()
                        .filter_map(|value| value.as_str().map(str::to_owned)),
                );
            }
            // A neutral grade keeps the adjustment's interval in the export's duration.
            *clip = serde_json::json!({
                "kind": "color-grade", "temperature": 0,
                "start": clip["start"], "duration": clip["duration"],
            });
        }
    }
    for track in doc
        .pointer_mut("/tracks/caption")
        .and_then(serde_json::Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if let Some(presenter) = track
            .as_object_mut()
            .and_then(|track| track.remove("presenter"))
        {
            // The presenter binds this font implicitly, so Studio must confirm its
            // digest just like an explicitly bound Motion asset before saving.
            if let Some(font) = track["style"]["font"].as_str() {
                motion_assets.insert(font.to_owned());
            }
            motion_assets.extend(
                presenter["resources"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter_map(|(_, v)| v.as_str().map(str::to_owned)),
            );
            // This copy collects media facts through the built-in text path. Word timing
            // belongs to the original presenter input and does not affect font facts.
            for clip in track["clips"].as_array_mut().into_iter().flatten() {
                for run in clip
                    .get_mut("runs")
                    .and_then(serde_json::Value::as_array_mut)
                    .into_iter()
                    .flatten()
                {
                    if let Some(run) = run.as_object_mut() {
                        run.remove("start");
                        run.remove("end");
                    }
                }
            }
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
    let motion_inputs: std::collections::BTreeMap<_, _> =
        valle_compiler::motion_preparation_inputs(&timeline)?
            .into_iter()
            .map(|input| (input.key(), input))
            .collect();
    prepare_motion_instances(&mut doc)?;
    let captured_motions = capture_motion_sources(&doc, base)?;
    let mut snapshot_inputs = captured_motions.snapshot_inputs();
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
        for input in motion_inputs.values() {
            used.remove(&input.component);
        }
        for (key, input) in &motion_inputs {
            used.insert(key.clone());
            used.extend(input.resources.values().cloned());
        }
        let mut ordered: Vec<_> = locators
            .iter()
            .filter(|(name, _)| used.contains(name.as_str()))
            .collect();
        ordered.sort_by_key(|(name, _)| !motion_inputs.contains_key(name.as_str()));
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
            if let Some(input) = motion_inputs.get(name) {
                let mut deps = Vec::new();
                {
                    for (control, key) in &input.resources {
                        let key = key.as_str();
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
                // A caption template draws the text of every caption bound to it.
                let mut caption_text = String::new();
                for band in ["visual", "captions"] {
                    for track in compiled_json["document"][band]["tracks"]
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
                                for run in item["runs"].as_array().into_iter().flatten() {
                                    caption_text.push_str(run["text"].as_str().unwrap_or_default());
                                }
                            }
                        }
                    }
                }
                {
                    for (control, key) in &input.resources {
                        let key = key.as_str();
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
                let font_blobs = super::motion::fixed_package_font_blobs(
                    &artifact,
                    &[],
                    (!caption_text.is_empty()).then_some(caption_text.as_str()),
                )?;
                for (i, bytes) in font_blobs.iter().enumerate() {
                    let digest = resources.intern_font(bytes)?;
                    deps.push(super::motion_package::FixedResourceDependency {
                        role: super::motion_package::font_dependency_role(&artifact, bytes, i),
                        resource_id: super::motion_package::font_resource_id(&digest),
                    });
                    catalog.insert_bytes(digest, bytes.to_vec());
                    let path = frozen.path().join(digest.as_hex());
                    std::fs::write(&path, bytes)?;
                    blobs.insert(
                        digest.as_hex().to_owned(),
                        PreviewFile::File {
                            path,
                            _directory: Arc::clone(&frozen),
                        },
                    );
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
                    let mut captured =
                        valle_project::folder::CapturedInput::media(path.clone(), hash);
                    captured.stamp = media.captured_stamp(&path, hash);
                    snapshot_inputs.push(captured);
                    (Vec::new(), hash, file)
                } else {
                    let asset = if let Some(asset) = motion_environments.remove(name) {
                        asset
                    } else {
                        let (asset, inputs) =
                            super::motion::capture_asset(&path).with_context(|| {
                                format!("reading resource {name}: {}", path.display())
                            })?;
                        snapshot_inputs.extend(inputs);
                        asset
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
                catalog.insert_file(hash, path);
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
    let (package, opened) = build_fixed_package(&files).map_err(|e| anyhow!(e))?;
    captured_motions.verify_dependencies()?;
    Ok(TimelinePreviewPackage {
        snapshot_inputs,
        timeline_json,
        canonical_timeline_json: canonical,
        fixed_package_manifest_json: package,
        resource_manifest_json: manifest.to_owned(),
        verified_binding_bundle_json: bindings,
        motion: serde_json::json!({"structures": structures, "problems": [], "shaders": []}),
        blobs,
        input_dependencies,
        catalog,
        opened,
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

/// Compilation captures bound resource facts; vary the artifact by its actual preparation inputs.
pub(crate) fn prepare_motion_instances(doc: &mut serde_json::Value) -> Result<()> {
    let inputs = valle_compiler::motion_preparation_inputs(&decode_timeline(&doc.to_string())?)?;
    let mut locators = doc["resources"].as_object().cloned().unwrap_or_default();
    let original = locators.clone();
    for input in inputs {
        let key = input.key();
        if original.contains_key(&key) {
            bail!("resource alias {key} collides with a prepared Motion instance");
        }
        locators.insert(key, input.locator.into());
    }
    doc["resources"] = locators.into();
    Ok(())
}

/// Prepare each bound Motion instance once and carry its validated composition into normalization.
pub(crate) struct CapturedMotionSources {
    instances: std::collections::BTreeMap<String, super::motion::CapturedTimelineComponent>,
}

impl CapturedMotionSources {
    pub(crate) fn snapshot_inputs(&self) -> Vec<valle_project::folder::CapturedInput> {
        self.instances
            .values()
            .flat_map(|instance| instance.snapshot_inputs())
            .collect()
    }
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
        std::collections::BTreeMap<String, valle_timeline::MotionSourceMetadata>,
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
            durations.insert(
                name.clone(),
                valle_timeline::MotionSourceMetadata {
                    duration: composition.duration()?,
                    role: input.compiled.artifact.role,
                },
            );
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
    let timeline = decode_timeline(&doc.to_string())?;
    let wire = timeline.clone().into_wire();
    let locators = &wire.resources;
    let mut instances = std::collections::BTreeMap::new();
    for input in valle_compiler::motion_preparation_inputs(&timeline)? {
        let key = input.key();
        if instances.contains_key(&key) {
            continue;
        }
        let mut assets = Vec::new();
        for (control, alias) in &input.resources {
            let asset = locators
                .get(alias)
                .ok_or_else(|| anyhow!("missing Motion resource {alias}"))?;
            assets.push(format!("{control}={}", base.join(asset).display()));
        }
        let mut captured = super::motion::capture_timeline_component(
            &base.join(&input.locator),
            &assets,
            (!input.data.is_null()).then_some(&input.data),
        )?;
        captured.placement = Some(input.placement());
        instances.insert(key, captured);
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
    let tracks = |band: &'static str| document["tracks"][band].as_array().into_iter().flatten();
    let visual = tracks("visual")
        .flat_map(|track| {
            ["clips", "transitions"]
                .into_iter()
                .flat_map(|items| track[items].as_array().into_iter().flatten())
        })
        .flat_map(|item| {
            let effects = item["effects"].as_array().into_iter().flatten();
            std::iter::once(item)
                .filter(|item| item["kind"] == "motion")
                .chain(effects)
        })
        .chain(
            tracks("adjustment")
                .flat_map(|track| track["clips"].as_array().into_iter().flatten())
                .filter(|clip| clip["kind"] == "motion"),
        )
        .filter_map(|item| item["component"].as_str());
    let caption = document["tracks"]["caption"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|track| track["presenter"]["component"].as_str());
    let declared = locators
        .into_iter()
        .flat_map(|resources| resources.iter())
        .filter(|(_, locator)| {
            locator.as_str().is_some_and(|locator| {
                let extension = Path::new(locator).extension().and_then(|e| e.to_str());
                matches!(extension, Some("tsx" | "jsx"))
            })
        })
        .map(|(alias, _)| alias.as_str());
    for name in visual.chain(caption).chain(declared) {
        let locator = locators
            .and_then(|l| l.get(name))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("missing Motion resource {name}"))?;
        if locator.contains("://") {
            bail!("Motion source {name} must be a local file");
        }
        let entry = base.join(locator);
        paths.extend(super::motion::motion_module_paths(&entry).unwrap_or_else(|_| vec![entry]));
    }
    Ok(paths)
}

pub(crate) fn motion_source_metadata(
    canonical: &serde_json::Value,
    author: &serde_json::Value,
) -> Result<std::collections::BTreeMap<String, valle_timeline::MotionSourceMetadata>> {
    let mut durations = std::collections::BTreeMap::new();
    for band in ["visual", "captions"] {
        for track in canonical["document"][band]["tracks"]
            .as_array()
            .into_iter()
            .flatten()
        {
            for item in track["items"].as_array().into_iter().flatten() {
                let (source, motion) = if item["type"] == "transition" {
                    // A Motion transition's template is its kernel's instance.
                    let source = &item["kernel"]["motion"];
                    (source, source.is_object())
                } else {
                    let source = &item["source"];
                    let motion = source["type"] == "motion"
                        || (band == "captions" && item["type"] == "motion");
                    (source, motion)
                };
                if motion {
                    insert_motion_metadata(&mut durations, source)?;
                }
                for effect in item["layer"]["effects"].as_array().into_iter().flatten() {
                    insert_motion_metadata(&mut durations, effect)?;
                }
            }
        }
    }
    for adjustment in canonical["document"]["adjustments"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let template = &adjustment["effect"]["motion"];
        if template.is_object() {
            insert_motion_metadata(&mut durations, template)?;
        }
    }
    for input in valle_compiler::motion_preparation_inputs(&decode_timeline(&author.to_string())?)?
    {
        let metadata = *durations
            .get(&input.key())
            .ok_or_else(|| anyhow!("prepared Motion clip {} is missing", input.clip_path))?;
        durations.insert(input.component, metadata);
    }
    Ok(durations)
}

/// The duration and role of a placed Motion instance, by the alias its component names.
fn insert_motion_metadata(
    durations: &mut std::collections::BTreeMap<String, valle_timeline::MotionSourceMetadata>,
    instance: &serde_json::Value,
) -> Result<()> {
    let alias = instance["component"]
        .as_str()
        .and_then(|id| id.strip_prefix("resource:"))
        .ok_or_else(|| anyhow!("invalid prepared Motion component"))?;
    durations.insert(
        alias.to_owned(),
        valle_timeline::MotionSourceMetadata {
            duration: serde_json::from_value(instance["sourceDuration"].clone())?,
            role: serde_json::from_value(instance["role"].clone())?,
        },
    );
    Ok(())
}

pub(super) fn collect_used_resources(
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
            if map.contains_key("component") {
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
    fn media_facts_accept_a_sparse_visual_track_without_optional_transitions() {
        let directory = tempfile::tempdir().unwrap();
        let author = serde_json::json!({"canvas":{"width":32,"height":32,"fps":4},"tracks":{"visual":[{"clips":[{"kind":"solid","start":0,"duration":1,"color":"#123456"}]}]}});
        let timeline = decode_timeline(&author.to_string()).unwrap();
        let result =
            prepare_timeline_media_facts(timeline, directory.path(), &mut Default::default())
                .unwrap();
        assert!(result.0.is_empty());
    }

    #[test]
    fn caption_baseline_compiles_one_template_for_three_hundred_clips() {
        let dir = tempfile::tempdir().unwrap();
        let font = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/fonts/noto/NotoSans-Regular.ttf")
            .canonicalize()
            .unwrap();
        std::fs::write(dir.path().join("caption.motion.tsx"), r##"export const composition={width:160,height:48,fps:4,duration:1};
export const role=captionPresenter({intro:0.25,outro:0.25});
export default function Caption(props){ const time = useCurrentTime(); const caption = useCaption();return <Scene><Text style={{fontFamily:caption.style.font,fontSize:caption.style.fontSize,color:time.clipSeconds<1?"#facc15":"white"}}>{caption.text}</Text></Scene>; }"##).unwrap();
        let clips: Vec<_> = (0..300).map(|i| serde_json::json!({"start":f64::from(i*2)+0.125,"duration":2,"runs":[{"text":format!("Word {}",i%100),"start":0,"end":1.75}]})).collect();
        let author = serde_json::json!({"canvas":{"width":160,"height":48,"fps":4},"resources":{"caption":"caption.motion.tsx","font":font},"tracks":{"caption":[{"presenter":{"component":"caption"},"style":{"font":"font","fontSize":20},"clips":clips}]}});
        let start = std::time::Instant::now();
        // The track's captions share its layout, so one template serves every word.
        let captured = capture_motion_sources(&author, dir.path()).unwrap();
        assert_eq!(captured.instances.len(), 1);
        let (prepared, metadata) = captured.prepare().unwrap();
        assert_eq!(prepared.len(), 1);
        assert_eq!(metadata.len(), 1);
        let template = &prepared.values().next().unwrap().compiled.artifact;
        assert!(template.reads_caption());
        let canonical = valle_compiler::compile_timeline_with_motion_sources(
            decode_timeline(&author.to_string()).unwrap(),
            &metadata,
        )
        .unwrap()
        .to_wire();
        use valle_timeline::internal::wire::document::CaptionItemWire;
        let words = canonical.document.captions.tracks[0]
            .items
            .iter()
            .filter_map(|item| match item {
                CaptionItemWire::Motion(caption) => Some(caption.runs[0].text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(words.len(), 300);
        assert_eq!(words[1], "Word 1");
        eprintln!(
            "native-caption-baseline: clips=300 compilations=1 elapsed-ms={}",
            start.elapsed().as_millis()
        );
    }

    #[test]
    fn presenter_media_facts_keep_word_timing_in_the_author_and_do_not_compile_a_template() {
        let dir = tempfile::tempdir().unwrap();
        let font = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/fonts/noto/NotoSans-Regular.ttf")
            .canonicalize()
            .unwrap();
        let author = serde_json::json!({"canvas":{"width":160,"height":48,"fps":4},"resources":{"words":"missing.motion.tsx","font":font},"tracks":{"caption":[{"presenter":{"component":"words"},"style":{"font":"font","fontSize":20},"clips":[{"start":0,"duration":2,"runs":[{"text":"Words","start":0,"end":1.75}]}]}]}});
        let timeline = decode_timeline(&author.to_string()).unwrap();
        let mut media = FrozenMediaCache::default();
        let (facts, _, dependencies) =
            prepare_timeline_media_facts(timeline.clone(), dir.path(), &mut media).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0]["id"], "resource:font");
        assert_eq!(facts[0]["entry"]["kind"], "font");
        assert_eq!(
            dependencies[&font.to_string_lossy().to_string()],
            ContentDigest::of_bytes(&std::fs::read(&font).unwrap())
        );
        let unchanged: serde_json::Value =
            serde_json::from_slice(&timeline_bytes(&timeline).unwrap()).unwrap();
        assert_eq!(
            unchanged["tracks"]["caption"][0]["clips"][0]["runs"][0]["end"],
            1.75
        );
    }

    #[test]
    fn media_facts_preserve_transition_indices_without_compiling_motion() {
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
            }, {
                "kind": "image", "src": "poster", "start": 0.5, "duration": 1
            }], "transitions": [{"from":0,"to":1,"kind":"circleOpen"}]}]}
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
