//! GPU weighted transparency. Integer sums preserve replay and submission-order invariance.
pub(super) struct Transparency {
    pipeline: wgpu::RenderPipeline,
    resolve: wgpu::ComputePipeline,
}

pub(super) struct TransparencyOutput {
    pub color: wgpu::Texture,
    pub depth: wgpu::Texture,
    _buffers: [wgpu::Buffer; 6],
}

impl Transparency {
    pub fn new(
        device: &wgpu::Device,
        module: &wgpu::ShaderModule,
        scene_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let mut entries = (0..6)
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect::<Vec<_>>();
        entries.extend([
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 7,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 8,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba16Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 9,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
        ]);
        let group = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Scene3D transparency"),
            entries: &entries,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Scene3D transparency"),
            bind_group_layouts: &[Some(scene_layout), Some(&group)],
            immediate_size: 0,
        });
        let make_pipeline = || {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Scene3D transparent triangles"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("raster_lit_vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some("transparent_fragment"),
                    compilation_options: Default::default(),
                    targets: &[],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = make_pipeline();
        let resolve = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Scene3D transparency resolve"),
            layout: Some(&layout),
            module,
            entry_point: Some("resolve_transparency"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { pipeline, resolve }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        group: &wgpu::BindGroup,
        count: u32,
        color: &wgpu::Texture,
        depth: &wgpu::Texture,
        dof: bool,
    ) -> TransparencyOutput {
        let extent = color.size();
        let pixels = u64::from(extent.width) * u64::from(extent.height);
        let buffers = std::array::from_fn(|i| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Scene3D fixed-point transparency"),
                size: pixels * if i == 5 { 4 } else { 8 },
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        });
        let texture = |format, size| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Scene3D resolved transparency"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            })
        };
        let output = texture(wgpu::TextureFormat::Rgba16Float, extent);
        let packed = texture(
            wgpu::TextureFormat::Rgba8Unorm,
            if dof {
                extent
            } else {
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                }
            },
        );
        let color_view = color.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let output_view = output.create_view(&Default::default());
        let packed_view = packed.create_view(&Default::default());
        let mut entries = buffers
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: b.as_entire_binding(),
            })
            .collect::<Vec<_>>();
        entries.extend([
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(&depth_view),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(&color_view),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: wgpu::BindingResource::TextureView(&output_view),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::TextureView(&packed_view),
            },
        ]);
        let accumulation = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Scene3D transparency"),
            layout: &self.resolve.get_bind_group_layout(1),
            entries: &entries,
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Scene3D transparent triangles"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: None,
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.set_bind_group(1, &accumulation, &[]);
            pass.draw(0..count * 2, 0..1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.resolve);
            pass.set_bind_group(0, group, &[]);
            pass.set_bind_group(1, &accumulation, &[]);
            pass.dispatch_workgroups(extent.width.div_ceil(8), extent.height.div_ceil(8), 1);
        }
        TransparencyOutput {
            color: output,
            depth: packed,
            _buffers: buffers,
        }
    }
}
