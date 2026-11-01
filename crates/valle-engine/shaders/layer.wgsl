// A layer composited onto its target at an opacity: its premultiplied pixels times the opacity,
// over what is there (the pipeline blends).

@group(0) @binding(5) var<storage, read> layers: array<vec4f>;
@group(1) @binding(0) var image: texture_2d<f32>;

struct Out {
  @builtin(position) pos: vec4f,
  @location(0) @interpolate(flat) opacity: f32,
}

@vertex fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Out {
  var out: Out;
  out.pos = whole(vi);
  out.opacity = layers[ii].x;
  return out;
}

@fragment fn fs(in: Out) -> @location(0) vec4f {
  return textureLoad(image, vec2<i32>(in.pos.xy), 0) * in.opacity;
}
