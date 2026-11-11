// Matches image::yuv::rgba, including centered chroma and rounding to an 8-bit base image.
struct Frame {
  size: vec2u,
  bt709: u32,
  full: u32,
  y: vec4u,
  u: vec4u,
  v: vec4u,
  a: vec4u,
  orientation: vec4u,
}
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<storage, read> planes: array<u32>;
@group(0) @binding(2) var output: texture_storage_2d<rgba8unorm, write>;

fn sample(plane: vec4u, at: vec2u) -> f32 {
  let i = plane.x + at.y * plane.y + at.x * plane.z;
  return f32((planes[i / 4u] >> ((i % 4u) * 8u)) & 255u);
}

fn chroma(plane: vec4u, lo: vec2u, hi: vec2u, t: vec2f, scale: f32) -> f32 {
  let a = (sample(plane, lo) - 128.0) * scale;
  let b = (sample(plane, vec2u(hi.x, lo.y)) - 128.0) * scale;
  let c = (sample(plane, vec2u(lo.x, hi.y)) - 128.0) * scale;
  let d = (sample(plane, hi) - 128.0) * scale;
  let top = a * (1.0 - t.x) + b * t.x;
  let bottom = c * (1.0 - t.x) + d * t.x;
  return top * (1.0 - t.y) + bottom * t.y;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
  let at = id.xy;
  if any(at >= frame.size) { return; }
  let last = (frame.size + 1u) / 2u - 1u;
  let c = clamp((vec2f(at) + 0.5) / 2.0 - 0.5, vec2f(0.0), vec2f(last));
  let lo = vec2u(floor(c));
  let hi = min(lo + 1u, last);
  let t = c - floor(c);
  let cs = select(255.0 / 224.0, 1.0, frame.full == 1u);
  let cb = chroma(frame.u, lo, hi, t, cs);
  let cr = chroma(frame.v, lo, hi, t, cs);
  let kr = select(0.299, 0.2126, frame.bt709 == 1u);
  let kb = select(0.114, 0.0722, frame.bt709 == 1u);
  let kg = 1.0 - kr - kb;
  let rv = 2.0 * (1.0 - kr);
  let bu = 2.0 * (1.0 - kb);
  let gu = -bu * kb / kg;
  let gv = -rv * kr / kg;
  let y0 = select(16.0, 0.0, frame.full == 1u);
  let ys = select(255.0 / 219.0, 1.0, frame.full == 1u);
  let y = (sample(frame.y, at) - y0) * ys;
  let rgb = floor(vec3f(y + rv * cr, y + gu * cb + gv * cr, y + bu * cb) + 0.5);
  var alpha = 255.0;
  if frame.a.z != 0u { alpha = sample(frame.a, at); }
  var destination = at;
  var width = frame.size.x;
  switch frame.orientation.x {
    case 1u: { destination = vec2u(frame.size.y - 1u - at.y, at.x); width = frame.size.y; }
    case 2u: { destination = frame.size - 1u - at; }
    case 3u: { destination = vec2u(at.y, frame.size.x - 1u - at.x); width = frame.size.y; }
    default: {}
  }
  if frame.orientation.y != 0u { destination.x = width - 1u - destination.x; }
  textureStore(output, vec2i(destination), clamp(vec4f(rgb, alpha), vec4f(0.0), vec4f(255.0)) / 255.0);
}
