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

// The part of the pixel centered at `p` inside a box (x, y, width, height) with its corners
// rounded by `radius`: exact for straight edges, as a rectangle's box filter is the product of
// its two sides', and past a corner's center on both axes the part inside its circle, from the
// distance to it.
fn coverage(p: vec2f, box: vec4f, radius: f32) -> f32 {
  let lo = box.xy;
  let hi = box.xy + box.zw;
  let side = clamp(min(p + 0.5, hi) - max(p - 0.5, lo), vec2f(0.0), vec2f(1.0));
  var cov = side.x * side.y;
  if (radius > 0.0) {
    let off = p - clamp(p, lo + radius, hi - radius);
    if (off.x != 0.0 && off.y != 0.0) {
      cov = min(cov, clamp(radius + 0.5 - length(off), 0.0, 1.0));
    }
  }
  return cov;
}
