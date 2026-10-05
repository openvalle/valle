//! `valle motion check/render/studio` —— Motion JSX artifact authoring tools.
//!
//! Batch authoring and Studio share one compilation and capability-admission implementation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use super::render_delivery::{DeliveryObservation, DeliveryPlan};
use crate::{
    MotionAction, MotionBindingArgs, MotionRenderTuningArgs, RenderOutputArgs, RenderVideoCodec,
};
use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use valle_motion::{
    ArtifactEnvelope, BuildFingerprint, ContentDigest, MotionViewport, NodeKind, ResourceRef,
    canonical_bytes,
};
use valle_timeline::{
    time::{FrameRate, RationalTime},
    wire::timeline::TimelineTimeWire,
};

pub fn run(action: MotionAction) -> Result<std::process::ExitCode> {
    match action {
        MotionAction::Check {
            input,
            host_duration,
            host_size,
            assets,
            bindings,
            data,
            font,
            frame,
            fps,
            ..
        } => {
            let temp = tempfile::tempdir()?;
            render(
                &input,
                host_duration.as_deref(),
                host_size,
                RenderOutputArgs {
                    output: Some(temp.path().join("check.png")),
                    frame: Some(frame),
                    ..Default::default()
                },
                &assets,
                &font,
                data.as_deref(),
                &bindings,
                valle_render::executor::skia::SkiaBackendKind::Raster,
                MotionRenderTuningArgs {
                    fps,
                    ..Default::default()
                },
                true,
            )
        }
        MotionAction::Review {
            input,
            host_duration,
            host_size,
            assets,
            bindings,
            data,
            font,
            fps,
            max_frames,
            trajectories,
            trajectory_sheet,
        } => review(
            &input,
            host_duration.as_deref(),
            host_size,
            &assets,
            &font,
            data.as_deref(),
            &bindings,
            fps.as_deref(),
            max_frames,
            trajectories,
            trajectory_sheet.as_deref(),
        ),
        MotionAction::Render {
            input,
            host_duration,
            host_size,
            delivery,
            backend,
            tuning,
            assets,
            bindings,
            font,
            data,
            ..
        } => render(
            &input,
            host_duration.as_deref(),
            host_size,
            delivery,
            &assets,
            &font,
            data.as_deref(),
            &bindings,
            backend.into(),
            tuning,
            false,
        ),
        MotionAction::Studio {
            input,
            fps,
            assets,
            bindings,
            font,
            data,
            web_assets_dir,
            port,
            ..
        } => studio(StudioRequest {
            input,
            fps,
            preview_files: Arc::new(Default::default()),
            asset_specs: assets,
            bindings,
            fonts: font,
            data,
            web_assets_dir,
            port,
        }),
    }
}

fn review(
    input: &Path,
    host_duration: Option<&str>,
    host_size: Option<(u32, u32)>,
    asset_specs: &[String],
    font_paths: &[PathBuf],
    data: Option<&Path>,
    bindings: &MotionBindingArgs,
    output_fps: Option<&str>,
    max_frames: Option<u32>,
    trajectories: bool,
    trajectory_sheet: Option<&Path>,
) -> Result<std::process::ExitCode> {
    let explicit_fonts = read_font_files(font_paths)?;
    let font_blobs = authoring_font_blobs(&explicit_fonts);
    let prepared = match compile_and_prepare(input, asset_specs, &font_blobs, data, None, true)? {
        Ok(prepared) => prepared,
        Err(()) => return Ok(std::process::ExitCode::FAILURE),
    };
    let artifact = &prepared.compiled.artifact;
    let delivery = Delivery::of(artifact, output_fps)?
        .with_host_duration(host_duration)?
        .with_host_size(host_size, artifact)?
        .with_caption_data(&prepared)?;
    let prop_bindings = read_prop_bindings(bindings.props.as_deref())?;
    let overrides = review_prop_overrides(artifact, &prop_bindings)?;
    let props = valle_motion::resolve_props(&artifact.controls, &overrides)?;
    let mut fonts = valle_motion::Fonts::default();
    for bytes in fixed_package_font_blobs(artifact, &explicit_fonts)? {
        fonts.register(valle_motion::motion_font_resource(
            valle_motion::FontSource::from_shared(Arc::new(bytes)),
        ))?;
    }
    for (name, asset) in &prepared.assets {
        if artifact
            .controls
            .assets
            .get(name)
            .is_some_and(|control| control.kind == valle_motion::AssetKind::Font)
        {
            fonts.register(
                valle_motion::motion_font_resource(valle_motion::FontSource::from_shared(
                    Arc::new(asset.bytes.clone()),
                ))
                .override_info(valle_motion::FontOverride {
                    family_name: Some(valle_motion::font_family_alias(&asset.hash).into()),
                    ..Default::default()
                }),
            )?;
        }
    }
    let styles = valle_motion::StyleCache::new();
    let opts = valle_motion::LayoutOptions {
        viewport: valle_motion::Viewport::new(delivery.canvas.tuple()),
        fonts: &fonts,
        styles: Some(&styles),
    };
    let report = valle_motion::review_motion(
        &prepared.scene,
        &props,
        &opts,
        delivery.fps,
        delivery.duration_frames,
        delivery.duration,
        max_frames.unwrap_or(delivery.duration_frames.max(2)),
        trajectories || trajectory_sheet.is_some(),
    )
    .map_err(|error| anyhow!(error))?;
    let mut output = serde_json::json!({
        "status": "ok",
        "component": artifact.component,
        "fps": format!("{}/{}", delivery.fps.numerator(), delivery.fps.denominator()),
        "framesAnalyzed": report.frames_analyzed,
        "nodesAnalyzed": report.nodes_analyzed,
        "issues": report.issues,
    });
    if trajectories {
        output["trajectories"] = serde_json::json!(report.trajectories);
    }
    if let Some(path) = trajectory_sheet {
        output["trajectorySheet"] = trajectory_sheet::render(
            path,
            artifact,
            &prepared.assets,
            &explicit_fonts,
            &prop_bindings,
            &report.trajectories,
            &delivery,
        )?;
    }
    crate::output::emit(output);
    Ok(std::process::ExitCode::SUCCESS)
}

mod trajectory_sheet;

/// Compile once and hand the same verified package used by Studio to Native delivery.
#[allow(clippy::too_many_arguments)]
fn render(
    input: &Path,
    host_duration: Option<&str>,
    host_size: Option<(u32, u32)>,
    output: RenderOutputArgs,
    asset_specs: &[String],
    font_paths: &[PathBuf],
    data: Option<&Path>,
    bindings: &MotionBindingArgs,
    backend: valle_render::executor::skia::SkiaBackendKind,
    tuning: MotionRenderTuningArgs,
    checking: bool,
) -> Result<std::process::ExitCode> {
    use valle_render::host::{
        NativeProject, NativeRenderOptions, NativeRenderer, NativeResourceCatalog,
    };

    if let Some((width, height)) = tuning.output_size {
        validate_pixel_size(MotionViewport::new(width, height), "output")?;
    }
    let frame = output.frame;
    let plan = DeliveryPlan::new(output)?;
    if plan.codec != Some(RenderVideoCodec::H264)
        && (tuning.hardware_encode || tuning.bitrate.is_some() || tuning.encode_threads.is_some())
    {
        bail!("H.264 encoder options require .mp4 output");
    }
    let observation = DeliveryObservation::new();
    let explicit_fonts = read_font_files(font_paths)?;
    let fonts = authoring_font_blobs(&explicit_fonts);
    let prepared = match compile_and_prepare(input, asset_specs, &fonts, data, None, true)? {
        Ok(prepared) => prepared,
        Err(()) => return Ok(std::process::ExitCode::FAILURE),
    };
    let artifact = &prepared.compiled.artifact;
    let delivery = Delivery::of(artifact, tuning.fps.as_deref())?
        .with_host_duration(host_duration)?
        .with_host_size(host_size, artifact)?
        .with_caption_data(&prepared)?;
    let (duration, fps, canvas) = (delivery.duration, delivery.fps, delivery.canvas);
    if let Some((width, height)) = tuning.output_size {
        // Delivery scaling must preserve the composition's aspect ratio: another shape is another
        // layout, and layout comes from the composition.
        if u64::from(width) * u64::from(canvas.height)
            != u64::from(height) * u64::from(canvas.width)
        {
            bail!(
                "--output-size must scale the composition canvas {}x{} proportionally; {width}x{height} changes the aspect ratio",
                canvas.width,
                canvas.height
            );
        }
    }
    let props = read_prop_bindings(bindings.props.as_deref())?;
    let font_blobs = fixed_package_font_blobs(artifact, &explicit_fonts)?;
    let package = super::motion_package::build_standalone_motion_package(
        super::motion_package::StandaloneMotionPackageInput {
            artifact,
            assets: &prepared.assets,
            font_blobs: &font_blobs,
            prop_bindings: &props,
            duration,
            frame_rate: fps,
            canvas: canvas.tuple(),
        },
    )?;
    // The builder already admitted and opened the closed package; reuse it.
    let opened = package.opened;
    // Freeze resources for the job; streaming decoders require stable files.
    let mut catalog = NativeResourceCatalog::new();
    let frozen = tempfile::tempdir().context("creating immutable render resources")?;
    for bytes in &font_blobs {
        catalog.admit_bytes(bytes.clone());
    }
    for asset in prepared.assets.values() {
        let path = frozen.path().join(asset.hash.as_hex());
        std::fs::write(&path, &asset.bytes)?;
        catalog.insert_file(asset.hash, path);
    }
    let project = NativeProject::from_render(opened.engine_render(), Arc::new(catalog));
    let renderer = NativeRenderer::new(
        project,
        NativeRenderOptions {
            backend,
            background: if plan.codec == Some(RenderVideoCodec::H264) {
                valle_engine::resource::OutputBackground::opaque_srgb([0, 0, 0])
            } else {
                valle_engine::resource::OutputBackground::Transparent
            },
            raster_workers: tuning.workers.map(usize::from),
            hardware_encode: tuning.hardware_encode,
            bitrate: tuning.bitrate.map(|value| value as usize),
            encode_threads: tuning.encode_threads.map(usize::from),
            output_size: tuning.output_size,
            progress: crate::output::render_progress(),
            ..NativeRenderOptions::default()
        },
    );
    if checking {
        let (summary, _) = plan
            .execute(&renderer)
            .map_err(|error| source_error(error, &prepared.compiled))?;
        let checked_frame = frame.unwrap_or(0);
        let host_frame =
            u32::try_from(checked_frame).context("checked frame must be nonnegative")?;
        let host = valle_motion::MotionHostContext::new(
            valle_motion::time::sample_time_at_frame(i64::from(host_frame), fps)
                .map_err(|error| anyhow!(error))?,
            duration,
            host_frame,
            delivery.duration_frames,
        )
        .context("checked frame is outside the host duration")?;
        let motion_context = valle_motion::motion_context_at_host(
            artifact.role,
            artifact
                .composition
                .as_ref()
                .expect("validated composition")
                .duration()
                .map_err(|error| anyhow!(error))?,
            host,
            fps,
        )
        .context("checked frame is outside the template")?;
        let evaluation = prepared.scene.frame_evaluation_stats(&motion_context);
        let compilation = observation.trace.metrics();
        crate::output::emit(
            serde_json::json!({"status":"ok", "component":artifact.component,
            "warnings":prepared.compiled.warnings,
            "nodes":artifact.nodes.len(),
            "expressions":artifact.exprs.len() + artifact.instance_groups.iter().map(|group| group.exprs.len()).sum::<usize>(),
            "fullEvaluationExpressions":artifact.exprs.len() + artifact.instance_groups.iter().map(valle_motion::InstanceGroup::frame_expression_evaluations).sum::<usize>(),
            "frame":checked_frame,
            "templates":artifact.instance_groups.len(),
            "instanceRows":artifact.instance_groups.iter().map(valle_motion::InstanceGroup::rows).sum::<usize>(),
            "evaluatedExpressions":evaluation.evaluated_expressions,
            "activeNodes":evaluation.active_nodes,
            "layoutNodes":evaluation.layout_nodes,
            "compilations": compilation.compilations,
            "timings": {
                "compile": compilation.elapsed.as_secs_f64() * 1000.0,
                "templateCompile": compilation.template_compile.as_secs_f64() * 1000.0,
                "instanceData": compilation.instance_data.as_secs_f64() * 1000.0,
                "framePrepare": summary.pipeline.as_ref().map(|pipeline| pipeline.evaluate_prepare_us as f64 / 1000.0),
            }}),
        );
    } else {
        plan.deliver_and_report(
            &renderer,
            &opened,
            &observation,
            Some(serde_json::json!({
                "warnings": prepared.compiled.warnings,
                "targetDurationSeconds": duration.as_f64(),
                "fps": format!("{}/{}", fps.numerator(), fps.denominator()),
                "totalFrames": delivery.duration_frames,
                "actualDurationSeconds": f64::from(delivery.duration_frames) * f64::from(fps.denominator()) / fps.numerator() as f64,
            })),
        ).map_err(|error| source_error(error,&prepared.compiled))?;
    }
    Ok(std::process::ExitCode::SUCCESS)
}

/// Runtime artifacts intentionally contain no source text. Attach the source
/// map at the authoring boundary, where the same frozen compilation is present.
fn source_error(
    error: anyhow::Error,
    compiled: &valle_compiler::motion::CompiledMotion,
) -> anyhow::Error {
    let message = format!("{error:#}");
    let expression = message
        .split("expression ")
        .nth(1)
        .and_then(|s| s.split_once(':'))
        .and_then(|(id, _)| id.parse::<u32>().ok());
    let instance = message
        .split("instance group ")
        .nth(1)
        .and_then(|s| s.split_once(':'))
        .and_then(|(id, _)| id.parse::<usize>().ok());
    let expressions = match instance {
        Some(group) => compiled
            .source_map
            .instance_exprs
            .get(group)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        None => &compiled.source_map.exprs,
    };
    let location = expression
        .and_then(|id| expressions.iter().find(|m| m.id.0 == id))
        .map(|m| (&m.source_path, &m.span))
        .or_else(|| {
            compiled
                .source_map
                .nodes
                .iter()
                .find(|node| {
                    message.contains(&format!("node `{}`", node.key))
                        || message.contains(&format!("Transition '{}'", node.key))
                })
                .map(|m| (&m.source_path, &m.span))
        });
    if let Some((path, span)) = location {
        anyhow!("{message} ({path}:{}:{})", span.line, span.column)
    } else {
        error
    }
}

#[derive(Clone)]
struct StudioRequest {
    input: PathBuf,
    fps: Option<String>,
    preview_files: Arc<crate::preview_store::PreviewStore>,
    asset_specs: Vec<String>,
    bindings: MotionBindingArgs,
    fonts: Vec<PathBuf>,
    data: Option<PathBuf>,
    web_assets_dir: Option<PathBuf>,
    port: u16,
}

fn studio(request: StudioRequest) -> Result<std::process::ExitCode> {
    validate_studio_request(&request)?;

    let runtime = crate::webruntime::resolve(request.web_assets_dir.as_deref())?;
    let generation = Arc::new(AtomicU64::new(1));
    let mut source_token = [0_u8; 32];
    getrandom::fill(&mut source_token).context("generating Motion Studio source token")?;
    let source_token = hex::encode(source_token);
    let config = studio_state_json(&request, 1)?;

    let mut runtime_files = runtime.serving_map();
    let mut local_runtime_files = BTreeMap::new();
    for (name, asset) in load_assets(&request.asset_specs)? {
        local_runtime_files.insert(format!("motion-assets/{name}"), asset.path);
    }
    for (index, path) in request.fonts.iter().enumerate() {
        local_runtime_files.insert(format!("motion-fonts/{index}"), path.clone());
    }
    mount_motion_runtime_fonts(&mut runtime_files)?;
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
        timeline_file: None,
        project: None,
        source_paths: std::sync::Mutex::new(Default::default()),
        last_report: RwLock::new(None),
        preview_files: Arc::clone(&request.preview_files),
        motion_source: Some(crate::webhost::MotionSourceCtx {
            token: source_token,
            input: request.input.clone(),
            data: request.data.clone(),
        }),
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
        project_id: None,
        revision: None,
    });
    eprintln!(
        "motion studio: {url} (input {}; Ctrl-C to stop)",
        request.input.display()
    );
    crate::webhost::serve_forever(server, state)?;
    Ok(std::process::ExitCode::SUCCESS)
}

#[derive(Debug, Clone)]
pub(super) struct BoundAsset {
    pub(super) path: PathBuf,
    pub(super) bytes: Arc<[u8]>,
    pub(super) hash: ContentDigest,
}

pub(crate) struct PreparedInput {
    scene: valle_motion::PreparedScene,
    pub(super) compiled: valle_compiler::motion::CompiledMotion,
    data_binding: Option<valle_compiler::motion::PrepareDataBinding>,
    pub(super) assets: BTreeMap<String, BoundAsset>,
    pub(super) shaders: valle_motion::shader::ShaderRegistry,
    canvas_size: MotionViewport,
}

/// Source closure and asset bytes captured before a Timeline save starts compiling.
/// Compilation may clone these values, but never reopens the author paths.
pub(crate) struct CapturedTimelineComponent {
    pub(crate) placement: Option<valle_compiler::motion::MotionPlacement>,
    graph: valle_compiler::motion::MotionModuleGraph,
    assets: BTreeMap<String, BoundAsset>,
    data_binding: Option<valle_compiler::motion::PrepareDataBinding>,
    dependencies: Vec<(PathBuf, ContentDigest)>,
}

impl CapturedTimelineComponent {
    pub(crate) fn dependency_digests(&self) -> impl Iterator<Item = (&Path, ContentDigest)> {
        self.dependencies
            .iter()
            .map(|(path, digest)| (path.as_path(), *digest))
    }

    pub(crate) fn verify_dependencies(&self) -> Result<()> {
        for (path, expected) in &self.dependencies {
            let bytes = std::fs::read(path)
                .with_context(|| format!("reading Motion dependency {}", path.display()))?;
            if ContentDigest::of_bytes(&bytes) != *expected {
                bail!("Motion dependency changed during save: {}", path.display());
            }
        }
        Ok(())
    }
}

pub(crate) fn capture_timeline_component(
    input: &Path,
    asset_specs: &[String],
    data: Option<&serde_json::Value>,
) -> Result<CapturedTimelineComponent> {
    let graph = load_motion_module_graph(input)?;
    let root = input.parent().unwrap_or_else(|| Path::new("."));
    let mut dependencies = Vec::new();
    for (module, source) in &graph.modules {
        let path = root.join(module);
        let bytes = std::fs::read(&path)
            .with_context(|| format!("reading Motion module {}", path.display()))?;
        if bytes != source.as_bytes() {
            bail!("Motion module changed during capture: {}", path.display());
        }
        dependencies.push((path, ContentDigest::of_bytes(&bytes)));
    }
    let mut assets = BTreeMap::new();
    for spec in asset_specs {
        let (name, path) = spec
            .split_once('=')
            .ok_or_else(|| anyhow!("asset binding `{spec}` must use NAME=PATH"))?;
        if name.is_empty() || path.is_empty() {
            bail!("asset binding `{spec}` must use non-empty NAME=PATH");
        }
        if assets.contains_key(name) {
            bail!("asset control `{name}` is bound more than once");
        }
        let path = PathBuf::from(path);
        let (asset, asset_dependencies) = load_asset_with_dependencies(&path)?;
        dependencies.extend(asset_dependencies);
        assets.insert(name.to_owned(), asset);
    }
    let captured = CapturedTimelineComponent {
        placement: None,
        graph,
        assets,
        data_binding: data.map(|value| valle_compiler::motion::PrepareDataBinding {
            source: "timeline-inline".into(),
            value: value.clone(),
        }),
        dependencies,
    };
    captured.verify_dependencies()?;
    Ok(captured)
}

pub(crate) fn compile_captured_timeline_component(
    captured: &CapturedTimelineComponent,
) -> Result<PreparedInput> {
    let mut assets = captured.assets.clone();
    let shaders = shader_registry(&assets)?;
    let resources = assets
        .iter()
        .map(|(control, asset)| ResourceRef {
            control: control.clone(),
            content_hash: asset.hash.clone(),
        })
        .collect::<Vec<_>>();
    let fonts = load_fonts(&[])?;
    let compiled = compile_with_font_assets(
        &captured.graph,
        &resources,
        &mut assets,
        &fonts,
        &shaders,
        captured.data_binding.as_ref(),
        captured.placement.as_ref(),
    )?
    .map_err(|diagnostics| {
        anyhow!(
            "Motion compilation failed: {}",
            diagnostics
                .into_iter()
                .map(|diagnostic| {
                    format!(
                        "{}: {}",
                        diagnostic
                            .source_path
                            .as_deref()
                            .unwrap_or(&captured.graph.entry),
                        diagnostic.message,
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    finish_prepared_input(compiled, captured.data_binding.clone(), assets, shaders)
}

fn finish_prepared_input(
    compiled: valle_compiler::motion::CompiledMotion,
    data_binding: Option<valle_compiler::motion::PrepareDataBinding>,
    assets: BTreeMap<String, BoundAsset>,
    shaders: valle_motion::shader::ShaderRegistry,
) -> Result<PreparedInput> {
    ensure_rendered_assets_are_bound(&compiled.artifact, &assets)?;
    compiled
        .artifact
        .validate_with_shaders(&shaders)
        .map_err(|errors| {
            anyhow!("compiler produced a scene that failed shader admission: {errors:?}")
        })?;
    let scene = valle_motion::prepare_scene(&compiled.artifact).map_err(|error| {
        anyhow!("compiler produced a scene that failed capability admission: {error}")
    })?;
    let canvas_size = compiled
        .artifact
        .composition
        .as_ref()
        .ok_or_else(|| anyhow!("Motion entry requires composition metadata"))?
        .viewport();
    Ok(PreparedInput {
        scene,
        compiled,
        data_binding,
        assets,
        shaders,
        canvas_size,
    })
}

fn compile_with_font_assets(
    graph: &valle_compiler::motion::MotionModuleGraph,
    resources: &[ResourceRef],
    assets: &mut BTreeMap<String, BoundAsset>,
    font_blobs: &[Arc<[u8]>],
    shaders: &valle_motion::shader::ShaderRegistry,
    data: Option<&valle_compiler::motion::PrepareDataBinding>,
    placement: Option<&valle_compiler::motion::MotionPlacement>,
) -> Result<
    Result<valle_compiler::motion::CompiledMotion, Vec<valle_compiler::motion::CompilerDiagnostic>>,
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
    // The compiler binds measurement to the entry composition before module constants run.
    let measure = valle_compiler::motion::MeasureEnv::new_unbound_shared_with_aliases(
        font_blobs,
        &measure_aliases,
    )
    .map_err(|diagnostic| anyhow!("{}", diagnostic.message))?;
    // The compiler's resolver carries string diagnostics. Keep dependency failures typed so
    // authoring commands can report a missing runtime instead of a source compilation failure.
    let audio_runtime_error = Arc::new(Mutex::new(None));
    let audio = if graph
        .modules
        .values()
        .any(|source| source.contains("audioAnalysis"))
    {
        let captured = assets.clone();
        let runtime_error = Arc::clone(&audio_runtime_error);
        Some(valle_compiler::motion::AudioAnalysisEnv::with_resolver(
            move |control| {
                let asset = captured
                    .get(control)
                    .ok_or_else(|| format!("asset `{control}` is not bound"))?;
                let frozen = tempfile::tempdir().map_err(|error| error.to_string())?;
                let path = frozen.path().join(asset.hash.as_hex());
                std::fs::write(&path, &asset.bytes).map_err(|error| error.to_string())?;
                let samples =
                    valle_media::codec::decode_audio_mono_f32(&path, 48_000).map_err(|error| {
                        let message = error.to_string();
                        if crate::ffmpeg_unavailable(&error) {
                            *runtime_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(
                                error.context(format!("loading audioAnalysis asset `{control}`")),
                            );
                        }
                        message
                    })?;
                Ok(valle_compiler::motion::AudioPcm {
                    content_hash: asset.hash,
                    sample_rate: 48_000,
                    samples,
                })
            },
        ))
    } else {
        None
    };
    let compilation =
        valle_compiler::motion::compile_motion_modules_with_full_env_and_data_and_audio(
            graph,
            resources,
            Some(&measure),
            Some(shaders),
            data,
            audio.as_ref(),
            placement,
        );
    if let Some(error) = audio_runtime_error
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
    {
        return Err(error);
    }
    let mut compiled = match compilation {
        Ok(compiled) => compiled,
        Err(diagnostics) => return Ok(Err(diagnostics)),
    };
    for (control, schema) in &compiled.artifact.controls.assets {
        if schema.kind == valle_motion::AssetKind::Environment {
            if let Some(asset) = assets.get_mut(control) {
                let environment = valle_motion::scene3d::EnvironmentAsset::from_encoded(
                    &asset.bytes,
                    Default::default(),
                )?;
                asset.bytes = environment.frozen_bytes()?.into();
                asset.hash = environment.content_digest();
                for resource in &mut compiled.artifact.resource_refs {
                    if resource.control == *control {
                        resource.content_hash = asset.hash;
                    }
                }
            }
        }
    }
    compiled
        .artifact
        .validate()
        .map_err(|e| anyhow!("invalid frozen Motion asset bindings: {e:?}"))?;
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
    }
    Ok(Ok(compiled))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FingerprintInputs<'a> {
    assets: BTreeMap<&'a str, &'a ContentDigest>,
    fonts: Vec<&'a ContentDigest>,
}

fn studio_state_json(request: &StudioRequest, generation: u64) -> Result<String> {
    let mut state: serde_json::Value =
        serde_json::from_str(&studio_native_state_json(request, generation)?)?;
    let data =
        load_prepare_data(&request.input, request.data.as_deref())?.map(|binding| binding.value);
    state["authorInputs"] = serde_json::json!({
        "fpsOverride": request.fps,
        "props": read_prop_bindings(request.bindings.props.as_deref())?,
        "data": data,
        "dataPath": request.data,
        "assetSpecs": request.asset_specs,
        "inputDigests": motion_watch_paths(request).iter().map(|path| -> Result<(String, ContentDigest)> {
            let path = path.canonicalize()?;
            Ok((path.to_string_lossy().into_owned(), ContentDigest::of_bytes(&std::fs::read(&path)?)))
        }).collect::<Result<BTreeMap<_, _>>>()?,
        "extraFonts": (0..request.fonts.len())
            .map(|index| format!("/motion-fonts/{index}"))
            .collect::<Vec<_>>(),
        "fontUrls": (0..request.fonts.len())
            .map(|index| serde_json::json!({"url":format!("/motion-fonts/{index}"),"role":"font"}))
            .chain(valle_motion::DEFAULT_MOTION_FONT_FILES.iter().map(|name| {
                serde_json::json!({"url":format!("/runtime/fonts/{name}"),"role":"font"})
            }))
            .chain(valle_motion::math_formula::formula_font_pack().map(|(face, _)| {
                serde_json::json!({"url":format!("/runtime/fonts/katex/{}",face.file_name),"role":"formula-font"})
            }))
            .collect::<Vec<_>>(),
    });
    Ok(state.to_string())
}

fn studio_native_state_json(request: &StudioRequest, generation: u64) -> Result<String> {
    let module_graph = load_motion_module_graph(&request.input)?;
    let mut assets = load_assets(&request.asset_specs)?;
    let shaders = shader_registry(&assets)?;
    let resources = assets
        .iter()
        .map(|(control, asset)| ResourceRef {
            control: control.clone(),
            content_hash: asset.hash.clone(),
        })
        .collect::<Vec<_>>();
    // Hot reload uses the same fonts and source composition as Studio preview.
    let explicit_font_blobs = read_font_files(&request.fonts)?;
    let font_blobs = authoring_font_blobs(&explicit_font_blobs);
    let data_binding = load_prepare_data(&request.input, request.data.as_deref())?;
    let compiled = match compile_with_font_assets(
        &module_graph,
        &resources,
        &mut assets,
        &font_blobs,
        &shaders,
        data_binding.as_ref(),
        None,
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
    let scene = match valle_motion::prepare_scene(&compiled.artifact) {
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
    let delivery = Delivery::of(&compiled.artifact, request.fps.as_deref())?;
    let prepared = PreparedInput {
        scene,
        compiled,
        data_binding,
        assets,
        shaders,
        canvas_size: delivery.canvas,
    };
    let font_hashes = font_blobs
        .iter()
        .map(|bytes| ContentDigest::of_bytes(bytes))
        .collect::<Vec<_>>();
    let envelope = envelope_for(&prepared, &font_hashes)?;
    let artifact_digest =
        ContentDigest::of_bytes(&canonical_bytes(&prepared.compiled.artifact)?).to_wire();
    let duration_frames = delivery.duration_frames;
    let props = read_prop_bindings(request.bindings.props.as_deref())?;
    let frame_rate = delivery.fps;
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
            prop_bindings: &props,
            duration: delivery.duration,
            frame_rate,
            canvas: delivery.canvas.tuple(),
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
    // Serve the prepared bytes, including normalized environment maps, rather than mutable files.
    request
        .preview_files
        .files
        .write()
        .map_err(|_| anyhow!("preview files poisoned"))?
        .replace(
            prepared
                .assets
                .values()
                .map(|asset| {
                    (
                        asset.hash.to_string(),
                        crate::preview_store::PreviewFile::Bytes(Arc::clone(&asset.bytes)),
                    )
                })
                .collect(),
        );
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
                "url": format!("/preview-assets/{}", prepared.assets[name].hash),
            })
        })
        .collect::<Vec<_>>();
    let resource_locators = prepared
        .assets
        .keys()
        .map(|control| {
            serde_json::json!({
                "id": format!("asset:{control}"),
                "url": format!("/preview-assets/{}", prepared.assets[control].hash),
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
            "num": delivery.fps.numerator(),
            "den": delivery.fps.denominator(),
        },
        "viewport": {
            "width": delivery.canvas.width,
            "height": delivery.canvas.height,
        },
        "fixedPackageManifestJson": fixed_package.fixed_package_manifest_json,
        "timelineJson": fixed_package.timeline_json,
        "timeline": fixed_package.timeline,
        "resourceManifestJson": fixed_package.resource_manifest_json,
        "resourceManifest": fixed_package.resource_manifest,
        "verifiedBindingBundleJson": fixed_package.verified_binding_bundle_json,
        "diagnostics": prepared.compiled.warnings,
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
    paths.extend(request.bindings.props.iter().cloned());
    paths.extend(
        request
            .asset_specs
            .iter()
            .filter_map(|spec| spec.split_once('=').map(|(_, path)| PathBuf::from(path))),
    );
    for spec in &request.asset_specs {
        if let Some((_, path)) = spec.split_once('=') {
            if path.ends_with(".shader.json") {
                let path = Path::new(path);
                if let Ok(bytes) = std::fs::read(path)
                    && let Ok(manifest) = valle_motion::shader::ShaderManifest::parse(&bytes)
                    && manifest.validate_shape().is_ok()
                {
                    paths.push(
                        path.parent()
                            .unwrap_or_else(|| Path::new("."))
                            .join(manifest.entry),
                    );
                }
            }
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

pub(crate) fn motion_module_paths(input: &Path) -> Result<Vec<PathBuf>> {
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
    font_blobs: &[Arc<[u8]>],
    data: Option<&Path>,
    inline_data: Option<&valle_compiler::motion::PrepareDataBinding>,
    emit_diagnostics: bool,
) -> Result<Result<PreparedInput, ()>> {
    let perf = perf_enabled();
    let compile_started = perf.then(Instant::now);
    let module_graph = load_motion_module_graph(input)?;
    let mut assets = load_assets(asset_specs)?;
    let shaders = shader_registry(&assets)?;
    let resources = assets
        .iter()
        .map(|(control, asset)| ResourceRef {
            control: control.clone(),
            content_hash: asset.hash.clone(),
        })
        .collect::<Vec<_>>();
    let data_binding = if let Some(binding) = inline_data {
        if data.is_some() {
            bail!("Motion data cannot be bound from a file and inline at the same time");
        }
        Some(binding.clone())
    } else {
        load_prepare_data(input, data)?
    };
    let compiled = match compile_with_font_assets(
        &module_graph,
        &resources,
        &mut assets,
        font_blobs,
        &shaders,
        data_binding.as_ref(),
        None,
    )? {
        Ok(compiled) => compiled,
        Err(diagnostics) => {
            if !emit_diagnostics {
                bail!(
                    "{}: {}",
                    input.display(),
                    diagnostics
                        .iter()
                        .map(|diagnostic| format!(
                            "{}:{}: {}",
                            diagnostic.span.line, diagnostic.span.column, diagnostic.message
                        ))
                        .collect::<Vec<_>>()
                        .join("; ")
                );
            }
            if crate::output::machine() {
                crate::output::emit(serde_json::json!({
                    "status": "error",
                    "error": {
                        "code": "motion_compile_failed",
                        "message": "Motion compilation failed",
                        "input": input,
                        "diagnostics": diagnostics,
                    },
                }));
                return Ok(Err(()));
            }
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
                        valle_motion::DiagClass::Warning => "warning",
                    },
                    diagnostic.code,
                    diagnostic.message
                );
            }
            return Ok(Err(()));
        }
    };

    for warning in &compiled.warnings {
        let diagnostic_path = warning
            .source_path
            .as_deref()
            .map_or_else(|| input.display().to_string(), ToOwned::to_owned);
        eprintln!(
            "{}:{}:{} [warning:{:?}] {}",
            diagnostic_path, warning.span.line, warning.span.column, warning.code, warning.message
        );
    }

    let compile_elapsed = compile_started.map(|started| started.elapsed());
    let prepare_started = perf.then(Instant::now);
    let prepared = finish_prepared_input(compiled, data_binding, assets, shaders)?;
    if let (Some(compile), Some(prepare_started)) = (compile_elapsed, prepare_started) {
        eprintln!(
            "[valle motion prepare] compile+admit {:.3} ms; prepare {:.3} ms",
            compile.as_secs_f64() * 1_000.0,
            prepare_started.elapsed().as_secs_f64() * 1_000.0,
        );
    }
    Ok(Ok(prepared))
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
        assets.insert(name.to_string(), load_asset(Path::new(path))?);
    }
    Ok(assets)
}

pub(super) fn load_asset(path: &Path) -> Result<BoundAsset> {
    Ok(load_asset_with_dependencies(path)?.0)
}

pub(super) fn load_asset_with_dependencies(
    path: &Path,
) -> Result<(BoundAsset, Vec<(PathBuf, ContentDigest)>)> {
    let bytes = std::fs::read(path).with_context(|| format!("reading asset {}", path.display()))?;
    let mut dependencies = vec![(path.to_owned(), ContentDigest::of_bytes(&bytes))];
    let frozen = if path.to_string_lossy().ends_with(".shader.json") {
        valle_motion::shader::ShaderPackage::admit_with_resolver(&bytes, |entry| {
            let source_path = path.parent().unwrap_or_else(|| Path::new(".")).join(entry);
            let source = std::fs::read(&source_path)
                .map_err(|error| format!("reading {}: {error}", source_path.display()))?;
            dependencies.push((source_path, ContentDigest::of_bytes(&source)));
            Ok(source)
        })?
        .frozen_bytes()?
    } else {
        bytes
    };
    Ok((
        BoundAsset {
            path: path.to_owned(),
            hash: ContentDigest::of_bytes(&frozen),
            bytes: frozen.into(),
        },
        dependencies,
    ))
}

fn shader_registry(
    assets: &BTreeMap<String, BoundAsset>,
) -> Result<valle_motion::shader::ShaderRegistry> {
    let mut registry = valle_motion::shader::ShaderRegistry::new();
    for (control, asset) in assets {
        if asset.path.to_string_lossy().ends_with(".shader.json") {
            registry.register_asset(
                control,
                valle_motion::shader::ShaderPackage::from_frozen(&asset.bytes)?,
            )?;
        }
    }
    Ok(registry)
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
            NodeKind::GeometryBatch { batch } => match &batch.geometry {
                valle_motion::GeometryBatchGeometry::Image { source, .. } => {
                    ("atlas image", source)
                }
                _ => continue,
            },
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

fn validate_studio_request(_request: &StudioRequest) -> Result<()> {
    Ok(())
}

/// The delivery contract every rendering path reads back from the artifact.
///
/// The artifact defines the source canvas and duration. CLI output FPS and host duration
/// can be overridden without changing that source contract. An artifact without a contract
/// is an in-memory compile and cannot supply a delivery canvas.
#[derive(Clone, Copy)]
struct Delivery {
    duration: RationalTime,
    duration_frames: u32,
    fps: FrameRate,
    canvas: MotionViewport,
}

impl Delivery {
    fn of(artifact: &valle_motion::SceneArtifact, output_fps: Option<&str>) -> Result<Self> {
        let Some(composition) = artifact.composition.as_ref() else {
            bail!(
                "{}",
                format!(
                    "this artifact has no delivery contract; add to the entry file: {}",
                    valle_motion::COMPOSITION_TEMPLATE
                )
            );
        };
        let default_fps = composition
            .frame_rate()
            .map_err(|error| anyhow!("composition frame rate is invalid: {error}"))?;
        let fps = output_fps
            .map(parse_output_fps)
            .transpose()?
            .or(default_fps)
            .ok_or_else(|| anyhow!("provide --fps or composition.fps"))?;
        let duration = composition
            .duration()
            .map_err(|error| anyhow!("composition duration is invalid: {error}"))?;
        let canvas = validate_pixel_size(composition.viewport(), "canvas")?;
        Ok(Self {
            duration,
            duration_frames: composition
                .duration_frames(fps)
                .map_err(|error| anyhow!("composition duration cannot be quantized: {error}"))?,
            fps,
            canvas,
        })
    }

    fn with_host_size(
        mut self,
        size: Option<(u32, u32)>,
        artifact: &valle_motion::SceneArtifact,
    ) -> Result<Self> {
        artifact
            .role
            .validate(
                artifact
                    .composition
                    .as_ref()
                    .context("composition missing")?
                    .duration()?,
                Some(self.duration),
            )
            .map_err(|reason| anyhow!(reason))?;
        if let Some((width, height)) = size {
            if matches!(artifact.role, valle_timeline::MotionRole::Clip) {
                bail!(
                    "--host-size requires a template role; clip layout uses its composition canvas"
                )
            }
            self.canvas = validate_pixel_size(MotionViewport { width, height }, "host viewport")?;
        }
        Ok(self)
    }

    fn with_caption_data(self, prepared: &PreparedInput) -> Result<Self> {
        if matches!(
            prepared.compiled.artifact.role,
            valle_timeline::MotionRole::CaptionPresenter { .. }
        ) {
            let binding = prepared
                .data_binding
                .as_ref()
                .context("captionPresenter needs frozen caption input")?;
            let data: valle_motion::caption::CaptionPresenterData =
                serde_json::from_value(binding.value.clone())?;
            data.validate(Some(self.duration))
                .map_err(|reason| anyhow!("{}: {reason}", binding.source))?;
        }
        Ok(self)
    }

    fn with_host_duration(mut self, input: Option<&str>) -> Result<Self> {
        if let Some(input) = input {
            let duration = RationalTime::from_exact(TimelineTimeWire::new(input)?.to_exact());
            if !duration.is_positive() {
                bail!("--host-duration must be positive");
            }
            self.duration_frames = valle_motion::duration_frames(duration, self.fps)
                .context("--host-duration must produce at least one output frame")?;
            self.duration = duration;
        }
        Ok(self)
    }
}

fn parse_output_fps(input: &str) -> Result<FrameRate> {
    use valle_timeline::time::ExactRational;
    let exact = if input.contains('/') {
        ExactRational::parse_canonical(input)?
    } else {
        TimelineTimeWire::new(input)?.to_exact()
    };
    FrameRate::from_exact(exact).map_err(Into::into)
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

/// Check frame-zero layout and emission so compilation cannot silently accept text that produces no
/// glyphs.
fn load_fonts(paths: &[PathBuf]) -> Result<Vec<Arc<[u8]>>> {
    let explicit = read_font_files(paths)?;
    Ok(authoring_font_blobs(&explicit))
}

fn read_font_files(paths: &[PathBuf]) -> Result<Vec<Arc<[u8]>>> {
    paths
        .iter()
        .map(|path| {
            std::fs::read(path)
                .map(Arc::<[u8]>::from)
                .with_context(|| format!("reading font {}", path.display()))
        })
        .collect()
}

pub(super) fn shared_default_fonts() -> &'static [Arc<[u8]>] {
    static FONTS: std::sync::OnceLock<Vec<Arc<[u8]>>> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        valle_motion::default_motion_fonts()
            .iter()
            .map(|bytes| Arc::from(*bytes))
            .collect()
    })
}

fn authoring_font_blobs(explicit: &[Arc<[u8]>]) -> Vec<Arc<[u8]>> {
    // User faces are preferred; the built-in palette supplies missing families/scripts.
    let mut blobs = explicit.to_vec();
    blobs.extend(shared_default_fonts().iter().cloned());
    for (_, bytes) in valle_motion::math_formula::formula_font_pack() {
        blobs.push(Arc::<[u8]>::from(bytes));
    }
    deduplicate_font_blobs(&mut blobs);
    blobs
}

pub(super) fn fixed_package_font_blobs(
    artifact: &valle_motion::SceneArtifact,
    explicit: &[Arc<[u8]>],
) -> Result<Vec<Arc<[u8]>>> {
    let mut blobs = explicit.to_vec();
    blobs.extend(super::motion_fonts::selected_default_fonts(artifact));
    blobs.extend(used_formula_font_blobs(artifact)?);
    deduplicate_font_blobs(&mut blobs);
    Ok(blobs)
}

/// Exact, artifact-wide formula font closure for the immutable Studio fixed package.
///
/// `latex` and `display` are static in `SceneArtifact`. Formula `fontSize`, color, visibility,
/// Props may vary by frame, but they cannot change the RaTeX face selected for a glyph.
/// Inspecting the emitted `ProgramRecording` also avoids freezing Size faces for delimiters that
/// RaTeX already lowered to paths.
fn used_formula_font_blobs(artifact: &valle_motion::SceneArtifact) -> Result<Vec<Arc<[u8]>>> {
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
            blobs.push(Arc::<[u8]>::from(bytes));
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

/// Keep the first occurrence of each distinct font. Byte equality is the same identity as the
/// content digest, but it avoids hashing every built-in face (~28 MB) on each invocation:
/// different lengths compare in O(1) and the handful of fonts makes the pairwise scan trivial.
fn deduplicate_font_blobs(blobs: &mut Vec<Arc<[u8]>>) {
    let mut kept: Vec<Arc<[u8]>> = Vec::with_capacity(blobs.len());
    for blob in blobs.drain(..) {
        if !kept.contains(&blob) {
            kept.push(blob);
        }
    }
    *blobs = kept;
}

/// Serve every built-in Motion and formula font from memory, sharing the native font bytes.
pub(crate) fn mount_motion_runtime_fonts(
    files: &mut BTreeMap<String, crate::webruntime::HostedFile>,
) -> Result<()> {
    use crate::webruntime::HostedFile;
    let default = valle_motion::DEFAULT_MOTION_FONT_FILES
        .iter()
        .zip(valle_motion::DEFAULT_MOTION_FONT_WEIGHTS.iter())
        .map(|(name, bytes)| (format!("runtime/fonts/{name}"), *bytes));
    let formula = valle_motion::math_formula::formula_font_pack()
        .into_iter()
        .map(|(face, bytes)| (format!("runtime/fonts/katex/{}", face.file_name), bytes));
    for (route, bytes) in default.chain(formula) {
        if let Some(file) = files.get(&route) {
            if !matches!(file, HostedFile::VerifiedRuntime(existing) if existing.as_ref() == bytes)
            {
                bail!("built-in font conflicts with Web runtime asset: {route}");
            }
        } else {
            files.insert(route, HostedFile::VerifiedRuntime(Arc::from(bytes)));
        }
    }
    Ok(())
}

fn read_prop_bindings(path: Option<&Path>) -> Result<BTreeMap<String, serde_json::Value>> {
    path.map(|path| {
        serde_json::from_str(&super::read(path)?).context("decode Motion --props object")
    })
    .transpose()
    .map(Option::unwrap_or_default)
}

fn review_prop_overrides(
    artifact: &valle_motion::SceneArtifact,
    bindings: &BTreeMap<String, serde_json::Value>,
) -> Result<BTreeMap<String, valle_motion::MotionValue>> {
    fn numbers<const N: usize>(name: &str, raw: &serde_json::Value) -> Result<[f64; N]> {
        let values = raw
            .as_array()
            .filter(|values| values.len() == N)
            .ok_or_else(|| anyhow!("Motion prop `{name}` must be an array of {N} numbers"))?;
        let parsed = values
            .iter()
            .map(|value| value.as_f64().filter(|number| number.is_finite()))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| anyhow!("Motion prop `{name}` must contain finite numbers"))?;
        parsed
            .try_into()
            .map_err(|_| anyhow!("invalid Motion prop `{name}`"))
    }

    let mut overrides = BTreeMap::new();
    for (name, raw) in bindings {
        let control = &artifact
            .controls
            .props
            .get(name)
            .ok_or_else(|| anyhow!("unknown Motion prop `{name}`"))?
            .control;
        let (kind, payload) = match control {
            valle_motion::ControlType::Number { .. } => ("number", raw.clone()),
            valle_motion::ControlType::Length => {
                let value =
                    valle_motion::controls::scalar_binding(raw, control).ok_or_else(|| {
                        anyhow!("Motion prop `{name}` requires a finite pixel length")
                    })?;
                ("length", serde_json::json!({"value": value, "unit": "px"}))
            }
            valle_motion::ControlType::Angle => {
                let value = valle_motion::controls::scalar_binding(raw, control)
                    .ok_or_else(|| anyhow!("Motion prop `{name}` requires a finite CSS angle"))?;
                ("angle", serde_json::json!({"value": value, "unit": "deg"}))
            }
            valle_motion::ControlType::Point => {
                let [x, y] = numbers::<2>(name, raw)?;
                ("point", serde_json::json!({"x": x, "y": y}))
            }
            valle_motion::ControlType::Rect => {
                let [x, y, width, height] = numbers::<4>(name, raw)?;
                (
                    "rect",
                    serde_json::json!({"x": x, "y": y, "width": width, "height": height}),
                )
            }
            valle_motion::ControlType::Color => {
                let color = if let Some(color) = raw.as_str() {
                    color.to_owned()
                } else {
                    let values = raw
                        .as_array()
                        .filter(|values| values.len() == 4)
                        .ok_or_else(|| {
                            anyhow!("Motion prop `{name}` must be an RGBA array or CSS color")
                        })?;
                    let channels = values
                        .iter()
                        .map(|value| value.as_u64().filter(|channel| *channel <= 255))
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| {
                            anyhow!("Motion prop `{name}` must contain 8-bit RGBA channels")
                        })?;
                    format!(
                        "#{:02x}{:02x}{:02x}{:02x}",
                        channels[0], channels[1], channels[2], channels[3]
                    )
                };
                ("color", serde_json::Value::String(color))
            }
            valle_motion::ControlType::Bool => ("bool", raw.clone()),
            valle_motion::ControlType::String => ("str", raw.clone()),
            valle_motion::ControlType::Select { .. } => ("enum", raw.clone()),
        };
        let value = serde_json::from_value(serde_json::json!({"kind": kind, "value": payload}))
            .with_context(|| format!("Motion prop `{name}` has the wrong type"))?;
        overrides.insert(name.clone(), value);
    }
    Ok(overrides)
}

#[cfg(test)]
mod capture_tests {
    use super::*;

    #[test]
    fn captured_component_tracks_imports_and_bound_asset_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("card.motion.tsx");
        let imported = dir.path().join("theme.motion.ts");
        let asset = dir.path().join("logo.bin");
        std::fs::write(
            &entry,
            "import { accent } from './theme.motion'; export const composition = { width: 64, height: 64, duration: 1 }; export default function Card() { return <Scene />; }",
        ).unwrap();
        let original_import = "export const accent = '#123456';";
        std::fs::write(&imported, original_import).unwrap();
        std::fs::write(&asset, b"original asset").unwrap();
        let captured =
            capture_timeline_component(&entry, &[format!("logo={}", asset.display())], None)
                .unwrap();
        assert_eq!(captured.dependency_digests().count(), 3);
        captured.verify_dependencies().unwrap();
        std::fs::write(&imported, "export const accent = '#abcdef';").unwrap();
        assert!(
            captured
                .verify_dependencies()
                .unwrap_err()
                .to_string()
                .contains("theme.motion.ts")
        );
        std::fs::write(&imported, original_import).unwrap();
        std::fs::write(&asset, b"replacement asset").unwrap();
        assert!(
            captured
                .verify_dependencies()
                .unwrap_err()
                .to_string()
                .contains("logo.bin")
        );
    }
}
