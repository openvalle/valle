//! A scene frame by frame: the variant and values the frontend gives at a time, styled, laid out
//! with its text and painted into a display list.

use std::rc::Rc;
use std::sync::Arc;

use crate::front::{Composition, Front, Slot};
use crate::image::{Image, Picture};
use crate::layout::{Layout, layout};
use crate::paint::{Item, Matrix, paint, transforms};
use crate::style::{Sheet, Style, gradient};
use crate::template::{Error, Template, V};
use crate::text::{Blob, Face, Text};
use crate::time::Tick;

/// The built-in sans-serif.
const SANS_SERIF: &[u8] = include_bytes!("../../../fonts/NotoSans-Variable.ttf");

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

pub struct Scene {
    front: Front,
    /// What each of the scene's resources shows.
    pictures: Vec<Picture>,
    /// The images they show, as display lists number them.
    images: Vec<Image>,
    text: Text,
    /// Most recently used first.
    sheets: Vec<(Rc<Template>, Sheet)>,
}

impl Scene {
    /// Loads the scene at `entry` as `Front::load` does, and the files of its resources; `read`
    /// gives a file's bytes by its `/`-separated path. Like a `Front`, a scene runs scene code on
    /// the thread that calls it, which needs a stack of more than 12 MiB.
    pub fn load(entry: &str, read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Scene, Error> {
        let text = |path: &str| read(path).and_then(|b| String::from_utf8(b).ok());
        let front = Front::load(entry, &text)?;
        let mut images = Vec::new();
        let pictures = (front.resources.iter())
            .map(|slot| {
                let fail = |e: String| format!("{entry}: resource {}: {e}", slot.name);
                let path = (slot.default.as_ref())
                    .ok_or_else(|| fail("nothing is bound to it, and it has no default".into()))?;
                let bytes = read(path).ok_or_else(|| fail(format!("{path}: not found")))?;
                Picture::load(&bytes, &mut images).map_err(|e| fail(format!("{path}: {e}")))
            })
            .collect::<Result<_, _>>()?;
        Ok(Scene {
            front,
            pictures,
            images,
            text: Text::new(Blob::new(Arc::new(SANS_SERIF)))?,
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

    /// The faces display lists name.
    pub fn faces(&self) -> &[Face] {
        self.text.faces()
    }

    /// The frame at `at`.
    pub fn frame(&mut self, at: Tick) -> Result<Frame, Error> {
        let frame = self.front.frame(at)?;
        match self
            .sheets
            .iter()
            .position(|(t, _)| Rc::ptr_eq(t, &frame.template))
        {
            Some(i) => {
                let kept = self.sheets.remove(i);
                self.sheets.insert(0, kept);
            }
            None => {
                let sheet = Sheet::new(&frame.template)?;
                self.sheets.insert(0, (frame.template.clone(), sheet));
                self.sheets.truncate(SHEETS);
            }
        }
        let c = self.front.composition;
        let canvas = [c.width as f32, c.height as f32];
        let styles = (self.sheets[0].1).compute(&frame.values, canvas, &self.front.resources)?;
        backgrounds(&frame.template, &styles, &self.pictures)?;
        let laid = layout(
            &frame.template,
            &frame.values,
            &styles,
            canvas,
            &mut self.text,
            &self.pictures,
        )?;
        self.text.sweep();
        let items = paint(&frame.template, &styles, &laid, &self.pictures);
        let transforms = transforms(&frame.template, &styles, &laid);
        Ok(Frame {
            template: frame.template,
            values: frame.values,
            styles,
            layout: laid,
            transforms,
            items,
        })
    }
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
