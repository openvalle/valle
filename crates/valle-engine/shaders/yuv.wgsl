// The SDR output as an encoder takes it: straight 8-bit sRGB, as `output` writes it, to BT.709
// Y'CbCr at limited range, luma at every pixel and chroma, the mean of each 2 × 2 block's, at
// half size each way rounded up: the planes the browser's export makes on the CPU.

@group(1) @binding(0) var image: texture_2d<f32>;

const K = vec3f(0.2126, 0.7152, 0.0722);

@vertex fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4f {
  return whole(vi);
}

fn rgb(p: vec2<i32>) -> vec3f {
  return round(textureLoad(image, p, 0).rgb * 255.0);
}

@fragment fn luma(@builtin(position) pos: vec4f) -> @location(0) vec4f {
  let y = round(16.0 + 219.0 / 255.0 * dot(K, rgb(vec2<i32>(pos.xy))));
  return vec4f(clamp(y, 0.0, 255.0) / 255.0, 0.0, 0.0, 1.0);
}

@fragment fn chroma(@builtin(position) pos: vec4f) -> @location(0) vec4f {
  let size = vec2<i32>(textureDimensions(image));
  let at = vec2<i32>(pos.xy) * 2;
  var c = vec2f(0.0);
  var n = 0.0;
  for (var k = 0; k < 4; k++) {
    let p = at + vec2<i32>(k & 1, k >> 1);
    if (all(p < size)) {
      let v = rgb(p);
      let l = dot(K, v);
      c += vec2f((v.b - l) / (2.0 * (1.0 - K.z)), (v.r - l) / (2.0 * (1.0 - K.x)));
      n += 1.0;
    }
  }
  let uv = round(128.0 + 224.0 / 255.0 * c / n);
  return vec4f(clamp(uv, vec2f(0.0), vec2f(255.0)) / 255.0, 0.0, 1.0);
}
