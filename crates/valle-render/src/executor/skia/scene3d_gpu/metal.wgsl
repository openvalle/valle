struct Globals { extent: vec2<f32>, depthScale: f32, padding: u32 };
@group(0) @binding(0) var<uniform> globals: Globals;
struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) invZ: f32,
    @location(1) @interpolate(flat) color: vec4<f32>,
};
@vertex fn vertex(@location(0) position: vec2<f32>, @location(1) invZ: f32,
    @location(2) color: vec4<f32>) -> Vertex {
    var out: Vertex;
    out.position = vec4(position / globals.extent * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.5, 1.0);
    out.invZ = invZ;
    out.color = color;
    return out;
}
struct Fragment {
    @location(0) color: vec4<f32>,
    @location(1) packedDepth: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};
fn pack_depth(depth: f32) -> vec4<f32> {
    let bits = bitcast<u32>(depth);
    return vec4<f32>(vec4(bits,bits >> 8u,bits >> 16u,bits >> 24u) & vec4(255u)) / 255.0;
}
@fragment fn fragment(in: Vertex) -> Fragment {
    var out: Fragment;
    out.color = in.color;
    out.depth = globals.depthScale / in.invZ;
    out.packedDepth = pack_depth(1.0 / in.invZ);
    return out;
}
