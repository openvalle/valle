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
    frame::{RenderQuality, RenderSpec},
    render::{FrameKey, RenderId},
    resource::{
        Dither, GamutMap, OutputAlphaMode, OutputBackground, OutputBitDepth, OutputColorEncoding,
        OutputSpec, SignalLuminance, ToneMap,
    },
};
use valle_media::codec::{AudioMuxer, Muxer, VideoFrameTransport};

use crate::executor::skia::SkiaBackendKind;

use super::{
    CompiledAudioMixer, FrameSchedule, Mp4Sink, NativeProject, PipelineError, PipelineReport,
    PngSink, ProgressCallback, RenderControl, StoryboardSink, render_schedule,
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
    /// Requested frame concurrency. `None` selects a bounded backend-aware default; zero is
    /// rejected. A value above one creates independent ordered runners. Exactly one splits a
    /// single runner into one CPU preparation owner and one physical executor with at most one
    /// queued `PreparedFrame`; it never creates a second GPU context.
    pub raster_workers: Option<usize>,
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
            raster_workers: None,
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

pub struct NativeRenderer {
    project: NativeProject,
    options: NativeRenderOptions,
}

impl NativeRenderer {
    pub fn new(project: NativeProject, options: NativeRenderOptions) -> Self {
        Self { project, options }
    }

    pub fn project(&self) -> &NativeProject {
        &self.project
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
        let (width, height) = self.output_extent()?;
        let spec = RenderSpec::new(
            width,
            height,
            RenderQuality::Preview,
            OutputSpec::srgb_preview(self.options.background)?,
        )?;
        let frame_count = self.project.compiled().canvas().frame_count();
        if frame.index() < 0 || frame.index() >= frame_count {
            return Err(NativeRenderError::FrameOutOfRange {
                frame: frame.index(),
                frame_count,
            });
        }
        let frames = [frame];
        let temporary = temporary_output(output)?;
        let mut sink = PngSink::new(&temporary);
        let rendered = render_schedule(
            &self.project,
            &frames,
            spec,
            &mut sink,
            &self.options.control,
            self.options.progress.as_ref(),
            self.options.backend,
            self.options.raster_workers,
        );
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
        let spec = RenderSpec::new(
            cell_width,
            cell_height,
            RenderQuality::Preview,
            OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0]))?,
        )?;
        let temporary = temporary_output(output)?;
        let mut sink =
            StoryboardSink::new(&temporary, frames.len(), columns, cell_width, cell_height)?;
        let rendered = render_schedule(
            &self.project,
            &frames,
            spec,
            &mut sink,
            &self.options.control,
            self.options.progress.as_ref(),
            self.options.backend,
            self.options.raster_workers,
        );
        let report = finish_delivery(rendered, &temporary, output)?;
        self.validate_report_render_id(&report)?;
        let actual_rows = u32::try_from(frames.len())
            .map_err(|_| NativeRenderError::InvalidStoryboard)?
            .div_ceil(columns);
        let output_width = cell_width
            .checked_mul(columns)
            .ok_or(NativeRenderError::InvalidStoryboard)?;
        let output_height = cell_height
            .checked_mul(actual_rows)
            .ok_or(NativeRenderError::InvalidStoryboard)?;
        Ok(RenderSummary {
            render_id: self.project.render_id(),
            frames: report.frames,
            width: output_width,
            height: output_height,
            audio_only: false,
            pipeline: Some(report),
        })
    }

    pub fn export_mp4(&self, output: &Path) -> Result<RenderSummary, NativeRenderError> {
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
        let muxer = Muxer::open_ext_rational_with_transport(
            &temporary,
            width,
            height,
            fps_num,
            fps.denominator(),
            self.options.bitrate,
            self.options.hardware_encode,
            self.options.encode_threads,
            selected_video_transport(self.options.hardware_encode, self.options.backend),
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
            &self.options.control,
            self.options.progress.as_ref(),
            self.options.backend,
            self.options.raster_workers,
        );
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
        let muxer = Muxer::open_ext_rational_with_transport(
            &temporary,
            width,
            height,
            fps_num,
            fps.denominator(),
            self.options.bitrate,
            self.options.hardware_encode,
            self.options.encode_threads,
            selected_video_transport(self.options.hardware_encode, self.options.backend),
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
            &self.options.control,
            self.options.progress.as_ref(),
            self.options.backend,
            self.options.raster_workers,
        );
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
                if let Some(samples) = mixer.mix_until_sample(target)? {
                    muxer.encode_audio(&samples)?;
                }
            }
            self.options.control.check()?;
            muxer.finish()?;
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

fn selected_video_transport(
    hardware_encode: bool,
    backend: SkiaBackendKind,
) -> VideoFrameTransport {
    if !hardware_encode {
        return VideoFrameTransport::Cpu;
    }
    #[cfg(target_os = "macos")]
    if backend == SkiaBackendKind::Metal {
        return VideoFrameTransport::SharedGpu;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = backend;
    VideoFrameTransport::Cpu
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
