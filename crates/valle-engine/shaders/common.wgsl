// What every pass shares: the target's size in px, the transforms instances are drawn with, the
// corners of their quads, where a point in px lands on the target, and how much of a pixel a
// rounded box covers. An instance is in its own px, which its transform takes to the target's.

struct Screen {
  // Width and height, px.
  size: vec4f,
}

// From an instance's px to the target's, and back.
struct Transform {
  forward: mat3x3f,
  inverse: mat3x3f,
}

@group(0) @binding(0) var<uniform> screen: Screen;
@group(0) @binding(8) var<storage, read> transforms: array<Transform>;

// A corner of the unit square, for vertices 0..4 drawn as a triangle strip.
fn corner(vi: u32) -> vec2f {
  return vec2f(f32(vi & 1u), f32(vi >> 1u));
}

// A point in target px, y down, in clip space.
fn clip(p: vec2f) -> vec4f {
  let s = screen.size.xy;
  return vec4f(p.x / s.x * 2.0 - 1.0, 1.0 - p.y / s.y * 2.0, 0.0, 1.0);
}

// A point in the px of transform `t`, in clip space: divided by its projective coordinate, so
// what is between corners is in perspective.
fn place(p: vec2f, t: u32) -> vec4f {
  let h = transforms[t].forward * vec3f(p, 1.0);
  let s = screen.size.xy;
  return vec4f(h.x / s.x * 2.0 - h.z, h.z - h.y / s.y * 2.0, 0.0, h.z);
}

// Corner `vi` of a rectangle (x, y, width, height) in the px of transform `t`, grown for an
// antialiased edge, in clip space.
fn quad(vi: u32, b: vec4f, t: u32) -> vec4f {
  let c = corner(vi);
  return place(grown(b.xy + c * b.zw, c * 2.0 - 1.0, t), t);
}

// A corner `p` in the px of transform `t` moved `out`, each axis -1 or 1, by as much as a target
// pixel reaches there. In perspective that differs from corner to corner, and in between it is no
// more than the corners' mix. A corner behind the viewer is not seen, and stays.
fn grown(p: vec2f, out: vec2f, t: u32) -> vec2f {
  let h = transforms[t].forward * vec3f(p, 1.0);
  if (h.z <= 0.0) {
    return p;
  }
  let j = steps(h.xy / h.z, t);
  return p + out * (abs(j[0]) + abs(j[1]));
}

// The target pixel centered at `p` in the px of transform `t`.
fn local(p: vec2f, t: u32) -> vec2f {
  let h = transforms[t].inverse * vec3f(p, 1.0);
  return h.xy / h.z;
}

// How far a step of a target pixel along x, and one along y, go in the px of transform `t`, at
// the target pixel centered at `p`.
fn steps(p: vec2f, t: u32) -> mat2x2f {
  let m = transforms[t].inverse;
  let h = m * vec3f(p, 1.0);
  let q = h.xy / h.z;
  return mat2x2f((m[0].xy - q * m[0].z) / h.z, (m[1].xy - q * m[1].z) / h.z);
}

// A triangle over the whole target, for vertices 0..3.
fn whole(vi: u32) -> vec4f {
  let c = vec2f(f32((vi << 1u) & 2u), f32(vi & 2u));
  return vec4f(c.x * 2.0 - 1.0, 1.0 - c.y * 2.0, 0.0, 1.0);
}

// The part of a target pixel inside a box (x, y, width, height) with elliptical corners, at `q`
// in the box's px, where a step of the pixel along x and one along y go `j`. On each axis of the
// box the pixel is a parallelogram's shadow, exactly covered between the box's edges, and the
// product of the two is exact for a rectangle the axes keep; past a corner's center on both axes
// it is the part inside its ellipse. `radii` are the corners' horizontal and vertical radii, the
// top left's and top right's then the bottom right's and bottom left's, and fit in the box.
fn coverage(q: vec2f, j: mat2x2f, box: vec4f, radii: array<vec4f, 2>) -> f32 {
  let lo = box.xy;
  let hi = box.xy + box.zw;
  let a = abs(j[0]);
  let b = abs(j[1]);
  let side = vec2f(
    before(hi.x - q.x, a.x, b.x) - before(lo.x - q.x, a.x, b.x),
    before(hi.y - q.y, a.y, b.y) - before(lo.y - q.y, a.y, b.y),
  );
  let tl = radii[0].xy;
  let tr = radii[0].zw;
  let br = radii[1].xy;
  let bl = radii[1].zw;
  var cov = side.x * side.y;
  cov = min(cov, rounded((lo + tl) - q, tl, vec2f(-1.0, -1.0), j));
  cov = min(cov, rounded(vec2f(q.x - (hi.x - tr.x), (lo.y + tr.y) - q.y), tr, vec2f(1.0, -1.0), j));
  cov = min(cov, rounded(q - (hi - br), br, vec2f(1.0, 1.0), j));
  cov = min(cov, rounded(vec2f((lo.x + bl.x) - q.x, q.y - (hi.y - bl.y)), bl, vec2f(-1.0, 1.0), j));
  return cov;
}

// The part of a pixel before `t` from its center on an axis, where its steps along x and y go `a`
// and `b`: the sum of two even spreads that long, linear in the middle and quadratic at the ends.
fn before(t: f32, a: f32, b: f32) -> f32 {
  let w = max(max(a, b), 1e-12);
  let n = min(a, b);
  let s = abs(t);
  var f = 0.5 + s / w;
  if (s > (w - n) * 0.5) {
    let r = max((w + n) * 0.5 - s, 0.0);
    f = 1.0 - r * r / max(2.0 * w * n, 1e-12);
  }
  return select(1.0 - f, f, t >= 0.0);
}

// The part of a pixel inside a corner's ellipse with radii `r`, `off` past its center on each
// axis, away from the box, which is `flip` times the box's axes: all of it before the center on
// either axis or at a square corner. The distance to a circle is exact; to an ellipse it is its
// implicit function over its gradient. Over a pixel it changes by its gradient's steps along `j`.
fn rounded(off: vec2f, r: vec2f, flip: vec2f, j: mat2x2f) -> f32 {
  if (off.x <= 0.0 || off.y <= 0.0 || r.x == 0.0) {
    return 1.0;
  }
  var dist = length(off) - r.x;
  var g = off;
  if (r.x != r.y) {
    let z = off / (r * r);
    dist = (dot(off, z) - 1.0) / (2.0 * length(z));
    g = z;
  }
  let n = normalize(g) * flip;
  let px = length(vec2f(dot(n, j[0]), dot(n, j[1])));
  return clamp(0.5 - dist / px, 0.0, 1.0);
}
