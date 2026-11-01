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

fn screened(b: vec3f, s: vec3f) -> vec3f {
  return b + s - b * s;
}

fn hard_light(b: vec3f, s: vec3f) -> vec3f {
  return select(screened(b, 2.0 * s - 1.0), b * 2.0 * s, s <= vec3f(0.5));
}

fn soft_light(b: vec3f, s: vec3f) -> vec3f {
  let d = select(sqrt(b), ((16.0 * b - 12.0) * b + 4.0) * b, b <= vec3f(0.25));
  return select(b + (2.0 * s - 1.0) * (d - b), b - (1.0 - 2.0 * s) * b * (1.0 - b), s <= vec3f(0.5));
}

fn luminosity(c: vec3f) -> f32 {
  return dot(c, vec3f(0.3, 0.59, 0.11));
}

// `c` moved to the luminosity `l`, its hue kept, and brought back into [0, 1] toward its gray.
fn with_luminosity(c: vec3f, l: f32) -> vec3f {
  let o = c + (l - luminosity(c));
  let m = luminosity(o);
  let low = min(min(o.r, o.g), o.b);
  let high = max(max(o.r, o.g), o.b);
  var out = o;
  if (low < 0.0) {
    out = m + (out - m) * m / (m - low);
  }
  if (high > 1.0) {
    out = m + (out - m) * (1.0 - m) / (high - m);
  }
  return out;
}

fn saturation(c: vec3f) -> f32 {
  return max(max(c.r, c.g), c.b) - min(min(c.r, c.g), c.b);
}

// `c` with the saturation `s`: its largest component `s`, its smallest 0, the middle in between
// as it was.
fn with_saturation(c: vec3f, s: f32) -> vec3f {
  let low = min(min(c.r, c.g), c.b);
  let high = max(max(c.r, c.g), c.b);
  if (high > low) {
    return (c - low) * s / (high - low);
  }
  return vec3f(0.0);
}

// A color `s` over `b`, mixed by mode `mode`: multiply, screen, overlay, darken, lighten,
// color-dodge, color-burn, hard-light, soft-light, difference, exclusion, hue, saturation, color
// and luminosity are 1 to 15; normal, 0, is `s`.
fn mixed(mode: u32, b: vec3f, s: vec3f) -> vec3f {
  let zero = vec3f(0.0);
  let one = vec3f(1.0);
  switch mode {
    case 1u: { return b * s; }
    case 2u: { return screened(b, s); }
    case 3u: { return hard_light(s, b); }
    case 4u: { return min(b, s); }
    case 5u: { return max(b, s); }
    case 6u: { return select(select(min(one, b / (1.0 - s)), one, s >= one), zero, b <= zero); }
    case 7u: { return select(select(1.0 - min(one, (1.0 - b) / s), zero, s <= zero), one, b >= one); }
    case 8u: { return hard_light(b, s); }
    case 9u: { return soft_light(b, s); }
    case 10u: { return abs(b - s); }
    case 11u: { return b + s - 2.0 * b * s; }
    case 12u: { return with_luminosity(with_saturation(s, saturation(b)), luminosity(b)); }
    case 13u: { return with_luminosity(with_saturation(b, saturation(s)), luminosity(b)); }
    case 14u: { return with_luminosity(s, luminosity(b)); }
    case 15u: { return with_luminosity(b, luminosity(s)); }
    default: { return s; }
  }
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
