//! Skia's Perlin noise tables for a seed, as `SkPerlinNoiseShader::PaintingData` builds them, and
//! the image through which the noise displacement kernel reads them on every backend.

/// The lattice selector and stored gradients of Skia's Perlin noise for one seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerlinTables {
    /// The lattice selector, a permutation of the 256 lattice points.
    pub selector: [u8; 256],
    /// Per lattice point and channel (red, green, blue, alpha), the x and y components of the
    /// point's gradient as Skia stores them: `(component + 1) * 32767.5`, rounded.
    pub samples: [[[u16; 2]; 4]; 256],
}

/// The width and height of [`PerlinTables::image`].
pub const PERLIN_IMAGE_SIZE: [u32; 2] = [256, 5];

impl PerlinTables {
    /// The tables of `seed`, which Skia truncates to an integer and keeps in 1 to 2^31 - 2.
    pub fn new(seed: f32) -> Self {
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
        let mut samples = [[[0_u16; 2]; 4]; 256];
        for (channel, raw) in noise.iter().enumerate() {
            for (point, stored) in samples.iter_mut().enumerate() {
                let sample = raw[usize::from(selector[point])];
                let gradient =
                    normalize(sample.map(|value| (i32::from(value) - 256) as f32 * (1.0 / 256.0)));
                stored[channel] =
                    gradient.map(|value| round_to_int((value + 1.0) * 32_767.5) as u16);
            }
        }
        Self { selector, samples }
    }

    /// The tables as the opaque RGBA8 pixels of a [`PERLIN_IMAGE_SIZE`] image, one lattice point
    /// per column, in the red, green and blue bytes. Rows 0 to 3 hold the point's red, green and
    /// alpha gradients, each its x and then its y sample, high byte first; row 4 holds the
    /// point's selector in red. Blue noise has no place: the kernel reads red and green over
    /// alpha.
    pub fn image(&self) -> Vec<u8> {
        let [width, height] = PERLIN_IMAGE_SIZE.map(|size| size as usize);
        let mut pixels = vec![0_u8; width * height * 4];
        for point in 0..width {
            let [red, green, _, alpha] = self.samples[point];
            let bytes = [red, green, alpha]
                .into_iter()
                .flatten()
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<_>>();
            for (row, rgb) in bytes.as_chunks::<3>().0.iter().enumerate() {
                let at = (row * width + point) * 4;
                pixels[at..at + 3].copy_from_slice(rgb);
                pixels[at + 3] = 255;
            }
            let at = (4 * width + point) * 4;
            pixels[at] = self.selector[point];
            pixels[at + 3] = 255;
        }
        pixels
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

/// `SkScalarRoundToInt`: the floor of the value plus one half in double precision, as a float,
/// saturated to the ints a float holds.
fn round_to_int(value: f32) -> i32 {
    let limit = 2_147_483_520.0_f32;
    let rounded = (f64::from(value) + 0.5).floor() as f32;
    if rounded.is_nan() {
        return limit as i32;
    }
    rounded.clamp(-limit, limit) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_hold_a_permutation_and_unit_gradients_in_their_image() {
        for seed in [0.0, 1.0, 7.0, 4_000_000.0, -3.5, 1e12, f32::NAN] {
            let tables = PerlinTables::new(seed);
            let mut seen = [false; 256];
            for point in tables.selector {
                seen[usize::from(point)] = true;
            }
            assert!(seen.iter().all(|seen| *seen), "seed {seed}");
            for samples in &tables.samples {
                for [x, y] in samples {
                    let unit = [x, y].map(|value| f64::from(*value) * 2.0 / 65_535.0 - 1.0);
                    // A raw sample at the lattice centre has no direction and stays zero.
                    let length = (unit[0] * unit[0] + unit[1] * unit[1]).sqrt();
                    assert!(
                        (length - 1.0).abs() < 1e-4 || length < 1e-4,
                        "seed {seed}: {length}"
                    );
                }
            }
            let image = tables.image();
            let [width, _] = PERLIN_IMAGE_SIZE.map(|size| size as usize);
            let byte = |row: usize, point: usize, channel: usize| {
                image[(row * width + point) * 4 + channel]
            };
            for point in 0..width {
                let rgb = (0..4)
                    .flat_map(|row| (0..3).map(move |channel| (row, channel)))
                    .map(|(row, channel)| byte(row, point, channel))
                    .collect::<Vec<_>>();
                let stored = rgb
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .collect::<Vec<_>>();
                let [red, green, _, alpha] = tables.samples[point];
                assert_eq!(stored, [red, green, alpha].concat());
                assert_eq!(byte(4, point, 0), tables.selector[point]);
                assert!((0..5).all(|row| byte(row, point, 3) == 255));
            }
        }
    }
}
