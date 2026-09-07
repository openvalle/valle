//! Delivery sinks. They receive completed compositor frames and never interpret visual semantics.

use std::{
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use valle_engine::render::{CompiledAudioItem, CompiledRender, FrameKey};
use valle_media::{SharedVideoFrame, SharedVideoFramePool, codec::Muxer, frame::RgbaFrame};

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
        let columns = columns.max(1);
        let cells = u32::try_from(cells).map_err(|_| anyhow!("too many storyboard cells"))?;
        let rows = cells.div_ceil(columns).max(1);
        let width = cell_width
            .checked_mul(columns)
            .ok_or_else(|| anyhow!("storyboard width overflow"))?;
        let height = cell_height
            .checked_mul(rows)
            .ok_or_else(|| anyhow!("storyboard height overflow"))?;
        Ok(Self {
            path: path.into(),
            columns,
            cell_width,
            cell_height,
            canvas: RgbaFrame::opaque_black(width, height),
        })
    }
}

impl FrameSink for StoryboardSink {
    fn push(&mut self, frame: DeliveredFrame<'_>) -> Result<()> {
        let fitted = contain_rgba(frame.pixels, self.cell_width, self.cell_height)?;
        let sequence = u32::try_from(frame.sequence)
            .map_err(|_| anyhow!("storyboard sequence does not fit u32"))?;
        let x = (sequence % self.columns) * self.cell_width;
        let y = (sequence / self.columns) * self.cell_height;
        source_over(&mut self.canvas, &fitted, x, y);
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        write_png(&self.path, &self.canvas)
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

fn source_over(destination: &mut RgbaFrame, source: &RgbaFrame, x: u32, y: u32) {
    for source_y in 0..source.height {
        let destination_y = y + source_y;
        if destination_y >= destination.height {
            break;
        }
        for source_x in 0..source.width {
            let destination_x = x + source_x;
            if destination_x >= destination.width {
                break;
            }
            let source_index = ((source_y * source.width + source_x) * 4) as usize;
            let destination_index =
                ((destination_y * destination.width + destination_x) * 4) as usize;
            let alpha = u32::from(source.data[source_index + 3]);
            let inverse = 255 - alpha;
            for channel in 0..3 {
                destination.data[destination_index + channel] =
                    ((u32::from(source.data[source_index + channel]) * alpha
                        + u32::from(destination.data[destination_index + channel]) * inverse
                        + 127)
                        / 255) as u8;
            }
            destination.data[destination_index + 3] = 255;
        }
    }
}
