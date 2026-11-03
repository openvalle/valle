//! A scene frame by frame: the variant and values the frontend gives at a time, styled, laid out
//! with its text and painted into a display list.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::eval::{self, Clock};
use crate::front::{Composition, Front, Holds, Inputs, Slot};
use crate::image::{Image, Picture};
use crate::layout::{Layout, layout};
use crate::paint::{Item, Matrix, paint, transforms};
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

pub struct Scene {
    front: Front,
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
            // An image has one file, a font the faces of its family.
            let mut picture = Picture::None;
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
                }
                .map_err(|e| fail(format!("{path}: {e}")))?;
            }
            pictures.push(picture);
        }
        let c = front.composition;
        let canvas = [c.width as f32, c.height as f32];
        let resources = front.resources.clone();
        let measuring = RefCell::new(measuring);
        let measured = RefCell::new(HashMap::new());
        front.set_measure(Box::new(move |content: &str, style: &str| {
            let key = (content.to_string(), style.to_string());
            if let Some(&m) = measured.borrow().get(&key) {
                return Ok(m);
            }
            let m = measure(
                &mut measuring.borrow_mut(),
                &resources,
                canvas,
                content,
                style,
            )?;
            measured.borrow_mut().insert(key, m);
            Ok(m)
        }));
        Ok(Scene {
            front,
            pictures,
            images,
            text,
            sheets: Vec::new(),
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
        backgrounds(&template, &styles, &self.pictures)?;
        let items = paint(&template, &styles, &laid, &self.pictures);
        let transforms = transforms(&template, &styles, &laid);
        Ok(Frame {
            template,
            values,
            styles,
            layout: laid,
            transforms,
            items,
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

/// What `measureText` gives: the border box width and height of a Text of `content` laid out
/// alone, absolute in the canvas, in `style`, max-content wide or as its max width wraps it, and
/// its count of lines.
fn measure(
    text: &mut Text,
    resources: &[Slot],
    canvas: [f32; 2],
    content: &str,
    style: &str,
) -> Result<[f64; 3], Error> {
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
            site: Some("measureText".into()),
        }],
        roots: vec![0],
        guards: Vec::new(),
    };
    let styles = Sheet::new(&template)?.compute(&[], true, canvas, resources)?;
    let laid = layout(&template, &[], &styles, canvas, text, &[])?;
    text.sweep();
    let b = laid.boxes[0].border;
    let lines = laid.lines[0].as_ref().map_or(0, |l| l.boxes.len());
    Ok([b.width.into(), b.height.into(), lines as f64])
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
