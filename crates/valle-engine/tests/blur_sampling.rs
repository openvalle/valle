//! Exact Gaussian sampling, reduced-copy quality, and continuity as copy sizes change.

use valle_engine::gpu::{Gpu, Renderer};
use valle_engine::layout::Rect;
use valle_engine::paint::{Filter, Item, SQUARE};

// Premultiplied sRGB, as the GPU working surface stores it.
type Pixel = [f64; 4];

fn gpu() -> Option<Gpu> {
    std::env::var("VALLE_TEST_LAYERS")
        .unwrap_or_default()
        .split(',')
        .any(|v| v == "gpu")
        .then(|| {
            valle_native::gpu::adapter(None)
                .and_then(|c| Gpu::with(c.adapter))
                .expect("GPU")
        })
}

fn folded(p: i32, low: i32, high: i32, mirrored: bool) -> i32 {
    if !mirrored {
        return p.clamp(low, high - 1);
    }
    let n = high - low;
    let q = (p - low).rem_euclid(2 * n);
    low + if q < n { q } else { 2 * n - 1 - q }
}

fn reference(
    mut pixels: Vec<Pixel>,
    size: [usize; 2],
    sigma: f64,
    bounds: Option<[i32; 4]>,
) -> Vec<Pixel> {
    let original = pixels.clone();
    let [w, h] = size;
    let [x, y, bw, bh] = bounds.unwrap_or([0, 0, w as i32, h as i32]);
    let n = (3.0 * sigma).ceil() as i32;
    let mut weights: Vec<_> = (-n..=n)
        .map(|i| (-f64::from(i * i) / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f64 = weights.iter().sum();
    for weight in &mut weights {
        *weight /= total;
    }
    for axis in 0..2 {
        let mut next = vec![[0.0; 4]; w * h];
        for py in 0..h {
            for px in 0..w {
                for (offset, weight) in (-n..=n).zip(&weights) {
                    let sx = px as i32 + if axis == 0 { offset } else { 0 };
                    let sy = py as i32 + if axis == 1 { offset } else { 0 };
                    let sx =
                        folded(sx, x, x + bw, bounds.is_some()).clamp(0, w as i32 - 1) as usize;
                    let sy =
                        folded(sy, y, y + bh, bounds.is_some()).clamp(0, h as i32 - 1) as usize;
                    for k in 0..4 {
                        next[py * w + px][k] += pixels[sy * w + sx][k] * weight;
                    }
                }
            }
        }
        pixels = next;
    }
    if let Some(bounds) = bounds {
        for (i, p) in pixels.iter_mut().enumerate() {
            // Item::Backdrop composites the filtered copy over what was already there.
            *p = if inside(i, size, Some(bounds)) {
                std::array::from_fn(|k| p[k] + original[i][k] * (1.0 - p[3]))
            } else {
                original[i]
            };
        }
    }
    pixels
}

fn data(pattern: Pattern, [w, h]: [usize; 2]) -> Vec<Pixel> {
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let v = match pattern {
                Pattern::Solid => return [0.25, 0.5, 0.75, 1.0],
                Pattern::Noise => (i * 17 + y * 7) % 11 < 5,
                Pattern::Edge => x >= w / 2,
                Pattern::Line => x == w / 2,
                // A single pixel nearly disappears into 8-bit rounding after a wide blur.
                Pattern::Spot => x.abs_diff(w / 2) <= 2 && y.abs_diff(h / 2) <= 2,
                Pattern::Transparent => {
                    return if (w / 4..3 * w / 4).contains(&x) && (h / 4..3 * h / 4).contains(&y) {
                        if x < w / 2 {
                            [0.5, 0.0, 0.0, 0.5]
                        } else {
                            [0.0, 0.5, 0.5, 0.5]
                        }
                    } else {
                        [0.0; 4]
                    };
                }
            };
            [f64::from(v), f64::from(v), f64::from(v), 1.0]
        })
        .collect()
}

fn items(data: &[Pixel], width: usize, sigma: f64, bounds: Option<[i32; 4]>) -> Vec<Item> {
    let items: Vec<_> = data
        .iter()
        .enumerate()
        .map(|(i, p)| Item::Rect {
            rect: Rect {
                x: (i % width) as f32,
                y: (i / width) as f32,
                width: 1.0,
                height: 1.0,
            },
            radii: SQUARE,
            color: p.map(|v| v as f32),
        })
        .collect();
    let filters = vec![Filter::Blur(sigma as f32)];
    match bounds {
        None => vec![Item::Filter { filters, items }],
        Some([x, y, w, h]) => {
            let mut drawn = items;
            drawn.push(Item::Backdrop {
                filters,
                rect: Rect {
                    x: x as f32,
                    y: y as f32,
                    width: w as f32,
                    height: h as f32,
                },
                radii: SQUARE,
            });
            drawn
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Pattern {
    Solid,
    Noise,
    Edge,
    Line,
    Spot,
    Transparent,
}

const PATTERNS: [Pattern; 6] = [
    Pattern::Solid,
    Pattern::Noise,
    Pattern::Edge,
    Pattern::Line,
    Pattern::Spot,
    Pattern::Transparent,
];

fn bounds([w, h]: [usize; 2]) -> [Option<[i32; 4]>; 6] {
    [
        None,
        Some([3, 5, 7, 9]),
        Some([4, 2, 1, 3]),
        Some([3, 5, w as i32 - 7, h as i32 - 10]),
        Some([-3, -5, 11, 13]),
        Some([w as i32 - 7, h as i32 - 6, 12, 11]),
    ]
}

fn inside(i: usize, [w, _]: [usize; 2], bounds: Option<[i32; 4]>) -> bool {
    bounds.is_none_or(|[x, y, width, height]| {
        (x..x + width).contains(&((i % w) as i32)) && (y..y + height).contains(&((i / w) as i32))
    })
}

// Signed errors, in 8-bit levels, including alpha. Compare both black and white composites so
// invisible RGB does not count, but neither colored fringes nor missing alpha can hide.
fn residuals(actual: &[u8], expected: &[Pixel]) -> Vec<Pixel> {
    actual
        .as_chunks::<4>()
        .0
        .iter()
        .zip(expected)
        .map(|(a, e)| {
            let alpha = f64::from(a[3]) / 255.0;
            [
                f64::from(a[0]) * alpha - e[0] * 255.0,
                f64::from(a[1]) * alpha - e[1] * 255.0,
                f64::from(a[2]) * alpha - e[2] * 255.0,
                f64::from(a[3]) - e[3] * 255.0,
            ]
        })
        .collect()
}

fn magnitude(d: Pixel) -> f64 {
    d[..3].iter().fold(d[3].abs(), |worst, v| {
        worst.max(v.abs()).max((v - d[3]).abs())
    })
}

// The maximum per-pixel error, mean of those errors, and fraction above two levels. The mean
// and fraction apply only inside the affected region, not to unchanged pixels around it.
fn check(errors: impl Iterator<Item = Pixel>, [max, mean, far]: [f64; 3], context: &str) {
    let mut peak = 0.0f64;
    let mut sum = 0.0;
    let mut count = 0;
    let mut over = 0;
    for d in errors {
        let error = magnitude(d);
        peak = peak.max(error);
        sum += error;
        count += 1;
        over += usize::from(error > 2.0);
    }
    assert!(count > 0, "{context}: empty comparison");
    let average = sum / count as f64;
    let fraction = over as f64 / count as f64;
    assert!(
        peak <= max && average <= mean && fraction <= far,
        "{context}: max {peak:.3}/{max}, mean {average:.3}/{mean}, \
         pixels above 2/255: {over}/{count} ({fraction:.3}/{far})"
    );
}

fn sample(
    renderer: &mut Renderer<'_>,
    source: &[Pixel],
    size: [usize; 2],
    sigma: f64,
    bounds: Option<[i32; 4]>,
    limit: [f64; 3],
    context: &str,
) -> Vec<Pixel> {
    let drawn = items(source, size[0], sigma, bounds);
    let actual = renderer.render(&drawn, &[], &[], &[]).unwrap();
    let expected = reference(source.to_vec(), size, sigma, bounds);
    let errors = residuals(&actual, &expected);
    if source.iter().all(|p| p[3] == 1.0) {
        assert!(
            actual.as_chunks::<4>().0.iter().all(|p| p[3] == 255),
            "{context}: opaque input lost alpha"
        );
    }
    for (i, &d) in errors.iter().enumerate() {
        if !inside(i, size, bounds) {
            assert!(magnitude(d) <= 1.0, "{context}: changed outside pixel {i}");
        }
    }
    check(
        errors
            .iter()
            .enumerate()
            .filter_map(|(i, &d)| inside(i, size, bounds).then_some(d)),
        limit,
        context,
    );
    errors
}

#[test]
fn gaussian_samples_keep_odd_kernels_and_mirrored_edges() {
    let Some(gpu) = gpu() else { return };
    let size = [32, 24];
    let source = data(Pattern::Noise, size);
    let mut renderer = Renderer::new(&gpu, 32, 24);
    // plan.rs also asserts that these stay on the full-resolution path. Radius 8 is even,
    // radius 15 odd; the final unpaired tap must not be dropped.
    for sigma in [0.01, 0.15, 0.7, 1.0, 1.25, 2.5, 5.0] {
        for bounds in bounds(size) {
            let drawn = items(&source, size[0], sigma, bounds);
            let actual = renderer.render(&drawn, &[], &[], &[]).unwrap();
            let expected = reference(source.clone(), size, sigma, bounds);
            for (i, (a, e)) in actual.as_chunks::<4>().0.iter().zip(expected).enumerate() {
                for k in 0..3 {
                    let wanted = (e[k] * 255.0).round() as u8;
                    assert!(
                        a[k].abs_diff(wanted) <= 1,
                        "sigma {sigma}, bounds {bounds:?}, {},{}: {} != {wanted}",
                        i % size[0],
                        i / size[0],
                        a[k]
                    );
                }
                assert_eq!(a[3], 255);
            }
        }
    }
}

#[test]
fn reduced_blurs_preserve_color_alpha_and_edges() {
    let Some(gpu) = gpu() else { return };
    for size in [[32, 24], [65, 49]] {
        let mut renderer = Renderer::new(&gpu, size[0] as u32, size[1] as u32);
        for pattern in PATTERNS {
            let source = data(pattern, size);
            // Area averaging and linear reconstruction approximate a Gaussian. For these
            // fixtures, allow at most 2 levels for shapes, 3 for high-frequency noise; a mean
            // below one level and a separate outlier limit reject widespread small errors.
            let limit = match pattern {
                Pattern::Solid => [1.0, 0.6, 0.0],
                Pattern::Noise => [3.0, 0.75, 0.05],
                _ => [2.0, 0.75, 0.0],
            };
            for sigma in [8.0, 30.0, 60.0] {
                for bounds in bounds(size) {
                    let context =
                        format!("{pattern:?}, {size:?}, sigma {sigma}, bounds {bounds:?}");
                    sample(&mut renderer, &source, size, sigma, bounds, limit, &context);
                }
            }
        }
    }
}

#[test]
fn changing_copy_size_keeps_blur_continuous() {
    let Some(gpu) = gpu() else { return };
    for size in [[32, 24], [65, 49]] {
        let mut renderer = Renderer::new(&gpu, size[0] as u32, size[1] as u32);
        for pattern in PATTERNS {
            let source = data(pattern, size);
            let limit = match pattern {
                Pattern::Solid => [1.0, 0.6, 0.0],
                // The smallest copy-space sigma has the largest noise approximation error.
                Pattern::Noise => [4.0, 1.0, 0.1],
                _ => [2.0, 0.75, 0.0],
            };
            // Differencing two separately rounded frames needs its own limit. Shapes still
            // meet the stricter per-frame bound above; at most 5% may have over two levels of
            // change beyond the reference's own change.
            let continuity = match pattern {
                Pattern::Solid | Pattern::Noise => limit,
                _ => [3.0, 0.75, 0.05],
            };
            for [before, after] in [[6.07, 6.08], [12.16, 12.17], [24.32, 24.34], [48.65, 48.67]] {
                for bounds in [None, Some([3, 5, size[0] as i32 - 7, size[1] as i32 - 10])] {
                    let context = format!(
                        "{pattern:?}, {size:?}, sigma {before} -> {after}, bounds {bounds:?}"
                    );
                    let a = sample(
                        &mut renderer,
                        &source,
                        size,
                        before,
                        bounds,
                        limit,
                        &context,
                    );
                    let b = sample(&mut renderer, &source, size, after, bounds, limit, &context);
                    // Subtract the CPU reference's own change: only the extra step introduced
                    // by switching resolution counts as a discontinuity.
                    check(
                        a.iter()
                            .zip(&b)
                            .enumerate()
                            .filter(|(i, _)| inside(*i, size, bounds))
                            .map(|(_, (a, b))| std::array::from_fn(|k| b[k] - a[k])),
                        continuity,
                        &context,
                    );
                }
            }
        }
    }
}
