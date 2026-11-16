//! Declarations bound to a variant, inherited and checked per frame.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::{svg, transition};

use super::computed::{Environment, FontMetrics, Instance, Shaded, Source, Style, compute};
use super::declaration::{Declared, overflow, pair, presented, set};
use super::property::{Field, camel, unsupported};
use super::{cache, tailwind};

use crate::front::Slot;
use crate::shader::{self, Program};
use crate::template::{Error, Kind, Node, Part, Piece, Prop, Tag, Template, V};

/// A `<ShaderLayer>`'s program and its uniforms' values, and its block when none is traced.
pub(super) struct Shading {
    pub(super) program: Rc<Program>,
    /// What errors call it, `<ShaderLayer>` or `<Transition>`.
    pub(super) tag: &'static str,
    /// A built-in transition's parameters' ranges.
    pub(super) limits: &'static [transition::Parameter],
    pub(super) transition: bool,
    pub(super) uniforms: Vec<(Rc<str>, Vec<Prop>)>,
    pub(super) inputs: Vec<(Rc<str>, u32)>,
    pub(super) outset: f64,
    pub(super) fixed: Option<Rc<[u8]>>,
}

impl Shading {
    pub(super) fn computed(&self, frame: &Frame) -> Result<Shaded, String> {
        let block = match &self.fixed {
            Some(b) => b.clone(),
            None => {
                let mut values: Vec<(&str, Vec<&V>)> = (self.uniforms.iter())
                    .filter(|(_, v)| !v.iter().any(|p| frame.unsettled(p)))
                    .map(|(k, v)| {
                        let v = (v.iter())
                            .map(|p| match p {
                                Prop::Const(v) => v,
                                Prop::Expr(e) => &frame.values[*e as usize],
                            })
                            .collect();
                        (&**k, v)
                    })
                    .collect();
                for (name, v) in &values {
                    let Some(param) = self.limits.iter().find(|p| p.name == *name) else {
                        continue;
                    };
                    if let [V::Num(x)] = v[..]
                        && !(param.min..=param.max).contains(x)
                    {
                        let x = crate::js::number_to_string(*x);
                        return Err(format!(
                            "params {name} is from {} to {}, not {x}",
                            param.min, param.max
                        ));
                    }
                }
                let defaults: Vec<V> = self.limits.iter().map(|p| V::Num(p.default)).collect();
                for (param, value) in self.limits.iter().zip(&defaults) {
                    if !values.iter().any(|(name, _)| *name == param.name) {
                        values.push((param.name, vec![value]));
                    }
                }
                Rc::from(self.program.block(&values)?)
            }
        };
        Ok(Shaded {
            program: self.program.clone(),
            block,
            inputs: self.inputs.clone(),
            outset: self.outset,
            transition: self.transition,
        })
    }
}

/// A variant's styles, bound once; `compute` gives each frame's.
pub struct Sheet {
    /// Declarations in children that may request this node's computed values.
    inherited_by: Vec<Vec<Prop>>,
    instances: Vec<Option<super::instances::Table>>,
    svg_instances: Vec<Option<super::instances::svg::Table>>,
    pub(super) nodes: Vec<Bound>,
    /// Each node's parent, a mask's source's the mask. Parents come before their children.
    pub(super) parents: Vec<Option<u32>>,
    /// Where each node's descendants end, past the last.
    pub(super) ends: Vec<u32>,
    /// Which of the program's expressions are late.
    pub(super) late: Vec<bool>,
}

/// A node's declarations: those before the first one bound to an expression, applied once, and
/// from there on the rest, set every frame in order (a constant `padding` written after a
/// traced `paddingTop` overrides it).
pub(super) struct Bound {
    pub(super) declared: Declared,
    /// The invariant part of eligible styles; geometry and uniforms stay frame-dependent.
    /// Boxed so inactive variants do not reserve a computed style for every node.
    pub(super) base: RefCell<Option<Box<cache::Base>>>,
    pub(super) updates: Option<cache::Updates>,
    /// Whether it is a Text.
    pub(super) text: bool,
    inline_content: bool,
    view: bool,
    /// Full declaration order only when a Text has flow-relative box properties.
    logical: Vec<([Field; 6], Prop, Rc<str>, bool)>,
    /// Each with what errors call it, and whether it is a presentation attribute.
    pub(super) rest: Vec<(Field, Prop, Rc<str>, bool)>,
    /// Where errors point.
    pub(super) place: Rc<str>,
    pub(super) source: Option<Source>,
    pub(super) svg: Option<Geometry>,
    pub(super) shader: Option<Shading>,
}

/// An SVG element's attributes that are not presentation attributes, and what they compute to
/// when none is traced.
pub(super) struct Geometry {
    pub(super) tag: Tag,
    pub(super) attributes: Vec<(Rc<str>, Prop)>,
    pub(super) template: Option<svg::Template>,
    pub(super) fixed: Option<Rc<svg::Computed>>,
}

impl Geometry {
    pub(super) fn computed(&self, frame: &Frame) -> Result<Rc<svg::Computed>, String> {
        if let Some(c) = &self.fixed {
            return Ok(c.clone());
        }
        let list: Vec<(&str, &V)> = (self.attributes.iter())
            .filter(|(_, p)| !frame.unsettled(p))
            .map(|(k, p)| {
                let v = match p {
                    Prop::Const(v) => v,
                    Prop::Expr(e) => &frame.values[*e as usize],
                };
                (&**k, v)
            })
            .collect();
        let shape = self.template.as_ref()
            .filter(|g| !g.parts.iter().any(|p| matches!(p, Part::Expr(e) if frame.late.is_some_and(|late| late[*e as usize]))))
            .map(|g| g.resolve(frame.values));
        let c = svg::computed(self.tag, &list, shape)
            .map_err(|e| format!("<{}> {e}", self.tag.name()))?;
        Ok(Rc::new(c))
    }
}

impl Sheet {
    pub fn new(template: &Template) -> Result<Sheet, Error> {
        let n = template.nodes.len();
        let mut parents = vec![None; n];
        for (i, node) in template.nodes.iter().enumerate() {
            let source = match node.kind {
                Kind::Mask { source, .. } => Some(source),
                _ => None,
            };
            for &c in node.children.iter().chain(&source) {
                let c = c as usize;
                if c <= i || c >= n || parents[c].is_some() {
                    return Err("engine bug: template nodes are not in preorder".into());
                }
                parents[c] = Some(i as u32);
            }
        }
        // In preorder, a node's descendants follow it.
        let mut ends: Vec<u32> = (1..=n as u32).collect();
        for i in (0..n).rev() {
            if let Some(p) = parents[i] {
                ends[p as usize] = ends[p as usize].max(ends[i]);
            }
        }
        let classes = tailwind::declarations(template)?;
        let late = template.program.late();
        let nodes: Vec<Bound> = (template.nodes.iter().zip(&classes))
            .enumerate()
            .map(|(i, (node, classes))| {
                let nested = parents[i]
                    .is_some_and(|p| matches!(template.nodes[p as usize].kind, Kind::Svg { .. }));
                bind(node, classes, &late, nested)
            })
            .collect::<Result<_, _>>()?;
        let mut inherited_by = vec![Vec::new(); n];
        for (i, bound) in nodes.iter().enumerate() {
            if let Some(p) = parents[i] {
                inherited_by[p as usize].extend(bound.rest.iter().filter_map(|(_, prop, ..)| {
                    match prop {
                        Prop::Const(v) if super::cascade::Keyword::of(v).is_none() => None,
                        _ => Some(prop.clone()),
                    }
                }));
            }
        }
        Ok(Sheet {
            inherited_by,
            instances: (0..n)
                .map(|i| {
                    template
                        .instances
                        .iter()
                        .find(|(node, _)| {
                            *node as usize == i && template.nodes[i].kind == Kind::View
                        })
                        .map(|(_, table)| {
                            super::instances::Table::new(table, template.nodes[i].place())
                        })
                        .transpose()
                })
                .collect::<Result<_, _>>()?,
            svg_instances: (0..n)
                .map(|i| {
                    let Kind::Svg { tag, .. } = template.nodes[i].kind else {
                        return Ok(None);
                    };
                    template
                        .instances
                        .iter()
                        .find(|(node, _)| *node as usize == i)
                        .map(|(_, table)| {
                            super::instances::svg::Table::new(table, tag, template.nodes[i].place())
                        })
                        .transpose()
                })
                .collect::<Result<_, Error>>()?,
            nodes,
            parents,
            ends,
            late,
        })
    }

    /// Drop computed styles when a scene switches variants. The parsed declarations remain
    /// reusable, but only the active variant retains frame-dependent style snapshots.
    pub(crate) fn clear_computed(&mut self) {
        for node in &mut self.nodes {
            *node.base.get_mut() = None;
        }
    }

    /// Which of the program's expressions are late.
    pub fn late(&self) -> &[bool] {
        &self.late
    }

    /// Node `i`'s descendants, a mask's source's among the mask's.
    pub fn descendants(&self, i: usize) -> std::ops::Range<usize> {
        i + 1..self.ends[i] as usize
    }

    /// Every node's computed style for the program's values at one frame on a canvas of `canvas`
    /// px, in template order, in a scene of these `resources`. Before the values are `settled`,
    /// declarations of late values are left out: those only paint, so layout goes without them.
    pub fn compute(
        &self,
        values: &[V],
        settled: bool,
        canvas: [f32; 2],
        resources: &[Slot],
    ) -> Result<Vec<Style>, Error> {
        self.computed(values, settled, canvas, resources, None)
    }

    pub(crate) fn compute_with_text(
        &self,
        values: &[V],
        settled: bool,
        canvas: [f32; 2],
        resources: &[Slot],
        text: &mut crate::text::Text,
    ) -> Result<Vec<Style>, Error> {
        let fonts = RefCell::new(text);
        let metrics = |font: &super::Font| fonts.borrow_mut().x_height(font).map(f64::from);
        self.computed(values, settled, canvas, resources, Some(&metrics))
    }

    fn computed(
        &self,
        values: &[V],
        settled: bool,
        canvas: [f32; 2],
        resources: &[Slot],
        fonts: Option<FontMetrics<'_>>,
    ) -> Result<Vec<Style>, Error> {
        let frame = Frame {
            fonts,
            values,
            late: (!settled).then_some(&self.late[..]),
            canvas: canvas.map(f64::from),
            resources,
        };
        let mut out: Vec<Style> = Vec::with_capacity(self.nodes.len());
        let mut calcs = 0;
        for i in 0..self.nodes.len() {
            let parent = self.parents[i].map(|p| &out[p as usize]);
            let style = self.style(i, parent, &frame, calcs)?;
            calcs += style.calcs.len();
            out.push(style);
        }
        // As Chrome finds them in its document: the first element of each id.
        let mut ids = HashMap::new();
        for (i, s) in out.iter().enumerate() {
            if let Some(id) = s.svg.as_deref().and_then(|c| c.id.clone()) {
                ids.entry(id).or_insert(i);
            }
        }
        self.references(&out, &ids)?;
        svg::filter::prepare(&mut out, &self.parents)?;
        let mut copied = 0;
        for (u, style) in out.iter_mut().enumerate() {
            self.instantiate(u, style, &ids, &frame, 0, &mut copied)?;
        }
        Ok(out)
    }

    /// The nodes whose styles can move a box of `motionBlur: "auto"` in a frame of these
    /// `styles`, or change its layout, in preorder: all but those apart from such boxes, an
    /// `<svg>`'s elements, a `<Mask>`'s source and a box out of flow whose `position` no traced
    /// value sets, each with what is in it, none of which lays out or moves a box outside it.
    pub fn reaching(&self, styles: &[Style]) -> Vec<usize> {
        let n = self.nodes.len();
        let mut held = vec![false; n];
        for i in (0..n).filter(|&i| styles[i].motion_auto) {
            let mut at = Some(i);
            while let Some(a) = at.filter(|&a| !held[a]) {
                held[a] = true;
                at = self.parents[a].map(|p| p as usize);
            }
        }
        let traced = |b: &Bound, field: Field| {
            (b.rest.iter()).any(|(f, p, ..)| *f == field && matches!(p, Prop::Expr(_)))
        };
        let mut out = Vec::new();
        let mut i = 0;
        while i < n {
            let b = &self.nodes[i];
            let source = (self.parents[i])
                .and_then(|p| self.nodes[p as usize].source)
                .is_some_and(|s| s.root == i);
            let apart = b.svg.as_ref().is_some_and(|g| g.tag != Tag::Svg)
                || source
                || (styles[i].layout.position.is_out_of_flow() && !traced(b, Field::Position));
            if apart && !held[i] {
                i = self.ends[i] as usize;
            } else {
                out.push(i);
                i += 1;
            }
        }
        out
    }

    /// Calls `f` with each expression the styles of `nodes` read.
    pub fn reads(&self, nodes: &[usize], mut f: impl FnMut(u32)) {
        for &i in nodes {
            if let Some(table) = &self.instances[i] {
                table.reads(&mut f);
            }
            if let Some(table) = &self.svg_instances[i] {
                table.reads(&mut f);
            }
        }
        let mut prop = |p: &Prop| {
            if let Prop::Expr(e) = p {
                f(*e);
            }
        };
        for &i in nodes {
            let b = &self.nodes[i];
            b.rest.iter().for_each(|(_, p, ..)| prop(p));
            if let Some(g) = &b.svg {
                g.attributes.iter().for_each(|(_, p)| prop(p));
                if let Some(t) = &g.template {
                    for p in &t.parts {
                        if let Part::Expr(e) = p {
                            prop(&Prop::Expr(*e));
                        }
                    }
                }
            }
            if let Some(s) = &b.shader {
                s.uniforms.iter().flat_map(|(_, v)| v).for_each(&mut prop);
            }
        }
    }

    /// The computed styles of `nodes` alone, in preorder, each one's parent among them, for the
    /// program's settled values at one frame: as `compute` gives them, but for the copies of
    /// `<use>`s. Each numbers its calcs as the node's style in `like`, another frame's, does, so
    /// that their layouts compare.
    pub fn compute_only(
        &self,
        nodes: &[usize],
        values: &[V],
        canvas: [f32; 2],
        resources: &[Slot],
        like: &[Style],
        text: &mut crate::text::Text,
    ) -> Result<Vec<Style>, Error> {
        let fonts = RefCell::new(text);
        let metrics = |font: &super::Font| fonts.borrow_mut().x_height(font).map(f64::from);
        let frame = Frame {
            fonts: Some(&metrics),
            values,
            late: None,
            canvas: canvas.map(f64::from),
            resources,
        };
        // Where each node's style is in `out`.
        let mut at = vec![usize::MAX; self.nodes.len()];
        let mut out: Vec<Style> = Vec::with_capacity(nodes.len());
        let (mut calcs, mut counted) = (0, 0);
        for &i in nodes {
            calcs += (like[counted..i].iter())
                .map(|s| s.calcs.len())
                .sum::<usize>();
            counted = i;
            let parent = self.parents[i].map(|p| &out[at[p as usize]]);
            let style = self.style(i, parent, &frame, calcs)?;
            at[i] = out.len();
            out.push(style);
        }
        Ok(out)
    }

    /// Node `i`'s computed style under `parent`, after `calcs` calcs in the frame.
    pub(super) fn style(
        &self,
        i: usize,
        parent: Option<&Style>,
        frame: &Frame,
        calcs: usize,
    ) -> Result<Style, Error> {
        let b = &self.nodes[i];
        let declared = if b.rest.is_empty() {
            Cow::Borrowed(&b.declared)
        } else {
            let mut d = b.declared.clone();
            for (field, prop, what, attribute) in &b.rest {
                if frame.unsettled(prop) {
                    continue;
                }
                let v = match prop {
                    Prop::Const(v) => v,
                    Prop::Expr(e) => &frame.values[*e as usize],
                };
                super::cascade::apply(&mut d, *field, &presented(v, *attribute), parent)
                    .map_err(|e| format!("{}: {what}: {e}", b.place))?;
            }
            Cow::Owned(d)
        };
        let writing = declared
            .writing
            .or(parent.map(|p| p.font.writing))
            .unwrap_or_default();
        let rtl = declared.rtl.or(parent.map(|p| p.font.rtl)).unwrap_or(false);
        let declared =
            if (writing != super::WritingMode::Horizontal || rtl) && !b.logical.is_empty() {
                let mut d = Declared::default();
                let axis = match writing {
                    super::WritingMode::Horizontal => 0,
                    super::WritingMode::VerticalRl => 2,
                    super::WritingMode::VerticalLr => 4,
                } + usize::from(rtl);
                for (field, prop, what, attribute) in &b.logical {
                    if frame.unsettled(prop) {
                        continue;
                    }
                    let v = match prop {
                        Prop::Const(v) => v,
                        Prop::Expr(e) => &frame.values[*e as usize],
                    };
                    super::cascade::apply(&mut d, field[axis], &presented(v, *attribute), parent)
                        .map_err(|e| format!("{}: {what}: {e}", b.place))?;
                }
                Cow::Owned(d)
            } else {
                declared
            };
        if parent.is_some_and(|p| p.inline_content) && !b.inline_content {
            if b.view
                && !matches!(
                    declared.display,
                    super::declaration::Shown::InlineBlock
                        | super::declaration::Shown::InlineFlex
                        | super::declaration::Shown::InlineGrid
                        | super::declaration::Shown::None
                )
            {
                return Err(format!(
                    "{}: a View inside Text needs display: inline-block, inline-flex or inline-grid",
                    b.place
                ));
            }
            if matches!(
                declared.position,
                taffy::Position::Absolute | taffy::Position::Fixed
            ) {
                return Err(format!(
                    "{}: out-of-flow boxes inside Text are not supported; position the box beside the Text or inside an inline View",
                    b.place
                ));
            }
        }
        let (canvas, resources) = (frame.canvas, frame.resources);
        let mut base = b.base.borrow_mut();
        let kept = base.as_ref().and_then(|s| {
            if declared.ex
                || !s.matches(parent, canvas, calcs, resources)
                || parent.is_some_and(|p| p.specified.is_some())
            {
                return None;
            }
            let mut style = s.style.clone();
            b.updates
                .as_ref()?
                .apply(&mut style, &declared, canvas)
                .then_some(style)
        });
        let mut style = match kept {
            Some(style) => style,
            None => {
                let style = compute(&declared, parent, &frame.environment(), calcs, i, b.text)
                    .map_err(|e| format!("{}: {e}", b.place))?;
                if b.updates.is_some() {
                    *base = Some(Box::new(cache::Base::new(
                        parent,
                        canvas,
                        calcs,
                        resources,
                        style.clone(),
                    )));
                }
                style
            }
        };
        style.source = b.source;
        let nested = self.parents[i].is_some_and(|p| self.nodes[p as usize].svg.is_some());
        let boxed = (b.svg.as_ref()).is_none_or(|g| g.tag == Tag::Svg && !nested);
        if boxed && style.filter_ref.is_some() {
            return Err(format!(
                "{}: filter: url() is supported on elements inside an <svg>",
                b.place
            ));
        }
        if boxed && (style.clip_ref.is_some() || style.mask_ref.is_some()) {
            return Err(format!(
                "{}: clip-path: url() and mask: url() are supported in an <svg> only, not on a \
                 box yet",
                b.place
            ));
        }
        style.inline_content = b.inline_content;
        if let Some(g) = &b.svg {
            // Character data is not an element: it uses its containing text element's pen.
            if g.tag == Tag::TextRun {
                style.non_scaling_stroke = parent.is_some_and(|p| p.non_scaling_stroke);
            }
            let mut svg = g.computed(frame).map_err(|e| format!("{}: {e}", b.place))?;
            if svg.font_relative {
                let c = Rc::make_mut(&mut svg);
                c.element.resolve_font(style.font.size);
                c.font_relative = false;
            }
            if declared.transform_set && svg.transform.is_some() {
                Rc::make_mut(&mut svg).transform = None;
            }
            style.svg = Some(svg);
        }
        if let Some(s) = &b.shader {
            let shaded = s.computed(frame);
            style.shader = Some(shaded.map_err(|e| format!("{}: {} {e}", b.place, s.tag))?);
        }
        if let Some(table) = &self.instances[i] {
            style.instances = Some(table.compute(frame.values, &style, &frame.environment())?);
        }
        if let Some(table) = &self.svg_instances[i] {
            style.svg_instances = Some(table.compute(frame.values, parent)?);
        }
        let needed = self.inherited_by[i].iter().any(|prop| {
            if frame.unsettled(prop) {
                return false;
            }
            let value = match prop {
                Prop::Const(v) => v,
                Prop::Expr(e) => &frame.values[*e as usize],
            };
            matches!(
                super::cascade::Keyword::of(value),
                Some(super::cascade::Keyword::Inherit | super::cascade::Keyword::Unset)
            )
        });
        if needed || b.svg.as_ref().is_some_and(|g| g.tag == Tag::Use) {
            style.specified = Some(Rc::new(super::cascade::Snapshot(super::cascade::resolved(
                &declared, &style, canvas,
            ))));
        }
        Ok(style)
    }

    /// That each `clip-path: url(#id)` names a `<clipPath>`, and each `mask: url(#id)` a
    /// `<mask>`, by the frame's `ids`, from elements that are not clip paths, in them or masks;
    /// and that the markers of each path, line, polyline and polygon name `<marker>`s.
    pub(super) fn references(
        &self,
        styles: &[Style],
        ids: &HashMap<Rc<str>, usize>,
    ) -> Result<(), Error> {
        const MARKERS: [&str; 3] = ["marker-start", "marker-mid", "marker-end"];
        let tag = |i: usize| self.nodes[i].svg.as_ref().map(|g| g.tag);
        for (i, s) in styles.iter().enumerate() {
            let markable = matches!(
                tag(i),
                Some(Tag::Path | Tag::Line | Tag::Polyline | Tag::Polygon)
            );
            let markers = s.markers.iter().zip(MARKERS);
            for (id, name) in markers.filter_map(|(m, n)| Some((m.as_deref()?, n))) {
                if markable && ids.get(id).and_then(|&n| tag(n)) != Some(Tag::Marker) {
                    let place = &self.nodes[i].place;
                    return Err(format!("{place}: {name}: url(#{id}) names no <marker>"));
                }
            }
        }
        let refers =
            |s: &&Style| s.clip_ref.is_some() || s.mask_ref.is_some() || s.filter_ref.is_some();
        for (i, s) in styles.iter().enumerate().filter(|(_, s)| refers(s)) {
            let place = &self.nodes[i].place;
            let parent = self.parents[i].and_then(|p| tag(p as usize));
            for (id, want, name) in [
                (&s.clip_ref, Tag::ClipPath, "clip-path"),
                (&s.mask_ref, Tag::Mask, "mask"),
                (&s.filter_ref, Tag::Filter, "filter"),
            ] {
                let Some(id) = id.as_deref() else {
                    continue;
                };
                if tag(i) == Some(Tag::Mask)
                    || name != "clip-path"
                        && (tag(i) == Some(Tag::ClipPath) || parent == Some(Tag::ClipPath))
                {
                    return Err(format!(
                        "{place}: {name}: url() on a <clipPath> or a <mask>, or in a <clipPath>, \
                         is not supported yet"
                    ));
                }
                if ids.get(id).and_then(|&n| tag(n)) != Some(want) {
                    return Err(format!(
                        "{place}: {name}: url(#{id}) names no <{}>",
                        want.name()
                    ));
                }
            }
        }
        Ok(())
    }

    /// The copy that `<use>` `u` with `style` makes of the element its href names, by the
    /// frame's `ids`, and the copies the `<use>`s in it make, `depth` copies deep; `copied`
    /// counts the elements the frame's copies hold.
    pub(super) fn instantiate(
        &self,
        u: usize,
        style: &mut Style,
        ids: &HashMap<Rc<str>, usize>,
        frame: &Frame,
        depth: usize,
        copied: &mut usize,
    ) -> Result<(), Error> {
        const DEEPEST: usize = 8;
        const MOST: usize = 100_000;
        let place = &self.nodes[u].place;
        let tag = |i: usize| self.nodes[i].svg.as_ref().map(|g| g.tag);
        let (r, pattern) = match style.svg.as_deref().map(|c| &c.element) {
            Some(svg::Element::Use { href, .. }) => {
                let r = ids
                    .get(href)
                    .copied()
                    .ok_or_else(|| format!("{place}: <use> href: #{href} names no element"))?;
                (r, false)
            }
            Some(svg::Element::Pattern(p)) if self.ends[u] as usize == u + 1 => {
                let mut next = p.href.clone();
                let mut visited = vec![u];
                let mut source = None;
                while let Some(id) = next {
                    let Some(&n) = ids.get(&id) else {
                        break;
                    };
                    if visited.contains(&n) {
                        break;
                    }
                    visited.push(n);
                    let Some(g) = &self.nodes[n].svg else {
                        break;
                    };
                    let c = g.computed(frame)?;
                    let svg::Element::Pattern(p) = &c.element else {
                        break;
                    };
                    if self.ends[n] as usize > n + 1 {
                        source = Some(n);
                        break;
                    }
                    next = p.href.clone();
                }
                let Some(r) = source else {
                    return Ok(());
                };
                (r, true)
            }
            _ => return Ok(()),
        };
        let copies = pattern
            || matches!(
                tag(r),
                Some(
                    Tag::G
                        | Tag::Text
                        | Tag::Path
                        | Tag::Circle
                        | Tag::Ellipse
                        | Tag::Rect
                        | Tag::Line
                        | Tag::Polyline
                        | Tag::Polygon
                        | Tag::Use
                        | Tag::Symbol
                        | Tag::Image
                )
            );
        if !copies {
            let href = match &style.svg.as_deref().unwrap().element {
                svg::Element::Use { href, .. } => &**href,
                _ => "",
            };
            let name = tag(r).map_or("", Tag::name);
            return Err(format!(
                "{place}: <use> href: #{href} names a <{name}>, which a <use> does not copy"
            ));
        }
        let end = self.ends[r] as usize;
        let href = style
            .svg
            .as_deref()
            .and_then(|c| match &c.element {
                svg::Element::Use { href, .. } => Some(&**href),
                _ => None,
            })
            .unwrap_or("");
        if (r..end).contains(&u) {
            return Err(format!(
                "{place}: <use> href: #{href} holds the <use>, which would copy itself"
            ));
        }
        if depth >= DEEPEST {
            return Err(format!(
                "{place}: <use>s copy <use>s more than {DEEPEST} deep"
            ));
        }
        *copied += end - r;
        if *copied > MOST {
            return Err(format!(
                "{place}: the frame's <use>s copy more than {MOST} elements"
            ));
        }
        let mut styles: Vec<Style> = Vec::with_capacity(end - r);
        for n in r..end {
            if pattern && n == r {
                let mut root = style.clone();
                root.instance = None;
                styles.push(root);
                continue;
            }
            let parent = match self.parents[n] {
                Some(p) if n > r => &styles[p as usize - r],
                _ => &*style,
            };
            let s = self.style(n, Some(parent), frame, 0)?;
            styles.push(s);
        }
        for (k, s) in styles.iter_mut().enumerate().skip(usize::from(pattern)) {
            self.instantiate(r + k, s, ids, frame, depth + 1, copied)?;
        }
        style.instance = Some(Rc::new(Instance { root: r, styles }));
        Ok(())
    }
}

/// What a frame's styles are computed for: the program's values, the canvas's size in px, and
/// the scene's resources.
pub(super) struct Frame<'a> {
    pub(super) fonts: Option<FontMetrics<'a>>,
    pub(super) values: &'a [V],
    /// The late expressions, to leave out, while the values are not settled.
    pub(super) late: Option<&'a [bool]>,
    pub(super) canvas: [f64; 2],
    pub(super) resources: &'a [Slot],
}

impl Frame<'_> {
    fn environment(&self) -> Environment<'_> {
        Environment {
            canvas: self.canvas,
            resources: self.resources,
            fonts: self.fonts,
        }
    }

    /// Whether a declaration of `prop` waits for the values to settle.
    pub(super) fn unsettled(&self, prop: &Prop) -> bool {
        matches!((prop, self.late), (Prop::Expr(e), Some(late)) if late.get(*e as usize) == Some(&true))
    }
}

/// A declaration as written: its key, its value, what errors call it, and whether it is a
/// presentation attribute.
pub(super) type Written<'a> = (Rc<str>, Cow<'a, Prop>, Rc<str>, bool);

/// A node's declarations, with those its classes make.
fn bind(
    node: &Node,
    classes: &[tailwind::Declaration],
    late: &[bool],
    nested: bool,
) -> Result<Bound, Error> {
    let place = node.place();
    let late_prop = |p: &Prop| matches!(p, Prop::Expr(e) if late.get(*e as usize) == Some(&true));
    if let Kind::Text { content } | Kind::Span { content, .. } = &node.kind
        && content
            .iter()
            .any(|p| matches!(p, Piece::Text(t) if late_prop(t)))
    {
        return Err(format!(
            "{place}: {LATE}, so it cannot be text, which is laid out"
        ));
    }
    let mut declared = Declared::default();
    let mut rest = Vec::new();
    let (tag, attributes, shape_template) = match &node.kind {
        Kind::Svg {
            tag,
            attributes,
            geometry,
        } => (Some(*tag), &attributes[..], geometry),
        _ => (None, &[][..], &None),
    };
    if matches!(&node.kind, Kind::Svg { tag, .. } if *tag != Tag::Svg || nested) {
        declared.transform_origin = [const {
            super::position::Offset {
                end: false,
                by: super::declaration::ZERO,
            }
        }; 2];
    }
    // SVG viewports and markers clip by default through the user-agent style.
    if matches!(tag, Some(Tag::Svg | Tag::Marker)) {
        declared.overflow = pair(&V::Str("hidden".into()), overflow)?.into();
    }
    // Presentation attributes, the classes, the style, then the important classes, each over
    // what comes before: each with its key, what errors call it, and whether it is an attribute.
    let class = |d: &tailwind::Declaration| {
        let what = format!("className {}: {}", d.class, d.property);
        let prop = Prop::Const(V::Str(d.value.as_str().into()));
        (
            camel(&d.property).into(),
            Cow::Owned(prop),
            what.into(),
            false,
        )
    };
    let mut written: Vec<Written> = Vec::new();
    for (k, p) in attributes {
        if tag.is_some_and(|t| svg::presentational(t, k)) {
            written.push((
                k.clone(),
                Cow::Borrowed(p),
                format!("attribute {k}").into(),
                true,
            ));
        }
    }
    written.extend(classes.iter().filter(|d| !d.important).map(class));
    for (k, p) in &node.style {
        written.push((
            k.clone(),
            Cow::Borrowed(p),
            format!("style {k}").into(),
            false,
        ));
    }
    written.extend(classes.iter().filter(|d| d.important).map(class));
    let modes = [
        super::WritingMode::Horizontal,
        super::WritingMode::VerticalRl,
        super::WritingMode::VerticalLr,
    ];
    let logical = if written.iter().any(|(key, ..)| {
        Field::by_name(key).is_some_and(|f| {
            modes
                .iter()
                .any(|&m| [false, true].iter().any(|&rtl| f.flow(key, m, rtl) != f))
        })
    }) {
        written
            .iter()
            .filter_map(|(key, prop, what, attribute)| {
                Field::by_name(key).map(|f| {
                    (
                        std::array::from_fn(|k| f.flow(key, modes[k / 2], k % 2 == 1)),
                        prop.as_ref().clone(),
                        what.clone(),
                        *attribute,
                    )
                })
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    for (key, prop, what, attribute) in written {
        let field = (Field::by_name(&key))
            .ok_or_else(|| format!("{place}: {}", unsupported(&what, &key)))?;
        if field == Field::XmlSpace && (!attribute || tag.is_none()) {
            return Err(format!(
                "{place}: xmlSpace is an SVG attribute, not a CSS property"
            ));
        }
        let unit = matches!(node.kind, Kind::Span { unit: true, .. });
        if unit && !field.textual() && !field.on_units() {
            return Err(format!(
                "{place}: {what}: a unit of a split text takes only text properties, opacity, \
                 transforms and filter"
            ));
        }
        if matches!(node.kind, Kind::Span { unit: false, .. })
            && !field.textual()
            && !field.inline_box()
        {
            return Err(format!(
                "{place}: {what}: a <Span> is part of a text, not a box, so it takes text properties, background, padding and border"
            ));
        }
        if tag.is_some_and(|t| t != Tag::Svg)
            && !field.drawn_in_svg()
            && !(tag == Some(Tag::Marker) && field == Field::Overflow)
        {
            return Err(format!(
                "{place}: {what} on an element in an <svg> is not supported yet"
            ));
        }
        let at = |e: String| format!("{place}: {what}: {e}");
        let drawn = tag.is_some_and(|t| t != Tag::Svg);
        if late_prop(&prop) && !drawn && !field.paints_only() {
            return Err(at(format!(
                "{LATE}, so it sets only what does not change layout, such as translate, \
                 opacity, colors or what an <svg> draws"
            )));
        }
        match &*prop {
            Prop::Const(v) if rest.is_empty() && super::cascade::Keyword::of(v).is_none() => {
                set(&mut declared, field, &presented(v, attribute)).map_err(at)?;
            }
            Prop::Const(v) => {
                // Checked now, so a bad constant fails on the first frame as it would up front.
                set(&mut Declared::default(), field, &presented(v, attribute)).map_err(at)?;
                rest.push((field, prop.into_owned(), what, attribute));
            }
            Prop::Expr(_) => rest.push((field, prop.into_owned(), what, attribute)),
        }
    }
    let svg = match tag {
        Some(tag) => {
            let geometry = attributes.iter().filter(|(k, _)| {
                !svg::presentational(tag, k)
                    || (nested && tag == Tag::Svg && matches!(k.as_ref(), "width" | "height"))
            });
            if let Some((k, _)) = geometry
                .clone()
                .find(|(_, p)| tag == Tag::Svg && !nested && late_prop(p))
            {
                return Err(format!(
                    "{place}: attribute {k}: {LATE}, and an <svg>'s {k} changes its layout"
                ));
            }
            let mut g = Geometry {
                tag,
                attributes: geometry.cloned().collect(),
                template: shape_template
                    .as_ref()
                    .map(|p| svg::Template::new(tag, p.clone())),
                fixed: None,
            };
            if g.attributes
                .iter()
                .map(|(_, p)| p)
                .all(|p| matches!(p, Prop::Const(_)))
                && g.template.is_none()
            {
                let none = Frame {
                    fonts: None,
                    values: &[],
                    late: None,
                    canvas: [0.0; 2],
                    resources: &[],
                };
                g.fixed = Some(g.computed(&none).map_err(|e| format!("{place}: {e}"))?);
            }
            Some(g)
        }
        None => None,
    };
    let source = match node.kind {
        Kind::Mask {
            source,
            luminance,
            invert,
        } => Some(Source {
            root: source as usize,
            luminance,
            invert,
        }),
        _ => None,
    };
    let shader = match &node.kind {
        Kind::Shader {
            code,
            uniforms,
            inputs,
            outset,
            transition,
            kind,
        } => {
            let tag = if *transition {
                "<Transition>"
            } else {
                "<ShaderLayer>"
            };
            let at = |e: String| format!("{place}: {tag} code {e}");
            let mut images = match transition {
                true => transition::IMAGES.to_vec(),
                false => vec!["content", "backdrop"],
            };
            for (name, _) in inputs {
                if images.contains(&&**name) {
                    return Err(format!(
                        "{place}: {tag} inputs {name}: content and backdrop are the layer's own"
                    ));
                }
                images.push(&**name);
            }
            let program = shader::program(code, &images).map_err(at)?;
            // A built-in transition takes its parameters only.
            let limits = kind
                .as_deref()
                .and_then(transition::kind)
                .map_or(&[][..], |k| k.params);
            if let Some(kind) = kind {
                for (name, _) in uniforms.iter().filter(|(k, _)| &**k != "progress") {
                    if !limits.iter().any(|p| p.name == &**name) {
                        let names: Vec<&str> = limits.iter().map(|p| p.name).collect();
                        let takes = match names.is_empty() {
                            true => "it takes none".to_string(),
                            false => format!("it takes {}", names.join(", ")),
                        };
                        return Err(format!(
                            "{place}: {tag} {kind} has no param {name}; {takes}"
                        ));
                    }
                }
            }
            let mut s = Shading {
                program,
                tag,
                limits,
                transition: *transition,
                uniforms: uniforms.clone(),
                inputs: inputs.clone(),
                outset: *outset,
                fixed: None,
            };
            if (uniforms.iter())
                .flat_map(|(_, v)| v)
                .all(|p| matches!(p, Prop::Const(_)))
            {
                let none = Frame {
                    fonts: None,
                    values: &[],
                    late: None,
                    canvas: [0.0; 2],
                    resources: &[],
                };
                let block = s
                    .computed(&none)
                    .map_err(|e| format!("{place}: {tag} {e}"))?;
                s.fixed = Some(block.block);
            }
            Some(s)
        }
        _ => None,
    };
    Ok(Bound {
        updates: if logical.is_empty() {
            cache::Updates::new(&rest)
        } else {
            None
        },
        logical,
        base: RefCell::default(),
        declared,
        view: matches!(node.kind, Kind::View),
        inline_content: matches!(node.kind, Kind::Text { .. } | Kind::Span { .. }),
        text: matches!(
            node.kind,
            Kind::Text { .. } | Kind::Svg { tag: Tag::Text, .. }
        ),
        rest,
        place,
        source,
        svg,
        shader,
    })
}

/// Why a value made from `bounds()` cannot go where layout reads it.
const LATE: &str = "a value from bounds() is known only after layout";

#[cfg(test)]
mod tests {

    use std::rc::Rc;

    use taffy::{Dimension, Display, Overflow};

    use crate::style::math::Math;

    use crate::svg;
    use crate::template::{Kind, Node, Prop, Tag, V};

    use super::*;
    use crate::style::*;
    use crate::testing::styles::{
        CANVAS, auto_px, error, num, one, px, resources, sheet, styles, text, view,
    };

    #[test]
    fn paint_order_inherits_completes_omitted_parts_and_rejects_duplicates() {
        use PaintPart::{Fill, Markers, Stroke};
        assert_eq!(one(&[]).paint_order, [Fill, Stroke, Markers]);
        for (value, expected) in [
            ("normal", [Fill, Stroke, Markers]),
            ("stroke", [Stroke, Fill, Markers]),
            ("markers stroke", [Markers, Stroke, Fill]),
            ("fill markers stroke", [Fill, Markers, Stroke]),
            ("STROKE FILL", [Stroke, Fill, Markers]),
        ] {
            assert_eq!(one(&[("paintOrder", text(value))]).paint_order, expected);
        }
        for value in [
            "",
            "none",
            "fill fill",
            "normal stroke",
            "fill stroke markers fill",
        ] {
            assert!(error(&[("paintOrder", text(value))]).contains("paintOrder"));
        }
        for (value, expected) in [
            ("inherit", [Markers, Stroke, Fill]),
            ("unset", [Markers, Stroke, Fill]),
            ("initial", [Fill, Stroke, Markers]),
            ("normal", [Fill, Stroke, Markers]),
        ] {
            let tree = vec![
                view(&[("paintOrder", text("markers stroke"))], &[1, 2]),
                view(&[], &[]),
                view(
                    &[("paintOrder", text("fill")), ("paintOrder", text(value))],
                    &[],
                ),
            ];
            let s = styles(tree, &[]).unwrap();
            assert_eq!(s[1].paint_order, [Markers, Stroke, Fill]);
            assert_eq!(s[2].paint_order, expected);
        }
    }

    #[test]
    fn visibility_inherits() {
        let tree = vec![
            view(&[("visibility", text("hidden"))], &[1, 2]),
            view(&[], &[]),
            view(&[("visibility", text("Visible"))], &[]),
        ];
        let s = styles(tree, &[]).unwrap();
        assert_eq!(
            s.iter().map(|s| s.visible).collect::<Vec<_>>(),
            [false, false, true]
        );
        assert!(!one(&[("visibility", text("collapse"))]).visible);
        assert!(error(&[("visibility", text("none"))]).contains("none is not one of visible"));
    }

    #[test]
    fn currentcolor_mixtures_follow_the_consuming_element_after_inheritance() {
        let mix = "color-mix(in srgb, currentColor, white)";
        let s = styles(
            vec![
                view(
                    &[
                        ("color", text("red")),
                        ("fill", text(mix)),
                        ("WebkitTextFillColor", text(mix)),
                        ("WebkitTextStrokeColor", text(mix)),
                        ("textEmphasisColor", text(mix)),
                        (
                            "textShadow",
                            text("1px 2px color-mix(in srgb, currentColor, white)"),
                        ),
                    ],
                    &[1],
                ),
                view(
                    &[
                        ("color", text("blue")),
                        ("background", text(mix)),
                        (
                            "border",
                            text("2px solid color-mix(in srgb, currentColor, white)"),
                        ),
                    ],
                    &[],
                ),
            ],
            &[],
        )
        .unwrap();
        let child = &s[1];
        let blue = [0.5, 0.5, 1.0, 1.0];
        assert_eq!(child.background, blue);
        assert_eq!(child.border_colors, [blue; 4]);
        for c in [
            &child.font.fill,
            &child.font.stroke_color,
            &child.font.emphasis_color,
            &child.font.shadows[0].color,
        ] {
            assert_eq!(color::resolve(c, child.color), blue);
        }
        let Paint::Mixed(fill) = &child.fill else {
            panic!("inherited mixture")
        };
        assert_eq!(fill.resolve(child.color), blue);
        assert_eq!(
            color::resolve(&s[0].font.fill, s[0].color),
            [1.0, 0.5, 0.5, 1.0]
        );
    }

    #[test]
    fn currentcolor_in_the_color_property_uses_the_inherited_color() {
        let s = styles(
            vec![
                view(&[("color", text("red"))], &[1]),
                view(
                    &[
                        ("color", text("color-mix(in srgb, currentColor, blue)")),
                        (
                            "backgroundColor",
                            text("color-mix(in srgb, currentColor, white)"),
                        ),
                    ],
                    &[],
                ),
            ],
            &[],
        )
        .unwrap();
        assert_eq!(s[1].color, [0.5, 0.0, 0.5, 1.0]);
        assert_eq!(s[1].background, [0.75, 0.5, 0.75, 1.0]);
    }

    #[test]
    fn color_currentcolor_inherits() {
        let tree = vec![
            view(&[("color", text("red"))], &[1]),
            view(
                &[("color", text("blue")), ("color", text("CurrentColor"))],
                &[],
            ),
        ];
        let s = styles(tree, &[]).unwrap();
        assert_eq!(s[1].color, color::parse("red").unwrap());
    }

    #[test]
    fn inherit_takes_back_what_an_earlier_declaration_set() {
        let tree = vec![
            view(&[("color", text("red"))], &[1]),
            view(&[("color", text("blue")), ("color", text("inherit"))], &[]),
        ];
        let s = styles(tree, &[]).unwrap();
        assert_eq!(s[1].color, color::parse("red").unwrap());
    }

    #[test]
    fn a_mask_source_inherits_from_the_mask() {
        let mut tree = vec![
            view(&[("color", text("red"))], &[2]),
            view(&[], &[]),
            view(&[], &[]),
        ];
        tree[0].kind = Kind::Mask {
            source: 1,
            luminance: true,
            invert: false,
        };
        let s = styles(tree, &[]).unwrap();
        let source = Source {
            root: 1,
            luminance: true,
            invert: false,
        };
        assert_eq!(s[0].source, Some(source));
        assert_eq!((s[1].source, s[1].color), (None, s[0].color));
        assert!(!s[0].keeps_3d());
    }

    #[test]
    fn what_reaches_a_blurred_box_is_all_but_what_lays_out_apart() {
        let absolute = || ("position", text("absolute"));
        let mut tree = vec![
            view(&[], &[1, 2, 4, 5, 7, 9]),
            view(&[], &[]),
            // Out of flow at every frame, with what is in it.
            view(&[absolute()], &[3]),
            view(&[("width", text("calc(50% + 1px)"))], &[]),
            // Out of flow at this one only.
            view(
                &[
                    ("position", Prop::Expr(0)),
                    ("width", text("calc(50% + 2px)")),
                ],
                &[],
            ),
            // Out of flow, around the blurred box.
            view(&[absolute()], &[6]),
            view(&[absolute(), ("motionBlur", text("auto"))], &[]),
            // An <svg>'s elements, and a mask's source.
            view(&[], &[8]),
            view(&[], &[]),
            view(&[], &[]),
            view(&[], &[]),
        ];
        let svg = |tag| Kind::Svg {
            geometry: None,
            tag,
            attributes: Vec::new(),
        };
        tree[7].kind = svg(Tag::Svg);
        tree[8].kind = svg(Tag::Circle);
        tree[9].kind = Kind::Mask {
            source: 10,
            luminance: false,
            invert: false,
        };
        let sheet = sheet(tree).unwrap();
        let values = [V::Str("absolute".into())];
        let styles = sheet.compute(&values, true, CANVAS, &resources()).unwrap();
        assert_eq!(sheet.reaching(&styles), [0, 1, 4, 5, 6, 7, 9]);
        // Alone, as `compute` gives them, their calcs numbered as there.
        let only = sheet.reaching(&styles);
        let alone = (sheet.compute_only(
            &only,
            &values,
            CANVAS,
            &resources(),
            &styles,
            &mut crate::testing::text(),
        ))
        .unwrap();
        assert!(only.iter().zip(&alone).all(|(&i, s)| *s == styles[i]));
    }

    #[test]
    fn a_span_takes_text_and_inline_box_properties() {
        let mut tree = vec![view(&[("fontSize", num(20.0))], &[1]), view(&[], &[])];
        let span = |style: &[(&str, Prop)]| Node {
            kind: Kind::Span {
                content: Vec::new(),
                unit: false,
            },
            ..view(style, &[])
        };
        tree[1] = span(&[("color", text("red")), ("letterSpacing", num(1.0))]);
        let s = styles(tree.clone(), &[]).unwrap();
        assert_eq!((s[1].font.size, s[1].font.spacing), (20.0, 1.0));
        tree[1] = span(&[("padding", num(1.0))]);
        assert!(styles(tree.clone(), &[]).is_ok());
        // perUnit's effect style retains its existing property set. Ordinary Spans
        // decorate text; split units additionally own opacity, filters and transforms.
        if let Kind::Span { unit, .. } = &mut tree[1].kind {
            *unit = true;
        }
        assert!(
            styles(tree.clone(), &[])
                .unwrap_err()
                .contains("a unit of a split text")
        );
        tree[1] = span(&[("width", num(1.0))]);
        let e = styles(tree, &[]).unwrap_err();
        assert!(
            e.ends_with(
                "style width: a <Span> is part of a text, not a box, so it takes text properties, background, padding and border"
            ),
            "{e}"
        );
    }

    #[test]
    fn svg_elements_take_presentation_attributes_under_their_style() {
        let svg =
            |tag, attributes: &[(&str, Prop)], style: &[(&str, Prop)], children: &[u32]| Node {
                kind: Kind::Svg {
                    geometry: None,
                    tag,
                    attributes: (attributes.iter())
                        .map(|(k, p)| (Rc::from(*k), p.clone()))
                        .collect(),
                },
                ..view(style, children)
            };
        let root = [
            ("width", text("40")),
            ("viewBox", text("0 0 4 2")),
            ("fill", text("red")),
        ];
        let mut tree = vec![
            svg(Tag::Svg, &root, &[], &[1, 2]),
            svg(
                Tag::Circle,
                &[("r", num(1.0)), ("fillOpacity", text("50%"))],
                &[("fill", text("none"))],
                &[],
            ),
            svg(
                Tag::Rect,
                &[("width", Prop::Expr(0)), ("color", text("blue"))],
                &[("fill", text("currentColor"))],
                &[],
            ),
        ];
        let s = styles(tree.clone(), &[V::Num(2.0)]).unwrap();
        // An <svg>'s width is its box's, a number in px, and it clips what it draws.
        assert_eq!(s[0].layout.size.width, Dimension::length(40.0));
        assert_eq!(s[0].layout.overflow.x, Overflow::Hidden);
        let red = color::parse("red").unwrap();
        assert_eq!((&s[0].fill, s[0].fill_opacity), (&Paint::Color(red), 1.0));
        // The style beats the attribute.
        assert_eq!((&s[1].fill, s[1].fill_opacity), (&Paint::None, 0.5));
        assert_eq!(s[2].fill, Paint::CurrentColor);
        assert_eq!(s[2].color, color::parse("blue").unwrap());
        let rect = s[2].svg.as_deref().map(|c| &c.element);
        let Some(svg::Element::Shape(svg::Shape::Rect { size, .. })) = rect else {
            panic!("{rect:?}");
        };
        assert_eq!(size[0].units, 2.0);
        for (circle, style, want) in [
            (vec![("r", num(-1.0))], vec![], "<circle> r is negative"),
            (
                vec![],
                vec![("width", num(4.0))],
                "style width on an element in an <svg> is not supported yet",
            ),
            (
                vec![("fill", text("url(g)"))],
                vec![],
                "attribute fill: url(g): url() names an SVG element, as url(#id)",
            ),
            (
                vec![("fillRule", text("even-odd"))],
                vec![],
                "attribute fillRule: ",
            ),
        ] {
            tree[1] = svg(Tag::Circle, &circle, &style, &[]);
            let e = styles(tree.clone(), &[V::Num(2.0)]).unwrap_err();
            assert!(e.starts_with("s.tsx:1:1: ") && e.contains(want), "{e}");
        }
    }

    #[test]
    fn markers_inherit_and_name_marker_elements() {
        let svg = |tag, attributes: &[(&str, Prop)], children: &[u32]| Node {
            kind: Kind::Svg {
                geometry: None,
                tag,
                attributes: (attributes.iter())
                    .map(|(k, p)| (Rc::from(*k), p.clone()))
                    .collect(),
            },
            ..view(&[], children)
        };
        let tree = |g: &[(&str, Prop)]| {
            vec![
                svg(Tag::Svg, &[], &[1, 4]),
                svg(Tag::G, g, &[2, 3]),
                svg(Tag::Path, &[("markerStart", text("none"))], &[]),
                svg(Tag::Circle, &[], &[]),
                svg(Tag::Marker, &[("id", text("m"))], &[]),
            ]
        };
        let s = styles(tree(&[("markerEnd", text("url(#m)"))]), &[]).unwrap();
        assert_eq!(s[2].markers, [None, None, Some("m".into())]);
        let all = Node {
            style: vec![("marker".into(), text("url(#m)"))],
            ..svg(Tag::G, &[("markerMid", text("none"))], &[2, 3])
        };
        let mut nodes = tree(&[]);
        nodes[1] = all;
        let s = styles(nodes, &[]).unwrap();
        // The shorthand sets all three; the path's own start beats what it inherits.
        let m = Some(Rc::from("m"));
        assert_eq!(s[1].markers, [m.clone(), m.clone(), m.clone()]);
        assert_eq!(s[2].markers, [None, m.clone(), m]);
        // A shape that takes no markers inherits the name of none unchecked.
        let e = styles(tree(&[("markerMid", text("url(#x)"))]), &[]).unwrap_err();
        assert!(e.ends_with("marker-mid: url(#x) names no <marker>"), "{e}");
        let e = error(&[("markerEnd", text("red"))]);
        assert!(
            e.contains("a marker is url(#id) of a <marker>, or none"),
            "{e}"
        );
    }

    #[test]
    fn uses_copy_with_styles_inherited_from_them() {
        let svg = |tag, attributes: &[(&str, Prop)], children: &[u32]| Node {
            kind: Kind::Svg {
                geometry: None,
                tag,
                attributes: (attributes.iter())
                    .map(|(k, p)| (Rc::from(*k), p.clone()))
                    .collect(),
            },
            ..view(&[], children)
        };
        let tree = vec![
            svg(Tag::Svg, &[("id", text("s"))], &[1, 4]),
            svg(Tag::Symbol, &[("id", text("icon"))], &[2]),
            svg(Tag::G, &[("id", text("g"))], &[3]),
            svg(Tag::Circle, &[], &[]),
            svg(
                Tag::Use,
                &[("href", text("#icon")), ("fill", text("red"))],
                &[],
            ),
        ];
        let s = styles(tree.clone(), &[]).unwrap();
        let copy = s[4].instance.as_deref().unwrap();
        assert_eq!((copy.root, copy.styles.len()), (1, 3));
        let red = color::parse("red").unwrap();
        assert_eq!(copy.style(3).map(|s| &s.fill), Some(&Paint::Color(red)));
        assert_eq!(copy.style(4), None);
        assert_eq!(s[3].fill, Paint::Color(color::BLACK));
        for (n, href, want) in [
            (4, "#nope", "<use> href: #nope names no element"),
            (4, "#s", "names a <svg>, which a <use> does not copy"),
            (3, "#g", "holds the <use>, which would copy itself"),
        ] {
            let mut tree = tree.clone();
            tree[n] = svg(Tag::Use, &[("href", text(href))], &[]);
            let e = styles(tree, &[]).unwrap_err();
            assert!(e.contains(want), "{e}");
        }
    }

    #[test]
    fn svg_references_name_clip_paths_and_masks() {
        let svg = |tag, attributes: &[(&str, Prop)], children: &[u32]| Node {
            kind: Kind::Svg {
                geometry: None,
                tag,
                attributes: (attributes.iter())
                    .map(|(k, p)| (Rc::from(*k), p.clone()))
                    .collect(),
            },
            ..view(&[], children)
        };
        let tree = vec![
            svg(Tag::Svg, &[], &[1, 2, 3]),
            svg(
                Tag::ClipPath,
                &[("id", text("c")), ("clipRule", text("evenodd"))],
                &[4],
            ),
            svg(
                Tag::Mask,
                &[("id", text("m")), ("maskType", text("alpha"))],
                &[5],
            ),
            svg(
                Tag::Rect,
                &[
                    ("clipPath", text("url(#c)")),
                    ("mask", text(" URL( '#m' ) ")),
                ],
                &[],
            ),
            svg(Tag::Rect, &[], &[]),
            svg(Tag::Circle, &[], &[]),
        ];
        let s = styles(tree.clone(), &[]).unwrap();
        assert_eq!(s[3].clip_ref.as_deref(), Some("c"));
        assert_eq!(s[3].mask_ref.as_deref(), Some("m"));
        // The clip rule is inherited; the mask type is not.
        assert!(s[4].clip_even_odd && s[2].mask_alpha && !s[5].mask_alpha);
        for (n, attribute, want) in [
            (
                3,
                ("clipPath", "url(#m)"),
                "clip-path: url(#m) names no <clipPath>",
            ),
            (3, ("mask", "url(#n)"), "mask: url(#n) names no <mask>"),
            (2, ("mask", "url(#m)"), "on a <clipPath> or a <mask>"),
            (0, ("mask", "url(#m)"), "not on a box yet"),
        ] {
            let mut tree = tree.clone();
            let Kind::Svg { tag, .. } = tree[n].kind else {
                unreachable!()
            };
            let children = tree[n].children.clone();
            tree[n] = svg(tag, &[(attribute.0, text(attribute.1))], &children);
            let e = styles(tree, &[]).unwrap_err();
            assert!(e.contains(want), "{e}");
        }
        let e = error(&[("clipPath", text("url(#c)"))]);
        assert!(e.contains("not on a box yet"), "{e}");
        let e = error(&[("mask", text("linear-gradient(#000, #fff)"))]);
        assert!(e.contains("a box takes maskImage"), "{e}");
    }

    #[test]
    fn svg_strokes_inherit_with_their_lengths_in_px_and_percentages() {
        let tree = vec![
            view(
                &[
                    ("fontSize", num(20.0)),
                    ("stroke", text("red")),
                    ("strokeWidth", text("0.5em")),
                    ("strokeDasharray", text("1, 2 5%")),
                ],
                &[1],
            ),
            view(
                &[
                    ("strokeOpacity", text("50%")),
                    ("strokeLinecap", text("ROUND")),
                    ("strokeLinejoin", text("bevel")),
                    ("strokeMiterlimit", num(2.0)),
                    ("strokeDashoffset", num(-3.0)),
                ],
                &[],
            ),
        ];
        let s = styles(tree, &[]).unwrap();
        let k = &s[1].stroke;
        let red = color::parse("red").unwrap();
        assert_eq!((&k.paint, k.opacity), (&Paint::Color(red), 0.5));
        // An em is the declaring element's.
        assert_eq!(k.width, Math::Sum([10.0, 0.0]));
        let dashes = [[1.0, 0.0], [2.0, 0.0], [0.0, 0.05]].map(Math::Sum);
        assert_eq!(k.dashes[..], dashes);
        assert_eq!(k.dash_offset, Math::Sum([-3.0, 0.0]));
        assert_eq!(
            (k.cap, k.join, k.miter_limit),
            (Cap::Round, Join::Bevel, 2.0)
        );
        assert_eq!(one(&[]).stroke, Stroke::initial());
        // SVG lengths take numbers, in px.
        let width = one(&[("strokeWidth", text("2"))]).stroke.width;
        assert_eq!(width, Math::Sum([2.0, 0.0]));
        for (key, value, want) in [
            ("strokeWidth", num(-1.0), "-1 is negative"),
            ("strokeMiterlimit", num(0.5), "0.5 is below 1"),
            (
                "strokeLinejoin",
                text("arcs"),
                "arcs is not one of miter, round, bevel",
            ),
            ("strokeDasharray", text("1 -2"), "-2 is negative"),
            ("strokeDasharray", text(","), "there are no dashes"),
            (
                "stroke",
                text("url(#g) red"),
                "url(#g) red: what follows url() is not supported yet",
            ),
            (
                "stopColor",
                text("none"),
                "a stop's color is a color or currentColor",
            ),
        ] {
            let e = error(&[(key, value)]);
            assert!(e.contains(want), "{e}");
        }
    }

    #[test]
    fn expressions_are_set_in_declaration_order() {
        let padding = |style: &[(&str, Prop)]| {
            let s = styles(vec![view(style, &[])], &[V::Num(10.0)]).unwrap();
            (s[0].layout.padding.top, s[0].layout.padding.left)
        };
        // A constant shorthand after a traced side overrides it, and the other way round.
        assert_eq!(
            padding(&[("paddingTop", Prop::Expr(0)), ("padding", num(4.0))]),
            (px(4.0), px(4.0))
        );
        assert_eq!(
            padding(&[("padding", num(4.0)), ("paddingTop", Prop::Expr(0))]),
            (px(10.0), px(4.0))
        );
        let sheet = sheet(vec![view(&[("width", Prop::Expr(0))], &[])]).unwrap();
        assert_eq!(
            sheet.compute(&[V::Num(5.0)], true, CANVAS, &[]).unwrap()[0]
                .layout
                .size
                .width,
            Dimension::length(5.0)
        );
        assert_eq!(
            sheet
                .compute(&[V::Str("abc".into())], true, CANVAS, &[])
                .unwrap_err(),
            "s.tsx:1:1: style width: abc is not a length"
        );
        assert_eq!(
            sheet
                .compute(&[V::Bool(true)], true, CANVAS, &[])
                .unwrap_err(),
            "s.tsx:1:1: style width: true is not a length"
        );
    }

    /// A View of these classes and this style.
    fn classed(classes: &[&str], style: &[(&str, Prop)], children: &[u32]) -> Node {
        let mut node = view(style, children);
        node.classes = classes.iter().map(|&c| c.into()).collect();
        node
    }

    #[test]
    fn classes_cascade_under_the_style_and_important_ones_over_it() {
        let s = styles(
            vec![classed(
                &["p-4", "mt-2!"],
                &[("padding", num(1.0)), ("marginTop", num(1.0))],
                &[],
            )],
            &[],
        )
        .unwrap()
        .remove(0);
        assert_eq!(s.layout.padding.top, px(1.0));
        assert_eq!(s.layout.margin.top, auto_px(8.0));
        // The parent's spacing between its children, the last of them without.
        let tree = vec![
            classed(&["flex", "space-x-[3px]"], &[], &[1, 2]),
            view(&[], &[]),
            view(&[], &[]),
        ];
        let s = styles(tree, &[]).unwrap();
        assert_eq!(s[0].layout.display, Display::Flex);
        assert_eq!(s[1].layout.margin.right, auto_px(3.0));
        assert_eq!(s[2].layout.margin.right, auto_px(0.0));
        let e = styles(vec![classed(&["hover:flex"], &[], &[])], &[]).unwrap_err();
        assert!(
            e.starts_with("s.tsx:1:1: className hover:flex: variants are not supported"),
            "{e}"
        );
        let s = styles(vec![classed(&["-z-[2.5]"], &[], &[])], &[]).unwrap();
        assert_eq!(s[0].z_index, Some(-2));
        let e = styles(vec![classed(&["h-[1lh]"], &[], &[])], &[]).unwrap_err();
        assert!(
            e.starts_with("s.tsx:1:1: className h-[1lh]: height: "),
            "{e}"
        );
    }

    #[test]
    fn nodes_out_of_preorder_are_an_engine_bug() {
        let tree = vec![view(&[], &[]), view(&[], &[0])];
        assert!(sheet(tree).err().unwrap().starts_with("engine bug"));
    }

    /// The writing modes and directions a box's flow-relative properties resolve in: each with
    /// the physical sides, top 0, right 1, bottom 2 and left 3, that its block axis and then its
    /// inline axis run from and to, as CSS Writing Modes 4 defines them, written out here rather
    /// than taken from the engine.
    const FLOWS: [(&str, &str, [usize; 2], [usize; 2]); 6] = [
        ("horizontal-tb", "ltr", [0, 2], [3, 1]),
        ("horizontal-tb", "rtl", [0, 2], [1, 3]),
        ("vertical-rl", "ltr", [1, 3], [0, 2]),
        ("vertical-rl", "rtl", [1, 3], [2, 0]),
        ("vertical-lr", "ltr", [3, 1], [0, 2]),
        ("vertical-lr", "rtl", [3, 1], [2, 0]),
    ];

    /// The style of a box whose parent sets `writing` and `direction`, as a Text in a scene
    /// would, so that the box inherits them and resolves `style` in them.
    fn flowing(writing: &str, direction: &str, style: &[(&str, Prop)]) -> Style {
        let parent = [
            ("writingMode", text(writing)),
            ("direction", text(direction)),
        ];
        styles(vec![view(&parent, &[1]), view(style, &[])], &[])
            .unwrap()
            .remove(1)
    }

    #[test]
    fn flow_relative_sides_are_the_physical_ones_of_every_writing_mode_and_direction() {
        // Four different lengths, so that each side shows which one reached it.
        let [bs, be, is, ie] = [1.0, 2.0, 3.0, 4.0];
        for (writing, direction, [b0, b1], [i0, i1]) in FLOWS {
            let at = |sides: [f32; 4]| {
                let mut want = [0.0; 4];
                for (side, v) in [b0, b1, i0, i1].into_iter().zip(sides) {
                    want[side] = v;
                }
                want
            };
            let want = at([bs, be, is, ie]);
            let flow = format!("{writing} {direction}");
            let longhands = |prefix: &str, suffix: &str| {
                vec![
                    (format!("{prefix}BlockStart{suffix}"), num(bs.into())),
                    (format!("{prefix}BlockEnd{suffix}"), num(be.into())),
                    (format!("{prefix}InlineStart{suffix}"), num(is.into())),
                    (format!("{prefix}InlineEnd{suffix}"), num(ie.into())),
                ]
            };
            let shorthands = |prefix: &str, suffix: &str| {
                let pair = |a: f32, b: f32| text(&format!("{a}px {b}px"));
                vec![
                    (format!("{prefix}Block{suffix}"), pair(bs, be)),
                    (format!("{prefix}Inline{suffix}"), pair(is, ie)),
                ]
            };
            let style = |declared: &[(String, Prop)]| {
                let declared: Vec<(&str, Prop)> = declared
                    .iter()
                    .map(|(k, p)| (k.as_str(), p.clone()))
                    .collect();
                flowing(writing, direction, &declared)
            };
            let sides = |r: taffy::Rect<taffy::LengthPercentage>| {
                [r.top, r.right, r.bottom, r.left].map(|v| v.into_raw().value())
            };
            let insets = |r: taffy::Rect<taffy::LengthPercentageAuto>| {
                [r.top, r.right, r.bottom, r.left].map(|v| v.into_raw().value())
            };
            for declared in [longhands("padding", ""), shorthands("padding", "")] {
                let s = style(&declared);
                assert_eq!(
                    sides(s.layout.padding),
                    want,
                    "padding, {flow}: {declared:?}"
                );
            }
            for declared in [longhands("margin", ""), shorthands("margin", "")] {
                let s = style(&declared);
                assert_eq!(
                    insets(s.layout.margin),
                    want,
                    "margin, {flow}: {declared:?}"
                );
            }
            for declared in [longhands("inset", ""), shorthands("inset", "")] {
                let s = style(&declared);
                assert_eq!(insets(s.layout.inset), want, "inset, {flow}: {declared:?}");
            }
            // Borders: each side's width, in both the longhand of a side and the shorthand of
            // an axis, and in the `border-*` shorthand of a side that sets its width as well.
            let solid = |w: f32| text(&format!("{w}px solid"));
            let widths = [
                longhands("border", "Width"),
                shorthands("border", "Width"),
                [
                    ("borderBlockStart", solid(bs)),
                    ("borderBlockEnd", solid(be)),
                    ("borderInlineStart", solid(is)),
                    ("borderInlineEnd", solid(ie)),
                ]
                .map(|(k, p)| (k.to_string(), p))
                .to_vec(),
            ];
            for declared in &widths {
                let mut declared = declared.clone();
                declared.insert(0, ("borderStyle".into(), text("solid")));
                let s = style(&declared);
                let got = s.border.map(|w| w as f32);
                assert_eq!(got, want, "border width, {flow}: {declared:?}");
            }
        }
    }

    #[test]
    fn flow_relative_and_physical_border_declarations_cascade_in_written_order() {
        // vertical-rl rtl: block start is right, inline end is top.
        let style = |declared: &[(&str, Prop)]| flowing("vertical-rl", "rtl", declared);
        let red = color::parse("red").unwrap();
        let blue = color::parse("blue").unwrap();
        // A side's longhand after its axis's shorthand overrides only its own part of it.
        let s = style(&[
            ("borderBlock", text("3px solid red")),
            ("borderBlockStartColor", text("blue")),
            ("borderInline", text("5px dashed red")),
            ("borderInlineEndWidth", num(7.0)),
        ]);
        assert_eq!(s.border, [7.0, 3.0, 5.0, 3.0]);
        assert_eq!(s.border_colors, [red, blue, red, red]);
        use BorderStyle::{Dashed, Solid};
        assert_eq!(s.border_styles, [Dashed, Solid, Dashed, Solid]);
        // A physical side and a logical one naming it: whichever is written last.
        let right = |declared: &[(&str, Prop)]| style(declared).border_colors[1];
        let physical_last = [
            ("borderStyle", text("solid")),
            ("borderBlockStartColor", text("red")),
            ("borderRightColor", text("blue")),
        ];
        assert_eq!(right(&physical_last), blue);
        let logical_last = [
            ("borderStyle", text("solid")),
            ("borderRightColor", text("blue")),
            ("borderBlockStartColor", text("red")),
        ];
        assert_eq!(right(&logical_last), red);
        // The same holds for widths reached through the whole-side shorthands.
        let s = style(&[
            ("borderRight", text("2px solid")),
            ("borderBlockStartWidth", num(9.0)),
            ("borderTopWidth", num(4.0)),
            ("borderInlineEnd", text("6px solid")),
        ]);
        assert_eq!([s.border[0], s.border[1]], [6.0, 9.0]);
    }

    #[test]
    fn physical_border_longhands_set_their_own_side() {
        let sides = ["Top", "Right", "Bottom", "Left"];
        let names = ["red", "green", "blue", "black"];
        let lines = ["dotted", "dashed", "double", "groove"];
        let mut declared: Vec<(String, Prop)> = Vec::new();
        for (k, side) in sides.iter().enumerate() {
            declared.push((format!("border{side}Width"), num((k + 1) as f64)));
            declared.push((format!("border{side}Style"), text(lines[k])));
            declared.push((format!("border{side}Color"), text(names[k])));
        }
        let declared: Vec<(&str, Prop)> = declared
            .iter()
            .map(|(k, p)| (k.as_str(), p.clone()))
            .collect();
        let s = one(&declared);
        assert_eq!(s.border, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(s.border_colors, names.map(|n| color::parse(n).unwrap()));
        use BorderStyle::{Dashed, Dotted, Double, Groove};
        assert_eq!(s.border_styles, [Dotted, Dashed, Double, Groove]);
        // Each whole side, then its corner radii, top left clockwise.
        for (k, side) in sides.iter().enumerate() {
            let key = format!("border{side}");
            let s = one(&[(key.as_str(), text("3px solid red"))]);
            let mut widths = [0.0; 4];
            widths[k] = 3.0;
            assert_eq!(s.border, widths, "{key}");
        }
        let corners = [
            "borderTopLeftRadius",
            "borderTopRightRadius",
            "borderBottomRightRadius",
            "borderBottomLeftRadius",
        ];
        let declared: Vec<(&str, Prop)> = (corners.iter().enumerate())
            .map(|(k, &c)| (c, num((k + 1) as f64)))
            .collect();
        let s = one(&declared);
        for (k, corner) in corners.iter().enumerate() {
            assert_eq!(s.radii[k][0].resolve(0.0), (k + 1) as f32, "{corner}");
        }
    }

    #[test]
    fn flow_relative_border_styles_and_colors_reach_their_physical_sides() {
        let [bs, be, is, ie] = ["red", "green", "blue", "black"];
        let lines = [
            BorderStyle::Dotted,
            BorderStyle::Dashed,
            BorderStyle::Double,
            BorderStyle::Groove,
        ];
        let names = ["dotted", "dashed", "double", "groove"];
        for (writing, direction, [b0, b1], [i0, i1]) in FLOWS {
            let flow = format!("{writing} {direction}");
            let order = [b0, b1, i0, i1];
            let mut colors = [[0.0; 4]; 4];
            let mut styles_at = [BorderStyle::None; 4];
            for (k, side) in order.into_iter().enumerate() {
                colors[side] = color::parse([bs, be, is, ie][k]).unwrap();
                styles_at[side] = lines[k];
            }
            let color = |k: &str, c: &str| (k.to_string(), text(c));
            let longhands = vec![
                color("borderBlockStartColor", bs),
                color("borderBlockEndColor", be),
                color("borderInlineStartColor", is),
                color("borderInlineEndColor", ie),
                color("borderBlockStartStyle", names[0]),
                color("borderBlockEndStyle", names[1]),
                color("borderInlineStartStyle", names[2]),
                color("borderInlineEndStyle", names[3]),
            ];
            let shorthands = vec![
                color("borderBlockColor", &format!("{bs} {be}")),
                color("borderInlineColor", &format!("{is} {ie}")),
                color("borderBlockStyle", &format!("{} {}", names[0], names[1])),
                color("borderInlineStyle", &format!("{} {}", names[2], names[3])),
            ];
            for declared in [longhands, shorthands] {
                let mut declared = declared;
                declared.insert(0, ("borderWidth".into(), num(2.0)));
                let declared: Vec<(&str, Prop)> = declared
                    .iter()
                    .map(|(k, p)| (k.as_str(), p.clone()))
                    .collect();
                let s = flowing(writing, direction, &declared);
                assert_eq!(s.border_colors, colors, "colors, {flow}: {declared:?}");
                assert_eq!(s.border_styles, styles_at, "styles, {flow}: {declared:?}");
            }
        }
    }

    #[test]
    fn flow_relative_sizes_overflow_and_corners_follow_the_writing_mode() {
        for (writing, direction, [b0, b1], [i0, i1]) in FLOWS {
            let flow = format!("{writing} {direction}");
            let vertical = writing != "horizontal-tb";
            let s = flowing(
                writing,
                direction,
                &[
                    ("inlineSize", num(10.0)),
                    ("blockSize", num(20.0)),
                    ("minInlineSize", num(1.0)),
                    ("minBlockSize", num(2.0)),
                    ("maxInlineSize", num(100.0)),
                    ("maxBlockSize", num(200.0)),
                    // Clip, not hidden: hidden on one axis makes the other hidden as well.
                    ("overflowInline", text("clip")),
                    ("overflowBlock", text("visible")),
                ],
            );
            let l = &s.layout;
            let size = |d: Dimension| d.into_raw().value();
            let limit = |d: taffy::LengthPercentageAuto| d.into_raw().value();
            let [(w, h), (min_w, min_h), (max_w, max_h)] = match vertical {
                false => [(10.0, 20.0), (1.0, 2.0), (100.0, 200.0)],
                true => [(20.0, 10.0), (2.0, 1.0), (200.0, 100.0)],
            };
            assert_eq!(
                [size(l.size.width), size(l.size.height)],
                [w, h],
                "size, {flow}"
            );
            assert_eq!(
                [limit(l.min_size.width), limit(l.min_size.height)],
                [min_w, min_h],
                "min size, {flow}"
            );
            assert_eq!(
                [limit(l.max_size.width), limit(l.max_size.height)],
                [max_w, max_h],
                "max size, {flow}"
            );
            let clipped = match vertical {
                false => [Overflow::Clip, Overflow::Visible],
                true => [Overflow::Visible, Overflow::Clip],
            };
            assert_eq!([l.overflow.x, l.overflow.y], clipped, "overflow, {flow}");

            // A corner is where its block side meets its inline side: top left 0, top right 1,
            // bottom right 2, bottom left 3.
            let corner = |block: usize, inline: usize| match (block.min(inline), block.max(inline))
            {
                (0, 3) => 0,
                (0, 1) => 1,
                (1, 2) => 2,
                (2, 3) => 3,
                other => unreachable!("sides {other:?} do not meet"),
            };
            let radii = [
                ("borderStartStartRadius", corner(b0, i0), 1.0_f32),
                ("borderStartEndRadius", corner(b0, i1), 2.0),
                ("borderEndStartRadius", corner(b1, i0), 3.0),
                ("borderEndEndRadius", corner(b1, i1), 4.0),
            ];
            let declared: Vec<(&str, Prop)> =
                radii.iter().map(|&(k, _, r)| (k, num(r.into()))).collect();
            let s = flowing(writing, direction, &declared);
            for (k, at, r) in radii {
                assert_eq!(s.radii[at][0].resolve(0.0), r, "{k}, {flow}");
            }
        }
    }
}
