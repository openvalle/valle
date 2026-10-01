//! Exact event predicates for a linearly interpolated polygon.
//!
//! Input f64 coordinates are dyadic rationals. We put every coordinate on one
//! integer grid, then keep all event polynomials integral. Quadratic roots are
//! compared algebraically: a numerical root estimate is used only in diagnostics.

use super::{PathData, Point};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{Signed, ToPrimitive, Zero};
use std::cmp::Ordering;
use valle_draw::PathVerb;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MorphIssueKind {
    InvalidEndpoint,
    ZeroEdge,
    Spike,
    Crossing,
    Contact,
    AreaCollapse,
}

#[derive(Debug, Clone)]
pub(super) struct MorphIssue {
    pub kind: MorphIssueKind,
    pub from: f64,
    pub through: f64,
    pub vertex: usize,
    pub edge: usize,
}

fn record_event(
    problem: MorphIssue,
    contact_is_error: bool,
    first_contact: &mut Option<MorphIssue>,
) -> Result<(), MorphIssue> {
    if problem.kind == MorphIssueKind::Contact && !contact_is_error {
        if first_contact.is_none() {
            *first_contact = Some(problem);
        }
        Ok(())
    } else {
        Err(problem)
    }
}

impl core::fmt::Display for MorphIssue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "linear morph {:?} at t in [{:.12}, {:.12}], vertex {}, edge {}",
            self.kind, self.from, self.through, self.vertex, self.edge
        )
    }
}

#[derive(Clone)]
struct Linear {
    start: BigInt,
    delta: BigInt,
}

impl Linear {
    fn subtract(&self, other: &Self) -> Self {
        Self {
            start: &self.start - &other.start,
            delta: &self.delta - &other.delta,
        }
    }

    fn at_endpoint(&self, end: bool) -> BigInt {
        if end {
            &self.start + &self.delta
        } else {
            self.start.clone()
        }
    }
}

#[derive(Clone)]
struct MovingPoint {
    x: Linear,
    y: Linear,
}

impl MovingPoint {
    fn subtract(&self, other: &Self) -> Self {
        Self {
            x: self.x.subtract(&other.x),
            y: self.y.subtract(&other.y),
        }
    }

    fn at_endpoint(&self, end: bool) -> IntegerPoint {
        IntegerPoint {
            x: self.x.at_endpoint(end),
            y: self.y.at_endpoint(end),
        }
    }
}

#[derive(Clone)]
struct IntegerPoint {
    x: BigInt,
    y: BigInt,
}

#[derive(Clone)]
struct AxisBounds {
    min: BigInt,
    max: BigInt,
}

impl AxisBounds {
    fn moving(value: &Linear) -> Self {
        let end = value.at_endpoint(true);
        Self {
            min: value.start.clone().min(end.clone()),
            max: value.start.clone().max(end),
        }
    }

    fn union(&self, other: &Self) -> Self {
        Self {
            min: self.min.clone().min(other.min.clone()),
            max: self.max.clone().max(other.max.clone()),
        }
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.min <= other.max && other.min <= self.max
    }
}

#[derive(Clone)]
struct PointBounds {
    x: AxisBounds,
    y: AxisBounds,
}

impl PointBounds {
    fn moving(point: &MovingPoint) -> Self {
        Self {
            x: AxisBounds::moving(&point.x),
            y: AxisBounds::moving(&point.y),
        }
    }

    fn union(&self, other: &Self) -> Self {
        Self {
            x: self.x.union(&other.x),
            y: self.y.union(&other.y),
        }
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.x.overlaps(&other.x) && self.y.overlaps(&other.y)
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Polynomial {
    c0: BigInt,
    c1: BigInt,
    c2: BigInt,
}

impl Polynomial {
    fn zero() -> Self {
        Self {
            c0: BigInt::zero(),
            c1: BigInt::zero(),
            c2: BigInt::zero(),
        }
    }

    fn is_zero(&self) -> bool {
        self.c0.is_zero() && self.c1.is_zero() && self.c2.is_zero()
    }

    fn add(&mut self, other: &Self) {
        self.c0 += &other.c0;
        self.c1 += &other.c1;
        self.c2 += &other.c2;
    }

    fn at_rational(&self, t: &BigRational) -> BigInt {
        let n = t.numer();
        let d = t.denom();
        &self.c0 * d * d + &self.c1 * n * d + &self.c2 * n * n
    }

    fn at_endpoint(&self, end: bool) -> BigInt {
        if end {
            &self.c0 + &self.c1 + &self.c2
        } else {
            self.c0.clone()
        }
    }

    fn strictly_one_side_on_unit(&self) -> bool {
        // Quadratic Bernstein coefficients have one strict sign, so no root can
        // exist anywhere in [0,1]. This exact rejection avoids most root and
        // point-on-segment work for well separated vertex/edge pairs.
        let middle_twice = BigInt::from(2) * &self.c0 + &self.c1;
        let end = self.at_endpoint(true);
        (self.c0.is_positive() && middle_twice.is_positive() && end.is_positive())
            || (self.c0.is_negative() && middle_twice.is_negative() && end.is_negative())
    }

    fn roots(&self) -> Vec<Root> {
        if self.c2.is_zero() {
            return if self.c1.is_zero() {
                Vec::new()
            } else {
                vec![Root::Rational(BigRational::new(-&self.c0, self.c1.clone()))]
            };
        }
        let mut a = self.c2.clone();
        let mut b = self.c1.clone();
        let mut c = self.c0.clone();
        if a.is_negative() {
            a = -a;
            b = -b;
            c = -c;
        }
        let discriminant = &b * &b - BigInt::from(4) * &a * &c;
        if discriminant.is_negative() {
            Vec::new()
        } else if discriminant.is_zero() {
            vec![Root::Rational(BigRational::new(-b, BigInt::from(2) * a))]
        } else {
            vec![
                Root::Quadratic {
                    a: a.clone(),
                    b: b.clone(),
                    c: c.clone(),
                    high: false,
                },
                Root::Quadratic {
                    a,
                    b,
                    c,
                    high: true,
                },
            ]
        }
    }
}

fn product(left: &Linear, right: &Linear) -> Polynomial {
    Polynomial {
        c0: &left.start * &right.start,
        c1: &left.start * &right.delta + &left.delta * &right.start,
        c2: &left.delta * &right.delta,
    }
}

fn cross(left: &MovingPoint, right: &MovingPoint) -> Polynomial {
    let xy = product(&left.x, &right.y);
    let yx = product(&left.y, &right.x);
    Polynomial {
        c0: xy.c0 - yx.c0,
        c1: xy.c1 - yx.c1,
        c2: xy.c2 - yx.c2,
    }
}

fn dot(left: &MovingPoint, right: &MovingPoint) -> Polynomial {
    let xx = product(&left.x, &right.x);
    let yy = product(&left.y, &right.y);
    Polynomial {
        c0: xx.c0 + yy.c0,
        c1: xx.c1 + yy.c1,
        c2: xx.c2 + yy.c2,
    }
}

#[derive(Clone)]
enum Root {
    Rational(BigRational),
    // a > 0, discriminant > 0; high picks the larger root.
    Quadratic {
        a: BigInt,
        b: BigInt,
        c: BigInt,
        high: bool,
    },
}

impl Root {
    fn zero() -> Self {
        Self::Rational(BigRational::from_integer(BigInt::zero()))
    }

    fn one() -> Self {
        Self::Rational(BigRational::from_integer(BigInt::from(1)))
    }

    fn cmp_rational(&self, value: &BigRational) -> Ordering {
        match self {
            Self::Rational(root) => root.cmp(value),
            Self::Quadratic { a, b, c, high } => {
                let vertex = BigRational::new(-b.clone(), BigInt::from(2) * a);
                let side = value.cmp(&vertex);
                if side == Ordering::Equal {
                    return if *high {
                        Ordering::Greater
                    } else {
                        Ordering::Less
                    };
                }
                if (*high && side == Ordering::Less) || (!*high && side == Ordering::Greater) {
                    return if *high {
                        Ordering::Greater
                    } else {
                        Ordering::Less
                    };
                }
                let polynomial = Polynomial {
                    c0: c.clone(),
                    c1: b.clone(),
                    c2: a.clone(),
                };
                match polynomial.at_rational(value).cmp(&BigInt::zero()) {
                    Ordering::Equal => Ordering::Equal,
                    Ordering::Greater => {
                        if *high {
                            Ordering::Less
                        } else {
                            Ordering::Greater
                        }
                    }
                    Ordering::Less => {
                        if *high {
                            Ordering::Greater
                        } else {
                            Ordering::Less
                        }
                    }
                }
            }
        }
    }

    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Rational(left), Self::Rational(right)) => left.cmp(right),
            (_, Self::Rational(right)) => self.cmp_rational(right),
            (Self::Rational(left), _) => other.cmp_rational(left).reverse(),
            (
                Self::Quadratic { a, b, c, high },
                Self::Quadratic {
                    a: other_a,
                    b: other_b,
                    c: other_c,
                    high: other_high,
                },
            ) => {
                // A common irrational root implies proportional rational quadratics.
                let linear = other_a * b - a * other_b;
                let constant = other_a * c - a * other_c;
                if linear.is_zero() && constant.is_zero() {
                    return high.cmp(other_high);
                }
                if !linear.is_zero() {
                    let candidate = BigRational::new(-constant, linear);
                    if self.cmp_rational(&candidate) == Ordering::Equal
                        && other.cmp_rational(&candidate) == Ordering::Equal
                    {
                        return Ordering::Equal;
                    }
                }
                let mut lo_left = BigRational::from_integer(BigInt::zero());
                let mut hi_left = BigRational::from_integer(BigInt::from(1));
                let mut lo_right = lo_left.clone();
                let mut hi_right = hi_left.clone();
                loop {
                    if hi_left <= lo_right {
                        return Ordering::Less;
                    }
                    if hi_right <= lo_left {
                        return Ordering::Greater;
                    }
                    refine(self, &mut lo_left, &mut hi_left);
                    refine(other, &mut lo_right, &mut hi_right);
                }
            }
        }
    }

    fn in_open_unit(&self) -> bool {
        self.cmp(&Self::zero()) == Ordering::Greater && self.cmp(&Self::one()) == Ordering::Less
    }

    fn in_closed_unit(&self) -> bool {
        self.cmp(&Self::zero()) != Ordering::Less && self.cmp(&Self::one()) != Ordering::Greater
    }

    fn approximate(&self) -> f64 {
        if let Self::Rational(value) = self {
            if let Some(approximation) = value.to_f64() {
                return approximation;
            }
        }
        let mut lo = BigRational::from_integer(BigInt::zero());
        let mut hi = BigRational::from_integer(BigInt::from(1));
        for _ in 0..64 {
            refine(self, &mut lo, &mut hi);
        }
        ((lo + hi) / BigInt::from(2)).to_f64().unwrap_or(0.0)
    }
}

fn refine(root: &Root, lo: &mut BigRational, hi: &mut BigRational) {
    let midpoint = (lo.clone() + hi.clone()) / BigInt::from(2);
    match root.cmp_rational(&midpoint) {
        Ordering::Less => *hi = midpoint,
        Ordering::Equal => {
            *lo = midpoint.clone();
            *hi = midpoint;
        }
        Ordering::Greater => *lo = midpoint,
    }
}

fn sign_at_root(polynomial: &Polynomial, root: &Root) -> Ordering {
    match root {
        Root::Rational(value) => polynomial.at_rational(value).cmp(&BigInt::zero()),
        Root::Quadratic { a, b, c, .. } => {
            let linear = &polynomial.c1 * a - &polynomial.c2 * b;
            let constant = &polynomial.c0 * a - &polynomial.c2 * c;
            if linear.is_zero() {
                return constant.cmp(&BigInt::zero());
            }
            let threshold = BigRational::new(-constant, linear.clone());
            let order = root.cmp_rational(&threshold);
            if linear.is_negative() {
                order.reverse()
            } else {
                order
            }
        }
    }
}

fn point_grid(from: &[Point], to: &[Point]) -> Vec<MovingPoint> {
    let coordinates = from
        .iter()
        .chain(to)
        .flat_map(|point| [point.x, point.y])
        .map(|value| BigRational::from_float(value).expect("validated finite coordinate"))
        .collect::<Vec<_>>();
    let scale = coordinates
        .iter()
        .map(|value| value.denom())
        .max()
        .cloned()
        .unwrap_or_else(|| BigInt::from(1));
    let integers = coordinates
        .iter()
        .map(|value| value.numer() * (&scale / value.denom()))
        .collect::<Vec<_>>();
    let count = from.len();
    (0..count)
        .map(|index| {
            let coordinate = |offset: usize| {
                let start = integers[index * 2 + offset].clone();
                Linear {
                    delta: &integers[(index + count) * 2 + offset] - &start,
                    start,
                }
            };
            MovingPoint {
                x: coordinate(0),
                y: coordinate(1),
            }
        })
        .collect()
}

fn rational_between(left: &Root, right: &Root) -> BigRational {
    let mut lo = BigRational::from_integer(BigInt::zero());
    let mut hi = BigRational::from_integer(BigInt::from(1));
    loop {
        let middle = (lo.clone() + hi.clone()) / BigInt::from(2);
        let from_left = left.cmp_rational(&middle);
        let from_right = right.cmp_rational(&middle);
        if from_left == Ordering::Less && from_right == Ordering::Greater {
            return middle;
        }
        if from_left != Ordering::Less {
            lo = middle;
        } else {
            hi = middle;
        }
    }
}

fn unit_partition(polynomials: &[&Polynomial]) -> Vec<Root> {
    let mut roots = vec![Root::zero(), Root::one()];
    for polynomial in polynomials {
        roots.extend(polynomial.roots().into_iter().filter(Root::in_closed_unit));
    }
    roots.sort_by(Root::cmp);
    roots.dedup_by(|a, b| a.cmp(b) == Ordering::Equal);
    roots
}

fn nonnegative_overlap(first: &Polynomial, second: &Polynomial) -> Option<(Root, Root)> {
    let roots = unit_partition(&[first, second]);
    for pair in roots.windows(2) {
        if pair[0].cmp(&pair[1]) != Ordering::Less {
            continue;
        }
        let middle = rational_between(&pair[0], &pair[1]);
        if first.at_rational(&middle) >= BigInt::zero()
            && second.at_rational(&middle) >= BigInt::zero()
        {
            return Some((pair[0].clone(), pair[1].clone()));
        }
    }
    for root in roots {
        if root.in_open_unit()
            && sign_at_root(first, &root) != Ordering::Less
            && sign_at_root(second, &root) != Ordering::Less
        {
            return Some((root.clone(), root));
        }
    }
    None
}

fn positive_overlap(polynomial: &Polynomial) -> Option<(Root, Root)> {
    let roots = unit_partition(&[polynomial]);
    for pair in roots.windows(2) {
        if pair[0].cmp(&pair[1]) == Ordering::Less {
            let middle = rational_between(&pair[0], &pair[1]);
            if polynomial.at_rational(&middle) > BigInt::zero() {
                return Some((pair[0].clone(), pair[1].clone()));
            }
        }
    }
    None
}

fn issue(
    kind: MorphIssueKind,
    from: &Root,
    through: &Root,
    vertex: usize,
    edge: usize,
) -> MorphIssue {
    MorphIssue {
        kind,
        from: from.approximate(),
        through: through.approximate(),
        vertex,
        edge,
    }
}

fn orientation(a: &IntegerPoint, b: &IntegerPoint, c: &IntegerPoint) -> BigInt {
    (&b.x - &a.x) * (&c.y - &a.y) - (&b.y - &a.y) * (&c.x - &a.x)
}

fn point_on_segment(p: &IntegerPoint, a: &IntegerPoint, b: &IntegerPoint) -> bool {
    orientation(a, b, p).is_zero()
        && ((&p.x - &a.x) * (&p.x - &b.x) + (&p.y - &a.y) * (&p.y - &b.y)) <= BigInt::zero()
}

fn segments_intersect(
    a: &IntegerPoint,
    b: &IntegerPoint,
    c: &IntegerPoint,
    d: &IntegerPoint,
) -> bool {
    let ab_c = orientation(a, b, c);
    let ab_d = orientation(a, b, d);
    let cd_a = orientation(c, d, a);
    let cd_b = orientation(c, d, b);
    (ab_c.signum() != ab_d.signum()
        && !ab_c.is_zero()
        && !ab_d.is_zero()
        && cd_a.signum() != cd_b.signum()
        && !cd_a.is_zero()
        && !cd_b.is_zero())
        || (ab_c.is_zero() && point_on_segment(c, a, b))
        || (ab_d.is_zero() && point_on_segment(d, a, b))
        || (cd_a.is_zero() && point_on_segment(a, c, d))
        || (cd_b.is_zero() && point_on_segment(b, c, d))
}

fn simple_at_endpoint(points: &[MovingPoint], end: bool, allowed_zero: &[usize]) -> bool {
    let original = points
        .iter()
        .map(|point| point.at_endpoint(end))
        .collect::<Vec<_>>();
    let count = original.len();
    for edge in 0..count {
        let next = (edge + 1) % count;
        let zero = original[edge].x == original[next].x && original[edge].y == original[next].y;
        if zero != allowed_zero.contains(&edge) {
            return false;
        }
    }
    let fixed = (0..count)
        .filter(|vertex| !allowed_zero.contains(&((vertex + count - 1) % count)))
        .map(|vertex| original[vertex].clone())
        .collect::<Vec<_>>();
    let count = fixed.len();
    if count < 3 {
        return false;
    }
    for vertex in 0..count {
        let prev = &fixed[(vertex + count - 1) % count];
        let center = &fixed[vertex];
        let next = &fixed[(vertex + 1) % count];
        if orientation(prev, center, next).is_zero()
            && ((&prev.x - &center.x) * (&next.x - &center.x)
                + (&prev.y - &center.y) * (&next.y - &center.y))
                > BigInt::zero()
        {
            return false;
        }
    }
    for edge in 0..count {
        let next = (edge + 1) % count;
        for other in (edge + 2)..count {
            if edge == 0 && other == count - 1 {
                continue;
            }
            let other_next = (other + 1) % count;
            if segments_intersect(
                &fixed[edge],
                &fixed[next],
                &fixed[other],
                &fixed[other_next],
            ) {
                return false;
            }
        }
    }
    true
}

fn vertex_edge_event(
    points: &[MovingPoint],
    vertex: usize,
    edge: usize,
    edge_next: usize,
) -> Option<MorphIssue> {
    let a = points[edge].subtract(&points[vertex]);
    let b = points[edge_next].subtract(&points[vertex]);
    let crossing = cross(&a, &b);
    if crossing.strictly_one_side_on_unit() {
        return None;
    }
    let segment = b.subtract(&a);
    let v_minus_a = points[vertex].subtract(&points[edge]);
    let v_minus_b = points[vertex].subtract(&points[edge_next]);
    let within_a = dot(&v_minus_a, &segment);
    let reverse_segment = a.subtract(&b);
    let within_b = dot(&v_minus_b, &reverse_segment);
    if crossing.is_zero() {
        if let Some((start, end)) = nonnegative_overlap(&within_a, &within_b) {
            return Some(issue(MorphIssueKind::Contact, &start, &end, vertex, edge));
        }
    } else {
        let double_root = !crossing.c2.is_zero()
            && &crossing.c1 * &crossing.c1 == BigInt::from(4) * &crossing.c2 * &crossing.c0;
        for time in crossing.roots().into_iter().filter(Root::in_open_unit) {
            if sign_at_root(&within_a, &time) != Ordering::Less
                && sign_at_root(&within_b, &time) != Ordering::Less
            {
                let kind = if double_root {
                    MorphIssueKind::Contact
                } else {
                    MorphIssueKind::Crossing
                };
                return Some(issue(kind, &time, &time, vertex, edge));
            }
        }
    }
    None
}

pub(super) fn simple_closed_contour(points: &[Point]) -> bool {
    closed_contour_winding(points).is_some()
}

pub(super) fn closed_contour_winding(points: &[Point]) -> Option<Ordering> {
    if points.len() < 3 {
        return None;
    }
    let moving = point_grid(points, points);
    if !simple_at_endpoint(&moving, false, &[]) {
        return None;
    }
    let mut area = Polynomial::zero();
    for edge in 0..moving.len() {
        area.add(&cross(&moving[edge], &moving[(edge + 1) % moving.len()]));
    }
    match area.at_endpoint(false).cmp(&BigInt::zero()) {
        Ordering::Equal => None,
        winding => Some(winding),
    }
}

pub(super) struct ContourHierarchy {
    pub parent: Vec<Option<usize>>,
    pub depth: Vec<usize>,
}

fn inside_polygon(point: &IntegerPoint, polygon: &[IntegerPoint]) -> bool {
    let mut inside = false;
    for edge in 0..polygon.len() {
        let a = &polygon[edge];
        let b = &polygon[(edge + 1) % polygon.len()];
        if (a.y > point.y) == (b.y > point.y) {
            continue;
        }
        let turn = orientation(a, b, point);
        if (b.y > a.y && turn.is_positive()) || (b.y < a.y && turn.is_negative()) {
            inside = !inside;
        }
    }
    inside
}

pub(super) fn strictly_inside_contour(contour: &[Point], point: Point) -> bool {
    let mut coordinates = contour.to_vec();
    coordinates.push(point);
    let fixed = point_grid(&coordinates, &coordinates)
        .into_iter()
        .map(|value| value.at_endpoint(false))
        .collect::<Vec<_>>();
    let candidate = fixed.last().unwrap();
    let polygon = &fixed[..contour.len()];
    (0..polygon.len()).all(|edge| {
        !point_on_segment(
            candidate,
            &polygon[edge],
            &polygon[(edge + 1) % polygon.len()],
        )
    }) && inside_polygon(candidate, polygon)
}

/// Exact nesting and boundary separation for independently simple closed contours.
pub(super) fn contour_hierarchy(contours: &[Vec<Point>]) -> Option<ContourHierarchy> {
    if contours.is_empty() || contours.iter().any(|points| !simple_closed_contour(points)) {
        return None;
    }
    let flat = contours.iter().flatten().copied().collect::<Vec<_>>();
    let fixed = point_grid(&flat, &flat)
        .into_iter()
        .map(|point| point.at_endpoint(false))
        .collect::<Vec<_>>();
    let mut ranges = Vec::with_capacity(contours.len());
    let mut start = 0;
    for contour in contours {
        ranges.push(start..start + contour.len());
        start += contour.len();
    }
    let bounds = ranges
        .iter()
        .map(|range| {
            let first = &fixed[range.start];
            range.clone().fold(
                (
                    first.x.clone(),
                    first.x.clone(),
                    first.y.clone(),
                    first.y.clone(),
                ),
                |(min_x, max_x, min_y, max_y), index| {
                    let point = &fixed[index];
                    (
                        min_x.min(point.x.clone()),
                        max_x.max(point.x.clone()),
                        min_y.min(point.y.clone()),
                        max_y.max(point.y.clone()),
                    )
                },
            )
        })
        .collect::<Vec<_>>();
    let areas = ranges
        .iter()
        .map(|range| {
            let polygon = &fixed[range.clone()];
            (0..polygon.len())
                .map(|at| {
                    &polygon[at].x * &polygon[(at + 1) % polygon.len()].y
                        - &polygon[at].y * &polygon[(at + 1) % polygon.len()].x
                })
                .sum::<BigInt>()
                .abs()
        })
        .collect::<Vec<_>>();
    for left in 0..contours.len() {
        for right in left + 1..contours.len() {
            let a = &bounds[left];
            let b = &bounds[right];
            if a.0 > b.1 || b.0 > a.1 || a.2 > b.3 || b.2 > a.3 {
                continue;
            }
            let a_points = &fixed[ranges[left].clone()];
            let b_points = &fixed[ranges[right].clone()];
            for edge_a in 0..a_points.len() {
                for edge_b in 0..b_points.len() {
                    if segments_intersect(
                        &a_points[edge_a],
                        &a_points[(edge_a + 1) % a_points.len()],
                        &b_points[edge_b],
                        &b_points[(edge_b + 1) % b_points.len()],
                    ) {
                        return None;
                    }
                }
            }
        }
    }
    let mut parent = vec![None; contours.len()];
    for child in 0..contours.len() {
        let point = &fixed[ranges[child].start];
        for outer in 0..contours.len() {
            if child == outer || areas[outer] <= areas[child] {
                continue;
            }
            if inside_polygon(point, &fixed[ranges[outer].clone()])
                && parent[child].is_none_or(|current| areas[outer] < areas[current])
            {
                parent[child] = Some(outer);
            }
        }
    }
    let mut depth = vec![0; contours.len()];
    for child in 0..contours.len() {
        let mut next = parent[child];
        while let Some(outer) = next {
            depth[child] += 1;
            if depth[child] >= contours.len() {
                return None;
            }
            next = parent[outer];
        }
    }
    Some(ContourHierarchy { parent, depth })
}

/// Certifies that two separately simple moving contours never touch one another.
/// A collapsed endpoint is permitted for a contour born from or shrinking to a point.
#[cfg(test)]
pub(super) fn check_contour_pair_morph(
    from_a: &[Point],
    to_a: &[Point],
    from_b: &[Point],
    to_b: &[Point],
    a_collapsed: [bool; 2],
    b_collapsed: [bool; 2],
) -> Result<(), MorphIssue> {
    scan_contour_pair_morph(from_a, to_a, from_b, to_b, a_collapsed, b_collapsed, true).map(|_| ())
}

pub(super) fn scan_contour_pair_morph(
    from_a: &[Point],
    to_a: &[Point],
    from_b: &[Point],
    to_b: &[Point],
    a_collapsed: [bool; 2],
    b_collapsed: [bool; 2],
    contact_is_error: bool,
) -> Result<Option<MorphIssue>, MorphIssue> {
    if a_collapsed == [true, true] || b_collapsed == [true, true] {
        return Ok(None);
    }
    let from = from_a.iter().chain(from_b).copied().collect::<Vec<_>>();
    let to = to_a.iter().chain(to_b).copied().collect::<Vec<_>>();
    let points = point_grid(&from, &to);
    let split = from_a.len();
    let vertex_bounds = points.iter().map(PointBounds::moving).collect::<Vec<_>>();
    let contour_bounds = |range: std::ops::Range<usize>| {
        (range.start + 1..range.end).fold(vertex_bounds[range.start].clone(), |bounds, at| {
            bounds.union(&vertex_bounds[at])
        })
    };
    if !contour_bounds(0..split).overlaps(&contour_bounds(split..points.len())) {
        return Ok(None);
    }
    let mut first_contact = None;
    for end in [false, true] {
        let side = usize::from(end);
        if a_collapsed[side] || b_collapsed[side] {
            continue;
        }
        for a in 0..split {
            let a_next = (a + 1) % split;
            for b in split..points.len() {
                let b_next = split + (b + 1 - split) % (points.len() - split);
                if segments_intersect(
                    &points[a].at_endpoint(end),
                    &points[a_next].at_endpoint(end),
                    &points[b].at_endpoint(end),
                    &points[b_next].at_endpoint(end),
                ) {
                    let time = if end { Root::one() } else { Root::zero() };
                    record_event(
                        issue(MorphIssueKind::Contact, &time, &time, a, b),
                        contact_is_error,
                        &mut first_contact,
                    )?;
                }
            }
        }
    }
    for (vertices, edges) in [
        (0..split, split..points.len()),
        (split..points.len(), 0..split),
    ] {
        for vertex in vertices {
            for edge in edges.clone() {
                let edge_next = if edge + 1 == edges.end {
                    edges.start
                } else {
                    edge + 1
                };
                let edge_bounds = vertex_bounds[edge].union(&vertex_bounds[edge_next]);
                if vertex_bounds[vertex].overlaps(&edge_bounds)
                    && let Some(problem) = vertex_edge_event(&points, vertex, edge, edge_next)
                {
                    record_event(problem, contact_is_error, &mut first_contact)?;
                }
            }
        }
    }
    Ok(first_contact)
}

fn zero_edge_time(edge: &MovingPoint) -> Option<Root> {
    if edge.x.delta.is_zero() && edge.y.delta.is_zero() {
        return (edge.x.start.is_zero() && edge.y.start.is_zero()).then_some(Root::zero());
    }
    let candidate = if !edge.x.delta.is_zero() {
        BigRational::new(-&edge.x.start, edge.x.delta.clone())
    } else {
        BigRational::new(-&edge.y.start, edge.y.delta.clone())
    };
    let matches = |coordinate: &Linear| {
        &coordinate.start * candidate.denom() + &coordinate.delta * candidate.numer()
            == BigInt::zero()
    };
    (matches(&edge.x) && matches(&edge.y)).then_some(Root::Rational(candidate))
}

/// Checks every t in [0,1] for a single closed line-only contour. The caller
/// supplies endpoint zero-edge markers created by a guaranteed construction.
#[cfg(test)]
pub(super) fn check_linear_morph(
    from: &PathData,
    to: &PathData,
    allowed_zero_start: &[usize],
    allowed_zero_end: &[usize],
) -> Result<(), MorphIssue> {
    scan_linear_morph(from, to, allowed_zero_start, allowed_zero_end, true).map(|_| ())
}

pub(super) fn scan_linear_morph(
    from: &PathData,
    to: &PathData,
    allowed_zero_start: &[usize],
    allowed_zero_end: &[usize],
    contact_is_error: bool,
) -> Result<Option<MorphIssue>, MorphIssue> {
    let count = from.points.len();
    if count < 3
        || count != to.points.len()
        || from.verbs != to.verbs
        || from.verbs.first() != Some(&PathVerb::Move)
        || from.verbs.last() != Some(&PathVerb::Close)
        || from.verbs[1..from.verbs.len() - 1]
            .iter()
            .any(|verb| *verb != PathVerb::Line)
    {
        return Err(issue(
            MorphIssueKind::InvalidEndpoint,
            &Root::zero(),
            &Root::zero(),
            0,
            0,
        ));
    }
    let points = point_grid(&from.points, &to.points);
    for (end, allowed) in [(false, allowed_zero_start), (true, allowed_zero_end)] {
        if !simple_at_endpoint(&points, end, allowed) {
            let time = if end { Root::one() } else { Root::zero() };
            return Err(issue(MorphIssueKind::InvalidEndpoint, &time, &time, 0, 0));
        }
    }
    for edge in 0..count {
        let vector = points[(edge + 1) % count].subtract(&points[edge]);
        if vector.x.start.is_zero()
            && vector.y.start.is_zero()
            && vector.x.delta.is_zero()
            && vector.y.delta.is_zero()
        {
            return Err(issue(
                MorphIssueKind::ZeroEdge,
                &Root::zero(),
                &Root::one(),
                edge,
                edge,
            ));
        }
        if let Some(time) = zero_edge_time(&vector) {
            let permitted = (time.cmp(&Root::zero()) == Ordering::Equal
                && allowed_zero_start.contains(&edge))
                || (time.cmp(&Root::one()) == Ordering::Equal && allowed_zero_end.contains(&edge));
            if !permitted && time.in_closed_unit() {
                return Err(issue(MorphIssueKind::ZeroEdge, &time, &time, edge, edge));
            }
        }
    }
    for vertex in 0..count {
        let prev = (vertex + count - 1) % count;
        let next = (vertex + 1) % count;
        let before = points[prev].subtract(&points[vertex]);
        let after = points[next].subtract(&points[vertex]);
        let collinear = cross(&before, &after);
        let overlap = dot(&before, &after);
        if collinear.is_zero() {
            if let Some((start, end)) = positive_overlap(&overlap) {
                return Err(issue(MorphIssueKind::Spike, &start, &end, vertex, prev));
            }
        } else {
            for time in collinear.roots().into_iter().filter(Root::in_open_unit) {
                if sign_at_root(&overlap, &time) == Ordering::Greater {
                    return Err(issue(MorphIssueKind::Spike, &time, &time, vertex, prev));
                }
            }
        }
    }
    let vertex_bounds = points.iter().map(PointBounds::moving).collect::<Vec<_>>();
    let edge_bounds = (0..count)
        .map(|edge| vertex_bounds[edge].union(&vertex_bounds[(edge + 1) % count]))
        .collect::<Vec<_>>();
    let mut first_contact = None;
    for vertex in 0..count {
        for edge in 0..count {
            if vertex == edge || vertex == (edge + 1) % count {
                continue;
            }
            if !vertex_bounds[vertex].overlaps(&edge_bounds[edge]) {
                continue;
            }
            if let Some(problem) = vertex_edge_event(&points, vertex, edge, (edge + 1) % count) {
                record_event(problem, contact_is_error, &mut first_contact)?;
            }
        }
    }
    let mut area = Polynomial::zero();
    for edge in 0..count {
        area.add(&cross(&points[edge], &points[(edge + 1) % count]));
    }
    if area.at_endpoint(false).is_zero() || area.at_endpoint(true).is_zero() {
        return Err(issue(
            MorphIssueKind::AreaCollapse,
            &Root::zero(),
            &Root::one(),
            0,
            0,
        ));
    }
    for time in area.roots().into_iter().filter(Root::in_open_unit) {
        return Err(issue(MorphIssueKind::AreaCollapse, &time, &time, 0, 0));
    }
    Ok(first_contact)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn polygon(points: &[(f64, f64)]) -> PathData {
        PathData::new(
            std::iter::once(PathVerb::Move)
                .chain(std::iter::repeat_n(PathVerb::Line, points.len() - 1))
                .chain(std::iter::once(PathVerb::Close))
                .collect(),
            points.iter().map(|(x, y)| Point::new(*x, *y)).collect(),
        )
        .unwrap()
    }

    #[test]
    fn safe_translation_has_no_event() {
        let from = polygon(&[(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)]);
        let to = polygon(&[(5.0, 0.0), (7.0, 0.0), (7.0, 2.0), (5.0, 2.0)]);
        assert!(check_linear_morph(&from, &to, &[], &[]).is_ok());
    }

    #[test]
    fn separate_contours_find_a_between_frame_collision_without_a_near_miss() {
        let outer = polygon(&[(0.0, 0.0), (200.0, 0.0), (200.0, 200.0), (0.0, 200.0)]);
        let incoming = polygon(&[(291.0, 40.0), (311.0, 40.0), (311.0, 60.0), (291.0, 60.0)]);
        let inside = polygon(&[(91.0, 40.0), (111.0, 40.0), (111.0, 60.0), (91.0, 60.0)]);
        let problem = check_contour_pair_morph(
            &outer.points,
            &outer.points,
            &incoming.points,
            &inside.points,
            [false, false],
            [false, false],
        )
        .unwrap_err();
        assert!((problem.from - 0.455).abs() < 1e-12, "{problem}");
        let near = polygon(&[
            (200.0 + 1e-12, 40.0),
            (220.0 + 1e-12, 40.0),
            (220.0 + 1e-12, 60.0),
            (200.0 + 1e-12, 60.0),
        ]);
        assert!(
            check_contour_pair_morph(
                &outer.points,
                &outer.points,
                &near.points,
                &near.points,
                [false, false],
                [false, false],
            )
            .is_ok()
        );
    }

    #[test]
    fn marked_convex_zero_edges_are_allowed_only_at_their_endpoint() {
        let square = polygon(&[(0.0, 0.0), (40.0, 0.0), (40.0, 40.0), (0.0, 40.0)]);
        let triangle = polygon(&[(100.0, 10.0), (160.0, 20.0), (120.0, 80.0)]);
        let (from, to) = square.convex_morph_pair(&triangle).unwrap();
        let zero_edges = |path: &PathData| {
            (0..path.points.len())
                .filter(|edge| path.points[*edge] == path.points[(edge + 1) % path.points.len()])
                .collect::<Vec<_>>()
        };
        let start = zero_edges(&from);
        let end = zero_edges(&to);
        assert!(!start.is_empty() && !end.is_empty());
        assert!(check_linear_morph(&from, &to, &start, &end).is_ok());
        assert_eq!(
            check_linear_morph(&from, &to, &[], &[]).unwrap_err().kind,
            MorphIssueKind::InvalidEndpoint
        );
    }

    #[test]
    fn an_intersection_between_regular_samples_is_detected() {
        let from = polygon(&[(6.0, 26.0), (11.0, 26.0), (27.0, 2.0), (23.0, 1.0)]);
        let to = polygon(&[(27.0, 13.0), (15.0, 28.0), (13.0, 15.0), (24.0, 2.0)]);
        let problem = check_linear_morph(&from, &to, &[], &[]).unwrap_err();
        assert!(
            matches!(
                problem.kind,
                MorphIssueKind::Crossing | MorphIssueKind::Spike
            ),
            "{problem}"
        );
        assert!(problem.from > 0.44 && problem.from < 0.47, "{problem}");
    }

    #[test]
    fn exact_collinear_interval_and_close_miss() {
        let stationary_a = Linear {
            start: BigInt::zero(),
            delta: BigInt::zero(),
        };
        let stationary_b = Linear {
            start: BigInt::from(3),
            delta: BigInt::zero(),
        };
        let moving = Linear {
            start: BigInt::from(-3),
            delta: BigInt::from(10),
        };
        let first = product(
            &moving.subtract(&stationary_a),
            &stationary_b.subtract(&stationary_a),
        );
        let second = product(
            &moving.subtract(&stationary_b),
            &stationary_a.subtract(&stationary_b),
        );
        let (start, end) = nonnegative_overlap(&first, &second).unwrap();
        assert!((start.approximate() - 0.3).abs() < 1e-15);
        assert!((end.approximate() - 0.6).abs() < 1e-15);

        let nearly_collinear = point_grid(
            &[
                Point::new(0.0, 0.0),
                Point::new(3.0, 0.0),
                Point::new(-3.0, 1e-12),
            ],
            &[
                Point::new(0.0, 0.0),
                Point::new(3.0, 0.0),
                Point::new(7.0, 1e-12),
            ],
        );
        let a = nearly_collinear[0].subtract(&nearly_collinear[2]);
        let b = nearly_collinear[1].subtract(&nearly_collinear[2]);
        let separated = cross(&a, &b);
        assert!(!separated.is_zero());
        assert!(separated.roots().is_empty());

        let edge = polygon(&[(0.0, 0.0), (3.0, 0.0), (3.0, 3.0), (0.0, 3.0)]);
        let near = polygon(&[(0.0, 0.0), (3.0, 0.0), (3.0, 3.0), (0.0, 3.0 + 1e-12)]);
        assert!(check_linear_morph(&edge, &near, &[], &[]).is_ok());
    }

    #[test]
    fn tangent_root_is_a_contact_and_interior_zero_edge_is_rejected() {
        let tangent = Polynomial {
            c0: BigInt::from(455 * 455),
            c1: BigInt::from(-2 * 455 * 1000),
            c2: BigInt::from(1000 * 1000),
        };
        let roots = tangent.roots();
        assert_eq!(roots.len(), 1);
        assert!((roots[0].approximate() - 0.455).abs() < 1e-15);
        assert_eq!(sign_at_root(&tangent, &roots[0]), Ordering::Equal);

        let from = polygon(&[(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)]);
        let to = polygon(&[(0.0, 0.0), (-2.0, 0.0), (-2.0, 2.0), (0.0, 2.0)]);
        let problem = check_linear_morph(&from, &to, &[], &[]).unwrap_err();
        assert_eq!(problem.kind, MorphIssueKind::ZeroEdge);
        assert_eq!(problem.from, 0.5);

        let tangent_from = polygon(&[
            (0.0, 0.0),
            (1.0, 0.0),
            (1.5, -3.0),
            (0.0, -0.25),
            (-3.0, -3.0),
        ]);
        let tangent_to = polygon(&[
            (0.0, 0.0),
            (1.0, 1.0),
            (1.5, -3.0),
            (1.0, 0.75),
            (-3.0, -3.0),
        ]);
        let problem = check_linear_morph(&tangent_from, &tangent_to, &[], &[]).unwrap_err();
        assert_eq!(problem.kind, MorphIssueKind::Contact, "{problem}");
        assert_eq!(problem.from, 0.5);
        let warning = scan_linear_morph(&tangent_from, &tangent_to, &[], &[], false)
            .unwrap()
            .expect("a tangent contact is reported without rejecting the morph");
        assert_eq!(warning.kind, MorphIssueKind::Contact);
        assert_eq!(warning.from, 0.5);
    }

    #[test]
    fn adjacent_edges_folding_back_form_a_spike() {
        let from = polygon(&[(0.0, 0.0), (2.0, -1.0), (1.0, 0.0), (5.0, -3.0)]);
        let to = polygon(&[(0.0, 0.0), (2.0, 1.0), (1.0, 0.0), (5.0, -3.0)]);
        let problem = check_linear_morph(&from, &to, &[], &[]).unwrap_err();
        assert_eq!(problem.kind, MorphIssueKind::Spike);
        assert_eq!(problem.from, 0.5);
        let already_spiked = polygon(&[(0.0, 0.0), (2.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
        assert!(!simple_closed_contour(&already_spiked.points));
    }
}
