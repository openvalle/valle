// Rounded rectangles and the rings between two, each side in a premultiplied working color: a
// background is a ring whose inside has no size, a border one with the padding box inside.

struct Rect {
  // x, y, width, height, and corner radii as `coverage` takes them.
  outer: vec4f,
  outer_radii: array<vec4f, 2>,
  inner: vec4f,
  inner_radii: array<vec4f, 2>,
  // Top, right, bottom, left.
  colors: array<vec4f, 4>,
}

@group(0) @binding(1) var<storage, read> rects: array<Rect>;

struct Out {
  @builtin(position) pos: vec4f,
  @location(0) @interpolate(flat) index: u32,
}

@vertex fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Out {
  let b = rects[ii].outer;
  var out: Out;
  // A pixel more around the box, for its antialiased edge.
  out.pos = clip(b.xy - 1.0 + corner(vi) * (b.zw + 2.0));
  out.index = ii;
  return out;
}

@fragment fn fs(in: Out) -> @location(0) vec4f {
  let r = rects[in.index];
  let p = in.pos.xy;
  let ring = coverage(p, r.outer, r.outer_radii) - coverage(p, r.inner, r.inner_radii);
  return side(p, r.outer, r.inner, r.colors) * max(ring, 0.0);
}

// The color at `p` of a ring's sides: of the side it is nearest in units of the side's width,
// so sides meet on the lines from the outer corners to the inner ones, antialiased there.
fn side(p: vec2f, outer: vec4f, inner: vec4f, colors: array<vec4f, 4>) -> vec4f {
  // To each side's outer edge, and each side's width: top, right, bottom, left.
  let d = vec4f(p.y - outer.y, outer.x + outer.z - p.x, outer.y + outer.w - p.y, p.x - outer.x);
  let w = vec4f(
    inner.y - outer.y,
    outer.x + outer.z - inner.x - inner.z,
    outer.y + outer.w - inner.y - inner.w,
    inner.x - outer.x,
  );
  // Top or bottom, and left or right, without dividing by a width of 0.
  var v = 2u;
  if (d.x * w.z <= d.z * w.x) {
    v = 0u;
  }
  var h = 1u;
  if (d.w * w.y <= d.y * w.w) {
    h = 3u;
  }
  let dv = d[v];
  let dh = d[h];
  let wv = w[v];
  let wh = w[h];
  // How much is the horizontal neighbour's: from the distance to the line through the corner
  // where their sides meet.
  var t = 0.5;
  if (wv <= 0.0) {
    t = 1.0;
  } else if (wh <= 0.0) {
    t = 0.0;
  } else {
    t = clamp(0.5 + (dv * wh - dh * wv) / length(vec2f(wv, wh)), 0.0, 1.0);
  }
  var c = colors;
  return mix(c[v], c[h], t);
}
