//! G0 Motion Glass reference math, kinematics, and material resolver.
//!
//! No Native/Web kernel execution lives here. G1 binds this into RenderGraph.

mod bounds;
mod cache;
mod culling;
mod gpu;
mod kinematics;
mod material;
mod metrics;
mod pack;
mod potential;
mod reference;
mod render;
mod response;
mod sampling;
mod track;

pub use bounds::{
    MotionGlassBackdropBounds, MotionGlassBoundsError, resolve_motion_glass_backdrop_bounds,
    validate_motion_glass_backdrop_bounds,
};
pub use cache::{GlassTrackCache, GlassTrackCacheKey};
pub use culling::{
    CullError, field_cull_bounds, field_sample_culled, in_any_support, member_cull_bounds,
};
pub use gpu::{
    GlassGpuUniformError, MOTION_GLASS_GPU_PATH_POINTS, MOTION_GLASS_GPU_SURFACES,
    MOTION_GLASS_GPU_UNIFORM_FLOATS, MotionGlassGpuUniforms,
    pack_motion_glass_foreground_gpu_uniforms, pack_motion_glass_gpu_uniforms,
};
pub use kinematics::{
    DevicePose, GlassKinematics, backward_derivative, kinematics_between, kinematics_from_centers,
};
pub use material::{
    ResolvedGlassMaterialBase, pack_material_base, resolve_material_base,
    resolve_material_base_with_geometry,
};
pub use metrics::{
    EXPECTED_GLASS_PASSES, EXPECTED_GLASS_READBACKS, EXPECTED_GLASS_RESOLVES,
    GlassStructureCounters, MAX_REFERENCE_PR_PIXELS, MAX_RESPONSE_INTEGRATION_SAMPLES,
    MAX_TRACK_SAMPLES_PER_FRAME, StructureError,
};
pub use pack::{
    BACKDROP_SCOPE_CURRENT, FieldGlassPackage, FieldMemberPackage, IndependentGlassPackage,
    PackError, glass_independent_output_margin, glass_shadow_radius, motion_glass_sample_margin,
    pack_field_glass, pack_independent_glass,
};
pub(crate) use pack::{assemble_field_glass, assemble_independent_glass};
pub use potential::{field_pseudo_distance, potential_w};
pub use reference::{materialization, signed_circle_distance};
pub(crate) use render::transformed_foreground_distance;
pub use render::{
    GlassRenderError, apply_glass_foreground_transformed, apply_rect_mask,
    composite_glass_contribution, quantize_to_f16, render_field_contribution,
    render_field_contribution_transformed, render_glass_contribution,
    render_glass_contribution_f16, render_glass_contribution_f16_transformed,
    render_glass_contribution_transformed,
};
pub use response::{
    CharacterKernel, RESPONSE_INTEGRATION_SAMPLES, ResponseManifest, causal_response, kernel_value,
    packed_current_response, response_force,
};
pub use track::{
    DeviceTrackError, GlassDeviceShape, GlassSurfaceKey, GlassSurfaceSpec,
    MAX_GLASS_DEVICE_SAMPLES, qualify_device_track,
};
pub use valle_draw::program::glass::{
    KERNEL_DERIVATIVE_STEP_SECONDS as DERIVATIVE_STEP_SECONDS, KERNEL_EPSILON as GLASS_EPSILON,
    KERNEL_MATERIALIZATION as MATERIALIZATION_FORMULA, KERNEL_NARROW_BAND as NARROW_BAND,
    KERNEL_POTENTIAL_ID as POTENTIAL_KERNEL_ID, MOTION_GLASS_KERNEL_DIGEST_HEX,
    motion_glass_kernel_digest, verify_kernel,
};
