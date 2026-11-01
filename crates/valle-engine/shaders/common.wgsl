// What every pass shares: the target's size in px, the corners instances are drawn with, and
// where a point in px lands on the target.

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
