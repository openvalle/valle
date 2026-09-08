use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::{
    Rect,
    requirements::{
        AlphaMode, ColorDomain, DestinationOperation, DestinationUse, DrawRequirements,
        ExternalTexture, Insets, LocalBounds, SamplingMode, TextureKind,
    },
};

use super::{
    BackdropScope, Clip, DrawProgram, FILTER_GAUSSIAN_SUPPORT_SIGMAS, Filter, GradientStop, Group,
    LinearColor, Node, NodeGeometry, NodeId, Paint, PaintId, PathData, PathId, RoundRect,
    ShaderLayer, ShaderUniformValue, Transform2d,
};

pub const MAX_NODES: usize = 65_536;
pub const MAX_ROOTS: usize = 4_096;
pub const MAX_PATHS: usize = 65_536;
pub const MAX_PAINTS: usize = 65_536;
pub const MAX_GROUP_DEPTH: usize = 128;
pub const MAX_EDGES: usize = 262_144;
pub const MAX_PATH_VERBS: usize = 1_000_000;
pub const MAX_PATH_POINTS: usize = 2_000_000;
pub const MAX_GLYPHS: usize = 1_000_000;
pub const MAX_FILTERS: usize = 65_536;
pub const MAX_BATCH_INSTANCES: usize = 1_000_000;
pub const MAX_SHADER_BINDINGS: usize = 65_536;
pub const MAX_GRADIENT_STOPS: usize = 65_536;
pub const MAX_STRING_BYTES: usize = 4_096;
pub const MAX_PACKED_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_LOCAL_INTERMEDIATE_PIXELS: u64 = 256 * 1024 * 1024;
/// Closed upper bound for every Gaussian sigma carried by a DrawProgram filter.
pub const MAX_FILTER_SIGMA: f32 = 1_000_000.0;
const MAX_LOCAL_COORDINATE: f64 = 16_777_216.0;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DrawProgramError {
    #[error("node {referenced:?} referenced by {owner:?} is out of range")]
    InvalidNodeId {
        owner: Option<NodeId>,
        referenced: NodeId,
    },
    #[error("path {referenced:?} referenced by node {owner:?} is out of range")]
    InvalidPathId { owner: NodeId, referenced: PathId },
    #[error("paint {referenced:?} referenced by node {owner:?} is out of range")]
    InvalidPaintId { owner: NodeId, referenced: PaintId },
    #[error("reserved node {id:?} was never defined")]
    UndefinedNode { id: NodeId },
    #[error("node {id:?} was defined more than once")]
    NodeAlreadyDefined { id: NodeId },
    #[error("cycle reaches node {at:?}")]
    Cycle { at: NodeId },
    #[error("group depth {depth} exceeds limit {limit}")]
    DepthLimit { depth: usize, limit: usize },
    #[error("node {id:?} has more than one structural parent")]
    MultipleParents { id: NodeId },
    #[error("node {id:?} is unreachable from the ordered root forest")]
    UnreachableNode { id: NodeId },
    #[error("path {id:?} is not referenced")]
    UnreachablePath { id: PathId },
    #[error("paint {id:?} is not referenced")]
    UnreachablePaint { id: PaintId },
    #[error("{kind} budget exceeded: {actual} > {limit}")]
    BudgetExceeded {
        kind: String,
        actual: usize,
        limit: usize,
    },
    #[error("invalid value at {location}: {reason}")]
    InvalidValue { location: String, reason: String },
    #[error("conflicting {kind} descriptor for identity {key}")]
    ConflictingRequirement { kind: String, key: String },
    #[error("arena order or IDs are not canonical")]
    NonCanonical,
    #[error("stored DrawRequirements do not equal validator-derived requirements")]
    RequirementsMismatch,
    #[error("stored node geometry does not equal validator-derived geometry")]
    GeometryMismatch,
}

pub(crate) fn canonicalize(
    viewport: Rect,
    roots: Vec<NodeId>,
    nodes: Vec<Option<Node>>,
    paths: Vec<PathData>,
    paints: Vec<Paint>,
) -> Result<DrawProgram, DrawProgramError> {
    validate_viewport(viewport)?;
    validate_raw(&roots, &nodes, &paths, &paints)?;

    let mut canonicalizer = Canonicalizer::new(&nodes, &paths, &paints);
    let mut canonical_roots = Vec::with_capacity(roots.len());
    for root in roots {
        canonical_roots.push(canonicalizer.visit(root, 0)?);
    }

    // Glass topology is recursive by construction. Run it only after canonical traversal has
    // rejected cycles and trees deeper than MAX_GROUP_DEPTH, so hostile input cannot exhaust the
    // thread stack before the structural limits take effect.
    validate_glass_topology(&canonicalizer.raw_roots, &nodes)?;

    // Detect cycles before enforcing the stricter forest ownership invariant, so malformed
    // cyclic input receives the most useful diagnostic.
    validate_single_parents(&canonicalizer.raw_roots, &nodes)?;

    if let Some(index) = canonicalizer
        .states
        .iter()
        .position(|state| *state == Visit::New)
    {
        return Err(DrawProgramError::UnreachableNode {
            id: NodeId::from_index(index),
        });
    }
    if let Some(index) = canonicalizer.path_map.iter().position(Option::is_none) {
        return Err(DrawProgramError::UnreachablePath {
            id: PathId::from_index(index),
        });
    }
    if let Some(index) = canonicalizer.paint_map.iter().position(Option::is_none) {
        return Err(DrawProgramError::UnreachablePaint {
            id: PaintId::from_index(index),
        });
    }

    let nodes = canonicalizer
        .nodes
        .into_iter()
        .map(|node| node.expect("visited node has canonical payload"))
        .collect();
    let mut program = DrawProgram {
        viewport,
        roots: canonical_roots,
        nodes,
        paths: canonicalizer.paths,
        paints: canonicalizer.paints,
        node_geometry: Vec::new(),
        requirements: DrawRequirements::default(),
    };
    let node_geometry = derive_node_geometries(&program)?;
    let requirements = derive_requirements(&program, &node_geometry)?;
    program.node_geometry = node_geometry;
    program.requirements = requirements;
    Ok(program)
}

pub(crate) fn validate_canonical(program: &DrawProgram) -> Result<(), DrawProgramError> {
    let rebuilt = canonicalize(
        program.viewport,
        program.roots.clone(),
        program.nodes.iter().cloned().map(Some).collect(),
        program.paths.clone(),
        program.paints.clone(),
    )?;
    if rebuilt.viewport != program.viewport
        || rebuilt.roots != program.roots
        || rebuilt.nodes != program.nodes
        || rebuilt.paths != program.paths
        || rebuilt.paints != program.paints
    {
        return Err(DrawProgramError::NonCanonical);
    }
    if rebuilt.requirements != program.requirements {
        return Err(DrawProgramError::RequirementsMismatch);
    }
    if rebuilt.node_geometry != program.node_geometry {
        return Err(DrawProgramError::GeometryMismatch);
    }
    Ok(())
}

fn budget(kind: &str, actual: usize, limit: usize) -> Result<(), DrawProgramError> {
    if actual > limit {
        Err(DrawProgramError::BudgetExceeded {
            kind: kind.into(),
            actual,
            limit,
        })
    } else {
        Ok(())
    }
}

fn validate_raw(
    roots: &[NodeId],
    nodes: &[Option<Node>],
    paths: &[PathData],
    paints: &[Paint],
) -> Result<(), DrawProgramError> {
    budget("roots", roots.len(), MAX_ROOTS)?;
    budget("nodes", nodes.len(), MAX_NODES)?;
    budget("paths", paths.len(), MAX_PATHS)?;
    budget("paints", paints.len(), MAX_PAINTS)?;

    for (index, node) in nodes.iter().enumerate() {
        if node.is_none() {
            return Err(DrawProgramError::UndefinedNode {
                id: NodeId::from_index(index),
            });
        }
    }
    for root in roots {
        check_node_id(None, *root, nodes.len())?;
    }

    let total_verbs = paths.iter().map(|path| path.verbs.len()).sum();
    let total_points = paths.iter().map(|path| path.points.len()).sum();
    budget("path verbs", total_verbs, MAX_PATH_VERBS)?;
    budget("path points", total_points, MAX_PATH_POINTS)?;
    for (index, path) in paths.iter().enumerate() {
        validate_path(path, index)?;
    }

    let mut total_stops = 0usize;
    for (index, paint) in paints.iter().enumerate() {
        total_stops = total_stops.saturating_add(validate_paint(paint, index)?);
    }
    budget("gradient stops", total_stops, MAX_GRADIENT_STOPS)?;

    let mut edges = 0usize;
    let mut glyphs = 0usize;
    let mut batch_instances = 0usize;
    let mut filters = 0usize;
    let mut shader_bindings = 0usize;
    for (index, node) in nodes.iter().enumerate() {
        let id = NodeId::from_index(index);
        let node = node.as_ref().expect("undefined nodes rejected above");
        match node {
            Node::Group(group) => {
                validate_group(id, group, nodes.len(), paths.len())?;
                edges = edges
                    .saturating_add(group.children.len())
                    .saturating_add(usize::from(group.mask.is_some()));
                filters = filters.saturating_add(group.filters.len()).saturating_add(
                    group
                        .backdrop
                        .as_ref()
                        .map_or(0, |value| value.filters.len()),
                );
                if let Some(shader) = &group.shader {
                    shader_bindings = shader_bindings
                        .saturating_add(shader.uniforms.len())
                        .saturating_add(shader.textures.len());
                }
            }
            Node::Path(path) => {
                check_path_id(id, path.path, paths.len())?;
                if let Some(fill) = path.fill {
                    check_paint_id(id, fill, paints.len())?;
                }
                if let Some(stroke) = &path.stroke {
                    check_paint_id(id, stroke.paint, paints.len())?;
                    finite_nonnegative(stroke.width, &format!("node[{index}].stroke.width"))?;
                    finite_f32(
                        stroke.miter_limit,
                        &format!("node[{index}].stroke.miterLimit"),
                    )?;
                    if stroke.width == 0.0 {
                        return invalid(
                            format!("node[{index}].stroke.width"),
                            "must be positive when a stroke is present",
                        );
                    }
                    if stroke.miter_limit < 1.0 {
                        return invalid(
                            format!("node[{index}].stroke.miterLimit"),
                            "must be at least 1",
                        );
                    }
                    validate_dash(
                        &stroke.dash,
                        stroke.dash_offset,
                        &format!("node[{index}].stroke"),
                    )?;
                }
            }
            Node::GeometryBatch(batch) => {
                batch_instances = batch_instances.saturating_add(batch.instances.len());
                for (instance_index, instance) in batch.instances.iter().enumerate() {
                    validate_point(
                        instance.position,
                        &format!("node[{index}].instances[{instance_index}].position"),
                    )?;
                    validate_point(
                        instance.size,
                        &format!("node[{index}].instances[{instance_index}].size"),
                    )?;
                    if instance.size[0] <= 0.0 || instance.size[1] <= 0.0 {
                        return invalid(
                            format!("node[{index}].instances[{instance_index}].size"),
                            "both dimensions must be positive",
                        );
                    }
                    validate_color(
                        instance.color,
                        &format!("node[{index}].instances[{instance_index}].color"),
                    )?;
                }
            }
            Node::Image(image) => {
                validate_texture(&image.texture, &format!("node[{index}].texture"))?;
                validate_normalized_rect(image.src, &format!("node[{index}].src"))?;
                validate_rect(image.dst, &format!("node[{index}].dst"))?;
                validate_unit(image.opacity, &format!("node[{index}].opacity"))?;
            }
            Node::GlyphRun(run) => {
                finite_f32(run.font_size, &format!("node[{index}].fontSize"))?;
                if run.font_size <= 0.0 {
                    return invalid(format!("node[{index}].fontSize"), "must be positive");
                }
                check_paint_id(id, run.paint, paints.len())?;
                if let Some(stroke) = &run.stroke {
                    check_paint_id(id, stroke.paint, paints.len())?;
                    validate_stroke(stroke, &format!("node[{index}].stroke"))?;
                }
                validate_rect(run.bounds, &format!("node[{index}].bounds"))?;
                if run.glyphs.is_empty() && !run.bounds.is_empty() {
                    return invalid(
                        format!("node[{index}].bounds"),
                        "must be empty exactly when the glyph run is empty",
                    );
                }
                glyphs = glyphs.saturating_add(run.glyphs.len());
                for (glyph_index, glyph) in run.glyphs.iter().enumerate() {
                    finite_f64(glyph.x, &format!("node[{index}].glyphs[{glyph_index}].x"))?;
                    finite_f64(glyph.y, &format!("node[{index}].glyphs[{glyph_index}].y"))?;
                }
                if run.source_node.is_some() != !run.source_ranges.is_empty() {
                    return invalid(
                        format!("node[{index}].sourceRanges"),
                        "source node and ranges must either both be present or both be absent",
                    );
                }
                if !run.source_ranges.is_empty() && run.source_ranges.len() != run.glyphs.len() {
                    return invalid(
                        format!("node[{index}].sourceRanges"),
                        "source ranges must be parallel to glyphs",
                    );
                }
                if let Some(source) = &run.source_node {
                    validate_key(source, &format!("node[{index}].sourceNode"))?;
                }
                for (range_index, [start, end]) in run.source_ranges.iter().copied().enumerate() {
                    if start > end {
                        return invalid(
                            format!("node[{index}].sourceRanges[{range_index}]"),
                            "start must not exceed end",
                        );
                    }
                }
            }
            Node::Shadow(shadow) => {
                validate_round_rect(shadow.shape, &format!("node[{index}].shape"))?;
                validate_point(
                    [f64::from(shadow.offset[0]), f64::from(shadow.offset[1])],
                    &format!("node[{index}].offset"),
                )?;
                finite_nonnegative(shadow.sigma_x, &format!("node[{index}].sigmaX"))?;
                finite_nonnegative(shadow.sigma_y, &format!("node[{index}].sigmaY"))?;
                finite_f32(shadow.spread, &format!("node[{index}].spread"))?;
                validate_color(shadow.color, &format!("node[{index}].color"))?;
            }
            Node::RuntimeShader(shader) => {
                validate_key(&shader.shader.uri, &format!("node[{index}].shader.uri"))?;
                validate_rect(shader.bounds, &format!("node[{index}].bounds"))?;
                validate_shader_bindings(
                    &shader.uniforms,
                    &shader.textures,
                    &format!("node[{index}]"),
                )?;
                shader_bindings = shader_bindings
                    .saturating_add(shader.uniforms.len())
                    .saturating_add(shader.textures.len());
                for (texture_index, texture) in shader.textures.iter().enumerate() {
                    validate_texture(
                        &texture.texture,
                        &format!("node[{index}].textures[{texture_index}].texture"),
                    )?;
                }
            }
            Node::Scene3d(scene) => {
                validate_rect(scene.bounds, &format!("node[{index}].bounds"))?;
            }
        }
    }
    budget("structural edges", edges, MAX_EDGES)?;
    budget("glyphs", glyphs, MAX_GLYPHS)?;
    budget("batch instances", batch_instances, MAX_BATCH_INSTANCES)?;
    budget("filters", filters, MAX_FILTERS)?;
    budget("shader bindings", shader_bindings, MAX_SHADER_BINDINGS)?;
    Ok(())
}

fn validate_group(
    owner: NodeId,
    group: &Group,
    node_count: usize,
    path_count: usize,
) -> Result<(), DrawProgramError> {
    validate_transform(group.transform, &format!("node[{}].transform", owner.raw()))?;
    for child in &group.children {
        check_node_id(Some(owner), *child, node_count)?;
    }
    if let Some(glass) = &group.glass {
        glass
            .derive_requirements()
            .map_err(|error| DrawProgramError::InvalidValue {
                location: format!("node[{}].glass", owner.raw()),
                reason: error.to_string(),
            })?;
        if group.backdrop.is_some() {
            return invalid(
                format!("node[{}]", owner.raw()),
                "glass and backdrop cannot both own the group destination read",
            );
        }
    }
    if let Some(foreground) = &group.glass_foreground {
        foreground
            .validate()
            .map_err(|error| DrawProgramError::InvalidValue {
                location: format!("node[{}].glassForeground", owner.raw()),
                reason: error.to_string(),
            })?;
    }
    if group.glass.is_some() && group.glass_foreground.is_some() {
        return invalid(
            format!("node[{}]", owner.raw()),
            "one group cannot be both a Glass material owner and foreground",
        );
    }
    match group.clip.as_ref() {
        Some(Clip::Rect(rect)) => validate_rect(*rect, &format!("node[{}].clip", owner.raw()))?,
        Some(Clip::RoundRect(round_rect)) => {
            validate_round_rect(*round_rect, &format!("node[{}].clip", owner.raw()))?
        }
        Some(Clip::Path { path, .. }) => check_path_id(owner, *path, path_count)?,
        None => {}
    }
    for (index, filter) in group.filters.iter().enumerate() {
        validate_filter(filter, &format!("node[{}].filters[{index}]", owner.raw()))?;
    }
    if let Some(mask) = &group.mask {
        check_node_id(Some(owner), mask.source, node_count)?;
    }
    validate_unit(group.opacity, &format!("node[{}].opacity", owner.raw()))?;
    if let Some(backdrop) = &group.backdrop {
        validate_scope(
            &backdrop.scope,
            &format!("node[{}].backdrop.scope", owner.raw()),
        )?;
        validate_rect(
            backdrop.bounds,
            &format!("node[{}].backdrop.bounds", owner.raw()),
        )?;
        validate_insets(
            backdrop.footprint,
            &format!("node[{}].backdrop.footprint", owner.raw()),
        )?;
        for (index, filter) in backdrop.filters.iter().enumerate() {
            validate_filter(
                filter,
                &format!("node[{}].backdrop.filters[{index}]", owner.raw()),
            )?;
        }
    }
    if let Some(shader) = &group.shader {
        validate_shader_layer(shader, &format!("node[{}].shader", owner.raw()))?;
    }
    if let Some(bounds) = group.layer_bounds {
        validate_rect(bounds, &format!("node[{}].layerBounds", owner.raw()))?;
    }
    Ok(())
}

fn validate_glass_topology(
    roots: &[NodeId],
    nodes: &[Option<Node>],
) -> Result<(), DrawProgramError> {
    fn has_local_effect(group: &Group) -> bool {
        group.transform != Transform2d::IDENTITY
            || group.clip.is_some()
            || !group.filters.is_empty()
            || group.mask.is_some()
            || group.opacity != 1.0
            || group.internal_blend != super::BlendMode::Normal
            || group.isolated
            || group.backdrop.is_some()
            || group.shader.is_some()
            || group.layer_bounds.is_some()
    }

    fn walk<'a>(
        id: NodeId,
        nodes: &'a [Option<Node>],
        active: Option<&'a super::MotionGlassProgram>,
        inside_foreground: bool,
        owners: &mut BTreeSet<String>,
        markers: &mut BTreeMap<String, BTreeSet<String>>,
    ) -> Result<(), DrawProgramError> {
        let node = nodes[id.index()]
            .as_ref()
            .expect("undefined nodes rejected before Glass topology validation");
        let Node::Group(group) = node else {
            if let Some(owner) = active
                && !inside_foreground
            {
                return invalid(
                    format!("node[{}]", id.raw()),
                    format!(
                        "Glass owner '{}' contains painted content outside a member foreground",
                        owner.owner_id
                    ),
                );
            }
            return Ok(());
        };

        if let Some(owner) = &group.glass {
            if active.is_some() && !inside_foreground {
                return invalid(
                    format!("node[{}].glass", id.raw()),
                    "a nested Glass material owner must live inside an outer member foreground",
                );
            }
            if has_local_effect(group) {
                return invalid(
                    format!("node[{}].glass", id.raw()),
                    "a Glass material owner group must not also carry transform, clip, filter, mask, opacity, blend, backdrop, shader, isolation, or layer bounds",
                );
            }
            if !owners.insert(owner.owner_id.clone()) {
                return invalid(
                    format!("node[{}].glass.ownerId", id.raw()),
                    "Glass material owner ids must be unique in one DrawProgram",
                );
            }
            markers.entry(owner.owner_id.clone()).or_default();
            for child in &group.children {
                walk(*child, nodes, Some(owner), false, owners, markers)?;
            }
            return Ok(());
        }

        if let Some(foreground) = &group.glass_foreground {
            let Some(owner) = active else {
                return invalid(
                    format!("node[{}].glassForeground", id.raw()),
                    "Glass foreground has no material owner ancestor",
                );
            };
            if inside_foreground {
                return invalid(
                    format!("node[{}].glassForeground", id.raw()),
                    "Glass foreground markers may not nest without a nested material owner",
                );
            }
            if has_local_effect(group) {
                return invalid(
                    format!("node[{}].glassForeground", id.raw()),
                    "a Glass foreground marker group must not carry unrelated group effects",
                );
            }
            if !foreground_matches_owner(foreground, owner) {
                return invalid(
                    format!("node[{}].glassForeground", id.raw()),
                    "Glass foreground is not the exact geometry and metadata of its material surface",
                );
            }
            let seen = markers
                .get_mut(&owner.owner_id)
                .expect("active owner registered before descendants");
            if !seen.insert(foreground.surface_id.clone()) {
                return invalid(
                    format!("node[{}].glassForeground.surfaceId", id.raw()),
                    "a Glass surface may own at most one foreground subtree",
                );
            }
            for child in &group.children {
                walk(*child, nodes, Some(owner), true, owners, markers)?;
            }
            return Ok(());
        }

        if active.is_some() && !inside_foreground {
            let paints_between = !group.filters.is_empty()
                || group.mask.is_some()
                || group.opacity != 1.0
                || group.internal_blend != super::BlendMode::Normal
                || group.isolated
                || group.backdrop.is_some()
                || group.shader.is_some();
            if paints_between {
                return invalid(
                    format!("node[{}]", id.raw()),
                    "an effect-bearing group cannot sit between a Glass material and its foreground",
                );
            }
        }
        for child in &group.children {
            walk(*child, nodes, active, inside_foreground, owners, markers)?;
        }
        if let Some(mask) = &group.mask {
            walk(
                mask.source,
                nodes,
                active,
                inside_foreground,
                owners,
                markers,
            )?;
        }
        Ok(())
    }

    let mut owners = BTreeSet::new();
    let mut markers = BTreeMap::new();
    for root in roots {
        walk(*root, nodes, None, false, &mut owners, &mut markers)?;
    }
    Ok(())
}

fn foreground_matches_owner(
    foreground: &super::MotionGlassForegroundProgram,
    owner: &super::MotionGlassProgram,
) -> bool {
    if foreground.owner_id != owner.owner_id
        || foreground.kernel_digest != owner.kernel_digest
        || foreground.schema_digest != owner.schema_digest
    {
        return false;
    }
    owner.surfaces.iter().any(|surface| {
        foreground.surface_id == surface.surface_id
            && foreground.shape == surface.shape
            && foreground.rect == surface.rect
            && foreground.local_to_owner == surface.local_to_owner
            && foreground.radius == surface.radius
            && foreground.presence == surface.presence
            && foreground.path_points == surface.path_points
    })
}

fn validate_stroke(stroke: &super::PathStroke, location: &str) -> Result<(), DrawProgramError> {
    finite_nonnegative(stroke.width, &format!("{location}.width"))?;
    if stroke.width == 0.0 {
        return invalid(format!("{location}.width"), "must be positive");
    }
    finite_f32(stroke.miter_limit, &format!("{location}.miterLimit"))?;
    if stroke.miter_limit < 1.0 {
        return invalid(format!("{location}.miterLimit"), "must be at least 1");
    }
    validate_dash(&stroke.dash, stroke.dash_offset, location)
}

fn validate_dash(dash: &[f32], offset: f32, location: &str) -> Result<(), DrawProgramError> {
    finite_f32(offset, &format!("{location}.dashOffset"))?;
    if dash.len() % 2 != 0 {
        return invalid(
            format!("{location}.dash"),
            "dash intervals must have even length",
        );
    }
    let mut total = 0.0f32;
    for (index, value) in dash.iter().copied().enumerate() {
        finite_nonnegative(value, &format!("{location}.dash[{index}]"))?;
        total += value;
    }
    if !dash.is_empty() && total <= 0.0 {
        return invalid(
            format!("{location}.dash"),
            "dash intervals must have positive total length",
        );
    }
    Ok(())
}

fn validate_round_rect(round_rect: RoundRect, location: &str) -> Result<(), DrawProgramError> {
    validate_rect(round_rect.rect, &format!("{location}.rect"))?;
    for (index, radii) in round_rect.radii.iter().copied().enumerate() {
        validate_point(radii, &format!("{location}.radii[{index}]"))?;
        if radii[0] < 0.0 || radii[1] < 0.0 {
            return invalid(
                format!("{location}.radii[{index}]"),
                "radii must be non-negative",
            );
        }
    }
    Ok(())
}

fn validate_transform(transform: Transform2d, location: &str) -> Result<(), DrawProgramError> {
    for (index, value) in transform.0.iter().copied().enumerate() {
        finite_f64(value, &format!("{location}[{index}]"))?;
    }
    if transform.inverse().is_none() {
        return invalid(location, "transform must be invertible");
    }
    Ok(())
}

fn validate_filter(filter: &Filter, location: &str) -> Result<(), DrawProgramError> {
    let validate_amount =
        |amount: f32, suffix: &str| finite_nonnegative(amount, &format!("{location}.{suffix}"));
    match filter {
        Filter::Blur { sigma_x, sigma_y } => {
            finite_nonnegative(*sigma_x, &format!("{location}.sigmaX"))?;
            finite_nonnegative(*sigma_y, &format!("{location}.sigmaY"))?;
            if *sigma_x > MAX_FILTER_SIGMA || *sigma_y > MAX_FILTER_SIGMA {
                return invalid(location, format!("blur sigma exceeds {MAX_FILTER_SIGMA}"));
            }
        }
        Filter::ColorMatrix { matrix } => {
            for (index, value) in matrix.iter().copied().enumerate() {
                finite_f32(value, &format!("{location}.matrix[{index}]"))?;
            }
        }
        Filter::Brightness { amount }
        | Filter::Contrast { amount }
        | Filter::Saturate { amount } => validate_amount(*amount, "amount")?,
        Filter::Grayscale { amount }
        | Filter::Invert { amount }
        | Filter::Opacity { amount }
        | Filter::Sepia { amount } => validate_unit(*amount, &format!("{location}.amount"))?,
        Filter::HueRotate { degrees } => finite_f32(*degrees, &format!("{location}.degrees"))?,
        Filter::DropShadow {
            offset,
            sigma_x,
            sigma_y,
            color,
        } => {
            finite_f32(offset[0], &format!("{location}.offset[0]"))?;
            finite_f32(offset[1], &format!("{location}.offset[1]"))?;
            finite_nonnegative(*sigma_x, &format!("{location}.sigmaX"))?;
            finite_nonnegative(*sigma_y, &format!("{location}.sigmaY"))?;
            if *sigma_x > MAX_FILTER_SIGMA || *sigma_y > MAX_FILTER_SIGMA {
                return invalid(
                    location,
                    format!("drop-shadow sigma exceeds {MAX_FILTER_SIGMA}"),
                );
            }
            validate_color(*color, &format!("{location}.color"))?;
        }
        Filter::NoiseDisplacement {
            frequency,
            octaves,
            scale,
            ..
        } => {
            finite_nonnegative(frequency[0], &format!("{location}.frequency[0]"))?;
            finite_nonnegative(frequency[1], &format!("{location}.frequency[1]"))?;
            if *octaves == 0 || *octaves > 8 {
                return invalid(format!("{location}.octaves"), "must be in 1..=8");
            }
            finite_nonnegative(*scale, &format!("{location}.scale"))?;
        }
        Filter::VelocityBlur {
            velocity,
            shutter_angle_degrees,
        } => {
            finite_f32(velocity[0], &format!("{location}.velocity[0]"))?;
            finite_f32(velocity[1], &format!("{location}.velocity[1]"))?;
            finite_nonnegative(
                *shutter_angle_degrees,
                &format!("{location}.shutterAngleDegrees"),
            )?;
            if *shutter_angle_degrees > 360.0 {
                return invalid(
                    format!("{location}.shutterAngleDegrees"),
                    "must not exceed 360",
                );
            }
        }
    }
    Ok(())
}

fn validate_shader_layer(shader: &ShaderLayer, location: &str) -> Result<(), DrawProgramError> {
    validate_key(&shader.shader.uri, &format!("{location}.shader.uri"))?;
    validate_rect(shader.bounds, &format!("{location}.bounds"))?;
    validate_shader_bindings(&shader.uniforms, &shader.textures, location)
}

fn validate_shader_bindings(
    uniforms: &[super::ShaderUniformBinding],
    textures: &[super::ShaderTextureBinding],
    location: &str,
) -> Result<(), DrawProgramError> {
    let mut names = BTreeSet::new();
    for (index, uniform) in uniforms.iter().enumerate() {
        validate_key(&uniform.name, &format!("{location}.uniforms[{index}].name"))?;
        if !names.insert(uniform.name.as_str()) {
            return invalid(
                format!("{location}.uniforms[{index}].name"),
                "binding names must be unique",
            );
        }
        match uniform.value {
            ShaderUniformValue::Float(value) => {
                finite_f32(value, &format!("{location}.uniforms[{index}].value"))?
            }
            ShaderUniformValue::Float2(value) => {
                finite_f32(value[0], &format!("{location}.uniforms[{index}].value[0]"))?;
                finite_f32(value[1], &format!("{location}.uniforms[{index}].value[1]"))?;
            }
            ShaderUniformValue::Color(color) => {
                validate_color(color, &format!("{location}.uniforms[{index}].value"))?
            }
            ShaderUniformValue::Bool(_) => {}
        }
    }
    for (index, texture) in textures.iter().enumerate() {
        validate_key(&texture.name, &format!("{location}.textures[{index}].name"))?;
        if !names.insert(texture.name.as_str()) {
            return invalid(
                format!("{location}.textures[{index}].name"),
                "binding names must be unique across uniforms and textures",
            );
        }
        validate_texture(
            &texture.texture,
            &format!("{location}.textures[{index}].texture"),
        )?;
    }
    Ok(())
}

fn validate_single_parents(
    roots: &[NodeId],
    nodes: &[Option<Node>],
) -> Result<(), DrawProgramError> {
    let mut incoming = vec![0u8; nodes.len()];
    for root in roots {
        increment_parent(&mut incoming, *root)?;
    }
    for node in nodes.iter().flatten() {
        if let Node::Group(group) = node {
            for child in &group.children {
                increment_parent(&mut incoming, *child)?;
            }
            if let Some(mask) = &group.mask {
                increment_parent(&mut incoming, mask.source)?;
            }
        }
    }
    Ok(())
}

fn increment_parent(incoming: &mut [u8], id: NodeId) -> Result<(), DrawProgramError> {
    let value = &mut incoming[id.index()];
    *value = value.saturating_add(1);
    if *value > 1 {
        Err(DrawProgramError::MultipleParents { id })
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
    New,
    Visiting,
    Done,
}

struct Canonicalizer<'a> {
    raw_roots: Vec<NodeId>,
    raw_nodes: &'a [Option<Node>],
    raw_paths: &'a [PathData],
    raw_paints: &'a [Paint],
    states: Vec<Visit>,
    node_map: Vec<Option<NodeId>>,
    path_map: Vec<Option<PathId>>,
    paint_map: Vec<Option<PaintId>>,
    nodes: Vec<Option<Node>>,
    paths: Vec<PathData>,
    paints: Vec<Paint>,
}

impl<'a> Canonicalizer<'a> {
    fn new(nodes: &'a [Option<Node>], paths: &'a [PathData], paints: &'a [Paint]) -> Self {
        Self {
            raw_roots: Vec::new(),
            raw_nodes: nodes,
            raw_paths: paths,
            raw_paints: paints,
            states: vec![Visit::New; nodes.len()],
            node_map: vec![None; nodes.len()],
            path_map: vec![None; paths.len()],
            paint_map: vec![None; paints.len()],
            nodes: Vec::with_capacity(nodes.len()),
            paths: Vec::with_capacity(paths.len()),
            paints: Vec::with_capacity(paints.len()),
        }
    }

    fn visit(&mut self, old: NodeId, group_depth: usize) -> Result<NodeId, DrawProgramError> {
        if group_depth == 0 {
            self.raw_roots.push(old);
        }
        match self.states[old.index()] {
            Visit::Visiting => return Err(DrawProgramError::Cycle { at: old }),
            Visit::Done => return Ok(self.node_map[old.index()].expect("done node has mapping")),
            Visit::New => {}
        }

        self.states[old.index()] = Visit::Visiting;
        let canonical_id = NodeId::from_index(self.nodes.len());
        self.node_map[old.index()] = Some(canonical_id);
        self.nodes.push(None);

        let raw = self.raw_nodes[old.index()]
            .as_ref()
            .expect("undefined nodes rejected before canonicalization")
            .clone();
        let rewritten = match raw {
            Node::Group(mut group) => {
                let next_depth = group_depth + 1;
                if next_depth > MAX_GROUP_DEPTH {
                    return Err(DrawProgramError::DepthLimit {
                        depth: next_depth,
                        limit: MAX_GROUP_DEPTH,
                    });
                }
                let mut children = Vec::with_capacity(group.children.len());
                for child in group.children {
                    children.push(self.visit(child, next_depth)?);
                }
                group.children = children;
                if let Some(Clip::Path { path, .. }) = &mut group.clip {
                    *path = self.remap_path(*path);
                }
                if let Some(mask) = &mut group.mask {
                    mask.source = self.visit(mask.source, next_depth)?;
                }
                Node::Group(group)
            }
            Node::Path(mut path) => {
                path.path = self.remap_path(path.path);
                path.fill = path.fill.map(|paint| self.remap_paint(paint));
                path.stroke = path.stroke.map(|mut stroke| {
                    stroke.paint = self.remap_paint(stroke.paint);
                    stroke
                });
                Node::Path(path)
            }
            Node::GlyphRun(mut run) => {
                run.paint = self.remap_paint(run.paint);
                if let Some(stroke) = &mut run.stroke {
                    stroke.paint = self.remap_paint(stroke.paint);
                }
                Node::GlyphRun(run)
            }
            leaf => leaf,
        };
        self.nodes[canonical_id.index()] = Some(rewritten);
        self.states[old.index()] = Visit::Done;
        Ok(canonical_id)
    }

    fn remap_path(&mut self, old: PathId) -> PathId {
        if let Some(mapped) = self.path_map[old.index()] {
            return mapped;
        }
        let mapped = PathId::from_index(self.paths.len());
        self.path_map[old.index()] = Some(mapped);
        self.paths.push(self.raw_paths[old.index()].clone());
        mapped
    }

    fn remap_paint(&mut self, old: PaintId) -> PaintId {
        if let Some(mapped) = self.paint_map[old.index()] {
            return mapped;
        }
        let mapped = PaintId::from_index(self.paints.len());
        self.paint_map[old.index()] = Some(mapped);
        self.paints.push(self.raw_paints[old.index()].clone());
        mapped
    }
}

fn derive_requirements(
    program: &DrawProgram,
    node_geometry: &[NodeGeometry],
) -> Result<DrawRequirements, DrawProgramError> {
    let mut requirements = DrawRequirements::default();
    let mut textures = BTreeMap::<(String, Option<i64>), ExternalTexture>::new();
    let mut texture_formats = BTreeMap::<String, (TextureKind, ColorDomain, AlphaMode)>::new();
    let mut fonts = BTreeSet::new();
    let mut shaders = BTreeMap::new();
    let mut scenes = BTreeSet::new();
    let mut color_domains = BTreeSet::from([ColorDomain::LinearRec2020]);
    let mut alpha_modes = BTreeSet::from([AlphaMode::Premultiplied]);
    let mut capabilities = BTreeSet::new();

    for node in &program.nodes {
        match node {
            Node::Group(group) => {
                if group.transform != Transform2d::IDENTITY {
                    let affine = group.transform.0[6] == 0.0
                        && group.transform.0[7] == 0.0
                        && group.transform.0[8] == 1.0;
                    capabilities.insert(if affine {
                        crate::requirements::DrawCapability::TransformAffine
                    } else {
                        crate::requirements::DrawCapability::TransformProjective
                    });
                }
                if let Some(clip) = &group.clip {
                    capabilities.insert(match clip {
                        Clip::Rect(_) => crate::requirements::DrawCapability::ClipRect,
                        Clip::RoundRect(_) => crate::requirements::DrawCapability::ClipRoundRect,
                        Clip::Path { .. } => crate::requirements::DrawCapability::ClipPath,
                    });
                }
                for filter in group.filters.iter().chain(
                    group
                        .backdrop
                        .iter()
                        .flat_map(|backdrop| backdrop.filters.iter()),
                ) {
                    insert_filter_capability(filter, &mut capabilities);
                }
                if let Some(mask) = &group.mask {
                    capabilities.insert(match mask.mode {
                        super::MaskMode::Alpha => crate::requirements::DrawCapability::MaskAlpha,
                        super::MaskMode::Luminance => {
                            crate::requirements::DrawCapability::MaskLuminance
                        }
                    });
                }
                if group.backdrop.is_some() {
                    capabilities.insert(crate::requirements::DrawCapability::BackdropRead);
                }
                if group.glass.is_some() {
                    capabilities.insert(crate::requirements::DrawCapability::BackdropRead);
                    capabilities.insert(crate::requirements::DrawCapability::MotionGlass);
                }
                if group.glass_foreground.is_some() {
                    capabilities.insert(crate::requirements::DrawCapability::MotionGlass);
                }
                if group.internal_blend != super::BlendMode::Normal {
                    capabilities.insert(crate::requirements::DrawCapability::Blend);
                }
                if let Some(shader) = &group.shader {
                    insert_shader_requirement(&mut shaders, &shader.shader)?;
                    for texture in &shader.textures {
                        insert_texture(&mut textures, &mut texture_formats, &texture.texture)?;
                        color_domains.insert(texture.texture.color_domain);
                        alpha_modes.insert(texture.texture.alpha);
                    }
                    capabilities.insert(crate::requirements::DrawCapability::GroupShader);
                }
            }
            Node::Image(image) => {
                insert_texture(&mut textures, &mut texture_formats, &image.texture)?;
                color_domains.insert(image.texture.color_domain);
                alpha_modes.insert(image.texture.alpha);
                capabilities.insert(crate::requirements::DrawCapability::ExternalTexture);
            }
            Node::GeometryBatch(_) => {
                capabilities.insert(crate::requirements::DrawCapability::GeometryBatch);
            }
            Node::GlyphRun(run) => {
                fonts.insert(run.font.clone());
                capabilities.insert(crate::requirements::DrawCapability::FontGlyphs);
                if run
                    .stroke
                    .as_ref()
                    .is_some_and(|stroke| !stroke.dash.is_empty())
                {
                    capabilities.insert(crate::requirements::DrawCapability::StrokeDash);
                }
            }
            Node::Shadow(_) => {
                capabilities.insert(crate::requirements::DrawCapability::BoxShadow);
            }
            Node::RuntimeShader(shader) => {
                insert_shader_requirement(&mut shaders, &shader.shader)?;
                for texture in &shader.textures {
                    insert_texture(&mut textures, &mut texture_formats, &texture.texture)?;
                    color_domains.insert(texture.texture.color_domain);
                    alpha_modes.insert(texture.texture.alpha);
                }
                capabilities.insert(crate::requirements::DrawCapability::RuntimeShader);
            }
            Node::Scene3d(scene) => {
                scenes.insert(scene.scene.clone());
                capabilities.insert(crate::requirements::DrawCapability::Scene3d);
            }
            Node::Path(path) => {
                if path
                    .stroke
                    .as_ref()
                    .is_some_and(|stroke| !stroke.dash.is_empty())
                {
                    capabilities.insert(crate::requirements::DrawCapability::StrokeDash);
                }
            }
        }
    }

    for paint in &program.paints {
        match paint {
            Paint::ConicGradient { .. } => {
                capabilities.insert(crate::requirements::DrawCapability::GradientConic);
            }
            Paint::LinearGradient { spread, .. }
            | Paint::RadialGradient { spread, .. }
            | Paint::TwoCircleGradient { spread, .. }
                if *spread != super::SpreadMode::Pad =>
            {
                capabilities.insert(crate::requirements::DrawCapability::GradientSpread);
            }
            _ => {}
        }
    }

    requirements.external_textures = textures.into_values().collect();
    requirements.fonts = fonts.into_iter().collect();
    requirements.runtime_shaders = shaders.into_values().collect();
    requirements.scene3d = scenes.into_iter().collect();
    requirements.destination_uses = derive_destination_uses(program, node_geometry)?;
    requirements.filter_footprint = derive_filter_footprint(program);
    requirements.color_domains = color_domains.into_iter().collect();
    requirements.alpha_modes = alpha_modes.into_iter().collect();
    requirements.capabilities = capabilities.into_iter().collect();
    let geometry = derive_program_geometry(program, node_geometry);
    requirements.content_bounds = geometry.content_bounds;
    requirements.output_bounds = geometry.output_bounds;
    requirements.max_intermediate_pixels = geometry.max_intermediate_pixels;
    Ok(requirements)
}

fn insert_filter_capability(
    filter: &Filter,
    capabilities: &mut BTreeSet<crate::requirements::DrawCapability>,
) {
    use crate::requirements::DrawCapability;
    capabilities.insert(match filter {
        Filter::Blur { .. } => DrawCapability::FilterBlur,
        Filter::ColorMatrix { .. }
        | Filter::Brightness { .. }
        | Filter::Contrast { .. }
        | Filter::Grayscale { .. }
        | Filter::HueRotate { .. }
        | Filter::Invert { .. }
        | Filter::Opacity { .. }
        | Filter::Saturate { .. }
        | Filter::Sepia { .. } => DrawCapability::FilterColorMatrix,
        Filter::DropShadow { .. } => DrawCapability::FilterDropShadow,
        Filter::NoiseDisplacement { .. } => DrawCapability::FilterNoiseDisplacement,
        Filter::VelocityBlur { .. } => DrawCapability::FilterVelocityBlur,
    });
}

fn insert_shader_requirement(
    shaders: &mut BTreeMap<String, crate::requirements::RuntimeShaderKey>,
    shader: &crate::requirements::RuntimeShaderKey,
) -> Result<(), DrawProgramError> {
    if let Some(existing) = shaders.insert(shader.uri.clone(), shader.clone())
        && existing != *shader
    {
        return Err(DrawProgramError::ConflictingRequirement {
            kind: "runtime shader".into(),
            key: shader.uri.clone(),
        });
    }
    Ok(())
}

fn derive_destination_uses(
    program: &DrawProgram,
    geometry: &[NodeGeometry],
) -> Result<Vec<DestinationUse>, DrawProgramError> {
    fn visit(
        program: &DrawProgram,
        geometry: &[NodeGeometry],
        id: NodeId,
        entry_is_external: bool,
        parent_to_program: Transform2d,
        uses: &mut Vec<DestinationUse>,
    ) -> Result<(), DrawProgramError> {
        let Node::Group(group) = &program.nodes[id.index()] else {
            return Ok(());
        };
        let child_entry_is_external = entry_is_external && !group.isolated;
        let local_to_program = group.transform.then(parent_to_program);
        let group_is_visible = geometry[id.index()].output_bounds.rect().is_some();
        if let Some(glass) = &group.glass
            && entry_is_external
            && group_is_visible
        {
            let footprint = glass_footprint(glass);
            uses.push(DestinationUse {
                node: id,
                scope: glass.backdrop.scope.clone(),
                output_bounds: map_required_bounds(glass.backdrop.output_bounds, local_to_program)?,
                sample_bounds: map_required_bounds(glass.backdrop.sample_bounds, local_to_program)?,
                operation: DestinationOperation::Backdrop {
                    footprint,
                    sampling: SamplingMode::LinearClamp,
                },
            });
        }
        if let Some(backdrop) = &group.backdrop {
            let scope_is_external = match backdrop.scope {
                BackdropScope::Current => child_entry_is_external,
                BackdropScope::LayerEntry(_) | BackdropScope::ScopeEntry(_) => true,
            };
            if scope_is_external && group_is_visible && !backdrop.bounds.is_empty() {
                uses.push(DestinationUse {
                    node: id,
                    scope: backdrop.scope.clone(),
                    output_bounds: map_required_bounds(backdrop.bounds, local_to_program)?,
                    sample_bounds: map_required_bounds(
                        outset_rect(backdrop.bounds, backdrop.footprint)?,
                        local_to_program,
                    )?,
                    operation: DestinationOperation::Backdrop {
                        footprint: backdrop.footprint,
                        sampling: backdrop.sampling,
                    },
                });
            }
        }
        for child in &group.children {
            visit(
                program,
                geometry,
                *child,
                child_entry_is_external,
                local_to_program,
                uses,
            )?;
        }
        if let Some(mask) = &group.mask {
            // Mask sources are local coverage programs: Current begins transparent, while an
            // explicit LayerEntry/ScopeEntry still remains an external destination dependency.
            visit(
                program,
                geometry,
                mask.source,
                false,
                local_to_program,
                uses,
            )?;
        }
        if group.internal_blend != super::BlendMode::Normal
            && entry_is_external
            && let Some(bounds) = geometry[id.index()].output_bounds.rect()
        {
            uses.push(DestinationUse {
                node: id,
                scope: BackdropScope::Current,
                output_bounds: map_required_bounds(bounds, parent_to_program)?,
                sample_bounds: map_required_bounds(bounds, parent_to_program)?,
                operation: DestinationOperation::Blend {
                    mode: group.internal_blend,
                },
            });
        }
        Ok(())
    }

    let mut uses = Vec::new();
    for root in &program.roots {
        visit(
            program,
            geometry,
            *root,
            true,
            Transform2d::IDENTITY,
            &mut uses,
        )?;
    }
    Ok(uses)
}

fn map_required_bounds(bounds: Rect, transform: Transform2d) -> Result<Rect, DrawProgramError> {
    let mapped = transform
        .map_bounds(bounds)
        .ok_or_else(|| DrawProgramError::InvalidValue {
            location: "destinationUses".into(),
            reason: "destination bounds cross or touch a projective vanishing line".into(),
        })?;
    validate_rect(mapped, "destinationUses")?;
    Ok(mapped)
}

fn glass_footprint(glass: &super::MotionGlassProgram) -> Insets {
    let output = glass.backdrop.output_bounds;
    let sample = glass.backdrop.sample_bounds;
    Insets::new(
        (output.left() - sample.left()).max(0.0) as f32,
        (output.top() - sample.top()).max(0.0) as f32,
        (sample.right() - output.right()).max(0.0) as f32,
        (sample.bottom() - output.bottom()).max(0.0) as f32,
    )
}

fn derive_program_geometry(program: &DrawProgram, nodes: &[NodeGeometry]) -> NodeGeometry {
    let mut geometry = NodeGeometry::EMPTY;
    for root in &program.roots {
        let root = nodes[root.index()];
        geometry.content_bounds = union_bounds(geometry.content_bounds, root.content_bounds);
        geometry.output_bounds = union_bounds(geometry.output_bounds, root.output_bounds);
        geometry.max_intermediate_pixels = geometry
            .max_intermediate_pixels
            .max(root.max_intermediate_pixels);
    }
    geometry
}

fn derive_node_geometries(program: &DrawProgram) -> Result<Vec<NodeGeometry>, DrawProgramError> {
    let mut geometry = vec![NodeGeometry::EMPTY; program.nodes.len()];
    for root in &program.roots {
        derive_node_geometry(program, *root, &mut geometry, None)?;
    }
    Ok(geometry)
}

fn derive_node_geometry(
    program: &DrawProgram,
    id: NodeId,
    geometry: &mut [NodeGeometry],
    glass_owner_to_parent: Option<Transform2d>,
) -> Result<NodeGeometry, DrawProgramError> {
    let leaf = |bounds: Rect| NodeGeometry {
        content_bounds: LocalBounds::from_rect(bounds),
        output_bounds: LocalBounds::from_rect(bounds),
        max_intermediate_pixels: 0,
    };
    let result = match &program.nodes[id.index()] {
        Node::Path(node) => {
            if node.fill.is_none() && node.stroke.is_none() {
                NodeGeometry::EMPTY
            } else if let Some(mut bounds) = path_bounds(&program.paths[node.path.index()]) {
                if let Some(stroke) = &node.stroke {
                    let join_scale = match stroke.join {
                        super::StrokeJoin::Miter => f64::from(stroke.miter_limit),
                        super::StrokeJoin::Round | super::StrokeJoin::Bevel => 1.0,
                    };
                    let outset = f64::from(stroke.width) * 0.5 * join_scale;
                    bounds = outset_rect(bounds, Insets::uniform(outset as f32))?;
                }
                leaf(bounds)
            } else {
                NodeGeometry::EMPTY
            }
        }
        Node::GeometryBatch(node) => {
            let mut bounds = LocalBounds::Empty;
            for instance in &node.instances {
                let rect = match node.geometry {
                    super::BatchGeometry::Circle => Rect::new(
                        instance.position[0] - instance.size[0] * 0.5,
                        instance.position[1] - instance.size[1] * 0.5,
                        instance.size[0],
                        instance.size[1],
                    ),
                    super::BatchGeometry::Rect => Rect::new(
                        instance.position[0],
                        instance.position[1],
                        instance.size[0],
                        instance.size[1],
                    ),
                };
                bounds = union_bounds(bounds, LocalBounds::from_rect(rect));
            }
            bounds.rect().map_or(NodeGeometry::EMPTY, leaf)
        }
        Node::Image(node) => {
            if node.opacity == 0.0 {
                NodeGeometry::EMPTY
            } else {
                leaf(node.dst)
            }
        }
        Node::GlyphRun(node) => {
            let mut bounds = node.bounds;
            if let Some(stroke) = &node.stroke {
                let join_scale = match stroke.join {
                    super::StrokeJoin::Miter => f64::from(stroke.miter_limit),
                    super::StrokeJoin::Round | super::StrokeJoin::Bevel => 1.0,
                };
                bounds = outset_rect(
                    bounds,
                    Insets::uniform((f64::from(stroke.width) * 0.5 * join_scale) as f32),
                )?;
            }
            leaf(bounds)
        }
        Node::Shadow(node) => {
            if node.color.alpha == 0.0 {
                NodeGeometry::EMPTY
            } else if node.inset {
                leaf(node.shape.rect)
            } else {
                let blur_x = FILTER_GAUSSIAN_SUPPORT_SIGMAS * node.sigma_x;
                let blur_y = FILTER_GAUSSIAN_SUPPORT_SIGMAS * node.sigma_y;
                let spread = node.spread.max(0.0);
                leaf(Rect::from_edges(
                    node.shape.rect.left() + f64::from(node.offset[0] - spread - blur_x),
                    node.shape.rect.top() + f64::from(node.offset[1] - spread - blur_y),
                    node.shape.rect.right() + f64::from(node.offset[0] + spread + blur_x),
                    node.shape.rect.bottom() + f64::from(node.offset[1] + spread + blur_y),
                ))
            }
        }
        Node::RuntimeShader(node) => leaf(node.bounds),
        Node::Scene3d(node) => leaf(node.bounds),
        Node::Group(group) => {
            let glass_owner_to_group = if group.glass.is_some() {
                Some(Transform2d::IDENTITY)
            } else if let Some(owner_to_parent) = glass_owner_to_parent {
                let Some(parent_to_group) = group.transform.inverse() else {
                    return invalid(
                        format!("node[{}].transform", id.raw()),
                        "Glass foreground ancestry contains a singular transform",
                    );
                };
                Some(owner_to_parent.then(parent_to_group))
            } else {
                None
            };
            let mut result = NodeGeometry::EMPTY;
            let mut peak_local_pixels = 0;
            for child in &group.children {
                let child = derive_node_geometry(program, *child, geometry, glass_owner_to_group)?;
                result.content_bounds = union_bounds(result.content_bounds, child.content_bounds);
                result.output_bounds = union_bounds(result.output_bounds, child.output_bounds);
                result.max_intermediate_pixels = result
                    .max_intermediate_pixels
                    .max(child.max_intermediate_pixels);
            }
            if let Some(glass) = &group.glass {
                let output = LocalBounds::from_rect(glass.backdrop.output_bounds);
                result.content_bounds = union_bounds(result.content_bounds, output);
                result.output_bounds = union_bounds(result.output_bounds, output);
                peak_local_pixels = peak_local_pixels.max(bounds_pixels(LocalBounds::from_rect(
                    glass.backdrop.sample_bounds,
                ))?);
            }
            if let Some(foreground) = &group.glass_foreground {
                let bounds = if foreground.presence == 0.0 {
                    LocalBounds::Empty
                } else {
                    let Some(owner_to_group) = glass_owner_to_group else {
                        return invalid(
                            format!("node[{}].glassForeground", id.raw()),
                            "Glass foreground has no material owner coordinate space",
                        );
                    };
                    let surface_to_group =
                        Transform2d(foreground.local_to_owner.map(f64::from)).then(owner_to_group);
                    let Some(bounds) = surface_to_group.map_bounds(foreground.rect) else {
                        return invalid(
                            format!("node[{}].glassForeground", id.raw()),
                            "Glass foreground shape crosses a projective horizon",
                        );
                    };
                    LocalBounds::from_rect(bounds)
                };
                result.content_bounds = intersect_bounds(result.content_bounds, bounds);
                result.output_bounds = intersect_bounds(result.output_bounds, bounds);
            }
            if let Some(backdrop) = &group.backdrop {
                let bounds = LocalBounds::from_rect(backdrop.bounds);
                result.content_bounds = union_bounds(result.content_bounds, bounds);
                result.output_bounds = union_bounds(result.output_bounds, bounds);
                peak_local_pixels = peak_local_pixels.max(bounds_pixels(LocalBounds::from_rect(
                    outset_rect(backdrop.bounds, backdrop.footprint)?,
                ))?);
            }
            if let Some(clip) = &group.clip {
                let clip = match clip {
                    Clip::Rect(rect) => LocalBounds::from_rect(*rect),
                    Clip::RoundRect(round_rect) => LocalBounds::from_rect(round_rect.rect),
                    Clip::Path { path, .. } => path_bounds(&program.paths[path.index()])
                        .map_or(LocalBounds::Empty, LocalBounds::from_rect),
                };
                result.content_bounds = intersect_bounds(result.content_bounds, clip);
                result.output_bounds = intersect_bounds(result.output_bounds, clip);
            }
            for filter in &group.filters {
                result.output_bounds = apply_filter_bounds(result.output_bounds, filter)?;
                peak_local_pixels = peak_local_pixels.max(bounds_pixels(result.output_bounds)?);
            }
            peak_local_pixels = peak_local_pixels.max(bounds_pixels(result.output_bounds)?);
            if let Some(mask) = &group.mask {
                let mask =
                    derive_node_geometry(program, mask.source, geometry, glass_owner_to_group)?;
                result.output_bounds = intersect_bounds(result.output_bounds, mask.output_bounds);
                result.max_intermediate_pixels = result
                    .max_intermediate_pixels
                    .max(mask.max_intermediate_pixels);
            }
            if group.opacity == 0.0 {
                result.output_bounds = LocalBounds::Empty;
            }
            if let Some(shader) = &group.shader {
                result.output_bounds = if result.output_bounds.rect().is_some() {
                    LocalBounds::from_rect(shader.bounds)
                } else {
                    LocalBounds::Empty
                };
                peak_local_pixels = peak_local_pixels.max(bounds_pixels(result.output_bounds)?);
            }
            if group.transform != Transform2d::IDENTITY {
                result.content_bounds = transform_bounds(result.content_bounds, group.transform)?;
                result.output_bounds = transform_bounds(result.output_bounds, group.transform)?;
            }
            let needs_intermediate = group.isolated
                || group.transform != Transform2d::IDENTITY
                || group.opacity != 1.0
                || group.internal_blend != super::BlendMode::Normal
                || !group.filters.is_empty()
                || group.mask.is_some()
                || group.backdrop.is_some()
                || group.glass.is_some()
                || group.glass_foreground.is_some()
                || group.shader.is_some();
            if needs_intermediate {
                result.max_intermediate_pixels = result
                    .max_intermediate_pixels
                    .max(peak_local_pixels)
                    .max(bounds_pixels(result.output_bounds)?);
            }
            if result.max_intermediate_pixels > MAX_LOCAL_INTERMEDIATE_PIXELS {
                return Err(DrawProgramError::BudgetExceeded {
                    kind: "local intermediate pixels".into(),
                    actual: usize::try_from(result.max_intermediate_pixels).unwrap_or(usize::MAX),
                    limit: usize::try_from(MAX_LOCAL_INTERMEDIATE_PIXELS).unwrap_or(usize::MAX),
                });
            }
            result
        }
    };
    geometry[id.index()] = result;
    Ok(result)
}

fn apply_filter_bounds(
    bounds: LocalBounds,
    filter: &Filter,
) -> Result<LocalBounds, DrawProgramError> {
    let Some(rect) = bounds.rect() else {
        return Ok(LocalBounds::Empty);
    };
    let expanded = match filter {
        Filter::Blur { sigma_x, sigma_y } => outset_rect(
            rect,
            Insets::new(
                FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_x,
                FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_y,
                FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_x,
                FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_y,
            ),
        )?,
        Filter::DropShadow {
            offset,
            sigma_x,
            sigma_y,
            ..
        } => {
            let shadow = Rect::from_edges(
                rect.left() + f64::from(offset[0] - FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_x),
                rect.top() + f64::from(offset[1] - FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_y),
                rect.right() + f64::from(offset[0] + FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_x),
                rect.bottom() + f64::from(offset[1] + FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_y),
            );
            union_bounds(LocalBounds::from_rect(rect), LocalBounds::from_rect(shadow))
                .rect()
                .unwrap_or(rect)
        }
        Filter::NoiseDisplacement { scale, .. } => outset_rect(rect, Insets::uniform(*scale))?,
        Filter::VelocityBlur {
            velocity,
            shutter_angle_degrees,
        } => {
            let scale = *shutter_angle_degrees / 360.0;
            let dx = velocity[0] * scale;
            let dy = velocity[1] * scale;
            Rect::from_edges(
                rect.left() + f64::from(dx.min(0.0)),
                rect.top() + f64::from(dy.min(0.0)),
                rect.right() + f64::from(dx.max(0.0)),
                rect.bottom() + f64::from(dy.max(0.0)),
            )
        }
        Filter::ColorMatrix { .. }
        | Filter::Brightness { .. }
        | Filter::Contrast { .. }
        | Filter::Grayscale { .. }
        | Filter::HueRotate { .. }
        | Filter::Invert { .. }
        | Filter::Opacity { .. }
        | Filter::Saturate { .. }
        | Filter::Sepia { .. } => rect,
    };
    Ok(LocalBounds::from_rect(expanded))
}

fn transform_bounds(
    bounds: LocalBounds,
    transform: Transform2d,
) -> Result<LocalBounds, DrawProgramError> {
    let Some(rect) = bounds.rect() else {
        return Ok(LocalBounds::Empty);
    };
    let mapped = transform
        .map_bounds(rect)
        .ok_or_else(|| DrawProgramError::InvalidValue {
            location: "derivedBounds".into(),
            reason: "projective transform crosses or touches its vanishing line".into(),
        })?;
    validate_rect(mapped, "derivedBounds")?;
    Ok(LocalBounds::from_rect(mapped))
}

fn path_bounds(path: &PathData) -> Option<Rect> {
    let first = *path.points.first()?;
    let mut left = first[0];
    let mut top = first[1];
    let mut right = first[0];
    let mut bottom = first[1];
    for [x, y] in path.points.iter().copied().skip(1) {
        left = left.min(x);
        top = top.min(y);
        right = right.max(x);
        bottom = bottom.max(y);
    }
    Some(Rect::from_edges(left, top, right, bottom))
}

fn union_bounds(left: LocalBounds, right: LocalBounds) -> LocalBounds {
    match (left.rect(), right.rect()) {
        (None, None) => LocalBounds::Empty,
        (Some(rect), None) | (None, Some(rect)) => LocalBounds::Finite { rect },
        (Some(left), Some(right)) => LocalBounds::from_rect(Rect::from_edges(
            left.left().min(right.left()),
            left.top().min(right.top()),
            left.right().max(right.right()),
            left.bottom().max(right.bottom()),
        )),
    }
}

fn intersect_bounds(left: LocalBounds, right: LocalBounds) -> LocalBounds {
    let (Some(left), Some(right)) = (left.rect(), right.rect()) else {
        return LocalBounds::Empty;
    };
    LocalBounds::from_rect(Rect::from_edges(
        left.left().max(right.left()),
        left.top().max(right.top()),
        left.right().min(right.right()),
        left.bottom().min(right.bottom()),
    ))
}

fn outset_rect(rect: Rect, insets: Insets) -> Result<Rect, DrawProgramError> {
    let result = Rect::from_edges(
        rect.left() - f64::from(insets.left),
        rect.top() - f64::from(insets.top),
        rect.right() + f64::from(insets.right),
        rect.bottom() + f64::from(insets.bottom),
    );
    validate_rect(result, "derivedBounds")?;
    Ok(result)
}

fn bounds_pixels(bounds: LocalBounds) -> Result<u64, DrawProgramError> {
    let Some(rect) = bounds.rect() else {
        return Ok(0);
    };
    let width = rect.width.ceil();
    let height = rect.height.ceil();
    if width > u64::MAX as f64 || height > u64::MAX as f64 {
        return invalid("derivedBounds", "pixel area exceeds u64");
    }
    (width as u64)
        .checked_mul(height as u64)
        .ok_or_else(|| DrawProgramError::InvalidValue {
            location: "derivedBounds".into(),
            reason: "pixel area overflows u64".into(),
        })
}

fn derive_filter_footprint(program: &DrawProgram) -> Insets {
    fn visit(program: &DrawProgram, node: NodeId, inherited: Insets, maximum: &mut Insets) {
        let Node::Group(group) = &program.nodes[node.index()] else {
            maximum.include(inherited);
            return;
        };
        let local = group
            .filters
            .iter()
            .fold(Insets::default(), |outset, filter| {
                outset.added(filter_footprint(filter))
            });
        let cumulative = inherited.added(local);
        maximum.include(cumulative);
        if let Some(glass) = &group.glass {
            maximum.include(cumulative.added(glass_footprint(glass)));
        }
        for child in &group.children {
            visit(program, *child, cumulative, maximum);
        }
        if let Some(mask) = &group.mask {
            visit(program, mask.source, cumulative, maximum);
        }
    }

    let mut maximum = Insets::default();
    for root in &program.roots {
        visit(program, *root, Insets::default(), &mut maximum);
    }
    maximum
}

fn filter_footprint(filter: &Filter) -> Insets {
    match filter {
        Filter::Blur { sigma_x, sigma_y } => Insets::new(
            FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_x,
            FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_y,
            FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_x,
            FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_y,
        ),
        Filter::DropShadow {
            offset,
            sigma_x,
            sigma_y,
            ..
        } => Insets::new(
            (FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_x - offset[0]).max(0.0),
            (FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_y - offset[1]).max(0.0),
            (FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_x + offset[0]).max(0.0),
            (FILTER_GAUSSIAN_SUPPORT_SIGMAS * *sigma_y + offset[1]).max(0.0),
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
        Filter::ColorMatrix { .. }
        | Filter::Brightness { .. }
        | Filter::Contrast { .. }
        | Filter::Grayscale { .. }
        | Filter::HueRotate { .. }
        | Filter::Invert { .. }
        | Filter::Opacity { .. }
        | Filter::Saturate { .. }
        | Filter::Sepia { .. } => Insets::default(),
    }
}

fn insert_texture(
    textures: &mut BTreeMap<(String, Option<i64>), ExternalTexture>,
    formats: &mut BTreeMap<String, (TextureKind, ColorDomain, AlphaMode)>,
    texture: &ExternalTexture,
) -> Result<(), DrawProgramError> {
    let format = (texture.kind, texture.color_domain, texture.alpha);
    if formats
        .insert(texture.key.clone(), format)
        .is_some_and(|existing| existing != format)
    {
        return Err(DrawProgramError::ConflictingRequirement {
            kind: "external texture".into(),
            key: texture.key.clone(),
        });
    }
    let identity = (texture.key.clone(), texture.sample_time_micros);
    if let Some(existing) = textures.insert(identity, texture.clone())
        && existing != *texture
    {
        return Err(DrawProgramError::ConflictingRequirement {
            kind: "external texture".into(),
            key: texture.key.clone(),
        });
    }
    Ok(())
}

fn validate_path(path: &PathData, index: usize) -> Result<(), DrawProgramError> {
    let expected_points: usize = path.verbs.iter().map(|verb| verb.point_count()).sum();
    if expected_points != path.points.len() {
        return invalid(
            format!("paths[{index}].points"),
            format!(
                "verb arity requires {expected_points}, got {}",
                path.points.len()
            ),
        );
    }
    for (point_index, [x, y]) in path.points.iter().copied().enumerate() {
        finite_f64(x, &format!("paths[{index}].points[{point_index}].x"))?;
        finite_f64(y, &format!("paths[{index}].points[{point_index}].y"))?;
    }
    let mut subpath_open = false;
    for (verb_index, verb) in path.verbs.iter().enumerate() {
        match verb {
            super::PathVerb::MoveTo => subpath_open = true,
            super::PathVerb::LineTo | super::PathVerb::QuadTo | super::PathVerb::CubicTo
                if !subpath_open =>
            {
                return invalid(
                    format!("paths[{index}].verbs[{verb_index}]"),
                    "drawing verb requires a preceding moveTo",
                );
            }
            super::PathVerb::Close if !subpath_open => {
                return invalid(
                    format!("paths[{index}].verbs[{verb_index}]"),
                    "close requires an open subpath",
                );
            }
            super::PathVerb::Close => subpath_open = false,
            _ => {}
        }
    }
    Ok(())
}

fn validate_paint(paint: &Paint, index: usize) -> Result<usize, DrawProgramError> {
    match paint {
        Paint::Solid(color) => {
            validate_color(*color, &format!("paints[{index}]"))?;
            Ok(0)
        }
        Paint::LinearGradient {
            start, end, stops, ..
        } => {
            validate_point(*start, &format!("paints[{index}].start"))?;
            validate_point(*end, &format!("paints[{index}].end"))?;
            validate_stops(stops, index)?;
            Ok(stops.len())
        }
        Paint::RadialGradient {
            center,
            radii,
            stops,
            ..
        } => {
            validate_point(*center, &format!("paints[{index}].center"))?;
            validate_point(*radii, &format!("paints[{index}].radii"))?;
            if radii[0] <= 0.0 || radii[1] <= 0.0 {
                return invalid(format!("paints[{index}].radii"), "must be positive");
            }
            validate_stops(stops, index)?;
            Ok(stops.len())
        }
        Paint::TwoCircleGradient {
            start,
            start_radius,
            end,
            end_radius,
            stops,
            ..
        } => {
            validate_point(*start, &format!("paints[{index}].start"))?;
            validate_point(*end, &format!("paints[{index}].end"))?;
            if !start_radius.is_finite()
                || !end_radius.is_finite()
                || *start_radius < 0.0
                || *end_radius < 0.0
            {
                return invalid(
                    format!("paints[{index}].radii"),
                    "must be finite and non-negative",
                );
            }
            validate_stops(stops, index)?;
            Ok(stops.len())
        }
        Paint::ConicGradient {
            center,
            start_angle_degrees,
            sweep_angle_degrees,
            stops,
            ..
        } => {
            validate_point(*center, &format!("paints[{index}].center"))?;
            finite_f64(
                *start_angle_degrees,
                &format!("paints[{index}].startAngleDegrees"),
            )?;
            finite_f64(
                *sweep_angle_degrees,
                &format!("paints[{index}].sweepAngleDegrees"),
            )?;
            if *sweep_angle_degrees <= 0.0 || *sweep_angle_degrees > 360.0 {
                return invalid(
                    format!("paints[{index}].sweepAngleDegrees"),
                    "must be in (0, 360]",
                );
            }
            validate_stops(stops, index)?;
            Ok(stops.len())
        }
    }
}

fn validate_stops(stops: &[GradientStop], paint_index: usize) -> Result<(), DrawProgramError> {
    if stops.len() < 2 {
        return invalid(
            format!("paints[{paint_index}].stops"),
            "a gradient needs at least two stops",
        );
    }
    let mut previous = -1.0f32;
    for (index, stop) in stops.iter().enumerate() {
        validate_unit(
            stop.offset,
            &format!("paints[{paint_index}].stops[{index}].offset"),
        )?;
        if stop.offset < previous {
            return invalid(
                format!("paints[{paint_index}].stops[{index}].offset"),
                "stops must be sorted",
            );
        }
        previous = stop.offset;
        validate_color(
            stop.color,
            &format!("paints[{paint_index}].stops[{index}].color"),
        )?;
    }
    Ok(())
}

fn validate_color(color: LinearColor, location: &str) -> Result<(), DrawProgramError> {
    finite_f32(color.red, &format!("{location}.red"))?;
    finite_f32(color.green, &format!("{location}.green"))?;
    finite_f32(color.blue, &format!("{location}.blue"))?;
    validate_unit(color.alpha, &format!("{location}.alpha"))?;
    if color.alpha == 0.0 && (color.red != 0.0 || color.green != 0.0 || color.blue != 0.0) {
        return invalid(location, "transparent premultiplied RGB must be zero");
    }
    Ok(())
}

fn validate_texture(texture: &ExternalTexture, location: &str) -> Result<(), DrawProgramError> {
    validate_key(&texture.key, &format!("{location}.key"))?;
    match (texture.kind, texture.sample_time_micros) {
        (TextureKind::Video, None) => {
            return invalid(
                format!("{location}.sampleTimeMicros"),
                "video textures must carry a source timestamp",
            );
        }
        (TextureKind::Image | TextureKind::Generated, Some(_)) => {
            return invalid(
                format!("{location}.sampleTimeMicros"),
                "only video textures may carry a source timestamp",
            );
        }
        (TextureKind::Video, Some(sample)) if !(0..=9_007_199_254_740_991).contains(&sample) => {
            return invalid(
                format!("{location}.sampleTimeMicros"),
                "video source timestamp must be a non-negative safe integer",
            );
        }
        (TextureKind::Video, Some(_)) | (TextureKind::Image | TextureKind::Generated, None) => {}
    }
    Ok(())
}

fn validate_scope(scope: &BackdropScope, location: &str) -> Result<(), DrawProgramError> {
    match scope {
        BackdropScope::Current => Ok(()),
        BackdropScope::LayerEntry(key) | BackdropScope::ScopeEntry(key) => {
            validate_key(key, location)
        }
    }
}

fn validate_key(value: &str, location: &str) -> Result<(), DrawProgramError> {
    if value.is_empty() {
        return invalid(location, "must not be empty");
    }
    if value.len() > MAX_STRING_BYTES {
        return invalid(location, format!("exceeds {MAX_STRING_BYTES} UTF-8 bytes"));
    }
    Ok(())
}

fn validate_rect(rect: Rect, location: &str) -> Result<(), DrawProgramError> {
    finite_f64(rect.x, &format!("{location}.x"))?;
    finite_f64(rect.y, &format!("{location}.y"))?;
    finite_f64(rect.width, &format!("{location}.width"))?;
    finite_f64(rect.height, &format!("{location}.height"))?;
    if rect.width < 0.0 || rect.height < 0.0 {
        return invalid(location, "width and height must be non-negative");
    }
    if [rect.left(), rect.top(), rect.right(), rect.bottom()]
        .into_iter()
        .any(|value| value.abs() > MAX_LOCAL_COORDINATE)
    {
        return invalid(
            location,
            format!("coordinate magnitude exceeds {MAX_LOCAL_COORDINATE}"),
        );
    }
    Ok(())
}

fn validate_normalized_rect(rect: Rect, location: &str) -> Result<(), DrawProgramError> {
    validate_rect(rect, location)?;
    if rect.is_empty() {
        return invalid(location, "width and height must be positive");
    }
    if rect.left() < 0.0 || rect.top() < 0.0 || rect.right() > 1.0 || rect.bottom() > 1.0 {
        return invalid(location, "must be contained by normalized [0,1] content");
    }
    Ok(())
}

fn validate_viewport(viewport: Rect) -> Result<(), DrawProgramError> {
    validate_rect(viewport, "viewport")?;
    if viewport.is_empty() {
        return invalid("viewport", "width and height must be positive");
    }
    Ok(())
}

fn validate_insets(insets: Insets, location: &str) -> Result<(), DrawProgramError> {
    finite_nonnegative(insets.left, &format!("{location}.left"))?;
    finite_nonnegative(insets.top, &format!("{location}.top"))?;
    finite_nonnegative(insets.right, &format!("{location}.right"))?;
    finite_nonnegative(insets.bottom, &format!("{location}.bottom"))
}

fn validate_point(point: [f64; 2], location: &str) -> Result<(), DrawProgramError> {
    finite_f64(point[0], &format!("{location}.x"))?;
    finite_f64(point[1], &format!("{location}.y"))
}

fn validate_unit(value: f32, location: &str) -> Result<(), DrawProgramError> {
    finite_f32(value, location)?;
    if !(0.0..=1.0).contains(&value) {
        return invalid(location, "must be in [0, 1]");
    }
    Ok(())
}

fn finite_nonnegative(value: f32, location: &str) -> Result<(), DrawProgramError> {
    finite_f32(value, location)?;
    if value < 0.0 {
        return invalid(location, "must be non-negative");
    }
    Ok(())
}

fn finite_f32(value: f32, location: &str) -> Result<(), DrawProgramError> {
    if !value.is_finite() {
        return invalid(location, "must be finite");
    }
    Ok(())
}

fn finite_f64(value: f64, location: &str) -> Result<(), DrawProgramError> {
    if !value.is_finite() {
        return invalid(location, "must be finite");
    }
    if value.abs() > MAX_LOCAL_COORDINATE {
        return invalid(
            location,
            format!("magnitude exceeds {MAX_LOCAL_COORDINATE}"),
        );
    }
    Ok(())
}

fn invalid<T>(
    location: impl Into<String>,
    reason: impl Into<String>,
) -> Result<T, DrawProgramError> {
    Err(DrawProgramError::InvalidValue {
        location: location.into(),
        reason: reason.into(),
    })
}

fn check_node_id(
    owner: Option<NodeId>,
    referenced: NodeId,
    count: usize,
) -> Result<(), DrawProgramError> {
    if referenced.index() >= count {
        Err(DrawProgramError::InvalidNodeId { owner, referenced })
    } else {
        Ok(())
    }
}

fn check_path_id(owner: NodeId, referenced: PathId, count: usize) -> Result<(), DrawProgramError> {
    if referenced.index() >= count {
        Err(DrawProgramError::InvalidPathId { owner, referenced })
    } else {
        Ok(())
    }
}

fn check_paint_id(
    owner: NodeId,
    referenced: PaintId,
    count: usize,
) -> Result<(), DrawProgramError> {
    if referenced.index() >= count {
        Err(DrawProgramError::InvalidPaintId { owner, referenced })
    } else {
        Ok(())
    }
}
