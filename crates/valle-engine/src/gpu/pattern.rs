//! Visible instances of oversized pattern tiles, without a tile-sized texture allocation.
use std::ops::Range;

use crate::layout::Rect;
use crate::paint::{Item, Matrix};

pub(super) fn repeats(
    matrix: Matrix,
    [x, y, w, h]: [f64; 4],
    cover: Rect,
    size: [u32; 2],
) -> Option<[Range<i64>; 2]> {
    let left = cover.x.max(0.);
    let top = cover.y.max(0.);
    let right = (cover.x + cover.width).min(size[0] as f32);
    let bottom = (cover.y + cover.height).min(size[1] as f32);
    if right <= left || bottom <= top {
        return Some([0..0, 0..0]);
    }
    let local = matrix.inverse()?.bounds(Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    });
    let from = [
        ((f64::from(local.x) - x) / w).floor(),
        ((f64::from(local.y) - y) / h).floor(),
    ];
    let to = [
        ((f64::from(local.x + local.width) - x) / w).ceil(),
        ((f64::from(local.y + local.height) - y) / h).ceil(),
    ];
    // More instances than output pixels cannot resolve as separate tiles. Retain the
    // sampled periodic representation for this subpixel case rather than expanding it.
    if (to[0] - from[0]) * (to[1] - from[1]) > f64::from(size[0]) * f64::from(size[1]) {
        return None;
    }
    Some([from[0] as i64..to[0] as i64, from[1] as i64..to[1] as i64])
}

/// Avoid clipping coverage a second time when all vector geometry already lies in its tile.
pub(super) fn contained(items: &[Item], size: [f64; 2], matrix: Matrix) -> bool {
    let inside = |r: Rect| {
        let r = matrix.bounds(r);
        r.x >= 0.
            && r.y >= 0.
            && f64::from(r.x + r.width) <= size[0]
            && f64::from(r.y + r.height) <= size[1]
    };
    items.iter().all(|item| match item {
        Item::Rect { rect, .. }
        | Item::Ring { rect, .. }
        | Item::Gradient { rect, .. }
        | Item::Image { rect, .. }
        | Item::Clip { rect, .. } => inside(*rect),
        Item::Path { path, .. } => {
            let [x, y, width, height] = crate::svg::bounds(path).map(|v| v as f32);
            inside(Rect {
                x,
                y,
                width,
                height,
            })
        }
        Item::Transform {
            matrix: local,
            items,
        } => contained(items, size, matrix.times(*local)),
        Item::Group { items, .. } => contained(items, size, matrix),
        Item::Mask { mask, .. } => contained(mask, size, matrix),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::{Gpu, Renderer};
    use crate::paint::{Item, SQUARE};

    #[test]
    fn visible_tiles_follow_offsets_transforms_and_output_resolution() {
        let cover = Rect {
            x: 0.,
            y: 0.,
            width: 128.,
            height: 64.,
        };
        assert_eq!(
            repeats(Matrix::IDENTITY, [0., 0., 8192., 8192.], cover, [128, 64]),
            Some([0..1, 0..1])
        );
        assert_eq!(
            repeats(Matrix::IDENTITY, [-64., -16., 8192., 32.], cover, [128, 64]),
            Some([0..1, 0..3])
        );
        assert!(
            repeats(
                Matrix::rotate(15.),
                [0., 0., 8192., 8192.],
                cover,
                [128, 64]
            )
            .is_some()
        );
        assert!(repeats(Matrix::IDENTITY, [0., 0., 8192., 1e-6], cover, [128, 64]).is_none());
        assert_eq!(
            repeats(
                Matrix::IDENTITY,
                [0., 0., 8192., 8192.],
                Rect { x: 500., ..cover },
                [128, 64]
            ),
            Some([0..0, 0..0])
        );
    }

    #[test]
    fn oversized_tiles_preserve_fine_edges_at_4k_and_fractional_seams() {
        if !std::env::var("VALLE_TEST_LAYERS")
            .unwrap_or_default()
            .split(',')
            .any(|v| v == "gpu")
        {
            return;
        }
        let gpu = valle_native::gpu::adapter(None)
            .and_then(|a| Gpu::with(a.adapter))
            .unwrap();
        for (width, height, scale) in [(128, 64, 1.), (3840, 2160, 4.)] {
            let mut renderer = Renderer::new(&gpu, width, height);
            let line = Item::Rect {
                rect: Rect {
                    x: 20.,
                    y: 10.,
                    width: 1.,
                    height: 40.,
                },
                radii: SQUARE,
                color: [0.1, 0.4, 0.7, 1.],
            };
            let pattern = Item::Pattern {
                rect: Rect {
                    x: 0.,
                    y: 0.,
                    width: width as f32 / scale,
                    height: height as f32 / scale,
                },
                matrix: Matrix::IDENTITY,
                region: [0., 0., 8192., 8192.],
                opacity: 1.,
                items: vec![line.clone()],
            };
            let transform = |item| Item::Transform {
                matrix: Matrix::affine([scale as f64, 0., 0., scale as f64, 0., 0.]),
                items: vec![item],
            };
            let actual = renderer
                .render(&[transform(pattern)], &[], &[], &[])
                .unwrap();
            let expected = renderer.render(&[transform(line)], &[], &[], &[]).unwrap();
            assert!(
                actual == expected,
                "{width}x{height}: {} channels differ",
                actual.iter().zip(&expected).filter(|(a, b)| a != b).count()
            );
        }
        let mut renderer = Renderer::new(&gpu, 128, 64);
        let line = Item::Rect {
            rect: Rect {
                x: 20.,
                y: 10.,
                width: 1.,
                height: 40.,
            },
            radii: SQUARE,
            color: [0.1, 0.4, 0.7, 1.],
        };
        let matrix = Matrix::translate(10., 0.).times(Matrix::rotate(12.));
        let pattern = Item::Pattern {
            rect: Rect {
                x: 0.,
                y: 0.,
                width: 128.,
                height: 64.,
            },
            matrix,
            region: [0., 0., 8192., 8192.],
            opacity: 1.,
            items: vec![line.clone()],
        };
        let actual = renderer.render(&[pattern], &[], &[], &[]).unwrap();
        let expected = renderer
            .render(
                &[Item::Transform {
                    matrix,
                    items: vec![line],
                }],
                &[],
                &[],
                &[],
            )
            .unwrap();
        assert!(
            actual == expected,
            "transformed tile differs from direct vector geometry"
        );

        let color = [0.1, 0.4, 0.7, 1.];
        let pattern = Item::Pattern {
            rect: Rect {
                x: 0.,
                y: 0.,
                width: 128.,
                height: 64.,
            },
            matrix: Matrix::IDENTITY,
            region: [-12.5, -2.5, 8192., 10.5],
            opacity: 0.5,
            items: vec![Item::Rect {
                rect: Rect {
                    x: 0.,
                    y: 0.,
                    width: 8192.,
                    height: 10.5,
                },
                radii: SQUARE,
                color,
            }],
        };
        let pixels = renderer.render(&[pattern], &[], &[], &[]).unwrap();
        for p in pixels.as_chunks::<4>().0 {
            assert!(p[3].abs_diff(128) <= 1, "seam pixel {p:?}");
        }
    }
}
