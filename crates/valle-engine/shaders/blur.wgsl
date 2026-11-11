// A layer blurred along one axis by a Gaussian into a new target, which the pass replaces: a pass
// along x, then one along y, blur it on both. Past the layer's edge its edge pixels go on, so what
// runs off the canvas blurs as if it went on. Within bounds, it reads only the pixels in them, as
// if mirrored at their edges, and the pixels out of them are its own.
//
// A large blur goes through a smaller copy of the layer, at the top left of its targets: a pass
// makes the copy, each of its px the mean of a block of the layer's, the two passes blur it, and
// a pass draws it back, between its px linearly. Past the copy's edges its edge px go on; within
// bounds its blocks divide them evenly, and it is mirrored at its own edges.

struct Blur {
  // x: the standard deviation, px; yz: the axis; w: 1 for the pass that makes a smaller copy, 2
  // for the one that draws it back.
  blur: vec4f,
  // x, y, width and height, whole px; none with no width.
  bounds: vec4f,
  // A smaller copy. xy: where its first px starts on the layer, px; zw: how many of the layer's
  // px each of its px is across and down, 0 for no copy.
  copy: vec4f,
  // xy: the copy's width and height, px.
  extent: vec4f,
  unused: vec4f,
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

// A px of a smaller copy: the mean of its block of the layer, each px of the layer as much as
// is in the block; the layer's edge px go on past its edges.
fn reduced(p: vec2<i32>, origin: vec2f, block: vec2f) -> vec4f {
  let size = vec2<i32>(textureDimensions(image));
  let start = origin + vec2f(p) * block;
  let end = start + block;
  let first = vec2<i32>(floor(start));
  let last = vec2<i32>(ceil(end)) - 1;
  var sum = vec4f(0.0);
  for (var y = first.y; y <= last.y; y++) {
    let wy = max(min(f32(y) + 1.0, end.y) - max(f32(y), start.y), 0.0);
    for (var x = first.x; x <= last.x; x++) {
      let wx = max(min(f32(x) + 1.0, end.x) - max(f32(x), start.x), 0.0);
      sum += textureLoad(image, clamp(vec2<i32>(x, y), vec2<i32>(0), size - 1), 0) * (wx * wy);
    }
  }
  return sum / (block.x * block.y);
}

// A smaller copy at `at`, a px center of the target: between the centers of its px linearly.
fn enlarged(at: vec2f, origin: vec2f, block: vec2f, extent: vec2<i32>) -> vec4f {
  let c = (at - origin) / block - 0.5;
  let f = floor(c);
  let t = c - f;
  let p = vec2<i32>(f);
  let last = extent - 1;
  let top = mix(
    textureLoad(image, clamp(p, vec2<i32>(0), last), 0),
    textureLoad(image, clamp(p + vec2<i32>(1, 0), vec2<i32>(0), last), 0),
    t.x,
  );
  let bottom = mix(
    textureLoad(image, clamp(p + vec2<i32>(0, 1), vec2<i32>(0), last), 0),
    textureLoad(image, clamp(p + vec2<i32>(1, 1), vec2<i32>(0), last), 0),
    t.x,
  );
  return mix(top, bottom, t.y);
}

@fragment fn fs(in: Out) -> @location(0) vec4f {
  let b = blurs[in.index].blur;
  let copy = blurs[in.index].copy;
  let extent = vec2<i32>(blurs[in.index].extent.xy);
  if (b.w == 1.0) {
    return reduced(vec2<i32>(in.pos.xy), copy.xy, copy.zw);
  }
  if (b.w == 2.0) {
    return enlarged(in.pos.xy, copy.xy, copy.zw, extent);
  }
  let sigma = b.x;
  let axis = vec2<i32>(b.yz);
  let size = vec2<i32>(textureDimensions(image));
  var low = vec2<i32>(0);
  var high = size;
  if (copy.z > 0.0) {
    high = extent;
  }
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
// A pair past the edge of a smaller copy is its edge px, as the sampler gives past the layer's.
fn paired(p: vec2<i32>, axis: vec2<i32>, i: i32, t: f32,
    low: vec2<i32>, high: vec2<i32>, size: vec2<i32>, mirrored: bool) -> vec4f {
  var at = vec2f(p) + vec2f(axis) * (f32(i) + t);
  if (mirrored) {
    let a = clamp(within(p + axis * i, low, high, true), vec2<i32>(0), size - 1);
    let b = clamp(within(p + axis * (i + 1), low, high, true), vec2<i32>(0), size - 1);
    at = mix(vec2f(a), vec2f(b), t);
  } else if (any(high < size)) {
    at = clamp(at, vec2f(low), vec2f(high - 1));
  }
  return textureSampleLevel(image, linear, (at + 0.5) / vec2f(size), 0.0);
}
