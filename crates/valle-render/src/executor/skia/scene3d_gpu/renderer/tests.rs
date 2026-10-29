//! Hardware tests exercise the production producer, including its retained asset cache.
use super::*;
use skia_safe::{Color, ImageInfo, Paint};
use std::collections::BTreeMap;
use valle_engine::motion::scene3d::*;
mod area;
mod bevel;
mod mesh;
mod orthographic;
mod punctual;
mod raster_math;

fn half(bits: u16) -> f32 {
    let exponent = (bits >> 10) & 31;
    let mantissa = bits & 1023;
    if exponent == 0 {
        f32::from(mantissa) / 16_777_216.0
    } else {
        f32::from_bits((u32::from(exponent) + 112) << 23 | u32::from(mantissa) << 13)
    }
}

fn fixture(environment: bool) -> (Arc<PreparedScene>, Frame3DState) {
    fixture_features(environment, false, false)
}

fn fixture_features(
    environment: bool,
    shadows: bool,
    blend: bool,
) -> (Arc<PreparedScene>, Frame3DState) {
    let (scene, frame, resources) = fixture_spec(environment, shadows, blend);
    (
        Arc::new(prepare_scene(&scene, 384, 256, &resources).unwrap()),
        frame,
    )
}

fn fixture_spec(
    environment: bool,
    shadows: bool,
    blend: bool,
) -> (Scene3DSpec, Frame3DState, SceneResources) {
    let model = admit_glb(include_bytes!(
        "../../../../../../valle-motion/tests/fixtures/scene3d/pbr.glb"
    ))
    .unwrap();
    let mut resources = SceneResources {
        models: BTreeMap::from([("model".into(), Arc::new(model))]),
        ..Default::default()
    };
    let mut scene = Scene3DSpec {
        meshes: vec![MeshSpec {
            key: "mesh".into(),
            model_control: Some("model".into()),
            geometry: None,
            material: MaterialSpec {
                kind: Some(MaterialKind::Pbr),
                ..Default::default()
            },
            material_overrides: vec![],
            animation_clip: None,
            node_ids: vec![],
        }],
        lights: vec![
            LightKind::Ambient,
            LightKind::Directional,
            LightKind::Hemisphere,
        ],
        anchors: vec![],
        pbr: Default::default(),
    };
    scene.pbr.tone_mapping = ToneMapping::Aces;
    if environment {
        let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 2 +X 4\n".to_vec();
        for i in 0..8 {
            bytes.extend(if i % 4 < 2 {
                [192, 24, 8, 130]
            } else {
                [8, 32, 192, 129]
            });
        }
        resources.environments.insert(
            "sky".into(),
            Arc::new(
                EnvironmentAsset::from_encoded(
                    &bytes,
                    EnvironmentSettings {
                        face_size: 16,
                        diffuse_size: 4,
                        samples: 32,
                    },
                )
                .unwrap(),
            ),
        );
        scene.pbr.environment = Some(EnvironmentSpec {
            control: "sky".into(),
            background: true,
        });
    }
    let mut frame = Frame3DState {
        exposure: 1.1,
        environment_intensity: 0.8,
        environment_rotation_degrees: 0.0,
        camera: CameraFrameState {
            position: Vec3::new(0.0, 0.0, 3.0),
            target: Vec3::ZERO,
            orthographic_height: None,
            fov_y_degrees: 45.0,
            near: 0.1,
            far: 100.0,
            depth_of_field: None,
        },
        meshes: vec![MeshFrameState {
            parents: Vec::new(),
            key: "mesh".into(),
            transform: Transform3D::default(),
            material: MaterialFrameState::default(),
            material_overrides: vec![],
            animation_time: None,
            nodes: vec![],
        }],
        lights: vec![
            LightFrameState::Ambient {
                color: Color4([0.6, 0.7, 0.8, 1.0]),
                intensity: 0.2,
            },
            LightFrameState::Directional {
                color: Color4([1.0, 0.8, 0.6, 1.0]),
                intensity: 2.5,
                direction: Vec3::new(0.6, 0.2, 1.0),
            },
            LightFrameState::Hemisphere {
                sky_color: Color4([0.6, 0.8, 1.0, 1.0]),
                ground_color: Color4([0.1, 0.15, 0.05, 1.0]),
                intensity: 0.8,
                direction: Vec3::new(0.0, 1.0, 0.0),
            },
        ],
    };
    scene.pbr.shadows = shadows;
    if shadows || blend {
        for (i, key) in ["front", "crossing"].into_iter().enumerate() {
            let mut mesh = scene.meshes[0].clone();
            mesh.key = key.into();
            mesh.material.double_sided = Some(true);
            mesh.material.alpha_mode = Some(if blend {
                AlphaMode::Blend
            } else {
                AlphaMode::Mask
            });
            scene.meshes.push(mesh);
            let mut state = frame.meshes[0].clone();
            state.key = key.into();
            state.transform.translation = Vec3::new(
                if i == 0 { -0.18 } else { 0.25 },
                0.05,
                0.6 + i as f32 * 0.15,
            );
            state.transform.scale = Vec3::new(0.6, 0.6, 0.6);
            state.transform.rotation_degrees =
                Vec3::new(0.0, if i == 0 { -20.0 } else { 25.0 }, 0.0);
            state.material.color = Some(Color4(if i == 0 {
                [0.8, 0.3, 0.1, if blend { 0.4 } else { 1.0 }]
            } else {
                [0.2, 0.5, 0.9, if blend { 0.6 } else { 1.0 }]
            }));
            state.material.alpha_cutoff = if blend { None } else { Some(0.5) };
            frame.meshes.push(state);
        }
        scene.lights.push(LightKind::Directional);
        frame.lights.push(LightFrameState::Directional {
            direction: Vec3::new(-0.5, 0.3, 1.0),
            color: Color4([0.4, 0.6, 1.0, 1.0]),
            intensity: 0.7,
        });
    }
    (scene, frame, resources)
}

#[test]
#[ignore = "requires a real GPU; exercises varying mip data, wrap/filter modes, normal maps and alpha-mask shadows"]
fn textured_mips_masks_and_samplers_match_cpu_reference() {
    let (mut rasterizer, mut context) = renderer();
    let (mut scene, mut frame, mut resources) = fixture_spec(false, true, false);
    let mut model =
        include_bytes!("../../../../../../valle-motion/tests/fixtures/scene3d/pbr.glb").to_vec();
    let json_len = u32::from_le_bytes(model[12..16].try_into().unwrap()) as usize;
    let json: serde_json::Value = serde_json::from_slice(&model[20..20 + json_len]).unwrap();
    let accessor = json["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"]
        .as_u64()
        .unwrap() as usize;
    let accessor = &json["accessors"][accessor];
    let view = accessor["bufferView"].as_u64().unwrap() as usize;
    let at = 28
        + json_len
        + json["bufferViews"][view]["byteOffset"]
            .as_u64()
            .unwrap_or(0) as usize
        + accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
    for (i, value) in [-0.4_f32, 1.8, 2.3, 1.8, 0.3, -0.4].into_iter().enumerate() {
        model[at + i * 4..at + i * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    resources
        .models
        .insert("model".into(), Arc::new(admit_glb(&model).unwrap()));
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, 16, 8);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let pixels = (0..128)
            .flat_map(|i| {
                if (i / 16 + i % 16) % 3 == 0 {
                    [64, 96, 160, 0]
                } else {
                    [192, 208, 240, 255]
                }
            })
            .collect::<Vec<u8>>();
        writer.write_image_data(&pixels).unwrap();
    }
    for role in [TextureRole::Color, TextureRole::Data] {
        resources.textures.insert(
            ("pattern".into(), role),
            Arc::new(MaterialImage::from_encoded(&encoded, role).unwrap()),
        );
    }
    for (wrap, mipmap, filter) in [
        (
            TextureWrap::Clamp,
            MipmapFilter::None,
            TextureFilter::Nearest,
        ),
        (
            TextureWrap::Repeat,
            MipmapFilter::Nearest,
            TextureFilter::Linear,
        ),
        (
            TextureWrap::Mirror,
            MipmapFilter::Linear,
            TextureFilter::Linear,
        ),
        (
            TextureWrap::Repeat,
            MipmapFilter::Linear,
            TextureFilter::Nearest,
        ),
    ] {
        for mesh in &mut scene.meshes {
            mesh.material.textures = [
                MaterialTextureSlot::BaseColor,
                MaterialTextureSlot::MetallicRoughness,
                MaterialTextureSlot::Normal,
                MaterialTextureSlot::Occlusion,
                MaterialTextureSlot::Emissive,
            ]
            .into_iter()
            .map(|slot| {
                (
                    slot,
                    Some(MaterialTexture {
                        control: "pattern".into(),
                        wrap_u: wrap,
                        wrap_v: wrap,
                        min_filter: filter,
                        mag_filter: filter,
                        mipmap,
                    }),
                )
            })
            .collect();
        }
        let prepared = Arc::new(prepare_scene(&scene, 384, 256, &resources).unwrap());
        for scale in [1.0, 0.08] {
            frame.meshes[0].transform.scale = Vec3::new(scale, scale, scale);
            frame.meshes[1].material.alpha_cutoff = Some(0.43);
            let geometry = prepare_gpu_geometry(&prepared, &frame).unwrap().unwrap();
            let input =
                Scene3dGpuInput::new(geometry, prepared.clone(), Arc::new(frame.clone())).unwrap();
            let output = rasterizer.render(&input, &mut context).unwrap().unwrap();
            let actual = read_color(&mut context, &output.image, 384, 256, ColorType::RGBAF16);
            let reference = render_scene(&prepared, &frame).unwrap();
            let mut maximum = 0.0_f32;
            for (i, (a, b)) in actual
                .as_chunks::<2>()
                .0
                .iter()
                .zip(&reference.premul_rgba16f)
                .enumerate()
            {
                let a = half(u16::from_le_bytes(*a));
                let b = half(*b);
                if i % 4 == 3 {
                    assert_eq!(
                        a,
                        b,
                        "mask coverage {wrap:?}/{mipmap:?}/{filter:?} scale={scale} pixel={}",
                        i / 4
                    );
                }
                maximum = maximum.max((a - b).abs());
            }
            eprintln!(
                "sampler {wrap:?}/{mipmap:?}/{filter:?} scale={scale}: max_f16_error={maximum}"
            );
            assert!(
                maximum <= 0.001,
                "{wrap:?}/{mipmap:?}/{filter:?} scale={scale}: {maximum}"
            );
        }
    }
}

fn read_color(
    context: &mut gpu::DirectContext,
    image: &Image,
    width: u32,
    height: u32,
    format: ColorType,
) -> Vec<u8> {
    let info = ImageInfo::new(
        (width as i32, height as i32),
        format,
        if format == ColorType::RGBA8888 {
            AlphaType::Unpremul
        } else {
            AlphaType::Premul
        },
        None,
    );
    let row = info.min_row_bytes();
    let mut bytes = vec![0; row * height as usize];
    assert!(image.read_pixels_with_context(
        context,
        &info,
        &mut bytes,
        row,
        (0, 0),
        skia_safe::image::CachingHint::Disallow
    ));
    bytes
}

#[test]
#[ignore = "requires a real GPU; verifies PCF, fixed-point OIT, geometry replay and GPU depth"]
fn shadows_and_transparency_match_cpu_reference_and_submission_order() {
    let (mut rasterizer, mut context) = renderer();
    for (environment, shadows, blend) in [
        (false, true, false),
        (false, false, true),
        (false, true, true),
        (true, true, true),
    ] {
        let (prepared, mut frame) = fixture_features(environment, shadows, blend);
        frame.camera.depth_of_field = Some(DepthOfFieldState {
            focus_distance: 3.0,
            max_blur_radius: 4.0,
        });
        let mut observed = BTreeMap::new();
        for sample in [0, 1, 2, 3, 4, 5, 75, 0, 2] {
            frame.meshes[1].transform.translation.0[0] = -0.18 + sample as f32 * 0.03;
            if blend {
                frame.meshes[2].material.color.as_mut().unwrap().0[3] =
                    [0.0, 0.000001, 0.25, 0.6, 1.0, 0.0000000001][sample % 6];
            }
            let geometry = prepare_gpu_geometry(&prepared, &frame).unwrap().unwrap();
            let input =
                Scene3dGpuInput::new(geometry, prepared.clone(), Arc::new(frame.clone())).unwrap();
            let output = rasterizer.render(&input, &mut context).unwrap().unwrap();
            let actual = read_color(&mut context, &output.image, 384, 256, ColorType::RGBAF16);
            let expected = render_scene_without_depth_of_field(&prepared, &frame).unwrap();
            let mut maximum = 0.0_f32;
            for (a, b) in actual
                .as_chunks::<2>()
                .0
                .iter()
                .zip(&expected.premul_rgba16f)
            {
                maximum = maximum.max((half(u16::from_le_bytes(*a)) - half(*b)).abs());
            }
            eprintln!("shadow={shadows} blend={blend} sample={sample}: max_f16_error={maximum}");
            assert!(
                maximum <= 0.001,
                "shadow={shadows} blend={blend} sample={sample}: {maximum}"
            );
            let depth = read_color(
                &mut context,
                output.depth_image.as_ref().unwrap(),
                384,
                256,
                ColorType::RGBA8888,
            );
            for (a, b) in depth.as_chunks::<4>().0.iter().zip(&expected.depth) {
                let a = f32::from_le_bytes(*a);
                if *b == f32::MAX {
                    assert_eq!(a, *b);
                } else {
                    assert!((a - b).abs() / b.max(1.0) <= 1e-5, "depth {a}/{b}");
                }
            }
            if let Some(previous) = observed.insert(sample, actual.clone()) {
                assert_eq!(previous, actual, "random-access replay");
            }
            if blend {
                let mut reordered = input.clone();
                let reverse = |len: usize| {
                    (0..len / 3)
                        .rev()
                        .flat_map(|i| [i * 3, i * 3 + 1, i * 3 + 2])
                        .collect::<Vec<_>>()
                };
                let order = reverse(reordered.geometry.vertices.len());
                reordered.geometry.vertices =
                    order.iter().map(|i| input.geometry.vertices[*i]).collect();
                let s = reordered.geometry.shading.as_mut().unwrap();
                let original = input.geometry.shading.as_ref().unwrap();
                s.vertices = order.iter().map(|i| original.vertices[*i]).collect();
                s.texture_vertices = Some(
                    order
                        .iter()
                        .map(|i| original.texture_vertices.as_ref().unwrap()[*i])
                        .collect(),
                );
                let output = rasterizer
                    .render(&reordered, &mut context)
                    .unwrap()
                    .unwrap();
                let other = read_color(&mut context, &output.image, 384, 256, ColorType::RGBAF16);
                assert_eq!(other, actual, "transparent submission order");
            }
        }
    }
}

#[test]
#[ignore = "requires a real GPU; verifies full shadow-cache keys, reuse and eviction"]
fn shadow_cache_tracks_casters_alpha_and_lights_and_survives_eviction() {
    let (mut rasterizer, mut context) = renderer();
    let (prepared, mut frame) = fixture_features(false, true, false);
    let run = |renderer: &mut GpuRasterizer,
               context: &mut gpu::DirectContext,
               frame: &Frame3DState| {
        let geometry = prepare_gpu_geometry(&prepared, frame).unwrap().unwrap();
        renderer
            .render(
                &Scene3dGpuInput::new(geometry, prepared.clone(), Arc::new(frame.clone())).unwrap(),
                context,
            )
            .unwrap()
            .unwrap()
    };
    let first = run(&mut rasterizer, &mut context, &frame);
    frame.camera.position.0[0] = 0.3;
    frame.exposure = 1.7;
    frame.meshes[1].material.roughness = Some(0.9);
    let reused = run(&mut rasterizer, &mut context, &frame);
    assert!(Arc::ptr_eq(
        first._shadows.as_ref().unwrap(),
        reused._shadows.as_ref().unwrap()
    ));
    frame.meshes[1].material.color.as_mut().unwrap().0[3] = 0.3;
    let alpha = run(&mut rasterizer, &mut context, &frame);
    assert!(!Arc::ptr_eq(
        alpha._shadows.as_ref().unwrap(),
        reused._shadows.as_ref().unwrap()
    ));
    frame.meshes[1].transform.translation.0[0] += 0.15;
    let moved = run(&mut rasterizer, &mut context, &frame);
    assert!(!Arc::ptr_eq(
        alpha._shadows.as_ref().unwrap(),
        moved._shadows.as_ref().unwrap()
    ));
    let LightFrameState::Directional { direction, .. } = &mut frame.lights[1] else {
        unreachable!()
    };
    *direction = Vec3::new(-0.3, 0.4, 1.0);
    let relit = run(&mut rasterizer, &mut context, &frame);
    assert!(!Arc::ptr_eq(
        relit._shadows.as_ref().unwrap(),
        moved._shadows.as_ref().unwrap()
    ));
    let expected = read_color(&mut context, &relit.image, 384, 256, ColorType::RGBAF16);
    rasterizer.trim_assets(0);
    assert!(rasterizer.shadows.is_none());
    assert!(rasterizer.atlas.is_none());
    let rebuilt = run(&mut rasterizer, &mut context, &frame);
    assert!(!Arc::ptr_eq(
        relit._shadows.as_ref().unwrap(),
        rebuilt._shadows.as_ref().unwrap()
    ));
    assert_eq!(
        read_color(&mut context, &rebuilt.image, 384, 256, ColorType::RGBAF16),
        expected
    );
    // Even the old borrowed producer remains valid while its output retains the owner.
    assert_eq!(
        read_color(&mut context, &relit.image, 384, 256, ColorType::RGBAF16),
        expected
    );
    context.set_resource_cache_limit(1024);
    let geometry = prepare_gpu_geometry(&prepared, &frame).unwrap().unwrap();
    let input = Scene3dGpuInput::new(geometry, prepared, Arc::new(frame)).unwrap();
    assert!(
        rasterizer.render(&input, &mut context).unwrap().is_none(),
        "oversized immutable shadow maps must select CPU fallback"
    );
    assert!(rasterizer.shadows.is_none());
    context.set_resource_cache_limit(64 * 1024 * 1024);
    let (mut scene, mut frame, resources) = fixture_spec(false, false, false);
    scene.pbr.shadows = true;
    scene.meshes[0].material.double_sided = Some(false);
    let LightFrameState::Directional { direction, .. } = &mut frame.lights[1] else {
        unreachable!()
    };
    *direction = Vec3::new(0.0, 0.0, -1.0);
    let prepared = Arc::new(prepare_scene(&scene, 384, 256, &resources).unwrap());
    let geometry = prepare_gpu_geometry(&prepared, &frame).unwrap().unwrap();
    assert!(
        geometry
            .shading
            .as_ref()
            .unwrap()
            .shadows
            .iter()
            .all(|s| s.vertices.is_empty())
    );
    let input = Scene3dGpuInput::new(geometry, prepared.clone(), Arc::new(frame.clone())).unwrap();
    let output = rasterizer.render(&input, &mut context).unwrap().unwrap();
    let actual = read_color(&mut context, &output.image, 384, 256, ColorType::RGBAF16);
    let reference = render_scene(&prepared, &frame).unwrap();
    for (a, b) in actual
        .as_chunks::<2>()
        .0
        .iter()
        .zip(&reference.premul_rgba16f)
    {
        assert!((half(u16::from_le_bytes(*a)) - half(*b)).abs() <= 0.001);
    }
}

#[cfg(target_os = "macos")]
fn renderer() -> (GpuRasterizer, gpu::DirectContext) {
    use objc2_metal::MTLDevice;
    use std::ptr;
    let device = objc2_metal::MTLCreateSystemDefaultDevice().expect("real Metal device");
    let queue = device.newCommandQueue().unwrap();
    // SAFETY: both contexts retain the same live Metal objects and use their exact queue.
    let backend = unsafe {
        gpu::mtl::BackendContext::new(
            ptr::from_ref(&*device).cast(),
            ptr::from_ref(&*queue).cast(),
        )
    };
    let mut context = gpu::direct_contexts::make_metal(&backend, None).unwrap();
    context.set_resource_cache_limit(64 * 1024 * 1024);
    let (device, queue) = super::super::metal::device(device, queue).unwrap();
    (GpuRasterizer::new(device, queue).unwrap(), context)
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
struct TestContext(super::super::super::vulkan::VulkanContext);
#[cfg(any(target_os = "linux", target_os = "windows"))]
impl std::ops::Deref for TestContext {
    type Target = gpu::DirectContext;
    fn deref(&self) -> &Self::Target {
        &self.0.context
    }
}
#[cfg(any(target_os = "linux", target_os = "windows"))]
impl std::ops::DerefMut for TestContext {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0.context
    }
}
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn renderer() -> (GpuRasterizer, TestContext) {
    let context = super::super::super::vulkan::VulkanContext::new().unwrap();
    let (device, queue) = context.shared_device().expect("real shared Vulkan device");
    (
        GpuRasterizer::new(device, queue).unwrap(),
        TestContext(context),
    )
}

#[test]
#[ignore = "requires a real GPU; compares five material maps, environment rotation/background and seeks"]
fn material_maps_and_environment_match_cpu_reference() {
    let (mut rasterizer, mut context) = renderer();
    for environment in [false, true] {
        let (prepared, mut frame) = fixture(environment);
        let mut observed = BTreeMap::new();
        let mut cached = None;
        for sample in [0, 7, 2, 15, 3, 0, 7] {
            frame.meshes[0].transform.rotation_degrees =
                Vec3::new(sample as f32 * 1.3, sample as f32 * 4.0, 0.0);
            frame.environment_rotation_degrees = sample as f32 * 17.0;
            frame.meshes[0].material.ior = Some(1.0 + sample as f32 * 0.08);
            frame.meshes[0].material.specular_intensity = Some(sample as f32 / 15.0);
            frame.meshes[0].material.specular_color = Some(Color4([0.9, 0.6, 0.4, 1.0]));
            frame.meshes[0].material.clearcoat = Some(sample as f32 / 15.0);
            frame.meshes[0].material.clearcoat_roughness = Some(0.2);
            let geometry = prepare_gpu_geometry(&prepared, &frame).unwrap().unwrap();
            assert_eq!(geometry.vertex_stride(), 108);
            assert_eq!(geometry.shading.as_ref().unwrap().textures.len(), 5);
            let input =
                Scene3dGpuInput::new(geometry, prepared.clone(), Arc::new(frame.clone())).unwrap();
            let output = rasterizer.render(&input, &mut context).unwrap().unwrap();
            let atlas = output._atlas.as_ref().unwrap();
            if let Some(previous) = &cached {
                assert!(Arc::ptr_eq(previous, atlas));
            }
            cached = Some(atlas.clone());
            let info = ImageInfo::new((384, 256), ColorType::RGBAF16, AlphaType::Premul, None);
            let mut surface = gpu::surfaces::render_target(
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
            surface.canvas().clear(Color::TRANSPARENT);
            // No destination color space: inspect the producer's raw sRGB F16 boundary.
            surface
                .canvas()
                .draw_image(&output.image, (0, 0), Some(&Paint::default()));
            let mut actual = vec![0; 384 * 256 * 8];
            assert!(surface.read_pixels(&info, &mut actual, 384 * 8, (0, 0)));
            let reference = render_scene(&prepared, &frame).unwrap();
            let expected = reference.premul_rgba16f_le_bytes();
            let mut maximum = 0.0_f32;
            for (i, (a, b)) in actual
                .as_chunks::<2>()
                .0
                .iter()
                .zip(expected.as_chunks::<2>().0)
                .enumerate()
            {
                let a = half(u16::from_le_bytes(*a));
                let b = half(u16::from_le_bytes(*b));
                if i % 4 == 3 {
                    assert_eq!(
                        a,
                        b,
                        "coverage env={environment} sample={sample} pixel={}",
                        i / 4
                    );
                }
                maximum = maximum.max((a - b).abs());
            }
            eprintln!("material maps env={environment} sample={sample}: max_f16_error={maximum}");
            assert!(
                maximum <= 0.001,
                "environment={environment} sample={sample}: {maximum}"
            );
            if let Some(previous) = observed.insert(sample, actual.clone()) {
                assert_eq!(previous, actual, "random access replay");
            }
        }
    }
}

#[test]
#[ignore = "requires a real GPU; verifies imported extensions and more than eight textures"]
fn extended_glb_materials_match_cpu_on_gpu() {
    use serde_json::json;
    let original = include_bytes!("../../../../../../valle-motion/tests/fixtures/scene3d/pbr.glb");
    let json_len = u32::from_le_bytes(original[12..16].try_into().unwrap()) as usize;
    let mut root: serde_json::Value = serde_json::from_slice(&original[20..20 + json_len]).unwrap();
    let mut bin = original[28 + json_len..].to_vec();
    let primitive = root["meshes"][0]["primitives"][0].clone();
    root["materials"] = json!([]);
    root["images"] = json!([]);
    root["textures"] = json!([]);
    root["meshes"] = json!([]);
    root["nodes"] = json!([]);
    root["extensionsUsed"] = json!(["KHR_materials_unlit", "KHR_materials_emissive_strength"]);
    for i in 0..12 {
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[20 + i * 18, 230 - i * 16, 60 + i * 10, 255])
                .unwrap();
        }
        bin.resize(bin.len().next_multiple_of(4), 0);
        let views = root["bufferViews"].as_array_mut().unwrap();
        let view = views.len();
        views.push(json!({"buffer":0,"byteOffset":bin.len(),"byteLength":png_bytes.len()}));
        bin.extend(png_bytes);
        root["images"]
            .as_array_mut()
            .unwrap()
            .push(json!({"bufferView":view,"mimeType":"image/png"}));
        root["textures"]
            .as_array_mut()
            .unwrap()
            .push(json!({"source":i}));
        let extensions = if i == 11 {
            json!({"KHR_materials_emissive_strength":{"emissiveStrength":4}})
        } else {
            json!({"KHR_materials_unlit":{}})
        };
        root["materials"].as_array_mut().unwrap().push(json!({
            "pbrMetallicRoughness":{"baseColorTexture":{"index":i},"metallicFactor":0},
            "emissiveFactor":if i==11 {[1,1,1]} else {[0,0,0]},
            "extensions":extensions
        }));
        let mut p = primitive.clone();
        p["material"] = json!(i);
        root["meshes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"primitives":[p]}));
        root["nodes"].as_array_mut().unwrap().push(json!({
            "mesh":i,"translation":[(i%4) as f32*0.65-0.975,(i/4) as f32*0.65-0.65,0],
            "scale":[0.24,0.24,0.24]
        }));
    }
    root["scene"] = json!(0);
    root["scenes"] = json!([{"nodes":(0..12).collect::<Vec<_>>()}]);
    root["buffers"][0]["byteLength"] = json!(bin.len());
    let mut json_bytes = serde_json::to_vec(&root).unwrap();
    json_bytes.resize(json_bytes.len().next_multiple_of(4), b' ');
    bin.resize(bin.len().next_multiple_of(4), 0);
    let mut bytes = Vec::new();
    for value in [
        0x46546c67u32,
        2,
        (28 + json_bytes.len() + bin.len()) as u32,
        json_bytes.len() as u32,
        0x4e4f534a,
    ] {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(json_bytes);
    bytes.extend((bin.len() as u32).to_le_bytes());
    bytes.extend(0x004e4942u32.to_le_bytes());
    bytes.extend(bin);
    let (mut scene, mut frame, mut resources) = fixture_spec(false, false, false);
    scene.meshes[0].material = MaterialSpec::default();
    scene.pbr.tone_mapping = ToneMapping::None;
    frame.exposure = 0.125;
    resources
        .models
        .insert("model".into(), Arc::new(admit_glb(&bytes).unwrap()));
    let prepared = Arc::new(prepare_scene(&scene, 384, 256, &resources).unwrap());
    let reference = render_scene(&prepared, &frame).unwrap();
    let geometry = prepare_gpu_geometry(&prepared, &frame).unwrap().unwrap();
    assert_eq!(geometry.shading.as_ref().unwrap().textures.len(), 12);
    let (mut rasterizer, mut context) = renderer();
    let input = Scene3dGpuInput::new(geometry, prepared, Arc::new(frame)).unwrap();
    let output = rasterizer.render(&input, &mut context).unwrap().unwrap();
    let actual = read_color(&mut context, &output.image, 384, 256, ColorType::RGBAF16);
    let mut maximum = 0.0f32;
    let mut lit = 0usize;
    let mut bright = false;
    for (a, &b) in actual
        .as_chunks::<2>()
        .0
        .iter()
        .zip(&reference.premul_rgba16f)
    {
        let a = half(u16::from_le_bytes(*a));
        let b = half(b);
        maximum = maximum.max((a - b).abs());
        lit += usize::from(a > 0.0);
        bright |= a > 0.7;
    }
    assert!(lit > 100 && bright, "nonempty exposed emission output");
    assert!(maximum < 0.01, "CPU/GPU max F16 difference {maximum}");
}

#[test]
#[ignore = "requires a real GPU; checks transformed UVs, normal bases and alpha shadows"]
fn transformed_material_textures_match_cpu_reference() {
    let (mut rasterizer, mut context) = renderer();
    let (mut scene, frame, mut resources) = fixture_spec(false, true, false);
    resources.models.insert(
        "model".into(),
        Arc::new(
            admit_glb(include_bytes!(
                "../../../../../../valle-motion/tests/fixtures/scene3d/texture-transform.glb"
            ))
            .unwrap(),
        ),
    );
    scene.meshes[0].material = MaterialSpec::default();
    let prepared = Arc::new(prepare_scene(&scene, 384, 256, &resources).unwrap());
    let reference = render_scene(&prepared, &frame).unwrap();
    let geometry = prepare_gpu_geometry(&prepared, &frame).unwrap().unwrap();
    if let Ok(directory) = std::env::var("VALLE_TEXTURE_TEST_OUTPUT") {
        let path = std::path::Path::new(&directory);
        std::fs::write(
            path.join("texture-packet.bin"),
            geometry.web_packet(&frame).unwrap(),
        )
        .unwrap();
        let atlas =
            GpuRasterAtlas::new(geometry.shading.as_ref().unwrap(), 4096, 256 * 1024 * 1024)
                .unwrap();
        let mut bytes = Vec::new();
        atlas.append_bytes(&mut bytes);
        std::fs::write(path.join("texture-atlas.bin"), bytes).unwrap();
        std::fs::write(
            path.join("texture-reference.rgba"),
            reference
                .premul_rgba16f
                .iter()
                .map(|&bits| (half(bits).clamp(0.0, 1.0) * 255.0).round() as u8)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        std::fs::write(
            path.join("texture-request.json"),
            serde_json::to_vec(
                &serde_json::json!({"width":384,"height":256,"scene":scene,"frame":frame}),
            )
            .unwrap(),
        )
        .unwrap();
    }
    let input = Scene3dGpuInput::new(geometry, prepared, Arc::new(frame)).unwrap();
    let output = rasterizer.render(&input, &mut context).unwrap().unwrap();
    let actual = read_color(&mut context, &output.image, 384, 256, ColorType::RGBAF16);
    let differences = actual
        .as_chunks::<2>()
        .0
        .iter()
        .zip(&reference.premul_rgba16f)
        .map(|(a, &b)| (half(u16::from_le_bytes(*a)) - half(b)).abs())
        .collect::<Vec<_>>();
    let maximum = differences.iter().copied().fold(0.0_f32, f32::max);
    let mean = differences.iter().sum::<f32>() / differences.len() as f32;
    assert!(
        maximum < 0.015 && mean < 0.0001,
        "transformed maps: max={maximum}, mean={mean}"
    );
    assert!(reference.object_ids.contains(&1));
}

#[test]
#[ignore = "requires a real GPU; verifies dielectric transmission and depth against CPU"]
fn dielectric_transmission_matches_cpu_reference() {
    let mut scene: Scene3DSpec = serde_json::from_str(include_str!(
        "../../../../../../valle-motion/tests/fixtures/scene3d/transmission-scene.json"
    ))
    .unwrap();
    let mut frame: Frame3DState = serde_json::from_str(include_str!(
        "../../../../../../valle-motion/tests/fixtures/scene3d/transmission-frame.json"
    ))
    .unwrap();
    let mut resources = SceneResources {
        models: BTreeMap::from([(
            "plane".into(),
            Arc::new(
                admit_glb(include_bytes!(
                    "../../../../../../valle-motion/tests/fixtures/scene3d/plane.glb"
                ))
                .unwrap(),
            ),
        )]),
        ..Default::default()
    };
    // The volume fixture has a rotated node and local Z scale; keep its optical thickness
    // imported so this also exercises instance-space -> world-space transmission.
    resources.models.insert(
        "volume".into(),
        Arc::new(
            admit_glb(include_bytes!(
                "../../../../../../valle-motion/tests/fixtures/scene3d/volume-plane.glb"
            ))
            .unwrap(),
        ),
    );
    let mut prepared = Arc::new(prepare_scene(&scene, 384, 256, &resources).unwrap());
    let (mut rasterizer, mut context) = renderer();
    for (thickness, ior, coat, height) in [
        (0.8, 1.5, 0.0, None),
        (0.8, 2.2, 0.7, None),
        (0.0, 1.0, 1.0, None),
        (0.8, 2.2, 0.7, Some(3.0)),
        (0.0, 1.0, 1.0, Some(3.0)),
        (0.8, 1.5, 0.0, None),
        (-1.0, 1.5, 0.0, None),
        (-1.0, 2.2, 0.7, Some(3.0)),
    ] {
        frame.camera.orthographic_height = height;
        frame.meshes[2].material.ior = Some(ior);
        frame.meshes[2].material.clearcoat = Some(coat);
        frame.meshes[2].material.clearcoat_roughness = Some(0.2);
        frame.meshes[2].material.specular_intensity = Some(0.8);
        if thickness < 0.0 {
            scene.meshes[2].model_control = Some("volume".into());
            frame.meshes[2].material.thickness = None;
            frame.meshes[2].transform.scale = Vec3::new(2.0, 1.4, 1.0);
            prepared = Arc::new(prepare_scene(&scene, 384, 256, &resources).unwrap());
        } else {
            frame.meshes[2].material.thickness = Some(thickness);
        }
        frame.meshes[2].material.attenuation_color = Some(Color4([0.9, 0.6, 0.4, 1.0]));
        frame.meshes[2].material.attenuation_distance = Some(1.5);
        let reference = render_scene(&prepared, &frame).unwrap();
        assert!(
            prepare_gpu_mesh_geometry(&prepared, &frame)
                .unwrap()
                .is_none()
        );
        let geometry = prepare_gpu_geometry(&prepared, &frame).unwrap().unwrap();
        if let Ok(directory) = std::env::var("VALLE_TRANSMISSION_TEST_OUTPUT") {
            let path = std::path::Path::new(&directory);
            std::fs::write(
                path.join("transmission-packet.bin"),
                geometry.web_packet(&frame).unwrap(),
            )
            .unwrap();
            let atlas =
                GpuRasterAtlas::new(geometry.shading.as_ref().unwrap(), 4096, 256 * 1024 * 1024)
                    .unwrap();
            let mut bytes = Vec::new();
            atlas.append_bytes(&mut bytes);
            std::fs::write(path.join("transmission-atlas.bin"), bytes).unwrap();
            std::fs::write(
                path.join("transmission-reference.rgba"),
                reference
                    .premul_rgba16f
                    .iter()
                    .map(|&bits| (half(bits).clamp(0.0, 1.0) * 255.0).round() as u8)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        }
        let input =
            Scene3dGpuInput::new(geometry, Arc::clone(&prepared), Arc::new(frame.clone())).unwrap();
        let output = rasterizer.render(&input, &mut context).unwrap().unwrap();
        let actual = read_color(&mut context, &output.image, 384, 256, ColorType::RGBAF16);
        let errors = actual
            .as_chunks::<2>()
            .0
            .iter()
            .zip(&reference.premul_rgba16f)
            .map(|(a, &b)| (half(u16::from_le_bytes(*a)) - half(b)).abs())
            .collect::<Vec<_>>();
        let max = errors.iter().copied().fold(0.0_f32, f32::max);
        let mean = errors.iter().sum::<f32>() / errors.len() as f32;
        assert!(
            max < 0.02 && mean < 0.0005,
            "thickness={thickness}, height={height:?}: max={max},mean={mean}"
        );
    }
}
