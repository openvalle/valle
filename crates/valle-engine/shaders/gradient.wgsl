// Gradients filling rounded rectangles: each pixel's place along the gradient line in its image,
// out from its center to the ending shape, or around its center, the image repeated from its
// tile, and the color there, mixed premultiplied in the gradient's color space between the two
// stops around it, then as a premultiplied working color.

struct Gradient {
  // Where it shows: x, y, width, height, and corner radii as `coverage` takes them.
  rect: vec4f,
  radii: array<vec4f, 2>,
  // The image: x, y, width and height.
  tile: vec4f,
  // From the image's top left: where the gradient line starts, and where it ends; the center,
  // and the ending shape's horizontal and vertical radii; or the center, and the fraction of a
  // turn clockwise from up the colors start at.
  shape: vec4f,
  // x: 1 when the stops repeat; y: the color space, 0 sRGB, 1 linear sRGB, 2 Oklab, 3 Oklch;
  // z: 0 along a line, 1 out to an ellipse, 2 around a turn.
  form: vec4u,
  // The first segment, and how many there are.
  segments: vec4u,
}

// The part of the line between two color stops.
struct Segment {
  // x, y: where it starts and ends along the line, 0 at the line's start and 1 at its end.
  at: vec4f,
  // The colors at its start and end.
  colors: array<vec4f, 2>,
}

@group(0) @binding(6) var<storage, read> gradients: array<Gradient>;
@group(0) @binding(7) var<storage, read> segments: array<Segment>;

struct Out {
  @builtin(position) pos: vec4f,
  @location(0) @interpolate(flat) index: u32,
}

@vertex fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Out {
  let b = gradients[ii].rect;
  var out: Out;
  // A pixel more around the box, for its antialiased edge.
  out.pos = clip(b.xy - 1.0 + corner(vi) * (b.zw + 2.0));
  out.index = ii;
  return out;
}

@fragment fn fs(in: Out) -> @location(0) vec4f {
  let g = gradients[in.index];
  let p = in.pos.xy;
  let cov = coverage(p, g.rect, g.radii);
  let size = g.tile.zw;
  let q = p - g.tile.xy;
  let image = q - size * floor(q / size);
  let v = image - g.shape.xy;
  var t = 0.0;
  switch g.form.z {
    case 0u: {
      let d = g.shape.zw - g.shape.xy;
      t = dot(v, d) / dot(d, d);
    }
    case 1u: {
      t = length(v / g.shape.zw);
    }
    default: {
      t = fract(atan2(v.x, -v.y) / 6.283185307 - g.shape.z);
    }
  }
  let first = g.segments.x;
  let last = first + g.segments.y - 1u;
  if (g.form.x == 1u) {
    let start = segments[first].at.x;
    let period = segments[last].at.y - start;
    t = t - period * floor((t - start) / period);
  }
  var c = segments[last].colors[1];
  for (var i = first; i <= last; i++) {
    let s = segments[i];
    if (t <= s.at.y) {
      let f = clamp((t - s.at.x) / max(s.at.y - s.at.x, 1e-9), 0.0, 1.0);
      c = mix(s.colors[0], s.colors[1], f);
      break;
    }
  }
  return working(c, g.form.y) * cov;
}

// A premultiplied color mixed in `space` as a premultiplied working color: sRGB-encoded,
// clipped to the sRGB gamut, as browsers draw gradients.
fn working(c: vec4f, space: u32) -> vec4f {
  if (space == 0u) {
    return c;
  }
  if (c.a <= 0.0) {
    return vec4f(0.0);
  }
  var v = c.rgb / c.a;
  if (space == 3u) {
    // Oklch: the hue in degrees is not premultiplied.
    let h = radians(c.b);
    v = vec3f(v.x, v.y * cos(h), v.y * sin(h));
  }
  if (space >= 2u) {
    v = linear_from_oklab(v);
  }
  return vec4f(encode(clamp(v, vec3f(0.0), vec3f(1.0))) * c.a, c.a);
}

fn linear_from_oklab(lab: vec3f) -> vec3f {
  let l = lab.x + 0.3963377774 * lab.y + 0.2158037573 * lab.z;
  let m = lab.x - 0.1055613458 * lab.y - 0.0638541728 * lab.z;
  let s = lab.x - 0.0894841775 * lab.y - 1.2914855480 * lab.z;
  let lms = vec3f(l * l * l, m * m * m, s * s * s);
  return vec3f(
    dot(lms, vec3f(4.0767416621, -3.3077115913, 0.2309699292)),
    dot(lms, vec3f(-1.2684380046, 2.6097574011, -0.3413193965)),
    dot(lms, vec3f(-0.0041960863, -0.7034186147, 1.7076147010)),
  );
}

// Linear sRGB channels in [0, 1], encoded.
fn encode(v: vec3f) -> vec3f {
  return select(1.055 * pow(v, vec3f(1.0 / 2.4)) - 0.055, 12.92 * v, v <= vec3f(0.0031308));
}
