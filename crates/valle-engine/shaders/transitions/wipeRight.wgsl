struct Params {
  progress: f32,
  softness: f32,
}
@group(2) @binding(0) var<uniform> params: Params;

fn shade(input: ShaderInput) -> vec4f {
  let uv = input.uv;
  let progress = clamp(params.progress, 0.0, 1.0);
  let edge = progress * (1.0 + 2.0 * params.softness) - params.softness;
  var m = step(uv.x, edge);
  if params.softness > 0.0 { m = 1.0 - smoothstep(edge - params.softness, edge + params.softness, uv.x); }
  return mix(getFromColor(uv), getToColor(uv), m);
}
