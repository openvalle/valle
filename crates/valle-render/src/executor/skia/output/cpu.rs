//! Resident Linear Rec.2020 F16 -> opaque sRGB8, independent of the host OS.
//!
//! Cache exact groups of four source pixels *before* spatial dither. There is no quantized color
//! key, full-frame float intermediate, or cached final pixel tied to a previous canvas position.
//! SIMD kernels share the scalar matrix coefficients, operation order, curve and rounding rules.

use valle_engine::{
    compositor::{
        bloom::half_to_f32,
        delivery::{output_luminance_weights, output_primaries_matrix},
        reference::{OUTPUT_DITHER_SIZE, output_dither_pixels},
    },
    resource::{ColorPrimaries, Dither, GamutMap, OutputSpec},
};

use super::{transfer_code, transfer_lut};

#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
mod neon;
#[cfg(test)]
mod tests;
#[cfg(all(target_arch = "x86_64", target_endian = "little"))]
mod x86;

const CACHE_BITS: u32 = 10;
const CACHE_SIZE: usize = 1 << CACHE_BITS;
const TILE: usize = OUTPUT_DITHER_SIZE as usize;
type Codes = [[f32; 4]; 3];

#[derive(Clone, Copy, Debug)]
struct Entry {
    keys: [u64; 4],
    codes: Codes,
    valid: bool,
}

const EMPTY: Entry = Entry {
    keys: [0; 4],
    codes: [[0.0; 4]; 3],
    valid: false,
};

#[derive(Debug)]
struct Parameters {
    matrix: [[f64; 3]; 3],
    weights: [f32; 3],
    gamut: GamutMap,
    curve: &'static [f32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
    Scalar,
    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    Neon,
    #[cfg(all(target_arch = "x86_64", target_endian = "little"))]
    Avx2,
}

impl Backend {
    fn detect() -> Self {
        #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
        if std::arch::is_aarch64_feature_detected!("neon") {
            return Self::Neon;
        }
        #[cfg(all(target_arch = "x86_64", target_endian = "little"))]
        if std::arch::is_x86_feature_detected!("avx2")
            && std::arch::is_x86_feature_detected!("f16c")
        {
            return Self::Avx2;
        }
        Self::Scalar
    }
}

#[derive(Debug)]
pub(super) struct Converter {
    parameters: Parameters,
    backend: Backend,
    cache: Vec<Entry>,
    previous: Entry,
    // One group per four horizontal pixels, already transposed for every kernel.
    noise: Vec<Codes>,
}

impl Converter {
    pub(super) fn retained_bytes(dither: Dither) -> usize {
        CACHE_SIZE * size_of::<Entry>()
            + if dither == Dither::None {
                0
            } else {
                TILE * TILE / 4 * size_of::<Codes>()
            }
    }

    pub(super) fn construction_bytes(dither: Dither) -> usize {
        if dither == Dither::None {
            0
        } else {
            TILE * TILE * 4
        }
    }

    pub(super) fn new(spec: OutputSpec) -> Self {
        let noise = match spec.dither() {
            Dither::None => Vec::new(),
            Dither::Triangular { seed } => output_dither_pixels(seed)
                .as_chunks::<16>()
                .0
                .iter()
                .map(|group| {
                    std::array::from_fn(|c| {
                        std::array::from_fn(|i| f32::from(group[i * 4 + c]) * (2.0 / 255.0) - 1.0)
                    })
                })
                .collect(),
        };
        Self {
            parameters: Parameters {
                matrix: output_primaries_matrix(ColorPrimaries::Rec709),
                weights: output_luminance_weights(ColorPrimaries::Rec709),
                gamut: spec.gamut_map(),
                curve: transfer_lut(spec.target().transfer).expect("admitted sRGB curve"),
            },
            backend: Backend::detect(),
            cache: vec![EMPTY; CACHE_SIZE],
            previous: EMPTY,
            noise,
        }
    }

    pub(super) fn row(
        &mut self,
        input: &[u8],
        y: usize,
        swap: bool,
        cached: bool,
        output: &mut [u8],
    ) -> Result<(), &'static str> {
        if cached {
            self.dispatch::<true>(input, y, swap, output)
        } else {
            self.dispatch::<false>(input, y, swap, output)
        }
    }

    fn dispatch<const CACHED: bool>(
        &mut self,
        input: &[u8],
        y: usize,
        swap: bool,
        output: &mut [u8],
    ) -> Result<(), &'static str> {
        // SAFETY: the private backend is selected once, using runtime CPU/OS feature detection.
        // No build-wide target-cpu or optional ISA is required by the portable fallback.
        unsafe {
            match self.backend {
                Backend::Scalar => self.run::<Scalar, CACHED>(input, y, swap, output),
                #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
                Backend::Neon => neon::row::<CACHED>(self, input, y, swap, output),
                #[cfg(all(target_arch = "x86_64", target_endian = "little"))]
                Backend::Avx2 => x86::row::<CACHED>(self, input, y, swap, output),
            }
        }
    }

    /// Caller must support K's ISA. All slice lengths are checked before any vector loads.
    #[inline(always)]
    unsafe fn run<K: Kernel, const CACHED: bool>(
        &mut self,
        input: &[u8],
        y: usize,
        swap: bool,
        output: &mut [u8],
    ) -> Result<(), &'static str> {
        if !output.len().is_multiple_of(4)
            || !input.len().is_multiple_of(8)
            || input.len() / 8 != output.len() / 4
        {
            return Err("invalid F16 output row length");
        }
        // Keep complete batches fixed-size so LLVM can use whole vector loads/stores. Treating
        // every batch as a possible short tail forces length-dependent copies into the hot loop.
        let (sources, source_tail) = input.as_chunks::<32>();
        let (destinations, destination_tail) = output.as_chunks_mut::<16>();
        for (group, (source, destination)) in
            sources.iter().zip(destinations.iter_mut()).enumerate()
        {
            // SAFETY: dispatch checked K's ISA; every source/destination is a complete batch.
            *destination = unsafe { self.batch::<K, CACHED>(source, group, y, swap)? };
        }
        if !destination_tail.is_empty() {
            let mut source = [0_u8; 32];
            for pixel in source.as_chunks_mut::<8>().0 {
                pixel[6..].copy_from_slice(&0x3c00_u16.to_ne_bytes());
            }
            source[..source_tail.len()].copy_from_slice(source_tail);
            // SAFETY: padded tail is a complete batch; only real pixels are stored.
            let result = unsafe { self.batch::<K, CACHED>(&source, sources.len(), y, swap)? };
            destination_tail.copy_from_slice(&result[..destination_tail.len()]);
        }
        Ok(())
    }

    #[inline(always)]
    unsafe fn batch<K: Kernel, const CACHED: bool>(
        &mut self,
        source: &[u8; 32],
        group: usize,
        y: usize,
        swap: bool,
    ) -> Result<[u8; 16], &'static str> {
        let keys = std::array::from_fn(|i| read_key(source[i * 8..][..8].try_into().unwrap()));
        // SAFETY: caller checked K's ISA and provided four complete F16 pixels.
        let codes = unsafe { self.colors::<K, CACHED>(keys)? };
        let noise = self.noise.get((y % TILE) * (TILE / 4) + group % (TILE / 4));
        // SAFETY: colors are finite and bounded to [0, 255]; noise is in [-1, 1].
        Ok(unsafe { K::pack(codes, noise, swap) })
    }

    #[inline(always)]
    unsafe fn colors<K: Kernel, const CACHED: bool>(
        &mut self,
        keys: [u64; 4],
    ) -> Result<Codes, &'static str> {
        if CACHED && self.previous.valid && self.previous.keys == keys {
            return Ok(self.previous.codes);
        }
        let slot = if CACHED { cache_slot(keys) } else { 0 };
        if CACHED && self.cache[slot].valid && self.cache[slot].keys == keys {
            self.previous = self.cache[slot];
            return Ok(self.previous.codes);
        }
        // Exact-key hits were validated when inserted. Reject NaN/infinity and transparency
        // before conversion or lookup indexing; finite extended-range and subnormal RGB is valid.
        if keys
            .iter()
            .any(|key| key >> 48 != 0x3c00 || (0..3).any(|c| (key >> (16 * c)) & 0x7c00 == 0x7c00))
        {
            return Err("opaque CPU SDR output contains transparent or non-finite pixels");
        }
        // SAFETY: dispatch verified K's ISA and the batch above is finite and opaque.
        let codes = unsafe { K::colors(&self.parameters, keys) };
        if CACHED {
            let entry = Entry {
                keys,
                codes,
                valid: true,
            };
            self.cache[slot] = entry;
            self.previous = entry;
        }
        Ok(codes)
    }
}

fn read_key(bytes: &[u8; 8]) -> u64 {
    #[cfg(target_endian = "little")]
    {
        u64::from_le_bytes(*bytes)
    }
    #[cfg(target_endian = "big")]
    {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .fold(0, |key, (i, h)| {
                key | (u64::from(u16::from_ne_bytes([h[0], h[1]])) << (i * 16))
            })
    }
}

fn cache_slot(keys: [u64; 4]) -> usize {
    let hash =
        keys[0] ^ keys[1].rotate_left(11) ^ keys[2].rotate_left(23) ^ keys[3].rotate_left(37);
    (hash.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> (64 - CACHE_BITS)) as usize
}

trait Kernel {
    /// Requires the kernel ISA and four finite, opaque F16 pixels. Returns sRGB codes in [0,255].
    unsafe fn colors(parameters: &Parameters, keys: [u64; 4]) -> Codes;
    /// Requires the kernel ISA and bounded codes/noise. Rounds nearest, ties away from zero.
    unsafe fn pack(codes: Codes, noise: Option<&Codes>, swap: bool) -> [u8; 16];
}

struct Scalar;

impl Kernel for Scalar {
    #[inline]
    unsafe fn colors(p: &Parameters, keys: [u64; 4]) -> Codes {
        let pixels = keys.map(|key| {
            let rgb: [f64; 3] =
                std::array::from_fn(|c| f64::from(half_to_f32((key >> (16 * c)) as u16)));
            let mut target = p
                .matrix
                .map(|m| ((rgb[0] * m[0] + rgb[1] * m[1]) + rgb[2] * m[2]) as f32);
            if p.gamut == GamutMap::ChromaCompress
                && !target.iter().all(|v| (0.0..=1.0).contains(v))
            {
                let y = ((target[0] * p.weights[0] + target[1] * p.weights[1])
                    + target[2] * p.weights[2])
                    .clamp(0.0, 1.0);
                let mut scale = 1.0_f32;
                for channel in target {
                    let delta = channel - y;
                    if delta > 0.0 {
                        scale = scale.min((1.0 - y) / delta);
                    } else if delta < 0.0 {
                        scale = scale.min(y / -delta);
                    }
                }
                target = target.map(|v| y + (v - y) * scale);
            }
            target.map(|v| transfer_code(v.clamp(0.0, 1.0), Some(p.curve)))
        });
        std::array::from_fn(|c| std::array::from_fn(|i| pixels[i][c]))
    }

    #[inline]
    unsafe fn pack(codes: Codes, noise: Option<&Codes>, swap: bool) -> [u8; 16] {
        let mut result = [255; 16];
        for i in 0..4 {
            for c in 0..3 {
                let destination = if swap { 2 - c } else { c };
                result[i * 4 + destination] = (codes[c][i] + noise.map_or(0.0, |n| n[c][i]))
                    .clamp(0.0, 255.0)
                    .round() as u8;
            }
        }
        result
    }
}
