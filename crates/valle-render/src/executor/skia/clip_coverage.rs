//! Clip-path coverage on Raster: on aarch64, `apply_whole_pixel_rect` multiplies the pixels of
//! the canvas's top layer by the coverage of a clip path that is a rectangle on whole pixels, with
//! the instructions of Skia's raster pipeline, in place of the layer of that coverage which Skia
//! restores over them with `DstIn`.

use skia_safe::{Canvas, Path as SkPath};

/// Multiplies the pixels of the canvas's top layer by the coverage of `outline`, as an
/// anti-aliased white fill of it in a layer pushed over them and a `DstIn` restore of that layer
/// do, where that coverage is whole: the outline is a rectangle whose sides the transform puts on
/// whole pixels, so that it covers the pixels inside it fully and no others. Returns `false`,
/// changing nothing, otherwise.
#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
pub(super) fn apply_whole_pixel_rect(canvas: &Canvas, outline: &SkPath) -> bool {
    fused::apply_whole_pixel_rect(canvas, outline)
}

/// The pixels repeat the instructions of Skia's raster pipeline on aarch64; elsewhere Skia's
/// layer stays.
#[cfg(not(all(target_arch = "aarch64", target_endian = "little")))]
pub(super) fn apply_whole_pixel_rect(_canvas: &Canvas, _outline: &SkPath) -> bool {
    false
}

#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
mod fused {
    use std::arch::aarch64::*;

    use skia_safe::{AlphaType, Canvas, ColorType, M44, Path as SkPath};
    use valle_engine::compositor::bloom::fill_image_rows;

    use super::super::gradient_fill::subtracts_exactly;

    pub(super) fn apply_whole_pixel_rect(canvas: &Canvas, outline: &SkPath) -> bool {
        // Skia draws on devices wider or taller than this in tiles.
        const UNTILED: i32 = 8191;
        // Under an empty clip no layer was pushed, and the coverage's draws change nothing.
        if canvas.is_clip_empty() || outline.is_inverse_fill_type() {
            return false;
        }
        let Some((rect, ..)) = outline.is_rect() else {
            return false;
        };
        let Some(layer) = canvas.access_top_layer_pixels() else {
            return false;
        };
        let (width, height) = (layer.info.width(), layer.info.height());
        if layer.info.color_type() != ColorType::RGBAF16
            || layer.info.alpha_type() != AlphaType::Premul
            || width > UNTILED
            || height > UNTILED
            || !layer.pixels.as_ptr().cast::<u16>().is_aligned()
            || !layer.row_bytes.is_multiple_of(8)
        {
            return false;
        }
        // The coverage's layer fills the clip of this one, which is all of it, so it starts where
        // this one does and maps the transform to its pixels alike: as `fill_rect_inside` has it,
        // by the product of a translation by the negated origin with it.
        let origin = layer.origin;
        let global = canvas.local_to_device();
        if global.rc(3, 3) != 1.0
            || !subtracts_exactly(global.rc(0, 3), origin.x)
            || !subtracts_exactly(global.rc(1, 3), origin.y)
        {
            return false;
        }
        let device = M44::concat(
            &M44::translate(-origin.x as f32, -origin.y as f32, 0.0),
            &global,
        )
        .to_m33();
        if !device.is_scale_translate() {
            return false;
        }
        // A side lands on a whole pixel where its product with the scale is a float, and adding
        // the translation to that rounds nowhere and gives a whole number: mapping the corners
        // with a fused multiply-add or without one puts it there.
        let side = |value: f32, scale: f32, translate: f32| {
            let product = value * scale;
            let sum = product + translate;
            (f64::from(product) == f64::from(value) * f64::from(scale)
                && f64::from(sum) == f64::from(product) + f64::from(translate)
                && sum.fract() == 0.0
                && sum.abs() <= 16_777_216.0)
                .then_some(sum as i32)
        };
        let (Some(left), Some(right), Some(top), Some(bottom)) = (
            side(rect.left, device.scale_x(), device.translate_x()),
            side(rect.right, device.scale_x(), device.translate_x()),
            side(rect.top, device.scale_y(), device.translate_y()),
            side(rect.bottom, device.scale_y(), device.translate_y()),
        ) else {
            return false;
        };
        let [left, right] = [left.min(right), left.max(right)].map(|x| x.clamp(0, width));
        let [top, bottom] = [top.min(bottom), top.max(bottom)].map(|y| y.clamp(0, height));
        // `DstIn` multiplies every half of a pixel by the coverage's alpha, 1 inside the
        // rectangle and 0 outside it.
        let row_bytes = layer.row_bytes;
        let used = width as usize * 8;
        fill_image_rows(layer.pixels, row_bytes, |first, rows| {
            for (index, row) in rows.chunks_mut(row_bytes).enumerate() {
                let y = (first + index) as i32;
                let [start, end] = if (top..bottom).contains(&y) {
                    [left, right].map(|x| x as usize * 8)
                } else {
                    [0; 2]
                };
                let (outside, rest) = row[..used].split_at_mut(start);
                let (inside, after) = rest.split_at_mut(end - start);
                scale(outside, 0.0);
                scale(inside, 1.0);
                scale(after, 0.0);
            }
        });
        true
    }

    /// Multiplies the halves of the pixels in `bytes` by `alpha`, as `load_f16_dst`, `dstin` and
    /// `store_f16` do with a coverage of that alpha.
    fn scale(bytes: &mut [u8], alpha: f32) {
        // Widening, multiplying by 1 and narrowing quiets signaling NaNs, which the compiler
        // would not keep for an `alpha` it knows.
        let alpha = std::hint::black_box(alpha);
        // SAFETY: NEON is part of the aarch64 baseline; each pixel holds four halves, aligned for
        // them by the alignment of the rows.
        unsafe {
            let alpha = vdupq_n_f32(alpha);
            for pixel in bytes.as_chunks_mut::<8>().0 {
                let pixel = pixel.as_mut_ptr().cast::<u16>();
                let color = vmulq_f32(vcvt_f32_f16(vreinterpret_f16_u16(vld1_u16(pixel))), alpha);
                vst1_u16(pixel, vreinterpret_u16_f16(vcvt_f16_f32(color)));
            }
        }
    }
}
