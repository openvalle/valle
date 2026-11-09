struct Params {
  progress: f32,
  midpoint: f32,
}
@group(2) @binding(0) var<uniform> params: Params;

fn shade(input: ShaderInput) -> vec4f {
  let uv = input.uv;
  let progress = clamp(params.progress, 0.0, 1.0);
  let a = getFromColor(uv);
  let b = getToColor(uv);
  let both = a * b;
  if progress < params.midpoint { return mix(a, both, progress / params.midpoint); }
  return mix(both, b, (progress - params.midpoint) / (1.0 - params.midpoint));
}
