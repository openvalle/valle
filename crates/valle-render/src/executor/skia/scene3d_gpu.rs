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

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
mod assets;
#[cfg(target_os = "macos")]
pub(super) mod metal;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
mod renderer;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
mod shadows;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
mod transparency;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod vulkan;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub(super) use renderer::{GpuRasterOutput, GpuRasterizer};

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
        for vertex in geometry
            .vertices
            .iter()
            .step_by(3)
            .filter(|_| geometry.shading.is_none())
        {
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
        let shading = self.geometry.shading.as_ref().map_or(0, |shading| {
            shading.vertices.capacity()
                    * std::mem::size_of::<valle_engine::motion::scene3d::GpuRasterVarying>()
                    + shading.values.capacity() * 16
                    + shading.texture_vertices.as_ref().map_or(0, |v| {
                        v.capacity()
                            * std::mem::size_of::<
                                valle_engine::motion::scene3d::GpuRasterTextureVarying,
                            >()
                    })
                    + shading
                        .shadows
                        .iter()
                        .map(|s| {
                            s.vertices.capacity()
                                * std::mem::size_of::<
                                    valle_engine::motion::scene3d::GpuRasterShadowVertex,
                                >()
                        })
                        .sum::<usize>()
                    + shading.shadows.capacity()
                        * std::mem::size_of::<valle_engine::motion::scene3d::GpuRasterShadow>()
        });
        cpu.checked_add((self.palette.len() * 32) as u64)?
            .checked_add(shading as u64)
    }

    pub(crate) fn upload_bytes(&self) -> Option<u64> {
        u64::try_from(self.geometry.vertices.len())
            .ok()?
            .checked_mul(self.geometry.vertex_stride())?
            .max(4)
            .checked_add(16)?
            .checked_add(
                self.geometry
                    .shading
                    .as_ref()
                    .map_or(0, |s| s.values.len() as u64 * 16),
            )
    }

    pub(crate) fn frame_bytes(&self) -> Option<u64> {
        self.resident_bytes()?
            .checked_add(self.upload_bytes()?)?
            .checked_add(self.geometry.shading.as_ref().map_or(0, |s| {
                s.shadows
                    .iter()
                    .map(|s| (s.vertices.len() as u64 * 28).max(4) + 16)
                    .sum::<u64>()
            }))
    }

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    pub(crate) fn texture_bytes(&self) -> u64 {
        let dof = self
            .frame
            .camera
            .depth_of_field
            .is_some_and(|d| d.max_blur_radius > 0.0);
        u64::from(self.geometry.width)
            * u64::from(self.geometry.height)
            * if self.has_blend() {
                if dof { 68 } else { 64 }
            } else if dof {
                16
            } else {
                12
            }
            + if self.has_blend() && !dof { 4 } else { 0 }
    }

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    pub(crate) fn has_blend(&self) -> bool {
        self.geometry.shading.as_ref().is_some_and(|s| {
            (0..s.values[1][0] as usize).any(|i| {
                s.values[valle_engine::motion::scene3d::GpuRasterShading::HEADER_LEN
                    + i * valle_engine::motion::scene3d::GpuRasterShading::MATERIAL_STRIDE
                    + 2][3]
                    != 0.0
            })
        })
    }

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    pub(super) fn vertex_bytes(&self) -> Vec<u8> {
        let mut bytes = self.geometry.vertex_bytes();
        if self.geometry.shading.is_some() {
            return bytes;
        }
        for (record, vertex) in bytes
            .as_chunks_mut::<{ GpuRasterGeometry::VERTEX_STRIDE as usize }>()
            .0
            .iter_mut()
            .zip(&self.geometry.vertices)
        {
            let color = self.palette[&vertex.color.map(f32::to_bits)];
            for (channel, value) in color.into_iter().enumerate() {
                record[12 + channel * 4..16 + channel * 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        // Picking keeps its CPU metadata. Reuse the unlit upload's object-id word for the
        // exact area reciprocal, rather than adding another vertex field or buffer.
        for triangle in bytes.as_chunks_mut::<108>().0 {
            let point = |at: usize| {
                [at, at + 4].map(|i| {
                    (f32::from_le_bytes(triangle[i..i + 4].try_into().unwrap()) * 256.0) as i64
                })
            };
            let [a, b, c] = [0, 36, 72].map(point);
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            for at in [28, 64, 100] {
                triangle[at..at + 4].copy_from_slice(&(1.0 / area as f32).to_le_bytes());
            }
        }
        bytes
    }
}
