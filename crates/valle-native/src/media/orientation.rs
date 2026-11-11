//! Display matrices orient coded pixels before layout sees their dimensions. Translation is
//! ignored: the oriented frame's top-left is the origin. Phone rotations and reflections need
//! only a permutation of pixels, without resampling.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orientation([i32; 4]);

impl Orientation {
    pub const IDENTITY: Self = Self([1, 0, 0, 1]);

    pub fn matrix(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 36 {
            return Err("a video's display matrix is truncated".into());
        }
        let word = |i: usize| i32::from_ne_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap());
        let values = [word(0), word(3), word(1), word(4)];
        let mut axes = [0; 4];
        for (row, out) in values
            .as_chunks::<2>()
            .0
            .iter()
            .zip(axes.as_chunks_mut::<2>().0)
        {
            let scale = row.iter().map(|n| f64::from(*n).abs()).fold(0.0, f64::max);
            if scale == 0.0 {
                return Err("a video's display matrix has no scale".into());
            }
            for (n, axis) in row.iter().zip(out) {
                let v = f64::from(*n) / scale;
                if (v - v.round()).abs() > 0.001 {
                    return Err("video display rotation must be a multiple of 90 degrees".into());
                }
                *axis = v.round() as i32;
            }
        }
        let [a, c, b, d] = axes;
        if a.abs() + c.abs() != 1 || b.abs() + d.abs() != 1 || a * b + c * d != 0 {
            return Err("a video's display matrix is not a rotation or reflection".into());
        }
        Ok(Self(axes))
    }

    pub fn size(self, [w, h]: [u32; 2]) -> [u32; 2] {
        if self.0[0] == 0 { [h, w] } else { [w, h] }
    }

    /// Copy a strided RGBA frame into packed rows in display order.
    pub fn rgba(self, data: &[u8], stride: usize, size: [u32; 2]) -> Vec<u8> {
        let [w, h] = size.map(|n| n as usize);
        if self == Self::IDENTITY {
            let mut out = Vec::with_capacity(w * h * 4);
            for row in data.chunks(stride).take(h) {
                out.extend_from_slice(&row[..w * 4]);
            }
            return out;
        }
        let [ow, oh] = self.size(size).map(|n| n as i32);
        let [a, c, b, d] = self.0;
        let mut out = vec![0; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let dx = a * x as i32 + c * y as i32 + if a + c < 0 { ow - 1 } else { 0 };
                let dy = b * x as i32 + d * y as i32 + if b + d < 0 { oh - 1 } else { 0 };
                let from = y * stride + x * 4;
                let to = (dy * ow + dx) as usize * 4;
                out[to..to + 4].copy_from_slice(&data[from..from + 4]);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix(values: [i32; 9]) -> Vec<u8> {
        values.into_iter().flat_map(i32::to_ne_bytes).collect()
    }

    #[test]
    fn display_matrix_rotates_strided_rows_without_padding() {
        let bytes = matrix([0, -65536, 0, 65536, 0, 0, 0, 0, 1 << 30]);
        let orientation = Orientation::matrix(&bytes).unwrap();
        let pixels = [
            1, 1, 1, 255, 2, 2, 2, 255, 99, 99, 99, 99, 3, 3, 3, 255, 4, 4, 4, 255,
        ];
        let rgba = orientation.rgba(&pixels, 12, [2, 2]);
        assert_eq!(
            rgba.as_chunks::<4>()
                .0
                .iter()
                .map(|p| p[0])
                .collect::<Vec<_>>(),
            [2, 4, 1, 3]
        );
        assert_eq!(orientation.size([2, 3]), [3, 2]);
        assert_eq!(Orientation::IDENTITY.rgba(&pixels, 12, [2, 2]).len(), 16);
    }

    #[test]
    fn malformed_and_non_quarter_turn_matrices_are_not_silently_ignored() {
        assert!(Orientation::matrix(&[0; 35]).is_err());
        assert!(Orientation::matrix(&[0; 36]).is_err());
        assert!(
            Orientation::matrix(&matrix([46341, -46341, 0, 46341, 46341, 0, 0, 0, 1 << 30]))
                .is_err()
        );
    }
}
