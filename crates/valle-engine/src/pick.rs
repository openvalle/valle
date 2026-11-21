//! Pick: what a frame draws at a point of the canvas, or in a part of it, as someone pointing at
//! the picture means it. Each box that paints something of its own is a hit where its border
//! box is, through its transform and those around it, inside the boxes around it that clip what
//! overflows them. A hidden box, one at no opacity, and those of a `<Mask>`'s source or drawn as
//! a picture (which another box shows) are not. Hits come in tree order reversed, which is the
//! order they paint in, the top first, z-index aside.

use std::rc::Rc;

use serde_json::{Value, json};
use taffy::Overflow;

use crate::layout::Rect;
use crate::scene::Frame;
use crate::style::gradient::Image;
use crate::template::{Kind, Tag};

/// A box hit, and where it is: its key path and where its JSX is written, when it has them, the
/// clip it is in, when it is in a Timeline's, and the rectangle around it on the canvas.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub node: usize,
    pub path: Option<Rc<str>>,
    pub site: Option<Rc<str>>,
    /// The ids in the clip table of the clips it is in, the innermost first.
    pub clips: Vec<Rc<str>>,
    pub bounds: Rect,
    /// How much of it is in the part picked, from 0 to 1; 1 at a point.
    pub cover: f32,
}

impl Hit {
    /// The clip of the document it is in: the outermost of its clips that is one of the
    /// document's own, not a part made for a transition, an effect or a nested timeline.
    pub fn clip(&self) -> Option<&str> {
        (self.clips.iter().rev())
            .map(|c| &**c)
            .find(|c| !c.contains(['~', '/']))
    }

    /// Its key path as its author wrote it: in a Timeline's clip, from the clip's component, not
    /// the keys the scene the Timeline lowers to gives its tracks and clips, which are none.
    pub fn key(&self) -> Option<&str> {
        let path = self.path.as_deref()?;
        match path.rsplit_once("~canvas/") {
            Some((_, key)) => Some(key),
            None if self.clips.is_empty() => Some(path),
            None => None,
        }
    }

    /// Where its JSX is written, in a file its author wrote: not the scene a Timeline lowers to,
    /// nor one of Valle's own, such as the one that draws captions.
    pub fn written(&self) -> Option<&str> {
        (self.site.as_deref())
            .filter(|s| !s.contains(".timeline.json.motion.tsx:") && !s.starts_with("__valle/"))
    }

    pub fn to_json(&self) -> Value {
        let b = self.bounds;
        let mut out = json!({
            "bounds": { "x": b.x, "y": b.y, "width": b.width, "height": b.height },
            "cover": self.cover,
        });
        if let Some(path) = self.key() {
            out["path"] = json!(path);
        }
        if let Some(site) = self.written() {
            out["site"] = json!(site);
        }
        if let Some(clip) = self.clip() {
            out["clip"] = json!(clip);
        }
        out
    }
}

/// The boxes `frame` draws at `point`, in canvas px, the top one first.
pub fn at(frame: &Frame, point: [f32; 2]) -> Vec<Hit> {
    let mut hits = Picker::new(frame).hits(|p, i| p.contains(i, point).then_some(1.0));
    hits.reverse();
    hits
}

/// The boxes `frame` draws in `area`, in canvas px: those most in it first, and of those the top
/// one first.
pub fn within(frame: &Frame, area: Rect) -> Vec<Hit> {
    let mut hits = Picker::new(frame).hits(|p, i| p.cover(i, area));
    hits.reverse();
    // Stable, so boxes as much in it keep the top first.
    hits.sort_by(|a, b| b.cover.total_cmp(&a.cover));
    hits
}

/// The hits that can be named, as JSON: an element with a key or a line, or a clip's own box,
/// once each, in the order they come.
pub fn named(hits: &[Hit]) -> Vec<Value> {
    let mut named: Vec<Value> = Vec::new();
    for v in hits.iter().map(Hit::to_json) {
        let new = !(named.iter()).any(|n| n["clip"] == v["clip"] && n["path"] == v["path"]);
        if new && (v.get("clip").is_some() || v.get("path").is_some()) {
            named.push(v);
        }
    }
    named
}

struct Picker<'a> {
    frame: &'a Frame,
    parents: Vec<Option<usize>>,
    /// Whether each box is shown where it is: under the roots, not in a mask's source or a
    /// picture, and not at no opacity.
    shown: Vec<bool>,
    clips: Vec<Vec<Rc<str>>>,
}

impl<'a> Picker<'a> {
    fn new(frame: &'a Frame) -> Picker<'a> {
        let t = &*frame.template;
        let n = t.nodes.len();
        let mut parents = vec![None; n];
        for (i, node) in t.nodes.iter().enumerate() {
            for &c in &node.children {
                parents[c as usize] = Some(i);
            }
        }
        let pictures: Vec<usize> = t.pictures.iter().map(|&(i, _)| i as usize).collect();
        let mut shown = vec![false; n];
        let mut clips = vec![Vec::new(); n];
        let mut stack: Vec<usize> = t.roots.iter().map(|&r| r as usize).collect();
        while let Some(i) = stack.pop() {
            let s = &frame.styles[i];
            shown[i] = !pictures.contains(&i) && s.opacity > 0.0;
            let mut own = parents[i].map_or_else(Vec::new, |p| clips[p].clone());
            if let Some((_, id)) = t.clips.iter().find(|(c, _)| *c as usize == i) {
                own.insert(0, id.clone());
            }
            clips[i] = own;
            if shown[i] {
                stack.extend(t.nodes[i].children.iter().map(|&c| c as usize));
            }
        }
        Picker {
            frame,
            parents,
            shown,
            clips,
        }
    }

    /// The boxes that paint, in tree order, that `hit` finds, with how much of each it does.
    fn hits(&self, hit: impl Fn(&Self, usize) -> Option<f32>) -> Vec<Hit> {
        let t = &*self.frame.template;
        (0..t.nodes.len())
            .filter(|&i| self.shown[i] && self.paints(i))
            .filter_map(|i| {
                let cover = hit(self, i)?;
                let node = &t.nodes[i];
                Some(Hit {
                    node: i,
                    path: node.path.clone(),
                    site: node.site.clone(),
                    clips: self.clips[i].clone(),
                    bounds: self.frame.transforms[i].bounds(self.frame.layout.boxes[i].border),
                    cover,
                })
            })
            .collect()
    }

    /// Whether box `i` paints something of its own.
    fn paints(&self, i: usize) -> bool {
        let s = &self.frame.styles[i];
        if !s.visible {
            return false;
        }
        match &self.frame.template.nodes[i].kind {
            Kind::Text { .. } | Kind::Image { .. } | Kind::Shader { .. } => true,
            Kind::Svg { tag, .. } => *tag == Tag::Svg,
            Kind::Span { .. } => false,
            Kind::View | Kind::Mask { .. } => {
                s.background[3] > 0.0
                    || s.layers.iter().any(|l| l.image != Image::None)
                    || s.border.iter().any(|&w| w > 0.0)
                    || !s.shadows.is_empty()
            }
        }
    }

    /// Whether `point` is in box `i`'s border box, and in the padding box of each box around it
    /// that clips.
    fn contains(&self, i: usize, point: [f32; 2]) -> bool {
        let inside = |j: usize, r: Rect| {
            let Some(back) = self.frame.transforms[j].inverse() else {
                return false;
            };
            let [x, y] = back.apply(point);
            r.width > 0.0
                && r.height > 0.0
                && x >= r.x
                && y >= r.y
                && x < r.x + r.width
                && y < r.y + r.height
        };
        let canvas = self.frame.layout.canvas;
        point[0] >= 0.0
            && point[1] >= 0.0
            && point[0] < canvas[0]
            && point[1] < canvas[1]
            && inside(i, self.frame.layout.boxes[i].border)
            && self
                .clippers(i)
                .all(|j| inside(j, self.frame.layout.boxes[j].padding))
    }

    /// How much of box `i`, as the rectangle around it on the canvas and inside those around it
    /// that clip, is in `area`; none if none of it is.
    fn cover(&self, i: usize, area: Rect) -> Option<f32> {
        let on = |j: usize, r: Rect| self.frame.transforms[j].bounds(r);
        let [w, h] = self.frame.layout.canvas;
        let mut seen = on(i, self.frame.layout.boxes[i].border);
        let whole = seen.width * seen.height;
        if whole <= 0.0 {
            return None;
        }
        let canvas = Rect {
            x: 0.0,
            y: 0.0,
            width: w,
            height: h,
        };
        for clip in self
            .clippers(i)
            .map(|j| on(j, self.frame.layout.boxes[j].padding))
        {
            seen = overlap(seen, clip)?;
        }
        let part = overlap(overlap(seen, canvas)?, area)?;
        Some(part.width * part.height / whole)
    }

    /// The boxes around `i` that clip what overflows them.
    fn clippers(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(self.parents[i], |&p| self.parents[p]).filter(|&p| {
            let o = self.frame.styles[p].layout.overflow;
            o.x != Overflow::Visible || o.y != Overflow::Visible
        })
    }
}

fn overlap(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    (right > x && bottom > y).then_some(Rect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    })
}
