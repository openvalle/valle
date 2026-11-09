struct Params {
  progress: f32,
  frequency: f32,
  speed: f32,
  amplitude: f32,
}
@group(2) @binding(0) var<uniform> params: Params;

fn shade(input: ShaderInput) -> vec4f {
  let uv = input.uv;
  let progress = clamp(params.progress, 0.0, 1.0);
  let d = uv - 0.5;
  let r = length(d);
  let offset = d * (sin(progress * r * params.frequency - progress * params.speed) + 0.5) * params.amplitude;
  return mix(getFromColor(uv + offset), getToColor(uv), smoothstep(0.2, 1.0, progress));
}
