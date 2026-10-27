//! Borrow wgpu output on the same Vulkan device for Ganesh sampling.
use ash::vk::Handle;
use skia_safe::{AlphaType, ColorSpace, ColorType, Image, gpu};

pub(super) fn import_image(
    context: &mut gpu::DirectContext,
    texture: &wgpu::Texture,
    color: ColorType,
    alpha: AlphaType,
    space: Option<ColorSpace>,
) -> Result<Image, String> {
    let format = match texture.format() {
        wgpu::TextureFormat::Rgba16Float => gpu::vk::Format::R16G16B16A16_SFLOAT,
        wgpu::TextureFormat::Rgba8Unorm => gpu::vk::Format::R8G8B8A8_UNORM,
        _ => return Err("unsupported Scene3D Vulkan output format".into()),
    };
    // SAFETY: the renderer and Ganesh use the same device/queue. The producer transitions
    // the image to RESOURCE before submission. SurfaceArena retains the wgpu owner until
    // all Ganesh reads complete, including early errors, invalidation and context teardown.
    unsafe {
        let raw = texture.as_hal::<wgpu::hal::api::Vulkan>()
            .ok_or("Vulkan texture unavailable")?;
        let mut info = gpu::vk::ImageInfo::new(
            raw.raw_handle().as_raw() as _, gpu::vk::Alloc::default(),
            gpu::vk::ImageTiling::OPTIMAL, gpu::vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            format, 1, None, None, None, None,
        );
        let usage = texture.usage();
        let mut flags = ash::vk::ImageUsageFlags::SAMPLED;
        if usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT) {
            flags |= ash::vk::ImageUsageFlags::COLOR_ATTACHMENT;
        }
        if usage.contains(wgpu::TextureUsages::STORAGE_BINDING) {
            flags |= ash::vk::ImageUsageFlags::STORAGE;
        }
        if usage.contains(wgpu::TextureUsages::COPY_SRC) {
            flags |= ash::vk::ImageUsageFlags::TRANSFER_SRC;
        }
        if usage.contains(wgpu::TextureUsages::COPY_DST) {
            flags |= ash::vk::ImageUsageFlags::TRANSFER_DST;
        }
        info.image_usage_flags = flags.as_raw();
        let backend = gpu::backend_textures::make_vk(
            (texture.width() as i32, texture.height() as i32), &info, "Scene3D Vulkan",
        );
        gpu::images::borrow_texture_from(
            context, &backend, gpu::SurfaceOrigin::TopLeft, color, alpha, space,
        ).ok_or_else(|| "cannot import Scene3D Vulkan texture".into())
    }
}
