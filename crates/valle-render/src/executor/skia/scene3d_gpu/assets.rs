//! One immutable float32 atlas for a scene's admitted mip chains and environment cubes.
use std::sync::Arc;
use valle_engine::{
    motion::{
        memory,
        scene3d::{GpuRasterAtlas, GpuRasterShading},
    },
    resource::ContentDigest,
};

pub(super) struct Atlas {
    pub key: ContentDigest,
    pub texture: wgpu::Texture,
    pub bytes: u64,
    _memory: Option<memory::MemoryLease>,
}

impl Atlas {
    pub fn key(shading: &GpuRasterShading) -> ContentDigest {
        ContentDigest::from_bytes(GpuRasterAtlas::key(shading))
    }

    pub fn build(
        shading: &GpuRasterShading,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        limit: usize,
    ) -> Result<Option<Arc<Self>>, String> {
        let Some(layout) = GpuRasterAtlas::new(
            shading,
            device.limits().max_texture_dimension_2d,
            limit as u64,
        ) else {
            return Ok(None);
        };
        let width = layout.width;
        let height = layout.height;
        let bytes = layout.byte_len();
        let Ok(memory) = memory::reserve_retained(memory::MemoryKind::Resource, bytes) else {
            return Ok(None);
        };
        let Ok(_staging) = memory::reserve_retained(memory::MemoryKind::Temporary, bytes) else {
            return Ok(None);
        };
        let mut data = Vec::with_capacity(bytes as usize);
        layout.append_bytes(&mut data);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Scene3D immutable linear mip atlas"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 16),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Ok(Some(Arc::new(Self {
            key: Self::key(shading),
            texture,
            bytes,
            _memory: memory,
        })))
    }
}
