// A layer composited onto its target at an opacity, inside a rounded rectangle or outside it: its
// premultiplied pixels times the opacity and the part of each pixel in the rectangle, or out of
// it, over what is there (the pipeline blends). The rectangle is in the px of a transform.

struct Layer {
  // x: the opacity; y: 1 to show the layer outside the rectangle.
  opacity: vec4f,
  // x, y, width, height.
  clip: vec4f,
  // As `coverage` takes them.
  radii: array<vec4f, 2>,
  // x: the rectangle's transform.
  form: vec4u,
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
  let t = l.form.x;
  var cov = coverage(local(in.pos.xy, t), steps(in.pos.xy, t), l.clip, l.radii);
  if (l.opacity.y > 0.0) {
    cov = 1.0 - cov;
  }
  return textureLoad(image, vec2<i32>(in.pos.xy), 0) * (l.opacity.x * cov);
}
