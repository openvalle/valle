//! The order of the planes of a 3D scene, back to front: a BSP tree of their rectangles where
//! they are seen, x and y in px and z toward the viewer, all divided through by their projective
//! coordinates so the viewer looks along z from infinitely far. A plane that crosses another's is
//! cut in two along it, each piece drawn in its turn. Ties keep the planes' order.

use super::Matrix4;
use crate::layout::Rect;

/// How far from a plane, in px, a point is on it.
const ON: f64 = 0.05;
/// The least projective coordinate drawn: what is nearer the viewer than this is behind it.
const NEAR: f64 = 1e-6;

/// A part of a plane, a convex polygon of points in space before they are divided through, and
/// the lines in the plane's px that cut it from the rest: a x + b y + c ≥ 0 keeps it.
struct Piece {
    plane: usize,
    points: Vec<[f64; 4]>,
    cuts: Vec<[f64; 3]>,
}

struct Node {
    /// Its plane's normal, of length 1, and distance: n · p + d is how far p is in front.
    normal: [f64; 3],
    d: f64,
    /// The pieces in its plane, in their order.
    on: Vec<Piece>,
    front: Option<Box<Node>>,
    back: Option<Box<Node>>,
}

/// The planes `spaces[i]` takes the rectangles `rects[i]` to, in the order to draw them: each
/// piece's plane and the lines that cut it. A plane edge on to the viewer, or behind it, is left
/// out.
pub(super) fn order(planes: &[(Matrix4, Rect)]) -> Vec<(usize, Vec<[f32; 3]>)> {
    let mut root: Option<Box<Node>> = None;
    for (i, (space, r)) in planes.iter().enumerate() {
        let corners = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].map(|[u, v]| {
            let p = [r.x + u * r.width, r.y + v * r.height].map(f64::from);
            space.apply4([p[0], p[1], 0.0, 1.0])
        });
        let points = near(&corners);
        if points.len() >= 3 {
            let piece = Piece {
                plane: i,
                points,
                cuts: Vec::new(),
            };
            insert(&mut root, piece, planes);
        }
    }
    let mut out = Vec::new();
    if let Some(root) = &root {
        walk(root, &mut out);
    }
    out
}

/// The part of a polygon in front of the viewer.
fn near(points: &[[f64; 4]]) -> Vec<[f64; 4]> {
    let mut out = Vec::new();
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        if a[3] >= NEAR {
            out.push(*a);
        }
        if (a[3] >= NEAR) != (b[3] >= NEAR) {
            let t = (NEAR - a[3]) / (b[3] - a[3]);
            out.push(std::array::from_fn(|k| a[k] + t * (b[k] - a[k])));
        }
    }
    out
}

fn divided(p: &[f64; 4]) -> [f64; 3] {
    [p[0] / p[3], p[1] / p[3], p[2] / p[3]]
}

/// The plane of a polygon, by Newell's sums, which hold for one not quite flat; `None` for one
/// with no area.
fn plane_of(points: &[[f64; 4]]) -> Option<([f64; 3], f64)> {
    let p: Vec<[f64; 3]> = points.iter().map(divided).collect();
    let mut n = [0.0; 3];
    let mut center = [0.0; 3];
    for (i, a) in p.iter().enumerate() {
        let b = p[(i + 1) % p.len()];
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
        for k in 0..3 {
            center[k] += a[k] / p.len() as f64;
        }
    }
    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if length < 1e-9 || !length.is_finite() {
        return None;
    }
    let n = n.map(|v| v / length);
    Some((n, -(n[0] * center[0] + n[1] * center[1] + n[2] * center[2])))
}

fn insert(node: &mut Option<Box<Node>>, piece: Piece, planes: &[(Matrix4, Rect)]) {
    let Some(n) = node else {
        if let Some((normal, d)) = plane_of(&piece.points) {
            *node = Some(Box::new(Node {
                normal,
                d,
                on: vec![piece],
                front: None,
                back: None,
            }));
        }
        return;
    };
    // How far in front of the node's plane, before and after dividing through.
    let ([x, y, z], d) = (n.normal, n.d);
    let f = |p: &[f64; 4]| x * p[0] + y * p[1] + z * p[2] + d * p[3];
    let s: Vec<f64> = piece.points.iter().map(|p| f(p) / p[3]).collect();
    let front = s.iter().all(|&s| s >= -ON);
    let back = s.iter().all(|&s| s <= ON);
    match (front, back) {
        (true, true) => n.on.push(piece),
        (true, false) => insert(&mut n.front, piece, planes),
        (false, true) => insert(&mut n.back, piece, planes),
        (false, false) => {
            // The node's plane in the piece's px, through its plane's space: the line where
            // n · p + d is 0.
            let m = &planes[piece.plane].0.0;
            let line = [0, 1, 3].map(|c| f(&m[c]));
            let (mut ahead, mut behind) = (Vec::new(), Vec::new());
            for (i, a) in piece.points.iter().enumerate() {
                let j = (i + 1) % piece.points.len();
                let b = &piece.points[j];
                if s[i] >= -ON {
                    ahead.push(*a);
                }
                if s[i] <= ON {
                    behind.push(*a);
                }
                if (s[i] > ON && s[j] < -ON) || (s[i] < -ON && s[j] > ON) {
                    // Along the points before dividing through, where the distance is linear.
                    let (fa, fb) = (f(a), f(b));
                    let t = fa / (fa - fb);
                    let p = std::array::from_fn(|k| a[k] + t * (b[k] - a[k]));
                    ahead.push(p);
                    behind.push(p);
                }
            }
            let cut = |line: [f64; 3], points| {
                let mut cuts = piece.cuts.clone();
                cuts.push(line);
                Piece {
                    plane: piece.plane,
                    points,
                    cuts,
                }
            };
            let behind = cut(line.map(|v| -v), behind);
            insert(&mut n.front, cut(line, ahead), planes);
            insert(&mut n.back, behind, planes);
        }
    }
}

/// Back to front from a viewer infinitely far along z.
fn walk(node: &Node, out: &mut Vec<(usize, Vec<[f32; 3]>)>) {
    let (first, last) = match node.normal[2] >= 0.0 {
        true => (&node.back, &node.front),
        false => (&node.front, &node.back),
    };
    if let Some(n) = first {
        walk(n, out);
    }
    for p in &node.on {
        let cuts = p.cuts.iter().map(|c| c.map(|v| v as f32)).collect();
        out.push((p.plane, cuts));
    }
    if let Some(n) = last {
        walk(n, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn planes_go_back_to_front_and_are_cut_where_they_cross() {
        let square = rect(-10.0, -10.0, 20.0, 20.0);
        let at = |z: f64| Matrix4::translate(0.0, 0.0, z);
        // Nearer first in the list, drawn last.
        let order = order(&[(at(5.0), square), (at(-5.0), square)]);
        assert_eq!(order, [(1, vec![]), (0, vec![])]);
        // Turned about y it crosses the flat one through its middle: its far half goes before
        // it and its near half after.
        let turned = Matrix4::rotate([0.0, 1.0, 0.0], 60.0);
        let order = super::order(&[(Matrix4::IDENTITY, square), (turned, square)]);
        let pieces: Vec<usize> = order.iter().map(|(p, _)| *p).collect();
        assert_eq!(pieces, [1, 0, 1]);
        // The far half of one turned 60° about y, x toward the viewer's right going away, is
        // where x > 0, which its cut keeps.
        let [a, b, c] = order[0].1[0];
        assert!(a > 0.0 && b.abs() < 1e-6 && c.abs() < 1e-6);
        let [a, ..] = order[2].1[0];
        assert!(a < 0.0);
        // Behind the viewer, nothing.
        let behind = Matrix4::perspective(100.0).times(at(200.0));
        assert_eq!(super::order(&[(behind, square)]), []);
    }
}
