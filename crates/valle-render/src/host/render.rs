//! Product-facing Native delivery API built on the single frame pipeline.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use thiserror::Error;
use valle_engine::{
    compositor::inspect::FrameCost,
    frame::{RenderQuality, RenderSpec},
    render::{FrameKey, RenderId},
    resource::{
        Dither, GamutMap, OutputAlphaMode, OutputBackground, OutputBitDepth, OutputColorEncoding,
        OutputSpec, SignalLuminance, ToneMap,
    },
};
use valle_media::codec::{
    AudioMuxer, Mp4VideoCodec, Muxer, TransparentVideoCodec, TransparentVideoMuxer,
    VideoFrameTransport,
};

use crate::executor::skia::SkiaBackendKind;

use super::{
    CompiledAudioMixer, FrameRenderError, FrameSchedule, Mp4Sink, NativeProject, PipelineError,
    PipelineReport, ProgressCallback, RenderControl, StoryboardSink,
    delivery::OutputTransaction,
    render_schedule,
    sink::{PngFramesSink, TransparentVideoSink},
};

#[derive(Clone)]
pub struct NativeRenderOptions {
    /// Immutable physical compositor backend used by every frame runner in this delivery.
    pub backend: SkiaBackendKind,
    pub output_size: Option<(u32, u32)>,
    pub background: OutputBackground,
    pub bitrate: Option<usize>,
    pub hardware_encode: bool,
    pub encode_threads: Option<usize>,
    /// Shared across deliveries created by this renderer. Native driver/codec internals are not a process hard cap.
    pub budget: super::NativeRenderBudget,
    pub control: RenderControl,
    pub progress: Option<ProgressCallback>,
}

impl Default for NativeRenderOptions {
    fn default() -> Self {
        Self {
            backend: SkiaBackendKind::Raster,
            output_size: None,
            background: OutputBackground::opaque_srgb([0, 0, 0]),
            bitrate: None,
            hardware_encode: false,
            encode_threads: None,
            budget: super::NativeRenderBudget::default(),
            control: RenderControl::default(),
            progress: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSummary {
    pub render_id: RenderId,
    pub frames: usize,
    pub width: u32,
    pub height: u32,
    pub audio_only: bool,
    pub pipeline: Option<PipelineReport>,
}

/// One frame's planned cost, or the reason the frame cannot be planned.
pub type PlannedFrameCost = (FrameKey, Result<FrameCost, FrameRenderError>);

pub struct NativeRenderer {
    project: NativeProject,
    options: NativeRenderOptions,
}

impl NativeRenderer {
    pub fn new(project: NativeProject, options: NativeRenderOptions) -> Self {
        Self { project, options }
    }

    fn encoder_slots(&self) -> Result<usize, NativeRenderError> {
        let requested = self
            .options
            .encode_threads
            .unwrap_or_else(|| (self.options.budget.threads() / 4).max(1));
        if requested == 0 || requested > self.options.budget.threads().saturating_sub(1).max(1) {
            return Err(super::NativeBudgetError::EncoderShare {
                requested,
                available: self.options.budget.threads().saturating_sub(1).max(1),
            }
            .into());
        }
        Ok(requested)
    }

    fn render_control(&self) -> RenderControl {
        self.options
            .control
            .clone()
            .with_budget(self.options.budget.clone())
    }

    fn reserve_codec(
        &self,
        hardware: bool,
    ) -> Result<Option<valle_engine::motion::cpu::CodecReservation>, NativeRenderError> {
        if hardware {
            return Ok(None);
        }
        let slots = self.encoder_slots()?;
        // A one-thread software codec is synchronous; the sink accounts for its caller. Higher
        // codec counts can keep computing between encode_video calls, so retain their share.
        if slots > 1 {
            Ok(Some(
                self.options
                    .budget
                    .cpu
                    .reserve_codec_slots(slots, || self.options.control.check())?,
            ))
        } else {
            Ok(None)
        }
    }

    fn encoder_threads(&self, hardware: bool) -> Result<Option<usize>, NativeRenderError> {
        if hardware {
            Ok(None)
        } else {
            Ok(Some(self.encoder_slots()?))
        }
    }

    pub fn project(&self) -> &NativeProject {
        &self.project
    }

    /// Plan every frame as delivery would, without fulfilling resources or rendering, and
    /// return each frame's planned cost or the reason it cannot be planned.
    pub fn plan_costs(&self) -> Result<Vec<PlannedFrameCost>, NativeRenderError> {
        let frames = FrameSchedule::All.frames(self.project.compiled().canvas())?;
        let (width, height) = self.output_extent()?;
        let spec = RenderSpec::new(
            width,
            height,
            RenderQuality::Final,
            OutputSpec::srgb_preview(self.options.background)?,
        )?;
        let capabilities = super::frame::delivery_capabilities().map_err(anyhow::Error::from)?;
        let render = self.project.render();
        let mut compiler = render.frame_compiler();
        Ok(frames
            .into_iter()
            .map(|frame| {
                let cost = super::frame::plan_frame_cost(
                    render,
                    &mut compiler,
                    &capabilities,
                    frame,
                    spec,
                );
                (frame, cost)
            })
            .collect())
    }

    pub fn preview_frame(
        &self,
        at_seconds: f64,
        output: &Path,
    ) -> Result<RenderSummary, NativeRenderError> {
        let frame = FrameSchedule::Single { at_seconds }
            .frames(self.project.compiled().canvas())?
            .into_iter()
            .next()
            .ok_or(NativeRenderError::EmptyProject)?;
        self.preview_frame_key(frame, output)
    }

    /// Render one exact frame already selected in the admitted render clock.
    ///
    /// Product/CLI callers use this entry point so no boundary can independently quantize a
    /// timestamp. Time-oriented UI callers may use [`Self::preview_frame`], whose quantization is
    /// owned by the same fixed render's [`CompiledCanvas`](valle_engine::render::CompiledCanvas).
    pub fn preview_frame_key(
        &self,
        frame: FrameKey,
        output: &Path,
    ) -> Result<RenderSummary, NativeRenderError> {
        self.render_png_frames(&[frame], &[output.to_owned()], None)
    }

    /// Deliver exact frame keys in caller order, retaining duplicates for contact sheets.
    /// A sequence and a sheet share the same frame evaluations and RGBA pixels. All paths are
    /// staged before publication; existing outputs and invalid keys fail without partial output.
    pub fn render_png_frames(
        &self,
        frames: &[FrameKey],
        outputs: &[PathBuf],
        storyboard: Option<(&Path, u32)>,
    ) -> Result<RenderSummary, NativeRenderError> {
        if frames.is_empty()
            || (!outputs.is_empty() && outputs.len() != frames.len())
            || (outputs.is_empty() && storyboard.is_none())
        {
            return Err(anyhow::anyhow!(
                "PNG delivery requires frames and matching output paths or a storyboard"
            )
            .into());
        }
        self.validate_frames(frames)?;
        let (width, height) = self.output_extent()?;
        let spec = RenderSpec::new(
            width,
            height,
            RenderQuality::Preview,
            OutputSpec::srgb_preview(self.options.background)?,
        )?;
        let mut paths = outputs.to_vec();
        if let Some((path, _)) = storyboard {
            paths.push(path.to_owned());
        }
        let transaction = OutputTransaction::new(&paths)?;
        let sheet = storyboard
            .map(|(_, columns)| {
                StoryboardSink::new(
                    transaction.temporary(outputs.len()),
                    frames.len(),
                    columns,
                    width,
                    height,
                )
            })
            .transpose()?;
        let (delivered_width, delivered_height) = if let Some((_, columns)) = storyboard {
            (
                width
                    .checked_mul(columns)
                    .ok_or(NativeRenderError::InvalidStoryboard)?,
                height
                    .checked_mul(
                        u32::try_from(frames.len())
                            .map_err(|_| NativeRenderError::InvalidStoryboard)?
                            .div_ceil(columns),
                    )
                    .ok_or(NativeRenderError::InvalidStoryboard)?,
            )
        } else {
            (width, height)
        };
        let mut sink = PngFramesSink {
            paths: (0..outputs.len())
                .map(|index| transaction.temporary(index).to_owned())
                .collect(),
            storyboard: sheet,
        };
        let report = render_schedule(
            &self.project,
            frames,
            spec,
            &mut sink,
            &self
                .render_control()
                .with_budget(self.options.budget.for_preview()),
            self.options.progress.as_ref(),
            self.options.backend,
        )?;
        self.validate_report_render_id(&report)?;
        self.options.control.check()?;
        transaction.publish()?;
        Ok(RenderSummary {
            render_id: self.project.render_id(),
            frames: report.frames,
            width: delivered_width,
            height: delivered_height,
            audio_only: false,
            pipeline: Some(report),
        })
    }

    fn validate_frames(&self, frames: &[FrameKey]) -> Result<(), NativeRenderError> {
        let frame_count = self.project.compiled().canvas().frame_count();
        for frame in frames {
            if frame.index() < 0 || frame.index() >= frame_count {
                return Err(NativeRenderError::FrameOutOfRange {
                    frame: frame.index(),
                    frame_count,
                });
            }
        }
        Ok(())
    }

    pub fn storyboard(
        &self,
        output: &Path,
        columns: u32,
        rows: u32,
    ) -> Result<RenderSummary, NativeRenderError> {
        let cells = columns
            .checked_mul(rows)
            .filter(|cells| *cells > 0)
            .ok_or(NativeRenderError::InvalidStoryboard)?;
        let canvas = self.project.compiled().canvas();
        let duration = canvas.duration().as_f64();
        let times = (0..cells)
            .map(|index| duration * (f64::from(index) + 0.5) / f64::from(cells))
            .collect();
        let frames = FrameSchedule::Sparse {
            times_seconds: times,
        }
        .frames(canvas)?;
        let [cell_width, cell_height] = storyboard_extent(canvas.width(), canvas.height());
        let mut options = self.options.clone();
        options.output_size = Some((cell_width, cell_height));
        NativeRenderer::new(self.project.clone(), options).render_png_frames(
            &frames,
            &[],
            Some((output, columns)),
        )
    }

    /// Lossless qtrle MOV using the same straight sRGB RGBA conversion as PNG delivery.
    pub fn export_transparent_mov(
        &self,
        output: &Path,
    ) -> Result<RenderSummary, NativeRenderError> {
        self.export_transparent_mov_with_codec(output, TransparentVideoCodec::Qtrle)
    }

    /// Transparent MOV using qtrle or ProRes 4444.
    pub fn export_transparent_mov_with_codec(
        &self,
        output: &Path,
        codec: TransparentVideoCodec,
    ) -> Result<RenderSummary, NativeRenderError> {
        if self.options.hardware_encode
            || self.options.bitrate.is_some()
            || self.options.encode_threads.is_some()
        {
            return Err(
                anyhow::anyhow!("transparent MOV does not accept H.264 encoder options").into(),
            );
        }
        if self.project.has_audio() {
            return Err(anyhow::anyhow!("transparent MOV currently delivers video only; export audio separately or use MP4 for an audio/video mix").into());
        }
        let (width, height) = self.output_extent()?;
        let spec = RenderSpec::new(
            width,
            height,
            RenderQuality::Final,
            OutputSpec::srgb_preview(self.options.background)?,
        )?;
        let canvas = self.project.compiled().canvas();
        let frames = FrameSchedule::All.frames(canvas)?;
        let transaction = OutputTransaction::new(&[output.to_owned()])?;
        let fps = canvas.frame_rate();
        let fps_num =
            u32::try_from(fps.numerator()).map_err(|_| NativeRenderError::InvalidFrameRate)?;
        let _codec = self.reserve_codec(false)?;
        let muxer = TransparentVideoMuxer::open_with_codec_and_threads(
            transaction.temporary(0),
            width,
            height,
            fps_num,
            fps.denominator(),
            codec,
            self.encoder_slots()?,
        )?;
        let mut sink = TransparentVideoSink(muxer);
        let report = render_schedule(
            &self.project,
            &frames,
            spec,
            &mut sink,
            &self.render_control(),
            self.options.progress.as_ref(),
            self.options.backend,
        )?;
        drop(sink);
        self.validate_report_render_id(&report)?;
        self.options.control.check()?;
        transaction.publish()?;
        Ok(RenderSummary {
            render_id: self.project.render_id(),
            frames: report.frames,
            width,
            height,
            audio_only: false,
            pipeline: Some(report),
        })
    }

    pub fn export_mp4(&self, output: &Path) -> Result<RenderSummary, NativeRenderError> {
        self.export_mp4_with_codec(output, Mp4VideoCodec::H264)
    }

    /// MP4 with the given video codec. HEVC always uses the platform hardware encoder.
    pub fn export_mp4_with_codec(
        &self,
        output: &Path,
        codec: Mp4VideoCodec,
    ) -> Result<RenderSummary, NativeRenderError> {
        let (width, height) = self.output_extent()?;
        if !self.project.has_visual_material() {
            return self.export_audio_only(output);
        }
        require_video_extent(width, height)?;
        let background = match self.options.background {
            OutputBackground::AuthorSrgbStraight { color } if color.0[3] == u8::MAX => {
                self.options.background
            }
            _ => return Err(NativeRenderError::VideoNeedsOpaqueBackground),
        };
        let output_spec = OutputSpec::new(
            OutputColorEncoding::REC709,
            OutputAlphaMode::Opaque,
            background,
            ToneMap::None,
            GamutMap::ChromaCompress,
            Dither::None,
            OutputBitDepth::Eight,
            SignalLuminance::SDR_100,
        )?;
        let spec = RenderSpec::new(width, height, RenderQuality::Final, output_spec)?;
        let canvas = self.project.compiled().canvas();
        let frames = FrameSchedule::All.frames(canvas)?;
        let temporary = temporary_output(output)?;
        let fps = canvas.frame_rate();
        let fps_num =
            u32::try_from(fps.numerator()).map_err(|_| NativeRenderError::InvalidFrameRate)?;
        let hardware = self.options.hardware_encode || codec == Mp4VideoCodec::Hevc;
        let _codec = self.reserve_codec(hardware)?;
        let muxer = Muxer::open_ext_rational_with_transport(
            &temporary,
            width,
            height,
            fps_num,
            fps.denominator(),
            codec,
            self.options.bitrate,
            hardware,
            self.encoder_threads(hardware)?,
            selected_video_transport(hardware),
        )?;
        let mut sink = Mp4Sink::new(
            muxer,
            self.project.compiled_arc(),
            Arc::clone(self.project.catalog()),
            0,
        )?;
        let rendered = render_schedule(
            &self.project,
            &frames,
            spec,
            &mut sink,
            &self.render_control(),
            self.options.progress.as_ref(),
            self.options.backend,
        );
        // Release FFmpeg's output handle before publishing or deleting the file on Windows.
        drop(sink);
        let report = finish_delivery(rendered, &temporary, output)?;
        self.validate_report_render_id(&report)?;
        Ok(RenderSummary {
            render_id: self.project.render_id(),
            frames: report.frames,
            width,
            height,
            audio_only: false,
            pipeline: Some(report),
        })
    }

    pub fn preview_range(
        &self,
        from_seconds: f64,
        to_seconds: f64,
        output: &Path,
        maximum_long_edge: u32,
    ) -> Result<RenderSummary, NativeRenderError> {
        let (source_width, source_height) = self.output_extent()?;
        let (width, height) =
            bounded_even_extent(source_width, source_height, maximum_long_edge.max(2))?;
        let output_spec = OutputSpec::new(
            OutputColorEncoding::REC709,
            OutputAlphaMode::Opaque,
            OutputBackground::opaque_srgb([0, 0, 0]),
            ToneMap::None,
            GamutMap::ChromaCompress,
            Dither::None,
            OutputBitDepth::Eight,
            SignalLuminance::SDR_100,
        )?;
        let spec = RenderSpec::new(width, height, RenderQuality::Preview, output_spec)?;
        let frames = FrameSchedule::Range {
            from_seconds,
            to_seconds,
        }
        .frames(self.project.compiled().canvas())?;
        let start_frame_boundary = frames.first().map_or(0, |frame| frame.index());
        let temporary = temporary_output(output)?;
        let fps = self.project.compiled().canvas().frame_rate();
        let fps_num =
            u32::try_from(fps.numerator()).map_err(|_| NativeRenderError::InvalidFrameRate)?;
        let _codec = self.reserve_codec(self.options.hardware_encode)?;
        let muxer = Muxer::open_ext_rational_with_transport(
            &temporary,
            width,
            height,
            fps_num,
            fps.denominator(),
            Mp4VideoCodec::H264,
            self.options.bitrate,
            self.options.hardware_encode,
            self.encoder_threads(self.options.hardware_encode)?,
            selected_video_transport(self.options.hardware_encode),
        )?;
        let mut sink = Mp4Sink::new(
            muxer,
            self.project.compiled_arc(),
            Arc::clone(self.project.catalog()),
            start_frame_boundary,
        )?;
        let rendered = render_schedule(
            &self.project,
            &frames,
            spec,
            &mut sink,
            &self.render_control(),
            self.options.progress.as_ref(),
            self.options.backend,
        );
        // Release FFmpeg's output handle before publishing or deleting the file on Windows.
        drop(sink);
        let report = finish_delivery(rendered, &temporary, output)?;
        self.validate_report_render_id(&report)?;
        Ok(RenderSummary {
            render_id: self.project.render_id(),
            frames: report.frames,
            width,
            height,
            audio_only: false,
            pipeline: Some(report),
        })
    }

    fn export_audio_only(&self, output: &Path) -> Result<RenderSummary, NativeRenderError> {
        if !self.project.has_audio() {
            return Err(NativeRenderError::EmptyProject);
        }
        let temporary = temporary_output(output)?;
        let rendered = (|| -> Result<(), NativeRenderError> {
            let mut muxer = AudioMuxer::open(&temporary)?;
            let (sample_rate, channels) = muxer.audio_format();
            let mut mixer = CompiledAudioMixer::new(
                self.project.compiled_arc(),
                Arc::clone(self.project.catalog()),
                sample_rate,
                channels,
                0,
            )?;
            let sample_count = self.project.compiled().canvas().sample_count();
            let chunk = i64::from(sample_rate);
            let mut target = 0_i64;
            while target < sample_count {
                self.options.control.check()?;
                target = target.saturating_add(chunk).min(sample_count);
                self.options
                    .budget
                    .cpu
                    .run(|| -> Result<(), NativeRenderError> {
                        if let Some(samples) = mixer.mix_until_sample(target)? {
                            muxer.encode_audio(&samples)?;
                        }
                        Ok(())
                    })?;
            }
            self.options.control.check()?;
            self.options.budget.cpu.run(|| muxer.finish())?;
            Ok(())
        })();
        if let Err(error) = rendered {
            remove_temporary(&temporary);
            return Err(error);
        }
        publish_output(&temporary, output)?;
        let (width, height) = self.output_extent()?;
        Ok(RenderSummary {
            render_id: self.project.render_id(),
            frames: 0,
            width,
            height,
            audio_only: true,
            pipeline: None,
        })
    }

    fn output_extent(&self) -> Result<(u32, u32), NativeRenderError> {
        let (width, height) = self.options.output_size.unwrap_or((
            self.project.compiled().canvas().width(),
            self.project.compiled().canvas().height(),
        ));
        if width == 0 || height == 0 {
            return Err(NativeRenderError::InvalidExtent { width, height });
        }
        Ok((width, height))
    }

    fn validate_report_render_id(&self, report: &PipelineReport) -> Result<(), NativeRenderError> {
        let expected = self.project.render_id();
        if report.render_id != expected {
            return Err(NativeRenderError::RenderMismatch {
                expected,
                actual: report.render_id,
            });
        }
        Ok(())
    }
}

fn require_video_extent(width: u32, height: u32) -> Result<(), NativeRenderError> {
    if width < 2 || height < 2 || width % 2 != 0 || height % 2 != 0 {
        return Err(NativeRenderError::InvalidVideoExtent { width, height });
    }
    Ok(())
}

/// A macOS hardware encoder hands out BGRA surfaces that either backend fills in place: Metal
/// renders into them and Raster writes them through a CPU mapping. VideoToolbox converts to YUV.
fn selected_video_transport(hardware_encode: bool) -> VideoFrameTransport {
    if hardware_encode && cfg!(target_os = "macos") {
        VideoFrameTransport::SharedGpu
    } else {
        VideoFrameTransport::Cpu
    }
}

fn storyboard_extent(width: u32, height: u32) -> [u32; 2] {
    const LONG_EDGE: u32 = 320;
    if width >= height {
        [
            LONG_EDGE,
            ((u64::from(LONG_EDGE) * u64::from(height)) / u64::from(width)).max(1) as u32,
        ]
    } else {
        [
            ((u64::from(LONG_EDGE) * u64::from(width)) / u64::from(height)).max(1) as u32,
            LONG_EDGE,
        ]
    }
}

fn bounded_even_extent(
    width: u32,
    height: u32,
    maximum_long_edge: u32,
) -> Result<(u32, u32), NativeRenderError> {
    let scale = (f64::from(maximum_long_edge) / f64::from(width.max(height))).min(1.0);
    let even = |value: f64| ((value.floor() as u32).max(2)) & !1;
    let result = (
        even(f64::from(width) * scale),
        even(f64::from(height) * scale),
    );
    require_video_extent(result.0, result.1)?;
    Ok(result)
}

fn temporary_output(output: &Path) -> Result<PathBuf, NativeRenderError> {
    static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let stem = output
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| NativeRenderError::InvalidOutput(output.to_path_buf()))?;
    let extension = output
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty())
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default();
    let nonce = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    Ok(parent.join(format!(
        ".{stem}.valle-part-{}-{epoch}-{nonce}{extension}",
        std::process::id()
    )))
}

fn finish_delivery(
    rendered: Result<PipelineReport, PipelineError>,
    temporary: &Path,
    output: &Path,
) -> Result<PipelineReport, NativeRenderError> {
    match rendered {
        Ok(report) => {
            if let Err(error) = publish_output(temporary, output) {
                remove_temporary(temporary);
                Err(error)
            } else {
                Ok(report)
            }
        }
        Err(error) => {
            remove_temporary(temporary);
            Err(error.into())
        }
    }
}

fn remove_temporary(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {}
    }
}

fn publish_output(temporary: &Path, output: &Path) -> Result<(), NativeRenderError> {
    std::fs::rename(temporary, output).map_err(|error| NativeRenderError::Publish {
        temporary: temporary.to_path_buf(),
        output: output.to_path_buf(),
        reason: error.to_string(),
    })
}

#[derive(Debug, Error)]
pub enum NativeRenderError {
    #[error(transparent)]
    Budget(#[from] super::NativeBudgetError),
    #[error(transparent)]
    Pipeline(#[from] PipelineError),
    #[error(transparent)]
    RenderSpec(#[from] valle_engine::frame::RenderSpecError),
    #[error(transparent)]
    OutputSpec(#[from] valle_engine::resource::OutputSpecError),
    #[error(transparent)]
    Audio(#[from] super::AudioMixError),
    #[error(transparent)]
    Schedule(#[from] super::FrameScheduleError),
    #[error(transparent)]
    Media(#[from] anyhow::Error),
    #[error("render extent must be non-empty, got {width}x{height}")]
    InvalidExtent { width: u32, height: u32 },
    #[error("H.264 delivery requires even dimensions of at least 2x2, got {width}x{height}")]
    InvalidVideoExtent { width: u32, height: u32 },
    #[error("H.264 delivery requires an explicit opaque background")]
    VideoNeedsOpaqueBackground,
    #[error("storyboard rows and columns must describe at least one cell")]
    InvalidStoryboard,
    #[error("project has neither visual nor audio content")]
    EmptyProject,
    #[error("compiled frame rate is not representable by the Native muxer")]
    InvalidFrameRate,
    #[error("preview frame {frame} is outside admitted range 0..{frame_count}")]
    FrameOutOfRange { frame: i64, frame_count: i64 },
    #[error("render report render {actual} does not match project render {expected}")]
    RenderMismatch {
        expected: RenderId,
        actual: RenderId,
    },
    #[error("invalid output path {0}")]
    InvalidOutput(PathBuf),
    #[error("could not publish {temporary} to {output}: {reason}")]
    Publish {
        temporary: PathBuf,
        output: PathBuf,
        reason: String,
    },
}
