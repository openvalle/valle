struct Params {
  progress: f32,
  directionX: f32,
  directionY: f32,
  softness: f32,
}
@group(2) @binding(0) var<uniform> params: Params;

fn shade(input: ShaderInput) -> vec4f {
  let uv = input.uv;
  let progress = clamp(params.progress, 0.0, 1.0);
  var v = vec2f(params.directionX, -params.directionY);
  if length(v) > 0.0 { v = normalize(v); } else { v = vec2f(1.0, 0.0); }
  v /= abs(v.x) + abs(v.y);
  let d = v.x * 0.5 + v.y * 0.5;
  let m = 1.0 - smoothstep(-params.softness, 0.0, v.x * uv.x + v.y * uv.y - (d - 0.5 + progress * (1.0 + params.softness)));
  return mix(getFromColor((uv - 0.5) * (1.0 - m) + 0.5), getToColor((uv - 0.5) * m + 0.5), m);
}
