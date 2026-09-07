//! Import-free raw wasm32 entry for Motion Glass parity and performance gates.
//!
//! Programs remain canonical JSON, while transforms and premultiplied working-linear pixels use
//! their native binary representation. The host allocates exact-sized regions through
//! [`glass_alloc_ffi`], calls [`glass_render_ffi`], then releases every region through
//! [`glass_dealloc_ffi`]. There is no fixed scratch arena, JSON pixel expansion, or hidden
//! full-frame composite.

use std::alloc::{Layout, alloc, dealloc};

use crate::compositor::{
    glass::{
        MOTION_GLASS_GPU_UNIFORM_FLOATS, apply_glass_foreground_transformed,
        pack_motion_glass_foreground_gpu_uniforms, pack_motion_glass_gpu_uniforms,
        render_field_contribution_transformed, render_glass_contribution_transformed,
    },
    reference::{PremulRgba32, ReferenceImage},
};
use crate::resource::Extent2d;
use valle_draw::program::glass::{
    GlassOwnerKind, MotionGlassForegroundProgram, MotionGlassProgram,
};

const FFI_ALIGNMENT: usize = 8;
const WASM_PAGE_BYTES: usize = 65_536;

#[unsafe(no_mangle)]
pub extern "C" fn glass_alloc_ffi(size: u32) -> u32 {
    let Ok(size) = usize::try_from(size) else {
        return 0;
    };
    let Ok(layout) = Layout::from_size_align(size.max(1), FFI_ALIGNMENT) else {
        return 0;
    };
    // SAFETY: the matching layout is supplied to `glass_dealloc_ffi`; allocation failure is
    // represented as the null pointer required by this raw ABI.
    unsafe { alloc(layout) as usize as u32 }
}

#[unsafe(no_mangle)]
pub extern "C" fn glass_dealloc_ffi(pointer: u32, size: u32) {
    if pointer == 0 {
        return;
    }
    let Ok(size) = usize::try_from(size) else {
        return;
    };
    let Ok(layout) = Layout::from_size_align(size.max(1), FFI_ALIGNMENT) else {
        return;
    };
    // SAFETY: the ABI requires the exact pointer/size pair returned by `glass_alloc_ffi`.
    unsafe { dealloc(pointer as usize as *mut u8, layout) };
}

#[unsafe(no_mangle)]
pub const extern "C" fn glass_uniform_float_count_ffi() -> u32 {
    MOTION_GLASS_GPU_UNIFORM_FLOATS as u32
}

/// Pack the exact uniform block consumed by the shared Native/CanvasKit SkSL.
#[unsafe(no_mangle)]
pub extern "C" fn glass_pack_uniforms_ffi(
    program_pointer: u32,
    program_length: u32,
    transform_pointer: u32,
    output_pointer: u32,
    output_length: u32,
) -> i32 {
    glass_pack_uniforms_impl(
        program_pointer,
        program_length,
        transform_pointer,
        output_pointer,
        output_length,
        GlassFfiProgramKind::Material,
    )
}

/// Pack the foreground mode of the same fixed production uniform block.
#[unsafe(no_mangle)]
pub extern "C" fn glass_pack_foreground_uniforms_ffi(
    program_pointer: u32,
    program_length: u32,
    transform_pointer: u32,
    output_pointer: u32,
    output_length: u32,
) -> i32 {
    glass_pack_uniforms_impl(
        program_pointer,
        program_length,
        transform_pointer,
        output_pointer,
        output_length,
        GlassFfiProgramKind::Foreground,
    )
}

#[derive(Clone, Copy)]
enum GlassFfiProgramKind {
    Material,
    Foreground,
}

fn glass_pack_uniforms_impl(
    program_pointer: u32,
    program_length: u32,
    transform_pointer: u32,
    output_pointer: u32,
    output_length: u32,
    kind: GlassFfiProgramKind,
) -> i32 {
    let output_bytes = MOTION_GLASS_GPU_UNIFORM_FLOATS * core::mem::size_of::<f32>();
    if output_length as usize != output_bytes {
        return -1;
    }
    let Some(program_bytes) = checked_input(program_pointer, program_length as usize, 1) else {
        return -2;
    };
    let Some(transform_bytes) = checked_input(
        transform_pointer,
        9 * core::mem::size_of::<f64>(),
        align_of::<f64>(),
    ) else {
        return -3;
    };
    let Some(output_range) = checked_range(output_pointer, output_bytes, align_of::<f32>()) else {
        return -4;
    };
    if ranges_overlap(
        output_range,
        checked_range_value(program_pointer, program_length as usize),
    ) || ranges_overlap(
        output_range,
        checked_range_value(transform_pointer, 9 * core::mem::size_of::<f64>()),
    ) {
        return -5;
    }
    // SAFETY: checked_input proved alignment and the exact nine-value range.
    let transform =
        unsafe { std::slice::from_raw_parts(transform_bytes.as_ptr().cast::<f64>(), 9) };
    let mut owner_to_device = [0.0; 9];
    owner_to_device.copy_from_slice(transform);
    let uniforms = match kind {
        GlassFfiProgramKind::Material => {
            let Ok(program) = serde_json::from_slice::<MotionGlassProgram>(program_bytes) else {
                return -6;
            };
            let Ok(uniforms) = pack_motion_glass_gpu_uniforms(&program, owner_to_device) else {
                return -7;
            };
            uniforms
        }
        GlassFfiProgramKind::Foreground => {
            let Ok(program) = serde_json::from_slice::<MotionGlassForegroundProgram>(program_bytes)
            else {
                return -6;
            };
            let Ok(uniforms) = pack_motion_glass_foreground_gpu_uniforms(&program, owner_to_device)
            else {
                return -7;
            };
            uniforms
        }
    };
    // SAFETY: output_range is aligned, exact-sized, and disjoint from both live inputs.
    let output = unsafe {
        std::slice::from_raw_parts_mut(
            output_pointer as usize as *mut f32,
            MOTION_GLASS_GPU_UNIFORM_FLOATS,
        )
    };
    output.copy_from_slice(&uniforms);
    0
}

/// Render one transparent material contribution.
///
/// Return codes: `0` success; negative values identify admission, range, parse, pixel, transform,
/// kernel or output failures. Every pointer range is checked against current wasm memory before a
/// Rust slice is formed, and the writable output must not overlap any input.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn glass_render_ffi(
    program_pointer: u32,
    program_length: u32,
    pixel_pointer: u32,
    width: u32,
    height: u32,
    transform_pointer: u32,
    output_pointer: u32,
    output_length: u32,
) -> i32 {
    glass_render_impl(
        program_pointer,
        program_length,
        pixel_pointer,
        width,
        height,
        transform_pointer,
        output_pointer,
        output_length,
        GlassFfiProgramKind::Material,
    )
}

/// Render the real-foreground coverage mode of the shared kernel against ordinary input pixels.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn glass_render_foreground_ffi(
    program_pointer: u32,
    program_length: u32,
    pixel_pointer: u32,
    width: u32,
    height: u32,
    transform_pointer: u32,
    output_pointer: u32,
    output_length: u32,
) -> i32 {
    glass_render_impl(
        program_pointer,
        program_length,
        pixel_pointer,
        width,
        height,
        transform_pointer,
        output_pointer,
        output_length,
        GlassFfiProgramKind::Foreground,
    )
}

#[allow(clippy::too_many_arguments)]
fn glass_render_impl(
    program_pointer: u32,
    program_length: u32,
    pixel_pointer: u32,
    width: u32,
    height: u32,
    transform_pointer: u32,
    output_pointer: u32,
    output_length: u32,
    kind: GlassFfiProgramKind,
) -> i32 {
    let Some(channel_count) = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
    else {
        return -1;
    };
    let Some(pixel_bytes) = channel_count.checked_mul(core::mem::size_of::<f32>()) else {
        return -1;
    };
    if output_length as usize != pixel_bytes {
        return -2;
    }
    let program = match checked_input(program_pointer, program_length as usize, 1) {
        Some(bytes) => bytes,
        None => return -3,
    };
    let pixel_bytes_in = match checked_input(pixel_pointer, pixel_bytes, align_of::<f32>()) {
        Some(bytes) => bytes,
        None => return -4,
    };
    let transform_bytes = match checked_input(
        transform_pointer,
        9 * core::mem::size_of::<f64>(),
        align_of::<f64>(),
    ) {
        Some(bytes) => bytes,
        None => return -5,
    };
    let output_range = match checked_range(output_pointer, pixel_bytes, align_of::<f32>()) {
        Some(range) => range,
        None => return -6,
    };
    if ranges_overlap(
        output_range,
        checked_range_value(program_pointer, program_length as usize),
    ) || ranges_overlap(
        output_range,
        checked_range_value(pixel_pointer, pixel_bytes),
    ) || ranges_overlap(
        output_range,
        checked_range_value(transform_pointer, 9 * core::mem::size_of::<f64>()),
    ) {
        return -7;
    }

    // SAFETY: `checked_input` verified the f32 alignment and exact byte range.
    let channels =
        unsafe { std::slice::from_raw_parts(pixel_bytes_in.as_ptr().cast::<f32>(), channel_count) };
    // SAFETY: `checked_input` verified the f64 alignment and exact nine-value byte range.
    let transform =
        unsafe { std::slice::from_raw_parts(transform_bytes.as_ptr().cast::<f64>(), 9) };
    let mut owner_to_device = [0.0_f64; 9];
    owner_to_device.copy_from_slice(transform);
    if !owner_to_device.iter().all(|value| value.is_finite()) {
        return -9;
    }
    let Ok(extent) = Extent2d::new(width, height) else {
        return -10;
    };
    let pixels = match channels
        .chunks_exact(4)
        .map(|value| PremulRgba32::from_premultiplied([value[0], value[1], value[2], value[3]]))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(pixels) => pixels,
        Err(_) => return -11,
    };
    let Ok(backdrop) = ReferenceImage::new(extent, pixels) else {
        return -12;
    };
    let rendered = match kind {
        GlassFfiProgramKind::Material => {
            let Ok(program) = serde_json::from_slice::<MotionGlassProgram>(program) else {
                return -8;
            };
            match program.owner_kind {
                GlassOwnerKind::Independent => {
                    render_glass_contribution_transformed(&program, owner_to_device, &backdrop)
                }
                GlassOwnerKind::Field => {
                    render_field_contribution_transformed(&program, owner_to_device, &backdrop)
                }
            }
        }
        GlassFfiProgramKind::Foreground => {
            let Ok(program) = serde_json::from_slice::<MotionGlassForegroundProgram>(program)
            else {
                return -8;
            };
            apply_glass_foreground_transformed(&program, owner_to_device, &backdrop)
        }
    };
    let Ok(rendered) = rendered else {
        return -13;
    };

    // SAFETY: the output range is in-bounds, aligned, exact-sized, and proven disjoint from all
    // live input slices above.
    let output = unsafe {
        std::slice::from_raw_parts_mut(output_pointer as usize as *mut f32, channel_count)
    };
    for (target, source) in output.chunks_exact_mut(4).zip(rendered.pixels()) {
        target.copy_from_slice(&source.channels());
    }
    0
}

fn checked_input(pointer: u32, length: usize, alignment: usize) -> Option<&'static [u8]> {
    checked_range(pointer, length, alignment)?;
    // SAFETY: `checked_range` proved that the immutable byte range lies in wasm linear memory.
    Some(unsafe { std::slice::from_raw_parts(pointer as usize as *const u8, length) })
}

fn checked_range(pointer: u32, length: usize, alignment: usize) -> Option<(usize, usize)> {
    let start = pointer as usize;
    if start == 0 || start % alignment != 0 {
        return None;
    }
    let end = start.checked_add(length)?;
    let memory_bytes = core::arch::wasm32::memory_size::<0>().checked_mul(WASM_PAGE_BYTES)?;
    (end <= memory_bytes).then_some((start, end))
}

fn checked_range_value(pointer: u32, length: usize) -> (usize, usize) {
    let start = pointer as usize;
    (start, start.saturating_add(length))
}

fn ranges_overlap(left: (usize, usize), right: (usize, usize)) -> bool {
    left.0 < right.1 && right.0 < left.1
}
