// A layer blended with what is under it, as CSS's mix-blend-mode has it (Compositing and Blending
// Level 1), at an opacity: composited over a copy of its target, with its colors where both show
// mixed with those under them by the mode, unpremultiplied. The pass writes the result in place
// of what was there.

struct Blend {
  // x: the opacity; y: the mode, as `mixed` numbers them.
  blend: vec4f,
  unused: array<vec4f, 4>,
}

@group(0) @binding(5) var<storage, read> blends: array<Blend>;
@group(1) @binding(0) var image: texture_2d<f32>;
@group(2) @binding(0) var under: texture_2d<f32>;

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

@fragment fn fs(in: Out) -> @location(0) vec4f {
  let blend = blends[in.index].blend;
  let p = vec2<i32>(in.pos.xy);
  let src = textureLoad(image, p, 0) * blend.x;
  let dst = textureLoad(under, p, 0);
  let mode = u32(blend.y);
  // plus-lighter: the sum, at most 1.
  if (mode == 16u) {
    return min(src + dst, vec4f(1.0));
  }
  if (src.a <= 0.0 || dst.a <= 0.0) {
    return src + dst * (1.0 - src.a);
  }
  let s = clamp(src.rgb / src.a, vec3f(0.0), vec3f(1.0));
  let b = clamp(dst.rgb / dst.a, vec3f(0.0), vec3f(1.0));
  let rgb = src.rgb * (1.0 - dst.a) + dst.rgb * (1.0 - src.a) + src.a * dst.a * mixed(mode, b, s);
  return vec4f(rgb, src.a + dst.a - src.a * dst.a);
}
