// Metal's fixed-function subpixel precision can drop a CPU-covered silhouette pixel.
// Rasterize integer bounding quads, then enforce the shared 1/256-pixel fill rule.
// The existing vertex buffer doubles as readonly storage; no per-pixel CPU work/readback.
@group(0) @binding(6) var<storage,read> rasterVertices:array<u32>;
fn vertex_float(at:u32) -> f32 {return bitcast<f32>(rasterVertices[at]);}
fn vertex_vec2(at:u32) -> vec2<f32> {return vec2(vertex_float(at),vertex_float(at+1u));}
fn vertex_vec3(at:u32) -> vec3<f32> {return vec3(vertex_float(at),vertex_float(at+1u),vertex_float(at+2u));}

// Signed 64-bit edge products using baseline u32 operations (no optional GPU int64).
fn product64(a:i32,b:i32) -> vec2<u32> {
    let x=u32(abs(a));let y=u32(abs(b));
    let low=(x&65535u)*(y&65535u);
    let first=(x>>16u)*(y&65535u)+(low>>16u);
    let second=(x&65535u)*(y>>16u)+(first&65535u);
    var out=vec2((second<<16u)|(low&65535u),(x>>16u)*(y>>16u)+(first>>16u)+(second>>16u));
    if (a<0)!=(b<0) {out=vec2(0u-out.x,~out.y+select(0u,1u,out.x==0u));}
    return out;
}
fn edge64(a:vec2<i32>,b:vec2<i32>,p:vec2<i32>) -> vec2<u32> {
    let x=product64(b.x-a.x,p.y-a.y);let y=product64(b.y-a.y,p.x-a.x);
    return vec2(x.x-y.x,x.y-y.y-select(0u,1u,x.x<y.x));
}
fn positive_float64(v:vec2<u32>) -> f32 {
    if v.y==0u {return f32(v.x);}
    let shift=40u-countLeadingZeros(v.y);
    var mantissa=(v.y<<(32u-shift))|(v.x>>shift);
    let remainder=v.x&((1u<<shift)-1u);let half=1u<<(shift-1u);
    if remainder>half || (remainder==half && (mantissa&1u)!=0u) {mantissa++;}
    return ldexp(f32(mantissa),i32(shift));
}
fn edge_inside(edge:vec2<u32>,a:vec2<i32>,b:vec2<i32>) -> bool {
    let d=b-a;
    return i32(edge.y)>=0 && (any(edge!=vec2(0u)) || d.y>0 || (d.y==0 && d.x<0));
}
fn raster_bary(base:u32,stride:u32,pixel:vec2<f32>) -> vec3<f32> {
    let a=vec2<i32>(vertex_vec2(base)*256.0);
    let b=vec2<i32>(vertex_vec2(base+stride)*256.0);
    let c=vec2<i32>(vertex_vec2(base+stride*2u)*256.0);
    let p=vec2<i32>(floor(pixel))*256+128;
    let x=edge64(b,c,p);let y=edge64(c,a,p);let z=edge64(a,b,p);
    if !edge_inside(x,b,c) || !edge_inside(y,c,a) || !edge_inside(z,a,b) {discard;}
    let inverse=vertex_float(base+select(6u,7u,stride==9u));
    return vec3(positive_float64(x),positive_float64(y),positive_float64(z))*inverse;
}
fn bbox_position(index:u32,stride:u32,extent:vec2<f32>) -> vec4<f32> {
    let base=(index/6u)*3u*stride;
    let a=vertex_vec2(base);let b=vertex_vec2(base+stride);let c=vertex_vec2(base+2u*stride);
    let lo=floor(min(a,min(b,c)));let hi=ceil(max(a,max(b,c)));
    let corner=array(vec2(0.0,0.0),vec2(1.0,0.0),vec2(0.0,1.0),vec2(0.0,1.0),vec2(1.0,0.0),vec2(1.0,1.0));
    let p=mix(lo,hi,corner[index%6u]);
    return vec4(p/extent*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.5,1.0);
}
fn raster_inverse_z(base:u32,stride:u32,bary:vec3<f32>) -> f32 {
    return stable(stable(stable(bary.x*vertex_float(base+2u))+stable(bary.y*vertex_float(base+stride+2u)))+stable(bary.z*vertex_float(base+stride*2u+2u)));
}
struct RasterVertex { @builtin(position) position:vec4<f32>,@location(0) @interpolate(flat) triangle:u32 };
@vertex fn raster_vertex(@builtin(vertex_index) index:u32) -> RasterVertex {
    return RasterVertex(bbox_position(index,9u,globals.extent),index/6u);
}
@fragment fn raster_fragment(in:RasterVertex) -> Fragment {
    let base=in.triangle*27u;let bary=raster_bary(base,9u,in.position.xy);
    let inverse=raster_inverse_z(base,9u,bary);
    let depth=precise_divide(1.0,inverse);
    return Fragment(vec4(vertex_vec3(base+3u),vertex_float(base+6u)),pack_depth(depth),globals.depthScale*depth);
}
@vertex fn raster_lit_vertex(@builtin(vertex_index) index:u32) -> LitVertex {
    var out=make_lit_vertex(vec2(0.0),1.0,0u,0u,vec3(0.0),0u);
    out.position=bbox_position(index,u32(shading[4].x),globals.extent);out.triangle=index/6u;
    return out;
}
fn raster_lit_sample(in:LitVertex) -> LitVertex {
    let stride=u32(shading[4].x);let base=in.triangle*3u*stride;
    let bary=raster_bary(base,stride,in.position.xy);let inverse=raster_inverse_z(base,stride,bary);
    var out=in;out.invZ=inverse;out.material=rasterVertices[base+12u];out.ids=vec2(rasterVertices[base+7u],rasterVertices[base+8u]);
    let weights=vec3(precise_divide(stable(bary.x*vertex_float(base+2u)),inverse),precise_divide(stable(bary.y*vertex_float(base+stride+2u)),inverse),precise_divide(stable(bary.z*vertex_float(base+stride*2u+2u)),inverse));
    let a=vertex_vec3(base+3u);let b=vertex_vec3(base+stride+3u);let c=vertex_vec3(base+stride*2u+3u);
    out.normalOverZ=vec3(
        stable(stable(stable(a.x*weights.x)+stable(b.x*weights.y))+stable(c.x*weights.z)),
        stable(stable(stable(a.y*weights.x)+stable(b.y*weights.y))+stable(c.y*weights.z)),
        stable(stable(stable(a.z*weights.x)+stable(b.z*weights.y))+stable(c.z*weights.z)));
    if stride>13u {
        // Match CPU's corrected weights before interpolation. Interpolating UV/Z and
        // dividing afterward moves alpha-mask boundaries by a pixel at delivery sizes.
        let ua=vertex_vec2(base+13u);let ub=vertex_vec2(base+stride+13u);let uc=vertex_vec2(base+stride*2u+13u);
        out.uv=vec2(
            stable(stable(stable(ua.x*weights.x)+stable(ub.x*weights.y))+stable(uc.x*weights.z)),
            stable(stable(stable(ua.y*weights.x)+stable(ub.y*weights.y))+stable(uc.y*weights.z)));
        out.uvDx=vertex_vec3(base+15u);out.uvDy=vertex_vec3(base+18u);out.tangent=vertex_vec3(base+21u);out.bitangent=vertex_vec3(base+24u);
    }
    return out;
}
@fragment fn raster_lit_fragment(in:LitVertex) -> Fragment {
    let sample=raster_lit_sample(in);
    if shading[HEADER+sample.material*MATERIAL+2u].w!=0.0 {discard;}
    let depth=precise_divide(1.0,sample.invZ);
    return Fragment(lit_color(sample),pack_depth(depth),globals.depthScale*depth);
}
