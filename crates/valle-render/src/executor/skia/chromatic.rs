//! Chromatic aberration on Raster: `draw_chromatic_aberration` fills the clip with the shared runtime
//! kernel over the input; on aarch64, `draw` computes the kernel's bytes pixel by pixel instead of
//! through its raster pipeline program.

use skia_safe::{IRect, Pixmap, Surface};

use super::draw::DrawError;

/// How the kernel's image shader reads the input for a clip.
#[derive(Debug, Clone, Copy)]
pub(super) struct Sampling {
    /// The canvas clip, which the kernel fills.
    pub(super) clip: IRect,
    /// The input pixels the shader reads, from the backing's corner: the source's or all of them.
    pub(super) size: [i32; 2],
    /// Whether the shader clamps to those pixels or decals them.
    pub(super) clamp: bool,
}

/// The kernel with `offset` over the sampling's clip, reading `pixels` placed at canvas `origin`,
/// as its draw under the Src blend writes it. Returns `false`, drawing nothing, where the kernel's
/// own draw stays.
#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
pub(super) fn draw(
    surface: &mut Surface,
    pixels: &Pixmap,
    sampling: Sampling,
    origin: [i32; 2],
    offset: [f32; 2],
) -> Result<bool, DrawError> {
    fused::draw(surface, pixels, sampling, origin, offset)
}

/// The kernel repeats the instructions of Skia's raster pipeline on aarch64; elsewhere the
/// kernel's own draw stays.
#[cfg(not(all(target_arch = "aarch64", target_endian = "little")))]
pub(super) fn draw(
    _surface: &mut Surface,
    _pixels: &Pixmap,
    _sampling: Sampling,
    _origin: [i32; 2],
    _offset: [f32; 2],
) -> Result<bool, DrawError> {
    Ok(false)
}

#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
mod fused {
    use std::arch::aarch64::*;

    use skia_safe::{
        AlphaType, ColorType, IRect, ImageInfo, Pixmap, Surface, surface::ContentChangeMode,
    };
    use valle_engine::compositor::bloom::fill_image_rows;

    use super::{super::draw::DrawError, Sampling};

    /// Pixels per vector.
    const LANES: usize = 4;

    pub(super) fn draw(
        surface: &mut Surface,
        pixels: &Pixmap,
        sampling: Sampling,
        origin: [i32; 2],
        offset: [f32; 2],
    ) -> Result<bool, DrawError> {
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
        let bytes = pixels
            .bytes()
            .ok_or_else(|| DrawError::Surface("Raster image has no pixels".into()))?;
        // The reads load the halves of a pixel together.
        if !bytes.as_ptr().cast::<u16>().is_aligned() || !pixels.row_bytes().is_multiple_of(8) {
            return Ok(false);
        }
        let kernel = Kernel {
            image: Input {
                bytes,
                stride: pixels.row_bytes() / 8,
                size: sampling.size.map(|value| value as f32),
            },
            clamp: sampling.clamp,
            // The image shader maps canvas points into the image by the inverse of its placement.
            translate: origin.map(|value| -(value as f32)),
            reach: offset.map(|value| value * 0.5),
        };
        let rect = sampling.clip;
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

    /// The input's half-float pixels that the image shader reads.
    struct Input<'a> {
        bytes: &'a [u8],
        /// Pixels per row.
        stride: usize,
        size: [f32; 2],
    }

    /// The kernel's uniform and child.
    struct Kernel<'a> {
        image: Input<'a>,
        clamp: bool,
        translate: [f32; 2],
        /// `offset * 0.5`: how far before and after each point the kernel reads red and blue.
        reach: [f32; 2],
    }

    /// The image shader's two bilinear taps along one axis at four points, before and after:
    /// the pixel each reads along the axis, its weight, and whether decal keeps it.
    #[derive(Clone, Copy)]
    struct Taps {
        index: [uint32x4_t; 2],
        weight: [float32x4_t; 2],
        kept: [uint32x4_t; 2],
    }

    impl Kernel<'_> {
        /// Writes the kernel's pixels of canvas row `y` from column `left` into `out`.
        fn row(&self, y: i32, left: i32, out: &mut [u8]) {
            // SAFETY: NEON is part of the aarch64 baseline; the load reads the four floats.
            unsafe {
                // The pipeline seeds each lane with its pixel's center.
                let iota = vld1q_f32([0.5, 1.5, 2.5, 3.5].as_ptr());
                let py = vdupq_n_f32(y as f32 + 0.5);
                // The kernel reads red at `p - offset * 0.5`, green at `p` and blue at
                // `p + offset * 0.5`. Along y their taps are the same for the whole row.
                let reach_y = vdupq_n_f32(self.reach[1]);
                let rows = [
                    self.rows(vsubq_f32(py, reach_y)),
                    self.rows(py),
                    self.rows(vaddq_f32(py, reach_y)),
                ];
                let reach_x = vdupq_n_f32(self.reach[0]);
                for (block, out) in out.chunks_mut(LANES * 8).enumerate() {
                    let column = (left as usize + block * LANES) as f32;
                    let px = vaddq_f32(vdupq_n_f32(column), iota);
                    let [red, red_alpha] =
                        self.read::<0>(self.taps(vsubq_f32(px, reach_x), 0), &rows[0]);
                    let [green, green_alpha] = self.read::<1>(self.taps(px, 0), &rows[1]);
                    let [blue, blue_alpha] =
                        self.read::<2>(self.taps(vaddq_f32(px, reach_x), 0), &rows[2]);
                    let alpha = vmaxq_f32(red_alpha, vmaxq_f32(green_alpha, blue_alpha));
                    let narrow = |value: float32x4_t| vreinterpret_u16_f16(vcvt_f16_f32(value));
                    let mut halves = [0_u16; LANES * 4];
                    vst4_u16(
                        halves.as_mut_ptr(),
                        uint16x4x4_t(narrow(red), narrow(green), narrow(blue), narrow(alpha)),
                    );
                    for (bytes, half) in out.as_chunks_mut::<2>().0.iter_mut().zip(&halves) {
                        *bytes = half.to_le_bytes();
                    }
                }
            }
        }

        /// The taps along y of four points on one row, with each tap's row offset in pixels.
        #[inline(always)]
        fn rows(&self, y: float32x4_t) -> Taps {
            let mut taps = self.taps(y, 1);
            // SAFETY: NEON is part of the aarch64 baseline.
            unsafe {
                let stride = vdupq_n_u32(self.image.stride as u32);
                taps.index = [
                    vmulq_u32(taps.index[0], stride),
                    vmulq_u32(taps.index[1], stride),
                ];
            }
            taps
        }

        /// The image shader's taps along `axis` (0 for x) at the four coordinates `at`: its
        /// translation, then `bilinear_setup` and `bilinear_nx` / `px` (or `ny` / `py`), the
        /// gather's clamp to the pixels, and the decal mask.
        #[inline(always)]
        fn taps(&self, at: float32x4_t, axis: usize) -> Taps {
            // SAFETY: NEON is part of the aarch64 baseline.
            unsafe {
                let half = vdupq_n_f32(0.5);
                let at = vaddq_f32(at, vdupq_n_f32(self.translate[axis]));
                let after = vaddq_f32(at, half);
                let fraction = vsubq_f32(after, vrndmq_f32(after));
                let before = vaddq_f32(at, vdupq_n_f32(-0.5));
                let limit = vdupq_n_f32(self.image.size[axis]);
                // The gather clamps to (0, size) exclusive and truncates.
                let last =
                    vreinterpretq_f32_u32(vsubq_u32(vreinterpretq_u32_f32(limit), vdupq_n_u32(1)));
                let index = |at: float32x4_t| {
                    let clamped = vminq_f32(vmaxq_f32(vdupq_n_f32(f32::MIN_POSITIVE), at), last);
                    vreinterpretq_u32_s32(vcvtq_s32_f32(clamped))
                };
                // Decal keeps [0, size): with bilinear reads the closed end is at 0.
                let zero = vdupq_n_f32(0.0);
                let kept = |at: float32x4_t| {
                    vorrq_u32(
                        vandq_u32(vcltq_f32(zero, at), vcltq_f32(at, limit)),
                        vceqq_f32(at, zero),
                    )
                };
                Taps {
                    index: [index(before), index(after)],
                    weight: [vsubq_f32(vdupq_n_f32(1.0), fraction), fraction],
                    kept: [kept(before), kept(after)],
                }
            }
        }

        /// `image.eval` at the four points whose taps are `columns` and `rows`: channel `C` and
        /// alpha of the shader's result, its four taps in their order, each read, decaled and
        /// accumulated with the product of its weights.
        #[inline(always)]
        fn read<const C: usize>(&self, columns: Taps, rows: &Taps) -> [float32x4_t; 2] {
            // SAFETY: NEON is part of the aarch64 baseline; the load reads the four indices.
            unsafe {
                // A row's taps read one pixel row, so away from the clamped edges the four points
                // of a column tap read four adjacent pixels.
                let iota = vld1q_u32([0, 1, 2, 3].as_ptr());
                let in_a_row = |index: uint32x4_t| {
                    let run = vaddq_u32(vdupq_laneq_u32::<0>(index), iota);
                    vminvq_u32(vceqq_u32(index, run)) != 0
                };
                let adjacent = [in_a_row(columns.index[0]), in_a_row(columns.index[1])];
                let mut color = vdupq_n_f32(0.0);
                let mut alpha = vdupq_n_f32(0.0);
                for row in 0..2 {
                    for column in 0..2 {
                        let index = vaddq_u32(rows.index[row], columns.index[column]);
                        let texels = if adjacent[column] {
                            self.load(vgetq_lane_u32::<0>(index))
                        } else {
                            self.gather(index)
                        };
                        let channel = match C {
                            0 => texels.0,
                            1 => texels.1,
                            _ => texels.2,
                        };
                        let widen = |halves: uint16x4_t| vcvt_f32_f16(vreinterpret_f16_u16(halves));
                        let (mut value, mut opacity) = (widen(channel), widen(texels.3));
                        if !self.clamp {
                            let kept = vandq_u32(columns.kept[column], rows.kept[row]);
                            let keep = |value: float32x4_t| {
                                vreinterpretq_f32_u32(vandq_u32(vreinterpretq_u32_f32(value), kept))
                            };
                            (value, opacity) = (keep(value), keep(opacity));
                        }
                        let scale = vmulq_f32(columns.weight[column], rows.weight[row]);
                        color = vfmaq_f32(color, scale, value);
                        alpha = vfmaq_f32(alpha, scale, opacity);
                    }
                }
                [color, alpha]
            }
        }

        /// The four pixels from index `first` into the input, as vectors of their red, green, blue
        /// and alpha halves.
        #[inline(always)]
        fn load(&self, first: u32) -> uint16x4x4_t {
            let pixels = &self.image.bytes[first as usize * 8..][..LANES * 8];
            // SAFETY: NEON is part of the aarch64 baseline; the load reads the aligned halves of
            // the four pixels inside the input.
            unsafe { vld4_u16(pixels.as_ptr().cast::<u16>()) }
        }

        /// The pixels at four indices into the input, as vectors of their red, green, blue and
        /// alpha halves.
        #[inline(always)]
        fn gather(&self, index: uint32x4_t) -> uint16x4x4_t {
            // SAFETY: NEON is part of the aarch64 baseline; the store writes the four lanes, and
            // each load reads the four aligned halves of one pixel inside the input.
            unsafe {
                let mut indices = [0_u32; LANES];
                vst1q_u32(indices.as_mut_ptr(), index);
                let pixel = |lane: usize| {
                    self.image.bytes[indices[lane] as usize * 8..][..8]
                        .as_ptr()
                        .cast::<u16>()
                };
                let zero = vdup_n_u16(0);
                let mut texels = uint16x4x4_t(zero, zero, zero, zero);
                texels = vld4_lane_u16::<0>(pixel(0), texels);
                texels = vld4_lane_u16::<1>(pixel(1), texels);
                texels = vld4_lane_u16::<2>(pixel(2), texels);
                vld4_lane_u16::<3>(pixel(3), texels)
            }
        }
    }
}
