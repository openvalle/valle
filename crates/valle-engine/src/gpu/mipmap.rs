//! Image mipmaps on the GPU, with the same integer filtering as `Image::halved`.
//! A changing video uploads its base level once instead of rebuilding every level on the CPU.

use super::Gpu;
use crate::image::Image;

pub(super) struct Mipmap {
    pipeline: wgpu::ComputePipeline,
}

impl Mipmap {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mipmap"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/mipmap.wgsl").into()),
        });
        Self {
            pipeline: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("mipmap"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            }),
        }
    }

    pub(super) fn texture(&self, gpu: &Gpu, image: &Image) -> wgpu::Texture {
        let most = gpu.device.limits().max_texture_dimension_2d;
        // Only an image too large for this device needs reduction before upload.
        let mut image = image.clone();
        while image.size.iter().any(|&n| n > most) {
            image = image.halved();
        }
        let [width, height] = image.size;
        let levels = u32::BITS - width.max(height).leading_zeros();
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("image mipmap"),
            size,
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        if image.yuv.is_some() {
            gpu.video.convert(gpu, &mut encoder, &image, &texture);
        } else {
            gpu.queue.write_texture(
                texture.as_image_copy(),
                &image.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * width),
                    rows_per_image: Some(height),
                },
                size,
            );
        }
        if levels > 1 {
            let view = |level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            };
            let layout = self.pipeline.get_bind_group_layout(0);
            for level in 1..levels {
                let (source, target) = (view(level - 1), view(level));
                let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("mipmap"),
                    layout: &layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&source),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&target),
                        },
                    ],
                });
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(
                    (width >> level).max(1).div_ceil(8),
                    (height >> level).max(1).div_ceil(8),
                    1,
                );
            }
        }
        if levels > 1 || image.yuv.is_some() {
            gpu.queue.submit([encoder.finish()]);
        }
        texture
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_level_matches_cpu_for_odd_even_and_single_pixel_axes() {
        if !std::env::var("VALLE_TEST_LAYERS")
            .unwrap_or_default()
            .split(',')
            .any(|s| s == "gpu")
        {
            return;
        }
        let gpu = Gpu::new().expect("GPU");
        for size in [[1, 1], [1, 17], [18, 1], [3, 5], [16, 8], [319, 179]] {
            let pixels: Vec<u8> = (0..size[0] * size[1] * 4)
                .map(|n| ((n * 73 + n / 7) % 256) as u8)
                .collect();
            let image = Image::new(size, pixels.into());
            let texture = gpu.mipmap.texture(&gpu, &image);
            for (level, expected) in std::iter::once(image.clone())
                .chain(image.mipmap())
                .enumerate()
            {
                let [w, h] = expected.size;
                let stride = (w * 4).div_ceil(256) * 256;
                let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: u64::from(stride * h),
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        mip_level: level as u32,
                        ..texture.as_image_copy()
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(stride),
                            rows_per_image: Some(h),
                        },
                    },
                    wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                );
                gpu.queue.submit([encoder.finish()]);
                buffer.map_async(wgpu::MapMode::Read, .., |r| r.expect("mapped"));
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .expect("drawn");
                let mapped = buffer.get_mapped_range(..).expect("readable");
                let pixels: Vec<_> = mapped
                    .chunks(stride as usize)
                    .flat_map(|r| r[..w as usize * 4].iter().copied())
                    .collect();
                assert_eq!(pixels, *expected.pixels, "{size:?} level {level}");
                drop(mapped);
                buffer.unmap();
            }
        }
    }
}
