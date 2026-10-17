//! Bounded, backend-independent bloom on premultiplied working-linear RGBA16F pixels.
//! Native and Web pass the same little-endian F16 payload through this pyramid.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BloomParams {
    pub threshold: f32,
    pub knee: f32,
    pub intensity: f32,
    pub radius: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlowParams {
    /// Premultiplied working-linear Rec.2020 color.
    pub color: [f32; 4],
    pub intensity: f32,
    pub radius: f32,
}

#[derive(Debug, thiserror::Error)]
pub enum BloomError {
    #[error("bloom dimensions or F16 payload are invalid")]
    InvalidImage,
    #[error("bloom parameters are outside their admitted ranges")]
    InvalidParameters,
}

const MAX_PIXELS: usize = 16_777_216;
// Authored radii stop at 128px; device transforms can enlarge them further.
// The pyramid averages across scales; this fixed gain restores the visible
// near-edge energy of the authored glow before the color peak is capped.
const GLOW_RESPONSE_GAIN: f64 = 12.0;
// The 13-tap downsample pattern (centre 4, edges 2, diagonals and the two-pixel axis taps 1),
// sampled bilinearly about the output pixel's centre, which lies between source pixels 2x and
// 2x + 1. Folding the bilinear taps gives these source offsets from (2x, 2y). They are symmetric
// about (0.5, 0.5), so every level stays centred on its input; the weights sum to 80.
const DOWN_TAPS: [(i32, i32, f64); 24] = [
    (0, 0, 9.0),
    (1, 0, 9.0),
    (0, 1, 9.0),
    (1, 1, 9.0),
    (-1, 0, 4.0),
    (-1, 1, 4.0),
    (2, 0, 4.0),
    (2, 1, 4.0),
    (0, -1, 4.0),
    (1, -1, 4.0),
    (0, 2, 4.0),
    (1, 2, 4.0),
    (-1, -1, 1.0),
    (2, -1, 1.0),
    (-1, 2, 1.0),
    (2, 2, 1.0),
    (-2, 0, 1.0),
    (-2, 1, 1.0),
    (3, 0, 1.0),
    (3, 1, 1.0),
    (0, -2, 1.0),
    (1, -2, 1.0),
    (0, 3, 1.0),
    (1, 3, 1.0),
];

/// The bounded F16 pyramid shared by the CPU reference/WASM and Native Skia GPU executors.
/// Resource 0 is the extracted bright or alpha image; every pass writes one new resource.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PyramidPassKind {
    Downsample {
        input: usize,
    },
    Blur {
        input: usize,
    },
    Upsample {
        coarse: usize,
        fine: Option<usize>,
        local_weight: f64,
    },
}

impl PyramidPassKind {
    fn reads(self) -> Vec<usize> {
        match self {
            Self::Downsample { input } | Self::Blur { input } => vec![input],
            Self::Upsample { coarse, fine, .. } => {
                fine.map_or_else(|| vec![coarse], |fine| vec![coarse, fine])
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PyramidPass {
    kind: PyramidPassKind,
    output: usize,
    width: u32,
    height: u32,
    retire_after: Vec<usize>,
}

impl PyramidPass {
    pub const fn kind(&self) -> PyramidPassKind {
        self.kind
    }

    pub const fn output(&self) -> usize {
        self.output
    }

    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    pub fn retire_after(&self) -> &[usize] {
        &self.retire_after
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PyramidGraph {
    spread: f64,
    passes: Vec<PyramidPass>,
    output: usize,
    resource_count: usize,
}

impl PyramidGraph {
    pub fn new(
        width: u32,
        height: u32,
        depth_radius: f32,
        spread_radius: f32,
    ) -> Result<Self, BloomError> {
        pixel_count(width, height)?;
        if !depth_radius.is_finite()
            || depth_radius < 0.0
            || !spread_radius.is_finite()
            || spread_radius < 0.0
        {
            return Err(BloomError::InvalidParameters);
        }
        let levels = ((f64::from(depth_radius) / 2.0).log2().ceil() as usize).clamp(1, 5);
        let spread = (f64::from(spread_radius) / 48.0).max(0.125);
        let mut passes = Vec::with_capacity(levels * 3);
        let mut next_resource = 1;
        let mut current = 0;
        let (mut current_width, mut current_height) = (width, height);
        let mut fine_levels = Vec::with_capacity(levels);
        for _ in 0..levels {
            current_width = current_width.div_ceil(2);
            current_height = current_height.div_ceil(2);
            let downsample = push_pyramid_pass(
                &mut passes,
                &mut next_resource,
                PyramidPassKind::Downsample { input: current },
                current_width,
                current_height,
            );
            let blurred = push_pyramid_pass(
                &mut passes,
                &mut next_resource,
                PyramidPassKind::Blur { input: downsample },
                current_width,
                current_height,
            );
            fine_levels.push((blurred, current_width, current_height));
            current = downsample;
        }
        let mut accumulated = fine_levels.pop().expect("positive pyramid level count").0;
        while let Some((fine, fine_width, fine_height)) = fine_levels.pop() {
            accumulated = push_pyramid_pass(
                &mut passes,
                &mut next_resource,
                PyramidPassKind::Upsample {
                    coarse: accumulated,
                    fine: Some(fine),
                    local_weight: 0.2,
                },
                fine_width,
                fine_height,
            );
        }
        let output = push_pyramid_pass(
            &mut passes,
            &mut next_resource,
            PyramidPassKind::Upsample {
                coarse: accumulated,
                fine: None,
                local_weight: 0.0,
            },
            width,
            height,
        );
        let mut last_use = vec![None; next_resource];
        for (index, pass) in passes.iter().enumerate() {
            for resource in pass.kind.reads() {
                last_use[resource] = Some(index);
            }
        }
        for (resource, last) in last_use.into_iter().enumerate() {
            if resource != output
                && let Some(index) = last
            {
                passes[index].retire_after.push(resource);
            }
        }
        Ok(Self {
            spread,
            passes,
            output,
            resource_count: next_resource,
        })
    }

    pub const fn spread(&self) -> f64 {
        self.spread
    }

    pub fn passes(&self) -> &[PyramidPass] {
        &self.passes
    }

    pub const fn output(&self) -> usize {
        self.output
    }

    pub const fn resource_count(&self) -> usize {
        self.resource_count
    }
}

fn push_pyramid_pass(
    passes: &mut Vec<PyramidPass>,
    next_resource: &mut usize,
    kind: PyramidPassKind,
    width: u32,
    height: u32,
) -> usize {
    let output = *next_resource;
    *next_resource += 1;
    passes.push(PyramidPass {
        kind,
        output,
        width,
        height,
        retire_after: Vec::new(),
    });
    output
}

#[derive(Clone)]
struct Level<const CHANNELS: usize> {
    width: usize,
    height: usize,
    pixels: Vec<[f32; CHANNELS]>,
}

impl<const CHANNELS: usize> Level<CHANNELS> {
    fn empty(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![[0.0; CHANNELS]; width * height],
        }
    }

    fn at(&self, x: i32, y: i32) -> [f32; CHANNELS] {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return [0.0; CHANNELS];
        }
        let (x, y) = (x as usize, y as usize);
        self.pixels[y * self.width + x]
    }

    /// Separable tent sum of one pixel; the weights total 16 when every tap lies inside the
    /// level. `tent_9` forms the same sums a band of rows at a time.
    #[cfg(test)]
    fn tent(&self, x: &TentAxis, y: &TentAxis) -> [f64; CHANNELS] {
        let mut sum = [0.0; CHANNELS];
        for &(sy, wy) in y.taps() {
            let row = &self.pixels[sy * self.width..(sy + 1) * self.width];
            let mut line = [0.0; CHANNELS];
            for &(sx, wx) in x.taps() {
                for (acc, value) in line.iter_mut().zip(row[sx]) {
                    *acc += wx * f64::from(value);
                }
            }
            for (acc, value) in sum.iter_mut().zip(line) {
                *acc += wy * value;
            }
        }
        sum
    }
}

/// One axis of the 1-2-1 tent: three taps `offset` apart around `center`, each interpolated
/// linearly between its two neighbouring pixels. Taps that share a pixel are merged and zero
/// or outside weights dropped, so each source pixel is read once.
#[derive(Clone, Copy)]
struct TentAxis {
    len: usize,
    taps: [(usize, f64); 6],
}

impl TentAxis {
    fn new(center: f64, offset: f64, extent: usize) -> Self {
        let mut axis = Self {
            len: 0,
            taps: [(0, 0.0); 6],
        };
        for (tap, weight) in [(-1.0, 1.0), (0.0, 2.0), (1.0, 1.0)] {
            let position = center + tap * offset;
            let lower = position.floor();
            let fraction = position - lower;
            for (index, share) in [(lower, 1.0 - fraction), (lower + 1.0, fraction)] {
                if share != 0.0 && index >= 0.0 && index < extent as f64 {
                    axis.add(index as usize, weight * share);
                }
            }
        }
        axis
    }

    fn add(&mut self, index: usize, weight: f64) {
        match self.taps[..self.len].iter_mut().find(|tap| tap.0 == index) {
            Some(tap) => tap.1 += weight,
            None => {
                self.taps[self.len] = (index, weight);
                self.len += 1;
            }
        }
    }

    fn taps(&self) -> &[(usize, f64)] {
        &self.taps[..self.len]
    }
}

/// The lowest and highest source index that any of `axes` reads.
fn tap_span(axes: &[TentAxis]) -> Option<(usize, usize)> {
    axes.iter()
        .flat_map(|axis| axis.taps().iter().map(|&(index, _)| index))
        .fold(None, |range: Option<(usize, usize)>, index| {
            Some(range.map_or((index, index), |(lo, hi)| (lo.min(index), hi.max(index))))
        })
}

/// Half-open columns and rows of a level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Area {
    columns: [usize; 2],
    rows: [usize; 2],
}

impl Area {
    #[cfg(test)]
    const fn full(width: usize, height: usize) -> Self {
        Self {
            columns: [0, width],
            rows: [0, height],
        }
    }

    const fn is_empty(self) -> bool {
        self.columns[0] >= self.columns[1] || self.rows[0] >= self.rows[1]
    }

    fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Self {
            columns: [
                self.columns[0].min(other.columns[0]),
                self.columns[1].max(other.columns[1]),
            ],
            rows: [
                self.rows[0].min(other.rows[0]),
                self.rows[1].max(other.rows[1]),
            ],
        }
    }
}

/// The source pixels the 13-tap downsample of `area` reads: offsets -2..=3 around (2x, 2y).
fn downsample_support(area: Area, [width, height]: [usize; 2]) -> Area {
    if area.is_empty() {
        return Area::default();
    }
    let span = |[start, end]: [usize; 2], extent: usize| {
        [(2 * start).saturating_sub(2), (2 * end + 2).min(extent)]
    };
    Area {
        columns: span(area.columns, width),
        rows: span(area.rows, height),
    }
}

/// The tent taps of every output column and row of a blur or upsample pass.
#[derive(Default)]
struct TentAxes {
    columns: Vec<TentAxis>,
    lines: Vec<TentAxis>,
}

impl TentAxes {
    /// Taps sit `spread` pixels from the centre on each axis. Fractional offsets interpolate
    /// instead of rounding, so the tent stays centred. Each row/column uses the same taps.
    fn blur([width, height]: [usize; 2], spread: f64) -> Self {
        let axes = |extent| {
            (0..extent)
                .map(|position| TentAxis::new(position as f64, spread, extent))
                .collect()
        };
        Self {
            columns: axes(width),
            lines: axes(height),
        }
    }

    /// Output pixel centres mapped onto the coarser source, with taps half the spread apart.
    fn upsample(
        [width, height]: [usize; 2],
        [source_width, source_height]: [usize; 2],
        spread: f64,
    ) -> Self {
        let axes = |extent, source_extent| {
            (0..extent)
                .map(|position| {
                    let center =
                        (position as f64 + 0.5) * source_extent as f64 / extent as f64 - 0.5;
                    TentAxis::new(center, 0.5 * spread, source_extent)
                })
                .collect()
        };
        Self {
            columns: axes(width, source_width),
            lines: axes(height, source_height),
        }
    }

    /// The source pixels the taps of `area` read.
    fn support(&self, area: Area) -> Area {
        if area.is_empty() {
            return Area::default();
        }
        let span = |axes: &[TentAxis]| tap_span(axes).map_or([0, 0], |(lo, hi)| [lo, hi + 1]);
        Area {
            columns: span(&self.columns[area.columns[0]..area.columns[1]]),
            rows: span(&self.lines[area.rows[0]..area.rows[1]]),
        }
    }
}

// Pyramid passes write independent rows. Keep each pixel's tap and summation order
// unchanged so native workers and the single-threaded Web build produce the same F16 bytes.
#[cfg(not(target_family = "wasm"))]
static PYRAMID_WORKERS: std::sync::LazyLock<Option<rayon::ThreadPool>> =
    std::sync::LazyLock::new(|| {
        let workers = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(8);
        (workers > 1)
            .then(|| {
                rayon::ThreadPoolBuilder::new()
                    .num_threads(workers)
                    .thread_name(|index| format!("valle-pyramid-{index}"))
                    .build()
                    .ok()
            })
            .flatten()
    });

/// Fills rows `top..bottom` of `next` as [`fill_image_rows`] does; `fill` receives the level
/// index of its first row.
fn fill_rows<const CHANNELS: usize>(
    next: &mut Level<CHANNELS>,
    [top, bottom]: [usize; 2],
    fill: impl Fn(usize, &mut [[f32; CHANNELS]]) + Sync,
) {
    let width = next.width;
    fill_image_rows(
        &mut next.pixels[top * width..bottom * width],
        width,
        |first, rows| fill(top + first, rows),
    );
}

/// Fills a row-major image whose rows are independent, splitting a large image across admitted
/// CPU helpers. `fill` receives the index of its first row and whole rows (a trailing partial
/// row stays with the last ones); every pixel keeps its own arithmetic, so the bytes do not
/// depend on how the rows were split.
pub fn fill_image_rows<T: Send>(
    pixels: &mut [T],
    width: usize,
    fill: impl Fn(usize, &mut [T]) + Sync,
) {
    #[cfg(target_family = "wasm")]
    let _ = width;
    #[cfg(not(target_family = "wasm"))]
    {
        let height = if width == 0 {
            0
        } else {
            pixels.len().div_ceil(width)
        };
        let large = height >= 128 && pixels.len() >= 65_536;
        if height > 0 && valle_motion::cpu::is_budgeted() {
            valle_motion::cpu::parallel_scope(if large { 8 } else { 1 }, |scope, count| {
                let rows_per_job = height.div_ceil(count).max(16);
                let mut chunks = pixels.chunks_mut(rows_per_job * width).enumerate();
                let (_, caller_rows) = chunks.next().unwrap();
                for (index, rows) in chunks {
                    let fill = &fill;
                    scope.spawn(move || fill(index * rows_per_job, rows));
                }
                fill(0, caller_rows);
            });
            return;
        }
        // All frame workers share one bounded pool instead of spawning another
        // machine-sized set of OS threads for every stage of every frame.
        if large && let Some(pool) = &*PYRAMID_WORKERS {
            let workers = pool.current_num_threads();
            let rows_per_worker = height.div_ceil(workers).max(16);
            pool.scope(|scope| {
                for (index, chunk) in pixels.chunks_mut(rows_per_worker * width).enumerate() {
                    let fill = &fill;
                    scope.spawn(move |_| fill(index * rows_per_worker, chunk));
                }
            });
            return;
        }
    }
    fill(0, pixels);
}

/// Working region: the input that can affect the visible output, padded by the complete support.
/// Align every level to the input's origin and a 32-pixel grid so cropping cannot
/// shift the pyramid taps. The complete support margin also covers every fine level, so
/// output pixels outside the region are transparent and need no pyramid work.
#[derive(Debug, Clone, Copy)]
pub struct PyramidRegion {
    pub origin: [i32; 2],
    pub size: [u32; 2],
    pub input_offset: [i32; 2],
}

impl PyramidRegion {
    /// Complete finite support in device pixels, including every pyramid level.
    fn support_margin(radius: f32, spread_radius: f32) -> Result<u32, BloomError> {
        if !radius.is_finite() || radius < 0.0 || !spread_radius.is_finite() || spread_radius < 0.0
        {
            return Err(BloomError::InvalidParameters);
        }
        let levels = ((f64::from(radius) / 2.0).log2().ceil() as u32).clamp(1, 5);
        let scale = f64::from(1_u32 << levels);
        let spread = (f64::from(spread_radius) / 48.0).max(0.125);
        let margin = if radius == 0.0 {
            0.0
        } else {
            (2.0 * (scale - 1.0)
                + spread.ceil() * scale
                + (0.5 * spread + 1.0) * (2.0 * scale - 2.0))
                .ceil()
        };
        if margin > f64::from(i32::MAX) {
            return Err(BloomError::InvalidImage);
        }
        Ok(margin as u32)
    }

    /// The output pixels of the region, in region pixels. `encode_region` reads only these.
    fn window(&self, output: [u32; 2]) -> Area {
        let span = |origin: i32, size: u32, extent: u32| {
            let start = (-i64::from(origin)).clamp(0, i64::from(size));
            let end = (i64::from(extent) - i64::from(origin)).clamp(start, i64::from(size));
            [start as usize, end as usize]
        };
        Area {
            columns: span(self.origin[0], self.size[0], output[0]),
            rows: span(self.origin[1], self.size[1], output[1]),
        }
    }

    pub fn new(
        input: [u32; 2],
        output: [u32; 2],
        offset: [i32; 2],
        radius: f32,
        spread_radius: f32,
    ) -> Result<Self, BloomError> {
        // Input describes an existing image. Only the output and the allocated working region
        // are limited by MAX_PIXELS; distant source pixels never enter the pyramid.
        source_pixel_count(input[0], input[1])?;
        pixel_count(output[0], output[1])?;
        // Downsample support, widest blur, and all bilinear/tent upsample taps.
        // Retain these zero-filled margins until the complete pyramid is evaluated.
        let margin = i64::from(Self::support_margin(radius, spread_radius)?);

        let mut result = Self {
            origin: [0; 2],
            size: [0; 2],
            input_offset: [0; 2],
        };
        for axis in 0..2 {
            let source = i64::from(offset[axis]);
            let source_end = source + i64::from(input[axis]);
            let visible_end = i64::from(output[axis]);
            // Contributions farther than the complete pyramid support cannot reach the output,
            // and the halo is exactly zero farther than that support from every source pixel.
            // Keep a second zero-filled support margin around the source that can contribute.
            // When none can, the halo is zero everywhere; keep a region around the output.
            let (mut relevant_start, mut relevant_end) =
                (source.max(-margin), source_end.min(visible_end + margin));
            if relevant_start >= relevant_end {
                relevant_start = relevant_start.min(0);
                relevant_end = relevant_end.max(visible_end);
            }
            let start = source + (relevant_start - margin - source).div_euclid(32) * 32;
            let end = relevant_end + margin;
            let size = ((end - start + 31) / 32) * 32;
            result.origin[axis] = start.try_into().map_err(|_| BloomError::InvalidImage)?;
            result.size[axis] = size.try_into().map_err(|_| BloomError::InvalidImage)?;
            result.input_offset[axis] = (source - start)
                .try_into()
                .map_err(|_| BloomError::InvalidImage)?;
        }
        pixel_count(result.size[0], result.size[1])?;
        Ok(result)
    }
}

pub fn apply_bloom_f16(
    input: &[u8],
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    offset: [i32; 2],
    params: BloomParams,
) -> Result<Vec<u8>, BloomError> {
    let region = PyramidRegion::new(
        [input_width, input_height],
        [output_width, output_height],
        offset,
        params.radius,
        params.radius,
    )?;
    let pixels = apply_bloom_region_f16(
        input,
        input_width,
        input_height,
        region.size[0],
        region.size[1],
        region.input_offset,
        region.window([output_width, output_height]),
        params,
    )?;
    Ok(encode_region(
        &pixels,
        region,
        [output_width, output_height],
    ))
}

pub fn apply_glow_f16(
    input: &[u8],
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    offset: [i32; 2],
    params: GlowParams,
) -> Result<Vec<u8>, BloomError> {
    let region = PyramidRegion::new(
        [input_width, input_height],
        [output_width, output_height],
        offset,
        params.radius,
        params.radius * 2.0,
    )?;
    let pixels = apply_glow_region_f16(
        input,
        input_width,
        input_height,
        region.size[0],
        region.size[1],
        region.input_offset,
        region.window([output_width, output_height]),
        params,
    )?;
    Ok(encode_region(
        &pixels,
        region,
        [output_width, output_height],
    ))
}

/// Source ROI is placed at `offset` inside the output ROI. All dimensions are device pixels.
/// Only the pixels of `window` are finished; the others keep the source.
#[allow(clippy::too_many_arguments)]
fn apply_bloom_region_f16(
    input: &[u8],
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    offset: [i32; 2],
    window: Area,
    params: BloomParams,
) -> Result<Vec<[f32; 4]>, BloomError> {
    if !params.threshold.is_finite()
        || !(0.0..=1.0).contains(&params.threshold)
        || !params.knee.is_finite()
        || !(0.0..=1.0).contains(&params.knee)
        || !params.intensity.is_finite()
        || !(0.0..=4.0).contains(&params.intensity)
        || !params.radius.is_finite()
        || params.radius < 0.0
    {
        return Err(BloomError::InvalidParameters);
    }
    let mut base = read_base(
        input,
        input_width,
        input_height,
        output_width,
        output_height,
        offset,
    )?;
    if params.radius == 0.0 || params.intensity == 0.0 {
        return Ok(base);
    }

    let width = output_width as usize;
    let threshold = f64::from(params.threshold);
    let knee = f64::from(params.knee);
    let bloom = pyramid_halo(
        [width, output_height as usize],
        params.radius,
        params.radius,
        window,
        |index| {
            let pixel = base[index];
            let alpha = f64::from(pixel[3]);
            if alpha <= 0.0 {
                return [0.0; 3];
            }
            let luminance = (0.2627 * f64::from(pixel[0])
                + 0.6780 * f64::from(pixel[1])
                + 0.0593 * f64::from(pixel[2]))
                / alpha;
            let weight = if knee == 0.0 {
                f64::from(luminance >= threshold)
            } else {
                let t = ((luminance - threshold + knee) / (2.0 * knee)).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            };
            let [red, green, blue, _] = quantize_halves([
                f64::from(pixel[0]) * weight,
                f64::from(pixel[1]) * weight,
                f64::from(pixel[2]) * weight,
                0.0,
            ]);
            [red, green, blue]
        },
    )?;
    fill_window(&mut base, width, window, |index, source| {
        let halo = &bloom.pixels[index];
        // A positive-zero halo leaves a positive-zero pixel unchanged.
        if is_positive_zero(halo) && is_positive_zero(source) {
            return;
        }
        let channel = |channel: usize| {
            (f64::from(source[channel]) + f64::from(params.intensity) * f64::from(halo[channel]))
                .clamp(-65504.0, 65504.0)
        };
        let [red, green, blue, _] = quantize_halves([channel(0), channel(1), channel(2), 0.0]);
        let halo_alpha = halo.iter().copied().fold(0.0_f32, f32::max).min(1.0);
        *source = [red, green, blue, source[3].max(halo_alpha)];
    });
    Ok(base)
}

/// Alpha-contour glow uses the same reduced-resolution pyramid, then tints the
/// halo and places the original subtree over it. Only the pixels of `window` are finished.
#[allow(clippy::too_many_arguments)]
fn apply_glow_region_f16(
    input: &[u8],
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    offset: [i32; 2],
    window: Area,
    params: GlowParams,
) -> Result<Vec<[f32; 4]>, BloomError> {
    if !params.intensity.is_finite()
        || !(0.0..=4.0).contains(&params.intensity)
        || !params.radius.is_finite()
        || params.radius < 0.0
        || params.color.iter().any(|value| !value.is_finite())
        || !(0.0..=1.0).contains(&params.color[3])
    {
        return Err(BloomError::InvalidParameters);
    }
    let mut base = read_base(
        input,
        input_width,
        input_height,
        output_width,
        output_height,
        offset,
    )?;
    if params.radius == 0.0 || params.intensity == 0.0 || params.color[3] == 0.0 {
        return Ok(base);
    }
    let width = output_width as usize;
    // Keep the level count tied to the authored radius while widening each tent
    // tap to approximate a Gaussian sigma without adding a global coarse tail.
    let halo = pyramid_halo(
        [width, output_height as usize],
        params.radius,
        params.radius * 2.0,
        window,
        |index| [base[index][3]],
    )?;
    let peak_color = params.color[..3].iter().copied().fold(0.0_f32, f32::max);
    fill_window(&mut base, width, window, |index, source| {
        let sample = &halo.pixels[index];
        // A positive-zero halo leaves a positive-zero pixel unchanged.
        if is_positive_zero(sample) && is_positive_zero(source) {
            return;
        }
        let gain = f64::from(sample[0]) * f64::from(params.intensity) * GLOW_RESPONSE_GAIN;
        let color_gain = if peak_color > 0.0 {
            gain.min(1.0 / f64::from(peak_color))
        } else {
            gain
        };
        let source_alpha = f64::from(source[3]);
        let halo_alpha = (gain * f64::from(params.color[3])).min(1.0);
        let channel = |channel: usize| {
            (f64::from(source[channel])
                + color_gain * f64::from(params.color[channel]) * (1.0 - source_alpha))
                .clamp(-65504.0, 65504.0)
        };
        *source = quantize_halves([
            channel(0),
            channel(1),
            channel(2),
            source_alpha + halo_alpha * (1.0 - source_alpha),
        ]);
    });
    Ok(base)
}

/// Applies `composite` to every pixel of `window` in a region `width` pixels wide, passing the
/// pixel's index in the region.
fn fill_window(
    pixels: &mut [[f32; 4]],
    width: usize,
    window: Area,
    composite: impl Fn(usize, &mut [f32; 4]) + Sync,
) {
    let [left, right] = window.columns;
    let [top, bottom] = window.rows;
    fill_image_rows(
        &mut pixels[top * width..bottom * width],
        width,
        |first, rows| {
            for (index, row) in rows.chunks_exact_mut(width).enumerate() {
                let start = (top + first + index) * width;
                for (x, pixel) in (left..right).zip(&mut row[left..right]) {
                    composite(start + x, pixel);
                }
            }
        },
    );
}

fn is_positive_zero<const CHANNELS: usize>(pixel: &[f32; CHANNELS]) -> bool {
    pixel.iter().all(|channel| channel.to_bits() == 0)
}

/// Encode the output window of the region directly, avoiding a padded F16 allocation followed
/// by decoding into another full-size f32 image just to crop it. Output pixels outside the
/// region are farther than the pyramid support from every source pixel and stay transparent.
fn encode_region(pixels: &[[f32; 4]], region: PyramidRegion, output: [u32; 2]) -> Vec<u8> {
    let (width, height) = (output[0] as usize, output[1] as usize);
    let region_width = region.size[0] as usize;
    let span = |origin: i32, size: u32, extent: usize| {
        let start = i64::from(origin).clamp(0, extent as i64);
        let end = (i64::from(origin) + i64::from(size)).clamp(start, extent as i64);
        (start as usize, end as usize)
    };
    let (left, right) = span(region.origin[0], region.size[0], width);
    let (top, bottom) = span(region.origin[1], region.size[1], height);
    let column = (left as i64 - i64::from(region.origin[0])) as usize;
    let mut bytes = vec![0_u8; width * height * 8];
    fill_image_rows(&mut bytes, width * 8, |first, rows| {
        for (index, encoded) in rows.chunks_exact_mut(width * 8).enumerate() {
            let y = first + index;
            if (top..bottom).contains(&y) {
                let row = (y as i64 - i64::from(region.origin[1])) as usize * region_width;
                encode_pixels(
                    &pixels[row + column..row + column + right - left],
                    &mut encoded[left * 8..right * 8],
                );
            }
        }
    });
    bytes
}

pub(crate) fn read_base(
    input: &[u8],
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    offset: [i32; 2],
) -> Result<Vec<[f32; 4]>, BloomError> {
    let input_count = source_pixel_count(input_width, input_height)?;
    let output_count = pixel_count(output_width, output_height)?;
    if input_count.checked_mul(8) != Some(input.len()) {
        return Err(BloomError::InvalidImage);
    }
    let iw = input_width as usize;
    let ih = input_height as usize;
    let ow = output_width as usize;
    let oh = output_height as usize;
    let row_bytes = iw * 8;
    // Decodes one input row into its output row, if it has one. Every input pixel must be
    // finite, including those placed outside the output.
    let decode_row = |row: &[u8], mut target: Option<&mut [[f32; 4]]>| {
        for (x, pixel) in row.as_chunks::<8>().0.iter().enumerate() {
            let Some(channels) = decode_pixel(pixel) else {
                return false;
            };
            let dx = x as i64 + i64::from(offset[0]);
            if let Some(target) = target.as_deref_mut()
                && dx >= 0
                && dx < ow as i64
            {
                target[dx as usize] = channels;
            }
        }
        true
    };
    let mut base = vec![[0.0_f32; 4]; output_count];
    let invalid = std::sync::atomic::AtomicBool::new(false);
    fill_image_rows(&mut base, ow, |first, rows| {
        for (index, target) in rows.chunks_exact_mut(ow).enumerate() {
            let y = (first + index) as i64 - i64::from(offset[1]);
            if (0..ih as i64).contains(&y) {
                let start = y as usize * row_bytes;
                if !decode_row(&input[start..start + row_bytes], Some(target)) {
                    invalid.store(true, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
            }
        }
    });
    let placed = (-i64::from(offset[1])).clamp(0, ih as i64) as usize
        ..(oh as i64 - i64::from(offset[1])).clamp(0, ih as i64) as usize;
    if invalid.into_inner()
        || input
            .chunks_exact(row_bytes)
            .enumerate()
            .any(|(y, row)| !placed.contains(&y) && !decode_row(row, None))
    {
        return Err(BloomError::InvalidImage);
    }
    Ok(base)
}

/// Runs the pyramid on the level-0 image whose pixel at each index `extract` returns. Only
/// the pixels that `window` of the output depends on are computed: walking the passes backwards,
/// each one needs the pixels its taps read from its inputs. Every other pixel stays zero, and
/// no computed pixel reads it.
fn pyramid_halo<const CHANNELS: usize>(
    [width, height]: [usize; 2],
    depth_radius: f32,
    spread_radius: f32,
    window: Area,
    extract: impl Fn(usize) -> [f32; CHANNELS] + Sync,
) -> Result<Level<CHANNELS>, BloomError> {
    let graph = PyramidGraph::new(
        u32::try_from(width).map_err(|_| BloomError::InvalidImage)?,
        u32::try_from(height).map_err(|_| BloomError::InvalidImage)?,
        depth_radius,
        spread_radius,
    )?;
    let mut sizes = vec![[width, height]; graph.resource_count()];
    for pass in graph.passes() {
        sizes[pass.output()] = [pass.width() as usize, pass.height() as usize];
    }
    let axes = graph
        .passes()
        .iter()
        .map(|pass| match pass.kind() {
            PyramidPassKind::Downsample { .. } => TentAxes::default(),
            PyramidPassKind::Blur { input } => TentAxes::blur(sizes[input], graph.spread()),
            PyramidPassKind::Upsample { coarse, .. } => {
                TentAxes::upsample(sizes[pass.output()], sizes[coarse], graph.spread())
            }
        })
        .collect::<Vec<_>>();
    let mut areas = vec![Area::default(); graph.resource_count()];
    areas[graph.output()] = window;
    for (pass, axes) in graph.passes().iter().zip(&axes).rev() {
        let area = areas[pass.output()];
        let mut read = |resource: usize, support: Area| {
            areas[resource] = areas[resource].union(support);
        };
        match pass.kind() {
            PyramidPassKind::Downsample { input } => {
                read(input, downsample_support(area, sizes[input]));
            }
            PyramidPassKind::Blur { input } => read(input, axes.support(area)),
            PyramidPassKind::Upsample { coarse, fine, .. } => {
                read(coarse, axes.support(area));
                if let Some(fine) = fine {
                    read(fine, area);
                }
            }
        }
    }
    let mut base = Level::empty(width, height);
    let [left, right] = areas[0].columns;
    fill_rows(&mut base, areas[0].rows, |first, rows| {
        for (index, row) in rows.chunks_mut(width).enumerate() {
            let start = (first + index) * width;
            for (x, pixel) in (left..right).zip(&mut row[left..right]) {
                *pixel = extract(start + x);
            }
        }
    });
    let mut images = vec![None; graph.resource_count()];
    images[0] = Some(base);
    for (pass, axes) in graph.passes().iter().zip(&axes) {
        let area = areas[pass.output()];
        let source = |id: usize| images[id].as_ref().expect("pyramid input is live");
        let image = match pass.kind() {
            PyramidPassKind::Downsample { input } => downsample_13(source(input), area),
            PyramidPassKind::Blur { input } => tent_9(source(input), sizes[input], axes, area),
            PyramidPassKind::Upsample {
                coarse,
                fine,
                local_weight,
            } => {
                let mut up = tent_9(source(coarse), sizes[pass.output()], axes, area);
                if let Some(fine) = fine {
                    let fine = source(fine);
                    let [left, right] = area.columns;
                    fill_rows(&mut up, area.rows, |first, rows| {
                        let mix = |local: f32, far: f32| {
                            local_weight * f64::from(local) + (1.0 - local_weight) * f64::from(far)
                        };
                        for (index, row) in rows.chunks_mut(fine.width).enumerate() {
                            let start = (first + index) * fine.width;
                            let (far, far_tail) =
                                row[left..right].as_flattened_mut().as_chunks_mut();
                            let (local, local_tail) = fine.pixels[start + left..start + right]
                                .as_flattened()
                                .as_chunks::<4>();
                            for (far, local) in far.iter_mut().zip(local) {
                                *far = quantize_halves(std::array::from_fn(|channel| {
                                    mix(local[channel], far[channel])
                                }));
                            }
                            for (far, local) in far_tail.iter_mut().zip(local_tail) {
                                *far = quantize_half(mix(*local, *far));
                            }
                        }
                    });
                }
                up
            }
        };
        debug_assert_eq!(
            (image.width, image.height),
            (pass.width() as usize, pass.height() as usize)
        );
        images[pass.output()] = Some(image);
        for &resource in pass.retire_after() {
            images[resource] = None;
        }
    }
    Ok(images[graph.output()]
        .take()
        .expect("pyramid output is live"))
}

fn source_pixel_count(width: u32, height: u32) -> Result<usize, BloomError> {
    let count = (width as usize)
        .checked_mul(height as usize)
        .ok_or(BloomError::InvalidImage)?;
    if count == 0 {
        return Err(BloomError::InvalidImage);
    }
    Ok(count)
}

pub(crate) fn pixel_count(width: u32, height: u32) -> Result<usize, BloomError> {
    let count = source_pixel_count(width, height)?;
    if count > MAX_PIXELS {
        return Err(BloomError::InvalidImage);
    }
    Ok(count)
}

const TILE_WIDTH: usize = 8;
const TILE_HEIGHT: usize = 4;

/// Output tiles of `TILE_WIDTH` x `TILE_HEIGHT` pixels that read a nonzero source pixel. The
/// others read only zeros, whose every sum is +0.0, so they keep the zero they start with.
struct ActiveTiles {
    columns: usize,
    active: Vec<bool>,
}

impl ActiveTiles {
    fn active(&self, tile_x: usize, tile_y: usize) -> bool {
        self.active[tile_y * self.columns + tile_x]
    }
}

/// The widest run of columns a pass computes at once. Helpers size their row buffers by the
/// run; the allocator keeps memory per thread and size class, so buffers as wide as a level
/// added up across helpers.
const RUN_COLUMNS: usize = 64;

/// Appends the `columns` of a tile to `runs`, extending the last run if it ends where the tile
/// starts and stays within `RUN_COLUMNS`.
fn push_run(runs: &mut Vec<[usize; 2]>, columns: [usize; 2]) {
    match runs.last_mut() {
        Some(run) if run[1] == columns[0] && columns[1] - run[0] <= RUN_COLUMNS => {
            run[1] = columns[1];
        }
        _ => runs.push(columns),
    }
}

/// `columns[i]` and `rows[j]` bound the source pixels that tile column `i` and tile row `j` read.
/// Only `support` is examined: the computed pixels read nothing else, so a tile whose examined
/// pixels are zero computes zeros. Small or mostly occupied sources compute every tile and
/// return `None`.
fn active_tiles<const CHANNELS: usize>(
    source: &Level<CHANNELS>,
    support: Area,
    columns: &[Option<(usize, usize)>],
    rows: &[Option<(usize, usize)>],
) -> Option<ActiveTiles> {
    if source.pixels.len() < 16_384 {
        return None;
    }
    let stride = source.width.div_ceil(64);
    let mut occupied = vec![0_u64; stride * source.height];
    let [left, right] = support.columns;
    let [top, bottom] = support.rows;
    let half = (right - left) * (bottom - top) / 2;
    let mut count = 0;
    for y in top..bottom {
        let row = &source.pixels[y * source.width..(y + 1) * source.width];
        for word in left / 64..right.div_ceil(64) {
            let pixels = &row[64 * word..(64 * word + 64).min(source.width)];
            // Most words of a sparse level are empty; signed zeros count as empty.
            let any = pixels
                .as_flattened()
                .iter()
                .fold(0, |any, channel| any | channel.to_bits() << 1);
            if any == 0 {
                continue;
            }
            let mut bits = 0;
            for (bit, pixel) in pixels.iter().enumerate() {
                bits |= u64::from(pixel.iter().any(|channel| *channel != 0.0)) << bit;
            }
            occupied[y * stride + word] = bits;
            count += bits.count_ones() as usize;
        }
        // Avoid building output tiles for dense images.
        if count > half {
            return None;
        }
    }
    let mut active = vec![false; columns.len() * rows.len()];
    for (tile_y, yrange) in rows.iter().enumerate() {
        let Some((top, bottom)) = *yrange else {
            continue;
        };
        for (tile_x, xrange) in columns.iter().enumerate() {
            let Some((left, right)) = *xrange else {
                continue;
            };
            let first_word = left / 64;
            let last_word = right / 64;
            active[tile_y * columns.len() + tile_x] = (top..=bottom).any(|y| {
                (first_word..=last_word).any(|word| {
                    let start = if word == first_word { left % 64 } else { 0 };
                    let end = if word == last_word { right % 64 } else { 63 };
                    let mask = (u64::MAX << start) & (u64::MAX >> (63 - end));
                    occupied[y * stride + word] & mask != 0
                })
            });
        }
    }
    Some(ActiveTiles {
        columns: columns.len(),
        active,
    })
}

// The entire support must be zero, including bilinear neighbours and every tent
// tap. Occupied tiles retain the reference summation and F16 rounding order.
fn tent_tiles<const CHANNELS: usize>(
    source: &Level<CHANNELS>,
    support: Area,
    columns: &[TentAxis],
    lines: &[TentAxis],
) -> Option<ActiveTiles> {
    active_tiles(
        source,
        support,
        &columns.chunks(TILE_WIDTH).map(tap_span).collect::<Vec<_>>(),
        &lines.chunks(TILE_HEIGHT).map(tap_span).collect::<Vec<_>>(),
    )
}

/// The tiles of the downsample of `source`: each reads offsets -2..=3 around twice its pixels.
fn downsample_tiles<const CHANNELS: usize>(
    source: &Level<CHANNELS>,
    support: Area,
) -> Option<ActiveTiles> {
    let spans = |size: usize, extent: usize| {
        (0..extent.div_ceil(2).div_ceil(size))
            .map(|tile| {
                Some((
                    (2 * tile * size).saturating_sub(2),
                    (2 * (tile + 1) * size + 1).min(extent - 1),
                ))
            })
            .collect::<Vec<_>>()
    };
    active_tiles(
        source,
        support,
        &spans(TILE_WIDTH, source.width),
        &spans(TILE_HEIGHT, source.height),
    )
}

/// The 13-tap downsample of one pixel from its taps at `DOWN_TAPS` offsets.
fn downsample_pixel<const CHANNELS: usize>(
    tap: impl Fn(i32, i32) -> [f32; CHANNELS],
) -> [f32; CHANNELS] {
    let mut sum = [0.0_f64; CHANNELS];
    for &(dx, dy, weight) in &DOWN_TAPS {
        for (acc, value) in sum.iter_mut().zip(tap(dx, dy)) {
            *acc += weight * f64::from(value);
        }
    }
    sum.map(|channel| quantize_half(channel / 80.0))
}

/// The even and odd source pixels, widened, of the six rows a span of interior pixels reads.
#[derive(Default)]
struct DownsampleRows<const CHANNELS: usize> {
    even: Vec<[f64; CHANNELS]>,
    odd: Vec<[f64; CHANNELS]>,
    sums: Vec<[f64; CHANNELS]>,
}

/// Downsamples `columns` of row `y`, whose taps all lie inside the source, one tap at a time
/// across the span. Each pixel still adds its taps in `DOWN_TAPS` order.
fn downsample_inner<const CHANNELS: usize>(
    source: &Level<CHANNELS>,
    y: usize,
    columns: std::ops::Range<usize>,
    out: &mut [[f32; CHANNELS]],
    rows: &mut DownsampleRows<CHANNELS>,
) {
    // Pixel x reads source columns 2x - 2..=2x + 3: even and odd columns x - 1..=x + 1.
    let span = columns.len() + 2;
    rows.even.clear();
    rows.even.resize(6 * span, [0.0; CHANNELS]);
    rows.odd.clear();
    rows.odd.resize(6 * span, [0.0; CHANNELS]);
    for line in 0..6 {
        let row = &source.pixels[(2 * y + line - 2) * source.width..][..source.width];
        let (pairs, _) = row[2 * (columns.start - 1)..2 * (columns.end + 1)].as_chunks::<2>();
        let even = &mut rows.even[line * span..(line + 1) * span];
        let odd = &mut rows.odd[line * span..(line + 1) * span];
        for ((pair, even), odd) in pairs.iter().zip(even).zip(odd) {
            *even = pair[0].map(f64::from);
            *odd = pair[1].map(f64::from);
        }
    }
    rows.sums.clear();
    rows.sums.resize(columns.len(), [0.0; CHANNELS]);
    for &(dx, dy, weight) in &DOWN_TAPS {
        let parity = if dx % 2 == 0 { &rows.even } else { &rows.odd };
        let start = (dy + 2) as usize * span + (dx.div_euclid(2) + 1) as usize;
        let values = parity[start..start + columns.len()].as_flattened();
        for (sum, value) in rows.sums.as_flattened_mut().iter_mut().zip(values) {
            *sum += weight * value;
        }
    }
    quantize_into(out.as_flattened_mut(), rows.sums.as_flattened(), 80.0);
}

fn downsample_13<const CHANNELS: usize>(source: &Level<CHANNELS>, area: Area) -> Level<CHANNELS> {
    let mut next = Level::empty(source.width.div_ceil(2), source.height.div_ceil(2));
    if area.is_empty() {
        return next;
    }
    let tiles = downsample_tiles(
        source,
        downsample_support(area, [source.width, source.height]),
    );
    let width = next.width;
    let [left, right] = area.columns;
    // Pixels whose taps all lie inside the source.
    let inner_columns = 1..source.width.saturating_sub(2) / 2;
    let inner_rows = 1..source.height.saturating_sub(2) / 2;
    fill_rows(&mut next, area.rows, |first, rows| {
        let mut runs: Vec<[usize; 2]> = Vec::new();
        let mut buffers = DownsampleRows::default();
        for (index, row) in rows.chunks_mut(width).enumerate() {
            let y = first + index;
            runs.clear();
            for tile_x in left / TILE_WIDTH..right.div_ceil(TILE_WIDTH) {
                if tiles
                    .as_ref()
                    .is_some_and(|tiles| !tiles.active(tile_x, y / TILE_HEIGHT))
                {
                    continue;
                }
                push_run(
                    &mut runs,
                    [
                        (tile_x * TILE_WIDTH).max(left),
                        ((tile_x + 1) * TILE_WIDTH).min(right),
                    ],
                );
            }
            for &[start, end] in &runs {
                let inner = if inner_rows.contains(&y) {
                    let inner_start = start.max(inner_columns.start).min(end);
                    inner_start..end.min(inner_columns.end).max(inner_start)
                } else {
                    start..start
                };
                for x in (start..inner.start).chain(inner.end..end) {
                    let (cx, cy) = ((2 * x) as i32, (2 * y) as i32);
                    row[x] = downsample_pixel(|dx, dy| source.at(cx + dx, cy + dy));
                }
                if !inner.is_empty() {
                    downsample_inner(source, y, inner.clone(), &mut row[inner], &mut buffers);
                }
            }
        }
    });
    next
}

/// Output rows that share the horizontal sums of the source rows they read.
const TENT_BAND: usize = 16;

/// The 1-2-1 tent of `area` of a `width` x `height` level from `source`, divided by its full
/// weight of 16. Each band of rows forms the horizontal sum of every source row it reads once,
/// then weights those sums for each of its rows. Every sum keeps the tap order of
/// `Level::tent`, and tiles whose whole support is zero are skipped, so no sample changes.
fn tent_9<const CHANNELS: usize>(
    source: &Level<CHANNELS>,
    [width, height]: [usize; 2],
    axes: &TentAxes,
    area: Area,
) -> Level<CHANNELS> {
    let mut next = Level::empty(width, height);
    if area.is_empty() {
        return next;
    }
    let tiles = tent_tiles(source, axes.support(area), &axes.columns, &axes.lines);
    let [left, right] = area.columns;
    fill_rows(&mut next, area.rows, |first, rows| {
        let end = first + rows.len() / width;
        let mut runs: Vec<[usize; 2]> = Vec::new();
        let mut lines = Vec::new();
        let mut sums = Vec::new();
        for start in (first..end).step_by(TENT_BAND) {
            let band = start..(start + TENT_BAND).min(end);
            let Some((top, bottom)) = tap_span(&axes.lines[band.clone()]) else {
                continue;
            };
            // Columns with an active tile in the band, joined into runs.
            runs.clear();
            for tile_x in left / TILE_WIDTH..right.div_ceil(TILE_WIDTH) {
                let active = tiles.as_ref().is_none_or(|tiles| {
                    (band.start / TILE_HEIGHT..=(band.end - 1) / TILE_HEIGHT)
                        .any(|tile_y| tiles.active(tile_x, tile_y))
                });
                if !active {
                    continue;
                }
                push_run(
                    &mut runs,
                    [
                        (tile_x * TILE_WIDTH).max(left),
                        ((tile_x + 1) * TILE_WIDTH).min(right),
                    ],
                );
            }
            for &[run_left, run_right] in &runs {
                let len = run_right - run_left;
                lines.clear();
                lines.resize((bottom + 1 - top) * len, [0.0_f64; CHANNELS]);
                for (row, line) in source
                    .pixels
                    .chunks_exact(source.width)
                    .skip(top)
                    .zip(lines.chunks_exact_mut(len))
                {
                    for (axis, sum) in axes.columns[run_left..run_right].iter().zip(line) {
                        for &(sx, wx) in axis.taps() {
                            for (acc, value) in sum.iter_mut().zip(row[sx]) {
                                *acc += wx * f64::from(value);
                            }
                        }
                    }
                }
                for y in band.clone() {
                    sums.clear();
                    sums.resize(len, [0.0_f64; CHANNELS]);
                    for &(sy, wy) in axes.lines[y].taps() {
                        let line = &lines[(sy - top) * len..(sy - top + 1) * len];
                        for (acc, value) in
                            sums.as_flattened_mut().iter_mut().zip(line.as_flattened())
                        {
                            *acc += wy * value;
                        }
                    }
                    let row = &mut rows[(y - first) * width..(y - first + 1) * width];
                    quantize_into(
                        row[run_left..run_right].as_flattened_mut(),
                        sums.as_flattened(),
                        16.0,
                    );
                }
            }
        }
    });
    next
}

#[cfg(test)]
fn upsample_9<const CHANNELS: usize>(
    source: &Level<CHANNELS>,
    width: usize,
    height: usize,
    spread: f64,
) -> Level<CHANNELS> {
    let axes = TentAxes::upsample([width, height], [source.width, source.height], spread);
    tent_9(source, [width, height], &axes, Area::full(width, height))
}

pub(crate) fn encode_output(pixels: &[[f32; 4]]) -> Vec<u8> {
    // The pixels need not form rows of a known width; encode them in fixed spans instead.
    const SPAN: usize = 1024;
    let mut bytes = vec![0_u8; pixels.len() * 8];
    fill_image_rows(&mut bytes, SPAN * 8, |first, spans| {
        encode_pixels(&pixels[first * SPAN..], spans);
    });
    bytes
}

/// Encodes leading `pixels` into every 8-byte slot of `bytes` as little-endian F16.
fn encode_pixels(pixels: &[[f32; 4]], bytes: &mut [u8]) {
    for (pixel, target) in pixels.iter().zip(bytes.as_chunks_mut::<8>().0) {
        #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
        {
            if neon::encode(pixel, target) {
                continue;
            }
        }
        for (&channel, bytes) in pixel.iter().zip(target.as_chunks_mut::<2>().0) {
            *bytes = f32_to_half(channel).to_le_bytes();
        }
    }
}

/// Widens a pixel of four little-endian halves, or returns `None` if a channel is not finite.
#[inline(always)]
fn decode_pixel(bytes: &[u8; 8]) -> Option<[f32; 4]> {
    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    {
        neon::decode(bytes)
    }
    #[cfg(not(all(target_arch = "aarch64", target_endian = "little")))]
    {
        let mut channels = [0.0; 4];
        for (channel, bytes) in channels.iter_mut().zip(bytes.as_chunks::<2>().0) {
            *channel = half_to_f32(u16::from_le_bytes(*bytes));
            if !channel.is_finite() {
                return None;
            }
        }
        Some(channels)
    }
}

/// The aarch64 baseline converts between f32 and binary16 in hardware. Both directions round to
/// nearest even, so every value that is not NaN gets the bits `f32_to_half` and `half_to_f32`
/// give it; NaNs keep the portable path, because the hardware keeps their payloads.
#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
mod neon {
    use std::arch::aarch64::*;

    /// Rounds `value`, which is not NaN, to binary16 and back.
    #[inline(always)]
    pub(super) fn round(value: f32) -> f32 {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe { vgetq_lane_f32::<0>(vcvt_f32_f16(vcvt_f16_f32(vdupq_n_f32(value)))) }
    }

    #[inline(always)]
    pub(super) fn decode(bytes: &[u8; 8]) -> Option<[f32; 4]> {
        // SAFETY: NEON is part of the aarch64 baseline; the load reads the eight bytes and the
        // store writes the four floats of `pixel`.
        unsafe {
            let wide = vcvt_f32_f16(vreinterpret_f16_u16(vreinterpret_u16_u8(vld1_u8(
                bytes.as_ptr(),
            ))));
            if vminvq_u32(vcltq_f32(vabsq_f32(wide), vdupq_n_f32(f32::INFINITY))) != u32::MAX {
                return None;
            }
            let mut pixel = [0.0; 4];
            vst1q_f32(pixel.as_mut_ptr(), wide);
            Some(pixel)
        }
    }

    /// Rounds four values to binary16 and back, or returns `None` if one is NaN.
    #[inline(always)]
    pub(super) fn round4(values: [f32; 4]) -> Option<[f32; 4]> {
        // SAFETY: NEON is part of the aarch64 baseline; the load reads the four floats of
        // `values` and the store writes the four floats of `rounded`.
        unsafe {
            let wide = vld1q_f32(values.as_ptr());
            if vminvq_u32(vceqq_f32(wide, wide)) != u32::MAX {
                return None;
            }
            let mut rounded = [0.0; 4];
            vst1q_f32(rounded.as_mut_ptr(), vcvt_f32_f16(vcvt_f16_f32(wide)));
            Some(rounded)
        }
    }

    /// Writes four little-endian halves, or returns `false` if a channel is NaN.
    #[inline(always)]
    pub(super) fn encode(pixel: &[f32; 4], bytes: &mut [u8; 8]) -> bool {
        // SAFETY: NEON is part of the aarch64 baseline; the load reads the four floats of
        // `pixel` and the store writes the eight bytes.
        unsafe {
            let wide = vld1q_f32(pixel.as_ptr());
            if vminvq_u32(vceqq_f32(wide, wide)) != u32::MAX {
                return false;
            }
            let halves = vreinterpret_u16_f16(vcvt_f16_f32(wide));
            vst1_u8(bytes.as_mut_ptr(), vreinterpret_u8_u16(halves));
            true
        }
    }
}

/// [`quantize_half`] of four values.
#[inline]
pub(crate) fn quantize_halves(values: [f64; 4]) -> [f32; 4] {
    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    {
        if let Some(rounded) = neon::round4(values.map(|value| value as f32)) {
            return rounded;
        }
    }
    values.map(quantize_half)
}

/// Writes `quantize_half(sum / divisor)` of every sum.
fn quantize_into(out: &mut [f32], sums: &[f64], divisor: f64) {
    let (out, out_tail) = out.as_chunks_mut::<4>();
    let (sums, sums_tail) = sums.as_chunks::<4>();
    for (out, sum) in out.iter_mut().zip(sums) {
        *out = quantize_halves(sum.map(|sum| sum / divisor));
    }
    for (out, sum) in out_tail.iter_mut().zip(sums_tail) {
        *out = quantize_half(sum / divisor);
    }
}

#[inline]
pub(crate) fn quantize_half(value: f64) -> f32 {
    let value = value as f32;
    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    {
        if !value.is_nan() {
            return neon::round(value);
        }
    }
    let bits = value.to_bits();
    let magnitude = bits & 0x7fff_ffff;
    // Values through the halfway point below the smallest half subnormal round to
    // signed zero. Normal half values only need their low 13 mantissa bits rounded
    // away; keep the f32 exponent rather than encoding and decoding it again.
    if magnitude <= 0x3300_0000 {
        return f32::from_bits(bits & 0x8000_0000);
    }
    if (0x3880_0000..0x477f_f000).contains(&magnitude) {
        return f32::from_bits((bits + 0x0fff + ((bits >> 13) & 1)) & !0x1fff);
    }
    if magnitude < 0x3880_0000 {
        // Half subnormals lie on the 2^-24 grid. Power-of-two scaling is exact
        // throughout this bounded f32 range; ties-to-even matches f32_to_half,
        // including signed zero, without encoding and decoding the exponent.
        return (value * 16_777_216.0).round_ties_even() * (1.0 / 16_777_216.0);
    }
    // Preserve the reference handling of overflow, infinities and NaNs.
    half_to_f32(f32_to_half(value))
}

/// Widen binary16 to f32. Every half value, subnormals included, is exact in f32.
pub fn half_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits & 0x8000) << 16;
    let exponent = u32::from((bits >> 10) & 0x1f);
    let fraction = u32::from(bits & 0x03ff);
    if exponent == 0 {
        if fraction == 0 {
            return f32::from_bits(sign);
        }
        let value = fraction as f32 / 16_777_216.0;
        return if sign == 0 { value } else { -value };
    }
    if exponent == 31 {
        return f32::from_bits(sign | 0x7f80_0000 | (fraction << 13));
    }
    f32::from_bits(sign | ((exponent + 112) << 23) | (fraction << 13))
}

/// Round a working-linear channel to binary16 for F16 staging images.
pub fn f32_to_half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x7f_ffff;
    if exponent == 0xff {
        return sign | if mantissa == 0 { 0x7c00 } else { 0x7e00 };
    }
    let half_exponent = exponent - 127 + 15;
    if half_exponent >= 31 {
        return sign | 0x7c00;
    }
    if half_exponent <= 0 {
        if half_exponent < -10 {
            return sign;
        }
        return sign | round_even(mantissa | 0x80_0000, (14 - half_exponent) as u32) as u16;
    }
    let rounded = round_even(mantissa, 13);
    if rounded == 0x400 {
        return sign | (((half_exponent + 1) as u16) << 10);
    }
    sign | ((half_exponent as u16) << 10) | rounded as u16
}

fn round_even(value: u32, shift: u32) -> u32 {
    let whole = value >> shift;
    let remainder = value & ((1u32 << shift) - 1);
    let halfway = 1u32 << (shift - 1);
    whole + u32::from(remainder > halfway || (remainder == halfway && whole & 1 != 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn sparse_upsampling_preserves_every_reference_f16_sample() {
        fn check<const CHANNELS: usize>(spread: f64, dense: bool) {
            let mut source = Level::<CHANNELS>::empty(129, 131);
            for (index, pixel) in source.pixels.iter_mut().enumerate() {
                for (channel, value) in pixel.iter_mut().enumerate() {
                    if dense || index % 37 == 0 || index == 129 * 131 - 1 {
                        *value = quantize_half((index % 29) as f64 / 16.0 - channel as f64 * 0.75);
                    } else if index % 13 == 0 {
                        *value = -0.0;
                    }
                }
            }
            let (width, height) = (257, 263);
            let result = upsample_9(&source, width, height, spread);
            let center = |position: usize, extent: usize, source_extent: usize| {
                (position as f64 + 0.5) * source_extent as f64 / extent as f64 - 0.5
            };
            let columns = (0..width)
                .map(|x| TentAxis::new(center(x, width, source.width), 0.5 * spread, source.width))
                .collect::<Vec<_>>();
            let lines = (0..height)
                .map(|y| {
                    TentAxis::new(
                        center(y, height, source.height),
                        0.5 * spread,
                        source.height,
                    )
                })
                .collect::<Vec<_>>();
            let mask = tent_tiles(
                &source,
                Area::full(source.width, source.height),
                &columns,
                &lines,
            );
            if dense {
                assert!(mask.is_none());
            } else if spread < 1.0 {
                assert!(mask.as_ref().unwrap().active.iter().any(|active| !active));
            }
            // Nine bilinear taps, each read separately with zero outside the level.
            let at = |x: f64, y: f64, channel: usize| {
                let inside = |p: f64, extent: usize| p >= 0.0 && p < extent as f64;
                if inside(x, source.width) && inside(y, source.height) {
                    f64::from(source.pixels[y as usize * source.width + x as usize][channel])
                } else {
                    0.0
                }
            };
            let per_tap = |cx: f64, cy: f64, channel: usize| {
                let mut sum = 0.0;
                for (dy, wy) in [(-1.0, 1.0), (0.0, 2.0), (1.0, 1.0)] {
                    for (dx, wx) in [(-1.0, 1.0), (0.0, 2.0), (1.0, 1.0)] {
                        let (px, py) = (cx + dx * 0.5 * spread, cy + dy * 0.5 * spread);
                        let (x0, y0) = (px.floor(), py.floor());
                        let (fx, fy) = (px - x0, py - y0);
                        sum += wx
                            * wy
                            * ((1.0 - fy)
                                * ((1.0 - fx) * at(x0, y0, channel)
                                    + fx * at(x0 + 1.0, y0, channel))
                                + fy * ((1.0 - fx) * at(x0, y0 + 1.0, channel)
                                    + fx * at(x0 + 1.0, y0 + 1.0, channel)));
                    }
                }
                sum
            };
            for y in 0..height {
                for x in 0..width {
                    let sum = source.tent(&columns[x], &lines[y]);
                    for (channel, folded) in sum.iter().enumerate() {
                        let reference = per_tap(
                            center(x, width, source.width),
                            center(y, height, source.height),
                            channel,
                        );
                        assert!(
                            (folded - reference).abs() <= 1e-12 * (1.0 + reference.abs()),
                            "folded tent ({x},{y}), spread={spread}: {folded} vs {reference}"
                        );
                    }
                    let expected = sum.map(|value| quantize_half(value / 16.0));
                    assert_eq!(
                        result.pixels[y * width + x].map(f32::to_bits),
                        expected.map(f32::to_bits),
                        "sample ({x},{y}), spread={spread}, dense={dense}"
                    );
                }
            }
        }
        for spread in [0.125, 5.0 / 24.0, 0.999, 2.625, 6.25] {
            check::<1>(spread, false);
            check::<3>(spread, false);
        }
        check::<1>(0.125, true);
        check::<3>(2.625, true);
    }

    #[test]
    fn pyramid_passes_match_per_pixel_sums_in_every_area() {
        fn check<const CHANNELS: usize>(dense: bool) {
            let (width, height) = (181, 133);
            let mut source = Level::<CHANNELS>::empty(width, height);
            for (index, pixel) in source.pixels.iter_mut().enumerate() {
                let (x, y) = (index % width, index / width);
                for (channel, value) in pixel.iter_mut().enumerate() {
                    if dense || ((x / 23 + y / 17) % 4 == 0 && (x + y) % 3 != 0) {
                        *value =
                            quantize_half(((x * 7 + y * 13 + channel * 5) % 31) as f64 / 8.0 - 1.5);
                    } else if index % 11 == 0 {
                        *value = -0.0;
                    }
                }
            }
            // Edges inside tiles, single pixels, borders, the whole level and nothing.
            let areas = |w: usize, h: usize| {
                [
                    Area::full(w, h),
                    Area {
                        columns: [3, w / 2 + 5],
                        rows: [5, h / 3 + 2],
                    },
                    Area {
                        columns: [w - 1, w],
                        rows: [0, 1],
                    },
                    Area {
                        columns: [w / 3, w - 2],
                        rows: [h - 7, h],
                    },
                    Area::default(),
                ]
            };
            let check_level = |result: &Level<CHANNELS>,
                               area: Area,
                               expected: &dyn Fn(usize, usize) -> [f32; CHANNELS],
                               label: &str| {
                for y in 0..result.height {
                    for x in 0..result.width {
                        let inside = (area.columns[0]..area.columns[1]).contains(&x)
                            && (area.rows[0]..area.rows[1]).contains(&y);
                        let wanted = if inside {
                            expected(x, y)
                        } else {
                            [0.0; CHANNELS]
                        };
                        assert_eq!(
                            result.pixels[y * result.width + x].map(f32::to_bits),
                            wanted.map(f32::to_bits),
                            "{label} ({x},{y}), area={area:?}, dense={dense}"
                        );
                    }
                }
            };
            let down = |x: usize, y: usize| {
                let (cx, cy) = ((2 * x) as i32, (2 * y) as i32);
                let mut sum = [0.0_f64; CHANNELS];
                for (dx, dy, weight) in DOWN_TAPS {
                    for (acc, value) in sum.iter_mut().zip(source.at(cx + dx, cy + dy)) {
                        *acc += weight * f64::from(value);
                    }
                }
                sum.map(|channel| quantize_half(channel / 80.0))
            };
            let tiles = downsample_tiles(&source, Area::full(width, height));
            assert_eq!(
                tiles.is_some_and(|tiles| tiles.active.iter().any(|active| !active)),
                !dense
            );
            for area in areas(width.div_ceil(2), height.div_ceil(2)) {
                check_level(&downsample_13(&source, area), area, &down, "downsample");
            }
            for spread in [0.125, 0.999, 2.625] {
                let upsampled = [2 * width - 1, 2 * height];
                for (size, axes) in [
                    ([width, height], TentAxes::blur([width, height], spread)),
                    (
                        upsampled,
                        TentAxes::upsample(upsampled, [width, height], spread),
                    ),
                ] {
                    let tent = |x: usize, y: usize| {
                        source
                            .tent(&axes.columns[x], &axes.lines[y])
                            .map(|channel| quantize_half(channel / 16.0))
                    };
                    if !dense && spread < 1.0 {
                        let tiles = tent_tiles(
                            &source,
                            Area::full(width, height),
                            &axes.columns,
                            &axes.lines,
                        )
                        .unwrap();
                        assert!(tiles.active.iter().any(|active| !active));
                    }
                    for area in areas(size[0], size[1]) {
                        check_level(&tent_9(&source, size, &axes, area), area, &tent, "tent");
                    }
                }
            }
        }
        check::<1>(false);
        check::<1>(true);
        check::<3>(false);
        check::<3>(true);
    }

    #[test]
    fn half_quantization_fast_path_preserves_reference_rounding() {
        let check = |value: f64| {
            assert_eq!(
                quantize_half(value).to_bits(),
                half_to_f32(f32_to_half(value as f32)).to_bits(),
                "input {value:?} ({:016x})",
                value.to_bits(),
            );
            let lanes = [value, -value, 0.25, value * 0.5];
            assert_eq!(
                quantize_halves(lanes).map(f32::to_bits),
                lanes.map(|lane| quantize_half(lane).to_bits()),
                "lanes {lanes:?}"
            );
        };
        // Check every finite half value and both sides of every rounding midpoint,
        // including ties with even/odd mantissas and exponent/subnormal boundaries.
        for bits in 0..0x7bff_u16 {
            let lower = half_to_f32(bits);
            let upper = half_to_f32(bits + 1);
            let middle = (f64::from(lower) + f64::from(upper)) * 0.5;
            for sign in [-1.0, 1.0] {
                check(sign * f64::from(lower));
                check(sign * middle);
                let middle_bits = (middle as f32).to_bits();
                check(sign * f64::from(f32::from_bits(middle_bits - 1)));
                check(sign * f64::from(f32::from_bits(middle_bits + 1)));
            }
        }
        for value in [
            0.0,
            -0.0,
            65504.0,
            65519.0,
            65520.0,
            65536.0,
            f64::INFINITY,
            f64::NAN,
        ] {
            check(value);
            check(-value);
        }
        let mut bits = 0_u32;
        for _ in 0..1_000_000 {
            bits = bits.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            check(f64::from(f32::from_bits(bits)));
        }
    }

    #[test]
    fn glow_and_bloom_preserve_reference_f16_bytes() {
        let mut pixels = Vec::new();
        for y in 0..13 {
            for x in 0..19 {
                let alpha = ((x * 17 + y * 11) % 97) as f32 / 96.0;
                let red = ((x * 13 + y * 7) % 29) as f32 / 28.0;
                let blue = ((x * 5 + y * 3) % 11) as f32 / 10.0 - 0.25;
                pixels.push([
                    alpha * red * 1.5,
                    alpha * (0.1 + red * 0.5),
                    alpha * blue,
                    alpha,
                ]);
            }
        }
        let input = encode_output(&pixels);
        let hash = |output: Vec<u8>| {
            Sha256::digest(output)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        // Captured from the centred three-channel kernel (24-texel downsample, interpolated blur taps).
        // Odd dimensions, fractional radii, HDR/negative colors and a clipped
        // input pin the tap order, transparent borders and F16 quantization.
        for (radius, glow_hash, bloom_hash) in [
            (
                0.0,
                "d979a904a910241c1531f7670a418e788f2cc56d81304dc9555a428781a6467b",
                "d979a904a910241c1531f7670a418e788f2cc56d81304dc9555a428781a6467b",
            ),
            (
                0.5,
                "5617093bcd560673340c109a9d9ce082513f61e0ebbe4ad42cf08ebc33bc3df3",
                "0fb6ef179176a827061baad752140b0f86901cdf3f787598083501ce9e1ae5de",
            ),
            (
                2.5,
                "5617093bcd560673340c109a9d9ce082513f61e0ebbe4ad42cf08ebc33bc3df3",
                "0fb6ef179176a827061baad752140b0f86901cdf3f787598083501ce9e1ae5de",
            ),
            (
                8.25,
                "66dfde0d7ce9eebb52f2a02d50e7708fd8ebf07f2c2dd1b24871f6b079cf872d",
                "a5077efde5a4ddb90cee57e19875bf0977c874d573c78cfc0b10a029925a83ee",
            ),
            (
                23.5,
                "7222c5beceb4ffe7c4c1eb605657eee0686816a169c24f8192938546772f18de",
                "c6aaeab3256e15e447fccfc064e80dfa307ef3254bf6ae573eccc4b8944fa98d",
            ),
            (
                48.0,
                "096f6503e15f581900fec497e2229f128e25e1ba5ebf3215e69232cc71d5d937",
                "df06bd2d3baa11a77c53dfb72aefc0265346b35cc9e1649adedb9df703bef084",
            ),
        ] {
            let glow = apply_glow_f16(
                &input,
                19,
                13,
                83,
                61,
                [-7, 11],
                GlowParams {
                    color: [0.05, 0.4, 0.7, 0.8],
                    intensity: 0.75,
                    radius,
                },
            )
            .unwrap();
            let bloom = apply_bloom_f16(
                &input,
                19,
                13,
                83,
                61,
                [-7, 11],
                BloomParams {
                    threshold: 0.25,
                    knee: 0.15,
                    intensity: 0.75,
                    radius,
                },
            )
            .unwrap();
            assert_eq!(hash(glow), glow_hash, "glow radius {radius}");
            assert_eq!(hash(bloom), bloom_hash, "bloom radius {radius}");
        }
    }

    #[test]
    fn offscreen_glow_and_bloom_equal_cropped_larger_outputs() {
        let input = encode_output(&vec![[1.0; 4]; 10 * 24]);
        for glow in [true, false] {
            let render = |width, offset| {
                if glow {
                    apply_glow_f16(
                        &input,
                        10,
                        24,
                        width,
                        96,
                        offset,
                        GlowParams {
                            color: [1.0; 4],
                            intensity: 1.0,
                            radius: 12.0,
                        },
                    )
                } else {
                    apply_bloom_f16(
                        &input,
                        10,
                        24,
                        width,
                        96,
                        offset,
                        BloomParams {
                            threshold: 0.8,
                            knee: 0.1,
                            intensity: 1.0,
                            radius: 12.0,
                        },
                    )
                }
                .unwrap()
            };
            let clipped = render(128, [-12, 36]);
            let reference = render(192, [52, 36]);
            assert!(
                clipped.iter().any(|byte| *byte != 0),
                "offscreen halo must enter the viewport"
            );
            for y in 0..96 {
                assert_eq!(
                    &clipped[y * 128 * 8..(y + 1) * 128 * 8],
                    &reference[(y * 192 + 64) * 8..(y + 1) * 192 * 8]
                );
            }
        }
    }

    #[test]
    fn distant_input_clipping_preserves_visible_f16_samples() {
        let input = encode_output(
            &(0..384 * 288)
                .map(|index| {
                    let alpha = (index % 17) as f32 / 16.0;
                    [alpha * 1.5, alpha * 0.25, alpha * -0.125, alpha]
                })
                .collect::<Vec<_>>(),
        );
        // The larger reference includes the entire input. Crop at a multiple of
        // every pyramid scale, including odd viewport dimensions and wide halos.
        for radius in [4.0, 12.0, 48.0, 128.0] {
            for glow in [true, false] {
                let render = |size: [u32; 2], offset| {
                    if glow {
                        apply_glow_f16(
                            &input,
                            384,
                            288,
                            size[0],
                            size[1],
                            offset,
                            GlowParams {
                                color: [0.05, 0.4, 0.7, 0.8],
                                intensity: 0.75,
                                radius,
                            },
                        )
                    } else {
                        apply_bloom_f16(
                            &input,
                            384,
                            288,
                            size[0],
                            size[1],
                            offset,
                            BloomParams {
                                threshold: 0.25,
                                knee: 0.15,
                                intensity: 0.75,
                                radius,
                            },
                        )
                    }
                    .unwrap()
                };
                let clipped = render([101, 77], [-228, -100]);
                let reference = render([640, 512], [28, 92]);
                for y in 0..77 {
                    assert_eq!(
                        &clipped[y * 101 * 8..(y + 1) * 101 * 8],
                        &reference[((y + 192) * 640 + 256) * 8..((y + 192) * 640 + 357) * 8],
                        "glow={glow}, radius={radius}, row={y}",
                    );
                }
            }
        }
        let clipped = PyramidRegion::new([384, 288], [101, 77], [-228, -100], 4.0, 4.0).unwrap();
        assert!(clipped.size[0] < 384 && clipped.size[1] < 288);
        for offset in [[-1000, 1000], [1000, -1000]] {
            let empty = apply_glow_f16(
                &input,
                384,
                288,
                101,
                77,
                offset,
                GlowParams {
                    color: [1.0; 4],
                    intensity: 1.0,
                    radius: 12.0,
                },
            )
            .unwrap();
            assert!(empty.iter().all(|byte| *byte == 0));
        }
    }

    #[test]
    fn oversized_source_metadata_does_not_enlarge_the_pyramid_allocation() {
        let region =
            PyramidRegion::new([7645, 5891], [3840, 2160], [-2430, -1579], 8.0, 16.0).unwrap();
        assert!(region.size[0] < 4000 && region.size[1] < 2400);
        assert!(PyramidGraph::new(region.size[0], region.size[1], 8.0, 16.0).is_ok());
        assert!(PyramidRegion::new([0, 1], [3840, 2160], [0, 0], 8.0, 16.0).is_err());
        assert!(PyramidRegion::new([7645, 5891], [8192, 8192], [0, 0], 8.0, 16.0).is_err());
        assert!(PyramidGraph::new(8192, 8192, 8.0, 16.0).is_err());
        // Metadata admission must not admit a truncated F16 payload.
        assert!(read_base(&[0; 8], 7645, 5891, 1, 1, [0, 0]).is_err());
    }

    #[test]
    fn aligned_input_crops_preserve_visible_f16_bytes() {
        let (width, height) = (640_u32, 512_u32);
        let input = encode_output(
            &(0..width * height)
                .map(|index| {
                    let alpha = (index % 17) as f32 / 16.0;
                    [alpha * 1.5, alpha * 0.25, alpha * -0.125, alpha]
                })
                .collect::<Vec<_>>(),
        );
        for radius in [0.0, 4.0, 8.0, 12.0, 48.0, 128.0] {
            for glow in [true, false] {
                let render = |bytes: &[u8], size: [u32; 2], offset| {
                    if glow {
                        apply_glow_f16(
                            bytes,
                            size[0],
                            size[1],
                            101,
                            77,
                            offset,
                            GlowParams {
                                color: [0.05, 0.4, 0.7, 0.8],
                                intensity: 0.75,
                                radius,
                            },
                        )
                    } else {
                        apply_bloom_f16(
                            bytes,
                            size[0],
                            size[1],
                            101,
                            77,
                            offset,
                            BloomParams {
                                threshold: 0.25,
                                knee: 0.15,
                                intensity: 0.75,
                                radius,
                            },
                        )
                    }
                    .unwrap()
                };
                let offset = [-228, -196];
                let region = PyramidRegion::new(
                    [width, height],
                    [101, 77],
                    offset,
                    radius,
                    radius * if glow { 2.0 } else { 1.0 },
                )
                .unwrap();
                let left = (region.origin[0] - offset[0]).max(0) as usize;
                let top = (region.origin[1] - offset[1]).max(0) as usize;
                let right = (i64::from(region.origin[0]) + i64::from(region.size[0])
                    - i64::from(offset[0]))
                .min(i64::from(width)) as usize;
                let bottom = (i64::from(region.origin[1]) + i64::from(region.size[1])
                    - i64::from(offset[1]))
                .min(i64::from(height)) as usize;
                assert_eq!((left % 32, top % 32), (0, 0));
                let cropped = (top..bottom)
                    .flat_map(|y| {
                        input[(y * width as usize + left) * 8..(y * width as usize + right) * 8]
                            .iter()
                            .copied()
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    render(
                        &cropped,
                        [(right - left) as u32, (bottom - top) as u32],
                        [offset[0] + left as i32, offset[1] + top as i32]
                    ),
                    render(&input, [width, height], offset),
                    "glow={glow}, radius={radius}",
                );
            }
        }
    }

    #[test]
    fn source_bounded_regions_keep_the_output_bytes() {
        // The previous region always spanned the whole output plus the support margin.
        let output_region =
            |input: [u32; 2], output: [u32; 2], offset: [i32; 2], radius, spread| {
                let margin = i64::from(PyramidRegion::support_margin(radius, spread).unwrap());
                let mut region = PyramidRegion {
                    origin: [0; 2],
                    size: [0; 2],
                    input_offset: [0; 2],
                };
                for axis in 0..2 {
                    let source = i64::from(offset[axis]);
                    let visible_end = i64::from(output[axis]);
                    let relevant_start = source.max(-margin).min(0);
                    let relevant_end = (source + i64::from(input[axis]))
                        .min(visible_end + margin)
                        .max(visible_end);
                    let start = source + (relevant_start - margin - source).div_euclid(32) * 32;
                    region.origin[axis] = start as i32;
                    region.size[axis] = (((relevant_end + margin - start + 31) / 32) * 32) as u32;
                    region.input_offset[axis] = (source - start) as i32;
                }
                region
            };
        let (width, height) = (61_u32, 47_u32);
        let input = encode_output(
            &(0..width * height)
                .map(|index| {
                    let (x, y) = (index % width, index / width);
                    let alpha = if (x + 2 * y) % 7 == 0 {
                        0.0
                    } else {
                        ((x * y) % 13) as f32 / 12.0
                    };
                    [alpha * 1.25, alpha * 0.5, alpha * 0.75, alpha]
                })
                .collect::<Vec<_>>(),
        );
        let output = [300, 220];
        let mut shrunk = 0;
        for offset in [[120, 90], [-20, 150], [250, -30], [0, 0], [400, 90]] {
            for radius in [4.0, 12.0, 48.0] {
                for glow in [true, false] {
                    let spread = radius * if glow { 2.0 } else { 1.0 };
                    let before = output_region([width, height], output, offset, radius, spread);
                    let after = PyramidRegion::new([width, height], output, offset, radius, spread)
                        .unwrap();
                    let area = |region: PyramidRegion| region.size[0] * region.size[1];
                    assert!(
                        area(after) <= area(before),
                        "offset={offset:?}, radius={radius}"
                    );
                    shrunk += usize::from(area(after) < area(before));
                    let reference = if glow {
                        let params = GlowParams {
                            color: [0.9, 0.3, 0.6, 0.85],
                            intensity: 1.2,
                            radius,
                        };
                        let pixels = apply_glow_region_f16(
                            &input,
                            width,
                            height,
                            before.size[0],
                            before.size[1],
                            before.input_offset,
                            Area::full(before.size[0] as usize, before.size[1] as usize),
                            params,
                        )
                        .unwrap();
                        (
                            encode_region(&pixels, before, output),
                            apply_glow_f16(
                                &input, width, height, output[0], output[1], offset, params,
                            )
                            .unwrap(),
                        )
                    } else {
                        let params = BloomParams {
                            threshold: 0.3,
                            knee: 0.2,
                            intensity: 0.9,
                            radius,
                        };
                        let pixels = apply_bloom_region_f16(
                            &input,
                            width,
                            height,
                            before.size[0],
                            before.size[1],
                            before.input_offset,
                            Area::full(before.size[0] as usize, before.size[1] as usize),
                            params,
                        )
                        .unwrap();
                        (
                            encode_region(&pixels, before, output),
                            apply_bloom_f16(
                                &input, width, height, output[0], output[1], offset, params,
                            )
                            .unwrap(),
                        )
                    };
                    assert!(
                        reference.0 == reference.1,
                        "glow={glow}, offset={offset:?}, radius={radius}"
                    );
                }
            }
        }
        assert!(shrunk >= 12, "only {shrunk} regions shrank");
    }

    #[test]
    fn pyramid_graph_keeps_inputs_live_and_sizes_odd_levels() {
        let graph = PyramidGraph::new(7, 5, 48.0, 48.0).unwrap();
        let levels = graph
            .passes()
            .iter()
            .filter(|pass| matches!(pass.kind(), PyramidPassKind::Downsample { .. }))
            .map(|pass| (pass.width(), pass.height()))
            .collect::<Vec<_>>();
        assert_eq!(levels, [(4, 3), (2, 2), (1, 1), (1, 1), (1, 1)]);

        let mut live = vec![false; graph.resource_count()];
        live[0] = true;
        for pass in graph.passes() {
            for input in pass.kind().reads() {
                assert!(live[input], "pass reads retired pyramid resource {input}");
            }
            assert!(!live[pass.output()]);
            live[pass.output()] = true;
            for &resource in pass.retire_after() {
                assert!(live[resource], "pyramid resource retired twice");
                assert_ne!(resource, graph.output());
                live[resource] = false;
            }
        }
        assert_eq!(live.iter().filter(|value| **value).count(), 1);
        assert!(live[graph.output()]);
    }

    #[test]
    fn half_roundtrip_preserves_every_finite_encoding() {
        for bits in 0_u16..=u16::MAX {
            if bits & 0x7c00 == 0x7c00 {
                continue;
            }
            assert_eq!(f32_to_half(half_to_f32(bits)), bits, "{bits:#06x}");
        }
    }

    #[test]
    fn half_pixels_convert_like_each_channel() {
        // Every half in every channel decodes to its value; an infinite or NaN channel rejects
        // the pixel.
        for bits in 0_u16..=u16::MAX {
            let value = half_to_f32(bits);
            for channel in 0..4 {
                let mut halves = [0x3c00_u16; 4];
                halves[channel] = bits;
                let bytes = std::array::from_fn(|index| halves[index / 2].to_le_bytes()[index % 2]);
                let mut expected = [1.0_f32; 4];
                expected[channel] = value;
                assert_eq!(
                    decode_pixel(&bytes).map(|pixel| pixel.map(f32::to_bits)),
                    value.is_finite().then_some(expected.map(f32::to_bits)),
                    "{bits:#06x} in channel {channel}"
                );
            }
        }
        // Encoding matches `f32_to_half` for each channel, NaNs of every payload included.
        let mut state = 0x1234_5678_u32;
        let mut pixels = (0..1 << 16)
            .map(|_| {
                std::array::from_fn(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    f32::from_bits(state)
                })
            })
            .collect::<Vec<[f32; 4]>>();
        pixels.extend([
            [f32::NAN, -f32::NAN, f32::from_bits(0x7f80_0001), 1.0],
            [f32::INFINITY, f32::NEG_INFINITY, 65520.0, -0.0],
            [
                65519.0,
                2.0_f32.powi(-25),
                -(2.0_f32.powi(-24) * 1.5),
                1e-40,
            ],
        ]);
        let mut bytes = vec![0; pixels.len() * 8];
        encode_pixels(&pixels, &mut bytes);
        for (pixel, encoded) in pixels.iter().zip(bytes.as_chunks::<8>().0) {
            let expected: [u8; 8] =
                std::array::from_fn(|index| f32_to_half(pixel[index / 2]).to_le_bytes()[index % 2]);
            assert_eq!(*encoded, expected, "{:08x?}", pixel.map(f32::to_bits));
        }
    }

    /// Run after changing the conversions, libm or the target.
    #[test]
    #[ignore = "exhaustive F32 domain; run with --release --ignored"]
    fn every_f32_quantizes_and_encodes_like_the_reference() {
        std::thread::scope(|scope| {
            for block in 0..64_u32 {
                scope.spawn(move || {
                    let mut bytes = [0_u8; 8];
                    for bits in block << 26..=(block << 26 | 0x03ff_ffff) {
                        let value = f32::from_bits(bits);
                        let half = f32_to_half(value);
                        let rounded = half_to_f32(half).to_bits();
                        assert_eq!(
                            quantize_half(f64::from(value)).to_bits(),
                            rounded,
                            "{bits:#010x}"
                        );
                        assert_eq!(
                            quantize_halves([f64::from(value), 1.5, -0.0, f64::from(value)])
                                .map(f32::to_bits),
                            [rounded, 1.5_f32.to_bits(), (-0.0_f32).to_bits(), rounded],
                            "{bits:#010x}"
                        );
                        encode_pixels(&[[value; 4]], &mut bytes);
                        let expected: [u8; 8] =
                            std::array::from_fn(|index| half.to_le_bytes()[index % 2]);
                        assert_eq!(bytes, expected, "{bits:#010x}");
                    }
                });
            }
        });
    }

    #[test]
    fn glow_of_a_grid_aligned_square_stays_centred() {
        // A 32px input covers whole texels on every level, so the halo must be symmetric.
        let (size, out) = (32_u32, 160_u32);
        let input = encode_output(&vec![[1.0; 4]; (size * size) as usize]);
        for radius in [12.0, 24.0, 48.0] {
            let pixels = apply_glow_f16(
                &input,
                size,
                size,
                out,
                out,
                [64, 64],
                GlowParams {
                    color: [0.0, 1.0, 1.0, 1.0],
                    intensity: 1.5,
                    radius,
                },
            )
            .unwrap();
            let (mut sum, mut sx, mut sy) = (0.0, 0.0, 0.0);
            for (index, pixel) in pixels.chunks_exact(8).enumerate() {
                let alpha = f64::from(half_to_f32(u16::from_le_bytes([pixel[6], pixel[7]])));
                sum += alpha;
                sx += alpha * ((index % out as usize) as f64 + 0.5);
                sy += alpha * ((index / out as usize) as f64 + 0.5);
            }
            let centroid = [sx / sum - 80.0, sy / sum - 80.0];
            assert!(
                centroid.iter().all(|offset| offset.abs() < 0.01),
                "radius {radius}: centroid offset {centroid:?}"
            );
        }
    }

    #[test]
    fn glow_tints_the_alpha_halo_and_preserves_the_source() {
        let (width, height) = (128_u32, 64_u32);
        let mut source = vec![0_u8; width as usize * height as usize * 8];
        for y in 20..44_usize {
            for x in 24..48_usize {
                let at = (y * width as usize + x) * 8;
                for channel in 0..4 {
                    source[at + channel * 2..at + channel * 2 + 2]
                        .copy_from_slice(&f32_to_half(1.0).to_le_bytes());
                }
            }
        }
        let params = GlowParams {
            color: [0.02, 0.7, 0.9, 1.0],
            intensity: 1.5,
            radius: 24.0,
        };
        let render =
            |params| apply_glow_f16(&source, width, height, width, height, [0, 0], params).unwrap();
        let full = render(params);
        let half = render(GlowParams {
            intensity: 0.75,
            ..params
        });
        let sample = |pixels: &[u8], x: usize, channel: usize| {
            let at = (32 * width as usize + x) * 8 + channel * 2;
            half_to_f32(u16::from_le_bytes([pixels[at], pixels[at + 1]]))
        };
        assert_eq!(sample(&full, 36, 0), 1.0);
        assert!(sample(&full, 60, 1) > sample(&full, 60, 0));
        assert!(sample(&full, 60, 1) > sample(&half, 60, 1));
        assert!(
            sample(&full, 120, 1) < 0.001,
            "far halo: {}",
            sample(&full, 120, 1)
        );
        assert_eq!(
            render(GlowParams {
                intensity: 0.0,
                ..params
            }),
            source
        );
    }

    #[test]
    fn intensity_and_radius_change_the_halo_deterministically() {
        let (width, height) = (128_u32, 64_u32);
        let mut source = vec![0_u8; width as usize * height as usize * 8];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let value = if (32..40).contains(&x) && (28..36).contains(&y) {
                    1.0
                } else {
                    0.0
                };
                let at = (y * width as usize + x) * 8;
                for channel in 0..4 {
                    source[at + channel * 2..at + channel * 2 + 2].copy_from_slice(
                        &f32_to_half(if channel == 3 { 1.0 } else { value }).to_le_bytes(),
                    );
                }
            }
        }
        let params = BloomParams {
            threshold: 0.8,
            knee: 0.1,
            intensity: 1.0,
            radius: 48.0,
        };
        let render = |params| {
            apply_bloom_f16(&source, width, height, width, height, [0, 0], params).unwrap()
        };
        let zero = render(BloomParams {
            intensity: 0.0,
            ..params
        });
        assert_eq!(zero, source);
        let one = render(params);
        assert_eq!(one, render(params));
        let half = render(BloomParams {
            intensity: 0.5,
            ..params
        });
        let wide = render(BloomParams {
            radius: 96.0,
            ..params
        });
        let sample = |pixels: &[u8], x: usize| {
            let at = (32 * width as usize + x) * 8;
            half_to_f32(u16::from_le_bytes([pixels[at], pixels[at + 1]]))
        };
        assert!(sample(&one, 60) > sample(&half, 60));
        // Equal-energy halos cross near the source; beyond that the wider one stays brighter.
        assert!(sample(&wide, 108) > sample(&one, 108));
        let energy = |pixels: &[u8]| -> f64 {
            pixels
                .chunks_exact(8)
                .zip(source.chunks_exact(8))
                .map(|(pixel, original)| {
                    f64::from(half_to_f32(u16::from_le_bytes([pixel[0], pixel[1]])))
                        - f64::from(half_to_f32(u16::from_le_bytes([original[0], original[1]])))
                })
                .sum()
        };
        let full_energy = energy(&one);
        let half_energy = energy(&half);
        assert!(full_energy > 0.0);
        assert!((full_energy - 2.0 * half_energy).abs() / full_energy < 0.01);
        assert_eq!(
            apply_bloom_f16(
                &source,
                width,
                height,
                width,
                height,
                [0, 0],
                BloomParams {
                    threshold: 1.1,
                    ..params
                }
            )
            .unwrap_err()
            .to_string(),
            BloomError::InvalidParameters.to_string()
        );
    }

    #[test]
    fn white_blooms_and_dim_gray_does_not() {
        let (width, height) = (640_u32, 360_u32);
        let mut bytes = vec![0; width as usize * height as usize * 8];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let value = if (120..180).contains(&x) && (150..210).contains(&y) {
                    1.0
                } else if (460..520).contains(&x) && (150..210).contains(&y) {
                    0.05
                } else {
                    0.005
                };
                let at = (y * width as usize + x) * 8;
                for channel in 0..4 {
                    let bits = f32_to_half(if channel == 3 { 1.0 } else { value }).to_le_bytes();
                    bytes[at + channel * 2..at + channel * 2 + 2].copy_from_slice(&bits);
                }
            }
        }
        let output = apply_bloom_f16(
            &bytes,
            width,
            height,
            width,
            height,
            [0, 0],
            BloomParams {
                threshold: 0.8,
                knee: 0.1,
                intensity: 1.2,
                radius: 48.0,
            },
        )
        .unwrap();
        let luminance_at = |x: usize| {
            let at = (180 * width as usize + x) * 8;
            half_to_f32(u16::from_le_bytes([output[at], output[at + 1]]))
        };
        assert!(luminance_at(210) > 0.014, "hot halo: {}", luminance_at(210));
        assert!(luminance_at(550) < 0.008, "dim halo: {}", luminance_at(550));
        assert_eq!(output.len(), bytes.len());
    }
}
