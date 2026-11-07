// Match Image::halved: even axes average two taps, odd axes weight three 1:2:1.
// Integer sums and rounding keep every mip texel identical to the CPU result.
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var output: texture_storage_2d<rgba8unorm, write>;

fn taps(n: u32) -> u32 {
  if (n == 1u) { return 1u; }
  return 2u + (n & 1u);
}
fn weight(i: u32, n: u32) -> u32 {
  return select(1u, 2u, n == 3u && i == 1u);
}
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) at: vec3u) {
  let size = textureDimensions(output);
  if (any(at.xy >= size)) { return; }
  let above = textureDimensions(source);
  let count = vec2u(taps(above.x), taps(above.y));
  var sum = vec4u(0u);
  var total = 0u;
  for (var y = 0u; y < count.y; y++) {
    for (var x = 0u; x < count.x; x++) {
      let w = weight(x, count.x) * weight(y, count.y);
      let p = min(2u * at.xy + vec2u(x, y), above - 1u);
      sum += vec4u(round(textureLoad(source, vec2i(p), 0) * 255.0)) * w;
      total += w;
    }
  }
  textureStore(output, vec2i(at.xy), vec4f((sum + total / 2u) / total) / 255.0);
}
