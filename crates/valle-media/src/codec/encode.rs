//! In-process H.264 MP4 encoding through libav with YUV420P output. Set hardware bitrate through
//! the codec setter; software encoding uses x264 CRF when no bitrate is specified.

use std::path::Path;

use crate::frame::RgbaFrame;
use anyhow::{Context, Result, anyhow};
use ff::{Dictionary, Rational, codec, encoder, format, frame, software};
use ffmpeg_next as ff;

use crate::Encoder;
use crate::ffi::{
    YuvMatrix, drain_video_encoder, ffmpeg_init, fill_plane0, set_sws_colorspace,
    tag_bt709_limited, write_mp4_header,
};

/// Shared x264 quality and speed defaults used by encoder and muxer paths.
pub(crate) const DEFAULT_X264_CRF: &str = "20";
pub(crate) const DEFAULT_X264_PRESET: &str = "veryfast";

/// Encode RGBA frames through YUV420P and H.264 into MP4.
pub struct Mp4Encoder {
    octx: format::context::Output,
    venc: encoder::Video,
    sws: software::scaling::Context,
    sidx: usize,
    enc_tb: Rational,
    ost_tb: Rational,
    width: u32,
    height: u32,
    frame_idx: i64,
    finished: bool,
}

impl Mp4Encoder {
    /// Create an MP4 encoder with even dimensions for YUV420P; absent bitrate selects CRF 20.
    pub fn new(
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: Option<usize>,
    ) -> Result<Self> {
        ffmpeg_init();
        if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(anyhow!(
                "encoder dims must be positive even numbers (got {width}x{height})"
            ));
        }
        let fps = fps.max(1);
        let enc_tb = Rational(1, fps as i32);

        let mut octx = format::output(&path).with_context(|| format!("open {}", path.display()))?;
        let global_header = octx.format().flags().contains(format::Flags::GLOBAL_HEADER);

        let codec =
            encoder::find(codec::Id::H264).ok_or_else(|| anyhow!("no H.264 encoder available"))?;
        let mut eb = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()?;
        eb.set_width(width);
        eb.set_height(height);
        eb.set_format(format::Pixel::YUV420P);
        eb.set_time_base(enc_tb);
        eb.set_frame_rate(Some(Rational(fps as i32, 1)));
        eb.set_aspect_ratio(Rational(1, 1));
        tag_bt709_limited(&mut eb); // Tag output as BT.709 with limited range.
        if global_header {
            eb.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        // Set bitrate through the codec API rather than the options dictionary.
        let mut opts = Dictionary::new();
        if let Some(b) = bitrate {
            eb.set_bit_rate(b);
        } else {
            opts.set("preset", DEFAULT_X264_PRESET);
            opts.set("crf", DEFAULT_X264_CRF);
        }
        let venc = eb.open_with(opts)?;

        let sidx = {
            let mut ost = octx.add_stream(codec)?;
            let sidx = ost.index();
            ost.set_parameters(&venc);
            ost.set_time_base(enc_tb);
            sidx
        };
        write_mp4_header(&mut octx)?;
        let ost_tb = octx.stream(sidx).unwrap().time_base();

        let mut sws = software::scaling::Context::get(
            format::Pixel::RGBA,
            width,
            height,
            format::Pixel::YUV420P,
            width,
            height,
            software::scaling::Flags::BILINEAR,
        )?;
        // Use the same BT.709 limited-range matrix for conversion and output tags.
        set_sws_colorspace(&mut sws, YuvMatrix::Bt709, false, false);

        Ok(Mp4Encoder {
            octx,
            venc,
            sws,
            sidx,
            enc_tb,
            ost_tb,
            width,
            height,
            frame_idx: 0,
            finished: false,
        })
    }
}

impl Encoder for Mp4Encoder {
    fn encode_frame(&mut self, frame: &RgbaFrame) -> Result<()> {
        if frame.width != self.width || frame.height != self.height {
            return Err(anyhow!(
                "frame {}x{} != encoder {}x{}",
                frame.width,
                frame.height,
                self.width,
                self.height
            ));
        }
        let row = self.width as usize * 4;
        // Allocate a fresh frame so threaded encoding cannot observe later buffer mutations.
        let mut rgba_f = frame::Video::new(format::Pixel::RGBA, self.width, self.height);
        fill_plane0(&mut rgba_f, &frame.data, row, self.height);
        let mut yuv = frame::Video::new(format::Pixel::YUV420P, self.width, self.height);
        self.sws.run(&rgba_f, &mut yuv)?;
        yuv.set_pts(Some(self.frame_idx));
        self.venc.send_frame(&yuv)?;
        drain_video_encoder(
            &mut self.venc,
            &mut self.octx,
            self.sidx,
            self.enc_tb,
            self.ost_tb,
        )?;
        self.frame_idx += 1;
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.venc.send_eof()?;
        drain_video_encoder(
            &mut self.venc,
            &mut self.octx,
            self.sidx,
            self.enc_tb,
            self.ost_tb,
        )?;
        self.octx.write_trailer()?;
        self.finished = true;
        Ok(())
    }
}
