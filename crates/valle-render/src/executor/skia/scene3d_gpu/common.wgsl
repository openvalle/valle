// Keep local aggregates immutable. Naga's SPIR-V backend attaches buffer layout
// decorations to their types, which Vulkan rejects on Function-storage variables.
struct Globals { extent: vec2<f32>, depthScale: f32, padding: u32 };
@group(0) @binding(0) var<uniform> globals: Globals;
struct Fragment {
    @location(0) color: vec4<f32>,
    @location(1) packedDepth: vec4<f32>,
};
fn pack_depth(depth: f32) -> vec4<f32> {
    let bits = bitcast<u32>(depth);
    return vec4<f32>(vec4(bits,bits >> 8u,bits >> 16u,bits >> 24u) & vec4(255u)) / 255.0;
}
