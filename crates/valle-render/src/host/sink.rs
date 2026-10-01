//! Delivery sinks. They receive completed compositor frames and never interpret visual semantics.

use std::{
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use valle_engine::render::{CompiledAudioItem, CompiledRender, FrameKey};
use valle_media::{
    SharedVideoFrame, SharedVideoFramePool,
    codec::{Muxer, TransparentVideoMuxer},
    frame::RgbaFrame,
};

use super::{CompiledAudioMixer, FrameEvidence, NativeResourceCatalog};

pub struct DeliveredFrame<'a> {
    pub sequence: usize,
    pub key: FrameKey,
    pub pixels: &'a RgbaFrame,
    pub evidence: &'a FrameEvidence,
}

pub struct DeliveredSharedFrame<'a> {
    pub sequence: usize,
    pub key: FrameKey,
    pub frame: SharedVideoFrame,
    pub evidence: &'a FrameEvidence,
}

pub trait FrameSink {
    fn push(&mut self, frame: DeliveredFrame<'_>) -> Result<()>;

    /// Returns an encoder-owned pool only when the sink requires direct shared-GPU delivery.
    fn shared_frame_pool(&self) -> Option<SharedVideoFramePool> {
        None
    }

    fn push_shared(&mut self, _frame: DeliveredSharedFrame<'_>) -> Result<()> {
        Err(anyhow!("sink does not accept shared GPU frames"))
    }

    fn finish(&mut self) -> Result<()>;
}

pub struct PngSink {
    path: PathBuf,
    frame: Option<RgbaFrame>,
}

impl PngSink {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            frame: None,
        }
    }
}

impl FrameSink for PngSink {
    fn push(&mut self, frame: DeliveredFrame<'_>) -> Result<()> {
        if self.frame.is_some() {
            return Err(anyhow!("PNG sink accepts exactly one frame"));
        }
        self.frame = Some(frame.pixels.clone());
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        let frame = self
            .frame
            .as_ref()
            .ok_or_else(|| anyhow!("PNG sink received no frame"))?;
        write_png(&self.path, frame)
    }
}

pub struct StoryboardSink {
    path: PathBuf,
    columns: u32,
    cell_width: u32,
    cell_height: u32,
    cells: usize,
    canvas: RgbaFrame,
}

impl StoryboardSink {
    pub fn new(
        path: impl Into<PathBuf>,
        cells: usize,
        columns: u32,
        cell_width: u32,
        cell_height: u32,
    ) -> Result<Self> {
        if columns == 0 || cells == 0 || cell_width == 0 || cell_height == 0 {
            return Err(anyhow!(
                "storyboard needs non-empty cells and positive columns"
            ));
        }
        let rows = u32::try_from(cells)
            .map_err(|_| anyhow!("too many storyboard cells"))?
            .div_ceil(columns);
        let width = cell_width
            .checked_mul(columns)
            .ok_or_else(|| anyhow!("storyboard width overflow"))?;
        let height = cell_height
            .checked_mul(rows)
            .ok_or_else(|| anyhow!("storyboard height overflow"))?;
        // A contact sheet is assembled in memory. Bound the allocation and all u32 row indices;
        // authors can reduce cell size with --output-size or select fewer frames.
        let bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(4));
        if bytes.is_none_or(|bytes| bytes > 512 * 1024 * 1024) {
            return Err(anyhow!(
                "storyboard exceeds 512 MiB; select fewer frames or reduce --output-size"
            ));
        }
        Ok(Self {
            path: path.into(),
            columns,
            cell_width,
            cell_height,
            cells,
            canvas: RgbaFrame::new(width, height),
        })
    }
}

impl FrameSink for StoryboardSink {
    fn push(&mut self, frame: DeliveredFrame<'_>) -> Result<()> {
        if frame.sequence >= self.cells {
            return Err(anyhow!("storyboard frame exceeds its cell count"));
        }
        let fitted = contain_rgba(frame.pixels, self.cell_width, self.cell_height)?;
        let sequence = u32::try_from(frame.sequence)
            .map_err(|_| anyhow!("storyboard sequence does not fit u32"))?;
        let x = (sequence % self.columns) * self.cell_width;
        let y = (sequence / self.columns) * self.cell_height;
        // Cells do not overlap. Copy straight RGBA verbatim so a sheet cell is exactly the
        // corresponding standalone PNG, including transparent and partially covered pixels.
        for row in 0..fitted.height {
            let source = (row * fitted.width * 4) as usize;
            let destination = (((y + row) * self.canvas.width + x) * 4) as usize;
            let length = (fitted.width * 4) as usize;
            self.canvas.data[destination..destination + length]
                .copy_from_slice(&fitted.data[source..source + length]);
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        write_png(&self.path, &self.canvas)
    }
}

/// Save each frame immediately and optionally copy the same pixels to a contact sheet.
pub(super) struct PngFramesSink {
    pub paths: Vec<PathBuf>,
    pub storyboard: Option<StoryboardSink>,
}

impl FrameSink for PngFramesSink {
    fn push(&mut self, frame: DeliveredFrame<'_>) -> Result<()> {
        if !self.paths.is_empty() {
            let path = self
                .paths
                .get(frame.sequence)
                .context("PNG frame has no output path")?;
            write_png(path, frame.pixels)?;
        }
        if let Some(storyboard) = &mut self.storyboard {
            storyboard.push(frame)?;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        if let Some(storyboard) = &mut self.storyboard {
            storyboard.finish()?;
        }
        Ok(())
    }
}

pub(super) struct TransparentVideoSink(pub TransparentVideoMuxer);

impl FrameSink for TransparentVideoSink {
    fn push(&mut self, frame: DeliveredFrame<'_>) -> Result<()> {
        self.0.encode_video(frame.pixels)
    }
    fn finish(&mut self) -> Result<()> {
        self.0.finish()
    }
}

pub struct Mp4Sink {
    muxer: Muxer,
    audio: Option<CompiledAudioMixer>,
    start_frame_boundary: i64,
    video_encode_us: u64,
    audio_encode_us: u64,
}

impl Mp4Sink {
    pub fn new(
        mut muxer: Muxer,
        render: Arc<CompiledRender>,
        catalog: Arc<NativeResourceCatalog>,
        start_frame_boundary: i64,
    ) -> Result<Self> {
        let has_audio = render.audio().tracks().iter().any(|track| {
            track.items().iter().any(|item| {
                matches!(
                    item,
                    CompiledAudioItem::Clip { .. } | CompiledAudioItem::Crossfade { .. }
                )
            })
        });
        let audio = has_audio
            .then(|| {
                muxer.expect_external_audio();
                let (sample_rate, channels) = muxer.audio_format();
                CompiledAudioMixer::new(
                    Arc::clone(&render),
                    Arc::clone(&catalog),
                    sample_rate,
                    channels,
                    start_frame_boundary,
                )
            })
            .transpose()?;
        Ok(Self {
            muxer,
            audio,
            start_frame_boundary,
            video_encode_us: 0,
            audio_encode_us: 0,
        })
    }

    fn advance_audio(&mut self, sequence: usize) -> Result<()> {
        let started = std::time::Instant::now();
        if let Some(audio) = &mut self.audio {
            let completed = i64::try_from(sequence)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| anyhow!("render sequence exceeds the compiled frame clock"))?;
            let target_frame = self
                .start_frame_boundary
                .checked_add(completed)
                .ok_or_else(|| anyhow!("render frame boundary overflow"))?;
            if let Some(samples) = audio.mix_until_frame_boundary(target_frame)? {
                self.muxer.encode_audio(&samples)?;
            }
        }
        self.audio_encode_us = self
            .audio_encode_us
            .saturating_add(u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX));
        Ok(())
    }
}

impl FrameSink for Mp4Sink {
    fn push(&mut self, frame: DeliveredFrame<'_>) -> Result<()> {
        let started = std::time::Instant::now();
        self.muxer.encode_video(frame.pixels)?;
        self.video_encode_us = self
            .video_encode_us
            .saturating_add(u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX));
        self.advance_audio(frame.sequence)
    }

    fn shared_frame_pool(&self) -> Option<SharedVideoFramePool> {
        self.muxer.shared_frame_pool()
    }

    fn push_shared(&mut self, frame: DeliveredSharedFrame<'_>) -> Result<()> {
        let started = std::time::Instant::now();
        self.muxer.encode_shared(frame.frame)?;
        self.video_encode_us = self
            .video_encode_us
            .saturating_add(u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX));
        self.advance_audio(frame.sequence)
    }

    fn finish(&mut self) -> Result<()> {
        let started = std::time::Instant::now();
        let result = self.muxer.finish();
        if std::env::var_os("VALLE_PERF").is_some() {
            eprintln!(
                "[valle delivery] video-encode={:.3}ms audio-encode={:.3}ms finish={:.3}ms",
                self.video_encode_us as f64 / 1_000.0,
                self.audio_encode_us as f64 / 1_000.0,
                started.elapsed().as_secs_f64() * 1_000.0,
            );
        }
        result
    }
}

pub fn write_png(path: &Path, frame: &RgbaFrame) -> Result<()> {
    let file = File::create(path).with_context(|| format!("create {}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    // The default level spends ~100 ms on a 1080p frame of smooth gradients or blur in lazy
    // match searching, a fifth of a preview. Level 2 encodes those 4-5x faster for 25-45%
    // larger files, and flat or text frames grow ~5%.
    encoder.set_deflate_compression(png::DeflateCompression::Level(2));
    let mut writer = encoder.write_header().context("write PNG header")?;
    writer
        .write_image_data(&frame.data)
        .context("write PNG pixels")
}

fn contain_rgba(source: &RgbaFrame, width: u32, height: u32) -> Result<RgbaFrame> {
    if width == 0 || height == 0 || source.width == 0 || source.height == 0 {
        return Err(anyhow!("contain resize requires non-empty frames"));
    }
    let scale = (f64::from(width) / f64::from(source.width))
        .min(f64::from(height) / f64::from(source.height));
    let target_width = (f64::from(source.width) * scale).round().max(1.0) as u32;
    let target_height = (f64::from(source.height) * scale).round().max(1.0) as u32;
    let offset_x = (width - target_width) / 2;
    let offset_y = (height - target_height) / 2;
    let mut output = RgbaFrame::new(width, height);
    for y in 0..target_height {
        let source_y = ((u64::from(y) * u64::from(source.height)) / u64::from(target_height))
            .min(u64::from(source.height - 1)) as u32;
        for x in 0..target_width {
            let source_x = ((u64::from(x) * u64::from(source.width)) / u64::from(target_width))
                .min(u64::from(source.width - 1)) as u32;
            let source_index = ((source_y * source.width + source_x) * 4) as usize;
            let target_index = (((y + offset_y) * width + x + offset_x) * 4) as usize;
            output.data[target_index..target_index + 4]
                .copy_from_slice(&source.data[source_index..source_index + 4]);
        }
    }
    Ok(output)
}
