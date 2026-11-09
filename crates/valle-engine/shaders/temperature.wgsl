struct Params { temperature: f32 }
@group(2) @binding(0) var<uniform> params: Params;
@group(2) @binding(1) var backdrop: texture_2d<f32>;
@group(2) @binding(2) var sampling: sampler;

fn shade(input: ShaderInput) -> vec4f {
  let c = textureSampleLevel(backdrop, sampling, input.uv, 0.0);
  let g = exp2(params.temperature * 0.25);
  return vec4f(c.r * g, c.g, c.b / g, c.a);
}
