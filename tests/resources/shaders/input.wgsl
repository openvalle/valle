// An external shader with a sparse texture binding and explicit integer sampling.
@group(2) @binding(3) var picture: texture_2d<f32>;
fn shade(input: ShaderInput) -> vec4f {
  let size = vec2i(textureDimensions(picture));
  let at = clamp(vec2i(input.uv * vec2f(size)), vec2i(0), size - vec2i(1));
  return textureLoad(picture, at, 0);
}
