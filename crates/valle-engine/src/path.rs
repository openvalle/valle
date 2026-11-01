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
}

/// Segments from where the one before ends, or from the origin.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path(pub Vec<Segment>);

impl Path {
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
            }
            at = match *s {
                Segment::Move(p)
                | Segment::Line(p)
                | Segment::Quad(_, p)
                | Segment::Cubic(_, _, p) => p,
            };
        }
        close(&mut open, at, start, &mut contours);
        contours
    }
}

fn line(out: &mut Vec<Curve>, from: Point, to: Point) {
    if from != to {
        let mid = [(from[0] + to[0]) * 0.5, (from[1] + to[1]) * 0.5];
        out.push([from, mid, to]);
    }
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

/// SVG path data (SVG 2, 9.3.9): commands, each a letter, upper case for absolute points and
/// lower case for points relative to where the path is, and their numbers, which may repeat for
/// the command again; a subpath starts with a move.
pub fn parse(d: &str) -> Result<Path, String> {
    let mut r = Reader {
        b: d.as_bytes(),
        i: 0,
    };
    let bad = |what: &str| format!("{d:?} is not path data: {what}");
    let mut out = Vec::new();
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
    }
    Ok(Path(out))
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

#[cfg(test)]
mod tests;
