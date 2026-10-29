//! Glass draws with ordinary groups and two shaders. A Glass frosts the backdrop inside its
//! shape before its own layers, then draws its surface at its shell: the optics shader, clipped to
//! its shape, over its shape blurred into a height field and reading the frosted backdrop; then
//! the rim shader, unclipped, over its shape barely blurred, for
//! the line of light its edge catches and the dark edge just outside. A GlassField draws once,
//! before the first of its nodes paints: the union of its members' shapes, bridged where they
//! come within its merge distance, masks its frost and its optics, and its rim draws over them.
//! Both shaders read a member's presence from the gray its shape is drawn in.

use takumi_core::scene::{NodePaint, PaintItemKind};
use takumi_core::style::Affine as TAffine;
use valle_draw::program::LinearColor;
use valle_draw::program::recording::{
    Affine, FillRule, FilterOp, Homography, MaskMode, MaskSource, Paint, RecordCmd, RoundRect,
    ShaderUniformBinding, ShaderUniformValue,
};
use valle_draw::{PathRef, PathVerb, Point, Rect};

use super::{EmitError, Emitter, affine_of, radii_of};
use crate::glass::{
    DEFAULT_BEVEL_MAX, DEFAULT_BEVEL_OF_SHORTER_SIDE, DEFAULT_REFRACTION_OF_BEVEL,
    DEFAULT_REFRACTION_OF_SHORTER_SIDE,
};
use crate::layout::bridge::{GlassMaterial, ResolvedGlass, ResolvedShaderLayer};
use valle_motion::PathData;

/// A Glass's shape in its local pixels.
pub(super) enum Outline {
    /// Its border box, rounded by its radii.
    Box(RoundRect),
    Path(PathData, FillRule),
}

impl Outline {
    pub(super) fn of(glass: &ResolvedGlass, border_box: Rect, radii: [Point; 4]) -> Self {
        match &glass.path {
            Some(clip) => Outline::Path(clip.path.clone(), clip.fill_rule),
            None => Outline::Box(RoundRect {
                rect: border_box,
                radii,
            }),
        }
    }

    /// The box the shape draws in: its border box, or its path's bounds in whole pixels.
    fn bounds(&self) -> Rect {
        match self {
            Outline::Box(rrect) => rrect.rect,
            Outline::Path(path, _) => whole_pixels(points_bounds(&path.points)),
        }
    }

    fn path(&self) -> (PathData, FillRule) {
        match self {
            Outline::Box(rrect) => (round_rect_path(*rrect), FillRule::NonZero),
            Outline::Path(path, rule) => (path.clone(), *rule),
        }
    }
}

/// The width of the line of light inside the silhouette and of the dark edge outside it, as
/// shares of the bevel, and their least widths in layer pixels.
const GLINT_OF_BEVEL: f64 = 0.075;
const GLINT_MIN: f64 = 0.65;
const CONTOUR_OF_BEVEL: f64 = 0.05;
const CONTOUR_MIN: f64 = 0.5;
/// Keep the normal field close to the actual silhouette, especially on small rounded controls.
/// Wider blurs displace the half-coverage contour inward at convex corners.
const RIM_SIGMA_MAX: f64 = 1.5;

/// The optics and rim shaders' inputs for one surface.
struct Optics {
    /// The height field's blur.
    sigma: f64,
    bevel: f64,
    refraction: f64,
    dispersion: f64,
    saturation: f64,
    tint: [f32; 4],
    light: [f32; 3],
    normal_map: [f32; 4],
    geometry: [f32; 4],
    intensity: f64,
    glint: f64,
    contour: f64,
}

impl Optics {
    /// `shorter` is the shorter side the defaults take their shares of; `to_viewport` maps the
    /// layer to the viewport near `at`, where its normals turn into screen normals. Presence is
    /// not here: the shaders read it from their content, a member's own in a field.
    fn of(material: &GlassMaterial, shorter: f64, to_viewport: Homography, at: Point) -> Self {
        let shorter = shorter.max(0.0);
        let bevel = material
            .bevel
            .unwrap_or((DEFAULT_BEVEL_OF_SHORTER_SIDE * shorter).min(DEFAULT_BEVEL_MAX));
        let refraction = material.refraction.unwrap_or(
            (DEFAULT_REFRACTION_OF_BEVEL * bevel).min(DEFAULT_REFRACTION_OF_SHORTER_SIDE * shorter),
        );
        Optics {
            sigma: bevel / 2.0,
            bevel,
            refraction,
            dispersion: material.dispersion,
            saturation: material.saturation,
            tint: material.tint.to_srgb_straight(),
            light: screen_light(material),
            normal_map: normal_map(to_viewport, at),
            geometry: [0.0; 4],
            intensity: material.intensity,
            glint: (GLINT_OF_BEVEL * bevel).max(GLINT_MIN),
            contour: (CONTOUR_OF_BEVEL * bevel).max(CONTOUR_MIN),
        }
    }

    /// Whether the surface changes what lies behind it.
    fn is_visible(&self) -> bool {
        (self.refraction > 0.0 && self.sigma > 0.0)
            || self.intensity > 0.0
            || self.tint[3] > 0.0
            || self.saturation != 1.0
    }

    /// A narrow normal field keeps highlights attached to the silhouette. Distance reads are
    /// reliable to about three sigma; the shader tapers the glow before that limit.
    fn rim_sigma(&self) -> f64 {
        (1.25 * self.glint).clamp(0.75, RIM_SIGMA_MAX)
    }

    fn shader(&self) -> ResolvedShaderLayer {
        ResolvedShaderLayer {
            program: crate::glass::optics::optics_program(),
            uniforms: vec![
                vector4("geometry", self.geometry),
                vector4("normalMap", self.normal_map),
                float("sigma", self.sigma),
                float("bevel", self.bevel),
                float("refraction", self.refraction),
                float("dispersion", self.dispersion),
                float("saturation", self.saturation),
                ShaderUniformBinding {
                    name: "tint".into(),
                    value: ShaderUniformValue::Color { value: self.tint },
                },
                ShaderUniformBinding {
                    name: "light".into(),
                    value: ShaderUniformValue::Float3 { value: self.light },
                },
                float("intensity", self.intensity),
            ],
            inputs: vec![backdrop()],
        }
    }

    fn rim(&self, margin: f64) -> ResolvedShaderLayer {
        let mut geometry = self.geometry;
        geometry[3] = margin as f32;
        ResolvedShaderLayer {
            program: crate::glass::optics::rim_program(),
            uniforms: vec![
                vector4("geometry", geometry),
                vector4("normalMap", self.normal_map),
                float("sigma", self.rim_sigma()),
                float("glint", self.glint),
                float("contour", self.contour),
                ShaderUniformBinding {
                    name: "light".into(),
                    value: ShaderUniformValue::Float3 { value: self.light },
                },
                float("intensity", self.intensity),
            ],
            inputs: vec![],
        }
    }
}

fn vector4(name: &str, value: [f32; 4]) -> ShaderUniformBinding {
    ShaderUniformBinding {
        name: name.into(),
        value: ShaderUniformValue::Float4 { value },
    }
}

fn float(name: &str, value: f64) -> ShaderUniformBinding {
    ShaderUniformBinding {
        name: name.into(),
        value: ShaderUniformValue::Float {
            value: value as f32,
        },
    }
}

/// What a shader layer draws over, read in place.
fn backdrop() -> crate::ShaderTextureInput {
    crate::ShaderTextureInput {
        kind: crate::shader::InputKind::Backdrop,
        name: "backdrop".into(),
        source: Some(crate::shader::BACKDROP_INPUT.into()),
        sampling: crate::shader::InputSampling::Linear,
        wrap: crate::shader::InputWrap::Clamp,
    }
}

/// A field member as the field draws it.
struct Member {
    outline: Outline,
    /// Its shape in the field's box.
    shape: PathData,
    fill_rule: FillRule,
    /// From its own pixels to the field's box.
    to_field: Homography,
    presence: f64,
    shadows: Vec<RecordCmd>,
}

/// One shape a surface's height field and mask are drawn from, in the layer's pixels.
struct Coverage {
    path: PathRef,
    fill_rule: FillRule,
    /// The presence of the Glass it is the shape of.
    presence: f64,
}

impl Emitter<'_> {
    /// Frost what lies behind a Glass's shape. Under its own projection, the shape is projected
    /// into the plane the projection maps into, where a blur has one radius.
    pub(super) fn glass_frost(
        &mut self,
        glass: &ResolvedGlass,
        material: &GlassMaterial,
        outline: &Outline,
        plane: Option<Homography>,
    ) {
        let projection = match plane {
            Some(plane) => match projected(outline, plane) {
                Some(projection) => Some(projection),
                None => return,
            },
            None => None,
        };
        let (outline, scale) = match &projection {
            Some((outline, scale)) => (outline, *scale),
            None => (outline, 1.0),
        };
        let sigma = material.frost * scale * glass.presence;
        let bounds = outline.bounds();
        if sigma <= 0.0 || bounds.is_empty() {
            return;
        }
        let Some(depth) = self.clip_outline(outline) else {
            return;
        };
        let filters = self
            .out
            .recording
            .intern_filters(&[FilterOp::Blur { sigma }]);
        self.push(RecordCmd::BeginBackdropFilter {
            filters,
            bounds: Some(bounds),
        });
        self.push(RecordCmd::End);
        self.close(depth);
    }

    /// Neutralize the complete ancestor projection while retaining its clip/opacity groups.
    /// The silhouette and material lengths then live in viewport pixels, where frost has a
    /// single radius. The centre Jacobian defines that radius under perspective.
    pub(super) fn glass_frost_in_viewport(
        &mut self,
        glass: &ResolvedGlass,
        material: &GlassMaterial,
        outline: &Outline,
        own_plane: Option<Homography>,
    ) {
        let Some(inverse) = self.world.inverse() else {
            return;
        };
        let map = own_plane.unwrap_or(Homography::IDENTITY).then(self.world);
        self.push(RecordCmd::BeginPerspective { matrix: inverse });
        self.glass_frost(glass, material, outline, Some(map));
        self.push(RecordCmd::End);
    }

    pub(super) fn glass_surface_in_viewport(
        &mut self,
        key: &str,
        glass: &ResolvedGlass,
        material: &GlassMaterial,
        outline: &Outline,
        crossed: usize,
    ) -> Result<(), EmitError> {
        let Some(inverse) = self.world.inverse() else {
            return Ok(());
        };
        let Some((outline, scale)) = projected(outline, self.world) else {
            return Ok(());
        };
        let material = GlassMaterial {
            bevel: material.bevel.map(|v| v * scale),
            refraction: material.refraction.map(|v| v * scale),
            ..*material
        };
        self.push(RecordCmd::BeginPerspective { matrix: inverse });
        let result = self.surface(
            key,
            glass,
            &material,
            &outline,
            Homography::IDENTITY,
            crossed + 1,
        );
        self.push(RecordCmd::End);
        result
    }

    /// Draw a Glass's surface. Its backdrop is what lies under the node's own layers, `crossed`
    /// groups above. Under its own projection, the group the node opened last, the surface draws
    /// in the plane the projection maps into, as its frost does: the projection closes around it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn glass_surface(
        &mut self,
        key: &str,
        glass: &ResolvedGlass,
        material: &GlassMaterial,
        outline: &Outline,
        plane: Option<Homography>,
        placement: Homography,
        crossed: usize,
    ) -> Result<(), EmitError> {
        let Some(plane) = plane else {
            return self.surface(key, glass, material, outline, placement, crossed);
        };
        let Some((outline, scale)) = projected(outline, plane) else {
            return Ok(());
        };
        let material = GlassMaterial {
            bevel: material.bevel.map(|bevel| bevel * scale),
            refraction: material.refraction.map(|refraction| refraction * scale),
            ..*material
        };
        self.push(RecordCmd::End);
        let drawn = self.surface(key, glass, &material, &outline, placement, crossed - 1);
        self.push(RecordCmd::BeginPerspective { matrix: plane });
        drawn
    }

    /// Draw a surface in the current plane, which `to_viewport` maps onto the viewport.
    fn surface(
        &mut self,
        key: &str,
        glass: &ResolvedGlass,
        material: &GlassMaterial,
        outline: &Outline,
        to_viewport: Homography,
        crossed: usize,
    ) -> Result<(), EmitError> {
        let bounds = outline.bounds();
        if bounds.is_empty() {
            return Ok(());
        }
        let center = Point::new(
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        );
        let mut optics = Optics::of(
            material,
            bounds.width.min(bounds.height),
            to_viewport,
            center,
        );
        if let Outline::Box(rrect) = outline {
            let radius = rrect.radii[0].x;
            if rrect
                .radii
                .iter()
                .all(|p| (p.x - radius).abs() < 1e-6 && (p.y - radius).abs() < 1e-6)
            {
                optics.geometry = [
                    bounds.width as f32,
                    bounds.height as f32,
                    radius.min(0.5 * bounds.width.min(bounds.height)).max(0.0) as f32,
                    0.0,
                ];
            }
        }
        if glass.presence <= 0.0 || !optics.is_visible() {
            return Ok(());
        }
        check_layer_area(key, bounds)?;
        let Some(mut depth) = self.clip_outline(outline) else {
            return Ok(());
        };
        // A shader layer's box starts at its origin.
        let (path, fill_rule) = outline.path();
        let offset = bounds.x != 0.0 || bounds.y != 0.0;
        let path = if offset {
            self.push(RecordCmd::BeginTransform {
                transform: Affine::translate(bounds.x, bounds.y),
            });
            depth += 1;
            mapped_path(
                &path,
                Homography::from_affine(Affine::translate(-bounds.x, -bounds.y)),
            )
        } else {
            Some(path)
        };
        let Some(path) = path.and_then(|path| self.path_ref(&path)) else {
            self.close(depth);
            return Ok(());
        };
        let shapes = [Coverage {
            path,
            fill_rule,
            presence: glass.presence,
        }];
        let size = Rect::new(0.0, 0.0, bounds.width, bounds.height);
        self.optics_layer(&optics, size, (crossed + depth) as u32, &shapes, None);
        self.close(depth);
        // The rim draws over the clipped surface and reaches just outside it.
        if optics.intensity > 0.0 {
            if offset {
                self.push(RecordCmd::BeginTransform {
                    transform: Affine::translate(bounds.x, bounds.y),
                });
            }
            self.rim_layer(&optics, size, &shapes, None);
            if offset {
                self.push(RecordCmd::End);
            }
        }
        Ok(())
    }

    /// Draw a GlassField's frost and surface, in the box its children lay out in. `first` is the
    /// first of its nodes to paint.
    pub(super) fn glass_field(&mut self, key: &str, first: &NodePaint) -> Result<(), EmitError> {
        let tree = self.tree;
        let field = tree.glass.fields.get(key).ok_or_else(|| {
            EmitError::Program(format!("GlassField '{key}' has no values for this frame"))
        })?;
        // The ordinary placements are absolute 2D coordinates. Retain the ancestor's
        // additional projection once, then collect projected field members in the viewport.
        let ancestor = if self.projections > 0 {
            let inverse = affine_of(self.cur)
                .inverse()
                .ok_or_else(|| EmitError::BadStyle {
                    node: key.to_owned(),
                    reason: "GlassField has a singular ancestor".into(),
                })?;
            Some(Homography::from_affine(inverse).then(self.world))
        } else {
            None
        };
        let space = if ancestor.is_some() {
            Affine::IDENTITY
        } else {
            affine_of(self.field_space(key, first))
        };
        let from_viewport = space.inverse().ok_or_else(|| EmitError::BadStyle {
            node: key.to_owned(),
            reason: "GlassField lays out in a box with a singular transform".into(),
        })?;
        // Each member's shape and outer shadows, mapped into the field's box.
        let mut members = Vec::new();
        for member in &field.members {
            let Some(glass) = tree.glass.glass.get(member) else {
                continue;
            };
            let Some(np) = self.painted(member) else {
                continue;
            };
            if glass.presence <= 0.0 {
                continue;
            }
            let Some(node) = tree.root.node_at_path(&np.path) else {
                continue;
            };
            let Ok(layout) = tree.layout.layout(np.node_id) else {
                continue;
            };
            let border_box = Rect::new(
                0.0,
                0.0,
                f64::from(layout.size.width),
                f64::from(layout.size.height),
            );
            let radii = radii_of(&node.context, layout.size.width, layout.size.height);
            let outline = Outline::of(glass, border_box, radii);
            let to_field = self
                .local_to_viewport(np, node, border_box, member)?
                .then(ancestor.unwrap_or(Homography::IDENTITY))
                .then(Homography::from_affine(from_viewport));
            let (path, fill_rule) = outline.path();
            let Some(shape) = mapped_path(&path, to_field) else {
                continue;
            };
            members.push(Member {
                shadows: self.box_shadow_cmds(np, node, border_box, radii, false),
                outline,
                shape,
                fill_rule,
                to_field,
                presence: glass.presence,
            });
        }
        let Some(extent) = members
            .iter()
            .map(|member| points_bounds(&member.shape.points))
            .reduce(union)
        else {
            return Ok(());
        };
        // Bridges stay inside the union's bounds; a pixel more holds their antialiasing.
        let bounds = whole_pixels(Rect::new(
            extent.x - 2.0,
            extent.y - 2.0,
            extent.width + 4.0,
            extent.height + 4.0,
        ));
        check_layer_area(key, bounds)?;
        let shorter = members
            .iter()
            .map(|member| {
                let bounds = points_bounds(&member.shape.points);
                bounds.width.min(bounds.height)
            })
            .fold(f64::INFINITY, f64::min);
        let to_viewport = Homography::from_affine(space);
        let center = Point::new(
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        );
        let scale = ancestor
            .and_then(|map| {
                map.inverse()
                    .and_then(|inv| inv.try_apply(center))
                    .and_then(|local| length_scale(map, local))
            })
            .unwrap_or(1.0);
        let material = GlassMaterial {
            bevel: field.material.bevel.map(|v| v * scale),
            refraction: field.material.refraction.map(|v| v * scale),
            frost: field.material.frost * scale,
            ..field.material
        };
        let optics = Optics::of(&material, shorter, to_viewport, center);

        // Draw at the bounds' origin: a shader layer's box starts at its origin.
        let target = Affine::translate(bounds.x, bounds.y).then(space);
        let mut depth = 0;
        if ancestor.is_some() {
            let inverse = self.world.inverse().ok_or_else(|| EmitError::BadStyle {
                node: key.to_owned(),
                reason: "GlassField draws under a singular projection".into(),
            })?;
            depth += self.begin_projection(Homography::from_affine(target).then(inverse));
        } else {
            let current = affine_of(self.cur)
                .inverse()
                .ok_or_else(|| EmitError::BadStyle {
                    node: key.to_owned(),
                    reason: "GlassField draws under a singular transform".into(),
                })?;
            let relative = target.then(current);
            if relative != Affine::IDENTITY {
                self.push(RecordCmd::BeginTransform {
                    transform: relative,
                });
                depth += 1;
            }
        }
        let to_layer = Homography::from_affine(Affine::translate(-bounds.x, -bounds.y));
        let mut shapes = Vec::with_capacity(members.len());
        for member in members {
            // The members' outer shadows lie beneath the field, each outside its own shape.
            if !member.shadows.is_empty() {
                let opened = self.begin_projection(member.to_field.then(to_layer));
                self.shadows_outside(member.shadows, &member.outline);
                self.close(opened);
            }
            if let Some(path) =
                mapped_path(&member.shape, to_layer).and_then(|path| self.path_ref(&path))
            {
                shapes.push(Coverage {
                    path,
                    fill_rule: member.fill_rule,
                    presence: member.presence,
                });
            }
        }
        let size = Rect::new(0.0, 0.0, bounds.width, bounds.height);
        let bridge = field.merge * scale / 1.35;
        let frost = material.frost;
        // A member coming in brings its frost in with it: while one does, the frost is masked by
        // the shapes' gray, their presences, and the surface by their coverage.
        let fading = shapes.iter().any(|shape| shape.presence < 1.0);
        if frost > 0.0 && fading {
            self.begin_coverage_mask(MaskMode::Luminance, &shapes, bridge, size);
            self.frost(frost, size);
            self.push(RecordCmd::End);
        }
        self.begin_coverage_mask(MaskMode::Alpha, &shapes, bridge, size);
        if frost > 0.0 && !fading {
            self.frost(frost, size);
        }
        if optics.is_visible() {
            self.optics_layer(&optics, size, 0, &shapes, Some(bridge));
        }
        self.push(RecordCmd::End);
        if optics.intensity > 0.0 {
            self.rim_layer(&optics, size, &shapes, Some(bridge));
        }
        self.close(depth);
        Ok(())
    }

    /// The optics shader over the shapes blurred into a height field, reading the backdrop
    /// `through` groups above.
    fn optics_layer(
        &mut self,
        optics: &Optics,
        size: Rect,
        through: u32,
        shapes: &[Coverage],
        bridge: Option<f64>,
    ) {
        let (program, uniforms, inputs) = self.intern_shader(optics.shader(), size, Some(through));
        self.push(RecordCmd::BeginShaderLayer {
            program,
            uniforms,
            inputs,
            bounds: size,
        });
        if optics.sigma > 0.0 {
            let filters = self.out.recording.intern_filters(&[FilterOp::Blur {
                sigma: optics.sigma,
            }]);
            self.push(RecordCmd::BeginFilter {
                filters,
                bounds: None,
            });
            self.coverage(shapes, bridge, size);
            self.push(RecordCmd::End);
        } else {
            self.coverage(shapes, bridge, size);
        }
        self.push(RecordCmd::End);
    }

    /// The rim shader over the shapes barely blurred. Its box reaches
    /// past `size` for the dark edge outside the shapes, unless that box would be too large.
    fn rim_layer(&mut self, optics: &Optics, size: Rect, shapes: &[Coverage], bridge: Option<f64>) {
        let margin = (optics.contour + 2.0).ceil();
        let pixels = (size.width + 2.0 * margin).ceil() * (size.height + 2.0 * margin).ceil();
        let margin = if pixels > valle_draw::program::MAX_SHADER_LAYER_PIXELS as f64 {
            0.0
        } else {
            margin
        };
        let outer = Rect::new(
            0.0,
            0.0,
            size.width + 2.0 * margin,
            size.height + 2.0 * margin,
        );
        let (program, uniforms, inputs) = self.intern_shader(optics.rim(margin), outer, None);
        let shifted = margin > 0.0;
        if shifted {
            self.push(RecordCmd::BeginTransform {
                transform: Affine::translate(-margin, -margin),
            });
        }
        self.push(RecordCmd::BeginShaderLayer {
            program,
            uniforms,
            inputs,
            bounds: outer,
        });
        if shifted {
            self.push(RecordCmd::BeginTransform {
                transform: Affine::translate(margin, margin),
            });
        }
        let filters = self.out.recording.intern_filters(&[FilterOp::Blur {
            sigma: optics.rim_sigma(),
        }]);
        self.push(RecordCmd::BeginFilter {
            filters,
            bounds: None,
        });
        self.coverage(shapes, bridge, size);
        self.push(RecordCmd::End);
        if shifted {
            self.push(RecordCmd::End);
        }
        self.push(RecordCmd::End);
        if shifted {
            self.push(RecordCmd::End);
        }
    }

    /// Open a mask of the shapes' coverage, by `mode`, around the groups that follow.
    fn begin_coverage_mask(
        &mut self,
        mode: MaskMode,
        shapes: &[Coverage],
        bridge: f64,
        size: Rect,
    ) {
        self.push(RecordCmd::BeginMask {
            source: MaskSource::Subtree,
            mode,
            rect: size,
        });
        self.push(RecordCmd::BeginMaskSource);
        self.coverage(shapes, Some(bridge), size);
        self.push(RecordCmd::End);
    }

    /// The shapes opaque, each in the gray of its presence. A bridge adds, under them, the necks
    /// between shapes nearer than about 1.35 sigma: their blur at their presences' alphas,
    /// thresholded at half, so a neck forms as the shapes on either side come in, in the gray
    /// of their presences.
    fn coverage(&mut self, shapes: &[Coverage], bridge: Option<f64>, size: Rect) {
        if let Some(sigma) = bridge {
            // Across a straight blurred edge, the gain makes the threshold's ramp one pixel wide.
            let gain = (sigma * valle_draw::math::sqrt(2.0 * core::f64::consts::PI)).max(1.0);
            let mut matrix = [0.0; 20];
            matrix[0] = 1.0;
            matrix[6] = 1.0;
            matrix[12] = 1.0;
            matrix[18] = gain;
            matrix[19] = 0.5 - 0.5 * gain;
            let filters = self
                .out
                .recording
                .intern_filters(&[FilterOp::Blur { sigma }, FilterOp::ColorMatrix { matrix }]);
            self.push(RecordCmd::BeginFilter {
                filters,
                bounds: Some(size),
            });
            for shape in shapes {
                let presence = shape.presence.clamp(0.0, 1.0) as f32;
                self.fill_gray(shape, presence, presence);
            }
            self.push(RecordCmd::End);
        }
        for shape in shapes {
            self.fill_gray(shape, shape.presence.clamp(0.0, 1.0) as f32, 1.0);
        }
    }

    fn fill_gray(&mut self, shape: &Coverage, gray: f32, alpha: f32) {
        let value = gray * alpha;
        self.push(RecordCmd::Path {
            path: shape.path,
            fill_rule: shape.fill_rule,
            fill: Some(Paint::FloatSolid(LinearColor {
                red: value,
                green: value,
                blue: value,
                alpha,
            })),
            stroke: None,
        });
    }

    /// Blur what lies behind the box, within the mask the caller opened.
    fn frost(&mut self, sigma: f64, size: Rect) {
        let filters = self
            .out
            .recording
            .intern_filters(&[FilterOp::Blur { sigma }]);
        self.push(RecordCmd::BeginBackdropFilter {
            filters,
            bounds: Some(size),
        });
        self.push(RecordCmd::End);
    }

    /// Paint outer shadows outside a Glass's shape, where CSS paints them.
    pub(super) fn shadows_outside(&mut self, shadows: Vec<RecordCmd>, outline: &Outline) {
        if shadows.is_empty() {
            return;
        }
        let mut extent = outline.bounds();
        for shadow in &shadows {
            if let RecordCmd::Shadow {
                rrect,
                dx,
                dy,
                blur_sigma,
                spread,
                ..
            } = shadow
            {
                let reach = spread.max(0.0) + 3.0 * blur_sigma + 2.0;
                extent = union(
                    extent,
                    Rect::new(
                        rrect.rect.x + dx - reach,
                        rrect.rect.y + dy - reach,
                        rrect.rect.width + 2.0 * reach,
                        rrect.rect.height + 2.0 * reach,
                    ),
                );
            }
        }
        let (shape, _) = outline.path();
        let mut clip = rect_path(extent);
        clip.verbs.extend(shape.verbs);
        clip.points.extend(shape.points);
        let Some(clip) = self.path_ref(&clip) else {
            return;
        };
        self.push(RecordCmd::BeginClipPath {
            path: clip,
            fill_rule: FillRule::EvenOdd,
        });
        for shadow in shadows {
            self.push(shadow);
        }
        self.push(RecordCmd::End);
    }

    /// Clip to a Glass's shape. A square box needs no clip: its bounds hold it. Returns the groups
    /// opened, or `None` when the shape is empty.
    fn clip_outline(&mut self, outline: &Outline) -> Option<usize> {
        match outline {
            Outline::Box(rrect) => {
                if rrect.radii.iter().any(super::radius_is_visible) {
                    self.push(RecordCmd::BeginClipRoundRect { rrect: *rrect });
                    Some(1)
                } else {
                    Some(0)
                }
            }
            Outline::Path(path, fill_rule) => {
                let path = self.path_ref(path)?;
                self.push(RecordCmd::BeginClipPath {
                    path,
                    fill_rule: *fill_rule,
                });
                Some(1)
            }
        }
    }

    /// Draw through `map`; returns the groups opened.
    fn begin_projection(&mut self, map: Homography) -> usize {
        let [a, c, e, b, d, f, g, h, i] = map.0;
        if g == 0.0 && h == 0.0 && i == 1.0 {
            self.push(RecordCmd::BeginTransform {
                transform: Affine([a, b, c, d, e, f]),
            });
        } else {
            self.push(RecordCmd::BeginPerspective { matrix: map });
        }
        1
    }

    fn close(&mut self, groups: usize) {
        for _ in 0..groups {
            self.push(RecordCmd::End);
        }
    }

    /// The absolute transform of the box a field's children lay out in: the nearest ancestor of
    /// `first` outside the field.
    fn field_space(&self, key: &str, first: &NodePaint) -> TAffine {
        let mut path = first.path.as_slice();
        while let Some((_, parent)) = path.split_last() {
            path = parent;
            let Some(paint) = self.paints().find(|paint| paint.path == path) else {
                continue;
            };
            let in_field = self
                .node_key(paint)
                .and_then(|node| self.tree.glass.field_of.get(node))
                .is_some_and(|field| field == key);
            if !in_field {
                return paint.transform;
            }
        }
        TAffine::IDENTITY
    }

    /// The node `key` as this frame paints it.
    fn painted(&self, key: &str) -> Option<&NodePaint> {
        self.paints()
            .find(|paint| self.node_key(paint) == Some(key))
    }

    /// Every node this frame paints.
    fn paints(&self) -> impl Iterator<Item = &NodePaint> {
        self.contexts.iter().flat_map(|cx| {
            cx.root()
                .into_iter()
                .chain(cx.in_paint_order().into_iter().flatten().filter_map(
                    |item| match &item.kind {
                        PaintItemKind::Node(paint) => Some(paint),
                        PaintItemKind::Context(_) => None,
                    },
                ))
        })
    }
}

/// A shader layer's local area is bounded; say so about the Glass rather than its layer.
fn check_layer_area(key: &str, bounds: Rect) -> Result<(), EmitError> {
    let pixels = bounds.width.ceil() * bounds.height.ceil();
    if pixels > valle_draw::program::MAX_SHADER_LAYER_PIXELS as f64 {
        return Err(EmitError::BadStyle {
            node: key.to_owned(),
            reason: format!(
                "Glass draws {}x{} local pixels; one Glass or GlassField draws at most {} (1920x1080)",
                bounds.width.ceil(),
                bounds.height.ceil(),
                valle_draw::program::MAX_SHADER_LAYER_PIXELS
            ),
        });
    }
    Ok(())
}

/// Keep the authored light in screen space; the shaders transform normals into that space.
fn screen_light(material: &GlassMaterial) -> [f32; 3] {
    let [x, y] = material.light;
    let length = valle_draw::math::hypot(x, y);
    [
        (x / length) as f32,
        (y / length) as f32,
        material.elevation as f32,
    ]
}

/// Inverse-transpose Jacobian, up to a positive common factor (the shader normalizes it).
fn normal_map(to_viewport: Homography, at: Point) -> [f32; 4] {
    let [a, b, _, d, e, _, g, h, i] = to_viewport.0;
    let w = g * at.x + h * at.y + i;
    let Some(p) = to_viewport.try_apply(at) else {
        return [1.0, 0.0, 0.0, 1.0];
    };
    let j = [
        (a - g * p.x) / w,
        (b - h * p.x) / w,
        (d - g * p.y) / w,
        (e - h * p.y) / w,
    ];
    let det = j[0] * j[3] - j[1] * j[2];
    let scale = j.iter().fold(0.0_f64, |s, v| s.max(v.abs()));
    if !det.is_finite() || det == 0.0 || !scale.is_finite() || scale == 0.0 {
        return [1.0, 0.0, 0.0, 1.0];
    }
    let factor = det.signum() / scale;
    [j[3], -j[2], -j[1], j[0]].map(|v| (v * factor) as f32)
}

/// A Glass's shape in the plane its own projection maps it into, its curves flattened, and how
/// much the projection scales lengths at the center of its box.
fn projected(outline: &Outline, plane: Homography) -> Option<(Outline, f64)> {
    let bounds = outline.bounds();
    let center = Point::new(
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height / 2.0,
    );
    let scale = length_scale(plane, center)?;
    let (path, fill_rule) = outline.path();
    Some((Outline::Path(mapped_path(&path, plane)?, fill_rule), scale))
}

/// How much `map` scales lengths at `at`: the square root of its Jacobian's determinant.
fn length_scale(map: Homography, at: Point) -> Option<f64> {
    let [a, b, _, d, e, _, g, h, i] = map.0;
    let w = g * at.x + h * at.y + i;
    let p = map.try_apply(at)?;
    let determinant = ((a - g * p.x) * (e - h * p.y) - (b - h * p.x) * (d - g * p.y)) / (w * w);
    let scale = valle_draw::math::sqrt(determinant.abs());
    (scale.is_finite() && scale > 0.0).then_some(scale)
}

/// Map a path through `map`, flattening its curves when the map is projective, which bends them.
/// `None` when a point leaves the viewer's side of the plane.
fn mapped_path(path: &PathData, map: Homography) -> Option<PathData> {
    /// Lines per projected curve.
    const STEPS: usize = 16;
    let affine = map.0[6] == 0.0 && map.0[7] == 0.0 && map.0[8] == 1.0;
    let project = |p: Point| {
        let w = map.0[6] * p.x + map.0[7] * p.y + map.0[8];
        (w > 0.0).then(|| map.try_apply(p)).flatten()
    };
    let mut out = PathData {
        verbs: Vec::with_capacity(path.verbs.len()),
        points: Vec::with_capacity(path.points.len()),
    };
    let mut points = path.points.iter().copied();
    let (mut last, mut start) = (Point::default(), Point::default());
    let line = |out: &mut PathData, p: Point| -> Option<()> {
        out.verbs.push(PathVerb::Line);
        out.points.push(project(p)?);
        Some(())
    };
    for verb in &path.verbs {
        match verb {
            PathVerb::Move => {
                let p = points.next()?;
                out.verbs.push(PathVerb::Move);
                out.points.push(project(p)?);
                (last, start) = (p, p);
            }
            PathVerb::Line => {
                let p = points.next()?;
                line(&mut out, p)?;
                last = p;
            }
            PathVerb::Quad => {
                let (c, p) = (points.next()?, points.next()?);
                if affine {
                    out.verbs.push(PathVerb::Quad);
                    out.points.extend([project(c)?, project(p)?]);
                } else {
                    for step in 1..=STEPS {
                        let t = step as f64 / STEPS as f64;
                        let s = 1.0 - t;
                        let q = Point::new(
                            s * s * last.x + 2.0 * s * t * c.x + t * t * p.x,
                            s * s * last.y + 2.0 * s * t * c.y + t * t * p.y,
                        );
                        line(&mut out, q)?;
                    }
                }
                last = p;
            }
            PathVerb::Cubic => {
                let (c1, c2, p) = (points.next()?, points.next()?, points.next()?);
                if affine {
                    out.verbs.push(PathVerb::Cubic);
                    out.points.extend([project(c1)?, project(c2)?, project(p)?]);
                } else {
                    for step in 1..=STEPS {
                        let t = step as f64 / STEPS as f64;
                        let s = 1.0 - t;
                        let (k0, k1, k2, k3) =
                            (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
                        let q = Point::new(
                            k0 * last.x + k1 * c1.x + k2 * c2.x + k3 * p.x,
                            k0 * last.y + k1 * c1.y + k2 * c2.y + k3 * p.y,
                        );
                        line(&mut out, q)?;
                    }
                }
                last = p;
            }
            PathVerb::Close => {
                out.verbs.push(PathVerb::Close);
                last = start;
            }
        }
    }
    Some(out)
}

/// A rounded rectangle as a path, its radii scaled down together where adjacent ones overlap, as
/// CSS does.
fn round_rect_path(rrect: RoundRect) -> PathData {
    const K: f64 = 0.552_284_749_830_793_6;
    let Rect {
        x,
        y,
        width: w,
        height: h,
    } = rrect.rect;
    let radii = rrect.radii.map(|r| Point::new(r.x.max(0.0), r.y.max(0.0)));
    let [tl, tr, br, bl] = radii;
    let scale = [
        (tl.x + tr.x, w),
        (bl.x + br.x, w),
        (tl.y + bl.y, h),
        (tr.y + br.y, h),
    ]
    .into_iter()
    .filter(|&(sum, side)| sum > side && sum > 0.0)
    .map(|(sum, side)| side / sum)
    .fold(1.0, f64::min);
    let [tl, tr, br, bl] = radii.map(|r| Point::new(r.x * scale, r.y * scale));
    let p = Point::new;
    PathData {
        verbs: vec![
            PathVerb::Move,
            PathVerb::Line,
            PathVerb::Cubic,
            PathVerb::Line,
            PathVerb::Cubic,
            PathVerb::Line,
            PathVerb::Cubic,
            PathVerb::Line,
            PathVerb::Cubic,
            PathVerb::Close,
        ],
        points: vec![
            p(x + tl.x, y),
            p(x + w - tr.x, y),
            p(x + w - tr.x + K * tr.x, y),
            p(x + w, y + tr.y - K * tr.y),
            p(x + w, y + tr.y),
            p(x + w, y + h - br.y),
            p(x + w, y + h - br.y + K * br.y),
            p(x + w - br.x + K * br.x, y + h),
            p(x + w - br.x, y + h),
            p(x + bl.x, y + h),
            p(x + bl.x - K * bl.x, y + h),
            p(x, y + h - bl.y + K * bl.y),
            p(x, y + h - bl.y),
            p(x, y + tl.y),
            p(x, y + tl.y - K * tl.y),
            p(x + tl.x - K * tl.x, y),
            p(x + tl.x, y),
        ],
    }
}

fn rect_path(rect: Rect) -> PathData {
    let Rect {
        x,
        y,
        width,
        height,
    } = rect;
    PathData {
        verbs: vec![
            PathVerb::Move,
            PathVerb::Line,
            PathVerb::Line,
            PathVerb::Line,
            PathVerb::Close,
        ],
        points: vec![
            Point::new(x, y),
            Point::new(x + width, y),
            Point::new(x + width, y + height),
            Point::new(x, y + height),
        ],
    }
}

fn points_bounds(points: &[Point]) -> Rect {
    let [x0, y0, x1, y1] = points.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |[x0, y0, x1, y1], p| [x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)],
    );
    if x0 > x1 || y0 > y1 {
        return Rect::new(0.0, 0.0, 0.0, 0.0);
    }
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

fn union(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.width).max(b.x + b.width);
    let y1 = (a.y + a.height).max(b.y + b.height);
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// The smallest whole-pixel box holding `rect`.
fn whole_pixels(rect: Rect) -> Rect {
    let x0 = rect.x.floor();
    let y0 = rect.y.floor();
    Rect::new(
        x0,
        y0,
        (rect.x + rect.width).ceil() - x0,
        (rect.y + rect.height).ceil() - y0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transformed_normal(map: Homography, normal: [f64; 2]) -> [f64; 2] {
        let m = normal_map(map, Point::new(0.0, 0.0)).map(f64::from);
        let v = [
            m[0] * normal[0] + m[1] * normal[1],
            m[2] * normal[0] + m[3] * normal[1],
        ];
        let length = v[0].hypot(v[1]);
        [v[0] / length, v[1] / length]
    }

    #[test]
    fn nonuniform_scale_keeps_the_light_peak_on_the_screen_normal() {
        // Ellipse made by scaling a circle 2x horizontally. Its diagonal screen normal
        // comes from local (2,-1), not from inverse-transforming the diagonal light.
        let actual = transformed_normal(
            Homography([2.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
            [2.0, -1.0],
        );
        let expected = 1.0 / 2.0_f64.sqrt();
        assert!((actual[0] - expected).abs() < 1e-6);
        assert!((actual[1] + expected).abs() < 1e-6);
    }

    #[test]
    fn mirrored_and_rotated_surfaces_keep_outward_normals() {
        let mirror = Homography([-1.0, 0.0, 10.0, 0.0, 1.0, 20.0, 0.0, 0.0, 1.0]);
        assert_eq!(transformed_normal(mirror, [1.0, 0.0]), [-1.0, 0.0]);
        let rotation = Homography([0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        assert_eq!(transformed_normal(rotation, [1.0, 0.0]), [0.0, 1.0]);
    }
}
