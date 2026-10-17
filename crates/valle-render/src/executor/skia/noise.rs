//! Noise displacement on Raster: `draw_noise_displacement` fills a rect with the shared runtime
//! kernel over Skia's Perlin noise; on aarch64, `draw` computes the kernel's bytes pixel by pixel
//! instead of through its raster pipeline program.

use skia_safe::{IRect, Image, Surface};

use super::draw::DrawError;

/// The Perlin noise the kernel reads, as Skia's noise shader is made from it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Noise {
    pub(super) frequency: [f32; 2],
    pub(super) octaves: u8,
    pub(super) seed: u32,
    pub(super) turbulence: bool,
}

/// The kernel with `uniforms` over `rect`, reading the `source` subset of `input` placed at canvas
/// `origin`, as its draw under the Src blend writes it. Returns `false`, drawing nothing, where
/// the kernel's own draw stays.
#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
pub(super) fn draw(
    surface: &mut Surface,
    input: &Image,
    source: IRect,
    origin: [i32; 2],
    rect: IRect,
    uniforms: &[f32; 15],
    noise: Noise,
) -> Result<bool, DrawError> {
    fused::draw(surface, input, source, origin, rect, uniforms, noise)
}

/// The kernel repeats the instructions of Skia's raster pipeline on aarch64; elsewhere the
/// kernel's own draw stays.
#[cfg(not(all(target_arch = "aarch64", target_endian = "little")))]
pub(super) fn draw(
    _surface: &mut Surface,
    _input: &Image,
    _source: IRect,
    _origin: [i32; 2],
    _rect: IRect,
    _uniforms: &[f32; 15],
    _noise: Noise,
) -> Result<bool, DrawError> {
    Ok(false)
}

#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
mod fused {
    use std::arch::aarch64::*;

    use skia_safe::{
        AlphaType, ColorType, IRect, Image, ImageInfo, Surface, surface::ContentChangeMode,
    };
    use valle_engine::compositor::bloom::fill_image_rows;

    use super::{
        super::draw::{DrawError, rows_clip},
        Noise,
    };

    /// Pixels per vector.
    const LANES: usize = 4;

    /// The noise channels the kernel reads: red and green displace, alpha unpremultiplies them.
    const CHANNELS: [usize; 3] = [0, 1, 3];

    pub(super) fn draw(
        surface: &mut Surface,
        input: &Image,
        source: IRect,
        origin: [i32; 2],
        rect: IRect,
        uniforms: &[f32; 15],
        noise: Noise,
    ) -> Result<bool, DrawError> {
        // Skia makes no noise shader for these frequencies, and a constant one for no octaves.
        if !(noise.frequency[0] >= 0.0 && noise.frequency[1] >= 0.0) || noise.octaves == 0 {
            return Ok(false);
        }
        let (Some(clip), Some(pixels)) = (rows_clip(surface), input.peek_pixels()) else {
            return Ok(false);
        };
        let target = surface.image_info();
        let half = |info: &ImageInfo| {
            info.color_type() == ColorType::RGBAF16 && info.alpha_type() == AlphaType::Premul
        };
        // In the target's color space the image reads its pixels as they are.
        if !half(&target)
            || !half(pixels.info())
            || pixels.info().color_space() != target.color_space()
        {
            return Ok(false);
        }
        let Some(rect) = IRect::intersect(&rect, &clip) else {
            return Ok(true);
        };
        let [to_local, origin_local, to_canvas, scale, bounds] =
            [0..4, 4..6, 6..10, 10..11, 11..15].map(|range| &uniforms[range]);
        let kernel = Kernel {
            to_local: to_local.try_into().unwrap(),
            local_origin: origin_local.try_into().unwrap(),
            to_canvas: to_canvas.try_into().unwrap(),
            scale: scale[0],
            bounds: bounds.try_into().unwrap(),
            // The image shader maps canvas points into the image by the inverse of its placement.
            image_offset: [
                -((origin[0] - source.left) as f32),
                -((origin[1] - source.top) as f32),
            ],
            image: Input {
                bytes: pixels
                    .bytes()
                    .ok_or_else(|| DrawError::Surface("Raster image has no pixels".into()))?,
                stride: pixels.row_bytes() / 8,
                size: [pixels.width() as f32, pixels.height() as f32],
            },
            lattice: Lattice::new(noise.seed as f32),
            frequency: noise.frequency,
            octaves: noise.octaves,
            turbulence: noise.turbulence,
        };
        // As for a Src draw: pixels a snapshot still shares are replaced before they are written,
        // by a copy unless the rect is the whole surface.
        let whole = rect == IRect::from_wh(surface.width(), surface.height());
        surface.notify_content_will_change(if whole {
            ContentChangeMode::Discard
        } else {
            ContentChangeMode::Retain
        });
        let mut out = surface
            .peek_pixels()
            .ok_or_else(|| DrawError::Surface("Raster surface has no pixels".into()))?;
        let row_bytes = out.row_bytes();
        let bytes = out
            .bytes_mut()
            .ok_or_else(|| DrawError::Surface("Raster surface pixels are not writable".into()))?;
        let span =
            rect.top as usize * row_bytes..(rect.bottom as usize * row_bytes).min(bytes.len());
        let columns = rect.left as usize * 8..rect.right as usize * 8;
        fill_image_rows(&mut bytes[span], row_bytes, |first, rows| {
            for (index, row) in rows.chunks_mut(row_bytes).enumerate() {
                let y = rect.top + (first + index) as i32;
                kernel.row(y, rect.left, &mut row[columns.clone()]);
            }
        });
        Ok(true)
    }

    /// The input's half-float pixels.
    struct Input<'a> {
        bytes: &'a [u8],
        /// Pixels per row.
        stride: usize,
        size: [f32; 2],
    }

    /// The kernel's uniforms and children.
    struct Kernel<'a> {
        to_local: [f32; 4],
        local_origin: [f32; 2],
        to_canvas: [f32; 4],
        scale: f32,
        bounds: [f32; 4],
        image_offset: [f32; 2],
        image: Input<'a>,
        lattice: Lattice,
        frequency: [f32; 2],
        octaves: u8,
        turbulence: bool,
    }

    impl Kernel<'_> {
        /// Writes the kernel's pixels of canvas row `y` from column `left` into `out`.
        fn row(&self, y: i32, left: i32, out: &mut [u8]) {
            // SAFETY: NEON is part of the aarch64 baseline; the load reads the four floats.
            let (iota, py) = unsafe {
                // The pipeline seeds each lane with its pixel's center.
                (
                    vld1q_f32([0.5, 1.5, 2.5, 3.5].as_ptr()),
                    vdupq_n_f32(y as f32 + 0.5),
                )
            };
            for (block, out) in out.chunks_mut(LANES * 8).enumerate() {
                let column = (left as usize + block * LANES) as f32;
                // SAFETY: NEON is part of the aarch64 baseline.
                let px = unsafe { vaddq_f32(vdupq_n_f32(column), iota) };
                let pixels = self.pixels(px, py);
                for (bytes, pixel) in out.as_chunks_mut::<8>().0.iter_mut().zip(&pixels) {
                    *bytes = pixel.to_le_bytes();
                }
            }
        }

        /// The kernel's `main` at the four points (`px`, `py`), as its raster pipeline program
        /// computes it: fused multiply-adds for each `dot`, and every other operation as written.
        /// Returns the four half-float pixels.
        #[inline(always)]
        fn pixels(&self, px: float32x4_t, py: float32x4_t) -> [u64; LANES] {
            // SAFETY: NEON is part of the aarch64 baseline; the stores write the four lanes of
            // each array, and each pixel read stays inside the input, whose indices are clamped
            // to its pixels.
            unsafe {
                let splat = |value: f32| vdupq_n_f32(value);
                let zero = splat(0.0);
                let one = splat(1.0);
                let half = splat(0.5);
                let dot = |row: &[f32], x: float32x4_t, y: float32x4_t| {
                    vfmaq_f32(vmulq_f32(splat(row[1]), y), splat(row[0]), x)
                };
                let local_x = vaddq_f32(
                    dot(&self.to_local[0..2], px, py),
                    splat(self.local_origin[0]),
                );
                let local_y = vaddq_f32(
                    dot(&self.to_local[2..4], px, py),
                    splat(self.local_origin[1]),
                );
                let [red, green, alpha] = self.noise(local_x, local_y);
                // `saturate(unpremul(noise))`: color over `max(alpha, 0.0001)`, clamped to [0, 1].
                let divisor = vmaxq_f32(alpha, splat(0.0001));
                let unit = |channel: float32x4_t| {
                    vminq_f32(vmaxq_f32(vdivq_f32(channel, divisor), zero), one)
                };
                let scale = splat(self.scale);
                let vector_x = vmulq_f32(scale, vsubq_f32(unit(red), half));
                let vector_y = vmulq_f32(scale, vsubq_f32(unit(green), half));
                let offset_x = dot(&self.to_canvas[0..2], vector_x, vector_y);
                let offset_y = dot(&self.to_canvas[2..4], vector_x, vector_y);
                let sample_x = vaddq_f32(vrndmq_f32(vaddq_f32(px, offset_x)), half);
                let sample_y = vaddq_f32(vrndmq_f32(vaddq_f32(py, offset_y)), half);
                let [left, top, right, bottom] = self.bounds;
                let outside = vorrq_u32(
                    vorrq_u32(
                        vcltq_f32(sample_x, splat(left)),
                        vcltq_f32(sample_y, splat(top)),
                    ),
                    vorrq_u32(
                        vcgtq_f32(sample_x, splat(right)),
                        vcgtq_f32(sample_y, splat(bottom)),
                    ),
                );
                // `image.eval(sample)`: the image shader translates the point into the image,
                // decals it to (0, size] on each axis and reads the nearest pixel, a point on a
                // pixel edge reading the pixel before it.
                let x = vaddq_f32(sample_x, splat(self.image_offset[0]));
                let y = vaddq_f32(sample_y, splat(self.image_offset[1]));
                let [width, height] = [splat(self.image.size[0]), splat(self.image.size[1])];
                let decal = |at: float32x4_t, limit: float32x4_t| {
                    vorrq_u32(
                        vandq_u32(vcltq_f32(zero, at), vcltq_f32(at, limit)),
                        vceqq_f32(at, limit),
                    )
                };
                let inside = vbicq_u32(vandq_u32(decal(x, width), decal(y, height)), outside);
                let texel = |at: float32x4_t, limit: float32x4_t| {
                    let below = vreinterpretq_f32_u32(vsubq_u32(
                        vreinterpretq_u32_f32(limit),
                        vdupq_n_u32(1),
                    ));
                    let clamped = vminq_f32(vmaxq_f32(splat(f32::MIN_POSITIVE), at), below);
                    let down = vsubq_u32(vreinterpretq_u32_f32(clamped), vdupq_n_u32(1));
                    vreinterpretq_u32_s32(vcvtq_s32_f32(vreinterpretq_f32_u32(down)))
                };
                let at = vmlaq_u32(
                    texel(x, width),
                    texel(y, height),
                    vdupq_n_u32(self.image.stride as u32),
                );
                let (mut indices, mut kept) = ([0_u32; LANES], [0_u32; LANES]);
                vst1q_u32(indices.as_mut_ptr(), at);
                vst1q_u32(kept.as_mut_ptr(), inside);
                let mut pixels = [0; LANES];
                for ((pixel, &index), &kept) in pixels.iter_mut().zip(&indices).zip(&kept) {
                    if kept != 0 {
                        let bytes = &self.image.bytes[index as usize * 8..][..8];
                        // The pixel widens to floats and the store narrows it back.
                        let halves = vcreate_u16(u64::from_le_bytes(bytes.try_into().unwrap()));
                        let stored = vcvt_f16_f32(vcvt_f32_f16(vreinterpret_f16_u16(halves)));
                        *pixel =
                            vget_lane_u64::<0>(vreinterpret_u64_u16(vreinterpret_u16_f16(stored)));
                    }
                }
                pixels
            }
        }

        /// Skia's Perlin noise stage at the four points (`x`, `y`): the premultiplied red, green
        /// and alpha of the noise. Blue does not reach the kernel's result.
        #[inline(always)]
        fn noise(&self, x: float32x4_t, y: float32x4_t) -> [float32x4_t; 3] {
            // SAFETY: NEON is part of the aarch64 baseline.
            unsafe {
                let splat = |value: f32| vdupq_n_f32(value);
                let zero = splat(0.0);
                let one = splat(1.0);
                let two = splat(2.0);
                let three = splat(3.0);
                let half = splat(0.5);
                let mut noise_x = vmulq_f32(vaddq_f32(x, half), splat(self.frequency[0]));
                let mut noise_y = vmulq_f32(vaddq_f32(y, half), splat(self.frequency[1]));
                let mut sums = [zero; 3];
                let mut ratio = 1.0_f32;
                let lerp = |from: float32x4_t, to: float32x4_t, t: float32x4_t| {
                    vfmaq_f32(from, vsubq_f32(to, from), t)
                };
                let smooth = |t: float32x4_t| {
                    vmulq_f32(vmulq_f32(t, t), vsubq_f32(three, vmulq_f32(two, t)))
                };
                for _ in 0..self.octaves {
                    let floor_x = vrndmq_f32(noise_x);
                    let floor_y = vrndmq_f32(noise_y);
                    let fract_x = vsubq_f32(noise_x, floor_x);
                    let fract_y = vsubq_f32(noise_y, floor_y);
                    let corners = self.lattice.corners(floor_x, floor_y);
                    let smooth_x = smooth(fract_x);
                    let smooth_y = smooth(fract_y);
                    let fract_x1 = vsubq_f32(fract_x, one);
                    let fract_y1 = vsubq_f32(fract_y, one);
                    let ratio_v = splat(ratio);
                    for (sum, &channel) in sums.iter_mut().zip(&CHANNELS) {
                        let dot = |corner: usize, x: float32x4_t, y: float32x4_t| {
                            let [gradient_x, gradient_y] = &corners[corner];
                            vfmaq_f32(vmulq_f32(gradient_y[channel], y), gradient_x[channel], x)
                        };
                        let a = lerp(
                            dot(0, fract_x, fract_y),
                            dot(1, fract_x1, fract_y),
                            smooth_x,
                        );
                        let b = lerp(
                            dot(2, fract_x, fract_y1),
                            dot(3, fract_x1, fract_y1),
                            smooth_x,
                        );
                        let mut color = lerp(a, b, smooth_y);
                        if self.turbulence {
                            color = vabsq_f32(color);
                        }
                        *sum = vfmaq_f32(*sum, color, ratio_v);
                    }
                    noise_x = vmulq_f32(noise_x, two);
                    noise_y = vmulq_f32(noise_y, two);
                    ratio *= 0.5;
                }
                if !self.turbulence {
                    for sum in &mut sums {
                        *sum = vfmaq_f32(half, *sum, half);
                    }
                }
                let clamp = |value: float32x4_t| vminq_f32(vmaxq_f32(zero, value), one);
                let [red, green, alpha] = sums;
                [
                    vmulq_f32(clamp(red), alpha),
                    vmulq_f32(clamp(green), alpha),
                    clamp(alpha),
                ]
            }
        }
    }

    /// Skia's Perlin noise tables for a seed, as `SkPerlinNoiseShader::PaintingData` builds them.
    struct Lattice {
        selector: [u8; 256],
        /// For each lattice point, the x components of its gradients in the four channels, then
        /// their y components: the stage's `mad(sample, 2 / 65535, -1)` of each stored sample.
        gradients: [[[f32; 4]; 2]; 256],
    }

    impl Lattice {
        fn new(seed: f32) -> Self {
            const MAXIMUM: i32 = i32::MAX;
            // `SkScalarTruncToInt` saturates to the ints a float holds, then truncates.
            let limit = 2_147_483_520.0_f32;
            let seed = if seed < limit { seed } else { limit };
            let mut state = (if seed > -limit { seed } else { -limit }) as i32;
            if state <= 0 {
                state = -(state % (MAXIMUM - 1)) + 1;
            }
            if state > MAXIMUM - 1 {
                state = MAXIMUM - 1;
            }
            // The Park-Miller generator of the SVG turbulence reference, by Schrage's method.
            let mut random = move || {
                let next = 16_807 * (state % 127_773) - 2_836 * (state / 127_773);
                state = if next <= 0 { next + MAXIMUM } else { next };
                state
            };
            let mut selector = [0_u8; 256];
            let mut noise = [[[0_u16; 2]; 256]; 4];
            for channel in &mut noise {
                for (point, sample) in channel.iter_mut().enumerate() {
                    selector[point] = point as u8;
                    let x = (random() % 512) as u16;
                    let y = (random() % 512) as u16;
                    *sample = [x, y];
                }
            }
            for point in (1..256).rev() {
                let swapped = selector[point];
                let other = (random() % 256) as usize;
                selector[point] = selector[other];
                selector[other] = swapped;
            }
            let mut gradients = [[[0.0; 4]; 2]; 256];
            for (channel, samples) in noise.iter().enumerate() {
                for (point, gradient) in gradients.iter_mut().enumerate() {
                    let sample = samples[usize::from(selector[point])];
                    let [x, y] = normalize(
                        sample.map(|value| (i32::from(value) - 256) as f32 * (1.0 / 256.0)),
                    );
                    for (axis, value) in [x, y].into_iter().enumerate() {
                        let stored = round_to_int((value + 1.0) * 32_767.5) as u16;
                        gradient[axis][channel] = f32::from(stored).mul_add(2.0 / 65_535.0, -1.0);
                    }
                }
            }
            Self {
                selector,
                gradients,
            }
        }

        /// The gradients at the corners of the lattice cells whose first corners are (`floor_x`,
        /// `floor_y`), in the stage's order: per corner, per axis, a vector of the four points
        /// for each channel.
        #[inline(always)]
        fn corners(
            &self,
            floor_x: float32x4_t,
            floor_y: float32x4_t,
        ) -> [[[float32x4_t; 4]; 2]; 4] {
            // SAFETY: NEON is part of the aarch64 baseline.
            unsafe {
                let one = vdupq_n_f32(1.0);
                let ceil_x = vaddq_f32(floor_x, one);
                let ceil_y = vaddq_f32(floor_y, one);
                let shared = vandq_u32(
                    vceqq_f32(floor_x, vdupq_laneq_f32::<0>(floor_x)),
                    vceqq_f32(floor_y, vdupq_laneq_f32::<0>(floor_y)),
                );
                if vminvq_u32(shared) != 0 {
                    // The four points are in one cell, so every lookup takes the same entries:
                    // the first point's are read once and spread to all four.
                    let first = |value: float32x4_t| vgetq_lane_u32::<0>(lookup(value)) as usize;
                    let lattice_x = vdupq_n_f32(f32::from(self.selector[first(floor_x)]));
                    let lattice_y = vdupq_n_f32(f32::from(self.selector[first(ceil_x)]));
                    return [
                        self.spread(first(vaddq_f32(lattice_x, floor_y))),
                        self.spread(first(vaddq_f32(lattice_y, floor_y))),
                        self.spread(first(vaddq_f32(lattice_x, ceil_y))),
                        self.spread(first(vaddq_f32(lattice_y, ceil_y))),
                    ];
                }
                let lattice_x = self.select(lookup(floor_x));
                let lattice_y = self.select(lookup(ceil_x));
                [
                    self.gather(lookup(vaddq_f32(lattice_x, floor_y))),
                    self.gather(lookup(vaddq_f32(lattice_y, floor_y))),
                    self.gather(lookup(vaddq_f32(lattice_x, ceil_y))),
                    self.gather(lookup(vaddq_f32(lattice_y, ceil_y))),
                ]
            }
        }

        /// The gradients of the lattice point at `index`, each spread to all four lanes: per
        /// axis, a vector for each channel.
        #[inline(always)]
        fn spread(&self, index: usize) -> [[float32x4_t; 4]; 2] {
            // SAFETY: NEON is part of the aarch64 baseline; each load reads the four floats of
            // one axis of the point.
            unsafe {
                let point = &self.gradients[index];
                let axis = |axis: usize| {
                    let channels = vld1q_f32(point[axis].as_ptr());
                    [
                        vdupq_laneq_f32::<0>(channels),
                        vdupq_laneq_f32::<1>(channels),
                        vdupq_laneq_f32::<2>(channels),
                        vdupq_laneq_f32::<3>(channels),
                    ]
                };
                [axis(0), axis(1)]
            }
        }

        /// The selector entries at four indices below 256, as floats.
        #[inline(always)]
        fn select(&self, index: uint32x4_t) -> float32x4_t {
            // SAFETY: NEON is part of the aarch64 baseline.
            unsafe {
                let mut indices = [0_u32; LANES];
                vst1q_u32(indices.as_mut_ptr(), index);
                let entry = |lane: usize| u32::from(self.selector[indices[lane] as usize & 0xff]);
                let entries = [entry(0), entry(1), entry(2), entry(3)];
                vcvtq_f32_s32(vreinterpretq_s32_u32(vld1q_u32(entries.as_ptr())))
            }
        }

        /// The gradients at four indices below 256: per axis, a vector of the four points for
        /// each channel.
        #[inline(always)]
        fn gather(&self, index: uint32x4_t) -> [[float32x4_t; 4]; 2] {
            // SAFETY: NEON is part of the aarch64 baseline; each load reads the four floats of
            // one axis of one lattice point.
            unsafe {
                let mut indices = [0_u32; LANES];
                vst1q_u32(indices.as_mut_ptr(), index);
                let point = |lane: usize| &self.gradients[indices[lane] as usize & 0xff];
                let points = [point(0), point(1), point(2), point(3)];
                let axis = |axis: usize| {
                    transpose([
                        vld1q_f32(points[0][axis].as_ptr()),
                        vld1q_f32(points[1][axis].as_ptr()),
                        vld1q_f32(points[2][axis].as_ptr()),
                        vld1q_f32(points[3][axis].as_ptr()),
                    ])
                };
                [axis(0), axis(1)]
            }
        }
    }

    /// The stage's lattice index of each lane: the nearest integer, ties to even, wrapped to 256.
    #[inline(always)]
    fn lookup(value: float32x4_t) -> uint32x4_t {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe {
            vandq_u32(
                vreinterpretq_u32_s32(vcvtnq_s32_f32(value)),
                vdupq_n_u32(0xff),
            )
        }
    }

    /// Rows of four lanes into columns.
    #[inline(always)]
    fn transpose(rows: [float32x4_t; 4]) -> [float32x4_t; 4] {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe {
            let low = vtrnq_f32(rows[0], rows[1]);
            let high = vtrnq_f32(rows[2], rows[3]);
            [
                vcombine_f32(vget_low_f32(low.0), vget_low_f32(high.0)),
                vcombine_f32(vget_low_f32(low.1), vget_low_f32(high.1)),
                vcombine_f32(vget_high_f32(low.0), vget_high_f32(high.0)),
                vcombine_f32(vget_high_f32(low.1), vget_high_f32(high.1)),
            ]
        }
    }

    /// `SkPoint::normalize`: the length in double precision, each coordinate scaled in double
    /// precision and rounded back, and the zero vector where that is not finite or vanishes.
    fn normalize([x, y]: [f32; 2]) -> [f32; 2] {
        let (wide_x, wide_y) = (f64::from(x), f64::from(y));
        let scale = 1.0 / (wide_x * wide_x + wide_y * wide_y).sqrt();
        let scaled = [(wide_x * scale) as f32, (wide_y * scale) as f32];
        if !scaled.iter().all(|value| value.is_finite()) || scaled == [0.0, 0.0] {
            return [0.0, 0.0];
        }
        scaled
    }

    /// `SkScalarRoundToInt`: the floor of the value plus one half in double precision, as a
    /// float, saturated to the ints a float holds.
    fn round_to_int(value: f32) -> i32 {
        let limit = 2_147_483_520.0_f32;
        let rounded = (f64::from(value) + 0.5).floor() as f32;
        if rounded.is_nan() {
            return limit as i32;
        }
        rounded.clamp(-limit, limit) as i32
    }
}
