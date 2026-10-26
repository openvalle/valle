//! Deferred GPU raster input. The CPU fallback retains the same admitted scene and frame.
use super::{
    surface::{raster_surface, working_info},
    target::SkiaObjectError,
};
use skia_safe::{
    AlphaType, BlendMode, ColorSpace, ColorType, Data, ImageInfo, Paint, image::CachingHint, images,
};
use std::{collections::BTreeMap, sync::Arc};
use valle_engine::motion::scene3d::{Frame3DState, GpuRasterGeometry, PreparedScene};

#[cfg(target_os = "macos")]
mod metal;
#[cfg(target_os = "macos")]
pub(super) use metal::{GpuRasterOutput, GpuRasterizer};

#[derive(Clone, Debug)]
pub(crate) struct Scene3dGpuInput {
    pub geometry: GpuRasterGeometry,
    pub prepared: Arc<PreparedScene>,
    pub frame: Arc<Frame3DState>,
    palette: BTreeMap<[u32; 4], [f32; 4]>,
}

impl Scene3dGpuInput {
    pub(crate) fn new(
        geometry: GpuRasterGeometry,
        prepared: Arc<PreparedScene>,
        frame: Arc<Frame3DState>,
    ) -> Result<Self, SkiaObjectError> {
        let mut palette = BTreeMap::new();
        for vertex in geometry.vertices.iter().step_by(3) {
            let key = vertex.color.map(f32::to_bits);
            if palette.contains_key(&key) {
                continue;
            }
            // Unlit colors are constant over a triangle. Normalize each distinct F16 color
            // once with the same CPU Skia conversion as the reference, rather than allowing
            // GPU color-conversion rounding to change an otherwise exact opaque pixel.
            let source_info = ImageInfo::new(
                (1, 1),
                ColorType::RGBAF32,
                AlphaType::Premul,
                ColorSpace::new_srgb(),
            );
            let source_bytes = vertex
                .color
                .into_iter()
                .flat_map(f32::to_ne_bytes)
                .collect::<Vec<_>>();
            let source = images::raster_from_data(&source_info, Data::new_copy(&source_bytes), 16)
                .ok_or(SkiaObjectError::InvalidPixelPayload)?;
            let info = working_info(
                valle_engine::resource::Extent2d::new(1, 1)
                    .map_err(|_| SkiaObjectError::InvalidExtent)?,
            )
            .map_err(|_| SkiaObjectError::InvalidWorkingPayload)?;
            let mut target =
                raster_surface(&info).map_err(|_| SkiaObjectError::InvalidWorkingPayload)?;
            let mut paint = Paint::default();
            paint.set_blend_mode(BlendMode::Src);
            target.canvas().draw_image(&source, (0, 0), Some(&paint));
            let read = info.with_color_type(ColorType::RGBAF32);
            let mut bytes = [0; 16];
            if !target.image_snapshot().read_pixels(
                &read,
                &mut bytes,
                16,
                (0, 0),
                CachingHint::Disallow,
            ) {
                return Err(SkiaObjectError::InvalidWorkingPayload);
            }
            palette.insert(
                key,
                std::array::from_fn(|i| {
                    f32::from_ne_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
                }),
            );
        }
        Ok(Self {
            geometry,
            prepared,
            frame,
            palette,
        })
    }

    pub(crate) fn resident_bytes(&self) -> Option<u64> {
        // This packet can be shared by repeated frames. Each GPU submission charges
        // its own vertex buffer, so overlapping duplicates cannot share one byte lease.
        let cpu = (self.geometry.vertices.capacity() as u64).checked_mul(std::mem::size_of::<
            valle_engine::motion::scene3d::GpuRasterVertex,
        >() as u64)?;
        cpu.checked_add((self.palette.len() * 32) as u64)
    }

    pub(crate) fn upload_bytes(&self) -> Option<u64> {
        u64::try_from(self.geometry.vertices.len())
            .ok()?
            .checked_mul(GpuRasterGeometry::VERTEX_STRIDE)?
            .max(4)
            .checked_add(16)
    }

    pub(crate) fn frame_bytes(&self) -> Option<u64> {
        self.resident_bytes()?.checked_add(self.upload_bytes()?)
    }

    #[cfg(target_os = "macos")]
    pub(super) fn vertex_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(
            self.geometry.vertices.len() * GpuRasterGeometry::VERTEX_STRIDE as usize,
        );
        for vertex in &self.geometry.vertices {
            let color = self.palette[&vertex.color.map(f32::to_bits)];
            for value in vertex
                .position
                .into_iter()
                .chain([vertex.inv_z])
                .chain(color)
            {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            bytes.extend_from_slice(&vertex.object_id.to_le_bytes());
            bytes.extend_from_slice(&vertex.node_id.to_le_bytes());
        }
        bytes
    }
}
