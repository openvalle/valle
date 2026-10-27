//! Metal device and texture interop; rendering itself is backend-independent.
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{MTLCommandQueue, MTLDevice};
use skia_safe::{AlphaType, ColorSpace, ColorType, Image, gpu};
use std::{ffi::c_void, ptr};

pub(crate) fn device(
    raw_device: Retained<ProtocolObject<dyn MTLDevice>>,
    raw_queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
) -> Result<(wgpu::Device, wgpu::Queue), String> {
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
    unsafe {
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
    .map_err(|error| error.to_string())
}

pub(super) fn import_image(
    context: &mut gpu::DirectContext,
    texture: &wgpu::Texture,
    color: ColorType,
    alpha: AlphaType,
    space: Option<ColorSpace>,
) -> Result<Image, String> {
    // SAFETY: the owning output retains the texture through the shared Ganesh completion fence.
    unsafe {
        let raw = texture
            .as_hal::<wgpu::hal::api::Metal>()
            .ok_or("Metal texture unavailable")?;
        let info = gpu::mtl::TextureInfo::new(ptr::from_ref(raw.raw_handle()).cast::<c_void>());
        let backend = gpu::backend_textures::make_mtl(
            (texture.width() as i32, texture.height() as i32),
            gpu::Mipmapped::No,
            &info,
            "Scene3D texture",
        );
        gpu::images::borrow_texture_from(
            context,
            &backend,
            gpu::SurfaceOrigin::TopLeft,
            color,
            alpha,
            space,
        )
        .ok_or_else(|| "cannot import Scene3D Metal texture".to_owned())
    }
}
