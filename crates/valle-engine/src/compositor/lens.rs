//! Bounded lens distortion over premultiplied working-linear RGBA16F pixels.

use super::{
    bloom::{BloomError, encode_output, pixel_count, quantize_half, read_base},
    radial::bilinear_decal,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LensDistortionParams {
    pub k1: f32,
    pub k2: f32,
    /// The uncropped source frame in device coordinates: x, y, width, height.
    pub frame: [f32; 4],
}

pub fn apply_lens_distortion_f16(
    input: &[u8],
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    input_offset: [i32; 2],
    output_origin: [i32; 2],
    params: LensDistortionParams,
) -> Result<Vec<u8>, BloomError> {
    if !params.k1.is_finite()
        || !params.k2.is_finite()
        || params.k1.abs() > 0.5
        || params.k2.abs() > 0.5
        || !params
            .frame
            .iter()
            .all(|value| value.is_finite() && value.abs() <= 10_000_000.0)
        || params.frame[2] <= 0.0
        || params.frame[3] <= 0.0
    {
        return Err(BloomError::InvalidParameters);
    }
    if params.k1 == 0.0 && params.k2 == 0.0 {
        return Ok(encode_output(&read_base(
            input,
            input_width,
            input_height,
            output_width,
            output_height,
            input_offset,
        )?));
    }
    // Keep the complete source ROI: a cropped output pixel can sample beyond its own ROI.
    let source = read_base(
        input,
        input_width,
        input_height,
        input_width,
        input_height,
        [0, 0],
    )?;
    let output_count = pixel_count(output_width, output_height)?;
    let mut output = vec![[0.0_f32; 4]; output_count];
    let [left, top, frame_width, frame_height] = params.frame.map(f64::from);
    let cx = left + frame_width * 0.5;
    let cy = top + frame_height * 0.5;
    let radius_squared = (frame_width * frame_width + frame_height * frame_height) * 0.25;
    let input_origin_x = i64::from(output_origin[0]) + i64::from(input_offset[0]);
    let input_origin_y = i64::from(output_origin[1]) + i64::from(input_offset[1]);
    for y in 0..output_height as usize {
        for x in 0..output_width as usize {
            let device_x = f64::from(output_origin[0]) + x as f64 + 0.5;
            let device_y = f64::from(output_origin[1]) + y as f64 + 0.5;
            if device_x < left
                || device_x >= left + frame_width
                || device_y < top
                || device_y >= top + frame_height
            {
                continue;
            }
            let dx = device_x - cx;
            let dy = device_y - cy;
            let r2 = ((dx * dx + dy * dy) / radius_squared).min(1.0);
            let scale = 1.0 + f64::from(params.k1) * r2 + f64::from(params.k2) * r2 * r2;
            let sample_x = cx + dx * scale - input_origin_x as f64 - 0.5;
            let sample_y = cy + dy * scale - input_origin_y as f64 - 0.5;
            output[y * output_width as usize + x] = bilinear_decal(
                &source,
                input_width as usize,
                input_height as usize,
                sample_x,
                sample_y,
            )
            .map(quantize_half);
        }
    }
    Ok(encode_output(&output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::bloom::{f32_to_half, half_to_f32};

    #[test]
    fn lens_changes_edges_and_is_crop_stable() {
        let mut source = vec![0_u8; 64 * 64 * 8];
        for y in 16..48 {
            for x in 8..16 {
                let at = (y * 64 + x) * 8;
                for channel in 0..4 {
                    source[at + channel * 2..at + channel * 2 + 2]
                        .copy_from_slice(&f32_to_half(1.0).to_le_bytes());
                }
            }
        }
        let params = LensDistortionParams {
            k1: 0.4,
            k2: 0.0,
            frame: [0.0, 0.0, 64.0, 64.0],
        };
        let full =
            apply_lens_distortion_f16(&source, 64, 64, 64, 64, [0, 0], [0, 0], params).unwrap();
        assert_ne!(full, source);
        let cropped =
            apply_lens_distortion_f16(&source, 64, 64, 24, 24, [0, -20], [0, 20], params).unwrap();
        for y in 0..24 {
            for x in 0..24 {
                let full_at = ((y + 20) * 64 + x) * 8;
                let cropped_at = (y * 24 + x) * 8;
                assert_eq!(
                    &cropped[cropped_at..cropped_at + 8],
                    &full[full_at..full_at + 8]
                );
            }
        }
        let alpha = (32 * 64 + 10) * 8 + 6;
        assert!(half_to_f32(u16::from_le_bytes([full[alpha], full[alpha + 1]])) <= 1.0);
        assert_eq!(
            source,
            apply_lens_distortion_f16(
                &source,
                64,
                64,
                64,
                64,
                [0, 0],
                [0, 0],
                LensDistortionParams { k1: 0.0, ..params }
            )
            .unwrap()
        );
        assert!(
            apply_lens_distortion_f16(
                &source,
                64,
                64,
                64,
                64,
                [0, 0],
                [0, 0],
                LensDistortionParams { k1: 0.6, ..params }
            )
            .is_err()
        );
    }
}
