@group(0) @binding(4) var shadowMaps:texture_2d_array<f32>;
@group(0) @binding(5) var<uniform> shadowRange:vec4<f32>;
struct ShadowVertex {
    @builtin(position) position:vec4<f32>,
    @location(0) depth:f32,
    @location(1) uv:vec2<f32>,
    @location(2) @interpolate(flat) material:u32,
    @location(3) @interpolate(flat) triangle:u32,
};
@vertex fn shadow_vertex(@builtin(vertex_index) index:u32) -> ShadowVertex {
    let base=(index/3u)*21u;
    return ShadowVertex(triangle_position(index,7u,vec2(1024.0),(vertex_float(index*7u+2u)-shadowRange.x)*shadowRange.y),0.0,vec2(0.0),rasterVertices[base+5u],index/3u);
}
struct ShadowFragment { @location(0) depth:f32 };
@fragment fn shadow_fragment(in:ShadowVertex) -> ShadowFragment {
    let base=in.triangle*21u;let bary=raster_bary(base,7u,in.position.xy);
    let depth=stable(stable(stable(vertex_float(base+2u)*bary.x)+stable(vertex_float(base+9u)*bary.y))+stable(vertex_float(base+16u)*bary.z));
    let uv=vertex_vec2(base+3u)*bary.x+vertex_vec2(base+10u)*bary.y+vertex_vec2(base+17u)*bary.z;
    let material=HEADER+in.material*MATERIAL;
    let alpha=shading[material+2u];
    if alpha.z>=0.0 {
        let binding=shading[material+4u];
        var coverage=alpha.y;
        if binding.x>=0.0 {coverage*=sample_image(u32(binding.x),u32(binding.y),texture_uv(material,0u,uv),0.0).a;}
        if coverage<alpha.z {discard;}
    }
    return ShadowFragment(depth);
}

fn shadow_visibility(light:u32,position:vec3<f32>,normal:vec3<f32>) -> f32 {
    if shading[4].z<0.0 {return 1.0;}
    let lightStart=HEADER+u32(shading[1].x)*MATERIAL;
    let layer=i32(shading[lightStart+light*2u].w);
    if layer<0 {return 1.0;}
    let start=u32(shading[4].z)+u32(layer)*4u;
    let r=shading[start];let u=shading[start+1u];let f=shading[start+2u];let scale=shading[start+3u];
    let p=vec2(stable(dot3(position,r.xyz)-r.w)*scale.x,stable(u.w-dot3(position,u.xyz))*scale.y);
    if any(p<vec2(0.0)) || any(p>=vec2(1024.0)) {return 1.0;}
    let denominator=max(abs(dot3(normal,f.xyz)),0.05);
    let slope=vec2(precise_divide(abs(dot3(normal,r.xyz)),denominator),precise_divide(abs(dot3(normal,u.xyz)),denominator));
    let bias=max(min(stable(stable(1.5*stable(precise_divide(slope.x,scale.x)+precise_divide(slope.y,scale.y)))+stable(f.w*0.00001)),f.w*0.02),0.0001);
    let receiver=dot3(position,f.xyz)-bias;let center=vec2<i32>(floor(p));
    var visible=0.0;
    for(var y=-1;y<=1;y++) {for(var x=-1;x<=1;x++) {
        let at=center+vec2(x,y);
        if any(at<vec2(0)) || any(at>=vec2(1024)) {visible+=1.0;}
        else if receiver<=textureLoad(shadowMaps,at,layer,0).r {visible+=1.0;}
    }}
    return visible/9.0;
}
