//! A file's sound through one FFmpeg version's API: its best audio stream decoded and resampled
//! to 48 kHz stereo, 32-bit float samples, interleaved.

use std::path::Path;

use super::ff;
use ff::{ChannelLayout, Packet, codec, format, frame, media, software};

use crate::media::RATE;

/// The sound of the file at `path`, interleaved stereo at `RATE`; none when it has no audio.
pub fn decode(path: &Path) -> Result<Option<Vec<f32>>, String> {
    let bad = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let mut input = format::input(path).map_err(|e| bad(&e))?;
    let Some(stream) = input.streams().best(media::Type::Audio) else {
        return Ok(None);
    };
    let index = stream.index();
    let context =
        codec::context::Context::from_parameters(stream.parameters()).map_err(|e| bad(&e))?;
    let mut decoder = context.decoder().audio().map_err(|e| bad(&e))?;
    let stereo = ChannelLayout::default(2);
    let packed = format::Sample::F32(format::sample::Type::Packed);
    let mut resampler: Option<software::resampling::Context> = None;
    let mut out = Vec::new();
    let mut take = |decoded: &mut frame::Audio, out: &mut Vec<f32>| -> Result<(), String> {
        // Its channels in the usual order, which files that name no layout have too.
        let layout = ChannelLayout::default(i32::from(decoded.channels()));
        decoded.set_channel_layout(layout);
        let r = match &mut resampler {
            Some(r) => r,
            None => resampler.insert(
                software::resampling::Context::get(
                    decoded.format(),
                    layout,
                    decoded.rate(),
                    packed,
                    stereo,
                    RATE,
                )
                .map_err(|e| bad(&e))?,
            ),
        };
        // Room for all it makes of the frame, with what it held back before.
        let room = decoded.samples() as u64 * u64::from(RATE) / u64::from(decoded.rate().max(1));
        let mut converted = frame::Audio::new(packed, room as usize + 1024, stereo);
        r.run(decoded, &mut converted).map_err(|e| bad(&e))?;
        let pairs = converted.plane::<(f32, f32)>(0);
        out.extend(pairs.iter().flat_map(|&(l, r)| [l, r]));
        Ok(())
    };
    let mut sent = false;
    loop {
        let mut decoded = frame::Audio::empty();
        if decoder.receive_frame(&mut decoded).is_ok() {
            take(&mut decoded, &mut out)?;
            continue;
        }
        if sent {
            break;
        }
        let mut packet = Packet::empty();
        match packet.read(&mut input) {
            Ok(()) if packet.stream() == index => {
                decoder.send_packet(&packet).map_err(|e| bad(&e))?;
            }
            Ok(()) => {}
            Err(_) => {
                sent = true;
                decoder.send_eof().map_err(|e| bad(&e))?;
            }
        }
    }
    // What the resampler still holds.
    if let Some(r) = &mut resampler {
        loop {
            let mut rest = frame::Audio::new(packed, 4096, stereo);
            r.flush(&mut rest).map_err(|e| bad(&e))?;
            if rest.samples() == 0 {
                break;
            }
            let pairs = rest.plane::<(f32, f32)>(0);
            out.extend(pairs.iter().flat_map(|&(l, r)| [l, r]));
        }
    }
    Ok(Some(out))
}
