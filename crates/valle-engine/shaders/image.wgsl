// Images filling rounded rectangles in tiles: each pixel's place in the tile it is in, if any,
// and the image there, stretched over the tile, read between its pixels linearly and clamped at
// its edges. Its colors are premultiplied working colors already.

struct Image {
  // Where it shows: x, y, width, height, and corner radii as `coverage` takes them.
  rect: vec4f,
  radii: array<vec4f, 2>,
  // The first tile: x, y, width and height; and on each axis how far the next one is, 0 when it
  // does not repeat.
  tile: vec4f,
  step: vec4f,
  unused: vec4f,
  // w: the transform.
  form: vec4u,
  more: vec4u,
}

@group(0) @binding(6) var<storage, read> images: array<Image>;
@group(1) @binding(0) var image: texture_2d<f32>;

struct Out {
  @builtin(position) pos: vec4f,
  @location(0) @interpolate(flat) index: u32,
}

@vertex fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Out {
  let g = images[ii];
  var out: Out;
  out.pos = quad(vi, g.rect, g.form.w);
  out.index = ii;
  return out;
}

@fragment fn fs(in: Out) -> @location(0) vec4f {
  let g = images[in.index];
  let p = local(in.pos.xy, g.form.w);
  let cov = coverage(p, steps(in.pos.xy, g.form.w), g.rect, g.radii);
  let size = g.tile.zw;
  let step = g.step.xy;
  // A pixel at the edge, its center outside, takes the color just inside, not the next tile's.
  let lo = g.rect.xy;
  let q = clamp(p, lo, max(lo, lo + g.rect.zw - 1e-3)) - g.tile.xy;
  let repeats = step > vec2f(0.0);
  let at = select(q, q - step * floor(q / step), repeats);
  // Outside a tile that does not repeat, or between tiles spaced apart.
  let out = select((q < vec2f(0.0)) | (q >= size), (at >= size) & (step > size), repeats);
  if (any(out)) {
    return vec4f(0.0);
  }
  // Between the centers of the texels around the point, the edge ones going on past them.
  let texels = vec2<i32>(textureDimensions(image));
  let c = at / size * vec2f(texels) - 0.5;
  let f = floor(c);
  let t = c - f;
  let last = texels - 1;
  let a = clamp(vec2<i32>(f), vec2<i32>(0), last);
  let b = clamp(vec2<i32>(f) + 1, vec2<i32>(0), last);
  let top = mix(textureLoad(image, a, 0), textureLoad(image, vec2<i32>(b.x, a.y), 0), t.x);
  let bottom = mix(textureLoad(image, vec2<i32>(a.x, b.y), 0), textureLoad(image, b, 0), t.x);
  return mix(top, bottom, t.y) * cov;
}
