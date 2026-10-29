// The reference uses fixed-point weighted OIT, not primitive sorting. Two u32 atomics
// implement nonnegative 64-bit sums without requiring optional shader integer features.
struct AtomicSum { lo:atomic<u32>, hi:atomic<u32> };
@group(1) @binding(0) var<storage,read_write> red:array<AtomicSum>;
@group(1) @binding(1) var<storage,read_write> green:array<AtomicSum>;
@group(1) @binding(2) var<storage,read_write> blue:array<AtomicSum>;
@group(1) @binding(3) var<storage,read_write> weights:array<AtomicSum>;
@group(1) @binding(4) var<storage,read_write> extinction:array<AtomicSum>;
@group(1) @binding(5) var<storage,read_write> nearest:array<atomic<u32>>;
@group(1) @binding(6) var opaqueDepth:texture_depth_2d;
@group(1) @binding(7) var opaqueColor:texture_2d<f32>;
@group(1) @binding(8) var resolvedColor:texture_storage_2d<rgba16float,write>;
@group(1) @binding(9) var resolvedDepth:texture_storage_2d<rgba8unorm,write>;

fn sum_add(axis:u32,index:u32,value:f32) {
    let rounded=floor(value+0.5);let high=u32(floor(rounded/4294967296.0));
    let low=u32(rounded-f32(high)*4294967296.0);
    switch axis {
        case 0u: {let previous=atomicAdd(&red[index].lo,low);atomicAdd(&red[index].hi,high+select(0u,1u,previous>0xffffffffu-low));}
        case 1u: {let previous=atomicAdd(&green[index].lo,low);atomicAdd(&green[index].hi,high+select(0u,1u,previous>0xffffffffu-low));}
        case 2u: {let previous=atomicAdd(&blue[index].lo,low);atomicAdd(&blue[index].hi,high+select(0u,1u,previous>0xffffffffu-low));}
        case 3u: {let previous=atomicAdd(&weights[index].lo,low);atomicAdd(&weights[index].hi,high+select(0u,1u,previous>0xffffffffu-low));}
        case 4u: {let previous=atomicAdd(&extinction[index].lo,low);atomicAdd(&extinction[index].hi,high+select(0u,1u,previous>0xffffffffu-low));}
        default: {}
    }
}
fn sum_read(axis:u32,index:u32) -> f32 {
    switch axis {
        case 0u: {return f32(atomicLoad(&red[index].hi))*4294967296.0+f32(atomicLoad(&red[index].lo));}
        case 1u: {return f32(atomicLoad(&green[index].hi))*4294967296.0+f32(atomicLoad(&green[index].lo));}
        case 2u: {return f32(atomicLoad(&blue[index].hi))*4294967296.0+f32(atomicLoad(&blue[index].lo));}
        case 3u: {return f32(atomicLoad(&weights[index].hi))*4294967296.0+f32(atomicLoad(&weights[index].lo));}
        case 4u: {return f32(atomicLoad(&extinction[index].hi))*4294967296.0+f32(atomicLoad(&extinction[index].lo));}
        default: {return 0.0;}
    }
}
fn optical_depth(alpha:f32) -> f32 {
    // log(1-a) loses tiny coverage to f32 subtraction. Retain those contributions.
    if alpha<0.01 {return alpha*(1.0+alpha*(0.5+alpha*(1.0/3.0+alpha*0.25)));}
    return -log(1.0-alpha);
}
@fragment fn transparent_fragment(input:LitVertex) {
    let in=raster_lit_sample(input);
    let material=HEADER+in.material*MATERIAL;
    if shading[material+2u].w==0.0 {discard;}
    let pixel=vec2<i32>(in.position.xy);let z=precise_divide(1.0,in.invZ);
    if input.position.z<=textureLoad(opaqueDepth,pixel,0) {discard;}
    let c=transmitted_color(in,lit_radiance(in));let alpha=c.a;
    if alpha<=0.0 {discard;}
    let optical=optical_depth(min(alpha,0.99999994))*1000000000.0;
    if alpha<1.0 && floor(optical+0.5)==0.0 {discard;}
    let index=u32(pixel.y)*u32(globals.extent.x)+u32(pixel.x);
    let distance=1.0+z;let weight=alpha*clamp(1.0/(distance*distance),0.01,8.0)*1000000000.0;
    sum_add(0u,index,weight*c.r);sum_add(1u,index,weight*c.g);sum_add(2u,index,weight*c.b);
    sum_add(3u,index,max(weight,1.0));
    if alpha>=1.0 {atomicOr(&extinction[index].hi,0x80000000u);}
    else {sum_add(4u,index,optical);}
    atomicMax(&nearest[index],~bitcast<u32>(z));
}

fn linear_srgb(v:f32) -> f32 {
    if v<=0.04045 {return v/12.92;}
    return pow((v+0.055)/1.055,2.4);
}
@compute @workgroup_size(8,8) fn resolve_transparency(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=vec2<u32>(globals.extent)) {return;}
    let pixel=vec2<i32>(id.xy);let index=id.y*u32(globals.extent.x)+id.x;
    let background=textureLoad(opaqueColor,pixel,0);var result=background;
    let weight=sum_read(3u,index);
    var depth=bitcast<f32>(0x7f7fffffu);
    let storedDepth=textureLoad(opaqueDepth,pixel,0);
    if storedDepth>0.0 {depth=precise_divide(globals.depthScale,storedDepth);}
    if weight>0.0 {
        var reveal=0.0;
        if (atomicLoad(&extinction[index].hi)&0x80000000u)==0u {reveal=exp(-sum_read(4u,index)/1000000000.0);}
        let coverage=1.0-reveal;let alpha=coverage+reveal*background.a;
        var backdrop=vec3(0.0);
        if background.a>0.0 {let c=background.rgb/background.a;backdrop=vec3(linear_srgb(c.r),linear_srgb(c.g),linear_srgb(c.b))*background.a;}
        let transparent=vec3(sum_read(0u,index),sum_read(1u,index),sum_read(2u,index))/weight;
        var straight=vec3(0.0);
        if alpha>0.0 {straight=clamp((transparent*coverage+backdrop*reveal)/alpha,vec3(0.0),vec3(1.0));}
        result=vec4(half_even(srgb(straight.r)*alpha),half_even(srgb(straight.g)*alpha),half_even(srgb(straight.b)*alpha),half_even(alpha));
        depth=bitcast<f32>(~atomicLoad(&nearest[index]));
    }
    textureStore(resolvedColor,pixel,result);
    if all(id.xy<textureDimensions(resolvedDepth)) {textureStore(resolvedDepth,pixel,pack_depth(depth));}
}

fn transmission_depth(at:vec2<i32>)->f32 {let value=textureLoad(opaqueDepth,at,0);if value<=0.0{return bitcast<f32>(0x7f7fffffu);}return precise_divide(globals.depthScale,value);}
