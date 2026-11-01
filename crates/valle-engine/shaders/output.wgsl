// The SDR output transform: premultiplied linear Rec.2020 working colors to straight sRGB,
// quantized to 8 bits here, so every GPU stores the same bytes.

@group(1) @binding(0) var image: texture_2d<f32>;

@vertex fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4f {
  return whole(vi);
}

fn encode(v: f32) -> f32 {
  let c = clamp(v, 0.0, 1.0);
  return select(1.055 * pow(c, 1.0 / 2.4) - 0.055, c * 12.92, c <= 0.0031308);
}

@fragment fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
  let c = textureLoad(image, vec2<i32>(pos.xy), 0);
  let a = round(clamp(c.a, 0.0, 1.0) * 255.0);
  if (a == 0.0) {
    return vec4f(0.0);
  }
  let rgb = c.rgb / c.a;
  // Linear Rec.2020 to linear sRGB.
  let srgb = vec3f(
    dot(rgb, vec3f(1.660491002108434, -0.587641138788550, -0.072849863319885)),
    dot(rgb, vec3f(-0.124550474521591, 1.132899897125960, -0.008349422604369)),
    dot(rgb, vec3f(-0.018150763354905, -0.100578898008007, 1.118729661362913)),
  );
  let e = vec3f(encode(srgb.r), encode(srgb.g), encode(srgb.b));
  return vec4f(round(e * 255.0), a) / 255.0;
}
