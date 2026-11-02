// Images filling rounded rectangles in tiles: each pixel's place in the tile it is in, if any,
// and the image there, stretched over the tile, read between its pixels linearly. A pixel that
// spans 2 or more of the image's px reads the level of its mipmap where it spans at least 1 and
// fewer than 2, as Skia picks it. On an axis where the rectangle spans tiles that touch, an edge
// pixel's neighbor is the next tile's, as Chrome reads a repeated image unless its tiles are
// spaced apart; elsewhere the edge goes on. A pixelated image reads the px nearest instead. Its
// colors are premultiplied working colors already.

struct Image {
  // Where it shows: x, y, width, height, and corner radii as `coverage` takes them.
  rect: vec4f,
  radii: array<vec4f, 2>,
  // The first tile: x, y, width and height; and on each axis how far the next one is, 0 when it
  // does not repeat.
  tile: vec4f,
  step: vec4f,
  unused: vec4f,
  // x: 1 if it reads its nearest px; w: the transform.
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

// Texel `i` of `n` on each axis: around to the other side where it `wraps`, else the edge one.
fn texel(i: vec2<i32>, n: vec2<i32>, wraps: vec2<bool>) -> vec2<i32> {
  return select(clamp(i, vec2<i32>(0), n - 1), (i % n + n) % n, wraps);
}

@fragment fn fs(in: Out) -> @location(0) vec4f {
  let g = images[in.index];
  let p = local(in.pos.xy, g.form.w);
  let size = g.tile.zw;
  // The image's px the pixel spans, the most along either side of it.
  let u = p / size * vec2f(textureDimensions(image));
  let spans = max(length(dpdx(u)), length(dpdy(u)));
  let last = f32(textureNumLevels(image) - 1u);
  let level = i32(clamp(floor(log2(max(spans, 1.0))), 0.0, last));
  let cov = coverage(p, steps(in.pos.xy, g.form.w), g.rect, g.radii);
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
  if (g.form.x == 1u) {
    // Between two px, the first, as Skia picks it.
    let n = vec2<i32>(textureDimensions(image));
    let i = vec2<i32>(floor(at * vec2f(n) / size - 1.0 / 1024.0));
    return textureLoad(image, clamp(i, vec2<i32>(0), n - 1), 0) * cov;
  }
  // Between the centers of the texels around the point.
  let texels = vec2<i32>(textureDimensions(image, level));
  let c = at / size * vec2f(texels) - 0.5;
  let f = floor(c);
  let t = c - f;
  let n = floor((lo - g.tile.xy) / step);
  let several = lo + g.rect.zw > g.tile.xy + (n + 1.0) * step + 1e-3;
  let wraps = repeats & several & !vec2<bool>(any(repeats & (step > size)));
  let a = texel(vec2<i32>(f), texels, wraps);
  let b = texel(vec2<i32>(f) + 1, texels, wraps);
  let top = mix(textureLoad(image, a, level), textureLoad(image, vec2<i32>(b.x, a.y), level), t.x);
  let bottom = mix(
    textureLoad(image, vec2<i32>(a.x, b.y), level),
    textureLoad(image, b, level),
    t.x,
  );
  return mix(top, bottom, t.y) * cov;
}
