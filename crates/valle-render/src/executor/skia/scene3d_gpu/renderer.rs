use super::shadows::{ShadowRasterizer, ShadowSet};
use super::transparency::{Transparency, TransparencyOutput};
use super::{Scene3dGpuInput, assets::Atlas};
use crate::executor::skia::surface::working_color_space;
use skia_safe::{AlphaType, ColorSpace, ColorType, Image, gpu};
use std::sync::Arc;
use wgpu::util::DeviceExt;

pub(crate) struct GpuRasterizer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: [Option<wgpu::RenderPipeline>; 6],
    module: wgpu::ShaderModule,
    lit_layout: wgpu::PipelineLayout,
    mesh: Option<Arc<MeshBuffer>>,
    background_pipelines: [Option<wgpu::RenderPipeline>; 2],
    atlas: Option<Arc<Atlas>>,
    shadow_rasterizer: ShadowRasterizer,
    shadows: Option<Arc<ShadowSet>>,
    transparency: Transparency,
    dfg: wgpu::Texture,
    _lut_memory: Option<valle_engine::motion::memory::MemoryLease>,
}

pub(crate) struct GpuRasterOutput {
    pub image: Image,
    pub depth_image: Option<Image>,
    pub upload_bytes: u64,
    _color: wgpu::Texture,
    _depth: wgpu::Texture,
    _packed_depth: Option<wgpu::Texture>,
    _vertices: Arc<wgpu::Buffer>,
    _mesh: Option<Arc<MeshBuffer>>,
    _uniform: wgpu::Buffer,
    _shading: Option<wgpu::Buffer>,
    _atlas: Option<Arc<Atlas>>,
    _shadows: Option<Arc<ShadowSet>>,
    _transparency: Option<TransparencyOutput>,
    _memory: Option<valle_engine::motion::memory::MemoryLease>,
}

struct MeshBuffer {
    key: valle_engine::motion::ContentDigest,
    buffer: Arc<wgpu::Buffer>,
    bytes: u64,
    budget: Option<valle_engine::motion::memory::MemoryBudget>,
    _memory: Option<valle_engine::motion::memory::MemoryLease>,
}

impl GpuRasterizer {
    pub(crate) fn trim_assets(&mut self, limit: usize) {
        if self.atlas.as_ref().map_or(0, |a| a.bytes)
            + self.shadows.as_ref().map_or(0, |s| s.bytes)
            + self.mesh.as_ref().map_or(0, |m| m.bytes)
            > limit as u64
        {
            self.atlas = None;
            self.shadows = None;
            self.mesh = None;
        }
    }

    pub(crate) fn new(device: wgpu::Device, queue: wgpu::Queue) -> Result<Self, String> {
        let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Scene3D raster"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
                    include_str!("common.wgsl"),
                    include_str!("assets.wgsl"),
                    include_str!("lighting.wgsl"),
                    include_str!("area.wgsl"),
                    include_str!("shadows.wgsl"),
                    include_str!("transparency.wgsl"),
                    include_str!("transmission.wgsl"),
                    include_str!("raster.wgsl"),
                    include_str!("mesh.wgsl")
                )
                .into(),
            ),
        });
        let binding = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let texture_binding = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let lit_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Scene3D lighting"),
            entries: &[
                binding(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                binding(
                    1,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                binding(2, texture_binding),
                binding(3, texture_binding),
                binding(
                    6,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                binding(
                    4,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                ),
            ],
        });
        let lit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Scene3D lighting"),
            bind_group_layouts: &[Some(&lit_group_layout)],
            immediate_size: 0,
        });
        let shadow_rasterizer = ShadowRasterizer::new(&device, &module);
        let transparency = Transparency::new(&device, &module, &lit_group_layout);
        if let Some(error) = pollster::block_on(error_scope.pop()) {
            return Err(error.to_string());
        }
        let lut_memory = valle_engine::motion::memory::reserve(
            valle_engine::motion::memory::MemoryKind::Resource,
            81_920,
        )
        .map_err(|error| error.to_string())?;
        let dfg = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Scene3D DFG"),
            size: wgpu::Extent3d {
                width: 128,
                height: 80,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &dfg,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            valle_engine::motion::scene3d::GpuRasterShading::lighting_lut(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(1024),
                rows_per_image: Some(80),
            },
            wgpu::Extent3d {
                width: 128,
                height: 80,
                depth_or_array_layers: 1,
            },
        );
        Ok(Self {
            device,
            queue,
            pipelines: [None, None, None, None, None, None],
            module,
            lit_layout,
            mesh: None,
            background_pipelines: [None, None],
            atlas: None,
            shadow_rasterizer,
            shadows: None,
            transparency,
            dfg,
            _lut_memory: lut_memory,
        })
    }

    fn make_pipeline(
        &self,
        mode: usize,
        packed_depth: bool,
        background: bool,
    ) -> wgpu::RenderPipeline {
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Scene3D raster"),
                layout: (mode != 0 || background).then_some(&self.lit_layout),
                vertex: wgpu::VertexState {
                    module: &self.module,
                    entry_point: Some(if background {
                        "background_vertex"
                    } else {
                        match mode {
                            0 => "raster_vertex",
                            2 => "mesh_vertex",
                            _ => "raster_lit_vertex",
                        }
                    }),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(!background),
                    depth_compare: Some(if background {
                        wgpu::CompareFunction::Always
                    } else if mode == 2 {
                        wgpu::CompareFunction::GreaterEqual
                    } else {
                        wgpu::CompareFunction::Greater
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.module,
                    entry_point: Some(if background {
                        "background_fragment"
                    } else if mode == 2 {
                        "mesh_fragment"
                    } else if mode != 0 {
                        "raster_lit_fragment"
                    } else {
                        "raster_fragment"
                    }),
                    compilation_options: Default::default(),
                    targets: &[
                        Some(wgpu::TextureFormat::Rgba16Float.into()),
                        if packed_depth {
                            Some(wgpu::TextureFormat::Rgba8Unorm.into())
                        } else {
                            None
                        },
                    ],
                }),
                multiview_mask: None,
                cache: None,
            })
    }

    pub(crate) fn render(
        &mut self,
        input: &Scene3dGpuInput,
        context: &mut gpu::DirectContext,
    ) -> Result<Option<GpuRasterOutput>, String> {
        self.device
            .poll(wgpu::PollType::Poll)
            .map_err(|error| error.to_string())?;
        let geometry = &input.geometry;
        if geometry.width > self.device.limits().max_texture_dimension_2d
            || geometry.height > self.device.limits().max_texture_dimension_2d
        {
            return Ok(None);
        }
        let dof = input
            .frame
            .camera
            .depth_of_field
            .is_some_and(|d| d.max_blur_radius > 0.0);
        let blend = input.has_blend();
        let opaque_depth = dof && !blend;
        let mut asset_upload_bytes = 0;
        self.trim_assets(context.resource_cache_limit());
        let atlas = if let Some(shading) = &geometry.shading {
            let key = Atlas::key(shading);
            if self.atlas.as_ref().is_none_or(|atlas| atlas.key != key) {
                self.atlas = None;
                self.shadows = None;
                self.mesh = None;
                self.atlas = Atlas::build(
                    shading,
                    &self.device,
                    &self.queue,
                    context.resource_cache_limit(),
                )?;
                asset_upload_bytes = self.atlas.as_ref().map_or(0, |a| a.bytes);
            }
            let Some(atlas) = self.atlas.clone() else {
                return Ok(None);
            };
            Some(atlas)
        } else {
            None
        };
        let mode = if geometry.mesh.is_some() {
            2
        } else {
            usize::from(geometry.shading.is_some())
        };
        let index = mode + 3 * usize::from(opaque_depth);
        let background = geometry
            .shading
            .as_ref()
            .is_some_and(|s| (s.values[1][3] as u32 & 2) != 0);
        if self.pipelines[index].is_none()
            || (background && self.background_pipelines[usize::from(opaque_depth)].is_none())
        {
            let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
            if self.pipelines[index].is_none() {
                self.pipelines[index] = Some(self.make_pipeline(mode, opaque_depth, false));
            }
            if background && self.background_pipelines[usize::from(opaque_depth)].is_none() {
                self.background_pipelines[usize::from(opaque_depth)] =
                    Some(self.make_pipeline(1, opaque_depth, true));
            }
            if let Some(error) = pollster::block_on(scope.pop()) {
                return Err(error.to_string());
            }
        }
        let pipeline = self.pipelines[index].as_ref().expect("pipeline prepared");
        let upload_bytes = input.upload_bytes().ok_or("vertex byte count overflow")?;
        let memory = valle_engine::motion::memory::reserve(
            valle_engine::motion::memory::MemoryKind::Temporary,
            input.texture_bytes() + upload_bytes,
        )
        .map_err(|error| error.to_string())?;
        let _staging = valle_engine::motion::memory::reserve(
            valle_engine::motion::memory::MemoryKind::Temporary,
            upload_bytes,
        )
        .map_err(|error| error.to_string())?;
        let make_texture = |format| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Scene3D raster target"),
                size: wgpu::Extent3d {
                    width: geometry.width,
                    height: geometry.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | interop_copy_usage(),
                view_formats: &[],
            })
        };
        let color = make_texture(wgpu::TextureFormat::Rgba16Float);
        let depth = make_texture(wgpu::TextureFormat::Depth32Float);
        let packed_depth = opaque_depth.then(|| make_texture(wgpu::TextureFormat::Rgba8Unorm));
        let packed_view = packed_depth
            .as_ref()
            .map(|t| t.create_view(&Default::default()));
        let color_view = color.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let mesh = if let Some(source) = &geometry.mesh {
            let budget = valle_engine::motion::cpu::memory_budget();
            if self.mesh.as_ref().is_none_or(|m| {
                m.key != source.key
                    || match (&m.budget, &budget) {
                        (None, None) => false,
                        (Some(a), Some(b)) => !a.same_budget(b),
                        _ => true,
                    }
            }) {
                self.mesh = None;
                let bytes = source.bytes().len() as u64;
                let available = (context.resource_cache_limit() as u64)
                    .saturating_sub(self.atlas.as_ref().map_or(0, |a| a.bytes))
                    .saturating_sub(self.shadows.as_ref().map_or(0, |s| s.bytes));
                if bytes > available || bytes > self.device.limits().max_storage_buffer_binding_size
                {
                    return Ok(None);
                }
                let Ok(memory) = valle_engine::motion::memory::reserve_retained(
                    valle_engine::motion::memory::MemoryKind::Resource,
                    bytes,
                ) else {
                    return Ok(None);
                };
                let Ok(_staging) = valle_engine::motion::memory::reserve_retained(
                    valle_engine::motion::memory::MemoryKind::Temporary,
                    bytes,
                ) else {
                    return Ok(None);
                };
                let buffer = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Scene3D immutable model"),
                        contents: source.bytes(),
                        usage: wgpu::BufferUsages::STORAGE,
                    });
                asset_upload_bytes += bytes;
                self.mesh = Some(Arc::new(MeshBuffer {
                    key: source.key,
                    buffer: Arc::new(buffer),
                    bytes,
                    budget,
                    _memory: memory,
                }));
            }
            self.mesh.clone()
        } else {
            None
        };
        let vertices = if let Some(mesh) = &mesh {
            Arc::clone(&mesh.buffer)
        } else {
            let mut bytes = input.vertex_bytes();
            if bytes.is_empty() {
                bytes.resize(4, 0);
            }
            Arc::new(
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Scene3D vertices"),
                        contents: &bytes,
                        usage: wgpu::BufferUsages::STORAGE,
                    }),
            )
        };
        let bytes = [
            geometry.width as f32,
            geometry.height as f32,
            geometry.depth_scale,
            0.0,
        ]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Scene3D camera"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let shading = geometry.shading.as_ref().map(|shading| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Scene3D lighting"),
                    contents: &shading.bytes(),
                    usage: wgpu::BufferUsages::STORAGE,
                })
        });
        let mut shadow_upload_bytes = 0;
        let shadows = if let (Some(data), Some(buffer), Some(atlas)) =
            (&geometry.shading, &shading, &atlas)
        {
            let key = ShadowRasterizer::key(data);
            if self.shadows.as_ref().is_none_or(|maps| maps.key != key) {
                self.shadows = None;
                self.shadows = self.shadow_rasterizer.render(
                    data,
                    buffer,
                    &uniform,
                    atlas,
                    &self.device,
                    &self.queue,
                    context
                        .resource_cache_limit()
                        .saturating_sub(atlas.bytes as usize)
                        .saturating_sub(self.mesh.as_ref().map_or(0, |m| m.bytes as usize)),
                )?;
                shadow_upload_bytes = self.shadows.as_ref().map_or(0, |s| s.upload_bytes);
            }
            let Some(maps) = self.shadows.clone() else {
                return Ok(None);
            };
            Some(maps)
        } else {
            None
        };
        let dfg_view = self.dfg.create_view(&Default::default());
        let atlas_view = atlas
            .as_ref()
            .map(|a| a.texture.create_view(&Default::default()));
        let shadow_view = shadows.as_ref().map(|s| {
            s.texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        });
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: vertices.as_entire_binding(),
            },
        ];
        if let Some(shading) = &shading {
            entries.extend([
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: shading.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&dfg_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(
                        atlas_view.as_ref().ok_or("missing material atlas")?,
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(
                        shadow_view.as_ref().ok_or("missing shadow maps")?,
                    ),
                },
            ]);
        }
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Scene3D camera"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Scene3D triangles"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    packed_view
                        .as_ref()
                        .map(|view| wgpu::RenderPassColorAttachment {
                            view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 1.0,
                                    g: 1.0,
                                    b: 127.0 / 255.0,
                                    a: 127.0 / 255.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &group, &[]);
            if geometry
                .shading
                .as_ref()
                .is_some_and(|s| (s.values[1][3] as u32 & 2) != 0)
            {
                pass.set_pipeline(
                    self.background_pipelines[usize::from(opaque_depth)]
                        .as_ref()
                        .expect("background prepared"),
                );
                pass.draw(0..3, 0..1);
            }
            pass.set_pipeline(pipeline);
            pass.draw(0..geometry.vertex_count(), 0..1);
        }
        let transparency = blend.then(|| {
            self.transparency.encode(
                &self.device,
                &mut encoder,
                &group,
                geometry.vertices.len() as u32,
                &color,
                &depth,
                dof,
            )
        });
        let color_texture = transparency.as_ref().map_or(&color, |o| &o.color);
        let depth_texture = dof
            .then(|| {
                transparency
                    .as_ref()
                    .map(|o| &o.depth)
                    .or(packed_depth.as_ref())
            })
            .flatten();
        // Establish the producer -> compositor dependency before submitting. On Vulkan this
        // transitions both images to SHADER_READ_ONLY_OPTIMAL, including compute OIT output.
        // Ganesh samples them on the exact same device/queue; no CPU wait/readback is involved.
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        encoder.transition_resources(
            std::iter::empty(),
            std::iter::once(color_texture)
                .chain(depth_texture)
                .map(|texture| wgpu::TextureTransition {
                    texture,
                    selector: None,
                    state: wgpu::TextureUses::RESOURCE,
                }),
        );
        self.queue.submit([encoder.finish()]);
        let depth_image = depth_texture
            .map(|texture| {
                import_image(
                    context,
                    texture,
                    ColorType::RGBA8888,
                    AlphaType::Unpremul,
                    None,
                )
            })
            .transpose()?;
        let image = import_image(
            context,
            color_texture,
            ColorType::RGBAF16,
            AlphaType::Premul,
            Some(if geometry.shading.is_some() {
                ColorSpace::new_srgb()
            } else {
                working_color_space().map_err(|error| error.to_string())?
            }),
        )?;
        Ok(Some(GpuRasterOutput {
            image,
            depth_image,
            upload_bytes: upload_bytes + shadow_upload_bytes + asset_upload_bytes,
            _color: color,
            _depth: depth,
            _packed_depth: packed_depth,
            _vertices: vertices,
            _mesh: mesh,
            _uniform: uniform,
            _shading: shading,
            _atlas: atlas,
            _shadows: shadows,
            _transparency: transparency,
            _memory: memory,
        }))
    }
}

#[cfg(target_os = "macos")]
use super::metal::import_image;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use super::vulkan::import_image;

#[cfg(test)]
mod tests;

// Ganesh Vulkan requires transfer source/destination usage even for borrowed sampled images.
pub(super) fn interop_copy_usage() -> wgpu::TextureUsages {
    if cfg!(any(target_os = "linux", target_os = "windows")) {
        wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST
    } else {
        wgpu::TextureUsages::empty()
    }
}
