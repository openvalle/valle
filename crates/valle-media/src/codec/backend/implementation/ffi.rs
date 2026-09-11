//! Shared libav FFI helpers. Codec and probe operations run in process without invoking ffmpeg or
//! ffprobe commands.

use std::sync::OnceLock;

use super::ff;
use anyhow::{Context as _, Result, bail};
use ff::{Packet, Rational, color, encoder, format, frame, software};

static FF_INIT: OnceLock<std::result::Result<(), String>> = OnceLock::new();

pub fn ffmpeg_init() -> Result<()> {
    ff::ffi::runtime::load()?;
    FF_INIT
        .get_or_init(|| ff::init().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| anyhow::anyhow!("FFmpeg initialization failed: {e}"))?;
    unsafe {
        ff::ffi::av_log_set_level(crate::codec::ffi::log_level());
    }
    Ok(())
}

/// Describe registered codecs separately from hardware availability, which requires a real open.
pub fn ffmpeg_capabilities() -> Result<serde_json::Value> {
    ffmpeg_init()?;
    let runtime = ff::ffi::runtime::load()?;
    let mut encoders = Vec::new();
    let mut decoders = Vec::new();
    let mut opaque = std::ptr::null_mut();
    unsafe {
        loop {
            let codec = ff::ffi::av_codec_iterate(&mut opaque);
            if codec.is_null() {
                break;
            }
            let name = std::ffi::CStr::from_ptr((*codec).name)
                .to_string_lossy()
                .into_owned();
            if ff::ffi::av_codec_is_encoder(codec) != 0 {
                encoders.push(name.clone());
            }
            if ff::ffi::av_codec_is_decoder(codec) != 0 {
                decoders.push(name);
            }
        }
    }
    encoders.sort();
    decoders.sort();
    Ok(serde_json::json!({
        "status": "ok", "operation": "media.capabilities",
        "ffmpegMajor": runtime.version.major(),
        "architecture": std::env::consts::ARCH,
        "libraries": runtime.libraries.iter().map(|library| serde_json::json!({
            "name": library.name, "path": library.path,
            "version": format!("{}.{}.{}", library.version >> 16, (library.version >> 8) & 255, library.version & 255),
        })).collect::<Vec<_>>(),
        "encoders": encoders, "decoders": decoders,
        "hardware": {"h264VideoToolboxAvailable": super::muxer::hw_h264_available()},
    }))
}

/// Resolve display sample-aspect-ratio with FFmpeg's container-over-codec precedence.
/// `decoder.aspect_ratio()` alone misses formats that keep the demuxer override on `AVStream`.
pub fn guessed_sample_aspect_ratio(
    input: &mut format::context::Input,
    stream_index: usize,
) -> Result<Rational> {
    let stream_ptr = {
        let stream = input
            .stream(stream_index)
            .context("sample-aspect-ratio stream index is unavailable")?;
        // SAFETY: the pointer belongs to `input`; the wrapper does not own the AVStream, and
        // `input` is exclusively borrowed for the following FFmpeg query.
        unsafe { stream.as_ptr() }
    };
    let ratio = unsafe {
        ff::ffi::av_guess_sample_aspect_ratio(
            input.as_mut_ptr(),
            stream_ptr.cast_mut(),
            std::ptr::null_mut(),
        )
    };
    Ok(Rational::from(ratio))
}

/// Write MP4/M4A headers with a 90 kHz movie clock for exact integer and fractional frame
/// boundaries. Use signed composition offsets for B frames and disable edit lists to preserve
/// boundary packets.
pub fn write_mp4_header(octx: &mut format::context::Output) -> Result<()> {
    let mut options = ff::Dictionary::new();
    options.set("movie_timescale", "90000");
    options.set("movflags", "+negative_cts_offsets");
    options.set("use_editlist", "0");
    let unused = octx.write_header_with(options)?;
    if let Some((key, _)) = unused.iter().next() {
        bail!("MP4 muxer rejected header option `{key}`");
    }
    Ok(())
}

// Color matrices, ranges, and metadata.
//
// Decode using source color tags with dimension-based defaults. Encode as BT.709 limited range with
// matching conversion and metadata.

pub use crate::codec::ffi::YuvMatrix;
/// libswscale coefficient table IDs.
const SWS_CS_ITU709: i32 = 1;
const SWS_CS_ITU601: i32 = 5;
/// Use BT.709 for untagged sources at least 720 pixels high, otherwise BT.601.
const HD_MIN_HEIGHT_PX: u32 = 720;
/// Unity contrast and saturation in 16.16 fixed-point form.
const SWS_UNITY_FIXED_16_16: i32 = 1 << 16;

fn sws_cs(matrix: YuvMatrix) -> i32 {
    match matrix {
        YuvMatrix::Bt709 => SWS_CS_ITU709,
        YuvMatrix::Bt601 => SWS_CS_ITU601,
    }
}

/// Select a matrix from source tags, falling back to height-based SD/HD defaults.
pub fn matrix_for(space: color::Space, height: u32) -> YuvMatrix {
    match space {
        color::Space::BT709 => YuvMatrix::Bt709,
        color::Space::BT470BG | color::Space::SMPTE170M | color::Space::SMPTE240M => {
            YuvMatrix::Bt601
        }
        _ => {
            if height >= HD_MIN_HEIGHT_PX {
                YuvMatrix::Bt709
            } else {
                YuvMatrix::Bt601
            }
        }
    }
}

/// Set explicit conversion matrix and range, keeping RGB full-range. Retain swscale defaults if the
/// format rejects these settings.
pub fn set_sws_colorspace(
    sws: &mut software::scaling::Context,
    m: YuvMatrix,
    yuv_full_range: bool,
    yuv_is_input: bool,
) {
    /// RGB always uses full range.
    const RGB_RANGE_FULL: i32 = 1;
    let yuv_range = yuv_full_range as i32;
    let (src_range, dst_range) = if yuv_is_input {
        (yuv_range, RGB_RANGE_FULL)
    } else {
        (RGB_RANGE_FULL, yuv_range)
    };
    unsafe {
        let table = ff::ffi::sws_getCoefficients(sws_cs(m));
        let _ = ff::ffi::sws_setColorspaceDetails(
            sws.as_mut_ptr(),
            table,
            src_range,
            table,
            dst_range,
            0, // Zero brightness offset.
            SWS_UNITY_FIXED_16_16,
            SWS_UNITY_FIXED_16_16,
        );
    }
}

/// Set primaries, transfer, matrix, and range tags before opening the video encoder.
pub fn tag_bt709_limited(enc: &mut encoder::video::Video) {
    unsafe {
        let p = enc.as_mut_ptr();
        (*p).color_primaries = ff::ffi::AVColorPrimaries::AVCOL_PRI_BT709;
        (*p).color_trc = ff::ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
        (*p).colorspace = ff::ffi::AVColorSpace::AVCOL_SPC_BT709;
        (*p).color_range = ff::ffi::AVColorRange::AVCOL_RANGE_MPEG;
    }
}

/// Copy tightly packed pixels row by row into a potentially padded frame plane.
pub fn fill_plane0(dst: &mut frame::Video, src: &[u8], row: usize, h: u32) {
    let stride = dst.stride(0);
    let data = dst.data_mut(0);
    if stride == row {
        data[..row * h as usize].copy_from_slice(&src[..row * h as usize]);
    } else {
        for y in 0..h as usize {
            data[y * stride..y * stride + row].copy_from_slice(&src[y * row..y * row + row]);
        }
    }
}

/// Read tightly packed pixels from a potentially padded frame plane.
pub fn read_plane0(src: &frame::Video, row: usize, h: u32) -> Vec<u8> {
    let stride = src.stride(0);
    let data = src.data(0);
    let mut out = vec![0u8; row * h as usize];
    if stride == row {
        out.copy_from_slice(&data[..row * h as usize]);
    } else {
        for y in 0..h as usize {
            out[y * row..y * row + row].copy_from_slice(&data[y * stride..y * stride + row]);
        }
    }
    out
}

/// Drain video packets, rescale timestamps, and write them after the caller sends EOF.
pub fn drain_video_encoder(
    enc: &mut encoder::Video,
    octx: &mut format::context::Output,
    stream_idx: usize,
    enc_tb: Rational,
    ost_tb: Rational,
) -> Result<()> {
    let mut pkt = Packet::empty();
    while enc.receive_packet(&mut pkt).is_ok() {
        pkt.set_stream(stream_idx);
        pkt.rescale_ts(enc_tb, ost_tb);
        pkt.write_interleaved(octx)?;
    }
    Ok(())
}

/// Drain audio packets, rescale timestamps, and write interleaved packets in DTS order.
pub fn drain_audio_encoder(
    enc: &mut encoder::Audio,
    octx: &mut format::context::Output,
    stream_idx: usize,
    enc_tb: Rational,
    ost_tb: Rational,
) -> Result<()> {
    let mut pkt = Packet::empty();
    while enc.receive_packet(&mut pkt).is_ok() {
        pkt.set_stream(stream_idx);
        pkt.rescale_ts(enc_tb, ost_tb);
        pkt.write_interleaved(octx)?;
    }
    Ok(())
}
