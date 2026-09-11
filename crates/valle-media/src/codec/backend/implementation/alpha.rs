//! Lossless transparent video encoding from straight-alpha RGBA8 to qtrle MOV. The native decoder
//! preserves RGB and alpha exactly. Hosts must choose a separate web-compatible delivery format
//! rather than discarding alpha here.

use std::path::Path;

use super::ff;
use anyhow::{Context, Result, anyhow, bail};
use ff::{Packet, Rational, codec, encoder, format, frame};

use super::ffi::{ffmpeg_init, fill_plane0, write_mp4_header};
use crate::frame::RgbaFrame;

const QTRLE_ENCODER: &str = "qtrle";

/// MOV muxer with one lossless qtrle video stream. Preserve straight RGBA at the API boundary and
/// never infer an opaque codec from the output extension.
pub struct TransparentVideoMuxer {
    octx: format::context::Output,
    venc: encoder::Video,
    stream_idx: usize,
    enc_tb: Rational,
    ost_tb: Rational,
    width: u32,
    height: u32,
    next_cfr_pts: i64,
    last_pts: Option<i64>,
    pending_packet: Option<Packet>,
    last_packet_duration: Option<i64>,
    nominal_frame_ticks: Option<i64>,
    finished: bool,
}

impl TransparentVideoMuxer {
    /// Create a lossless RGBA MOV with an exact rational constant frame rate.
    pub fn open(path: &Path, width: u32, height: u32, fps_num: u32, fps_den: u32) -> Result<Self> {
        if fps_num == 0 || fps_den == 0 || fps_num > i32::MAX as u32 || fps_den > i32::MAX as u32 {
            return Err(anyhow!(
                "transparent video fps must be a positive i32 rational (got {fps_num}/{fps_den})"
            ));
        }
        Self::open_with_clock(
            path,
            width,
            height,
            Rational(fps_den as i32, fps_num as i32),
            Some(Rational(fps_num as i32, fps_den as i32)),
            false,
        )
    }

    /// Open a VFR-capable transparent MOV using the caller's exact source clock.
    ///
    /// This is used by sequential media tools that must preserve or deliberately rebase source
    /// PTS. Every frame must then be submitted through [`Self::encode_video_at`].
    #[cfg(any(test, feature = "tool-segment"))]
    pub(crate) fn open_timed(
        path: &Path,
        width: u32,
        height: u32,
        time_base: Rational,
        nominal_frame_rate: Option<Rational>,
    ) -> Result<Self> {
        Self::open_with_clock(path, width, height, time_base, nominal_frame_rate, true)
    }

    fn open_with_clock(
        path: &Path,
        width: u32,
        height: u32,
        enc_tb: Rational,
        nominal_frame_rate: Option<Rational>,
        preserve_start_timestamp: bool,
    ) -> Result<Self> {
        ffmpeg_init()?;
        if width == 0 || height == 0 {
            return Err(anyhow!(
                "transparent video dimensions must be positive (got {width}x{height})"
            ));
        }
        if enc_tb.numerator() <= 0 || enc_tb.denominator() <= 0 {
            return Err(anyhow!(
                "transparent video time base must be positive (got {}/{})",
                enc_tb.numerator(),
                enc_tb.denominator()
            ));
        }

        // Force the MOV muxer independently of the output filename extension.
        let mut octx = format::output_as(path, "mov")
            .with_context(|| format!("open transparent MOV {}", path.display()))?;
        let global_header = octx.format().flags().contains(format::Flags::GLOBAL_HEADER);
        let codec = encoder::find_by_name(QTRLE_ENCODER)
            .ok_or_else(|| anyhow!("the installed FFmpeg has no transparent encoder '{QTRLE_ENCODER}'; inspect it with `valle media capabilities`"))?;
        let nominal_frame_ticks = nominal_frame_rate
            .map(|rate| frame_ticks(enc_tb, rate))
            .transpose()?;
        let mut video = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()?;
        video.set_width(width);
        video.set_height(height);
        // qtrle uses packed ARGB; encoding reorders channels without loss.
        video.set_format(format::Pixel::ARGB);
        video.set_time_base(enc_tb);
        video.set_frame_rate(nominal_frame_rate);
        video.set_aspect_ratio(Rational(1, 1));
        if global_header {
            video.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let venc = video.open_as(codec)?;

        let stream_idx = {
            let mut stream = octx.add_stream(codec)?;
            let stream_idx = stream.index();
            stream.set_parameters(&venc);
            stream.set_time_base(enc_tb);
            stream_idx
        };

        // Use a 90 kHz movie clock to preserve low and fractional frame-rate boundaries.
        if preserve_start_timestamp {
            write_timestamped_mov_header(&mut octx)?;
        } else {
            write_mp4_header(&mut octx)?;
        }
        let ost_tb = octx
            .stream(stream_idx)
            .ok_or_else(|| anyhow!("transparent MOV video stream missing after header"))?
            .time_base();

        Ok(Self {
            octx,
            venc,
            stream_idx,
            enc_tb,
            ost_tb,
            width,
            height,
            next_cfr_pts: 0,
            last_pts: None,
            pending_packet: None,
            last_packet_duration: None,
            nominal_frame_ticks,
            finished: false,
        })
    }

    /// Encode one straight-alpha RGBA8 frame without RGB or alpha loss.
    pub fn encode_video(&mut self, rgba: &RgbaFrame) -> Result<()> {
        let pts = self.next_cfr_pts;
        self.encode_video_at(rgba, pts)?;
        self.next_cfr_pts = pts
            .checked_add(1)
            .ok_or_else(|| anyhow!("transparent CFR timestamp overflowed"))?;
        Ok(())
    }

    /// Encode one straight-alpha RGBA8 frame at an explicit PTS in the configured time base.
    pub(crate) fn encode_video_at(&mut self, rgba: &RgbaFrame, pts: i64) -> Result<()> {
        if rgba.width != self.width || rgba.height != self.height {
            return Err(anyhow!(
                "frame {}x{} != transparent muxer {}x{}",
                rgba.width,
                rgba.height,
                self.width,
                self.height
            ));
        }
        if let Some(last) = self.last_pts
            && pts <= last
        {
            return Err(anyhow!(
                "transparent video PTS is not strictly increasing: {pts} <= {last}"
            ));
        }
        let expected = self.width as usize * self.height as usize * 4;
        if rgba.data.len() != expected {
            return Err(anyhow!(
                "RGBA buffer has {} bytes, expected {expected} for {}x{}",
                rgba.data.len(),
                self.width,
                self.height
            ));
        }

        let mut argb = vec![0u8; expected];
        for (src, dst) in rgba.data.chunks_exact(4).zip(argb.chunks_exact_mut(4)) {
            dst[0] = src[3];
            dst[1] = src[0];
            dst[2] = src[1];
            dst[3] = src[2];
        }
        let mut encoded = frame::Video::new(format::Pixel::ARGB, self.width, self.height);
        fill_plane0(&mut encoded, &argb, self.width as usize * 4, self.height);
        encoded.set_pts(Some(pts));
        self.venc.send_frame(&encoded)?;
        self.collect_packets()?;
        self.last_pts = Some(pts);
        Ok(())
    }

    /// Flush the encoder and write the trailer; safe to call repeatedly.
    pub fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.venc.send_eof()?;
        self.collect_packets()?;
        if let Some(mut packet) = self.pending_packet.take() {
            let duration = self
                .last_packet_duration
                .or(self.nominal_frame_ticks)
                .unwrap_or(1)
                .max(1);
            packet.set_duration(duration);
            self.write_packet(packet)?;
        }
        self.octx.write_trailer()?;
        self.finished = true;
        Ok(())
    }

    fn collect_packets(&mut self) -> Result<()> {
        loop {
            let mut packet = Packet::empty();
            match self.venc.receive_packet(&mut packet) {
                Ok(()) => self.queue_packet(packet)?,
                Err(ff::Error::Eof)
                | Err(ff::Error::Other {
                    errno: ff::util::error::EAGAIN,
                }) => break,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn queue_packet(&mut self, packet: Packet) -> Result<()> {
        if let Some(mut previous) = self.pending_packet.take() {
            let previous_pts = previous.pts().context("qtrle packet has no PTS")?;
            let next_pts = packet.pts().context("qtrle packet has no PTS")?;
            let duration = next_pts
                .checked_sub(previous_pts)
                .filter(|duration| *duration > 0)
                .context("qtrle packet PTS is not strictly increasing")?;
            previous.set_duration(duration);
            self.write_packet(previous)?;
            self.last_packet_duration = Some(duration);
        }
        self.pending_packet = Some(packet);
        Ok(())
    }

    fn write_packet(&mut self, mut packet: Packet) -> Result<()> {
        packet.set_stream(self.stream_idx);
        packet.rescale_ts(self.enc_tb, self.ost_tb);
        packet.write_interleaved(&mut self.octx)?;
        Ok(())
    }
}

fn frame_ticks(time_base: Rational, frame_rate: Rational) -> Result<i64> {
    if frame_rate.numerator() <= 0 || frame_rate.denominator() <= 0 {
        return Err(anyhow!(
            "transparent video frame rate must be positive (got {}/{})",
            frame_rate.numerator(),
            frame_rate.denominator()
        ));
    }
    let numerator = i128::from(time_base.denominator())
        .checked_mul(i128::from(frame_rate.denominator()))
        .context("transparent frame-duration numerator overflowed")?;
    let denominator = i128::from(time_base.numerator())
        .checked_mul(i128::from(frame_rate.numerator()))
        .context("transparent frame-duration denominator overflowed")?;
    let rounded = numerator
        .checked_add(denominator / 2)
        .context("transparent frame-duration rounding overflowed")?
        / denominator;
    i64::try_from(rounded.max(1)).context("transparent frame duration exceeds i64")
}

fn write_timestamped_mov_header(output: &mut format::context::Output) -> Result<()> {
    let mut options = ff::Dictionary::new();
    options.set("movie_timescale", "90000");
    options.set("use_editlist", "1");
    let unused = output.write_header_with(options)?;
    if let Some((key, _)) = unused.iter().next() {
        bail!("MOV muxer rejected header option `{key}`");
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::codec::decode_rgba_frames;

    pub(crate) fn cfr_writer_keeps_every_lossless_rgba_frame() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("foreground.mov");
        let first = RgbaFrame {
            width: 2,
            height: 2,
            data: vec![
                10, 20, 30, 64, 40, 50, 60, 128, 10, 20, 30, 64, 40, 50, 60, 128,
            ],
        };
        let second = RgbaFrame {
            width: 2,
            height: 2,
            data: vec![
                70, 80, 90, 192, 100, 110, 120, 255, 70, 80, 90, 192, 100, 110, 120, 255,
            ],
        };
        let mut writer = TransparentVideoMuxer::open(&path, 2, 2, 30, 1).unwrap();
        writer.encode_video(&first).unwrap();
        writer.encode_video(&second).unwrap();
        writer.finish().unwrap();
        drop(writer);

        assert_eq!(decode_rgba_frames(&path, None).unwrap(), [first, second]);
    }

    pub(crate) fn timestamped_writer_keeps_a_single_nonzero_pts_frame() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("foreground.mov");
        let frame = RgbaFrame {
            width: 2,
            height: 2,
            data: vec![
                10, 20, 30, 64, 40, 50, 60, 128, 10, 20, 30, 64, 40, 50, 60, 128,
            ],
        };
        let mut writer = TransparentVideoMuxer::open_timed(
            &path,
            2,
            2,
            Rational(1, 1_000),
            Some(Rational(30, 1)),
        )
        .unwrap();
        writer.encode_video_at(&frame, 900).unwrap();
        writer.finish().unwrap();
        drop(writer);

        assert_eq!(decode_rgba_frames(&path, None).unwrap(), [frame]);
    }
}
