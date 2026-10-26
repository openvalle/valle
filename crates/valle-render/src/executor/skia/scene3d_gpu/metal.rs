use super::Scene3dGpuInput;
use crate::executor::skia::surface::working_color_space;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{MTLCommandQueue, MTLDevice};
use skia_safe::{AlphaType, ColorType, Image, gpu};
use std::{ffi::c_void, ptr};
use wgpu::util::DeviceExt;

pub(crate) struct GpuRasterizer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
}

pub(crate) struct GpuRasterOutput {
    pub image: Image,
    pub upload_bytes: u64,
    _color: wgpu::Texture,
    _depth: wgpu::Texture,
    _vertices: wgpu::Buffer,
    _uniform: wgpu::Buffer,
    _memory: Option<valle_engine::motion::memory::MemoryLease>,
}

impl GpuRasterizer {
    pub(crate) fn new(
        raw_device: Retained<ProtocolObject<dyn MTLDevice>>,
        raw_queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    ) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::METAL));
        let adapter = adapters
            .into_iter()
            .find(|adapter| {
                // SAFETY: the handle is inspected only while its adapter is alive.
                unsafe { adapter.as_hal::<wgpu::hal::api::Metal>() }
                    .is_some_and(|adapter| ptr::eq(adapter.raw_device(), &*raw_device))
            })
            .ok_or("wgpu did not expose the existing Skia Metal device")?;
        let descriptor = wgpu::DeviceDescriptor::default();
        // SAFETY: the adapter's exact Metal device and its existing queue are retained above.
        // No optional features are enabled; default limits are validated by wgpu-core.
        let (device, queue) = unsafe {
            adapter.create_device_from_hal::<wgpu::hal::api::Metal>(
                wgpu::hal::OpenDevice {
                    device: wgpu::hal::metal::Device::device_from_raw(
                        raw_device,
                        descriptor.required_features,
                        &descriptor.required_limits,
                    ),
                    queue: wgpu::hal::metal::Queue::queue_from_raw(raw_queue, 1.0),
                },
                &descriptor,
            )
        }
        .map_err(|error| error.to_string())?;
        let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Scene3D unlit raster"),
            source: wgpu::ShaderSource::Wgsl(include_str!("metal.wgsl").into()),
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Float32x4, 3 => Uint32, 4 => Uint32];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Scene3D unlit raster"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: valle_engine::motion::scene3d::GpuRasterGeometry::VERTEX_STRIDE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
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
                module: &module,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba16Float.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        if let Some(error) = pollster::block_on(error_scope.pop()) {
            return Err(error.to_string());
        }
        Ok(Self {
            device,
            queue,
            pipeline,
        })
    }

    pub(crate) fn render(
        &mut self,
        input: &Scene3dGpuInput,
        context: &mut gpu::DirectContext,
    ) -> Result<GpuRasterOutput, String> {
        self.device
            .poll(wgpu::PollType::Poll)
            .map_err(|error| error.to_string())?;
        let geometry = &input.geometry;
        let upload_bytes = input.upload_bytes().ok_or("vertex byte count overflow")?;
        let memory = valle_engine::motion::memory::reserve(
            valle_engine::motion::memory::MemoryKind::Temporary,
            u64::from(geometry.width) * u64::from(geometry.height) * 12 + upload_bytes,
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
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let color = make_texture(wgpu::TextureFormat::Rgba16Float);
        let depth = make_texture(wgpu::TextureFormat::Depth32Float);
        let color_view = color.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let mut bytes = input.vertex_bytes();
        if bytes.is_empty() {
            bytes.resize(4, 0);
        }
        let vertices = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Scene3D vertices"),
                contents: &bytes,
                usage: wgpu::BufferUsages::VERTEX,
            });
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
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Scene3D camera"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Scene3D triangles"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..geometry.vertices.len() as u32, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        // SAFETY: these textures and buffers are retained in GpuRasterOutput until Ganesh's
        // completion fence. Both APIs submit to the exact same Metal command queue.
        let image = unsafe {
            let raw = color
                .as_hal::<wgpu::hal::api::Metal>()
                .ok_or("Metal texture unavailable")?;
            let info = gpu::mtl::TextureInfo::new(ptr::from_ref(raw.raw_handle()).cast::<c_void>());
            let backend = gpu::backend_textures::make_mtl(
                (geometry.width as i32, geometry.height as i32),
                gpu::Mipmapped::No,
                &info,
                "Scene3D unlit raster",
            );
            gpu::images::borrow_texture_from(
                context,
                &backend,
                gpu::SurfaceOrigin::TopLeft,
                ColorType::RGBAF16,
                AlphaType::Premul,
                working_color_space().map_err(|error| error.to_string())?,
            )
            .ok_or("cannot import Scene3D Metal color texture")?
        };
        Ok(GpuRasterOutput {
            image,
            upload_bytes,
            _color: color,
            _depth: depth,
            _vertices: vertices,
            _uniform: uniform,
            _memory: memory,
        })
    }
}
