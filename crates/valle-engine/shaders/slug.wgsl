// Glyphs by Slug coverage (see slug.rs): each instance an outline in font units, placed at an
// origin in px and scaled, in a premultiplied working color.

struct Glyph {
  // The origin, px, on the baseline; px per font unit.
  place: vec4f,
  // The outline's bounds in font units, y up: min x, min y, max x, max y.
  bounds: vec4f,
  // Vertical and horizontal bands per font unit.
  bands: vec4f,
  // The outline's first curve texel and first list word; how many horizontal and vertical
  // bands it has.
  at: vec4<u32>,
  color: vec4f,
}

@group(0) @binding(2) var<storage, read> glyphs: array<Glyph>;
@group(0) @binding(3) var<storage, read> curves: array<vec4f>;
@group(0) @binding(4) var<storage, read> lists: array<u32>;

struct Out {
  @builtin(position) pos: vec4f,
  // The pixel in font units, y up.
  @location(0) em: vec2f,
  @location(1) @interpolate(flat) index: u32,
}

@vertex fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Out {
  let g = glyphs[ii];
  let s = g.place.z;
  // The bounds in px, a pixel more around them for the antialiased edge.
  let lo = g.place.xy + vec2f(g.bounds.x, -g.bounds.w) * s - 1.0;
  let hi = g.place.xy + vec2f(g.bounds.z, -g.bounds.y) * s + 1.0;
  let p = mix(lo, hi, corner(vi));
  var out: Out;
  out.pos = clip(p);
  out.em = vec2f(p.x - g.place.x, g.place.y - p.y) / s;
  out.index = ii;
  return out;
}

// Which crossings of y = 0 count, from the signs of a curve's three y: bit 0 for the one where
// y falls, bit 8 for the one where it rises.
fn crossings(y: vec3f) -> u32 {
  let i = select(0u, 1u, y.x < 0.0) | select(0u, 2u, y.y < 0.0) | select(0u, 4u, y.z < 0.0);
  return (0x2E74u >> i) & 0x0101u;
}

// The x where the curve falls through y = 0, and where it rises through it.
fn roots(p1: vec2f, p2: vec2f, p3: vec2f) -> vec2f {
  let a = p1 - 2.0 * p2 + p3;
  let b = p1 - p2;
  let d = sqrt(max(b.y * b.y - a.y * p1.y, 0.0));
  var t = vec2f(b.y - d, b.y + d) / a.y;
  // Nearly a line: -2b t + c = 0.
  if (abs(a.y) < 1.0 / 65536.0) {
    t = vec2f(p1.y * 0.5 / b.y);
  }
  return (a.x * t - 2.0 * b.x) * t + p1.x;
}

// A ray from the pixel at `em` to +x through the curves the band at `head` lists: the winding it
// crosses, each crossing counted by how much of the pixel lies before it, and the weight of the
// nearest crossing. With `swap` the axes are swapped, which makes it the ray to +y.
fn ray(g: Glyph, em: vec2f, ppu: f32, head: u32, swap: bool) -> vec2f {
  let count = lists[g.at.y + head];
  let start = g.at.y + lists[g.at.y + head + 1u];
  var cov = 0.0;
  var weight = 0.0;
  for (var k = 0u; k < count; k++) {
    let c = g.at.x + lists[start + k];
    let a = curves[c];
    let b = curves[c + 1u];
    var p1 = a.xy - em;
    var p2 = a.zw - em;
    var p3 = b.xy - em;
    if (swap) {
      p1 = p1.yx;
      p2 = p2.yx;
      p3 = p3.yx;
    }
    // Curves come by their far end first, so the rest end before the pixel.
    if (max(max(p1.x, p2.x), p3.x) * ppu < -0.5) {
      break;
    }
    let code = crossings(vec3f(p1.y, p2.y, p3.y));
    if (code != 0u) {
      let r = roots(p1, p2, p3) * ppu;
      if ((code & 1u) != 0u) {
        cov += saturate(r.x + 0.5);
        weight = max(weight, saturate(1.0 - abs(r.x) * 2.0));
      }
      if (code > 1u) {
        cov -= saturate(r.y + 0.5);
        weight = max(weight, saturate(1.0 - abs(r.y) * 2.0));
      }
    }
  }
  return vec2f(cov, weight);
}

@fragment fn fs(in: Out) -> @location(0) vec4f {
  let g = glyphs[in.index];
  let ppu = g.place.z;
  let last = vec2f(f32(g.at.w), f32(g.at.z)) - 1.0;
  let band = clamp(floor((in.em - g.bounds.xy) * g.bands.xy), vec2f(0.0), last);
  let h = ray(g, in.em, ppu, 2u * u32(band.y), false);
  let v = ray(g, in.em, ppu, 2u * (g.at.z + u32(band.x)), true);
  // Swapping the axes reverses the winding.
  let cov = vec2f(h.x, -v.x);
  let w = vec2f(h.y, v.y);
  let mixed = abs(cov.x * w.x + cov.y * w.y) / max(w.x + w.y, 1.0 / 65536.0);
  let c = saturate(max(mixed, min(abs(cov.x), abs(cov.y))));
  return g.color * c;
}
