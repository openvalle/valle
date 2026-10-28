//! Explicit diagnostic readback of the working image, before any delivery transform.
use super::{
    SkiaExecuteError,
    surface::{PlanImage, SurfaceFrame, working_info},
};
use valle_engine::{
    motion::memory::{self, MemoryKind, MemoryLease},
    resource::Extent2d,
};

/// Raw linear Rec.2020, premultiplied RGBA binary16, little-endian and tightly packed.
/// HDR values and negative channels are preserved. This is not a delivery image.
#[derive(Debug)]
pub struct WorkingF16Capture {
    pub width: u32,
    pub height: u32,
    pub rgba16f_le: Vec<u8>,
    _memory: Option<MemoryLease>,
}

pub(super) fn capture(
    source: &PlanImage,
    extent: Extent2d,
    surfaces: &mut SurfaceFrame<'_>,
) -> Result<WorkingF16Capture, SkiaExecuteError> {
    let bytes = u64::from(extent.width()) * u64::from(extent.height()) * 8;
    // Diagnostic storage is additional to the admitted production frame. Retaining the
    // result must not borrow the frame's credit or silently escape the host ledger.
    let memory = memory::reserve_retained(MemoryKind::Delivery, bytes)
        .map_err(|error| SkiaExecuteError::Program(format!("working capture: {error}")))?;
    let _readback = memory::reserve_retained(MemoryKind::Temporary, bytes)
        .map_err(|error| SkiaExecuteError::Program(format!("working capture: {error}")))?;
    let mut pixels =
        vec![0; usize::try_from(bytes).map_err(|_| SkiaExecuteError::InvalidRenderExtent)?];
    if let Some(image) = source.image() {
        let roi = source.roi();
        if roi.x < 0
            || roi.y < 0
            || i64::from(roi.x) + i64::from(roi.width) > i64::from(extent.width())
            || i64::from(roi.y) + i64::from(roi.height) > i64::from(extent.height())
        {
            return Err(SkiaExecuteError::Program(
                "working capture ROI outside frame".into(),
            ));
        }
        let image = surfaces.prepare_cpu_image(image)?;
        let capture_extent = Extent2d::new(roi.width, roi.height)
            .map_err(|_| SkiaExecuteError::InvalidRenderExtent)?;
        let info = working_info(capture_extent)?;
        let offset = (roi.y as usize * extent.width() as usize + roi.x as usize) * 8;
        if !image.read_pixels(
            &info,
            &mut pixels[offset..],
            extent.width() as usize * 8,
            (0, 0),
            skia_safe::image::CachingHint::Disallow,
        ) {
            return Err(SkiaExecuteError::Program(
                "working F16 readback failed".into(),
            ));
        }
    }
    #[cfg(target_endian = "big")]
    for channel in pixels.chunks_exact_mut(2) {
        channel.swap(0, 1);
    }
    Ok(WorkingF16Capture {
        width: extent.width(),
        height: extent.height(),
        rgba16f_le: pixels,
        _memory: memory,
    })
}
