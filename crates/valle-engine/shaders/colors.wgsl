// A layer's colors through 4 × 5 matrices into a new target, which the pass replaces: unpremultiplied,
// each matrix's result clamped to [0, 1] before the next, as CSS's filter functions have them. The
// layer is read moved by an offset, between its pixels linearly, and transparent past its edges.

struct Colors {
  // In the first of a run: x, how many matrices follow; yz, the offset, px. In the others: the
  // factors of red, green, blue and alpha in each of them, then their offsets.
  m: array<vec4f, 5>,
}

@group(0) @binding(5) var<storage, read> colors: array<Colors>;
@group(1) @binding(0) var image: texture_2d<f32>;

struct Out {
  @builtin(position) pos: vec4f,
  @location(0) @interpolate(flat) index: u32,
}

@vertex fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Out {
  var out: Out;
  out.pos = whole(vi);
  out.index = ii;
  return out;
}

fn texel(p: vec2<i32>) -> vec4f {
  let size = vec2<i32>(textureDimensions(image));
  if (any(p < vec2<i32>(0)) || any(p >= size)) {
    return vec4f(0.0);
  }
  return textureLoad(image, p, 0);
}

@fragment fn fs(in: Out) -> @location(0) vec4f {
  let head = colors[in.index].m[0];
  // The pixel's center less the offset, from the first pixel's center.
  let q = in.pos.xy - head.yz - 0.5;
  let f = floor(q);
  let t = q - f;
  let p = vec2<i32>(f);
  let top = mix(texel(p), texel(p + vec2<i32>(1, 0)), t.x);
  let bottom = mix(texel(p + vec2<i32>(0, 1)), texel(p + vec2<i32>(1, 1)), t.x);
  let c = mix(top, bottom, t.y);
  var u = vec4f(0.0);
  if (c.a > 0.0) {
    u = vec4f(c.rgb / c.a, c.a);
  }
  for (var i = 1u; i <= u32(head.x); i++) {
    let m = colors[in.index + i].m;
    let v = vec4f(dot(m[0], u), dot(m[1], u), dot(m[2], u), dot(m[3], u)) + m[4];
    u = clamp(v, vec4f(0.0), vec4f(1.0));
  }
  return vec4f(u.rgb * u.a, u.a);
}
