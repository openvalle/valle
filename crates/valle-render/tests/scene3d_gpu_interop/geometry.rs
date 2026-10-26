use super::*;
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};
use valle_motion::scene3d::{
    AlphaMode, AnchorSpec, CameraFrameState, Color4, Frame3DState, GpuRasterGeometry,
    GpuRasterShading, LightFrameState, LightKind, MaterialFrameState, MaterialKind, MaterialSpec,
    MeshFrameState, MeshSpec, Scene3DSpec, SceneResources, ToneMapping, Transform3D, Vec3,
    admit_glb, prepare_gpu_geometry, prepare_scene, render_scene_with_workers,
};
use wgpu::util::DeviceExt;

const GEOMETRY: &str = r#"
struct Globals { extent: vec2<f32>, depthScale: f32, barrier: u32 };
@group(0) @binding(0) var<uniform> globals: Globals;
struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) invZ: f32,
    @location(1) @interpolate(flat) color: vec4<f32>,
    @location(2) @interpolate(flat) ids: vec2<u32>,
};
@vertex fn vertex(@location(0) position: vec2<f32>, @location(1) invZ: f32,
    @location(2) color: vec4<f32>, @location(3) objectId: u32, @location(4) nodeId: u32) -> Vertex {
    var out: Vertex;
    out.position = vec4(position / globals.extent * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.5, 1.0);
    out.invZ = invZ; out.color = color; out.ids = vec2(objectId, nodeId);
    return out;
}
struct Fragment {
    @location(0) color: vec4<f32>,
    @location(1) depth: f32,
    @location(2) objectId: u32,
    @location(3) nodeId: u32,
    @builtin(frag_depth) depthTest: f32,
};
@fragment fn fragment(in: Vertex) -> Fragment {
    var out: Fragment;
    out.color = in.color; out.depth = 1.0 / in.invZ;
    out.objectId = in.ids.x; out.nodeId = in.ids.y;
    out.depthTest = out.depth * globals.depthScale;
    return out;
}
"#;

const LIT_VALIDATION: &str = r#"
struct ValidationFragment {
    @location(0) color: vec4<f32>, @location(1) depth: f32,
    @location(2) objectId: u32, @location(3) nodeId: u32,
    @builtin(frag_depth) depthTest: f32,
};
@fragment fn validate_lit(in: LitVertex) -> ValidationFragment {
    var out: ValidationFragment;
    out.color = lit_color(in); out.depth = 1.0 / in.invZ;
    out.objectId = in.ids.x; out.nodeId = in.ids.y;
    out.depthTest = globals.depthScale / in.invZ;
    return out;
}
"#;

fn read_metadata(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    textures: [&wgpu::Texture; 3],
) -> [Vec<u32>; 3] {
    let (width, height) = (textures[0].width(), textures[0].height());
    let stride = (width * 4).div_ceil(256) * 256;
    let plane_bytes = u64::from(stride) * u64::from(height);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("final metadata validation"),
        size: plane_bytes * 3,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    for (i, texture) in textures.iter().enumerate() {
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: i as u64 * plane_bytes,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }
    let submission = queue.submit([encoder.finish()]);
    let (send, receive) = mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap()
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(30)),
        })
        .unwrap();
    receive
        .recv_timeout(Duration::from_secs(30))
        .unwrap()
        .unwrap();
    let bytes = buffer.slice(..).get_mapped_range().unwrap();
    let planes = std::array::from_fn(|plane| {
        (0..height)
            .flat_map(|y| (0..width).map(move |x| (y, x)))
            .map(|(y, x)| {
                let offset =
                    plane * plane_bytes as usize + y as usize * stride as usize + x as usize * 4;
                u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
            })
            .collect()
    });
    drop(bytes);
    buffer.unmap();
    planes
}

#[test]
#[ignore = "requires a real Metal device; compares actual admitted GLB geometry with CPU"]
fn actual_model_geometry_preserves_coverage_depth_ids_and_anchors() {
    verify_model_geometry(None);
}

#[test]
#[ignore = "requires real Metal; validates the production Lambert/PBR shader and exact visibility"]
fn lit_model_geometry_preserves_coverage_depth_ids_and_bounded_shading_error() {
    for kind in [MaterialKind::Lambert, MaterialKind::Pbr] {
        for tone in [ToneMapping::None, ToneMapping::Aces] {
            verify_model_geometry(Some((kind, tone)));
        }
    }
}

fn verify_model_geometry(lighting: Option<(MaterialKind, ToneMapping)>) {
    let (device, queue, mut context) = shared_device();
    let source = if lighting.is_some() {
        format!(
            "{}\n{}\n{LIT_VALIDATION}",
            include_str!("../../src/executor/skia/scene3d_gpu/metal.wgsl"),
            include_str!("../../src/executor/skia/scene3d_gpu/lighting.wgsl")
        )
    } else {
        GEOMETRY.to_owned()
    };
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("real Scene3D geometry"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Float32x4, 3 => Uint32, 4 => Uint32];
    let lit_attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Float32x4, 3 => Uint32, 4 => Uint32, 5 => Float32x3, 6 => Uint32];
    let formats = [
        wgpu::TextureFormat::Rgba16Float,
        wgpu::TextureFormat::R32Float,
        wgpu::TextureFormat::R32Uint,
        wgpu::TextureFormat::R32Uint,
    ];
    let targets = formats.map(|format| Some(format.into()));
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("real Scene3D geometry"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some(if lighting.is_some() {
                "lit_vertex"
            } else {
                "vertex"
            }),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: GpuRasterGeometry::VERTEX_STRIDE
                    + if lighting.is_some() { 16 } else { 0 },
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: if lighting.is_some() {
                    &lit_attributes
                } else {
                    &attributes
                },
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
            entry_point: Some(if lighting.is_some() {
                "validate_lit"
            } else {
                "fragment"
            }),
            compilation_options: Default::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: None,
    });
    let dfg = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("reference BRDF LUT"),
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg16Float,
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
        GpuRasterShading::brdf_lut(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(64),
            rows_per_image: Some(16),
        },
        wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
    );
    let dfg_view = dfg.create_view(&Default::default());
    for (name, model) in [
        (
            "triangle",
            include_bytes!("../../../valle-motion/tests/fixtures/scene3d/triangle.glb").as_slice(),
        ),
        (
            "sphere",
            include_bytes!("../../../valle-motion/tests/fixtures/scene3d/sphere.glb").as_slice(),
        ),
    ] {
        let model = Arc::new(admit_glb(model).unwrap());
        let resources = SceneResources {
            models: BTreeMap::from([("model".to_owned(), model)]),
            ..Default::default()
        };
        let scene = Scene3DSpec {
            meshes: ["mesh", "behind", "tie", "masked"]
                .map(|key| MeshSpec {
                    key: key.into(),
                    model_control: Some("model".into()),
                    geometry: None,
                    material: MaterialSpec {
                        kind: Some(lighting.map_or(MaterialKind::Unlit, |(kind, _)| kind)),
                        alpha_mode: Some(if key == "masked" {
                            AlphaMode::Mask
                        } else {
                            AlphaMode::Opaque
                        }),
                        double_sided: Some(key == "masked"),
                        ..Default::default()
                    },
                    material_overrides: vec![],
                    animation_clip: None,
                    node_ids: vec![],
                })
                .into(),
            lights: if lighting.is_some() {
                vec![
                    LightKind::Ambient,
                    LightKind::Hemisphere,
                    LightKind::Directional,
                    LightKind::Directional,
                ]
            } else {
                vec![]
            },
            anchors: vec![AnchorSpec {
                key: "center".into(),
                parent: "mesh".into(),
                position: Vec3::ZERO,
            }],
            pbr: valle_motion::scene3d::PbrOptions {
                tone_mapping: lighting.map_or(ToneMapping::None, |(_, tone)| tone),
                ..Default::default()
            },
        };
        for (width, height) in [(128u32, 128u32), (130, 98)] {
            let prepared = prepare_scene(&scene, width, height, &resources).unwrap();
            let make_texture = |format| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("real geometry target"),
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
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            };
            let planes = formats.map(make_texture);
            let views = planes
                .each_ref()
                .map(|texture| texture.create_view(&Default::default()));
            let depth_test = make_texture(wgpu::TextureFormat::Depth32Float);
            let depth_view = depth_test.create_view(&Default::default());
            let info = ImageInfo::new(
                (width as i32, height as i32),
                ColorType::RGBAF16,
                AlphaType::Premul,
                None,
            );
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
            for sample in [0u32, 7, 2, 15, 3, 8, 0, 7] {
                let frame = Frame3DState {
                    exposure: if lighting.is_some() {
                        0.25 + sample as f32 * 0.1
                    } else {
                        1.0
                    },
                    environment_intensity: 0.0,
                    environment_rotation_degrees: 0.0,
                    camera: CameraFrameState {
                        position: Vec3::new(0.0, 0.0, 5.0),
                        target: Vec3::ZERO,
                        fov_y_degrees: 45.0,
                        near: if sample == 2 { 4.9 } else { 0.1 },
                        far: if sample == 15 { 5.1 } else { 100.0 },
                        depth_of_field: None,
                    },
                    meshes: ["mesh", "behind", "tie", "masked"]
                        .into_iter()
                        .enumerate()
                        .map(|(index, key)| MeshFrameState {
                            key: key.into(),
                            material: MaterialFrameState {
                                metallic: Some(index as f32 * 0.25),
                                roughness: Some(if sample == 0 {
                                    0.0525
                                } else {
                                    0.1 + index as f32 * 0.25
                                }),
                                color: Some(Color4([
                                    0.2 + index as f32 * 0.15,
                                    0.6,
                                    0.9,
                                    if key == "masked" && sample % 2 == 0 {
                                        0.2
                                    } else {
                                        0.9
                                    },
                                ])),
                                ..Default::default()
                            },
                            material_overrides: vec![],
                            animation_time: None,
                            transform: Transform3D {
                                translation: Vec3::new(
                                    if sample == 8 { 2.0 } else { 0.0 },
                                    0.0,
                                    match key {
                                        "behind" => -0.25,
                                        "masked" => 0.2,
                                        _ => 0.0,
                                    },
                                ),
                                rotation_degrees: Vec3::new(
                                    sample as f32 * 1.3,
                                    sample as f32 * 7.5,
                                    0.0,
                                ),
                                scale: Vec3::new(
                                    if sample % 2 == 0 { 1.0 } else { -1.0 },
                                    1.0,
                                    1.0,
                                ),
                            },
                            nodes: vec![],
                        })
                        .collect(),
                    lights: if lighting.is_some() {
                        vec![
                            LightFrameState::Ambient {
                                color: Color4([0.6, 0.7, 0.8, 1.0]),
                                intensity: 0.3,
                            },
                            LightFrameState::Hemisphere {
                                direction: Vec3::new(0.2, 1.0, 0.0),
                                sky_color: Color4([0.7, 0.8, 1.0, 1.0]),
                                ground_color: Color4([0.3, 0.2, 0.1, 1.0]),
                                intensity: 0.5,
                            },
                            LightFrameState::Directional {
                                direction: Vec3::new(0.8, 0.2, 1.0),
                                color: Color4([1.0, 0.8, 0.6, 1.0]),
                                intensity: 2.0,
                            },
                            LightFrameState::Directional {
                                direction: Vec3::new(-1.0, 0.2, 1.0),
                                color: Color4([0.3, 0.5, 1.0, 1.0]),
                                intensity: 0.7,
                            },
                        ]
                    } else {
                        vec![]
                    },
                };
                let geometry = prepare_gpu_geometry(&prepared, &frame)
                    .unwrap()
                    .expect("untextured input should be supported");
                let reference = render_scene_with_workers(&prepared, &frame, 1).unwrap();
                assert_eq!(geometry.anchors, reference.anchors);
                let mut vertex_bytes = geometry.vertex_bytes();
                if vertex_bytes.is_empty() {
                    vertex_bytes.resize(4, 0);
                }
                let vertex = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("prepared model vertices"),
                    contents: &vertex_bytes,
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let mut globals = Vec::new();
                for value in [width as f32, height as f32, geometry.depth_scale, 0.0] {
                    globals.extend_from_slice(&value.to_le_bytes());
                }
                let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("camera"),
                    contents: &globals,
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let shading = geometry.shading.as_ref().map(|shading| {
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("production lighting input"),
                        contents: &shading.bytes(),
                        usage: wgpu::BufferUsages::STORAGE,
                    })
                });
                let mut entries = vec![wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                }];
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
                    ]);
                }
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("camera"),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &entries,
                });
                let clears = [
                    wgpu::Color::TRANSPARENT,
                    wgpu::Color {
                        r: f64::from(f32::MAX),
                        ..wgpu::Color::TRANSPARENT
                    },
                    wgpu::Color::TRANSPARENT,
                    wgpu::Color {
                        r: f64::from(u32::MAX),
                        ..wgpu::Color::TRANSPARENT
                    },
                ];
                let attachments = std::array::from_fn::<_, 4, _>(|i| {
                    Some(wgpu::RenderPassColorAttachment {
                        view: &views[i],
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(clears[i]),
                            store: wgpu::StoreOp::Store,
                        },
                    })
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("actual models"),
                        color_attachments: &attachments,
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
                    pass.set_vertex_buffer(0, vertex.slice(..));
                    pass.draw(0..geometry.vertices.len() as u32, 0..1);
                }
                queue.submit([encoder.finish()]);
                let image = import(
                    &mut context,
                    &planes[0],
                    ColorType::RGBAF16,
                    AlphaType::Premul,
                );
                let mut paint = Paint::default();
                paint.set_blend_mode(BlendMode::Src);
                output.canvas().draw_image(&image, (0, 0), Some(&paint));
                context.flush_and_submit_surface(&mut output, gpu::SyncCpu::No);
                let mut color = vec![0; width as usize * height as usize * 8];
                assert!(output.read_pixels(&info, &mut color, width as usize * 8, (0, 0)));
                let [depth, object_ids, node_ids] =
                    read_metadata(&device, &queue, [&planes[1], &planes[2], &planes[3]]);
                let id_errors = object_ids
                    .iter()
                    .zip(&reference.object_ids)
                    .filter(|(a, b)| a != b)
                    .count();
                let node_errors = node_ids
                    .iter()
                    .zip(&reference.node_ids)
                    .filter(|(a, b)| a != b)
                    .count();
                let expected_color = reference.premul_rgba16f_le_bytes();
                assert!(
                    color
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .all(|value| u16::from_ne_bytes(*value) <= 0x3c00),
                    "opaque shader output must be finite and inside [0,1]"
                );
                assert!(
                    color
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .zip(expected_color.as_chunks::<8>().0.iter())
                        .all(|(a, b)| a[6..8] == b[6..8]),
                    "coverage alpha must be exact"
                );
                for (actual, expected) in depth.iter().zip(&reference.depth) {
                    if *expected == f32::MAX {
                        assert_eq!(*actual, f32::MAX.to_bits());
                    }
                }
                let color_errors = color
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .zip(expected_color.as_chunks::<8>().0.iter())
                    .filter(|(a, b)| a != b)
                    .count();
                let half = |bits: u16| {
                    let exponent = (bits >> 10) & 31;
                    let mantissa = bits & 1023;
                    if exponent == 0 {
                        f32::from(mantissa) / 16_777_216.0
                    } else {
                        f32::from_bits(
                            (u32::from(exponent) + 112) << 23 | u32::from(mantissa) << 13,
                        )
                    }
                };
                let max_color_error = color
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .zip(expected_color.as_chunks::<2>().0.iter())
                    .map(|(a, b)| {
                        (half(u16::from_ne_bytes(*a)) - half(u16::from_ne_bytes(*b))).abs()
                    })
                    .fold(0.0_f32, f32::max);
                let relative_depth_error = depth
                    .iter()
                    .zip(&reference.depth)
                    .filter(|(_, z)| **z != f32::MAX)
                    .map(|(actual, expected)| (f32::from_bits(*actual) - expected).abs() / expected)
                    .fold(0.0_f32, f32::max);
                eprintln!(
                    "geometry {name} {lighting:?} {width}x{height} frame={sample} triangles={} color_mismatch={color_errors} max_color_error={max_color_error} id_mismatch={id_errors} node_mismatch={node_errors} relative_depth_error={relative_depth_error}",
                    geometry.vertices.len() / 3
                );
                assert_eq!(
                    id_errors, 0,
                    "GPU coverage or depth ownership differs from the CPU reference"
                );
                assert_eq!(node_errors, 0);
                if lighting.is_some() {
                    assert!(
                        max_color_error <= 0.001,
                        "GPU lighting exceeds the F16 sRGB shading tolerance: {max_color_error}"
                    );
                } else {
                    assert_eq!(color_errors, 0);
                }
                assert!(relative_depth_error < 1.0e-5);
                let digest: [u8; 32] = Sha256::digest(&color).into();
                if let Some(prior) = observed.insert(sample, digest) {
                    assert_eq!(prior, digest);
                }
            }
            drop(output);
            context.free_gpu_resources();
        }
    }
}
