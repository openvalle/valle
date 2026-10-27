//! Finite-support fourth-order Deriche Gaussian approximation.
//!
//! The two conjugate pole pairs below are Deriche's optimized order-four coefficients
//! (Getreuer, IPOL 2013, https://www.ipol.im/pub/art/2013/87/). We evaluate the partial
//! fractions directly, rather than expanding a poorly conditioned fourth-order denominator.
//! Removing the expired input from each recurrence preserves Motion's finite, transparent
//! support. This matters near black: an infinite Gaussian tail is visible after sRGB encoding.
//! Factored pole pairs stay well-conditioned in f32; coefficients are normalized in f64.

use valle_engine::compositor::bloom::fill_image_rows;

use super::{
    constant_horizontal_spans, fill_constant_vertical_rows, horizontal_span,
    kernel_with_radius_limit, vertical_span,
};
use valle_draw::program::FILTER_GAUSSIAN_SUPPORT_SIGMAS;

pub(super) const MAX_SIGMA: f32 = 32.0;
// Dense 1080p M2 Pro benchmarks favor recursion above sigma 12. The interval cost test
// below preserves direct convolution for small spans even above that threshold.
const MIN_SIGMA: f32 = 12.0;

#[derive(Clone, Copy)]
struct Pair {
    pole: [f32; 2],
    expired: [f32; 2],
    weight: [f32; 2],
}

struct Recursive {
    pairs: [Pair; 2],
    radius: usize,
    center: f32,
}

impl Recursive {
    fn new(sigma: f32) -> Self {
        let radius = (sigma * FILTER_GAUSSIAN_SUPPORT_SIGMAS).ceil() as usize;
        let pairs = [
            (0.84, 1.8675, 1.783, 0.6318),
            (-0.34015, -0.1299, 1.723, 1.997),
        ];
        let mut mass = 0.0;
        let result = pairs.map(|(ar, ai, lr, li)| {
            let sigma = f64::from(sigma);
            let pole = complex_exp(-lr / sigma, -li / sigma);
            let expired = complex_exp(
                -lr * (radius + 1) as f64 / sigma,
                -li * (radius + 1) as f64 / sigma,
            );
            // Sum the finite geometric series, including the center once per direction.
            let denominator = (1.0 - pole[0]).powi(2) + pole[1].powi(2);
            let sum = [
                ((1.0 - expired[0]) * (1.0 - pole[0]) + expired[1] * pole[1]) / denominator,
                ((1.0 - expired[0]) * pole[1] - expired[1] * (1.0 - pole[0])) / denominator,
            ];
            mass += 4.0 * (ar * sum[0] - ai * sum[1]) - 2.0 * ar;
            (pole, expired, [2.0 * ar, 2.0 * ai])
        });
        let result = result.map(|(pole, expired, weight)| Pair {
            pole: pole.map(|value| value as f32),
            expired: expired.map(|value| value as f32),
            weight: weight.map(|value| (value / mass) as f32),
        });
        Self {
            pairs: result,
            radius,
            center: result.iter().map(|pair| pair.weight[0]).sum(),
        }
    }

    /// A lane is a channel horizontally, or one channel of a column vertically. Output
    /// bands warm the finite recurrence with their own halo, so scheduling and seeks cannot
    /// change pixels. State is bounded by the row width, not by sigma or image height.
    fn line(&self, source: &[f32], lanes: usize, output: &mut [f32], first: usize) {
        let input_length = source.len() / lanes;
        let count = output.len() / lanes;
        let start = first as isize - self.radius as isize;
        let end = start + count as isize;
        let mut small_state = [0.0_f32; 16];
        let mut large_state = if lanes > 4 {
            vec![0.0; lanes * 4]
        } else {
            Vec::new()
        };
        let state = if lanes <= 4 {
            &mut small_state[..lanes * 4]
        } else {
            &mut large_state[..]
        };
        let mut small_zero = [0.0_f32; 4];
        let large_zero = if lanes > 4 {
            vec![0.0; lanes]
        } else {
            Vec::new()
        };
        let zero = if lanes <= 4 {
            &mut small_zero[..lanes]
        } else {
            &large_zero[..]
        };
        for reverse in [false, true] {
            state.fill(0.0);
            for step in 0..count + self.radius {
                let position = if reverse {
                    end + self.radius as isize - 1 - step as isize
                } else {
                    start - self.radius as isize + step as isize
                };
                let expired = position
                    + if reverse {
                        self.radius as isize + 1
                    } else {
                        -(self.radius as isize) - 1
                    };
                let input_row = (0..input_length as isize)
                    .contains(&position)
                    .then(|| &source[position as usize * lanes..][..lanes]);
                let expired_row = (step > self.radius
                    && (0..input_length as isize).contains(&expired))
                .then(|| &source[expired as usize * lanes..][..lanes]);
                let destination = if (start..end).contains(&position) {
                    Some(&mut output[(position - start) as usize * lanes..][..lanes])
                } else {
                    None
                };
                self.step(
                    state,
                    input_row.unwrap_or(zero),
                    expired_row.unwrap_or(zero),
                    destination,
                    reverse,
                );
            }
        }
    }

    #[inline]
    fn step(
        &self,
        state: &mut [f32],
        input: &[f32],
        expired: &[f32],
        mut output: Option<&mut [f32]>,
        reverse: bool,
    ) {
        let lanes = input.len();
        let (real0, rest) = state.split_at_mut(lanes);
        let (imaginary0, rest) = rest.split_at_mut(lanes);
        let (real1, imaginary1) = rest.split_at_mut(lanes);
        let [a, b] = self.pairs;
        for lane in 0..lanes {
            let x = input[lane];
            let old = expired[lane];
            let r0 =
                x + a.pole[0] * real0[lane] - a.pole[1] * imaginary0[lane] - a.expired[0] * old;
            let i0 = a.pole[1] * real0[lane] + a.pole[0] * imaginary0[lane] - a.expired[1] * old;
            let r1 =
                x + b.pole[0] * real1[lane] - b.pole[1] * imaginary1[lane] - b.expired[0] * old;
            let i1 = b.pole[1] * real1[lane] + b.pole[0] * imaginary1[lane] - b.expired[1] * old;
            real0[lane] = r0;
            imaginary0[lane] = i0;
            real1[lane] = r1;
            imaginary1[lane] = i1;
            if let Some(output) = output.as_deref_mut() {
                let sum = a.weight[0] * r0 - a.weight[1] * i0 + b.weight[0] * r1 - b.weight[1] * i1;
                if reverse {
                    output[lane] += sum - self.center * x;
                } else {
                    output[lane] = sum;
                }
            }
        }
    }
}

fn complex_exp(real: f64, imaginary: f64) -> [f64; 2] {
    let magnitude = real.exp();
    let (sin, cos) = imaginary.sin_cos();
    [magnitude * cos, magnitude * sin]
}

struct Axis {
    weights: Vec<f32>,
    recursive: Option<Recursive>,
}
impl Axis {
    fn new(sigma: f32) -> Self {
        Self {
            weights: kernel_with_radius_limit(sigma, 288).expect("admitted blur sigma"),
            recursive: (sigma > MIN_SIGMA).then(|| Recursive::new(sigma)),
        }
    }
    fn radius(&self) -> usize {
        self.weights.len() / 2
    }
}

pub(super) fn applies(x: f32, y: f32) -> bool {
    x > MIN_SIGMA || y > MIN_SIGMA
}

pub(super) fn convolve(
    source: &[f32],
    width: usize,
    height: usize,
    channels: usize,
    sigma_x: f32,
    sigma_y: f32,
) -> Vec<f32> {
    let x = Axis::new(sigma_x);
    let y = Axis::new(sigma_y);
    if !applies(sigma_x, sigma_y) || exceeds_finite_half(source) {
        // Recursive subtraction is undefined for infinities. Keep direct Gaussian arithmetic
        // for nonfinite or unusually large F32 inputs; F16's entire finite range is admitted.
        return super::convolve(source, width, height, channels, &x.weights, &y.weights);
    }
    let stride = (width + 2 * x.radius()) * channels;
    let mut horizontal = vec![0.0; stride * height];
    fill_image_rows(&mut horizontal, stride, |first, rows| {
        let mut work = Vec::new();
        let mut offset = 0;
        while offset < rows.len() / stride {
            let input = &source[(first + offset) * width * channels..][..width * channels];
            let mut end = offset + 1;
            while end < rows.len() / stride
                && super::float_bits(input)
                    == super::float_bits(
                        &source[(first + end) * width * channels..][..width * channels],
                    )
            {
                end += 1;
            }
            // Exact empty-row admission also avoids building recurrences for sparse text/glow.
            if input.iter().all(|value| *value == 0.0) {
                offset = end;
                continue;
            }
            let row = &mut rows[offset * stride..][..stride];
            constant_horizontal_spans(input, row, channels, &x.weights, &mut work);
            for span in &work {
                let recursive_cost = (span.len() + 2 * x.radius() * channels) * 35;
                if let Some(filter) = &x.recursive
                    && span.len() * x.weights.len() > recursive_cost
                {
                    filter.line(
                        input,
                        channels,
                        &mut row[span.clone()],
                        span.start / channels,
                    );
                } else {
                    horizontal_span(input, row, span.clone(), channels, &x.weights);
                }
            }
            let (prefix, remaining) = rows.split_at_mut((offset + 1) * stride);
            let row = &prefix[offset * stride..];
            for target in remaining[..(end - offset - 1) * stride].chunks_exact_mut(stride) {
                target.copy_from_slice(row);
            }
            offset = end;
        }
    });
    let mut output = vec![0.0; stride * (height + 2 * y.radius())];
    let mut constant = vec![false; output.len() / stride];
    fill_constant_vertical_rows(&horizontal, &mut output, stride, &y.weights, &mut constant);
    // Fixed bands are part of the arithmetic, not the worker schedule: changing the CPU
    // budget only groups these jobs differently. Recurrence warm-up must never depend on
    // how Rayon happened to divide the image.
    let band_rows = 64;
    let bands = output.len().div_ceil(stride * band_rows);
    let fill = |first: usize, rows: &mut [f32]| {
        for (index, band) in rows.chunks_mut(stride * band_rows).enumerate() {
            vertical(
                &horizontal,
                stride,
                first + index * band_rows,
                band,
                &y,
                &constant,
            );
        }
    };
    valle_engine::motion::cpu::parallel_scope(
        if output.len() >= 65_536 { 8 } else { 1 },
        |scope, count| {
            let rows_per_job = bands.div_ceil(count) * band_rows;
            let mut chunks = output.chunks_mut(stride * rows_per_job).enumerate();
            let (_, caller) = chunks.next().expect("nonempty blur output");
            for (index, rows) in chunks {
                let fill = &fill;
                scope.spawn(move || fill(index * rows_per_job, rows));
            }
            fill(0, caller);
        },
    );
    output
}

fn exceeds_finite_half(source: &[f32]) -> bool {
    // Absolute IEEE bits have the same order as positive floats, including infinities and
    // NaNs above every finite value. A reduction lets LLVM vectorize the admission scan;
    // a per-element short-circuit branch costs several milliseconds on an entire 1080p image.
    source.iter().fold(0_u32, |maximum, value| {
        maximum.max(value.to_bits() & 0x7fff_ffff)
    }) > 65_504.0_f32.to_bits()
}

fn vertical(
    horizontal: &[f32],
    stride: usize,
    first: usize,
    rows: &mut [f32],
    filter: &Axis,
    constant: &[bool],
) {
    let count = rows.len() / stride;
    let mut offset = 0;
    while offset < count {
        if constant[first + offset] {
            offset += 1;
            continue;
        }
        let start = offset;
        while offset < count && !constant[first + offset] {
            offset += 1;
        }
        let output = &mut rows[start * stride..offset * stride];
        if let Some(recursive) = &filter.recursive {
            recursive.line(horizontal, stride, output, first + start);
        } else {
            for (index, row) in output.chunks_exact_mut(stride).enumerate() {
                vertical_span(
                    horizontal,
                    row,
                    first + start + index,
                    0,
                    stride,
                    &filter.weights,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recurrence_admission_preserves_nonfinite_direct_arithmetic() {
        assert!(!exceeds_finite_half(&[-65_504.0, 0.0, -0.0, 65_504.0]));
        for value in [
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            -f32::NAN,
            65_505.0,
        ] {
            assert!(exceeds_finite_half(&[0.5, value, 0.0]));
            let weights = kernel_with_radius_limit(16.0, 288).unwrap();
            let source = [0.5, value, 0.0];
            let expected = super::super::convolve(&source, 3, 1, 1, &weights, &weights);
            let actual = convolve(&source, 3, 1, 1, 16.0, 16.0);
            assert!(
                actual
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| a.to_bits() == b.to_bits())
            );
        }
    }

    #[test]
    fn production_admission_keeps_wide_skia_blurs_unchanged() {
        for sigma in [0.0, 12.0, 32.0] {
            assert!(super::super::supports(&valle_draw::program::Filter::Blur {
                sigma_x: sigma,
                sigma_y: sigma,
            }));
        }
        for sigma in [32.01, 96.0, f32::NAN, f32::INFINITY, -1.0] {
            assert!(!super::super::supports(
                &valle_draw::program::Filter::Blur {
                    sigma_x: sigma,
                    sigma_y: sigma,
                }
            ));
        }
    }

    #[test]
    fn recursive_kernel_matches_finite_gaussian_including_transparent_boundaries() {
        for sigma in [3.01, 4.0, 8.0, 16.0, 32.0, 64.0, 96.0] {
            let reference = kernel_with_radius_limit(sigma, 288).unwrap();
            let filter = Recursive::new(sigma);
            let mut actual = vec![0.0; reference.len()];
            filter.line(&[1.0], 1, &mut actual, 0);
            let l1 = actual
                .iter()
                .zip(&reference)
                .map(|(a, b)| (a - b).abs())
                .sum::<f32>();
            let mass = actual.iter().sum::<f32>();
            assert!(l1 < 0.0007, "sigma={sigma} L1={l1}");
            assert!((mass - 1.0).abs() < 0.00001, "sigma={sigma} mass={mass}");
            assert!(actual.iter().all(|value| *value >= 0.0));
        }
    }

    #[test]
    fn bands_channels_and_transparent_input_are_deterministic() {
        for channels in [1, 4, 17] {
            let input: Vec<_> = (0..351 * channels)
                .map(|i| ((i * 71 % 103) as f32 - 45.0) / 103.0)
                .collect();
            for sigma in [3.1, 19.5, 96.0] {
                let filter = Recursive::new(sigma);
                let mut full = vec![0.0; (351 + filter.radius * 2) * channels];
                filter.line(&input, channels, &mut full, 0);
                let mut bands = vec![0.0; full.len()];
                for (i, band) in bands.chunks_mut(37 * channels).enumerate() {
                    filter.line(&input, channels, band, i * 37);
                }
                let error = full
                    .iter()
                    .zip(&bands)
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0_f32, f32::max);
                assert!(
                    error < 2e-6,
                    "channels={channels} sigma={sigma} error={error}"
                );
            }
        }
        assert!(
            convolve(&[0.0; 32], 8, 4, 1, 96.0, 4.0)
                .iter()
                .all(|v| *v == 0.0)
        );
    }

    #[test]
    fn worker_budget_does_not_change_recurrence_boundaries_or_pixels() {
        let source: Vec<_> = (0..320 * 181 * 4)
            .map(|i| (i * 71 % 103) as f32 / 103.0)
            .collect();
        let render = |threads| {
            valle_engine::motion::cpu::CpuBudget::new(threads)
                .unwrap()
                .run(|| convolve(&source, 320, 181, 4, 13.0, 29.0))
        };
        let single = render(1);
        for threads in [2, 4, 8] {
            assert_eq!(single, render(threads), "threads={threads}");
        }
    }

    #[test]
    fn recursive_separable_output_stays_within_sdr_tolerance() {
        for (width, height, channels) in [(1, 1, 1), (3, 2, 4), (47, 29, 4)] {
            let input: Vec<_> = (0..width * height * channels)
                .map(|i| (i * 71 % 103) as f32 / 103.0)
                .collect();
            for (sx, sy) in [
                (3.1, 0.0),
                (0.7, 8.0),
                (4.0, 4.0),
                (17.0, 6.0),
                (96.0, 96.0),
            ] {
                let x = kernel_with_radius_limit(sx, 288).unwrap();
                let y = kernel_with_radius_limit(sy, 288).unwrap();
                let expected = super::super::convolve(&input, width, height, channels, &x, &y);
                let actual = convolve(&input, width, height, channels, sx, sy);
                assert_eq!(actual.len(), expected.len());
                let error = actual
                    .iter()
                    .zip(&expected)
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0_f32, f32::max);
                // 12.92 is the maximum sRGB transfer slope. This absolute linear bound
                // therefore covers dark edges, not merely already-bright sample pixels.
                assert!(
                    error * 12.92 * 255.0 < 2.0,
                    "{width}x{height} sigma={sx},{sy} error={error}"
                );
            }
        }
    }

    #[test]
    #[ignore = "manual 1080p algorithm throughput benchmark"]
    fn benchmark_recursive_and_direct_1080p() {
        use std::{hint::black_box, time::Instant};
        let width = 1920;
        let height = 1080;
        for pattern in ["noise", "solid", "sparse"] {
            let input = (0..width * height * 4)
                .map(|i| match pattern {
                    "solid" => 0.5,
                    "sparse"
                        if (400..440).contains(&(i / 4 % width))
                            && (300..340).contains(&(i / 4 / width)) =>
                    {
                        0.5
                    }
                    "sparse" => 0.0,
                    _ => (i * 71 % 103) as f32 / 103.0,
                })
                .collect::<Vec<_>>();
            for sigma in [4.0, 16.0, 32.0] {
                let kernel = kernel_with_radius_limit(sigma, 288).unwrap();
                for repeat in 0..4 {
                    let start = Instant::now();
                    black_box(super::super::convolve(
                        black_box(&input),
                        width,
                        height,
                        4,
                        &kernel,
                        &kernel,
                    ));
                    let direct = start.elapsed().as_secs_f64() * 1000.0;
                    for threads in [1, 4] {
                        let budget = valle_engine::motion::cpu::CpuBudget::new(threads).unwrap();
                        let start = Instant::now();
                        budget.run(|| {
                            black_box(convolve(black_box(&input), width, height, 4, sigma, sigma))
                        });
                        eprintln!(
                            "blur-bench pattern={pattern} sigma={sigma} repeat={repeat} threads={threads} direct_ms={direct:.3} recursive_ms={:.3}",
                            start.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                }
            }
        }
    }
}
