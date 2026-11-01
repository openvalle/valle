// A layer composited onto its target at an opacity, inside a rounded rectangle: its premultiplied
// pixels times the opacity and the part of each pixel in the rectangle, over what is there (the
// pipeline blends).

struct Layer {
  // x: the opacity.
  opacity: vec4f,
  // x, y, width, height.
  clip: vec4f,
  // As `coverage` takes them.
  radii: array<vec4f, 2>,
}

@group(0) @binding(5) var<storage, read> layers: array<Layer>;
@group(1) @binding(0) var image: texture_2d<f32>;

struct Out {
  @builtin(position) pos: vec4f,
  @location(0) @interpolate(flat) index: u32,
}

@vertex fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Out {
  var out: Out;
  out.pos = whole(vi);
  out.index = ii;
  return out;
}

@fragment fn fs(in: Out) -> @location(0) vec4f {
  let l = layers[in.index];
  let cov = coverage(in.pos.xy, l.clip, l.radii);
  return textureLoad(image, vec2<i32>(in.pos.xy), 0) * (l.opacity.x * cov);
}
