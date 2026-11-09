struct Params {
  progress: f32,
  intensity: f32,
}
@group(2) @binding(0) var<uniform> params: Params;

fn shade(input: ShaderInput) -> vec4f {
  let uv = input.uv;
  let progress = clamp(params.progress, 0.0, 1.0);
  let spread = params.intensity * (0.5 - distance(0.5, progress));
  var a = vec4f(0.0);
  var b = vec4f(0.0);
  for (var i = 0; i < 6; i++) {
    for (var j = 0; j < 6; j++) {
      let at = uv + spread * (vec2f(f32(i), f32(j)) / 6.0 - 0.5);
      a += getFromColor(at);
      b += getToColor(at);
    }
  }
  return mix(a / 36.0, b / 36.0, progress);
}
