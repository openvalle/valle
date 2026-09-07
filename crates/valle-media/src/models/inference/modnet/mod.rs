//! Typed MODNet host contract shared by the reference CLI and future Valle integration.

use std::fs;
use std::path::Path;
#[cfg(any(all(feature = "model-coreml", target_os = "macos"), test))]
use std::path::{Component, PathBuf};
use std::time::Instant;

pub use crate::models::runtime::RunReport;
#[cfg(any(
    feature = "model-modnet-onnx",
    all(feature = "model-coreml", target_os = "macos")
))]
use crate::models::runtime::TensorInput;
use crate::models::runtime::TensorOutput;
#[cfg(feature = "model-modnet-onnx")]
use crate::models::runtime::onnx::{OnnxSession, OnnxSessionOptions};
use crate::models::spec::{Artifact, Backend, Dimension, ModelManifest, Route};
use anyhow::{Context, Result, bail, ensure};
use image::GrayImage;
use serde::Deserialize;

pub const ADAPTER: &str = "modnet-image-matting";
pub const CONTRACT_VERSION: u32 = 1;
const INPUT_NAME: &str = "img";
const OUTPUT_NAME: &str = "matte";
const NORMALIZATION_SCALE: f32 = 2.0 / 255.0;
const NORMALIZATION_BIAS: f32 = -1.0;

pub struct RunRequest<'a> {
    pub manifest: &'a ModelManifest,
    pub route: &'a Route,
    pub artifact: &'a Artifact,
    pub artifact_root: &'a Path,
    pub compile_cache: &'a Path,
    pub input: &'a Path,
    pub output: &'a Path,
}

/// Immutable release selection used to construct one reusable model session.
#[derive(Clone, Copy)]
pub struct SessionRequest<'a> {
    pub manifest: &'a ModelManifest,
    pub route: &'a Route,
    pub artifact: &'a Artifact,
    pub artifact_root: &'a Path,
    /// Required for CoreML and ignored by ONNX. It must be outside `artifact_root`.
    pub compile_cache: Option<&'a Path>,
}

/// One-frame timing split. `load_ms` is the one-time session load cost and is
/// repeated in each result for provenance; `total_ms` covers only this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatteTimings {
    pub load_ms: f64,
    pub preprocess_ms: f64,
    pub inference_ms: f64,
    pub postprocess_ms: f64,
    pub total_ms: f64,
}

/// An alpha8 matte with the exact dimensions of its RGBA8 source frame.
#[derive(Debug, Clone, PartialEq)]
pub struct MatteFrame {
    pub width: u32,
    pub height: u32,
    pub alpha: Vec<u8>,
    pub timings: MatteTimings,
}

trait MatteBackend {
    fn infer(&mut self, input: &[f32], input_shape: [usize; 4]) -> Result<TensorOutput>;
}

/// A load-once MODNet graph plus its release-owned preprocessing contract.
pub struct MatteSession {
    backend: Box<dyn MatteBackend>,
    model_width: u32,
    model_height: u32,
    load_ms: f64,
}

#[derive(Debug, Deserialize)]
struct Preprocess {
    color_space: String,
    resize: Resize,
    normalization: Normalization,
}

#[derive(Debug, Deserialize)]
struct Resize {
    algorithm: String,
    width: u32,
    height: u32,
    dimension_multiple: u32,
}

#[derive(Debug, Deserialize)]
struct Normalization {
    scale: f32,
    bias: f32,
}

#[derive(Debug, Deserialize)]
struct Postprocess {
    clamp: [f32; 2],
    resize_to_source: String,
    encoding: String,
}

#[derive(Debug, Default, Deserialize)]
struct ArtifactMetadata {
    #[serde(default)]
    fixed_shape: bool,
    width: Option<u32>,
    height: Option<u32>,
    default_width: Option<u32>,
    default_height: Option<u32>,
}

#[cfg(feature = "model-modnet-onnx")]
#[derive(Debug, Default, Deserialize)]
struct OnnxRouteOptions {
    intra_threads: Option<usize>,
    memory_pattern: Option<bool>,
    prepacking: Option<bool>,
}

#[cfg(feature = "model-modnet-onnx")]
fn parse_onnx_route_options(value: &serde_json::Value) -> Result<OnnxRouteOptions> {
    if value.is_null() {
        Ok(OnnxRouteOptions::default())
    } else {
        serde_json::from_value(value.clone()).context("MODNet ONNX route options are invalid")
    }
}

#[cfg(all(feature = "model-coreml", target_os = "macos"))]
#[derive(Debug, Default, Deserialize)]
struct CoreMlOptions {
    compute_units: Option<String>,
}

impl MatteSession {
    /// Validate one explicit release route and load exactly its referenced graph.
    pub fn load(request: SessionRequest<'_>) -> Result<Self> {
        let load_started = Instant::now();
        validate_contract(request.manifest, request.route, request.artifact)?;
        let preprocess: Preprocess =
            serde_json::from_value(request.manifest.contract.preprocess.clone())
                .context("MODNet preprocess contract is invalid")?;
        let postprocess: Postprocess =
            serde_json::from_value(request.manifest.contract.postprocess.clone())
                .context("MODNet postprocess contract is invalid")?;
        validate_preprocess(&preprocess)?;
        validate_postprocess(&postprocess)?;
        let metadata: ArtifactMetadata = serde_json::from_value(request.artifact.metadata.clone())
            .context("MODNet artifact metadata is invalid")?;
        let (model_width, model_height) = resolve_model_dimensions(&preprocess, &metadata)?;
        let entrypoint = request.artifact_root.join(&request.artifact.entrypoint);
        ensure!(
            entrypoint.exists(),
            "MODNet model artifact is missing: {}",
            entrypoint.display()
        );
        let backend = load_backend(&request, &entrypoint)?;

        Ok(Self {
            backend,
            model_width,
            model_height,
            load_ms: elapsed_ms(load_started),
        })
    }

    pub const fn model_width(&self) -> u32 {
        self.model_width
    }

    pub const fn model_height(&self) -> u32 {
        self.model_height
    }

    pub const fn load_ms(&self) -> f64 {
        self.load_ms
    }

    /// Infer one tightly packed RGBA8 frame without reloading the selected graph.
    pub fn inference_rgba8(&mut self, width: u32, height: u32, rgba: &[u8]) -> Result<MatteFrame> {
        let total_started = Instant::now();
        validate_rgba8(width, height, rgba)?;

        let preprocess_started = Instant::now();
        let input = area_resize_rgba8_nchw(
            rgba,
            width,
            height,
            self.model_width,
            self.model_height,
            NORMALIZATION_SCALE,
            NORMALIZATION_BIAS,
        );
        let preprocess_ms = elapsed_ms(preprocess_started);

        let inference_started = Instant::now();
        let matte = self.backend.infer(
            &input,
            [1, 3, self.model_height as usize, self.model_width as usize],
        )?;
        let inference_ms = elapsed_ms(inference_started);
        validate_matte_output(&matte, self.model_width, self.model_height)?;

        let postprocess_started = Instant::now();
        let alpha = resize_matte_half_pixel(
            &matte.data,
            self.model_width,
            self.model_height,
            width,
            height,
        );
        let postprocess_ms = elapsed_ms(postprocess_started);
        Ok(MatteFrame {
            width,
            height,
            alpha,
            timings: MatteTimings {
                load_ms: self.load_ms,
                preprocess_ms,
                inference_ms,
                postprocess_ms,
                total_ms: elapsed_ms(total_started),
            },
        })
    }
}

fn load_backend(request: &SessionRequest<'_>, _entrypoint: &Path) -> Result<Box<dyn MatteBackend>> {
    match request.route.backend {
        Backend::OnnxCpu => {
            #[cfg(feature = "model-modnet-onnx")]
            {
                let route_options = parse_onnx_route_options(&request.route.options)?;
                Ok(Box::new(OnnxMatteBackend {
                    session: OnnxSession::load_with_options(
                        _entrypoint,
                        &OnnxSessionOptions {
                            intra_threads: route_options.intra_threads,
                            memory_pattern: route_options.memory_pattern.unwrap_or(true),
                            prepacking: route_options.prepacking.unwrap_or(true),
                        },
                    )?,
                }))
            }
            #[cfg(not(feature = "model-modnet-onnx"))]
            {
                bail!("MODNet ONNX route selected, but ONNX support is not compiled in")
            }
        }
        Backend::Coreml => {
            #[cfg(all(feature = "model-coreml", target_os = "macos"))]
            {
                Ok(Box::new(load_coreml_backend(request, _entrypoint)?))
            }
            #[cfg(all(feature = "model-coreml", not(target_os = "macos")))]
            {
                bail!("MODNet CoreML route selected, but CoreML is only available on macOS")
            }
            #[cfg(not(feature = "model-coreml"))]
            {
                bail!("MODNet CoreML route selected, but CoreML support is not compiled in")
            }
        }
        backend => bail!("MODNet adapter does not implement backend {backend}"),
    }
}

#[cfg(feature = "model-modnet-onnx")]
struct OnnxMatteBackend {
    session: OnnxSession,
}

#[cfg(feature = "model-modnet-onnx")]
impl MatteBackend for OnnxMatteBackend {
    fn infer(&mut self, input: &[f32], input_shape: [usize; 4]) -> Result<TensorOutput> {
        self.session.run_f32(
            TensorInput::borrowed(INPUT_NAME, input_shape, input),
            OUTPUT_NAME,
        )
    }
}

/// Compatibility file runner. All model semantics and graph execution go
/// through [`MatteSession`]; this wrapper only decodes and encodes files.
pub fn run(request: RunRequest<'_>) -> Result<RunReport> {
    validate_run_paths(request.input, request.output)?;
    let total_started = Instant::now();
    let mut session = MatteSession::load(SessionRequest {
        manifest: request.manifest,
        route: request.route,
        artifact: request.artifact,
        artifact_root: request.artifact_root,
        compile_cache: Some(request.compile_cache),
    })?;
    let model_width = session.model_width();
    let model_height = session.model_height();

    let decode_started = Instant::now();
    let source = image::open(request.input)
        .with_context(|| format!("failed to open input image {}", request.input.display()))?
        .to_rgba8();
    let decode_ms = elapsed_ms(decode_started);
    let frame = session.inference_rgba8(source.width(), source.height(), source.as_raw())?;
    let timings = frame.timings;

    let encode_started = Instant::now();
    if let Some(parent) = request
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let image = GrayImage::from_raw(frame.width, frame.height, frame.alpha)
        .context("failed to construct output alpha image")?;
    image
        .save(request.output)
        .with_context(|| format!("failed to save {}", request.output.display()))?;
    let encode_ms = elapsed_ms(encode_started);

    Ok(RunReport {
        model: request.manifest.model.id.clone(),
        version: request.manifest.model.version.clone(),
        backend: request.route.backend.to_string(),
        artifact: request.artifact.id.clone(),
        input: request.input.to_path_buf(),
        output: request.output.to_path_buf(),
        source_width: frame.width,
        source_height: frame.height,
        model_width,
        model_height,
        load_ms: timings.load_ms,
        preprocess_ms: decode_ms + timings.preprocess_ms,
        inference_ms: timings.inference_ms,
        postprocess_ms: timings.postprocess_ms + encode_ms,
        total_ms: elapsed_ms(total_started),
    })
}

fn validate_preprocess(preprocess: &Preprocess) -> Result<()> {
    ensure!(preprocess.color_space == "rgb", "MODNet expects RGB input");
    ensure!(
        preprocess.resize.algorithm == "area",
        "MODNet requires area resize, got {:?}",
        preprocess.resize.algorithm
    );
    ensure!(
        preprocess.resize.dimension_multiple > 0,
        "dimension_multiple must be positive"
    );
    ensure!(
        preprocess.normalization.scale == NORMALIZATION_SCALE
            && preprocess.normalization.bias == NORMALIZATION_BIAS,
        "MODNet requires (x / 255 - 0.5) / 0.5 normalization"
    );
    Ok(())
}

fn validate_postprocess(postprocess: &Postprocess) -> Result<()> {
    ensure!(
        postprocess.clamp == [0.0, 1.0]
            && postprocess.resize_to_source == "bilinear-half-pixel"
            && postprocess.encoding == "alpha8-png",
        "unsupported MODNet postprocess contract"
    );
    Ok(())
}

fn resolve_model_dimensions(
    preprocess: &Preprocess,
    metadata: &ArtifactMetadata,
) -> Result<(u32, u32)> {
    let model_width = metadata
        .width
        .or(metadata.default_width)
        .unwrap_or(preprocess.resize.width);
    let model_height = metadata
        .height
        .or(metadata.default_height)
        .unwrap_or(preprocess.resize.height);
    ensure!(
        model_width.is_multiple_of(preprocess.resize.dimension_multiple)
            && model_height.is_multiple_of(preprocess.resize.dimension_multiple),
        "MODNet dimensions {model_width}x{model_height} are not multiples of {}",
        preprocess.resize.dimension_multiple
    );
    if metadata.fixed_shape {
        ensure!(
            model_width == preprocess.resize.width && model_height == preprocess.resize.height,
            "fixed artifact dimensions disagree with the preprocessing contract"
        );
    }
    Ok((model_width, model_height))
}

fn validate_matte_output(matte: &TensorOutput, model_width: u32, model_height: u32) -> Result<()> {
    let expected_shape = vec![1, 1, model_height as usize, model_width as usize];
    ensure!(
        matte.name == OUTPUT_NAME,
        "MODNet backend returned output {:?}, expected {OUTPUT_NAME:?}",
        matte.name
    );
    ensure!(
        matte.shape == expected_shape,
        "MODNet output has shape {:?}, expected {:?}",
        matte.shape,
        expected_shape
    );
    ensure!(
        matte.data.len() == model_width as usize * model_height as usize,
        "MODNet output has {} values, expected {}",
        matte.data.len(),
        model_width as usize * model_height as usize
    );
    ensure!(
        matte.data.iter().all(|value| value.is_finite()),
        "MODNet output contains non-finite values"
    );
    Ok(())
}

fn validate_run_paths(input: &Path, output: &Path) -> Result<()> {
    ensure!(
        output
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("png")),
        "MODNet alpha output must use a .png extension"
    );
    if output.exists() {
        ensure!(
            fs::canonicalize(input)? != fs::canonicalize(output)?,
            "refusing to overwrite the input image with the alpha output"
        );
    }
    Ok(())
}

fn validate_rgba8(width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    ensure!(
        width > 0 && height > 0,
        "MODNet frame dimensions must be positive"
    );
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .context("MODNet RGBA8 frame dimensions overflow addressable memory")?;
    ensure!(
        rgba.len() == expected,
        "MODNet {width}x{height} RGBA8 frame requires {expected} bytes, got {}",
        rgba.len()
    );
    Ok(())
}

fn validate_contract(manifest: &ModelManifest, route: &Route, artifact: &Artifact) -> Result<()> {
    manifest.validate()?;
    ensure!(manifest.model.id == "modnet", "manifest is not MODNet");
    ensure!(
        manifest.contract.adapter == ADAPTER && manifest.contract.version == CONTRACT_VERSION,
        "unsupported MODNet adapter contract {}@{}",
        manifest.contract.adapter,
        manifest.contract.version
    );
    ensure!(
        manifest.contract.inputs.len() == 1 && manifest.contract.outputs.len() == 1,
        "MODNet contract must expose exactly one input and one output"
    );
    let input = &manifest.contract.inputs[0];
    let output = &manifest.contract.outputs[0];
    ensure!(
        input.name == INPUT_NAME
            && input.dtype == "f32"
            && input.layout == "nchw"
            && input.shape
                == vec![
                    Dimension::Fixed(1),
                    Dimension::Fixed(3),
                    Dimension::Symbol("height".into()),
                    Dimension::Symbol("width".into()),
                ],
        "MODNet adapter requires an img [1,3,height,width] f32 NCHW input"
    );
    ensure!(
        output.name == OUTPUT_NAME
            && output.dtype == "f32"
            && output.layout == "nchw"
            && output.shape
                == vec![
                    Dimension::Fixed(1),
                    Dimension::Fixed(1),
                    Dimension::Symbol("height".into()),
                    Dimension::Symbol("width".into()),
                ],
        "MODNet adapter requires a matte [1,1,height,width] f32 NCHW output"
    );
    ensure!(route.artifact == artifact.id, "route and artifact disagree");
    match route.backend {
        Backend::OnnxCpu => ensure!(artifact.format == "onnx", "ONNX route needs ONNX artifact"),
        Backend::Coreml => ensure!(
            artifact.format == "coreml-package",
            "CoreML route needs a CoreML package"
        ),
        backend => bail!("MODNet adapter does not implement backend {backend}"),
    }
    Ok(())
}

#[cfg(all(feature = "model-coreml", target_os = "macos"))]
struct CoreMlMatteBackend {
    session: crate::models::runtime::coreml::CoreMlSession,
}

#[cfg(all(feature = "model-coreml", target_os = "macos"))]
impl MatteBackend for CoreMlMatteBackend {
    fn infer(&mut self, input: &[f32], input_shape: [usize; 4]) -> Result<TensorOutput> {
        let mut outputs = self.session.run_f32(
            &[TensorInput::borrowed(INPUT_NAME, input_shape, input)],
            &[OUTPUT_NAME],
        )?;
        outputs
            .pop()
            .context("MODNet CoreML inference returned no matte output")
    }
}

#[cfg(all(feature = "model-coreml", target_os = "macos"))]
fn load_coreml_backend(
    request: &SessionRequest<'_>,
    entrypoint: &Path,
) -> Result<CoreMlMatteBackend> {
    use crate::models::runtime::coreml::{CoreMlComputeUnits, CoreMlSession};

    let options: CoreMlOptions = serde_json::from_value(request.route.options.clone())
        .context("invalid CoreML route options")?;
    let compute_units = match options.compute_units.as_deref() {
        Some("cpu-only") => CoreMlComputeUnits::CpuOnly,
        Some("cpu-and-gpu") => CoreMlComputeUnits::CpuAndGpu,
        Some("cpu-and-neural-engine") | None => CoreMlComputeUnits::CpuAndNeuralEngine,
        Some("all") => CoreMlComputeUnits::All,
        Some(value) => bail!("unsupported CoreML compute_units {value:?}"),
    };
    let compile_cache = request
        .compile_cache
        .context("MODNet CoreML route requires an explicit compile_cache")?;
    ensure_compile_cache_outside_artifact_root(request.artifact_root, compile_cache)?;
    // Any graph or weight change must invalidate the device-compiled bundle. Using only the
    // package Manifest.json is insufficient because its references can stay stable while the
    // model or weight bytes change.
    let digest = crate::models::runtime::coreml::artifact_files_digest(
        request
            .artifact
            .files
            .iter()
            .map(|file| (file.path.as_str(), file.bytes, file.sha256.as_str())),
    )?;
    let cache_key = format!(
        "{}-{}-{}-{digest}",
        request.manifest.model.id, request.manifest.model.version, request.artifact.id
    );
    Ok(CoreMlMatteBackend {
        session: CoreMlSession::load(entrypoint, compile_cache, &cache_key, compute_units)?,
    })
}

#[cfg(any(all(feature = "model-coreml", target_os = "macos"), test))]
fn ensure_compile_cache_outside_artifact_root(
    artifact_root: &Path,
    compile_cache: &Path,
) -> Result<()> {
    let artifact_root = resolve_path_for_containment(artifact_root)?;
    let compile_cache = resolve_path_for_containment(compile_cache)?;
    ensure!(
        !compile_cache.starts_with(&artifact_root),
        "MODNet CoreML compile_cache {} must be outside artifact root {}",
        compile_cache.display(),
        artifact_root.display()
    );
    Ok(())
}

#[cfg(any(all(feature = "model-coreml", target_os = "macos"), test))]
fn resolve_path_for_containment(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)
        .with_context(|| format!("failed to resolve absolute path {}", path.display()))?;
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    let mut cursor = normalized.as_path();
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(cursor) {
            Ok(mut resolved) => {
                for part in missing.iter().rev() {
                    resolved.push(part);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let part = cursor.file_name().with_context(|| {
                    format!("no existing ancestor for path {}", normalized.display())
                })?;
                missing.push(part.to_os_string());
                cursor = cursor.parent().with_context(|| {
                    format!("no existing ancestor for path {}", normalized.display())
                })?;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to resolve path {}", cursor.display()));
            }
        }
    }
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

#[derive(Debug)]
struct Contribution {
    pixels: Vec<(usize, f32)>,
}

fn area_contributions(source: usize, destination: usize) -> Vec<Contribution> {
    let scale = source as f64 / destination as f64;
    (0..destination)
        .map(|output| {
            let start = output as f64 * scale;
            let end = (output + 1) as f64 * scale;
            let first = start.floor() as usize;
            let last = end.ceil().min(source as f64) as usize;
            let pixels = (first..last)
                .filter_map(|input| {
                    let overlap = (end.min((input + 1) as f64) - start.max(input as f64)).max(0.0);
                    (overlap > 0.0).then_some((input.min(source - 1), (overlap / scale) as f32))
                })
                .collect();
            Contribution { pixels }
        })
        .collect()
}

fn area_resize_rgba8_nchw(
    source: &[u8],
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
    normalization_scale: f32,
    normalization_bias: f32,
) -> Vec<f32> {
    let source_width = source_width as usize;
    let source_height = source_height as usize;
    let width = width as usize;
    let height = height as usize;
    let horizontal_weights = area_contributions(source_width, width);
    let vertical_weights = area_contributions(source_height, height);
    let mut horizontal = vec![0.0_f32; source_height * width * 3];
    for y in 0..source_height {
        for (x, contribution) in horizontal_weights.iter().enumerate() {
            for channel in 0..3 {
                horizontal[(y * width + x) * 3 + channel] = contribution
                    .pixels
                    .iter()
                    .map(|&(source_x, weight)| {
                        source[(y * source_width + source_x) * 4 + channel] as f32 * weight
                    })
                    .sum();
            }
        }
    }
    let mut interleaved = vec![0.0_f32; height * width * 3];
    for (y, contribution) in vertical_weights.iter().enumerate() {
        for x in 0..width {
            for channel in 0..3 {
                interleaved[(y * width + x) * 3 + channel] = contribution
                    .pixels
                    .iter()
                    .map(|&(source_y, weight)| {
                        horizontal[(source_y * width + x) * 3 + channel] * weight
                    })
                    .sum();
            }
        }
    }
    let plane = width * height;
    let mut nchw = vec![0.0_f32; plane * 3];
    for y in 0..height {
        for x in 0..width {
            for channel in 0..3 {
                // OpenCV's INTER_AREA returns an 8-bit image before MODNet normalization. Keep
                // that quantization boundary; retaining fractional resize values subtly changes
                // the matte even though the graph itself is identical.
                let resized = interleaved[(y * width + x) * 3 + channel]
                    .round()
                    .clamp(0.0, 255.0);
                nchw[channel * plane + y * width + x] =
                    resized * normalization_scale + normalization_bias;
            }
        }
    }
    nchw
}

fn resize_matte_half_pixel(
    source: &[f32],
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let source_width = source_width as usize;
    let source_height = source_height as usize;
    let width = width as usize;
    let height = height as usize;
    let mut output = vec![0_u8; width * height];
    for y in 0..height {
        let source_y = ((y as f32 + 0.5) * source_height as f32 / height as f32 - 0.5)
            .clamp(0.0, source_height.saturating_sub(1) as f32);
        let y0 = source_y.floor() as usize;
        let y1 = (y0 + 1).min(source_height - 1);
        let wy = source_y - y0 as f32;
        for x in 0..width {
            let source_x = ((x as f32 + 0.5) * source_width as f32 / width as f32 - 0.5)
                .clamp(0.0, source_width.saturating_sub(1) as f32);
            let x0 = source_x.floor() as usize;
            let x1 = (x0 + 1).min(source_width - 1);
            let wx = source_x - x0 as f32;
            let top =
                source[y0 * source_width + x0] * (1.0 - wx) + source[y0 * source_width + x1] * wx;
            let bottom =
                source[y1 * source_width + x0] * (1.0 - wx) + source[y1 * source_width + x1] * wx;
            let value = top * (1.0 - wy) + bottom * wy;
            output[y * width + x] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use image::{Rgba, RgbaImage};

    use super::*;

    #[cfg(feature = "model-modnet-onnx")]
    #[test]
    fn onnx_route_options_accept_a_host_thread_override() {
        let defaults = parse_onnx_route_options(&serde_json::Value::Null).unwrap();
        assert_eq!(defaults.intra_threads, None);

        let configured = parse_onnx_route_options(&serde_json::json!({
            "intra_threads": 3,
            "memory_pattern": false,
            "prepacking": false,
        }))
        .unwrap();
        assert_eq!(configured.intra_threads, Some(3));
        assert_eq!(configured.memory_pattern, Some(false));
        assert_eq!(configured.prepacking, Some(false));
    }

    #[derive(Default)]
    struct FakeState {
        calls: usize,
        inputs: Vec<Vec<f32>>,
        shapes: Vec<[usize; 4]>,
    }

    struct FakeBackend {
        state: Rc<RefCell<FakeState>>,
    }

    impl MatteBackend for FakeBackend {
        fn infer(&mut self, input: &[f32], input_shape: [usize; 4]) -> Result<TensorOutput> {
            let mut state = self.state.borrow_mut();
            state.calls += 1;
            state.inputs.push(input.to_vec());
            state.shapes.push(input_shape);
            let plane = input_shape[2] * input_shape[3];
            Ok(TensorOutput {
                name: OUTPUT_NAME.into(),
                shape: vec![1, 1, input_shape[2], input_shape[3]],
                data: vec![0.5; plane],
            })
        }
    }

    fn release_manifest() -> ModelManifest {
        serde_json::from_str(include_str!("../../catalog/release-modnet-1.0.0.json")).unwrap()
    }

    #[test]
    fn area_resize_preserves_constant_color_and_nchw_order() {
        let image = RgbaImage::from_pixel(4, 4, Rgba([255, 128, 0, 7]));
        let output = area_resize_rgba8_nchw(
            image.as_raw(),
            image.width(),
            image.height(),
            2,
            2,
            NORMALIZATION_SCALE,
            NORMALIZATION_BIAS,
        );
        assert_eq!(output.len(), 12);
        assert!(output[..4].iter().all(|value| (*value - 1.0).abs() < 1e-6));
        assert!(output[4..8].iter().all(|value| value.abs() < 0.004));
        assert!(output[8..].iter().all(|value| (*value + 1.0).abs() < 1e-6));
    }

    #[test]
    fn area_resize_averages_then_quantizes_like_opencv() {
        let mut image = RgbaImage::new(2, 2);
        image.put_pixel(0, 0, Rgba([0, 0, 0, 0]));
        image.put_pixel(1, 0, Rgba([100, 100, 100, 100]));
        image.put_pixel(0, 1, Rgba([200, 200, 200, 200]));
        image.put_pixel(1, 1, Rgba([255, 255, 255, 255]));
        let output = area_resize_rgba8_nchw(
            image.as_raw(),
            image.width(),
            image.height(),
            1,
            1,
            1.0,
            0.0,
        );
        for value in output {
            assert!((value - 139.0).abs() < 1e-5);
        }
    }

    #[test]
    fn adapter_contract_is_machine_readable() {
        let manifest = release_manifest();
        assert_eq!(manifest.contract.adapter, ADAPTER);
        manifest.validate().unwrap();

        let route = manifest
            .routes
            .iter()
            .find(|route| route.backend == Backend::OnnxCpu)
            .unwrap();
        let artifact = manifest
            .artifacts
            .iter()
            .find(|artifact| artifact.id == route.artifact)
            .unwrap();
        validate_contract(&manifest, route, artifact).unwrap();

        let mut wrong_input = manifest.clone();
        wrong_input.contract.inputs[0].name = "image".into();
        assert!(validate_contract(&wrong_input, route, artifact).is_err());

        let mut wrong_output = manifest.clone();
        wrong_output.contract.outputs[0].name = "alpha".into();
        assert!(validate_contract(&wrong_output, route, artifact).is_err());

        let mut unsupported = route.clone();
        unsupported.backend = Backend::OnnxCuda;
        assert!(validate_contract(&manifest, &unsupported, artifact).is_err());
    }

    #[test]
    fn session_reuses_one_backend_and_ignores_rgba_alpha() {
        let state = Rc::new(RefCell::new(FakeState::default()));
        let mut session = MatteSession {
            backend: Box::new(FakeBackend {
                state: Rc::clone(&state),
            }),
            model_width: 2,
            model_height: 2,
            load_ms: 9.25,
        };
        let first = [10, 20, 30, 0, 40, 50, 60, 127];
        let second = [10, 20, 30, 255, 40, 50, 60, 1];

        let first_result = session.inference_rgba8(2, 1, &first).unwrap();
        let second_result = session.inference_rgba8(2, 1, &second).unwrap();

        assert_eq!((first_result.width, first_result.height), (2, 1));
        assert_eq!(first_result.alpha, [128, 128]);
        assert_eq!(second_result.alpha, first_result.alpha);
        assert_eq!(second_result.timings.load_ms, 9.25);
        assert!(second_result.timings.preprocess_ms.is_finite());
        assert!(second_result.timings.inference_ms.is_finite());
        assert!(second_result.timings.postprocess_ms.is_finite());
        let state = state.borrow();
        assert_eq!(state.calls, 2);
        assert_eq!(state.shapes, [[1, 3, 2, 2], [1, 3, 2, 2]]);
        assert_eq!(state.inputs[0], state.inputs[1]);
    }

    #[test]
    fn output_and_rgba_validation_are_explicit() {
        let matte = TensorOutput {
            name: OUTPUT_NAME.into(),
            shape: vec![1, 1, 1, 1],
            data: vec![f32::NAN],
        };
        assert!(validate_matte_output(&matte, 1, 1).is_err());

        let wrong_name = TensorOutput {
            name: "alpha".into(),
            shape: vec![1, 1, 1, 1],
            data: vec![0.5],
        };
        assert!(validate_matte_output(&wrong_name, 1, 1).is_err());

        let wrong_shape = TensorOutput {
            name: OUTPUT_NAME.into(),
            shape: vec![1, 1, 1, 2],
            data: vec![0.5, 0.5],
        };
        assert!(validate_matte_output(&wrong_shape, 1, 1).is_err());

        let state = Rc::new(RefCell::new(FakeState::default()));
        let mut session = MatteSession {
            backend: Box::new(FakeBackend {
                state: Rc::clone(&state),
            }),
            model_width: 2,
            model_height: 2,
            load_ms: 0.0,
        };
        let error = session
            .inference_rgba8(2, 1, &[0; 7])
            .unwrap_err()
            .to_string();
        assert!(error.contains("requires 8 bytes, got 7"));
        assert_eq!(state.borrow().calls, 0);
    }

    #[test]
    fn coreml_compile_cache_must_be_outside_artifact_slot() {
        let base = std::env::temp_dir().join(format!(
            "valle-modnet-cache-contract-{}",
            std::process::id()
        ));
        let artifact_root = base.join("artifact-slot");
        let inside = artifact_root.join("compiled-cache");
        let outside = base.join("compiled-cache");
        assert!(
            ensure_compile_cache_outside_artifact_root(&artifact_root, &artifact_root).is_err()
        );
        assert!(ensure_compile_cache_outside_artifact_root(&artifact_root, &inside).is_err());
        ensure_compile_cache_outside_artifact_root(&artifact_root, &outside).unwrap();
    }
}
