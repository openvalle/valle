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
