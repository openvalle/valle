// The engine's part of a shader authors write: the quad of its box, grown by its outset and an
// antialiased edge, in the px of its transform, whose fragments the shader colors; and the
// images it reads, taken out of their layers in its box's px or copied from the scene's, their
// bottom row first and with straight alpha, as WebGL reads a texture.

struct Host {
  // The box's width and height in px, its outset, and its transform's index; the size of the
  // image taken in pixels, and the box's top left in px; and whether the image is the scene's.
  h: array<vec4f, 4>,
}

@group(1) @binding(0) var<uniform> host: Host;
@group(2) @binding(0) var image: texture_2d<f32>;

struct Quad {
  @builtin(position) pos: vec4f,
  @location(0) local: vec2f,
}

@vertex fn vs(@builtin(vertex_index) vi: u32) -> Quad {
  let size = host.h[0].xy;
  let outset = host.h[0].z;
  let t = u32(host.h[0].w);
  let origin = host.h[1].zw;
  let c = corner(vi);
  let p = grown(origin - outset + c * (size + 2.0 * outset), c * 2.0 - 1.0, t);
  var out: Quad;
  out.pos = place(p, t);
  out.local = p - origin;
  return out;
}

@vertex fn vx(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4f {
  return whole(vi);
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

// A pixel of an image the shader reads: of a layer, where the box's px it is at is on it; or of
// an image of the scene, which is as big (h[2].x is 1), flipped.
@fragment fn fx(@builtin(position) pos: vec4f) -> @location(0) vec4f {
  var c: vec4f;
  if (host.h[2].x > 0.5) {
    c = textureLoad(image, vec2<i32>(i32(pos.x), i32(host.h[1].y - pos.y)), 0);
  } else {
    let size = host.h[0].xy;
    let t = u32(host.h[0].w);
    let uv = pos.xy / host.h[1].xy;
    let local = vec2f(uv.x, 1.0 - uv.y) * size + host.h[1].zw;
    let h = transforms[t].forward * vec3f(local, 1.0);
    if (h.z <= 0.0) {
      return vec4f(0.0);
    }
    c = sampled(h.xy / h.z);
  }
  if (c.a <= 0.0) {
    return vec4f(0.0);
  }
  return vec4f(c.rgb / c.a, c.a);
}
