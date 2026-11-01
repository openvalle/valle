// Valle's effects on a layer into a new target, which the pass replaces. The layer is read
// between its pixels linearly, and transparent past its edges.
//
// - 0, chromatic aberration: red from half an offset back and blue from half of it on, so red
//   moves on and blue back, and green where it is; the alpha the most of the three's, as each
//   channel already carries its own.
// - 1, lens distortion: in the px of a transform, each point of a frame shows the one as much
//   farther from its center as 1 + k1 r² + k2 r⁴, r its distance over the half diagonal's;
//   nothing shows out of the frame, its edges antialiased.
// - 2, film grain: in the px of a transform, value noise of a pattern a seed picks, cells a size
//   across from an origin, each corner's value from a hash of its cell and the seed, added to the
//   colors' encoded values by up to an amount of how far each is from 0 or 1, whichever is
//   nearer, so it never clips.
// - 3, radial blur: in the px of a transform, a pass along each ray from a center, 1, 2 and 1 of
//   the points a part of the reach before each pixel, at it and after it, the reach an amount
//   but at most half of the way to the center. Passes a part 1, 2, 4… of 2ⁿ − 1 apart weigh a
//   triangle across the reach.

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

fn lens(p: vec2f, t: u32, k: vec2f, frame: vec4f) -> vec4f {
  let q = local(p, t);
  let none = array<vec4f, 2>(vec4f(0.0), vec4f(0.0));
  let inside = coverage(q, steps(p, t), frame, none);
  if (inside <= 0.0) {
    return vec4f(0.0);
  }
  let c = frame.xy + frame.zw * 0.5;
  let d = q - c;
  let r2 = dot(d, d) / (dot(frame.zw, frame.zw) * 0.25);
  let seen = c + d * (1.0 + k.x * r2 + k.y * r2 * r2);
  let h = transforms[t].forward * vec3f(seen, 1.0);
  return sampled(h.xy / h.z) * inside;
}

// PCG's hash of a word.
fn hash(v: u32) -> u32 {
  let s = v * 747796405u + 2891336453u;
  let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
  return (w >> 22u) ^ w;
}

// The noise at a lattice point, in [-1, 1].
fn lattice(seed: u32, p: vec2<i32>) -> f32 {
  let h = hash(seed ^ hash(bitcast<u32>(p.x) ^ hash(bitcast<u32>(p.y))));
  return f32(h >> 8u) / 8388607.5 - 1.0;
}

fn grain(p: vec2f, t: u32, seed: u32, amount: f32, size: f32, origin: vec2f) -> vec4f {
  let c = texel(vec2<i32>(p));
  if (c.a <= 0.0 || amount <= 0.0) {
    return c;
  }
  // A lattice point at each pixel's center when the cells are a pixel.
  let q = (local(p, t) - origin - 0.5) / size;
  let f = floor(q);
  let w = q - f;
  let i = vec2<i32>(f);
  let top = mix(lattice(seed, i), lattice(seed, i + vec2<i32>(1, 0)), w.x);
  let bottom = mix(lattice(seed, i + vec2<i32>(0, 1)), lattice(seed, i + vec2<i32>(1, 1)), w.x);
  let n = mix(top, bottom, w.y);
  let v = c.rgb / c.a;
  let room = max(min(v, 1.0 - v), vec3f(0.0));
  return vec4f((v + n * amount * room) * c.a, c.a);
}

fn radial(p: vec2f, t: u32, c: vec2f, amount: f32, part: f32) -> vec4f {
  let here = texel(vec2<i32>(p));
  let q = local(p, t);
  let d = q - c;
  let distance = length(d);
  let reach = part * min(amount, distance * 0.5);
  if (reach <= 0.0) {
    return here;
  }
  let step = d * (reach / distance);
  let before = transforms[t].forward * vec3f(q - step, 1.0);
  let after = transforms[t].forward * vec3f(q + step, 1.0);
  return (sampled(before.xy / before.z) + 2.0 * here + sampled(after.xy / after.z)) * 0.25;
}

@fragment fn fs(in: Out) -> @location(0) vec4f {
  let e = effects[in.index];
  let kind = u32(e.head.x);
  if (kind == 1u) {
    return lens(in.pos.xy, u32(e.head.y), e.head.zw, e.more[0]);
  }
  if (kind == 2u) {
    let seed = bitcast<u32>(e.head.z);
    return grain(in.pos.xy, u32(e.head.y), seed, e.head.w, e.more[0].x, e.more[0].yz);
  }
  if (kind == 3u) {
    return radial(in.pos.xy, u32(e.head.y), e.head.zw, e.more[0].x, e.more[0].y);
  }
  return aberration(in.pos.xy, e.head.yz);
}
