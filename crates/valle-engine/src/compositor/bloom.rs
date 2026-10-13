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
const DOWN_TAPS: [(i32, i32, f64); 13] = [
    (0, 0, 4.0),
    (-1, 0, 2.0),
    (1, 0, 2.0),
    (0, -1, 2.0),
    (0, 1, 2.0),
    (-1, -1, 1.0),
    (1, -1, 1.0),
    (-1, 1, 1.0),
    (1, 1, 1.0),
    (-2, 0, 1.0),
    (2, 0, 1.0),
    (0, -2, 1.0),
    (0, 2, 1.0),
];
const TENT: [(i32, i32, f64); 9] = [
    (-1, -1, 1.0),
    (0, -1, 2.0),
    (1, -1, 1.0),
    (-1, 0, 2.0),
    (0, 0, 4.0),
    (1, 0, 2.0),
    (-1, 1, 1.0),
    (0, 1, 2.0),
    (1, 1, 1.0),
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

    fn sample(&self, x: Option<usize>, y: Option<usize>) -> [f32; CHANNELS] {
        match (x, y) {
            (Some(x), Some(y)) => self.pixels[y * self.width + x],
            _ => [0.0; CHANNELS],
        }
    }

    fn bilinear(&self, x: LinearAxis, y: LinearAxis) -> [f64; CHANNELS] {
        let weights = [
            (x.indices[0], y.indices[0], x.weights[0] * y.weights[0]),
            (x.indices[1], y.indices[0], x.weights[1] * y.weights[0]),
            (x.indices[0], y.indices[1], x.weights[0] * y.weights[1]),
            (x.indices[1], y.indices[1], x.weights[1] * y.weights[1]),
        ];
        let mut value = [0.0; CHANNELS];
        for (sx, sy, weight) in weights {
            for (dst, src) in value.iter_mut().zip(self.sample(sx, sy)) {
                *dst += f64::from(src) * weight;
            }
        }
        value
    }
}

#[derive(Clone, Copy)]
struct LinearAxis {
    indices: [Option<usize>; 2],
    weights: [f64; 2],
}

impl LinearAxis {
    fn new(position: f64, extent: usize) -> Self {
        let lower = position.floor() as i32;
        let fraction = position - f64::from(lower);
        Self {
            indices: [sample_index(lower, extent), sample_index(lower + 1, extent)],
            weights: [1.0 - fraction, fraction],
        }
    }
}

fn sample_index(position: i32, extent: usize) -> Option<usize> {
    (position >= 0 && position < extent as i32).then_some(position as usize)
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

fn fill_rows<const CHANNELS: usize>(
    next: &mut Level<CHANNELS>,
    fill: impl Fn(usize, &mut [[f32; CHANNELS]]) + Sync,
) {
    #[cfg(not(target_family = "wasm"))]
    {
        let width = next.width;
        if valle_motion::cpu::is_budgeted() {
            let requested = if next.height >= 128 && next.pixels.len() >= 65_536 {
                8
            } else {
                1
            };
            valle_motion::cpu::parallel_scope(requested, |scope, count| {
                let rows_per_job = next.height.div_ceil(count).max(16);
                let mut chunks = next.pixels.chunks_mut(rows_per_job * width).enumerate();
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
        if next.height >= 128
            && next.pixels.len() >= 65_536
            && let Some(pool) = &*PYRAMID_WORKERS
        {
            let workers = pool.current_num_threads();
            let rows_per_worker = next.height.div_ceil(workers).max(16);
            pool.scope(|scope| {
                for (index, chunk) in next.pixels.chunks_mut(rows_per_worker * width).enumerate() {
                    let fill = &fill;
                    scope.spawn(move |_| fill(index * rows_per_worker, chunk));
                }
            });
            return;
        }
    }
    fill(0, &mut next.pixels);
}

/// Working region includes the offscreen input that can affect the visible output.
/// Align every level to the input's origin and a 32-pixel grid so cropping cannot
/// shift the pyramid taps. The complete support margin also covers every fine level.
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
                + spread.round() * scale
                + (0.5 * spread + 1.0) * (2.0 * scale - 2.0))
                .ceil()
        };
        if margin > f64::from(i32::MAX) {
            return Err(BloomError::InvalidImage);
        }
        Ok(margin as u32)
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
            let visible_end = i64::from(output[axis]);
            // Contributions farther than the complete pyramid support cannot
            // reach the output. Keep a second zero-filled support margin around
            // their intersection, including when the source is wholly offscreen.
            let relevant_start = source.max(-margin).min(0);
            let relevant_end = (source + i64::from(input[axis]))
                .min(visible_end + margin)
                .max(visible_end);
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
        params,
    )?;
    Ok(encode_region(
        &pixels,
        region,
        [output_width, output_height],
    ))
}

/// Source ROI is placed at `offset` inside the output ROI. All dimensions are device pixels.
fn apply_bloom_region_f16(
    input: &[u8],
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    offset: [i32; 2],
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

    let mut bright = Level::<3>::empty(output_width as usize, output_height as usize);
    for (dst, pixel) in bright.pixels.iter_mut().zip(&base) {
        let alpha = f64::from(pixel[3]);
        if alpha <= 0.0 {
            continue;
        }
        let luminance = (0.2627 * f64::from(pixel[0])
            + 0.6780 * f64::from(pixel[1])
            + 0.0593 * f64::from(pixel[2]))
            / alpha;
        let threshold = f64::from(params.threshold);
        let knee = f64::from(params.knee);
        let weight = if knee == 0.0 {
            f64::from(luminance >= threshold)
        } else {
            let t = ((luminance - threshold + knee) / (2.0 * knee)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        for (channel, source) in dst.iter_mut().zip(pixel) {
            *channel = quantize_half(f64::from(*source) * weight);
        }
    }

    let bloom = pyramid_halo(bright, params.radius, params.radius)?;
    for (source, halo) in base.iter_mut().zip(bloom.pixels) {
        for channel in 0..3 {
            source[channel] = quantize_half(
                (f64::from(source[channel])
                    + f64::from(params.intensity) * f64::from(halo[channel]))
                .clamp(-65504.0, 65504.0),
            );
        }
        let halo_alpha = halo.into_iter().fold(0.0_f32, f32::max).min(1.0);
        source[3] = source[3].max(halo_alpha);
    }
    Ok(base)
}

/// Alpha-contour glow uses the same reduced-resolution pyramid, then tints the
/// halo and places the original subtree over it.
fn apply_glow_region_f16(
    input: &[u8],
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    offset: [i32; 2],
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
    let mut coverage = Level::<1>::empty(output_width as usize, output_height as usize);
    for (target, source) in coverage.pixels.iter_mut().zip(&base) {
        *target = [source[3]];
    }
    // Keep the level count tied to the authored radius while widening each tent
    // tap to approximate a Gaussian sigma without adding a global coarse tail.
    let halo = pyramid_halo(coverage, params.radius, params.radius * 2.0)?;
    let peak_color = params.color[..3].iter().copied().fold(0.0_f32, f32::max);
    for (source, sample) in base.iter_mut().zip(halo.pixels) {
        let gain = f64::from(sample[0]) * f64::from(params.intensity) * GLOW_RESPONSE_GAIN;
        let color_gain = if peak_color > 0.0 {
            gain.min(1.0 / f64::from(peak_color))
        } else {
            gain
        };
        let source_alpha = f64::from(source[3]);
        let halo_alpha = (gain * f64::from(params.color[3])).min(1.0);
        for (channel, color) in source[..3].iter_mut().zip(params.color) {
            *channel = quantize_half(
                (f64::from(*channel) + color_gain * f64::from(color) * (1.0 - source_alpha))
                    .clamp(-65504.0, 65504.0),
            );
        }
        source[3] = quantize_half(source_alpha + halo_alpha * (1.0 - source_alpha));
    }
    Ok(base)
}

/// PyramidRegion always contains the output. Encode that window directly, avoiding a padded
/// F16 allocation followed by decoding into another full-size f32 image just to crop it.
fn encode_region(pixels: &[[f32; 4]], region: PyramidRegion, output: [u32; 2]) -> Vec<u8> {
    let left = (-i64::from(region.origin[0])) as usize;
    let top = (-i64::from(region.origin[1])) as usize;
    let (width, height) = (output[0] as usize, output[1] as usize);
    debug_assert!(left + width <= region.size[0] as usize);
    debug_assert!(top + height <= region.size[1] as usize);
    let mut bytes = Vec::with_capacity(width * height * 8);
    for row in pixels
        .chunks_exact(region.size[0] as usize)
        .skip(top)
        .take(height)
    {
        for pixel in &row[left..left + width] {
            for &channel in pixel {
                bytes.extend_from_slice(&f32_to_half(channel).to_le_bytes());
            }
        }
    }
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
    let ow = output_width as usize;
    let oh = output_height as usize;
    let mut base = vec![[0.0_f32; 4]; output_count];
    for (index, pixel) in input.chunks_exact(8).enumerate() {
        let x = index % iw;
        let y = index / iw;
        let dx = x as i64 + i64::from(offset[0]);
        let dy = y as i64 + i64::from(offset[1]);
        let mut channels = [0.0; 4];
        for (channel, bytes) in channels.iter_mut().zip(pixel.chunks_exact(2)) {
            *channel = half_to_f32(u16::from_le_bytes([bytes[0], bytes[1]]));
            if !channel.is_finite() {
                return Err(BloomError::InvalidImage);
            }
        }
        if dx >= 0 && dx < ow as i64 && dy >= 0 && dy < oh as i64 {
            base[dy as usize * ow + dx as usize] = channels;
        }
    }
    Ok(base)
}

fn pyramid_halo<const CHANNELS: usize>(
    bright: Level<CHANNELS>,
    depth_radius: f32,
    spread_radius: f32,
) -> Result<Level<CHANNELS>, BloomError> {
    let graph = PyramidGraph::new(
        u32::try_from(bright.width).map_err(|_| BloomError::InvalidImage)?,
        u32::try_from(bright.height).map_err(|_| BloomError::InvalidImage)?,
        depth_radius,
        spread_radius,
    )?;
    let mut images = vec![None; graph.resource_count()];
    images[0] = Some(bright);
    for pass in graph.passes() {
        let source = |id: usize| images[id].as_ref().expect("pyramid input is live");
        let image = match pass.kind() {
            PyramidPassKind::Downsample { input } => downsample_13(source(input)),
            PyramidPassKind::Blur { input } => blur_9(source(input), graph.spread()),
            PyramidPassKind::Upsample {
                coarse,
                fine,
                local_weight,
            } => {
                let mut up = upsample_9(
                    source(coarse),
                    pass.width() as usize,
                    pass.height() as usize,
                    graph.spread(),
                );
                if let Some(fine) = fine {
                    let local = source(fine);
                    for (far, local) in up.pixels.iter_mut().zip(&local.pixels) {
                        for channel in 0..CHANNELS {
                            far[channel] = quantize_half(
                                local_weight * f64::from(local[channel])
                                    + (1.0 - local_weight) * f64::from(far[channel]),
                            );
                        }
                    }
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

fn downsample_13<const CHANNELS: usize>(source: &Level<CHANNELS>) -> Level<CHANNELS> {
    let mut next = Level::empty(source.width.div_ceil(2), source.height.div_ceil(2));
    let width = next.width;
    fill_rows(&mut next, |start_y, rows| {
        for (row_index, row) in rows.chunks_mut(width).enumerate() {
            let y = start_y + row_index;
            for (x, out) in row.iter_mut().enumerate() {
                let mut sum = [0.0_f64; CHANNELS];
                let cx = (x * 2 + 1) as i32;
                let cy = (y * 2 + 1) as i32;
                for (dx, dy, weight) in DOWN_TAPS {
                    for (acc, value) in sum.iter_mut().zip(source.at(cx + dx, cy + dy)) {
                        *acc += weight * f64::from(value);
                    }
                }
                *out = sum.map(|channel| quantize_half(channel / 20.0));
            }
        }
    });
    next
}

fn blur_9<const CHANNELS: usize>(source: &Level<CHANNELS>, spread: f64) -> Level<CHANNELS> {
    let mut next = Level::empty(source.width, source.height);
    // Each row/column uses the same taps. Preserve the original round operation,
    // but perform it once per axis instead of nine times per output pixel.
    let axes = |extent| {
        (0..extent)
            .map(|position| {
                [-1, 0, 1].map(|tap| {
                    sample_index(
                        (position as f64 + f64::from(tap) * spread).round() as i32,
                        extent,
                    )
                })
            })
            .collect::<Vec<_>>()
    };
    let columns = axes(source.width);
    let lines = axes(source.height);
    let width = next.width;
    fill_rows(&mut next, |start_y, rows| {
        for (row_index, row) in rows.chunks_mut(width).enumerate() {
            let y = start_y + row_index;
            for (x, out) in row.iter_mut().enumerate() {
                let mut sum = [0.0_f64; CHANNELS];
                for (dx, dy, weight) in TENT {
                    let sx = columns[x][(dx + 1) as usize];
                    let sy = lines[y][(dy + 1) as usize];
                    for (acc, value) in sum.iter_mut().zip(source.sample(sx, sy)) {
                        *acc += weight * f64::from(value);
                    }
                }
                *out = sum.map(|channel| quantize_half(channel / 16.0));
            }
        }
    });
    next
}

const UPSAMPLE_TILE_WIDTH: usize = 8;
const UPSAMPLE_TILE_HEIGHT: usize = 4;

struct UpsampleTiles {
    columns: usize,
    active: Vec<bool>,
}

// The entire support must be zero, including bilinear neighbours and every tent
// tap. Occupied tiles retain the reference summation and F16 rounding order.
fn upsample_tiles<const CHANNELS: usize>(
    source: &Level<CHANNELS>,
    columns: &[[LinearAxis; 3]],
    lines: &[[LinearAxis; 3]],
) -> Option<UpsampleTiles> {
    if source.pixels.len() < 16_384 {
        return None;
    }
    let stride = source.width.div_ceil(64);
    let mut occupied = vec![0_u64; stride * source.height];
    let mut count = 0;
    for (y, row) in source.pixels.chunks(source.width).enumerate() {
        for (x, pixel) in row.iter().enumerate() {
            if pixel.iter().any(|channel| *channel != 0.0) {
                occupied[y * stride + x / 64] |= 1_u64 << (x % 64);
                count += 1;
                // Avoid building output tiles for dense images.
                if count > source.pixels.len() / 2 {
                    return None;
                }
            }
        }
    }
    let span = |axes: &[[LinearAxis; 3]]| {
        axes.iter()
            .flat_map(|taps| taps.iter())
            .flat_map(|axis| axis.indices.into_iter().flatten())
            .fold(None, |range: Option<(usize, usize)>, index| {
                Some(range.map_or((index, index), |(lo, hi)| (lo.min(index), hi.max(index))))
            })
    };
    let xs = columns
        .chunks(UPSAMPLE_TILE_WIDTH)
        .map(span)
        .collect::<Vec<_>>();
    let ys = lines
        .chunks(UPSAMPLE_TILE_HEIGHT)
        .map(span)
        .collect::<Vec<_>>();
    let mut active = vec![false; xs.len() * ys.len()];
    for (tile_y, yrange) in ys.into_iter().enumerate() {
        let Some((top, bottom)) = yrange else {
            continue;
        };
        for (tile_x, xrange) in xs.iter().copied().enumerate() {
            let Some((left, right)) = xrange else {
                continue;
            };
            let first_word = left / 64;
            let last_word = right / 64;
            active[tile_y * xs.len() + tile_x] = (top..=bottom).any(|y| {
                (first_word..=last_word).any(|word| {
                    let start = if word == first_word { left % 64 } else { 0 };
                    let end = if word == last_word { right % 64 } else { 63 };
                    let mask = (u64::MAX << start) & (u64::MAX >> (63 - end));
                    occupied[y * stride + word] & mask != 0
                })
            });
        }
    }
    Some(UpsampleTiles {
        columns: xs.len(),
        active,
    })
}

fn upsample_9<const CHANNELS: usize>(
    source: &Level<CHANNELS>,
    width: usize,
    height: usize,
    spread: f64,
) -> Level<CHANNELS> {
    let mut next = Level::empty(width, height);
    let axes = |extent, source_extent| {
        (0..extent)
            .map(|position| {
                let center = (position as f64 + 0.5) * source_extent as f64 / extent as f64 - 0.5;
                [-1, 0, 1].map(|tap| {
                    LinearAxis::new(center + f64::from(tap) * 0.5 * spread, source_extent)
                })
            })
            .collect::<Vec<_>>()
    };
    let columns = axes(width, source.width);
    let lines = axes(height, source.height);
    let tiles = upsample_tiles(source, &columns, &lines);
    fill_rows(&mut next, |start_y, rows| {
        for (row_index, row) in rows.chunks_mut(width).enumerate() {
            let y = start_y + row_index;
            for (tile_x, tile) in row.chunks_mut(UPSAMPLE_TILE_WIDTH).enumerate() {
                if tiles.as_ref().is_some_and(|tiles| {
                    !tiles.active[(y / UPSAMPLE_TILE_HEIGHT) * tiles.columns + tile_x]
                }) {
                    continue;
                }
                for (offset, out) in tile.iter_mut().enumerate() {
                    let x = tile_x * UPSAMPLE_TILE_WIDTH + offset;
                    let mut sum = [0.0_f64; CHANNELS];
                    for (dx, dy, weight) in TENT {
                        let sample = source
                            .bilinear(columns[x][(dx + 1) as usize], lines[y][(dy + 1) as usize]);
                        for (acc, value) in sum.iter_mut().zip(sample) {
                            *acc += weight * value;
                        }
                    }
                    *out = sum.map(|channel| quantize_half(channel / 16.0));
                }
            }
        }
    });
    next
}

pub(crate) fn encode_output(pixels: &[[f32; 4]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(pixels.len() * 8);
    for pixel in pixels {
        for channel in pixel {
            bytes.extend_from_slice(&f32_to_half(*channel).to_le_bytes());
        }
    }
    bytes
}

#[inline]
pub(crate) fn quantize_half(value: f64) -> f32 {
    let value = value as f32;
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

pub(crate) fn half_to_f32(bits: u16) -> f32 {
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
            let columns = (0..width)
                .map(|x| {
                    [-1, 0, 1].map(|tap| {
                        LinearAxis::new(
                            (x as f64 + 0.5) * source.width as f64 / width as f64 - 0.5
                                + f64::from(tap) * 0.5 * spread,
                            source.width,
                        )
                    })
                })
                .collect::<Vec<_>>();
            let lines = (0..height)
                .map(|y| {
                    [-1, 0, 1].map(|tap| {
                        LinearAxis::new(
                            (y as f64 + 0.5) * source.height as f64 / height as f64 - 0.5
                                + f64::from(tap) * 0.5 * spread,
                            source.height,
                        )
                    })
                })
                .collect::<Vec<_>>();
            let mask = upsample_tiles(&source, &columns, &lines);
            if dense {
                assert!(mask.is_none());
            } else if spread < 1.0 {
                assert!(mask.as_ref().unwrap().active.iter().any(|active| !active));
            }
            for y in 0..height {
                for x in 0..width {
                    let cx = (x as f64 + 0.5) * source.width as f64 / width as f64 - 0.5;
                    let cy = (y as f64 + 0.5) * source.height as f64 / height as f64 - 0.5;
                    let mut sum = [0.0; CHANNELS];
                    for (dx, dy, weight) in TENT {
                        let sample = source.bilinear(
                            LinearAxis::new(cx + f64::from(dx) * 0.5 * spread, source.width),
                            LinearAxis::new(cy + f64::from(dy) * 0.5 * spread, source.height),
                        );
                        for (acc, value) in sum.iter_mut().zip(sample) {
                            *acc += weight * value;
                        }
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
    fn half_quantization_fast_path_preserves_reference_rounding() {
        let check = |value: f64| {
            assert_eq!(
                quantize_half(value).to_bits(),
                half_to_f32(f32_to_half(value as f32)).to_bits(),
                "input {value:?} ({:016x})",
                value.to_bits(),
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
        // Captured from the original three-channel, per-pixel sampling kernel.
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
                "2930313f6761f65dc5ac4cf225abc3cccb1ba5435eeccafeac9d8ba10d27f2b7",
                "5ddc958698726fa0652b6089800ca2b66f71c6a9ddc23b9eb58b0fcb511579d1",
            ),
            (
                2.5,
                "2930313f6761f65dc5ac4cf225abc3cccb1ba5435eeccafeac9d8ba10d27f2b7",
                "5ddc958698726fa0652b6089800ca2b66f71c6a9ddc23b9eb58b0fcb511579d1",
            ),
            (
                8.25,
                "37d437708a0541eae5f587386d04ecf6345d6654889f833967d27aa2097c4f2e",
                "3aca5351bbf0489dfd33817667958d51e866752e00908fa61609c298bd0d50e9",
            ),
            (
                23.5,
                "b3c5169209c28e8ef56b01fdfa52b5dbd1d0e2148ee567fd42596f757434da08",
                "3f272f1560b0e41c919dbfe69c422bfd2fe963ce948464d0e8257dd3440397ab",
            ),
            (
                48.0,
                "373d63ca7450537d3c1181a7a2168f8f2cf6b8da48fa91ccaf883f2af31cfd83",
                "9fe4cb564f8907f80fb2352977d64c31d2b1ffa24e1447593c01baf69818c1bd",
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
        assert!(sample(&wide, 85) > sample(&one, 85));
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
