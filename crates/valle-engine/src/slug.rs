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
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn face(coords: Vec<i16>) -> Face {
        Face {
            data: crate::testing::built_in(0),
            index: 0,
            coords,
            slanted: false,
        }
    }

    fn glyph(face: &Face, c: char) -> u32 {
        let font = FontRef::from_index(face.data.data(), face.index).unwrap();
        font.charmap().map(c).unwrap().to_u32()
    }

    fn of(c: char) -> Outline {
        let f = face(Vec::new());
        outline(&f, glyph(&f, c), 0.0).unwrap()
    }

    /// The curve at a texel, relative to `p`.
    fn curve(o: &Outline, texel: u32, p: [f32; 2]) -> [[f32; 2]; 3] {
        let [a, b] = [o.curves[texel as usize], o.curves[texel as usize + 1]];
        [
            [a[0] - p[0], a[1] - p[1]],
            [a[2] - p[0], a[3] - p[1]],
            [b[0] - p[0], b[1] - p[1]],
        ]
    }

    fn list(o: &Outline, band: usize) -> &[u32] {
        let [count, start] = [o.lists[2 * band], o.lists[2 * band + 1]];
        &o.lists[start as usize..(start + count) as usize]
    }

    /// Which crossings of the line `v = 0` count, from the signs of the three points: bit 0 for the
    /// crossing where `v` falls, bit 8 for the one where it rises.
    fn crossings(v: [f32; 3]) -> u32 {
        let i = u32::from(v[0] < 0.0) | u32::from(v[1] < 0.0) << 1 | u32::from(v[2] < 0.0) << 2;
        (0x2E74 >> i) & 0x0101
    }

    /// Where the curve crosses its axis `cut = 0`: the other coordinate at the falling and the
    /// rising crossing.
    fn roots(c: [[f32; 2]; 3], cut: usize) -> [f32; 2] {
        let o = 1 - cut;
        let a = [0, 1].map(|k| c[0][k] - 2.0 * c[1][k] + c[2][k]);
        let b = [0, 1].map(|k| c[0][k] - c[1][k]);
        let [ay, by, cy] = [a[cut], b[cut], c[0][cut]];
        let square = by * by - ay * cy;
        let d = square.max(0.0).sqrt();
        let fall = if by > 0.0 {
            cy / (by + d)
        } else {
            (by - d) / ay
        };
        let rise = if by < 0.0 {
            cy / (by - d)
        } else {
            (by + d) / ay
        };
        let t = if square < 0.0 {
            [by / ay; 2]
        } else {
            [fall, rise]
        };
        t.map(|t| (a[o] * t - 2.0 * b[o]) * t + c[0][o])
    }

    /// A pixel's coverage at `p`, `ppu` pixels per font unit: the shader's arithmetic.
    fn coverage(o: &Outline, p: [f32; 2], ppu: f32) -> f32 {
        let mut cov = [0.0f32; 2];
        let mut weight = [0.0f32; 2];
        // The horizontal ray, then the vertical one.
        for (ray, cut, band) in [
            (0, 1, o.band(1, p[1])),
            (1, 0, o.bands[0] as usize + o.band(0, p[0])),
        ] {
            // Against the horizontal ray a falling curve winds one way; against the vertical one a
            // rising curve does.
            let sign = if ray == 0 { 1.0 } else { -1.0 };
            for &texel in list(o, band) {
                let c = curve(o, texel, p);
                if c[0][ray].max(c[1][ray]).max(c[2][ray]) * ppu < -0.5 {
                    break;
                }
                let code = crossings([c[0][cut], c[1][cut], c[2][cut]]);
                let r = roots(c, cut).map(|r| r * ppu);
                if code & 1 != 0 {
                    cov[ray] += sign * (r[0] + 0.5).clamp(0.0, 1.0);
                    weight[ray] = weight[ray].max((1.0 - r[0].abs() * 2.0).clamp(0.0, 1.0));
                }
                if code > 1 {
                    cov[ray] -= sign * (r[1] + 0.5).clamp(0.0, 1.0);
                    weight[ray] = weight[ray].max((1.0 - r[1].abs() * 2.0).clamp(0.0, 1.0));
                }
            }
        }
        let mixed = (cov[0] * weight[0] + cov[1] * weight[1]).abs()
            / (weight[0] + weight[1]).max(1.0 / 65536.0);
        mixed.max(cov[0].abs().min(cov[1].abs())).clamp(0.0, 1.0)
    }

    /// The area inside the outline, by Green's theorem: ∮ x dy over its curves, exact by Simpson's
    /// rule since the integrand is cubic.
    fn area(o: &Outline) -> f32 {
        let texels: BTreeSet<u32> = (0..o.bands[0] as usize)
            .flat_map(|b| list(o, b).iter().copied())
            .collect();
        let mut sum = 0.0;
        for texel in texels {
            let [p0, p1, p2] = curve(o, texel, [0.0, 0.0]);
            let at = |t: f32| {
                let x = (1.0 - t) * (1.0 - t) * p0[0] + 2.0 * t * (1.0 - t) * p1[0] + t * t * p2[0];
                let dy = 2.0 * (1.0 - t) * (p1[1] - p0[1]) + 2.0 * t * (p2[1] - p1[1]);
                x * dy
            };
            sum += (at(0.0) + 4.0 * at(0.5) + at(1.0)) / 6.0;
        }
        sum.abs()
    }

    #[test]
    fn coverage_adds_up_to_the_area() {
        for c in ['o', 'g', '@', 'M', 'e', '&'] {
            let o = of(c);
            // Pixels of 4 font units: the glyph is about 180 px tall.
            let s = 4.0;
            let [x0, y0, x1, y1] = o.bounds;
            let mut sum = 0.0;
            let mut y = y0 - s;
            while y < y1 + s {
                let mut x = x0 - s;
                while x < x1 + s {
                    sum += coverage(&o, [x, y], 1.0 / s) * s * s;
                    x += s;
                }
                y += s;
            }
            let a = area(&o);
            assert!((sum - a).abs() < a * 0.005, "{c}: {sum} vs {a}");
        }
    }

    #[test]
    fn edges_cover_what_lies_inside() {
        let o = of('l');
        let [x0, y0, x1, y1] = o.bounds;
        let mid = (y0 + y1) / 2.0;
        // A pixel of 10 font units centered on the stem's left edge, then inside, then outside.
        let ppu = 0.1;
        assert_eq!(coverage(&o, [x0, mid], ppu), 0.5);
        assert_eq!(coverage(&o, [x0 + 2.5, mid], ppu), 0.75);
        assert_eq!(coverage(&o, [(x0 + x1) / 2.0, mid], ppu), 1.0);
        assert_eq!(coverage(&o, [x0 - 5.0, mid], ppu), 0.0);
        assert_eq!(coverage(&o, [x1 + 20.0, mid], ppu), 0.0);
        // The hole of an o is empty.
        let o = of('o');
        let [x0, y0, x1, y1] = o.bounds;
        assert_eq!(coverage(&o, [(x0 + x1) / 2.0, (y0 + y1) / 2.0], 0.1), 0.0);
    }

    #[test]
    fn lines_cross_where_they_are_from_any_pixel() {
        // Pixel centers land anywhere, and moving a line's points to one rounds them: its a is then
        // a stray ulp rather than 0.
        let o = of('V');
        let texels: BTreeSet<u32> = (0..(o.bands[0] + o.bands[1]) as usize)
            .flat_map(|b| list(&o, b).iter().copied())
            .collect();
        let mut lines = 0;
        for texel in texels {
            let c = curve(&o, texel, [0.0, 0.0]);
            if (0..2).any(|k| c[0][k] - 2.0 * c[1][k] + c[2][k] != 0.0) {
                continue;
            }
            lines += 1;
            // The rays along the line never cross it.
            for cut in [0, 1].into_iter().filter(|&k| c[0][k] != c[2][k]) {
                let other = 1 - cut;
                for i in 0..1000 {
                    let mut p = [-3.7, -3.7];
                    p[cut] = c[0][cut] + (c[2][cut] - c[0][cut]) * (i as f32 + 0.5) / 1000.0;
                    let moved = curve(&o, texel, p);
                    let code = crossings([moved[0][cut], moved[1][cut], moved[2][cut]]);
                    assert_ne!(code, 0, "curve {texel} at {p:?}");
                    let s = (p[cut] - c[0][cut]) / (c[2][cut] - c[0][cut]);
                    let want = c[0][other] + (c[2][other] - c[0][other]) * s - p[other];
                    let r = roots(moved, cut);
                    let got = if code & 1 != 0 { r[0] } else { r[1] };
                    assert!((got - want).abs() < 0.01, "curve {texel}: {got} vs {want}");
                }
            }
        }
        // Four slanted, three level.
        assert_eq!(lines, 7);
    }

    #[test]
    fn rays_get_nothing_from_a_curve_that_turns_back_before_them() {
        // Its control point is past the ray, so both crossings count, but it turns back first.
        let c = [[-5.0, 2.5], [0.0, -1.5], [5.0, 2.5]];
        assert_eq!(crossings([2.5, -1.5, 2.5]), 0x0101);
        let [fall, rise] = roots(c, 1);
        assert_eq!(fall, rise);
        // A pixel just past its tip, outside, and one inside.
        let o = encode(
            1.0,
            &[vec![
                [[0.0, 0.0], [5.0, -4.0], [10.0, 0.0]],
                [[10.0, 0.0], [5.0, 0.0], [0.0, 0.0]],
            ]],
        );
        assert_eq!(coverage(&o, [5.0, -2.5], 1.0), 0.0);
        assert_eq!(coverage(&o, [5.0, -1.0], 1.0), 1.0);
    }

    #[test]
    fn bands_list_every_curve_reaching_them_in_order() {
        let o = of('g');
        assert_eq!(o.bands, [16, 16]);
        // Every curve is in some list, and lists name only curves, never a contour's last texel.
        let all: BTreeSet<u32> = (0..32).flat_map(|b| list(&o, b).iter().copied()).collect();
        assert_eq!(*all.last().unwrap() as usize, o.curves.len() - 2);
        let [x0, y0, x1, y1] = o.bounds;
        for (cut, base, lo, hi) in [(1, 0, y0, y1), (0, 16, x0, x1)] {
            let sort = 1 - cut;
            let step = (hi - lo) / 16.0;
            for band in 0..16 {
                let [from, to] = [lo + step * band as f32, lo + step * (band + 1) as f32];
                let listed = list(&o, base + band);
                let reach = |t: u32| {
                    let c = curve(&o, t, [0.0, 0.0]);
                    c[0][sort].max(c[1][sort]).max(c[2][sort])
                };
                assert!(listed.windows(2).all(|w| reach(w[0]) >= reach(w[1])));
                for &t in &all {
                    let c = curve(&o, t, [0.0, 0.0]);
                    let v = [c[0][cut], c[1][cut], c[2][cut]];
                    // A curve the ray runs along never crosses it.
                    let across = !(v[0] == v[1] && v[1] == v[2]);
                    let (min, max) = (v[0].min(v[1]).min(v[2]), v[0].max(v[1]).max(v[2]));
                    let reaches = across && max >= from && min <= to;
                    let near = across && max >= from - 0.01 && min <= to + 0.01;
                    assert!(!reaches || listed.contains(&t), "band {band}: curve {t}");
                    assert!(near || !listed.contains(&t), "band {band}: curve {t}");
                }
            }
        }
    }

    #[test]
    fn spaces_have_no_curves_and_weights_change_the_outline() {
        let o = of(' ');
        assert!(o.curves.is_empty() && o.lists.is_empty());
        assert_eq!(o.em, 1000.0);
        let regular = face(Vec::new());
        let font = FontRef::from_index(regular.data.data(), 0).unwrap();
        let black = font.axes().location([("wght", 900.0)]);
        let f = face(black.coords().iter().map(|c| c.to_bits()).collect());
        let bold = outline(&f, glyph(&f, 'l'), 0.0).unwrap();
        let regular = of('l');
        assert!(bold.bounds[2] - bold.bounds[0] > regular.bounds[2] - regular.bounds[0]);
    }

    #[test]
    fn slanted_faces_lean_by_a_quarter_of_the_height() {
        let upright = face(Vec::new());
        let slanted = Face {
            slanted: true,
            ..upright.clone()
        };
        let id = glyph(&upright, 'l');
        let points = |f: &Face| {
            let (_, contours) = contours(f, id).unwrap();
            contours.into_iter().flatten().flatten().collect::<Vec<_>>()
        };
        let leaning: Vec<[f32; 2]> = (points(&upright).into_iter())
            .map(|[x, y]| [x + y / 4.0, y])
            .collect();
        assert_eq!(points(&slanted), leaning);
        assert!(leaning.iter().any(|p| p[1] > 500.0));
    }

    #[test]
    fn strokes_cover_a_band_centered_on_the_edges() {
        let f = face(Vec::new());
        // 40 font units wide, on the stem of an l, a rectangle 1000 units to the em.
        let o = outline(&f, glyph(&f, 'l'), 0.04).unwrap();
        let [x0, y0, x1, y1] = of('l').bounds;
        let grown = [x0 - 20.0, y0 - 20.0, x1 + 20.0, y1 + 20.0];
        assert!(
            (0..4).all(|k| (o.bounds[k] - grown[k]).abs() < 0.01),
            "{:?}",
            o.bounds
        );
        // Pixels of 10 font units across the stem, and in the corner, which is mitered.
        let mid = (y0 + y1) / 2.0;
        let ppu = 0.1;
        assert_eq!(coverage(&o, [x0 - 20.0, mid], ppu), 0.5);
        assert_eq!(coverage(&o, [x0 - 10.0, mid], ppu), 1.0);
        assert_eq!(coverage(&o, [x0 + 10.0, mid], ppu), 1.0);
        assert_eq!(coverage(&o, [x0 + 20.0, mid], ppu), 0.5);
        assert_eq!(coverage(&o, [(x0 + x1) / 2.0, mid], ppu), 0.0);
        assert_eq!(coverage(&o, [x0 - 15.0, y1 + 15.0], ppu), 1.0);
    }

    #[test]
    fn runs_hold_every_pixel_that_shows_and_solid_ones_are_covered() {
        let f = face(Vec::new());
        // Outlines with holes, and a stroke, which is a thin ring.
        let (em, o) = contours(&f, glyph(&f, 'O')).unwrap();
        let ring = path::stroke(&o, 0.02 * em, em / 4096.0);
        // The ring, 20 units across, is thinner than a cell grown by the margin: none of it is solid.
        let ring = (ring, false);
        let shapes = [
            (o, true),
            (contours(&f, glyph(&f, '@')).unwrap().1, true),
            ring,
        ];
        for (contours, filled) in shapes {
            let o = encode(em, &contours);
            // 4 font units a pixel.
            let ppu = 0.25;
            let runs = o.runs([48, 48], 1.5 / ppu, false);
            let [x0, y0, x1, y1] = o.bounds;
            let (mut solid, mut drawn, mut all) = (0, 0, 0);
            let mut y = y0 - 10.0;
            while y < y1 + 10.0 {
                let mut x = x0 - 10.0;
                while x < x1 + 10.0 {
                    let c = coverage(&o, [x, y], ppu);
                    let run = runs.iter().find(|r| {
                        let [a, b, c, d] = r.rect;
                        x >= a && x < c && y >= b && y < d
                    });
                    match run {
                        None => assert_eq!(c, 0.0, "{x}, {y} shows {c} out of the runs"),
                        Some(r) if r.solid => {
                            assert_eq!(c, 1.0, "{x}, {y} in a solid run is {c}");
                            solid += 1;
                        }
                        Some(_) => drawn += 1,
                    }
                    all += 1;
                    x += 4.0;
                }
                y += 4.0;
            }
            assert!(
                (solid > 0) == filled && solid + drawn < all,
                "{solid} solid, {drawn} drawn of {all}"
            );
        }
    }

    #[test]
    fn crossings_count_a_vertex_once_and_a_touch_not_at_all() {
        let mut out = Vec::new();
        // Up to a vertex at y = 1, and on up: the line there crosses once.
        super::crossings([[0.0, 0.0], [0.0, 0.5], [0.0, 1.0]], 1.0, &mut out);
        super::crossings([[0.0, 1.0], [0.0, 1.5], [0.0, 2.0]], 1.0, &mut out);
        assert_eq!(out, [(0.0, 1)]);
        // A peak at y = 1: up then down, which the line touches.
        out.clear();
        super::crossings([[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]], 1.0, &mut out);
        super::crossings([[1.0, 1.0], [1.5, 0.5], [2.0, 0.0]], 1.0, &mut out);
        assert_eq!(out, []);
        // A curve that turns: crossed on the way up and on the way down.
        out.clear();
        super::crossings([[0.0, 0.0], [1.0, 2.0], [2.0, 0.0]], 0.5, &mut out);
        assert_eq!(out.iter().map(|c| c.1).collect::<Vec<_>>(), [1, -1]);
        assert!((out[0].0 + out[1].0 - 2.0).abs() < 1e-5, "{out:?}");
    }
}
