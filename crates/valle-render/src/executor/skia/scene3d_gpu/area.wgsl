// Linearly transformed cosine area integration. Fit and edge approximation:
// Heitz et al., via Three.js r186; MIT notice in scene3d/raster/assets/THREE-LICENSE.txt.
fn area_start(index:u32)->u32 {
    let c=vec3<u32>(shading[1].xyz);
    return HEADER+c.x*MATERIAL+c.y*2u+c.z*3u+u32(shading[5].x)*4u+index*4u;
}
fn ltc_texel(table:u32,p:vec2<i32>)->vec4<f32> {
    return textureLoad(dfg,clamp(p,vec2(0),vec2(63))+vec2(i32(table)*64,16),0);
}
fn ltc_lookup(table:u32,p:vec2<f32>)->vec4<f32> {
    let base=vec2<i32>(floor(p));let t=p-floor(p);
    let a=ltc_texel(table,base);let b=ltc_texel(table,base+vec2(1,0));
    let c=ltc_texel(table,base+vec2(0,1));let d=ltc_texel(table,base+vec2(1,1));
    return (a*(1.0-t.x)+b*t.x)*(1.0-t.y)+(c*(1.0-t.x)+d*t.x)*t.y;
}
fn ltc_edge(a:vec3<f32>,b:vec3<f32>)->vec3<f32> {
    let x=dot3(a,b);let y=abs(x);
    let ratio=(0.8543985+(0.4965155+0.0145206*y)*y)/(3.417594+(4.1616724+y)*y);
    let theta=select(0.5/sqrt(max(1.0-x*x,1e-7))-ratio,ratio,x>0.0);
    return cross(a,b)*theta;
}
fn ltc_integral(at:u32,n:vec3<f32>,v:vec3<f32>,position:vec3<f32>,m:vec4<f32>)->f32 {
    let center=shading[at].xyz;let w=shading[at+1u].xyz;let h=shading[at+2u].xyz;
    let corners=array<vec3<f32>,4>(center+w-h,center-w-h,center-w+h,center+w+h);
    let lightNormal=cross(corners[1]-corners[0],corners[3]-corners[0]);
    if dot3(lightNormal,position-corners[0])<0.0 {return 0.0;}
    var tangent=v-n*dot3(n,v);
    if dot3(tangent,tangent)<1e-12 {
        let axis=select(vec3(0.0,1.0,0.0),vec3(1.0,0.0,0.0),abs(n.x)<0.9);
        tangent=axis-n*dot3(n,axis);
    }
    tangent=unit(tangent);let bitangent=-cross(n,tangent);
    var q:array<vec3<f32>,4>;
    for(var i=0u;i<4u;i++) {
        let delta=corners[i]-position;let x=dot3(delta,tangent);let y=dot3(delta,bitangent);let z=dot3(delta,n);
        q[i]=vec3(m.x*x+m.z*z,y,m.y*x+m.w*z);
    }
    var clipped:array<vec3<f32>,6>;var count=0u;var previous=q[3];
    for(var i=0u;i<4u;i++) {
        let current=q[i];
        if (current.z>=0.0)!=(previous.z>=0.0) {
            clipped[count]=unit(previous+(current-previous)*(previous.z/(previous.z-current.z)));count++;
        }
        if current.z>=0.0 {clipped[count]=unit(current);count++;}
        previous=current;
    }
    if count<3u {return 0.0;}
    var form=vec3(0.0);
    for(var i=0u;i<count;i++) {form+=ltc_edge(clipped[i],clipped[(i+1u)%count]);}
    return max(form.z,0.0);
}
// x/y: diffuse/specular form factors; z/w: fitted Fresnel amplitudes.
fn area_evaluate(at:u32,n:vec3<f32>,v:vec3<f32>,position:vec3<f32>,roughness:f32)->vec4<f32> {
    let coord=vec2(clamp(roughness,0.0,1.0),sqrt(max(1.0-clamp(dot3(n,v),0.0,1.0),0.0)))*63.0;
    let m=ltc_lookup(0u,coord);let amp=ltc_lookup(1u,coord);
    return vec4(ltc_integral(at,n,n,position,vec4(1.0,0.0,0.0,1.0)),ltc_integral(at,n,v,position,m),amp.xy);
}
