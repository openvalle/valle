use super::*;
use crate::scene3d::raster::{
    Camera, Frame3DState, Lights, PreparedScene, collect_geometry, geometry_reuse_tests::fixture,
    prepare_gpu_geometry,
};
use crate::scene3d::{LightFrameState, Vec3};

fn subject() -> (PreparedScene, Frame3DState) {
    fixture(
        include_bytes!("../../../../../tests/fixtures/scene3d/rigged-simple.glb"),
        true,
    )
}

fn maps(prepared: &PreparedScene, frame: &Frame3DState, workers: usize) -> Arc<ShadowMaps> {
    let camera = Camera::new(frame, prepared.width, prepared.height).unwrap();
    let geometry = collect_geometry(prepared, frame, &camera).unwrap();
    let lights = Lights::new(&prepared.scene, frame, prepared.environment.clone());
    prepared.shadow_cache.render(
        &lights.directional,
        &geometry.casters,
        &geometry.frame_meshes,
        workers,
    )
}

fn key(prepared: &PreparedScene, frame: &Frame3DState) -> ContentDigest {
    let camera = Camera::new(frame, prepared.width, prepared.height).unwrap();
    let geometry = collect_geometry(prepared, frame, &camera).unwrap();
    let lights = Lights::new(&prepared.scene, frame, prepared.environment.clone());
    geometry
        .casters
        .content_key(&lights.directional, &geometry.frame_meshes)
}

fn assert_maps(a: &ShadowMaps, b: &ShadowMaps) {
    assert_eq!(a.maps.len(), b.maps.len());
    for (a, b) in a.maps.iter().zip(&b.maps) {
        assert_eq!(a.is_some(), b.is_some());
        if let (Some(a), Some(b)) = (a, b) {
            assert_eq!(a.depth, b.depth);
            assert!(a.depth.iter().any(|&z| z != f32::MAX));
        }
    }
}

#[test]
fn camera_and_shading_reuse_maps_with_one_shared_cpu_native_and_web_key() {
    let (prepared, mut frame) = subject();
    let first = maps(&prepared, &frame, 1);
    let second = maps(&prepared, &frame, 1);
    assert!(!Arc::ptr_eq(&first, &second));
    assert_maps(&first, &second);
    let before = key(&prepared, &frame);
    frame.camera.position.0[0] += 0.3;
    frame.exposure = 1.8;
    frame.meshes[0].material.roughness = Some(0.7);
    if let LightFrameState::Directional { intensity, .. } = &mut frame.lights[1] {
        *intensity = 1.2;
    }
    assert_eq!(before, key(&prepared, &frame));
    let reused = maps(&prepared, &frame, 4);
    assert!(Arc::ptr_eq(&second, &reused));
    let gpu = prepare_gpu_geometry(&prepared, &frame).unwrap().unwrap();
    assert_eq!(before, gpu.shading.as_ref().unwrap().shadow_content_key());
    let packet = gpu.web_packet(&frame).unwrap();
    assert_eq!(&packet[36 * 4..44 * 4], before.as_bytes());
    prepared.trim_shadow_cache();
    let cold = maps(&prepared, &frame, 4);
    assert!(!Arc::ptr_eq(&reused, &cold));
    assert_maps(&cold, &reused);
}

#[test]
fn bands_keep_serial_depth_bits_for_skinned_and_morphed_random_access_frames() {
    for source in [
        include_bytes!("../../../../../tests/fixtures/scene3d/rigged-simple.glb").as_slice(),
        include_bytes!("../../../../../tests/fixtures/scene3d/animated-morph-cube.glb").as_slice(),
    ] {
        let (prepared, mut frame) = fixture(source, true);
        for time in [1.0, 0.0, 0.7, 1.0] {
            frame.meshes[0].animation_time = Some(time);
            prepared.trim_shadow_cache();
            let serial = maps(&prepared, &frame, 1);
            for workers in [2, 4, 8] {
                prepared.trim_shadow_cache();
                let parallel = cpu::CpuBudget::new(workers)
                    .unwrap()
                    .run(|| maps(&prepared, &frame, workers));
                assert_maps(&serial, &parallel);
            }
        }
    }
}

#[test]
fn caster_deformation_alpha_sidedness_and_light_changes_invalidate_shadows() {
    let (mut prepared, frame) = subject();
    for material in &mut prepared.meshes[0].materials {
        material.values.alpha_mode = crate::scene3d::AlphaMode::Mask;
        material.values.alpha_cutoff = Some(0.5);
    }
    let before = key(&prepared, &frame);
    for change in 0..5 {
        let mut changed = frame.clone();
        match change {
            0 => changed.meshes[0].animation_time = Some(0.7),
            1 => changed.meshes[0].transform.translation.0[0] += 0.125,
            2 => changed.meshes[0].material.color.as_mut().unwrap().0[3] = 0.3,
            3 => changed.meshes[0].material.alpha_cutoff = Some(0.25),
            _ => {
                if let LightFrameState::Directional { direction, .. } = &mut changed.lights[1] {
                    *direction = Vec3::new(-0.3, 0.4, 1.0);
                }
            }
        }
        assert_ne!(before, key(&prepared, &changed), "change {change}");
    }
    prepared.meshes[0].materials[0].values.double_sided ^= true;
    assert_ne!(before, key(&prepared, &frame));
}

#[test]
fn alpha_texture_content_role_and_sampler_are_part_of_shadow_identity() {
    use crate::scene3d::{
        MaterialImage, MaterialTexture, MipmapFilter, TextureFilter, TextureRole, TextureWrap,
        asset::ModelTextureBinding,
    };
    let (mut prepared, frame) = subject();
    let encoded = |alpha| {
        let image = image::RgbaImage::from_fn(2, 2, |x, _| {
            image::Rgba([255, 255, 255, if x == 0 { alpha } else { 255 }])
        });
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    };
    prepared.meshes[0]
        .images
        .push(MaterialImage::from_encoded(&encoded(220), TextureRole::Color).unwrap());
    let index = prepared.meshes[0].images.len() - 1;
    let mut source = MaterialTexture {
        control: "alpha".into(),
        wrap_u: TextureWrap::Repeat,
        wrap_v: TextureWrap::Clamp,
        min_filter: TextureFilter::Nearest,
        mag_filter: TextureFilter::Nearest,
        mipmap: MipmapFilter::None,
    };
    prepared.meshes[0].materials[0].values.base_color_texture =
        Some(ModelTextureBinding::external(index, &source));
    prepared.meshes[0].materials[0].values.alpha_mode = crate::scene3d::AlphaMode::Mask;
    prepared.meshes[0].materials[0].values.alpha_cutoff = Some(0.5);
    let serial = maps(&prepared, &frame, 1);
    prepared.trim_shadow_cache();
    let parallel = cpu::CpuBudget::new(4)
        .unwrap()
        .run(|| maps(&prepared, &frame, 4));
    assert_maps(&serial, &parallel);
    let before = key(&prepared, &frame);
    source.wrap_u = TextureWrap::Mirror;
    prepared.meshes[0].materials[0].values.base_color_texture =
        Some(ModelTextureBinding::external(index, &source));
    assert_ne!(before, key(&prepared, &frame));
    let before = key(&prepared, &frame);
    prepared.meshes[0].images[index] =
        MaterialImage::from_encoded(&encoded(128), TextureRole::Color).unwrap();
    assert_ne!(before, key(&prepared, &frame));
    let before = key(&prepared, &frame);
    prepared.meshes[0].images[index] =
        MaterialImage::from_encoded(&encoded(128), TextureRole::Data).unwrap();
    assert_ne!(before, key(&prepared, &frame));
}

#[test]
fn eviction_keeps_live_leases_and_pressure_bypasses_optional_retention() {
    let (prepared, frame) = subject();
    let memory = MemoryBudget::new(Some(16 * 1024 * 1024));
    let budget = cpu::CpuBudget::new_with_memory(2, Some(memory.clone())).unwrap();
    let retained = budget.run(|| {
        drop(maps(&prepared, &frame, 2));
        maps(&prepared, &frame, 2)
    });
    assert!(memory.report().used_bytes > 4 * 1024 * 1024);
    prepared.trim_shadow_cache();
    assert!(memory.report().used_bytes > 0);
    drop(retained);
    assert_eq!(memory.report().used_bytes, 0);
    budget.run(|| {
        drop(maps(&prepared, &frame, 2));
        drop(maps(&prepared, &frame, 2));
    });
    assert!(memory.report().used_bytes > 0);
    let tiny = MemoryBudget::new(Some(1024));
    let tiny_budget = cpu::CpuBudget::new_with_memory(1, Some(tiny.clone())).unwrap();
    tiny_budget.run(|| {
        let first = maps(&prepared, &frame, 1);
        let second = maps(&prepared, &frame, 1);
        let third = maps(&prepared, &frame, 1);
        assert_maps(&first, &second);
        assert_maps(&first, &third);
        assert!(!Arc::ptr_eq(&second, &third));
    });
    assert_eq!(
        memory.report().used_bytes,
        0,
        "a different budget cannot reuse an old ledger's cache"
    );
    assert_eq!(tiny.report().used_bytes, 0);
}
