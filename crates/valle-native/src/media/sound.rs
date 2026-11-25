//! A file's sound through one FFmpeg version's API: its best audio stream decoded and resampled
//! to 32-bit float samples, interleaved, at a rate and channel count the caller picks, whole or
//! a block at a time.

use std::path::Path;

use super::ff;
use ff::{ChannelLayout, Packet, codec, format, frame, media, software};

use crate::media::RATE;

/// The sound of the file at `path`, interleaved, `channels` of them at `rate`; none when it has
/// no audio.
pub fn decode(path: &Path, rate: u32, channels: u16) -> Result<Option<Vec<f32>>, String> {
    let mut out = Vec::new();
    let found = blocks(path, rate, &|_| channels, &mut |block, _| {
        out.extend_from_slice(block);
        Ok(())
    })?;
    Ok(found.map(|_| out))
}

/// The sound of the file at `path` a block at a time, interleaved at `rate`, as many channels as
/// `channels` makes of the file's own count: each block with that count. Gives the count, or none
/// when it has no audio.
pub fn blocks(
    path: &Path,
    rate: u32,
    channels: &dyn Fn(u16) -> u16,
    each: &mut crate::media::Blocks,
) -> Result<Option<u16>, String> {
    let bad = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let mut input = format::input(path).map_err(|e| bad(&e))?;
    let Some(stream) = input.streams().best(media::Type::Audio) else {
        return Ok(None);
    };
    let index = stream.index();
    let context =
        codec::context::Context::from_parameters(stream.parameters()).map_err(|e| bad(&e))?;
    let mut decoder = context.decoder().audio().map_err(|e| bad(&e))?;
    let source = decoder.channels().max(1);
    let count = channels(source);
    let output = ChannelLayout::default(i32::from(count));
    let packed = format::Sample::F32(format::sample::Type::Packed);
    let mut resampler: Option<software::resampling::Context> = None;
    let mut take = |decoded: &mut frame::Audio| -> Result<(), String> {
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
                    output,
                    rate,
                )
                .map_err(|e| bad(&e))?,
            ),
        };
        // Room for all it makes of the frame, with what it held back before.
        let room = decoded.samples() as u64 * u64::from(rate) / u64::from(decoded.rate().max(1));
        let mut converted = frame::Audio::new(packed, room as usize + 1024, output);
        r.run(decoded, &mut converted).map_err(|e| bad(&e))?;
        each(converted.plane::<f32>(0), count)
    };
    let mut sent = false;
    loop {
        let mut decoded = frame::Audio::empty();
        if decoder.receive_frame(&mut decoded).is_ok() {
            take(&mut decoded)?;
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
            let mut rest = frame::Audio::new(packed, 4096, output);
            r.flush(&mut rest).map_err(|e| bad(&e))?;
            if rest.samples() == 0 {
                break;
            }
            each(rest.plane::<f32>(0), count)?;
        }
    }
    Ok(Some(count))
}

/// Streaming stereo f32 rate conversion for video and WAV exports.
pub struct OutputRate {
    converter: software::resampling::Context,
    input: u64,
    output: u64,
}

impl OutputRate {
    pub fn new(rate: u32) -> Result<Self, String> {
        let format = format::Sample::F32(format::sample::Type::Packed);
        let stereo = ChannelLayout::default(2);
        let converter =
            software::resampling::Context::get(format, stereo, RATE, format, stereo, rate)
                .map_err(|e| format!("cannot resample audio to {rate} Hz: {e}"))?;
        Ok(Self {
            converter,
            input: 0,
            output: 0,
        })
    }

    fn block(&mut self, samples: &[f32], capacity: usize) -> Result<Vec<f32>, String> {
        let format = self.converter.input().format;
        let layout = self.converter.input().channel_layout;
        let mut input = frame::Audio::new(format, samples.len() / 2, layout);
        input.set_rate(RATE);
        input.plane_mut::<f32>(0).copy_from_slice(samples);
        let mut out = frame::Audio::new(format, capacity, layout);
        self.converter
            .run(&input, &mut out)
            .map_err(|e| e.to_string())?;
        self.output += out.samples() as u64;
        Ok(out.plane::<f32>(0).to_vec())
    }
}

impl crate::media::audio::Resample for OutputRate {
    fn convert(&mut self, samples: Option<&[f32]>) -> Result<Vec<f32>, String> {
        let rate = u64::from(self.converter.output().rate);
        if let Some(samples) = samples {
            if samples.is_empty() {
                return Ok(Vec::new());
            }
            let n = (samples.len() / 2) as u64;
            self.input += n;
            let room = (n * rate).div_ceil(u64::from(RATE));
            return self.block(samples, room as usize + 32);
        }
        // Extend the filter with silence up to the last real output sample. swr's ordinary
        // EOF flush can return zero samples for inputs shorter than its initial filter delay.
        let length = ((u128::from(self.input) * u128::from(rate) + u128::from(RATE / 2))
            / u128::from(RATE)) as u64;
        let mut tail = Vec::new();
        while self.output < length {
            let remaining = length - self.output;
            tail.extend(self.block(&[0.0; 512], remaining.min(4096) as usize)?);
        }
        Ok(tail)
    }
}
