//! The caption a captionPresenter draws: the frozen input its host prepares, and the binding of a
//! compiled template to one caption.
//!
//! A template compiles once for a caption track. What varies from caption to caption, the text
//! and its runs, stays a read in the template (`Expr::Caption`), and the authored
//! `caption.runs.map(...)` stays one row of children ([`CaptionRows`]). Binding the template to a
//! caption repeats each row once per run and replaces every read with its value, which gives an
//! ordinary artifact that prepares and draws like any other.
pub use valle_timeline::motion::{
    CAPTION_FONT_CONTROL, CAPTION_FONT_URI, CaptionPresenterData, CaptionPresenterLayout,
    CaptionPresenterRun, CaptionPresenterShadow, CaptionPresenterStyle,
};

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use valle_draw::program::AuthorColor;

use crate::artifact::{
    BatchPositions, BoolValue, ChildRange, ColorValue, GradientStopValue, MaskValue, NodeId,
    NodeKind, NumberValue, PaintValue, PathValue, PointValue, RectValue, SceneArtifact, SceneNode,
    ShaderTextureInput, ShaderUniformBinding, ShaderUniformValue, StyleValue, TextValue,
    TransitionEffect,
};
use crate::expr::{CaptionInput, Expr, ExprId};
use crate::value::MotionValue;

/// Children of `parent` that repeat once per run of the caption: an authored
/// `caption.runs.map(...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptionRows {
    pub parent: NodeId,
    /// One row: this range of `node_children`, inside the parent's children.
    pub children: ChildRange,
}

/// One caption, as a template is bound to it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundCaption {
    pub runs: Vec<BoundCaptionRun>,
}

/// One run with every field resolved.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundCaptionRun {
    pub text: String,
    /// Clip-local seconds.
    pub start: f64,
    pub end: f64,
    pub font_size: f64,
    pub color: AuthorColor,
}

impl BoundCaption {
    /// The caption `data` describes, drawn for `duration` seconds. A run without times spans the
    /// caption, and a run without a size or color takes the track style's.
    pub fn from_data(data: &CaptionPresenterData, duration: f64) -> Result<Self, CaptionBindError> {
        let runs = data
            .runs
            .iter()
            .map(|run| {
                let (start, end) = match (run.start, run.end) {
                    (Some(start), Some(end)) => (start, end),
                    _ => (0.0, duration),
                };
                let color = run.color.as_deref().unwrap_or(&data.style.color);
                Ok(BoundCaptionRun {
                    text: run.text.clone(),
                    start,
                    end,
                    font_size: run.font_size.unwrap_or(data.style.font_size),
                    color: parse_color(color)?,
                })
            })
            .collect::<Result<_, CaptionBindError>>()?;
        Ok(Self { runs })
    }

    /// The whole text: the runs, joined.
    pub fn text(&self) -> String {
        self.runs.iter().map(|run| run.text.as_str()).collect()
    }
}

/// Parse a caption color, a hex color as Timeline writes them.
pub fn parse_color(color: &str) -> Result<AuthorColor, CaptionBindError> {
    valle_draw::Rgba::parse(color)
        .map(AuthorColor::from_srgb8)
        .ok_or_else(|| CaptionBindError(format!("`{color}` is not a caption color")))
}

/// Why a template cannot draw a caption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptionBindError(pub String);

impl core::fmt::Display for CaptionBindError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CaptionBindError {}

fn fail<T>(message: impl Into<String>) -> Result<T, CaptionBindError> {
    Err(CaptionBindError(message.into()))
}

/// The key of a node in row `row`.
fn row_key(key: &str, row: usize) -> String {
    format!("{key}#{row}")
}

impl SceneArtifact {
    /// Whether this is a captionPresenter template that reads its caption, and so must be bound
    /// to one before it draws.
    pub fn reads_caption(&self) -> bool {
        !self.caption_rows.is_empty()
            || self
                .exprs
                .iter()
                .chain(self.instance_groups.iter().flat_map(|group| &group.exprs))
                .any(|expr| matches!(expr, Expr::Caption { .. }))
    }

    /// Bind a captionPresenter template to one caption: each caption row repeats once per run,
    /// with keys `{key}#{index}`, and each read of the caption takes its value. The result reads
    /// no caption.
    pub fn bind_caption(&self, caption: &BoundCaption) -> Result<SceneArtifact, CaptionBindError> {
        if !matches!(
            self.role,
            valle_timeline::MotionRole::CaptionPresenter { .. }
        ) {
            return fail("only a captionPresenter template draws a caption");
        }
        let Some(first) = caption.runs.first() else {
            return fail("a caption has at least one run");
        };
        let text = caption.text();
        let count = caption.runs.len();
        let value = |input: CaptionInput, row: usize, run: &BoundCaptionRun| match input {
            CaptionInput::Text => MotionValue::Str(text.clone()),
            CaptionInput::RunCount => MotionValue::Number(count as f64),
            CaptionInput::RunIndex => MotionValue::Number(row as f64),
            CaptionInput::RunText => MotionValue::Str(run.text.clone()),
            CaptionInput::RunStart => MotionValue::Number(run.start),
            CaptionInput::RunEnd => MotionValue::Number(run.end),
            CaptionInput::RunFontSize => MotionValue::Number(run.font_size),
            CaptionInput::RunColor => MotionValue::Color(run.color),
        };

        let row_keys = self.caption_row_keys()?;
        // What a row reads takes that row's values: its run, and the bounds of its own nodes.
        // The authored expressions serve the first row; every other row gets its own copies.
        let row_dependent = caption_row_dependent(&self.exprs, &row_keys);
        let bind_leaf = |expr: &Expr, row: usize| -> Expr {
            match expr {
                Expr::Caption { input } => Expr::Const {
                    value: value(*input, row, &caption.runs[row]),
                },
                Expr::NodeBounds { key } if row_keys.contains(key) => Expr::NodeBounds {
                    key: row_key(key, row),
                },
                other => other.clone(),
            }
        };
        let mut exprs: Vec<Expr> = self.exprs.iter().map(|expr| bind_leaf(expr, 0)).collect();
        let mut copies = Vec::with_capacity(count.saturating_sub(1));
        for row in 1..count {
            let mut copy = vec![None; self.exprs.len()];
            for (at, expr) in self.exprs.iter().enumerate() {
                if !row_dependent[at] {
                    continue;
                }
                let mut expr = bind_leaf(expr, row);
                // The arena is topological, so every row-dependent child already has its copy.
                expr.remap_children(|id| copy[id.0 as usize].unwrap_or(id));
                copy[at] = Some(ExprId(exprs.len() as u32));
                exprs.push(expr);
            }
            copies.push(copy);
        }

        let mut instance_groups = self.instance_groups.clone();
        for group in &mut instance_groups {
            for expr in &mut group.exprs {
                if let Expr::Caption { input } = *expr {
                    if input.reads_row() {
                        return fail("an instance template cannot read a caption row");
                    }
                    *expr = Expr::Const {
                        value: value(input, 0, first),
                    };
                }
            }
        }

        let mut rows_by_parent = BTreeMap::<u32, Vec<ChildRange>>::new();
        for rows in self
            .caption_rows
            .iter()
            .filter(|rows| rows.children.start < rows.children.end)
        {
            rows_by_parent
                .entry(rows.parent.0)
                .or_default()
                .push(rows.children);
        }
        for ranges in rows_by_parent.values_mut() {
            ranges.sort_by_key(|range| range.start);
        }
        let mut emitter = Emitter {
            template: self,
            rows_by_parent,
            row_keys: &row_keys,
            copies: &copies,
            count,
            nodes: Vec::with_capacity(self.nodes.len()),
            children: Vec::with_capacity(self.node_children.len()),
        };
        let root = emitter.emit(self.root, None, 0)?;
        Ok(SceneArtifact {
            role: self.role,
            format_version: self.format_version,
            capability_set: self.capability_set.clone(),
            component: self.component.clone(),
            controls: self.controls.clone(),
            composition: self.composition.clone(),
            resource_refs: self.resource_refs.clone(),
            shaders: self.shaders.clone(),
            exprs,
            instance_groups,
            caption_rows: Vec::new(),
            nodes: emitter.nodes,
            node_children: emitter.children,
            root,
            camera: self.camera.clone(),
        })
    }

    /// The keys of every node in a caption row, checking that the rows are well formed: each one a
    /// range of its parent's children, apart from the other rows, and none inside another row.
    pub(crate) fn caption_row_keys(&self) -> Result<BTreeSet<String>, CaptionBindError> {
        let mut keys = BTreeSet::new();
        let mut in_row = vec![false; self.nodes.len()];
        let mut claimed = vec![false; self.node_children.len()];
        for rows in &self.caption_rows {
            let Some(parent) = self.nodes.get(rows.parent.0 as usize) else {
                return fail("a caption row's parent is not a node");
            };
            let range = rows.children;
            if range.start > range.end
                || range.start < parent.children.start
                || range.end > parent.children.end
            {
                return fail("a caption row must be a range of its parent's children");
            }
            for slot in range.start..range.end {
                if std::mem::replace(&mut claimed[slot as usize], true) {
                    return fail("caption rows cannot overlap");
                }
                let mut pending = vec![self.node_children[slot as usize]];
                while let Some(id) = pending.pop() {
                    let Some(node) = self.nodes.get(id.0 as usize) else {
                        return fail("a caption row holds a node that does not exist");
                    };
                    in_row[id.0 as usize] = true;
                    keys.insert(node.key.clone());
                    pending.extend(
                        self.node_children
                            .get(node.children.start as usize..node.children.end as usize)
                            .unwrap_or_default(),
                    );
                }
            }
        }
        if self
            .caption_rows
            .iter()
            .any(|rows| in_row[rows.parent.0 as usize])
        {
            return fail("a caption row cannot hold another caption row");
        }
        Ok(keys)
    }

    /// Whether node `at` is in a caption row, for every node.
    pub(crate) fn caption_row_nodes(&self) -> Vec<bool> {
        let mut in_row = vec![false; self.nodes.len()];
        for rows in &self.caption_rows {
            let mut pending = self
                .node_children
                .get(rows.children.start as usize..rows.children.end as usize)
                .unwrap_or_default()
                .to_vec();
            while let Some(id) = pending.pop() {
                let Some(node) = self.nodes.get(id.0 as usize) else {
                    continue;
                };
                if std::mem::replace(&mut in_row[id.0 as usize], true) {
                    continue;
                }
                pending.extend(
                    self.node_children
                        .get(node.children.start as usize..node.children.end as usize)
                        .unwrap_or_default(),
                );
            }
        }
        in_row
    }
}

/// Which expressions read a caption row, through its run or the bounds of a node in it.
pub(crate) fn caption_row_dependent(exprs: &[Expr], row_keys: &BTreeSet<String>) -> Vec<bool> {
    let mut dependent = Vec::with_capacity(exprs.len());
    for expr in exprs {
        let direct = match expr {
            Expr::Caption { input } => input.reads_row(),
            Expr::NodeBounds { key } => row_keys.contains(key),
            _ => false,
        };
        let inherited = expr
            .children()
            .iter()
            .any(|id| dependent.get(id.0 as usize).copied().unwrap_or(false));
        dependent.push(direct || inherited);
    }
    dependent
}

struct Emitter<'a> {
    template: &'a SceneArtifact,
    rows_by_parent: BTreeMap<u32, Vec<ChildRange>>,
    row_keys: &'a BTreeSet<String>,
    copies: &'a [Vec<Option<ExprId>>],
    count: usize,
    nodes: Vec<SceneNode>,
    children: Vec<NodeId>,
}

impl Emitter<'_> {
    /// Emit node `id` and its subtree in the compiler's order: a node before its descendants, and
    /// a parent's child range after its children's ranges.
    fn emit(
        &mut self,
        id: NodeId,
        row: Option<usize>,
        depth: usize,
    ) -> Result<NodeId, CaptionBindError> {
        // Artifacts are validated before they bind; this only keeps a malformed one off the stack.
        if depth > 4096 {
            return fail("the scene is nested too deeply to bind");
        }
        let Some(source) = self.template.nodes.get(id.0 as usize) else {
            return fail("a scene child is not a node");
        };
        let mut node = source.clone();
        node.children = ChildRange::EMPTY;
        if let Some(row) = row {
            node.key = row_key(&node.key, row);
            let keys = self.row_keys;
            rename_node_keys(&mut node, &mut |key| {
                if keys.contains(key.as_str()) {
                    *key = row_key(key, row);
                }
            });
            let copy = row.checked_sub(1).map(|at| &self.copies[at]);
            remap_node_exprs(&mut node, &mut |expr| {
                copy.and_then(|copy| copy.get(expr.0 as usize).copied().flatten())
                    .unwrap_or(expr)
            })?;
        }
        let emitted = NodeId(self.nodes.len() as u32);
        self.nodes.push(node);

        let rows = self.rows_by_parent.get(&id.0).cloned().unwrap_or_default();
        let mut rows = rows.into_iter().peekable();
        let mut direct = Vec::new();
        let mut slot = source.children.start;
        while slot < source.children.end {
            if let Some(range) = rows.next_if(|range| range.start == slot) {
                for at in 0..self.count {
                    for slot in range.start..range.end {
                        let child = self.template.node_children[slot as usize];
                        direct.push(self.emit(child, Some(at), depth + 1)?);
                    }
                }
                slot = range.end;
                continue;
            }
            let child = self.template.node_children[slot as usize];
            direct.push(self.emit(child, row, depth + 1)?);
            slot += 1;
        }
        let start = self.children.len() as u32;
        self.children.extend(direct);
        self.nodes[emitted.0 as usize].children = ChildRange {
            start,
            end: self.children.len() as u32,
        };
        Ok(emitted)
    }
}

/// Rename the keys `node` refers to: its rich-text group, mask subtree and shader inputs.
fn rename_node_keys(node: &mut SceneNode, rename: &mut impl FnMut(&mut String)) {
    fn inputs(inputs: &mut [ShaderTextureInput], rename: &mut impl FnMut(&mut String)) {
        for input in inputs {
            if let Some(source) = &mut input.source {
                rename(source);
            }
        }
    }
    match &mut node.kind {
        NodeKind::Text {
            per_unit: Some(per_unit),
            ..
        } => {
            if let Some(group) = &mut per_unit.group_key {
                rename(group);
            }
        }
        NodeKind::Mask {
            source: MaskValue::Subtree { source },
            ..
        } => rename(source),
        NodeKind::ShaderLayer { inputs: list, .. } => inputs(list, rename),
        NodeKind::Transition {
            effect: TransitionEffect::Shader { inputs: list, .. },
            ..
        } => inputs(list, rename),
        _ => {}
    }
}

/// Rewrite every expression `node` refers to, as [`SceneNode::expr_refs_outside_per_unit`] and
/// [`SceneNode::per_unit_expr_refs`] list them. A caption row cannot hold the kinds whose
/// bindings this does not rewrite.
pub(crate) fn remap_node_exprs(
    node: &mut SceneNode,
    map: &mut impl FnMut(ExprId) -> ExprId,
) -> Result<(), CaptionBindError> {
    fn number(value: &mut NumberValue, map: &mut impl FnMut(ExprId) -> ExprId) {
        if let NumberValue::Expr { expr } = value {
            *expr = map(*expr);
        }
    }
    fn point(value: &mut PointValue, map: &mut impl FnMut(ExprId) -> ExprId) {
        if let PointValue::Expr { expr } = value {
            *expr = map(*expr);
        }
    }
    fn color(value: &mut ColorValue, map: &mut impl FnMut(ExprId) -> ExprId) {
        if let ColorValue::Expr { expr } = value {
            *expr = map(*expr);
        }
    }
    fn rect(value: &mut RectValue, map: &mut impl FnMut(ExprId) -> ExprId) {
        if let RectValue::Expr { expr } = value {
            *expr = map(*expr);
        }
    }
    fn path(value: &mut PathValue, map: &mut impl FnMut(ExprId) -> ExprId) {
        if let PathValue::Expr { expr, .. } = value {
            *expr = map(*expr);
        }
    }
    fn stops(stops: &mut [GradientStopValue], map: &mut impl FnMut(ExprId) -> ExprId) {
        for stop in stops {
            number(&mut stop.offset, map);
            color(&mut stop.color, map);
        }
    }
    fn paint(value: &mut PaintValue, map: &mut impl FnMut(ExprId) -> ExprId) {
        match value {
            PaintValue::Solid { color: value } => color(value, map),
            PaintValue::Linear {
                start,
                end,
                stops: items,
                ..
            } => {
                point(start, map);
                point(end, map);
                stops(items, map);
            }
            PaintValue::Radial {
                center,
                radius,
                stops: items,
                ..
            } => {
                point(center, map);
                number(radius, map);
                stops(items, map);
            }
            PaintValue::Conic {
                center,
                start_angle,
                stops: items,
                ..
            } => {
                point(center, map);
                number(start_angle, map);
                stops(items, map);
            }
        }
    }
    fn uniforms(uniforms: &mut [ShaderUniformBinding], map: &mut impl FnMut(ExprId) -> ExprId) {
        for uniform in uniforms {
            match &mut uniform.value {
                ShaderUniformValue::Float { value } => number(value, map),
                ShaderUniformValue::Float2 { value } => point(value, map),
                ShaderUniformValue::Float3 { value } => {
                    value.iter_mut().for_each(|value| number(value, map))
                }
                ShaderUniformValue::Float4 { value } | ShaderUniformValue::Float2x2 { value } => {
                    value.iter_mut().for_each(|value| number(value, map))
                }
                ShaderUniformValue::Float3x3 { value } => {
                    value.iter_mut().for_each(|value| number(value, map))
                }
                ShaderUniformValue::Float4x4 { value } => {
                    value.iter_mut().for_each(|value| number(value, map))
                }
                ShaderUniformValue::Color { value } => color(value, map),
                ShaderUniformValue::Bool {
                    value: BoolValue::Expr { expr },
                } => *expr = map(*expr),
                ShaderUniformValue::Bool {
                    value: BoolValue::Static { .. },
                } => {}
            }
        }
    }

    if let Some(expr) = &mut node.visibility {
        *expr = map(*expr);
    }
    for expr in node.class_conditions.values_mut() {
        *expr = map(*expr);
    }
    for style in &mut node.styles {
        if let StyleValue::Expr { expr } = &mut style.value {
            *expr = map(*expr);
        }
    }
    match &mut node.kind {
        NodeKind::Group
        | NodeKind::Box
        | NodeKind::Shutter { .. }
        | NodeKind::Echo { .. }
        | NodeKind::TimeScope { .. }
        | NodeKind::Image { .. }
        | NodeKind::MathFormula { .. } => {}
        NodeKind::Text {
            text,
            per_unit,
            path: text_path,
        } => {
            if let TextValue::Expr { expr } = text {
                *expr = map(*expr);
            }
            if let Some(value) = text_path {
                path(value, map);
            }
            if let Some(per_unit) = per_unit {
                let style = &mut per_unit.style;
                if let Some(value) = &mut style.opacity {
                    number(value, map);
                }
                if let Some(value) = &mut style.translate {
                    point(value, map);
                }
                if let Some(value) = &mut style.scale {
                    point(value, map);
                }
                if let Some(value) = &mut style.rotate {
                    number(value, map);
                }
                if let Some(value) = &mut style.color {
                    color(value, map);
                }
                if let Some(value) = &mut style.blur {
                    number(value, map);
                }
            }
        }
        NodeKind::Clip { path: value, .. } => path(value, map),
        NodeKind::Mask {
            source, rect: r, ..
        } => {
            if let MaskValue::Paint { paint: value } = source {
                paint(value, map);
            }
            rect(r, map);
        }
        NodeKind::Path {
            d,
            fill,
            stroke,
            trim_start,
            trim_end,
            ..
        } => {
            path(d, map);
            if let Some(value) = fill {
                paint(value, map);
            }
            if let Some(value) = stroke {
                paint(&mut value.paint, map);
                number(&mut value.width, map);
                number(&mut value.dash_offset, map);
            }
            number(trim_start, map);
            number(trim_end, map);
        }
        NodeKind::GeometryBatch { batch } => {
            if let BatchPositions::Particles { frame, .. } = &mut batch.positions {
                *frame = map(*frame);
            }
            if let Some(field) = &mut batch.position_field {
                field.progress = map(field.progress);
            }
            if let Some(field) = &mut batch.size_field {
                field.progress = map(field.progress);
            }
            if let Some(field) = &mut batch.fill_field {
                field.progress = map(field.progress);
            }
            for field in [
                &mut batch.opacity_field,
                &mut batch.rotation_field,
                &mut batch.skew_x_field,
                &mut batch.stroke_width_field,
            ]
            .into_iter()
            .flatten()
            {
                field.progress = map(field.progress);
            }
        }
        NodeKind::Transition { effect, progress } => {
            number(progress, map);
            match effect {
                TransitionEffect::Builtin { params, .. } => {
                    params.values_mut().for_each(|value| number(value, map))
                }
                TransitionEffect::Shader { uniforms: list, .. } => uniforms(list, map),
            }
        }
        NodeKind::ShaderLayer { uniforms: list, .. } => uniforms(list, map),
        NodeKind::Video {
            source_start,
            speed,
            ..
        } => {
            number(source_start, map);
            number(speed, map);
        }
        NodeKind::InstanceBatch { .. } | NodeKind::InstanceLayout { .. } => {
            return fail("a caption row cannot hold an instanced map");
        }
        NodeKind::Scene3D { .. } => return fail("a caption row cannot hold a Scene3D"),
        NodeKind::Glass(_) | NodeKind::GlassField(_) => {
            return fail("a caption row cannot hold Glass");
        }
    }
    Ok(())
}
