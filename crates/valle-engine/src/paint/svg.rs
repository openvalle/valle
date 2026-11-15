//! What the elements of an `<svg>` draw, or of an SVG file: each element and its children in
//! its user units, with the paint servers, clip paths, masks, markers and copies they name by id.

use std::cell::RefCell;
use std::collections::HashMap;

use taffy::Display;

use super::{Blend, Cache, Filter, Item, Matrix, Ramp, SQUARE, Tiles, work};
use crate::image::Picture;
use crate::layout::{Natural, Rect};
use crate::path::Path;
use crate::style::color;
use crate::style::math::Calc;
use crate::style::{Instance, Paint, PaintPart, Style};
use crate::svg::file::{Document, Embedded};
use crate::svg::{self, Element};
use crate::template::Template;

/// What SVG file `document` draws in `first`, the rect its Image shows it in, in px, as Chrome
/// draws an SVG image: its root's elements through its viewBox into the rect, as its fit has it;
/// without one, unscaled in a viewport its natural size if it has one, which then stretches to
/// the rect as an image does, else the rect's. The rect clips them. `None` if it draws nothing,
/// as with a viewBox without area.
pub fn document(document: &Document, first: Rect, cache: &Cache) -> Option<Item> {
    let root = document.styles.first()?.svg.as_deref()?;
    let Element::Svg { view_box, fit, .. } = root.element else {
        return None;
    };
    let size = [first.width, first.height];
    let viewport = match (view_box, document.natural) {
        (None, Natural::Size(natural)) => natural,
        _ => size,
    };
    if viewport.iter().any(|&v| v <= 0.0) {
        return None;
    }
    let viewport = viewport.map(f64::from);
    let (matrix, units) = match view_box {
        Some([.., w, h]) if w <= 0.0 || h <= 0.0 => return None,
        Some(b) => (svg::viewport(b, fit, viewport), [b[2], b[3]]),
        None => (Matrix::IDENTITY, viewport),
    };
    let [kx, ky] = [0, 1].map(|k| f64::from(size[k]) / viewport[k]);
    let scaled = Matrix::affine([kx, 0.0, 0.0, ky, first.x.into(), first.y.into()]);
    let matrix = scaled.times(matrix);
    let mut items = Vec::new();
    let drawing = Drawing::new(
        &document.template,
        &document.styles,
        &document.embedded,
        &[],
        cache,
    );
    drawing.element(0, units, matrix, &mut items, None);
    (!items.is_empty()).then(|| Item::Clip {
        rect: first,
        radii: SQUARE,
        items: vec![Item::Transform { matrix, items }],
    })
}

/// The elements of a document of SVG, a frame's or a file's, by their styles at a frame.
pub struct Drawing<'a> {
    cache: &'a Cache,
    template: &'a Template,
    styles: &'a [Style],
    /// Its elements by id: as Chrome finds them in its document, the first of each.
    ids: HashMap<&'a str, usize>,
    /// External images loaded by an SVG file's image elements and filter primitives.
    embedded: &'a [Embedded],
    pictures: &'a [Picture],
    /// The masks and markers what is drawing is in, innermost last.
    resources: RefCell<Vec<usize>>,
}

impl<'a> Drawing<'a> {
    pub fn new(
        template: &'a Template,
        styles: &'a [Style],
        embedded: &'a [Embedded],
        pictures: &'a [Picture],
        cache: &'a Cache,
    ) -> Drawing<'a> {
        let mut ids = HashMap::new();
        for (i, s) in styles.iter().enumerate() {
            if let Some(id) = s.svg.as_deref().and_then(|c| c.id.as_deref()) {
                ids.entry(id).or_insert(i);
            }
        }
        Drawing {
            cache,
            template,
            styles,
            ids,
            embedded,
            pictures,
            resources: RefCell::new(Vec::new()),
        }
    }

    /// An element in an `<svg>`, or an SVG file's root, and its children, in user units of a
    /// viewport `size` units big, which `matrix` takes to px, with its style in `copy` if that
    /// holds it: a shape's fill, stroke and markers in paint order, unless hidden, each opacity
    /// on its paint's alpha; a `<use>`'s copy; then its mask, its clip path, and its opacity on all
    /// it draws as one layer, and its transform.
    pub fn element<'s>(
        &'s self,
        i: usize,
        size: [f64; 2],
        matrix: Matrix,
        out: &mut Vec<Item>,
        copy: Option<&'s Instance>,
    ) {
        let s = self.style(i, copy);
        let Some(c) = s.svg.as_deref() else {
            return;
        };
        if let Some(rows) = &s.svg_instances {
            if !s.visible || s.layout.display == Display::None {
                return;
            }
            if matches!(
                c.element,
                Element::Shape(svg::Shape::Circle { .. } | svg::Shape::Ellipse { .. })
            ) && c.transform.is_none()
                && !s.transformed()
                && rows.iter().all(|r| {
                    matches!(r.stroke.as_ref().unwrap_or(&s.stroke.paint), Paint::None)
                        && r.opacity == 1.0
                        && !matches!(r.fill.as_ref().unwrap_or(&s.fill), Paint::Url(_))
                })
            {
                let rows: Vec<_> = rows
                    .iter()
                    .filter_map(|r| {
                        let ([x, y], [rx, ry]) = svg::ellipse_geometry(&r.shape, size)?;
                        let mut color = match r.fill.as_ref().unwrap_or(&s.fill) {
                            Paint::None => return None,
                            Paint::Color(c) => *c,
                            Paint::Mixed(c) => c.resolve(color::resolve(&r.color, s.color)),
                            Paint::CurrentColor => color::resolve(&r.color, s.color),
                            Paint::Url(_) => unreachable!(),
                        };
                        color[3] *= r.fill_opacity.unwrap_or(s.fill_opacity);
                        (color[3] > 0.0).then(|| super::PathInstance {
                            matrix: Matrix::affine([
                                rx.into(),
                                0.0,
                                0.0,
                                ry.into(),
                                x.into(),
                                y.into(),
                            ]),
                            color: work(color),
                            even_odd: r.even_odd.unwrap_or(s.even_odd),
                        })
                    })
                    .collect();
                if !rows.is_empty() {
                    out.push(Item::Paths {
                        path: svg::ellipse([0.0; 2], [1.0; 2]).expect("unit ellipse"),
                        rows,
                    });
                }
                return;
            }
            let mut style = s.clone();
            style.svg_instances = None;
            for row in rows.iter() {
                style.fill.clone_from(row.fill.as_ref().unwrap_or(&s.fill));
                style.fill_opacity = row.fill_opacity.unwrap_or(s.fill_opacity);
                style.even_odd = row.even_odd.unwrap_or(s.even_odd);
                style.color = color::resolve(&row.color, s.color);
                style.opacity = row.opacity;
                style
                    .stroke
                    .paint
                    .clone_from(row.stroke.as_ref().unwrap_or(&s.stroke.paint));
                style.stroke.opacity = row.stroke_opacity.unwrap_or(s.stroke.opacity);
                let shape = svg::Computed {
                    font_relative: false,
                    element: Element::Shape(row.shape.clone()),
                    transform: c.transform,
                    path_length: c.path_length,
                    id: None,
                };
                self.draw_element(i, size, matrix, out, copy, (&style, &shape));
            }
            return;
        }
        self.draw_element(i, size, matrix, out, copy, (s, c));
    }

    /// Construct the stroke in viewport units, then return it to the path's user space so
    /// its paint servers, clips and filters continue to use the same coordinate system.
    fn stroke(&self, path: &Path, pen: &svg::Pen, matrix: Matrix, fixed: bool) -> Option<Path> {
        if !fixed {
            return self.cache.stroke(path, pen, matrix.radius_scale());
        }
        let inverse = matrix.inverse()?;
        let mut pen = pen.clone();
        if let Some(length) = pen.path_length.take() {
            let k = svg::arclen(&svg::bez(path, 1.0)) / length;
            pen.dashes.iter_mut().for_each(|dash| *dash *= k);
            pen.dash_offset *= k;
        }
        let path = path.mapped(|p| matrix.apply(p));
        self.cache
            .stroke(&path, &pen, 1.0)
            .map(|p| p.mapped(|q| inverse.apply(q)))
    }

    fn shape(
        &self,
        i: usize,
        size: [f64; 2],
        matrix: Matrix,
        copy: Option<&Instance>,
        (s, shape, path_length): (&Style, &svg::Shape, Option<f64>),
    ) -> Vec<Item> {
        let scale = matrix.radius_scale();
        let mut items = Vec::new();

        let Some(path) = svg::path(shape, size) else {
            return Vec::new();
        };
        let bbox = if matches!(shape, svg::Shape::Text { .. }) {
            self.text_bbox(i, size, copy)
                .unwrap_or_else(|| svg::bounds(&path))
        } else {
            svg::bounds(&path)
        };
        let k = &s.stroke;
        let [w, h] = size;
        let diagonal = ((w * w + h * h) / 2.0).sqrt() as f32;
        let length = |l: &Calc| f64::from(l.resolve(diagonal));
        let width = length(&k.width);
        let mut stroke = self
            .paint(&k.paint, k.opacity, s, size, bbox, matrix)
            .and_then(|paint| {
                let pen = svg::Pen {
                    width,
                    cap: k.cap,
                    join: k.join,
                    miter_limit: k.miter_limit,
                    dashes: k.dashes.iter().map(length).collect(),
                    dash_offset: length(&k.dash_offset),
                    path_length,
                };
                let outline = (pen.width > 0.0)
                    .then(|| self.stroke(&path, &pen, matrix, s.non_scaling_stroke))
                    .flatten()?;
                let cover = svg::bounds(&outline);
                painted(outline, false, paint, cover, scale)
            });
        let mut markers = self.markers(s, shape, &path, width, size, matrix);
        let fill = self.paint(&s.fill, s.fill_opacity, s, size, bbox, matrix);
        let mut fill = fill.and_then(|f| {
            // Solid ellipses share one outline. Their center and radii belong to their
            // transform, so a dot field does not encode a new path at every position.
            if let Fill::Color(color) = &f
                && let Some(([x, y], [rx, ry])) = svg::ellipse_geometry(shape, size)
            {
                return Some(Item::Transform {
                    matrix: Matrix::affine([rx.into(), 0.0, 0.0, ry.into(), x.into(), y.into()]),
                    items: vec![Item::Path {
                        path: svg::ellipse([0.0; 2], [1.0; 2])?,
                        even_odd: s.even_odd,
                        color: *color,
                    }],
                });
            }
            painted(path, s.even_odd, f, bbox, scale)
        });
        for part in s.paint_order {
            match part {
                PaintPart::Fill => items.extend(fill.take()),
                PaintPart::Stroke => items.extend(stroke.take()),
                PaintPart::Markers => items.append(&mut markers),
            }
        }
        items
    }

    fn draw_element(
        &self,
        i: usize,
        size: [f64; 2],
        matrix: Matrix,
        out: &mut Vec<Item>,
        copy: Option<&Instance>,
        (s, c): (&Style, &svg::Computed),
    ) {
        if s.layout.display == Display::None || s.opacity <= 0.0 {
            return;
        }
        let local = self.local(i, s, c, size, copy);
        let matrix = local.map_or(matrix, |t| matrix.times(t));
        let mut items = Vec::new();
        match &c.element {
            Element::Svg { .. } if !self.template.roots.contains(&(i as u32)) => {
                let Some(view) = c.element.viewport(size) else {
                    return;
                };
                for &k in &self.template.nodes[i].children {
                    self.element(
                        k as usize,
                        view.units,
                        matrix.times(view.matrix),
                        &mut items,
                        copy,
                    );
                }
                if !items.is_empty() {
                    items = vec![Item::Transform {
                        matrix: view.matrix,
                        items,
                    }];
                    if s.layout.overflow.x != taffy::Overflow::Visible
                        || s.layout.overflow.y != taffy::Overflow::Visible
                    {
                        items = vec![Item::Clip {
                            rect: rect(view.rect),
                            radii: SQUARE,
                            items,
                        }];
                    }
                }
            }
            Element::G | Element::Text(_) | Element::Svg { .. } => {
                for &k in &self.template.nodes[i].children {
                    self.element(k as usize, size, matrix, &mut items, copy);
                }
            }
            Element::Use { .. } => {
                let Some(instance) = s.instance.as_deref() else {
                    return;
                };
                let root = instance.root;
                match self.symbol(i, size, copy) {
                    Some((fitted, [w, h], units)) => {
                        let mut drawn = Vec::new();
                        for &k in &self.template.nodes[root].children {
                            let m = matrix.times(fitted);
                            self.element(k as usize, units, m, &mut drawn, Some(instance));
                        }
                        if !drawn.is_empty() {
                            items.push(Item::Clip {
                                rect: rect([0.0, 0.0, w, h]),
                                radii: SQUARE,
                                items: vec![Item::Transform {
                                    matrix: fitted,
                                    items: drawn,
                                }],
                            });
                        }
                    }
                    None => self.element(root, size, matrix, &mut items, Some(instance)),
                }
            }
            Element::Shape(shape) if s.visible => {
                let decorations = match shape {
                    svg::Shape::Text { decorations, .. } => decorations.as_slice(),
                    _ => &[],
                };
                for through in [false, true] {
                    if through {
                        items.extend(self.shape(i, size, matrix, copy, (s, shape, c.path_length)));
                    }
                    for d in decorations.iter().filter(|d| d.through == through) {
                        let owner = self.style(d.by, copy);
                        let shape = svg::Shape::Text {
                            path: d.path.clone(),
                            bounds: self.bbox(d.by, size, copy).unwrap_or([0.0; 4]),
                            decorations: Vec::new(),
                        };
                        items.extend(self.shape(d.by, size, matrix, copy, (owner, &shape, None)));
                    }
                }
            }
            Element::Image { .. } if s.visible => {
                let Some((viewport, first, image)) = self.image(i, size, copy) else {
                    return;
                };
                match image {
                    Picture::Image { image, .. } => items.push(Item::Image {
                        rect: rect(viewport),
                        radii: SQUARE,
                        tiles: Tiles {
                            first: rect(first),
                            step: [None; 2],
                        },
                        image: *image,
                        pixelated: s.pixelated,
                    }),
                    Picture::Svg(doc) => {
                        if let Some(item) = document(doc, rect(first), self.cache) {
                            items.push(Item::Clip {
                                rect: rect(viewport),
                                radii: SQUARE,
                                items: vec![item],
                            });
                        }
                    }
                    Picture::None => {}
                }
            }
            Element::TextRun(_)
            | Element::Shape(_)
            | Element::Image { .. }
            | Element::Filter(_)
            | Element::Primitive(_)
            | Element::Defs
            | Element::Pattern(_)
            | Element::Gradient(_)
            | Element::Stop(_)
            | Element::ClipPath { .. }
            | Element::Mask(_)
            | Element::Symbol { .. }
            | Element::Marker(_) => {}
        }
        if items.is_empty() && s.filter_ref.is_none() {
            return;
        }
        if let Some(id) = &s.filter_ref {
            let Some(graph) = self
                .ids
                .get(&**id)
                .and_then(|n| self.styles[*n].filter_graph.clone())
            else {
                return;
            };
            let bbox = self.bbox(i, size, copy).unwrap_or([0.; 4]);
            let images = self.filter_images(&graph, bbox, size, matrix);
            items = vec![Item::FilterGraph {
                graph,
                images,
                bbox,
                viewport: size,
                items,
            }];
        }
        if !s.filter.is_empty() {
            let frame = rect(self.bbox(i, size, copy).unwrap_or([0.0; 4]));
            items = vec![Item::Filter {
                filters: super::filters(&s.filter, frame),
                items,
            }];
        }
        for (id, clips) in [(&s.mask_ref, false), (&s.clip_ref, true)] {
            let Some(id) = id.as_deref() else {
                continue;
            };
            let shown = match clips {
                true => self.clip(id, i, size, copy),
                false => self.mask(id, i, size, matrix, copy),
            };
            let Some(mask) = shown else {
                return;
            };
            items = vec![Item::Mask {
                mask: vec![mask],
                items,
            }];
        }
        if s.opacity < 1.0
            || s.blend != Blend::Normal
            || s.isolate
            || matches!(c.element, Element::Svg { .. }) && needs_isolation(&items)
        {
            items = vec![Item::Group {
                opacity: s.opacity as f32,
                blend: s.blend,
                items,
            }];
        }
        match local {
            Some(matrix) => out.push(Item::Transform { matrix, items }),
            None => out.extend(items),
        }
    }

    /// CSS transforms use the selected SVG reference box without pixel snapping. The SVG
    /// attribute supplies the transform only when CSS has not overridden it (even with none).
    fn local(
        &self,
        i: usize,
        s: &Style,
        c: &svg::Computed,
        size: [f64; 2],
        copy: Option<&Instance>,
    ) -> Option<Matrix> {
        use crate::style::transform::ReferenceBox as B;
        let mut result = None;
        if c.transform.is_some() || !s.transform.is_empty() {
            let mut reference = match s.transform_box {
                B::Fill | B::Content => self.bbox(i, size, copy),
                B::Stroke | B::Border => self.stroke_bbox(i, size, copy),
                // viewBox establishes a new user coordinate system: its origin is (0, 0),
                // not its min-x/min-y, which belong to the outer viewport transform.
                B::View => Some([0.0, 0.0, size[0], size[1]]),
            }
            .unwrap_or([0.0; 4]);
            // x/y place the use's copied graphics inside its own user coordinate system.
            if s.transform_box != B::View
                && let Element::Use { at, .. } = c.element
            {
                reference[0] += at[0].resolve(size[0]);
                reference[1] += at[1].resolve(size[1]);
            }
            let [x, y, w, h] = reference;
            let size = [w as f32, h as f32];
            let origin =
                [0, 1].map(|k| [x, y][k] + f64::from(s.transform_origin[k].resolve(size[k])));
            let mut matrix = s.transform.iter().fold(super::Matrix4::IDENTITY, |m, f| {
                super::transform_function(m, f, size)
            });
            if let Some(attribute) = c.transform {
                let [a, b, t] = attribute.0;
                matrix = matrix.times(super::Matrix4::affine([a[0], a[1], b[0], b[1], t[0], t[1]]));
            }
            let [x, y] = origin;
            let z = s.transform_origin_z;
            result = Some(
                super::Matrix4::translate(x, y, z)
                    .times(matrix)
                    .times(super::Matrix4::translate(-x, -y, -z))
                    .flatten(),
            );
        }
        if let Element::Use { at, .. } = c.element {
            let [x, y] = [at[0].resolve(size[0]), at[1].resolve(size[1])];
            if x != 0.0 || y != 0.0 {
                result = Some(
                    result
                        .unwrap_or(Matrix::IDENTITY)
                        .times(Matrix::translate(x, y)),
                );
            }
        }
        result
    }

    /// Stroke bounds include caps and joins, but not markers or dashing gaps.
    fn stroke_bbox(&self, i: usize, size: [f64; 2], copy: Option<&Instance>) -> Option<[f64; 4]> {
        let s = self.style(i, copy);
        let c = s.svg.as_deref()?;
        if let Element::Shape(shape) = &c.element {
            let path = svg::path(shape, size)?;
            if !matches!(s.stroke.paint, Paint::None) {
                let diagonal = ((size[0] * size[0] + size[1] * size[1]) / 2.0).sqrt() as f32;
                let pen = svg::Pen {
                    width: s.stroke.width.resolve(diagonal).into(),
                    cap: s.stroke.cap,
                    join: s.stroke.join,
                    miter_limit: s.stroke.miter_limit,
                    dashes: Vec::new(),
                    dash_offset: 0.0,
                    path_length: None,
                };
                if let Some(outline) = self.cache.stroke(&path, &pen, 1.0) {
                    return Some(svg::bounds(&outline));
                }
            }
        }
        match &c.element {
            Element::G | Element::Text(_) => {
                self.around_bounds(&self.template.nodes[i].children, size, copy, true)
            }
            Element::Use { .. } => {
                let instance = s.instance.as_deref()?;
                match self.symbol(i, size, copy) {
                    Some((fitted, _, units)) => self
                        .around_bounds(
                            &self.template.nodes[instance.root].children,
                            units,
                            Some(instance),
                            true,
                        )
                        .map(|b| mapped(fitted, b)),
                    None => self.around_bounds(&[instance.root as u32], size, Some(instance), true),
                }
            }
            _ => self.bbox(i, size, copy),
        }
    }

    /// SVG element `i`'s style: its copy's in `copy` if that holds it, else the frame's.
    fn style<'s>(&'s self, i: usize, copy: Option<&'s Instance>) -> &'s Style {
        copy.and_then(|c| c.style(i)).unwrap_or(&self.styles[i])
    }

    /// If `<use>` `i`, with its style in `copy` if that holds it, in a viewport `size` user units
    /// big, copies a `<symbol>`: the transform its viewBox and fit make from the symbol's user
    /// units to the viewport the `<use>` gives it, that viewport's size, and the size its
    /// percentages measure. `None` too where a viewport or viewBox has no area.
    fn symbol(
        &self,
        i: usize,
        size: [f64; 2],
        copy: Option<&Instance>,
    ) -> Option<(Matrix, [f64; 2], [f64; 2])> {
        let s = self.style(i, copy);
        let Element::Use { size: viewport, .. } = s.svg.as_deref()?.element else {
            return None;
        };
        let root = s.instance.as_deref()?.styles.first()?;
        let Element::Symbol { view_box, fit } = root.svg.as_deref()?.element else {
            return None;
        };
        let [w, h] = [0, 1].map(|k| viewport[k].map_or(size[k], |l| l.resolve(size[k])));
        match view_box {
            _ if w <= 0.0 || h <= 0.0 => None,
            Some([.., vw, vh]) if vw <= 0.0 || vh <= 0.0 => None,
            Some(b) => Some((svg::viewport(b, fit, [w, h]), [w, h], [b[2], b[3]])),
            None => Some((Matrix::IDENTITY, [w, h], [w, h])),
        }
    }

    /// Text runs share the owning text element's typographic reference box.
    fn text_bbox(&self, mut i: usize, size: [f64; 2], copy: Option<&Instance>) -> Option<[f64; 4]> {
        if matches!(
            self.template.nodes[i].kind,
            crate::template::Kind::Svg {
                tag: crate::template::Tag::Text,
                ..
            }
        ) {
            return self.bbox(i, size, copy);
        }
        while let Some((parent, _)) = self
            .template
            .nodes
            .iter()
            .enumerate()
            .find(|(_, n)| n.children.contains(&(i as u32)))
        {
            i = parent;
            if matches!(
                self.template.nodes[i].kind,
                crate::template::Kind::Svg {
                    tag: crate::template::Tag::Text,
                    ..
                }
            ) {
                return self.bbox(i, size, copy);
            }
        }
        None
    }

    /// `i`'s bounding box in its user units, in a viewport `size` user units big, with its style
    /// in `copy` if that holds it: a shape's path's, a group's around its children's, a
    /// `<use>`'s around its copy's; `None` where it has none.
    fn bbox(&self, i: usize, size: [f64; 2], copy: Option<&Instance>) -> Option<[f64; 4]> {
        let s = self.style(i, copy);
        let c = s.svg.as_deref()?;
        if s.layout.display == Display::None {
            return None;
        }
        match &c.element {
            Element::Shape(svg::Shape::Text { bounds, .. }) => Some(*bounds),
            Element::Shape(shape) => svg::path(shape, size).map(|p| svg::bounds(&p)),
            Element::G | Element::Text(_) | Element::ClipPath { .. } => {
                self.around(&self.template.nodes[i].children, size, copy)
            }
            Element::Svg { .. } => {
                let view = c.element.viewport(size)?;
                self.around(&self.template.nodes[i].children, view.units, copy)
                    .map(|b| mapped(view.matrix, b))
            }
            Element::Image { .. } => self.image(i, size, copy).map(|(viewport, ..)| viewport),
            Element::Use { .. } => {
                let instance = s.instance.as_deref()?;
                let root = instance.root;
                match self.symbol(i, size, copy) {
                    Some((fitted, _, units)) => {
                        let children = &self.template.nodes[root].children;
                        let b = self.around(children, units, Some(instance))?;
                        Some(mapped(fitted, b))
                    }
                    None => self.around(&[root as u32], size, Some(instance)),
                }
            }
            _ => None,
        }
    }

    /// Paint only image primitives which contribute to the graph, in its user coordinates.
    fn filter_images(
        &self,
        graph: &svg::filter::Graph,
        bbox: [f64; 4],
        size: [f64; 2],
        matrix: Matrix,
    ) -> Vec<(usize, Vec<Item>)> {
        let regions = graph.regions(bbox, size);
        let needed = graph.needed();
        let mut images = Vec::new();
        for (index, node) in graph.nodes.iter().enumerate() {
            let svg::filter::Operation::Image(image) = &node.op else {
                continue;
            };
            if !needed[index + 2] {
                continue;
            }
            let region = regions[index + 2];
            let mut items = Vec::new();
            if region[2] <= 0. || region[3] <= 0. {
                images.push((index, items));
                continue;
            }
            if let Some(id) = image.href.strip_prefix('#') {
                let Some(&source) = self.ids.get(id) else {
                    continue;
                };
                if self.resources.borrow().contains(&source) {
                    continue;
                }
                self.resources.borrow_mut().push(source);
                let units = if graph.definition.primitive_user_space {
                    [1.; 2]
                } else {
                    [bbox[2], bbox[3]]
                };
                let placement = Matrix::affine([units[0], 0., 0., units[1], region[0], region[1]]);
                let viewport = [region[2] / units[0], region[3] / units[1]];
                let mut drawn = Vec::new();
                if let Some(Element::Symbol { view_box, fit }) =
                    self.styles[source].svg.as_deref().map(|s| &s.element)
                {
                    let view =
                        view_box.map_or(Matrix::IDENTITY, |b| svg::viewport(b, *fit, viewport));
                    let space = view_box.map_or(viewport, |b| [b[2], b[3]]);
                    for &child in &self.template.nodes[source].children {
                        self.element(
                            child as usize,
                            space,
                            matrix.times(placement).times(view),
                            &mut drawn,
                            None,
                        );
                    }
                    drawn = vec![Item::Transform {
                        matrix: view,
                        items: drawn,
                    }];
                } else {
                    self.element(source, viewport, matrix.times(placement), &mut drawn, None);
                }
                self.resources.borrow_mut().pop();
                items.push(Item::Transform {
                    matrix: placement,
                    items: drawn,
                });
            } else {
                let picture = match image
                    .href
                    .strip_prefix("resource://")
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    Some(resource) => self.pictures.get(resource),
                    None => self
                        .embedded
                        .iter()
                        .find(|e| e.node as usize == image.node)
                        .map(|e| &e.picture),
                };
                if let Some(picture) = picture {
                    let [nw, nh] = match picture.natural() {
                        Natural::Size(s) => s.map(f64::from),
                        Natural::Ratio(r) => [150. * f64::from(r.unwrap_or(2.)), 150.],
                    };
                    if nw > 0. && nh > 0. {
                        let placed = Matrix::translate(region[0], region[1]).times(svg::viewport(
                            [0., 0., nw, nh],
                            image.fit,
                            [region[2], region[3]],
                        ));
                        let first = rect(mapped(placed, [0., 0., nw, nh]));
                        match picture {
                            Picture::Image { image: texture, .. } => items.push(Item::Image {
                                rect: rect(region),
                                radii: SQUARE,
                                tiles: Tiles {
                                    first,
                                    step: [None; 2],
                                },
                                image: *texture,
                                pixelated: self.styles[image.node].pixelated,
                            }),
                            Picture::Svg(doc) => items.extend(document(doc, first, self.cache)),
                            Picture::None => {}
                        }
                    }
                }
            }
            images.push((index, items));
        }
        images
    }

    /// If `i` is an `<image>` of a file, in a viewport `size` user units big: the viewport it
    /// gives its image, the rect its fit puts the image in, and the scene's image. `None` too
    /// where its viewport has no area.
    fn image(
        &self,
        i: usize,
        size: [f64; 2],
        copy: Option<&Instance>,
    ) -> Option<([f64; 4], [f64; 4], &Picture)> {
        let s = self.style(i, copy);
        let Element::Image {
            at,
            size: given,
            fit,
            resource,
        } = s.svg.as_deref()?.element
        else {
            return None;
        };
        let picture = match resource {
            Some(r) => self.pictures.get(r as usize)?,
            None => &self.embedded.iter().find(|e| e.node as usize == i)?.picture,
        };
        let [nw, nh] = match picture.natural() {
            Natural::Size(size) => size.map(f64::from),
            Natural::Ratio(ratio) => [150.0 * f64::from(ratio.unwrap_or(2.0)), 150.0],
        };
        let given = [0, 1].map(|k| given[k].map(|l| l.resolve(size[k])));
        let [w, h] = match given {
            [Some(w), Some(h)] => [w, h],
            [Some(w), None] => [w, w * nh / nw],
            [None, Some(h)] => [h * nw / nh, h],
            [None, None] => [nw, nh],
        };
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        let [x, y] = [0, 1].map(|k| at[k].resolve(size[k]));
        let placed = Matrix::translate(x, y).times(svg::viewport([0.0, 0.0, nw, nh], fit, [w, h]));
        Some(([x, y, w, h], mapped(placed, [0.0, 0.0, nw, nh]), picture))
    }

    /// The box around the bounding boxes of `elements`, each through its transform.
    fn around(
        &self,
        elements: &[u32],
        size: [f64; 2],
        copy: Option<&Instance>,
    ) -> Option<[f64; 4]> {
        self.around_bounds(elements, size, copy, false)
    }

    fn around_bounds(
        &self,
        elements: &[u32],
        size: [f64; 2],
        copy: Option<&Instance>,
        stroke: bool,
    ) -> Option<[f64; 4]> {
        let boxes = elements.iter().filter_map(|&k| {
            let k = k as usize;
            let b = if stroke {
                self.stroke_bbox(k, size, copy)
            } else {
                self.bbox(k, size, copy)
            }?;
            let c = self.style(k, copy).svg.as_deref()?;
            let s = self.style(k, copy);
            Some(self.local(k, s, c, size, copy).map_or(b, |t| mapped(t, b)))
        });
        boxes.reduce(|[x, y, w, h], [u, v, p, q]| {
            let lo = [x.min(u), y.min(v)];
            let hi = [(x + w).max(u + p), (y + h).max(v + q)];
            [lo[0], lo[1], hi[0] - lo[0], hi[1] - lo[1]]
        })
    }

    /// What clip path `id` shows of `i`, in `i`'s user units in a viewport `size` user units
    /// big, as the alpha of a mask: the shapes in it that are not hidden, opaque, each with its
    /// clip rule. `None` where it shows nothing.
    fn clip(&self, id: &str, i: usize, size: [f64; 2], copy: Option<&Instance>) -> Option<Item> {
        let &n = self.ids.get(id)?;
        if self.resources.borrow().contains(&n) {
            return None;
        }
        let c = self.styles[n].svg.as_deref()?;
        let Element::ClipPath { user_space } = c.element else {
            return None;
        };
        let mut matrix = self
            .local(n, &self.styles[n], c, size, None)
            .unwrap_or(Matrix::IDENTITY);
        if !user_space {
            let bbox @ [.., w, h] = self.bbox(i, size, copy)?;
            if w <= 0.0 || h <= 0.0 {
                return None;
            }
            matrix = matrix.times(svg::fractions(bbox));
        }
        let mut items = Vec::new();
        self.resources.borrow_mut().push(n);
        for &k in &self.template.nodes[n].children {
            self.clip_element(k as usize, size, None, &mut items);
        }
        if let Some(id) = self.styles[n].clip_ref.as_deref() {
            items = match self.clip(id, n, size, None) {
                Some(mask) => vec![Item::Mask {
                    mask: vec![mask],
                    items,
                }],
                None => Vec::new(),
            };
        }
        self.resources.borrow_mut().pop();
        (!items.is_empty()).then_some(Item::Transform { matrix, items })
    }

    /// Clip contents contribute silhouettes, ignoring paint, opacity and filters. A use
    /// must directly reference a shape or text; groups and indirect use chains draw nothing.
    fn clip_element(&self, i: usize, size: [f64; 2], copy: Option<&Instance>, out: &mut Vec<Item>) {
        let s = self.style(i, copy);
        let Some(c) = s.svg.as_deref() else { return };
        if s.layout.display == Display::None {
            return;
        }
        let mut items = Vec::new();
        match &c.element {
            Element::Shape(shape) if s.visible => {
                if let Some(path) = svg::path(shape, size) {
                    items.push(Item::Path {
                        path,
                        even_odd: s.clip_even_odd,
                        color: [1.0; 4],
                    });
                }
                if let svg::Shape::Text { decorations, .. } = shape {
                    items.extend(decorations.iter().map(|d| Item::Path {
                        path: d.path.clone(),
                        even_odd: false,
                        color: [1.0; 4],
                    }));
                }
            }
            Element::Text(_) => {
                for &k in &self.template.nodes[i].children {
                    self.clip_element(k as usize, size, copy, &mut items);
                }
            }
            Element::Use { .. } => {
                if let Some(instance) = s.instance.as_deref()
                    && instance
                        .style(instance.root)
                        .and_then(|s| s.svg.as_deref())
                        .is_some_and(|c| matches!(c.element, Element::Shape(_) | Element::Text(_)))
                {
                    self.clip_element(instance.root, size, Some(instance), &mut items);
                }
            }
            _ => {}
        }
        if items.is_empty() {
            return;
        }
        if let Some(id) = s.clip_ref.as_deref() {
            let Some(mask) = self.clip(id, i, size, copy) else {
                return;
            };
            items = vec![Item::Mask {
                mask: vec![mask],
                items,
            }];
        }
        match self.local(i, s, c, size, copy) {
            Some(matrix) => out.push(Item::Transform { matrix, items }),
            None => out.extend(items),
        }
    }

    /// What mask `id` shows of `i`, in `i`'s user units in a viewport `size` user units big,
    /// which `matrix` takes to px, as the alpha of a mask: what its children draw, by their
    /// luminance unless its type is alpha, inside its region. `None` where it shows nothing, as
    /// a mask in itself does.
    fn mask(
        &self,
        id: &str,
        i: usize,
        size: [f64; 2],
        matrix: Matrix,
        copy: Option<&Instance>,
    ) -> Option<Item> {
        let &n = self.ids.get(id)?;
        let c = self.styles[n].svg.as_deref()?;
        let Element::Mask(m) = c.element else {
            return None;
        };
        let bbox = (self.bbox(i, size, copy)).filter(|&[.., w, h]| w > 0.0 && h > 0.0);
        let [x, y, w, h] = match m.user_space {
            true => {
                let [x, y, w, h] = m.region;
                let [vw, vh] = size;
                [x.resolve(vw), y.resolve(vh), w.resolve(vw), h.resolve(vh)]
            }
            false => {
                let [x, y, w, h] = m.region.map(|l| l.units + l.percent / 100.0);
                let [bx, by, bw, bh] = bbox?;
                [bx + x * bw, by + y * bh, w * bw, h * bh]
            }
        };
        if w <= 0.0 || h <= 0.0 || self.resources.borrow().contains(&n) {
            return None;
        }
        let content = match m.content_user_space {
            true => Matrix::IDENTITY,
            false => svg::fractions(bbox?),
        };
        let mut items = Vec::new();
        self.resources.borrow_mut().push(n);
        for &k in &self.template.nodes[n].children {
            self.element(k as usize, size, matrix.times(content), &mut items, None);
        }
        self.resources.borrow_mut().pop();
        if items.is_empty() {
            return None;
        }
        if !m.content_user_space {
            items = vec![Item::Transform {
                matrix: content,
                items,
            }];
        }
        if !self.styles[n].mask_alpha {
            items = vec![Item::Filter {
                filters: vec![Filter::Luminance],
                items,
            }];
        }
        Some(Item::Clip {
            rect: rect([x, y, w, h]),
            radii: SQUARE,
            items,
        })
    }

    /// The markers of `shape`, with style `s`, at the vertices of its `path`, for a stroke `width`
    /// wide, in a viewport `size` user units big, which `matrix` takes to px: at each its start's,
    /// middle's or end's, and at a lone vertex its start's, then its end's.
    fn markers(
        &self,
        s: &Style,
        shape: &svg::Shape,
        path: &Path,
        width: f64,
        size: [f64; 2],
        matrix: Matrix,
    ) -> Vec<Item> {
        let mut out = Vec::new();
        if s.markers.iter().all(Option::is_none) {
            return out;
        }
        let vertices = svg::vertices(shape, path);
        let last = vertices.len().saturating_sub(1);
        // Each marker as drawn once.
        let mut drawn: [Option<Option<(svg::Orient, Matrix, Item)>>; 3] = Default::default();
        for (n, &([x, y], angle)) in vertices.iter().enumerate() {
            let which: &[usize] = match (n == 0, n == last) {
                (true, true) => &[0, 2],
                (true, false) => &[0],
                (false, true) => &[2],
                (false, false) => &[1],
            };
            for &k in which {
                let Some(id) = s.markers[k].as_deref() else {
                    continue;
                };
                let made = drawn[k].get_or_insert_with(|| self.marker(id, width, size, matrix));
                let Some((orient, base, item)) = made else {
                    continue;
                };
                let turn = match *orient {
                    svg::Orient::Auto => angle,
                    svg::Orient::AutoStartReverse if k == 0 => angle + 180.0,
                    svg::Orient::AutoStartReverse => angle,
                    svg::Orient::Angle(a) => a,
                };
                let at = Matrix::translate(f64::from(x), f64::from(y));
                out.push(Item::Transform {
                    matrix: at.times(Matrix::rotate(turn)).times(*base),
                    items: vec![item.clone()],
                });
            }
        }
        out
    }

    /// Marker `id` for a shape whose stroke is `width` wide, in a viewport `size` user units big,
    /// which `matrix` takes to px: how it turns, the transform from its viewport to a vertex at
    /// the origin along x, and what it draws in its viewport, clipped to it. `None` where it draws
    /// nothing, as a marker in itself does.
    fn marker(
        &self,
        id: &str,
        width: f64,
        size: [f64; 2],
        matrix: Matrix,
    ) -> Option<(svg::Orient, Matrix, Item)> {
        let &n = self.ids.get(id)?;
        let Element::Marker(m) = self.styles[n].svg.as_deref()?.element else {
            return None;
        };
        let [w, h] = [m.size[0].resolve(size[0]), m.size[1].resolve(size[1])];
        let scale = if m.user_space { 1.0 } else { width };
        if w <= 0.0 || h <= 0.0 || scale <= 0.0 || self.resources.borrow().contains(&n) {
            return None;
        }
        let (fitted, units) = match m.view_box {
            Some([.., vw, vh]) if vw <= 0.0 || vh <= 0.0 => return None,
            Some(b) => (svg::viewport(b, m.fit, [w, h]), [b[2], b[3]]),
            None => (Matrix::IDENTITY, [w, h]),
        };
        let [rx, ry] = [0, 1].map(|k| m.reference[k].resolve(units[k]) as f32);
        let [rx, ry] = fitted.apply([rx, ry]).map(f64::from);
        let base = Matrix::scale(scale).times(Matrix::translate(-rx, -ry));
        let mut items = Vec::new();
        self.resources.borrow_mut().push(n);
        for &c in &self.template.nodes[n].children {
            let m = matrix.times(base).times(fitted);
            self.element(c as usize, units, m, &mut items, None);
        }
        self.resources.borrow_mut().pop();
        if items.is_empty() {
            return None;
        }
        let item = Item::Clip {
            rect: rect([0.0, 0.0, w, h]),
            radii: SQUARE,
            items: vec![Item::Transform {
                matrix: fitted,
                items,
            }],
        };
        Some((m.orient, base, item))
    }

    /// A shape's paint at `opacity`, in a viewport `size` user units big, over the shape's
    /// bounding box `bbox`; `None` where it paints nothing.
    fn paint(
        &self,
        paint: &Paint,
        opacity: f64,
        s: &Style,
        size: [f64; 2],
        bbox: [f64; 4],
        parent: Matrix,
    ) -> Option<Fill> {
        let mut color = match paint {
            Paint::None => return None,
            Paint::Color(c) => *c,
            Paint::Mixed(c) => c.resolve(s.color),
            Paint::CurrentColor => s.color,
            Paint::Url(id) => {
                if self.ids.get(&**id).is_some_and(|&n| {
                    matches!(
                        self.styles[n].svg.as_deref().map(|s| &s.element),
                        Some(Element::Pattern(_))
                    )
                }) {
                    return self.pattern(id, size, bbox, parent, opacity);
                }
                let (mut ramp, matrix) = self.gradient(id, size, bbox)?;
                let colors = ramp.segments.iter_mut().flat_map(|s| &mut s.colors);
                colors.for_each(|c| c[3] *= opacity as f32);
                let shows = (ramp.segments.iter()).any(|s| s.colors.iter().any(|c| c[3] > 0.0));
                return shows.then_some(Fill::Ramp(ramp, matrix));
            }
        };
        color[3] *= opacity;
        (color[3] > 0.0).then(|| Fill::Color(work(color)))
    }

    /// Resolve one pattern tile; only the tiles intersecting a painted path are repeated.
    fn pattern(
        &self,
        id: &str,
        size: [f64; 2],
        bbox: [f64; 4],
        parent: Matrix,
        opacity: f64,
    ) -> Option<Fill> {
        let &root = self.ids.get(id)?;
        if self.resources.borrow().contains(&root) {
            return None;
        }
        let (mut next, mut visited, mut children) = (Some(root), Vec::new(), None);
        let mut p = svg::Pattern::default();
        while let Some(n) = next {
            if visited.contains(&n) {
                break;
            }
            visited.push(n);
            let Element::Pattern(from) = &self.styles[n].svg.as_deref()?.element else {
                break;
            };
            for k in 0..4 {
                p.region[k] = p.region[k].or(from.region[k]);
            }
            p.user_space = p.user_space.or(from.user_space);
            p.content_user_space = p.content_user_space.or(from.content_user_space);
            p.transform = p.transform.or(from.transform);
            p.view_box = p.view_box.or(from.view_box);
            p.fit = p.fit.or(from.fit);
            if children.is_none() && !self.template.nodes[n].children.is_empty() {
                children = Some(n);
            }
            next = from.href.as_ref().and_then(|h| self.ids.get(&**h).copied());
        }
        let children = children?;
        let user = p.user_space.unwrap_or(false);
        let content_user = p.content_user_space.unwrap_or(true);
        let basis = if user { size } else { [1.0; 2] };
        let [x, y, w, h] =
            std::array::from_fn(|k| p.region[k].unwrap_or_default().resolve(basis[k % 2]));
        if w <= 0.0 || h <= 0.0 || (!user || !content_user) && (bbox[2] <= 0.0 || bbox[3] <= 0.0) {
            return None;
        }
        let base = if user {
            Matrix::IDENTITY
        } else {
            svg::fractions(bbox)
        };
        let matrix = base.times(p.transform.unwrap_or(Matrix::IDENTITY));
        let (content, units) = if let Some(b) = p.view_box {
            if b[2] <= 0.0 || b[3] <= 0.0 {
                return None;
            }
            let fit = p.fit.unwrap_or(svg::Fit {
                align: Some([0.5; 2]),
                slice: false,
            });
            (svg::viewport(b, fit, [w, h]), [b[2], b[3]])
        } else {
            let scales = match (user, content_user) {
                (true, false) => [bbox[2], bbox[3]],
                (false, true) => [1.0 / bbox[2], 1.0 / bbox[3]],
                _ => [1.0; 2],
            };
            (
                Matrix::affine([scales[0], 0.0, 0.0, scales[1], 0.0, 0.0]),
                if content_user { size } else { [1.0; 2] },
            )
        };
        let mut items = Vec::new();
        let copy = self.styles[root].instance.as_deref();
        self.resources.borrow_mut().push(root);
        for &child in &self.template.nodes[children].children {
            self.element(
                child as usize,
                units,
                parent.times(matrix).times(content),
                &mut items,
                copy,
            );
        }
        self.resources.borrow_mut().pop();
        if items.is_empty() {
            return None;
        }
        let tile = Item::Transform {
            matrix: content,
            items,
        };
        Some(Fill::Pattern {
            tile: Box::new(tile),
            matrix,
            region: [x, y, w, h],
            opacity: opacity as f32,
        })
    }

    /// Gradient `id` over a shape's bounding box `bbox`, in a viewport `size` user units big,
    /// and the transform from where its ramp is to the shape's user units: each attribute the
    /// first of the gradients along its `href`s has, and the stops of the first with any. `None`
    /// if there is no such gradient; an `href` to none ends the chain.
    fn gradient(&self, id: &str, size: [f64; 2], bbox: [f64; 4]) -> Option<(Ramp, Matrix)> {
        let (mut chain, mut stops) = (Vec::new(), Vec::new());
        let mut next = Some(id);
        while let Some(id) = next
            && chain.len() < 16
        {
            let Some(&n) = self.ids.get(id) else {
                break;
            };
            let Some(Element::Gradient(g)) = self.styles[n].svg.as_deref().map(|c| &c.element)
            else {
                break;
            };
            if stops.is_empty() {
                for &k in &self.template.nodes[n].children {
                    let s = &self.styles[k as usize];
                    let Some(Element::Stop(offset)) = s.svg.as_deref().map(|c| &c.element) else {
                        continue;
                    };
                    let mut color = match &s.stop_color {
                        Paint::CurrentColor => s.color,
                        Paint::Color(c) => *c,
                        Paint::Mixed(c) => c.resolve(s.color),
                        _ => color::BLACK,
                    };
                    color[3] *= s.stop_opacity;
                    stops.push((*offset, color));
                }
            }
            chain.push(g);
            next = g.href.as_deref();
        }
        let linear = self.styles[*self.ids.get(id)?].gradient_linear;
        svg::gradient(&chain, &stops, bbox, size, linear)
    }
}

/// An outer SVG is an isolated group. Only allocate that layer when a blend can read past
/// it: filters, masks and existing normal groups already isolate their descendants.
pub(super) fn needs_isolation(items: &[Item]) -> bool {
    items.iter().any(|item| match item {
        Item::Group { blend, .. } => *blend != Blend::Normal,
        Item::Transform { items, .. } | Item::Clip { items, .. } | Item::Cut { items, .. } => {
            needs_isolation(items)
        }
        _ => false,
    })
}

/// The bounding box of `[x, y, width, height]` through `m`.
fn mapped(m: Matrix, b: [f64; 4]) -> [f64; 4] {
    let r = m.bounds(rect(b));
    [r.x, r.y, r.width, r.height].map(f64::from)
}

/// A rectangle of x, y, width and height.
fn rect([x, y, width, height]: [f64; 4]) -> Rect {
    Rect {
        x: x as f32,
        y: y as f32,
        width: width as f32,
        height: height as f32,
    }
}

/// A shape's paint: a working color, or a ramp in the space `Matrix` takes to its user units.
enum Fill {
    Color([f32; 4]),
    Ramp(Ramp, Matrix),
    Pattern {
        tile: Box<Item>,
        matrix: Matrix,
        region: [f64; 4],
        opacity: f32,
    },
}

/// `path` filled with `fill`: a ramp over the path's bounding box `cover`, in the space it is
/// in, masked by the path; `scale` is px a user unit, for the edge pixels the path covers.
fn painted(path: Path, even_odd: bool, fill: Fill, cover: [f64; 4], scale: f64) -> Option<Item> {
    let (ramp, matrix) = match fill {
        Fill::Color(color) => {
            return Some(Item::Path {
                path,
                even_odd,
                color,
            });
        }
        Fill::Ramp(ramp, matrix) => (ramp, matrix),
        Fill::Pattern {
            tile,
            matrix,
            region: [x, y, w, h],
            opacity,
        } => {
            return Some(Item::Mask {
                mask: vec![Item::Path {
                    path,
                    even_odd,
                    color: [1.0; 4],
                }],
                items: vec![Item::Pattern {
                    rect: rect(cover),
                    matrix,
                    region: [x, y, w, h],
                    opacity,
                    items: vec![*tile],
                }],
            });
        }
    };
    let margin = 2.0 / scale.max(1e-9);
    let [x, y, w, h] = cover;
    let around = Rect {
        x: (x - margin) as f32,
        y: (y - margin) as f32,
        width: (w + 2.0 * margin) as f32,
        height: (h + 2.0 * margin) as f32,
    };
    let rect = matrix.inverse()?.bounds(around);
    // Where its colors go from the image's top left, as a gradient image has it.
    let ramp = Ramp {
        geometry: ramp.geometry.moved([-rect.x, -rect.y]),
        ..ramp
    };
    let gradient = Item::Gradient {
        rect,
        radii: SQUARE,
        tiles: Tiles {
            first: rect,
            step: [None; 2],
        },
        ramp,
    };
    Some(Item::Mask {
        mask: vec![Item::Path {
            path,
            even_odd,
            color: [1.0; 4],
        }],
        items: vec![Item::Transform {
            matrix,
            items: vec![gradient],
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_color_space_inherits_through_elements_not_href() {
        use crate::style::gradient::Space;
        let source = indoc::indoc! {r##"
            <svg xmlns="http://www.w3.org/2000/svg" width="120" height="80">
              <defs color-interpolation="linearRGB">
                <linearGradient id="linear"><stop stop-color="#808080"/><stop offset="1" stop-color="white" stop-opacity="0.5"/></linearGradient>
                <linearGradient id="override" href="#linear" style="color-interpolation:sRGB"/>
                <radialGradient id="inherit" href="#linear" style="color-interpolation:inherit"/>
                <linearGradient id="initial" href="#linear" style="color-interpolation:initial"/>
              </defs>
              <linearGradient id="separate" href="#linear"/>
            </svg>
        "##};
        let doc = Document::parse(source, &mut vec![]).unwrap();
        let cache = Cache::default();
        let drawing = Drawing::new(&doc.template, &doc.styles, &[], &[], &cache);
        for (id, linear) in [
            ("linear", true),
            ("override", false),
            ("inherit", true),
            ("initial", false),
            ("separate", false),
        ] {
            let (ramp, _) = drawing
                .gradient(id, [120.0, 80.0], [0.0, 0.0, 120.0, 80.0])
                .unwrap();
            assert_eq!(
                ramp.space,
                if linear {
                    Space::SrgbLinear
                } else {
                    Space::Srgb
                }
            );
            let first = ramp.segments[0].colors[0];
            let gray = 128.0 / 255.0;
            let expected = if linear { color::linear(gray) } else { gray };
            assert!((f64::from(first[0]) - expected).abs() < 1e-6);
            assert_eq!(ramp.segments.last().unwrap().colors[1][3], 0.5);
            assert!(!ramp.premultiplied);
        }
        let error =
            Document::parse(&source.replace("linearRGB", "display-p3"), &mut vec![]).unwrap_err();
        assert!(error.contains("colorInterpolation"), "{error}");
    }

    #[test]
    fn clipping_cycles_are_empty_and_shared_references_can_be_reused() {
        let source = indoc::indoc! {r##"
            <svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
              <defs>
                <clipPath id="a" clip-path="url(#b)"><rect width="100" height="100"/></clipPath>
                <clipPath id="b" clip-path="url(#a)"><rect width="100" height="100"/></clipPath>
                <clipPath id="good"><rect width="50" height="100"/></clipPath>
                <clipPath id="both" clip-path="url(#good)"><rect width="100" height="50" clip-path="url(#good)"/></clipPath>
              </defs>
              <rect id="target" width="100" height="100"/>
            </svg>
        "##};
        let d = Document::parse(source, &mut vec![]).unwrap();
        let cache = Cache::default();
        let drawing = Drawing::new(&d.template, &d.styles, &d.embedded, &[], &cache);
        let target = drawing.ids["target"];
        assert!(drawing.clip("a", target, [100.0; 2], None).is_none());
        assert!(drawing.resources.borrow().is_empty());
        let first = drawing.clip("both", target, [100.0; 2], None).unwrap();
        assert_eq!(Some(first), drawing.clip("both", target, [100.0; 2], None));
        assert!(drawing.resources.borrow().is_empty());
    }
    use crate::front::Inputs;
    use crate::scene::Scene;
    use crate::time::Tick;

    #[test]
    fn nested_viewports_resolve_percentages_and_bounds() {
        let xml = indoc::indoc! {r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="200">
          <svg id="inner" x="10%" y="10%" width="55%" height="50%"
            style="width:20px" viewBox="0 0 110 50" preserveAspectRatio="none">
            <rect width="100%" height="100%"/>
          </svg>
          <svg id="zero" width="0" height="100"><rect width="400" height="200"/></svg>
        </svg>"#};
        let doc = Document::parse(xml, &mut Vec::new()).unwrap();
        let cache = Cache::default();
        let drawing = Drawing::new(&doc.template, &doc.styles, &[], &[], &cache);
        let i = drawing.ids["inner"];
        let view = doc.styles[i]
            .svg
            .as_deref()
            .unwrap()
            .element
            .viewport([400.0, 200.0])
            .unwrap();
        for (actual, expected) in view.rect.into_iter().zip([40.0, 20.0, 220.0, 100.0]) {
            assert!((actual - expected).abs() < 1e-9);
        }
        assert_eq!(view.units, [110.0, 50.0]);
        for (actual, expected) in view
            .matrix
            .apply([110.0, 50.0])
            .into_iter()
            .zip([260.0, 120.0])
        {
            assert!((actual - expected).abs() < 1e-9);
        }
        for (actual, expected) in drawing
            .bbox(i, [400.0, 200.0], None)
            .unwrap()
            .into_iter()
            .zip(view.rect)
        {
            assert!((actual - expected).abs() < 1e-5);
        }
        let mut items = Vec::new();
        drawing.element(
            drawing.ids["zero"],
            [400.0, 200.0],
            Matrix::IDENTITY,
            &mut items,
            None,
        );
        assert!(items.is_empty());
    }

    #[test]
    fn nested_viewports_recompute_sizes_on_backward_seeks() {
        let source = indoc::indoc! {r##"
            export const composition = { width: 200, height: 100, fps: 4, duration: 2 };
            export default function Scene() {
              const { seconds } = useCurrentTime();
              return <svg width={200} height={100}>
                <svg x={10} width={80+seconds*20} height="80%" viewBox="0 0 40 40">
                  <svg x="10%" y="20%" width="80%" height="50%"><rect width="100%" height="100%"/></svg>
                </svg>
              </svg>;
            }
        "##};
        let load = || {
            Scene::load(
                "nested.motion.tsx",
                &|_| Some(source.as_bytes().to_vec()),
                &Inputs::default(),
                crate::testing::fonts(),
            )
            .unwrap()
        };
        let mut scene = load();
        let initial = scene.frame(Tick::ZERO).unwrap().items;
        assert_ne!(
            initial,
            scene.frame(Tick::from_seconds(1.0).unwrap()).unwrap().items
        );
        for frame in [0, 5, 2, 7, 0] {
            let at = Tick::from_seconds(frame as f64 / 4.0).unwrap();
            assert_eq!(
                scene.frame(at).unwrap().items,
                load().frame(at).unwrap().items
            );
        }
    }

    #[test]
    fn clipping_text_ignores_paint_and_uses_instance_typography() {
        let source = indoc::indoc! {r##"
            export const composition = { width: 200, height: 100, fps: 1, duration: 1 };
            export default function Scene() { return <svg width={200} height={100}>
              <defs><text id="label" x={0} y={50}>Ab<tspan>cd</tspan></text>
              <clipPath id="cut"><use href="#label" x={10} fontSize={SIZE} ATTRS /></clipPath></defs>
              <rect width={200} height={100} fill="red" clipPath="url(#cut)"/>
            </svg>; }
        "##};
        let render = |attrs: &str, size: &str| {
            let source = source.replace("ATTRS", attrs).replace("SIZE", size);
            let mut scene = Scene::load(
                "clip.motion.tsx",
                &|_| Some(source.as_bytes().to_vec()),
                &Inputs::default(),
                crate::testing::fonts(),
            )
            .unwrap();
            scene.frame(Tick::ZERO).unwrap().items
        };
        let normal = render("", "32");
        assert!(!normal.is_empty());
        assert_eq!(
            normal,
            render(
                "fill=\"none\" stroke=\"blue\" strokeWidth={20} opacity={0}",
                "32"
            )
        );
        let decorated = render("textDecoration=\"underline\"", "32");
        assert_ne!(normal, decorated);
        assert_eq!(
            decorated,
            render(
                "textDecoration=\"underline\" fill=\"none\" stroke=\"red\" strokeWidth={15} opacity={0}",
                "32"
            )
        );
        assert_ne!(normal, render("", "48"));
        assert!(render("style={{display: \"none\"}}", "32").is_empty());
    }

    #[test]
    fn clipping_text_follows_positions_on_backward_seeks() {
        let source = indoc::indoc! {r##"
            export const composition = { width: 200, height: 100, fps: 4, duration: 2 };
            export default function Scene() {
              const { seconds } = useCurrentTime();
              return <svg width={200} height={100}><defs>
                <clipPath id="cut"><text x={10+seconds*20} y={55} fontSize={40}>A<tspan dx={seconds*4}>B</tspan></text></clipPath>
              </defs><rect width={200} height={100} fill="red" clipPath="url(#cut)"/></svg>;
            }
        "##};
        let load = || {
            Scene::load(
                "clip.motion.tsx",
                &|_| Some(source.as_bytes().to_vec()),
                &Inputs::default(),
                crate::testing::fonts(),
            )
            .unwrap()
        };
        let mut scene = load();
        for frame in [0, 5, 2, 7, 0] {
            let at = Tick::from_seconds(frame as f64 / 4.0).unwrap();
            assert_eq!(
                scene.frame(at).unwrap().items,
                load().frame(at).unwrap().items
            );
        }
    }

    #[test]
    fn pattern_tiles_follow_geometry_on_backward_seeks() {
        let source = indoc::indoc! {r##"
            export const composition = { width: 120, height: 80, fps: 4, duration: 2 };
            export default function Scene() {
              const { seconds } = useCurrentTime();
              return <svg width={120} height={80}><defs>
                <pattern id="p" patternUnits="userSpaceOnUse" width={10+seconds*5} height={20}
                  patternTransform={`rotate(${seconds*30})`}>
                  <circle cx={5} cy={10} r={3} fill="red"/>
                </pattern></defs><rect width={120} height={80} fill="url(#p)"/></svg>;
            }
        "##};
        let load = || {
            Scene::load(
                "pattern.motion.tsx",
                &|_| Some(source.as_bytes().to_vec()),
                &Inputs::default(),
                crate::testing::fonts(),
            )
            .unwrap()
        };
        let mut scene = load();
        for frame in [0, 5, 2, 7, 0] {
            let at = Tick::from_seconds(frame as f64 / 4.0).unwrap();
            assert_eq!(
                scene.frame(at).unwrap().items,
                load().frame(at).unwrap().items
            );
        }
    }

    #[test]
    fn patterns_resolve_coordinates_references_and_cycles() {
        let xml = indoc::indoc! {r##"<svg xmlns="http://www.w3.org/2000/svg">
          <defs>
            <pattern id="base" width="0.25" height="0.5" patternContentUnits="objectBoundingBox"><rect width="0.1" height="0.2"/></pattern>
            <pattern id="copy" href="#base" fill="red" x="0.1" patternTransform="translate(0.2 0)"/>
            <pattern id="empty" width="0" height="1"><rect width="1" height="1"/></pattern>
            <pattern id="a" href="#b"/><pattern id="b" href="#a"/>
            <pattern id="recursive" width="10" height="10" patternUnits="userSpaceOnUse"><rect width="10" height="10" fill="url(#recursive)"/></pattern>
          </defs>
        </svg>"##};
        let doc = Document::parse(xml, &mut Vec::new()).unwrap();
        let cache = Cache::default();
        let drawing = Drawing::new(&doc.template, &doc.styles, &[], &[], &cache);
        let instance = doc.styles[drawing.ids["copy"]].instance.as_ref().unwrap();
        assert_eq!(
            instance.styles[1].fill,
            Paint::Color(color::parse("red").unwrap())
        );
        let resolve = |id| {
            drawing.pattern(
                id,
                [300.0, 200.0],
                [10.0, 20.0, 80.0, 60.0],
                Matrix::IDENTITY,
                1.0,
            )
        };
        let Some(Fill::Pattern { matrix, region, .. }) = resolve("copy") else {
            panic!("missing referenced pattern")
        };
        assert!(
            region
                .into_iter()
                .zip([0.1, 0.0, 0.25, 0.5])
                .all(|(a, b)| (a - b).abs() < 1e-6)
        );
        assert!(
            matrix
                .apply([0.0, 0.0])
                .into_iter()
                .zip([26.0, 20.0])
                .all(|(a, b)| (a - b).abs() < 1e-5)
        );
        assert!(
            matrix
                .apply([1.0, 1.0])
                .into_iter()
                .zip([106.0, 80.0])
                .all(|(a, b)| (a - b).abs() < 1e-5)
        );
        for id in ["empty", "a", "recursive"] {
            assert!(resolve(id).is_none(), "{id}");
        }
    }

    #[test]
    fn non_scaling_strokes_use_viewport_geometry_without_inheritance() {
        let doc = Document::parse(
            indoc::indoc! {r#"<svg xmlns="http://www.w3.org/2000/svg">
          <g vector-effect="non-scaling-stroke"><path id="normal" d="M0 0H10" />
            <path id="fixed" d="M0 0H10" vector-effect="non-scaling-stroke" />
          </g></svg>"#},
            &mut Vec::new(),
        )
        .unwrap();
        let cache = Cache::default();
        let drawing = Drawing::new(&doc.template, &doc.styles, &[], &[], &cache);
        assert!(!doc.styles[drawing.ids["normal"]].non_scaling_stroke);
        assert!(doc.styles[drawing.ids["fixed"]].non_scaling_stroke);
        let path = crate::path::parse("M0 0H10").unwrap();
        let pen = svg::Pen {
            width: 4.0,
            cap: crate::style::Cap::Butt,
            join: crate::style::Join::Miter,
            miter_limit: 4.0,
            dashes: vec![],
            dash_offset: 0.0,
            path_length: None,
        };
        let matrix = Matrix::affine([3.0, 0.0, 0.0, 2.0, 5.0, 20.0]);
        for (fixed, expected) in [
            (true, [5.0, 18.0, 30.0, 4.0]),
            (false, [5.0, 16.0, 30.0, 8.0]),
        ] {
            let stroke = drawing.stroke(&path, &pen, matrix, fixed).unwrap();
            assert_eq!(svg::bounds(&stroke.mapped(|p| matrix.apply(p))), expected);
        }
        assert!(
            drawing
                .stroke(&path, &pen, Matrix::scale(0.0), true)
                .is_none()
        );
    }

    #[test]
    fn svg_effects_keep_filter_inside_mask_and_opacity() {
        let xml = indoc::indoc! {r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="80">
          <defs><clipPath id="c"><rect width="30" height="30" /></clipPath></defs>
          <g id="effect" opacity="0.5" clip-path="url(#c)"
             style="filter: blur(2px); mix-blend-mode: multiply; isolation: isolate">
            <rect width="40" height="40" fill="red" />
          </g>
        </svg>"#};
        let doc = Document::parse(xml, &mut Vec::new()).unwrap();
        let cache = Cache::default();
        let drawing = Drawing::new(&doc.template, &doc.styles, &[], &[], &cache);
        let mut items = Vec::new();
        drawing.element(
            drawing.ids["effect"],
            [80.0; 2],
            Matrix::IDENTITY,
            &mut items,
            None,
        );
        let [
            Item::Group {
                opacity,
                blend,
                items,
            },
        ] = items.as_slice()
        else {
            panic!("{items:?}")
        };
        assert_eq!((*opacity, *blend), (0.5, Blend::Multiply));
        let [Item::Mask { items, .. }] = items.as_slice() else {
            panic!("{items:?}")
        };
        assert!(matches!(items.as_slice(), [Item::Filter { .. }]));
        assert!(needs_isolation(&[Item::Group {
            opacity: 1.0,
            blend: Blend::Multiply,
            items: vec![]
        }]));
        assert!(!needs_isolation(&[Item::Group {
            opacity: 1.0,
            blend: Blend::Normal,
            items: vec![]
        }]));
    }

    #[test]
    fn css_transforms_use_svg_reference_boxes_and_override_attributes() {
        for (style, attribute, expected) in [
            ("transform: rotate(90deg)", "", [-10.0, 20.0]),
            (
                "transform: rotate(90deg); transform-box: fill-box; transform-origin: center",
                "",
                [50.0, 0.0],
            ),
            (
                "transform: scale(2); transform-box: content-box; transform-origin: top left",
                "",
                [20.0, 10.0],
            ),
            (
                "transform: scale(2); transform-box: stroke-box; transform-origin: top left",
                "",
                [25.0, 15.0],
            ),
            (
                "transform: scale(2); transform-box: border-box; transform-origin: top left",
                "",
                [25.0, 15.0],
            ),
            (
                "transform: rotate(90deg); transform-box: view-box; transform-origin: center",
                "",
                [140.0, -30.0],
            ),
            ("transform: none", "scale(3)", [20.0, 10.0]),
            ("transform: translate(5px, 7px)", "scale(3)", [25.0, 17.0]),
            ("translate: 5px 7px", "scale(3)", [65.0, 37.0]),
            (
                "transform: translate(50%, 50%); transform-box: fill-box",
                "",
                [40.0, 20.0],
            ),
        ] {
            let xml = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="10 20 200 100">
                <rect id="r" x="20" y="10" width="40" height="20" stroke="black" stroke-width="10"
                    transform="{attribute}" style="{style}" />
            </svg>"#
            )
            .replace(" transform=\"\"", "");
            let doc = Document::parse(&xml, &mut Vec::new()).unwrap();
            let cache = Cache::default();
            let drawing = Drawing::new(&doc.template, &doc.styles, &[], &[], &cache);
            let i = drawing.ids["r"];
            let s = &doc.styles[i];
            let m = drawing
                .local(i, s, s.svg.as_deref().unwrap(), [200.0, 100.0], None)
                .unwrap_or(Matrix::IDENTITY);
            let actual = m.apply([20.0, 10.0]);
            for k in 0..2 {
                assert!(
                    (actual[k] - expected[k]).abs() < 0.001,
                    "{style}: {actual:?}"
                );
            }
        }
    }

    #[test]
    fn svg_css_transforms_follow_geometry_on_backward_seeks() {
        let source = indoc::indoc! {r#"
            export const composition = { width: 200, height: 100, fps: 4, duration: 2 };
            export default function Scene() {
              const { seconds } = useCurrentTime();
              return <svg width={200} height={100}>
                <rect x={20} y={10} width={40 + seconds * 20} height={20}
                  transform="translate(500)"
                  style={{ transformBox: "fill-box", transformOrigin: "center",
                    transform: `rotate(${seconds * 90}deg)` }} />
              </svg>;
            }
        "#};
        let load = || {
            Scene::load(
                "transform.motion.tsx",
                &|_| Some(source.as_bytes().to_vec()),
                &Inputs::default(),
                crate::testing::fonts(),
            )
            .unwrap()
        };
        let mut scene = load();
        for frame in [0, 5, 2, 7, 0] {
            let time = Tick::from_seconds(frame as f64 / 4.0).unwrap();
            assert_eq!(
                scene.frame(time).unwrap().items,
                load().frame(time).unwrap().items
            );
        }
    }

    #[test]
    fn inline_and_file_shapes_order_fill_stroke_and_markers() {
        fn colors(items: &[Item]) -> Vec<[f32; 4]> {
            items
                .iter()
                .flat_map(|item| match item {
                    Item::Path { color, .. } => vec![*color],
                    Item::Transform { items, .. }
                    | Item::Clip { items, .. }
                    | Item::Group { items, .. } => colors(items),
                    _ => panic!("unexpected paint {item:?}"),
                })
                .collect()
        }
        for (order, expected) in [
            ("normal", ["red", "blue", "green"]),
            ("fill markers stroke", ["red", "green", "blue"]),
            ("stroke", ["blue", "red", "green"]),
            ("stroke markers fill", ["blue", "green", "red"]),
            ("markers", ["green", "red", "blue"]),
            ("markers stroke fill", ["green", "blue", "red"]),
        ] {
            let attribute = format!("paint-order=\"{order}\"");
            for inherited in [true, false] {
                let (root, shape) = if inherited {
                    (attribute.as_str(), "")
                } else {
                    ("", attribute.as_str())
                };
                let xml = format!(
                    r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="80" {root}>
                <defs><marker id="m" markerWidth="8" markerHeight="8" markerUnits="userSpaceOnUse">
                    <path d="M0 0H8V8Z" fill="green" />
                </marker></defs>
                <path d="M20 20H60V60Z" {shape} fill="red" stroke="blue" stroke-width="10" marker-start="url(#m)" />
            </svg>"##
                );
                let doc = Document::parse(&xml, &mut Vec::new()).unwrap();
                let first = rect([0.0, 0.0, 80.0, 80.0]);
                let file = document(&doc, first, &Cache::default()).unwrap();
                let source = format!(
                    "export const composition = {{ width: 80, height: 80, fps: 1, duration: 1 }};\n\
                 export default function Scene() {{ return ({}); }}",
                    xml.replace("paint-order", "paintOrder")
                        .replace("stroke-width", "strokeWidth")
                        .replace("marker-start", "markerStart")
                );
                let mut scene = Scene::load(
                    "stroke.motion.tsx",
                    &|_| Some(source.as_bytes().to_vec()),
                    &Inputs::default(),
                    crate::testing::fonts(),
                )
                .unwrap();
                let inline = scene.frame(Tick::ZERO).unwrap();
                let expected = expected.map(|c| work(color::parse(c).unwrap()));
                assert_eq!(colors(&[file]), expected, "file: {order}");
                assert_eq!(colors(&inline.items), expected, "inline: {order}");
            }
        }
    }
}
