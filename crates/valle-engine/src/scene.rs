//! A scene frame by frame: the variant and values the frontend gives at a time, styled, laid out
//! with its text and painted into a display list.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use crate::eval;
use kurbo::{BezPath, Point, Shape};

use crate::front::{Composition, Front, Holds, Inputs, Slot, Typeset};
use crate::image::{Image, Picture};
use crate::layout::{Layout, Natural, layout};
use crate::paint::{Cache, Item, Matrix, transforms, transforms_of};
use crate::slug;
use crate::style::{Sheet, Style, filter, gradient};
use crate::template::{
    Error, Kind, Measure, Node, Op, Piece, Program, Prop, Sample, Tag, Template, V,
};
use crate::text::{Blob, Face, FontFile, Fonts, Text};
use crate::time::Tick;

mod video;
use video::Shown;
pub use video::Video;

mod check;
pub use check::Checker;

/// Variants whose sheets are kept, as many as the frontend keeps.
const SHEETS: usize = 64;

/// A frame: the variant and values the frontend gave, laid out and painted.
#[derive(Debug)]
pub struct Frame {
    pub template: Rc<Template>,
    pub values: Vec<V>,
    /// Each node's computed style.
    pub styles: Rc<Vec<Style>>,
    pub layout: Rc<Layout>,
    /// Each node's transform from its px, those of `layout`, to the canvas's.
    pub transforms: Vec<Matrix>,
    pub items: Vec<Item>,
    /// The pictures its boxes draw, to draw before `items`, which read them: each image, and
    /// what it shows.
    pub pictures: Vec<(u32, Vec<Item>)>,
}

/// A frame laid out, before it is painted.
struct Laid {
    template: Rc<Template>,
    values: Vec<V>,
    styles: Vec<Style>,
    layout: Rc<Layout>,
}

/// The preceding frame's layout and its inputs. Painting data is shared with that frame rather
/// than copied, and only text values are retained from its expression program.
struct RetainedLayout {
    template: Rc<Template>,
    styles: Rc<Vec<Style>>,
    text: Vec<V>,
    natural: Vec<Natural>,
    layout: Rc<Layout>,
}

fn text_values<'a>(template: &'a Template, values: &'a [V]) -> impl Iterator<Item = &'a V> {
    template
        .nodes
        .iter()
        .flat_map(|n| match &n.kind {
            Kind::Text { content } | Kind::Span { content, .. } => &content[..],
            _ => &[],
        })
        .filter_map(|p| match p {
            Piece::Text(Prop::Expr(e)) => Some(&values[*e as usize]),
            _ => None,
        })
}

/// The node each `bounds()` of a variant measures, by its key path.
type Measured = HashMap<Rc<str>, usize>;

pub struct Scene {
    front: Front,
    /// The decoder of each video resource the host gave one, by its index.
    videos: Vec<Option<Shown>>,
    /// What each of the scene's resources shows.
    pictures: Vec<Picture>,
    /// The images they show, as display lists number them.
    images: Vec<Image>,
    text: Text,
    paint: Cache,
    retained: Option<RetainedLayout>,
    /// Most recently used first.
    sheets: Vec<(Rc<Template>, Sheet, Measured)>,
}

impl Scene {
    /// Loads the scene at `entry` as `Front::load` does, its controls bound to `inputs`, and the
    /// files of its resources; `read` gives a file's bytes by its `/`-separated path or its web
    /// address, and `fonts` the built-in fonts when text first needs them. Like a
    /// `Front`, a scene runs scene code on the thread that calls it, which needs a stack of more
    /// than 12 MiB.
    pub fn load(
        entry: &str,
        read: &dyn Fn(&str) -> Option<Vec<u8>>,
        inputs: &Inputs,
        fonts: Fonts,
    ) -> Result<Scene, Error> {
        let text = |path: &str| read(path).and_then(|b| String::from_utf8(b).ok());
        let front = Front::load(entry, &text, inputs)?;
        let mut images = Vec::new();
        let mut pictures = Vec::new();
        let mut text = Text::new(fonts.clone());
        // `measureText`'s own text, of the same fonts.
        let mut measuring = Text::new(fonts);
        for slot in &front.resources {
            let fail = |e: String| format!("{entry}: resource {}: {e}", slot.name);
            let files = (inputs.resources.get(&slot.name)).unwrap_or(&slot.default);
            if files.is_empty() {
                return Err(fail("nothing is bound to it, and it has no default".into()));
            }
            // An image has one file, a font the faces of its family; the host decodes a video.
            let mut picture = Picture::None;
            if slot.kind == Holds::Video {
                pictures.push(picture);
                continue;
            }
            for path in files {
                let bytes = read(path).ok_or_else(|| fail(format!("{path}: not found")))?;
                picture = match slot.kind {
                    Holds::Image => Picture::load(&bytes, &mut images),
                    Holds::Font => {
                        let blob = Blob::from(bytes);
                        (measuring.add_font(&slot.name, path, blob.clone()))
                            .and_then(|()| text.add_font(&slot.name, path, blob))
                            .map(|()| Picture::None)
                    }
                    Holds::Video => unreachable!("skipped above"),
                    Holds::Shader => String::from_utf8(bytes)
                        .map(|code| front.set_shader(&slot.name, code.into()))
                        .map(|()| Picture::None)
                        .map_err(|_| "a shader file is UTF-8 text".to_string()),
                }
                .map_err(|e| fail(format!("{path}: {e}")))?;
            }
            pictures.push(picture);
        }
        // The pictures its role reads, after its resources: files bound to them, or placeholders
        // of the composition's size.
        let c = front.composition;
        for (k, name) in front.role.inputs().iter().enumerate() {
            let image = match inputs.pictures.get(*name) {
                Some(path) => {
                    let fail = |e: String| format!("{entry}: input://{name}: {path}: {e}");
                    let bytes = read(path).ok_or_else(|| fail("not found".into()))?;
                    Image::decode(&bytes).map_err(fail)?
                }
                None => Image::placeholder([c.width, c.height], k),
            };
            let size = image.size;
            images.push(image);
            pictures.push(Picture::Image {
                image: (images.len() - 1) as u32,
                size,
            });
        }
        // Then those its boxes draw, frame by frame: empty until the GPU draws them.
        for _ in &front.drawn {
            images.push(Image::new([c.width, c.height], Arc::from([])));
            pictures.push(Picture::Image {
                image: (images.len() - 1) as u32,
                size: [c.width, c.height],
            });
        }
        front.set_typeset(Box::new(Typesetter {
            text: RefCell::new(measuring),
            resources: front.resources.clone(),
            canvas: [c.width as f32, c.height as f32],
            measured: RefCell::default(),
        }));
        let videos = front.resources.iter().map(|_| None).collect();
        Ok(Scene {
            front,
            videos,
            pictures,
            images,
            text,
            sheets: Vec::new(),
            paint: Cache::default(),
            retained: None,
        })
    }

    /// The scene's resources, as display lists and templates number them.
    pub fn resources(&self) -> &[Slot] {
        &self.front.resources
    }

    /// What each resource shows.
    pub fn pictures(&self) -> &[Picture] {
        &self.pictures
    }

    /// The images display lists name.
    pub fn images(&self) -> &[Image] {
        &self.images
    }

    pub fn composition(&self) -> Composition {
        self.front.composition
    }

    /// How many times the scene was traced, and why it was traced again last.
    pub fn traces(&self) -> (usize, Option<&str>) {
        (self.front.traces(), self.front.retraced())
    }

    /// The files of its font resources, a face each.
    pub fn font_files(&self) -> &[FontFile] {
        self.text.font_files()
    }

    /// The faces display lists name.
    pub fn faces(&self) -> &[Face] {
        self.text.faces()
    }

    /// Check this scene without producing display lists or final transforms.
    pub fn into_checker(self) -> Checker {
        Checker::new(self)
    }

    /// The frame at `at`.
    pub fn frame(&mut self, at: Tick) -> Result<Frame, Error> {
        let natural = self.pictures.iter().map(Picture::natural).collect();
        let Laid {
            template,
            values,
            mut styles,
            layout: laid,
        } = self.laid(at)?;
        if styles.iter().any(|s| s.motion_auto) {
            self.trails(at, &template, &values, &mut styles, &laid)?;
        }
        self.text.sweep();
        self.show_videos(&template, &values, &styles)?;
        backgrounds(&template, &styles, &self.pictures)?;
        self.paint.sweep();
        let items = self.paint.paint(&template, &styles, &laid, &self.pictures);
        let pictures = self.paint.drawn(&template, &styles, &laid, &self.pictures);
        let transforms = transforms(&template, &styles, &laid);
        let styles = Rc::new(styles);
        self.retained = Some(RetainedLayout {
            text: text_values(&template, &values).cloned().collect(),
            template: template.clone(),
            styles: styles.clone(),
            natural,
            layout: laid.clone(),
        });
        Ok(Frame {
            template,
            values,
            styles,
            layout: laid,
            transforms,
            items,
            pictures,
        })
    }

    /// The variant at `at`, its values, its styles and its layout.
    fn laid(&mut self, at: Tick) -> Result<Laid, Error> {
        let frame = self.front.frame(at)?;
        self.lay(at, frame.template, frame.values)
    }

    /// `template`, a variant, with its `values` at `at`, styled and laid out.
    fn lay(&mut self, at: Tick, template: Rc<Template>, values: Vec<V>) -> Result<Laid, Error> {
        let styles = self.styled(&template, &values)?;
        self.layout(at, template, values, styles)
    }

    /// Bind a variant and validate its computed styles, before any layout or drawing.
    fn styled(&mut self, template: &Rc<Template>, values: &[V]) -> Result<Vec<Style>, Error> {
        if let Some((previous, sheet, _)) = self.sheets.first_mut()
            && !Rc::ptr_eq(previous, template)
        {
            sheet.clear_computed();
        }
        match self
            .sheets
            .iter()
            .position(|(t, ..)| Rc::ptr_eq(t, template))
        {
            Some(i) => {
                let kept = self.sheets.remove(i);
                self.sheets.insert(0, kept);
            }
            None => {
                // A shader reads raster images only.
                for node in &template.nodes {
                    let Kind::Shader { inputs, .. } = &node.kind else {
                        continue;
                    };
                    for (name, r) in inputs {
                        if let Picture::Svg(_) = self.pictures[*r as usize] {
                            return Err(format!(
                                "{}: <ShaderLayer> inputs {name}: an SVG file is not supported \
                                 yet: a shader reads a PNG, JPEG or WebP image",
                                node.place()
                            ));
                        }
                    }
                }
                let sheet = Sheet::new(template)?;
                let measured = measured(template)?;
                self.sheets.insert(0, (template.clone(), sheet, measured));
                self.sheets.truncate(SHEETS);
            }
        }
        let c = self.front.composition;
        let canvas = [c.width as f32, c.height as f32];
        let (_, sheet, measured) = &self.sheets[0];
        let resources = &self.front.resources;
        let settled = measured.is_empty();
        sheet.compute(values, settled, canvas, resources)
    }

    /// Layout-dependent checks, including font shaping and the settling of `bounds()` values.
    fn layout(
        &mut self,
        at: Tick,
        template: Rc<Template>,
        mut values: Vec<V>,
        mut styles: Vec<Style>,
    ) -> Result<Laid, Error> {
        let c = self.front.composition;
        let canvas = [c.width as f32, c.height as f32];
        let (_, sheet, measured) = &self.sheets[0];
        let resources = &self.front.resources;
        let settled = measured.is_empty();
        // Reuse only a fully known layout whose inputs still match. Late bounds remain on the
        // existing layout/settle path. Changes to paint or post-layout movement need no layout.
        let retained = self.retained.as_ref().filter(|last| {
            settled
                && Rc::ptr_eq(&template, &last.template)
                && last.layout.canvas == canvas
                && text_values(&template, &values).eq(last.text.iter())
                && styles
                    .iter()
                    .zip(last.styles.iter())
                    .all(|(a, b)| a.lays_out_as(b))
                && self
                    .pictures
                    .iter()
                    .map(Picture::natural)
                    .eq(last.natural.iter().copied())
        });
        let laid = match retained {
            Some(last) => last.layout.clone(),
            None => Rc::new(layout(
                &template,
                &values,
                &styles,
                canvas,
                &mut self.text,
                &self.pictures,
            )?),
        };
        if !settled {
            // The late values, from the boxes laid out, set only what paints.
            let clock = self.front.clock(at);
            let bounds = |path: &str, m: Measure| {
                let b = laid.boxes[measured[path]].border;
                f64::from(match m {
                    Measure::X => b.x,
                    Measure::Y => b.y,
                    Measure::Width => b.width,
                    Measure::Height => b.height,
                })
            };
            eval::settle(&template.program, clock, &mut values, &bounds)?;
            let painted = sheet.compute(&values, true, canvas, resources)?;
            contained(&template, sheet, &styles, &painted)?;
            styles = painted;
        }
        Ok(Laid {
            template,
            values,
            styles,
            layout: laid,
        })
    }

    /// The trail of each box of `motionBlur: "auto"`, in its own px: from where its center is on
    /// the canvas when a 180° shutter opens, a quarter of a frame before `at`, to where it is when
    /// the shutter closes, a quarter after. The times hold inside the scene or wrap round it as
    /// Echo's and Shutter's do. The ends are of the frame's variant, which is not traced again
    /// for them: one where it fails or another holds counts as not moving.
    fn trails(
        &mut self,
        at: Tick,
        template: &Rc<Template>,
        values: &[V],
        styles: &mut [Style],
        laid: &Layout,
    ) -> Result<(), Error> {
        let c = self.front.composition;
        // A quarter of a frame back, the first of two samples of a 360° shutter.
        let back = c.fps.shutter(360, 0, 2);
        let reach = self.reach(template, styles);
        let mut ends = Vec::new();
        for offset in [back, -back] {
            let sample = Sample {
                offset,
                duration: c.duration,
                looped: c.looped,
            };
            let at = sample.at(at);
            if let Some(reach) = &reach {
                let then = eval::run_only(&template.program, self.front.clock(at), &reach.read);
                let Some(then) = then.ok().filter(|then| template.holds(then)) else {
                    ends.push(None);
                    continue;
                };
                if let Some(moved) =
                    self.moved(template, &reach.nodes, &then, values, styles, laid)?
                {
                    ends.push(Some((moved, None)));
                    continue;
                }
            }
            let Some(then) = self.front.values(template, at) else {
                ends.push(None);
                continue;
            };
            let end = self.lay(at, template.clone(), then)?;
            let moved = transforms(&end.template, &end.styles, &end.layout);
            ends.push(Some((moved, Some(end.layout))));
        }
        let now = transforms(template, styles, laid);
        let center = |m: &Matrix, l: &Layout, i: usize| {
            let b = l.boxes[i].border;
            m.apply([b.x + b.width / 2.0, b.y + b.height / 2.0])
        };
        for (i, style) in styles.iter_mut().enumerate() {
            if !style.motion_auto {
                continue;
            }
            let Some(inverse) = now[i].inverse() else {
                continue;
            };
            let here = center(&now[i], laid, i);
            let [open, close] = [0, 1].map(|k| {
                let end = ends[k].as_ref();
                let end = end.map(|(m, l)| center(&m[i], l.as_deref().unwrap_or(laid), i));
                inverse.apply(end.unwrap_or(here))
            });
            if let Some(filter::Function::MotionBlur(trail)) = style.filter.last_mut() {
                *trail = [0, 1].map(|k| f64::from(close[k] - open[k]));
            }
        }
        Ok(())
    }

    /// What the ends of the trails in a frame of `template`, its variant, with these `styles`
    /// restyle alone, and what they read; `None` where that is a value from `bounds()`, which an
    /// end's own layout gives. Other late values only paint, so the frame's layout holds there.
    fn reach(&self, template: &Rc<Template>, styles: &[Style]) -> Option<Reach> {
        let (_, sheet, _) = self.sheets.iter().find(|(t, ..)| Rc::ptr_eq(t, template))?;
        let nodes = sheet.reaching(styles);
        let mut roots: Vec<u32> = template.decided().collect();
        sheet.reads(&nodes, |e| roots.push(e));
        for &i in &nodes {
            if let Kind::Text { content } | Kind::Span { content, .. } = &template.nodes[i].kind {
                roots.extend(content.iter().filter_map(|p| match p {
                    Piece::Text(Prop::Expr(e)) => Some(*e),
                    _ => None,
                }));
            }
        }
        let read = template.program.reading(roots);
        if (read.iter().zip(sheet.late())).any(|(&read, &late)| read && late) {
            return None;
        }
        Some(Reach { nodes, read })
    }

    /// Restyle a trail sample. A changed layout needs the full layout-dependent checks.
    fn restyled(
        &self,
        template: &Rc<Template>,
        reach: &[usize],
        then: &[V],
        now: &[V],
        styles: &[Style],
    ) -> Result<Option<Vec<Style>>, Error> {
        let Some((_, sheet, _)) = self.sheets.iter().find(|(t, ..)| Rc::ptr_eq(t, template)) else {
            return Ok(None);
        };
        let c = self.front.composition;
        let canvas = [c.width as f32, c.height as f32];
        let resources = &self.front.resources;
        let moved = sheet.compute_only(reach, then, canvas, resources, styles)?;
        let alike = (reach.iter().zip(&moved)).all(|(&i, s)| s.lays_out_as(&styles[i]))
            && same_text(template, reach, then, now);
        if !alike {
            return Ok(None);
        }
        Ok(Some(moved))
    }

    /// The transforms of the boxes of `motionBlur: "auto"` at another time of `template`, the
    /// frame's variant, whose values are `then` there: where all that reaches them, `reach`,
    /// lays out as in the frame, of the values `now`, `styles` and `laid`, that alone is styled
    /// again, on the frame's boxes, and `None` where it does not.
    fn moved(
        &self,
        template: &Rc<Template>,
        reach: &[usize],
        then: &[V],
        now: &[V],
        styles: &mut [Style],
        laid: &Layout,
    ) -> Result<Option<Vec<Matrix>>, Error> {
        let Some(mut moved) = self.restyled(template, reach, then, now, styles)? else {
            return Ok(None);
        };
        // Their styles there in place of the frame's while their transforms read them.
        for (&i, s) in reach.iter().zip(&mut moved) {
            std::mem::swap(&mut styles[i], s);
        }
        let transforms = transforms_of(template, styles, laid, reach.iter().copied());
        for (&i, s) in reach.iter().zip(&mut moved) {
            std::mem::swap(&mut styles[i], s);
        }
        Ok(Some(transforms))
    }
}

/// The nodes the ends of a frame's trails restyle alone, as `Sheet::reaching` gives them, and the
/// expressions their styles, their text and the variant's guards read, which alone they evaluate.
struct Reach {
    nodes: Vec<usize>,
    read: Vec<bool>,
}

/// Whether the Texts and Spans among `nodes` hold the same text with the values `a` as with `b`.
fn same_text(template: &Template, nodes: &[usize], a: &[V], b: &[V]) -> bool {
    nodes.iter().all(|&i| match &template.nodes[i].kind {
        Kind::Text { content } | Kind::Span { content, .. } => content.iter().all(|p| match p {
            Piece::Text(Prop::Expr(e)) => a[*e as usize] == b[*e as usize],
            _ => true,
        }),
        _ => true,
    })
}

/// `measureText` and `textOutline`: a text laid out alone, absolute in the canvas, in a style,
/// max-content wide or as its max width wraps it, in a text of the scene's fonts of its own.
struct Typesetter {
    text: RefCell<Text>,
    resources: Vec<Slot>,
    canvas: [f32; 2],
    /// Measures by text and style.
    measured: RefCell<HashMap<(String, String), [f64; 3]>>,
}

impl Typesetter {
    /// `content` in `style`, aligned as `align` says if it says: its style and layout.
    fn laid(
        &self,
        text: &mut Text,
        content: &str,
        style: &str,
        align: Option<&str>,
    ) -> Result<(Style, Layout), Error> {
        let declared: Vec<(String, serde_json::Value)> =
            serde_json::from_str(style).map_err(|e| e.to_string())?;
        let mut props = vec![
            ("position".into(), Prop::Const(V::Str("absolute".into()))),
            ("width".into(), Prop::Const(V::Str("max-content".into()))),
        ];
        for (k, v) in declared {
            let v = match v {
                serde_json::Value::Number(n) => V::Num(n.as_f64().unwrap_or(f64::NAN)),
                serde_json::Value::String(s) => V::Str(s.into()),
                _ => return Err(format!("style {k} must be a number or a string")),
            };
            props.push((k.into(), Prop::Const(v)));
        }
        if let Some(align) = align {
            props.push(("textAlign".into(), Prop::Const(V::Str(align.into()))));
        }
        let template = Template {
            program: Program::default(),
            nodes: vec![Node {
                kind: Kind::Text {
                    content: vec![Piece::Text(Prop::Const(V::Str(content.into())))],
                },
                path: None,
                classes: Vec::new(),
                style: props,
                children: Vec::new(),
                site: None,
            }],
            roots: vec![0],
            guards: Vec::new(),
            pictures: Vec::new(),
        };
        let mut styles = Sheet::new(&template)?.compute(&[], true, self.canvas, &self.resources)?;
        let laid = layout(&template, &[], &styles, self.canvas, text, &[])?;
        text.sweep();
        Ok((styles.remove(0), laid))
    }
}

impl Typeset for Typesetter {
    fn measure(&self, content: &str, style: &str) -> Result<[f64; 3], Error> {
        let key = (content.to_string(), style.to_string());
        if let Some(&m) = self.measured.borrow().get(&key) {
            return Ok(m);
        }
        let (_, laid) = self.laid(&mut self.text.borrow_mut(), content, style, None)?;
        let b = laid.boxes[0].border;
        let lines = laid.lines[0].as_ref().map_or(0, |l| l.boxes.len());
        let m = [b.width.into(), b.height.into(), lines as f64];
        self.measured.borrow_mut().insert(key, m);
        Ok(m)
    }

    /// The outlines as path data in canvas px, to 1/1000 px: all of them as one path, its
    /// bounds, and each glyph with ink, with its baseline origin, its line and its word, the
    /// words being what white space parts.
    fn outline(
        &self,
        content: &str,
        style: &str,
        [ox, oy]: [f64; 2],
        align: &str,
    ) -> Result<String, Error> {
        let part = match align {
            "left" => 0.0,
            "center" => 0.5,
            "right" => 1.0,
            _ => {
                return Err(format!(
                    "align is \"left\", \"center\" or \"right\", not {align:?}"
                ));
            }
        };
        let mut text = self.text.borrow_mut();
        let (s, laid) = self.laid(&mut text, content, style, Some(align))?;
        let (border, at) = (laid.boxes[0].border, laid.boxes[0].content);
        let mut all = BezPath::new();
        let mut glyphs = Vec::new();
        if let Some(lines) = &laid.lines[0] {
            let first = lines.baselines.first().copied().unwrap_or(0.0);
            let dx = ox - f64::from(border.width) * part - f64::from(border.x);
            let dy = oy - f64::from(at.y + first);
            // Each byte's word.
            let mut word = vec![0u32; lines.string.len() + 1];
            let (mut count, mut inside) = (0, false);
            for (i, c) in lines.string.char_indices() {
                if c.is_whitespace() {
                    inside = false;
                } else if !inside {
                    (count, inside) = (count + 1, true);
                }
                word[i] = count.max(1) - 1;
            }
            word[lines.string.len()] = count.max(1) - 1;
            let round = |v: f64| (v * 1000.0).round() / 1000.0;
            for run in &lines.runs {
                let k = s.font.size;
                for g in &lines.glyphs[run.glyphs[0] as usize..run.glyphs[1] as usize] {
                    let (em, contours) = slug::contours(&text.faces()[g.face as usize], g.id)?;
                    if contours.is_empty() {
                        continue;
                    }
                    let k = k / f64::from(em);
                    let x = f64::from(at.x + g.x) + dx;
                    let y = f64::from(at.y + g.y) + dy;
                    let p = |[px, py]: [f32; 2]| {
                        Point::new(round(x + f64::from(px) * k), round(y - f64::from(py) * k))
                    };
                    let mut path = BezPath::new();
                    for contour in &contours {
                        path.move_to(p(contour[0][0]));
                        for &[a, c, e] in contour {
                            // A line is a curve with its control point halfway.
                            let mid = [(a[0] + e[0]) * 0.5, (a[1] + e[1]) * 0.5];
                            match c == mid {
                                true => path.line_to(p(e)),
                                false => path.quad_to(p(c), p(e)),
                            }
                        }
                        path.close_path();
                    }
                    all.extend(path.iter());
                    glyphs.push(serde_json::json!({
                        "path": path.to_svg(),
                        "x": round(x),
                        "y": round(y),
                        "line": run.line,
                        "word": word[g.cluster as usize],
                    }));
                }
            }
        }
        let r = all.bounding_box();
        let bounds = serde_json::json!({
            "x": r.x0, "y": r.y0, "width": r.width(), "height": r.height(),
        });
        let out = serde_json::json!({ "path": all.to_svg(), "bounds": bounds, "glyphs": glyphs });
        Ok(out.to_string())
    }
}

/// The node each `bounds()` of the variant measures.
fn measured(template: &Template) -> Result<Measured, Error> {
    let mut out = HashMap::new();
    for op in &template.program.ops {
        let Op::Bounds(path, _) = op else {
            continue;
        };
        if out.contains_key(path) {
            continue;
        }
        let mut nodes = (template.nodes.iter().enumerate())
            .filter(|(_, n)| n.path.as_deref() == Some(&**path))
            .map(|(i, _)| i);
        let fail = |e: &str| Err(format!("bounds(\"{path}\"): {e}"));
        let Some(i) = nodes.next() else {
            return fail("no element has this key path");
        };
        if nodes.next().is_some() {
            return fail("more than one element has this key path; give them keys that differ");
        }
        if matches!(template.nodes[i].kind, Kind::Svg { tag, .. } if tag != Tag::Svg) {
            return fail("an element in an <svg> has no box; measure the <svg>");
        }
        out.insert(path.clone(), i);
    }
    Ok(out)
}

/// Stops a frame whose late values give a box a transform or a filter, or take it away, where
/// that changes which box contains its absolute and fixed boxes: they were laid out without it.
fn contained(
    template: &Template,
    sheet: &Sheet,
    laid: &[Style],
    painted: &[Style],
) -> Result<(), Error> {
    for (i, (a, b)) in laid.iter().zip(painted).enumerate() {
        let positioned = |d: usize| {
            matches!(
                painted[d].layout.position,
                taffy::Position::Absolute | taffy::Position::Fixed
            )
        };
        if a.contains_all() != b.contains_all() && sheet.descendants(i).any(positioned) {
            let at = template.nodes[i].place();
            return Err(format!(
                "{at}: a value from bounds() gives the box a transform or a filter, which makes                  it the containing block of the absolute boxes in it, known only after layout;                  give it one that does not depend on bounds() too, such as translate: 0px"
            ));
        }
    }
    Ok(())
}

/// Stops a frame that shows an SVG file as a background or mask image, which only an Image does
/// yet.
fn backgrounds(template: &Template, styles: &[Style], pictures: &[Picture]) -> Result<(), Error> {
    for (i, s) in styles.iter().enumerate() {
        for (layers, what) in [(&s.layers, "backgroundImage"), (&s.masks, "maskImage")] {
            let svg = (layers.iter()).any(|l| {
                matches!(l.image, gradient::Image::Url(r) if matches!(pictures[r as usize], Picture::Svg(_)))
            });
            if svg {
                let at = template.nodes[i].place();
                return Err(format!(
                    "{at}: {what}: an SVG file shows in an <Image> only, not as an image of a box yet"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::*;
    use crate::layout::Rect;
    use crate::paint::{SQUARE, work};
    use crate::style::color;
    use crate::testing::fonts;

    const SOURCE: &str = indoc! {"
    export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
    export default function Scene() {
      const { seconds, progress } = useCurrentTime();
      return (
        <View style={{ display: 'flex', padding: 10 }}>
          <View style={{ width: 100 * progress, height: 20, background: 'red' }} />
          {seconds < 1 ? <Text style={{ color: 'blue' }}>early</Text> : null}
        </View>
      );
    }
"};

    /// Runs `f` on a thread with the stack scene code needs.
    fn big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(32 << 20)
            .spawn(f)
            .expect("a thread")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    }

    fn load(source: &'static str) -> Result<Scene, Error> {
        Scene::load(
            "s.tsx",
            &|path| (path == "s.tsx").then(|| source.as_bytes().to_vec()),
            &Inputs::default(),
            fonts(),
        )
    }

    #[test]
    fn frames_paint_from_source() {
        big_stack(|| {
            let mut scene = load(SOURCE).unwrap();
            assert_eq!(scene.composition().width, 200);
            let at = |s: &str| Tick::parse_seconds(s).unwrap();
            let red = work(color::parse("red").unwrap());
            let items = scene.frame(at("0.5")).unwrap().items;
            let [first, Item::Glyphs { face: 0, color, .. }] = &items[..] else {
                panic!("{items:?}")
            };
            assert_eq!(*color, work(color::parse("blue").unwrap()));
            let rect = Rect {
                x: 10.0,
                y: 10.0,
                width: 25.0,
                height: 20.0,
            };
            assert_eq!(
                *first,
                Item::Rect {
                    rect,
                    radii: SQUARE,
                    color: red
                }
            );
            assert_eq!(scene.faces().len(), 1);
            // A second variant, then the first again: its sheet is kept.
            let late = scene.frame(at("1.5")).unwrap().items;
            assert!(matches!(&late[..], [Item::Rect { rect, .. }] if rect.width == 75.0));
            scene.frame(at("0.25")).unwrap();
            assert_eq!(scene.sheets.len(), 2);
        });
    }

    #[test]
    fn errors_point_at_the_source() {
        big_stack(|| {
            let mut scene = load(indoc! {"
                export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
                export default function Scene() {
                  return <View style={{ width: 'wide' }} />;
                }
            "})
            .unwrap();
            let e = scene.frame(Tick(0)).unwrap_err();
            assert!(e.starts_with("s.tsx:3:"), "{e}");
            assert!(e.contains("style width"), "{e}");
        });
    }

    /// A PNG of RGBA `pixels`, rows of `width`.
    fn png(width: u32, pixels: &[[u8; 4]]) -> Vec<u8> {
        let mut out = Vec::new();
        let height = pixels.len() as u32 / width;
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(pixels.as_flattened()).unwrap();
        writer.finish().unwrap();
        out
    }

    #[test]
    fn images_come_from_the_resources_the_scene_declares() {
        big_stack(|| {
            let source = indoc! {"
            export const composition = { width: 40, height: 20, fps: 1, duration: 1 };
            export const controls = {
              resources: { dot: resource({ kind: 'image', default: '../img/dot.png' }) },
            };
            export default function Scene() {
              return <Image src='resource://dot' style={{ width: 10 }} />;
            }
        "};
            let image = png(2, &[[255, 0, 0, 255], [255, 255, 255, 128]]);
            let files = |source: String, image: Option<Vec<u8>>| {
                move |path: &str| match path {
                    "scenes/s.tsx" => Some(source.clone().into_bytes()),
                    "img/dot.png" => image.clone(),
                    _ => None,
                }
            };
            let read = files(source.into(), Some(image.clone()));
            let mut scene =
                Scene::load("scenes/s.tsx", &read, &Inputs::default(), fonts()).unwrap();
            // The default from the scene's folder.
            assert_eq!(scene.resources()[0].default, ["img/dot.png"]);
            assert_eq!(scene.images()[0].size, [2, 1]);
            // Premultiplied in 8 bits, as Chrome keeps it.
            assert_eq!(scene.images()[0].pixels[4..], [128, 128, 128, 128]);
            let frame = scene.frame(Tick(0)).unwrap();
            // As tall as its width and the image's ratio make it.
            let border = frame.layout.boxes[0].border;
            assert_eq!([border.width, border.height], [10.0, 5.0]);
            assert!(matches!(&frame.items[..], [Item::Image { image: 0, .. }]));

            let read = files(source.replace("://dot", "://logo"), Some(image));
            let e = Scene::load("scenes/s.tsx", &read, &Inputs::default(), fonts())
                .and_then(|mut s| s.frame(Tick(0)).map(drop))
                .unwrap_err();
            assert!(e.starts_with("scenes/s.tsx:6:"), "{e}");
            assert!(e.contains("declares no resource logo"), "{e}");
            let read = files(source.into(), None);
            let e = Scene::load("scenes/s.tsx", &read, &Inputs::default(), fonts())
                .err()
                .unwrap();
            assert!(e.contains("resource dot: img/dot.png: not found"), "{e}");

            // A file bound in place of the default.
            let wide = png(3, &[[0, 0, 255, 255]; 3]);
            let read = move |path: &str| match path {
                "scenes/s.tsx" => Some(source.as_bytes().to_vec()),
                "other/wide.png" => Some(wide.clone()),
                _ => None,
            };
            let bind = |name: &str| Inputs {
                resources: [(name.to_string(), vec!["other/wide.png".to_string()])].into(),
                ..Inputs::default()
            };
            let scene = Scene::load("scenes/s.tsx", &read, &bind("dot"), fonts()).unwrap();
            assert_eq!(scene.images()[0].size, [3, 1]);
            let e = Scene::load("scenes/s.tsx", &read, &bind("logo"), fonts())
                .err()
                .unwrap();
            assert_eq!(
                e,
                "scenes/s.tsx: resources: the scene declares no resource logo in controls.resources"
            );

            // A file on the web, by its address, as the default or bound.
            let web = source.replace("../img/dot.png", "https://x.org/dot.png");
            let read = move |path: &str| match path {
                "scenes/s.tsx" => Some(web.clone().into_bytes()),
                "https://x.org/dot.png" => Some(png(2, &[[0, 0, 0, 255]; 2])),
                "https://x.org/wide.png" => Some(png(3, &[[0, 0, 0, 255]; 3])),
                _ => None,
            };
            let scene = Scene::load("scenes/s.tsx", &read, &Inputs::default(), fonts()).unwrap();
            assert_eq!(scene.resources()[0].default, ["https://x.org/dot.png"]);
            assert_eq!(scene.images()[0].size, [2, 1]);
            let bind = |location: &str| Inputs {
                resources: [("dot".to_string(), vec![location.to_string()])].into(),
                ..Inputs::default()
            };
            let scene = Scene::load(
                "scenes/s.tsx",
                &read,
                &bind("https://x.org/wide.png"),
                fonts(),
            )
            .unwrap();
            assert_eq!(scene.images()[0].size, [3, 1]);
            let e = Scene::load("scenes/s.tsx", &read, &bind("file:///wide.png"), fonts())
                .err()
                .unwrap();
            assert_eq!(
                e,
                "scenes/s.tsx: resource dot: file:///wide.png: write a path, or an http:// or \
             https:// address"
            );
        });
    }

    #[test]
    fn background_and_mask_images_take_their_natural_size() {
        big_stack(|| {
            let source = indoc! {"
            export const composition = { width: 200, height: 40, fps: 1, duration: 1 };
            export const controls = {
              resources: { dot: resource({ kind: 'image', default: './dot.png' }) },
            };
            const image = 'url(resource://dot)';
            export default function Scene() {
              return (
                <View style={{ display: 'flex' }}>
                  <View style={{ width: 40, height: 30, backgroundImage: image }} />
                  <View style={{ width: 40, height: 30, backgroundImage: image, backgroundSize: 'auto 4px' }} />
                  <View style={{ width: 40, height: 30, backgroundImage: image, backgroundSize: 'contain' }} />
                  <View style={{ width: 40, height: 30, backgroundImage: image, backgroundSize: 'cover' }} />
                  <View style={{ width: 40, height: 30, backgroundColor: 'red', maskImage: image }} />
                </View>
              );
            }
        "};
            let image = png(2, &[[255, 0, 0, 255], [255, 255, 255, 128]]);
            let read = |path: &str| match path {
                "scenes/s.tsx" => Some(source.as_bytes().to_vec()),
                "scenes/dot.png" => Some(image.clone()),
                _ => None,
            };
            let frame = Scene::load("scenes/s.tsx", &read, &Inputs::default(), fonts())
                .unwrap()
                .frame(Tick(0))
                .unwrap();
            let size = |item: &Item| match item {
                Item::Image {
                    tiles, image: 0, ..
                } => Some([tiles.first.width, tiles.first.height]),
                _ => None,
            };
            let sizes: Vec<_> = (frame.items.iter())
                .filter_map(|item| match item {
                    Item::Mask { mask, .. } => mask.first().and_then(size),
                    item => size(item),
                })
                .collect();
            // 2 × 1 as it is, 4 px tall, fitting the box and covering it, and as a mask.
            assert_eq!(
                sizes,
                [
                    [2.0, 1.0],
                    [8.0, 4.0],
                    [40.0, 20.0],
                    [60.0, 30.0],
                    [2.0, 1.0]
                ]
            );
        });
    }

    #[test]
    fn svg_files_show_in_images_only() {
        big_stack(|| {
            let source = indoc! {"
            export const composition = { width: 200, height: 40, fps: 1, duration: 1 };
            export const controls = {
              resources: {
                dot: resource({ kind: 'image', default: './dot.png' }),
                icon: resource({ kind: 'image', default: './icon.svg' }),
              },
            };
            export default function Scene() {
              return (
                <View style={{ display: 'flex', alignItems: 'flex-start' }}>
                  <Image src='resource://icon' style={{ width: 20 }} />
                  <Image src='resource://dot' />
                  <View style={{ width: 40, height: 30, backgroundImage: 'url(resource://dot)' }} />
                </View>
              );
            }
        "};
            let icon = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 2 1'><rect width='2' height='1'/></svg>";
            let image = png(2, &[[255, 0, 0, 255], [255, 255, 255, 128]]);
            let files = |source: String| {
                let image = image.clone();
                move |path: &str| match path {
                    "scenes/s.tsx" => Some(source.clone().into_bytes()),
                    "scenes/dot.png" => Some(image.clone()),
                    "scenes/icon.svg" => Some(icon.as_bytes().to_vec()),
                    _ => None,
                }
            };
            let mut scene = Scene::load(
                "scenes/s.tsx",
                &files(source.into()),
                &Inputs::default(),
                fonts(),
            )
            .unwrap();
            // The images display lists name are the files that are not SVG.
            assert_eq!(scene.images().len(), 1);
            assert!(matches!(
                scene.pictures()[0],
                Picture::Image { image: 0, .. }
            ));
            let frame = scene.frame(Tick(0)).unwrap();
            // As tall as its viewBox's ratio makes it.
            let border = frame.layout.boxes[1].border;
            assert_eq!([border.width, border.height], [20.0, 10.0]);
            assert!(matches!(
                &frame.items[..],
                [
                    Item::Clip { .. },
                    Item::Image { image: 0, .. },
                    Item::Image { image: 0, .. }
                ]
            ));

            let source = source.replace("url(resource://dot)", "url(resource://icon)");
            let e = Scene::load(
                "scenes/s.tsx",
                &files(source.clone()),
                &Inputs::default(),
                fonts(),
            )
            .and_then(|mut s| s.frame(Tick(0)).map(drop))
            .unwrap_err();
            assert!(e.starts_with("scenes/s.tsx:13:"), "{e}");
            assert!(
                e.contains("backgroundImage: an SVG file shows in an <Image> only"),
                "{e}"
            );

            let broken = |path: &str| match path {
                "scenes/icon.svg" => {
                    Some(b"<svg xmlns='http://www.w3.org/2000/svg'><text/></svg>".to_vec())
                }
                _ => files(source.clone())(path),
            };
            let e = Scene::load("scenes/s.tsx", &broken, &Inputs::default(), fonts())
                .err()
                .unwrap();
            assert_eq!(
                e,
                "scenes/s.tsx: resource icon: scenes/icon.svg: 1:41: <text> is not supported yet"
            );
        });
    }

    #[test]
    fn auto_motion_blur_measures_the_trail_on_the_canvas() {
        big_stack(|| {
            // At 30 fps a 180° shutter is open a 60th of a second: 120 px a second moves 2 px. The
            // second box turns a quarter in a box turned back a quarter, and moves as fast twice
            // over: 4 px along its own y.
            let mut scene = load(indoc! {"
            export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
            const box = { position: 'absolute', width: 10, height: 10, motionBlur: 'auto' };
            function Turned() {
              const { seconds } = useCurrentTime();
              return <View style={{ ...box, rotate: '90deg', translate: `${120 * seconds}px` }} />;
            }
            export default function Scene() {
              const { seconds } = useCurrentTime();
              return (
                <View>
                  <View style={{ ...box, left: 120 * seconds }} />
                  <View style={{ position: 'absolute', rotate: '-90deg' }}>
                    <TimeScope rate={2}>
                      <Turned />
                    </TimeScope>
                  </View>
                  <View style={{ ...box, top: 50 }} />
                </View>
              );
            }
        "})
            .unwrap();
            let frame = scene.frame(Tick::parse_seconds("1").unwrap()).unwrap();
            let trail = |i: usize| match frame.styles[i].filter.last() {
                Some(filter::Function::MotionBlur(t)) => t.map(|v| (v * 1e3).round() / 1e3),
                other => panic!("{other:?}"),
            };
            assert_eq!(trail(1), [2.0, 0.0]);
            assert_eq!(trail(3), [0.0, -4.0]);
            assert_eq!(trail(4), [0.0, 0.0]);
            // Held at the start: only the half after the frame moves.
            let frame = scene.frame(Tick::ZERO).unwrap();
            let Some(filter::Function::MotionBlur(t)) = frame.styles[1].filter.last() else {
                panic!()
            };
            assert!((t[0] - 1.0).abs() < 1e-3, "{t:?}");
        });
    }

    #[test]
    fn auto_motion_blur_ends_are_of_the_frames_variant() {
        big_stack(|| {
            // At 4 fps the ends are a 16th of a second either side of a frame. The box goes at
            // 1.03 s, between the frame at 1 s and its later end: that end counts as not moving, and
            // the scene is not traced again for it.
            let mut scene = load(indoc! {"
            export const composition = { width: 200, height: 100, fps: 4, duration: 2 };
            export default function Scene() {
              const { seconds } = useCurrentTime();
              const box = { position: 'absolute', width: 10, height: 10, motionBlur: 'auto' };
              return <View>{seconds < 1.03 ? <View style={{ ...box, left: 40 * seconds }} /> : null}</View>;
            }
        "})
            .unwrap();
            let frame = scene.frame(Tick::parse_seconds("1").unwrap()).unwrap();
            let Some(filter::Function::MotionBlur(t)) = frame.styles[1].filter.last() else {
                panic!()
            };
            assert_eq!(t.map(|v| (v * 1e3).round() / 1e3), [2.5, 0.0]);
            assert_eq!(scene.traces().0, 1);
        });
    }

    #[test]
    fn auto_motion_blur_ends_restyle_alone_what_reaches_the_box() {
        big_stack(|| {
            // The last node's trail at 1 s, against where frames at its ends put its center on the
            // canvas, which it is not turned from: each scene but the first moves it by layout.
            let trail = |source: &'static str| {
                let mut scene = load(source).unwrap();
                let back = scene.composition().fps.shutter(360, 0, 2);
                let at = Tick::parse_seconds("1").unwrap();
                let mut center = |at: Tick| {
                    let frame = scene.frame(at).unwrap();
                    let i = frame.styles.len() - 1;
                    let b = frame.layout.boxes[i].border;
                    frame.transforms[i].apply([b.x + b.width / 2.0, b.y + b.height / 2.0])
                };
                let [open, close] = [at + back, at - back].map(&mut center);
                let frame = scene.frame(at).unwrap();
                let Some(filter::Function::MotionBlur(t)) =
                    frame.styles.last().unwrap().filter.last()
                else {
                    panic!()
                };
                let ends = [0, 1].map(|k| f64::from(close[k] - open[k]));
                assert!(
                    (0..2).all(|k| (t[k] - ends[k]).abs() < 1e-3),
                    "{t:?} {ends:?}"
                );
                t.map(|v| (v * 1e3).round() / 1e3)
            };
            // Moved by its translate only, beside boxes and a text out of flow that change.
            assert_eq!(
                trail(indoc! {"
                export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
                export default function Scene() {
                  const { seconds } = useCurrentTime();
                  const moved = { width: 10, height: 10, translate: `${120 * seconds}px`, motionBlur: 'auto' };
                  return (
                    <View>
                      <View style={{ position: 'absolute', left: 100 * seconds, width: 4, height: 4 }} />
                      <Text style={{ position: 'absolute', top: 50 }}>{`${10 ** (Math.floor(seconds * 120 + 0.5) - 120)}`}</Text>
                      <View style={moved} />
                    </View>
                  );
                }
            "}),
                [2.0, 0.0]
            );
            // By a box in flow before it, which grows.
            assert_eq!(
                trail(indoc! {"
                export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
                export default function Scene() {
                  const { seconds } = useCurrentTime();
                  return (
                    <View style={{ display: 'flex' }}>
                      <View style={{ width: 120 * seconds, height: 10 }} />
                      <View style={{ width: 10, height: 10, motionBlur: 'auto' }} />
                    </View>
                  );
                }
            "}),
                [2.0, 0.0]
            );
            // By a text before it, 1 at 1 s, 0.1 before and 10 after.
            let t = trail(indoc! {"
            export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
            export default function Scene() {
              const { seconds } = useCurrentTime();
              return (
                <View style={{ display: 'flex' }}>
                  <Text>{`${10 ** (Math.floor(seconds * 120 + 0.5) - 120)}`}</Text>
                  <View style={{ width: 10, height: 10, motionBlur: 'auto' }} />
                </View>
              );
            }
        "});
            assert!(t[0] != 0.0, "{t:?}");
            // By the box out of flow it is in, which moves.
            assert_eq!(
                trail(indoc! {"
                export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
                export default function Scene() {
                  const { seconds } = useCurrentTime();
                  return (
                    <View>
                      <View style={{ position: 'absolute', left: 120 * seconds }}>
                        <View style={{ width: 10, height: 10, motionBlur: 'auto' }} />
                      </View>
                    </View>
                  );
                }
            "}),
                [2.0, 0.0]
            );
            // Moved by its translate, where a box out of flow takes the place of another that moves.
            assert_eq!(
                trail(indoc! {"
                export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
                export default function Scene() {
                  const { seconds } = useCurrentTime();
                  const mark = bounds('mark');
                  const moved = { width: 10, height: 10, translate: `${120 * seconds}px`, motionBlur: 'auto' };
                  return (
                    <View>
                      <View key='mark' style={{ position: 'absolute', left: 100 * seconds, width: 4, height: 4 }} />
                      <View style={{ position: 'absolute', width: 4, height: 4, translate: `${mark.x}px` }} />
                      <View style={moved} />
                    </View>
                  );
                }
            "}),
                [2.0, 0.0]
            );
            // Moved by where a box that moves is laid out.
            assert_eq!(
                trail(indoc! {"
                export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
                export default function Scene() {
                  const { seconds } = useCurrentTime();
                  const mark = bounds('mark');
                  const moved = { width: 10, height: 10, translate: `${mark.x}px`, motionBlur: 'auto' };
                  return (
                    <View>
                      <View key='mark' style={{ position: 'absolute', left: 120 * seconds, width: 4, height: 4 }} />
                      <View style={moved} />
                    </View>
                  );
                }
            "}),
                [2.0, 0.0]
            );
            // Out of flow at its static place, under a box in flow that grows.
            assert_eq!(
                trail(indoc! {"
                export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
                export default function Scene() {
                  const { seconds } = useCurrentTime();
                  return (
                    <View>
                      <View style={{ width: 10, height: 120 * seconds }} />
                      <View style={{ position: 'absolute', width: 10, height: 10, motionBlur: 'auto' }} />
                    </View>
                  );
                }
            "}),
                [0.0, 2.0]
            );
        });
    }

    #[test]
    fn measure_text_gives_the_box_the_text_lays_out_in() {
        big_stack(|| {
            let mut scene = load(indoc! {"
            export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
            const one = measureText('Measure me', { fontSize: 20, fontWeight: 700 });
            const wrapped = measureText('one two three four', { fontSize: 16, maxWidth: 60 });
            export default function Scene() {
              return (
                <View>
                  <View style={{ width: one.width, height: one.height }} />
                  <Text style={{ position: 'absolute', fontSize: 20, fontWeight: 700 }}>
                    Measure me
                  </Text>
                  <View style={{ width: wrapped.width, height: wrapped.height }} />
                  <Text style={{ position: 'absolute', fontSize: 16, maxWidth: 60 }}>
                    one two three four
                  </Text>
                  <View style={{ width: wrapped.lines }} />
                </View>
              );
            }
        "})
            .unwrap();
            let frame = scene.frame(Tick::ZERO).unwrap();
            let size = |i: usize| {
                let b = frame.layout.boxes[i].border;
                [b.width, b.height]
            };
            assert_eq!(size(1), size(2));
            assert_eq!(size(3), size(4));
            assert!(size(3)[0] <= 60.0, "{:?}", size(3));
            let lines = frame.layout.lines[4].as_ref().unwrap().boxes.len();
            assert!(lines > 1);
            assert_eq!(size(5)[0], lines as f32);
            assert!(
                load(indoc! {"
                export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
                const m = measureText('x', { padding: { top: 1 } });
                export default function Scene() {
                  return <View />;
                }
            "})
                .and_then(|mut s| s.frame(Tick::ZERO).map(|_| ()))
                .is_err()
            );
        });
    }

    #[test]
    fn text_outline_gives_the_glyphs_as_paths() {
        big_stack(|| {
            let mut scene = load(indoc! {"
            export const composition = { width: 200, height: 100, fps: 30, duration: 2 };
            const style = { fontSize: 40, fontWeight: 700 };
            const m = measureText('Hi there', style);
            const left = textOutline('Hi there', { ...style, origin: { x: 100, y: 50 } });
            const center = textOutline('Hi there', { ...style, origin: { x: 100, y: 50 }, align: 'center' });
            const two = textOutline('a b\\nc', { fontSize: 20, whiteSpace: 'pre-line' });
            const g = left.glyphs;
            const b = left.bounds;
            export default function Scene() {
              return (
                <View>
                  <View style={{ width: g.length, height: g[1].word + 10 * g[2].word }} />
                  <View style={{ width: g[0].x, height: g[0].y }} />
                  <View style={{ width: center.glyphs[0].x + m.width / 2, height: center.bounds.width - b.width }} />
                  <View style={{ width: b.y, height: b.y + b.height }} />
                  <View style={{ width: two.glyphs.map((x) => x.line).join('') === '001' ? 1 : 0, height: two.glyphs[2].word }} />
                  <View style={{ width: left.path.startsWith('M') && left.path.includes(g[3].path) ? 1 : 0 }} />
                </View>
              );
            }
        "})
            .unwrap();
            let frame = scene.frame(Tick::ZERO).unwrap();
            let size = |i: usize| {
                let b = frame.layout.boxes[i].border;
                [b.width, b.height]
            };
            // "Hi there": 7 glyphs with ink, "i" of word 0, "t" of word 1.
            assert_eq!(size(1), [7.0, 10.0]);
            assert_eq!(size(2), [100.0, 50.0]);
            let [x, w] = size(3);
            assert!((x - 100.0).abs() < 0.01 && w.abs() < 0.01, "{:?}", size(3));
            // Ink above the baseline and below it.
            let [top, bottom] = size(4);
            assert!(top < 30.0 && bottom > 50.0, "{:?}", size(4));
            assert_eq!(size(5), [1.0, 2.0]);
            assert_eq!(size(6)[0], 1.0);
            for bad in [
                "textOutline('x', { align: 'middle' })",
                "textOutline('x', { origin: { x: '1' } })",
                "textOutline('x', { padding: { top: 1 } })",
            ] {
                let source = format!(
                    "export const composition = {{ width: 200, height: 100, fps: 30, duration: 2 }};\n\
                 const o = {bad};\n\
                 export default function Scene() {{ return <View />; }}\n"
                );
                assert!(
                    load(Box::leak(source.into_boxed_str()))
                        .and_then(|mut s| s.frame(Tick::ZERO).map(|_| ()))
                        .is_err(),
                    "{bad}"
                );
            }
        });
    }

    #[test]
    fn a_role_reads_the_pictures_the_timeline_gives_or_placeholders() {
        big_stack(|| {
            let source = indoc! {"
            export const composition = { width: 40, height: 20, fps: 30, duration: 1 };
            export const role = transition();
            export default function Scene() {
              return (
                <View style={{ display: 'flex' }}>
                  <Image src='input://from' style={{ width: 10, height: 10 }} />
                  <Image src='input://to' style={{ width: 10, height: 10 }} />
                </View>
              );
            }
        "};
            let image = png(2, &[[255, 0, 0, 255], [0, 0, 255, 255]]);
            let read = move |path: &str| match path {
                "s.tsx" => Some(source.as_bytes().to_vec()),
                "b.png" => Some(image.clone()),
                _ => None,
            };
            let mut inputs = Inputs::default();
            inputs.pictures.insert("to".into(), "b.png".into());
            let mut scene = Scene::load("s.tsx", &read, &inputs, fonts()).unwrap();
            // The placeholder is the composition's size; the bound file its own.
            let sizes: Vec<_> = (scene.pictures().iter())
                .map(|p| match p {
                    Picture::Image { image, size } => (*image, *size),
                    _ => panic!("{p:?}"),
                })
                .collect();
            assert_eq!(sizes, [(0, [40, 20]), (1, [2, 1])]);
            let frame = scene.frame(Tick(0)).unwrap();
            assert!(matches!(
                &frame.items[..],
                [Item::Image { image: 0, .. }, Item::Image { image: 1, .. }]
            ));

            inputs.pictures.insert("source".into(), "b.png".into());
            let e = Scene::load("s.tsx", &read, &inputs, fonts()).err().unwrap();
            assert!(
                e.starts_with("s.tsx: pictures: the scene reads no input://source"),
                "{e}"
            );
        });
    }

    #[test]
    fn paint_only_changes_reuse_layout_and_keep_each_frames_styles() {
        big_stack(|| {
            let mut scene = load(indoc! {"
            export const composition = {width: 200, height: 100, fps: 30, duration: 2};
            export default function Scene() {
              const { seconds } = useCurrentTime();
              return <View style={{translate: `${seconds * 20}px 0px`, opacity: 1 - seconds / 4}}>
                <Text style={{fontSize: 20}}>same text</Text>
              </View>;
            }
        "})
            .unwrap();
            let first = scene.frame(Tick::parse_seconds("0").unwrap()).unwrap();
            let next = scene.frame(Tick::parse_seconds("1").unwrap()).unwrap();
            assert!(Rc::ptr_eq(&first.layout, &next.layout));
            assert_ne!(first.styles[0].opacity, next.styles[0].opacity);
            assert_ne!(first.transforms, next.transforms);
            let again = scene.frame(Tick::parse_seconds("0").unwrap()).unwrap();
            assert_eq!(first.items, again.items);
        });
    }

    #[test]
    fn layout_reuse_observes_text_size_and_containing_blocks() {
        big_stack(|| {
            for expression in [
                "<Text>{seconds.toFixed(1)}</Text>",
                "<View style={{width: 20 + seconds, height: 10}} />",
                "<View style={{transform: seconds < 1 ? 'none' : 'translateX(0px)'}}><View style={{position:'fixed', left: 0, top: 0}} /></View>",
                "<Text style={{fontSize: 10 + seconds}}>text</Text>",
            ] {
                let source = format!(
                    "export const composition = {{width:200,height:100,fps:30,duration:2}}; export default function Scene() {{ const {{seconds}} = useCurrentTime(); return {expression}; }}"
                );
                let mut scene = Scene::load(
                    "s.tsx",
                    &|p| (p == "s.tsx").then(|| source.as_bytes().to_vec()),
                    &Inputs::default(),
                    fonts(),
                )
                .unwrap();
                let first = scene.frame(Tick::parse_seconds("0").unwrap()).unwrap();
                let next = scene.frame(Tick::parse_seconds("1.5").unwrap()).unwrap();
                assert!(!Rc::ptr_eq(&first.layout, &next.layout), "{expression}");
                let again = scene.frame(Tick::parse_seconds("0").unwrap()).unwrap();
                assert_eq!(first.items, again.items, "{expression}");
            }
        });
    }

    #[test]
    fn styled_variants_match_fresh_scenes_when_seeking_in_both_directions() {
        big_stack(|| {
            let source = indoc! {"
            export const composition = {width:200, height:100, fps:30, duration:3};
            function Label() { return <Text style={{fontSize:'1.5em'}}>label</Text>; }
            export default function Scene() {
              const {seconds} = useCurrentTime();
              return <View style={{fontSize:12 + seconds * 2, color:seconds < 1 ? 'red' : 'blue'}}>
                <Label />
                {seconds < 1.5 ? <View style={{position:'absolute', left:seconds * 20, opacity:seconds / 3}} /> : <Text>late</Text>}
              </View>;
            }
        "};
            let mut reused = load(source).unwrap();
            for seconds in ["0", "1.2", "2.4", "0.4", "2.0", "1.0", "0"] {
                let at = Tick::parse_seconds(seconds).unwrap();
                let actual = reused.frame(at).unwrap();
                let expected = load(source).unwrap().frame(at).unwrap();
                assert_eq!(actual.styles, expected.styles, "{seconds}");
                assert_eq!(actual.items, expected.items, "{seconds}");
            }
        });
    }
}
