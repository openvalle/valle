// A layer blurred along one axis by a Gaussian into a new target, which the pass replaces: a pass
// along x, then one along y, blur it on both. Past the layer's edge its edge pixels go on, so what
// runs off the canvas blurs as if it went on. Within bounds, it reads only the pixels in them, as
// if mirrored at their edges, and the pixels out of them are its own.

struct Blur {
  // x: the standard deviation, px; yz: the axis.
  blur: vec4f,
  // x, y, width and height, whole px; none with no width.
  bounds: vec4f,
  unused: array<vec4f, 3>,
}

@group(0) @binding(5) var<storage, read> blurs: array<Blur>;
@group(1) @binding(0) var image: texture_2d<f32>;
@group(1) @binding(1) var linear: sampler;

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

// `p` in [low, high): clamped, or mirrored at the edges, each edge pixel taken twice.
fn within(p: vec2<i32>, low: vec2<i32>, high: vec2<i32>, mirrored: bool) -> vec2<i32> {
  if (!mirrored) {
    return clamp(p, low, high - 1);
  }
  let n = max(high - low, vec2<i32>(1));
  let q = ((p - low) % (2 * n) + 2 * n) % (2 * n);
  return low + select(q, 2 * n - 1 - q, q >= n);
}

@fragment fn fs(in: Out) -> @location(0) vec4f {
  let b = blurs[in.index].blur;
  let sigma = b.x;
  let axis = vec2<i32>(b.yz);
  let size = vec2<i32>(textureDimensions(image));
  var low = vec2<i32>(0);
  var high = size;
  let bounds = blurs[in.index].bounds;
  if (bounds.z > 0.0) {
    low = vec2<i32>(bounds.xy);
    high = low + vec2<i32>(bounds.zw);
  }
  let p = vec2<i32>(in.pos.xy);
  let mirrored = bounds.z > 0.0;
  // Pixels out to three standard deviations either side, weighted by the Gaussian, k^(i²) with
  // k = e^(-1 / 2σ²): each weight the last times a ratio that goes up by k² each step.
  let n = i32(ceil(3.0 * sigma));
  let k = exp(-1.0 / (2.0 * sigma * sigma));
  var sum = pixel(p, low, high, size, mirrored);
  var total = 1.0;
  var w = 1.0;
  var ratio = k;
  // Two adjacent taps are one filtered sample: a*A + b*B = (a+b)*mix(A,B,b/(a+b)).
  // Keep the same finite kernel, including its last tap when the radius is odd.
  for (var i = 1; i <= n; i += 2) {
    let a = w * ratio;
    ratio *= k * k;
    var b = 0.0;
    if (i < n) {
      b = a * ratio;
      ratio *= k * k;
    }
    w = b;
    let weight = a + b;
    if (weight == 0.0) { break; }
    let t = b / weight;
    let after = paired(p, axis, i, t, low, high, size, mirrored);
    let before = paired(p, -axis, i, t, low, high, size, mirrored);
    sum += (after + before) * weight;
    total += 2.0 * weight;
  }
  return sum / total;
}

// The layer's pixel at `p`, within the bounds; bounds may run off the canvas.
fn pixel(p: vec2<i32>, low: vec2<i32>, high: vec2<i32>, size: vec2<i32>, mirrored: bool) -> vec4f {
  return textureLoad(image, clamp(within(p, low, high, mirrored), vec2<i32>(0), size - 1), 0);
}

// Fold the two integer taps separately: at a mirrored edge they may coincide or run backwards.
// After folding and canvas clamping they are still adjacent, so filtering gives the same pair.
fn paired(p: vec2<i32>, axis: vec2<i32>, i: i32, t: f32,
    low: vec2<i32>, high: vec2<i32>, size: vec2<i32>, mirrored: bool) -> vec4f {
  var at = vec2f(p) + vec2f(axis) * (f32(i) + t);
  if (mirrored) {
    let a = clamp(within(p + axis * i, low, high, true), vec2<i32>(0), size - 1);
    let b = clamp(within(p + axis * (i + 1), low, high, true), vec2<i32>(0), size - 1);
    at = mix(vec2f(a), vec2f(b), t);
  }
  return textureSampleLevel(image, linear, (at + 0.5) / vec2f(size), 0.0);
}
