//! Process-local render-graph structure hashing.
//!
//! This is deliberately an explicit byte projection rather than a serde shape. The value is a
//! memoization key inside one Product engine process, not a wire format, and therefore must not
//! acquire a format header or make serde field names part of cache identity.

use sha2::{Digest, Sha256};
use valle_draw::{
    Rect,
    program::{BackdropScope, BlendMode},
};

use crate::{
    frame::{RasterFit, RenderQuality, RenderSpec},
    prepare::{
        BoundsReason, ExternalPlacement, ExternalSample, PreparedBlurAxis, PreparedEffect,
        PreparedEffectKernel, PreparedEffectSpace, PreparedExternalBackdrop, PreparedMask,
        PreparedMaskShape, PreparedProgramKind, PreparedTransitionKernel, PreparedUnitRect,
    },
    resource::{
        ColorDescription, ColorPrimaries, ColorRange, Dither, ExternalPixelLayout,
        ExternalResourceDesc, GamutMap, InputAlphaMode, LogicalTextureDesc, MatrixCoefficients,
        OperatorAlphaBehavior, OperatorColorDomain, OutputAlphaMode, OutputBackground,
        OutputBitDepth, OutputSpec, PixelOrientation, ResourceInterpretation, ResourceKey,
        SignalLuminance, ToneMap, TransferFunction, VisualInterpretation, WorkingAlphaMode,
        WorkingColorSpace,
    },
};

use super::{
    GraphCapability, GraphOrigin, GraphResource, GraphResourceKind, GraphRoi, LayerRole,
    LogicalPassKind, OrderEdge, OrderReason, PassStage, RenderGraph, ResourceAccess, ResourceEdge,
};

const DOMAIN: &[u8] = b"valle.engine/render-graph-structure\0";

pub(super) fn hash(graph: &RenderGraph) -> crate::resource::ContentDigest {
    let mut writer = StructureHash::new();
    writer.fixed(DOMAIN);
    writer.fixed(graph.render_id.as_bytes());
    write_render_spec(&mut writer, graph.render_spec);

    writer.len(graph.programs.len());
    for program in &graph.programs {
        writer.u32(program.id.get());
        write_program_kind(&mut writer, program.kind);
        writer.str(&program.semantic_path);
    }

    writer.len(graph.resources.len());
    for resource in &graph.resources {
        write_resource(&mut writer, resource);
    }

    writer.len(graph.passes.len());
    for pass in &graph.passes {
        writer.u32(pass.id.get());
        writer.str(&pass.semantic_path);
        write_pass_stage(&mut writer, pass.stage);
        write_pass_kind(&mut writer, &pass.kind);
    }

    writer.len(graph.edges.len());
    for edge in &graph.edges {
        write_resource_edge(&mut writer, edge);
    }
    writer.len(graph.order_edges.len());
    for edge in &graph.order_edges {
        write_order_edge(&mut writer, edge);
    }
    writer.len(graph.capabilities.len());
    for capability in &graph.capabilities {
        write_graph_capability(&mut writer, *capability);
    }
    writer.u32(graph.output.get());
    writer.finish()
}

struct StructureHash(Sha256);

impl StructureHash {
    fn new() -> Self {
        Self(Sha256::new())
    }

    fn finish(self) -> crate::resource::ContentDigest {
        crate::resource::ContentDigest::from_bytes(self.0.finalize().into())
    }

    fn fixed(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.len(bytes.len());
        self.fixed(bytes);
    }

    fn str(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    fn tag(&mut self, value: u8) {
        self.fixed(&[value]);
    }

    fn bool(&mut self, value: bool) {
        self.tag(u8::from(value));
    }

    fn u8(&mut self, value: u8) {
        self.tag(value);
    }

    fn u16(&mut self, value: u16) {
        self.fixed(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.fixed(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.fixed(&value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.fixed(&value.to_le_bytes());
    }

    fn f32(&mut self, value: f32) {
        self.u32(value.to_bits());
    }

    fn f64(&mut self, value: f64) {
        self.u64(value.to_bits());
    }

    fn len(&mut self, value: usize) {
        self.u64(u64::try_from(value).expect("in-memory graph length fits u64"));
    }

    fn option<T>(&mut self, value: Option<T>, write: impl FnOnce(&mut Self, T)) {
        match value {
            Some(value) => {
                self.tag(1);
                write(self, value);
            }
            None => self.tag(0),
        }
    }
}

fn write_render_spec(writer: &mut StructureHash, spec: RenderSpec) {
    writer.u32(spec.width());
    writer.u32(spec.height());
    writer.tag(match spec.quality() {
        RenderQuality::Preview => 0,
        RenderQuality::Final => 1,
    });
    write_output_spec(writer, spec.output());
}

fn write_output_spec(writer: &mut StructureHash, spec: OutputSpec) {
    let target = spec.target();
    write_color_primaries(writer, target.primaries);
    write_transfer(writer, target.transfer);
    writer.tag(match spec.alpha() {
        OutputAlphaMode::Opaque => 0,
        OutputAlphaMode::StraightCoverage => 1,
        OutputAlphaMode::PremultipliedCoverage => 2,
    });
    match spec.background() {
        OutputBackground::Transparent => writer.tag(0),
        OutputBackground::AuthorSrgbStraight { color } => {
            writer.tag(1);
            writer.fixed(&color.0);
        }
    }
    writer.tag(match spec.tone_map() {
        ToneMap::None => 0,
        ToneMap::ReinhardLuminance => 1,
    });
    writer.tag(match spec.gamut_map() {
        GamutMap::Clip => 0,
        GamutMap::ChromaCompress => 1,
    });
    match spec.dither() {
        Dither::None => writer.tag(0),
        Dither::Triangular { seed } => {
            writer.tag(1);
            writer.u64(seed);
        }
    }
    writer.tag(match spec.bit_depth() {
        OutputBitDepth::Eight => 0,
        OutputBitDepth::Ten => 1,
        OutputBitDepth::Twelve => 2,
        OutputBitDepth::Sixteen => 3,
        OutputBitDepth::Float16 => 4,
    });
    write_luminance(writer, spec.luminance());
}

fn write_luminance(writer: &mut StructureHash, luminance: SignalLuminance) {
    writer.u16(luminance.reference_white().get());
    writer.u16(luminance.peak().get());
}

fn write_program_kind(writer: &mut StructureHash, kind: PreparedProgramKind) {
    writer.tag(match kind {
        PreparedProgramKind::Motion => 0,
        PreparedProgramKind::Caption => 1,
        PreparedProgramKind::Solid => 2,
    });
}

fn write_resource(writer: &mut StructureHash, resource: &GraphResource) {
    writer.u32(resource.id.get());
    writer.str(&resource.semantic_path);
    write_resource_kind(writer, &resource.kind);
    writer.option(resource.texture.as_ref(), write_texture);
    write_roi(writer, &resource.roi);
    write_origin(writer, &resource.origin);
}

fn write_resource_kind(writer: &mut StructureHash, kind: &GraphResourceKind) {
    match kind {
        GraphResourceKind::ExternalResource {
            handle,
            key,
            expected,
        } => {
            writer.tag(0);
            writer.u32(handle.get());
            write_resource_key(writer, key);
            write_external_desc(writer, expected);
        }
        GraphResourceKind::Layer { role } => {
            writer.tag(1);
            writer.tag(match role {
                LayerRole::ImportedSource => 0,
                LayerRole::ProgramSource => 1,
                LayerRole::Filtered => 2,
                LayerRole::Masked => 3,
                LayerRole::Grouped => 4,
            });
        }
        GraphResourceKind::Composite { version } => {
            writer.tag(2);
            writer.u32(version.get());
        }
        GraphResourceKind::BackdropView {
            source_version,
            scope,
        } => {
            writer.tag(3);
            writer.u32(source_version.get());
            write_backdrop_scope(writer, scope);
        }
        GraphResourceKind::Mask => writer.tag(4),
        GraphResourceKind::Auxiliary => writer.tag(5),
        GraphResourceKind::HistorySlot { slot } => {
            writer.tag(6);
            writer.u32(*slot);
        }
        GraphResourceKind::Output { spec } => {
            writer.tag(7);
            write_output_spec(writer, *spec);
        }
    }
}

fn write_resource_key(writer: &mut StructureHash, key: &ResourceKey) {
    writer.fixed(key.content.as_bytes());
    match &key.interpretation {
        ResourceInterpretation::Visual { interpretation } => {
            writer.tag(0);
            write_visual_interpretation(writer, *interpretation);
        }
        ResourceInterpretation::FontFace { face_index } => {
            writer.tag(1);
            writer.u32(*face_index);
        }
        ResourceInterpretation::RuntimeShader {
            abi_digest,
            color_domain,
            alpha_behavior,
        } => {
            writer.tag(2);
            writer.fixed(abi_digest.as_bytes());
            match color_domain {
                OperatorColorDomain::WorkingLinear => writer.tag(0),
                OperatorColorDomain::PerceptualSrgb => writer.tag(1),
                OperatorColorDomain::AssetEncoded(transfer) => {
                    writer.tag(2);
                    write_transfer(writer, *transfer);
                }
            }
            writer.tag(match alpha_behavior {
                OperatorAlphaBehavior::PreservesCoverage => 0,
                OperatorAlphaBehavior::RewritesCoverage => 1,
                OperatorAlphaBehavior::FiltersPremultiplied => 2,
            });
        }
        ResourceInterpretation::Scene3d { topology_digest } => {
            writer.tag(3);
            writer.fixed(topology_digest.as_bytes());
        }
        ResourceInterpretation::AudioPcm {
            sample_rate,
            channels,
        } => {
            writer.tag(4);
            writer.u32(sample_rate.get());
            writer.u16(channels.get());
        }
    }
}

fn write_visual_interpretation(writer: &mut StructureHash, value: VisualInterpretation) {
    write_color_description(writer, value.color);
    write_luminance(writer, value.luminance);
    writer.tag(match value.alpha {
        InputAlphaMode::Opaque => 0,
        InputAlphaMode::StraightCoverage => 1,
        InputAlphaMode::PremultipliedCoverage => 2,
    });
    writer.tag(match value.orientation {
        PixelOrientation::Identity => 0,
        PixelOrientation::MirrorHorizontal => 1,
        PixelOrientation::Rotate180 => 2,
        PixelOrientation::MirrorVertical => 3,
        PixelOrientation::MirrorHorizontalRotate270 => 4,
        PixelOrientation::Rotate90 => 5,
        PixelOrientation::MirrorHorizontalRotate90 => 6,
        PixelOrientation::Rotate270 => 7,
    });
    writer.u32(value.sample_aspect_ratio.numerator());
    writer.u32(value.sample_aspect_ratio.denominator());
    for fraction in [
        value.crop.left(),
        value.crop.top(),
        value.crop.right(),
        value.crop.bottom(),
    ] {
        writer.u32(fraction.numerator());
        writer.u32(fraction.denominator());
    }
}

fn write_color_description(writer: &mut StructureHash, value: ColorDescription) {
    write_color_primaries(writer, value.primaries);
    write_transfer(writer, value.transfer);
    writer.tag(match value.matrix {
        MatrixCoefficients::Identity => 0,
        MatrixCoefficients::Bt601 => 1,
        MatrixCoefficients::Bt709 => 2,
        MatrixCoefficients::Bt2020Ncl => 3,
    });
    writer.tag(match value.range {
        ColorRange::Full => 0,
        ColorRange::Limited => 1,
    });
}

fn write_color_primaries(writer: &mut StructureHash, value: ColorPrimaries) {
    writer.tag(match value {
        ColorPrimaries::Rec709 => 0,
        ColorPrimaries::DisplayP3 => 1,
        ColorPrimaries::Rec2020 => 2,
    });
}

fn write_transfer(writer: &mut StructureHash, value: TransferFunction) {
    writer.tag(match value {
        TransferFunction::Linear => 0,
        TransferFunction::Srgb => 1,
        TransferFunction::Rec709 => 2,
        TransferFunction::Pq => 3,
        TransferFunction::Hlg => 4,
    });
}

fn write_external_desc(writer: &mut StructureHash, value: &ExternalResourceDesc) {
    match value {
        ExternalResourceDesc::VisualFrame {
            extent,
            pixel_layout,
        } => {
            writer.tag(0);
            writer.u32(extent.width());
            writer.u32(extent.height());
            writer.tag(match pixel_layout {
                ExternalPixelLayout::Rgba8 => 0,
                ExternalPixelLayout::Rgba16Float => 1,
                ExternalPixelLayout::Nv12 => 2,
                ExternalPixelLayout::P010 => 3,
                ExternalPixelLayout::I420 => 4,
            });
        }
        ExternalResourceDesc::FontBytes => writer.tag(1),
        ExternalResourceDesc::RuntimeShader => writer.tag(2),
        ExternalResourceDesc::Scene3d => writer.tag(3),
        ExternalResourceDesc::AudioBlock {
            sample_rate,
            channels,
            frames,
        } => {
            writer.tag(4);
            writer.u32(sample_rate.get());
            writer.u16(channels.get());
            writer.u32(frames.get());
        }
    }
}

fn write_texture(writer: &mut StructureHash, value: &LogicalTextureDesc) {
    writer.u32(value.extent.width());
    writer.u32(value.extent.height());
    writer.tag(match value.format {
        crate::resource::TextureFormat::Rgba16Float => 0,
        crate::resource::TextureFormat::Rgba32Float => 1,
    });
    writer.tag(match value.working_space {
        WorkingColorSpace::LinearRec2020D65 => 0,
    });
    writer.tag(match value.alpha {
        WorkingAlphaMode::PremultipliedCoverage => 0,
    });
    writer.len(value.usages().len());
    for usage in value.usages() {
        writer.tag(match usage {
            crate::resource::TextureUsage::Sampled => 0,
            crate::resource::TextureUsage::StorageRead => 1,
            crate::resource::TextureUsage::StorageWrite => 2,
            crate::resource::TextureUsage::ColorAttachment => 3,
            crate::resource::TextureUsage::CopySource => 4,
            crate::resource::TextureUsage::CopyDestination => 5,
        });
    }
    writer.u8(value.sample_count());
}

fn write_roi(writer: &mut StructureHash, roi: &GraphRoi) {
    match roi {
        GraphRoi::FullFrame => writer.tag(0),
        GraphRoi::Static { rect } => {
            writer.tag(1);
            writer.i32(rect.x);
            writer.i32(rect.y);
            writer.u32(rect.width);
            writer.u32(rect.height);
        }
        GraphRoi::Dynamic { binding } => {
            writer.tag(2);
            writer.u32(binding.get());
        }
    }
}

fn write_origin(writer: &mut StructureHash, origin: &GraphOrigin) {
    match origin {
        GraphOrigin::Static { x, y } => {
            writer.tag(0);
            writer.i32(*x);
            writer.i32(*y);
        }
        GraphOrigin::Dynamic { bounds } => {
            writer.tag(1);
            writer.u32(bounds.get());
        }
    }
}

fn write_pass_stage(writer: &mut StructureHash, stage: PassStage) {
    writer.tag(match stage {
        PassStage::Visual => 0,
        PassStage::Caption => 1,
        PassStage::Output => 2,
    });
}

fn write_pass_kind(writer: &mut StructureHash, kind: &LogicalPassKind) {
    match kind {
        LogicalPassKind::ClearComposite {
            output,
            working_linear_rec2020_premul,
        } => {
            writer.tag(0);
            writer.u32(output.get());
            write_f32_slice(writer, working_linear_rec2020_premul);
        }
        LogicalPassKind::Import {
            external,
            source_pipeline,
            output,
            placement,
            transform,
        } => {
            writer.tag(1);
            writer.u32(external.get());
            writer.option(source_pipeline.chroma_key.as_ref(), write_effect);
            writer.u32(output.get());
            write_external_placement(writer, *placement);
            writer.u32(transform.get());
        }
        LogicalPassKind::Draw {
            program,
            external_inputs,
            destination_inputs,
            output,
            transform,
            bounds_reason,
        } => {
            writer.tag(2);
            writer.u32(program.get());
            write_resource_ids(writer, external_inputs);
            write_resource_ids(writer, destination_inputs);
            writer.u32(output.get());
            writer.u32(transform.get());
            write_bounds_reason(writer, *bounds_reason);
        }
        LogicalPassKind::Group { input, output } => {
            writer.tag(3);
            writer.u32(input.get());
            writer.u32(output.get());
        }
        LogicalPassKind::BackdropRead {
            token,
            output,
            sample_bounds,
            output_bounds,
        } => {
            writer.tag(4);
            write_backdrop_scope(writer, &token.scope);
            writer.u32(token.composite.get());
            writer.u32(token.version.get());
            writer.u32(output.get());
            writer.u32(sample_bounds.get());
            writer.u32(output_bounds.get());
        }
        LogicalPassKind::Filter {
            input,
            output,
            effect,
        } => {
            writer.tag(5);
            writer.u32(input.get());
            writer.u32(output.get());
            write_effect(writer, effect);
        }
        LogicalPassKind::Mask {
            input,
            output,
            mask,
        } => {
            writer.tag(6);
            writer.u32(input.get());
            writer.u32(output.get());
            write_mask(writer, *mask);
        }
        LogicalPassKind::CompositeLayer {
            backdrop,
            layer,
            output,
            opacity,
        } => {
            writer.tag(7);
            writer.u32(backdrop.get());
            writer.u32(layer.get());
            writer.u32(output.get());
            writer.u32(opacity.get());
        }
        LogicalPassKind::Blend {
            backdrop,
            layer,
            output,
            mode,
            opacity,
        } => {
            writer.tag(8);
            writer.u32(backdrop.get());
            writer.u32(layer.get());
            writer.u32(output.get());
            write_blend_mode(writer, *mode);
            writer.u32(opacity.get());
        }
        LogicalPassKind::Transition {
            backdrop,
            from,
            to,
            output,
            kernel,
            progress,
            from_opacity,
            to_opacity,
        } => {
            writer.tag(9);
            writer.u32(backdrop.get());
            writer.u32(from.get());
            writer.u32(to.get());
            writer.u32(output.get());
            write_transition(writer, *kernel);
            writer.u32(progress.get());
            writer.u32(from_opacity.get());
            writer.u32(to_opacity.get());
        }
        LogicalPassKind::AdjustmentEffect {
            input,
            output,
            effect,
        } => {
            writer.tag(10);
            writer.u32(input.get());
            writer.u32(output.get());
            write_effect(writer, effect);
        }
        LogicalPassKind::Caption {
            backdrop,
            program,
            external_inputs,
            output,
            transform,
            bounds,
            bounds_reason,
            opacity,
        } => {
            writer.tag(11);
            writer.u32(backdrop.get());
            writer.u32(program.get());
            write_resource_ids(writer, external_inputs);
            writer.u32(output.get());
            writer.u32(transform.get());
            writer.u32(bounds.get());
            write_bounds_reason(writer, *bounds_reason);
            writer.u32(opacity.get());
        }
        LogicalPassKind::OutputTransform {
            input,
            output,
            spec,
        } => {
            writer.tag(12);
            writer.u32(input.get());
            writer.u32(output.get());
            write_output_spec(writer, *spec);
        }
    }
}

fn write_resource_ids(writer: &mut StructureHash, ids: &[super::ResourceId]) {
    writer.len(ids.len());
    for id in ids {
        writer.u32(id.get());
    }
}

fn write_effect(writer: &mut StructureHash, effect: &PreparedEffect) {
    writer.str(&effect.semantic_path);
    match effect.space {
        PreparedEffectSpace::Layer { transform, bounds } => {
            writer.tag(0);
            writer.u32(transform.get());
            writer.u32(bounds.get());
        }
        PreparedEffectSpace::Root => writer.tag(1),
    }
    match effect.kernel {
        PreparedEffectKernel::ChromaKey {
            key_working_linear_rec2020,
            intensity,
            shadow,
            feather_sigma_device_px,
            edge_clean,
        } => {
            writer.tag(0);
            write_f32_slice(writer, &key_working_linear_rec2020);
            writer.f32(intensity);
            writer.f32(shadow);
            writer.u32(feather_sigma_device_px.get());
            writer.f32(edge_clean);
        }
        PreparedEffectKernel::ColorGrade {
            brightness,
            contrast,
            saturation,
            temperature,
            vignette,
        } => {
            writer.tag(1);
            writer.f32(brightness);
            writer.f32(contrast);
            writer.f32(saturation);
            writer.f32(temperature);
            writer.f32(vignette);
        }
        PreparedEffectKernel::GaussianBlur {
            sigma_device_px,
            region,
        } => {
            writer.tag(2);
            writer.u32(sigma_device_px.get());
            writer.option(region, write_unit_rect);
        }
        PreparedEffectKernel::Mosaic {
            block_size_device_px,
            region,
        } => {
            writer.tag(3);
            writer.u32(block_size_device_px.get());
            writer.option(region, write_unit_rect);
        }
        PreparedEffectKernel::DirectionalBlur {
            axis,
            span_device_px,
        } => {
            writer.tag(4);
            writer.tag(match axis {
                PreparedBlurAxis::Horizontal => 0,
                PreparedBlurAxis::Vertical => 1,
            });
            writer.u32(span_device_px.get());
        }
        PreparedEffectKernel::Spotlight {
            center,
            radius,
            feather,
            intensity,
        } => {
            writer.tag(5);
            write_f32_slice(writer, &center);
            writer.f32(radius);
            writer.f32(feather);
            writer.f32(intensity);
        }
        PreparedEffectKernel::ExtensionColorGain {
            implementation_sha256,
            gain,
            past_frames,
            future_frames,
        } => {
            writer.tag(6);
            writer.fixed(&implementation_sha256);
            writer.f32(gain);
            writer.u32(past_frames);
            writer.u32(future_frames);
        }
    }
}

fn write_unit_rect(writer: &mut StructureHash, rect: PreparedUnitRect) {
    writer.f32(rect.x);
    writer.f32(rect.y);
    writer.f32(rect.width);
    writer.f32(rect.height);
}

fn write_external_placement(writer: &mut StructureHash, placement: ExternalPlacement) {
    writer.tag(match placement.fit {
        RasterFit::Contain => 0,
        RasterFit::Cover => 1,
        RasterFit::Fill => 2,
    });
    write_rect(writer, placement.content_rect);
    write_rect(writer, placement.clip_rect);
    write_external_sample(writer, placement.sample);
    writer.option(placement.backdrop, |writer, backdrop| match backdrop {
        PreparedExternalBackdrop::Color {
            working_linear_rec2020_premul,
        } => {
            writer.tag(0);
            write_f32_slice(writer, &working_linear_rec2020_premul);
        }
        PreparedExternalBackdrop::Blur {
            sigma_device_px,
            sample,
        } => {
            writer.tag(1);
            writer.u32(sigma_device_px.get());
            write_external_sample(writer, sample);
        }
    });
}

fn write_external_sample(writer: &mut StructureHash, sample: ExternalSample) {
    match sample {
        ExternalSample::Empty => writer.tag(0),
        ExternalSample::Texture {
            texture_from_content,
            input_sample_bounds,
            sampling,
        } => {
            writer.tag(1);
            write_f64_slice(writer, &texture_from_content);
            write_rect(writer, input_sample_bounds);
            writer.tag(match sampling {
                crate::prepare::ExternalSampling::LinearClampToSampleBounds => 0,
            });
        }
    }
}

fn write_rect(writer: &mut StructureHash, rect: Rect) {
    writer.f64(rect.x);
    writer.f64(rect.y);
    writer.f64(rect.width);
    writer.f64(rect.height);
}

fn write_mask(writer: &mut StructureHash, mask: PreparedMask) {
    writer.tag(match mask.shape() {
        PreparedMaskShape::Rect => 0,
        PreparedMaskShape::Ellipse => 1,
    });
    write_f64_slice(writer, &mask.device_from_mask().matrix());
    writer.f64(mask.feather_sigma_device_px());
    writer.bool(mask.invert());
}

fn write_transition(writer: &mut StructureHash, kernel: PreparedTransitionKernel) {
    match kernel {
        PreparedTransitionKernel::Fade => writer.tag(0),
        PreparedTransitionKernel::WipeLeft => writer.tag(1),
        PreparedTransitionKernel::WipeRight => writer.tag(2),
        PreparedTransitionKernel::CircleOpen => writer.tag(3),
        PreparedTransitionKernel::SimpleZoom => writer.tag(4),
        PreparedTransitionKernel::CrossWarp => writer.tag(5),
        PreparedTransitionKernel::LinearBlur => writer.tag(6),
        PreparedTransitionKernel::DirectionalWarp => writer.tag(7),
        PreparedTransitionKernel::DreamyZoom => writer.tag(8),
        PreparedTransitionKernel::Ripple => writer.tag(9),
        PreparedTransitionKernel::FlyEye => writer.tag(10),
        PreparedTransitionKernel::MultiplyBlend => writer.tag(11),
        PreparedTransitionKernel::Perlin => writer.tag(12),
        PreparedTransitionKernel::ExtensionCrossFade {
            implementation_sha256,
            past_frames,
            future_frames,
        } => {
            writer.tag(13);
            writer.fixed(&implementation_sha256);
            writer.u32(past_frames);
            writer.u32(future_frames);
        }
    }
}

fn write_backdrop_scope(writer: &mut StructureHash, scope: &BackdropScope) {
    match scope {
        BackdropScope::Current => writer.tag(0),
        BackdropScope::LayerEntry(key) => {
            writer.tag(1);
            writer.str(key);
        }
        BackdropScope::ScopeEntry(key) => {
            writer.tag(2);
            writer.str(key);
        }
    }
}

fn write_blend_mode(writer: &mut StructureHash, mode: BlendMode) {
    writer.tag(match mode {
        BlendMode::Normal => 0,
        BlendMode::Multiply => 1,
        BlendMode::Screen => 2,
        BlendMode::Overlay => 3,
        BlendMode::Darken => 4,
        BlendMode::Lighten => 5,
        BlendMode::ColorDodge => 6,
        BlendMode::ColorBurn => 7,
        BlendMode::LinearBurn => 8,
        BlendMode::HardLight => 9,
        BlendMode::SoftLight => 10,
        BlendMode::Difference => 11,
        BlendMode::Exclusion => 12,
        BlendMode::Hue => 13,
        BlendMode::Saturation => 14,
        BlendMode::Color => 15,
        BlendMode::Luminosity => 16,
    });
}

fn write_bounds_reason(writer: &mut StructureHash, reason: BoundsReason) {
    writer.tag(match reason {
        BoundsReason::Exact => 0,
        BoundsReason::ConservativeCameraTarget => 1,
    });
}

fn write_resource_edge(writer: &mut StructureHash, edge: &ResourceEdge) {
    writer.u32(edge.pass.get());
    writer.u32(edge.resource.get());
    writer.tag(match edge.access {
        ResourceAccess::Read => 0,
        ResourceAccess::Write => 1,
    });
}

fn write_order_edge(writer: &mut StructureHash, edge: &OrderEdge) {
    writer.u32(edge.before.get());
    writer.u32(edge.after.get());
    writer.tag(match edge.reason {
        OrderReason::VisualSpine => 0,
        OrderReason::CaptionTerminal => 1,
        OrderReason::OutputTerminal => 2,
    });
}

fn write_graph_capability(writer: &mut StructureHash, capability: GraphCapability) {
    writer.tag(match capability {
        GraphCapability::Clear => 0,
        GraphCapability::ExternalImport => 1,
        GraphCapability::SourcePipeline => 2,
        GraphCapability::DrawProgram => 3,
        GraphCapability::BackdropRead => 4,
        GraphCapability::Group => 5,
        GraphCapability::Filter => 6,
        GraphCapability::Mask => 7,
        GraphCapability::Blend => 8,
        GraphCapability::Transition => 9,
        GraphCapability::AdjustmentEffect => 10,
        GraphCapability::Caption => 11,
        GraphCapability::OutputTransform => 12,
    });
}

fn write_f32_slice(writer: &mut StructureHash, values: &[f32]) {
    writer.len(values.len());
    for value in values {
        writer.f32(*value);
    }
}

fn write_f64_slice(writer: &mut StructureHash, values: &[f64]) {
    writer.len(values.len());
    for value in values {
        writer.f64(*value);
    }
}
