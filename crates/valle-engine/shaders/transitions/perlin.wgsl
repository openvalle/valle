struct Params {
  progress: f32,
  scale: f32,
  smoothness: f32,
}
@group(2) @binding(0) var<uniform> params: Params;

fn lattice(cell: vec2f) -> f32 {
  let q = vec2u(vec2i(cell) + vec2i(4096));
  var h = q.x * 1664525u + q.y * 1013904223u;
  h ^= h >> 16u;
  h *= 2246822519u;
  h ^= h >> 13u;
  return f32(h & 65535u) / 65535.0;
}
fn noise(p: vec2f) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f);
  let a = lattice(i);
  let b = lattice(i + vec2f(1.0, 0.0));
  let c = lattice(i + vec2f(0.0, 1.0));
  let d = lattice(i + vec2f(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn shade(input: ShaderInput) -> vec4f {
  let uv = input.uv;
  let progress = clamp(params.progress, 0.0, 1.0);
  let n = noise(uv * params.scale);
  let p = mix(-params.smoothness, 1.0 + params.smoothness, progress);
  let m = 1.0 - smoothstep(p - params.smoothness, p + params.smoothness, n);
  return mix(getFromColor(uv), getToColor(uv), m);
}
