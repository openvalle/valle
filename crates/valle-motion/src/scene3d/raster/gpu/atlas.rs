//! Shared CPU layout of immutable linear mip texels; GPU APIs own allocation and upload.
use super::GpuRasterShading;
use crate::scene3d::TextureRole;
use sha2::{Digest, Sha256};

enum Pixels<'a> {
    Rgba(&'a [f32]),
    Rgb(&'a [[f32; 3]]),
}
struct Level<'a> {
    width: u32,
    height: u32,
    pixels: Pixels<'a>,
}

pub struct GpuRasterAtlas<'a> {
    pub width: u32,
    pub height: u32,
    pub key: [u8; 32],
    images: Vec<Vec<Level<'a>>>,
    headers: usize,
}

impl<'a> GpuRasterAtlas<'a> {
    pub fn key(shading: &GpuRasterShading) -> [u8; 32] {
        let mut hash = Sha256::new();
        for image in &shading.textures {
            hash.update(image.content_digest().as_bytes());
            hash.update([match image.role() {
                TextureRole::Color => 0,
                TextureRole::Data => 1,
            }]);
        }
        if let Some(environment) = &shading.environment {
            hash.update(environment.content_digest().as_bytes());
        }
        hash.finalize().into()
    }

    pub fn new(shading: &'a GpuRasterShading, max_edge: u32, max_bytes: u64) -> Option<Self> {
        if max_edge == 0 {
            return None;
        }
        let mut images = shading
            .textures
            .iter()
            .map(|image| {
                image
                    .gpu_mips()
                    .map(|([width, height], pixels)| Level {
                        width,
                        height,
                        pixels: Pixels::Rgba(pixels),
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        if let Some(environment) = &shading.environment {
            let mut cubes = environment.gpu_cubes();
            let (size, pixels) = cubes.next()?;
            images.push(vec![Level {
                width: size,
                height: size.checked_mul(6)?,
                pixels: Pixels::Rgb(pixels),
            }]);
            images.push(
                cubes
                    .map(|(size, pixels)| Level {
                        width: size,
                        height: size * 6,
                        pixels: Pixels::Rgb(pixels),
                    })
                    .collect(),
            );
        }
        let headers = images
            .len()
            .checked_add(images.iter().map(Vec::len).sum::<usize>())?;
        let texels = images
            .iter()
            .flatten()
            .try_fold(headers as u64, |n, l| {
                n.checked_add(u64::from(l.width) * u64::from(l.height))
            })?
            .max(1);
        let width = texels.min(u64::from(max_edge.min(4096))) as u32;
        let height = u32::try_from(texels.div_ceil(u64::from(width))).ok()?;
        let bytes = u64::from(width) * u64::from(height) * 16;
        if height > max_edge || bytes > max_bytes {
            return None;
        }
        Some(Self {
            width,
            height,
            key: Self::key(shading),
            images,
            headers,
        })
    }

    pub fn byte_len(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * 16
    }

    /// Append one padded RGBA32F texture, after the caller has reserved its upload storage.
    pub fn append_bytes(&self, output: &mut Vec<u8>) {
        let start = output.len();
        let split = |value: usize| [(value & 65535) as f32, (value >> 16) as f32];
        let mut metadata = vec![[0.0; 4]; self.headers];
        let mut level_at = self.images.len();
        let mut pixels_at = self.headers;
        for (image, levels) in self.images.iter().enumerate() {
            let [lo, hi] = split(level_at);
            metadata[image] = [lo, hi, levels.len() as f32, 0.0];
            for level in levels {
                let [lo, hi] = split(pixels_at);
                metadata[level_at] = [lo, hi, level.width as f32, level.height as f32];
                level_at += 1;
                pixels_at += level.width as usize * level.height as usize;
            }
        }
        for value in metadata.into_iter().flatten() {
            output.extend_from_slice(&value.to_le_bytes());
        }
        for level in self.images.iter().flatten() {
            match &level.pixels {
                Pixels::Rgba(values) => {
                    for value in *values {
                        output.extend_from_slice(&value.to_le_bytes());
                    }
                }
                Pixels::Rgb(values) => {
                    for pixel in *values {
                        for value in pixel.iter().copied().chain([1.0]) {
                            output.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                }
            }
        }
        output.resize(start + self.byte_len() as usize, 0);
    }
}
