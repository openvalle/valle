//! Gradient fills on Raster: on aarch64, `fill_rect_inside` computes the pixels that the
//! anti-aliased edges of a gradient-filled rectangle leave fully covered, with the instructions
//! of Skia's raster pipeline, and leaves the rest of the draw to Skia.

use skia_safe::{Canvas, Matrix, Paint as SkPaint, Path as SkPath, Point};
use valle_draw::program::{Paint, PathData};

/// The points of a linear gradient's shader.
pub(super) fn linear_points(start: [f64; 2], end: [f64; 2]) -> (Point, Point) {
    (
        Point::new(start[0] as f32, start[1] as f32),
        Point::new(end[0] as f32, end[1] as f32),
    )
}

/// The center, radius and local matrix of a radial gradient's shader: a circle of the larger
/// radius, scaled to the radii about the center.
pub(super) fn radial_geometry(center: [f64; 2], radii: [f64; 2]) -> (Point, f32, Matrix) {
    let center = Point::new(center[0] as f32, center[1] as f32);
    let radius = radii[0].max(radii[1]) as f32;
    let mut matrix = Matrix::new_identity();
    matrix.pre_scale(
        (radii[0] as f32 / radius, radii[1] as f32 / radius),
        Some(center),
    );
    (center, radius, matrix)
}

/// The center, the start and end angles and the local matrix of a conic gradient's sweep shader.
/// Skia measures sweep angles in [0, 360) from the positive x axis, so the start angle rotates
/// the shader: as a sweep start it would clamp the part of the turn before it to an end color.
pub(super) fn conic_geometry(
    center: [f64; 2],
    start: f64,
    sweep: f64,
) -> (Point, (f32, f32), Matrix) {
    let center = Point::new(center[0] as f32, center[1] as f32);
    let local = Matrix::rotate_deg_pivot(start as f32, center);
    (center, (0.0, sweep as f32), local)
}

/// Draws `path` with `paint`, which fills it with the shader of `gradient`, as Skia's draw of it
/// writes it, computing the pixels its anti-aliased edges leave fully covered itself when the
/// path is a rectangle. Returns `false`, drawing nothing, where Skia's draw stays.
#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
pub(super) fn fill_rect_inside(
    canvas: &Canvas,
    data: &PathData,
    path: &SkPath,
    paint: &SkPaint,
    gradient: &Paint,
) -> bool {
    fused::fill_rect_inside(canvas, data, path, paint, gradient)
}

/// The pixels repeat the instructions of Skia's raster pipeline on aarch64; elsewhere Skia's draw
/// stays.
#[cfg(not(all(target_arch = "aarch64", target_endian = "little")))]
pub(super) fn fill_rect_inside(
    _canvas: &Canvas,
    _data: &PathData,
    _path: &SkPath,
    _paint: &SkPaint,
    _gradient: &Paint,
) -> bool {
    false
}

#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
pub(super) use fused::subtracts_exactly;

#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
mod fused {
    use std::arch::aarch64::*;

    use skia_safe::{
        AlphaType, BlendMode, Canvas, ClipOp, Color4f, ColorType, IRect, M44, Matrix,
        Paint as SkPaint, PaintStyle, Path as SkPath, Point, Rect, Region, TileMode,
        matrix::TypeMask, region, surface::ContentChangeMode,
    };
    use valle_draw::program::{Paint, PathData, PathVerb};
    use valle_engine::compositor::bloom::fill_image_rows;

    use super::super::{
        draw::{gradient_parts, tile_mode},
        surface::working_color_space,
    };

    /// Pixels per vector.
    const LANES: usize = 4;

    /// The centers of four neighbouring pixels from the first one's column.
    const CENTERS: [f32; LANES] = [0.5, 1.5, 2.5, 3.5];

    /// Skia's `kDegenerateThreshold`: shorter gradients draw a color instead.
    const DEGENERATE: f32 = 1.0 / 32_768.0;

    pub(super) fn fill_rect_inside(
        canvas: &Canvas,
        data: &PathData,
        path: &SkPath,
        paint: &SkPaint,
        gradient: &Paint,
    ) -> bool {
        use PathVerb::{Close, LineTo, MoveTo};
        // Skia draws on devices wider or taller than this in tiles.
        const UNTILED: i32 = 8191;
        // Smaller fills cost more in the clip and the extra pass than they save.
        const MIN_PIXELS: i64 = 16_384;
        // Four corners give the rectangle one edge on each side, and the paint draws the shader
        // as it is over the pixels it covers fully.
        if data.verbs.as_slice() != [MoveTo, LineTo, LineTo, LineTo, Close]
            || data.points.len() != 4
            || paint.alpha_f() != 1.0
            || paint.as_blend_mode() != Some(BlendMode::SrcOver)
            || paint.style() != PaintStyle::Fill
            || !paint.is_anti_alias()
            || paint.color_filter().is_some()
            || paint.mask_filter().is_some()
            || paint.image_filter().is_some()
            || paint.path_effect().is_some()
        {
            return false;
        }
        let Some((rect, ..)) = path.is_rect() else {
            return false;
        };
        let Ok(space) = working_color_space() else {
            return false;
        };
        // The pixels of the canvas's top device, in the space the colors are given in, where the
        // shader looks them up as they are, and where the device starts on the canvas. The stores
        // write the halves of whole pixels.
        let Some(origin) = canvas.access_top_layer_pixels().and_then(|layer| {
            (layer.info.color_type() == ColorType::RGBAF16
                && layer.info.alpha_type() == AlphaType::Premul
                && layer.info.color_space().as_ref() == Some(&space)
                && layer.info.width() <= UNTILED
                && layer.info.height() <= UNTILED
                && layer.pixels.as_ptr().cast::<u16>().is_aligned()
                && layer.row_bytes.is_multiple_of(8))
            .then_some(layer.origin)
        }) else {
            return false;
        };
        // When the transform changes, the device maps it to its pixels as the product of a
        // translation by the negated origin with it. A layer pushed without filters starts
        // inside its parent's clip, so the origins of its parents lie between the canvas's and
        // its own, and until then its device keeps its parent's mapping less the step between
        // their origins. Where the transform's translation less every whole number of pixels up
        // to the origin is a float, each of those subtractions is exact, and the mappings agree.
        let global = canvas.local_to_device();
        if global.rc(3, 3) != 1.0
            || !canvas.is_clip_rect()
            || !subtracts_exactly(global.rc(0, 3), origin.x)
            || !subtracts_exactly(global.rc(1, 3), origin.y)
        {
            return false;
        }
        let device = M44::concat(
            &M44::translate(-origin.x as f32, -origin.y as f32, 0.0),
            &global,
        )
        .to_m33();
        // The canvas gives the clip's bounds in its own coordinates.
        let Some(clip) = canvas
            .device_clip_bounds()
            .map(|clip| clip.with_offset((-origin.x, -origin.y)))
        else {
            return false;
        };
        let Some(shading) = Shading::new(gradient, &device) else {
            return false;
        };
        // Skia stores opaque colors as they are inside a run of pixels it fills whole, but blends
        // them in with full coverage in a column where a clip cuts such a run short on one side,
        // which rounds differently. Upright sides, and turned ones its fixed point makes upright,
        // end their runs where the fill can tell, and it writes those columns as Skia's draw of
        // the whole does; otherwise only runs across the whole clip keep their cuts under the
        // hole. Translucent colors blend alike either way.
        let plan = if !shading.opaque {
            inside(rect, &device, clip).map(Plan::from)
        } else if device.is_scale_translate() {
            upright(rect, &device, clip)
        } else {
            rows_across(rect, &device, clip)
                .map(Plan::from)
                .or_else(|| nearly_upright(rect, &device, clip))
        };
        let Some(Plan {
            hole,
            blend,
            restore,
        }) = plan
        else {
            return false;
        };
        let rects = region::Iterator::new(&hole).collect::<Vec<_>>();
        if rects
            .iter()
            .map(|rect| i64::from(rect.width()) * i64::from(rect.height()))
            .sum::<i64>()
            < MIN_PIXELS
        {
            return false;
        }
        // As before Skia's own draw, pixels a snapshot still shares are copied before they change.
        // SAFETY: the surface handle is dropped here, while the canvas it owns is borrowed.
        if let Some(mut surface) = unsafe { canvas.surface() } {
            surface.notify_content_will_change(ContentChangeMode::Retain);
        }
        let Some(layer) = canvas.access_top_layer_pixels() else {
            return false;
        };
        let row_bytes = layer.row_bytes;
        let bounds = *hole.bounds();
        let span = bounds.top as usize * row_bytes
            ..(bounds.bottom as usize * row_bytes).min(layer.pixels.len());
        fill_image_rows(&mut layer.pixels[span], row_bytes, |first, rows| {
            for (index, row) in rows.chunks_mut(row_bytes).enumerate() {
                let y = bounds.top + (first + index) as i32;
                // The region's rectangles come in bands of rows, each band sorted by column.
                let band = rects.partition_point(|rect| rect.bottom <= y);
                for rect in rects[band..].iter().take_while(|rect| rect.top <= y) {
                    let (mut left, mut right) = (rect.left, rect.right);
                    if blend == Some(left) {
                        shading.blend(y, left, &mut row[left as usize * 8..][..8]);
                        left += 1;
                    } else if blend == Some(right - 1) {
                        right -= 1;
                        shading.blend(y, right, &mut row[right as usize * 8..][..8]);
                    }
                    shading.row(y, left, &mut row[left as usize * 8..right as usize * 8]);
                }
            }
        });
        // The canvas takes the region in its own coordinates too.
        canvas.save();
        canvas.clip_region(&hole.translated((origin.x, origin.y)), ClipOp::Difference);
        canvas.draw_path(path, paint);
        canvas.restore();
        if !restore.is_empty()
            && let Some(layer) = canvas.access_top_layer_pixels()
        {
            for y in bounds.top..bounds.bottom {
                for &x in &restore {
                    let at = y as usize * row_bytes + x as usize * 8;
                    shading.row(y, x, &mut layer.pixels[at..at + 8]);
                }
            }
        }
        true
    }

    /// Whether `value` less every whole number from 0 to `count` is a float, so that taking
    /// such numbers from it one after another rounds nowhere.
    pub(in super::super) fn subtracts_exactly(value: f32, count: i32) -> bool {
        // The differences are whole multiples of the value's lowest bit or of 1, whichever is
        // smaller, and floats hold the multiples below 2^24 of a power of two.
        let step = if value == 0.0 {
            1.0
        } else {
            let bits = value.abs().to_bits();
            let exponent = (bits >> 23) as i32;
            let significand = if exponent == 0 {
                bits
            } else {
                bits & 0x7f_ffff | 0x80_0000
            };
            2_f64
                .powi(exponent.max(1) - 150 + significand.trailing_zeros() as i32)
                .min(1.0)
        };
        let limit = step * 16_777_216.0;
        count >= 0
            && value.is_finite()
            && f64::from(value).abs() < limit
            && (f64::from(value) - f64::from(count)).abs() < limit
    }

    /// The pixels the fill computes, and the columns beside them where Skia's draw of the rest
    /// writes other bytes than its draw of the whole.
    struct Plan {
        /// The pixels the fill computes; Skia draws the rest.
        hole: Region,
        /// A column of the hole where Skia's draw of the whole blends the colors in with full
        /// coverage.
        blend: Option<i32>,
        /// Columns beside the hole, in its rows, where Skia's draw of the whole stores the colors
        /// but its draw of the rest blends them in with full coverage, ending runs there.
        restore: Vec<i32>,
    }

    impl From<Region> for Plan {
        fn from(hole: Region) -> Plan {
            Plan {
                hole,
                blend: None,
                restore: Vec::new(),
            }
        }
    }

    /// Where a left or right side of an upright rectangle lies against the clip's.
    enum Side {
        /// Past the clip's side, which cuts the rows short there.
        Past,
        /// On the clip's side.
        On,
        /// Inside it by a margin, with the column where the hole starts or ends on that side,
        /// a pixel inside the first or last one Skia's draw of the whole stores in its rows.
        In(i32),
    }

    /// The plan for the opaque fill of `rect` under `matrix`, which only scales and translates
    /// it, in `clip`.
    fn upright(rect: Rect, matrix: &Matrix, clip: IRect) -> Option<Plan> {
        // The sides as Skia maps the bounds of the rectangle's points, and the points themselves
        // alike.
        let (mapped, _) = matrix.map_rect(rect);
        if !mapped.is_finite() {
            return None;
        }
        sides_plan(
            [mapped.left, mapped.top, mapped.right, mapped.bottom].map(f64::from),
            clip,
        )
    }

    /// The plan for the opaque fill of `rect` under `matrix`, which turns or skews it, in `clip`,
    /// where Skia takes the rectangle for an upright one: its top and bottom sides lie wholly
    /// above and below the clip, and its fixed point makes the left and right edges upright.
    fn nearly_upright(rect: Rect, matrix: &Matrix, clip: IRect) -> Option<Plan> {
        if matrix.has_perspective() {
            return None;
        }
        // The corners as Skia maps the path's points, a point at a time, down each side.
        let mut corners = [Point::default(); 4];
        matrix.map_points(
            &mut corners,
            &[
                Point::new(rect.left, rect.top),
                Point::new(rect.left, rect.bottom),
                Point::new(rect.right, rect.top),
                Point::new(rect.right, rect.bottom),
            ],
        );
        let (top, bottom) = (clip.top as f32, clip.bottom as f32);
        let above = corners.map(|corner| corner.y < top);
        // Skia's line clipper drops the top and bottom sides, and cuts the others at the clip's
        // top and bottom.
        if !corners
            .iter()
            .all(|corner| corner.is_finite() && (corner.y < top || corner.y > bottom))
            || above[0] == above[1]
            || above[0] != above[2]
            || above[1] != above[3]
        {
            return None;
        }
        let first = column([corners[0], corners[1]], clip)?;
        let second = column([corners[2], corners[3]], clip)?;
        sides_plan(
            [
                first.min(second),
                f64::NEG_INFINITY,
                first.max(second),
                f64::INFINITY,
            ],
            clip,
        )
    }

    /// Where Skia's edge along a side of a rectangle from `ends`, one above `clip` and one below
    /// it, runs over the clip's rows: past the clip's left or right side, as an infinity, where
    /// its line clipper moves the edge onto that side, or inside the clip in a column of 256ths of
    /// a pixel, where its fixed point makes the edge upright.
    fn column(ends: [Point; 2], clip: IRect) -> Option<f64> {
        let (left, right) = (clip.left as f32, clip.right as f32);
        // The clipper's cut at a row, `sect_with_horizontal`, from either end of the edge.
        let cut = |[start, end]: [Point; 2], y: i32| {
            let [x0, y0, x1, y1] = [start.x, start.y, end.x, end.y].map(f64::from);
            (x0 + (f64::from(y) - y0) * (x1 - x0) / (y1 - y0)).clamp(x0.min(x1), x0.max(x1)) as f32
        };
        let mut found = None;
        for ends in [ends, [ends[1], ends[0]]] {
            let [upper, lower] = [clip.top, clip.bottom].map(|y| cut(ends, y));
            let column = if upper.max(lower) <= left {
                f64::NEG_INFINITY
            } else if upper.min(lower) >= right {
                f64::INFINITY
            } else if upper.min(lower) > left && upper.max(lower) < right {
                // `SkAnalyticEdge::setLine` truncates the ends to 256ths of a pixel, and divides
                // the step between them, in 64ths, by the rows', truncating the slope in 16.16
                // fixed point toward 0; the edge then keeps the column of its upper end.
                let [from, to] = [upper, lower].map(|x| (x * 256.0) as i32);
                let rows = i64::from(clip.bottom - clip.top) * 64;
                if i64::from((to - from) >> 2).abs() * 65_536 >= rows {
                    return None;
                }
                f64::from(from) / 256.0
            } else {
                return None;
            };
            if found.is_some_and(|found| found != column) {
                return None;
            }
            found = Some(column);
        }
        found
    }

    /// The plan for the opaque fill of an upright rectangle with sides `[left, top, right,
    /// bottom]`, as Skia maps them, in `clip`.
    fn sides_plan([left, top, right, bottom]: [f64; 4], clip: IRect) -> Option<Plan> {
        // Skia's analytic coverage truncates columns to a 256th of a pixel, so a side this far
        // inside the clip's stays inside it.
        const EDGE: f64 = 1.0 / 128.0;
        // Past this distance the top and bottom sides leave the rows covered fully, including
        // the snap of their rows to a quarter of a pixel.
        const NEAR: f64 = 0.25;
        let (clip_left, clip_right) = (f64::from(clip.left), f64::from(clip.right));
        let first = if left < clip_left {
            Side::Past
        } else if left == clip_left {
            Side::On
        } else if left >= clip_left + EDGE && left < clip_right {
            Side::In((left + EDGE).ceil() as i32 + 1)
        } else {
            return None;
        };
        let last = if right > clip_right {
            Side::Past
        } else if right == clip_right {
            Side::On
        } else if right <= clip_right - EDGE && right > clip_left {
            Side::In((right - EDGE).floor() as i32 - 1)
        } else {
            return None;
        };
        // In each row it covers fully, Skia's draw of the whole stores the colors from the first
        // pixel an inner side leaves whole, or from the clip's side, to the other side's. Where
        // the clip cuts the rectangle short on one side only, it blends them in with full
        // coverage in the column there instead, which the hole takes in to blend alike. Under the
        // hole, the draw of the rest blends them in the same way in the columns where the hole
        // cuts its rows short beside an inner side, which the fill stores again afterwards, and
        // in all rows beside a side on the clip's, where the draw of the whole cuts nothing.
        let (columns, blend, restore) = match (first, last) {
            (Side::Past | Side::On, Side::Past | Side::On) => {
                ((clip.left + 1, clip.right - 1), None, Vec::new())
            }
            (Side::Past, Side::In(to)) => ((clip.left, to), Some(clip.left), vec![to]),
            (Side::In(from), Side::Past) => {
                ((from, clip.right), Some(clip.right - 1), vec![from - 1])
            }
            (Side::In(from), Side::In(to)) => ((from, to), None, vec![from - 1, to]),
            (Side::On, Side::In(_)) | (Side::In(_), Side::On) => return None,
        };
        // Rows inside the clip, inset by a pixel, so the clip Skia draws the rest under keeps its
        // bounds.
        let rows = (
            (top + NEAR).ceil().max(f64::from(clip.top + 1)),
            (bottom - NEAR).floor().min(f64::from(clip.bottom - 1)),
        );
        (columns.0 < columns.1 && rows.0 < rows.1).then(|| Plan {
            hole: Region::from_rect(IRect::from_ltrb(
                columns.0,
                rows.0 as i32,
                columns.1,
                rows.1 as i32,
            )),
            blend,
            restore,
        })
    }

    /// The device corners of `rect` under `matrix`, in order around it, where a float's step
    /// stays far below a pixel.
    fn corners(rect: Rect, matrix: &Matrix) -> Option<[[f64; 2]; 4]> {
        const REACH: f64 = 1_048_576.0;
        let mut values = [0.0_f32; 9];
        matrix.get_9(&mut values);
        let m = values.map(f64::from);
        let map = |x: f32, y: f32| {
            let (x, y) = (f64::from(x), f64::from(y));
            [m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5]]
        };
        let corners = [
            map(rect.left, rect.top),
            map(rect.right, rect.top),
            map(rect.right, rect.bottom),
            map(rect.left, rect.bottom),
        ];
        corners
            .as_flattened()
            .iter()
            .all(|value| value.abs() < REACH)
            .then_some(corners)
    }

    /// A bound on how far the columns of the corners of `rect` under `matrix`, as Skia maps them
    /// with floats, stray from the exact ones: a few steps of a float as large as their terms.
    fn slack(rect: Rect, matrix: &Matrix) -> f64 {
        let [scale, skew, translate] = [matrix.scale_x(), matrix.skew_x(), matrix.translate_x()]
            .map(|value| f64::from(value.abs()));
        let (x, y) = (
            rect.left.abs().max(rect.right.abs()),
            rect.top.abs().max(rect.bottom.abs()),
        );
        (f64::from(x) * scale + f64::from(y) * skew + translate) / 1_048_576.0
    }

    /// The clip inset by a pixel, so the clip Skia draws the rest under keeps its bounds.
    fn inset(clip: IRect) -> Option<IRect> {
        let bounds = IRect::from_ltrb(clip.left + 1, clip.top + 1, clip.right - 1, clip.bottom - 1);
        (!bounds.is_empty()).then_some(bounds)
    }

    /// The rows of `clip`, inset by a pixel, that the device quad of `rect` under `matrix` covers
    /// fully from the clip's left side to its right one: each edge lies past them by a margin or
    /// wholly past a side of the clip, facing away from it.
    fn rows_across(rect: Rect, matrix: &Matrix, clip: IRect) -> Option<Region> {
        // Past this distance an edge leaves the rows covered fully, whatever rounding it takes,
        // including the snap of its ends to a quarter of a row.
        const NEAR: f64 = 0.25;
        let corners = corners(rect, matrix)?;
        let bounds = inset(clip)?;
        let center = [
            corners.iter().map(|corner| corner[0]).sum::<f64>() / 4.0,
            corners.iter().map(|corner| corner[1]).sum::<f64>() / 4.0,
        ];
        let (left, right) = (f64::from(clip.left), f64::from(clip.right));
        let slack = slack(rect, matrix);
        let (mut top, mut bottom) = (f64::from(bounds.top), f64::from(bounds.bottom));
        for (index, start) in corners.iter().enumerate() {
            let end = corners[(index + 1) % 4];
            // Skia's line clipper moves an edge whose ends both lie past the clip's left or right
            // side onto that side, where the runs end as they do at the clip. The strip beyond
            // it, along the edges beside it, lies past the side too where the edge faces away
            // from the clip, so the other edges keep the quad over the rows up to there.
            let outward = start[0] - corners[(index + 3) % 4][0];
            if (start[0].max(end[0]) <= left - slack && outward <= 0.0)
                || (start[0].min(end[0]) >= right + slack && outward >= 0.0)
            {
                continue;
            }
            let mut normal = [end[1] - start[1], start[0] - end[0]];
            let length = normal[0].hypot(normal[1]);
            if length.is_nan() || length <= 0.0 {
                return None;
            }
            let mut offset = normal[0] * start[0] + normal[1] * start[1];
            if normal[0] * center[0] + normal[1] * center[1] < offset {
                normal = [-normal[0], -normal[1]];
                offset = -offset;
            }
            // Both ends of a row lie on the quad's side of the edge, past the margin:
            // normal . (x, y) >= offset + NEAR * length.
            for x in [left, right] {
                let rest = offset + NEAR * length - normal[0] * x;
                if normal[1] > 0.0 {
                    top = top.max(rest / normal[1]);
                } else if normal[1] < 0.0 {
                    bottom = bottom.min(rest / normal[1]);
                } else if rest > 0.0 {
                    return None;
                }
            }
        }
        let (top, bottom) = (top.ceil(), bottom.floor());
        (top < bottom).then(|| {
            Region::from_rect(IRect::from_ltrb(
                bounds.left,
                top as i32,
                bounds.right,
                bottom as i32,
            ))
        })
    }

    /// The pixels of `clip`, inset by a pixel, whose centers lie well inside the device quad of
    /// `rect` under `matrix`.
    fn inside(rect: Rect, matrix: &Matrix, clip: IRect) -> Option<Region> {
        // A pixel whose center lies this far inside the edges is covered fully, whatever rounding
        // the edges take on their way to Skia's fixed point and the region's scan conversion
        // takes.
        const MARGIN: f64 = 2.0;
        let corners = corners(rect, matrix)?;
        let origin = corners[0];
        let across = [corners[1][0] - origin[0], corners[1][1] - origin[1]];
        let down = [corners[3][0] - origin[0], corners[3][1] - origin[1]];
        let area = (across[0] * down[1] - across[1] * down[0]).abs();
        // The margin as fractions of the distances between opposite edges.
        let u = MARGIN * down[0].hypot(down[1]) / area;
        let v = MARGIN * across[0].hypot(across[1]) / area;
        if u.is_nan() || v.is_nan() || u.max(v) >= 0.5 {
            return None;
        }
        let point = |a: f64, b: f64| {
            Point::new(
                (origin[0] + a * across[0] + b * down[0]) as f32,
                (origin[1] + a * across[1] + b * down[1]) as f32,
            )
        };
        let quad = SkPath::polygon(
            &[
                point(u, v),
                point(1.0 - u, v),
                point(1.0 - u, 1.0 - v),
                point(u, 1.0 - v),
            ],
            true,
            None,
            None,
        );
        let bounds = inset(clip)?;
        let mut hole = Region::new();
        hole.set_path(&quad, &Region::from_rect(bounds))
            .then_some(hole)
    }

    /// The stages of a gradient shader's pipeline on Skia's Raster device, from device pixel
    /// centers to the premultiplied color, and the blend that writes it.
    struct Shading {
        /// Device coordinates to the gradient's unit space: the shader's points to their unit
        /// square after the inverse of the transform and local matrix.
        unit: Stage,
        kind: Kind,
        tile: Tile,
        ramp: Ramp,
        /// Opaque colors replace the pixels under them; others blend over them.
        opaque: bool,
    }

    /// A matrix stage of Skia's raster pipeline, as `SkRasterPipeline::appendMatrix` picks it.
    #[derive(Clone, Copy)]
    enum Stage {
        Identity,
        Translate([f32; 2]),
        ScaleTranslate([f32; 4]),
        Affine([f32; 6]),
    }

    /// The stage from unit coordinates to the gradient's `t`.
    enum Kind {
        Linear,
        /// `xy_to_radius`.
        Radial,
        /// `xy_to_unit_angle`, then the matrix from the full turn to the sweep's angles.
        Sweep(Stage),
    }

    /// The tile stage on `t`.
    enum Tile {
        None,
        Clamp,
        Repeat,
        Mirror,
    }

    /// Skia's lookup of a gradient's colors: the stops `SkGradientBaseShader` keeps of the
    /// colors and positions, premultiplied, with the factor and bias of each that
    /// `AppendGradientFillStages` computes.
    enum Ramp {
        /// `evenly_spaced_2_stop_gradient`.
        Two([f32; 8]),
        /// `evenly_spaced_gradient`.
        Even(Vec<[f32; 8]>),
        /// `gradient`: where each stop after the first starts, padded with NaN, which no `t`
        /// reaches, to a power of two above their count.
        Stops {
            starts: Vec<f32>,
            records: Vec<[f32; 8]>,
        },
    }

    impl Shading {
        /// The pipeline of `gradient`'s shader, as Skia builds it, under the device transform
        /// `device`; `None` where Skia draws a color or nothing instead, or runs other stages.
        fn new(gradient: &Paint, device: &Matrix) -> Option<Shading> {
            let identity = Matrix::new_identity();
            let (stops, spread, to_unit, local, kind, whole_turn) = match gradient {
                Paint::LinearGradient {
                    start,
                    end,
                    stops,
                    spread,
                } => {
                    let (start, end) = super::linear_points(*start, *end);
                    let length = (end - start).length();
                    if !length.is_finite() || length <= DEGENERATE {
                        return None;
                    }
                    // `pts_to_unit_matrix`.
                    let mut direction = end - start;
                    let inverse = if length != 0.0 { 1.0 / length } else { 0.0 };
                    direction.scale(inverse);
                    let mut to_unit = Matrix::new_identity();
                    to_unit.set_sin_cos((-direction.y, direction.x), start);
                    to_unit.post_translate((-start.x, -start.y));
                    to_unit.post_scale((inverse, inverse), None);
                    (stops, spread, to_unit, identity, Kind::Linear, false)
                }
                Paint::RadialGradient {
                    center,
                    radii,
                    stops,
                    spread,
                } => {
                    let (center, radius, local) = super::radial_geometry(*center, *radii);
                    if !radius.is_finite() || radius <= DEGENERATE || local.invert().is_none() {
                        return None;
                    }
                    // `rad_to_unit_matrix`.
                    let inverse = 1.0 / radius;
                    let mut to_unit = Matrix::new_identity();
                    to_unit.set_translate((-center.x, -center.y));
                    to_unit.post_scale((inverse, inverse), None);
                    (stops, spread, to_unit, local, Kind::Radial, false)
                }
                Paint::ConicGradient {
                    center,
                    start_angle_degrees,
                    sweep_angle_degrees,
                    stops,
                    spread,
                } => {
                    let (center, (start, end), local) =
                        super::conic_geometry(*center, *start_angle_degrees, *sweep_angle_degrees);
                    if !start_angle_degrees.is_finite()
                        || !end.is_finite()
                        || start > end
                        || (start - end).abs() <= DEGENERATE
                    {
                        return None;
                    }
                    let (first, last) = (start / 360.0, end / 360.0);
                    let angles = Stage::of(
                        &(Matrix::scale((1.0 / (last - first), 1.0))
                            * Matrix::translate((-first, 0.0))),
                    )?;
                    let to_unit = Matrix::translate((-center.x, -center.y));
                    let whole_turn = start <= 0.0 && end >= 360.0;
                    (
                        stops,
                        spread,
                        to_unit,
                        local,
                        Kind::Sweep(angles),
                        whole_turn,
                    )
                }
                Paint::Solid(_) | Paint::TwoCircleGradient { .. } => return None,
            };
            // A sweep over a whole turn clamps.
            let tile = if whole_turn {
                TileMode::Clamp
            } else {
                tile_mode(*spread)
            };
            let (colors, positions) = gradient_parts(stops);
            let (ramp, opaque, even) = Ramp::new(&colors, &positions, tile)?;
            let tile = match tile {
                TileMode::Clamp if even => Tile::Clamp,
                TileMode::Clamp => Tile::None,
                TileMode::Repeat => Tile::Repeat,
                TileMode::Mirror => Tile::Mirror,
                TileMode::Decal => return None,
            };
            // `MatrixRec::apply`: the shader's points to their unit square after the inverse of
            // the transform and the local matrix.
            let inverse = Matrix::concat(device, &local).invert()?;
            let unit = Stage::of(&Matrix::concat(&to_unit, &inverse))?;
            Some(Shading {
                unit,
                kind,
                tile,
                ramp,
                opaque,
            })
        }

        /// Writes the pixels of row `y` from column `left` into `out`.
        fn row(&self, y: i32, left: i32, out: &mut [u8]) {
            // SAFETY: NEON is part of the aarch64 baseline.
            let y = unsafe { vdupq_n_f32(y as f32 + 0.5) };
            // The count of stops the pixel before reached, which neighbours usually share.
            let mut guess = 0;
            let (blocks, tail) = out.as_chunks_mut::<{ LANES * 8 }>();
            let mut x = left;
            for block in blocks {
                let colors = self.four(x, y, &mut guess);
                // SAFETY: the block holds four pixels of four halves, aligned for the halves by
                // the alignment of the rows.
                unsafe { self.store(colors, block.as_mut_ptr().cast()) };
                x += LANES as i32;
            }
            if !tail.is_empty() {
                let colors = self.four(x, y, &mut guess);
                let mut padded = [0_u16; LANES * 4];
                for (half, bytes) in padded.iter_mut().zip(tail.as_chunks::<2>().0) {
                    *half = u16::from_le_bytes(*bytes);
                }
                // SAFETY: the buffer holds four pixels of four halves.
                unsafe { self.store(colors, padded.as_mut_ptr()) };
                for (bytes, half) in tail.as_chunks_mut::<2>().0.iter_mut().zip(&padded) {
                    *bytes = half.to_le_bytes();
                }
            }
        }

        /// Blends the color of the pixel at column `x` of row `y` into the pixel's halves in
        /// `pixel` with full coverage, as Skia's draw does in a column where a clip cuts a run
        /// short: `lerp_u8` by the coverage of 255.
        fn blend(&self, y: i32, x: i32, pixel: &mut [u8]) {
            assert!(pixel.len() >= 8);
            // SAFETY: NEON is part of the aarch64 baseline; the pixel holds four halves, aligned
            // for them by the alignment of the rows.
            unsafe {
                let [color, ..] = self.four(x, vdupq_n_f32(y as f32 + 0.5), &mut 0);
                let pixel = pixel.as_mut_ptr().cast::<u16>();
                let under = vcvt_f32_f16(vreinterpret_f16_u16(vld1_u16(pixel)));
                // `from_byte(255)` is `255 * (1 / 255.0)`, and `lerp(d, s, c)` is
                // `mad(s - d, c, d)`.
                let coverage = vdupq_n_f32(255.0 * (1.0 / 255.0));
                let color = vfmaq_f32(under, vsubq_f32(color, under), coverage);
                vst1_u16(pixel, vreinterpret_u16_f16(vcvt_f16_f32(color)));
            }
        }

        /// The colors of the four pixels from column `x` in the row at `y`.
        #[inline(always)]
        fn four(&self, x: i32, y: float32x4_t, guess: &mut usize) -> [float32x4_t; LANES] {
            // SAFETY: NEON is part of the aarch64 baseline.
            unsafe {
                // `seed_shader`: the pixel centers, which a float holds exactly.
                let x = vaddq_f32(vdupq_n_f32(x as f32), vld1q_f32(CENTERS.as_ptr()));
                let (u, v) = self.unit.apply(x, y);
                let t = match &self.kind {
                    Kind::Linear => u,
                    Kind::Radial => vsqrtq_f32(vaddq_f32(vmulq_f32(u, u), vmulq_f32(v, v))),
                    Kind::Sweep(angles) => angles.apply(unit_angle(u, v), v).0,
                };
                let t = self.tile.apply(t);
                self.ramp.colors(t, guess)
            }
        }

        /// Writes `colors` into the four pixels of halves at `pixels`, over them unless the
        /// colors are opaque.
        ///
        /// # Safety
        ///
        /// `pixels` points to sixteen halves.
        #[inline(always)]
        unsafe fn store(&self, colors: [float32x4_t; LANES], pixels: *mut u16) {
            // SAFETY: NEON is part of the aarch64 baseline; the caller provides the halves.
            unsafe {
                for (index, mut color) in colors.into_iter().enumerate() {
                    let pixel = pixels.add(index * 4);
                    if !self.opaque {
                        // `srcover`: `mad(d, inv(sa), s)`.
                        let under = vcvt_f32_f16(vreinterpret_f16_u16(vld1_u16(pixel)));
                        let inverse = vsubq_f32(vdupq_n_f32(1.0), vdupq_laneq_f32::<3>(color));
                        color = vfmaq_f32(color, under, inverse);
                    }
                    vst1_u16(pixel, vreinterpret_u16_f16(vcvt_f16_f32(color)));
                }
            }
        }
    }

    impl Stage {
        fn of(matrix: &Matrix) -> Option<Stage> {
            let mask = matrix.get_type();
            let scale_translate = TypeMask::SCALE | TypeMask::TRANSLATE;
            Some(if mask == TypeMask::IDENTITY {
                Stage::Identity
            } else if mask == TypeMask::TRANSLATE {
                Stage::Translate([matrix.translate_x(), matrix.translate_y()])
            } else if mask | scale_translate == scale_translate {
                Stage::ScaleTranslate([
                    matrix.scale_x(),
                    matrix.scale_y(),
                    matrix.translate_x(),
                    matrix.translate_y(),
                ])
            } else if !matrix.has_perspective() {
                let mut values = [0.0; 9];
                matrix.get_9(&mut values);
                Stage::Affine([
                    values[0], values[1], values[2], values[3], values[4], values[5],
                ])
            } else {
                return None;
            })
        }

        /// `matrix_translate`, `matrix_scale_translate` or `matrix_2x3` on `(x, y)`.
        #[inline(always)]
        fn apply(&self, x: float32x4_t, y: float32x4_t) -> (float32x4_t, float32x4_t) {
            // SAFETY: NEON is part of the aarch64 baseline.
            unsafe {
                // Skia's `mad(f, m, a)` is `vfmaq_f32(a, f, m)`.
                match *self {
                    Stage::Identity => (x, y),
                    Stage::Translate([tx, ty]) => {
                        (vaddq_f32(x, vdupq_n_f32(tx)), vaddq_f32(y, vdupq_n_f32(ty)))
                    }
                    Stage::ScaleTranslate([sx, sy, tx, ty]) => (
                        vfmaq_f32(vdupq_n_f32(tx), x, vdupq_n_f32(sx)),
                        vfmaq_f32(vdupq_n_f32(ty), y, vdupq_n_f32(sy)),
                    ),
                    Stage::Affine(m) => (
                        vfmaq_f32(
                            vfmaq_f32(vdupq_n_f32(m[2]), y, vdupq_n_f32(m[1])),
                            x,
                            vdupq_n_f32(m[0]),
                        ),
                        vfmaq_f32(
                            vfmaq_f32(vdupq_n_f32(m[5]), y, vdupq_n_f32(m[4])),
                            x,
                            vdupq_n_f32(m[3]),
                        ),
                    ),
                }
            }
        }
    }

    /// `xy_to_unit_angle`: the angle of `(x, y)` in turns, from Skia's polynomial for `atan`.
    #[inline(always)]
    fn unit_angle(x: float32x4_t, y: float32x4_t) -> float32x4_t {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe {
            let (xabs, yabs) = (vabsq_f32(x), vabsq_f32(y));
            let slope = vdivq_f32(vminq_f32(xabs, yabs), vmaxq_f32(xabs, yabs));
            let s = vmulq_f32(slope, slope);
            // Skia disables contraction with -ffp-contract=off outside Windows. Its Windows
            // /fp:precise build contracts this polynomial's multiply-add expressions into FMA.
            // Match that rounding before the ramp magnifies it into a stored F16 difference.
            #[cfg(target_os = "windows")]
            let phi = {
                let phi = vfmaq_f32(vdupq_n_f32(0.02476102), s, vdupq_n_f32(-0.0070547382));
                let phi = vfmaq_f32(vdupq_n_f32(-0.05185397), s, phi);
                vfmaq_f32(vdupq_n_f32(0.15912117), s, phi)
            };
            #[cfg(not(target_os = "windows"))]
            let phi = {
                let phi = vaddq_f32(
                    vdupq_n_f32(0.02476102),
                    vmulq_f32(s, vdupq_n_f32(-0.0070547382)),
                );
                let phi = vaddq_f32(vdupq_n_f32(-0.05185397), vmulq_f32(s, phi));
                vaddq_f32(vdupq_n_f32(0.15912117), vmulq_f32(s, phi))
            };
            let phi = vmulq_f32(slope, phi);
            let phi = vbslq_f32(
                vcltq_f32(xabs, yabs),
                vsubq_f32(vdupq_n_f32(0.25), phi),
                phi,
            );
            let phi = vbslq_f32(
                vcltq_f32(x, vdupq_n_f32(0.0)),
                vsubq_f32(vdupq_n_f32(0.5), phi),
                phi,
            );
            let phi = vbslq_f32(
                vcltq_f32(y, vdupq_n_f32(0.0)),
                vsubq_f32(vdupq_n_f32(1.0), phi),
                phi,
            );
            // A NaN, as at the center, is the start.
            vbslq_f32(vceqq_f32(phi, phi), phi, vdupq_n_f32(0.0))
        }
    }

    impl Tile {
        /// `clamp_x_1`, `repeat_x_1` or `mirror_x_1`.
        #[inline(always)]
        fn apply(&self, t: float32x4_t) -> float32x4_t {
            // SAFETY: NEON is part of the aarch64 baseline.
            unsafe {
                // `clamp_01_` is `min(max(0, v), 1)`.
                let clamp =
                    |v: float32x4_t| vminq_f32(vmaxq_f32(vdupq_n_f32(0.0), v), vdupq_n_f32(1.0));
                let one = vdupq_n_f32(1.0);
                match self {
                    Tile::None => t,
                    Tile::Clamp => clamp(t),
                    Tile::Repeat => clamp(vsubq_f32(t, vrndmq_f32(t))),
                    Tile::Mirror => {
                        let shifted = vsubq_f32(t, one);
                        let floor = vrndmq_f32(vmulq_f32(shifted, vdupq_n_f32(0.5)));
                        let folded = vsubq_f32(vsubq_f32(shifted, vaddq_f32(floor, floor)), one);
                        clamp(vabsq_f32(folded))
                    }
                }
            }
        }
    }

    impl Ramp {
        /// The lookup of `colors` at `positions` under `tile`, with whether the colors are
        /// opaque and whether the stops are evenly spaced, the only ones Skia clamps `t` for.
        fn new(
            colors: &[Color4f],
            positions: &[f32],
            tile: TileMode,
        ) -> Option<(Ramp, bool, bool)> {
            let count = colors.len();
            if count < 2 || positions.len() != count {
                return None;
            }
            // `SkGradientBaseShader`'s constructor: stops at 0 and 1 where the positions stop
            // short of them, positions pinned to increase up to 1, evenly spaced stops without
            // positions, and runs of equal positions down to their ends.
            let first = positions[0] > 0.0;
            let last = positions[count - 1] != 1.0;
            let opaque = colors.iter().all(|color| color.a == 1.0);
            let mut stops = Vec::with_capacity(count + 2);
            if first {
                stops.push(colors[0]);
            }
            stops.extend_from_slice(colors);
            if last {
                stops.push(colors[count - 1]);
            }
            let mut at = Vec::with_capacity(stops.len());
            at.push(0.0_f32);
            let mut previous = 0.0_f32;
            let start = usize::from(!first);
            let step = positions[start] - previous;
            let mut uniform = true;
            let ends = positions[start..].iter().copied().map(Some);
            for end in ends.chain(last.then_some(None)) {
                let current = end.map_or(1.0, |position| pin(position, previous, 1.0));
                uniform &= (step - (current - previous)).abs() <= 1.0 / 4096.0;
                at.push(current);
                previous = current;
            }
            let at = if uniform {
                None
            } else {
                let clamp = tile == TileMode::Clamp;
                let (mut kept, mut kept_at) = (Vec::new(), Vec::new());
                let mut run = 0;
                for next in 1..=stops.len() {
                    if next == stops.len() || at[run] != at[next] {
                        let repeated = next - run > 1;
                        if !(repeated && !clamp && at[run] == 0.0) {
                            kept.push(stops[run]);
                            kept_at.push(at[run]);
                        }
                        if repeated && !(!clamp && at[next - 1] == 1.0) {
                            kept.push(stops[next - 1]);
                            kept_at.push(at[next - 1]);
                        }
                        run = next;
                    }
                }
                stops = kept;
                Some(kept_at)
            };
            // `SkColor4fXformer`: in the destination's space, premultiplied.
            let colors = stops
                .iter()
                .map(|color| {
                    [
                        color.r * color.a,
                        color.g * color.a,
                        color.b * color.a,
                        color.a,
                    ]
                })
                .collect::<Vec<_>>();
            let count = colors.len();
            let record = |factor: [f32; 4], bias: [f32; 4]| {
                [
                    factor[0], factor[1], factor[2], factor[3], bias[0], bias[1], bias[2], bias[3],
                ]
            };
            let minus =
                |a: [f32; 4], b: [f32; 4]| [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]];
            let times = |a: [f32; 4], s: f32| [a[0] * s, a[1] * s, a[2] * s, a[3] * s];
            let even = at.is_none();
            let ramp = match at {
                None if count == 2 => Ramp::Two(record(minus(colors[1], colors[0]), colors[0])),
                None => {
                    // `init_stop_evenly`.
                    let gaps = (count - 1) as f32;
                    let mut records = colors
                        .windows(2)
                        .enumerate()
                        .map(|(index, pair)| {
                            let factor = times(minus(pair[1], pair[0]), gaps);
                            record(factor, minus(pair[0], times(factor, index as f32 / gaps)))
                        })
                        .collect::<Vec<_>>();
                    records.push(record([0.0; 4], colors[count - 1]));
                    Ramp::Even(records)
                }
                Some(at) => {
                    // The stops at 0 and 1 the constructor added fall to the search.
                    let (first, last) = if count > 2 {
                        (
                            usize::from(colors[0] == colors[1]),
                            if colors[count - 2] != colors[count - 1] {
                                count - 1
                            } else {
                                count - 2
                            },
                        )
                    } else {
                        (0, 1)
                    };
                    let mut starts = Vec::new();
                    let mut records = vec![record([0.0; 4], colors[first])];
                    let (mut left, mut color) = (at[first], colors[first]);
                    for index in first..last {
                        let (right, next) = (at[index + 1], colors[index + 1]);
                        if left < right {
                            // `init_stop_pos`.
                            let reciprocal = 1.0 / (right - left);
                            if reciprocal.is_finite() {
                                let factor = times(minus(next, color), reciprocal);
                                records.push(record(factor, minus(color, times(factor, left))));
                                starts.push(left);
                            }
                        }
                        (left, color) = (right, next);
                    }
                    starts.push(left);
                    records.push(record([0.0; 4], color));
                    starts.resize((starts.len() + 1).next_power_of_two(), f32::NAN);
                    Ramp::Stops { starts, records }
                }
            };
            Some((ramp, opaque, even))
        }

        /// The premultiplied colors of the four lanes of `t`.
        #[inline(always)]
        fn colors(&self, t: float32x4_t, guess: &mut usize) -> [float32x4_t; LANES] {
            // SAFETY: NEON is part of the aarch64 baseline.
            unsafe {
                match self {
                    Ramp::Two(record) => {
                        let (factor, bias) = load(record);
                        [
                            vfmaq_laneq_f32::<0>(bias, factor, t),
                            vfmaq_laneq_f32::<1>(bias, factor, t),
                            vfmaq_laneq_f32::<2>(bias, factor, t),
                            vfmaq_laneq_f32::<3>(bias, factor, t),
                        ]
                    }
                    Ramp::Even(records) => {
                        let scale = vdupq_n_f32((records.len() - 1) as f32);
                        let index = vreinterpretq_u32_s32(vcvtq_s32_f32(vmulq_f32(t, scale)));
                        let last = records.len() - 1;
                        lookup(
                            records,
                            [
                                (vgetq_lane_u32::<0>(index) as usize).min(last),
                                (vgetq_lane_u32::<1>(index) as usize).min(last),
                                (vgetq_lane_u32::<2>(index) as usize).min(last),
                                (vgetq_lane_u32::<3>(index) as usize).min(last),
                            ],
                            t,
                        )
                    }
                    Ramp::Stops { starts, records } => {
                        let counts = counts(starts, t, guess);
                        lookup(records, counts, t)
                    }
                }
            }
        }
    }

    /// For each lane of `t`, how many of `starts` it reaches, as Skia's `gradient` stage counts
    /// them, from `guess`, which it updates.
    #[inline(always)]
    fn counts(starts: &[f32], t: float32x4_t, guess: &mut usize) -> [usize; LANES] {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe {
            // `starts` increase, so the lanes that reach the guessed stop's start but not the
            // next one's reach exactly that many. A NaN reaches none.
            let count = *guess;
            let below = if count == 0 {
                f32::NEG_INFINITY
            } else {
                starts[count - 1]
            };
            let reached = vcgeq_f32(t, vdupq_n_f32(below));
            let passed = vcgeq_f32(t, vdupq_n_f32(starts[count]));
            if vminvq_u32(vbicq_u32(reached, passed)) == u32::MAX {
                return [count; LANES];
            }
            // Otherwise a binary search: the padding is a power of two above the count.
            let mut counts = vdupq_n_u32(0);
            let mut step = starts.len() / 2;
            while step > 0 {
                let probe = vaddq_u32(counts, vdupq_n_u32(step as u32 - 1));
                let probed = [
                    starts[vgetq_lane_u32::<0>(probe) as usize],
                    starts[vgetq_lane_u32::<1>(probe) as usize],
                    starts[vgetq_lane_u32::<2>(probe) as usize],
                    starts[vgetq_lane_u32::<3>(probe) as usize],
                ];
                let reached = vcgeq_f32(t, vld1q_f32(probed.as_ptr()));
                counts = vaddq_u32(counts, vandq_u32(reached, vdupq_n_u32(step as u32)));
                step /= 2;
            }
            let counts = [
                vgetq_lane_u32::<0>(counts) as usize,
                vgetq_lane_u32::<1>(counts) as usize,
                vgetq_lane_u32::<2>(counts) as usize,
                vgetq_lane_u32::<3>(counts) as usize,
            ];
            *guess = counts[LANES - 1];
            counts
        }
    }

    /// `gradient_lookup`: `mad(t, factor, bias)` with the factor and bias of each lane's stop.
    #[inline(always)]
    fn lookup(records: &[[f32; 8]], index: [usize; LANES], t: float32x4_t) -> [float32x4_t; LANES] {
        // SAFETY: NEON is part of the aarch64 baseline.
        unsafe {
            let (f0, b0) = load(&records[index[0]]);
            let (f1, b1) = load(&records[index[1]]);
            let (f2, b2) = load(&records[index[2]]);
            let (f3, b3) = load(&records[index[3]]);
            [
                vfmaq_laneq_f32::<0>(b0, f0, t),
                vfmaq_laneq_f32::<1>(b1, f1, t),
                vfmaq_laneq_f32::<2>(b2, f2, t),
                vfmaq_laneq_f32::<3>(b3, f3, t),
            ]
        }
    }

    /// The factor and the bias of a stop.
    #[inline(always)]
    fn load(record: &[f32; 8]) -> (float32x4_t, float32x4_t) {
        // SAFETY: NEON is part of the aarch64 baseline; the record holds eight floats.
        unsafe {
            (
                vld1q_f32(record.as_ptr()),
                vld1q_f32(record.as_ptr().add(4)),
            )
        }
    }

    /// `SkTPin`: `std::max(low, std::min(value, high))`.
    fn pin(value: f32, low: f32, high: f32) -> f32 {
        let lower = if high < value { high } else { value };
        if low < lower { lower } else { low }
    }
}
