//! Bounded WebGL2 upload packet. Contains triangles and material records, never CPU coverage.
use super::{GpuRasterAtlas, GpuRasterGeometry};
use crate::scene3d::Frame3DState;

const HEADER_WORDS: usize = 64;
const TILE: u32 = 16;
pub const WEB_GPU_PACKET_MAGIC: u32 = 0x33534756;

fn texture(output: &mut Vec<u8>, data: &[u8]) -> Option<[u32; 3]> {
    let offset = u32::try_from(output.len()).ok()?;
    let texels = data.len().div_ceil(16).max(1);
    let width = texels.min(4096);
    let height = texels.div_ceil(width);
    if height > 4096 {
        return None;
    }
    output.extend_from_slice(data);
    output.resize(offset as usize + width * height * 16, 0);
    Some([width as u32, height as u32, offset])
}

impl GpuRasterGeometry {
    /// The immutable atlas is fetched separately and retained by content key. Repeated frames
    /// transfer only geometry, uniforms and tile references, including animated model frames.
    pub fn web_packet(&self, frame: &Frame3DState) -> Option<Vec<u8>> {
        let shading = self.shading.as_ref();
        let atlas = match shading {
            Some(s) => Some(GpuRasterAtlas::new(s, 4096, 256 * 1024 * 1024)?),
            None => None,
        };
        let upload_limit = u64::from(self.width) * u64::from(self.height) * 12;
        let initial_bytes = HEADER_WORDS as u64 * 4
            + self.vertices.len() as u64 * self.vertex_stride()
            + shading.map_or(0, |s| {
                s.values.len() as u64 * 16
                    + s.shadows
                        .iter()
                        .map(|s| s.vertices.len() as u64 * 28)
                        .sum::<u64>()
            });
        if initial_bytes > upload_limit {
            return None;
        }
        let blend = shading.is_some_and(|s| {
            (0..s.values[1][0] as usize).any(|i| {
                s.values[super::GpuRasterShading::HEADER_LEN
                    + i * super::GpuRasterShading::MATERIAL_STRIDE
                    + 2][3]
                    != 0.0
            })
        });
        let dof = frame
            .camera
            .depth_of_field
            .filter(|d| d.max_blur_radius > 0.0);
        let mut header = [0u32; HEADER_WORDS];
        header[..9].copy_from_slice(&[
            WEB_GPU_PACKET_MAGIC,
            self.width,
            self.height,
            u32::try_from(self.vertices.len()).ok()?,
            (self.vertex_stride() / 4) as u32,
            u32::from(shading.is_some())
                | (u32::from(blend) << 1)
                | (u32::from(dof.is_some()) << 2)
                | (u32::from(shading.is_some_and(|s| s.values[1][3] as u32 & 2 != 0)) << 3),
            self.depth_scale.to_bits(),
            dof.map_or(1.0, |d| d.focus_distance).to_bits(),
            dof.map_or(0.0, |d| d.max_blur_radius).to_bits(),
        ]);
        let bounds = if self.vertices.is_empty() {
            [0.0; 4]
        } else {
            [
                self.vertices
                    .iter()
                    .map(|v| v.position[0])
                    .fold(f32::INFINITY, f32::min),
                self.vertices
                    .iter()
                    .map(|v| v.position[1])
                    .fold(f32::INFINITY, f32::min),
                self.vertices
                    .iter()
                    .map(|v| v.position[0])
                    .fold(f32::NEG_INFINITY, f32::max),
                self.vertices
                    .iter()
                    .map(|v| v.position[1])
                    .fold(f32::NEG_INFINITY, f32::max),
            ]
        };
        for (word, bound) in header[44..48].iter_mut().zip(bounds) {
            *word = bound.to_bits();
        }
        let mut vertices = self.vertex_bytes();
        if shading.is_none() {
            for (encoded, triangle) in vertices
                .as_chunks_mut::<108>()
                .0
                .iter_mut()
                .zip(self.vertices.as_chunks::<3>().0)
            {
                let point = |i: usize| triangle[i].position.map(|v| (v * 256.0) as i64);
                let [a, b, c] = [0, 1, 2].map(point);
                let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
                for at in [28, 64, 100] {
                    encoded[at..at + 4].copy_from_slice(&(1.0 / area as f32).to_le_bytes());
                }
            }
        }
        let materials = shading.map_or_else(Vec::new, |s| s.bytes());
        let mut output = vec![0; HEADER_WORDS * 4];
        header[9..12].copy_from_slice(&texture(&mut output, &vertices)?);
        header[12..15].copy_from_slice(&texture(&mut output, &materials)?);
        if let Some(atlas) = &atlas {
            header[15] = atlas.width;
            header[16] = atlas.height;
            header[17] = u32::try_from(atlas.byte_len()).ok()?;
            for (word, value) in header[28..36].iter_mut().zip(atlas.key.as_chunks::<4>().0) {
                *word = u32::from_le_bytes(*value);
            }
        }

        let tiles_x = self.width.div_ceil(TILE);
        let tiles_y = self.height.div_ceil(TILE);
        let mut tiles = if blend {
            vec![Vec::<u32>::new(); (tiles_x * tiles_y) as usize]
        } else {
            Vec::new()
        };
        let mut reference_bytes = 0u64;
        if let Some(s) = shading.filter(|_| blend) {
            for (index, triangle) in self.vertices.as_chunks::<3>().0.iter().enumerate() {
                let material = s.vertices[index * 3].material_index as usize;
                if s.values[super::GpuRasterShading::HEADER_LEN
                    + material * super::GpuRasterShading::MATERIAL_STRIDE
                    + 2][3]
                    == 0.0
                {
                    continue;
                }
                let bound = |axis: usize, maximum: bool| {
                    let v = triangle.iter().map(|v| v.position[axis]);
                    if maximum {
                        v.fold(f32::NEG_INFINITY, f32::max).ceil()
                    } else {
                        v.fold(f32::INFINITY, f32::min).floor()
                    }
                };
                let x0 = (bound(0, false).max(0.0) as u32).min(self.width);
                let y0 = (bound(1, false).max(0.0) as u32).min(self.height);
                let x1 = (bound(0, true).max(0.0) as u32).min(self.width);
                let y1 = (bound(1, true).max(0.0) as u32).min(self.height);
                for y in y0 / TILE..y1.div_ceil(TILE) {
                    for x in x0 / TILE..x1.div_ceil(TILE) {
                        reference_bytes += 4;
                        if initial_bytes + tiles.len() as u64 * 16 + reference_bytes > upload_limit
                        {
                            return None;
                        }
                        tiles[(y * tiles_x + x) as usize].push(index as u32);
                    }
                }
            }
        }
        let mut offsets = Vec::new();
        let mut references = Vec::new();
        for tile in &tiles {
            let start = u32::try_from(references.len() / 4).ok()?;
            for value in [start, tile.len() as u32, 0, 0] {
                offsets.extend_from_slice(&value.to_le_bytes());
            }
            for value in tile {
                references.extend_from_slice(&value.to_le_bytes());
            }
        }
        header[18..21].copy_from_slice(&texture(&mut output, &references)?);
        header[21..24].copy_from_slice(&texture(&mut output, &offsets)?);
        header[27] = tiles.len() as u32;
        if let Some(s) = shading {
            header[24] = s.shadows.len() as u32;
            for (layer, shadow) in s.shadows.iter().enumerate() {
                if layer >= 2 {
                    return None;
                }
                let encoded = shadow.vertex_bytes();
                let [width, height, offset] = texture(&mut output, &encoded)?;
                header[48 + layer * 8..56 + layer * 8].copy_from_slice(&[
                    shadow.vertices.len() as u32,
                    width,
                    height,
                    offset,
                    shadow.values[3][2].to_bits(),
                    shadow.values[3][3].to_bits(),
                    shadow.light,
                    0,
                ]);
            }
        }
        let shadow_key = shading.map_or_else(
            || crate::ContentDigest::of_bytes(&[]),
            |s| s.shadow_content_key(),
        );
        for (word, value) in header[36..44]
            .iter_mut()
            .zip(shadow_key.as_bytes().as_chunks::<4>().0)
        {
            *word = u32::from_le_bytes(*value);
        }
        // Match Native's bounded geometry packet admission. Dense tiny layers retain CPU fallback.
        if output.len() as u64 > upload_limit {
            return None;
        }
        header[25] = u32::try_from(output.len()).ok()?;
        for (destination, word) in output[..HEADER_WORDS * 4]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(header)
        {
            *destination = word.to_le_bytes();
        }
        Some(output)
    }
}
