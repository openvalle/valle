//! Color matrices on Raster: on aarch64, `draw` computes the pixels of a chain of color matrix
//! filters directly, with the instructions of Skia's raster pipeline, instead of drawing their
//! image filters.

use skia_safe::{IRect, Pixmap, Surface};

use super::draw::DrawError;

/// The chain of row-major `matrices`, each keeping transparent black, over the canvas clip,
/// from the `subset` at the corner of `pixels` placed at canvas `origin`, as the draw of their
/// image filters under the Src blend writes it: the chain of each pixel inside the placement and
/// transparent black around it. With `rounded`, each matrix after the first reads the half
/// floats the one before it wrote, as when each is drawn into its own image; otherwise the chain
/// keeps full precision, as one draw of their composed filters does. Returns `false`, drawing
/// nothing, where that draw stays, which includes a placement outside the clip: Skia rejects
/// that draw before filtering, so it writes nothing.
#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
pub(super) fn draw(
    surface: &mut Surface,
    pixels: &Pixmap,
    subset: IRect,
    origin: [i32; 2],
    matrices: &[[f32; 20]],
    rounded: bool,
) -> Result<bool, DrawError> {
    fused::draw(surface, pixels, subset, origin, matrices, rounded)
}

/// The chain repeats the instructions of Skia's raster pipeline on aarch64; elsewhere the draw
/// of the filters stays.
#[cfg(not(all(target_arch = "aarch64", target_endian = "little")))]
pub(super) fn draw(
    _surface: &mut Surface,
    _pixels: &Pixmap,
    _subset: IRect,
    _origin: [i32; 2],
    _matrices: &[[f32; 20]],
    _rounded: bool,
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

    use super::super::draw::{DrawError, rows_clip};

    /// Pixels per vector.
    const LANES: usize = 4;

    pub(super) fn draw(
        surface: &mut Surface,
        pixels: &Pixmap,
        subset: IRect,
        origin: [i32; 2],
        matrices: &[[f32; 20]],
        rounded: bool,
    ) -> Result<bool, DrawError> {
        let target = surface.image_info();
        let half = |info: &ImageInfo| {
            info.color_type() == ColorType::RGBAF16 && info.alpha_type() == AlphaType::Premul
        };
        // In the target's color space the filters read the pixels as they are. Skia makes no
        // filter of a matrix that is not finite, and one that does not keep transparent black
        // also changes the pixels around the placement.
        if !half(&target)
            || !half(pixels.info())
            || pixels.info().color_space() != target.color_space()
            || matrices.is_empty()
            || matrices
                .iter()
                .any(|matrix| matrix[19] != 0.0 || !matrix.iter().all(|value| value.is_finite()))
            || subset.is_empty()
            || subset.left != 0
            || subset.top != 0
            || subset.right > pixels.width()
            || subset.bottom > pixels.height()
        {
            return Ok(false);
        }
        let placed = IRect::from_xywh(origin[0], origin[1], subset.width(), subset.height());
        let Some(clip) = rows_clip(surface).filter(|clip| IRect::intersects(&placed, clip)) else {
            return Ok(false);
        };
        let bytes = pixels
            .bytes()
            .ok_or_else(|| DrawError::Surface("Raster image has no pixels".into()))?;
        // The reads load the halves of four pixels together.
        if !bytes.as_ptr().cast::<u16>().is_aligned() || !pixels.row_bytes().is_multiple_of(8) {
            return Ok(false);
        }
        let chain = Chain {
            bytes,
            stride: pixels.row_bytes(),
            origin,
            matrices,
            rounded,
        };
        // As for a Src draw: pixels a snapshot still shares are replaced before they are written,
        // by a copy unless the clip is the whole surface.
        let whole = clip == IRect::from_wh(surface.width(), surface.height());
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
            clip.top as usize * row_bytes..(clip.bottom as usize * row_bytes).min(bytes.len());
        // The columns of the clip that the placement covers, from the clip's left edge.
        let left = placed.left.clamp(clip.left, clip.right);
        let right = placed.right.clamp(left, clip.right);
        let columns = [left - clip.left, right - clip.left].map(|value| value as usize * 8);
        fill_image_rows(&mut bytes[span], row_bytes, |first, rows| {
            for (index, row) in rows.chunks_mut(row_bytes).enumerate() {
                let y = clip.top + (first + index) as i32;
                let row = &mut row[clip.left as usize * 8..clip.right as usize * 8];
                if !(placed.top..placed.bottom).contains(&y) {
                    row.fill(0);
                    continue;
                }
                let (before, row) = row.split_at_mut(columns[0]);
                let (inside, after) = row.split_at_mut(columns[1] - columns[0]);
                before.fill(0);
                if !inside.is_empty() {
                    chain.row(y, left, inside);
                }
                after.fill(0);
            }
        });
        Ok(true)
    }

    struct Chain<'a> {
        bytes: &'a [u8],
        /// Bytes per row.
        stride: usize,
        /// The canvas pixel of the input's corner.
        origin: [i32; 2],
        matrices: &'a [[f32; 20]],
        rounded: bool,
    }

    impl Chain<'_> {
        /// Writes the chain's pixels of canvas row `y` from column `left` into `out`.
        fn row(&self, y: i32, left: i32, out: &mut [u8]) {
            let start =
                (y - self.origin[1]) as usize * self.stride + (left - self.origin[0]) as usize * 8;
            let input = &self.bytes[start..][..out.len()];
            let (blocks, tail) = out.as_chunks_mut::<{ LANES * 8 }>();
            let (inputs, input_tail) = input.as_chunks::<{ LANES * 8 }>();
            for (out, input) in blocks.iter_mut().zip(inputs) {
                // SAFETY: the block holds four pixels of four halves, aligned for the halves by
                // the alignment of the input's rows.
                let halves = unsafe { self.four(input.as_ptr().cast()) };
                for (bytes, half) in out.as_chunks_mut::<2>().0.iter_mut().zip(&halves) {
                    *bytes = half.to_le_bytes();
                }
            }
            if !tail.is_empty() {
                let mut padded = [0_u16; LANES * 4];
                for (half, bytes) in padded.iter_mut().zip(input_tail.as_chunks::<2>().0) {
                    *half = u16::from_le_bytes(*bytes);
                }
                // SAFETY: the buffer holds four pixels of four halves.
                let halves = unsafe { self.four(padded.as_ptr()) };
                for (bytes, half) in tail.as_chunks_mut::<2>().0.iter_mut().zip(&halves) {
                    *bytes = half.to_le_bytes();
                }
            }
        }

        /// The chain's halves of the four pixels of halves at `input`.
        ///
        /// # Safety
        ///
        /// `input` points to sixteen halves.
        #[inline(always)]
        unsafe fn four(&self, input: *const u16) -> [u16; LANES * 4] {
            // SAFETY: NEON is part of the aarch64 baseline; the caller provides the halves.
            unsafe {
                let halves = vld4_u16(input);
                let widen = |value: uint16x4_t| vcvt_f32_f16(vreinterpret_f16_u16(value));
                let mut pixel = [
                    widen(halves.0),
                    widen(halves.1),
                    widen(halves.2),
                    widen(halves.3),
                ];
                for (index, matrix) in self.matrices.iter().enumerate() {
                    if self.rounded && index > 0 {
                        let round = |value: float32x4_t| vcvt_f32_f16(vcvt_f16_f32(value));
                        pixel = [
                            round(pixel[0]),
                            round(pixel[1]),
                            round(pixel[2]),
                            round(pixel[3]),
                        ];
                    }
                    pixel = apply(matrix, pixel);
                }
                let narrow = |value: float32x4_t| vreinterpret_u16_f16(vcvt_f16_f32(value));
                let mut halves = [0_u16; LANES * 4];
                vst4_u16(
                    halves.as_mut_ptr(),
                    uint16x4x4_t(
                        narrow(pixel[0]),
                        narrow(pixel[1]),
                        narrow(pixel[2]),
                        narrow(pixel[3]),
                    ),
                );
                halves
            }
        }
    }

    /// The stages of a color matrix filter on premultiplied pixels: `unpremul`, `matrix_4x5`,
    /// `clamp_01` and `premul`, in their operations and operand order.
    #[inline(always)]
    fn apply(matrix: &[f32; 20], [r, g, b, a]: [float32x4_t; 4]) -> [float32x4_t; 4] {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe {
            let inverse = vdivq_f32(vdupq_n_f32(1.0), a);
            let scale = vbslq_f32(
                vcltq_f32(inverse, vdupq_n_f32(f32::INFINITY)),
                inverse,
                vdupq_n_f32(0.0),
            );
            let [r, g, b] = [
                vmulq_f32(r, scale),
                vmulq_f32(g, scale),
                vmulq_f32(b, scale),
            ];
            // Skia's `mad(f, m, a)` is `vfmaq_f32(a, f, m)`.
            let row = |m: &[f32]| {
                let value = vfmaq_f32(vdupq_n_f32(m[4]), a, vdupq_n_f32(m[3]));
                let value = vfmaq_f32(value, b, vdupq_n_f32(m[2]));
                let value = vfmaq_f32(value, g, vdupq_n_f32(m[1]));
                vfmaq_f32(value, r, vdupq_n_f32(m[0]))
            };
            let clamp = |value: float32x4_t| {
                vminq_f32(vmaxq_f32(vdupq_n_f32(0.0), value), vdupq_n_f32(1.0))
            };
            let [r, g, b, a] = [
                clamp(row(&matrix[0..5])),
                clamp(row(&matrix[5..10])),
                clamp(row(&matrix[10..15])),
                clamp(row(&matrix[15..20])),
            ];
            [vmulq_f32(r, a), vmulq_f32(g, a), vmulq_f32(b, a), a]
        }
    }
}
