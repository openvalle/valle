//! Stage 28 feasibility probe: WGSL rasterization -> shared Metal textures -> Ganesh.
#![cfg(target_os = "macos")]

#[path = "scene3d_gpu_interop/geometry.rs"]
mod geometry;

use std::{collections::BTreeMap, ffi::c_void, time::Instant};

use sha2::{Digest, Sha256};
use skia_safe::{
    AlphaType, BlendMode, ColorType, Data, FilterMode, Image, ImageInfo, MipmapMode, Paint,
    RuntimeEffect, SamplingOptions, TileMode, gpu, runtime_effect::ChildPtr,
};

const RASTER: &str = r#"
@group(0) @binding(0) var<uniform> frame: vec4<u32>;
struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) near: u32,
};
@vertex fn vertex(@builtin(vertex_index) index: u32, @builtin(instance_index) instance: u32) -> Vertex {
    let points = array(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(0.0, 1.0));
    let near = instance == 0u;
    var out: Vertex;
    out.position = vec4(points[index] * select(1.0, 0.5, near), select(0.75, 0.25, near), 1.0);
    out.near = select(0u, 1u, near);
    return out;
}
struct Fragment {
    @location(0) color: vec4<f32>,
    @location(1) depth: vec4<f32>,
};
@fragment fn fragment(in: Vertex) -> Fragment {
    var out: Fragment;
    out.color = select(vec4(0.0, 0.5, 1.0, 1.0), vec4(2.0 + f32(frame.x % 4u) * 0.25, 0.0, 0.0, 1.0), in.near != 0u);
    // Match the existing DOF raw-f32 RGBA8 depth input, without a CPU packing pass.
    let bits = bitcast<u32>(select(6.0, 2.0, in.near != 0u));
    out.depth = vec4<f32>(vec4(bits, bits >> 8u, bits >> 16u, bits >> 24u) & vec4(255u)) / 255.0;
    return out;
}
"#;

const COMPOSE: &str = r#"
uniform shader color;
uniform shader packedDepth;
uniform float width;
half4 main(float2 xy) {
    bool showDepth = xy.x >= width;
    float2 p = xy - float2(showDepth ? width : 0.0, 0.0);
    half4 c = color.eval(p);
    if (!showDepth) { return c; }
    float4 b = floor(float4(packedDepth.eval(p)) * 255.0 + 0.5);
    if (c.a == 0.0) {
        return b == float4(255.0, 255.0, 127.0, 127.0) ? half4(0.0) : half4(1.0);
    }
    float exponent = b.w * 2.0 + floor(b.z / 128.0);
    float mantissa = mod(b.z, 128.0) * 65536.0 + b.y * 256.0 + b.x;
    float depth = exp2(exponent - 127.0) * (1.0 + mantissa / 8388608.0);
    return half4(depth / 8.0, 0.0, 0.0, 1.0);
}
"#;

fn import(
    context: &mut gpu::DirectContext,
    texture: &wgpu::Texture,
    color_type: ColorType,
    alpha_type: AlphaType,
) -> Image {
    // SAFETY: only Metal textures from the same live device are imported. The caller retains
    // their wgpu owners until Ganesh has submitted and the final validation read has completed.
    unsafe {
        let raw = texture.as_hal::<wgpu::hal::api::Metal>().unwrap();
        let handle = std::ptr::from_ref(raw.raw_handle()).cast::<c_void>();
        let info = gpu::mtl::TextureInfo::new(handle);
        let backend = gpu::backend_textures::make_mtl(
            (texture.width() as i32, texture.height() as i32),
            gpu::Mipmapped::No,
            &info,
            "scene3d-probe",
        );
        gpu::images::borrow_texture_from(
            context,
            &backend,
            gpu::SurfaceOrigin::TopLeft,
            color_type,
            alpha_type,
            None,
        )
        .unwrap()
    }
}

fn shared_device() -> (wgpu::Device, wgpu::Queue, gpu::DirectContext) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    // SAFETY: Ganesh retains these exact wgpu Metal objects. Both APIs submit on the same
    // command queue; no CPU wait or texture readback is needed at the producer boundary.
    let backend = unsafe {
        let raw_device = device.as_hal::<wgpu::hal::api::Metal>().unwrap();
        let raw_queue = queue.as_hal::<wgpu::hal::api::Metal>().unwrap();
        gpu::mtl::BackendContext::new(
            std::ptr::from_ref(&**raw_device.raw_device()).cast::<c_void>(),
            std::ptr::from_ref(raw_queue.as_raw()).cast::<c_void>(),
        )
    };
    let mut context = gpu::direct_contexts::make_metal(&backend, None).unwrap();
    context.set_resource_cache_limit(32 * 1024 * 1024);
    (device, queue, context)
}

#[test]
#[ignore = "requires a real Metal device"]
fn wgsl_raster_color_and_depth_share_skia_device_without_intermediate_readback() {
    let (device, queue, mut context) = shared_device();
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("scene3d-interop"),
        source: wgpu::ShaderSource::Wgsl(RASTER.into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("scene3d-interop"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vertex"),
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
            module: &module,
            entry_point: Some("fragment"),
            compilation_options: Default::default(),
            targets: &[
                Some(wgpu::TextureFormat::Rgba16Float.into()),
                Some(wgpu::TextureFormat::Rgba8Unorm.into()),
            ],
        }),
        multiview_mask: None,
        cache: None,
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("frame"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("frame"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    let compositor = RuntimeEffect::make_for_shader(COMPOSE, None).unwrap();
    for (width, height) in [(64u32, 64u32), (1920, 1080), (3840, 2160)] {
        // Color, packed depth, depth attachment, and the double-width F16 validation target.
        let texture_bytes = u64::from(width) * u64::from(height) * 32;
        assert!(texture_bytes <= 256 * 1024 * 1024);
        let make_texture = |format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("scene3d-interop"),
                size: wgpu::Extent3d {
                    width,
                    height,
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
        let packed_depth = make_texture(wgpu::TextureFormat::Rgba8Unorm);
        let depth = make_texture(wgpu::TextureFormat::Depth32Float);
        let color_view = color.create_view(&Default::default());
        let packed_view = packed_depth.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let info = ImageInfo::new(
            (width as i32 * 2, height as i32),
            ColorType::RGBAF16,
            AlphaType::Premul,
            None,
        );
        let read_info = info.with_color_type(ColorType::RGBAF32);
        let mut output = gpu::surfaces::render_target(
            &mut context,
            gpu::Budgeted::Yes,
            &info,
            None,
            gpu::SurfaceOrigin::TopLeft,
            None,
            None,
            None,
        )
        .unwrap();
        let mut observed = BTreeMap::new();
        let started = Instant::now();
        for frame in [0u32, 3, 1, 0, 3, 2, 1, 0] {
            let mut bytes = [0; 16];
            bytes[..4].copy_from_slice(&frame.to_ne_bytes());
            queue.write_buffer(&uniform, 0, &bytes);
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let attachment = |view, clear| {
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(clear),
                            store: wgpu::StoreOp::Store,
                        },
                    })
                };
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("two overlapping depths"),
                    color_attachments: &[
                        attachment(&color_view, wgpu::Color::TRANSPARENT),
                        attachment(
                            &packed_view,
                            wgpu::Color {
                                r: 1.0,
                                g: 1.0,
                                b: 127.0 / 255.0,
                                a: 127.0 / 255.0,
                            },
                        ),
                    ],
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
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                // Near first, far second: a missing or reversed depth test gives the wrong color.
                pass.draw(0..3, 0..2);
            }
            queue.submit([encoder.finish()]);
            let images = [
                import(&mut context, &color, ColorType::RGBAF16, AlphaType::Premul),
                import(
                    &mut context,
                    &packed_depth,
                    ColorType::RGBA8888,
                    AlphaType::Unpremul,
                ),
            ];
            let children = images
                .iter()
                .map(|image| {
                    ChildPtr::Shader(
                        image
                            .to_raw_shader(
                                (TileMode::Decal, TileMode::Decal),
                                SamplingOptions::new(FilterMode::Nearest, MipmapMode::None),
                                None,
                            )
                            .unwrap(),
                    )
                })
                .collect::<Vec<_>>();
            let shader = compositor
                .make_shader(
                    Data::new_copy(&(width as f32).to_ne_bytes()),
                    &children,
                    None,
                )
                .unwrap();
            let mut paint = Paint::default();
            paint.set_blend_mode(BlendMode::Src).set_shader(shader);
            output.canvas().draw_paint(&paint);
            context.flush_and_submit_surface(&mut output, gpu::SyncCpu::No);
            // Final output validation only. There is no CPU copy between rasterization and Skia.
            let stride = width as usize * 2 * 16;
            let mut bytes = vec![0; stride * height as usize];
            assert!(output.read_pixels(&read_info, &mut bytes, stride, (0, 0)));
            let pixel = |x: u32, y: u32| {
                let offset = y as usize * stride + x as usize * 16;
                std::array::from_fn::<_, 4, _>(|i| {
                    f32::from_ne_bytes(
                        bytes[offset + i * 4..offset + i * 4 + 4]
                            .try_into()
                            .unwrap(),
                    )
                })
            };
            assert_eq!(
                pixel(width / 2, height / 2),
                [2.0 + (frame % 4) as f32 * 0.25, 0.0, 0.0, 1.0]
            );
            assert_eq!(pixel(width / 2, height / 8), [0.0, 0.5, 1.0, 1.0]);
            assert_eq!(pixel(width + width / 2, height / 2), [0.25, 0.0, 0.0, 1.0]);
            assert_eq!(pixel(width + width / 2, height / 8), [0.75, 0.0, 0.0, 1.0]);
            assert_eq!(pixel(0, 0), [0.0; 4]);
            assert_eq!(
                pixel(width, 0),
                [0.0; 4],
                "background depth must retain f32::MAX"
            );
            let digest: [u8; 32] = Sha256::digest(&bytes).into();
            if let Some(previous) = observed.insert(frame, digest) {
                assert_eq!(digest, previous, "random-access frame changed");
            }
        }
        eprintln!(
            "WGSL -> Skia shared Metal: {width}x{height}, 8 frames, texture_bytes={texture_bytes}, intermediate_readback_bytes=0, validation_inclusive_elapsed={:?}",
            started.elapsed()
        );
        // Drop Skia borrows before their wgpu owners, then exercise resource-cache eviction.
        drop(output);
        context.free_gpu_resources();
    }
}
