// Rounded rectangles: a box in px, a premultiplied working color and corner radii that fit in the
// box.

struct Rect {
  // x, y, width, height.
  box: vec4f,
  color: vec4f,
  // As `coverage` takes them.
  radii: array<vec4f, 2>,
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
  return r.color * coverage(in.pos.xy, r.box, r.radii);
}
