//! Decoded YUV planes into an image's base mip, using the CPU conversion's matrix, range,
//! centered bilinear chroma and byte rounding. No RGBA frame is allocated or read back on CPU.

use wgpu::util::DeviceExt;

use super::{Gpu, bytes};
use crate::image::{Image, yuv::Format};

pub(super) struct Video {
    pipeline: wgpu::ComputePipeline,
}

impl Video {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("video YUV"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/video.wgsl").into()),
        });
        Self {
            pipeline: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("video YUV"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            }),
        }
    }

    pub(super) fn convert(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        image: &Image,
        target: &wgpu::Texture,
    ) {
        let layout = image.yuv.as_ref().expect("YUV image");
        let plane = |i: usize, component: u32, step: u32| {
            let (offset, stride) = layout.planes[i];
            [offset as u32 + component, stride as u32, step, 0]
        };
        let (u, v) = match layout.format {
            Format::Nv12 => (plane(1, 0, 2), plane(1, 1, 2)),
            _ => (plane(1, 0, 1), plane(2, 0, 1)),
        };
        let alpha = if layout.format == Format::I420a {
            plane(3, 0, 1)
        } else {
            [0; 4]
        };
        let words = [
            [
                image.size[0],
                image.size[1],
                layout.bt709 as u32,
                layout.full as u32,
            ],
            plane(0, 0, 1),
            u,
            v,
            alpha,
        ];
        let info = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("video layout"),
                contents: &bytes(words.as_flattened()),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        // Storage buffers hold u32 words. Padding the final word does not change plane offsets.
        let data = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("video planes"),
            size: (image.pixels.len() as u64).div_ceil(4) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: true,
        });
        data.get_mapped_range_mut(..)
            .expect("mapped video planes")
            .slice(..image.pixels.len())
            .copy_from_slice(&image.pixels);
        data.unmap();
        let view = target.create_view(&wgpu::TextureViewDescriptor {
            mip_level_count: Some(1),
            ..Default::default()
        });
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("video YUV"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: info.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: data.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(image.size[0].div_ceil(8), image.size[1].div_ceil(8), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::yuv;

    #[test]
    fn yuv_mipmaps_match_cpu_with_padding_alpha_odd_sizes_and_both_ranges_and_matrices() {
        if !std::env::var("VALLE_TEST_LAYERS")
            .unwrap_or_default()
            .split(',')
            .any(|s| s == "gpu")
        {
            return;
        }
        let gpu = valle_native::gpu::adapter(None)
            .and_then(|c| Gpu::with(c.adapter))
            .unwrap();
        for size in [[1, 1], [1, 17], [18, 1], [3, 5], [16, 8], [319, 179]] {
            let [w, h] = size.map(|n| n as usize);
            for format in [Format::I420, Format::Nv12, Format::I420a] {
                let mut pixels = vec![137; 7];
                let mut planes = Vec::new();
                let count = match format {
                    Format::I420 => 3,
                    Format::Nv12 => 2,
                    Format::I420a => 4,
                };
                for i in 0..count {
                    let (width, height, step) = match (format, i) {
                        (_, 0) | (Format::I420a, 3) => (w, h, 1),
                        (Format::Nv12, 1) => (w.div_ceil(2), h.div_ceil(2), 2),
                        _ => (w.div_ceil(2), h.div_ceil(2), 1),
                    };
                    let stride = width * step + 5;
                    planes.push((pixels.len(), stride));
                    pixels.extend(
                        (0..stride * height + 3).map(|k| ((k * 73 + k / 7 + i * 31) % 256) as u8),
                    );
                }
                for (bt709, full) in [(false, false), (false, true), (true, false), (true, true)] {
                    let image = Image::from_yuv(
                        size,
                        pixels.clone(),
                        yuv::Layout {
                            format,
                            planes: planes.clone(),
                            bt709,
                            full,
                        },
                    )
                    .unwrap();
                    let texture = gpu.mipmap.texture(&gpu, &image);
                    let cpu = image.to_rgba();
                    for (level, expected) in
                        std::iter::once(cpu.clone()).chain(cpu.mipmap()).enumerate()
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
                        buffer.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
                        gpu.device
                            .poll(wgpu::PollType::wait_indefinitely())
                            .unwrap();
                        let mapped = buffer.get_mapped_range(..).unwrap();
                        let actual = mapped
                            .chunks(stride as usize)
                            .flat_map(|row| row[..w as usize * 4].iter());
                        let max = actual
                            .zip(expected.pixels.iter())
                            .map(|(a, b)| a.abs_diff(*b))
                            .max()
                            .unwrap();
                        // GPU fused arithmetic can put a half-byte on the other side of rounding.
                        assert!(
                            max <= 1,
                            "{size:?} {format:?} bt709={bt709} full={full}, mip {level}: max {max}"
                        );
                        drop(mapped);
                        buffer.unmap();
                    }
                }
            }
        }
    }
}
