//! Construction-only lowering from the Motion recorder into structured DrawProgram.
//!
//! This module is intentionally not a wire compatibility surface. It exists only while the Motion
//! producer is being moved to the structured builder in the Product Compositor module; its output
//! is always the current DrawProgram contract and no executor can consume the recording itself.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::requirements::{
    AlphaMode, ColorDomain, DigestBytes, ExternalTexture, FontKey, Insets, RuntimeShaderKey,
    SamplingMode, Scene3dKey, TextureKind,
};
use crate::{Rect, program::recording};

use super::{
    Affine2d, BackdropRead, BackdropScope, BatchGeometry, BatchInstance, BlendMode, Clip,
    DrawProgram, DrawProgramBuilder, DrawProgramError, FillRule, Filter, GeometryBatchNode, Glyph,
    GlyphRun, Group, ImageNode, LinearColor, Mask, MaskMode, Node, NodeId, Paint, PaintId,
    PathData, PathId, PathNode, PathStroke, RoundRect, Scene3dNode, ShaderLayer,
    ShaderTextureBinding, ShaderUniformBinding, ShaderUniformValue, ShadowNode, SpreadMode,
    StrokeCap, StrokeJoin, Transform2d,
};

#[derive(Debug, Error)]
pub enum ProgramRecordingError {
    #[error("invalid Motion recording: {0}")]
    InvalidRecording(String),
    #[error("recording command {command} references an invalid {table} entry {index}")]
    InvalidReference {
        command: usize,
        table: &'static str,
        index: u32,
    },
    #[error("recording command {command} closes the root frame")]
    UnexpectedEnd { command: usize },
    #[error("recording has {depth} unclosed structural groups")]
    UnclosedGroups { depth: usize },
    #[error("recording texture {key:?} has no frozen interpreted-content extent")]
    MissingTextureDescriptor { key: String },
    #[error("recording font family {family:?} does not carry a real font content digest")]
    MissingFontContentDigest { family: String },
    #[error(transparent)]
    Program(#[from] DrawProgramError),
}

/// Compile a construction recording into the only executable local IR.
pub fn compile_recording(
    viewport: Rect,
    recording: &recording::ProgramRecording,
) -> Result<DrawProgram, ProgramRecordingError> {
    recording
        .validate()
        .map_err(|error| ProgramRecordingError::InvalidRecording(format!("{error:?}")))?;
    RecordingCompiler::new(viewport, recording, None).compile()
}

/// Interpreted display-content dimensions used to resolve CSS object-fit before DrawProgram is
/// frozen. Orientation, sample aspect ratio and descriptor crop must already be reflected here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProgramTextureExtent {
    pub width: f64,
    pub height: f64,
}

/// Immutable semantic descriptor lookup supplied by a real producer. It contains no decoded
/// pixels or platform object.
pub trait ProgramResourceCatalog {
    fn texture_extent(&self, key: &str) -> Option<ProgramTextureExtent>;

    /// Content digest bound to a Motion resource control used by Scene3D. The digest, rather than
    /// a host path or decoded object, becomes part of the frame request identity.
    fn scene3d_resource_digest(&self, _control: &str) -> Option<[u8; 32]> {
        None
    }

    /// Resolve a construction-only generated texture into a typed Scene3D frame. Product Motion
    /// emission supplies this map; other generated keys remain untyped and are rejected later.
    fn scene3d_key(&self, _key: &str) -> Option<Scene3dKey> {
        None
    }
}

impl<F> ProgramResourceCatalog for F
where
    F: Fn(&str) -> Option<ProgramTextureExtent>,
{
    fn texture_extent(&self, key: &str) -> Option<ProgramTextureExtent> {
        self(key)
    }
}

/// Resolves object-fit using intrinsic image dimensions from the immutable semantic descriptor.
pub fn compile_recording_with_catalog(
    viewport: Rect,
    recording: &recording::ProgramRecording,
    catalog: &dyn ProgramResourceCatalog,
) -> Result<DrawProgram, ProgramRecordingError> {
    recording
        .validate()
        .map_err(|error| ProgramRecordingError::InvalidRecording(format!("{error:?}")))?;
    RecordingCompiler::new(viewport, recording, Some(catalog)).compile()
}

// These folds transfer fields without multiplying transforms or opacity. A clip remains
// inside its parent's opacity layer; arbitrary effect order and clip intersections are not folded.
fn merge_recording_groups(outer: &Group, inner: &Group) -> Option<Group> {
    if outer.is_plain() {
        return Some(inner.clone());
    }
    if outer.is_transform_only()
        && !outer.isolated
        && outer.layer_bounds.is_none()
        && inner.transform == Transform2d::IDENTITY
    {
        let mut merged = inner.clone();
        merged.transform = outer.transform;
        return Some(merged);
    }
    if outer.is_raster_group()
        && outer.transform == Transform2d::IDENTITY
        && inner.is_raster_tree_group()
        && inner.clip.is_some()
        && inner.transform == Transform2d::IDENTITY
        && inner.opacity == 1.0
        && !inner.isolated
        && inner.layer_bounds.is_none()
    {
        let mut merged = outer.clone();
        merged.clip = inner.clip.clone();
        merged.children = inner.children.clone();
        return Some(merged);
    }
    None
}

struct RecordingFrame {
    group: Group,
    alpha_mask: bool,
}

struct RecordingCompiler<'a> {
    source: &'a recording::ProgramRecording,
    viewport: Rect,
    builder: DrawProgramBuilder,
    roots: Vec<NodeId>,
    stack: Vec<RecordingFrame>,
    paths: BTreeMap<(u32, u32, u32, u32), PathId>,
    paints: Vec<(Paint, PaintId)>,
    catalog: Option<&'a dyn ProgramResourceCatalog>,
}

impl<'a> RecordingCompiler<'a> {
    fn new(
        viewport: Rect,
        source: &'a recording::ProgramRecording,
        catalog: Option<&'a dyn ProgramResourceCatalog>,
    ) -> Self {
        Self {
            source,
            viewport,
            builder: DrawProgramBuilder::new(viewport),
            roots: Vec::new(),
            stack: Vec::new(),
            paths: BTreeMap::new(),
            paints: Vec::new(),
            catalog,
        }
    }

    fn compile(mut self) -> Result<DrawProgram, ProgramRecordingError> {
        for (command_index, command) in self.source.cmds.iter().enumerate() {
            self.command(command_index, command)?;
        }
        if !self.stack.is_empty() {
            return Err(ProgramRecordingError::UnclosedGroups {
                depth: self.stack.len(),
            });
        }
        for root in self.roots {
            self.builder.add_root(root);
        }
        self.builder.finish().map_err(Into::into)
    }

    fn command(
        &mut self,
        command_index: usize,
        command: &recording::RecordCmd,
    ) -> Result<(), ProgramRecordingError> {
        use recording::RecordCmd;
        match command {
            RecordCmd::BeginGroup => self.begin(Group::plain(Vec::new())),
            RecordCmd::BeginAlphaMask => {
                self.begin(Group::plain(Vec::new()));
                self.stack.last_mut().unwrap().alpha_mask = true;
            }
            RecordCmd::BeginTransform { transform } => {
                let mut group = Group::plain(Vec::new());
                group.transform = Transform2d::from_affine(Affine2d(transform.0));
                self.begin(group);
            }
            RecordCmd::BeginPerspective { matrix } => {
                let mut group = Group::plain(Vec::new());
                group.transform = Transform2d(matrix.0);
                self.begin(group);
            }
            RecordCmd::BeginOpacity { alpha } => {
                let mut group = Group::plain(Vec::new());
                group.opacity = *alpha as f32;
                self.begin(group);
            }
            RecordCmd::BeginClipRect { rect } => {
                let mut group = Group::plain(Vec::new());
                group.clip = Some(Clip::Rect(*rect));
                self.begin(group);
            }
            RecordCmd::BeginClipRoundRect { rrect } => {
                let mut group = Group::plain(Vec::new());
                group.clip = Some(Clip::RoundRect(round_rect(*rrect)));
                self.begin(group);
            }
            RecordCmd::BeginClipPath { path, fill_rule } => {
                let path = self.path(command_index, *path)?;
                let mut group = Group::plain(Vec::new());
                group.clip = Some(Clip::Path {
                    path,
                    fill_rule: fill_rule_of(*fill_rule),
                });
                self.begin(group);
            }
            RecordCmd::BeginSaveLayer { bounds, alpha } => {
                let mut group = Group::plain(Vec::new());
                group.opacity = *alpha as f32;
                group.layer_bounds = *bounds;
                group.isolated = true;
                self.begin(group);
            }
            RecordCmd::BeginBlend { mode } => {
                let mut group = Group::plain(Vec::new());
                group.internal_blend = blend_mode(*mode)?;
                self.begin(group);
            }
            RecordCmd::BeginMask { source, mode, rect } => {
                let source = self.mask_source(command_index, source, *rect)?;
                let mut group = Group::plain(Vec::new());
                group.mask = Some(Mask {
                    source,
                    mode: match mode {
                        recording::MaskMode::Alpha => MaskMode::Alpha,
                        recording::MaskMode::Luminance => MaskMode::Luminance,
                    },
                });
                self.begin(group);
            }
            RecordCmd::BeginFilter { filters, bounds } => {
                let mut group = Group::plain(Vec::new());
                group.filters = self.filters(command_index, *filters)?;
                group.layer_bounds = *bounds;
                self.begin(group);
            }
            RecordCmd::BeginBackdropFilter { filters, bounds } => {
                let filters = self.filters(command_index, *filters)?;
                let mut group = Group::plain(Vec::new());
                group.backdrop = Some(BackdropRead {
                    scope: BackdropScope::Current,
                    bounds: bounds.unwrap_or(self.builder_viewport()),
                    footprint: filters.iter().fold(Insets::default(), |sum, filter| {
                        sum.added(recording_filter_footprint(filter))
                    }),
                    sampling: SamplingMode::LinearClamp,
                    filters,
                });
                self.begin(group);
            }
            RecordCmd::BeginShaderLayer {
                program,
                uniforms,
                inputs,
                bounds,
            } => {
                let program = self.source.shader_programs.get(program.index()).ok_or(
                    ProgramRecordingError::InvalidReference {
                        command: command_index,
                        table: "shaderPrograms",
                        index: program.0,
                    },
                )?;
                let uniforms = self
                    .source
                    .shader_uniforms
                    .get(uniforms.range())
                    .ok_or(ProgramRecordingError::InvalidReference {
                        command: command_index,
                        table: "shaderUniforms",
                        index: uniforms.start,
                    })?
                    .iter()
                    .map(shader_uniform)
                    .collect();
                let inputs = self
                    .source
                    .shader_inputs
                    .get(inputs.range())
                    .ok_or(ProgramRecordingError::InvalidReference {
                        command: command_index,
                        table: "shaderInputs",
                        index: inputs.start,
                    })?
                    .iter()
                    .map(|input| {
                        Ok(ShaderTextureBinding {
                            name: input.name.clone(),
                            texture: self.image_texture(command_index, input.image)?,
                            sampling: SamplingMode::LinearClamp,
                        })
                    })
                    .collect::<Result<Vec<_>, ProgramRecordingError>>()?;
                let mut group = Group::plain(Vec::new());
                group.shader = Some(ShaderLayer {
                    shader: RuntimeShaderKey {
                        uri: program.uri.clone(),
                        content_hash: program.content_hash,
                        abi_hash: program.abi_hash,
                    },
                    bounds: *bounds,
                    uniforms,
                    textures: inputs,
                });
                self.begin(group);
            }
            RecordCmd::BeginMotionGlass { program } => {
                let mut group = Group::plain(Vec::new());
                group.glass = Some(program.clone());
                self.begin(group);
            }
            RecordCmd::BeginMotionGlassForeground { program } => {
                let mut group = Group::plain(Vec::new());
                group.glass_foreground = Some(program.clone());
                self.begin(group);
            }
            RecordCmd::End => self.end(command_index)?,
            RecordCmd::Path {
                path,
                fill_rule,
                fill,
                stroke,
            } => {
                let path = self.path(command_index, *path)?;
                let fill = fill
                    .as_ref()
                    .map(|paint| self.paint(command_index, paint))
                    .transpose()?;
                let stroke = stroke
                    .as_ref()
                    .map(|stroke| self.stroke(command_index, stroke))
                    .transpose()?;
                let node = self.builder.push_node(Node::Path(PathNode {
                    path,
                    fill_rule: fill_rule_of(*fill_rule),
                    fill,
                    stroke,
                }));
                self.push_node(node);
            }
            RecordCmd::GeometryBatch {
                geometry,
                instances,
            } => {
                let instances = self
                    .source
                    .batch_instances
                    .get(instances.range())
                    .ok_or(ProgramRecordingError::InvalidReference {
                        command: command_index,
                        table: "batchInstances",
                        index: instances.start,
                    })?
                    .iter()
                    .map(|instance| BatchInstance {
                        position: [instance.position.x, instance.position.y],
                        size: [instance.size.x, instance.size.y],
                        color: LinearColor::from_srgb8(instance.color),
                    })
                    .collect();
                let node = self
                    .builder
                    .push_node(Node::GeometryBatch(GeometryBatchNode {
                        geometry: match geometry {
                            recording::BatchGeometry::Circle => BatchGeometry::Circle,
                            recording::BatchGeometry::Rect => BatchGeometry::Rect,
                        },
                        instances,
                    }));
                self.push_node(node);
            }
            RecordCmd::GlyphRun {
                font,
                glyphs,
                paint,
                stroke,
                source,
            } => {
                let face = self.source.fonts.get(font.index()).ok_or(
                    ProgramRecordingError::InvalidReference {
                        command: command_index,
                        table: "fonts",
                        index: font.0,
                    },
                )?;
                let glyphs = self.source.glyphs.get(glyphs.range()).ok_or(
                    ProgramRecordingError::InvalidReference {
                        command: command_index,
                        table: "glyphs",
                        index: glyphs.start,
                    },
                )?;
                let bounds = glyph_bounds(glyphs, face.size);
                let source_node = source
                    .as_ref()
                    .and_then(|source| self.source.text_sources.get(source.node.index()))
                    .cloned();
                let source_ranges = source
                    .as_ref()
                    .and_then(|source| self.source.glyph_source_ranges.get(source.ranges.range()))
                    .map(|ranges| {
                        ranges
                            .iter()
                            .map(|range| [range.start, range.end])
                            .collect()
                    })
                    .unwrap_or_default();
                let glyphs = glyphs
                    .iter()
                    .map(|glyph| Glyph {
                        id: glyph.id,
                        x: glyph.x,
                        y: glyph.y,
                    })
                    .collect();
                let paint = self.paint(command_index, paint)?;
                let stroke = stroke
                    .as_ref()
                    .map(|stroke| self.stroke(command_index, stroke))
                    .transpose()?;
                let node = self.builder.push_node(Node::GlyphRun(GlyphRun {
                    font: font_key(face)?,
                    font_size: face.size as f32,
                    glyphs,
                    bounds,
                    paint,
                    stroke,
                    source_node,
                    source_ranges,
                }));
                self.push_node(node);
            }
            RecordCmd::Image {
                image,
                dst,
                src,
                alpha,
                fit,
            } => {
                let image_source = self.source.images.get(image.index()).ok_or(
                    ProgramRecordingError::InvalidReference {
                        command: command_index,
                        table: "images",
                        index: image.0,
                    },
                )?;
                let intrinsic = match self.catalog {
                    Some(catalog) => {
                        catalog.texture_extent(&image_source.asset).ok_or_else(|| {
                            ProgramRecordingError::MissingTextureDescriptor {
                                key: image_source.asset.clone(),
                            }
                        })?
                    }
                    None => ProgramTextureExtent {
                        width: f64::from(image_source.width),
                        height: f64::from(image_source.height),
                    },
                };
                let dst = fit.place(*dst, intrinsic.width, intrinsic.height);
                let src = src.map_or(Rect::new(0.0, 0.0, 1.0, 1.0), |src| {
                    Rect::new(
                        src.x / f64::from(image_source.width.max(1)),
                        src.y / f64::from(image_source.height.max(1)),
                        src.width / f64::from(image_source.width.max(1)),
                        src.height / f64::from(image_source.height.max(1)),
                    )
                });
                let node = self.builder.push_node(Node::Image(ImageNode {
                    texture: image_texture(image_source),
                    src,
                    dst,
                    sampling: SamplingMode::LinearClamp,
                    opacity: *alpha as f32,
                }));
                self.push_node(node);
            }
            RecordCmd::Texture {
                texture,
                dst,
                alpha,
            } => {
                let source = self.source.generated_textures.get(texture.index()).ok_or(
                    ProgramRecordingError::InvalidReference {
                        command: command_index,
                        table: "generatedTextures",
                        index: texture.0,
                    },
                )?;
                let node = if let Some(scene) = self
                    .catalog
                    .and_then(|catalog| catalog.scene3d_key(&source.provider_key))
                {
                    let scene = self.builder.push_node(Node::Scene3d(Scene3dNode {
                        scene,
                        bounds: *dst,
                    }));
                    if *alpha == 1.0 {
                        scene
                    } else {
                        let mut group = Group::plain(vec![scene]);
                        group.opacity = *alpha as f32;
                        self.builder.push_node(Node::Group(group))
                    }
                } else {
                    self.builder.push_node(Node::Image(ImageNode {
                        texture: ExternalTexture {
                            key: source.provider_key.clone(),
                            kind: TextureKind::Generated,
                            color_domain: ColorDomain::LinearRec2020,
                            alpha: AlphaMode::Premultiplied,
                            sample_time_micros: None,
                        },
                        src: Rect::new(0.0, 0.0, 1.0, 1.0),
                        dst: *dst,
                        sampling: SamplingMode::LinearClamp,
                        opacity: *alpha as f32,
                    }))
                };
                self.push_node(node);
            }
            RecordCmd::Shadow {
                rrect,
                dx,
                dy,
                blur_sigma,
                spread,
                color,
                inset,
            } => {
                let node = self.builder.push_node(Node::Shadow(ShadowNode {
                    shape: round_rect(*rrect),
                    offset: [*dx as f32, *dy as f32],
                    sigma_x: *blur_sigma as f32,
                    sigma_y: *blur_sigma as f32,
                    spread: *spread as f32,
                    color: LinearColor::from_srgb8(*color),
                    inset: *inset,
                }));
                self.push_node(node);
            }
        }
        Ok(())
    }

    fn begin(&mut self, group: Group) {
        self.stack.push(RecordingFrame {
            group,
            alpha_mask: false,
        });
    }

    fn end(&mut self, command: usize) -> Result<(), ProgramRecordingError> {
        let mut frame = self
            .stack
            .pop()
            .ok_or(ProgramRecordingError::UnexpectedEnd { command })?;
        if frame.alpha_mask {
            if frame.group.children.len() != 2 {
                return Err(ProgramRecordingError::InvalidRecording(
                    "alpha mask requires mask and content groups".into(),
                ));
            }
            let source = frame.group.children.remove(0);
            frame.group.mask = Some(Mask {
                source,
                mode: MaskMode::Alpha,
            });
        }
        // Fold construction wrappers only when the child is the sole, last-owned node.
        // Destination-reading subtrees retain every scope boundary.
        if frame.group.children.len() == 1 && self.raster_scope(&frame.group.children) {
            let child = frame.group.children[0];
            if child.index() + 1 == self.builder.nodes.len()
                && let Some(Some(Node::Group(inner))) = self.builder.nodes.last()
                && let Some(merged) = merge_recording_groups(&frame.group, inner)
            {
                self.builder.nodes.pop();
                frame.group = merged;
            }
        }
        let node = self.builder.push_node(Node::Group(frame.group));
        self.push_node(node);
        Ok(())
    }

    fn raster_scope(&self, children: &[NodeId]) -> bool {
        let mut pending = children.to_vec();
        while let Some(node) = pending.pop() {
            match self.builder.nodes[node.index()].as_ref() {
                Some(Node::Group(group)) if group.is_raster_tree_group() => {
                    pending.extend(&group.children)
                }
                Some(Node::Group(_) | Node::RuntimeShader(_)) | None => return false,
                _ => {}
            }
        }
        true
    }

    fn push_node(&mut self, node: NodeId) {
        if let Some(frame) = self.stack.last_mut() {
            frame.group.children.push(node);
        } else {
            self.roots.push(node);
        }
    }

    fn builder_viewport(&self) -> Rect {
        self.viewport
    }

    fn path(
        &mut self,
        command: usize,
        path: crate::PathRef,
    ) -> Result<PathId, ProgramRecordingError> {
        let key = (
            path.verbs.start,
            path.verbs.end,
            path.points.start,
            path.points.end,
        );
        if let Some(path) = self.paths.get(&key) {
            return Ok(*path);
        }
        let verbs = self.source.verbs.get(path.verbs.range()).ok_or(
            ProgramRecordingError::InvalidReference {
                command,
                table: "verbs",
                index: path.verbs.start,
            },
        )?;
        let points = self.source.points.get(path.points.range()).ok_or(
            ProgramRecordingError::InvalidReference {
                command,
                table: "points",
                index: path.points.start,
            },
        )?;
        let path = self.builder.push_path(PathData {
            verbs: verbs
                .iter()
                .map(|verb| match verb {
                    crate::PathVerb::Move => super::PathVerb::MoveTo,
                    crate::PathVerb::Line => super::PathVerb::LineTo,
                    crate::PathVerb::Quad => super::PathVerb::QuadTo,
                    crate::PathVerb::Cubic => super::PathVerb::CubicTo,
                    crate::PathVerb::Close => super::PathVerb::Close,
                })
                .collect(),
            points: points.iter().map(|point| [point.x, point.y]).collect(),
        });
        self.paths.insert(key, path);
        Ok(path)
    }

    fn paint(
        &mut self,
        command: usize,
        paint: &recording::Paint,
    ) -> Result<PaintId, ProgramRecordingError> {
        let paint = paint_of(self.source, command, paint)?;
        if let Some((_, id)) = self.paints.iter().find(|(existing, _)| *existing == paint) {
            return Ok(*id);
        }
        let id = self.builder.push_paint(paint.clone());
        self.paints.push((paint, id));
        Ok(id)
    }

    fn stroke(
        &mut self,
        command: usize,
        stroke: &recording::Stroke,
    ) -> Result<PathStroke, ProgramRecordingError> {
        Ok(PathStroke {
            paint: self.paint(command, &stroke.paint)?,
            width: stroke.width as f32,
            dash: stroke
                .dash
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|value| *value as f32)
                .collect(),
            dash_offset: stroke.dash_offset as f32,
            cap: match stroke.cap {
                crate::Cap::Butt => StrokeCap::Butt,
                crate::Cap::Round => StrokeCap::Round,
                crate::Cap::Square => StrokeCap::Square,
            },
            join: match stroke.join {
                crate::Join::Miter => StrokeJoin::Miter,
                crate::Join::Round => StrokeJoin::Round,
                crate::Join::Bevel => StrokeJoin::Bevel,
            },
            miter_limit: stroke.miter_limit as f32,
        })
    }

    fn filters(
        &self,
        command: usize,
        span: crate::Span,
    ) -> Result<Vec<Filter>, ProgramRecordingError> {
        self.source
            .filters
            .get(span.range())
            .ok_or(ProgramRecordingError::InvalidReference {
                command,
                table: "filters",
                index: span.start,
            })
            .map(|filters| filters.iter().copied().map(filter_of).collect())
    }

    fn mask_source(
        &mut self,
        command: usize,
        source: &recording::MaskSource,
        rect: Rect,
    ) -> Result<NodeId, ProgramRecordingError> {
        match source {
            recording::MaskSource::Image { image } => {
                let node = self.builder.push_node(Node::Image(ImageNode {
                    texture: self.image_texture(command, *image)?,
                    src: Rect::new(0.0, 0.0, 1.0, 1.0),
                    dst: rect,
                    sampling: SamplingMode::LinearClamp,
                    opacity: 1.0,
                }));
                Ok(node)
            }
            recording::MaskSource::Paint { paint } => {
                let path = self.builder.push_path(rect_path(rect));
                let paint = self.paint(command, paint)?;
                Ok(self.builder.push_node(Node::Path(PathNode {
                    path,
                    fill_rule: FillRule::NonZero,
                    fill: Some(paint),
                    stroke: None,
                })))
            }
        }
    }

    fn image_texture(
        &self,
        command: usize,
        image: recording::ImageId,
    ) -> Result<ExternalTexture, ProgramRecordingError> {
        self.source
            .images
            .get(image.index())
            .map(image_texture)
            .ok_or(ProgramRecordingError::InvalidReference {
                command,
                table: "images",
                index: image.0,
            })
    }
}

fn fill_rule_of(value: recording::FillRule) -> FillRule {
    match value {
        recording::FillRule::NonZero => FillRule::NonZero,
        recording::FillRule::EvenOdd => FillRule::EvenOdd,
    }
}

fn round_rect(value: recording::RoundRect) -> RoundRect {
    RoundRect {
        rect: value.rect,
        radii: value.radii.map(|radius| [radius.x, radius.y]),
    }
}

fn blend_mode(value: recording::BlendMode) -> Result<BlendMode, ProgramRecordingError> {
    Ok(match value {
        recording::BlendMode::Normal => BlendMode::Normal,
        recording::BlendMode::Multiply => BlendMode::Multiply,
        recording::BlendMode::Screen => BlendMode::Screen,
        recording::BlendMode::Overlay => BlendMode::Overlay,
        recording::BlendMode::Darken => BlendMode::Darken,
        recording::BlendMode::Lighten => BlendMode::Lighten,
        recording::BlendMode::ColorDodge => BlendMode::ColorDodge,
        recording::BlendMode::ColorBurn => BlendMode::ColorBurn,
        recording::BlendMode::HardLight => BlendMode::HardLight,
        recording::BlendMode::SoftLight => BlendMode::SoftLight,
        recording::BlendMode::Difference => BlendMode::Difference,
        recording::BlendMode::Exclusion => BlendMode::Exclusion,
        recording::BlendMode::Hue => BlendMode::Hue,
        recording::BlendMode::Saturation => BlendMode::Saturation,
        recording::BlendMode::Color => BlendMode::Color,
        recording::BlendMode::Luminosity => BlendMode::Luminosity,
        recording::BlendMode::DestinationIn => {
            return Err(ProgramRecordingError::InvalidRecording(
                "destination-in is only legal inside a structured mask".into(),
            ));
        }
    })
}

fn filter_of(value: recording::FilterOp) -> Filter {
    match value {
        recording::FilterOp::Blur { sigma } => Filter::Blur {
            sigma_x: sigma as f32,
            sigma_y: sigma as f32,
        },
        recording::FilterOp::Brightness { amount } => Filter::Brightness {
            amount: amount as f32,
        },
        recording::FilterOp::Contrast { amount } => Filter::Contrast {
            amount: amount as f32,
        },
        recording::FilterOp::Grayscale { amount } => Filter::Grayscale {
            amount: amount as f32,
        },
        recording::FilterOp::HueRotate { degrees } => Filter::HueRotate {
            degrees: degrees as f32,
        },
        recording::FilterOp::Invert { amount } => Filter::Invert {
            amount: amount as f32,
        },
        recording::FilterOp::Opacity { amount } => Filter::Opacity {
            amount: amount as f32,
        },
        recording::FilterOp::Saturate { amount } => Filter::Saturate {
            amount: amount as f32,
        },
        recording::FilterOp::Sepia { amount } => Filter::Sepia {
            amount: amount as f32,
        },
        recording::FilterOp::DropShadow {
            dx,
            dy,
            sigma,
            color,
        } => Filter::DropShadow {
            offset: [dx as f32, dy as f32],
            sigma_x: sigma as f32,
            sigma_y: sigma as f32,
            color: LinearColor::from_srgb8(color),
        },
        recording::FilterOp::NoiseDisplacement {
            frequency_x,
            frequency_y,
            octaves,
            seed,
            scale,
            turbulence,
        } => Filter::NoiseDisplacement {
            frequency: [frequency_x as f32, frequency_y as f32],
            octaves,
            seed,
            scale: scale as f32,
            turbulence,
        },
        recording::FilterOp::VelocityBlur {
            velocity_x,
            velocity_y,
            shutter_angle,
        } => Filter::VelocityBlur {
            velocity: [velocity_x as f32, velocity_y as f32],
            shutter_angle_degrees: shutter_angle as f32,
        },
    }
}

fn recording_filter_footprint(filter: &Filter) -> Insets {
    match filter {
        Filter::Blur { sigma_x, sigma_y } => Insets::new(
            3.0 * *sigma_x,
            3.0 * *sigma_y,
            3.0 * *sigma_x,
            3.0 * *sigma_y,
        ),
        Filter::DropShadow {
            offset,
            sigma_x,
            sigma_y,
            ..
        } => Insets::new(
            (3.0 * *sigma_x - offset[0]).max(0.0),
            (3.0 * *sigma_y - offset[1]).max(0.0),
            (3.0 * *sigma_x + offset[0]).max(0.0),
            (3.0 * *sigma_y + offset[1]).max(0.0),
        ),
        Filter::NoiseDisplacement { scale, .. } => Insets::uniform(*scale),
        Filter::VelocityBlur {
            velocity,
            shutter_angle_degrees,
        } => {
            let scale = *shutter_angle_degrees / 360.0;
            let dx = velocity[0] * scale;
            let dy = velocity[1] * scale;
            Insets::new((-dx).max(0.0), (-dy).max(0.0), dx.max(0.0), dy.max(0.0))
        }
        _ => Insets::default(),
    }
}

fn paint_of(
    source: &recording::ProgramRecording,
    command: usize,
    paint: &recording::Paint,
) -> Result<Paint, ProgramRecordingError> {
    let stops = |span: crate::Span, opacity: f32| {
        source
            .gradient_stops
            .get(span.range())
            .ok_or(ProgramRecordingError::InvalidReference {
                command,
                table: "gradientStops",
                index: span.start,
            })
            .map(|stops| {
                stops
                    .iter()
                    .map(|stop| super::GradientStop {
                        offset: stop.offset as f32,
                        color: LinearColor::from_srgb8(stop.color).scale_opacity(opacity),
                    })
                    .collect::<Vec<_>>()
            })
    };
    Ok(match paint {
        recording::Paint::Solid(color) => Paint::Solid(LinearColor::from_srgb8(*color)),
        recording::Paint::Linear(gradient) => Paint::LinearGradient {
            start: [gradient.start.x, gradient.start.y],
            end: [gradient.end.x, gradient.end.y],
            stops: stops(gradient.stops, gradient.alpha as f32)?,
            spread: spread_mode(gradient.spread),
        },
        recording::Paint::Radial(gradient) => Paint::RadialGradient {
            center: [gradient.center.x, gradient.center.y],
            radii: [gradient.radii.x, gradient.radii.y],
            stops: stops(gradient.stops, gradient.alpha as f32)?,
            spread: spread_mode(gradient.spread),
        },
        recording::Paint::TwoCircle(g) => Paint::TwoCircleGradient {
            start: [g.start.x, g.start.y],
            start_radius: g.start_radius,
            end: [g.end.x, g.end.y],
            end_radius: g.end_radius,
            stops: stops(g.stops, g.alpha as f32)?,
            spread: spread_mode(g.spread),
        },
        recording::Paint::Conic(gradient) => Paint::ConicGradient {
            center: [gradient.center.x, gradient.center.y],
            start_angle_degrees: gradient.start_angle,
            sweep_angle_degrees: gradient.sweep_angle,
            stops: stops(gradient.stops, gradient.alpha as f32)?,
            spread: spread_mode(gradient.spread),
        },
    })
}

fn spread_mode(value: recording::SpreadMode) -> SpreadMode {
    match value {
        recording::SpreadMode::Pad => SpreadMode::Pad,
        recording::SpreadMode::Repeat => SpreadMode::Repeat,
        recording::SpreadMode::Reflect => SpreadMode::Reflect,
    }
}

fn image_texture(source: &recording::ImageSource) -> ExternalTexture {
    ExternalTexture {
        key: source.asset.clone(),
        kind: if source.source_time_s.is_some() {
            TextureKind::Video
        } else {
            TextureKind::Image
        },
        color_domain: ColorDomain::LinearRec2020,
        alpha: AlphaMode::Premultiplied,
        sample_time_micros: source
            .source_time_s
            .map(|seconds| (seconds * 1_000_000.0).round() as i64),
    }
}

fn font_key(face: &recording::FontFace) -> Result<FontKey, ProgramRecordingError> {
    let encoded = face.family.strip_prefix("valle-face-").ok_or_else(|| {
        ProgramRecordingError::MissingFontContentDigest {
            family: face.family.clone(),
        }
    })?;
    let (face_hash, face_index) = encoded.rsplit_once('-').ok_or_else(|| {
        ProgramRecordingError::MissingFontContentDigest {
            family: face.family.clone(),
        }
    })?;
    let face_hash = DigestBytes::from_hex(face_hash);
    let face_index = face_index.parse::<u32>().ok();
    if face_hash.is_none() || face_index.is_none() {
        return Err(ProgramRecordingError::MissingFontContentDigest {
            family: face.family.clone(),
        });
    }
    Ok(FontKey {
        face_hash: face_hash.expect("validated above"),
        face_index: face_index.expect("validated above"),
    })
}

fn shader_uniform(value: &recording::ShaderUniformBinding) -> ShaderUniformBinding {
    ShaderUniformBinding {
        name: value.name.clone(),
        value: match value.value {
            recording::ShaderUniformValue::Float { value } => ShaderUniformValue::Float(value),
            recording::ShaderUniformValue::Float2 { value } => ShaderUniformValue::Float2(value),
            recording::ShaderUniformValue::Color { value } => {
                ShaderUniformValue::Color(LinearColor::from_srgb_straight(value))
            }
            recording::ShaderUniformValue::Bool { value } => ShaderUniformValue::Bool(value),
        },
    }
}

fn glyph_bounds(glyphs: &[recording::Glyph], size: f64) -> Rect {
    let mut bounds: Option<Rect> = None;
    for glyph in glyphs {
        let cell = Rect::new(
            glyph.x - size * 0.5,
            glyph.y - size * 1.5,
            size * 2.0,
            size * 2.0,
        );
        bounds = Some(match bounds {
            None => cell,
            Some(bounds) => Rect::from_edges(
                bounds.left().min(cell.left()),
                bounds.top().min(cell.top()),
                bounds.right().max(cell.right()),
                bounds.bottom().max(cell.bottom()),
            ),
        });
    }
    bounds.unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0))
}

fn rect_path(rect: Rect) -> PathData {
    PathData {
        verbs: vec![
            super::PathVerb::MoveTo,
            super::PathVerb::LineTo,
            super::PathVerb::LineTo,
            super::PathVerb::LineTo,
            super::PathVerb::Close,
        ],
        points: vec![
            [rect.left(), rect.top()],
            [rect.right(), rect.top()],
            [rect.right(), rect.bottom()],
            [rect.left(), rect.bottom()],
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::{ProgramRecordingError, font_key};
    use crate::program::recording::FontFace;

    #[test]
    fn nested_transform_opacity_clip_wrappers_use_one_group_per_author_layer() {
        use crate::program::{
            Node, compile_recording,
            recording::{Affine, Paint, ProgramRecording, RecordCmd},
        };
        use crate::{Point, Rect, Rgba};
        let mut recording = ProgramRecording::new();
        for _ in 0..96 {
            recording.push(RecordCmd::BeginTransform {
                transform: Affine::translate(0.01, 0.01),
            });
            recording.push(RecordCmd::BeginSaveLayer {
                bounds: None,
                alpha: 0.997,
            });
            recording.push(RecordCmd::BeginClipRect {
                rect: Rect::new(0.0, 0.0, 16.0, 16.0),
            });
        }
        let path = recording
            .begin_path()
            .move_to(Point::new(0.0, 0.0))
            .line_to(Point::new(16.0, 0.0))
            .line_to(Point::new(0.0, 16.0))
            .close()
            .finish();
        recording.push(RecordCmd::Path {
            path,
            fill_rule: crate::program::recording::FillRule::NonZero,
            fill: Some(Paint::Solid(Rgba::new(255, 0, 0, 255))),
            stroke: None,
        });
        for _ in 0..96 * 3 {
            recording.push(RecordCmd::End);
        }
        let program = compile_recording(Rect::new(0.0, 0.0, 32.0, 32.0), &recording).unwrap();
        assert_eq!(
            program
                .nodes()
                .iter()
                .filter(|node| matches!(node, Node::Group(_)))
                .count(),
            96
        );
        for node in program.nodes() {
            if let Node::Group(group) = node {
                assert_eq!(group.opacity, 0.997_f32);
                assert!(group.clip.is_some());
                assert!(group.isolated);
            }
        }
        program.validate().unwrap();
    }

    #[test]
    fn group_folding_preserves_nested_opacity_and_clip_boundaries() {
        use crate::Rect;
        use crate::program::{Clip, Group, Transform2d};
        let mut outer = Group::plain(Vec::new());
        outer.opacity = 0.5;
        let mut inner = outer.clone();
        assert!(super::merge_recording_groups(&outer, &inner).is_none());
        outer.clip = Some(Clip::Rect(Rect::new(0.0, 0.0, 8.0, 8.0)));
        inner.opacity = 1.0;
        inner.clip = outer.clip.clone();
        assert!(super::merge_recording_groups(&outer, &inner).is_none());
        inner.transform = Transform2d::IDENTITY;
        inner.backdrop = Some(crate::program::BackdropRead {
            scope: crate::program::BackdropScope::Current,
            bounds: Rect::new(0.0, 0.0, 8.0, 8.0),
            footprint: Default::default(),
            sampling: crate::requirements::SamplingMode::LinearClamp,
            filters: Vec::new(),
        });
        assert!(super::merge_recording_groups(&outer, &inner).is_none());
    }

    fn face(family: impl Into<String>) -> FontFace {
        FontFace {
            family: family.into(),
            weight: 400,
            italic: false,
            size: 24.0,
        }
    }

    #[test]
    fn font_key_accepts_only_the_real_content_digest_family_contract() {
        let digest = "a".repeat(64);
        let key = font_key(&face(format!("valle-face-{digest}-2"))).unwrap();
        assert_eq!(key.face_hash.as_hex(), digest);
        assert_eq!(key.face_index, 2);

        for family in [
            "Inter".to_owned(),
            format!("valle-face-{}-0", "A".repeat(64)),
            format!("valle-face-{}-0", "f".repeat(63)),
            format!("valle-face-{}-not-an-index", "f".repeat(64)),
        ] {
            assert!(matches!(
                font_key(&face(family)),
                Err(ProgramRecordingError::MissingFontContentDigest { .. })
            ));
        }
    }
}
