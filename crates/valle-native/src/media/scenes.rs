//! How much each frame of a file's video differs from the one before, through one FFmpeg
//! version's API: every frame decoded on the CPU and shrunk to a thumbnail, for finding cuts.

use std::path::Path;

use super::ff;
use ff::{Packet, codec, format, frame, media, software};

use crate::media::TICKS_PER_SECOND;

/// The thumbnail frames are compared at, in px: small enough that noise and grain average out.
const SIZE: [u32; 2] = [96, 54];

/// Each frame after the first, by when it starts in ticks, with how much it differs from the
/// frame before: the mean difference of their thumbnails' channels, from 0 to 100. None when the
/// file has no video.
pub fn differences(path: &Path) -> Result<Option<Vec<(i64, f64)>>, String> {
    let bad = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let mut input = format::input(path).map_err(|e| bad(&e))?;
    let Some(stream) = input.streams().best(media::Type::Video) else {
        return Ok(None);
    };
    let index = stream.index();
    let tb = stream.time_base();
    let (num, den) = (tb.numerator() as i128, tb.denominator().max(1) as i128);
    let mut context =
        codec::context::Context::from_parameters(stream.parameters()).map_err(|e| bad(&e))?;
    // Every frame is decoded, so on as many threads as the machine has.
    context.set_threading(codec::threading::Config {
        kind: codec::threading::Type::Frame,
        count: 0,
    });
    let mut decoder = context.decoder().video().map_err(|e| bad(&e))?;
    type Key = (format::Pixel, u32, u32);
    let mut scaler: Option<(Key, software::scaling::Context)> = None;
    let mut previous: Option<Vec<u8>> = None;
    let mut out = Vec::new();
    let mut take = |f: &frame::Video| -> Result<(), String> {
        let key = (f.format(), f.width(), f.height());
        if scaler.as_ref().is_none_or(|(k, _)| *k != key) {
            let s = software::scaling::Context::get(
                key.0,
                key.1,
                key.2,
                format::Pixel::RGB24,
                SIZE[0],
                SIZE[1],
                software::scaling::Flags::AREA,
            )
            .map_err(|e| bad(&e))?;
            scaler = Some((key, s));
        }
        let (_, s) = scaler.as_mut().expect("just made");
        let mut small = frame::Video::new(format::Pixel::RGB24, SIZE[0], SIZE[1]);
        s.run(f, &mut small).map_err(|e| bad(&e))?;
        let (data, stride, row) = (small.data(0), small.stride(0), SIZE[0] as usize * 3);
        let pixels: Vec<u8> = (0..SIZE[1] as usize)
            .flat_map(|y| &data[y * stride..y * stride + row])
            .copied()
            .collect();
        if let Some(before) = &previous {
            let sum: u64 = before
                .iter()
                .zip(&pixels)
                .map(|(a, b)| u64::from(a.abs_diff(*b)))
                .sum();
            let pts = f.timestamp().or(f.pts()).unwrap_or(0) as i128;
            let tick = (pts * num * TICKS_PER_SECOND as i128).div_euclid(den) as i64;
            out.push((tick, sum as f64 * 100.0 / (pixels.len() as f64 * 255.0)));
        }
        previous = Some(pixels);
        Ok(())
    };
    let mut sent = false;
    loop {
        let mut decoded = frame::Video::empty();
        if decoder.receive_frame(&mut decoded).is_ok() {
            take(&decoded)?;
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
    Ok(Some(out))
}
