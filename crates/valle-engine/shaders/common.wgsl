// What every pass shares: the target's size in px, the corners instances are drawn with, where a
// point in px lands on the target, and how much of a pixel a rounded box covers.

struct Screen {
  // Width and height, px.
  size: vec4f,
}

@group(0) @binding(0) var<uniform> screen: Screen;

// A corner of the unit square, for vertices 0..4 drawn as a triangle strip.
fn corner(vi: u32) -> vec2f {
  return vec2f(f32(vi & 1u), f32(vi >> 1u));
}

// A point in target px, y down, in clip space.
fn clip(p: vec2f) -> vec4f {
  let s = screen.size.xy;
  return vec4f(p.x / s.x * 2.0 - 1.0, 1.0 - p.y / s.y * 2.0, 0.0, 1.0);
}

// A triangle over the whole target, for vertices 0..3.
fn whole(vi: u32) -> vec4f {
  let c = vec2f(f32((vi << 1u) & 2u), f32(vi & 2u));
  return vec4f(c.x * 2.0 - 1.0, 1.0 - c.y * 2.0, 0.0, 1.0);
}

// The part of the pixel centered at `p` inside a box (x, y, width, height) with elliptical
// corners: exact for straight edges, as a rectangle's box filter is the product of its two
// sides', and past a corner's center on both axes the part inside its ellipse. `radii` are the
// corners' horizontal and vertical radii, the top left's and top right's then the bottom right's
// and bottom left's, and fit in the box.
fn coverage(p: vec2f, box: vec4f, radii: array<vec4f, 2>) -> f32 {
  let lo = box.xy;
  let hi = box.xy + box.zw;
  let side = clamp(min(p + 0.5, hi) - max(p - 0.5, lo), vec2f(0.0), vec2f(1.0));
  let tl = radii[0].xy;
  let tr = radii[0].zw;
  let br = radii[1].xy;
  let bl = radii[1].zw;
  var cov = side.x * side.y;
  cov = min(cov, rounded((lo + tl) - p, tl));
  cov = min(cov, rounded(vec2f(p.x - (hi.x - tr.x), (lo.y + tr.y) - p.y), tr));
  cov = min(cov, rounded(p - (hi - br), br));
  cov = min(cov, rounded(vec2f((lo.x + bl.x) - p.x, p.y - (hi.y - bl.y)), bl));
  return cov;
}

// The part of a pixel inside a corner's ellipse with radii `r`, `off` past its center on each
// axis, away from the box: all of it before the center on either axis or at a square corner. The
// distance to a circle is exact; to an ellipse it is its implicit function over its gradient.
fn rounded(off: vec2f, r: vec2f) -> f32 {
  if (off.x <= 0.0 || off.y <= 0.0 || r.x == 0.0) {
    return 1.0;
  }
  var dist = length(off) - r.x;
  if (r.x != r.y) {
    let z = off / (r * r);
    dist = (dot(off, z) - 1.0) / (2.0 * length(z));
  }
  return clamp(0.5 - dist, 0.0, 1.0);
}
