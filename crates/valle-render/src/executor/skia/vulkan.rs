//! Headless Ganesh Vulkan ownership. No window system, swapchain or SDK is required at runtime.

use std::ffi::CStr;

use ash::{Entry, vk, vk::Handle};
use skia_safe::gpu;

use super::surface::{SurfaceError, working_info};

const API_VERSION: u32 = vk::API_VERSION_1_1;

// Field order is intentional: all Skia resources must die before the device and loader.
pub(super) struct VulkanContext {
    pub context: gpu::DirectContext,
    pub scene3d_gpu: Option<Result<Box<super::scene3d_gpu::GpuRasterizer>, String>>,
    owner: VulkanOwner,
}

enum VulkanOwner {
    Shared { device: wgpu::Device, queue: wgpu::Queue },
    RasterOnly { _device: DeviceOwner },
}

struct InstanceOwner {
    instance: ash::Instance,
    _entry: Entry,
}

impl Drop for InstanceOwner {
    fn drop(&mut self) {
        // SAFETY: DeviceOwner is dropped before the last instance owner.
        unsafe { self.instance.destroy_instance(None) };
    }
}

struct DeviceOwner {
    device: ash::Device,
    _instance: std::sync::Arc<InstanceOwner>,
}

impl Drop for DeviceOwner {
    fn drop(&mut self) {
        // SAFETY: the context has released its resources; the queue belongs solely to it.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
        }
    }
}

impl Drop for VulkanContext {
    fn drop(&mut self) {
        self.context.flush_submit_and_sync_cpu();
        self.context.release_resources_and_abandon();
    }
}

impl VulkanContext {
    pub fn new() -> Result<Self, SurfaceError> {
        match Self::shared() {
            Ok(context) => return Ok(context),
            Err(error) if std::env::var_os("VALLE_PERF").is_some() => {
                eprintln!("[valle vulkan] shared Scene3D device unavailable: {error}");
            }
            Err(_) => {}
        }
        // Preserve phase-one 2D acceleration on hardware below wgpu's device limits.
        Self::raster_only()
    }

    pub fn shared_device(&self) -> Option<(wgpu::Device, wgpu::Queue)> {
        match &self.owner {
            VulkanOwner::Shared { device, queue } => Some((device.clone(), queue.clone())),
            VulkanOwner::RasterOnly { .. } => None,
        }
    }

    fn shared() -> Result<Self, SurfaceError> {
        let started = std::time::Instant::now();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let mut adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN));
        adapters.retain(|adapter| matches!(adapter.get_info().device_type,
            wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::IntegratedGpu | wgpu::DeviceType::VirtualGpu));
        adapters.sort_by_key(|adapter| match adapter.get_info().device_type {
            wgpu::DeviceType::DiscreteGpu => 0,
            wgpu::DeviceType::IntegratedGpu => 1,
            _ => 2,
        });
        let mut last_error = unavailable("no hardware wgpu Vulkan adapter");
        for adapter in adapters {
            let result = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .map_err(|error| unavailable(format!("create shared Vulkan device: {error}")))
                .and_then(|(device, queue)| Self::from_wgpu(device, queue));
            match result {
                Ok(context) => {
                    if std::env::var_os("VALLE_PERF").is_some() {
                        eprintln!("[valle vulkan] device={} shared-wgpu initialize={:.3}ms",
                            adapter.get_info().name, started.elapsed().as_secs_f64() * 1000.0);
                    }
                    return Ok(context);
                }
                Err(error) => last_error = error,
            }
        }
        Err(last_error)
    }

    fn from_wgpu(device: wgpu::Device, queue: wgpu::Queue) -> Result<Self, SurfaceError> {
        let context = {
            // SAFETY: wgpu owns the loader, instance and device. They are retained in owner,
            // after Ganesh and every imported image. Only this executor submits to the queue.
            let raw = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }
                .ok_or_else(|| unavailable("missing wgpu Vulkan device"))?;
            let instance = raw.shared_instance();
            let get_proc = |of| {
                let function = unsafe {
                    match of {
                        gpu::vk::GetProcOf::Instance(handle, name) => instance.entry()
                            .get_instance_proc_addr(vk::Instance::from_raw(handle as u64), name),
                        gpu::vk::GetProcOf::Device(handle, name) => instance.raw_instance()
                            .get_device_proc_addr(vk::Device::from_raw(handle as u64), name),
                    }
                };
                function.map_or(std::ptr::null(), |function| function as *const std::ffi::c_void)
            };
            let instance_extensions: Vec<_> = instance.extensions().iter()
                .filter_map(|name| name.to_str().ok()).collect();
            let device_extensions: Vec<_> = raw.enabled_device_extensions().iter()
                .filter_map(|name| name.to_str().ok()).collect();
            let backend = unsafe {
                gpu::vk::BackendContext::new_builder(
                    instance.raw_instance().handle().as_raw() as _,
                    raw.raw_physical_device().as_raw() as _, raw.raw_device().handle().as_raw() as _,
                    (raw.raw_queue().as_raw() as _, raw.queue_family_index() as usize),
                    &get_proc, Some(instance.instance_api_version().into()),
                ).with_extensions(&instance_extensions, &device_extensions).build()
            };
            gpu::direct_contexts::make_vulkan(&backend, None)
                .ok_or_else(|| unavailable("cannot create Ganesh on wgpu Vulkan device"))?
        };
        let mut result = Self {
            context, scene3d_gpu: None, owner: VulkanOwner::Shared { device, queue },
        };
        result.probe()?;
        Ok(result)
    }

    fn raster_only() -> Result<Self, SurfaceError> {
        let started = std::time::Instant::now();
        // SAFETY: ash owns the dynamically loaded library until InstanceOwner drops.
        let entry = unsafe { Entry::load() }
            .map_err(|error| unavailable(format!("cannot load Vulkan driver: {error}")))?;
        let app = vk::ApplicationInfo::default()
            .application_name(c"Valle")
            .api_version(API_VERSION);
        // No presentation or optional device extensions are needed for offscreen Ganesh.
        let info = vk::InstanceCreateInfo::default().application_info(&app);
        let instance = unsafe { entry.create_instance(&info, None) }
            .map_err(|error| unavailable(format!("create Vulkan 1.1 instance: {error}")))?;
        let owner = std::sync::Arc::new(InstanceOwner { instance, _entry: entry });
        let instance = &owner.instance;
        let physical_devices = unsafe { instance.enumerate_physical_devices() }
            .map_err(|error| unavailable(format!("enumerate GPUs: {error}")))?;
        let mut candidates = Vec::new();
        for physical in physical_devices {
            // SAFETY: handles were enumerated from the live instance.
            let properties = unsafe { instance.get_physical_device_properties(physical) };
            let rank = match properties.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 0,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
                vk::PhysicalDeviceType::VIRTUAL_GPU => 2,
                // Software Vulkan must not masquerade as hardware acceleration in auto mode.
                _ => continue,
            };
            if properties.api_version < API_VERSION {
                continue;
            }
            let required = vk::FormatFeatureFlags::COLOR_ATTACHMENT
                | vk::FormatFeatureFlags::COLOR_ATTACHMENT_BLEND
                | vk::FormatFeatureFlags::SAMPLED_IMAGE
                | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR
                | vk::FormatFeatureFlags::TRANSFER_SRC
                | vk::FormatFeatureFlags::TRANSFER_DST;
            if ![vk::Format::R16G16B16A16_SFLOAT, vk::Format::R8G8B8A8_UNORM]
                .into_iter()
                .all(|format| unsafe {
                    instance.get_physical_device_format_properties(physical, format)
                }.optimal_tiling_features.contains(required))
            {
                continue;
            }
            let families = unsafe { instance.get_physical_device_queue_family_properties(physical) };
            if let Some(index) = families.iter().position(|family| {
                family.queue_count > 0 && family.queue_flags.contains(vk::QueueFlags::GRAPHICS)
            }) {
                candidates.push((rank, physical, index as u32, properties));
            }
        }
        candidates.sort_by_key(|candidate| candidate.0);
        let mut last_error = unavailable("no hardware GPU supports Vulkan 1.1 with F16 rendering");
        for (_, physical, family, properties) in candidates {
            match Self::create(owner.clone(), physical, family) {
                Ok(context) => {
                    if std::env::var_os("VALLE_PERF").is_some() {
                        let name = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) };
                        eprintln!("[valle vulkan] device={} initialize={:.3}ms",
                            name.to_string_lossy(), started.elapsed().as_secs_f64() * 1000.0);
                    }
                    return Ok(context);
                }
                Err(error) => last_error = error,
            }
        }
        Err(last_error)
    }

    fn create(
        instance: std::sync::Arc<InstanceOwner>,
        physical: vk::PhysicalDevice,
        family: u32,
    ) -> Result<Self, SurfaceError> {
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        let info = vk::DeviceCreateInfo::default().queue_create_infos(&queues);
        let device = unsafe { instance.instance.create_device(physical, &info, None) }
            .map_err(|error| unavailable(format!("create graphics device: {error}")))?;
        let owner = DeviceOwner { device, _instance: instance };
        let instance = &owner._instance;
        let queue = unsafe { owner.device.get_device_queue(family, 0) };
        let get_proc = |of| {
            // Skia resolves these functions synchronously while constructing its context.
            let function = unsafe {
                match of {
                    gpu::vk::GetProcOf::Instance(handle, name) => instance._entry
                        .get_instance_proc_addr(vk::Instance::from_raw(handle as u64), name),
                    gpu::vk::GetProcOf::Device(handle, name) => instance.instance
                        .get_device_proc_addr(vk::Device::from_raw(handle as u64), name),
                }
            };
            function.map_or(std::ptr::null(), |function| function as *const std::ffi::c_void)
        };
        // SAFETY: DeviceOwner outlives Ganesh and every surface/image from this context.
        // Cap the API before Skia constructs its allocator, matching VkApplicationInfo.
        let backend = unsafe {
            gpu::vk::BackendContext::new_builder(
                instance.instance.handle().as_raw() as _,
                physical.as_raw() as _,
                owner.device.handle().as_raw() as _,
                (queue.as_raw() as _, family as usize),
                &get_proc,
                Some(API_VERSION.into()),
            ).build()
        };
        let context = gpu::direct_contexts::make_vulkan(&backend, None)
            .ok_or_else(|| unavailable("cannot create Ganesh Vulkan context"))?;
        drop(backend);
        let mut result = Self {
            context, scene3d_gpu: None, owner: VulkanOwner::RasterOnly { _device: owner },
        };
        result.probe()?;
        Ok(result)
    }

    fn probe(&mut self) -> Result<(), SurfaceError> {
        self.context.set_resource_cache_limit(512 * 1024 * 1024);
        // Probe the actual working format and a synchronized readback before advertising Vulkan.
        // An installed loader alone is not evidence that rendering works.
        let info = working_info(valle_engine::resource::Extent2d::new(1, 1).unwrap())?;
        let mut surface = gpu::surfaces::render_target(
            &mut self.context, gpu::Budgeted::Yes, &info, None,
            gpu::SurfaceOrigin::TopLeft, None, false, false,
        ).ok_or_else(|| unavailable("cannot allocate F16 render target"))?;
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        let mut pixels = [0_u8; 8];
        if !surface.read_pixels(&info, &mut pixels, 8, (0, 0)) {
            return Err(unavailable("F16 render target readback failed"));
        }
        Ok(())
    }
}

fn unavailable(reason: impl Into<String>) -> SurfaceError {
    SurfaceError::VulkanUnavailable(reason.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a hardware Vulkan GPU"]
    fn skia_only_device_remains_available_without_wgpu() {
        let mut context = VulkanContext::raster_only().unwrap();
        assert!(context.shared_device().is_none());
        assert!(context.scene3d_gpu.is_none());
        context.probe().unwrap();
    }

    #[test]
    #[ignore = "requires a hardware Vulkan GPU"]
    fn effect_passes_round_f16_to_nearest_even() {
        let mut context = VulkanContext::new().unwrap();
        let info = working_info(valle_engine::resource::Extent2d::new(1, 1).unwrap()).unwrap();
        let mut surface = gpu::surfaces::render_target(
            &mut context.context, gpu::Budgeted::Yes, &info, None,
            gpu::SurfaceOrigin::TopLeft, None, false, false,
        ).unwrap();
        let effect = skia_safe::RuntimeEffect::make_for_shader(
            super::super::f16_gpu::rounded_shader(
                "uniform float4 value; half4 main(float2 xy) { return half4(value); }"
            ), None,
        ).unwrap();
        for (values, expected) in [
            ([1.0006_f32, 0.5003, 0.25015, 1.0], [0x3c01, 0x3801, 0x3401, 0x3c00]),
            ([1.0 + 1.0 / 2048.0, 1.0 + 3.0 / 2048.0, -1.0006, 1.0], [0x3c00, 0x3c02, 0xbc01, 0x3c00]),
            ([0.0, 1.0 / 33_554_432.0, 3.0 / 33_554_432.0, 1.0], [0, 0, 2, 0x3c00]),
        ] {
            let bytes: Vec<_> = values.into_iter().flat_map(f32::to_ne_bytes).collect();
            let shader = effect.make_shader(skia_safe::Data::new_copy(&bytes), &[], None).unwrap();
            let mut paint = skia_safe::Paint::default();
            paint.set_shader(shader);
            paint.set_blend_mode(skia_safe::BlendMode::Src);
            surface.canvas().draw_paint(&paint);
            let mut pixels = [0_u8; 8];
            assert!(surface.read_pixels(&info, &mut pixels, 8, (0, 0)));
            let bits: Vec<_> = pixels.chunks_exact(2)
                .map(|bytes| u16::from_ne_bytes([bytes[0], bytes[1]])).collect();
            assert_eq!(bits, expected, "{values:?}");
        }
    }
}
