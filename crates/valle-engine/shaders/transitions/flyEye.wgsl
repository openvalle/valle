struct Params {
  progress: f32,
  size: f32,
  frequency: f32,
  colorSeparation: f32,
}
@group(2) @binding(0) var<uniform> params: Params;

fn shade(input: ShaderInput) -> vec4f {
  let uv = input.uv;
  let progress = clamp(params.progress, 0.0, 1.0);
  let rest = 1.0 - progress;
  let d = params.size * vec2f(cos(params.frequency * uv.x), sin(params.frequency * uv.y));
  let destination = getToColor(uv + rest * d);
  let g = getFromColor(uv + progress * d);
  let r = getFromColor(uv + progress * d * (1.0 - params.colorSeparation)).r;
  let b = getFromColor(uv + progress * d * (1.0 + params.colorSeparation)).b;
  return mix(vec4f(r, g.g, b, g.a), destination, progress);
}
