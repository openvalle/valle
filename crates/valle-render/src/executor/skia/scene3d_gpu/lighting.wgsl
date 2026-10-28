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
    @location(4) uv: vec2<f32>,
    @location(5) @interpolate(flat) uvDx: vec3<f32>,
    @location(6) @interpolate(flat) uvDy: vec3<f32>,
    @location(7) @interpolate(flat) tangent: vec3<f32>,
    @location(8) @interpolate(flat) bitangent: vec3<f32>,
    @location(9) @interpolate(flat) triangle:u32,
};

fn make_lit_vertex(position: vec2<f32>, invZ: f32, objectId: u32, nodeId: u32, normalOverZ: vec3<f32>, material: u32) -> LitVertex {
    return LitVertex(vec4(position / globals.extent * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.5, 1.0),
        invZ,normalOverZ,material,vec2(objectId,nodeId),vec2(0.0),vec3(0.0),vec3(0.0),vec3(0.0),vec3(0.0),0xffffffffu);
}

// Zero-valued uniform boundary: retain f32 rounding in the normal/GGX terms.
// Contraction and reciprocal-square-root substitution amplify narrow highlights.
fn stable(v: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(v) ^ globals.padding);
}
fn stable4(v:vec4<f32>)->vec4<f32> {
    return bitcast<vec4<f32>>(bitcast<vec4<u32>>(v)^vec4(globals.padding));
}

// Correct the hardware division estimate before discontinuous PCF/DOF decisions.
fn precise_divide(a:f32,b:f32) -> f32 {
    let q=stable(a/b);
    let residual=stable(fma(-q,b,a));
    return stable(fma(residual,stable(1.0/b),q));
}

fn dot3(a: vec3<f32>, b: vec3<f32>) -> f32 {
    return stable(stable(stable(a.x * b.x) + stable(a.y * b.y)) + stable(a.z * b.z));
}

fn unit(v: vec3<f32>) -> vec3<f32> {
    return v * precise_divide(1.0,max(stable(sqrt(dot3(v, v))), 1e-12));
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

fn scattering(f0:vec3<f32>,fab:vec2<f32>) -> mat2x3<f32> {
    let single=f0*fab.x+fab.y;let missing=1.0-fab.x-fab.y;let average=f0+(vec3(1.0)-f0)/21.0;
    return mat2x3(single,single*average/(vec3(1.0)-missing*average)*missing);
}
fn camera_ray(pixel:vec2<u32>) -> vec3<f32> {
    let counts=vec3<u32>(shading[1].xyz);
    let rayStart=HEADER+counts.x*MATERIAL+counts.y*2u+counts.z*3u;
    return shading[rayStart+pixel.x].xyz+shading[rayStart+u32(globals.extent.x)+pixel.y].xyz;
}
fn lit_radiance(in: LitVertex) -> vec4<f32> {
    let counts = vec3<u32>(shading[1].xyz);
    let material = HEADER + in.material * MATERIAL;
    let baseTex=texture_slot(material,0u,in);
    let base = shading[material].rgb*baseTex.rgb;
    let kind = u32(shading[material].a);
    let emission = shading[material + 1u].rgb*texture_slot(material,4u,in).rgb;
    let mr=texture_slot(material,1u,in);
    let metallic = shading[material + 1u].a*mr.b;
    let roughness = max(shading[material + 2u].x*mr.g,0.0525);
    let coverage=shading[material+2u].y*baseTex.a;
    if shading[material+2u].z >= 0.0 && coverage < shading[material+2u].z {discard;}
    let ao=1.0+shading[material+3u].y*(texture_slot(material,3u,in).r-1.0);
    let lightStart = HEADER + counts.x * MATERIAL;
    let hemiStart = lightStart + counts.y * 2u;
    let pixel = vec2<u32>(in.position.xy);
    let ray = camera_ray(pixel);
    var n = unit(select(in.normalOverZ,in.normalOverZ / in.invZ,in.triangle==0xffffffffu));
    if shading[material+6u].x >= 0.0 && dot(in.tangent,in.tangent)>0.0 {
        let texel=texture_slot(material,2u,in);let t=unit(in.tangent-n*dot3(n,in.tangent));
        let c=cross(n,t);let b=c*select(1.0,-1.0,dot3(c,in.bitangent)<0.0);
        n=unit(t*((texel.r*2.0-1.0)*shading[material+3u].x)+b*((texel.g*2.0-1.0)*shading[material+3u].x)+n*(texel.b*2.0-1.0));
    }
    let v = unit(-ray);
    var radiance = base;
    var fab = vec2(0.0);
    if kind == 1u {
        var factor = shading[0].rgb*ao;
        for (var i = 0u; i < counts.y; i++) {
            let at = lightStart + i * 2u;
            factor += shading[at + 1u].rgb * max(dot(n, shading[at].xyz), 0.0)*shadow_visibility(i,camera_position(ray,in.invZ),n);
        }
        for (var i = 0u; i < counts.z; i++) {
            let at = hemiStart + i * 3u;
            let t = dot(n, shading[at].xyz) * 0.5 + 0.5;
            factor += (shading[at + 2u].rgb * (1.0 - t) + shading[at + 1u].rgb * t)*ao;
        }
        factor+=env_diffuse(n)*ao;
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
        radiance = base * (1.0 - metallic) * (1.0 - single - multi) * irradiance*ao;
        if shading[3].z >= 0.0 {
            let reflection=n*(2.0*dot3(n,v))-v;let r4=roughness*roughness*roughness*roughness;
            let specular=env_specular(unit(reflection*(1.0-r4)+n*r4),roughness);let diffuse=env_diffuse(n);
            let specularAo=clamp(pow(clamp(dot3(n,v),0.0,1.0)+ao,exp2(-16.0*roughness-1.0))-1.0+ao,0.0,1.0);
            let metal=scattering(base,fab);let dielectric=scattering(vec3(0.04),fab);
            let s=dielectric[0]*(1.0-metallic)+metal[0]*metallic;let m=dielectric[1]*(1.0-metallic)+metal[1]*metallic;
            radiance+=(specular*s+diffuse*m)*specularAo+base*(1.0-metallic)*(1.0-single-multi)*diffuse*ao;
        }
    }
    radiance += emission;
    if kind == 2u {
        for (var i = 0u; i < counts.y; i++) {
            let at = lightStart + i * 2u;
            radiance += direct(base, metallic, roughness, n, v, shading[at].xyz, fab) * shading[at + 1u].rgb*shadow_visibility(i,camera_position(ray,in.invZ),n);
        }
    }
    var display = radiance * shading[0].a;
    if (u32(shading[1].a)&1u) != 0u { display = aces(radiance, shading[0].a); }
    display = clamp(display, vec3(0.0), vec3(1.0));
    return vec4(display,select(1.0,coverage,shading[material+2u].w!=0.0));
}
fn lit_color(in:LitVertex) -> vec4<f32> {
    let c=lit_radiance(in);
    return vec4(half_even(srgb(c.r)*c.a),half_even(srgb(c.g)*c.a),half_even(srgb(c.b)*c.a),half_even(c.a));
}

fn camera_position(ray:vec3<f32>,inverse:f32) -> vec3<f32> {
    let z=precise_divide(1.0,inverse);let eye=shading[2].xyz;
    return vec3(stable(eye.x+stable(ray.x*z)),stable(eye.y+stable(ray.y*z)),stable(eye.z+stable(ray.z*z)));
}

@vertex fn background_vertex(@builtin(vertex_index) index:u32) -> LitVertex {
    let point=vec2(select(-1.0,3.0,index==1u),select(-1.0,3.0,index==2u));
    return LitVertex(vec4(point,0.5,1.0),1.0,vec3(0.0),0u,vec2(0u),vec2(0.0),
        vec3(0.0),vec3(0.0),vec3(0.0),vec3(0.0),0xffffffffu);
}
@fragment fn background_fragment(in:LitVertex) -> Fragment {
    var radiance=env_specular(unit(camera_ray(vec2<u32>(in.position.xy))),0.0);
    var display=radiance*shading[0].a;
    if (u32(shading[1].a)&1u)!=0u {display=aces(radiance,shading[0].a);}
    display=clamp(display,vec3(0.0),vec3(1.0));
    return Fragment(vec4(half_even(srgb(display.r)),half_even(srgb(display.g)),half_even(srgb(display.b)),1.0),pack_depth(bitcast<f32>(0x7f7fffffu)));
}
