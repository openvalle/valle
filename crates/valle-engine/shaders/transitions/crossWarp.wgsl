struct Params {
  progress: f32,
  centerX: f32,
  centerY: f32,
}
@group(2) @binding(0) var<uniform> params: Params;

fn shade(input: ShaderInput) -> vec4f {
  let uv = input.uv;
  let progress = clamp(params.progress, 0.0, 1.0);
  let c = vec2f(params.centerX, 1.0 - params.centerY);
  let x = smoothstep(0.0, 1.0, progress * 2.0 + uv.x - 1.0);
  return mix(getFromColor((uv - c) * (1.0 - x) + c), getToColor((uv - c) * x + c), x);
}
