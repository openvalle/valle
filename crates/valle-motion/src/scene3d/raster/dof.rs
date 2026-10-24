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

#[derive(Clone, Copy)]
struct NearTap {
    dx: f32,
    dy: f32,
    weight: f32,
    fixed: Option<FixedNearTap>,
}

#[derive(Clone, Copy)]
struct FixedNearTap {
    dx: isize,
    dy: isize,
    distance: f32,
}

fn fixed_offset(offset: f32, extent: usize) -> Option<isize> {
    let rounded = nearest_pixel(offset);
    (0..extent)
        .all(|pixel| nearest_pixel(pixel as f32 + offset) == pixel as isize + rounded)
        .then_some(rounded)
}

fn near_kernel(radius: f32, width: usize, height: usize) -> [NearTap; 32] {
    let radii = [1.0, radius / 3.0, radius * 2.0 / 3.0, radius];
    std::array::from_fn(|index| {
        let ring = index / DIRECTIONS.len();
        let (dx, dy) = DIRECTIONS[index % DIRECTIONS.len()];
        let (dx, dy) = (dx * radii[ring], dy * radii[ring]);
        // Fractional offsets can round differently near zero or half-integer ties.
        // Use integer offsets and one distance only after checking the entire axis.
        let fixed = fixed_offset(dx, width)
            .zip(fixed_offset(dy, height))
            .map(|(dx, dy)| FixedNearTap {
                dx,
                dy,
                distance: libm::sqrtf((dx as f32) * (dx as f32) + (dy as f32) * (dy as f32)),
            });
        NearTap {
            dx,
            dy,
            weight: NEAR_RING_WEIGHTS[ring],
            fixed,
        }
    })
}

/// Same integer coordinate as libm::roundf(value) as isize. libm rounds by
/// truncating value + copysign(0.5 - 0.25 * EPSILON, value); the integer cast
/// already truncates, so the intermediate floating-point truncation is redundant.
#[inline]
fn nearest_pixel(value: f32) -> isize {
    const BIAS: f32 = 0.5 - 0.25 * f32::EPSILON;
    (value + BIAS.copysign(value)) as isize
}

struct Source {
    pixels: Vec<[f32; 4]>,
    offset: usize,
    width: usize,
}

impl Source {
    #[inline]
    fn at(&self, x: usize, y: usize) -> [f32; 4] {
        self.pixels[y * self.width + x - self.offset]
    }
}

pub(super) fn apply(frame: &mut RasterFrame, camera: &CameraFrameState, workers: usize) {
    let Some(dof) = camera.depth_of_field else {
        return;
    };
    if dof.max_blur_radius == 0.0 {
        return;
    }
    let width = frame.width as usize;
    let height = frame.height as usize;
    let mut near_bounds: Option<(usize, usize, usize, usize)> = None;
    let mut visible_bounds: Option<(usize, usize, usize, usize)> = None;
    for (index, pixel) in frame.premul_rgba16f.as_chunks::<4>().0.iter().enumerate() {
        // RasterFrame guarantees zero RGB at zero coverage. Only covered pixels can
        // contribute to either gather; expand their union by the complete kernel support.
        if pixel[3] != 0 {
            let x = index % width;
            let y = index / width;
            visible_bounds = Some(match visible_bounds {
                Some((x0, x1, y0, y1)) => (x0.min(x), x1.max(x), y0.min(y), y1.max(y)),
                None => (x, x, y, y),
            });
        }
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
    let margin = dof.max_blur_radius.ceil() as usize;
    let expand = |(min_x, max_x, min_y, max_y): (usize, usize, usize, usize)| {
        (
            min_x.saturating_sub(margin),
            (max_x + margin).min(width - 1),
            min_y.saturating_sub(margin),
            (max_y + margin).min(height - 1),
        )
    };
    let Some((min_x, max_x, min_y, max_y)) = visible_bounds.map(expand) else {
        return;
    };
    let near_kernel = near_bounds.map(|bounds| {
        (
            expand(bounds),
            near_kernel(dof.max_blur_radius, width, height),
        )
    });
    // The output footprint already includes one blur radius. Its gathers can
    // reach another radius, including the fixed one-pixel near ring. Decode only
    // this input rectangle but keep all original canvas/sample coordinates.
    let sample_margin = dof.max_blur_radius.max(1.0).ceil() as usize;
    let source_x0 = min_x.saturating_sub(sample_margin);
    let source_y0 = min_y.saturating_sub(sample_margin);
    let source_x1 = (max_x + sample_margin + 1).min(width);
    let source_y1 = (max_y + sample_margin + 1).min(height);
    let mut source = Source {
        pixels: Vec::with_capacity((source_x1 - source_x0) * (source_y1 - source_y0)),
        offset: source_y0 * (source_x1 - source_x0) + source_x0,
        width: source_x1 - source_x0,
    };
    for y in source_y0..source_y1 {
        let row = &frame.premul_rgba16f[(y * width + source_x0) * 4..(y * width + source_x1) * 4];
        source
            .pixels
            .extend(row.as_chunks::<4>().0.iter().map(|pixel| decode(pixel)));
    }
    if std::env::var_os("VALLE_PERF_SCENE3D").is_some() {
        eprintln!(
            "[valle scene3d-region] cpu-dof pixels={} full={}",
            source.pixels.len(),
            width * height
        );
    }
    let reach = Reach::new(
        frame,
        [min_x, max_x],
        [min_y, max_y],
        [source_x0, source_x1],
        [source_y0, source_y1],
        sample_margin,
    );
    let depth = frame.depth.as_slice();
    let stride = width * 4;
    let render_rows = |first_y: usize, rows: &mut [u16]| {
        for (row, pixels) in rows.chunks_exact_mut(stride).enumerate() {
            let y = first_y + row;
            for x in min_x..=max_x {
                if !reach.covered(x, y) {
                    continue;
                }
                let index = y * width + x;
                let center_depth = effective_depth(depth[index], camera.far);
                let radius = coc(dof, center_depth);
                let original = source.at(x, y);
                let mut color = if radius >= 0.75 {
                    gather_main(
                        &source,
                        depth,
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
                    original
                };
                if let Some(((near_min_x, near_max_x, near_min_y, near_max_y), kernel)) =
                    &near_kernel
                    && x >= *near_min_x
                    && x <= *near_max_x
                    && y >= *near_min_y
                    && y <= *near_max_y
                {
                    let near = gather_near(
                        &source,
                        depth,
                        width,
                        height,
                        x,
                        y,
                        center_depth,
                        dof,
                        camera.far,
                        kernel,
                    );
                    if near[3] > 0.0 {
                        for axis in 0..3 {
                            color[axis] = near[axis] + color[axis] * (1.0 - near[3]);
                        }
                        color[3] = near[3] + color[3] * (1.0 - near[3]);
                    }
                }
                if radius >= 0.75 || color != original {
                    pixels[x * 4..x * 4 + 4].copy_from_slice(&encode(color));
                }
            }
        }
    };
    let rows = &mut frame.premul_rgba16f[min_y * stride..(max_y + 1) * stride];
    let worker_count =
        if cfg!(target_arch = "wasm32") || (max_x - min_x + 1) * (max_y - min_y + 1) < 32_768 {
            1
        } else {
            workers
                .clamp(1, super::MAX_RASTER_WORKERS)
                .min(max_y - min_y + 1)
        };
    crate::cpu::parallel_scope(worker_count, |scope, worker_count| {
        if worker_count == 1 {
            render_rows(min_y, rows);
        } else {
            let rows_per_job = (max_y - min_y + 1).div_ceil(worker_count);
            let mut chunks = rows.chunks_mut(rows_per_job * stride).enumerate();
            let (_, caller_rows) = chunks.next().unwrap();
            for (job, rows) in chunks {
                let render_rows = &render_rows;
                scope.spawn(move || render_rows(min_y + job * rows_per_job, rows));
            }
            render_rows(min_y, caller_rows);
        }
    });
}

/// The output pixels whose gathers can reach a pixel that is not all zero bits. Both gathers read
/// only pixels within `margin` of the one they write, and a zero pixel decodes to transparent and
/// adds nothing. A pixel whose reach, itself included, is all zero would gather transparent and
/// write zero bits over its zero bits, so it is left as it is.
struct Reach {
    x0: usize,
    y0: usize,
    columns: usize,
    covered: Vec<bool>,
}

impl Reach {
    fn new(
        frame: &RasterFrame,
        [min_x, max_x]: [usize; 2],
        [min_y, max_y]: [usize; 2],
        [source_x0, source_x1]: [usize; 2],
        [source_y0, source_y1]: [usize; 2],
        margin: usize,
    ) -> Self {
        let width = frame.width as usize;
        let columns = max_x - min_x + 1;
        // Rows of the source: whether a nonzero pixel lies within `margin` columns of each
        // output column, from running counts along the row.
        let mut near = vec![false; (source_y1 - source_y0) * columns];
        let mut counts = vec![0_u32; source_x1 - source_x0 + 1];
        for (y, near) in (source_y0..source_y1).zip(near.chunks_exact_mut(columns)) {
            let row =
                &frame.premul_rgba16f[(y * width + source_x0) * 4..(y * width + source_x1) * 4];
            for (at, pixel) in row.as_chunks::<4>().0.iter().enumerate() {
                counts[at + 1] = counts[at] + u32::from(*pixel != [0; 4]);
            }
            for (x, near) in (min_x..=max_x).zip(near) {
                let first = x.saturating_sub(margin).max(source_x0) - source_x0;
                let last = (x + margin + 1).min(source_x1) - source_x0;
                *near = counts[last] > counts[first];
            }
        }
        // Then whether such a row lies within `margin` rows, by running counts down the columns.
        let mut counts = vec![0_u32; (source_y1 - source_y0 + 1) * columns];
        for (at, near) in near.chunks_exact(columns).enumerate() {
            let (above, below) = counts.split_at_mut((at + 1) * columns);
            for ((count, &before), &near) in below[..columns]
                .iter_mut()
                .zip(&above[at * columns..])
                .zip(near)
            {
                *count = before + u32::from(near);
            }
        }
        let mut covered = vec![false; (max_y - min_y + 1) * columns];
        for (y, covered) in (min_y..=max_y).zip(covered.chunks_exact_mut(columns)) {
            let first = y.saturating_sub(margin).max(source_y0) - source_y0;
            let last = (y + margin + 1).min(source_y1) - source_y0;
            let first = &counts[first * columns..][..columns];
            let last = &counts[last * columns..][..columns];
            for ((covered, first), last) in covered.iter_mut().zip(first).zip(last) {
                *covered = last > first;
            }
        }
        Self {
            x0: min_x,
            y0: min_y,
            columns,
            covered,
        }
    }

    fn covered(&self, x: usize, y: usize) -> bool {
        self.covered[(y - self.y0) * self.columns + x - self.x0]
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
    source: &Source,
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
    let mut sum = source.at(x, y);
    let mut count = 1.0;
    let tolerance = (center_depth * 0.005).max(0.005);
    for ring in MAIN_RINGS {
        for (dx, dy) in DIRECTIONS {
            let sx = nearest_pixel(x as f32 + dx * radius * ring);
            let sy = nearest_pixel(y as f32 + dy * radius * ring);
            if sx < 0 || sy < 0 || sx >= width as isize || sy >= height as isize {
                continue;
            }
            let sample = sy as usize * width + sx as usize;
            let sample_depth = effective_depth(depth[sample], far);
            if center_depth < dof.focus_distance && sample_depth > center_depth + tolerance {
                continue;
            }
            if sample_depth < center_depth - tolerance {
                if sample_depth <= dof.focus_distance {
                    continue;
                }
                let dx = sx as f32 - x as f32;
                let dy = sy as f32 - y as f32;
                if libm::sqrtf(dx * dx + dy * dy) > coc(dof, sample_depth) {
                    continue;
                }
            }
            let color = source.at(sx as usize, sy as usize);
            for axis in 0..4 {
                sum[axis] += color[axis];
            }
            count += 1.0;
        }
    }
    sum.map(|value| value / count)
}

#[allow(clippy::too_many_arguments)]
fn gather_near(
    source: &Source,
    depth: &[f32],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    center_depth: f32,
    dof: DepthOfFieldState,
    far: f32,
    kernel: &[NearTap; 32],
) -> [f32; 4] {
    let mut sum = [0.0; 4];
    let tolerance = (center_depth * 0.005).max(0.005);
    for &NearTap {
        dx,
        dy,
        weight,
        fixed,
    } in kernel
    {
        let (sx, sy) = if let Some(tap) = fixed {
            (x as isize + tap.dx, y as isize + tap.dy)
        } else {
            (nearest_pixel(x as f32 + dx), nearest_pixel(y as f32 + dy))
        };
        if sx < 0 || sy < 0 || sx >= width as isize || sy >= height as isize {
            continue;
        }
        let sample = sy as usize * width + sx as usize;
        let sample_depth = effective_depth(depth[sample], far);
        if sample_depth >= dof.focus_distance
            || sample_depth >= center_depth - tolerance
            || source.at(sx as usize, sy as usize)[3] == 0.0
        {
            continue;
        }
        let distance = fixed.map_or_else(
            || {
                let dx = sx as f32 - x as f32;
                let dy = sy as f32 - y as f32;
                libm::sqrtf(dx * dx + dy * dy)
            },
            |tap| tap.distance,
        );
        if distance > coc(dof, sample_depth) {
            continue;
        }
        let color = source.at(sx as usize, sy as usize);
        for axis in 0..4 {
            sum[axis] += color[axis] * weight;
        }
    }
    sum.map(|value| value / NEAR_WEIGHT_SUM)
}

fn decode(pixel: &[u16]) -> [f32; 4] {
    let alpha = f16_bits_to_f32(pixel[3]);
    if alpha == 0.0 {
        return [0.0; 4];
    }
    if alpha == 1.0 {
        return [
            super::color::decode_opaque(pixel[0]),
            super::color::decode_opaque(pixel[1]),
            super::color::decode_opaque(pixel[2]),
            1.0,
        ];
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
    let rgb = if alpha == 1.0 {
        std::array::from_fn(|axis| super::color::encode_opaque(color[axis].clamp(0.0, 1.0)))
    } else if alpha > 0.0 {
        std::array::from_fn(|axis| {
            f32_to_f16_bits(linear_to_srgb((color[axis] / alpha).clamp(0.0, 1.0)) * alpha)
        })
    } else {
        [0; 3]
    };
    [rgb[0], rgb[1], rgb[2], f32_to_f16_bits(alpha)]
}

#[cfg(test)]
#[path = "dof_reference.rs"]
mod reference;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene3d::Vec3;

    #[test]
    fn integer_sampling_matches_libm_at_halfway_boundaries() {
        // Include both neighbors of every half-integer across the admitted layer
        // extent and blur footprint, especially negative off-canvas coordinates.
        for twice in -34..=(2 * (crate::scene3d::MAX_LAYER_EDGE + 17) as i32) {
            let center = twice as f32 / 2.0;
            for value in [center.next_down(), center, center.next_up()] {
                assert_eq!(
                    nearest_pixel(value),
                    libm::roundf(value) as isize,
                    "{value:?}"
                );
            }
        }
        let mut bits = 17_u32;
        for _ in 0..100_000 {
            bits = bits.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let value = f32::from_bits(bits);
            assert_eq!(
                nearest_pixel(value),
                libm::roundf(value) as isize,
                "{value:?}"
            );
        }
    }

    #[test]
    fn fixed_near_taps_require_identical_offsets_across_the_frame() {
        for radius in [6.0, 12.0, 16.0] {
            assert!(
                near_kernel(radius, 4096, 2160)
                    .iter()
                    .all(|tap| tap.fixed.is_some())
            );
        }
        let taps = near_kernel(1.5, 4096, 2160);
        assert!(taps.iter().any(|tap| tap.fixed.is_none()));
        for tap in taps {
            if let Some(fixed) = tap.fixed {
                for (x, y) in [(0, 0), (1, 1), (1023, 1024), (4095, 2159)] {
                    let sx = nearest_pixel(x as f32 + tap.dx);
                    let sy = nearest_pixel(y as f32 + tap.dy);
                    assert_eq!((sx, sy), (x + fixed.dx, y + fixed.dy));
                    let dx = sx as f32 - x as f32;
                    let dy = sy as f32 - y as f32;
                    assert_eq!(
                        fixed.distance.to_bits(),
                        libm::sqrtf(dx * dx + dy * dy).to_bits()
                    );
                }
            }
        }
    }

    fn fixture(width: u32, height: u32, edge: bool) -> RasterFrame {
        let count = (width * height) as usize;
        let mut frame = RasterFrame {
            width,
            height,
            premul_rgba16f: vec![0; count * 4],
            depth: vec![CLEAR_DEPTH; count],
            object_ids: vec![0; count],
            node_ids: vec![super::super::super::BACKGROUND_NODE_ID; count],
            anchors: Vec::new(),
        };
        for y in 0..height {
            for x in 0..width {
                if !edge
                    && (x < width / 3
                        || x >= width * 2 / 3
                        || y < height / 3
                        || y >= height * 2 / 3)
                {
                    continue;
                }
                let tag = (x * 13 + y * 47) % 17;
                if tag < 4 {
                    continue;
                }
                let alpha = [0.015625, 0.25, 0.7, 1.0][(tag % 4) as usize];
                let color = [
                    alpha * (tag as f32 / 17.0),
                    alpha * 0.37,
                    alpha * 0.82,
                    alpha,
                ];
                let at = (y * width + x) as usize;
                frame.premul_rgba16f[at * 4..at * 4 + 4].copy_from_slice(&encode(color));
                frame.depth[at] = [2.0, 4.49, 4.5, 4.51, 8.0][(tag % 5) as usize];
            }
        }
        frame
    }

    fn camera(focus: f32, radius: f32) -> CameraFrameState {
        CameraFrameState {
            position: Vec3([0.0, 0.0, 4.5]),
            target: Vec3([0.0; 3]),
            fov_y_degrees: 45.0,
            near: 0.1,
            far: 100.0,
            depth_of_field: Some(DepthOfFieldState {
                focus_distance: focus,
                max_blur_radius: radius,
            }),
        }
    }

    #[test]
    fn cropped_dof_is_bit_exact_to_original_at_edges_and_fractional_radii() {
        for edge in [false, true] {
            let source = fixture(59, 43, edge);
            for radius in [0.0, 0.4, 0.75, 1.0, 1.5, 3.0, 6.0, 12.5, 16.0] {
                for focus in [2.0, 4.5, 8.0] {
                    let camera = camera(focus, radius);
                    let mut expected = source.clone();
                    reference::apply(&mut expected, &camera);
                    let mut actual = source.clone();
                    apply(&mut actual, &camera, 1);
                    assert_eq!(
                        actual, expected,
                        "edge={edge} radius={radius} focus={focus}"
                    );
                }
            }
        }
    }

    #[test]
    fn parallel_dof_matches_original_and_does_not_change_depth_or_ids() {
        let source = fixture(384, 192, true);
        let camera = camera(4.5, 6.0);
        let mut expected = source.clone();
        reference::apply(&mut expected, &camera);
        for workers in [1, 2, 8] {
            let mut actual = source.clone();
            apply(&mut actual, &camera, workers);
            assert_eq!(actual, expected, "workers={workers}");
        }
    }

    /// Islands of a few pixels, far apart, so most of the blurred area reaches only transparent
    /// pixels; a transparent pixel away from them keeps color bits.
    fn islands(width: u32, height: u32) -> RasterFrame {
        let mut frame = fixture(width, height, true);
        for y in 0..height {
            for x in 0..width {
                if x % 23 > 2 || y % 19 > 2 {
                    let at = (y * width + x) as usize;
                    frame.premul_rgba16f[at * 4..at * 4 + 4].fill(0);
                    frame.depth[at] = CLEAR_DEPTH;
                }
            }
        }
        let at = (11 * width + 12) as usize;
        frame.premul_rgba16f[at * 4..at * 4 + 3].copy_from_slice(&[0x3c00, 0x3800, 0x3400]);
        frame
    }

    #[test]
    fn dof_leaves_pixels_that_reach_only_zero_pixels_and_matches_original() {
        let source = islands(97, 61);
        for radius in [0.4, 0.75, 1.0, 1.5, 3.0, 6.0, 12.5, 16.0] {
            for focus in [2.0, 4.5, 8.0] {
                let camera = camera(focus, radius);
                let mut expected = source.clone();
                reference::apply(&mut expected, &camera);
                for workers in [1, 4] {
                    let mut actual = source.clone();
                    apply(&mut actual, &camera, workers);
                    assert_eq!(
                        actual, expected,
                        "radius={radius} focus={focus} workers={workers}"
                    );
                }
            }
        }
    }
}
