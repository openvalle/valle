//! A video file's frames through one FFmpeg version's API, by time: decoded on the machine's
//! hardware (VideoToolbox) or by FFmpeg's own decoder on the CPU, and converted on the CPU to
//! straight sRGB RGBA by the matrix and range the file tags, BT.709 or BT.601 by its height when
//! it tags none.

use std::path::Path;
use std::ptr;

use super::ff;
use ff::{Packet, codec, decoder, format, frame, media, software};

use crate::media::{Decoding, Read, TICKS_PER_SECOND};

/// How far past the frame decoded last a frame is still reached by decoding on, not by seeking:
/// 2 s, in ticks.
const AHEAD: i64 = 2 * TICKS_PER_SECOND;

pub struct Reader {
    input: format::context::Input,
    decoder: decoder::Video,
    stream: usize,
    /// The stream's time base, a tick count each.
    base: (i64, i64),
    size: [u32; 2],
    /// The frame shown now and the one after it, each with when it starts, in ticks.
    current: Option<(i64, frame::Video)>,
    next: Option<(i64, frame::Video)>,
    /// Whether the file's last packet went to the decoder, and whether the decoder gave its last
    /// frame.
    sent: bool,
    drained: bool,
    /// The RGBA of `current`, once converted.
    converted: Option<(i64, Vec<u8>)>,
    scaler: Option<(format::Pixel, software::scaling::Context)>,
}

impl Reader {
    pub fn open(path: &Path, decoding: Decoding) -> Result<Reader, String> {
        let bad = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
        let input = format::input(path).map_err(|e| bad(&e))?;
        let stream = (input.streams().best(media::Type::Video)).ok_or_else(|| bad(&"no video"))?;
        let index = stream.index();
        let tb = stream.time_base();
        let mut context =
            codec::context::Context::from_parameters(stream.parameters()).map_err(|e| bad(&e))?;
        if decoding == Decoding::Hardware {
            // Frames stay in the hardware's memory until they are read back.
            let mut device = ptr::null_mut();
            // SAFETY: FFmpeg fills `device` or leaves it null; the context takes a reference.
            unsafe {
                let made = ff::ffi::av_hwdevice_ctx_create(
                    &mut device,
                    ff::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                    ptr::null(),
                    ptr::null_mut(),
                    0,
                );
                if made < 0 || device.is_null() {
                    return Err(bad(&"the hardware decoder cannot start"));
                }
                (*context.as_mut_ptr()).hw_device_ctx = ff::ffi::av_buffer_ref(device);
                ff::ffi::av_buffer_unref(&mut device);
            }
        }
        let decoder = context.decoder().video().map_err(|e| bad(&e))?;
        let size = [decoder.width(), decoder.height()];
        if size[0] == 0 || size[1] == 0 {
            return Err(bad(&"a video without a size"));
        }
        Ok(Reader {
            input,
            decoder,
            stream: index,
            base: (tb.numerator() as i64, tb.denominator() as i64),
            size,
            current: None,
            next: None,
            sent: false,
            drained: false,
            converted: None,
            scaler: None,
        })
    }

    /// When a frame of the stream starts, in ticks.
    fn tick(&self, f: &frame::Video) -> i64 {
        let pts = f.timestamp().or(f.pts()).unwrap_or(0) as i128;
        let (num, den) = (self.base.0 as i128, self.base.1.max(1) as i128);
        (pts * num * TICKS_PER_SECOND as i128).div_euclid(den) as i64
    }

    /// The next frame decoded, read back from the hardware if it is there; none past the end.
    fn decode(&mut self) -> Result<Option<frame::Video>, String> {
        loop {
            let mut decoded = frame::Video::empty();
            if self.decoder.receive_frame(&mut decoded).is_ok() {
                return Ok(Some(self.read_back(decoded)?));
            }
            if self.sent {
                return Ok(None);
            }
            let mut packet = Packet::empty();
            match packet.read(&mut self.input) {
                Ok(()) if packet.stream() == self.stream => {
                    (self.decoder.send_packet(&packet)).map_err(|e| e.to_string())?;
                }
                Ok(()) => {}
                Err(_) => {
                    self.sent = true;
                    self.decoder.send_eof().map_err(|e| e.to_string())?;
                }
            }
        }
    }

    fn read_back(&self, decoded: frame::Video) -> Result<frame::Video, String> {
        if decoded.format() != format::Pixel::VIDEOTOOLBOX {
            return Ok(decoded);
        }
        let mut read = frame::Video::empty();
        // SAFETY: both frames are valid; FFmpeg allocates the copy's planes.
        unsafe {
            if ff::ffi::av_hwframe_transfer_data(read.as_mut_ptr(), decoded.as_ptr(), 0) < 0 {
                return Err("the hardware decoder's frame cannot be read".into());
            }
            ff::ffi::av_frame_copy_props(read.as_mut_ptr(), decoded.as_ptr());
        }
        Ok(read)
    }

    /// Back to the keyframe at or before `at`, nothing decoded since.
    fn seek(&mut self, at: i64) -> Result<(), String> {
        let micros = (at as i128 * 1_000_000 / TICKS_PER_SECOND as i128) as i64;
        (self.input.seek(micros, ..micros)).map_err(|e| e.to_string())?;
        self.decoder.flush();
        self.current = None;
        self.next = None;
        self.sent = false;
        self.drained = false;
        Ok(())
    }

    fn rgba(&mut self, f: &frame::Video) -> Result<Vec<u8>, String> {
        let [w, h] = self.size;
        let format = f.format();
        if self.scaler.as_ref().is_none_or(|(p, _)| *p != format) {
            let flags = software::scaling::Flags::BILINEAR
                | software::scaling::Flags::ACCURATE_RND
                | software::scaling::Flags::FULL_CHR_H_INT;
            let mut scaler = software::scaling::Context::get(
                format,
                f.width(),
                f.height(),
                format::Pixel::RGBA,
                w,
                h,
                flags,
            )
            .map_err(|e| e.to_string())?;
            // SAFETY: the frame and the scaler are valid; the tables are FFmpeg's own.
            unsafe {
                let p = f.as_ptr();
                let bt709 = match (*p).colorspace {
                    ff::ffi::AVColorSpace::AVCOL_SPC_BT709 => true,
                    ff::ffi::AVColorSpace::AVCOL_SPC_BT470BG
                    | ff::ffi::AVColorSpace::AVCOL_SPC_SMPTE170M
                    | ff::ffi::AVColorSpace::AVCOL_SPC_SMPTE240M => false,
                    _ => h >= 720,
                };
                let full = (*p).color_range == ff::ffi::AVColorRange::AVCOL_RANGE_JPEG;
                let table = ff::ffi::sws_getCoefficients(if bt709 { 1 } else { 5 });
                ff::ffi::sws_setColorspaceDetails(
                    scaler.as_mut_ptr(),
                    table,
                    full as i32,
                    table,
                    1,
                    0,
                    1 << 16,
                    1 << 16,
                );
            }
            self.scaler = Some((format, scaler));
        }
        let (_, scaler) = self.scaler.as_mut().expect("just made");
        let mut out = frame::Video::new(format::Pixel::RGBA, w, h);
        scaler.run(f, &mut out).map_err(|e| e.to_string())?;
        let (stride, row) = (out.stride(0), w as usize * 4);
        let data = out.data(0);
        let mut pixels = Vec::with_capacity(row * h as usize);
        for y in 0..h as usize {
            pixels.extend_from_slice(&data[y * stride..y * stride + row]);
        }
        Ok(pixels)
    }
}

impl Read for Reader {
    fn size(&self) -> [u32; 2] {
        self.size
    }

    fn frame(&mut self, at: i64) -> Result<Vec<u8>, String> {
        let at = at.max(0);
        let behind = self.current.as_ref().is_some_and(|(t, _)| at < *t);
        let far = (self.current.as_ref()).map_or(at > AHEAD, |(t, _)| at > t + AHEAD);
        if behind || far {
            self.seek(at)?;
        }
        // Decode on until the frame after the one shown starts after `at`.
        loop {
            if self.next.is_none() && !self.drained {
                match self.decode()? {
                    Some(f) => self.next = Some((self.tick(&f), f)),
                    None => self.drained = true,
                }
            }
            match &self.next {
                Some((t, _)) if *t <= at || self.current.is_none() => {
                    self.current = self.next.take();
                }
                _ => break,
            }
        }
        let (start, f) = self.current.take().ok_or("the video has no frames")?;
        let pixels = match &self.converted {
            Some((t, p)) if *t == start => p.clone(),
            _ => {
                let p = self.rgba(&f)?;
                self.converted = Some((start, p.clone()));
                p
            }
        };
        self.current = Some((start, f));
        Ok(pixels)
    }
}
