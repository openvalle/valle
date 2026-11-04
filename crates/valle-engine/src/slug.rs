//! Glyph outlines, and filled paths, for Slug coverage (Eric Lengyel, "GPU-Centered Font
//! Rendering Directly from Glyph Outlines", JCGT 2017). An outline becomes quadratic curves, and
//! its bounds are cut into horizontal and vertical bands, each listing the curves that reach into
//! it, so that a pixel reads only the curves near it. Its coverage comes from two rays from its
//! center, right through its horizontal band and up through its vertical one: each curve a ray
//! crosses counts by how much of the pixel lies before the crossing.

use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId, MetadataProvider};

use crate::path::{self, Curve};
use crate::template::Error;
use crate::text::Face;

/// Most bands along an axis.
const BANDS: usize = 256;

/// Bands along an axis for `n` curves crossing it: one a curve up to 16, as a glyph has, then one
/// every 4 curves, so that a pixel of a large path reads about as many curves as one of a glyph.
fn bands(n: usize) -> u32 {
    n.min(16).max(n.div_ceil(4)).clamp(1, BANDS) as u32
}
/// How far past a band's edges, in font units, a curve still counts as reaching into it, so the
/// GPU's band of a point near an edge lists every curve there.
const BAND_MARGIN: f32 = 1.0 / 1024.0;

/// A glyph's outline, in font units, y up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outline {
    /// Font units per em.
    pub em: f32,
    /// Min x, min y, max x, max y of the control points.
    pub bounds: [f32; 4],
    /// Each contour's curves in order, a texel each: the start point and the control point; a
    /// curve ends where the next texel starts, and each contour ends with a texel of its last
    /// end point.
    pub curves: Vec<[f32; 4]>,
    /// How many horizontal and vertical bands.
    pub bands: [u32; 2],
    /// Each band's curve count and where its list of curve texels starts here: horizontal bands
    /// bottom up, then vertical bands left to right. The lists follow, the curves of a
    /// horizontal band by their right end first, of a vertical band by their top first, so the
    /// rays stop at the first curve that ends before the pixel.
    pub lists: Vec<u32>,
}

impl Outline {
    /// Band `i` along x and y holds the points whose `(p - bounds.min) * scale` floors to `i`.
    pub fn band_scale(&self) -> [f32; 2] {
        let scale = |n: u32, lo: f32, hi: f32| {
            if hi > lo { n as f32 / (hi - lo) } else { 0.0 }
        };
        let [x0, y0, x1, y1] = self.bounds;
        [scale(self.bands[1], x0, x1), scale(self.bands[0], y0, y1)]
    }

    /// The band along an axis (0 for x, the vertical bands) of a coordinate there.
    pub fn band(&self, axis: usize, v: f32) -> usize {
        let n = self.bands[1 - axis];
        let at = ((v - self.bounds[axis]) * self.band_scale()[axis]).floor();
        at.clamp(0.0, n.saturating_sub(1) as f32) as usize
    }
}

/// The outline of glyph `glyph` of `face`, in its variation instance; with a `stroke` above 0,
/// that of a stroke that many ems wide along its edges, within 1/4096 em (under 0.1 px up to 400
/// px text).
pub fn outline(face: &Face, glyph: u32, stroke: f32) -> Result<Outline, Error> {
    let (em, mut contours) = contours(face, glyph)?;
    if stroke > 0.0 {
        contours = path::stroke(&contours, stroke * em, em / 4096.0);
    }
    Ok(encode(em, &contours))
}

/// The closed contours of glyph `glyph` of `face`, in its variation instance, in font units y
/// up, and the font's units per em.
pub fn contours(face: &Face, glyph: u32) -> Result<(f32, Vec<Vec<Curve>>), Error> {
    let bad = |e: &dyn std::fmt::Display| format!("font file: {e}");
    let font = FontRef::from_index(face.data.data(), face.index).map_err(|e| bad(&e))?;
    let em = f32::from(font.head().map_err(|e| bad(&e))?.units_per_em());
    let outlines = font.outline_glyphs();
    let Some(drawn) = outlines.get(GlyphId::new(glyph)) else {
        return Ok((em, Vec::new()));
    };
    let coords: Vec<NormalizedCoord> = face
        .coords
        .iter()
        .map(|&c| NormalizedCoord::from_bits(c))
        .collect();
    // Cubic curves (CFF and CFF2 outlines) as quadratic ones within 1/4096 em.
    let mut pen = Pen {
        tolerance: em / 4096.0,
        ..Pen::default()
    };
    drawn
        .draw(
            DrawSettings::unhinted(Size::unscaled(), LocationRef::new(&coords)),
            &mut pen,
        )
        .map_err(|e| format!("glyph {glyph}: {e}"))?;
    pen.end();
    // Slanted as Skia slants a face for an italic it does not have, by a quarter of the height.
    if face.slanted {
        for p in pen.contours.iter_mut().flatten().flatten() {
            p[0] += p[1] / 4.0;
        }
    }
    Ok((em, pen.contours))
}

#[derive(Default)]
struct Pen {
    contours: Vec<Vec<Curve>>,
    open: Vec<Curve>,
    start: [f32; 2],
    at: [f32; 2],
    tolerance: f32,
}

impl Pen {
    fn to(&mut self, control: [f32; 2], end: [f32; 2]) {
        if control != self.at || end != self.at {
            self.open.push([self.at, control, end]);
        }
        self.at = end;
    }

    /// Closes the open contour, back to its start.
    fn end(&mut self) {
        if self.at != self.start {
            let [x, y] = self.start;
            self.line_to(x, y);
        }
        if !self.open.is_empty() {
            self.contours.push(std::mem::take(&mut self.open));
        }
    }
}

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.end();
        self.start = [x, y];
        self.at = [x, y];
    }

    fn line_to(&mut self, x: f32, y: f32) {
        // A line is a curve with its control point halfway.
        let mid = [(self.at[0] + x) * 0.5, (self.at[1] + y) * 0.5];
        self.to(mid, [x, y]);
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.to([cx, cy], [x, y]);
    }

    fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
        let points = [self.at, [ax, ay], [bx, by], [x, y]];
        path::cubic(points, self.tolerance, &mut self.open);
        self.at = [x, y];
    }

    fn close(&mut self) {
        self.end();
    }
}

/// A run of cells along a row of a grid over an outline's bounds: its rectangle in font units, y
/// up, min x, min y, max x, max y; and whether it is all inside the outline, where a pixel is
/// covered without reading a curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Run {
    pub rect: [f32; 4],
    pub solid: bool,
}

impl Outline {
    /// The curve at `texel`: its start, control and end point.
    fn curve(&self, texel: u32) -> [[f32; 2]; 3] {
        let [a, b] = [self.curves[texel as usize], self.curves[texel as usize + 1]];
        [[a[0], a[1]], [a[2], a[3]], [b[0], b[1]]]
    }

    /// The curves of band `b`: horizontal bands from 0, then vertical ones.
    fn listed(&self, b: usize) -> &[u32] {
        let [count, start] = [self.lists[2 * b], self.lists[2 * b + 1]];
        &self.lists[start as usize..(start + count) as usize]
    }

    /// Of a grid of `cells` columns and rows over the bounds, the cells that show anything, in
    /// runs along each row: those with a curve within `margin` font units, whose pixels the
    /// curves cover, and the `solid` ones inside the outline, filled where it winds at all or,
    /// `even_odd`, an odd number of times. The cells out of it, which show nothing, are left out.
    /// Neighbouring runs share their edges exactly.
    pub fn runs(&self, cells: [usize; 2], margin: f32, even_odd: bool) -> Vec<Run> {
        let [x0, y0, x1, y1] = self.bounds;
        let [columns, rows] = cells.map(|n| n.max(1));
        if self.curves.is_empty() || x1 <= x0 || y1 <= y0 {
            return Vec::new();
        }
        let edge = |n: usize, lo: f32, hi: f32, i: usize| match i == n {
            true => hi,
            false => lo + (hi - lo) * i as f32 / n as f32,
        };
        let [xs, ys] = [(columns, x0, x1), (rows, y0, y1)]
            .map(|(n, lo, hi)| (0..=n).map(|i| edge(n, lo, hi, i)).collect::<Vec<f32>>());
        // The cells a curve, grown by the margin, reaches into: every curve is in a vertical band
        // or a horizontal one, or both. A long curve goes in pieces about half a cell long, each
        // within the box of its own control points, as a whole curve's box holds much it is not
        // near.
        let mut near = vec![false; rows * columns];
        let cell = |v: f32, lines: &[f32]| {
            lines.partition_point(|&l| l <= v).clamp(1, lines.len() - 1) - 1
        };
        let size = ((x1 - x0) / columns as f32).min((y1 - y0) / rows as f32);
        // Each curve once, though it is in every band it crosses.
        let mut seen = vec![false; self.curves.len()];
        for b in 0..(self.bands[0] + self.bands[1]) as usize {
            for &texel in self.listed(b) {
                if std::mem::replace(&mut seen[texel as usize], true) {
                    continue;
                }
                let [p0, p1, p2] = self.curve(texel);
                let long =
                    (p1[0] - p0[0]).hypot(p1[1] - p0[1]) + (p2[0] - p1[0]).hypot(p2[1] - p1[1]);
                let n = ((2.0 * long / size).ceil() as usize).clamp(1, 1024);
                let at = |t: f32| {
                    [0, 1].map(|k| {
                        (1.0 - t) * (1.0 - t) * p0[k] + 2.0 * t * (1.0 - t) * p1[k] + t * t * p2[k]
                    })
                };
                let slope = |t: f32| {
                    [0, 1].map(|k| 2.0 * ((1.0 - t) * (p1[k] - p0[k]) + t * (p2[k] - p1[k])))
                };
                for piece in 0..n {
                    let [t0, t1] = [piece as f32 / n as f32, (piece + 1) as f32 / n as f32];
                    let [a, d] = [at(t0), slope(t0)];
                    let c = [a, [0, 1].map(|k| a[k] + d[k] * (t1 - t0) / 2.0), at(t1)];
                    let lo = [0, 1].map(|k| c[0][k].min(c[1][k]).min(c[2][k]) - margin);
                    let hi = [0, 1].map(|k| c[0][k].max(c[1][k]).max(c[2][k]) + margin);
                    let [i0, i1] = [cell(lo[0], &xs), cell(hi[0], &xs)];
                    let [j0, j1] = [cell(lo[1], &ys), cell(hi[1], &ys)];
                    for j in j0..=j1 {
                        near[j * columns + i0..=j * columns + i1].fill(true);
                    }
                }
            }
        }
        let mut runs: Vec<Run> = Vec::new();
        let mut crossed = Vec::new();
        for j in 0..rows {
            // Where the curves of the band of the row's middle cross it, and which way, from the
            // left: the winding of the ray along x from each cell's middle is that of the
            // crossings to its right.
            let y = (ys[j] + ys[j + 1]) / 2.0;
            crossed.clear();
            for &texel in self.listed(self.band(1, y)) {
                crossings(self.curve(texel), y, &mut crossed);
            }
            crossed.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding: i32 = crossed.iter().map(|c| c.1).sum();
            let mut passed = 0;
            let mut open: Option<(usize, bool)> = None;
            for i in 0..=columns {
                let kind = (i < columns).then(|| match near[j * columns + i] {
                    true => Some(false),
                    false => {
                        let x = (xs[i] + xs[i + 1]) / 2.0;
                        while let Some(&(at, way)) = crossed.get(passed)
                            && at <= x
                        {
                            winding -= way;
                            passed += 1;
                        }
                        let inside = if even_odd {
                            winding % 2 != 0
                        } else {
                            winding != 0
                        };
                        inside.then_some(true)
                    }
                });
                let kind = kind.flatten();
                if let Some((first, solid)) = open
                    && kind != Some(solid)
                {
                    runs.push(Run {
                        rect: [xs[first], ys[j], xs[i], ys[j + 1]],
                        solid,
                    });
                    open = None;
                }
                if open.is_none() {
                    open = kind.map(|solid| (i, solid));
                }
            }
        }
        runs
    }
}

/// Where curve `c` crosses the line at `y`, and which way, 1 up or -1 down: each part of it that
/// goes one way counted from its lower end up to before its upper end, so that a crossing where
/// two curves meet counts once, and a vertex the line only touches not at all.
fn crossings(c: [[f32; 2]; 3], y: f32, out: &mut Vec<(f32, i32)>) {
    let [p0, p1, p2] = c.map(|p| p.map(f64::from));
    let y = f64::from(y);
    let a = p0[1] - 2.0 * p1[1] + p2[1];
    let b = 2.0 * (p1[1] - p0[1]);
    let at = |t: f64, k: usize| {
        (1.0 - t) * (1.0 - t) * p0[k] + 2.0 * t * (1.0 - t) * p1[k] + t * t * p2[k]
    };
    // Its parts each way: split where y turns.
    let turn = if a != 0.0 { -b / (2.0 * a) } else { -1.0 };
    let mut ends = vec![0.0];
    if turn > 0.0 && turn < 1.0 {
        ends.push(turn);
    }
    ends.push(1.0);
    for w in ends.windows(2) {
        let [t0, t1] = [w[0], w[1]];
        let [ya, yb] = [at(t0, 1), at(t1, 1)];
        if ya == yb || y < ya.min(yb) || y >= ya.max(yb) {
            continue;
        }
        // The root in the part, which y crosses once.
        let t = if a.abs() < 1e-12 {
            (y - p0[1]) / b
        } else {
            let d = (b * b - 4.0 * a * (p0[1] - y)).max(0.0).sqrt();
            let r = [(-b - d) / (2.0 * a), (-b + d) / (2.0 * a)];
            let mid = (t0 + t1) / 2.0;
            if (r[0] - mid).abs() <= (r[1] - mid).abs() {
                r[0]
            } else {
                r[1]
            }
        };
        let t = t.clamp(t0, t1);
        out.push((at(t, 0) as f32, if yb > ya { 1 } else { -1 }));
    }
}

/// The outline of closed contours of quadratic curves, y up, `em` units per em.
pub fn encode(em: f32, contours: &[Vec<Curve>]) -> Outline {
    let mut out = Outline {
        em,
        ..Outline::default()
    };
    // Each curve with its texel.
    let mut curves = Vec::new();
    for contour in contours {
        for c in contour {
            curves.push((c, out.curves.len() as u32));
            out.curves.push([c[0][0], c[0][1], c[1][0], c[1][1]]);
        }
        let end = contour[contour.len() - 1][2];
        out.curves.push([end[0], end[1], 0.0, 0.0]);
    }
    if curves.is_empty() {
        return out;
    }
    let mut bounds = [f32::INFINITY, f32::INFINITY, -f32::INFINITY, -f32::INFINITY];
    for (c, _) in &curves {
        for p in *c {
            bounds = [
                bounds[0].min(p[0]),
                bounds[1].min(p[1]),
                bounds[2].max(p[0]),
                bounds[3].max(p[1]),
            ];
        }
    }
    out.bounds = bounds;

    // A ray along an axis crosses only the curves that move across it: a horizontal ray skips
    // horizontal lines, a vertical one vertical lines.
    let across = |axis: usize| -> Vec<(&Curve, u32)> {
        let other = 1 - axis;
        curves
            .iter()
            .copied()
            .filter(|(c, _)| !(c[0][other] == c[1][other] && c[1][other] == c[2][other]))
            .collect()
    };
    let horizontal = across(0);
    let vertical = across(1);
    out.bands = [bands(horizontal.len()), bands(vertical.len())];

    let mut lists: Vec<Vec<u32>> = Vec::new();
    // Horizontal bands cut y and sort by x; vertical ones cut x and sort by y.
    for (cut, list) in [(1, &horizontal), (0, &vertical)] {
        let sort = 1 - cut;
        let n = out.bands[1 - cut] as usize;
        let mut bands: Vec<Vec<(f32, u32)>> = vec![Vec::new(); n];
        for &(c, texel) in list {
            let lo = c[0][cut].min(c[1][cut]).min(c[2][cut]);
            let hi = c[0][cut].max(c[1][cut]).max(c[2][cut]);
            let reach = c[0][sort].max(c[1][sort]).max(c[2][sort]);
            let first = out.band(cut, lo - BAND_MARGIN);
            let last = out.band(cut, hi + BAND_MARGIN);
            for band in &mut bands[first..=last] {
                band.push((reach, texel));
            }
        }
        for mut band in bands {
            band.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
            lists.push(band.into_iter().map(|(_, t)| t).collect());
        }
    }
    let mut start = 2 * lists.len();
    for list in &lists {
        out.lists.extend([list.len() as u32, start as u32]);
        start += list.len();
    }
    for list in lists {
        out.lists.extend(list);
    }
    out
}

#[cfg(test)]
mod tests;
