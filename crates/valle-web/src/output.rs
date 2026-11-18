//! GPU output for WebCodecs. Both NV12 planes share one mapped buffer and one padded row stride.

use valle_engine::gpu::Gpu;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct VideoOutput {
    planes: [wgpu::TextureView; 2],
    staging: wgpu::Buffer,
    #[wasm_bindgen(readonly)]
    pub stride: u32,
}

impl VideoOutput {
    pub fn new(device: &wgpu::Device, [width, height]: [u32; 2]) -> Self {
        let [y, uv] = Gpu::PLANES;
        let chroma_width = width.div_ceil(2);
        let chroma_height = height.div_ceil(2);
        let stride = (2 * chroma_width).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let planes =
            [(width, height, y), (chroma_width, chroma_height, uv)].map(|(w, h, format)| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("video output plane"),
                        size: wgpu::Extent3d {
                            width: w,
                            height: h,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    })
                    .create_view(&Default::default())
            });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("NV12 readback"),
            size: u64::from(stride) * u64::from(height + chroma_height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            planes,
            staging,
            stride,
        }
    }

    pub fn views(&self) -> [&wgpu::TextureView; 2] {
        self.planes.each_ref()
    }

    pub fn copy(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut offset = 0;
        for view in &self.planes {
            let texture = view.texture();
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &self.staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(self.stride),
                        rows_per_image: None,
                    },
                },
                texture.size(),
            );
            offset += u64::from(self.stride) * u64::from(texture.height());
        }
    }
}

#[wasm_bindgen]
impl VideoOutput {
    /// The worker maps this directly, keeping pixel data outside the WASM heap.
    #[wasm_bindgen(getter)]
    pub fn buffer(&self) -> wgpu::webgpu::GpuBuffer {
        self.staging
            .as_webgpu()
            .expect("browser WebGPU buffer")
            .clone()
    }
}
