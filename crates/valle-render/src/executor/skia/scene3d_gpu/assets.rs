//! One immutable float32 atlas for a scene's admitted mip chains and environment cubes.
use sha2::{Digest, Sha256};
use std::sync::Arc;
use valle_engine::{
    motion::{
        memory,
        scene3d::{GpuRasterShading, TextureRole},
    },
    resource::ContentDigest,
};

pub(super) struct Atlas {
    pub key: ContentDigest,
    pub texture: wgpu::Texture,
    pub bytes: u64,
    _memory: Option<memory::MemoryLease>,
}

enum Pixels<'a> {
    Rgba(&'a [f32]),
    Rgb(&'a [[f32; 3]]),
}
struct Level<'a> {
    width: u32,
    height: u32,
    pixels: Pixels<'a>,
}

impl Atlas {
    pub fn key(shading: &GpuRasterShading) -> ContentDigest {
        let mut hash = Sha256::new();
        for image in &shading.textures {
            hash.update(image.content_digest().as_bytes());
            hash.update([match image.role() {
                TextureRole::Color => 0,
                TextureRole::Data => 1,
            }]);
        }
        if let Some(env) = &shading.environment {
            hash.update(env.content_digest().as_bytes());
        }
        ContentDigest::from_bytes(hash.finalize().into())
    }

    pub fn build(
        shading: &GpuRasterShading,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        limit: usize,
    ) -> Result<Option<Arc<Self>>, String> {
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
        if let Some(env) = &shading.environment {
            let mut cubes = env.gpu_cubes();
            let (size, pixels) = cubes.next().ok_or("missing diffuse cube")?;
            images.push(vec![Level {
                width: size,
                height: size * 6,
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
        let headers = images.len() + images.iter().map(Vec::len).sum::<usize>();
        let texels = headers
            + images
                .iter()
                .flatten()
                .map(|l| l.width as usize * l.height as usize)
                .sum::<usize>();
        let width = texels.clamp(1, 4096) as u32;
        let Ok(texels) = u32::try_from(texels.max(1)) else {
            return Ok(None);
        };
        let height = texels.div_ceil(width);
        if height > device.limits().max_texture_dimension_2d {
            return Ok(None);
        }
        let bytes = u64::from(width) * u64::from(height) * 16;
        if bytes > limit as u64 {
            return Ok(None);
        }
        let Ok(memory) = memory::reserve_retained(memory::MemoryKind::Resource, bytes) else {
            return Ok(None);
        };
        let Ok(_staging) = memory::reserve_retained(memory::MemoryKind::Temporary, bytes) else {
            return Ok(None);
        };
        let split = |v: usize| [(v & 65535) as f32, (v >> 16) as f32];
        let mut metadata = vec![[0.0_f32; 4]; headers];
        let mut level_at = images.len();
        let mut pixels_at = headers;
        for (image, levels) in images.iter().enumerate() {
            let [lo, hi] = split(level_at);
            metadata[image] = [lo, hi, levels.len() as f32, 0.0];
            for level in levels {
                let [lo, hi] = split(pixels_at);
                metadata[level_at] = [lo, hi, level.width as f32, level.height as f32];
                level_at += 1;
                pixels_at += level.width as usize * level.height as usize;
            }
        }
        let mut data = Vec::with_capacity(bytes as usize);
        for value in metadata.into_iter().flatten() {
            data.extend_from_slice(&value.to_le_bytes());
        }
        for level in images.iter().flatten() {
            match &level.pixels {
                Pixels::Rgba(values) => {
                    for value in *values {
                        data.extend_from_slice(&value.to_le_bytes());
                    }
                }
                Pixels::Rgb(values) => {
                    for pixel in *values {
                        for value in pixel.iter().copied().chain([1.0]) {
                            data.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                }
            }
        }
        data.resize(bytes as usize, 0);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Scene3D immutable linear mip atlas"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 16),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Ok(Some(Arc::new(Self {
            key: Self::key(shading),
            texture,
            bytes,
            _memory: memory,
        })))
    }
}
