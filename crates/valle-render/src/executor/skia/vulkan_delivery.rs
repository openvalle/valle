//! Bounded asynchronous GPU output. The final CPU readback remains explicit: a software encoder
//! consumes the mapped copy while the same Vulkan queue is rendering subsequent frames.

use std::sync::{Arc, Mutex};

use ash::vk::Handle;
use skia_safe::{ColorType, Surface, gpu};
use valle_engine::resource::OutputSpec;
use valle_media::frame::RgbaFrame;

use super::{SkiaExternalObject, scene3d_gpu::GpuRasterOutput, surface::SurfaceError};

pub(crate) struct VulkanOutput {
    // Skia borrows the texture. Destroy the surface first, including on early errors.
    pub surface: Surface,
    context: gpu::DirectContext,
    texture: wgpu::Texture,
    buffer: wgpu::Buffer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    stride: u32,
    family: u32,
    submitted: bool,
}

pub(crate) struct SubmittedVulkanOutput {
    output: VulkanOutput,
    submission: wgpu::SubmissionIndex,
    mapped: Arc<Mutex<Option<Result<(), wgpu::BufferAsyncError>>>>,
    // Keep every raw Skia import alive until the copy behind Ganesh has completed.
    _imports: Vec<(GpuRasterOutput, SkiaExternalObject)>,
}

fn failure(error: impl std::fmt::Display) -> SurfaceError {
    SurfaceError::VulkanUnavailable(format!("output delivery: {error}"))
}

impl VulkanOutput {
    pub(super) fn new(
        context: &mut super::vulkan::VulkanContext,
        width: u32,
        height: u32,
        output: OutputSpec,
    ) -> Result<Self, SurfaceError> {
        let (device, queue) = context.shared_device()
            .ok_or_else(|| failure("shared wgpu device unavailable"))?;
        let stride = width.checked_mul(4).and_then(|n| n.checked_add(255))
            .map(|n| n / 256 * 256).ok_or_else(|| failure("row size overflow"))?;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Valle Vulkan output"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // Establish wgpu's tracked state before Ganesh borrows the texture. Skia returns it in
        // precisely this layout, so the later wgpu copy can transition it without discarding it.
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        // A state transition alone does not mark the texture initialized in wgpu. A real clear
        // prevents its safety initialization from overwriting Ganesh's pixels before readback.
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Initialize external Vulkan output"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view, depth_slice: None, resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        }));
        queue.submit([encoder.finish()]);
        let family;
        let surface = unsafe {
            let raw_device = device.as_hal::<wgpu::hal::api::Vulkan>()
                .ok_or_else(|| failure("missing Vulkan device"))?;
            family = raw_device.queue_family_index();
            let raw = texture.as_hal::<wgpu::hal::api::Vulkan>()
                .ok_or_else(|| failure("missing Vulkan texture"))?;
            let mut info = gpu::vk::ImageInfo::new(
                raw.raw_handle().as_raw() as _, gpu::vk::Alloc::default(),
                gpu::vk::ImageTiling::OPTIMAL, gpu::vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                gpu::vk::Format::R8G8B8A8_UNORM, 1, None, None, None, None,
            );
            info.image_usage_flags = (ash::vk::ImageUsageFlags::COLOR_ATTACHMENT
                | ash::vk::ImageUsageFlags::TRANSFER_SRC | ash::vk::ImageUsageFlags::TRANSFER_DST).as_raw();
            info.current_queue_family = family;
            let target = gpu::backend_render_targets::make_vk((width as i32, height as i32), &info);
            gpu::surfaces::wrap_backend_render_target(
                &mut context.context, &target, gpu::SurfaceOrigin::TopLeft,
                ColorType::RGBA8888, super::output::output_color_space(output)
                    .map_err(failure)?, None,
            ).ok_or_else(|| failure("cannot wrap output texture"))?
        };
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Valle Vulkan asynchronous readback"),
            size: u64::from(stride) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Ok(Self { surface, context: context.context.clone(), texture, buffer, device, queue,
            stride, family, submitted: false })
    }

    pub fn submit(
        mut self,
        context: &mut gpu::DirectContext,
        imports: Vec<(GpuRasterOutput, SkiaExternalObject)>,
    ) -> Result<SubmittedVulkanOutput, SurfaceError> {
        let state = gpu::vk::mutable_texture_states::new_vulkan(
            gpu::vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL, self.family,
        );
        context.flush_surface_with_texture_state(&mut self.surface, &gpu::FlushInfo::default(), Some(&state));
        if !context.submit(gpu::SyncCpu::No) {
            // Imported resources cannot be released while Ganesh still references them.
            context.flush_submit_and_sync_cpu();
            return Err(failure("Ganesh submission failed"));
        }
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0, bytes_per_row: Some(self.stride), rows_per_image: None,
                },
            },
            self.texture.size(),
        );
        let submission = self.queue.submit([encoder.finish()]);
        let mapped = Arc::new(Mutex::new(None));
        let callback = mapped.clone();
        self.buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            *callback.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
        });
        self.submitted = true;
        Ok(SubmittedVulkanOutput { output: self, submission, mapped, _imports: imports })
    }
}

impl Drop for VulkanOutput {
    fn drop(&mut self) {
        if !self.submitted {
            self.context.flush_submit_and_sync_cpu();
        }
    }
}

impl SubmittedVulkanOutput {
    pub fn ready(&self) -> Result<bool, SurfaceError> {
        self.output.device.poll(wgpu::PollType::Poll).map_err(failure)?;
        Ok(self.mapped.lock().unwrap_or_else(|e| e.into_inner()).is_some())
    }

    pub fn read(&self) -> Result<RgbaFrame, SurfaceError> {
        let profile = std::env::var_os("VALLE_PERF_READBACK").is_some();
        let started = std::time::Instant::now();
        let result = self.mapped.lock().unwrap_or_else(|e| e.into_inner());
        match result.as_ref() {
            Some(Ok(())) => {},
            Some(Err(error)) => return Err(failure(error)),
            None => return Err(failure("readback is not complete")),
        }
        let width = self.output.texture.width();
        let height = self.output.texture.height();
        let bytes = self.output.buffer.slice(..).get_mapped_range().map_err(failure)?;
        let mapped = std::time::Instant::now();
        let mut frame = RgbaFrame::new(width, height);
        let allocated = std::time::Instant::now();
        for (source, target) in bytes.chunks_exact(self.output.stride as usize)
            .zip(frame.data.chunks_exact_mut(width as usize * 4)) {
            target.copy_from_slice(&source[..target.len()]);
        }
        if profile {
            eprintln!("[valle vulkan readback] map-us={} allocate-us={} copy-us={} bytes={}",
                mapped.duration_since(started).as_micros(),
                allocated.duration_since(mapped).as_micros(),
                allocated.elapsed().as_micros(), frame.data.len());
        }
        Ok(frame)
    }
}

impl Drop for SubmittedVulkanOutput {
    fn drop(&mut self) {
        // Error, cancellation and panic paths must retire borrowed Skia/Scene3D resources too.
        // A normal completed delivery returns immediately from this wait.
        let _ = self.output.device.poll(wgpu::PollType::Wait {
            submission_index: Some(self.submission.clone()), timeout: None,
        });
        self.output.buffer.unmap();
    }
}
