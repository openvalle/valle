//! The browser receives only the sampling footprint of the deferred reference DOF kernel.
use valle_motion::scene3d::{CameraFrameState, RasterFrame};

const MAGIC: u32 = u32::from_le_bytes(*b"VDOF");

pub(super) fn encode(raster: &RasterFrame, camera: &CameraFrameState) -> Vec<u8> {
    let (width, height) = (raster.width, raster.height);
    let (focus, radius) = camera
        .depth_of_field
        .map_or((1.0, 0.0), |dof| (dof.focus_distance, dof.max_blur_radius));
    let mut visible = None;
    let mut near = None;
    for (i, pixel) in raster.premul_rgba16f.as_chunks::<4>().0.iter().enumerate() {
        if pixel[3] == 0 {
            continue;
        }
        let (x, y) = (i as u32 % width, i as u32 / width);
        grow(&mut visible, x, y);
        if raster.depth[i] < focus {
            grow(&mut near, x, y);
        }
    }
    let expand = |bounds: Option<[u32; 4]>, margin: u32| {
        bounds.map_or([0; 4], |[x0, y0, x1, y1]| {
            [
                x0.saturating_sub(margin),
                y0.saturating_sub(margin),
                x1.saturating_add(margin + 1).min(width),
                y1.saturating_add(margin + 1).min(height),
            ]
        })
    };
    let bounds = expand(visible, radius.ceil() as u32);
    let near_bounds = expand(near, radius.ceil() as u32);
    let [x0, y0, x1, y1] = if visible.is_some() {
        expand(visible, 2 * radius.max(1.0).ceil() as u32)
    } else {
        [0, 0, 1, 1]
    };
    let pixels = (x1 - x0) as usize * (y1 - y0) as usize;
    let mut packet = Vec::with_capacity(80 + pixels * 12);
    for word in [MAGIC, width, height, x0, y0, x1 - x0, y1 - y0] {
        packet.extend_from_slice(&word.to_le_bytes());
    }
    for value in [
        width as f32,
        height as f32,
        focus,
        radius,
        bounds[0] as f32,
        bounds[1] as f32,
        bounds[2] as f32,
        bounds[3] as f32,
        near_bounds[0] as f32,
        near_bounds[1] as f32,
        near_bounds[2] as f32,
        near_bounds[3] as f32,
        camera.far,
    ] {
        packet.extend_from_slice(&value.to_le_bytes());
    }
    for y in y0..y1 {
        let start = (y as usize * width as usize + x0 as usize) * 4;
        for value in &raster.premul_rgba16f[start..start + (x1 - x0) as usize * 4] {
            packet.extend_from_slice(&value.to_le_bytes());
        }
    }
    for y in y0..y1 {
        let start = y as usize * width as usize + x0 as usize;
        for value in &raster.depth[start..start + (x1 - x0) as usize] {
            packet.extend_from_slice(&value.to_le_bytes());
        }
    }
    packet
}

fn grow(bounds: &mut Option<[u32; 4]>, x: u32, y: u32) {
    *bounds = Some(match *bounds {
        Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        None => [x, y, x, y],
    });
}
