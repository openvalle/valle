use std::{collections::BTreeMap, sync::Arc};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use valle_draw::{
    Rect,
    program::{BackdropScope, DrawProgram, DrawProgramBuilder, NodeId, ProgramTextureExtent},
    requirements::{DestinationOperation, FontKey, RuntimeShaderKey, Scene3dKey, TextureKind},
};
use valle_timeline::{FrameRate, RationalTime};

use crate::resource::{
    ContentDigest, Extent2d, ExternalHandleId, ResourceSample, SemanticAsset, SemanticAssetKind,
    SemanticFont, SemanticStructure, StructureDescriptor,
};

use super::{
    bounds::BoundsReason,
    request::{DynamicBindingId, RequestAllocator, RequestError},
};

pub(crate) struct BuiltMotionProgram {
    pub program: DrawProgram,
    pub scene3d_frames: Vec<valle_motion::Scene3DFrameRequest>,
    pub layout_boxes: BTreeMap<String, [f32; 4]>,
}

pub(crate) struct CompiledMotionProgramContext<'a> {
    pub prepared: &'a valle_motion::PreparedScene,
    pub instance: &'a crate::render::CompiledMotionInstance,
    pub evaluated: &'a crate::render::EvaluatedSourceRef,
    pub source_frame: u32,
    pub viewport: Extent2d,
    pub fps: FrameRate,
    pub styles: &'a valle_motion::StyleCache,
    pub faces: &'a valle_motion::FaceCache,
    pub program_to_device: super::DeviceTransform,
    pub clip_id: &'a str,
    pub render_seed: u32,
    pub assets: &'a BTreeMap<String, SemanticAsset>,
}

/// Timeline Motion producer. Every input is already compiled or a
/// resolver-verified immutable fact; no Timeline `MotionContent`, WordsDoc,
/// registry, or locator is reconstructed here.
pub(crate) fn build_compiled_motion_program(
    context: CompiledMotionProgramContext<'_>,
) -> Result<BuiltMotionProgram, ProgramPrepareError> {
    let artifact = context.prepared.artifact();
    let overrides = compiled_motion_overrides(
        artifact,
        context
            .evaluated
            .motion_props()
            .ok_or_else(|| ProgramPrepareError::MotionProps {
                reason: "compiled Motion props are missing".into(),
            })?,
    )?;
    let props = valle_motion::resolve_props(&artifact.controls, &overrides).map_err(|error| {
        ProgramPrepareError::MotionProps {
            reason: error.to_string(),
        }
    })?;

    let phase_windows = context.instance.phases().layout();
    let total_frames = phase_windows.duration_frames;
    let local_frame = context.source_frame;
    if local_frame >= total_frames {
        return Err(ProgramPrepareError::CompiledInvariant {
            reason: format!(
                "compiled source frame {local_frame} is outside Motion duration {total_frames}"
            ),
        });
    }
    let motion_context = valle_motion::motion_context_at(local_frame, &phase_windows, context.fps)
        .ok_or_else(|| ProgramPrepareError::CompiledInvariant {
            reason: format!("local frame {local_frame} is outside Motion duration {total_frames}"),
        })?;

    let cue_windows = compiled_cue_windows(context.instance, context.fps)?;
    let cues =
        valle_motion::CueSchedule::resolve(&artifact.controls, &cue_windows).map_err(|error| {
            ProgramPrepareError::MotionSignals {
                reason: error.to_string(),
            }
        })?;
    let signals = cues.sample(local_frame, context.fps);

    let mut fonts = valle_motion::Fonts::default();
    valle_motion::register_default_motion_fonts(&mut fonts).map_err(|error| {
        ProgramPrepareError::MotionLayout {
            reason: error.to_string(),
        }
    })?;
    for resource in context.evaluated.motion_artifact_dependencies() {
        let crate::render::VerifiedResourceFacts::Font { bytes, .. } = resource.facts() else {
            continue;
        };
        let digest = *resource.digest();
        register_motion_dependency_font(&mut fonts, bytes, &digest)?;
    }
    let options = valle_motion::LayoutOptions {
        viewport: valle_motion::Viewport::new((
            context.viewport.width(),
            context.viewport.height(),
        )),
        fonts: &fonts,
        styles: Some(context.styles),
    };
    let mut tree = valle_motion::build_tree(
        context.prepared,
        &motion_context,
        &props,
        &signals,
        &options,
    )
    .map_err(|error| ProgramPrepareError::MotionLayout {
        reason: error.to_string(),
    })?;
    if !tree.glass.surfaces.is_empty() {
        let glass = tree.glass.clone();
        let instance = valle_timeline::internal::MotionInstanceId::new(context.clip_id.to_owned())
            .map_err(|error| ProgramPrepareError::MotionGlass {
                reason: error.to_string(),
            })?;
        install_glass_programs(
            &mut tree,
            &[(motion_context.sample, glass)],
            context.program_to_device,
            context.viewport,
            instance,
            context.render_seed,
        )?;
    }
    let layout_boxes = valle_motion::layout_boxes(artifact, &tree).map_err(|error| {
        ProgramPrepareError::MotionLayout {
            reason: error.to_string(),
        }
    })?;
    let catalog = CompiledMotionProgramCatalog {
        assets: context.assets,
    };
    let report = valle_motion::emit_program_with_faces(
        &tree,
        &valle_motion::default_font_naming,
        Some(context.faces),
        &catalog,
    )
    .map_err(|error| ProgramPrepareError::MotionEmit {
        reason: error.to_string(),
    })?;
    if !report.unsupported.is_empty() {
        return Err(ProgramPrepareError::UnsupportedMotionPaint {
            features: report
                .unsupported
                .into_iter()
                .map(|(node, feature)| format!("{node}: {feature}"))
                .collect(),
        });
    }
    Ok(BuiltMotionProgram {
        program: report.program,
        scene3d_frames: report.scene3d_frames,
        layout_boxes,
    })
}

fn register_motion_dependency_font(
    fonts: &mut valle_motion::Fonts,
    bytes: &[u8],
    digest: &ContentDigest,
) -> Result<(), ProgramPrepareError> {
    let is_default = valle_motion::DEFAULT_MOTION_FONT_WEIGHTS
        .iter()
        .any(|default| *default == bytes);
    if !is_default {
        // Preserve the font's own name-table family for artifacts that author that family
        // directly. Takumi deduplicates by content + family, so the content-addressed alias below
        // remains a distinct lookup for compiler-lowered `asset://...` references.
        fonts
            .register(valle_motion::FontResource::new(bytes.to_vec()))
            .map_err(|error| ProgramPrepareError::MotionLayout {
                reason: error.to_string(),
            })?;
    }
    fonts
        .register(
            valle_motion::FontResource::new(bytes.to_vec()).override_info(
                valle_motion::FontOverride {
                    family_name: Some(Arc::<str>::from(valle_motion::font_family_alias(digest))),
                    ..Default::default()
                },
            ),
        )
        .map_err(|error| ProgramPrepareError::MotionLayout {
            reason: error.to_string(),
        })?;
    Ok(())
}

fn cue_duration_frames(
    duration: valle_timeline::RationalTime,
    fps: FrameRate,
) -> Result<u32, ProgramPrepareError> {
    let interval = valle_timeline::internal::quantize::quantize_frame_interval(
        valle_timeline::RationalTime::ZERO,
        duration,
        fps,
    )
    .map_err(|error| ProgramPrepareError::MotionSignals {
        reason: error.to_string(),
    })?;
    u32::try_from(interval.duration_frames).map_err(|_| ProgramPrepareError::MotionSignals {
        reason: "cue duration frame count exceeds u32".into(),
    })
}

fn compiled_motion_overrides(
    artifact: &valle_motion::SceneArtifact,
    values: &BTreeMap<String, crate::render::EvaluatedMotionValue>,
) -> Result<BTreeMap<String, valle_motion::MotionValue>, ProgramPrepareError> {
    use crate::render::EvaluatedMotionValue;
    use valle_motion::{ControlType, MotionValue};
    let mut output = BTreeMap::new();
    for (name, value) in values {
        let control = &artifact
            .controls
            .props
            .get(name)
            .ok_or_else(|| ProgramPrepareError::MotionProps {
                reason: format!("compiled prop {name:?} is absent from the artifact"),
            })?
            .control;
        let converted = match (control, value) {
            (ControlType::Number { .. }, EvaluatedMotionValue::Scalar(value)) => {
                MotionValue::Number(*value)
            }
            (ControlType::Length, EvaluatedMotionValue::Scalar(value)) => {
                MotionValue::Length(valle_motion::value::Length::px(*value))
            }
            (ControlType::Angle, EvaluatedMotionValue::Scalar(value)) => {
                MotionValue::Angle(valle_motion::value::Angle::deg(*value))
            }
            (ControlType::Point, EvaluatedMotionValue::Vec2(value)) => {
                MotionValue::Point(valle_draw::Point::new(value[0], value[1]))
            }
            (ControlType::Color, EvaluatedMotionValue::Vec4(value)) => {
                MotionValue::Color(valle_draw::Rgba::new(
                    value[0].round().clamp(0.0, 255.0) as u8,
                    value[1].round().clamp(0.0, 255.0) as u8,
                    value[2].round().clamp(0.0, 255.0) as u8,
                    value[3].round().clamp(0.0, 255.0) as u8,
                ))
            }
            (ControlType::Rect, EvaluatedMotionValue::Vec4(value)) => MotionValue::Rect(
                valle_draw::Rect::new(value[0], value[1], value[2], value[3]),
            ),
            (ControlType::Bool, EvaluatedMotionValue::Boolean(value)) => MotionValue::Bool(*value),
            (ControlType::String, EvaluatedMotionValue::String(value)) => {
                MotionValue::Str(value.clone())
            }
            (ControlType::Select { .. }, EvaluatedMotionValue::String(value)) => {
                MotionValue::Enum(value.clone())
            }
            _ => {
                return Err(ProgramPrepareError::MotionProps {
                    reason: format!("compiled prop {name:?} no longer matches its artifact type"),
                });
            }
        };
        output.insert(name.clone(), converted);
    }
    Ok(output)
}

fn compiled_cue_windows(
    instance: &crate::render::CompiledMotionInstance,
    fps: FrameRate,
) -> Result<BTreeMap<String, valle_motion::CueWindow>, ProgramPrepareError> {
    let mut output = BTreeMap::new();
    for (name, cue) in instance.cues() {
        let (start, end, enter, exit) = match cue {
            crate::render::CompiledMotionCue::SourceRange {
                start,
                end,
                enter_duration,
                exit_duration,
            } => (*start, *end, *enter_duration, *exit_duration),
        };
        let interval = valle_timeline::internal::quantize::quantize_frame_interval(
            start,
            end.checked_sub(start)
                .map_err(|error| ProgramPrepareError::MotionSignals {
                    reason: error.to_string(),
                })?,
            fps,
        )
        .map_err(|error| ProgramPrepareError::MotionSignals {
            reason: error.to_string(),
        })?;
        output.insert(
            name.clone(),
            valle_motion::CueWindow {
                start_frame: u32::try_from(interval.start_frame).map_err(|_| {
                    ProgramPrepareError::MotionSignals {
                        reason: "cue starts outside u32".into(),
                    }
                })?,
                end_frame: u32::try_from(interval.end_frame).map_err(|_| {
                    ProgramPrepareError::MotionSignals {
                        reason: "cue ends outside u32".into(),
                    }
                })?,
                enter_frames: cue_duration_frames(enter, fps)?,
                exit_frames: cue_duration_frames(exit, fps)?,
            },
        );
    }
    Ok(output)
}

struct CompiledMotionProgramCatalog<'a> {
    assets: &'a BTreeMap<String, SemanticAsset>,
}

impl valle_draw::program::ProgramResourceCatalog for CompiledMotionProgramCatalog<'_> {
    fn texture_extent(&self, asset_key: &str) -> Option<ProgramTextureExtent> {
        let control = asset_key.strip_prefix("asset://").unwrap_or(asset_key);
        let asset = self.assets.get(control)?;
        let extent = asset.descriptor.extent()?;
        let interpretation = asset.descriptor.visual_interpretation()?;
        let [width, height] =
            super::external::interpreted_display_extent(extent, interpretation).ok()?;
        Some(ProgramTextureExtent { width, height })
    }

    fn scene3d_resource_digest(&self, control: &str) -> Option<[u8; 32]> {
        self.assets
            .get(control)
            .map(|asset| *asset.digest.as_bytes())
    }
}

fn install_glass_programs(
    tree: &mut valle_motion::LayoutTree,
    frames: &[(
        valle_timeline::internal::SampleTime,
        valle_motion::GlassLayoutFrame,
    )],
    program_to_device: super::DeviceTransform,
    viewport: Extent2d,
    instance: valle_timeline::internal::MotionInstanceId,
    render_seed: u32,
) -> Result<(), ProgramPrepareError> {
    use crate::compositor::glass::{
        FieldGlassPackage, FieldMemberPackage, GlassSurfaceSpec, IndependentGlassPackage,
        qualify_device_track, resolve_material_base_with_geometry,
    };
    use valle_motion::glass::GlassSurfaceTrackSample;

    let current = tree.glass.clone();
    let epoch = valle_timeline::internal::TrackEpoch::new(
        render_seed,
        instance.clone(),
        valle_timeline::internal::TimeMapSegment::new(0),
        0,
        valle_timeline::internal::DiscontinuityIndex::NONE,
    );
    let program_normalize = [
        1.0 / f64::from(viewport.width()),
        0.0,
        0.0,
        0.0,
        1.0 / f64::from(viewport.height()),
        0.0,
        0.0,
        0.0,
        1.0,
    ];
    let viewport_to_device = matrix_mul(program_to_device.matrix(), program_normalize);

    struct QualifiedSurface {
        spec: GlassSurfaceSpec,
        shapes: Vec<crate::compositor::glass::GlassDeviceShape>,
        path_points: Vec<[f32; 2]>,
    }
    let mut qualified = BTreeMap::<String, QualifiedSurface>::new();
    for surface in &current.surfaces {
        let mean_axis = ((surface.local_rect.width + surface.local_rect.height) * 0.5).max(1e-9);
        let maximum_radius = surface.local_rect.width.min(surface.local_rect.height) * 0.5;
        let normalized_radius = f64::from(surface.radius).clamp(0.0, maximum_radius) / mean_axis;
        let normalized_path = surface
            .path_points
            .iter()
            .map(|point| {
                [
                    point[0] / surface.local_rect.width as f32,
                    point[1] / surface.local_rect.height as f32,
                ]
            })
            .collect::<Vec<_>>();
        let spec = GlassSurfaceSpec {
            surface_id: surface.surface_id.clone(),
            shape: surface.shape,
            radius: normalized_radius as f32,
        };
        let mut entries = Vec::with_capacity(frames.len());
        for (time, frame) in frames {
            let sampled = frame
                .surfaces
                .iter()
                .find(|candidate| candidate.surface_id == surface.surface_id)
                .ok_or_else(|| ProgramPrepareError::MotionGlass {
                    reason: format!(
                        "Glass surface '{}' disappeared from fixed layout topology",
                        surface.surface_id
                    ),
                })?;
            let transform = super::DeviceTransform::from_projective(matrix_mul(
                viewport_to_device,
                sampled.viewport_matrix,
            ))
            .map_err(|error| ProgramPrepareError::MotionGlass {
                reason: error.to_string(),
            })?;
            entries.push((
                GlassSurfaceTrackSample {
                    surface_id: valle_motion::GlassSurfaceId::new(surface.surface_id.clone())
                        .map_err(|error| ProgramPrepareError::MotionGlass {
                            reason: error.to_string(),
                        })?,
                    instance: instance.clone(),
                    epoch: epoch.clone(),
                    time: *time,
                    local: valle_motion::glass::GlassLocalShape {
                        rect: sampled.local_rect,
                        presence: sampled.presence,
                        intensity: sampled.motion.intensity,
                        drive_translation: valle_draw::Point::new(
                            sampled.motion.drive[0],
                            sampled.motion.drive[1],
                        ),
                        drive_pressure: sampled.motion.drive[2],
                        drive_twist: sampled.motion.drive[3],
                    },
                },
                transform,
            ));
        }
        let shapes = qualify_device_track(&spec, &entries).map_err(|error| {
            ProgramPrepareError::MotionGlass {
                reason: format!("Glass surface '{}': {error}", surface.surface_id),
            }
        })?;
        qualified.insert(
            surface.surface_id.clone(),
            QualifiedSurface {
                spec,
                shapes,
                path_points: normalized_path,
            },
        );
    }

    let mut material_programs = BTreeMap::new();
    let mut foreground_programs = BTreeMap::new();
    for surface in current
        .surfaces
        .iter()
        .filter(|surface| surface.field_id.is_none())
    {
        let material = surface
            .material
            .ok_or_else(|| ProgramPrepareError::MotionGlass {
                reason: format!("independent Glass '{}' has no material", surface.surface_id),
            })?;
        let qualified_surface = &qualified[&surface.surface_id];
        let last = qualified_surface
            .shapes
            .last()
            .expect("current Glass frame produces one qualified sample");
        let radius =
            f64::from(last.radius).max((last.rect.width.min(last.rect.height) * 0.5).max(1.0));
        let resolved = resolve_material_base_with_geometry(
            material.clarity,
            material.depth,
            linear_tint(material.tint)?,
            1.0,
            1.0,
            radius,
        )
        .map_err(|reason| ProgramPrepareError::MotionGlass {
            reason: format!("Glass material '{}': {reason}", surface.surface_id),
        })?;
        let mut program =
            crate::compositor::glass::assemble_independent_glass(&IndependentGlassPackage {
                spec: &qualified_surface.spec,
                shapes: &qualified_surface.shapes,
                material: resolved,
                character: character_kernel(surface.motion.character),
                settle: surface.motion.settle_seconds,
                path_points: qualified_surface.path_points.clone(),
            })
            .map_err(|error| ProgramPrepareError::MotionGlass {
                reason: error.to_string(),
            })?;
        qualify_program_geometry(
            &mut program,
            std::slice::from_ref(surface),
            surface.owner_to_viewport,
            viewport_to_device,
        )?;
        qualify_program_light(
            &mut program,
            surface
                .environment
                .as_ref()
                .ok_or_else(|| ProgramPrepareError::MotionGlass {
                    reason: format!(
                        "independent Glass '{}' has no environment",
                        surface.surface_id
                    ),
                })?,
            surface.owner_to_viewport,
            viewport_to_device,
        )?;
        let foreground = valle_draw::program::MotionGlassForegroundProgram::from_owner(
            &program,
            &surface.surface_id,
        )
        .map_err(|error| ProgramPrepareError::MotionGlass {
            reason: error.to_string(),
        })?;
        foreground_programs.insert(surface.node_key.clone(), foreground);
        material_programs.insert(surface.surface_id.clone(), program);
    }

    for field in &current.fields {
        let members = field
            .member_surface_ids
            .iter()
            .map(|id| {
                let member = &qualified[id];
                FieldMemberPackage {
                    spec: &member.spec,
                    shapes: &member.shapes,
                    path_points: &member.path_points,
                }
            })
            .collect::<Vec<_>>();
        let radius = members
            .iter()
            .filter_map(|member| member.shapes.last())
            .map(|shape| {
                f64::from(shape.radius)
                    .max((shape.rect.width.min(shape.rect.height) * 0.5).max(1.0))
            })
            .fold(f64::INFINITY, f64::min);
        let resolved = resolve_material_base_with_geometry(
            field.material.clarity,
            field.material.depth,
            linear_tint(field.material.tint)?,
            1.0,
            1.0,
            radius,
        )
        .map_err(|reason| ProgramPrepareError::MotionGlass {
            reason: format!("GlassField material '{}': {reason}", field.field_id),
        })?;
        let mut program = crate::compositor::glass::assemble_field_glass(&FieldGlassPackage {
            field_id: &field.field_id,
            members: &members,
            material: resolved,
            character: character_kernel(field.motion.character),
            settle: field.motion.settle_seconds,
            merge_distance: field.merge_distance,
        })
        .map_err(|error| ProgramPrepareError::MotionGlass {
            reason: error.to_string(),
        })?;
        let current_members = current
            .surfaces
            .iter()
            .filter(|surface| surface.field_id.as_deref() == Some(field.field_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        qualify_program_geometry(
            &mut program,
            &current_members,
            field.owner_to_viewport,
            viewport_to_device,
        )?;
        qualify_program_light(
            &mut program,
            &field.environment,
            field.owner_to_viewport,
            viewport_to_device,
        )?;
        for surface in &current_members {
            let foreground = valle_draw::program::MotionGlassForegroundProgram::from_owner(
                &program,
                &surface.surface_id,
            )
            .map_err(|error| ProgramPrepareError::MotionGlass {
                reason: error.to_string(),
            })?;
            foreground_programs.insert(surface.node_key.clone(), foreground);
        }
        material_programs.insert(field.field_id.clone(), program);
    }
    tree.glass.material_programs = material_programs;
    tree.glass.foreground_programs = foreground_programs;
    Ok(())
}

fn character_kernel(
    character: valle_motion::glass::GlassCharacter,
) -> crate::compositor::glass::CharacterKernel {
    match character {
        valle_motion::glass::GlassCharacter::Responsive => {
            crate::compositor::glass::CharacterKernel::Responsive
        }
        valle_motion::glass::GlassCharacter::Fluid => {
            crate::compositor::glass::CharacterKernel::Fluid
        }
        valle_motion::glass::GlassCharacter::Viscous => {
            crate::compositor::glass::CharacterKernel::Viscous
        }
        valle_motion::glass::GlassCharacter::Elastic => {
            crate::compositor::glass::CharacterKernel::Elastic
        }
    }
}

fn foreground_tone(
    tone: valle_motion::glass::GlassForegroundTone,
) -> valle_draw::program::PackedGlassForegroundTone {
    match tone {
        valle_motion::glass::GlassForegroundTone::Auto => {
            valle_draw::program::PackedGlassForegroundTone::Auto
        }
        valle_motion::glass::GlassForegroundTone::Light => {
            valle_draw::program::PackedGlassForegroundTone::Light
        }
        valle_motion::glass::GlassForegroundTone::Dark => {
            valle_draw::program::PackedGlassForegroundTone::Dark
        }
        valle_motion::glass::GlassForegroundTone::None => {
            valle_draw::program::PackedGlassForegroundTone::None
        }
    }
}

fn linear_tint(color: valle_draw::Rgba) -> Result<[f32; 4], ProgramPrepareError> {
    let pixel =
        crate::compositor::reference::decode_author_srgb(crate::resource::AuthorSrgbStraight([
            color.r, color.g, color.b, color.a,
        ]))
        .map_err(|error| ProgramPrepareError::MotionGlass {
            reason: error.to_string(),
        })?;
    let [r, g, b, a] = pixel.channels();
    Ok(if a > 0.0 {
        [r / a, g / a, b / a, a]
    } else {
        [0.0, 0.0, 0.0, 0.0]
    })
}

fn qualify_program_light(
    program: &mut valle_draw::program::MotionGlassProgram,
    environment: &valle_motion::GlassLayoutEnvironment,
    owner_to_viewport: [f64; 9],
    viewport_to_device: [f64; 9],
) -> Result<(), ProgramPrepareError> {
    let authored = environment.light_direction;
    let authored_length = authored[0].hypot(authored[1]);
    if !authored_length.is_finite()
        || authored_length <= f64::EPSILON
        || !environment.light_elevation.is_finite()
        || !(0.0..=1.0).contains(&environment.light_elevation)
        || !environment.light_intensity.is_finite()
        || !(0.0..=1.0).contains(&environment.light_intensity)
    {
        return Err(ProgramPrepareError::MotionGlass {
            reason: format!("Glass owner '{}' has an invalid light", program.owner_id),
        });
    }
    let direction = match environment.light_space {
        valle_motion::glass::GlassLightSpace::Screen => {
            [authored[0] / authored_length, authored[1] / authored_length]
        }
        valle_motion::glass::GlassLightSpace::World => {
            let owner_to_device = matrix_mul(viewport_to_device, owner_to_viewport);
            let center = program.backdrop.output_bounds.center();
            let origin = project(owner_to_device, [center.x, center.y]).ok_or_else(|| {
                ProgramPrepareError::MotionGlass {
                    reason: format!(
                        "Glass owner '{}' light crosses a transform horizon",
                        program.owner_id
                    ),
                }
            })?;
            let endpoint = project(
                owner_to_device,
                [center.x + authored[0], center.y + authored[1]],
            )
            .ok_or_else(|| ProgramPrepareError::MotionGlass {
                reason: format!(
                    "Glass owner '{}' light crosses a transform horizon",
                    program.owner_id
                ),
            })?;
            let transformed = [endpoint[0] - origin[0], endpoint[1] - origin[1]];
            let length = transformed[0].hypot(transformed[1]);
            if !length.is_finite() || length <= f64::EPSILON {
                return Err(ProgramPrepareError::MotionGlass {
                    reason: format!(
                        "Glass owner '{}' world light collapses in device space",
                        program.owner_id
                    ),
                });
            }
            [transformed[0] / length, transformed[1] / length]
        }
    };
    program.material.light = valle_draw::program::PackedGlassLight {
        direction: [direction[0] as f32, direction[1] as f32],
        elevation: environment.light_elevation as f32,
        intensity: environment.light_intensity as f32,
    };
    program
        .validate()
        .map_err(|error| ProgramPrepareError::MotionGlass {
            reason: error.to_string(),
        })
}

fn qualify_program_geometry(
    program: &mut valle_draw::program::MotionGlassProgram,
    surfaces: &[valle_motion::GlassLayoutSurface],
    owner_to_viewport: [f64; 9],
    viewport_to_device: [f64; 9],
) -> Result<(), ProgramPrepareError> {
    let viewport_to_owner =
        matrix_inverse(owner_to_viewport).ok_or_else(|| ProgramPrepareError::MotionGlass {
            reason: format!(
                "Glass owner '{}' has a degenerate transform",
                program.owner_id
            ),
        })?;
    let owner_to_device = matrix_mul(viewport_to_device, owner_to_viewport);
    for packed in &mut program.surfaces {
        let surface = surfaces
            .iter()
            .find(|surface| surface.surface_id == packed.surface_id)
            .ok_or_else(|| ProgramPrepareError::MotionGlass {
                reason: format!(
                    "Glass owner '{}' lost surface '{}'",
                    program.owner_id, packed.surface_id
                ),
            })?;
        // `viewport_matrix` maps the normalized box to the viewport, but the analytic Glass
        // shapes must stay in layout-pixel local coordinates. Keeping a scalar radius in the
        // normalized 1x1 box turns every circular corner into an ellipse whenever width !=
        // height (and turns a capsule into a flattened rounded rectangle). Convert local pixels
        // to normalized coordinates in the matrix instead, so rect, radius and path geometry all
        // share one isotropic authoring space before the actual CSS transform is applied.
        let local_pixels_to_normalized = [
            1.0 / surface.local_rect.width,
            0.0,
            0.0,
            0.0,
            1.0 / surface.local_rect.height,
            0.0,
            0.0,
            0.0,
            1.0,
        ];
        let local_to_owner = matrix_mul(
            matrix_mul(viewport_to_owner, surface.viewport_matrix),
            local_pixels_to_normalized,
        );
        packed.rect = Rect::new(
            0.0,
            0.0,
            surface.local_rect.width,
            surface.local_rect.height,
        );
        packed.local_to_owner = matrix_f32(local_to_owner)?;
        let maximum_radius = surface.local_rect.width.min(surface.local_rect.height) * 0.5;
        packed.radius = f64::from(surface.radius).clamp(0.0, maximum_radius) as f32;
        packed.path_points = surface.path_points.clone();
        packed.foreground_tone = foreground_tone(surface.foreground.tone);
        let protects_foreground = !matches!(
            surface.foreground.tone,
            valle_motion::glass::GlassForegroundTone::None
        ) && surface.foreground.protection > 0.0
            && surface.foreground.bounds.is_some();
        packed.foreground_protection = if protects_foreground {
            surface.foreground.protection as f32
        } else {
            0.0
        };
        packed.foreground_bounds = protects_foreground
            .then_some(surface.foreground.bounds)
            .flatten();
        packed.foreground_luma = protects_foreground
            .then_some(surface.foreground.luma)
            .flatten();
    }
    let bounds =
        crate::compositor::glass::resolve_motion_glass_backdrop_bounds(program, owner_to_device)
            .map_err(|error| ProgramPrepareError::MotionGlass {
                reason: error.to_string(),
            })?;
    program.backdrop.output_bounds = bounds.output_owner;
    program.backdrop.sample_bounds = bounds.sample_owner;
    program
        .validate()
        .map_err(|error| ProgramPrepareError::MotionGlass {
            reason: error.to_string(),
        })
}

fn matrix_mul(left: [f64; 9], right: [f64; 9]) -> [f64; 9] {
    let mut output = [0.0; 9];
    for row in 0..3 {
        for column in 0..3 {
            output[row * 3 + column] = (0..3)
                .map(|index| left[row * 3 + index] * right[index * 3 + column])
                .sum();
        }
    }
    output
}

fn matrix_inverse(matrix: [f64; 9]) -> Option<[f64; 9]> {
    let [a, b, c, d, e, f, g, h, i] = matrix;
    let cofactors = [
        e * i - f * h,
        c * h - b * i,
        b * f - c * e,
        f * g - d * i,
        a * i - c * g,
        c * d - a * f,
        d * h - e * g,
        b * g - a * h,
        a * e - b * d,
    ];
    let determinant = a * cofactors[0] + b * cofactors[3] + c * cofactors[6];
    (determinant.is_finite() && determinant.abs() > 1e-12)
        .then(|| cofactors.map(|value| value / determinant))
}

fn matrix_f32(matrix: [f64; 9]) -> Result<[f32; 9], ProgramPrepareError> {
    let output = matrix.map(|value| value as f32);
    if output.into_iter().all(f32::is_finite) {
        Ok(output)
    } else {
        Err(ProgramPrepareError::MotionGlass {
            reason: "Glass transform exceeds packed f32 range".into(),
        })
    }
}

fn project(matrix: [f64; 9], point: [f64; 2]) -> Option<[f64; 2]> {
    let denominator = matrix[6] * point[0] + matrix[7] * point[1] + matrix[8];
    if !denominator.is_finite() || denominator.abs() <= 1e-12 {
        return None;
    }
    let output = [
        (matrix[0] * point[0] + matrix[1] * point[1] + matrix[2]) / denominator,
        (matrix[3] * point[0] + matrix[4] * point[1] + matrix[5]) / denominator,
    ];
    output.into_iter().all(f64::is_finite).then_some(output)
}

#[derive(Debug, Clone)]
pub struct FixtureProgram {
    program: DrawProgram,
    assets: BTreeMap<String, SemanticAsset>,
    fonts: Vec<SemanticFont>,
    structures: BTreeMap<String, SemanticStructure>,
}

impl FixtureProgram {
    pub fn new(program: DrawProgram) -> Self {
        Self {
            program,
            assets: BTreeMap::new(),
            fonts: Vec::new(),
            structures: BTreeMap::new(),
        }
    }

    /// Creates the deterministic empty DrawProgram used by C1 reference hosts that must not own
    /// or depend on DrawProgram construction policy themselves.
    pub fn empty(viewport: Extent2d) -> Self {
        let program = DrawProgramBuilder::new(Rect::new(
            0.0,
            0.0,
            f64::from(viewport.width()),
            f64::from(viewport.height()),
        ))
        .finish()
        .expect("a non-empty viewport always forms a valid empty fixture program");
        Self::new(program)
    }

    pub fn with_asset(mut self, name: impl Into<String>, asset: SemanticAsset) -> Self {
        self.assets.insert(name.into(), asset);
        self
    }

    pub fn with_font(mut self, font: SemanticFont) -> Self {
        self.fonts.push(font);
        self
    }

    pub fn with_structure(mut self, structure: SemanticStructure) -> Self {
        self.structures.insert(structure.key.clone(), structure);
        self
    }

    pub fn program(&self) -> &DrawProgram {
        &self.program
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct ProgramId(u32);

impl ProgramId {
    pub(crate) fn new(value: u32) -> Result<Self, ProgramPrepareError> {
        if value == 0 {
            Err(ProgramPrepareError::ProgramBudgetExceeded)
        } else {
            Ok(Self(value))
        }
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for ProgramId {
    type Error = ProgramPrepareError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ProgramId> for u32 {
    fn from(value: ProgramId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreparedProgramKind {
    Motion,
    Caption,
    Solid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramTextureBinding {
    pub key: String,
    pub handle: ExternalHandleId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramFontBinding {
    pub face_hash: ContentDigest,
    pub face_index: u32,
    pub handle: ExternalHandleId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramStructureBinding {
    pub key: String,
    pub handle: ExternalHandleId,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgramResourceBindings {
    pub textures: Vec<ProgramTextureBinding>,
    pub fonts: Vec<ProgramFontBinding>,
    pub runtime_shaders: Vec<ProgramStructureBinding>,
    pub scenes: Vec<ProgramStructureBinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedDestinationUse {
    pub node: NodeId,
    pub scope: BackdropScope,
    pub sample_bounds: DynamicBindingId,
    pub output_bounds: DynamicBindingId,
    pub operation: DestinationOperation,
    pub bounds_reason: BoundsReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedProgram {
    pub id: ProgramId,
    pub kind: PreparedProgramKind,
    pub semantic_path: String,
    pub content_hash: ContentDigest,
    pub viewport: Rect,
    pub packed: Vec<u8>,
    pub requirements: valle_draw::requirements::DrawRequirements,
    pub resources: ProgramResourceBindings,
    pub destination_uses: Vec<PreparedDestinationUse>,
    /// Producer-owned validated arena. It is never serialized; decoded PreparedFrame packets
    /// leave this empty and must pass packed DrawProgram admission before lowering.
    #[serde(skip)]
    runtime_program: Option<Arc<DrawProgram>>,
}

impl PreparedProgram {
    pub(crate) fn runtime_program(&self) -> Option<&DrawProgram> {
        self.runtime_program.as_deref()
    }
}

impl PartialEq for PreparedProgram {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.kind == other.kind
            && self.semantic_path == other.semantic_path
            && self.content_hash == other.content_hash
            && self.viewport == other.viewport
            && self.packed == other.packed
            && self.requirements == other.requirements
            && self.resources == other.resources
            && self.destination_uses == other.destination_uses
    }
}

pub(crate) struct ProgramPrepareContext<'a> {
    pub kind: PreparedProgramKind,
    pub semantic_path: &'a str,
    pub fixture: &'a FixtureProgram,
    pub caption_fonts: &'a [SemanticFont],
}

pub(crate) fn prepare_program(
    id: ProgramId,
    context: ProgramPrepareContext<'_>,
    requests: &mut RequestAllocator,
) -> Result<PreparedProgram, ProgramPrepareError> {
    context.fixture.program.validate()?;
    let requirements = context.fixture.program.requirements().clone();
    let mut bindings = ProgramResourceBindings::default();

    for texture in &requirements.external_textures {
        let asset = context.fixture.assets.get(&texture.key).ok_or_else(|| {
            ProgramPrepareError::MissingTexture {
                key: texture.key.clone(),
            }
        })?;
        let kind_matches = match texture.kind {
            TextureKind::Video => asset.kind == SemanticAssetKind::Video,
            TextureKind::Image => {
                matches!(
                    asset.kind,
                    SemanticAssetKind::Image | SemanticAssetKind::Lottie
                )
            }
            TextureKind::Generated => matches!(
                asset.kind,
                SemanticAssetKind::Image | SemanticAssetKind::Lottie
            ),
        };
        if !kind_matches {
            return Err(ProgramPrepareError::TextureKindMismatch {
                key: texture.key.clone(),
            });
        }
        let sample = if texture.kind == TextureKind::Video {
            let micros = texture.sample_time_micros.ok_or_else(|| {
                ProgramPrepareError::MissingVideoSampleTime {
                    key: texture.key.clone(),
                }
            })?;
            // DrawProgram's external-texture ABI freezes Motion's node-local
            // `sourceStart + sourceFrame / fps * speed` clock in integer microseconds. Bind that
            // exact producer timestamp; the outer Motion prop HoldEnd sentinel is unrelated.
            ResourceSample::SourceTime(
                RationalTime::new(micros, 1_000_000)
                    .expect("the DrawProgram microsecond clock has a fixed non-zero denominator"),
            )
        } else {
            ResourceSample::Static
        };
        let path = format!("{}.texture[{}]", context.semantic_path, texture.key);
        let handle = requests.asset(asset, sample, &path)?;
        bindings.textures.push(ProgramTextureBinding {
            key: texture.key.clone(),
            handle,
        });
    }

    for font in &requirements.fonts {
        let semantic =
            find_font(font, context.caption_fonts, &context.fixture.fonts).ok_or_else(|| {
                ProgramPrepareError::MissingFont {
                    face_hash: font.face_hash.as_hex(),
                    face_index: font.face_index,
                }
            })?;
        let path = format!(
            "{}.font[{}:{}]",
            context.semantic_path, semantic.digest, font.face_index
        );
        let handle = requests.font(semantic.digest.clone(), semantic.face_index, &path)?;
        bindings.fonts.push(ProgramFontBinding {
            face_hash: semantic.digest,
            face_index: font.face_index,
            handle,
        });
    }

    for shader in &requirements.runtime_shaders {
        let structure = context.fixture.structures.get(&shader.uri).ok_or_else(|| {
            ProgramPrepareError::MissingStructure {
                key: shader.uri.clone(),
            }
        })?;
        validate_shader(shader, structure)?;
        let StructureDescriptor::RuntimeShader {
            abi_digest,
            color_domain,
            alpha_behavior,
            ..
        } = &structure.descriptor
        else {
            return Err(ProgramPrepareError::StructureKindMismatch {
                key: shader.uri.clone(),
            });
        };
        let path = format!("{}.shader[{}]", context.semantic_path, shader.uri);
        let handle = requests.runtime_shader(
            structure.digest.clone(),
            abi_digest.clone(),
            *color_domain,
            *alpha_behavior,
            &path,
        )?;
        bindings.runtime_shaders.push(ProgramStructureBinding {
            key: shader.uri.clone(),
            handle,
        });
    }

    for scene in &requirements.scene3d {
        let scene_digest = ContentDigest::from_bytes(scene.content_hash.into_bytes());
        let structure = context
            .fixture
            .structures
            .values()
            .find(|candidate| candidate.digest == scene_digest)
            .ok_or_else(|| ProgramPrepareError::MissingStructure {
                key: scene.content_hash.as_hex(),
            })?;
        let StructureDescriptor::Scene3d {
            topology_digest, ..
        } = &structure.descriptor
        else {
            return Err(ProgramPrepareError::StructureKindMismatch {
                key: structure.key.clone(),
            });
        };
        validate_scene(scene, structure)?;
        let path = format!("{}.scene3d[{}]", context.semantic_path, structure.key);
        let handle = requests.scene3d(
            structure.digest.clone(),
            topology_digest.clone(),
            b"null".to_vec(),
            &path,
        )?;
        bindings.scenes.push(ProgramStructureBinding {
            key: structure.key.clone(),
            handle,
        });
    }

    let packed = context.fixture.program.packed_bytes()?;
    let content_hash = ContentDigest::of_bytes(&packed);
    Ok(PreparedProgram {
        id,
        kind: context.kind,
        semantic_path: context.semantic_path.to_owned(),
        content_hash,
        viewport: context.fixture.program.viewport(),
        packed,
        requirements,
        resources: bindings,
        destination_uses: Vec::new(),
        runtime_program: Some(Arc::new(context.fixture.program.clone())),
    })
}

fn find_font<'a>(
    requirement: &FontKey,
    semantic: &'a [SemanticFont],
    fixtures: &'a [SemanticFont],
) -> Option<&'a SemanticFont> {
    let digest = ContentDigest::from_bytes(requirement.face_hash.into_bytes());
    semantic
        .iter()
        .chain(fixtures)
        .find(|font| font.digest == digest && font.face_index == requirement.face_index)
}

fn validate_shader(
    requirement: &RuntimeShaderKey,
    structure: &SemanticStructure,
) -> Result<(), ProgramPrepareError> {
    let StructureDescriptor::RuntimeShader { abi_digest, .. } = &structure.descriptor else {
        return Err(ProgramPrepareError::StructureKindMismatch {
            key: structure.key.clone(),
        });
    };
    let content_digest = ContentDigest::from_bytes(requirement.content_hash.into_bytes());
    let required_abi = ContentDigest::from_bytes(requirement.abi_hash.into_bytes());
    if structure.digest != content_digest || *abi_digest != required_abi {
        return Err(ProgramPrepareError::StructureDigestMismatch {
            key: structure.key.clone(),
        });
    }
    Ok(())
}

fn validate_scene(
    requirement: &Scene3dKey,
    structure: &SemanticStructure,
) -> Result<(), ProgramPrepareError> {
    let StructureDescriptor::Scene3d {
        topology_digest, ..
    } = &structure.descriptor
    else {
        return Err(ProgramPrepareError::StructureKindMismatch {
            key: structure.key.clone(),
        });
    };
    let content_digest = ContentDigest::from_bytes(requirement.content_hash.into_bytes());
    let topology_digest_from_requirement =
        ContentDigest::from_bytes(requirement.topology_hash.into_bytes());
    if content_digest == structure.digest && topology_digest_from_requirement == *topology_digest {
        Ok(())
    } else {
        Err(ProgramPrepareError::StructureDigestMismatch {
            key: structure.key.clone(),
        })
    }
}

#[derive(Debug, Error)]
pub enum ProgramPrepareError {
    #[error("prepared program id budget exceeded")]
    ProgramBudgetExceeded,
    #[error("missing frame-local Scene3D request {content_hash}")]
    MissingScene3dFrame { content_hash: String },
    #[error("invalid frame-local Scene3D request: {reason}")]
    Scene3dFrame { reason: String },
    #[error("Motion DrawProgram producer received a non-Motion visual source")]
    NotMotionSource,
    #[error("Motion props are invalid: {reason}")]
    MotionProps { reason: String },
    #[error("compiled Motion invariant failed: {reason}")]
    CompiledInvariant { reason: String },
    #[error("Motion signals are invalid: {reason}")]
    MotionSignals { reason: String },
    #[error("Motion layout failed: {reason}")]
    MotionLayout { reason: String },
    #[error("Motion Glass preparation failed: {reason}")]
    MotionGlass { reason: String },
    #[error("Motion DrawProgram emission failed: {reason}")]
    MotionEmit { reason: String },
    #[error("Motion paint output contains unsupported features: {features:?}")]
    UnsupportedMotionPaint { features: Vec<String> },
    #[error("DrawProgram external texture {key:?} has no frozen semantic binding")]
    MissingTexture { key: String },
    #[error("DrawProgram texture {key:?} kind does not match its frozen semantic asset")]
    TextureKindMismatch { key: String },
    #[error("DrawProgram video texture {key:?} has no frozen source timestamp")]
    MissingVideoSampleTime { key: String },
    #[error("DrawProgram generated texture {key:?} lacks a typed Scene3D resource contract")]
    UntypedGeneratedTexture { key: String },
    #[error("DrawProgram font {face_hash}:{face_index} has no frozen semantic face")]
    MissingFont { face_hash: String, face_index: u32 },
    #[error("DrawProgram structure {key:?} has no frozen semantic binding")]
    MissingStructure { key: String },
    #[error("DrawProgram structure {key:?} has the wrong semantic kind")]
    StructureKindMismatch { key: String },
    #[error("DrawProgram structure {key:?} digest or ABI does not match")]
    StructureDigestMismatch { key: String },
    #[error(transparent)]
    Program(#[from] valle_draw::program::DrawProgramError),
    #[error(transparent)]
    Packed(#[from] valle_draw::program::PackedDrawError),
    #[error(transparent)]
    Request(#[from] RequestError),
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEPENDENCY_FONT: &[u8] =
        include_bytes!("../../../valle-motion/assets/fonts/katex/KaTeX_AMS-Regular.ttf");

    fn emitted_font_request(
        fonts: &valle_motion::Fonts,
        family: &str,
    ) -> valle_draw::requirements::FontKey {
        let source = format!(
            r##"
export default function Card() {{
  return (
    <Scene style={{{{ width: 320, height: 180 }}}}>
      <Text style={{{{ fontFamily: {family:?}, fontSize: 32 }}}}>VALLE</Text>
    </Scene>
  );
}}
"##
        );
        let compiled = valle_compiler::motion::compile_motion(&source)
            .unwrap_or_else(|diagnostics| panic!("font fixture must compile: {diagnostics:#?}"));
        let prepared = valle_motion::prepare_scene(&compiled.artifact).expect("prepare font scene");
        let props = valle_motion::resolve_props(&compiled.artifact.controls, &BTreeMap::new())
            .expect("resolve font fixture props");
        let windows = valle_motion::phase_windows(&compiled.artifact.controls.phase_spec(), 30);
        let context = valle_motion::motion_context_at(0, &windows, FrameRate::new(30, 1).unwrap())
            .expect("font fixture frame zero");
        let tree = valle_motion::build_tree(
            &prepared,
            &context,
            &props,
            &valle_motion::ResolvedSignals::default(),
            &valle_motion::LayoutOptions {
                viewport: valle_motion::Viewport::new((320, 180)),
                fonts,
                styles: None,
            },
        )
        .expect("layout with dependency font family");
        let report = valle_motion::emit(&tree, &valle_motion::default_font_naming)
            .expect("emit dependency font glyphs");
        assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
        assert!(
            report
                .program
                .nodes()
                .iter()
                .any(|node| matches!(node, valle_draw::program::Node::GlyphRun(_))),
            "selected dependency font must emit glyphs"
        );
        assert_eq!(report.program.requirements().fonts.len(), 1);
        report.program.requirements().fonts[0].clone()
    }

    #[test]
    fn dependency_font_keeps_internal_family_and_content_addressed_alias() {
        assert!(
            valle_motion::DEFAULT_MOTION_FONT_WEIGHTS
                .iter()
                .all(|default| *default != DEPENDENCY_FONT),
            "fixture must exercise the non-default dependency path"
        );
        let digest = ContentDigest::of_bytes(DEPENDENCY_FONT);
        let alias = valle_motion::font_family_alias(&digest);
        let mut family_probe = valle_motion::Fonts::default();
        let internal_family = family_probe
            .register(valle_motion::FontResource::new(DEPENDENCY_FONT.to_vec()))
            .expect("probe dependency font name table")
            .into_iter()
            .next()
            .expect("dependency font declares an internal family")
            .name;

        let mut fonts = valle_motion::Fonts::default();
        valle_motion::register_default_motion_fonts(&mut fonts).expect("register default fonts");
        register_motion_dependency_font(&mut fonts, DEPENDENCY_FONT, &digest)
            .expect("register dependency font under both families");

        let internal_request = emitted_font_request(&fonts, &internal_family);
        let alias_request = emitted_font_request(&fonts, &alias);
        assert_eq!(internal_request, alias_request);
        assert_eq!(internal_request.face_hash.into_bytes(), *digest.as_bytes());
        assert_eq!(internal_request.face_index, 0);
    }
}
