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

/// Most bands along an axis. A glyph gets one per curve up to this.
const BANDS: usize = 16;
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
    out.bands = [
        horizontal.len().clamp(1, BANDS) as u32,
        vertical.len().clamp(1, BANDS) as u32,
    ];

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
