// The SDR output: premultiplied working colors to straight 8-bit sRGB, the values clamped to
// [0, 1] and quantized here, so every GPU stores the same bytes.

@group(1) @binding(0) var image: texture_2d<f32>;

@vertex fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4f {
  return whole(vi);
}

@fragment fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
  let c = textureLoad(image, vec2<i32>(pos.xy), 0);
  let a = round(clamp(c.a, 0.0, 1.0) * 255.0);
  if (a == 0.0) {
    return vec4f(0.0);
  }
  let rgb = clamp(c.rgb / c.a, vec3f(0.0), vec3f(1.0));
  return vec4f(round(rgb * 255.0), a) / 255.0;
}
