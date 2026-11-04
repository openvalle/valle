//! Geometry kept across a scene's frames, separate from colors and placement.
use std::cell::RefCell;
use std::collections::HashMap;

use crate::path::Path;
use crate::svg::{self, Pen};

struct Stroke {
    path: Option<Path>,
    used: bool,
}

/// A scene's reusable painting geometry. Geometry unused in the previous frame goes away;
/// no frame's styles, colors or transforms are kept.
#[derive(Default)]
pub struct Cache {
    strokes: RefCell<HashMap<Vec<u64>, Stroke>>,
}

impl Cache {
    pub fn sweep(&mut self) {
        let strokes = self.strokes.get_mut();
        strokes.retain(|_, stroke| std::mem::take(&mut stroke.used));
    }

    pub(super) fn stroke(&self, path: &Path, pen: &Pen, scale: f64) -> Option<Path> {
        let mut key = vec![
            scale.to_bits(),
            pen.width.to_bits(),
            pen.cap as u64,
            pen.join as u64,
            pen.miter_limit.to_bits(),
            pen.dash_offset.to_bits(),
            u64::from(pen.path_length.is_some()),
            pen.path_length.unwrap_or(0.0).to_bits(),
            path.0.len() as u64,
        ];
        key.extend(path.key().into_iter().map(u64::from));
        key.extend(pen.dashes.iter().map(|v| v.to_bits()));
        let mut strokes = self.strokes.borrow_mut();
        if let Some(stroke) = strokes.get_mut(&key) {
            stroke.used = true;
            return stroke.path.clone();
        }
        let path = svg::stroke(path, pen, scale);
        strokes.insert(
            key,
            Stroke {
                path: path.clone(),
                used: true,
            },
        );
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::parse;
    use crate::style::{Cap, Join};

    #[test]
    fn strokes_reuse_geometry_but_observe_every_pen_input() {
        let mut cache = Cache::default();
        let path = parse("M 0 0 C 30 0 10 30 40 30").unwrap();
        let pen = Pen {
            width: 3.0,
            cap: Cap::Round,
            join: Join::Round,
            miter_limit: 4.0,
            dashes: vec![4.0, 2.0],
            dash_offset: 0.0,
            path_length: None,
        };
        for scale in [1.0, 1.5, 2.0, 1.0] {
            for variant in 0..7 {
                let mut p = pen.clone();
                match variant {
                    0 => {}
                    1 => p.width = 6.0,
                    2 => p.dash_offset = 1.0,
                    3 => p.dashes = vec![1.0, 3.0],
                    4 => p.path_length = Some(10.0),
                    5 => p.cap = Cap::Square,
                    _ => p.join = Join::Bevel,
                }
                let expected = svg::stroke(&path, &p, scale);
                assert_eq!(cache.stroke(&path, &p, scale), expected);
                let count = cache.strokes.borrow().len();
                assert_eq!(cache.stroke(&path, &p, scale), expected);
                assert_eq!(cache.strokes.borrow().len(), count);
            }
        }
        cache.sweep();
        assert!(!cache.strokes.borrow().is_empty());
        cache.sweep();
        assert!(cache.strokes.borrow().is_empty());
    }
}
