//! A scene frame by frame: the variant and values the frontend gives at a time, styled, laid out
//! with its text and painted into a display list.

use std::rc::Rc;
use std::sync::Arc;

use crate::front::{Composition, Front};
use crate::layout::{Layout, layout};
use crate::paint::{Item, Matrix, paint, transforms};
use crate::style::{Sheet, Style};
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
    text: Text,
    /// Most recently used first.
    sheets: Vec<(Rc<Template>, Sheet)>,
}

impl Scene {
    /// Loads the scene at `entry` as `Front::load` does. Like a `Front`, a scene runs scene code
    /// on the thread that calls it, which needs a stack of more than 12 MiB.
    pub fn load(entry: &str, read: &dyn Fn(&str) -> Option<String>) -> Result<Scene, Error> {
        Ok(Scene {
            front: Front::load(entry, read)?,
            text: Text::new(Blob::new(Arc::new(SANS_SERIF)))?,
            sheets: Vec::new(),
        })
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
        let styles = self.sheets[0].1.compute(&frame.values, canvas)?;
        let laid = layout(
            &frame.template,
            &frame.values,
            &styles,
            canvas,
            &mut self.text,
        )?;
        self.text.sweep();
        let items = paint(&frame.template, &styles, &laid);
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

#[cfg(test)]
mod tests;
