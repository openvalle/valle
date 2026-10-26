@group(0) @binding(3) var atlas: texture_2d<f32>;
const HEADER: u32 = 5u;
const MATERIAL: u32 = 9u;

fn atlas_at(index: u32) -> vec4<f32> {
    let width = textureDimensions(atlas).x;
    return textureLoad(atlas,vec2<i32>(i32(index % width),i32(index / width)),0);
}
fn offset(v: vec4<f32>) -> u32 { return u32(v.x) + u32(v.y) * 65536u; }
fn mip_at(image: u32, level: u32) -> vec4<f32> { return atlas_at(offset(atlas_at(image)) + level); }
fn wrapped(value: i32, size: i32, mode: u32) -> i32 {
    if mode == 0u { return clamp(value,0,size-1); }
    if mode == 2u {
        let p = ((value % (2*size)) + 2*size) % (2*size);
        return select(2*size-1-p,p,p<size);
    }
    return ((value % size)+size)%size;
}
fn image_pixel(mip: vec4<f32>, p: vec2<i32>, sampling: u32) -> vec4<f32> {
    let x = wrapped(p.x,i32(mip.z),sampling % 4u);
    let y = wrapped(p.y,i32(mip.w),(sampling / 4u) % 4u);
    return atlas_at(offset(mip)+u32(y)*u32(mip.z)+u32(x));
}
fn image_level(image: u32, level: u32, uv: vec2<f32>, sampling: u32, linearFilter: bool) -> vec4<f32> {
    let mip = mip_at(image,level);
    let p = vec2(stable(uv.x*mip.z),stable(uv.y*mip.w))-0.5;
    if !linearFilter {return image_pixel(mip,vec2<i32>(floor(p+0.5)),sampling);}
    let i = vec2<i32>(floor(p)); let t = p-floor(p);
    let a = image_pixel(mip,i,sampling); let b = image_pixel(mip,i+vec2(1,0),sampling);
    let c = image_pixel(mip,i+vec2(0,1),sampling); let d = image_pixel(mip,i+vec2(1,1),sampling);
    let top=stable4(stable4(a*stable(1.0-t.x))+stable4(b*t.x));
    let bottom=stable4(stable4(c*stable(1.0-t.x))+stable4(d*t.x));
    return stable4(stable4(top*stable(1.0-t.y))+stable4(bottom*t.y));
}
fn sample_image(image: u32, sampling: u32, uv: vec2<f32>, lod: f32) -> vec4<f32> {
    let filterMode = select((sampling/16u)%8u,(sampling/128u)%2u,lod<=0.0);
    let linearFilter = (filterMode % 2u) == 1u;
    let levels = atlas_at(image).z;
    let level = select(0.0,clamp(lod,0.0,levels-1.0),filterMode>=2u);
    var a = u32(floor(level));
    if filterMode == 2u || filterMode == 3u {a=u32(floor(level+0.5));}
    let first = image_level(image,a,uv,sampling,linearFilter);
    if filterMode < 4u || a+1u >= u32(levels) {return first;}
    return stable4(first+stable4(stable4(image_level(image,a+1u,uv,sampling,linearFilter)-first)*stable(level-f32(a))));
}
fn texture_slot(material: u32, slot: u32, in: LitVertex) -> vec4<f32> {
    let binding = shading[material+4u+slot];
    if binding.x < 0.0 {return vec4(1.0);}
    let image = u32(binding.x);
    var uv=in.uv;
    // Hardware triangle interpolation carries UV/Z; exact bbox fragments already carry UV.
    if in.triangle==0xffffffffu {uv=in.uv/in.invZ;}
    let dx = (in.uvDx.xy-uv*in.uvDx.z)/in.invZ;
    let dy = (in.uvDy.xy-uv*in.uvDy.z)/in.invZ;
    let size = mip_at(image,0u).zw;
    let x = dx*size; let y = dy*size;
    let lod = 0.5*log2(max(max(dot(x,x),dot(y,y)),1e-16));
    return sample_image(image,u32(binding.y),uv,lod);
}

struct CubeUv { face: u32, uv: vec2<f32> };
fn face_uv(d: vec3<f32>) -> CubeUv {
    let a = abs(d); var f = 0u; var p = vec2(0.0);
    if a.x >= a.y && a.x >= a.z {
        if d.x >= 0.0 { f=0u;p=vec2(-d.z,-d.y)/a.x; } else {f=1u;p=vec2(d.z,-d.y)/a.x;}
    } else if a.y >= a.z {
        if d.y >= 0.0 {f=2u;p=vec2(d.x,d.z)/a.y;} else {f=3u;p=vec2(d.x,-d.z)/a.y;}
    } else {
        if d.z >= 0.0 {f=4u;p=vec2(d.x,-d.y)/a.z;} else {f=5u;p=vec2(-d.x,-d.y)/a.z;}
    }
    return CubeUv(f,p*0.5+0.5);
}
fn face_direction(face: u32, uv: vec2<f32>) -> vec3<f32> {
    let p = uv*2.0-1.0; var d = vec3(0.0);
    switch face {
        case 0u: { d=vec3(1.0,-p.y,-p.x); }
        case 1u: { d=vec3(-1.0,-p.y,p.x); }
        case 2u: { d=vec3(p.x,1.0,p.y); }
        case 3u: { d=vec3(p.x,-1.0,-p.y); }
        case 4u: { d=vec3(p.x,-p.y,1.0); }
        default: { d=vec3(-p.x,-p.y,-1.0); }
    }
    return d/sqrt(dot3(d,d));
}
fn cube_pixel(mip: vec4<f32>, face: u32, p: vec2<i32>) -> vec3<f32> {
    let size = i32(mip.z); var f = face; var q = p;
    if any(p<vec2(0)) || any(p>=vec2(size)) {
        let next = face_uv(face_direction(face,(vec2<f32>(p)+0.5)/f32(size)));
        f = next.face; q = vec2<i32>(next.uv*f32(size));
    }
    q = clamp(q,vec2(0),vec2(size-1));
    return atlas_at(offset(mip)+f*u32(size*size)+u32(q.y*size+q.x)).rgb;
}
fn cube_level(image: u32, level: u32, direction: vec3<f32>) -> vec3<f32> {
    let mip = mip_at(image,level); let f = face_uv(direction);
    let p = f.uv*mip.z-0.5; let i = vec2<i32>(floor(p)); let t = p-floor(p);
    let a = cube_pixel(mip,f.face,i);let b = cube_pixel(mip,f.face,i+vec2(1,0));
    let c = cube_pixel(mip,f.face,i+vec2(0,1));let d = cube_pixel(mip,f.face,i+vec2(1,1));
    return mix(mix(a,b,t.x),mix(c,d,t.x),t.y);
}
fn environment_direction(d: vec3<f32>) -> vec3<f32> {
    let s = shading[2].w;let c = shading[3].y;
    return vec3(c*d.x-s*d.z,d.y,s*d.x+c*d.z);
}
fn env_diffuse(d: vec3<f32>) -> vec3<f32> {
    if shading[3].z < 0.0 { return vec3(0.0); }
    return cube_level(u32(shading[3].z),0u,environment_direction(d))*shading[3].x;
}
fn env_specular(d: vec3<f32>, roughness: f32) -> vec3<f32> {
    if shading[3].z < 0.0 {return vec3(0.0);}
    let image = u32(shading[3].z)+1u;let last = u32(atlas_at(image).z)-1u;
    let level = clamp(roughness,0.0,1.0)*f32(last);let a = u32(floor(level));
    return mix(cube_level(image,a,environment_direction(d)),cube_level(image,min(a+1u,last),environment_direction(d)),level-f32(a))*shading[3].x;
}
