//! CLI composition root for Timeline, Project, Motion, assets, and media tools. Native rendering
//! consumes complete immutable fixed packages.

pub mod cmd;
mod events;
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded.rs"));
}
mod output;
mod webhost;
pub mod webruntime;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

/// Logical authoring canvas shared by every `valle motion` command.
#[derive(clap::Args, Debug, Clone, Copy, Default)]
pub struct MotionCanvasArgs {
    /// Canvas dimensions, such as 1920x1080.
    #[arg(long, value_parser = parse_canvas_size, default_value = "1920x1080")]
    pub size: Option<(u32, u32)>,
}

/// One complete immutable Timeline fixed render package.
#[derive(clap::Args, Debug, Clone)]
pub struct FixedRenderPackageArgs {
    /// Canonical `valle.fixed-render-package@1` outer manifest.
    #[arg(long, value_name = "PATH")]
    pub package_manifest: PathBuf,
    /// Canonical Timeline document JSON.
    #[arg(long, value_name = "PATH")]
    pub canonical_timeline: PathBuf,
    /// Complete resource manifest JSON.
    #[arg(long, value_name = "PATH")]
    pub resource_manifest: PathBuf,
    /// Closed verified resource binding bundle JSON.
    #[arg(long, value_name = "PATH")]
    pub verified_binding_bundle: PathBuf,
    /// Canonical execution profile member bound by the outer manifest.
    #[arg(long, value_name = "PATH")]
    pub execution_profile: PathBuf,
    /// Explicit fulfillment, `RESOURCE_ID=PATH_OR_URL`; copied to local CAS. Pin needs every
    /// declared resource either supplied here or already stored locally.
    #[arg(long = "resource", value_name = "RESOURCE_ID=PATH_OR_URL")]
    pub resources: Vec<String>,
}

/// Set process allocator defaults before other initialization, while respecting environment
/// overrides. Rust uses mimalloc and native libraries use the system allocator. Shorten mimalloc's
/// purge delay, use a custom macOS memory tag, and pin glibc's mmap threshold so large native
/// buffers can be returned to the OS.
pub fn tune_process_allocators() {
    use libmimalloc_sys as mi;
    // The bundled mimalloc 3.3.2 uses option slot 15 for purge_delay, which the sys bindings omit.
    // Read the option back to detect incompatible slot changes.
    const MI_OPTION_PURGE_DELAY: mi::mi_option_t = 15;
    unsafe extern "C" {
        // Public mimalloc API provided by the bundled static library.
        fn mi_option_get(option: mi::mi_option_t) -> std::ffi::c_long;
    }
    unsafe {
        if std::env::var_os("MIMALLOC_PURGE_DELAY").is_none() {
            mi::mi_option_set(MI_OPTION_PURGE_DELAY, 25);
            if mi_option_get(MI_OPTION_PURGE_DELAY) != 25 {
                eprintln!("warn: mimalloc purge_delay tuning ineffective (option slot drift?)");
            }
        }
        if std::env::var_os("MIMALLOC_OS_TAG").is_none() {
            mi::mi_option_set(mi::mi_option_os_tag, 240);
        }
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os("MALLOC_MMAP_THRESHOLD_").is_none() {
        // Pin glibc's initial 128 KiB mmap threshold to prevent automatic increases.
        unsafe { libc::mallopt(libc::M_MMAP_THRESHOLD, 131072) };
    }
}

#[derive(Parser)]
#[command(
    name = "valle",
    version,
    about = "Valle: a video creation and editing engine built for AI agents",
    after_help = "Examples:\n  valle motion check examples/hello.motion.tsx\n  valle timeline render examples/timeline.json -o timeline.mp4\n  valle project create demo --timeline examples/timeline.json\n  valle assets add cover.png --tag demo\n  valle models install dpdfnet\n  valle media enhance speech.wav -o clean.wav\n\nUse --json for one result or --events for NDJSON progress and a final report.\nRun `valle docs` for bundled guides, or `valle docs cli` for workflows and output contracts."
)]
pub struct Cli {
    /// Emit one JSON result. Studio emits readiness and keeps serving.
    #[arg(long, global = true, conflicts_with = "events")]
    pub json: bool,
    /// Emit progress and results as NDJSON.
    #[arg(long, global = true, conflicts_with = "json")]
    pub events: bool,
    /// FFmpeg diagnostics written to stderr. Info includes encoder parameters and statistics.
    #[arg(long, global = true, value_enum, default_value = "error")]
    pub ffmpeg_log_level: FfmpegLogLevelArg,
    /// User-installed FFmpeg shared-library directory or prefix (also VALLE_FFMPEG_DIR).
    #[arg(long, global = true)]
    pub ffmpeg_dir: Option<std::path::PathBuf>,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FfmpegLogLevelArg {
    Error,
    Warning,
    Info,
    Debug,
}

impl From<FfmpegLogLevelArg> for valle_media::codec::ffi::FfmpegLogLevel {
    fn from(value: FfmpegLogLevelArg) -> Self {
        match value {
            FfmpegLogLevelArg::Error => Self::Error,
            FfmpegLogLevelArg::Warning => Self::Warning,
            FfmpegLogLevelArg::Info => Self::Info,
            FfmpegLogLevelArg::Debug => Self::Debug,
        }
    }
}

/// Public authoring and media commands.
#[derive(Subcommand)]
pub enum Cmd {
    /// Show bundled licenses, acknowledgements, and dependency source download links.
    Licenses,
    /// Read bundled guides offline; omit TOPIC to list available documentation.
    Docs {
        #[arg(value_parser = clap::builder::PossibleValuesParser::new(cmd::docs::topics()))]
        topic: Option<String>,
    },
    /// Check or render Motion JSX, or open Studio.
    Motion {
        #[command(subcommand)]
        action: MotionAction,
    },
    /// Validate and render a timeline document.
    Timeline {
        #[command(subcommand)]
        action: TimelineAction,
    },
    /// Offline file-to-file media processing and author analysis tools.
    Media {
        #[command(subcommand)]
        action: MediaAction,
    },
    /// Manage local model weights. Commands requiring missing weights report the install command.
    Models {
        #[command(subcommand)]
        action: ModelsAction,
    },
    /// Create, edit, inspect and render versioned projects.
    Project {
        /// Emit a machine-readable JSON envelope.
        #[arg(skip)]
        json: bool,
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Manage the asset library under `~/.valle/assets/`.
    Assets {
        /// Emit a machine-readable JSON envelope.
        #[arg(skip)]
        json: bool,
        /// Emit progress and the final report as NDJSON on stdout. Conflicts with --json.
        #[arg(skip)]
        events: bool,
        #[command(subcommand)]
        action: AssetsAction,
    },
}

/// File-level offline tools exposed under `valle media <tool>`.
#[derive(Subcommand)]
pub enum MediaAction {
    /// Inspect user-installed FFmpeg libraries, versions, registered codecs and hardware support.
    Capabilities,
    /// Transcribe audio or video into a standalone word-level transcript.
    ///
    /// ASR and forced alignment are currently unavailable on Windows.
    Transcribe {
        #[command(flatten)]
        args: TranscribeArgs,
    },
    /// Extract a transparent foreground from an image or video.
    Matte {
        #[command(flatten)]
        args: MatteArgs,
    },
    /// Enhance speech and suppress noise in audio or a video's audio track.
    Enhance {
        #[command(flatten)]
        args: EnhanceArgs,
    },
    /// Separate a media file's audio into vocals and instrumental stems.
    Separate {
        #[command(flatten)]
        args: SeparateArgs,
    },
    /// Detect shot boundaries and write a canonical standalone shots JSON document.
    Shots {
        #[command(flatten)]
        args: ShotsArgs,
    },
    /// Segment a prompted object and write a strict binary mask artifact.
    Segment {
        #[command(flatten)]
        args: SegmentArgs,
    },
    /// Inpaint selected pixels in an image or video using a strict binary mask.
    Inpaint {
        #[command(flatten)]
        args: InpaintArgs,
    },
    /// Enlarge an image or video with the published Real-ESRGAN x4 model.
    Upscale {
        #[command(flatten)]
        args: UpscaleArgs,
    },
    /// Increase a video's frame rate with ordered RIFE frame interpolation.
    Interpolate {
        #[command(flatten)]
        args: InterpolateArgs,
    },
}

/// Public inference backend choices. Execution providers remain route internals.
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum MediaBackendArg {
    #[default]
    Auto,
    #[value(name = "coreml")]
    CoreMl,
    Onnx,
}

impl From<MediaBackendArg> for valle_media::models::RunBackendPreference {
    fn from(value: MediaBackendArg) -> Self {
        match value {
            MediaBackendArg::Auto => Self::Auto,
            MediaBackendArg::CoreMl => Self::CoreMl,
            MediaBackendArg::Onnx => Self::Onnx,
        }
    }
}

/// Arguments for `valle media transcribe`.
#[derive(clap::Args)]
pub struct TranscribeArgs {
    /// Input audio or video supported by the configured media decoder.
    pub input: PathBuf,
    /// Output analysis JSON; defaults to `.words.json`, or `.sentences.json` with sentence level.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Language hint such as `zh` or `en`; omission enables automatic detection.
    #[arg(long)]
    pub lang: Option<String>,
    /// Skip forced alignment and write only text to stdout; cannot be combined with `--output`.
    #[arg(long)]
    pub text_only: bool,
    /// JSON presentation level: `word` or punctuation-derived `sentence`.
    #[arg(long, default_value = "word")]
    pub level: String,
    /// Transcription model id. Install pinned official HF weights with `valle models install` before offline use.
    #[arg(long, default_value = "qwen3-asr-0.6b")]
    pub model: String,
    /// Exact non-default transcription model version. Omission selects Valle's pinned release.
    #[arg(long)]
    pub model_version: Option<String>,
    /// Public backend preference. Qwen transcription currently accepts `auto` only; its native CPU route remains internal.
    #[arg(long, value_enum, default_value_t)]
    pub backend: MediaBackendArg,
    /// Replace an existing output atomically where the platform supports it.
    #[arg(long)]
    pub overwrite: bool,
    /// Persist the same versioned run envelope emitted by `--json`.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
    /// Emit one versioned machine-readable envelope to stdout.
    #[arg(skip)]
    pub json: bool,
}

/// Arguments for `valle media matte`.
#[derive(clap::Args)]
pub struct MatteArgs {
    /// Input PNG image or video supported by the configured media decoder.
    pub input: PathBuf,
    /// Transparent PNG or lossless RGBA MOV; defaults beside the input according to its type.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Half-open source range in seconds: `start,end`; an empty end means EOF.
    #[arg(long)]
    pub range: Option<String>,
    /// Foreground sampling frame rate.
    #[arg(long, default_value_t = 5.0)]
    pub fps: f64,
    /// Model id from the embedded Valle model catalog.
    #[arg(long, default_value = "birefnet")]
    pub model: String,
    /// Exact non-default model version. Omission selects Valle's pinned default release.
    #[arg(long)]
    pub model_version: Option<String>,
    /// Inference backend. ONNX is available on every supported platform;
    /// CoreML is also available on macOS. An explicit backend never falls back silently.
    #[arg(long, value_enum, default_value_t)]
    pub backend: MediaBackendArg,
    /// Replace an existing output atomically where the platform supports it.
    #[arg(long)]
    pub overwrite: bool,
    /// Persist the same versioned run envelope emitted by `--json`.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
    /// Emit one versioned machine-readable envelope to stdout.
    #[arg(skip)]
    pub json: bool,
}

/// Arguments for `valle media enhance`.
#[derive(clap::Args)]
pub struct EnhanceArgs {
    /// Input audio or video supported by the configured media decoder.
    pub input: PathBuf,
    /// 48 kHz mono `.wav` (float32) or `.flac` (PCM24); defaults to `<stem>.enhanced.wav`.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Model id from the embedded Valle model catalog.
    #[arg(long, default_value = "dpdfnet")]
    pub model: String,
    /// Exact non-default model version. Omission selects Valle's pinned default release.
    #[arg(long)]
    pub model_version: Option<String>,
    /// Inference backend. The current DPDFNet adapter implements ONNX only; explicit CoreML fails.
    #[arg(long, value_enum, default_value_t)]
    pub backend: MediaBackendArg,
    /// Replace an existing output atomically where the platform supports it.
    #[arg(long)]
    pub overwrite: bool,
    /// Persist the same versioned run envelope emitted by `--json`.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
    /// Emit one versioned machine-readable envelope to stdout.
    #[arg(skip)]
    pub json: bool,
}

/// Arguments for `valle media separate`.
#[derive(clap::Args)]
pub struct SeparateArgs {
    /// Input audio or video supported by the configured media decoder.
    pub input: PathBuf,
    /// New directory receiving `vocals.wav` and `instrumental.wav`.
    #[arg(short = 'o', long = "output")]
    pub output_dir: PathBuf,
    /// Model id from the embedded Valle model catalog.
    #[arg(long, default_value = "demucs")]
    pub model: String,
    /// Exact non-default model version. Omission selects Valle's pinned default release.
    #[arg(long)]
    pub model_version: Option<String>,
    /// Inference backend. The current Demucs adapter implements ONNX only; explicit CoreML fails.
    #[arg(long, value_enum, default_value_t)]
    pub backend: MediaBackendArg,
    /// Replace an existing report; the multi-output directory itself is never overwritten.
    #[arg(long)]
    pub overwrite: bool,
    /// Persist the same versioned run envelope emitted by `--json`.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
    /// Emit one versioned machine-readable envelope to stdout.
    #[arg(skip)]
    pub json: bool,
}

/// Arguments for `valle media shots`.
#[derive(clap::Args)]
pub struct ShotsArgs {
    /// Input video supported by the configured media decoder.
    pub input: PathBuf,
    /// Canonical standalone shots JSON; defaults to `<stem>.shots.json` beside the input.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Explicit detector id. `auto` selects a backend within this model, never another detector.
    #[arg(long, default_value = "omnishotcut")]
    pub model: String,
    /// Exact non-default model version. Omission selects Valle's pinned default release.
    #[arg(long)]
    pub model_version: Option<String>,
    /// Inference backend. Current shot-detector adapters implement ONNX only; explicit CoreML fails.
    #[arg(long, value_enum, default_value_t)]
    pub backend: MediaBackendArg,
    /// Replace an existing output atomically where the platform supports it.
    #[arg(long)]
    pub overwrite: bool,
    /// Persist the same versioned run envelope emitted by `--json`.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
    /// Emit one versioned machine-readable envelope to stdout.
    #[arg(skip)]
    pub json: bool,
}

/// Arguments for `valle media segment`.
#[derive(clap::Args)]
pub struct SegmentArgs {
    /// Input PNG or video supported by the configured media decoder.
    pub input: PathBuf,
    /// Output `.png` for an image or lossless Gray8 `.mkv` for video.
    #[arg(short, long)]
    pub output: PathBuf,
    /// Optional transparent foreground (`.png` for images, lossless RGBA `.mov` for video).
    #[arg(long, value_name = "PATH")]
    pub foreground_output: Option<PathBuf>,
    /// `valle.segment-prompt@1` JSON containing exactly three labeled points.
    #[arg(long)]
    pub prompt: PathBuf,
    /// Half-open source range in seconds: `start,end`; an empty end means EOF.
    #[arg(long)]
    pub range: Option<String>,
    /// Foreground probability threshold; defaults to 0.5.
    #[arg(long)]
    pub threshold: Option<f32>,
    /// Model id from the embedded Valle model catalog.
    #[arg(long, default_value = "edgetam")]
    pub model: String,
    /// Exact non-default model version. Omission selects Valle's pinned default release.
    #[arg(long)]
    pub model_version: Option<String>,
    /// Inference backend. The current EdgeTAM adapter implements ONNX only; explicit CoreML fails.
    #[arg(long, value_enum, default_value_t)]
    pub backend: MediaBackendArg,
    /// Replace an existing mask atomically; invalid when `--foreground-output` is requested.
    #[arg(long)]
    pub overwrite: bool,
    /// Persist the same versioned run envelope emitted by `--json`.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
    /// Emit one versioned machine-readable envelope to stdout.
    #[arg(skip)]
    pub json: bool,
}

/// Arguments for `valle media inpaint`.
#[derive(clap::Args)]
pub struct InpaintArgs {
    /// Input PNG or video supported by the configured media decoder.
    pub input: PathBuf,
    /// Strict binary Gray8 PNG or lossless FFV1 mask video.
    #[arg(long)]
    pub mask: PathBuf,
    /// Output PNG or H.264/AAC MP4; defaults beside the input.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Half-open source range in seconds: `start,end`; mask must be the same range rebased to zero.
    #[arg(long)]
    pub range: Option<String>,
    /// Model id from the embedded Valle model catalog.
    #[arg(long, default_value = "lama")]
    pub model: String,
    /// Exact non-default model version. Omission selects Valle's pinned default release.
    #[arg(long)]
    pub model_version: Option<String>,
    /// Inference backend. The current LaMa adapter implements ONNX only; explicit CoreML fails.
    #[arg(long, value_enum, default_value_t)]
    pub backend: MediaBackendArg,
    /// Replace an existing output atomically where the platform supports it.
    #[arg(long)]
    pub overwrite: bool,
    /// Persist the same versioned run envelope emitted by `--json`.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
    /// Emit one versioned machine-readable envelope to stdout.
    #[arg(skip)]
    pub json: bool,
}

/// Arguments for `valle media upscale`.
#[derive(clap::Args)]
pub struct UpscaleArgs {
    /// Input PNG or video supported by the configured media decoder.
    pub input: PathBuf,
    /// Output PNG or H.264/AAC MP4; defaults beside the input.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Half-open source range in seconds for video: `start,end`; an empty end means EOF.
    #[arg(long)]
    pub range: Option<String>,
    /// Published native scale. The current release supports x4 only.
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u32).range(4..=4))]
    pub scale: u32,
    /// Model id from the embedded Valle model catalog.
    #[arg(long, default_value = "realesrgan")]
    pub model: String,
    /// Exact non-default model version. Omission selects Valle's pinned default release.
    #[arg(long)]
    pub model_version: Option<String>,
    /// Inference backend. The current Real-ESRGAN adapter implements ONNX only; explicit CoreML fails.
    #[arg(long, value_enum, default_value_t)]
    pub backend: MediaBackendArg,
    /// Replace an existing output atomically where the platform supports it.
    #[arg(long)]
    pub overwrite: bool,
    /// Persist the same versioned run envelope emitted by `--json`.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
    /// Emit one versioned machine-readable envelope to stdout.
    #[arg(skip)]
    pub json: bool,
}

/// Arguments for `valle media interpolate`.
#[derive(clap::Args)]
pub struct InterpolateArgs {
    /// Input video supported by the configured media decoder.
    pub input: PathBuf,
    /// H.264/AAC MP4 output.
    #[arg(short, long)]
    pub output: PathBuf,
    /// Target constant frame rate; it must exceed the source frame rate.
    #[arg(long, default_value_t = 60)]
    pub fps: u32,
    /// Optional canonical shots JSON used to prevent interpolation across boundaries.
    #[arg(long)]
    pub shots: Option<PathBuf>,
    /// Model id from the embedded Valle model catalog.
    #[arg(long, default_value = "rife")]
    pub model: String,
    /// Exact non-default model version. Omission selects Valle's pinned default release.
    #[arg(long)]
    pub model_version: Option<String>,
    /// Inference backend. The current RIFE adapter implements ONNX only; explicit CoreML fails.
    #[arg(long, value_enum, default_value_t)]
    pub backend: MediaBackendArg,
    /// Replace an existing output atomically where the platform supports it.
    #[arg(long)]
    pub overwrite: bool,
    /// Persist the same versioned run envelope emitted by `--json`.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
    /// Emit one versioned machine-readable envelope to stdout.
    #[arg(skip)]
    pub json: bool,
}

/// Delivery controls independent of the logical Motion authoring canvas.
#[derive(clap::Args, Debug, Clone, Default)]
pub struct MotionRenderTuningArgs {
    /// Concurrent frame renderers (1-8). Defaults: Raster at most 2; Metal 1.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=8))]
    pub workers: Option<u8>,
    /// Require hardware H.264 encoding. Fails if unavailable; no software fallback.
    #[arg(long, conflicts_with = "frame")]
    pub hardware_encode: bool,
    /// Hardware encoder target bitrate in bits/second. Defaults to a resolution-based estimate.
    #[arg(long, requires = "hardware_encode", conflicts_with = "frame", value_parser = clap::value_parser!(u32).range(1..))]
    pub bitrate: Option<u32>,
    /// Software H.264 encoder threads. Defaults to automatic selection.
    #[arg(long, conflicts_with_all = ["hardware_encode", "frame"], value_parser = clap::value_parser!(u16).range(1..))]
    pub encode_threads: Option<u16>,
    /// Delivery dimensions; scales the logical --size canvas without changing layout.
    #[arg(long, value_parser = parse_canvas_size)]
    pub output_size: Option<(u32, u32)>,
}

/// `valle motion` artifact authoring commands.
#[derive(Subcommand)]
pub enum MotionAction {
    /// Compile and validate Motion JSX without writing an artifact.
    Check {
        /// Motion JSX source file.
        input: PathBuf,
        /// Bind an asset control as name=path; may be repeated.
        #[arg(long = "asset", value_name = "NAME=PATH")]
        assets: Vec<String>,
        #[command(flatten)]
        bindings: MotionBindingArgs,
        /// JSON object bound to `controls.data` during prepare.
        #[arg(long, value_name = "PATH")]
        data: Option<PathBuf>,
        /// Fonts used by actual rendering; may be repeated.
        #[arg(long)]
        font: Vec<PathBuf>,
        #[arg(long, default_value = "5")]
        duration: String,
        #[arg(long, default_value = "30")]
        fps: String,
        /// Validate this exact frame through the native raster renderer.
        #[arg(long, default_value_t = 0)]
        frame: i64,
        /// Logical canvas shared by compilation, measurement, and validation.
        #[command(flatten)]
        canvas: MotionCanvasArgs,
    },
    /// Compile Motion JSX and render an MP4 or one PNG frame.
    Render {
        /// Motion JSX source file.
        input: PathBuf,
        /// Render one exact zero-based frame to PNG instead of MP4.
        #[arg(long)]
        frame: Option<i64>,
        /// Destination file. Existing files are never overwritten.
        #[arg(short, long)]
        output: PathBuf,
        /// Compositor backend. Auto uses an available Metal device on macOS, otherwise CPU Raster.
        #[arg(long, value_enum, default_value = "auto")]
        backend: MotionRenderBackend,
        #[command(flatten)]
        tuning: MotionRenderTuningArgs,
        /// Bind an asset control as name=path; may be repeated.
        #[arg(long = "asset", value_name = "NAME=PATH")]
        assets: Vec<String>,
        /// Add a font alongside the deterministic default fonts; may be repeated.
        #[arg(long)]
        font: Vec<PathBuf>,
        #[command(flatten)]
        bindings: MotionBindingArgs,
        /// JSON object bound to controls.data during prepare.
        #[arg(long, value_name = "PATH")]
        data: Option<PathBuf>,
        /// Duration in seconds as an exact Timeline decimal.
        #[arg(long, default_value = "5")]
        duration: String,
        /// Frames per second, including rational rates such as 30000/1001.
        #[arg(long, default_value = "30")]
        fps: String,
        #[command(flatten)]
        canvas: MotionCanvasArgs,
    },
    /// Open local Motion Studio with hot reload, diagnostics, source mapping, and web preview.
    Studio {
        /// Motion JSX source file.
        input: PathBuf,
        /// Bind an asset control as name=path; may be repeated.
        #[arg(long = "asset", value_name = "NAME=PATH")]
        assets: Vec<String>,
        /// Add a font file for measurement and preview alongside the default fonts; may be
        /// repeated.
        #[arg(long)]
        font: Vec<PathBuf>,
        #[command(flatten)]
        bindings: MotionBindingArgs,
        /// JSON object bound to `controls.data`; Studio watches it for re-prepare.
        #[arg(long, value_name = "PATH")]
        data: Option<PathBuf>,
        /// Override the bundled Studio web resources for development.
        #[arg(long)]
        web_assets_dir: Option<PathBuf>,
        /// Local port; use 0 to let the OS select an available port.
        #[arg(long, default_value_t = 9527)]
        port: u16,
        /// Studio duration in seconds, expressed as a Timeline q6 JSON decimal.
        #[arg(long, default_value = "5")]
        duration: String,
        #[arg(long, default_value = "30")]
        fps: String,
        #[command(flatten)]
        canvas: MotionCanvasArgs,
    },
}

#[derive(clap::Args, Clone, Debug, Default)]
pub struct MotionBindingArgs {
    /// JSON object of constant Motion prop values.
    #[arg(long, value_name = "PATH")]
    props: Option<PathBuf>,
    /// JSON object of Timeline source-range cue bindings, in seconds.
    #[arg(long, value_name = "PATH")]
    cues: Option<PathBuf>,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum MotionRenderBackend {
    Auto,
    Raster,
    #[cfg(target_os = "macos")]
    Metal,
}

impl From<MotionRenderBackend> for valle_render::executor::skia::SkiaBackendKind {
    fn from(value: MotionRenderBackend) -> Self {
        match value {
            MotionRenderBackend::Auto => Self::preferred_available(),
            MotionRenderBackend::Raster => Self::Raster,
            #[cfg(target_os = "macos")]
            MotionRenderBackend::Metal => Self::Metal,
        }
    }
}

/// Fixed-package Native delivery operations.
#[derive(Subcommand)]
pub enum FixedRenderAction {
    /// Validate and compile the complete fixed render package without rendering pixels.
    Open,
    /// Retain this verified package's complete resource set in the local asset store.
    Pin,
    /// Release this package's persistent resource retention; active renders remain protected.
    Unpin,
    /// Render one exact admitted frame to PNG.
    Preview {
        /// Exact zero-based FrameKey in the admitted fixed package.
        #[arg(long, default_value_t = 0)]
        frame: i64,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Export the complete admitted fixed package to MP4 (or audio-only media when applicable).
    Export {
        #[arg(short, long)]
        output: PathBuf,
    },
}

/// Asset library commands.
#[derive(Subcommand)]
pub enum AssetsAction {
    /// Import files. Repeated imports reuse the same ID and restore removed assets.
    Add {
        /// File paths after shell wildcard expansion.
        paths: Vec<String>,
        /// Import mode: reflink (default), copy, or reference (register without copying).
        #[arg(long, default_value = "reflink")]
        mode: String,
        /// Set the asset kind when probing cannot identify it.
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        title: Option<String>,
        /// May be repeated.
        #[arg(long = "tag")]
        tags: Vec<String>,
    },
    /// Remove asset bytes while keeping metadata; --purge also removes metadata and requires
    /// --force for annotated assets.
    #[command(name = "remove")]
    Rm {
        hash: String,
        #[arg(long)]
        purge: bool,
        #[arg(long)]
        force: bool,
    },
    /// List assets, excluding removed entries by default.
    List {
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        /// List references marked invalid by verify.
        #[arg(long)]
        stale: bool,
        /// List removed assets that can be restored.
        #[arg(long)]
        removed: bool,
    },
    /// Edit title or subkind; use tag to change tags.
    Edit {
        hash: String,
        #[arg(long)]
        title: Option<String>,
        /// Audio subkind: music or sfx.
        #[arg(long)]
        subkind: Option<String>,
    },
    /// Add or remove tags; both options may be repeated.
    Tag {
        hash: String,
        #[arg(long = "add")]
        add: Vec<String>,
        #[arg(long = "rm")]
        rm: Vec<String>,
    },
    /// Annotate an asset, a time point, or a time range; use --rm to remove an annotation.
    Annotate {
        hash: String,
        /// Time marker in seconds.
        #[arg(long)]
        at: Option<f64>,
        /// Time range in seconds, supplied as START END.
        #[arg(long, num_args = 2, value_names = ["START", "END"])]
        range: Option<Vec<f64>>,
        #[arg(long)]
        text: Option<String>,
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Attach an entity ID; may be repeated.
        #[arg(long = "entity")]
        entities: Vec<String>,
        /// Replace an existing annotation; omit to create one.
        #[arg(long)]
        id: Option<String>,
        /// Remove an annotation by its ID, such as n2.
        #[arg(long)]
        rm: Option<String>,
    },
    /// Manage entities with stable IDs and aliases.
    Entity {
        #[command(subcommand)]
        action: EntityCmd,
    },
    /// Show an asset's metadata and analysis.
    Show { hash: String },
    /// Show asset counts, kind distribution, analysis coverage, costs, and stale entries.
    #[command(name = "stats")]
    Describe,
    /// Resolve a hash or unique prefix to an absolute path.
    Resolve { hash: String },
    /// Read word or sentence transcripts from stored ASR results without rerunning a model.
    Transcript {
        /// Asset hash or unique prefix.
        hash: String,
        /// Transcript level: word for precise timing, or sentence for readable text (default).
        #[arg(long, default_value = "sentence")]
        level: String,
    },
    /// Run analyzers on an explicit selection, resuming completed work. Semantic analysis never
    /// runs automatically.
    Analyze {
        /// Asset hashes; alternatively select assets with --all, --kind, or --tag.
        hashes: Vec<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        /// Comma-separated analyzers: shots, beats, vlm. Use `valle media transcribe` for ASR.
        #[arg(long, value_delimiter = ',')]
        with: Vec<String>,
        /// Per-run cost limit in CNY. Stop when exceeded and keep completed results.
        #[arg(long)]
        budget: Option<f64>,
        /// Recompute results, ignoring cached analysis slots.
        #[arg(long)]
        force: bool,
    },
    /// Search assets and timed evidence using local full-text search.
    Search {
        query: String,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        /// Entity ID, such as e1.
        #[arg(long)]
        entity: Option<String>,
        /// Structured filters, such as orientation=portrait,min-dur=5.
        #[arg(long)]
        filter: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Low-level asset library maintenance.
    Maintenance {
        #[command(subcommand)]
        action: AssetsMaintenanceAction,
    },
}

/// `valle assets entity <op>`。
#[derive(Subcommand)]
pub enum AssetsMaintenanceAction {
    /// Check reference validity, orphaned files, and drift; use --deep to recompute hashes.
    Verify {
        #[arg(long)]
        deep: bool,
    },
    /// Remove orphaned blobs and temporary writes while preserving metadata and analysis.
    Gc,
    /// Rebuild index.db from the authoritative metadata, analysis, and annotation trees.
    Reindex,
    /// Run read-only SQL; the authorizer enforces SELECT-only access.
    Sql { query: String },
}

#[derive(Subcommand)]
pub enum EntityCmd {
    /// Create an entity with a free-form kind, such as person, place, or product.
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long = "alias")]
        aliases: Vec<String>,
    },
    List,
    /// Rename an entity or replace its complete alias list.
    Edit {
        id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long = "alias")]
        aliases: Option<Vec<String>>,
    },
}

/// Timeline full-document project operations.
#[derive(Subcommand)]
pub enum ProjectAction {
    /// Create a project from one complete sparse timeline.json.
    Create {
        project_id: String,
        #[arg(long)]
        timeline: PathBuf,
        #[arg(long)]
        intent: Option<String>,
    },
    /// Read HEAD or one pinned historical full snapshot.
    #[command(name = "show")]
    GetTimeline {
        project_id: String,
        #[arg(long, value_parser = parse_project_revision)]
        revision: Option<u64>,
        /// Write the sparse timeline.json to a new file.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Page through the single linear immutable revision history.
    #[command(name = "history")]
    ListTimelineRevisions {
        project_id: String,
        #[arg(long, value_parser = parse_project_revision)]
        cursor: Option<u64>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Submit one complete sparse Timeline against a base revision.
    #[command(name = "apply")]
    EditTimeline {
        project_id: String,
        #[arg(long, value_parser = parse_project_revision)]
        base_revision: u64,
        #[arg(long)]
        timeline: PathBuf,
        #[arg(long)]
        intent: Option<String>,
    },
    /// Serve the local Project Studio authoring host for one fixed initial revision.
    Studio {
        project_id: String,
        #[arg(long, value_parser = parse_project_revision)]
        revision: Option<u64>,
        /// Override the bundled Web assets directory.
        #[arg(long)]
        web_assets_dir: Option<PathBuf>,
        /// Local loopback port; 0 asks the OS for an available port.
        #[arg(long, default_value_t = 9527)]
        port: u16,
    },
    /// Append a new revision containing one prior complete Timeline.
    #[command(name = "restore")]
    RestoreTimelineRevision {
        project_id: String,
        #[arg(long, value_parser = parse_project_revision)]
        base_revision: u64,
        #[arg(long = "revision", value_parser = parse_project_revision)]
        source_revision: u64,
        #[arg(long)]
        intent: Option<String>,
    },
    /// Render the current or a historical project revision.
    Render {
        project_id: String,
        #[arg(long, value_parser = parse_project_revision)]
        revision: Option<u64>,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        frame: Option<i64>,
    },
}

fn parse_project_revision(value: &str) -> Result<u64, String> {
    let revision = value
        .parse::<u64>()
        .map_err(|_| "revision must be an integer from 1 through 9007199254740991".to_owned())?;
    valle_project::revision::validate_public_revision(revision).map_err(|error| error.to_string())
}

#[derive(Subcommand)]
pub enum ModelsAction {
    /// List pinned models and per-artifact local status without network access.
    List {
        /// Emit machine-readable JSON to stdout.
        #[arg(skip)]
        json: bool,
    },
    /// Explicitly download and verify one or more runnable artifacts.
    Install {
        /// Model id shown by `valle models list`.
        id: String,
        /// Exact version, or `latest` together with `--refresh-catalog`.
        #[arg(long)]
        version: Option<String>,
        /// Artifact route selection for this host.
        #[arg(long, value_enum, default_value_t)]
        backend: ModelsInstallBackendArg,
        /// Refresh the remote catalog during this explicit install operation.
        #[arg(long)]
        refresh_catalog: bool,
        /// Emit machine-readable JSON to stdout.
        #[arg(skip)]
        json: bool,
    },
    /// Verify catalog artifact hashes offline. Uninstalled variants are reported as missing;
    /// use --artifact to verify just the variant returned by install.
    Verify {
        /// Model id shown by `valle models list`.
        id: String,
        /// Exact installed version; omission selects Valle's pinned default.
        #[arg(long)]
        version: Option<String>,
        /// Restrict verification to artifacts used by one public backend.
        #[arg(long, value_enum)]
        backend: Option<MediaBackendArg>,
        /// Restrict verification to one exact artifact id.
        #[arg(long)]
        artifact: Option<String>,
        /// Emit machine-readable JSON to stdout.
        #[arg(skip)]
        json: bool,
    },
}

/// Installation can request all runnable routes; media execution cannot.
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum ModelsInstallBackendArg {
    #[default]
    Auto,
    #[value(name = "coreml")]
    CoreMl,
    Onnx,
    All,
}

impl From<ModelsInstallBackendArg> for valle_media::models::InstallBackendSelection {
    fn from(value: ModelsInstallBackendArg) -> Self {
        match value {
            ModelsInstallBackendArg::Auto => Self::Auto,
            ModelsInstallBackendArg::CoreMl => Self::CoreMl,
            ModelsInstallBackendArg::Onnx => Self::Onnx,
            ModelsInstallBackendArg::All => Self::All,
        }
    }
}

#[derive(Subcommand)]
pub enum TimelineAction {
    /// Validate a timeline without rendering.
    Check { input: PathBuf },
    /// Prepare resources and render a timeline to MP4 or one frame to PNG.
    Render {
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        frame: Option<i64>,
    },
}

/// Dispatch one public command.
pub fn dispatch(cmd: Cmd) -> Result<std::process::ExitCode> {
    match cmd {
        Cmd::Licenses => {
            if output::machine() {
                output::emit(
                    serde_json::json!({"status": "ok", "complete": cfg!(feature = "embedded-runtime"), "text": embedded::NOTICES}),
                );
            } else {
                use std::io::Write;
                std::io::stdout()
                    .lock()
                    .write_all(embedded::NOTICES.as_bytes())?;
            }
            Ok(std::process::ExitCode::SUCCESS)
        }
        Cmd::Docs { topic } => cmd::docs::run(topic.as_deref()),
        Cmd::Motion { action } => cmd::motion::run(action),
        Cmd::Timeline { action } => cmd::timeline::run(action),
        Cmd::Media { action } => dispatch_media(action),
        Cmd::Models { action } => match action {
            ModelsAction::List { json } => cmd::models::run_list(json || output::machine()),
            ModelsAction::Install {
                id,
                version,
                backend,
                refresh_catalog,
                json,
            } => cmd::models::run_install(
                id,
                version,
                backend.into(),
                refresh_catalog,
                json || output::machine(),
            ),
            ModelsAction::Verify {
                id,
                version,
                backend,
                artifact,
                json,
            } => cmd::models::run_verify(
                id,
                version,
                backend.map(Into::into),
                artifact,
                json || output::machine(),
            ),
        },
        Cmd::Project { json, action } => cmd::project::run(json || output::machine(), action),
        Cmd::Assets {
            json,
            events,
            action,
        } => cmd::assets::run(
            json || output::machine(),
            events || output::events(),
            action,
        ),
    }
}

fn dispatch_media(action: MediaAction) -> Result<std::process::ExitCode> {
    match action {
        MediaAction::Capabilities => {
            output::emit(valle_media::codec::ffi::ffmpeg_capabilities()?);
            Ok(std::process::ExitCode::SUCCESS)
        }
        MediaAction::Matte { args } => cmd::matte::run(
            &args.input,
            args.output,
            args.range,
            args.fps,
            args.model,
            args.model_version,
            args.backend,
            args.overwrite,
            args.report,
            args.json || output::machine(),
        ),
        #[cfg(target_os = "windows")]
        MediaAction::Transcribe { args } => cmd::media::render_error(
            &valle_media::tools::ToolError::new(
                valle_media::tools::ToolErrorCode::UnsupportedAdapter,
                "ASR and forced alignment are not supported on Windows yet",
            ),
            args.json || output::machine(),
        ),
        #[cfg(not(target_os = "windows"))]
        MediaAction::Transcribe { args } => cmd::transcribe::run(
            &args.input,
            args.output,
            args.lang.as_deref(),
            args.text_only,
            &args.level,
            args.model,
            args.model_version,
            args.backend,
            args.overwrite,
            args.report,
            args.json || output::machine(),
        ),
        MediaAction::Enhance { args } => cmd::enhance::run(
            &args.input,
            args.output,
            args.model,
            args.model_version,
            args.backend,
            args.overwrite,
            args.report,
            args.json || output::machine(),
        ),
        MediaAction::Separate { args } => cmd::separate::run(
            &args.input,
            args.output_dir,
            args.model,
            args.model_version,
            args.backend,
            args.overwrite,
            args.report,
            args.json || output::machine(),
        ),
        MediaAction::Shots { args } => cmd::shots::run(
            &args.input,
            args.output,
            args.model,
            args.model_version,
            args.backend,
            args.overwrite,
            args.report,
            args.json || output::machine(),
        ),
        MediaAction::Segment { args } => cmd::segment::run(
            &args.input,
            args.output,
            args.foreground_output,
            args.prompt,
            args.range,
            args.threshold,
            args.model,
            args.model_version,
            args.backend,
            args.overwrite,
            args.report,
            args.json || output::machine(),
        ),
        MediaAction::Inpaint { args } => cmd::inpaint::run(
            &args.input,
            &args.mask,
            args.output,
            args.range,
            args.model,
            args.model_version,
            args.backend,
            args.overwrite,
            args.report,
            args.json || output::machine(),
        ),
        MediaAction::Upscale { args } => cmd::upscale::run(
            &args.input,
            args.output,
            args.range,
            args.scale,
            args.model,
            args.model_version,
            args.backend,
            args.overwrite,
            args.report,
            args.json || output::machine(),
        ),
        MediaAction::Interpolate { args } => cmd::interpolate::run(
            &args.input,
            args.output,
            args.fps,
            args.shots,
            args.model,
            args.model_version,
            args.backend,
            args.overwrite,
            args.report,
            args.json || output::machine(),
        ),
    }
}

/// Parse CLI arguments, dispatch the command, and render errors consistently.
pub fn run() -> std::process::ExitCode {
    let argv = std::env::args_os().collect::<Vec<_>>();
    let cli = match Cli::try_parse_from(&argv) {
        Ok(cli) => cli,
        Err(error)
            if argv
                .iter()
                .take_while(|v| *v != "--")
                .any(|v| v == "--json" || v == "--events")
                && !matches!(
                    error.kind(),
                    clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
                ) =>
        {
            // On successful parsing, only clap decides which tokens are flags. For parse errors,
            // honor output flags before `--`, which makes subsequent tokens positional values.
            output::init(
                argv.iter()
                    .take_while(|v| *v != "--")
                    .any(|v| v == "--json"),
                argv.iter()
                    .take_while(|v| *v != "--")
                    .any(|v| v == "--events"),
            );
            let summary = error
                .to_string()
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("invalid command-line arguments")
                .trim()
                .trim_start_matches("error:")
                .trim()
                .to_owned();
            output::error_with_code(
                "invalid_arguments",
                &format!("invalid command-line arguments: {summary}"),
            );
            return std::process::ExitCode::from(2);
        }
        Err(error) => {
            let exit_code = u8::try_from(error.exit_code()).unwrap_or(1);
            if let Err(print_error) = error.print() {
                eprintln!("error: failed to print command-line diagnostic: {print_error}");
            }
            return std::process::ExitCode::from(exit_code);
        }
    };
    output::init(cli.json, cli.events);
    valle_media::codec::ffi::set_ffmpeg_log_level(cli.ffmpeg_log_level.into());
    if let Some(directory) = cli.ffmpeg_dir
        && let Err(error) = valle_media::codec::ffi::set_ffmpeg_directory(directory)
    {
        output::error(&format!("{error:#}"));
        return std::process::ExitCode::FAILURE;
    }
    match dispatch(cli.cmd) {
        Ok(code) => code,
        Err(e) => {
            output::error(&format!("{e:#}"));
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod project_revision_tests {
    use super::*;

    #[test]
    fn project_revision_flags_accept_only_positive_javascript_safe_integers() {
        let valid = parse_project_revision("9007199254740991").unwrap();
        assert_eq!(valid, 9_007_199_254_740_991);

        for argv in [
            vec!["valle", "project", "show", "p1", "--revision", "0"],
            vec![
                "valle",
                "project",
                "history",
                "p1",
                "--cursor",
                "9007199254740992",
            ],
            vec![
                "valle",
                "project",
                "apply",
                "p1",
                "--base-revision",
                "0",
                "--timeline",
                "timeline.json",
            ],
            vec![
                "valle",
                "project",
                "restore",
                "p1",
                "--base-revision",
                "1",
                "--revision",
                "9007199254740992",
            ],
        ] {
            let error = Cli::try_parse_from(argv)
                .err()
                .expect("unsafe Project revision must be rejected by clap");
            assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
            assert!(error.to_string().contains("1 through 9007199254740991"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_size_surface_rejects_noncanonical_flags() {
        for argv in [
            vec![
                "valle",
                "motion",
                "check",
                "scene.motion.tsx",
                "--design-width",
                "1280",
            ],
            vec![
                "valle",
                "motion",
                "studio",
                "scene.motion.tsx",
                "--width",
                "1280",
            ],
        ] {
            let error = Cli::try_parse_from(argv)
                .err()
                .expect("flag must be rejected");
            assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
        }
    }
}

fn parse_canvas_size(value: &str) -> Result<(u32, u32), String> {
    let (w, h) = value.split_once('x').ok_or("size must be WIDTHxHEIGHT")?;
    let width = w.parse::<u32>().map_err(|_| "invalid width")?;
    let height = h.parse::<u32>().map_err(|_| "invalid height")?;
    if width == 0 || height == 0 {
        return Err("size must be positive".into());
    }
    Ok((width, height))
}
