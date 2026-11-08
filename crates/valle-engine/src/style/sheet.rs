//! Declarations bound to a variant, inherited and checked per frame.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::{svg, transition};

use super::computed::{Instance, Shaded, Source, Style, compute};
use super::declaration::{Declared, overflow, pair, presented, set};
use super::property::{Field, camel, unsupported};
use super::{cache, tailwind};

use crate::front::Slot;
use crate::shader::{self, Program};
use crate::template::{Error, Kind, Node, Piece, Prop, Tag, Template, V};

/// A `<ShaderLayer>`'s program and its uniforms' values, and its block when none is traced.
pub(super) struct Shading {
    pub(super) program: Rc<Program>,
    /// What errors call it, `<ShaderLayer>` or `<Transition>`.
    pub(super) tag: &'static str,
    /// A built-in transition's parameters' ranges.
    pub(super) limits: &'static [transition::Range],
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
                let values: Vec<(&str, Vec<&V>)> = (self.uniforms.iter())
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
                    let Some(&(_, low, high)) = self.limits.iter().find(|l| l.0 == *name) else {
                        continue;
                    };
                    if let [V::Num(x)] = v[..]
                        && !(low..=high).contains(x)
                    {
                        let x = crate::js::number_to_string(*x);
                        return Err(format!("params {name} is from {low} to {high}, not {x}"));
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
        let c = svg::computed(self.tag, &list).map_err(|e| format!("<{}> {e}", self.tag.name()))?;
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
        let nodes = (template.nodes.iter().zip(&classes))
            .map(|(node, classes)| bind(node, classes, &late))
            .collect::<Result<_, _>>()?;
        Ok(Sheet {
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
        let frame = Frame {
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
    ) -> Result<Vec<Style>, Error> {
        let frame = Frame {
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
                set(&mut d, *field, &presented(v, *attribute))
                    .map_err(|e| format!("{}: {what}: {e}", b.place))?;
            }
            Cow::Owned(d)
        };
        let (canvas, resources) = (frame.canvas, frame.resources);
        let mut base = b.base.borrow_mut();
        let kept = base.as_ref().and_then(|s| {
            if !s.matches(parent, canvas, calcs, resources) {
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
                let style = compute(&declared, parent, canvas, calcs, i, b.text, resources)
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
        let boxed = (b.svg.as_ref()).is_none_or(|g| g.tag == Tag::Svg);
        if boxed && (style.clip_ref.is_some() || style.mask_ref.is_some()) {
            return Err(format!(
                "{}: clip-path: url() and mask: url() are supported in an <svg> only, not on a \
                 box yet",
                b.place
            ));
        }
        if let Some(g) = &b.svg {
            style.svg = Some(g.computed(frame).map_err(|e| format!("{}: {e}", b.place))?);
        }
        if let Some(s) = &b.shader {
            let shaded = s.computed(frame);
            style.shader = Some(shaded.map_err(|e| format!("{}: {} {e}", b.place, s.tag))?);
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
        let refers = |s: &&Style| s.clip_ref.is_some() || s.mask_ref.is_some();
        for (i, s) in styles.iter().enumerate().filter(|(_, s)| refers(s)) {
            let place = &self.nodes[i].place;
            let parent = self.parents[i].and_then(|p| tag(p as usize));
            for (id, want, name) in [
                (&s.clip_ref, Tag::ClipPath, "clip-path"),
                (&s.mask_ref, Tag::Mask, "mask"),
            ] {
                let Some(id) = id.as_deref() else {
                    continue;
                };
                if matches!(tag(i), Some(Tag::ClipPath | Tag::Mask))
                    || parent == Some(Tag::ClipPath)
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
        let Some(svg::Element::Use { href, .. }) = style.svg.as_deref().map(|c| &c.element) else {
            return Ok(());
        };
        let place = &self.nodes[u].place;
        let tag = |i: usize| self.nodes[i].svg.as_ref().map(|g| g.tag);
        let Some(&r) = ids.get(href) else {
            return Err(format!("{place}: <use> href: #{href} names no element"));
        };
        let copies = matches!(
            tag(r),
            Some(
                Tag::G
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
            let name = tag(r).map_or("", Tag::name);
            return Err(format!(
                "{place}: <use> href: #{href} names a <{name}>, which a <use> does not copy"
            ));
        }
        let end = self.ends[r] as usize;
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
            let parent = match self.parents[n] {
                Some(p) if n > r => &styles[p as usize - r],
                _ => &*style,
            };
            let s = self.style(n, Some(parent), frame, 0)?;
            styles.push(s);
        }
        for (k, s) in styles.iter_mut().enumerate() {
            self.instantiate(r + k, s, ids, frame, depth + 1, copied)?;
        }
        style.instance = Some(Rc::new(Instance { root: r, styles }));
        Ok(())
    }
}

/// What a frame's styles are computed for: the program's values, the canvas's size in px, and
/// the scene's resources.
pub(super) struct Frame<'a> {
    pub(super) values: &'a [V],
    /// The late expressions, to leave out, while the values are not settled.
    pub(super) late: Option<&'a [bool]>,
    pub(super) canvas: [f64; 2],
    pub(super) resources: &'a [Slot],
}

impl Frame<'_> {
    /// Whether a declaration of `prop` waits for the values to settle.
    pub(super) fn unsettled(&self, prop: &Prop) -> bool {
        matches!((prop, self.late), (Prop::Expr(e), Some(late)) if late.get(*e as usize) == Some(&true))
    }
}

/// A declaration as written: its key, its value, what errors call it, and whether it is a
/// presentation attribute.
pub(super) type Written<'a> = (Rc<str>, Cow<'a, Prop>, Rc<str>, bool);

/// A node's declarations, with those its classes make.
fn bind(node: &Node, classes: &[tailwind::Declaration], late: &[bool]) -> Result<Bound, Error> {
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
    let (tag, attributes) = match &node.kind {
        Kind::Svg { tag, attributes } => (Some(*tag), &attributes[..]),
        _ => (None, &[][..]),
    };
    // An <svg> clips what it draws to its box, as Chrome's style for it has it.
    if tag == Some(Tag::Svg) {
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
    for (key, prop, what, attribute) in written {
        let field = (Field::by_name(&key))
            .ok_or_else(|| format!("{place}: {}", unsupported(&what, &key)))?;
        let unit = matches!(node.kind, Kind::Span { unit: true, .. });
        if unit && !field.textual() && !field.on_units() {
            return Err(format!(
                "{place}: {what}: a unit of a split text takes only text properties, opacity, \
                 transforms and filter"
            ));
        }
        if matches!(node.kind, Kind::Span { unit: false, .. }) && !field.textual() {
            return Err(format!(
                "{place}: {what}: a <Span> is part of a text, not a box, so it takes only text \
                 properties"
            ));
        }
        if tag.is_some_and(|t| t != Tag::Svg) && !field.drawn_in_svg() {
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
            Prop::Const(v) if rest.is_empty() => {
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
            let geometry = attributes
                .iter()
                .filter(|(k, _)| !svg::presentational(tag, k));
            if let Some((k, _)) = geometry
                .clone()
                .find(|(_, p)| tag == Tag::Svg && late_prop(p))
            {
                return Err(format!(
                    "{place}: attribute {k}: {LATE}, and an <svg>'s {k} changes its layout"
                ));
            }
            let mut g = Geometry {
                tag,
                attributes: geometry.cloned().collect(),
                fixed: None,
            };
            if g.attributes
                .iter()
                .all(|(_, p)| matches!(p, Prop::Const(_)))
            {
                let none = Frame {
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
                .map_or(&[][..], |k| k.1);
            if let Some(kind) = kind {
                for (name, _) in uniforms.iter().filter(|(k, _)| &**k != "progress") {
                    if !limits.iter().any(|l| l.0 == &**name) {
                        let names: Vec<&str> = limits.iter().map(|l| l.0).collect();
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
        updates: cache::Updates::new(&rest),
        base: RefCell::default(),
        declared,
        text: matches!(node.kind, Kind::Text { .. }),
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
        let alone = (sheet.compute_only(&only, &values, CANVAS, &resources(), &styles)).unwrap();
        assert!(only.iter().zip(&alone).all(|(&i, s)| *s == styles[i]));
    }

    #[test]
    fn a_span_takes_only_text_properties() {
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
        let e = styles(tree, &[]).unwrap_err();
        assert!(
            e.ends_with(
                "style padding: a <Span> is part of a text, not a box, so it takes only text properties"
            ),
            "{e}"
        );
    }

    #[test]
    fn svg_elements_take_presentation_attributes_under_their_style() {
        let svg =
            |tag, attributes: &[(&str, Prop)], style: &[(&str, Prop)], children: &[u32]| Node {
                kind: Kind::Svg {
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
            (
                4,
                ("clipPath", "url(#c)"),
                "in a <clipPath>, is not supported yet",
            ),
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
}
