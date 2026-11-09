struct Params {
  progress: f32,
  strength: f32,
  centerX: f32,
  centerY: f32,
}
@group(2) @binding(0) var<uniform> params: Params;

fn shade(input: ShaderInput) -> vec4f {
  let uv = input.uv;
  let progress = clamp(params.progress, 0.0, 1.0);
  let ratio = input.size.x / input.size.y;
  let c = vec2f(params.centerX, 1.0 - params.centerY);
  let k = sin(progress * 3.14159265) * params.strength;
  let a = radians(12.0) * k;
  var p = mat2x2f(vec2f(cos(a), sin(a)), vec2f(-sin(a), cos(a))) * ((uv - c) * vec2f(ratio, 1.0));
  p = p / vec2f(ratio, 1.0) / (1.0 + 0.5 * k) + c;
  return mix(getFromColor(p), getToColor(p), smoothstep(0.0, 1.0, progress));
}
