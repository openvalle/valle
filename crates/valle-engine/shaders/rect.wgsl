// Rounded rectangles: a box in px, a premultiplied working color and a corner radius that fits
// in the box.

struct Rect {
  // x, y, width, height.
  box: vec4f,
  color: vec4f,
  // x: the corner radius.
  radius: vec4f,
}

@group(0) @binding(1) var<storage, read> rects: array<Rect>;

struct Out {
  @builtin(position) pos: vec4f,
  @location(0) @interpolate(flat) index: u32,
}

@vertex fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Out {
  let b = rects[ii].box;
  var out: Out;
  // A pixel more around the box, for its antialiased edge.
  out.pos = clip(b.xy - 1.0 + corner(vi) * (b.zw + 2.0));
  out.index = ii;
  return out;
}

@fragment fn fs(in: Out) -> @location(0) vec4f {
  let r = rects[in.index];
  let lo = r.box.xy;
  let hi = r.box.xy + r.box.zw;
  let p = in.pos.xy;
  // The part of the pixel inside the box, exact for straight edges: a rectangle's box filter is
  // the product of its two sides'.
  let side = clamp(min(p + 0.5, hi) - max(p - 0.5, lo), vec2f(0.0), vec2f(1.0));
  var cov = side.x * side.y;
  // Past a corner's center on both axes, the part inside its circle, from the distance to it.
  let radius = r.radius.x;
  if (radius > 0.0) {
    let off = p - clamp(p, lo + radius, hi - radius);
    if (off.x != 0.0 && off.y != 0.0) {
      cov = min(cov, clamp(radius + 0.5 - length(off), 0.0, 1.0));
    }
  }
  return r.color * cov;
}
