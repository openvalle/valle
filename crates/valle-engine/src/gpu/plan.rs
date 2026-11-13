//! Per-frame GPU work and its affected regions.
use std::rc::Rc;

use super::{
    ABERRATION, DISPLACEMENT, GRAIN, LAYER, LENS, LIGHT, LUMINANCE, MOTION, MOTION_TAPS, RADIAL,
    Step, TRANSFORM, keeps_transparent,
};

use crate::layout::Rect;
use crate::paint::{Filter, Matrix};
use crate::shader::Program;

/// What is under what a plan is at, as a backdrop reads it.
#[derive(Clone, Default)]
pub(super) struct Seen {
    pub(super) here: Under,
    /// Around the innermost group, which a mask in it keeps: the backdrop of the box they are of.
    pub(super) group: Under,
}

/// The nearest target that started empty for what is drawn in it, a group's or a filter's layer,
/// or the frame, and the layers begun since, each with the instance compositing it.
#[derive(Clone, Default)]
pub(super) struct Under {
    pub(super) root: usize,
    pub(super) open: Vec<(usize, u32)>,
}

/// Where what is drawn into a layer is on it, px: min x, min y, max x, max y; none if nothing
/// is. A blur needs to run only within it, grown by the blur's spread.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Reach(pub(super) Option<[f32; 4]>);

impl Reach {
    /// Anywhere: what an effect may draw where nothing was.
    pub(super) const ALL: Reach = Reach(Some([f32::MIN, f32::MIN, f32::MAX, f32::MAX]));

    /// The bounds of `rect` in px `matrix` takes to the layer's; anywhere if they are not
    /// finite, or if a corner goes behind the eye, where its projection wraps around.
    pub(super) fn of(matrix: Matrix, rect: Rect) -> Reach {
        let m = &matrix.0;
        let behind = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]
            .iter()
            .any(|[u, v]| {
                let x = f64::from(rect.x) + u * f64::from(rect.width);
                let y = f64::from(rect.y) + v * f64::from(rect.height);
                m[0][2] * x + m[1][2] * y + m[2][2] <= 0.0
            });
        if behind {
            return Reach::ALL;
        }
        let b = matrix.bounds(rect);
        let r = [b.x, b.y, b.x + b.width, b.y + b.height];
        if r.iter().all(|v| v.is_finite()) {
            Reach(Some(r))
        } else {
            Reach::ALL
        }
    }

    pub(super) fn union(self, other: Reach) -> Reach {
        match (self.0, other.0) {
            (Some(a), Some(b)) => Reach(Some([
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ])),
            (a, b) => Reach(a.or(b)),
        }
    }

    /// Grown by `d` px on each side.
    pub(super) fn grown(self, d: f32) -> Reach {
        Reach(
            self.0
                .map(|[x0, y0, x1, y1]| [x0 - d, y0 - d, x1 + d, y1 + d]),
        )
    }

    /// The part also in `other`.
    pub(super) fn meet(self, other: Reach) -> Reach {
        match (self.0, other.0) {
            (Some(a), Some(b)) => Reach(Some([
                a[0].max(b[0]),
                a[1].max(b[1]),
                a[2].min(b[2]),
                a[3].min(b[3]),
            ])),
            _ => Reach(None),
        }
    }

    /// The whole px of a target of `size` it touches, x, y, width and height: `None` if none.
    pub(super) fn pixels(self, [w, h]: [u32; 2]) -> Option<[u32; 4]> {
        let [x0, y0, x1, y1] = self.0?;
        let [w, h] = [w as f32, h as f32];
        let [x0, y0] = [x0.floor().clamp(0.0, w), y0.floor().clamp(0.0, h)];
        let [x1, y1] = [x1.ceil().clamp(0.0, w), y1.ceil().clamp(0.0, h)];
        (x1 > x0 && y1 > y0).then(|| [x0, y0, x1 - x0, y1 - y0].map(|v| v as u32))
    }

    /// Moved by `offset` px.
    pub(super) fn moved(self, [dx, dy]: [f32; 2]) -> Reach {
        Reach(
            self.0
                .map(|[x0, y0, x1, y1]| [x0 + dx, y0 + dy, x1 + dx, y1 + dy]),
        )
    }
}

/// The standard deviation a smaller copy is blurred by at the least, in its own px: under it the
/// copy's px would show in the blur.
const LEAST: f32 = 3.0;

/// The most of a layer's px that a px of a smaller copy is across.
const MOST: u32 = 16;

/// A smaller copy of a layer that a large blur goes through: where its first px starts on the
/// layer, how many of the layer's px each of its px is across and down, its width and height in
/// px, and the standard deviations it is blurred by along x and y, in its px.
struct Smaller {
    origin: [f32; 2],
    block: [f32; 2],
    extent: [f32; 2],
    sigma: [f32; 2],
}

impl Smaller {
    /// The copy for a blur of `sigma` px drawn within `clip` on a target of `size`, if the blur
    /// is large enough for one. Within `bounds`, its px divide them evenly, so that it mirrors
    /// at their edges as they do. Otherwise it has the blocks the pixels in `clip` are drawn
    /// back from, and those its blur reaches from them: a block past the target's edge is its
    /// edge px, as every block after it is, so the copy ends there.
    fn of(sigma: f32, clip: [f32; 4], bounds: Option<[f32; 4]>, size: [u32; 2]) -> Option<Smaller> {
        // Averaging blocks `b` px across and drawing them back linearly blur by
        // (3 b² − 1) / 12 px² between them; the copy's own blur is the rest, in its px.
        let within =
            |b: f32| (sigma * sigma - (3.0 * b * b - 1.0).max(0.0) / 12.0).max(0.0) / (b * b);
        let mut by = 1;
        while by < MOST && within((2 * by) as f32) >= LEAST * LEAST {
            by *= 2;
        }
        if by == 1 {
            return None;
        }
        let by = by as f32;
        if let Some([x, y, w, h]) = bounds {
            let along = |length: f32, size: u32| {
                let count = (length / by).ceil();
                (count >= 1.0 && count <= size as f32).then(|| (length / count, count))
            };
            let (across, width) = along(w, size[0])?;
            let (down, height) = along(h, size[1])?;
            return Some(Smaller {
                origin: [x, y],
                block: [across, down],
                extent: [width, height],
                sigma: [within(across).sqrt(), within(down).sqrt()],
            });
        }
        let inner = within(by).sqrt();
        let spread = (3.0 * inner).ceil();
        let [x, y, w, h] = Reach(Some(clip)).pixels(size)?.map(|v| v as f32);
        let along = |from: f32, to: f32, size: u32| {
            let size = size as f32;
            let first = ((from + 0.5) / by - 0.5).floor() - spread;
            let last = ((to - 0.5) / by - 0.5).floor() + 1.0 + spread;
            let first = first.max(-1.0);
            let extent = last.min((size / by).ceil()) - first + 1.0;
            (extent <= size).then_some((first * by, extent))
        };
        let (left, width) = along(x, x + w, size[0])?;
        let (top, height) = along(y, y + h, size[1])?;
        Some(Smaller {
            origin: [left, top],
            block: [by, by],
            extent: [width, height],
            sigma: [inner, inner],
        })
    }
}

#[derive(Default)]
pub(super) struct Plan {
    /// The targets' width and height, px.
    pub(super) size: [u32; 2],
    pub(super) transforms: Vec<u32>,
    pub(super) rects: Vec<u32>,
    pub(super) gradients: Vec<u32>,
    pub(super) segments: Vec<u32>,
    pub(super) glyphs: Vec<u32>,
    pub(super) layers: Vec<u32>,
    pub(super) steps: Vec<Step>,
    pub(super) targets: usize,
    pub(super) seen: Seen,
    /// The frame's paths, encoded as glyph outlines are (curve words, list words), which go
    /// after the atlas for this frame only; and where in `glyphs` each path instance has its
    /// first curve texel and list word, which are from the start of these.
    pub(super) paths: (Vec<u32>, Vec<u32>),
    pub(super) placed: Vec<usize>,
    pub(super) shadings: Vec<Shading>,
    /// Where what was drawn since the innermost layer that blurs began is.
    pub(super) reach: Reach,
    /// By target, where it may have anything: the steps that read it run only there. Anywhere
    /// until a layer's is known.
    pub(super) bounds: Vec<Reach>,
}

/// A shader drawn: its program, the words of its host block, its uniform block, and the frame's
/// images it reads by name.
pub(super) type Shading = (
    Rc<Program>,
    [f32; 16],
    Rc<[u8]>,
    Vec<(Rc<str>, u32)>,
    [&'static str; 2],
);

impl Plan {
    /// A new layer.
    pub(super) fn layer(&mut self) -> usize {
        self.targets += 1;
        self.bounds.resize(self.targets, Reach::ALL);
        self.targets - 1
    }

    /// Where target `t` may have anything.
    pub(super) fn bound(&self, t: usize) -> Reach {
        self.bounds.get(t).copied().unwrap_or(Reach::ALL)
    }

    /// Adds the words of a layer instance, or of a pass that takes their place: its index.
    pub(super) fn words(&mut self, words: impl IntoIterator<Item = f32>) -> u32 {
        let at = (self.layers.len() / LAYER) as u32;
        self.layers.extend(words.into_iter().map(f32::to_bits));
        at
    }

    /// `layer` composited onto `target`, all of it.
    pub(super) fn composite(&mut self, target: usize, layer: usize) {
        let at = self.whole();
        self.steps.push(Step::Composite { target, layer, at });
    }

    /// Adds `layer`'s colors to `target`'s, the alpha the most of the two.
    pub(super) fn add(&mut self, target: usize, layer: usize) {
        let at = self.whole();
        self.steps.push(Step::Add { target, layer, at });
    }

    /// A layer's words for all of it at full opacity.
    pub(super) fn whole(&mut self) -> u32 {
        let words = [1.0, 0.0, 0.0, 0.0, -1.0e7, -1.0e7, 2.0e7, 2.0e7];
        self.words(words.into_iter().chain([0.0; 12]))
    }

    /// A new layer of `layer` blurred by a Gaussian of `sigma` px: along x, then along y. Within
    /// `bounds` (x, y, width and height, whole px) if they are given, which it reads as if
    /// mirrored at their edges; otherwise `reach`, where the layer has anything, grows by the
    /// blur's three standard deviations. A large blur goes through a smaller copy of the layer,
    /// which costs as much whatever its size.
    pub(super) fn blurred(
        &mut self,
        mut layer: usize,
        sigma: f32,
        bounds: Option<[f32; 4]>,
        reach: &mut Reach,
    ) -> usize {
        if sigma > 0.0 {
            let clip = match bounds {
                Some([x, y, w, h]) => Reach(Some([x, y, x + w, y + h])),
                None => {
                    *reach = reach.grown((3.0 * sigma).ceil() + 1.0);
                    *reach
                }
            };
            let Some(clip) = clip.0 else {
                return layer;
            };
            let Some(smaller) = Smaller::of(sigma, clip, bounds, self.size) else {
                for [x, y] in [[1.0, 0.0], [0.0, 1.0]] {
                    let words = [sigma, x, y, 0.0].into_iter();
                    let words = words.chain(bounds.unwrap_or([0.0; 4])).chain([0.0; 12]);
                    layer = self.blur(layer, words, clip);
                }
                return layer;
            };
            // The copy is at the top left of its targets: made, blurred there, and drawn back.
            let Smaller {
                origin: [x, y],
                block: [across, down],
                extent: [w, h],
                sigma: [sx, sy],
            } = smaller;
            let copy = [x, y, across, down, w, h, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
            let all = [0.0, 0.0, w, h];
            // Bounds are the whole copy, which then mirrors at its own edges.
            let mirrored = bounds.map_or([0.0; 4], |_| all);
            let passes = [
                ([0.0, 0.0, 0.0, 1.0], [0.0; 4], all),
                ([sx, 1.0, 0.0, 0.0], mirrored, all),
                ([sy, 0.0, 1.0, 0.0], mirrored, all),
                ([0.0, 0.0, 0.0, 2.0], [0.0; 4], clip),
            ];
            for (pass, bounds, clip) in passes {
                let words = pass.into_iter().chain(bounds).chain(copy);
                layer = self.blur(layer, words, clip);
            }
        }
        layer
    }

    /// A new layer of one pass of a blur over `layer`, with its words, only within `clip`.
    fn blur(&mut self, layer: usize, words: impl Iterator<Item = f32>, clip: [f32; 4]) -> usize {
        let at = self.words(words);
        let target = self.layer();
        self.steps.push(Step::Blur {
            target,
            layer,
            at,
            clip,
        });
        target
    }

    /// A new layer of `layer` through `filters`, in the px `matrix` takes to the target's, where
    /// a blur is as much larger as the transform makes it and an offset turns with it. Blurs are
    /// within `bounds` if they are given. `reach` is where the layer has anything, and becomes
    /// where the new one may.
    pub(super) fn filtered(
        &mut self,
        mut layer: usize,
        filters: &[Filter],
        matrix: Matrix,
        bounds: Option<[f32; 4]>,
        reach: &mut Reach,
    ) -> usize {
        let scale = matrix.radius_scale() as f32;
        let [x, y, _] = matrix.0;
        for filter in filters {
            // The layers each filter makes have what is in it only where it may now reach.
            let first = self.targets;
            layer = match filter {
                Filter::Blur(sigma) => self.blurred(layer, sigma * scale, bounds, reach),
                Filter::Colors(matrices) => {
                    // Colors that make something of transparent show where nothing was.
                    if !keeps_transparent(matrices) {
                        *reach = Reach::ALL;
                    }
                    self.colored(layer, [0.0; 2], matrices, false)
                }
                Filter::Luminance => self.colored(layer, [0.0; 2], &[LUMINANCE], true),
                Filter::Aberration(offset) => {
                    let d = f64::from(*offset);
                    let [dx, dy] = [0, 1].map(|k| (x[k] * d) as f32);
                    *reach = reach.grown(dx.abs().max(dy.abs()).ceil());
                    self.effect(layer, ABERRATION, &[dx, dy])
                }
                Filter::Lens { k, frame } => {
                    *reach = Reach::ALL;
                    let Some(t) = self.transform(matrix) else {
                        continue;
                    };
                    let [x0, y0, w, h] = [frame.x, frame.y, frame.width, frame.height];
                    self.effect(layer, LENS, &[t as f32, k[0], k[1], x0, y0, w, h])
                }
                Filter::Grain {
                    seed,
                    amount,
                    size,
                    origin,
                } => {
                    *reach = Reach::ALL;
                    let Some(t) = self.transform(matrix) else {
                        continue;
                    };
                    let seed = f32::from_bits(*seed);
                    self.effect(
                        layer,
                        GRAIN,
                        &[t as f32, seed, *amount, *size, origin[0], origin[1]],
                    )
                }
                Filter::Radial { center, amount } => {
                    *reach = Reach::ALL;
                    let Some(t) = self.transform(matrix) else {
                        continue;
                    };
                    if *amount <= 0.0 {
                        continue;
                    }
                    // Passes of three taps, each twice as far apart as the last, 1, 2, 4… parts
                    // of 2ⁿ − 1 of the reach: together a triangle across it either way, its taps
                    // at most a px apart where the transform stretches it most.
                    let reach = f64::from(*amount) * x[0].hypot(x[1]).max(y[0].hypot(y[1]));
                    let mut passes = 1;
                    while f64::from((1u32 << passes) - 1) < reach && passes < 16 {
                        passes += 1;
                    }
                    let parts = ((1u32 << passes) - 1) as f32;
                    for k in 0..passes {
                        let part = (1u32 << k) as f32 / parts;
                        let words = [t as f32, center[0], center[1], *amount, part];
                        layer = self.effect(layer, RADIAL, &words);
                    }
                    layer
                }
                Filter::Motion([tx, ty]) => {
                    let (tx, ty) = (f64::from(*tx), f64::from(*ty));
                    let [dx, dy] = [0, 1].map(|k| x[k] * tx + y[k] * ty);
                    let length = dx.hypot(dy);
                    if length <= 0.0 {
                        continue;
                    }
                    // A tap a px or less apart, so that between them it reads as one smear.
                    let taps = (length.ceil() + 1.0).min(MOTION_TAPS) as f32;
                    *reach = reach.grown(length.ceil() as f32);
                    self.effect(layer, MOTION, &[taps, dx as f32, dy as f32])
                }
                Filter::Bloom {
                    radius,
                    intensity,
                    threshold,
                    knee,
                } => {
                    if *radius <= 0.0 || *intensity <= 0.0 {
                        continue;
                    }
                    let light = self.effect(layer, LIGHT, &[*threshold, *knee, *intensity]);
                    let halo = self.blurred(light, radius * scale / 2.0, bounds, reach);
                    let both = self.layer();
                    self.composite(both, layer);
                    self.add(both, halo);
                    both
                }
                Filter::Displacement {
                    seed,
                    frequency: [fx, fy],
                    scale,
                    octaves,
                    turbulence,
                    origin: [ox, oy],
                } => {
                    *reach = Reach::ALL;
                    let Some(t) = self.transform(matrix) else {
                        continue;
                    };
                    let [seed, octaves] = [f32::from_bits(*seed), *octaves as f32];
                    let turbulence = f32::from(u8::from(*turbulence));
                    let words = [
                        t as f32, seed, *fx, *fy, *scale, octaves, turbulence, *ox, *oy,
                    ];
                    self.effect(layer, DISPLACEMENT, &words)
                }
                Filter::Shadow {
                    offset: [dx, dy],
                    sigma,
                    color,
                } => {
                    let a = color[3];
                    if a <= 0.0 {
                        continue;
                    }
                    // The alpha in the color, moved and blurred, under the layer.
                    let [r, g, b] = [0, 1, 2].map(|k| color[k] / a);
                    let mut tint = [0.0; 20];
                    tint[15] = a;
                    tint[16..19].copy_from_slice(&[r, g, b]);
                    let (dx, dy) = (f64::from(*dx), f64::from(*dy));
                    let offset = [0, 1].map(|k| (x[k] * dx + y[k] * dy) as f32);
                    let shadow = self.colored(layer, offset, &[tint], false);
                    let mut moved = reach.moved(offset);
                    let shadow = self.blurred(shadow, sigma * scale, bounds, &mut moved);
                    *reach = reach.union(moved);
                    let both = self.layer();
                    self.composite(both, shadow);
                    self.composite(both, layer);
                    both
                }
            };
            for t in first..self.targets {
                self.bounds[t] = *reach;
            }
        }
        layer
    }

    /// A new layer of `layer` through one of Valle's effects, `kind`, with its parameters.
    pub(super) fn effect(&mut self, layer: usize, kind: f32, parameters: &[f32]) -> usize {
        let words = std::iter::once(kind).chain(parameters.iter().copied());
        let at = self.words(words.chain(std::iter::repeat(0.0)).take(LAYER));
        let target = self.layer();
        self.steps.push(Step::Effect { target, layer, at });
        target
    }

    /// A new layer of `layer` moved by `offset` px, its colors through `matrices`, its
    /// `premultiplied` ones if asked.
    pub(super) fn colored(
        &mut self,
        layer: usize,
        offset: [f32; 2],
        matrices: &[[f32; 20]],
        premultiplied: bool,
    ) -> usize {
        let count = matrices.len() as f32;
        let flag = f32::from(u8::from(premultiplied));
        let at = self.words(
            [count, offset[0], offset[1], flag]
                .into_iter()
                .chain([0.0; 16]),
        );
        for m in matrices {
            self.words(*m);
        }
        let target = self.layer();
        self.steps.push(Step::Colors { target, layer, at });
        target
    }

    /// Adds `m` and its inverse to the transforms: its index, or `None` if it has no inverse.
    pub(super) fn transform(&mut self, m: Matrix) -> Option<u32> {
        let inverse = m.inverse()?;
        let i = (self.transforms.len() / TRANSFORM) as u32;
        for c in m.0.into_iter().chain(inverse.0) {
            let column = [c[0], c[1], c[2], 0.0];
            self.transforms.extend(column.map(|v| (v as f32).to_bits()));
        }
        Some(i)
    }
}
