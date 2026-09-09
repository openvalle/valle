//! File and project Timeline delivery through the fixed-package admission path.
use crate::TimelineAction;
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
        TimelineAction::Check { input } => {
            let timeline = decode_timeline(&super::read(&input)?)?;
            valle_compiler::compile_timeline(timeline)?;
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
    super::fixed_render::require_new_output(output)?;
    let extension = if frame.is_some() { "png" } else { "mp4" };
    if !output
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(extension))
    {
        bail!("output must have an .{extension} extension");
    }
    let doc: serde_json::Value = serde_json::from_slice(&timeline_bytes(&timeline)?)?;
    let canonical = encode_canonical(&valle_compiler::compile_timeline(timeline)?)?;
    let frozen = tempfile::tempdir().context("creating immutable render resources")?;
    let mut resources = super::motion_package::FixedResources::new();
    let mut catalog = NativeResourceCatalog::new();
    if let Some(locators) = doc["resources"].as_object() {
        let mut ordered: Vec<_> = locators.iter().collect();
        ordered.sort_by_key(|(name, _)| motion_component(&doc, name).is_none());
        let mut motion_asset_kinds = std::collections::BTreeMap::new();
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
                require_consistent_motion_bindings(&doc, name, &clip["resources"])?;
                let mut specs = Vec::new();
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
                        specs.push(format!("{control}={}", base.join(locator).display()));
                        deps.push(super::motion_package::FixedResourceDependency {
                            role: control.clone(),
                            resource_id: format!("resource:{key}"),
                        });
                    }
                }
                let canvas = (
                    doc["canvas"]["width"].as_u64().unwrap_or(1920) as u32,
                    doc["canvas"]["height"].as_u64().unwrap_or(1080) as u32,
                );
                let prepared = super::motion::compile_timeline_component(&path, &specs, canvas)?;
                let artifact = prepared.compiled.artifact;
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
                        if motion_asset_kinds
                            .insert(key.to_owned(), kind)
                            .is_some_and(|old| old != kind)
                        {
                            bail!("resource {key} is used with conflicting Motion asset kinds");
                        }
                    }
                }
                for (i, bytes) in valle_motion::DEFAULT_MOTION_FONT_WEIGHTS.iter().enumerate() {
                    let font_id = format!("font:{name}:{i}");
                    resources.add_font(&font_id, bytes)?;
                    deps.push(super::motion_package::FixedResourceDependency {
                        role: format!("font:{i}"),
                        resource_id: font_id,
                    });
                    catalog.insert_bytes(ContentDigest::of_bytes(bytes), bytes.to_vec())?;
                }
                for (i, (_, bytes)) in valle_motion::math_formula::formula_font_pack().enumerate() {
                    let font_id = format!("font:{name}:formula:{i}");
                    resources.add_font(&font_id, bytes)?;
                    deps.push(super::motion_package::FixedResourceDependency {
                        role: format!("formula:{i}"),
                        resource_id: font_id,
                    });
                    catalog.insert_bytes(ContentDigest::of_bytes(bytes), bytes.to_vec())?;
                }
                for (i, shader) in prepared.shaders.packages().enumerate() {
                    let shader_id = format!("shader:{name}:{i}");
                    resources.add_shader(&shader_id, shader)?;
                    deps.push(super::motion_package::FixedResourceDependency {
                        role: format!("shader:{i}"),
                        resource_id: shader_id,
                    });
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
            let bytes = std::fs::read(&path)
                .with_context(|| format!("reading resource {name}: {}", path.display()))?;
            let hash = ContentDigest::of_bytes(&bytes);
            let frozen_path = frozen.path().join(hash.as_hex());
            std::fs::write(&frozen_path, &bytes)?;
            let path = frozen_path;
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
                catalog.insert_file(hash, path)?;
                continue;
            }
            let kind = resource_kind(&doc, name)
                .or_else(|| motion_asset_kinds.get(name).copied())
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
                bytes: bytes.clone(),
                hash,
            };
            if kind == AssetKind::Video {
                let probe = valle_media::codec::decode::probe_video_presentation(&path)?;
                use valle_timeline::internal::wire::resource::*;
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
                    audio_stream: None,
                };
                resources.add(
                    &id,
                    ResourceEntryWire::Video {
                        digest: hash,
                        descriptor: descriptor.clone(),
                    },
                    valle_engine::render::VerifiedResourceFacts::Video {
                        descriptor,
                        temporal_footprint: Default::default(),
                    },
                    Vec::new(),
                )?;
            } else {
                resources.add_asset(&id, kind, &asset)?;
            }
            catalog.insert_file(hash, path)?;
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
    let opened = open_verified_fixed_package(&package, &files).map_err(|e| anyhow!(e))?;
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
    let summary = match frame {
        Some(f) => renderer.preview_frame_key(valle_engine::render::FrameKey::new(f), output)?,
        None => renderer.export_mp4(output)?,
    };
    super::fixed_render::print_delivery_report(
        &opened,
        if frame.is_some() { "preview" } else { "export" },
        output,
        &summary,
    )?;
    Ok(std::process::ExitCode::SUCCESS)
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
                map.iter()
                    .find_map(|(k, v)| scan(v, name, audio || k == "audio"))
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
                map.values().find_map(|v| motion_component(v, name))
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
                || map.values().any(|v| has_visual_source(v, name, kind))
        }
        serde_json::Value::Array(values) => values.iter().any(|v| has_visual_source(v, name, kind)),
        _ => false,
    }
}

fn require_consistent_motion_bindings(
    value: &serde_json::Value,
    name: &str,
    bindings: &serde_json::Value,
) -> Result<()> {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("kind").and_then(|v| v.as_str()) == Some("motion")
                && map.get("component").and_then(|v| v.as_str()) == Some(name)
                && &value["resources"] != bindings
            {
                bail!(
                    "Motion component {name} has different resource bindings across clips; declare separate resource aliases for each binding set"
                );
            }
            for child in map.values() {
                require_consistent_motion_bindings(child, name, bindings)?;
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                require_consistent_motion_bindings(child, name, bindings)?;
            }
        }
        _ => {}
    }
    Ok(())
}
