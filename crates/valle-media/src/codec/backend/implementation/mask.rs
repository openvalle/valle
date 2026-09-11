//! Lossless Gray8 mask encoding in the selected FFmpeg ABI.
use super::ff::{Rational, codec, encoder, format, frame};
use super::ffi::{drain_video_encoder, ffmpeg_init, fill_plane0};
use crate::frame::Gray8Frame;
use anyhow::{Context as _, Result as AnyResult, anyhow, ensure};
use std::path::Path;
const MASK_VIDEO_ENCODER: &str = "ffv1";
const MASK_VIDEO_MUXER: &str = "matroska";
pub(crate) struct BinaryMaskVideoMuxer {
    output: format::context::Output,
    encoder: encoder::Video,
    stream_index: usize,
    encoder_time_base: Rational,
    output_time_base: Rational,
    width: u32,
    height: u32,
    last_pts: Option<i64>,
    finished: bool,
}

impl BinaryMaskVideoMuxer {
    pub(crate) fn open(
        path: &Path,
        width: u32,
        height: u32,
        time_base: Rational,
        nominal_fps: f64,
    ) -> AnyResult<Self> {
        ffmpeg_init()?;
        ensure!(
            width > 0 && height > 0,
            "mask video dimensions must be positive"
        );
        ensure!(
            time_base.numerator() > 0 && time_base.denominator() > 0,
            "mask video time base must be positive"
        );
        let mut output = format::output_as(path, MASK_VIDEO_MUXER)
            .with_context(|| format!("open mask video {}", path.display()))?;
        let global_header = output
            .format()
            .flags()
            .contains(format::Flags::GLOBAL_HEADER);
        let codec = encoder::find_by_name(MASK_VIDEO_ENCODER)
            .ok_or_else(|| anyhow!("lossless encoder {MASK_VIDEO_ENCODER:?} is unavailable"))?;
        let mut video = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()?;
        video.set_width(width);
        video.set_height(height);
        video.set_format(format::Pixel::GRAY8);
        video.set_time_base(time_base);
        video.set_aspect_ratio(Rational(1, 1));
        if let Some(rate) = fps_rational(nominal_fps) {
            video.set_frame_rate(Some(rate));
        }
        if global_header {
            video.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let encoder = video.open_as(codec)?;
        let stream_index = {
            let mut stream = output.add_stream(codec)?;
            let index = stream.index();
            stream.set_parameters(&encoder);
            stream.set_time_base(time_base);
            index
        };
        output.write_header()?;
        let output_time_base = output
            .stream(stream_index)
            .context("mask video stream missing after header")?
            .time_base();
        Ok(Self {
            output,
            encoder,
            stream_index,
            encoder_time_base: time_base,
            output_time_base,
            width,
            height,
            last_pts: None,
            finished: false,
        })
    }

    pub(crate) fn encode(&mut self, mask: &Gray8Frame, pts: i64) -> AnyResult<()> {
        ensure!(
            mask.width == self.width && mask.height == self.height,
            "mask frame {}x{} does not match muxer {}x{}",
            mask.width,
            mask.height,
            self.width,
            self.height
        );
        ensure!(
            mask.is_binary_mask(),
            "mask video input is not binary Gray8"
        );
        if let Some(last) = self.last_pts {
            ensure!(
                pts > last,
                "output PTS is not strictly increasing: {pts} <= {last}"
            );
        }
        let mut encoded = frame::Video::new(format::Pixel::GRAY8, self.width, self.height);
        fill_plane0(&mut encoded, &mask.data, self.width as usize, self.height);
        encoded.set_pts(Some(pts));
        self.encoder.send_frame(&encoded)?;
        drain_video_encoder(
            &mut self.encoder,
            &mut self.output,
            self.stream_index,
            self.encoder_time_base,
            self.output_time_base,
        )?;
        self.last_pts = Some(pts);
        Ok(())
    }

    pub(crate) fn finish(&mut self) -> AnyResult<()> {
        if self.finished {
            return Ok(());
        }
        self.encoder.send_eof()?;
        drain_video_encoder(
            &mut self.encoder,
            &mut self.output,
            self.stream_index,
            self.encoder_time_base,
            self.output_time_base,
        )?;
        self.output.write_trailer()?;
        self.finished = true;
        Ok(())
    }
}

fn fps_rational(fps: f64) -> Option<Rational> {
    const SCALE: f64 = 1_000_000.0;
    if !fps.is_finite() || fps <= 0.0 {
        return None;
    }
    let numerator = (fps * SCALE).round();
    if numerator <= 0.0 || numerator > i32::MAX as f64 {
        return None;
    }
    let numerator = numerator as i32;
    let denominator = SCALE as i32;
    let divisor = gcd_i32(numerator, denominator);
    Some(Rational(numerator / divisor, denominator / divisor))
}
fn gcd_i32(mut left: i32, mut right: i32) -> i32 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left.abs().max(1)
}
