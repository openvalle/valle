struct Params {
  progress: f32,
  centerX: f32,
  centerY: f32,
  softness: f32,
}
@group(2) @binding(0) var<uniform> params: Params;

fn shade(input: ShaderInput) -> vec4f {
  let uv = input.uv;
  let progress = clamp(params.progress, 0.0, 1.0);
  let size = input.size;
  let c = vec2f(params.centerX, 1.0 - params.centerY) * size;
  let far = max(c, size - c);
  let s = max(params.softness, 0.5);
  let r = progress * (length(far) + 2.0 * s) - s;
  let m = 1.0 - smoothstep(r - s, r + s, length(uv * size - c));
  return mix(getFromColor(uv), getToColor(uv), m);
}
