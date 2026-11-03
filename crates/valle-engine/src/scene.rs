//! A scene frame by frame: the variant and values the frontend gives at a time, styled, laid out
//! with its text and painted into a display list.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use crate::eval::{self, Clock};
use kurbo::{BezPath, Point, Shape};

use crate::front::{Composition, Front, Holds, Inputs, Slot, Typeset};
use crate::image::{Image, Picture};
use crate::layout::{Layout, layout};
use crate::paint::{Item, Matrix, drawn, paint, transforms};
use crate::slug;
use crate::style::{Sheet, Style, filter, gradient};
use crate::template::{
    Error, Kind, Measure, Node, Op, Piece, Program, Prop, Sample, Tag, Template, V,
};
use crate::text::{Blob, Face, FontFile, Fonts, Text};
use crate::time::Tick;

/// Variants whose sheets are kept, as many as the frontend keeps.
const SHEETS: usize = 64;

/// A frame: the variant and values the frontend gave, laid out and painted.
#[derive(Debug)]
pub struct Frame {
    pub template: Rc<Template>,
    pub values: Vec<V>,
    /// Each node's computed style.
    pub styles: Vec<Style>,
    pub layout: Layout,
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
    layout: Layout,
}

/// The node each `bounds()` of a variant measures, by its key path.
type Measured = HashMap<Rc<str>, usize>;

/// A video the host decodes, for a video resource.
pub trait Video {
    /// The size of its frames, in px.
    fn size(&self) -> [u32; 2];
    /// The frame it shows `at` into it: the last that starts at or before then, or the first,
    /// before it starts.
    fn frame(&mut self, at: Tick) -> Result<Image, String>;
}

/// A video resource's decoder, the image its frames go into, and the time of the one there.
struct Shown {
    video: Box<dyn Video>,
    image: usize,
    at: Option<Tick>,
}

pub struct Scene {
    front: Front,
    /// The decoder of each video resource the host gave one, by its index.
    videos: Vec<Option<Shown>>,
    /// What each of the scene's resources shows.
    pictures: Vec<Picture>,
    /// The images they show, as display lists number them.
    images: Vec<Image>,
    text: Text,
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
            images.push(Image {
                size: [c.width, c.height],
                pixels: Arc::from([]),
            });
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
        })
    }

    /// The file of each video resource, by its name: the one bound to it, else its default.
    pub fn video_files(&self, inputs: &Inputs) -> Vec<(String, String)> {
        (self.front.resources.iter())
            .filter(|s| s.kind == Holds::Video)
            .filter_map(|s| {
                let files = inputs.resources.get(&s.name).unwrap_or(&s.default);
                Some((s.name.clone(), files.first()?.clone()))
            })
            .collect()
    }

    /// Gives video resource `name` its decoder.
    pub fn set_video(&mut self, name: &str, video: Box<dyn Video>) -> Result<(), Error> {
        let slot = (self.front.resources.iter())
            .position(|s| s.name == name && s.kind == Holds::Video)
            .ok_or_else(|| format!("the scene declares no video resource {name}"))?;
        let size = video.size();
        let pixels = vec![0; size[0] as usize * size[1] as usize * 4];
        self.images.push(Image {
            size,
            pixels: pixels.into(),
        });
        let image = self.images.len() - 1;
        self.pictures[slot] = Picture::Image {
            image: image as u32,
            size,
        };
        self.videos[slot] = Some(Shown {
            video,
            image,
            at: None,
        });
        Ok(())
    }

    /// The frame of each video a shown `<Video>` shows, decoded into its image. A video
    /// resource shows one time in a frame.
    fn show_videos(
        &mut self,
        template: &Template,
        values: &[V],
        styles: &[Style],
    ) -> Result<(), Error> {
        let mut asked: Vec<Option<Tick>> = vec![None; self.videos.len()];
        for (i, node) in template.nodes.iter().enumerate() {
            let Kind::Image {
                resource,
                video: Some(time),
            } = &node.kind
            else {
                continue;
            };
            let s = &styles[i];
            if s.layout.display == taffy::Display::None || !s.visible {
                continue;
            }
            let r = *resource as usize;
            let name = &self.front.resources[r].name;
            let seconds = match time {
                Prop::Const(V::Num(x)) => *x,
                Prop::Expr(e) => match &values[*e as usize] {
                    V::Num(x) => *x,
                    v => {
                        return Err(format!(
                            "{}: <Video> time is a number, not {v:?}",
                            node.place()
                        ));
                    }
                },
                Prop::Const(v) => {
                    return Err(format!(
                        "{}: <Video> time is a number, not {v:?}",
                        node.place()
                    ));
                }
            };
            let at = Tick::from_seconds(seconds)
                .map_err(|e| format!("{}: <Video> time: {e}", node.place()))?;
            match asked[r] {
                Some(other) if other != at => {
                    return Err(format!(
                        "{}: <Video> resource://{name} shows {} s here and {} s elsewhere in the \
                         frame: a video resource shows one time a frame, so declare another \
                         resource of the same file",
                        node.place(),
                        at.seconds_text(),
                        other.seconds_text()
                    ));
                }
                _ => asked[r] = Some(at),
            }
        }
        for (r, at) in asked.into_iter().enumerate() {
            let Some(at) = at else { continue };
            let name = &self.front.resources[r].name;
            let shown = self.videos[r]
                .as_mut()
                .ok_or_else(|| format!("video resource {name}: the host gave no decoder for it"))?;
            if shown.at == Some(at) {
                continue;
            }
            let frame =
                (shown.video.frame(at)).map_err(|e| format!("video resource {name}: {e}"))?;
            if frame.size != self.images[shown.image].size {
                return Err(format!("video resource {name}: a frame of another size"));
            }
            self.images[shown.image] = frame;
            shown.at = Some(at);
        }
        Ok(())
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

    /// The files of its font resources, a face each.
    pub fn font_files(&self) -> &[FontFile] {
        self.text.font_files()
    }

    /// The faces display lists name.
    pub fn faces(&self) -> &[Face] {
        self.text.faces()
    }

    /// The frame at `at`.
    pub fn frame(&mut self, at: Tick) -> Result<Frame, Error> {
        let Laid {
            template,
            values,
            mut styles,
            layout: laid,
        } = self.laid(at)?;
        if styles.iter().any(|s| s.motion_auto) {
            self.trails(at, &template, &mut styles, &laid)?;
        }
        self.text.sweep();
        self.show_videos(&template, &values, &styles)?;
        backgrounds(&template, &styles, &self.pictures)?;
        let items = paint(&template, &styles, &laid, &self.pictures);
        let pictures = drawn(&template, &styles, &laid, &self.pictures);
        let transforms = transforms(&template, &styles, &laid);
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
        match self
            .sheets
            .iter()
            .position(|(t, ..)| Rc::ptr_eq(t, &frame.template))
        {
            Some(i) => {
                let kept = self.sheets.remove(i);
                self.sheets.insert(0, kept);
            }
            None => {
                // A shader reads raster images only.
                for node in &frame.template.nodes {
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
                let sheet = Sheet::new(&frame.template)?;
                let measured = measured(&frame.template)?;
                self.sheets
                    .insert(0, (frame.template.clone(), sheet, measured));
                self.sheets.truncate(SHEETS);
            }
        }
        let c = self.front.composition;
        let canvas = [c.width as f32, c.height as f32];
        let (_, sheet, measured) = &self.sheets[0];
        let resources = &self.front.resources;
        let settled = measured.is_empty();
        let mut styles = sheet.compute(&frame.values, settled, canvas, resources)?;
        let laid = layout(
            &frame.template,
            &frame.values,
            &styles,
            canvas,
            &mut self.text,
            &self.pictures,
        )?;
        let mut values = frame.values;
        if !settled {
            // The late values, from the boxes laid out, set only what paints.
            let seconds = at.seconds();
            let clock = Clock {
                seconds,
                progress: (seconds / c.duration.seconds()).clamp(0.0, 1.0),
            };
            let bounds = |path: &str, m: Measure| {
                let b = laid.boxes[measured[path]].border;
                f64::from(match m {
                    Measure::X => b.x,
                    Measure::Y => b.y,
                    Measure::Width => b.width,
                    Measure::Height => b.height,
                })
            };
            eval::settle(&frame.template.program, clock, &mut values, &bounds)?;
            let painted = sheet.compute(&values, true, canvas, resources)?;
            contained(&frame.template, sheet, &styles, &painted)?;
            styles = painted;
        }
        Ok(Laid {
            template: frame.template,
            values,
            styles,
            layout: laid,
        })
    }

    /// The trail of each box of `motionBlur: "auto"`, in its own px: from where its center is on
    /// the canvas when a 180° shutter opens, a quarter of a frame before `at`, to where it is when
    /// the shutter closes, a quarter after. The times hold inside the scene or wrap round it as
    /// Echo's and Shutter's do; an end where another variant holds counts as not moving.
    fn trails(
        &mut self,
        at: Tick,
        template: &Rc<Template>,
        styles: &mut [Style],
        laid: &Layout,
    ) -> Result<(), Error> {
        let c = self.front.composition;
        // A quarter of a frame back, the first of two samples of a 360° shutter.
        let back = c.fps.shutter(360, 0, 2);
        let mut ends = Vec::new();
        for offset in [back, -back] {
            let sample = Sample {
                offset,
                duration: c.duration,
                looped: c.looped,
            };
            let end = self.laid(sample.at(at))?;
            ends.push(Rc::ptr_eq(&end.template, template).then(|| {
                let moved = transforms(&end.template, &end.styles, &end.layout);
                (moved, end.layout)
            }));
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
                inverse.apply(end.map_or(here, |(m, l)| center(&m[i], l, i)))
            });
            if let Some(filter::Function::MotionBlur(trail)) = style.filter.last_mut() {
                *trail = [0, 1].map(|k| f64::from(close[k] - open[k]));
            }
        }
        Ok(())
    }
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
mod tests;
