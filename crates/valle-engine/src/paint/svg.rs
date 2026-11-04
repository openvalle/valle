//! What the elements of an `<svg>` draw, or of an SVG file: each element and its children in
//! its user units, with the paint servers, clip paths, masks, markers and copies they name by id.

use std::cell::RefCell;
use std::collections::HashMap;

use taffy::Display;

use super::{Blend, Cache, Filter, Item, Matrix, Ramp, SQUARE, Tiles, work};
use crate::layout::{Natural, Rect};
use crate::path::Path;
use crate::style::color;
use crate::style::math::Calc;
use crate::style::{Instance, Paint, Style};
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
    let Element::Svg { view_box, fit } = root.element else {
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
    /// The images a file's `<image>`s show.
    embedded: &'a [Embedded],
    /// The masks and markers what is drawing is in, innermost last.
    resources: RefCell<Vec<usize>>,
}

impl<'a> Drawing<'a> {
    pub fn new(
        template: &'a Template,
        styles: &'a [Style],
        embedded: &'a [Embedded],
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
            resources: RefCell::new(Vec::new()),
        }
    }

    /// An element in an `<svg>`, or an SVG file's root, and its children, in user units of a
    /// viewport `size` units big, which `matrix` takes to px, with its style in `copy` if that
    /// holds it: a shape filled, then stroked, unless it is hidden, each opacity on its paint's
    /// alpha; a `<use>`'s copy; then its mask, its clip path, and its opacity on all it draws as
    /// one layer, and its transform.
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
        if s.layout.display == Display::None || s.opacity <= 0.0 {
            return;
        }
        let local = local(c, size);
        let matrix = local.map_or(matrix, |t| matrix.times(t));
        let scale = matrix.radius_scale();
        let mut items = Vec::new();
        match &c.element {
            Element::G | Element::Svg { .. } => {
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
                let Some(path) = svg::path(shape, size) else {
                    return;
                };
                let bbox = svg::bounds(&path);
                let k = &s.stroke;
                let [w, h] = size;
                let diagonal = ((w * w + h * h) / 2.0).sqrt() as f32;
                let length = |l: &Calc| f64::from(l.resolve(diagonal));
                let width = length(&k.width);
                let stroke = self
                    .paint(&k.paint, k.opacity, s, size, bbox)
                    .and_then(|paint| {
                        let pen = svg::Pen {
                            width,
                            cap: k.cap,
                            join: k.join,
                            miter_limit: k.miter_limit,
                            dashes: k.dashes.iter().map(length).collect(),
                            dash_offset: length(&k.dash_offset),
                            path_length: c.path_length,
                        };
                        let outline = (pen.width > 0.0)
                            .then(|| self.cache.stroke(&path, &pen, scale))
                            .flatten()?;
                        let cover = svg::bounds(&outline);
                        painted(outline, false, paint, cover, scale)
                    });
                let markers = self.markers(s, shape, &path, width, size, matrix);
                let fill = self.paint(&s.fill, s.fill_opacity, s, size, bbox);
                items.extend(fill.and_then(|f| {
                    // Solid ellipses share one outline. Their center and radii belong to their
                    // transform, so a dot field does not encode a new path at every position.
                    if let Fill::Color(color) = &f
                        && let Some(([x, y], [rx, ry])) = svg::ellipse_geometry(shape, size)
                    {
                        return Some(Item::Transform {
                            matrix: Matrix::affine([
                                rx.into(),
                                0.0,
                                0.0,
                                ry.into(),
                                x.into(),
                                y.into(),
                            ]),
                            items: vec![Item::Path {
                                path: svg::ellipse([0.0; 2], [1.0; 2])?,
                                even_odd: s.even_odd,
                                color: *color,
                            }],
                        });
                    }
                    painted(path, s.even_odd, f, bbox, scale)
                }));
                items.extend(stroke);
                items.extend(markers);
            }
            Element::Image { .. } if s.visible => {
                let Some((viewport, first, image)) = self.image(i, size, copy) else {
                    return;
                };
                items.push(Item::Image {
                    rect: rect(viewport),
                    radii: SQUARE,
                    tiles: Tiles {
                        first: rect(first),
                        step: [None; 2],
                    },
                    image,
                    pixelated: s.pixelated,
                });
            }
            Element::Shape(_)
            | Element::Image { .. }
            | Element::Defs
            | Element::Gradient(_)
            | Element::Stop(_)
            | Element::ClipPath { .. }
            | Element::Mask(_)
            | Element::Symbol { .. }
            | Element::Marker(_) => {}
        }
        if items.is_empty() {
            return;
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
        if s.opacity < 1.0 {
            items = vec![Item::Group {
                opacity: s.opacity as f32,
                blend: Blend::Normal,
                items,
            }];
        }
        match local {
            Some(matrix) => out.push(Item::Transform { matrix, items }),
            None => out.extend(items),
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
            Element::Shape(shape) => svg::path(shape, size).map(|p| svg::bounds(&p)),
            Element::G => self.around(&self.template.nodes[i].children, size, copy),
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

    /// If `i` is an `<image>` of a file, in a viewport `size` user units big: the viewport it
    /// gives its image, the rect its fit puts the image in, and the scene's image. `None` too
    /// where its viewport has no area.
    fn image(
        &self,
        i: usize,
        size: [f64; 2],
        copy: Option<&Instance>,
    ) -> Option<([f64; 4], [f64; 4], u32)> {
        let s = self.style(i, copy);
        let Element::Image {
            at,
            size: given,
            fit,
        } = s.svg.as_deref()?.element
        else {
            return None;
        };
        let embedded = self.embedded.iter().find(|e| e.node as usize == i)?;
        let [nw, nh] = embedded.size.map(f64::from);
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
        Some((
            [x, y, w, h],
            mapped(placed, [0.0, 0.0, nw, nh]),
            embedded.image,
        ))
    }

    /// The box around the bounding boxes of `elements`, each through its transform.
    fn around(
        &self,
        elements: &[u32],
        size: [f64; 2],
        copy: Option<&Instance>,
    ) -> Option<[f64; 4]> {
        let boxes = elements.iter().filter_map(|&k| {
            let k = k as usize;
            let b = self.bbox(k, size, copy)?;
            let c = self.style(k, copy).svg.as_deref()?;
            Some(local(c, size).map_or(b, |t| mapped(t, b)))
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
        let c = self.styles[n].svg.as_deref()?;
        let Element::ClipPath { user_space } = c.element else {
            return None;
        };
        let mut matrix = c.transform.unwrap_or(Matrix::IDENTITY);
        if !user_space {
            let bbox @ [.., w, h] = self.bbox(i, size, copy)?;
            if w <= 0.0 || h <= 0.0 {
                return None;
            }
            matrix = matrix.times(svg::fractions(bbox));
        }
        let mut items = Vec::new();
        for &k in &self.template.nodes[n].children {
            let s = &self.styles[k as usize];
            let Some(c) = s.svg.as_deref() else {
                continue;
            };
            let Element::Shape(shape) = &c.element else {
                continue;
            };
            if s.layout.display == Display::None || !s.visible {
                continue;
            }
            let Some(path) = svg::path(shape, size) else {
                continue;
            };
            let path = Item::Path {
                path,
                even_odd: s.clip_even_odd,
                color: [1.0; 4],
            };
            items.push(match c.transform {
                Some(matrix) => Item::Transform {
                    matrix,
                    items: vec![path],
                },
                None => path,
            });
        }
        (!items.is_empty()).then_some(Item::Transform { matrix, items })
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
    ) -> Option<Fill> {
        let mut color = match paint {
            Paint::None => return None,
            Paint::Color(c) => *c,
            Paint::CurrentColor => s.color,
            Paint::Url(id) => {
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
                    let mut color = match s.stop_color {
                        Paint::CurrentColor => s.color,
                        Paint::Color(c) => c,
                        _ => color::BLACK,
                    };
                    color[3] *= s.stop_opacity;
                    stops.push((*offset, color));
                }
            }
            chain.push(g);
            next = g.href.as_deref();
        }
        svg::gradient(&chain, &stops, bbox, size)
    }
}

/// An SVG element's transform from its user units to its parent's, in a viewport `size` user
/// units big: its `transform`, then a `<use>`'s move by its x and y, as SVG appends it.
fn local(c: &svg::Computed, size: [f64; 2]) -> Option<Matrix> {
    let Element::Use { at, .. } = c.element else {
        return c.transform;
    };
    let [x, y] = [at[0].resolve(size[0]), at[1].resolve(size[1])];
    let moved = (x != 0.0 || y != 0.0).then(|| Matrix::translate(x, y));
    match (c.transform, moved) {
        (Some(t), Some(m)) => Some(t.times(m)),
        (t, m) => t.or(m),
    }
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
