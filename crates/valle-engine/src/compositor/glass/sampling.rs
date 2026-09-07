//! Canonical working-image sampling shared by the CPU reference paths.

use crate::compositor::reference::ReferenceImage;

/// Bilinear sampling with Skia's pixel-center convention: pixel `(x, y)` is centered at
/// `(x + 0.5, y + 0.5)`. Positions outside the image clamp to the border.
pub(crate) fn sample_bilinear(image: &ReferenceImage, x: f64, y: f64) -> [f64; 4] {
    let extent = image.extent();
    let width = f64::from(extent.width());
    let height = f64::from(extent.height());
    let px = (x - 0.5).clamp(0.0, width - 1.0);
    let py = (y - 0.5).clamp(0.0, height - 1.0);
    let x0 = px.floor() as u32;
    let y0 = py.floor() as u32;
    let x1 = (x0 + 1).min(extent.width() - 1);
    let y1 = (y0 + 1).min(extent.height() - 1);
    let fx = px - px.floor();
    let fy = py - py.floor();
    let stride = extent.width() as usize;
    let pixels = image.pixels();
    let a = pixels[y0 as usize * stride + x0 as usize].channels();
    let b = pixels[y0 as usize * stride + x1 as usize].channels();
    let c = pixels[y1 as usize * stride + x0 as usize].channels();
    let d = pixels[y1 as usize * stride + x1 as usize].channels();
    let mut output = [0.0; 4];
    for (index, channel) in output.iter_mut().enumerate() {
        let top = f64::from(a[index]) + f64::from(b[index] - a[index]) * fx;
        let bottom = f64::from(c[index]) + f64::from(d[index] - c[index]) * fx;
        *channel = top + (bottom - top) * fy;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::reference::PremulRgba32;
    use crate::resource::Extent2d;

    #[test]
    fn integer_pixel_centers_are_exact() {
        let image = ReferenceImage::new(
            Extent2d::new(2, 1).unwrap(),
            vec![
                PremulRgba32::from_straight([0.2, 0.2, 0.2], 1.0).unwrap(),
                PremulRgba32::from_straight([0.8, 0.8, 0.8], 1.0).unwrap(),
            ],
        )
        .unwrap();
        assert!((sample_bilinear(&image, 0.5, 0.5)[0] - 0.2).abs() < 1.0e-6);
        assert!((sample_bilinear(&image, 1.5, 0.5)[0] - 0.8).abs() < 1.0e-6);
        assert!((sample_bilinear(&image, 1.0, 0.5)[0] - 0.5).abs() < 1.0e-6);
    }
}
