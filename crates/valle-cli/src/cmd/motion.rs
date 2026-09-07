//! `valle motion check/render/studio` —— Motion JSX artifact authoring tools.
//!
//! Batch authoring and Studio share one compilation and capability-admission implementation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::{MotionAction, MotionCanvasArgs};
use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use valle_motion::{
    ArtifactEnvelope, BuildFingerprint, ContentDigest, CueWindow, MotionViewport, NodeKind,
    ResourceRef, canonical_bytes, motion_context_at, phase_windows, resolve_props,
};
use valle_timeline::internal::quantize::quantize_frame_boundary;
use valle_timeline::{
    time::{FrameRate, RationalTime},
    wire::timeline::TimelineTimeWire,
};

pub fn run(action: MotionAction) -> Result<std::process::ExitCode> {
    match action {
        MotionAction::Check {
            input,
            assets,
            data,
            canvas,
        } => check(
            &input,
            &assets,
            data.as_deref(),
            resolve_canvas_size(canvas)?,
        ),
        MotionAction::Render {
            frame,
            input,
            output,
            backend,
            assets,
            font,
            data,
            duration,
            fps,
            canvas,
        } => render(
            &input,
            &output,
            &assets,
            &font,
            data.as_deref(),
            parse_duration(&duration)?,
            parse_fps(&fps)?,
            resolve_canvas_size(canvas)?,
            frame,
            backend.into(),
        ),
        MotionAction::Studio {
            input,
            assets,
            font,
            data,
            web_assets_dir,
            port,
            duration,
            fps,
            canvas,
        } => studio(StudioRequest {
            input,
            asset_specs: assets,
            fonts: font,
            data,
            web_assets_dir,
            port,
            duration: parse_duration(&duration)?,
            fps: parse_fps(&fps)?,
            canvas_size: resolve_canvas_size(canvas)?,
        }),
    }
}

/// Compile once and hand the same verified package used by Studio to Native delivery.
#[allow(clippy::too_many_arguments)]
fn render(
    input: &Path,
    output: &Path,
    asset_specs: &[String],
    font_paths: &[PathBuf],
    data: Option<&Path>,
    duration: RationalTime,
    fps: FrameRate,
    canvas: MotionViewport,
    frame: Option<i64>,
    backend: valle_render::executor::skia::SkiaBackendKind,
) -> Result<std::process::ExitCode> {
    use valle_engine::fixed_package::{fixed_package_files, open_verified_fixed_package};
    use valle_render::host::{
        NativeProject, NativeRenderOptions, NativeRenderer, NativeResourceCatalog,
    };

    super::fixed_render::require_new_output(output)?;
    let extension = if frame.is_some() { "png" } else { "mp4" };
    if !output
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
    {
        bail!("output must have an .{extension} extension");
    }
    let frames = duration_frames(duration, fps)?;
    let explicit_fonts = read_font_files(font_paths)?;
    let fonts = authoring_font_blobs(&explicit_fonts);
    let prepared = match compile_and_prepare(input, asset_specs, &fonts, canvas, data)? {
        Ok(prepared) => prepared,
        Err(()) => return Ok(std::process::ExitCode::FAILURE),
    };
    let artifact = &prepared.compiled.artifact;
    let cue_end = frames
        .saturating_sub(artifact.controls.phase_spec().exit_frames)
        .max(1)
        .min(frames);
    let cues = artifact
        .controls
        .cues
        .keys()
        .map(|name| {
            (
                name.clone(),
                CueWindow {
                    start_frame: 0,
                    end_frame: cue_end,
                    enter_frames: 0,
                    exit_frames: 0,
                },
            )
        })
        .collect();
    let font_blobs = fixed_package_font_blobs(artifact, &explicit_fonts)?;
    let package = super::motion_package::build_standalone_motion_package(
        super::motion_package::StandaloneMotionPackageInput {
            artifact,
            assets: &prepared.assets,
            font_blobs: &font_blobs,
            shaders: &prepared.shaders,
            cue_bindings: &cues,
            duration,
            frame_rate: fps,
            canvas: canvas.tuple(),
        },
    )?;
    let files = fixed_package_files(
        &package.timeline_json,
        &package.resource_manifest_json,
        &package.verified_binding_bundle_json,
        &package.execution_profile_json,
    );
    let opened = open_verified_fixed_package(&package.fixed_package_manifest_json, &files)
        .map_err(|error| anyhow!(error))?;
    // Freeze resources for the job; streaming decoders require stable files.
    let mut catalog = NativeResourceCatalog::new();
    let frozen = tempfile::tempdir().context("creating immutable render resources")?;
    for bytes in &font_blobs {
        catalog.insert_bytes(ContentDigest::of_bytes(bytes), bytes.clone())?;
    }
    for asset in prepared.assets.values() {
        let path = frozen.path().join(asset.hash.as_hex());
        std::fs::write(&path, &asset.bytes)?;
        catalog.insert_file(asset.hash, path)?;
    }
    let project = NativeProject::from_render(opened.engine_render(), Arc::new(catalog));
    let renderer = NativeRenderer::new(
        project,
        NativeRenderOptions {
            backend,
            progress: crate::output::render_progress(),
            ..NativeRenderOptions::default()
        },
    );
    let summary = match frame {
        Some(frame) => {
            renderer.preview_frame_key(valle_engine::render::FrameKey::new(frame), output)?
        }
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

#[derive(Clone)]
struct StudioRequest {
    input: PathBuf,
    asset_specs: Vec<String>,
    fonts: Vec<PathBuf>,
    data: Option<PathBuf>,
    web_assets_dir: Option<PathBuf>,
    port: u16,
    duration: RationalTime,
    fps: FrameRate,
    canvas_size: MotionViewport,
}

fn studio(request: StudioRequest) -> Result<std::process::ExitCode> {
    validate_studio_request(&request)?;
    let cache_root = crate::webruntime::default_cache_root()?;
    let runtime = crate::webruntime::resolve(request.web_assets_dir.as_deref(), &cache_root)?;
    let generation = Arc::new(AtomicU64::new(1));
    let config = studio_state_json(&request, 1)?;

    let mut runtime_files = runtime.serving_map();
    let mut local_runtime_files = BTreeMap::new();
    for (name, asset) in load_assets(&request.asset_specs)? {
        local_runtime_files.insert(format!("motion-assets/{name}"), asset.path);
    }
    for (index, path) in request.fonts.iter().enumerate() {
        local_runtime_files.insert(format!("motion-fonts/{index}"), path.clone());
    }
    install_motion_runtime_font_files(&mut local_runtime_files, &cache_root)?;
    for (route, path) in local_runtime_files {
        if let Some(crate::webruntime::HostedFile::VerifiedRuntime(runtime_bytes)) =
            runtime_files.get(&route)
        {
            let local_bytes = std::fs::read(&path)
                .with_context(|| format!("reading local Studio runtime file {}", path.display()))?;
            if runtime_bytes.as_ref() != local_bytes.as_slice() {
                bail!(
                    "local Studio route '{}' conflicts with a different verified runtime asset",
                    route
                );
            }
            continue;
        }
        if runtime_files
            .insert(
                route.clone(),
                crate::webruntime::HostedFile::LocalPath(path),
            )
            .is_some()
        {
            bail!("local Studio route '{}' is duplicated", route);
        }
    }
    let state = Arc::new(crate::webhost::StudioHost {
        runtime_files: runtime_files.clone(),
        assets_dir: None,
        config_json: Arc::new(RwLock::new(config)),
        sse: crate::webhost::SseBroadcaster::default(),
        capture_dir: None,
        library: None,
        project: None,
        last_report: RwLock::new(None),
    });
    let (server, addr) = crate::webhost::bind(request.port)?;
    let url = format!("http://{addr}/studio");
    spawn_motion_studio_watcher(request.clone(), Arc::clone(&state), generation);
    if crate::output::machine() && !crate::output::events() {
        crate::output::emit(
            serde_json::json!({"status":"ready", "url":url, "port":addr.port(),
            "runtime_version":runtime.manifest.runtime_version, "runtime_source":runtime.source.as_str()}),
        );
    }
    crate::events::emit(crate::events::EventKind::Ready {
        url: url.clone(),
        port: addr.port(),
        runtime_version: runtime.manifest.runtime_version.clone(),
        runtime_source: runtime.source.as_str().to_owned(),
    });
    eprintln!(
        "motion studio: {url} (input {}; Ctrl-C to stop)",
        request.input.display()
    );
    crate::webhost::serve_forever(server, state)?;
    Ok(std::process::ExitCode::SUCCESS)
}

fn check(
    input: &Path,
    asset_specs: &[String],
    data: Option<&Path>,
    canvas_size: MotionViewport,
) -> Result<std::process::ExitCode> {
    // Use deterministic fallback fonts and a shared canvas so relative-unit measurements agree
    // across check, render, and Studio.
    let font_blobs = load_fonts(&[])?;
    match compile_and_prepare(input, asset_specs, &font_blobs, canvas_size, data)? {
        Ok(prepared) => {
            if let Err(error) = line_box_emit_smoke(&prepared, &font_blobs) {
                eprintln!("error: {error}");
                return Ok(std::process::ExitCode::FAILURE);
            }
            let artifact = &prepared.compiled.artifact;
            crate::output::emit(
                serde_json::json!({"status":"ok","component":artifact.component,"nodes":artifact.nodes.len(),"expressions":artifact.exprs.len()}),
            );
            Ok(std::process::ExitCode::SUCCESS)
        }
        Err(()) => Ok(std::process::ExitCode::FAILURE),
    }
}

#[derive(Debug, Clone)]
pub(super) struct BoundAsset {
    pub(super) path: PathBuf,
    pub(super) bytes: Vec<u8>,
    pub(super) hash: ContentDigest,
}

pub(super) struct PreparedInput {
    pub(super) compiled: valle_compiler::motion::CompiledMotion,
    data_binding: Option<valle_compiler::motion::PrepareDataBinding>,
    prepared: valle_motion::PreparedScene,
    pub(super) assets: BTreeMap<String, BoundAsset>,
    pub(super) shaders: valle_motion::shader::ShaderRegistry,
    canvas_size: MotionViewport,
}

fn compile_with_font_assets(
    graph: &valle_compiler::motion::MotionModuleGraph,
    resources: &[ResourceRef],
    assets: &BTreeMap<String, BoundAsset>,
    font_blobs: &[Vec<u8>],
    canvas_size: MotionViewport,
    shaders: &valle_motion::shader::ShaderRegistry,
    data: Option<&valle_compiler::motion::PrepareDataBinding>,
) -> Result<
    Result<
        (
            valle_compiler::motion::CompiledMotion,
            Vec<(String, Vec<u8>)>,
        ),
        Vec<valle_compiler::motion::CompilerDiagnostic>,
    >,
> {
    // Asset-control kinds live in the compiled schema, but measureText needs font aliases while
    // compiling. Detect font containers from their bytes first, so the authoritative compile sees
    // every possible `asset://control` font without a fallback-font discovery pass changing
    // topology or tripping a budget before the real pass.
    let measure_aliases = assets
        .iter()
        .filter(|(_, asset)| ttf_parser::Face::parse(&asset.bytes, 0).is_ok())
        .map(|(control, asset)| (format!("asset://{control}"), asset.bytes.clone()))
        .collect::<Vec<_>>();
    let measure = valle_compiler::motion::MeasureEnv::new_with_aliases(
        font_blobs,
        &measure_aliases,
        canvas_size.tuple(),
    )
    .map_err(|diagnostic| anyhow!("{}", diagnostic.message))?;
    let compiled = match valle_compiler::motion::compile_motion_modules_with_full_env_and_data(
        graph,
        resources,
        Some(&measure),
        Some(shaders),
        data,
    ) {
        Ok(compiled) => compiled,
        Err(diagnostics) => return Ok(Err(diagnostics)),
    };
    let mut render_aliases = Vec::new();
    for (control, schema) in &compiled.artifact.controls.assets {
        if schema.kind != valle_motion::AssetKind::Font {
            continue;
        }
        let Some(asset) = assets.get(control) else {
            continue;
        };
        if ttf_parser::Face::parse(&asset.bytes, 0).is_err() {
            bail!(
                "asset control `{control}` is declared as a font, but its bound bytes are not a valid font"
            );
        }
        render_aliases.push((
            valle_motion::font_family_alias(&asset.hash),
            asset.bytes.clone(),
        ));
    }
    Ok(Ok((compiled, render_aliases)))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FingerprintInputs<'a> {
    assets: BTreeMap<&'a str, &'a ContentDigest>,
    fonts: Vec<&'a ContentDigest>,
}

fn studio_state_json(request: &StudioRequest, generation: u64) -> Result<String> {
    let module_graph = load_motion_module_graph(&request.input)?;
    let assets = load_assets(&request.asset_specs)?;
    let shaders = valle_compiler::load_shader_registry(&[], request.input.parent())?;
    let resources = assets
        .iter()
        .map(|(control, asset)| ResourceRef {
            control: control.clone(),
            content_hash: asset.hash.clone(),
        })
        .collect::<Vec<_>>();
    // Hot reload must measure with the same fonts and viewport used by Studio preview.
    let explicit_font_blobs = read_font_files(&request.fonts)?;
    let font_blobs = authoring_font_blobs(&explicit_font_blobs);
    let data_binding = load_prepare_data(&request.input, request.data.as_deref())?;
    let (compiled, _) = match compile_with_font_assets(
        &module_graph,
        &resources,
        &assets,
        &font_blobs,
        request.canvas_size,
        &shaders,
        data_binding.as_ref(),
    )? {
        Ok(compiled) => compiled,
        Err(diagnostics) => {
            return Ok(serde_json::to_string(&serde_json::json!({
                "status": "error",
                "protocolVersion": crate::webruntime::PROTOCOL_VERSION,
                "generation": generation,
                "input": request.input,
                "diagnostics": diagnostics,
            }))?);
        }
    };
    if let Err(error) = ensure_rendered_assets_are_bound(&compiled.artifact, &assets) {
        return studio_runtime_error_json(request, generation, "motion-resource-unbound", &error);
    }
    if let Err(errors) = compiled.artifact.validate_with_shaders(&shaders) {
        return studio_runtime_error_json(
            request,
            generation,
            "motion-shader-admission",
            &anyhow!(format!("{errors:?}")),
        );
    }
    let prepared_scene = match valle_motion::prepare_scene(&compiled.artifact) {
        Ok(prepared) => prepared,
        Err(error) => {
            return studio_runtime_error_json(
                request,
                generation,
                "motion-capability-admission",
                &anyhow!(error.to_string()),
            );
        }
    };
    let prepared = PreparedInput {
        compiled,
        data_binding,
        prepared: prepared_scene,
        assets,
        shaders,
        canvas_size: request.canvas_size,
    };
    let font_hashes = font_blobs
        .iter()
        .map(|bytes| ContentDigest::of_bytes(bytes))
        .collect::<Vec<_>>();
    let envelope = envelope_for(&prepared, &font_hashes)?;
    let artifact_digest =
        ContentDigest::of_bytes(&canonical_bytes(&prepared.compiled.artifact)?).to_wire();
    let duration_frames = duration_frames(request.duration, request.fps)?;
    let timing = prepared.compiled.artifact.controls.phase_spec();
    let default_cue_end = duration_frames
        .saturating_sub(timing.exit_frames)
        .max(1)
        .min(duration_frames);
    let cue_bindings = prepared
        .compiled
        .artifact
        .controls
        .cues
        .keys()
        .map(|name| {
            (
                name.clone(),
                CueWindow {
                    start_frame: 0,
                    end_frame: default_cue_end,
                    enter_frames: 0,
                    exit_frames: 0,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let studio_cue_bindings = cue_bindings
        .iter()
        .map(|(name, cue)| {
            (
                name,
                serde_json::json!({
                    "type": "sourceRange",
                    "startFrame": cue.start_frame,
                    "endFrame": cue.end_frame,
                    "enterFrames": cue.enter_frames,
                    "exitFrames": cue.exit_frames,
                }),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let frame_rate = request.fps;
    let runtime_font_blobs =
        fixed_package_font_blobs(&prepared.compiled.artifact, &explicit_font_blobs)?;
    let fixed_package = match super::motion_package::build_standalone_motion_package(
        super::motion_package::StandaloneMotionPackageInput {
            artifact: &prepared.compiled.artifact,
            assets: &prepared.assets,
            // Freeze every face that the Motion DrawProgram can request by content digest.
            // Formula faces are needed only when RaTeX glyph runs are merged into the final
            // ProgramRecording; ordinary Motion packages keep the smaller default closure.
            font_blobs: &runtime_font_blobs,
            shaders: &prepared.shaders,
            cue_bindings: &cue_bindings,
            duration: request.duration,
            frame_rate,
            canvas: request.canvas_size.tuple(),
        },
    ) {
        Ok(package) => package,
        Err(error) => {
            return studio_runtime_error_json(
                request,
                generation,
                "motion-fixed-package-admission",
                &error,
            );
        }
    };
    let asset_urls = prepared
        .assets
        .iter()
        .filter(|(name, _)| {
            matches!(
                prepared.compiled.artifact.controls.assets[*name].kind,
                valle_motion::AssetKind::Image | valle_motion::AssetKind::Model3d
            )
        })
        .map(|(name, _)| {
            let kind = match prepared.compiled.artifact.controls.assets[name].kind {
                valle_motion::AssetKind::Image => "image",
                valle_motion::AssetKind::Model3d => "model3d",
                _ => unreachable!("filter above admits only Studio visual resources"),
            };
            serde_json::json!({
                "name": name,
                "kind": kind,
                "url": format!("/motion-assets/{name}"),
            })
        })
        .collect::<Vec<_>>();
    let resource_locators = prepared
        .assets
        .keys()
        .map(|control| {
            serde_json::json!({
                "id": format!("asset:{control}"),
                "url": format!("/motion-assets/{control}"),
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::to_string(&serde_json::json!({
        "status": "ok",
        "protocolVersion": crate::webruntime::PROTOCOL_VERSION,
        "generation": generation,
        "input": request.input,
        "artifactDigest": artifact_digest,
        "artifact": envelope.artifact,
        "preparedData": prepared.data_binding.as_ref().map_or_else(
            || serde_json::json!({}),
            |binding| binding.value.clone(),
        ),
        "dataSource": prepared.data_binding.as_ref().map(|binding| binding.source.clone()),
        "timing": {
            "enterFrames": timing.enter_frames,
            "exitFrames": timing.exit_frames,
        },
        "cueBindings": studio_cue_bindings,
        "sourceMap": prepared.compiled.source_map,
        "assets": asset_urls,
        "resourceLocators": resource_locators,
        "shaders": prepared.shaders.packages().map(|package| {
            serde_json::json!({
                "uri": package.uri().to_string(),
                "manifestBytes": package.canonical_manifest,
                "sourceBytes": package.source.as_bytes(),
            })
        }).collect::<Vec<_>>(),
        "durationFrames": duration_frames,
        "fps": {
            "num": request.fps.numerator(),
            "den": request.fps.denominator(),
        },
        "viewport": {
            "width": request.canvas_size.width,
            "height": request.canvas_size.height,
        },
        "fixedPackageManifestJson": fixed_package.fixed_package_manifest_json,
        "timelineJson": fixed_package.timeline_json,
        "timeline": fixed_package.timeline,
        "resourceManifestJson": fixed_package.resource_manifest_json,
        "resourceManifest": fixed_package.resource_manifest,
        "verifiedBindingBundleJson": fixed_package.verified_binding_bundle_json,
        "diagnostics": [],
        // Use the shared runtime asset map.
        "runtimeBaseUrl": "/",
        "runtimeAssets": crate::webruntime::runtime_assets_json(),
    }))?)
}

fn studio_runtime_error_json(
    request: &StudioRequest,
    generation: u64,
    code: &str,
    error: &anyhow::Error,
) -> Result<String> {
    Ok(serde_json::to_string(&serde_json::json!({
        "status": "error",
        "protocolVersion": crate::webruntime::PROTOCOL_VERSION,
        "generation": generation,
        "input": request.input,
        "diagnostics": [{
            "class": "resource",
            "code": code,
            "message": format!("{error:#}"),
        }],
    }))?)
}

fn spawn_motion_studio_watcher(
    request: StudioRequest,
    state: Arc<crate::webhost::StudioHost>,
    generation: Arc<AtomicU64>,
) {
    std::thread::spawn(move || {
        let mut paths = motion_watch_paths(&request);
        let mut last = path_fingerprints(&paths);
        loop {
            std::thread::sleep(Duration::from_millis(200));
            paths = motion_watch_paths(&request);
            let now = path_fingerprints(&paths);
            if now == last {
                continue;
            }
            let mut settled = now;
            loop {
                std::thread::sleep(Duration::from_millis(60));
                let again = path_fingerprints(&paths);
                if again == settled {
                    break;
                }
                settled = again;
            }
            last = settled;
            let next = generation.fetch_add(1, Ordering::SeqCst) + 1;
            let config = studio_state_json(&request, next).unwrap_or_else(|error| {
                studio_runtime_error_json(&request, next, "motion-studio-reload", &error)
                    .unwrap_or_else(|_| "{\"status\":\"error\"}".into())
            });
            let ok = serde_json::from_str::<serde_json::Value>(&config)
                .ok()
                .and_then(|value| value["status"].as_str().map(|status| status == "ok"))
                .unwrap_or(false);
            if let Ok(mut slot) = state.config_json.write() {
                *slot = config;
            }
            state.sse.broadcast(
                "motion",
                &serde_json::json!({ "generation": next, "ok": ok }).to_string(),
            );
            crate::events::emit(crate::events::EventKind::Reload { generation: next });
            eprintln!(
                "motion studio: reloaded {} (generation {next}, {})",
                request.input.display(),
                if ok { "ok" } else { "diagnostics" }
            );
        }
    });
}

fn motion_watch_paths(request: &StudioRequest) -> Vec<PathBuf> {
    let mut paths =
        motion_module_paths(&request.input).unwrap_or_else(|_| vec![request.input.clone()]);
    paths.extend(request.fonts.iter().cloned());
    paths.extend(request.data.iter().cloned());
    paths.extend(
        request
            .asset_specs
            .iter()
            .filter_map(|spec| spec.split_once('=').map(|(_, path)| PathBuf::from(path))),
    );
    if let Some(root) = request.input.parent().map(|path| path.join("shaders")) {
        let mut packages = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        packages.sort();
        for package in packages {
            paths.push(package.join("manifest.json"));
            paths.push(package.join("shader.vsksl"));
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn load_motion_module_graph(input: &Path) -> Result<valle_compiler::motion::MotionModuleGraph> {
    valle_compiler::motion::MotionModuleGraph::from_entry_path(input).map_err(|diagnostics| {
        anyhow!(
            "invalid Motion module graph: {}",
            diagnostics
                .into_iter()
                .map(|diagnostic| diagnostic.message)
                .collect::<Vec<_>>()
                .join("; ")
        )
    })
}

fn motion_module_paths(input: &Path) -> Result<Vec<PathBuf>> {
    let root = input.parent().unwrap_or_else(|| Path::new("."));
    let graph = load_motion_module_graph(input)?;
    Ok(graph.modules.keys().map(|path| root.join(path)).collect())
}

fn path_fingerprints(paths: &[PathBuf]) -> Vec<(PathBuf, Option<(std::time::SystemTime, u64)>)> {
    paths
        .iter()
        .map(|path| {
            let fingerprint = std::fs::metadata(path)
                .ok()
                .and_then(|meta| Some((meta.modified().ok()?, meta.len())));
            (path.clone(), fingerprint)
        })
        .collect()
}

/// Compile-time measurements are baked into the artifact. Load fonts before compiling and use the
/// same font bytes and canvas dimensions for rendering.
fn compile_and_prepare(
    input: &Path,
    asset_specs: &[String],
    font_blobs: &[Vec<u8>],
    canvas_size: MotionViewport,
    data: Option<&Path>,
) -> Result<Result<PreparedInput, ()>> {
    let perf = perf_enabled();
    let compile_started = perf.then(Instant::now);
    let module_graph = load_motion_module_graph(input)?;
    let assets = load_assets(asset_specs)?;
    let shaders = valle_compiler::load_shader_registry(&[], input.parent())?;
    let resources = assets
        .iter()
        .map(|(control, asset)| ResourceRef {
            control: control.clone(),
            content_hash: asset.hash.clone(),
        })
        .collect::<Vec<_>>();
    let data_binding = load_prepare_data(input, data)?;
    let (compiled, _) = match compile_with_font_assets(
        &module_graph,
        &resources,
        &assets,
        font_blobs,
        canvas_size,
        &shaders,
        data_binding.as_ref(),
    )? {
        Ok(compiled) => compiled,
        Err(diagnostics) => {
            eprintln!("{}: {} diagnostic(s)", input.display(), diagnostics.len());
            for diagnostic in diagnostics {
                let diagnostic_path = diagnostic
                    .source_path
                    .as_deref()
                    .map_or_else(|| input.display().to_string(), ToOwned::to_owned);
                eprintln!(
                    "  {}:{}:{} [{}:{:?}] {}",
                    diagnostic_path,
                    diagnostic.span.line,
                    diagnostic.span.column,
                    match diagnostic.class {
                        valle_motion::DiagClass::Illegal => "illegal",
                        valle_motion::DiagClass::Unsupported => "unsupported",
                    },
                    diagnostic.code,
                    diagnostic.message
                );
            }
            if crate::output::machine() {
                crate::output::error("Motion compilation failed; see diagnostics on stderr");
            }
            return Ok(Err(()));
        }
    };

    ensure_rendered_assets_are_bound(&compiled.artifact, &assets)?;

    compiled
        .artifact
        .validate_with_shaders(&shaders)
        .map_err(|errors| {
            anyhow!("compiler produced a scene that failed shader admission: {errors:?}")
        })?;
    let compile_elapsed = compile_started.map(|started| started.elapsed());
    let prepare_started = perf.then(Instant::now);
    let prepared = valle_motion::prepare_scene(&compiled.artifact).map_err(|error| {
        anyhow!("compiler produced a scene that failed capability admission: {error}")
    })?;
    if let (Some(compile), Some(prepare_started)) = (compile_elapsed, prepare_started) {
        eprintln!(
            "[valle motion prepare] compile+admit {:.3} ms; prepare {:.3} ms",
            compile.as_secs_f64() * 1_000.0,
            prepare_started.elapsed().as_secs_f64() * 1_000.0,
        );
    }
    Ok(Ok(PreparedInput {
        compiled,
        data_binding,
        prepared,
        assets,
        shaders,
        canvas_size,
    }))
}

fn perf_enabled() -> bool {
    std::env::var_os("VALLE_PERF").is_some_and(|value| {
        let value = value.to_string_lossy();
        !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
    })
}

fn load_assets(specs: &[String]) -> Result<BTreeMap<String, BoundAsset>> {
    let mut assets = BTreeMap::new();
    for spec in specs {
        let (name, path) = spec
            .split_once('=')
            .ok_or_else(|| anyhow!("asset binding `{spec}` must use NAME=PATH"))?;
        if name.is_empty() || path.is_empty() {
            bail!("asset binding `{spec}` must use non-empty NAME=PATH");
        }
        if assets.contains_key(name) {
            bail!("asset control `{name}` is bound more than once");
        }
        let bytes =
            std::fs::read(path).with_context(|| format!("reading asset `{name}` at {path}"))?;
        assets.insert(
            name.to_string(),
            BoundAsset {
                path: PathBuf::from(path),
                hash: ContentDigest::of_bytes(&bytes),
                bytes,
            },
        );
    }
    Ok(assets)
}

fn load_prepare_data(
    input: &Path,
    path: Option<&Path>,
) -> Result<Option<valle_compiler::motion::PrepareDataBinding>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading Motion data binding {}", path.display()))?;
    let value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing Motion data binding {}", path.display()))?;
    let base = input.parent().unwrap_or_else(|| Path::new("."));
    let source = path
        .strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    Ok(Some(valle_compiler::motion::PrepareDataBinding {
        source,
        value,
    }))
}

fn envelope_for(
    prepared: &PreparedInput,
    font_hashes: &[ContentDigest],
) -> Result<ArtifactEnvelope> {
    let fingerprint_inputs = FingerprintInputs {
        assets: prepared
            .assets
            .iter()
            .map(|(name, asset)| (name.as_str(), &asset.hash))
            .collect(),
        fonts: font_hashes.iter().collect(),
    };
    let assets_digest = ContentDigest::of_bytes(&canonical_bytes(&fingerprint_inputs)?);
    let artifact = prepared.compiled.artifact.clone();
    Ok(ArtifactEnvelope {
        build_fingerprint: BuildFingerprint {
            compiler_version: valle_compiler::motion::MOTION_COMPILER_ID.into(),
            math_engine: valle_motion::MOTION_MATH_ENGINE_ID.into(),
            normalized_ast_digest: prepared.compiled.normalized_ast_digest,
            prepared_data_digest: prepared.compiled.prepared_data_digest,
            assets_digest,
            canvas_size: prepared.canvas_size,
            layout_engine: valle_motion::LAYOUT_ENGINE_ID.into(),
            formula_layout_engine: if artifact
                .nodes
                .iter()
                .any(|node| matches!(node.kind, NodeKind::MathFormula { .. }))
            {
                valle_motion::math_formula::FORMULA_LAYOUT_ENGINE.into()
            } else {
                String::new()
            },
        },
        artifact,
    })
}

fn ensure_rendered_assets_are_bound(
    artifact: &valle_motion::SceneArtifact,
    assets: &BTreeMap<String, BoundAsset>,
) -> Result<()> {
    for (name, control) in &artifact.controls.assets {
        if control.required && !assets.contains_key(name) {
            bail!("required asset control `{name}` is not bound; pass --asset {name}=PATH");
        }
    }
    for node in &artifact.nodes {
        // Require explicit asset bindings for both images and videos.
        let (label, source) = match &node.kind {
            NodeKind::Image { source } => ("image", source),
            NodeKind::Video { source, .. } => ("video", source),
            _ => continue,
        };
        let control = source
            .strip_prefix("asset://")
            .expect("validated Image/Video source uses asset://");
        if !assets.contains_key(control) {
            bail!(
                "{label} control `{control}` is used by node `{}` but is not bound; pass --asset {control}=PATH",
                node.key
            );
        }
    }
    Ok(())
}

fn validate_studio_request(request: &StudioRequest) -> Result<()> {
    if !request.duration.is_positive() {
        bail!("--duration must be a positive decimal number of seconds");
    }
    validate_pixel_size(request.canvas_size, "canvas")?;
    duration_frames(request.duration, request.fps).map(|_| ())
}

fn resolve_canvas_size(args: MotionCanvasArgs) -> Result<MotionViewport> {
    let (width, height) = args.size.unwrap_or((1920, 1080));
    validate_pixel_size(MotionViewport::new(width, height), "canvas")
}

fn validate_pixel_size(size: MotionViewport, label: &str) -> Result<MotionViewport> {
    if size.width == 0 || size.height == 0 {
        bail!("{label} dimensions must be positive");
    }
    if size.width > i32::MAX as u32 || size.height > i32::MAX as u32 {
        bail!("{label} dimensions exceed Skia's i32 domain");
    }
    Ok(size)
}

fn duration_frames(duration: RationalTime, frame_rate: FrameRate) -> Result<u32> {
    let frames =
        quantize_frame_boundary(duration, frame_rate).context("quantize exact Studio duration")?;
    let frames = u32::try_from(frames).context("duration exceeds the u32 frame domain")?;
    if frames == 0 {
        bail!("duration must quantize to at least one frame");
    }
    Ok(frames)
}

fn parse_duration(value: &str) -> Result<RationalTime> {
    let value = TimelineTimeWire::new(value)
        .with_context(|| format!("invalid exact --duration `{value}`"))?;
    Ok(RationalTime::from_exact(value.to_exact()))
}

fn parse_fps(value: &str) -> Result<FrameRate> {
    let (num, den) = match value.split_once('/') {
        Some((num, den)) => (num, den),
        None => (value, "1"),
    };
    let num: i64 = num
        .parse()
        .with_context(|| format!("invalid fps numerator `{num}`"))?;
    let den: u32 = den
        .parse()
        .with_context(|| format!("invalid fps denominator `{den}`"))?;
    FrameRate::new(num, den).map_err(|_| anyhow!("fps numerator and denominator must be positive"))
}

/// Check frame-zero layout and emission so compilation cannot silently accept text that produces no
/// glyphs.
fn line_box_emit_smoke(prepared: &PreparedInput, font_blobs: &[Vec<u8>]) -> Result<()> {
    use valle_motion::{
        CueSchedule, Fonts, LayoutOptions, Viewport, build_tree, default_font_naming, emit,
    };

    let artifact = &prepared.compiled.artifact;
    let props = resolve_props(&artifact.controls, &BTreeMap::new())
        .map_err(|error| anyhow!("line-box smoke props: {error}"))?;
    let fps = FrameRate::new(30, 1).expect("30 fps is valid");
    let windows = phase_windows(&artifact.controls.phase_spec(), 30);
    let ctx = motion_context_at(0, &windows, fps)
        .ok_or_else(|| anyhow!("line-box smoke context: no frame 0"))?;
    // `motion check` validates a component, not a Timeline instance, so it has no external signal
    // bindings. Optional cues retain their product default (inactive); required cues get a
    // deterministic one-frame window solely so layout can visit every authored expression.
    let required_cues = artifact
        .controls
        .cues
        .iter()
        .filter(|(_, control)| control.required)
        .map(|(name, _)| {
            (
                name.clone(),
                CueWindow {
                    start_frame: 0,
                    end_frame: 1,
                    enter_frames: 0,
                    exit_frames: 0,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let cues = CueSchedule::resolve(&artifact.controls, &required_cues)
        .map_err(|error| anyhow!("line-box smoke cues: {error}"))?;
    let signals = cues.sample(0, fps);
    let mut fonts = Fonts::default();
    valle_motion::register_default_motion_fonts(&mut fonts)
        .map_err(|error| anyhow!("line-box smoke font: {error}"))?;
    for bytes in font_blobs {
        if valle_motion::DEFAULT_MOTION_FONT_WEIGHTS
            .iter()
            .any(|pack| *pack == bytes.as_slice())
        {
            continue;
        }
        fonts
            .register(valle_motion::FontResource::new(bytes.clone()))
            .map_err(|error| anyhow!("line-box smoke font: {error}"))?;
    }
    let tree = build_tree(
        &prepared.prepared,
        &ctx,
        &props,
        &signals,
        &LayoutOptions {
            viewport: Viewport::new(prepared.canvas_size.tuple()),
            fonts: &fonts,
            styles: None,
        },
    )
    .map_err(|error| anyhow!("line-box smoke layout: {error}"))?;
    let report = emit(&tree, &default_font_naming)
        .map_err(|error| anyhow!("line-box smoke emit: {error}"))?;
    let empty = report
        .unsupported
        .iter()
        .filter(|(_, what)| what.contains("produced no glyphs"))
        .map(|(key, what)| format!("{key}: {what}"))
        .collect::<Vec<_>>();
    if !empty.is_empty() {
        bail!(
            "line-box smoke: text produced no glyphs at frame 0 ({})",
            empty.join("; ")
        );
    }
    Ok(())
}

fn load_fonts(paths: &[PathBuf]) -> Result<Vec<Vec<u8>>> {
    let explicit = read_font_files(paths)?;
    Ok(authoring_font_blobs(&explicit))
}

fn read_font_files(paths: &[PathBuf]) -> Result<Vec<Vec<u8>>> {
    paths
        .iter()
        .map(|path| std::fs::read(path).with_context(|| format!("reading font {}", path.display())))
        .collect()
}

fn authoring_font_blobs(explicit: &[Vec<u8>]) -> Vec<Vec<u8>> {
    // Explicit faces extend the deterministic Motion pack. Keeping the same base pack in
    // authoring and fixed-package execution prevents `--font` from silently changing generic
    // fallback metrics during compile while the Product engine still renders with the defaults.
    let mut blobs = valle_motion::DEFAULT_MOTION_FONT_WEIGHTS
        .iter()
        .map(|bytes| bytes.to_vec())
        .collect::<Vec<_>>();
    blobs.extend(explicit.iter().cloned());
    for (_, bytes) in valle_motion::math_formula::formula_font_pack() {
        blobs.push(bytes.to_vec());
    }
    deduplicate_font_blobs(&mut blobs);
    blobs
}

fn fixed_package_font_blobs(
    artifact: &valle_motion::SceneArtifact,
    explicit: &[Vec<u8>],
) -> Result<Vec<Vec<u8>>> {
    let mut blobs = valle_motion::DEFAULT_MOTION_FONT_WEIGHTS
        .iter()
        .map(|bytes| bytes.to_vec())
        .collect::<Vec<_>>();
    blobs.extend(used_formula_font_blobs(artifact)?);
    blobs.extend(explicit.iter().cloned());
    deduplicate_font_blobs(&mut blobs);
    Ok(blobs)
}

/// Exact, artifact-wide formula font closure for the immutable Studio fixed package.
///
/// `latex` and `display` are static in `SceneArtifact`. Formula `fontSize`, color, visibility,
/// props and cues may vary by frame, but they cannot change the RaTeX face selected for a glyph.
/// Inspecting the emitted `ProgramRecording` also avoids freezing Size faces for delimiters that
/// RaTeX already lowered to paths.
fn used_formula_font_blobs(artifact: &valle_motion::SceneArtifact) -> Result<Vec<Vec<u8>>> {
    if !artifact
        .nodes
        .iter()
        .any(|node| matches!(node.kind, NodeKind::MathFormula { .. }))
    {
        return Ok(Vec::new());
    }

    let registry = valle_motion::math_formula::FormulaFontRegistry::load_default()
        .map_err(|error| anyhow!("load embedded formula font registry: {error}"))?;
    let policy = valle_motion::math_formula::AdmitPolicy::default();
    let mut used_families = BTreeSet::new();
    for node in &artifact.nodes {
        let NodeKind::MathFormula { latex, display, .. } = &node.kind else {
            continue;
        };
        let fragment = valle_motion::math_formula::emit_formula(
            latex,
            *display,
            valle_motion::math_formula::FormulaStyle::default(),
            registry,
            &policy,
        )
        .with_context(|| format!("collect formula font closure for node `{}`", node.key))?;
        used_families.extend(fragment.list.fonts.iter().map(|font| font.family.clone()));
    }

    let mut matched_families = BTreeSet::new();
    let mut blobs = Vec::new();
    for (face, bytes) in valle_motion::math_formula::formula_font_pack() {
        let family = registry
            .get(face.ratex_name)
            .with_context(|| format!("resolve locked formula face `{}`", face.ratex_name))?
            .family
            .clone();
        if used_families.contains(&family) {
            matched_families.insert(family);
            blobs.push(bytes.to_vec());
        }
    }
    if matched_families != used_families {
        let missing = used_families
            .difference(&matched_families)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        bail!("formula emitter selected faces outside the locked font pack: {missing}");
    }
    Ok(blobs)
}

fn deduplicate_font_blobs(blobs: &mut Vec<Vec<u8>>) {
    let mut identities = std::collections::BTreeSet::new();
    blobs.retain(|bytes| identities.insert(ContentDigest::of_bytes(bytes)));
}

pub(crate) fn install_default_motion_font_files(
    files: &mut BTreeMap<String, PathBuf>,
    cache_root: &Path,
) -> Result<()> {
    let dir = cache_root.join("default-motion-fonts");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    for (bytes, name) in valle_motion::DEFAULT_MOTION_FONT_WEIGHTS
        .iter()
        .zip(valle_motion::DEFAULT_MOTION_FONT_FILES)
    {
        let path = dir.join(name);
        let stale = path
            .metadata()
            .map(|meta| meta.len() as usize != bytes.len())
            .unwrap_or(true);
        if stale {
            std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
        }
        files.insert(format!("runtime/fonts/{name}"), path);
    }
    Ok(())
}

/// Mount the complete built-in Motion font closure for every browser host.
///
/// Keeping this as one operation is intentional: render and Studio use the same Motion
/// configuration. A host that exposes only the base runtime font can
/// pass startup validation and still fail later when a real weight, fallback face or formula is
/// first requested.
pub(crate) fn install_motion_runtime_font_files(
    files: &mut BTreeMap<String, PathBuf>,
    cache_root: &Path,
) -> Result<()> {
    install_default_motion_font_files(files, cache_root)?;
    install_formula_font_files(files, cache_root)
}

pub(crate) fn install_formula_font_files(
    files: &mut BTreeMap<String, PathBuf>,
    cache_root: &Path,
) -> Result<()> {
    let dir = cache_root.join("formula-fonts");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    for (face, bytes) in valle_motion::math_formula::formula_font_pack() {
        let path = dir.join(face.file_name);
        let stale = path
            .metadata()
            .map(|meta| meta.len() as usize != bytes.len())
            .unwrap_or(true);
        if stale {
            std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
        }
        files.insert(format!("runtime/fonts/katex/{}", face.file_name), path);
    }
    Ok(())
}

pub(super) fn compile_timeline_component(
    input: &Path,
    assets: &[String],
    canvas: (u32, u32),
) -> Result<PreparedInput> {
    let size = MotionViewport {
        width: canvas.0,
        height: canvas.1,
    };
    compile_and_prepare(input, assets, &load_fonts(&[])?, size, None)?
        .map_err(|_| anyhow!("Motion component compilation failed"))
}
