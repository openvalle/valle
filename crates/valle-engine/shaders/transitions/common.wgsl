// The two isolated child layers of a Transition, with straight-alpha sRGB pixels.
@group(2) @binding(1) var valle_from: texture_2d<f32>;
@group(2) @binding(2) var valle_to: texture_2d<f32>;
@group(2) @binding(3) var valle_sampling: sampler;
fn getFromColor(uv: vec2f) -> vec4f {
  return textureSampleLevel(valle_from, valle_sampling, uv, 0.0);
}
fn getToColor(uv: vec2f) -> vec4f {
  return textureSampleLevel(valle_to, valle_sampling, uv, 0.0);
}
