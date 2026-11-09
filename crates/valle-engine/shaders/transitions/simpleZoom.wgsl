struct Params {
  progress: f32,
  quickness: f32,
  centerX: f32,
  centerY: f32,
}
@group(2) @binding(0) var<uniform> params: Params;

fn shade(input: ShaderInput) -> vec4f {
  let uv = input.uv;
  let progress = clamp(params.progress, 0.0, 1.0);
  let c = vec2f(params.centerX, 1.0 - params.centerY);
  let zoom = smoothstep(0.0, params.quickness, progress);
  let source = getFromColor(c + (uv - c) * (1.0 - zoom));
  return mix(source, getToColor(uv), smoothstep(params.quickness - 0.2, 1.0, progress));
}
