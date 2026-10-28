// Native and WebGL share standard triangle coverage. The vertex buffer doubles as
// readonly storage for material interpolation; no bounding-quad overdraw or integer edge emulation.
@group(0) @binding(6) var<storage,read> rasterVertices:array<u32>;
fn vertex_float(at:u32) -> f32 {return bitcast<f32>(rasterVertices[at]);}
fn vertex_vec2(at:u32) -> vec2<f32> {return vec2(vertex_float(at),vertex_float(at+1u));}
fn vertex_vec3(at:u32) -> vec3<f32> {return vec3(vertex_float(at),vertex_float(at+1u),vertex_float(at+2u));}

// Coverage belongs to the GPU triangle rasterizer. Floating barycentrics only interpolate
// already-covered fragments; they never decide whether an edge pixel exists.
fn raster_edge(a:vec2<f32>,b:vec2<f32>,p:vec2<f32>) -> f32 {
    let ab=b-a;let ap=p-a;
    return ab.x*ap.y-ab.y*ap.x;
}
fn raster_bary(base:u32,stride:u32,pixel:vec2<f32>) -> vec3<f32> {
    let a=vertex_vec2(base);let b=vertex_vec2(base+stride);let c=vertex_vec2(base+stride*2u);
    let p=floor(pixel)+vec2(0.5);
    // The packet stores the reciprocal area in its original 1/256-pixel units.
    let inverse=vertex_float(base+select(6u,7u,stride==9u))*65536.0;
    return vec3(raster_edge(b,c,p),raster_edge(c,a,p),raster_edge(a,b,p))*inverse;
}
fn triangle_position(index:u32,stride:u32,extent:vec2<f32>,depth:f32) -> vec4<f32> {
    let p=vertex_vec2(index*stride);
    return vec4(p/extent*vec2(2.0,-2.0)+vec2(-1.0,1.0),depth,1.0);
}
fn raster_inverse_z(base:u32,stride:u32,bary:vec3<f32>) -> f32 {
    return stable(stable(stable(bary.x*vertex_float(base+2u))+stable(bary.y*vertex_float(base+stride+2u)))+stable(bary.z*vertex_float(base+stride*2u+2u)));
}
struct RasterVertex { @builtin(position) position:vec4<f32>,@location(0) @interpolate(flat) triangle:u32 };
@vertex fn raster_vertex(@builtin(vertex_index) index:u32) -> RasterVertex {
    return RasterVertex(triangle_position(index,9u,globals.extent,vertex_float(index*9u+2u)*globals.depthScale),index/3u);
}
@fragment fn raster_fragment(in:RasterVertex) -> Fragment {
    let base=in.triangle*27u;let bary=raster_bary(base,9u,in.position.xy);
    let inverse=raster_inverse_z(base,9u,bary);
    let depth=precise_divide(1.0,inverse);
    return Fragment(vec4(vertex_vec3(base+3u),vertex_float(base+6u)),pack_depth(depth));
}
@vertex fn raster_lit_vertex(@builtin(vertex_index) index:u32) -> LitVertex {
    return LitVertex(triangle_position(index,u32(shading[4].x),globals.extent,vertex_float(index*u32(shading[4].x)+2u)*globals.depthScale),1.0,vec3(0.0),0u,vec2(0u),
        vec2(0.0),vec3(0.0),vec3(0.0),vec3(0.0),vec3(0.0),index/3u);
}
fn raster_lit_sample(in:LitVertex) -> LitVertex {
    let stride=u32(shading[4].x);let base=in.triangle*3u*stride;
    let bary=raster_bary(base,stride,in.position.xy);let inverse=raster_inverse_z(base,stride,bary);
    let weights=vec3(precise_divide(stable(bary.x*vertex_float(base+2u)),inverse),precise_divide(stable(bary.y*vertex_float(base+stride+2u)),inverse),precise_divide(stable(bary.z*vertex_float(base+stride*2u+2u)),inverse));
    let a=vertex_vec3(base+3u);let b=vertex_vec3(base+stride+3u);let c=vertex_vec3(base+stride*2u+3u);
    let normal=vec3(
        stable(stable(stable(a.x*weights.x)+stable(b.x*weights.y))+stable(c.x*weights.z)),
        stable(stable(stable(a.y*weights.x)+stable(b.y*weights.y))+stable(c.y*weights.z)),
        stable(stable(stable(a.z*weights.x)+stable(b.z*weights.y))+stable(c.z*weights.z)));
    var uv=in.uv;var dx=in.uvDx;var dy=in.uvDy;var tangent=in.tangent;var bitangent=in.bitangent;
    if stride>13u {
        // Match CPU's corrected weights before interpolation. Interpolating UV/Z and
        // dividing afterward moves alpha-mask boundaries by a pixel at delivery sizes.
        let ua=vertex_vec2(base+13u);let ub=vertex_vec2(base+stride+13u);let uc=vertex_vec2(base+stride*2u+13u);
        uv=vec2(
            stable(stable(stable(ua.x*weights.x)+stable(ub.x*weights.y))+stable(uc.x*weights.z)),
            stable(stable(stable(ua.y*weights.x)+stable(ub.y*weights.y))+stable(uc.y*weights.z)));
        dx=vertex_vec3(base+15u);dy=vertex_vec3(base+18u);tangent=vertex_vec3(base+21u);bitangent=vertex_vec3(base+24u);
    }
    return LitVertex(in.position,inverse,normal,rasterVertices[base+12u],vec2(rasterVertices[base+7u],rasterVertices[base+8u]),
        uv,dx,dy,tangent,bitangent,in.triangle);
}
@fragment fn raster_lit_fragment(in:LitVertex) -> Fragment {
    let sample=raster_lit_sample(in);
    if shading[HEADER+sample.material*MATERIAL+2u].w!=0.0 {discard;}
    let depth=precise_divide(1.0,sample.invZ);
    return Fragment(lit_color(sample),pack_depth(depth));
}
