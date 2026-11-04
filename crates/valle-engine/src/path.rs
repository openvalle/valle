//! Paths: SVG path data, as `clip-path: path()` takes it, in absolute commands with arcs made
//! cubic curves; and the closed contours of quadratic curves that fill it, which the GPU draws.

/// A point, px, y down.
pub type Point = [f32; 2];

/// A quadratic curve: start, control and end point.
pub type Curve = [Point; 3];

#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    /// Starts a subpath.
    Move(Point),
    Line(Point),
    /// A quadratic curve through its control point to its end, and a cubic one through two.
    Quad(Point, Point),
    Cubic(Point, Point, Point),
    /// Closes the subpath, back to where it started: a path along it goes on around.
    Close,
}

/// Segments from where the one before ends, or from the origin.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path(pub Vec<Segment>);

impl Path {
    /// An exact geometry key, independent of its paint and placement.
    pub(crate) fn key(&self) -> Vec<u32> {
        let mut key = Vec::with_capacity(self.0.len() * 3);
        for segment in &self.0 {
            let (tag, points): (u32, &[[f32; 2]]) = match segment {
                Segment::Move(a) => (0, std::slice::from_ref(a)),
                Segment::Line(a) => (1, std::slice::from_ref(a)),
                Segment::Quad(a, b) => (2, &[*a, *b]),
                Segment::Cubic(a, b, c) => (3, &[*a, *b, *c]),
                Segment::Close => (4, &[]),
            };
            key.push(tag);
            key.extend(points.iter().flatten().map(|v| v.to_bits()));
        }
        key
    }

    /// The path moved by `by`.
    pub fn moved(&self, by: Point) -> Path {
        let p = |q: Point| [q[0] + by[0], q[1] + by[1]];
        Path(
            (self.0.iter())
                .map(|s| match *s {
                    Segment::Move(a) => Segment::Move(p(a)),
                    Segment::Line(a) => Segment::Line(p(a)),
                    Segment::Quad(a, b) => Segment::Quad(p(a), p(b)),
                    Segment::Cubic(a, b, c) => Segment::Cubic(p(a), p(b), p(c)),
                    Segment::Close => Segment::Close,
                })
                .collect(),
        )
    }

    /// Its subpaths closed, as filling closes them, of quadratic curves: a line a curve with its
    /// control point halfway, and a cubic curve split into curves no further than `tolerance` px
    /// from it. Segments of no length go.
    pub fn quads(&self, tolerance: f32) -> Vec<Vec<Curve>> {
        let mut contours = Vec::new();
        let mut open = Vec::new();
        let (mut start, mut at) = ([0.0; 2], [0.0; 2]);
        let close = |open: &mut Vec<Curve>, at, start, contours: &mut Vec<Vec<Curve>>| {
            line(open, at, start);
            if !open.is_empty() {
                contours.push(std::mem::take(open));
            }
        };
        for s in &self.0 {
            match *s {
                Segment::Move(p) => {
                    close(&mut open, at, start, &mut contours);
                    start = p;
                }
                Segment::Line(p) => line(&mut open, at, p),
                Segment::Quad(c, p) if at != c || c != p => open.push([at, c, p]),
                Segment::Quad(..) => {}
                Segment::Cubic(c1, c2, p) => cubic([at, c1, c2, p], tolerance, &mut open),
                Segment::Close => close(&mut open, at, start, &mut contours),
            }
            at = match *s {
                Segment::Move(p)
                | Segment::Line(p)
                | Segment::Quad(_, p)
                | Segment::Cubic(_, _, p) => p,
                Segment::Close => start,
            };
        }
        close(&mut open, at, start, &mut contours);
        contours
    }
}

fn line(out: &mut Vec<Curve>, from: Point, to: Point) {
    if from != to {
        out.push([from, mid(from, to), to]);
    }
}

fn mid(a: Point, b: Point) -> Point {
    [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
}

/// A cubic curve as quadratic curves no further than `tolerance` from it: as many pieces of it,
/// one for each even step of its parameter, each with the piece's ends and its control point
/// halfway between where the piece's two cubic control points would put it. One such curve is at
/// most √3/36 of the cubic's third difference from it, and a piece's difference is the whole's
/// over the cube of how many there are.
pub fn cubic(points: [Point; 4], tolerance: f32, out: &mut Vec<Curve>) {
    let [p0, p1, p2, p3] = points.map(|p| p.map(f64::from));
    let d = [0, 1].map(|k| p3[k] - 3.0 * p2[k] + 3.0 * p1[k] - p0[k]);
    let far = 3f64.sqrt() / 36.0 * d[0].hypot(d[1]);
    let n = (far / f64::from(tolerance)).cbrt().ceil().clamp(1.0, 64.0);
    let at = |t: f64| -> [f64; 2] {
        let s = 1.0 - t;
        std::array::from_fn(|k| {
            s * s * s * p0[k]
                + 3.0 * s * s * t * p1[k]
                + 3.0 * s * t * t * p2[k]
                + t * t * t * p3[k]
        })
    };
    let tangent = |t: f64| -> [f64; 2] {
        let s = 1.0 - t;
        std::array::from_fn(|k| {
            3.0 * (s * s * (p1[k] - p0[k])
                + 2.0 * s * t * (p2[k] - p1[k])
                + t * t * (p3[k] - p2[k]))
        })
    };
    let h = 1.0 / n;
    let mut start = points[0];
    for i in 0..n as usize {
        let (t0, t1) = (i as f64 * h, (i + 1) as f64 * h);
        let (a, b) = (at(t0), at(t1));
        let (ta, tb) = (tangent(t0), tangent(t1));
        let c = [0, 1].map(|k| ((2.0 * (a[k] + b[k]) + h * (ta[k] - tb[k])) / 4.0) as f32);
        let end = match i + 1 == n as usize {
            true => points[3],
            false => b.map(|v| v as f32),
        };
        if start != end || c != start {
            out.push([start, c, end]);
        }
        start = end;
    }
}

/// How far a stroke's outer corner reaches, in half its width, before it is cut off: Skia's
/// default, with which Chrome strokes text.
const MITER_LIMIT: f32 = 4.0;

/// The outline of a stroke `width` wide along closed contours, centered on them, as Skia
/// strokes them: each contour moved half the width to its left, and moved to its right and
/// reversed, which wind around what the stroke covers. A curve moves as curves no further than
/// `tolerance` from where it should. At a corner, the outer side meets in a miter, or in a bevel
/// past `MITER_LIMIT`, and the inner side goes through the corner.
pub fn stroke(contours: &[Vec<Curve>], width: f32, tolerance: f32) -> Vec<Vec<Curve>> {
    let mut out = Vec::new();
    for contour in contours {
        let curves: Vec<Curve> = contour.iter().flat_map(|&c| unfolded(c)).collect();
        for d in [width / 2.0, -width / 2.0] {
            let mut side = Vec::new();
            for (i, &c) in curves.iter().enumerate() {
                offset(c, d, tolerance, 0, &mut side);
                let next = curves[(i + 1) % curves.len()];
                join(c[2], tangents(c)[1], tangents(next)[0], d, &mut side);
            }
            if d < 0.0 {
                side.reverse();
                side.iter_mut().for_each(|c| c.swap(0, 2));
            }
            if !side.is_empty() {
                out.push(side);
            }
        }
    }
    out
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

/// `a` moved `d` times along `v`.
fn along(a: Point, v: Point, d: f32) -> Point {
    [a[0] + v[0] * d, a[1] + v[1] * d]
}

fn cross(a: Point, b: Point) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}

fn dot(a: Point, b: Point) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

fn unit(v: Point) -> Option<Point> {
    let l = v[0].hypot(v[1]);
    (l > 1e-6).then(|| [v[0] / l, v[1] / l])
}

/// A direction turned a quarter left.
fn left(v: Point) -> Point {
    [-v[1], v[0]]
}

/// The point of curve `c` at `t`.
fn point(c: Curve, t: f32) -> Point {
    let s = 1.0 - t;
    std::array::from_fn(|k| s * s * c[0][k] + 2.0 * s * t * c[1][k] + t * t * c[2][k])
}

/// The directions a curve starts and ends in, as units.
fn tangents(c: Curve) -> [Point; 2] {
    let [p0, p1, p2] = c;
    let dir = |v, or| unit(v).or(unit(or)).unwrap_or([1.0, 0.0]);
    [dir(sub(p1, p0), sub(p2, p0)), dir(sub(p2, p1), sub(p2, p0))]
}

/// A curve that turns back where it stops, its control point in line with its ends but past
/// one of them, as the two lines on either side of where it stops; other curves as they are.
fn unfolded(c: Curve) -> Vec<Curve> {
    let [p0, p1, p2] = c;
    let (a, b) = (sub(p1, p0), sub(p2, p1));
    let (la, lb) = (a[0].hypot(a[1]), b[0].hypot(b[1]));
    if dot(a, b) >= 0.0 || cross(a, b).abs() > 1e-3 * la * lb {
        return vec![c];
    }
    let tip = point(c, la / (la + lb));
    let mut out = Vec::new();
    line(&mut out, p0, tip);
    line(&mut out, tip, p2);
    out
}

/// Curve `c` moved `d` to its left: a curve from its ends moved along their normals, with its
/// control point where the moved tangents there meet (Tiller and Hanson), or the halves of `c`
/// moved alike where its middle is further than `tolerance` from `c`'s middle moved.
fn offset(c: Curve, d: f32, tolerance: f32, depth: u32, out: &mut Vec<Curve>) {
    let [t0, t1] = tangents(c);
    let (a, b) = (along(c[0], left(t0), d), along(c[2], left(t1), d));
    let turn = cross(t0, t1);
    let control = match turn.abs() < 1e-4 {
        true => mid(a, b),
        false => along(a, t0, cross(sub(b, a), t1) / turn),
    };
    let moved = [a, control, b];
    // The tangent halfway is the chord's.
    let middle = left(unit(sub(c[2], c[0])).unwrap_or(t0));
    let off = sub(point(moved, 0.5), along(point(c, 0.5), middle, d));
    if depth < 4 && off[0].hypot(off[1]) > tolerance {
        let m = point(c, 0.5);
        offset([c[0], mid(c[0], c[1]), m], d, tolerance, depth + 1, out);
        offset([m, mid(c[1], c[2]), c[2]], d, tolerance, depth + 1, out);
        return;
    }
    out.push(moved);
}

/// Where a curve ending in direction `a` meets the next, starting in direction `b`, at `p`: the
/// lines from the first's end moved `d` to its left to the next's start moved alike.
fn join(p: Point, a: Point, b: Point, d: f32, out: &mut Vec<Curve>) {
    let (from, to) = (along(p, left(a), d), along(p, left(b), d));
    let cos = dot(a, b);
    if cos > 1.0 - 1e-6 {
        line(out, from, to);
    } else if cross(a, b) * d > 0.0 {
        // The inner side of a turn, which goes back through the corner.
        line(out, from, p);
        line(out, p, to);
    } else if 1.0 + cos >= 2.0 / (MITER_LIMIT * MITER_LIMIT) {
        // The miter's tip is 1 / cos(θ / 2) out along the normals' bisector.
        let n = [left(a)[0] + left(b)[0], left(a)[1] + left(b)[1]];
        let tip = along(p, n, d / (1.0 + cos));
        line(out, from, tip);
        line(out, tip, to);
    } else {
        line(out, from, to);
    }
}

/// The left and right of where curves are between heights `lo` and `hi`, if they are there: from
/// their ends, where they cross the two heights, and their turns left and right, in that band.
pub fn across(curves: &[Curve], lo: f32, hi: f32) -> Option<[f32; 2]> {
    // Crossings computed in f32 land a little off the heights they cross.
    let near = (hi - lo).abs().max(1.0) * 1e-4;
    let mut out: Option<[f32; 2]> = None;
    for &[p0, p1, p2] in curves {
        let ys = [p0[1], p1[1], p2[1]];
        if ys.iter().all(|&y| y < lo) || ys.iter().all(|&y| y > hi) {
            continue;
        }
        let at = |t: f32, k: usize| {
            let s = 1.0 - t;
            s * s * p0[k] + 2.0 * s * t * p1[k] + t * t * p2[k]
        };
        let mut ts = [0.0, 1.0, f32::NAN, f32::NAN, f32::NAN, f32::NAN, f32::NAN];
        // y = a t² + b t + p0.y, and x likewise.
        let a = p0[1] - 2.0 * p1[1] + p2[1];
        let b = 2.0 * (p1[1] - p0[1]);
        for (k, h) in [lo, hi].into_iter().enumerate() {
            let c = p0[1] - h;
            if a.abs() < 1e-6 {
                if b != 0.0 {
                    ts[2 + 2 * k] = -c / b;
                }
            } else {
                let d = b * b - 4.0 * a * c;
                if d >= 0.0 {
                    let r = d.sqrt();
                    ts[2 + 2 * k] = (-b - r) / (2.0 * a);
                    ts[3 + 2 * k] = (-b + r) / (2.0 * a);
                }
            }
        }
        let ax = p0[0] - 2.0 * p1[0] + p2[0];
        if ax != 0.0 {
            ts[6] = (p0[0] - p1[0]) / ax;
        }
        for t in ts {
            if !(0.0..=1.0).contains(&t) {
                continue;
            }
            let y = at(t, 1);
            if y < lo - near || y > hi + near {
                continue;
            }
            let x = at(t, 0);
            out = Some(match out {
                None => [x, x],
                Some([x0, x1]) => [x0.min(x), x1.max(x)],
            });
        }
    }
    out
}

/// SVG path data (SVG 2, 9.3.9): commands, each a letter, upper case for absolute points and
/// lower case for points relative to where the path is, and their numbers, which may repeat for
/// the command again; a subpath starts with a move.
pub fn parse(d: &str) -> Result<Path, String> {
    commands(d).map(|(path, _)| path)
}

/// As `parse`, with whether each segment ends a command, which puts a vertex there for markers:
/// an arc's quarter turns but its last do not, nor a move a command after a close makes.
pub fn commands(d: &str) -> Result<(Path, Vec<bool>), String> {
    let mut r = Reader {
        b: d.as_bytes(),
        i: 0,
    };
    let bad = |what: &str| format!("{d:?} is not path data: {what}");
    let mut out = Vec::new();
    let mut ends = Vec::new();
    let (mut start, mut at) = ([0.0f32; 2], [0.0f32; 2]);
    // The command before, and the control point a smooth curve after it reflects.
    let mut command = None;
    let mut control: Option<(u8, Point)> = None;
    // Whether the subpath was closed, so that what follows starts another where it started.
    let mut closed = false;
    r.space();
    if r.done() {
        return Err(bad("it is empty"));
    }
    while !r.done() {
        let c = match r.letter() {
            Some(c) => c,
            None => match command {
                Some(b'M') => b'L',
                Some(b'm') => b'l',
                Some(c) if !matches!(c, b'Z' | b'z') => c,
                _ => return Err(bad("numbers need a command before them")),
            },
        };
        if command.is_none() && !matches!(c, b'M' | b'm') {
            return Err(bad("it starts with a move, M or m"));
        }
        let relative = c.is_ascii_lowercase();
        let upper = c.to_ascii_uppercase();
        let name = c as char;
        let number = |r: &mut Reader| {
            r.number()
                .ok_or_else(|| bad(&format!("{name} needs numbers")))
        };
        let point = |r: &mut Reader, from: Point| -> Result<Point, String> {
            let p = [number(r)?, number(r)?];
            Ok(match relative {
                true => [from[0] + p[0], from[1] + p[1]],
                false => p,
            })
        };
        if closed && upper != b'M' && upper != b'Z' {
            out.push(Segment::Move(start));
        }
        closed = false;
        let reflected = |kind: u8| match control {
            Some((k, [x, y])) if k == kind => [2.0 * at[0] - x, 2.0 * at[1] - y],
            _ => at,
        };
        let mut next = None;
        match upper {
            b'M' => {
                let p = point(&mut r, at)?;
                out.push(Segment::Move(p));
                (start, at) = (p, p);
            }
            b'L' => {
                at = point(&mut r, at)?;
                out.push(Segment::Line(at));
            }
            b'H' | b'V' => {
                let v = number(&mut r)?;
                let k = usize::from(upper == b'V');
                at[k] = if relative { at[k] + v } else { v };
                out.push(Segment::Line(at));
            }
            b'C' | b'S' => {
                let c1 = match upper {
                    b'C' => point(&mut r, at)?,
                    _ => reflected(b'C'),
                };
                let c2 = point(&mut r, at)?;
                at = point(&mut r, at)?;
                out.push(Segment::Cubic(c1, c2, at));
                next = Some((b'C', c2));
            }
            b'Q' | b'T' => {
                let c = match upper {
                    b'Q' => point(&mut r, at)?,
                    _ => reflected(b'Q'),
                };
                at = point(&mut r, at)?;
                out.push(Segment::Quad(c, at));
                next = Some((b'Q', c));
            }
            b'A' => {
                let radii = [number(&mut r)?, number(&mut r)?];
                let turn = number(&mut r)?;
                let [Some(large), Some(sweep)] = [r.flag(), r.flag()] else {
                    return Err(bad("A needs flags, 0 or 1, after its radii and angle"));
                };
                let to = point(&mut r, at)?;
                arc(&mut out, at, radii, turn, large, sweep, to);
                at = to;
            }
            b'Z' => {
                out.push(Segment::Close);
                at = start;
                closed = true;
            }
            _ => {
                return Err(bad(&format!(
                    "{name} is not a command: M, L, H, V, C, S, Q, T, A or Z"
                )));
            }
        }
        command = Some(c);
        control = next;
        ends.resize(out.len(), false);
        if let Some(end) = ends.last_mut() {
            *end = true;
        }
    }
    Ok((Path(out), ends))
}

/// Numbers separated by white space or a comma, as SVG's `points` and `viewBox` list them; `None`
/// unless that is all `s` is.
pub fn numbers(s: &str) -> Option<Vec<f32>> {
    let mut r = Reader {
        b: s.as_bytes(),
        i: 0,
    };
    r.space();
    let mut out = Vec::new();
    while !r.done() {
        out.push(r.number()?);
    }
    Some(out)
}

/// SVG path data's characters.
struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn done(&self) -> bool {
        self.i >= self.b.len()
    }

    fn peek(&self) -> u8 {
        self.b.get(self.i).copied().unwrap_or(0)
    }

    fn space(&mut self) {
        while matches!(self.peek(), b' ' | b'\t' | b'\n' | b'\r' | b'\x0c') {
            self.i += 1;
        }
    }

    /// Past whitespace and a comma, if there is one, between numbers.
    fn separator(&mut self) {
        self.space();
        if self.peek() == b',' {
            self.i += 1;
            self.space();
        }
    }

    /// A command's letter, if one is next.
    fn letter(&mut self) -> Option<u8> {
        let c = self.peek();
        c.is_ascii_alphabetic().then(|| {
            self.i += 1;
            self.space();
            c
        })
    }

    /// A number: a sign, digits with a decimal point or not, and an exponent; where one ends and
    /// the next starts needs no separator when it is clear, as in `1-2` or `.5.5`.
    fn number(&mut self) -> Option<f32> {
        let from = self.i;
        let digits = |r: &mut Self| {
            let at = r.i;
            while r.peek().is_ascii_digit() {
                r.i += 1;
            }
            r.i > at
        };
        if matches!(self.peek(), b'+' | b'-') {
            self.i += 1;
        }
        let whole = digits(self);
        let fraction = self.peek() == b'.' && {
            self.i += 1;
            digits(self)
        };
        if !whole && !fraction {
            self.i = from;
            return None;
        }
        if matches!(self.peek(), b'e' | b'E') {
            let at = self.i;
            self.i += 1;
            if matches!(self.peek(), b'+' | b'-') {
                self.i += 1;
            }
            if !digits(self) {
                self.i = at;
            }
        }
        let text = std::str::from_utf8(&self.b[from..self.i]).ok()?;
        let v = text.parse::<f32>().ok().filter(|v| v.is_finite())?;
        self.separator();
        Some(v)
    }

    /// An arc's flag, one character, 0 or 1.
    fn flag(&mut self) -> Option<bool> {
        let f = match self.peek() {
            b'0' => false,
            b'1' => true,
            _ => return None,
        };
        self.i += 1;
        self.separator();
        Some(f)
    }
}

/// An elliptical arc from `from` to `to` (SVG 2, B.2.4) as cubic curves, each a quarter turn at
/// most: radii too small to reach grow until they do, and a radius of 0 makes it a line.
fn arc(
    out: &mut Vec<Segment>,
    from: Point,
    radii: [f32; 2],
    degrees: f32,
    large: bool,
    sweep: bool,
    to: Point,
) {
    if from == to {
        return;
    }
    let [mut rx, mut ry] = radii.map(|r| f64::from(r.abs()));
    if rx == 0.0 || ry == 0.0 {
        out.push(Segment::Line(to));
        return;
    }
    let [x1, y1] = from.map(f64::from);
    let [x2, y2] = to.map(f64::from);
    let (sin, cos) = f64::from(degrees).to_radians().sin_cos();
    let [dx, dy] = [(x1 - x2) / 2.0, (y1 - y2) / 2.0];
    let [x, y] = [cos * dx + sin * dy, -sin * dx + cos * dy];
    let reach = (x * x) / (rx * rx) + (y * y) / (ry * ry);
    if reach > 1.0 {
        rx *= reach.sqrt();
        ry *= reach.sqrt();
    }
    let (rx2, ry2) = (rx * rx, ry * ry);
    let num = rx2 * ry2 - rx2 * y * y - ry2 * x * x;
    let den = rx2 * y * y + ry2 * x * x;
    let mut k = (num / den).max(0.0).sqrt();
    if large == sweep {
        k = -k;
    }
    let [cx, cy] = [k * rx * y / ry, -k * ry * x / rx];
    let center = [
        cos * cx - sin * cy + (x1 + x2) / 2.0,
        sin * cx + cos * cy + (y1 + y2) / 2.0,
    ];
    let angle =
        |u: [f64; 2], v: [f64; 2]| (u[0] * v[1] - u[1] * v[0]).atan2(u[0] * v[0] + u[1] * v[1]);
    let u = [(x - cx) / rx, (y - cy) / ry];
    let v = [(-x - cx) / rx, (-y - cy) / ry];
    let first = angle([1.0, 0.0], u);
    let mut turn = angle(u, v);
    let tau = std::f64::consts::TAU;
    if !sweep && turn > 0.0 {
        turn -= tau;
    } else if sweep && turn < 0.0 {
        turn += tau;
    }
    let n = (turn.abs() / (tau / 4.0)).ceil().max(1.0);
    let step = turn / n;
    let t = 4.0 / 3.0 * (step / 4.0).tan();
    // A point of the unit circle, and the ellipse's point for it.
    let placed = |[a, b]: [f64; 2]| -> Point {
        [
            (center[0] + cos * rx * a - sin * ry * b) as f32,
            (center[1] + sin * rx * a + cos * ry * b) as f32,
        ]
    };
    for i in 0..n as usize {
        let a0 = first + i as f64 * step;
        let a1 = a0 + step;
        let (s0, c0) = a0.sin_cos();
        let (s1, c1) = a1.sin_cos();
        let end = if i + 1 == n as usize {
            to
        } else {
            placed([c1, s1])
        };
        out.push(Segment::Cubic(
            placed([c0 - t * s0, s0 + t * c0]),
            placed([c1 + t * s1, s1 - t * c1]),
            end,
        ));
    }
}

pub mod edit;

#[cfg(test)]
mod tests;
