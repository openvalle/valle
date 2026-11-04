//! Compare the GPU's paired samples against a discrete Gaussian, including reflected borders.
use valle_engine::gpu::{Gpu, Renderer};
use valle_engine::layout::Rect;
use valle_engine::paint::{Filter, Item, SQUARE};

fn gpu() -> Option<Gpu> {
    std::env::var("VALLE_TEST_LAYERS")
        .unwrap_or_default()
        .split(',')
        .any(|v| v == "gpu")
        .then(|| Gpu::new().expect("GPU"))
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
    mut pixels: Vec<f64>,
    size: [usize; 2],
    sigma: f64,
    bounds: Option<[i32; 4]>,
) -> Vec<f64> {
    let [w, h] = size;
    let [x, y, bw, bh] = bounds.unwrap_or([0, 0, w as i32, h as i32]);
    let n = (3.0 * sigma).ceil() as i32;
    let weights: Vec<_> = (-n..=n)
        .map(|i| (-f64::from(i * i) / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f64 = weights.iter().sum();
    for axis in 0..2 {
        let mut next = vec![0.0; w * h];
        for py in 0..h {
            for px in 0..w {
                for (offset, weight) in (-n..=n).zip(&weights) {
                    let sx = px as i32 + if axis == 0 { offset } else { 0 };
                    let sy = py as i32 + if axis == 1 { offset } else { 0 };
                    let sx =
                        folded(sx, x, x + bw, bounds.is_some()).clamp(0, w as i32 - 1) as usize;
                    let sy =
                        folded(sy, y, y + bh, bounds.is_some()).clamp(0, h as i32 - 1) as usize;
                    next[py * w + px] += pixels[sy * w + sx] * weight / total;
                }
            }
        }
        pixels = next;
    }
    pixels
}

#[test]
fn gaussian_samples_keep_odd_kernels_and_mirrored_edges() {
    let Some(gpu) = gpu() else { return };
    let size = [32, 24];
    let mut renderer = Renderer::new(&gpu, size[0] as u32, size[1] as u32);
    let data: Vec<f64> = (0..size[0] * size[1])
        .map(|i| f64::from((i * 17 + i / size[0] * 7) % 11 < 5))
        .collect();
    let items: Vec<_> = data
        .iter()
        .enumerate()
        .map(|(i, &v)| Item::Rect {
            rect: Rect {
                x: (i % size[0]) as f32,
                y: (i / size[0]) as f32,
                width: 1.0,
                height: 1.0,
            },
            radii: SQUARE,
            color: [v as f32, v as f32, v as f32, 1.0],
        })
        .collect();
    for sigma in [0.01, 0.15, 0.7, 1.0, 1.25, 8.0, 30.0] {
        for bounds in [
            None,
            Some([3, 5, 7, 9]),
            Some([4, 2, 1, 3]),
            Some([-3, -5, 11, 13]),
            Some([25, 18, 12, 11]),
        ] {
            let filters = vec![Filter::Blur(sigma as f32)];
            let drawn = match bounds {
                None => vec![Item::Filter {
                    filters,
                    items: items.clone(),
                }],
                Some([x, y, w, h]) => {
                    let mut drawn = items.clone();
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
            };
            let actual = renderer.render(&drawn, &[], &[], &[]).unwrap();
            let expected = reference(data.clone(), size, sigma, bounds);
            for y in 0..size[1] {
                for x in 0..size[0] {
                    let inside = bounds.is_none_or(|[bx, by, w, h]| {
                        (bx..bx + w).contains(&(x as i32)) && (by..by + h).contains(&(y as i32))
                    });
                    let i = y * size[0] + x;
                    let v = if inside { expected[i] } else { data[i] };
                    let wanted = (v * 255.0).round() as u8;
                    for &a in &actual[i * 4..i * 4 + 3] {
                        assert!(
                            a.abs_diff(wanted) <= 1,
                            "sigma {sigma}, bounds {bounds:?}, {x},{y}: {a} != {wanted}"
                        );
                    }
                    assert_eq!(actual[i * 4 + 3], 255);
                }
            }
        }
    }
}
