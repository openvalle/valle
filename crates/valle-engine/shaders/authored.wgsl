// The interface shared by authored effects. UV and input textures have their origin at the
// bottom left. Colors are sRGB with straight alpha; the fragment entry handles coverage and
// premultiplication, as every other painter does before compositing.
struct ShaderInput {
  uv: vec2f,
  size: vec2f,
  pixel: vec2f,
}

struct ValleHost {
  h: array<vec4f, 4>,
}
@group(1) @binding(0) var<uniform> valle_host: ValleHost;

@fragment fn valle_fragment(@location(0) local: vec2f) -> @location(0) vec4f {
  let size = valle_host.h[0].xy;
  let fw = max(fwidth(local), vec2f(1e-6));
  let half = 0.5 * size + valle_host.h[0].z;
  let d = abs(local - 0.5 * size);
  let coverage = clamp((half - d) / fw + 0.5, vec2f(0.0), vec2f(1.0));
  let uv = vec2f(local.x, size.y - local.y) / size;
  let color = clamp(shade(ShaderInput(uv, size, uv * valle_host.h[1].xy)), vec4f(0.0), vec4f(1.0));
  return vec4f(color.rgb * color.a, color.a) * (coverage.x * coverage.y);
}
