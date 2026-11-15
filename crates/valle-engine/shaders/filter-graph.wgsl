// SVG filter graph kernels. Intermediate textures contain premultiplied encoded sRGB;
// color operations explicitly enter/leave the primitive's selected space.
struct Parameters { m: array<vec4f, 5> }
@group(0) @binding(5) var<storage, read> graph: array<Parameters>;
@group(1) @binding(0) var first: texture_2d<f32>;
@group(2) @binding(0) var second: texture_2d<f32>;
struct Out {
  @builtin(position) pos: vec4f,
  @location(0) @interpolate(flat) index: u32,
}
@vertex fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Out {
  return Out(whole(vi), ii);
}
fn pixel(image: texture_2d<f32>, p: vec2i) -> vec4f {
  if (any(p < vec2i(0)) || any(p >= vec2i(textureDimensions(image)))) {
    return vec4f(0.0);
  }
  return textureLoad(image, p, 0);
}
fn sample_at(image: texture_2d<f32>, at: vec2f) -> vec4f {
  let q = at - 0.5;
  let f = floor(q);
  let t = q - f;
  let p = vec2i(f);
  return mix(
    mix(pixel(image, p), pixel(image, p + vec2i(1, 0)), t.x),
    mix(pixel(image, p + vec2i(0, 1)), pixel(image, p + vec2i(1, 1)), t.x), t.y);
}
fn straight(c: vec4f) -> vec4f {
  if (c.a > 0.0) { return vec4f(c.rgb / c.a, c.a); }
  return vec4f(0.0);
}
fn decode(c: vec3f) -> vec3f {
  return select(c / 12.92, pow(max((c + 0.055) / 1.055, vec3f(0.0)), vec3f(2.4)), c > vec3f(0.04045));
}
fn encode(c: vec3f) -> vec3f {
  return select(c * 12.92, 1.055 * pow(max(c, vec3f(0.0)), vec3f(1.0 / 2.4)) - 0.055, c > vec3f(0.0031308));
}
fn working(c: vec4f, linear: bool) -> vec4f {
  let u = straight(c);
  return vec4f(select(u.rgb, decode(u.rgb), linear) * u.a, u.a);
}
fn output(c: vec4f, linear: bool) -> vec4f {
  let v = clamp(c, vec4f(0.0), vec4f(1.0));
  let u = straight(v);
  return vec4f(clamp(select(u.rgb, encode(u.rgb), linear), vec3f(0.0), vec3f(1.0)) * u.a, u.a);
}
fn data(index: u32, at: u32) -> f32 {
  return graph[index + 1u + at / 20u].m[(at % 20u) / 4u][at % 4u];
}
fn transfer(index: u32, c: vec4f) -> vec4f {
  var result = c;
  for (var k = 0u; k < 4u; k++) {
    let offset = k * 4u;
    let mode = u32(data(index, offset));
    let a = data(index, offset + 1u);
    let b = data(index, offset + 2u);
    let d = data(index, offset + 3u);
    if (mode == 1u || mode == 2u) {
      let n = u32(b);
      let start = u32(a);
      if (n > 0u) {
        if (mode == 2u) {
          result[k] = data(index, start + min(u32(floor(c[k] * f32(n))), n - 1u));
        } else {
          let p = c[k] * f32(n - 1u);
          let low = min(u32(floor(p)), n - 1u);
          result[k] = mix(data(index, start + low), data(index, start + min(low + 1u, n - 1u)), fract(p));
        }
      }
    } else if (mode == 3u) {
      result[k] = a * c[k] + b;
    } else if (mode == 4u) {
      result[k] = a * pow(max(c[k], 0.0), b) + d;
    }
  }
  return clamp(result, vec4f(0.0), vec4f(1.0));
}
// Permutation and four independent gradient tables use SVG 1.1's Park-Miller seed.
fn lattice(index: u32, n: i32) -> i32 { return i32(data(index, u32(n & 255))); }
fn gradient(index: u32, n: i32, p: vec2f) -> vec4f {
  var g = vec4f(0.0);
  for (var c = 0u; c < 4u; c++) {
    let at = 256u + (c * 256u + u32(n)) * 2u;
    g[c] = dot(vec2f(data(index, at), data(index, at + 1u)), p);
  }
  return g;
}
fn noise(index: u32, p: vec2f, stitch: vec4f) -> vec4f {
  let q = p + 4096.0;
  let f = floor(q);
  let r = q - f;
  var lo = vec2i(f);
  var hi = lo + 1;
  if (stitch.x > 0.0) {
    if (f32(lo.x) >= stitch.z) { lo.x -= i32(stitch.x); }
    if (f32(hi.x) >= stitch.z) { hi.x -= i32(stitch.x); }
  }
  if (stitch.y > 0.0) {
    if (f32(lo.y) >= stitch.w) { lo.y -= i32(stitch.y); }
    if (f32(hi.y) >= stitch.w) { hi.y -= i32(stitch.y); }
  }
  let i = lattice(index, lo.x);
  let j = lattice(index, hi.x);
  let s = r * r * (3.0 - 2.0 * r);
  let a = mix(gradient(index, lattice(index, i + lo.y), r),
    gradient(index, lattice(index, j + lo.y), r - vec2f(1.0, 0.0)), s.x);
  let b = mix(gradient(index, lattice(index, i + hi.y), r - vec2f(0.0, 1.0)),
    gradient(index, lattice(index, j + hi.y), r - vec2f(1.0)), s.x);
  return mix(a, b, s.y);
}
fn turbulence(index: u32, at: vec2f) -> vec4f {
  let h = graph[index].m;
  var p = (at - h[3].xy) / h[3].zw * h[2].xy;
  var stitch = h[4];
  var sum = vec4f(0.0);
  var amplitude = 1.0;
  let fractal = h[2].w != 0.0;
  for (var octave = 0u; octave < u32(h[2].z); octave++) {
    let value = noise(index, p, stitch);
    sum += select(abs(value), value, fractal) * amplitude;
    amplitude *= 0.5;
    // Further octaves are below f32 precision, well below the RGBA16F target.
    if (amplitude < 0.00000006) { break; }
    p *= 2.0;
    stitch = vec4f(stitch.xy * 2.0, stitch.zw * 2.0 - 4096.0);
  }
  let value = clamp(select(sum, sum * 0.5 + 0.5, fractal), vec4f(0.0), vec4f(1.0));
  return vec4f(value.rgb * value.a, value.a);
}

fn wrapped_pixel(t: u32, p: vec2i, r: vec4f) -> vec4f {
  let q = local(vec2f(p) + 0.5, t);
  let wrapped = r.xy + fract((q - r.xy) / r.zw) * r.zw;
  let at = transforms[t].forward * vec3f(wrapped, 1.0);
  return sample_at(first, at.xy / at.z);
}
// Convolution edges are relative to the input primitive, not the full render target.
fn kernel_sample(index: u32, at: vec2f, linear: bool) -> vec4f {
  let h = graph[index].m;
  let r = vec4f(data(index, 0u), data(index, 1u), data(index, 2u), data(index, 3u));
  if (any(r.zw <= vec2f(0.0))) { return vec4f(0.0); }
  var p = at;
  let edge = u32(h[4].z);
  if (edge == 0u) {
    let half_pixel = vec2f(data(index, 4u), data(index, 5u)) * 0.5;
    p = clamp(p, r.xy + min(half_pixel, r.zw * 0.5), r.xy + r.zw - min(half_pixel, r.zw * 0.5));
  } else if (edge == 1u) {
    p = r.xy + fract((p - r.xy) / r.zw) * r.zw;
  } else if (any(p < r.xy) || any(p >= r.xy + r.zw)) { return vec4f(0.0); }
  let point = transforms[u32(h[0].z)].forward * vec3f(p, 1.0);
  if (edge == 1u) {
    // Wrap each bilinear tap as well, so a fractional offset cannot sample
    // transparent pixels just outside the input instead of its opposite edge.
    let q = point.xy / point.z - 0.5;
    let f = floor(q);
    let p0 = vec2i(f);
    let t = u32(h[0].z);
    let value = mix(mix(wrapped_pixel(t, p0, r), wrapped_pixel(t, p0 + vec2i(1, 0), r), q.x - f.x),
      mix(wrapped_pixel(t, p0 + vec2i(0, 1), r), wrapped_pixel(t, p0 + vec2i(1, 1), r), q.x - f.x), q.y - f.y);
    return working(value, linear);
  }
  return working(sample_at(first, point.xy / point.z), linear);
}
fn convolve(v: Out, linear: bool) -> vec4f {
  let h = graph[v.index].m;
  let order = vec2u(h[2].xy);
  let anchor = h[2].zw;
  let unit = h[3].xy;
  let preserve = h[4].w != 0.0;
  let at = local(v.pos.xy, u32(h[0].z));
  var sum = vec4f(0.0);
  for (var row = 0u; row < order.y; row++) {
    for (var col = 0u; col < order.x; col++) {
      let weight = data(v.index, 6u + (order.y - row - 1u) * order.x + order.x - col - 1u);
      if (weight != 0.0) {
        let sample = kernel_sample(v.index, at + (vec2f(f32(col), f32(row)) - anchor) * unit, linear);
        sum += select(sample, straight(sample), preserve) * weight;
      }
    }
  }
  sum /= h[3].z;
  let alpha = select(clamp(sum.a + h[3].w, 0.0, 1.0), pixel(first, vec2i(v.pos.xy)).a, preserve);
  if (preserve) { return vec4f(clamp(sum.rgb + h[3].w, vec3f(0.0), vec3f(1.0)) * alpha, alpha); }
  return vec4f(clamp(sum.rgb + h[3].w * alpha, vec3f(0.0), vec3f(alpha)), alpha);
}

fn height_at(t: u32, at: vec2f) -> f32 {
  let p = transforms[t].forward * vec3f(at, 1.0);
  return sample_at(first, p.xy / p.z).a;
}
fn unit_vector(v: vec3f) -> vec3f {
  let d = length(v);
  if (d > 0.0) { return v / d; }
  return vec3f(0.0);
}
fn lighting(v: Out, linear: bool) -> vec4f {
  let h = graph[v.index].m;
  let t = u32(h[0].z);
  let at = local(v.pos.xy, t);
  let unit = h[2].xy;
  let bounds = vec4f(data(v.index, 0u), data(v.index, 1u), data(v.index, 2u), data(v.index, 3u));
  let lo = max(at - unit, bounds.xy + unit * 0.5);
  let hi = min(at + unit, bounds.xy + bounds.zw - unit * 0.5);
  // Truncate the Sobel kernel at the input edge, then normalize by its surviving
  // weights and distance. This gives SVG's distinct corner/edge/interior factors.
  let left = at.x - unit.x >= bounds.x;
  let right = at.x + unit.x < bounds.x + bounds.z;
  let top = at.y - unit.y >= bounds.y;
  let bottom = at.y + unit.y < bounds.y + bounds.w;
  var dx = 2.0 * (height_at(t, vec2f(hi.x, at.y)) - height_at(t, vec2f(lo.x, at.y)));
  var dy = 2.0 * (height_at(t, vec2f(at.x, hi.y)) - height_at(t, vec2f(at.x, lo.y)));
  if (top) { dx += height_at(t, vec2f(hi.x, at.y - unit.y)) - height_at(t, vec2f(lo.x, at.y - unit.y)); }
  if (bottom) { dx += height_at(t, vec2f(hi.x, at.y + unit.y)) - height_at(t, vec2f(lo.x, at.y + unit.y)); }
  if (left) { dy += height_at(t, vec2f(at.x - unit.x, hi.y)) - height_at(t, vec2f(at.x - unit.x, lo.y)); }
  if (right) { dy += height_at(t, vec2f(at.x + unit.x, hi.y)) - height_at(t, vec2f(at.x + unit.x, lo.y)); }
  let weights = vec2f(2.0 + f32(top) + f32(bottom), 2.0 + f32(left) + f32(right));
  let slope = 2.0 * vec2f(dx, dy) / (weights * max(hi - lo, vec2f(0.000001)));
  let normal = normalize(vec3f(-h[2].z * slope, 1.0));
  let point = vec3f(at, height_at(t, at) * h[2].z);
  let kind = u32(data(v.index, 4u));
  let position = vec3f(data(v.index, 5u), data(v.index, 6u), data(v.index, 7u));
  var light = position;
  if (kind != 0u) { light = unit_vector(position - point); }
  var intensity = 1.0;
  if (kind == 2u) {
    let aim = vec3f(data(v.index, 8u), data(v.index, 9u), data(v.index, 10u));
    let direction = unit_vector(aim - position);
    let cosine = dot(-light, direction);
    let cone = data(v.index, 12u);
    if (cosine <= 0.0 || cosine < cone) { intensity = 0.0; }
    else {
      intensity = pow(cosine, data(v.index, 11u));
      // Smooth the cone cutoff over roughly one sample footprint.
      if (cone > -1.0) {
        let footprint = max(unit.x, unit.y) / max(length(position - point), 0.000001);
        intensity *= clamp((cosine - cone) / max(footprint, 0.000001), 0.0, 1.0);
      }
    }
  }
  let color = select(h[3].rgb, decode(h[3].rgb), linear);
  let amount = h[2].w * max(dot(normal, light), 0.0) * intensity;
  return vec4f(clamp(color * amount, vec3f(0.0), vec3f(1.0)), 1.0);
}

fn filtered(v: Out) -> vec4f {
  let h = graph[v.index].m;
  let op = u32(h[0].x);
  let linear = h[0].y != 0.0;
  let t = u32(h[0].z);
  let a = pixel(first, vec2i(v.pos.xy));
  let b = pixel(second, vec2i(v.pos.xy));
  let x = working(a, linear);
  let y = working(b, linear);
  var result = x;
  switch (op) {
    case 0u: { return a; }
    case 1u: { return vec4f(0.0, 0.0, 0.0, a.a); }
    case 2u: { return vec4f(h[2].rgb * h[2].a, h[2].a); }
    case 3u: { return sample_at(first, v.pos.xy - h[2].xy); }
    case 4u: {
      let u = straight(x);
      var c = vec4f(0.0);
      for (var row = 0u; row < 4u; row++) {
        c[row] = dot(vec4f(data(v.index, row * 5u), data(v.index, row * 5u + 1u),
          data(v.index, row * 5u + 2u), data(v.index, row * 5u + 3u)), u) + data(v.index, row * 5u + 4u);
      }
      c = clamp(c, vec4f(0.0), vec4f(1.0));
      result = vec4f(c.rgb * c.a, c.a);
    }
    case 5u: {
      let rgb = mixed(u32(h[2].x), straight(y).rgb, straight(x).rgb);
      result = vec4f((1.0 - x.a) * y.rgb + (1.0 - y.a) * x.rgb + x.a * y.a * rgb, x.a + y.a - x.a * y.a);
    }
    case 6u: {
      switch (u32(h[2].x)) {
        case 1u: { result = x * y.a; }
        case 2u: { result = x * (1.0 - y.a); }
        case 3u: { result = x * y.a + y * (1.0 - x.a); }
        case 4u: { result = x * (1.0 - y.a) + y * (1.0 - x.a); }
        case 5u: {
          let k = h[3];
          result = clamp(k.x * x * y + k.y * x + k.z * y + vec4f(k.w), vec4f(0.0), vec4f(1.0));
          result = vec4f(min(result.rgb, vec3f(result.a)), result.a);
        }
        default: { result = x + y * (1.0 - x.a); }
      }
    }
    case 7u: {
      let u = transfer(v.index, straight(x));
      result = vec4f(u.rgb * u.a, u.a);
    }
    case 8u: {
      let sigma = h[2].z;
      let axis = h[2].xy;
      if (sigma > 0.0) {
        let n = i32(ceil(3.0 * sigma));
        var sum = x;
        var total = 1.0;
        for (var i = 1; i <= n; i++) {
          let distance = f32(i);
          let weight = exp(-0.5 * distance * distance / (sigma * sigma));
          let d = axis * distance;
          sum += (working(sample_at(first, v.pos.xy - d), linear) + working(sample_at(first, v.pos.xy + d), linear)) * weight;
          total += 2.0 * weight;
        }
        result = sum / total;
      }
    }
    case 9u: {
      let radius = i32(floor(h[2].z));
      let axis = h[2].xy;
      let dilate = h[2].w != 0.0;
      for (var i = 1; i <= radius; i++) {
        let d = axis * f32(i);
        let l = working(sample_at(first, v.pos.xy - d), linear);
        let r = working(sample_at(first, v.pos.xy + d), linear);
        result = select(min(result, min(l, r)), max(result, max(l, r)), dilate);
      }
    }
    case 10u: {
      let u = straight(y);
      let delta = (vec2f(u[u32(h[2].z)], u[u32(h[2].w)]) - 0.5) * h[2].xy;
      let q = local(v.pos.xy, t);
      let at = transforms[t].forward * vec3f(q + delta, 1.0);
      return sample_at(first, at.xy / at.z);
    }
    case 11u: {
      let alpha = a.a * h[2].a;
      return vec4f(h[2].rgb * alpha, alpha);
    }
    // Match the noise raster sampling phase used by Chrome and WebKit.
    case 13u: { result = turbulence(v.index, local(v.pos.xy + 0.5, t)); }
    case 14u: { result = convolve(v, linear); }
    case 15u: { result = lighting(v, linear); }
    case 12u: {
      let tile = h[2];
      if (any(tile.zw <= vec2f(0.0))) { return vec4f(0.0); }
      let q = local(v.pos.xy, t);
      let wrapped = tile.xy + fract((q - tile.xy) / tile.zw) * tile.zw;
      let at = transforms[t].forward * vec3f(wrapped, 1.0);
      return sample_at(first, at.xy / at.z);
    }
    default: {}
  }
  return output(result, linear);
}
@fragment fn fs(v: Out) -> @location(0) vec4f {
  let h = graph[v.index].m;
  let q = local(v.pos.xy, u32(h[0].z));
  let region = h[1];
  let distance = abs(q - region.xy - region.zw * 0.5);
  let coverage = clamp((region.zw * 0.5 - distance) / max(fwidth(q), vec2f(1e-6)) + 0.5, vec2f(0.0), vec2f(1.0));
  if (any(coverage <= vec2f(0.0))) { return vec4f(0.0); }
  return filtered(v) * coverage.x * coverage.y;
}
