// Valle's effects on a layer into a new target, which the pass replaces. The layer is read
// between its pixels linearly, and transparent past its edges.
//
// - 0, chromatic aberration: red from half an offset back and blue from half of it on, so red
//   moves on and blue back, and green where it is; the alpha the most of the three's, as each
//   channel already carries its own.

struct Effect {
  // x: which effect; then its parameters.
  head: vec4f,
  more: array<vec4f, 4>,
}

@group(0) @binding(5) var<storage, read> effects: array<Effect>;
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

// The layer at `q`, px from its top left, between its pixels' centers linearly.
fn sampled(q: vec2f) -> vec4f {
  let c = q - 0.5;
  let f = floor(c);
  let t = c - f;
  let p = vec2<i32>(f);
  let top = mix(texel(p), texel(p + vec2<i32>(1, 0)), t.x);
  let bottom = mix(texel(p + vec2<i32>(0, 1)), texel(p + vec2<i32>(1, 1)), t.x);
  return mix(top, bottom, t.y);
}

fn aberration(p: vec2f, offset: vec2f) -> vec4f {
  let r = sampled(p - offset * 0.5);
  let g = sampled(p);
  let b = sampled(p + offset * 0.5);
  return vec4f(r.r, g.g, b.b, max(r.a, max(g.a, b.a)));
}

@fragment fn fs(in: Out) -> @location(0) vec4f {
  let e = effects[in.index];
  return aberration(in.pos.xy, e.head.yz);
}
