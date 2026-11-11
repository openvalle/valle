//! Display orientation of a decoded video, without changing its sampling or color space.

/// Clockwise quarter turns, followed by a horizontal reflection in display coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Orientation {
    pub turns: u32,
    pub flip: bool,
}

impl Orientation {
    pub fn size(self, [w, h]: [u32; 2]) -> [u32; 2] {
        if self.turns % 2 == 1 { [h, w] } else { [w, h] }
    }

    fn point(self, [x, y]: [u32; 2], [w, h]: [u32; 2]) -> [u32; 2] {
        let [x, y] = match self.turns % 4 {
            1 => [h - 1 - y, x],
            2 => [w - 1 - x, h - 1 - y],
            3 => [y, w - 1 - x],
            _ => [x, y],
        };
        [
            if self.flip {
                self.size([w, h])[0] - 1 - x
            } else {
                x
            },
            y,
        ]
    }

    /// RGBA fallback; YUV is oriented during its existing GPU conversion pass.
    pub fn rgba(self, bytes: Vec<u8>, size: [u32; 2]) -> Vec<u8> {
        if self == Self::default() {
            return bytes;
        }
        let [w, h] = size;
        let width = self.size(size)[0];
        let mut out = vec![0; bytes.len()];
        for y in 0..h {
            for x in 0..w {
                let [dx, dy] = self.point([x, y], size);
                let from = ((y * w + x) * 4) as usize;
                let to = ((dy * width + dx) * 4) as usize;
                out[to..to + 4].copy_from_slice(&bytes[from..from + 4]);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_display_orientations_preserve_pixels() {
        let pixels: Vec<u8> = (0..6).flat_map(|n| [n, n, n, 255]).collect();
        let expected = [
            [0, 1, 2, 3, 4, 5],
            [4, 2, 0, 5, 3, 1],
            [5, 4, 3, 2, 1, 0],
            [1, 3, 5, 0, 2, 4],
        ];
        for (turns, expected) in expected.into_iter().enumerate() {
            for flip in [false, true] {
                let o = Orientation {
                    turns: turns as u32,
                    flip,
                };
                let size = o.size([2, 3]);
                let mut expected = expected;
                if flip {
                    for row in expected.chunks_mut(size[0] as usize) {
                        row.reverse();
                    }
                }
                let actual = o.rgba(pixels.clone(), [2, 3]);
                assert_eq!(
                    actual
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|p| p[0])
                        .collect::<Vec<_>>(),
                    expected
                );
                assert!(actual.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
            }
        }
    }
}
