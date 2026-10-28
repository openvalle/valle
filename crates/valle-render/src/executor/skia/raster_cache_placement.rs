use valle_engine::prepare::DeviceRect;

/// A scheduled raster pass translates its canvas by -ROI before concatenating this matrix.
/// Bounded integer translation cancels exactly in Skia f32 arithmetic. Keep all other matrix
/// bits, the cropped dimensions, and fractional/projective placements unchanged. This helper
/// is only for destination-independent RasterNode/RasterTree inputs, never filter passes.
pub(super) fn local_placement(
    mut matrix: [f64; 9],
    roi: DeviceRect,
) -> ([u64; 9], (i32, i32, u32, u32)) {
    const MAX_COORD: f64 = 1_048_576.;
    let integer = |v: f64| v.is_finite() && v.abs() <= MAX_COORD && v.fract() == 0.;
    let can_translate = matrix[6] == 0.
        && matrix[7] == 0.
        && matrix[8] == 1.
        && integer(matrix[2])
        && integer(matrix[5])
        && integer(f64::from(roi.x))
        && integer(f64::from(roi.y));
    let origin = if can_translate {
        matrix[2] -= f64::from(roi.x);
        matrix[5] -= f64::from(roi.y);
        (0, 0)
    } else {
        (roi.x, roi.y)
    };
    (
        matrix.map(f64::to_bits),
        (origin.0, origin.1, roi.width, roi.height),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use skia_safe::{Matrix, scalar};

    #[test]
    fn normalized_integer_placements_have_the_same_actual_skia_matrix() {
        let mut expected = None;
        for delta in [-1_000_000, -512, -1, 0, 17, 999_999] {
            let matrix = [
                1.25,
                0.125,
                30. + delta as f64,
                -0.25,
                0.75,
                44. - delta as f64,
                0.,
                0.,
                1.,
            ];
            let roi = DeviceRect {
                x: 20 + delta,
                y: 36 - delta,
                width: 128,
                height: 96,
            };
            let key = local_placement(matrix, roi);
            let mut actual = Matrix::translate((-roi.x as f32, -roi.y as f32));
            actual.pre_concat(&Matrix::new_all(
                matrix[0] as scalar,
                matrix[1] as scalar,
                matrix[2] as scalar,
                matrix[3] as scalar,
                matrix[4] as scalar,
                matrix[5] as scalar,
                0.,
                0.,
                1.,
            ));
            let normalized = key.0.map(|bits| f64::from_bits(bits) as f32);
            let mut actual_values = [0.; 9];
            actual.get_9(&mut actual_values);
            assert_eq!(actual_values, normalized);
            if let Some(expected) = expected {
                assert_eq!(key, expected);
            } else {
                expected = Some(key);
            }
        }
    }

    #[test]
    fn phase_crop_and_unsafe_transforms_remain_distinct() {
        let roi = DeviceRect {
            x: 20,
            y: 36,
            width: 128,
            height: 96,
        };
        let matrix = [1., 0., 30., 0., 1., 44., 0., 0., 1.];
        let key = local_placement(matrix, roi);
        let mut cropped = roi;
        cropped.width -= 1;
        assert_ne!(key, local_placement(matrix, cropped));
        cropped = roi;
        cropped.x += 1;
        assert_ne!(key, local_placement(matrix, cropped));
        for (index, value) in [
            (2, 30.5),
            (5, 44.125),
            (2, 2_000_000.),
            (6, 0.001),
            (8, 0.5),
        ] {
            let mut changed = matrix;
            changed[index] = value;
            let raw = local_placement(changed, roi);
            assert_eq!(raw.0, changed.map(f64::to_bits));
            assert_eq!(raw.1, (roi.x, roi.y, roi.width, roi.height));
            assert_ne!(key, raw);
        }
    }

    #[test]
    fn translated_local_raster_preserves_f16_coverage_and_color() {
        use skia_safe::{AlphaType, Color4f, ColorType, ImageInfo, Paint, Rect};
        let info = ImageInfo::new(
            (96, 80),
            ColorType::RGBAF16,
            AlphaType::Premul,
            Some(super::super::surface::working_color_space().unwrap()),
        );
        let mut expected = None;
        for (dx, dy) in [(0, 0), (45, -50), (-511, 129), (999_000, -999_000)] {
            let mut surface = skia_safe::surfaces::raster(&info, None, None).unwrap();
            let canvas = surface.canvas();
            canvas.clear(Color4f::new(0., 0., 0., 0.));
            canvas.translate((-(dx + 20) as f32, -(dy + 30) as f32));
            canvas.concat(&Matrix::new_all(
                1.25,
                0.125,
                (dx + 24) as f32,
                -0.25,
                0.75,
                (dy + 38) as f32,
                0.,
                0.,
                1.,
            ));
            let mut paint = Paint::new(Color4f::new(1.5, 0.003, 0.2, 0.6), None);
            paint.set_anti_alias(true);
            canvas.draw_oval(Rect::from_xywh(0.25, 3.5, 42.125, 53.75), &paint);
            paint.set_color4f(Color4f::new(0.01, 0.3, 0.9, 0.25), None);
            canvas.draw_rect(Rect::from_xywh(3.125, 17.75, 59.25, 25.5), &paint);
            let mut pixels = vec![0; 96 * 80 * 8];
            assert!(surface.read_pixels(&info, &mut pixels, 96 * 8, (0, 0)));
            if let Some(expected) = &expected {
                assert_eq!(&pixels, expected);
            } else {
                assert!(pixels.iter().any(|&v| v != 0));
                expected = Some(pixels);
            }
        }
    }
}
