// Same glTF GGX, DFG lookup and ACES fit as valle-motion's CPU reference.
// BRDF/ACES attribution: valle-motion/src/scene3d/raster/assets/THREE-LICENSE.txt.
@group(0) @binding(1) var<storage, read> shading: array<vec4<f32>>;
@group(0) @binding(2) var dfg: texture_2d<f32>;

struct LitVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) invZ: f32,
    @location(1) normalOverZ: vec3<f32>,
    @location(2) @interpolate(flat) material: u32,
    @location(3) @interpolate(flat) ids: vec2<u32>,
};

@vertex fn lit_vertex(@location(0) position: vec2<f32>, @location(1) invZ: f32,
    @location(3) objectId: u32, @location(4) nodeId: u32,
    @location(5) normalOverZ: vec3<f32>, @location(6) material: u32) -> LitVertex {
    var out: LitVertex;
    out.position = vec4(position / globals.extent * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.5, 1.0);
    out.invZ = invZ;
    out.normalOverZ = normalOverZ;
    out.material = material;
    out.ids = vec2(objectId, nodeId);
    return out;
}

// Zero-valued uniform boundary: retain f32 rounding in the normal/GGX terms.
// Contraction and reciprocal-square-root substitution amplify narrow highlights.
fn stable(v: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(v) ^ globals.padding);
}

fn dot3(a: vec3<f32>, b: vec3<f32>) -> f32 {
    return stable(stable(stable(a.x * b.x) + stable(a.y * b.y)) + stable(a.z * b.z));
}

fn unit(v: vec3<f32>) -> vec3<f32> {
    return v * stable(1.0 / max(stable(sqrt(dot3(v, v))), 1e-12));
}

fn dfg_at(roughness: f32, nv: f32) -> vec2<f32> {
    let p = clamp(vec2(roughness, nv), vec2(0.0), vec2(1.0)) * 16.0 - 0.5;
    let i = vec2<i32>(floor(p));
    let t = p - floor(p);
    let a = textureLoad(dfg, clamp(i, vec2(0), vec2(15)), 0).rg;
    let b = textureLoad(dfg, clamp(i + vec2(1, 0), vec2(0), vec2(15)), 0).rg;
    let c = textureLoad(dfg, clamp(i + vec2(0, 1), vec2(0), vec2(15)), 0).rg;
    let d = textureLoad(dfg, clamp(i + vec2(1, 1), vec2(0), vec2(15)), 0).rg;
    return (a * (1.0 - t.x) + b * t.x) * (1.0 - t.y)
        + (c * (1.0 - t.x) + d * t.x) * t.y;
}

const PI: f32 = 3.141592653589793;

fn direct(base: vec3<f32>, metallic: f32, roughness: f32,
    n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, fab: vec2<f32>) -> vec3<f32> {
    let nv = max(dot3(n, v), 0.0);
    let nl = max(dot3(n, l), 0.0);
    if nl == 0.0 || nv == 0.0 { return vec3(0.0); }
    let h = unit(v + l);
    let nh = max(dot3(n, h), 0.0);
    let vh = clamp(dot3(v, h), 0.0, 1.0);
    let alpha = stable(roughness * roughness);
    let a2 = stable(alpha * alpha);
    let denom = stable(stable(stable(nh * nh) * stable(a2 - 1.0)) + 1.0);
    let distribution = a2 / stable(stable(PI * denom) * denom);
    let visibility = 0.5 / max(nl * sqrt(nv * nv * (1.0 - a2) + a2)
        + nv * sqrt(nl * nl * (1.0 - a2) + a2), 1e-8);
    let f0 = vec3(0.04) * (1.0 - metallic) + base * metallic;
    let power = pow(1.0 - vh, 5.0);
    let dielectric = 0.04 + 0.96 * power;
    let fresnel = f0 + (vec3(1.0) - f0) * power;
    let compensation = vec3(1.0) + f0 * (1.0 / max(fab.x + fab.y, 1e-6) - 1.0);
    return nl * (base * (1.0 - metallic) * (1.0 - dielectric) / PI
        + fresnel * distribution * visibility * compensation);
}

fn aces(color: vec3<f32>, exposure: f32) -> vec3<f32> {
    let c = color * exposure / 0.6;
    let v = vec3(dot(vec3(0.59719, 0.35458, 0.04823), c),
        dot(vec3(0.07600, 0.90834, 0.01566), c), dot(vec3(0.02840, 0.13383, 0.83777), c));
    let fitted = (v * (v + 0.0245786) - 0.000090537)
        / (v * (0.983729 * v + 0.432951) + 0.238081);
    return clamp(vec3(dot(vec3(1.60475, -0.53108, -0.07367), fitted),
        dot(vec3(-0.10208, 1.10813, -0.00605), fitted),
        dot(vec3(-0.00327, -0.07276, 1.07602), fitted)), vec3(0.0), vec3(1.0));
}

fn srgb(v: f32) -> f32 {
    if v <= 0.0031308 { return v * 12.92; }
    return 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}

// Match the reference's explicit round-to-nearest-even F16 boundary before the
// render attachment conversion (whose rounding is backend-dependent).
fn half_even(v: f32) -> f32 {
    let bits = bitcast<u32>(clamp(v, 0.0, 1.0));
    let exponent = i32((bits >> 23u) & 255u) - 112;
    var mantissa = bits & 0x7fffffu;
    var shift = 13u;
    if exponent < -10 { return 0.0; }
    if exponent <= 0 {
        mantissa |= 0x800000u;
        shift = u32(14 - exponent);
    }
    var rounded = mantissa >> shift;
    let remainder = mantissa & ((1u << shift) - 1u);
    let halfway = 1u << (shift - 1u);
    if remainder > halfway || (remainder == halfway && (rounded & 1u) != 0u) { rounded++; }
    var halfBits = rounded;
    if exponent > 0 { halfBits = (u32(exponent) << 10u) + rounded; }
    return unpack2x16float(halfBits).x;
}

fn lit_color(in: LitVertex) -> vec4<f32> {
    let counts = vec3<u32>(shading[1].xyz);
    let material = 2u + in.material * 3u;
    let base = shading[material].rgb;
    let kind = u32(shading[material].a);
    let emission = shading[material + 1u].rgb;
    let metallic = shading[material + 1u].a;
    let roughness = shading[material + 2u].x;
    let lightStart = 2u + counts.x * 3u;
    let hemiStart = lightStart + counts.y * 2u;
    let rayStart = hemiStart + counts.z * 3u;
    let pixel = vec2<u32>(in.position.xy);
    let ray = shading[rayStart + pixel.x].xyz
        + shading[rayStart + u32(globals.extent.x) + pixel.y].xyz;
    let n = unit(in.normalOverZ / in.invZ);
    let v = unit(-ray);
    var radiance = base;
    var fab = vec2(0.0);
    if kind == 1u {
        var factor = shading[0].rgb;
        for (var i = 0u; i < counts.y; i++) {
            let at = lightStart + i * 2u;
            factor += shading[at + 1u].rgb * max(dot(n, shading[at].xyz), 0.0);
        }
        for (var i = 0u; i < counts.z; i++) {
            let at = hemiStart + i * 3u;
            let t = dot(n, shading[at].xyz) * 0.5 + 0.5;
            factor += shading[at + 2u].rgb * (1.0 - t) + shading[at + 1u].rgb * t;
        }
        radiance = base * factor;
    }
    if kind == 2u {
        fab = dfg_at(roughness, clamp(dot(n, v), 0.0, 1.0));
        let single = 0.04 * fab.x + fab.y;
        let missing = 1.0 - fab.x - fab.y;
        let average = 0.04 + 0.96 / 21.0;
        let multi = single * average / (1.0 - missing * average) * missing;
        var irradiance = shading[0].rgb / PI;
        for (var i = 0u; i < counts.z; i++) {
            let at = hemiStart + i * 3u;
            let t = dot(n, shading[at].xyz) * 0.5 + 0.5;
            irradiance += (shading[at + 2u].rgb * (1.0 - t) + shading[at + 1u].rgb * t) / PI;
        }
        radiance = base * (1.0 - metallic) * (1.0 - single - multi) * irradiance;
    }
    radiance += emission;
    if kind == 2u {
        for (var i = 0u; i < counts.y; i++) {
            let at = lightStart + i * 2u;
            radiance += direct(base, metallic, roughness, n, v, shading[at].xyz, fab) * shading[at + 1u].rgb;
        }
    }
    var display = radiance * shading[0].a;
    if shading[1].a != 0.0 { display = aces(radiance, shading[0].a); }
    display = clamp(display, vec3(0.0), vec3(1.0));
    return vec4(half_even(srgb(display.r)), half_even(srgb(display.g)), half_even(srgb(display.b)), 1.0);
}

@fragment fn lit_fragment(in: LitVertex) -> Fragment {
    var out: Fragment;
    out.color = lit_color(in);
    out.depth = globals.depthScale / in.invZ;
    return out;
}
