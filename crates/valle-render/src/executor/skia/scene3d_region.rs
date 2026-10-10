//! Compact Scene3D staging buffers; coordinates remain in the original layer.
use skia_safe::Data;
use std::io::{self, Read};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Region {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl Region {
    pub fn full(width: usize, height: usize) -> Self {
        Self {
            x0: 0,
            y0: 0,
            x1: width,
            y1: height,
        }
    }

    pub fn rgba(pixels: &[u16], width: usize) -> Option<Self> {
        let mut bounds: Option<Self> = None;
        for (y, row) in pixels.chunks_exact(width * 4).enumerate() {
            let mut covered = row
                .chunks_exact(4)
                .enumerate()
                .filter(|(_, p)| p.iter().any(|&channel| channel != 0));
            if let Some((first, _)) = covered.next() {
                let last = covered.next_back().map_or(first, |(x, _)| x);
                bounds = Some(match bounds {
                    Some(b) => Self {
                        x0: b.x0.min(first),
                        x1: b.x1.max(last + 1),
                        y1: y + 1,
                        ..b
                    },
                    None => Self {
                        x0: first,
                        x1: last + 1,
                        y0: y,
                        y1: y + 1,
                    },
                });
            }
        }
        bounds
    }

    pub fn expand(self, margin: usize, width: usize, height: usize) -> Self {
        Self {
            x0: self.x0.saturating_sub(margin),
            y0: self.y0.saturating_sub(margin),
            x1: self.x1.saturating_add(margin).min(width),
            y1: self.y1.saturating_add(margin).min(height),
        }
    }

    pub fn width(self) -> usize {
        self.x1 - self.x0
    }
    pub fn height(self) -> usize {
        self.y1 - self.y0
    }
    pub fn pixels(self) -> usize {
        self.width() * self.height()
    }

    pub fn color_data(self, pixels: &[u16], width: usize) -> Data {
        #[cfg(target_endian = "little")]
        {
            // SAFETY: initialized u16 values can be read as bytes. The stream copies
            // into Skia-owned storage before returning; no borrowed pixels escape.
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    pixels.as_ptr().cast::<u8>(),
                    std::mem::size_of_val(pixels),
                )
            };
            self.copy_bytes(bytes, width, 8)
        }
        #[cfg(target_endian = "big")]
        {
            let bytes: Vec<_> = pixels.iter().flat_map(|v| v.to_le_bytes()).collect();
            self.copy_bytes(&bytes, width, 8)
        }
    }

    pub fn depth_data(self, depth: &[f32], width: usize) -> Data {
        #[cfg(target_endian = "little")]
        {
            // SAFETY: initialized f32 values have four readable bytes. Skia owns
            // an independent copy, including the exact clear-depth sentinel bits.
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    depth.as_ptr().cast::<u8>(),
                    std::mem::size_of_val(depth),
                )
            };
            self.copy_bytes(bytes, width, 4)
        }
        #[cfg(target_endian = "big")]
        {
            let bytes: Vec<_> = depth.iter().flat_map(|v| v.to_le_bytes()).collect();
            self.copy_bytes(&bytes, width, 4)
        }
    }

    fn copy_bytes(self, bytes: &[u8], width: usize, pixel_bytes: usize) -> Data {
        let stride = width * pixel_bytes;
        if self.x0 == 0 && self.x1 == width {
            return Data::new_copy(&bytes[self.y0 * stride..self.y1 * stride]);
        }
        let rows = RegionRows {
            bytes,
            stride,
            row_bytes: self.width() * pixel_bytes,
            offset: self.y0 * stride + self.x0 * pixel_bytes,
            in_row: 0,
            rows_left: self.height(),
        };
        // Stream rows directly into Skia's allocation, without a second compact Vec.
        Data::from_stream(rows, self.pixels() * pixel_bytes).expect("validated Scene3D region")
    }
}

struct RegionRows<'a> {
    bytes: &'a [u8],
    stride: usize,
    row_bytes: usize,
    offset: usize,
    in_row: usize,
    rows_left: usize,
}

impl Read for RegionRows<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let mut written = 0;
        while written < output.len() && self.rows_left != 0 {
            let count = (self.row_bytes - self.in_row).min(output.len() - written);
            let start = self.offset + self.in_row;
            output[written..written + count].copy_from_slice(&self.bytes[start..start + count]);
            written += count;
            self.in_row += count;
            if self.in_row == self.row_bytes {
                self.offset += self.stride;
                self.rows_left -= 1;
                self.in_row = 0;
            }
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_staging_preserves_rows_bits_and_ownership() {
        let mut color = (0..7 * 5 * 4).map(|i| i as u16).collect::<Vec<_>>();
        let mut depth = (0..7 * 5)
            .map(|i| f32::from_bits(0x3f800000 + i))
            .collect::<Vec<_>>();
        for region in [
            Region::full(7, 5),
            Region {
                x0: 2,
                y0: 1,
                x1: 6,
                y1: 4,
            },
            Region {
                x0: 6,
                y0: 4,
                x1: 7,
                y1: 5,
            },
        ] {
            let expected_color: Vec<_> = (region.y0..region.y1)
                .flat_map(|y| {
                    color[(y * 7 + region.x0) * 4..(y * 7 + region.x1) * 4]
                        .iter()
                        .flat_map(|v| v.to_le_bytes())
                })
                .collect();
            let expected_depth: Vec<_> = (region.y0..region.y1)
                .flat_map(|y| {
                    depth[y * 7 + region.x0..y * 7 + region.x1]
                        .iter()
                        .flat_map(|v| v.to_le_bytes())
                })
                .collect();
            assert_eq!(region.color_data(&color, 7).as_bytes(), expected_color);
            assert_eq!(region.depth_data(&depth, 7).as_bytes(), expected_depth);
        }
        let region = Region {
            x0: 2,
            y0: 1,
            x1: 6,
            y1: 4,
        };
        let color_copy = region.color_data(&color, 7);
        let depth_copy = region.depth_data(&depth, 7);
        color.fill(0);
        depth.fill(0.0);
        assert!(color_copy.as_bytes().iter().any(|&v| v != 0));
        assert!(depth_copy.as_bytes().iter().any(|&v| v != 0));
    }

    #[test]
    fn region_stream_supports_partial_row_reads() {
        let bytes = (0..24).collect::<Vec<u8>>();
        for chunk in 1..=12 {
            let mut stream = RegionRows {
                bytes: &bytes,
                stride: 8,
                row_bytes: 3,
                offset: 2,
                in_row: 0,
                rows_left: 3,
            };
            let mut actual = Vec::new();
            let mut buffer = vec![0; chunk];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                actual.extend_from_slice(&buffer[..count]);
            }
            assert_eq!(actual, [2, 3, 4, 10, 11, 12, 18, 19, 20]);
        }
    }
}
