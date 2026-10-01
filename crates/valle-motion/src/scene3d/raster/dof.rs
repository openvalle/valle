//! Depth-aware circle-of-confusion blur over the resolved Scene3D color plane.
use super::{CLEAR_DEPTH, RasterFrame, f16_bits_to_f32, f32_to_f16_bits};
use crate::scene3d::{
    CameraFrameState, DepthOfFieldState,
    asset::{linear_to_srgb, srgb_to_linear},
};

const DIAGONAL: f32 = 0.707_106_77;
const DIRECTIONS: [(f32, f32); 8] = [
    (1.0, 0.0),
    (DIAGONAL, DIAGONAL),
    (0.0, 1.0),
    (-DIAGONAL, DIAGONAL),
    (-1.0, 0.0),
    (-DIAGONAL, -DIAGONAL),
    (0.0, -1.0),
    (DIAGONAL, -DIAGONAL),
];
const MAIN_RINGS: [f32; 4] = [0.125, 0.25, 0.5, 1.0];
const NEAR_RING_WEIGHTS: [f32; 4] = [4.0, 2.0, 1.0, 0.5];
const NEAR_WEIGHT_SUM: f32 = 8.0 * (4.0 + 2.0 + 1.0 + 0.5);

pub(super) fn apply(frame: &mut RasterFrame, camera: &CameraFrameState) {
    let Some(dof) = camera.depth_of_field else {
        return;
    };
    if dof.max_blur_radius == 0.0 {
        return;
    }
    let width = frame.width as usize;
    let height = frame.height as usize;
    let mut source = Vec::with_capacity(width * height);
    let mut near_bounds: Option<(usize, usize, usize, usize)> = None;
    for (index, pixel) in frame.premul_rgba16f.chunks_exact(4).enumerate() {
        source.push(decode(pixel));
        let depth = frame.depth[index];
        if depth != CLEAR_DEPTH && depth < dof.focus_distance && f16_bits_to_f32(pixel[3]) > 0.0 {
            let x = index % width;
            let y = index / width;
            near_bounds = Some(match near_bounds {
                Some((min_x, max_x, min_y, max_y)) => {
                    (min_x.min(x), max_x.max(x), min_y.min(y), max_y.max(y))
                }
                None => (x, x, y, y),
            });
        }
    }
    let near_bounds = near_bounds.map(|(min_x, max_x, min_y, max_y)| {
        let margin = dof.max_blur_radius.ceil() as usize;
        (
            min_x.saturating_sub(margin),
            (max_x + margin).min(width - 1),
            min_y.saturating_sub(margin),
            (max_y + margin).min(height - 1),
        )
    });
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let center_depth = effective_depth(frame.depth[index], camera.far);
            let radius = coc(dof, center_depth);
            let mut color = if radius >= 0.75 {
                gather_main(
                    &source,
                    &frame.depth,
                    width,
                    height,
                    x,
                    y,
                    center_depth,
                    radius,
                    dof,
                    camera.far,
                )
            } else {
                source[index]
            };
            if near_bounds.is_some_and(|(min_x, max_x, min_y, max_y)| {
                x >= min_x && x <= max_x && y >= min_y && y <= max_y
            }) {
                let near = gather_near(
                    &source,
                    &frame.depth,
                    width,
                    height,
                    x,
                    y,
                    center_depth,
                    dof,
                    camera.far,
                );
                if near[3] > 0.0 {
                    for axis in 0..3 {
                        color[axis] = near[axis] + color[axis] * (1.0 - near[3]);
                    }
                    color[3] = near[3] + color[3] * (1.0 - near[3]);
                }
            }
            if radius >= 0.75 || color != source[index] {
                let offset = index * 4;
                frame.premul_rgba16f[offset..offset + 4].copy_from_slice(&encode(color));
            }
        }
    }
}

fn effective_depth(depth: f32, far: f32) -> f32 {
    if depth == CLEAR_DEPTH { far } else { depth }
}

fn coc(dof: DepthOfFieldState, depth: f32) -> f32 {
    (dof.max_blur_radius * (1.0 - dof.focus_distance / depth).abs()).min(dof.max_blur_radius)
}

#[allow(clippy::too_many_arguments)]
fn gather_main(
    source: &[[f32; 4]],
    depth: &[f32],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    center_depth: f32,
    radius: f32,
    dof: DepthOfFieldState,
    far: f32,
) -> [f32; 4] {
    let mut sum = source[y * width + x];
    let mut count = 1.0;
    let tolerance = (center_depth * 0.005).max(0.005);
    for ring in MAIN_RINGS {
        for (dx, dy) in DIRECTIONS {
            let sx = libm::roundf(x as f32 + dx * radius * ring) as isize;
            let sy = libm::roundf(y as f32 + dy * radius * ring) as isize;
            if sx < 0 || sy < 0 || sx >= width as isize || sy >= height as isize {
                continue;
            }
            let sample = sy as usize * width + sx as usize;
            let sample_depth = effective_depth(depth[sample], far);
            let dx = sx as f32 - x as f32;
            let dy = sy as f32 - y as f32;
            let distance = libm::sqrtf(dx * dx + dy * dy);
            if (sample_depth < center_depth - tolerance
                && (sample_depth <= dof.focus_distance || distance > coc(dof, sample_depth)))
                || (center_depth < dof.focus_distance && sample_depth > center_depth + tolerance)
            {
                continue;
            }
            for axis in 0..4 {
                sum[axis] += source[sample][axis];
            }
            count += 1.0;
        }
    }
    sum.map(|value| value / count)
}

#[allow(clippy::too_many_arguments)]
fn gather_near(
    source: &[[f32; 4]],
    depth: &[f32],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    center_depth: f32,
    dof: DepthOfFieldState,
    far: f32,
) -> [f32; 4] {
    let mut sum = [0.0; 4];
    let radii = [
        1.0,
        dof.max_blur_radius / 3.0,
        dof.max_blur_radius * 2.0 / 3.0,
        dof.max_blur_radius,
    ];
    let tolerance = (center_depth * 0.005).max(0.005);
    for (ring_radius, weight) in radii.into_iter().zip(NEAR_RING_WEIGHTS) {
        for (dx, dy) in DIRECTIONS {
            let sx = libm::roundf(x as f32 + dx * ring_radius) as isize;
            let sy = libm::roundf(y as f32 + dy * ring_radius) as isize;
            if sx < 0 || sy < 0 || sx >= width as isize || sy >= height as isize {
                continue;
            }
            let sample = sy as usize * width + sx as usize;
            let sample_depth = effective_depth(depth[sample], far);
            if sample_depth >= dof.focus_distance
                || sample_depth >= center_depth - tolerance
                || source[sample][3] == 0.0
            {
                continue;
            }
            let dx = sx as f32 - x as f32;
            let dy = sy as f32 - y as f32;
            let distance = libm::sqrtf(dx * dx + dy * dy);
            if distance > coc(dof, sample_depth) {
                continue;
            }
            for axis in 0..4 {
                sum[axis] += source[sample][axis] * weight;
            }
        }
    }
    sum.map(|value| value / NEAR_WEIGHT_SUM)
}

fn decode(pixel: &[u16]) -> [f32; 4] {
    let alpha = f16_bits_to_f32(pixel[3]);
    if alpha == 0.0 {
        return [0.0; 4];
    }
    [
        srgb_to_linear((f16_bits_to_f32(pixel[0]) / alpha).clamp(0.0, 1.0)) * alpha,
        srgb_to_linear((f16_bits_to_f32(pixel[1]) / alpha).clamp(0.0, 1.0)) * alpha,
        srgb_to_linear((f16_bits_to_f32(pixel[2]) / alpha).clamp(0.0, 1.0)) * alpha,
        alpha,
    ]
}

fn encode(color: [f32; 4]) -> [u16; 4] {
    let alpha = color[3].clamp(0.0, 1.0);
    let rgb = if alpha > 0.0 {
        std::array::from_fn(|axis| {
            f32_to_f16_bits(linear_to_srgb((color[axis] / alpha).clamp(0.0, 1.0)) * alpha)
        })
    } else {
        [0; 3]
    };
    [rgb[0], rgb[1], rgb[2], f32_to_f16_bits(alpha)]
}
