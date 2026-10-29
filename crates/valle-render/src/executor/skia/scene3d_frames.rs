//! Current-frame material inputs are resolved after the compositor supplies their pixels.
//! A cached request retains only immutable topology/assets, never a previous host picture.
use super::{SkiaExternalObject, draw::DrawError, surface::SurfaceFrame};
use skia_safe::Image;
use std::{collections::BTreeMap, sync::Arc};
use valle_draw::requirements::HostFrame;
use valle_engine::{
    motion::{
        SealedScene3DFrame,
        memory::{self, MemoryKind},
        scene3d::{self, MaterialImage, SceneResources, TextureRole},
    },
    resource::{Extent2d, ResourceKey},
};

pub(crate) struct Scene3dFrames {
    pub key: ResourceKey,
    pub frame: SealedScene3DFrame,
    pub resources: SceneResources,
}
impl Scene3dFrames {
    pub fn render(
        &self,
        frames: &BTreeMap<HostFrame, Image>,
        surfaces: &mut SurfaceFrame<'_>,
    ) -> Result<(Image, bool, u64), DrawError> {
        let error = |e: String| DrawError::Internal(format!("Scene3D frame material: {e}"));
        let request = self.frame.request();
        let mut resources = self.resources.clone();
        let mut leases = Vec::new();
        for frame in request.scene.input_frames() {
            let image = frames
                .get(&frame)
                .ok_or_else(|| DrawError::MissingTexture(frame.source().into()))?;
            let pixels = image.width() as u64 * image.height() as u64;
            if pixels * 16 > scene3d::MAX_TEXTURE_STORAGE_BYTES
                || image.width() > 4096
                || image.height() > 4096
            {
                return Err(error(
                    "host texture exceeds the material image budget".into(),
                ));
            }
            leases.push(
                memory::reserve(MemoryKind::Temporary, pixels * 64)
                    .map_err(|e| error(e.to_string()))?,
            );
            let bytes = surfaces
                .read_scene_frame(image)
                .map_err(|e| error(e.to_string()))?;
            let texture = MaterialImage::from_working_rgba32f(
                image.width() as u32,
                image.height() as u32,
                &bytes,
            )
            .map_err(|e| error(e.to_string()))?;
            resources.textures.insert(
                (frame.source().into(), TextureRole::Color),
                Arc::new(texture),
            );
        }
        let _working = memory::reserve(
            MemoryKind::Temporary,
            u64::from(request.width) * u64::from(request.height) * 96,
        )
        .map_err(|e| error(e.to_string()))?;
        let prepared = Arc::new(
            scene3d::prepare_scene(&request.scene, request.width, request.height, &resources)
                .map_err(|e| error(e.to_string()))?,
        );
        #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
        if surfaces.backend_kind().supports_scene3d_gpu()
            && let Some(geometry) = scene3d::prepare_gpu_geometry(&prepared, &request.frame)
                .map_err(|e| error(e.to_string()))?
        {
            let object = SkiaExternalObject::scene3d_gpu(
                self.key.clone(),
                geometry,
                Arc::clone(&prepared),
                Arc::new(request.frame.clone()),
            )
            .map_err(|e| error(e.to_string()))?;
            if let Some((image, bytes)) = surfaces
                .render_scene3d_gpu(&object)
                .map_err(|e| error(e.to_string()))?
            {
                return Ok((image, true, bytes));
            }
        }
        let raster =
            scene3d::render_scene(&prepared, &request.frame).map_err(|e| error(e.to_string()))?;
        let extent =
            Extent2d::new(request.width, request.height).map_err(|e| error(e.to_string()))?;
        let object =
            SkiaExternalObject::scene3d_rgba16f(self.key.clone(), extent, &raster.premul_rgba16f)
                .map_err(|e| error(e.to_string()))?;
        Ok((
            object.visual_image().unwrap().clone(),
            false,
            u64::from(request.width) * u64::from(request.height) * 8,
        ))
    }
}
