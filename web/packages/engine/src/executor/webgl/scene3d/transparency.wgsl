// WebGL2 has no fragment atomics. One fragment resolves only its tile's transparent
// triangles, using the same nonnegative 64-bit fixed-point sums as Native.
@group(1) @binding(0) var transparentReferences:texture_2d<u32>;
@group(1) @binding(1) var transparentTiles:texture_2d<u32>;
@group(1) @binding(2) var opaqueColor:texture_2d<f32>;
@group(1) @binding(3) var opaqueDepth:texture_2d<f32>;

fn web_reference(index:u32)->u32 {
    let at=index/4u;let width=textureDimensions(transparentReferences).x;
    return textureLoad(transparentReferences,vec2<i32>(i32(at%width),i32(at/width)),0)[index%4u];
}
fn web_inside(triangle:u32,pixel:vec2<f32>)->bool {
    let stride=u32(shading[4].x);let base=triangle*stride*3u;
    let a=vertex_vec2(base);let b=vertex_vec2(base+stride);let c=vertex_vec2(base+2u*stride);
    let p=floor(pixel)+vec2(0.5);
    return web_edge_inside(raster_edge(b,c,p),b,c)&&web_edge_inside(raster_edge(c,a,p),c,a)&&web_edge_inside(raster_edge(a,b,p),a,b);
}
// WebGL's tile resolve has no fixed-function triangle coverage. Keep its top-left fill
// convention, sharing floating interpolation with Native without emulated 64-bit products.
fn web_edge_inside(edge:f32,a:vec2<f32>,b:vec2<f32>)->bool {
    let d=b-a;
    return edge>0.0 || (edge==0.0 && (d.y>0.0 || (d.y==0.0 && d.x<0.0)));
}
fn web_sum(value:f32)->vec2<u32> {
    let rounded=floor(value+0.5);let high=u32(floor(rounded/4294967296.0));
    return vec2(u32(rounded-f32(high)*4294967296.0),high);
}
fn web_add(a:vec2<u32>,b:vec2<u32>)->vec2<u32> {
    let low=a.x+b.x;return vec2(low,a.y+b.y+select(0u,1u,low<a.x));
}
fn web_read(value:vec2<u32>)->f32 {return f32(value.y)*4294967296.0+f32(value.x);}
fn web_optical(alpha:f32)->f32 {
    if alpha<0.01 {return alpha*(1.0+alpha*(0.5+alpha*(1.0/3.0+alpha*0.25)));}
    return -log(1.0-alpha);
}
fn web_linear(v:f32)->f32 {if v<=0.04045{return v/12.92;}return pow((v+0.055)/1.055,2.4);}
fn web_unpack_depth(value:vec4<f32>)->f32 {
    let code=vec4<u32>(floor(value*255.0+0.5));
    return bitcast<f32>(code.x|(code.y<<8u)|(code.z<<16u)|(code.w<<24u));
}
@fragment fn web_transparency(input:LitVertex)->Fragment {
    let pixel=vec2<i32>(input.position.xy);
    let background=textureLoad(opaqueColor,pixel,0);
    let opaque=web_unpack_depth(textureLoad(opaqueDepth,pixel,0));
    let tile=u32(pixel.y)/16u*((u32(globals.extent.x)+15u)/16u)+u32(pixel.x)/16u;
    let width=textureDimensions(transparentTiles).x;
    let span=textureLoad(transparentTiles,vec2<i32>(i32(tile%width),i32(tile/width)),0).xy;
    var sums:array<vec2<u32>,5>;var solid=false;var nearest=opaque;
    for(var i=0u;i<span.y;i++) {
        let triangle=web_reference(span.x+i);
        if !web_inside(triangle,input.position.xy) {continue;}
        var seed=make_lit_vertex(vec2(0.0),1.0,0u,0u,vec3(0.0),0u);
        seed.triangle=triangle;seed.position=input.position;
        let sample=raster_lit_sample(seed);let z=precise_divide(1.0,sample.invZ);
        if z>=opaque {continue;}
        let material=HEADER+sample.material*MATERIAL;
        let alpha_values=shading[material+2u];
        let coverage=alpha_values.y*texture_slot(material,0u,sample).a;
        if alpha_values.z>=0.0&&coverage<alpha_values.z {continue;}
        let color=transmitted_color(sample,lit_radiance(sample));let alpha=color.a;
        if alpha<=0.0 {continue;}
        let optical=web_optical(min(alpha,0.99999994))*1000000000.0;
        if alpha<1.0&&floor(optical+0.5)==0.0 {continue;}
        let distance=1.0+z;let weight=alpha*clamp(1.0/(distance*distance),0.01,8.0)*1000000000.0;
        sums[0]=web_add(sums[0],web_sum(weight*color.r));sums[1]=web_add(sums[1],web_sum(weight*color.g));sums[2]=web_add(sums[2],web_sum(weight*color.b));
        sums[3]=web_add(sums[3],web_sum(max(weight,1.0)));
        if alpha>=1.0 {solid=true;} else {sums[4]=web_add(sums[4],web_sum(optical));}
        nearest=min(nearest,z);
    }
    let weight=web_read(sums[3]);var result=background;
    if weight>0.0 {
        var reveal=0.0;if !solid {reveal=exp(-web_read(sums[4])/1000000000.0);}
        let coverage=1.0-reveal;let alpha=coverage+reveal*background.a;
        var backdrop=vec3(0.0);
        if background.a>0.0 {let c=background.rgb/background.a;backdrop=vec3(web_linear(c.r),web_linear(c.g),web_linear(c.b))*background.a;}
        let transparent=vec3(web_read(sums[0]),web_read(sums[1]),web_read(sums[2]))/weight;
        var straight=vec3(0.0);if alpha>0.0 {straight=clamp((transparent*coverage+backdrop*reveal)/alpha,vec3(0.0),vec3(1.0));}
        result=vec4(half_even(srgb(straight.r)*alpha),half_even(srgb(straight.g)*alpha),half_even(srgb(straight.b)*alpha),half_even(alpha));
    }
    return Fragment(result,pack_depth(nearest));
}

fn transmission_depth(at:vec2<i32>)->f32 {return web_unpack_depth(textureLoad(opaqueDepth,at,0));}
