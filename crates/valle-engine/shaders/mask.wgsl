// A layer composited onto its target through a mask: its premultiplied pixels times the mask's
// alpha there, over what is there (the pipeline blends).

@group(1) @binding(0) var image: texture_2d<f32>;
@group(2) @binding(0) var mask: texture_2d<f32>;

@vertex fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4f {
  return whole(vi);
}

@fragment fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
  let p = vec2<i32>(pos.xy);
  return textureLoad(image, p, 0) * textureLoad(mask, p, 0).a;
}
