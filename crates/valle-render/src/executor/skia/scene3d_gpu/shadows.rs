//! One bounded shadow set, keyed by projected geometry, alpha sampling and light projection.
use super::assets::Atlas;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use valle_engine::{
    motion::{
        memory,
        scene3d::{GpuRasterShading, GpuRasterShadow},
    },
    resource::ContentDigest,
};
use wgpu::util::DeviceExt;

pub(super) struct ShadowSet {
    pub key: ContentDigest,
    pub texture: wgpu::Texture,
    pub upload_bytes: u64,
    pub bytes: u64,
    _depth: wgpu::Texture,
    _buffers: Vec<wgpu::Buffer>,
    _memory: Option<memory::MemoryLease>,
}

pub(super) struct ShadowRasterizer {
    pipeline: wgpu::RenderPipeline,
}

impl ShadowRasterizer {
    pub fn new(device: &wgpu::Device, module: &wgpu::ShaderModule) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Scene3D shadow caster"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Scene3D shadow caster"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Scene3D directional shadow"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("shadow_vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("shadow_fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::R32Float.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline }
    }

    pub fn key(shading: &GpuRasterShading) -> ContentDigest {
        if shading.shadows.is_empty() {
            return ContentDigest::of_bytes(&[]);
        }
        let mut key = Sha256::new();
        for map in &shading.shadows {
            key.update(map.light.to_le_bytes());
            for value in map.values.into_iter().flatten() {
                key.update(value.to_le_bytes());
            }
            for v in &map.vertices {
                for value in v.position.into_iter().chain([v.depth]).chain(v.uv) {
                    key.update(value.to_le_bytes());
                }
                key.update(v.material.to_le_bytes());
            }
        }
        let start = GpuRasterShading::HEADER_LEN;
        for material in 0..shading.values[1][0] as usize {
            for value in
                &shading.values[start + material * GpuRasterShading::MATERIAL_STRIDE + 2][1..]
            {
                key.update(value.to_le_bytes());
            }
            for value in shading.values[start + material * GpuRasterShading::MATERIAL_STRIDE + 4] {
                key.update(value.to_le_bytes());
            }
        }
        key.update(Atlas::key(shading).as_bytes());
        ContentDigest::from_bytes(key.finalize().into())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        shading: &GpuRasterShading,
        buffer: &wgpu::Buffer,
        uniform: &wgpu::Buffer,
        atlas: &Atlas,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        limit: usize,
    ) -> Result<Option<Arc<ShadowSet>>, String> {
        let count = shading.shadows.len() as u32;
        let size = if count == 0 { 1 } else { GpuRasterShadow::SIZE };
        let upload_bytes = shading
            .shadows
            .iter()
            .map(|s| (s.vertices.len() as u64 * 28).max(4) + 16)
            .sum::<u64>();
        let required =
            u64::from(size) * u64::from(size) * u64::from(count.max(1)) * 8 + upload_bytes;
        if required > limit as u64 {
            return Ok(None);
        }
        // Maps are required frame working storage, explicitly prepaid by admission. The lease
        // can outlive that frame in this bounded cache; it keeps charging the same host ledger.
        let Ok(memory) = memory::reserve(memory::MemoryKind::Resource, required) else {
            return Ok(None);
        };
        let Ok(_staging) = memory::reserve(memory::MemoryKind::Temporary, upload_bytes) else {
            return Ok(None);
        };
        let texture = |format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Scene3D cached shadow maps"),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: count.max(1),
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let maps = texture(wgpu::TextureFormat::R32Float);
        let depth = texture(wgpu::TextureFormat::Depth32Float);
        let atlas_view = atlas.texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut buffers = Vec::new();
        for layer in 0..count {
            let map = &shading.shadows[layer as usize];
            let mut vertex_bytes = map.vertex_bytes();
            if vertex_bytes.is_empty() {
                vertex_bytes.resize(4, 0);
            }
            let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Scene3D shadow vertices"),
                contents: &vertex_bytes,
                usage: wgpu::BufferUsages::STORAGE,
            });
            let range = [map.values[3][2], map.values[3][3], 0.0, 0.0]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>();
            let range = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Scene3D shadow range"),
                contents: &range,
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Scene3D shadow caster"),
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: vertices.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&atlas_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: range.as_entire_binding(),
                    },
                ],
            });
            let view = |texture: &wgpu::Texture| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            };
            let color_view = view(&maps);
            let depth_view = view(&depth);
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Scene3D directional shadow"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: f32::MAX as f64,
                                g: 0.0,
                                b: 0.0,
                                a: 0.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..map.vertices.len() as u32 * 2, 0..1);
            }
            buffers.extend([vertices, range]);
        }
        queue.submit([encoder.finish()]);
        Ok(Some(Arc::new(ShadowSet {
            key: Self::key(shading),
            texture: maps,
            upload_bytes,
            bytes: required,
            _depth: depth,
            _buffers: buffers,
            _memory: memory,
        })))
    }
}
