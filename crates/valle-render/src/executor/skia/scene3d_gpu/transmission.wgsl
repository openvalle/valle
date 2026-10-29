// Shared Native/Web screen-space dielectric stage. The opaque snapshot is read-only;
// missing/offscreen/foreground samples are transparent, never previous-frame data.
fn transmission_linear(v:f32)->f32 {
    if v<=0.04045 {return v/12.92;}return pow((v+0.055)/1.055,2.4);
}
fn transmission_pixel(at:vec2<i32>,near:f32)->vec4<f32> {
    if any(at<vec2(0)) || any(at>=vec2<i32>(globals.extent)) {return vec4(0.0);}
    if transmission_depth(at)<=near {return vec4(0.0);}
    let p=textureLoad(opaqueColor,at,0);
    if p.a<=0.0 {return vec4(0.0);}
    let c=p.rgb/p.a;
    return vec4(vec3(transmission_linear(c.r),transmission_linear(c.g),transmission_linear(c.b))*p.a,p.a);
}
fn transmission_sample(p:vec2<f32>,near:f32)->vec4<f32> {
    let q=p-vec2(0.5);let i=vec2<i32>(floor(q));let t=q-floor(q);
    let a=transmission_pixel(i,near);let b=transmission_pixel(i+vec2(1,0),near);
    let c=transmission_pixel(i+vec2(0,1),near);let d=transmission_pixel(i+vec2(1,1),near);
    return (a*(1.0-t.x)+b*t.x)*(1.0-t.y)+(c*(1.0-t.x)+d*t.x)*t.y;
}
fn transmitted_color(in:LitVertex,reflected:vec4<f32>)->vec4<f32> {
    let material=HEADER+in.material*MATERIAL;
    let parameters=shading[material+19u];if parameters.x<=0.0 {return reflected;}
    let ray=camera_ray(vec2<u32>(in.position.xy));let v=unit(-ray);let n=lit_normal(in,material);
    let cosine=clamp(dot3(n,v),0.0,1.0);let eta=1.0/shading[material+22u].x;
    let direction=unit(-v*eta+n*(eta*cosine-sqrt(max(1.0-eta*eta*(1.0-cosine*cosine),0.0))));
    let distance=parameters.y/max(-dot3(direction,n),0.05);
    let mr=texture_slot(material,1u,in);let metallic=shading[material+1u].a*mr.b;
    let optics=shading[material+21u];let f0=max(max(optics.r,optics.g),optics.b);
    let cn=unit(select(in.normalOverZ,in.normalOverZ/in.invZ,in.triangle==0xffffffffu));
    let coat=1.0-shading[material+22u].y*(0.04+0.96*pow(1.0-clamp(dot3(cn,v),0.0,1.0),5.0));
    let weight=parameters.x*(1.0-metallic)*(1.0-(f0+(optics.a-f0)*pow(1.0-cosine,5.0)))*coat;
    let base=shading[material].rgb*texture_slot(material,0u,in).rgb;
    var tint=base;
    if parameters.z>0.0 && distance>0.0 {
        let color=shading[material+20u].rgb;
        tint*=select(pow(max(color,vec3(1e-30)),vec3(distance/parameters.z)),vec3(0.0),color==vec3(0.0));
    }
    let z=precise_divide(1.0,in.invZ);let exit=camera_position(vec2<u32>(in.position.xy),in.invZ)+direction*distance-shading[2].xyz;
    let counts=vec3<u32>(shading[1].xyz);
    let at=HEADER+counts.x*MATERIAL+counts.y*2u+counts.z*3u+u32(globals.extent.x)+u32(globals.extent.y);
    let right=shading[at];let up=shading[at+1u];let forward=shading[at+2u];let exitDepth=dot3(exit,forward.xyz);
    if exitDepth<forward.w {let alpha=1.0-weight;return vec4(clamp(reflected.rgb/max(alpha,1e-6),vec3(0.0),vec3(1.0)),reflected.a*alpha);}
    let screen=(vec2(dot3(exit,right.xyz)*right.w,-dot3(exit,up.xyz)*up.w)/select(exitDepth,1.0,orthographic())*0.5+vec2(0.5))*globals.extent;
    let roughness=max(shading[material+2u].x*mr.g,0.0525);
    let radius=min(roughness*roughness*distance*up.w*globals.extent.y/(2.0*select(z,1.0,orthographic())),32.0);
    var sampled=transmission_sample(screen,z)*0.4;
    sampled+=transmission_sample(screen+vec2(-radius,0.0),z)*0.15;
    sampled+=transmission_sample(screen+vec2(radius,0.0),z)*0.15;
    sampled+=transmission_sample(screen+vec2(0.0,-radius),z)*0.15;
    sampled+=transmission_sample(screen+vec2(0.0,radius),z)*0.15;
    let alpha=1.0-weight+weight*sampled.a;
    return vec4(clamp((reflected.rgb+weight*tint*sampled.rgb)/max(alpha,1e-6),vec3(0.0),vec3(1.0)),reflected.a*alpha);
}
