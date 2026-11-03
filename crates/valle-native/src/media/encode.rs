//! Frames into a video file through one FFmpeg version's API: straight sRGB RGBA converted on the
//! CPU to the encoder's YUV, tagged BT.709 primaries and matrix, limited range, and the sRGB
//! transfer, so players show the colors a PNG of the frame has. Sound, when there is some, goes
//! beside them: AAC in an .mp4, 16-bit PCM in a .mov.

use std::path::Path;

use super::ff;
use ff::{ChannelLayout, Dictionary, Packet, Rational, codec, encoder, format, frame, software};

use crate::media::{Codec, Encoding, Options, RATE, Write};

pub struct Writer {
    output: format::context::Output,
    encoder: encoder::Video,
    scaler: software::scaling::Context,
    pixel: format::Pixel,
    stream: usize,
    rate: Rational,
    size: [u32; 2],
    next: i64,
    done: bool,
    sound: Option<Sound>,
}

/// The sound's stream: its encoder, converting from interleaved f32, the samples not yet in a
/// frame, and the next frame's first sample.
struct Sound {
    encoder: encoder::Audio,
    resampler: software::resampling::Context,
    format: format::Sample,
    stream: usize,
    /// Samples a frame.
    size: usize,
    pending: Vec<f32>,
    next: i64,
}

/// The FFmpeg encoder of `codec` that `encoding` names.
fn encoder_name(codec: Codec, encoding: Encoding) -> &'static str {
    match (codec, encoding) {
        (Codec::H264, Encoding::Hardware) => "h264_videotoolbox",
        (Codec::H264, Encoding::Software) => "libx264",
        (Codec::Hevc, Encoding::Hardware) => "hevc_videotoolbox",
        (Codec::Hevc, Encoding::Software) => "libx265",
        (Codec::ProRes4444, Encoding::Hardware) => "prores_videotoolbox",
        (Codec::ProRes4444, Encoding::Software) => "prores_ks",
    }
}

/// Whether the FFmpeg loaded has the encoder.
pub fn has(codec: Codec, encoding: Encoding) -> bool {
    encoder::find_by_name(encoder_name(codec, encoding)).is_some()
}

impl Writer {
    pub fn new(path: &Path, o: &Options) -> Result<Writer, String> {
        let name = encoder_name(o.codec, o.encoding);
        let found = encoder::find_by_name(name)
            .ok_or_else(|| format!("the FFmpeg loaded has no {name} encoder"))?;
        let [width, height] = o.size;
        let pixel = match (o.codec, o.encoding) {
            (Codec::ProRes4444, Encoding::Hardware) => format::Pixel::AYUV64LE,
            (Codec::ProRes4444, Encoding::Software) => format::Pixel::YUVA444P10LE,
            (_, Encoding::Hardware) => format::Pixel::NV12,
            (_, Encoding::Software) => format::Pixel::YUV420P,
        };
        let rate = Rational(o.fps[1] as i32, o.fps[0] as i32);
        let mut output = format::output(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let global = (output.format().flags()).contains(format::Flags::GLOBAL_HEADER);
        let mut setup = (codec::context::Context::new_with_codec(found)
            .encoder()
            .video())
        .map_err(|e| e.to_string())?;
        setup.set_width(width);
        setup.set_height(height);
        setup.set_format(pixel);
        setup.set_time_base(rate);
        setup.set_frame_rate(Some(Rational(o.fps[0] as i32, o.fps[1] as i32)));
        setup.set_aspect_ratio(Rational(1, 1));
        if global {
            setup.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        // SAFETY: the context is open for writing and not yet opened as an encoder.
        unsafe {
            let p = setup.as_mut_ptr();
            (*p).color_primaries = ff::ffi::AVColorPrimaries::AVCOL_PRI_BT709;
            (*p).color_trc = ff::ffi::AVColorTransferCharacteristic::AVCOL_TRC_IEC61966_2_1;
            (*p).colorspace = ff::ffi::AVColorSpace::AVCOL_SPC_BT709;
            (*p).color_range = ff::ffi::AVColorRange::AVCOL_RANGE_MPEG;
        }
        let mut options = Dictionary::new();
        match (o.codec, o.encoding) {
            (Codec::ProRes4444, Encoding::Software) => {
                options.set("profile", "4444");
                options.set("vendor", "apl0");
            }
            (Codec::ProRes4444, Encoding::Hardware) => options.set("profile", "4444"),
            (_, Encoding::Software) => {
                options.set("preset", "medium");
                options.set("crf", if o.codec == Codec::H264 { "18" } else { "20" });
                if o.codec == Codec::Hevc {
                    options.set("x265-params", "log-level=error");
                }
            }
            (_, Encoding::Hardware) => {}
        }
        if let Some(bitrate) = o.bitrate {
            setup.set_bit_rate(bitrate as usize);
        } else if o.encoding == Encoding::Hardware && o.codec != Codec::ProRes4444 {
            // VideoToolbox has no quality target on every Mac: bits a pixel a frame instead.
            let bits = if o.codec == Codec::H264 { 0.15 } else { 0.1 };
            let fps = o.fps[0] as f64 / o.fps[1] as f64;
            setup.set_bit_rate((width as f64 * height as f64 * fps * bits) as usize);
        }
        let opened = setup
            .open_with(options)
            .map_err(|e| format!("{name}: {e}"))?;
        let sound = match o.sound {
            true => Some(Sound::new(&mut output, path, global)?),
            false => None,
        };
        let stream = {
            let mut s = output.add_stream(found).map_err(|e| e.to_string())?;
            s.set_parameters(&opened);
            s.set_time_base(rate);
            if o.codec == Codec::Hevc {
                // `hvc1`, which QuickTime and Safari play; FFmpeg writes `hev1`.
                // SAFETY: the stream's parameters belong to the output, not yet written.
                unsafe {
                    let tag = u32::from_le_bytes(*b"hvc1");
                    (*(*s.as_mut_ptr()).codecpar).codec_tag = tag;
                }
            }
            s.index()
        };
        output
            .write_header()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let flags = software::scaling::Flags::BILINEAR | software::scaling::Flags::ACCURATE_RND;
        let mut scaler = software::scaling::Context::get(
            format::Pixel::RGBA,
            width,
            height,
            pixel,
            width,
            height,
            flags,
        )
        .map_err(|e| e.to_string())?;
        // BT.709 coefficients, RGB full range to YUV limited range.
        // SAFETY: the scaler is valid; the table is FFmpeg's own.
        unsafe {
            let table = ff::ffi::sws_getCoefficients(1);
            ff::ffi::sws_setColorspaceDetails(
                scaler.as_mut_ptr(),
                table,
                1,
                table,
                0,
                0,
                1 << 16,
                1 << 16,
            );
        }
        Ok(Writer {
            output,
            encoder: opened,
            scaler,
            pixel,
            stream,
            rate,
            size: o.size,
            next: 0,
            done: false,
            sound,
        })
    }

    fn drain(&mut self) -> Result<(), String> {
        let mut packet = Packet::empty();
        let to = self.output.stream(self.stream).map(|s| s.time_base());
        let to = to.ok_or("the video stream is gone")?;
        while self.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.stream);
            packet.rescale_ts(self.rate, to);
            (packet.write_interleaved(&mut self.output)).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

impl Write for Writer {
    fn push(&mut self, rgba: &[u8]) -> Result<(), String> {
        let [w, h] = self.size;
        let mut source = frame::Video::new(format::Pixel::RGBA, w, h);
        let (stride, row) = (source.stride(0), w as usize * 4);
        let data = source.data_mut(0);
        for y in 0..h as usize {
            data[y * stride..y * stride + row].copy_from_slice(&rgba[y * row..(y + 1) * row]);
        }
        let mut converted = frame::Video::new(self.pixel, w, h);
        (self.scaler.run(&source, &mut converted)).map_err(|e| e.to_string())?;
        // SAFETY: the frame is ours; ProRes writes its tags from the frame.
        unsafe {
            let p = converted.as_mut_ptr();
            (*p).color_primaries = ff::ffi::AVColorPrimaries::AVCOL_PRI_BT709;
            (*p).color_trc = ff::ffi::AVColorTransferCharacteristic::AVCOL_TRC_IEC61966_2_1;
            (*p).colorspace = ff::ffi::AVColorSpace::AVCOL_SPC_BT709;
            (*p).color_range = ff::ffi::AVColorRange::AVCOL_RANGE_MPEG;
        }
        converted.set_pts(Some(self.next));
        self.next += 1;
        (self.encoder.send_frame(&converted)).map_err(|e| e.to_string())?;
        self.drain()
    }

    fn sound(&mut self, samples: &[f32]) -> Result<(), String> {
        let sound = (self.sound.as_mut()).ok_or("the video was opened without sound")?;
        sound.pending.extend_from_slice(samples);
        sound.encode(&mut self.output, false)
    }

    fn finish(&mut self) -> Result<(), String> {
        if std::mem::replace(&mut self.done, true) {
            return Ok(());
        }
        self.encoder.send_eof().map_err(|e| e.to_string())?;
        self.drain()?;
        if let Some(sound) = &mut self.sound {
            sound.encode(&mut self.output, true)?;
            sound.encoder.send_eof().map_err(|e| e.to_string())?;
            sound.drain(&mut self.output)?;
        }
        self.output.write_trailer().map_err(|e| e.to_string())
    }
}

impl Sound {
    /// The sound's encoder and stream in `output`, not yet written: AAC in an .mp4, 16-bit PCM in a
    /// .mov.
    fn new(
        output: &mut format::context::Output,
        path: &Path,
        global: bool,
    ) -> Result<Sound, String> {
        let mov = (path.extension()).is_some_and(|e| e.eq_ignore_ascii_case("mov"));
        let (name, format) = match mov {
            true => (
                "pcm_s16le",
                format::Sample::I16(format::sample::Type::Packed),
            ),
            false => ("aac", format::Sample::F32(format::sample::Type::Planar)),
        };
        let found = encoder::find_by_name(name)
            .ok_or_else(|| format!("the FFmpeg loaded has no {name} encoder"))?;
        let stereo = ChannelLayout::default(2);
        let mut setup = (codec::context::Context::new_with_codec(found)
            .encoder()
            .audio())
        .map_err(|e| e.to_string())?;
        setup.set_rate(RATE as i32);
        setup.set_channel_layout(stereo);
        setup.set_format(format);
        setup.set_time_base(Rational(1, RATE as i32));
        if !mov {
            setup.set_bit_rate(256_000);
        }
        if global {
            setup.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let opened = setup.open().map_err(|e| format!("{name}: {e}"))?;
        let stream = {
            let mut s = output.add_stream(found).map_err(|e| e.to_string())?;
            s.set_parameters(&opened);
            s.set_time_base(Rational(1, RATE as i32));
            s.index()
        };
        let packed = format::Sample::F32(format::sample::Type::Packed);
        let resampler =
            software::resampling::Context::get(packed, stereo, RATE, format, stereo, RATE)
                .map_err(|e| e.to_string())?;
        let size = match opened.frame_size() {
            0 => 1024,
            n => n as usize,
        };
        Ok(Sound {
            encoder: opened,
            resampler,
            format,
            stream,
            size,
            pending: Vec::new(),
            next: 0,
        })
    }

    /// The packets the encoder has made, into the file.
    fn drain(&mut self, output: &mut format::context::Output) -> Result<(), String> {
        let to = output.stream(self.stream).map(|s| s.time_base());
        let to = to.ok_or("the sound stream is gone")?;
        let mut packet = Packet::empty();
        while self.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.stream);
            packet.rescale_ts(Rational(1, RATE as i32), to);
            (packet.write_interleaved(output)).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// The pending samples into the encoder a frame at a time: the whole frames, or all at the
    /// `end`.
    fn encode(&mut self, output: &mut format::context::Output, end: bool) -> Result<(), String> {
        let stereo = ChannelLayout::default(2);
        let packed = format::Sample::F32(format::sample::Type::Packed);
        while self.pending.len() >= 2 * self.size || (end && !self.pending.is_empty()) {
            let n = (self.pending.len() / 2).min(self.size);
            let mut source = frame::Audio::new(packed, n, stereo);
            source.set_rate(RATE);
            let samples = self.pending.drain(..2 * n);
            let pairs = source.plane_mut::<(f32, f32)>(0);
            for (o, s) in pairs.iter_mut().zip(samples.as_slice().chunks(2)) {
                *o = (s[0], s[1]);
            }
            drop(samples);
            let mut converted = frame::Audio::new(self.format, n, stereo);
            (self.resampler.run(&source, &mut converted)).map_err(|e| e.to_string())?;
            converted.set_rate(RATE);
            converted.set_pts(Some(self.next));
            self.next += n as i64;
            (self.encoder.send_frame(&converted)).map_err(|e| e.to_string())?;
            self.drain(output)?;
        }
        Ok(())
    }
}
