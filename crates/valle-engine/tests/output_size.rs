//! Delivery pixels are independent of layout pixels. GPU checks run with the case suite's flag.

use valle_engine::gpu::{Gpu, Renderer, output_size};
use valle_engine::image::Image;
use valle_engine::layout::Rect;
use valle_engine::paint::{Filter, Item, SQUARE, Tiles};
use valle_engine::style::Blend;

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

fn rect(x: f32, y: f32, width: f32, height: f32, color: [f32; 4]) -> Item {
    Item::Rect {
        rect: Rect {
            x,
            y,
            width,
            height,
        },
        radii: SQUARE,
        color,
    }
}

fn pixel(p: &[u8], width: usize, x: usize, y: usize) -> &[u8] {
    &p[(y * width + x) * 4..(y * width + x + 1) * 4]
}

#[test]
fn delivery_limits_include_portrait_and_reject_more_than_8k() {
    for size in [[7680, 4320], [4320, 7680], [5760, 5760], [1, 1]] {
        assert!(output_size(size).is_ok(), "{size:?}");
    }
    for size in [[0, 1080], [7681, 1], [7680, 4321], [u32::MAX, u32::MAX]] {
        assert!(output_size(size).is_err(), "{size:?}");
    }
}

#[test]
fn output_resolves_details_that_do_not_fit_in_a_design_pixel() {
    let Some(gpu) = gpu() else { return };
    let lines: Vec<_> = (0..16)
        .map(|i| rect(i as f32 / 4.0, 0.0, 0.125, 4.0, [1.0; 4]))
        .collect();
    let mut renderer = Renderer::at_size(&gpu, [4, 4], [32, 32]).unwrap();
    let pixels = renderer.render(&lines, &[], &[], &[]).unwrap();
    for x in 0..32 {
        assert_eq!(
            pixel(&pixels, 32, x, 16)[3],
            if x % 2 == 0 { 255 } else { 0 }
        );
    }
}

#[test]
fn scaling_preserves_group_coverage_and_clips_letterbox() {
    let Some(gpu) = gpu() else { return };
    // Content deliberately extends past the design canvas; it must not paint the letterbox.
    let items = [Item::Group {
        opacity: 0.5,
        blend: Blend::Normal,
        items: vec![rect(-100.0, -100.0, 300.0, 300.0, [1.0, 0.0, 0.0, 1.0])],
    }];
    for size in [[16, 8], [64, 32], [64, 64], [16, 16]] {
        let mut renderer = Renderer::at_size(&gpu, [32, 16], size).unwrap();
        let p = renderer.render(&items, &[], &[], &[]).unwrap();
        let [w, h] = size.map(|v| v as usize);
        let offset = (h - w / 2) / 2;
        for y in 0..h {
            for x in 0..w {
                let a = pixel(&p, w, x, y)[3];
                assert_eq!(
                    a,
                    if (offset..h - offset).contains(&y) {
                        128
                    } else {
                        0
                    },
                    "{size:?}, {x},{y}"
                );
            }
        }
    }
}

#[test]
fn original_size_keeps_existing_pixels_and_filter_passes() {
    let Some(gpu) = gpu() else { return };
    let items = [Item::Filter {
        filters: vec![Filter::Blur(1.25)],
        items: vec![rect(6.5, 7.25, 14.0, 9.0, [0.2, 0.6, 1.0, 0.7])],
    }];
    let mut old = Renderer::new(&gpu, 32, 32);
    let mut sized = Renderer::at_size(&gpu, [32, 32], [32, 32]).unwrap();
    assert_eq!(
        old.render(&items, &[], &[], &[]).unwrap(),
        sized.render(&items, &[], &[], &[]).unwrap()
    );
}

#[test]
fn drawn_inputs_scale_without_baking_in_letterbox() {
    let Some(gpu) = gpu() else { return };
    let bounds = Rect {
        x: 0.0,
        y: 0.0,
        width: 32.0,
        height: 16.0,
    };
    let source = vec![
        rect(0.0, 0.0, 16.0, 16.0, [1.0, 0.0, 0.0, 1.0]),
        rect(16.0, 0.0, 16.0, 16.0, [0.0, 1.0, 0.0, 1.0]),
    ];
    let image = Image::new([32, 16], std::sync::Arc::from([]));
    let items = [Item::Image {
        rect: bounds,
        radii: SQUARE,
        tiles: Tiles {
            first: bounds,
            step: [None; 2],
        },
        image: 0,
        pixelated: false,
    }];
    for size in [[64, 32], [64, 64], [16, 16], [46, 30]] {
        let mut renderer = Renderer::at_size(&gpu, [32, 16], size).unwrap();
        let p = renderer
            .render(
                &items,
                &[(0, source.clone())],
                &[],
                std::slice::from_ref(&image),
            )
            .unwrap();
        let [w, h] = size.map(|v| v as usize);
        assert_eq!(pixel(&p, w, w / 4, h / 2), [255, 0, 0, 255]);
        assert_eq!(pixel(&p, w, 3 * w / 4, h / 2), [0, 255, 0, 255]);
        if h > w / 2 {
            assert_eq!(pixel(&p, w, w / 2, 0)[3], 0);
        }
    }
}
