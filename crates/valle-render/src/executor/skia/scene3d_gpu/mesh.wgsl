// Static model data lives at binding 6. Frame parameters live after the camera-ray
// tables in shading. Morph -> skin -> node -> mesh -> camera matches the CPU order.
struct MeshVertex { position:vec3<f32>, normal:vec3<f32> };
fn mesh_affine(at:u32,p:vec3<f32>) -> vec3<f32> {
    return vec3(dot3(shading[at].xyz,p)+shading[at].w,
        dot3(shading[at+1u].xyz,p)+shading[at+1u].w,
        dot3(shading[at+2u].xyz,p)+shading[at+2u].w);
}
fn mesh_normal(at:u32,n:vec3<f32>) -> vec3<f32> {
    return vec3(dot3(shading[at].xyz,n),dot3(shading[at+1u].xyz,n),dot3(shading[at+2u].xyz,n));
}
fn mesh_rotate(v:vec3<f32>,s:vec3<f32>,c:vec3<f32>) -> vec3<f32> {
    let x=vec3(v.x,stable(v.y*c.x)-stable(v.z*s.x),stable(v.y*s.x)+stable(v.z*c.x));
    let y=vec3(stable(x.x*c.y)+stable(x.z*s.y),x.y,stable(-x.x*s.y)+stable(x.z*c.y));
    return vec3(stable(y.x*c.z)-stable(y.y*s.z),stable(y.x*s.z)+stable(y.y*c.z),y.z);
}
fn mesh_local(source:u32,instance:u32) -> MeshVertex {
    var p=vertex_vec3(source+4u);var n=vertex_vec3(source+7u);
    let count=min(rasterVertices[source],u32(shading[instance+8u].w));
    for(var i=0u;i<count;i++) {
        let weight=shading[instance+10u+i/4u][i%4u];
        if weight!=0.0 {
            p=stable4(vec4(p+stable4(vec4(vertex_vec3(source+18u+i*6u)*weight,0.0)).xyz,0.0)).xyz;
            n=stable4(vec4(n+stable4(vec4(vertex_vec3(source+21u+i*6u)*weight,0.0)).xyz,0.0)).xyz;
        }
    }
    if shading[instance+8u].w>0.0 && rasterVertices[source+1u]!=0u {n=unit(n);}
    if shading[instance+7u].w>0.0 {
        var sp=vec3(0.0);var sn=vec3(0.0);
        for(var i=0u;i<4u;i++) {
            let weight=vertex_float(source+14u+i);
            if weight!=0.0 {
                let joint=instance+12u+rasterVertices[source+10u+i]*6u;
                sp=stable4(vec4(sp+stable4(vec4(mesh_affine(joint,p)*weight,0.0)).xyz,0.0)).xyz;
                sn=stable4(vec4(sn+stable4(vec4(mesh_normal(joint+3u,n)*weight,0.0)).xyz,0.0)).xyz;
            }
        }
        p=sp;n=unit(sn);
    }
    return MeshVertex(p,n);
}
@vertex fn mesh_vertex(@builtin(vertex_index) draw_index:u32) -> LitVertex {
    // Finite reverse Z includes the far plane at depth zero, so use GreaterEqual
    // and submit triangles in reverse order. Equal depths still select the first
    // source triangle, exactly as the projected packet's forward/Greater ordering.
    let index=u32(shading[4].y)-3u-(draw_index/3u)*3u+draw_index%3u;
    let camera=u32(shading[4].x);
    let entry=shading[camera+4u+rasterVertices[index*4u+1u]];
    let instance=u32(entry.x);
    let corner=index%3u;
    let ordered=index-corner+select(corner,select(corner,3u-corner,corner>0u),entry.y!=0.0);
    let source=rasterVertices[ordered*4u];
    let vertex=mesh_local(source,instance);
    var local_normal=vertex.normal;
    if rasterVertices[source+1u]==0u {
        // Flat normals after deformation belong to the source triangle, before winding correction.
        let start=(index/3u)*12u;
        let a=mesh_local(rasterVertices[start],instance).position;
        let b=mesh_local(rasterVertices[start+4u],instance).position;
        let c=mesh_local(rasterVertices[start+8u],instance).position;
        local_normal=unit(cross(b-a,c-a));
    }
    let scale=shading[instance+6u].xyz;
    let sine=shading[instance+7u].xyz;let cosine=shading[instance+8u].xyz;
    let world=mesh_rotate(mesh_affine(instance,vertex.position)*scale,sine,cosine)+shading[instance+9u].xyz;
    let normal=unit(mesh_rotate(unit(mesh_normal(instance+3u,local_normal))/scale,sine,cosine));
    let relative=world-shading[camera+3u].xyz;
    let view=vec3(dot3(relative,shading[camera].xyz),dot3(relative,shading[camera+1u].xyz),dot3(relative,shading[camera+2u].xyz));
    let near=shading[camera+2u].w;let far=shading[camera+3u].w;
    var xy=view.xy*vec2(shading[camera].w,shading[camera+1u].w);
    if view.z>0.0 {
        let screen=(xy/view.z*vec2(0.5,-0.5)+vec2(0.5))*globals.extent;
        let snapped=floor(screen*256.0+vec2(0.5))/256.0;
        xy=(snapped/globals.extent*vec2(2.0,-2.0)+vec2(-1.0,1.0))*view.z;
    }
    // Finite reverse-Z gives hardware both near and far clipping, including triangles
    // crossing the camera plane. No CPU clipping or projected vertex upload is needed.
    var depth=near*(far-view.z)/(far-near);
    // Reciprocal lowering may round an exactly-on-near-plane depth above clip.w.
    // Clamp only vertices already inside the validated camera interval; outside
    // vertices must keep their out-of-range depth so hardware still clips them.
    if view.z>=near && view.z<=far {depth=clamp(depth,0.0,view.z);}
    let clip=vec4(xy,depth,view.z);
    return LitVertex(clip,1.0,normal,rasterVertices[index*4u+2u],vec2<u32>(entry.zw),
        vec2(0.0),vec3(0.0),vec3(0.0),vec3(0.0),vec3(0.0),0u);
}
@fragment fn mesh_fragment(in:LitVertex,@builtin(front_facing) front:bool) -> Fragment {
    if !front && shading[HEADER+in.material*MATERIAL+3u].z==0.0 {discard;}
    let sample=LitVertex(in.position,in.position.w,select(-in.normalOverZ,in.normalOverZ,front),
        in.material,in.ids,in.uv,in.uvDx,in.uvDy,in.tangent,in.bitangent,in.triangle);
    return Fragment(lit_color(sample),pack_depth(precise_divide(1.0,sample.invZ)));
}
