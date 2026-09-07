//! Typed BiRefNet host contract shared by the reference CLI and future Valle integration.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
#[cfg(any(all(feature = "model-coreml", target_os = "macos"), test))]
use std::path::{Component, PathBuf};
use std::time::Instant;

#[cfg(any(
    feature = "model-birefnet-onnx",
    all(feature = "model-coreml", target_os = "macos")
))]
use crate::models::runtime::TensorInput;
#[cfg(feature = "model-birefnet-onnx")]
use crate::models::runtime::onnx::{OnnxSession, OnnxSessionOptions};
use crate::models::runtime::{RunReport, TensorOutput};
use crate::models::spec::{Artifact, Backend, Dimension, ModelManifest, Route};
use anyhow::{Context, Result, bail, ensure};
use image::GrayImage;
use serde::Deserialize;
use serde::de::DeserializeOwned;

pub const ADAPTER: &str = "birefnet-image-matting";
pub const CONTRACT_VERSION: u32 = 1;

const ALLOWED_SIZES: [u32; 3] = [512, 768, 1024];
const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const IMAGENET_STD: [f32; 3] = [0.229, 0.224, 0.225];
const INPUT_NAME: &str = "img";
const OUTPUT_NAME: &str = "alpha";

pub struct RunRequest<'a> {
    pub manifest: &'a ModelManifest,
    pub route: &'a Route,
    pub artifact: &'a Artifact,
    pub artifact_root: &'a Path,
    pub compile_cache: &'a Path,
    pub input: &'a Path,
    pub output: &'a Path,
    pub size: Option<u32>,
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
    pub size: Option<u32>,
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

/// A load-once BiRefNet graph plus its release-owned preprocessing contract.
pub struct MatteSession {
    backend: Box<dyn MatteBackend>,
    model_size: u32,
    load_ms: f64,
}

#[derive(Debug, Deserialize)]
struct Preprocess {
    color_space: String,
    normalization: Normalization,
    #[serde(default)]
    profiles: BTreeMap<String, [u32; 2]>,
}

#[derive(Debug, Deserialize)]
struct Normalization {
    mean: [f32; 3],
    std: [f32; 3],
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
    default_profile: Option<String>,
    #[serde(default)]
    validated_sizes: Vec<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct SizingRouteOptions {
    default_profile: Option<String>,
}

#[cfg(feature = "model-birefnet-onnx")]
#[derive(Debug, Default, Deserialize)]
struct OnnxRouteOptions {
    intra_threads: Option<usize>,
    memory_pattern: Option<bool>,
    prepacking: Option<bool>,
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
                .context("BiRefNet preprocess contract is invalid")?;
        let postprocess: Postprocess =
            serde_json::from_value(request.manifest.contract.postprocess.clone())
                .context("BiRefNet postprocess contract is invalid")?;
        validate_preprocess(&preprocess)?;
        validate_postprocess(&postprocess)?;

        let metadata: ArtifactMetadata = serde_json::from_value(request.artifact.metadata.clone())
            .context("BiRefNet artifact metadata is invalid")?;
        let sizing: SizingRouteOptions = parse_route_options(&request.route.options)
            .context("BiRefNet route sizing options are invalid")?;
        let model_size = resolve_model_size(
            &preprocess,
            &metadata,
            sizing.default_profile.as_deref(),
            request.size,
        )?;
        let entrypoint = request.artifact_root.join(&request.artifact.entrypoint);
        ensure!(
            entrypoint.exists(),
            "BiRefNet model artifact is missing: {}",
            entrypoint.display()
        );

        let backend = load_backend(&request, &entrypoint)?;

        Ok(Self {
            backend,
            model_size,
            load_ms: elapsed_ms(load_started),
        })
    }

    pub const fn model_width(&self) -> u32 {
        self.model_size
    }

    pub const fn model_height(&self) -> u32 {
        self.model_size
    }

    pub const fn load_ms(&self) -> f64 {
        self.load_ms
    }

    /// Infer one tightly packed RGBA8 frame without reloading the selected graph.
    pub fn inference_rgba8(&mut self, width: u32, height: u32, rgba: &[u8]) -> Result<MatteFrame> {
        let total_started = Instant::now();
        validate_rgba8(width, height, rgba)?;

        let preprocess_started = Instant::now();
        let input = bilinear_resize_rgba8_nchw(
            rgba,
            width,
            height,
            self.model_size,
            self.model_size,
            IMAGENET_MEAN,
            IMAGENET_STD,
        );
        let preprocess_ms = elapsed_ms(preprocess_started);

        let inference_started = Instant::now();
        let matte = self.backend.infer(
            &input,
            [1, 3, self.model_size as usize, self.model_size as usize],
        )?;
        let inference_ms = elapsed_ms(inference_started);
        validate_matte_output(&matte, self.model_size)?;

        let postprocess_started = Instant::now();
        let alpha =
            resize_matte_half_pixel(&matte.data, self.model_size, self.model_size, width, height);
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
            #[cfg(feature = "model-birefnet-onnx")]
            {
                let route_options: OnnxRouteOptions =
                    parse_route_options(&request.route.options)
                        .context("BiRefNet ONNX route options are invalid")?;
                let options = OnnxSessionOptions {
                    intra_threads: route_options.intra_threads,
                    memory_pattern: route_options.memory_pattern.unwrap_or(true),
                    prepacking: route_options.prepacking.unwrap_or(true),
                };
                Ok(Box::new(OnnxMatteBackend {
                    session: OnnxSession::load_with_options(_entrypoint, &options)?,
                }))
            }
            #[cfg(not(feature = "model-birefnet-onnx"))]
            {
                bail!("BiRefNet ONNX route selected, but ONNX support is not compiled in")
            }
        }
        Backend::Coreml => {
            #[cfg(all(feature = "model-coreml", target_os = "macos"))]
            {
                Ok(Box::new(load_coreml_backend(request, _entrypoint)?))
            }
            #[cfg(all(feature = "model-coreml", not(target_os = "macos")))]
            {
                bail!("BiRefNet CoreML route selected, but CoreML is only available on macOS")
            }
            #[cfg(not(feature = "model-coreml"))]
            {
                bail!("BiRefNet CoreML route selected, but CoreML support is not compiled in")
            }
        }
        backend => bail!("BiRefNet adapter does not implement backend {backend}"),
    }
}

#[cfg(feature = "model-birefnet-onnx")]
struct OnnxMatteBackend {
    session: OnnxSession,
}

#[cfg(feature = "model-birefnet-onnx")]
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
        size: request.size,
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

fn validate_matte_output(matte: &TensorOutput, model_size: u32) -> Result<()> {
    let expected_shape = vec![1, 1, model_size as usize, model_size as usize];
    ensure!(
        matte.name == OUTPUT_NAME,
        "BiRefNet backend returned output {:?}, expected {OUTPUT_NAME:?}",
        matte.name
    );
    ensure!(
        matte.shape == expected_shape,
        "BiRefNet output has shape {:?}, expected {:?}",
        matte.shape,
        expected_shape
    );
    ensure!(
        matte.data.len() == model_size as usize * model_size as usize,
        "BiRefNet output has {} values, expected {}",
        matte.data.len(),
        model_size as usize * model_size as usize
    );
    ensure!(
        matte.data.iter().all(|value| value.is_finite()),
        "BiRefNet output contains non-finite values"
    );
    Ok(())
}

fn validate_preprocess(preprocess: &Preprocess) -> Result<()> {
    ensure!(
        preprocess.color_space == "rgb",
        "BiRefNet expects RGB input"
    );
    ensure!(
        preprocess.normalization.mean == IMAGENET_MEAN
            && preprocess.normalization.std == IMAGENET_STD,
        "BiRefNet requires ImageNet normalization"
    );
    for (name, [width, height]) in &preprocess.profiles {
        ensure!(
            width == height && ALLOWED_SIZES.contains(width),
            "BiRefNet profile {name:?} has unsupported dimensions {width}x{height}"
        );
    }
    Ok(())
}

fn validate_postprocess(postprocess: &Postprocess) -> Result<()> {
    ensure!(
        postprocess.clamp == [0.0, 1.0]
            && postprocess.resize_to_source == "bilinear-half-pixel"
            && postprocess.encoding == "alpha8-png",
        "unsupported BiRefNet postprocess contract"
    );
    Ok(())
}

fn validate_run_paths(input: &Path, output: &Path) -> Result<()> {
    ensure!(
        output
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("png")),
        "BiRefNet alpha output must use a .png extension"
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
        "BiRefNet frame dimensions must be positive"
    );
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .context("BiRefNet RGBA8 frame dimensions overflow addressable memory")?;
    ensure!(
        rgba.len() == expected,
        "BiRefNet {width}x{height} RGBA8 frame requires {expected} bytes, got {}",
        rgba.len()
    );
    Ok(())
}

fn resolve_model_size(
    preprocess: &Preprocess,
    metadata: &ArtifactMetadata,
    route_default_profile: Option<&str>,
    requested: Option<u32>,
) -> Result<u32> {
    let fixed_size = match (metadata.width, metadata.height) {
        (Some(width), Some(height)) => {
            ensure!(
                width == height,
                "BiRefNet only supports square artifacts, got {width}x{height}"
            );
            Some(width)
        }
        (None, None) => None,
        _ => bail!("BiRefNet artifact metadata has incomplete dimensions"),
    };
    let default_size = match (metadata.default_width, metadata.default_height) {
        (Some(width), Some(height)) => {
            ensure!(
                width == height,
                "BiRefNet default dimensions must be square, got {width}x{height}"
            );
            Some(width)
        }
        (None, None) => None,
        _ => bail!("BiRefNet artifact metadata has incomplete default dimensions"),
    };
    if metadata.fixed_shape {
        ensure!(
            fixed_size.is_some(),
            "fixed BiRefNet artifact metadata must declare width and height"
        );
    }
    let profile = route_default_profile.or(metadata.default_profile.as_deref());
    let profile_size = profile
        .map(|name| {
            let [width, height] =
                preprocess.profiles.get(name).copied().ok_or_else(|| {
                    anyhow::anyhow!("BiRefNet has no preprocess profile {name:?}")
                })?;
            ensure!(
                width == height,
                "BiRefNet profile {name:?} is not square: {width}x{height}"
            );
            Ok(width)
        })
        .transpose()?;
    let size = if metadata.fixed_shape {
        requested.or(fixed_size)
    } else {
        requested.or(profile_size).or(default_size).or(fixed_size)
    }
    .unwrap_or(ALLOWED_SIZES[0]);
    ensure!(
        ALLOWED_SIZES.contains(&size),
        "unsupported BiRefNet size {size}; expected 512, 768 or 1024"
    );
    ensure!(
        metadata.validated_sizes.is_empty() || metadata.validated_sizes.contains(&size),
        "BiRefNet artifact was not validated at size {size}"
    );
    if metadata.fixed_shape {
        ensure!(
            fixed_size == Some(size),
            "fixed BiRefNet artifact uses size {}, requested {size}",
            fixed_size.expect("fixed shape was validated")
        );
    }
    Ok(size)
}

fn validate_contract(manifest: &ModelManifest, route: &Route, artifact: &Artifact) -> Result<()> {
    manifest.validate()?;
    ensure!(manifest.model.id == "birefnet", "manifest is not BiRefNet");
    ensure!(
        manifest.contract.adapter == ADAPTER && manifest.contract.version == CONTRACT_VERSION,
        "unsupported BiRefNet adapter contract {}@{}",
        manifest.contract.adapter,
        manifest.contract.version
    );
    ensure!(
        manifest.contract.inputs.len() == 1 && manifest.contract.outputs.len() == 1,
        "BiRefNet contract must expose exactly one input and one output"
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
        "BiRefNet adapter requires a [1,3,height,width] f32 NCHW input"
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
        "BiRefNet adapter requires a [1,1,height,width] f32 NCHW output"
    );
    ensure!(route.artifact == artifact.id, "route and artifact disagree");
    match route.backend {
        Backend::OnnxCpu => ensure!(artifact.format == "onnx", "ONNX route needs ONNX artifact"),
        Backend::Coreml => ensure!(
            artifact.format == "coreml-package",
            "CoreML route needs a CoreML package"
        ),
        backend => bail!("BiRefNet adapter does not implement backend {backend}"),
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
            .context("BiRefNet CoreML inference returned no alpha output")
    }
}

#[cfg(all(feature = "model-coreml", target_os = "macos"))]
fn load_coreml_backend(
    request: &SessionRequest<'_>,
    entrypoint: &Path,
) -> Result<CoreMlMatteBackend> {
    use crate::models::runtime::coreml::{CoreMlComputeUnits, CoreMlSession};

    let options: CoreMlOptions =
        parse_route_options(&request.route.options).context("invalid CoreML route options")?;
    let compute_units = match options.compute_units.as_deref() {
        Some("cpu-only") => CoreMlComputeUnits::CpuOnly,
        Some("cpu-and-gpu") => CoreMlComputeUnits::CpuAndGpu,
        Some("cpu-and-neural-engine") | None => CoreMlComputeUnits::CpuAndNeuralEngine,
        Some("all") => CoreMlComputeUnits::All,
        Some(value) => bail!("unsupported CoreML compute_units {value:?}"),
    };
    let compile_cache = request
        .compile_cache
        .context("BiRefNet CoreML route requires an explicit compile_cache")?;
    ensure_compile_cache_outside_artifact_root(request.artifact_root, compile_cache)?;
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
        "BiRefNet CoreML compile_cache {} must be outside artifact root {}",
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

fn bilinear_resize_rgba8_nchw(
    source: &[u8],
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
    mean: [f32; 3],
    std: [f32; 3],
) -> Vec<f32> {
    let source_width = source_width as usize;
    let source_height = source_height as usize;
    let width = width as usize;
    let height = height as usize;
    let plane = width * height;
    let mut nchw = vec![0.0_f32; plane * 3];
    for y in 0..height {
        let source_y = ((y as f64 + 0.5) * source_height as f64 / height as f64 - 0.5)
            .clamp(0.0, source_height.saturating_sub(1) as f64);
        let y0 = source_y.floor() as usize;
        let y1 = (y0 + 1).min(source_height - 1);
        let wy = source_y - y0 as f64;
        for x in 0..width {
            let source_x = ((x as f64 + 0.5) * source_width as f64 / width as f64 - 0.5)
                .clamp(0.0, source_width.saturating_sub(1) as f64);
            let x0 = source_x.floor() as usize;
            let x1 = (x0 + 1).min(source_width - 1);
            let wx = source_x - x0 as f64;
            for channel in 0..3 {
                let top = source[(y0 * source_width + x0) * 4 + channel] as f64 * (1.0 - wx)
                    + source[(y0 * source_width + x1) * 4 + channel] as f64 * wx;
                let bottom = source[(y1 * source_width + x0) * 4 + channel] as f64 * (1.0 - wx)
                    + source[(y1 * source_width + x1) * 4 + channel] as f64 * wx;
                // OpenCV resizes the uint8 image before converting it to float. Preserve that
                // quantization boundary and half-pixel sampling. OpenCV's optimized INTER_LINEAR
                // kernels can differ from this scalar interpolation by one input LSB.
                let resized = (top * (1.0 - wy) + bottom * wy).round().clamp(0.0, 255.0) as f32;
                nchw[channel * plane + y * width + x] =
                    (resized / 255.0 - mean[channel]) / std[channel];
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

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn parse_route_options<T>(value: &serde_json::Value) -> serde_json::Result<T>
where
    T: DeserializeOwned + Default,
{
    if value.is_null() {
        Ok(T::default())
    } else {
        serde_json::from_value(value.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use image::{Rgba, RgbaImage};

    use super::*;

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
        serde_json::from_str(include_str!("../../catalog/release-birefnet-1.0.0.json")).unwrap()
    }

    fn release_preprocess() -> Preprocess {
        serde_json::from_value(release_manifest().contract.preprocess).unwrap()
    }

    fn recover_u8(value: f32, channel: usize) -> u8 {
        (((value * IMAGENET_STD[channel] + IMAGENET_MEAN[channel]) * 255.0)
            .round()
            .clamp(0.0, 255.0)) as u8
    }

    #[test]
    fn bilinear_preprocess_preserves_constant_color_and_nchw_order() {
        let image = RgbaImage::from_pixel(2, 2, Rgba([255, 128, 0, 7]));
        let output = bilinear_resize_rgba8_nchw(
            image.as_raw(),
            image.width(),
            image.height(),
            3,
            3,
            IMAGENET_MEAN,
            IMAGENET_STD,
        );
        assert_eq!(output.len(), 27);
        assert!(output[..9].iter().all(|&value| recover_u8(value, 0) == 255));
        assert!(
            output[9..18]
                .iter()
                .all(|&value| recover_u8(value, 1) == 128)
        );
        assert!(output[18..].iter().all(|&value| recover_u8(value, 2) == 0));
    }

    #[test]
    fn bilinear_preprocess_matches_frozen_opencv_fixture_within_one_lsb() {
        let mut image = RgbaImage::new(2, 2);
        image.put_pixel(0, 0, Rgba([0, 0, 0, 0]));
        image.put_pixel(1, 0, Rgba([64, 64, 64, 64]));
        image.put_pixel(0, 1, Rgba([128, 128, 128, 128]));
        image.put_pixel(1, 1, Rgba([255, 255, 255, 255]));
        let output = bilinear_resize_rgba8_nchw(
            image.as_raw(),
            image.width(),
            image.height(),
            3,
            3,
            IMAGENET_MEAN,
            IMAGENET_STD,
        );
        // Frozen cv2 5.0 INTER_LINEAR output. INTER_LINEAR_EXACT produces 192 at index 7.
        let opencv = [0_u8, 32, 64, 64, 112, 160, 128, 191, 255];
        let recovered: Vec<u8> = output[..9]
            .iter()
            .map(|&value| recover_u8(value, 0))
            .collect();
        let max_difference = recovered
            .iter()
            .zip(opencv)
            .map(|(&actual, expected)| actual.abs_diff(expected))
            .max()
            .unwrap();
        assert!(max_difference <= 1, "actual={recovered:?}");
    }

    #[test]
    fn dynamic_size_accepts_all_supported_profiles() {
        let metadata = ArtifactMetadata {
            default_profile: Some("balanced".into()),
            validated_sizes: ALLOWED_SIZES.to_vec(),
            ..ArtifactMetadata::default()
        };
        let preprocess = release_preprocess();
        assert_eq!(
            resolve_model_size(&preprocess, &metadata, None, None).unwrap(),
            768
        );
        assert_eq!(
            resolve_model_size(&preprocess, &metadata, Some("coarse"), None).unwrap(),
            512
        );
        for size in ALLOWED_SIZES {
            assert_eq!(
                resolve_model_size(&preprocess, &metadata, None, Some(size)).unwrap(),
                size
            );
        }
        assert!(resolve_model_size(&preprocess, &metadata, None, Some(640)).is_err());
    }

    #[test]
    fn fixed_size_rejects_a_different_request() {
        let metadata = ArtifactMetadata {
            fixed_shape: true,
            width: Some(1024),
            height: Some(1024),
            ..ArtifactMetadata::default()
        };
        let preprocess = release_preprocess();
        assert_eq!(
            resolve_model_size(&preprocess, &metadata, None, None).unwrap(),
            1024
        );
        assert!(resolve_model_size(&preprocess, &metadata, None, Some(512)).is_err());
    }

    #[test]
    fn matte_resize_uses_half_pixel_coordinates_and_alpha8_rounding() {
        let output = resize_matte_half_pixel(&[0.0, 1.0, 1.0, 0.0], 2, 2, 3, 3);
        assert_eq!(output, [0, 128, 255, 128, 128, 128, 255, 128, 0]);
    }

    #[test]
    fn matte_validation_rejects_non_finite_values() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let matte = TensorOutput {
                name: "alpha".into(),
                shape: vec![1, 1, 1, 1],
                data: vec![value],
            };
            assert!(validate_matte_output(&matte, 1).is_err());
        }

        let wrong_name = TensorOutput {
            name: "mask".into(),
            shape: vec![1, 1, 1, 1],
            data: vec![0.5],
        };
        assert!(validate_matte_output(&wrong_name, 1).is_err());

        let wrong_shape = TensorOutput {
            name: OUTPUT_NAME.into(),
            shape: vec![1, 1, 1, 2],
            data: vec![0.5, 0.5],
        };
        assert!(validate_matte_output(&wrong_shape, 1).is_err());
    }

    #[test]
    fn session_reuses_one_backend_and_ignores_rgba_alpha() {
        let state = Rc::new(RefCell::new(FakeState::default()));
        let mut session = MatteSession {
            backend: Box::new(FakeBackend {
                state: Rc::clone(&state),
            }),
            model_size: 2,
            load_ms: 12.5,
        };
        let first = [10, 20, 30, 0, 40, 50, 60, 127];
        let second = [10, 20, 30, 255, 40, 50, 60, 1];

        let first_result = session.inference_rgba8(2, 1, &first).unwrap();
        let second_result = session.inference_rgba8(2, 1, &second).unwrap();

        assert_eq!((first_result.width, first_result.height), (2, 1));
        assert_eq!(first_result.alpha, [128, 128]);
        assert_eq!(second_result.alpha, first_result.alpha);
        assert_eq!(second_result.timings.load_ms, 12.5);
        assert!(second_result.timings.preprocess_ms.is_finite());
        assert!(second_result.timings.inference_ms.is_finite());
        assert!(second_result.timings.postprocess_ms.is_finite());
        let state = state.borrow();
        assert_eq!(state.calls, 2);
        assert_eq!(state.shapes, [[1, 3, 2, 2], [1, 3, 2, 2]]);
        assert_eq!(state.inputs[0], state.inputs[1]);
    }

    #[test]
    fn rgba_frame_validation_is_explicit() {
        let state = Rc::new(RefCell::new(FakeState::default()));
        let mut session = MatteSession {
            backend: Box::new(FakeBackend {
                state: Rc::clone(&state),
            }),
            model_size: 2,
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
        wrong_output.contract.outputs[0].name = "mask".into();
        assert!(validate_contract(&wrong_output, route, artifact).is_err());

        let mut unsupported = route.clone();
        unsupported.backend = Backend::OnnxCuda;
        assert!(validate_contract(&manifest, &unsupported, artifact).is_err());
    }

    #[test]
    fn coreml_compile_cache_must_be_outside_artifact_slot() {
        let base = std::env::temp_dir().join(format!(
            "valle-birefnet-cache-contract-{}",
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

    #[test]
    fn null_route_options_use_adapter_defaults() {
        let sizing: SizingRouteOptions = parse_route_options(&serde_json::Value::Null).unwrap();
        assert_eq!(sizing.default_profile, None);

        #[cfg(feature = "model-birefnet-onnx")]
        {
            let onnx: OnnxRouteOptions = parse_route_options(&serde_json::Value::Null).unwrap();
            assert_eq!(onnx.intra_threads, None);
            assert_eq!(onnx.memory_pattern, None);
            assert_eq!(onnx.prepacking, None);
        }
    }
}
