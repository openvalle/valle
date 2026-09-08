//! COLR paint callbacks lower directly to the existing recording vocabulary.
//! No raster cache or second font/layout engine: glyph IDs and variations come from shaping.
use ttf_parser::{Face, GlyphId, OutlineBuilder, RgbaColor, Transform, colr};
use valle_draw::program::recording::{
    Affine, BlendMode, FillRule, LinearGradient, Paint, ProgramRecording, RecordCmd, SpreadMode,
    TwoCircleGradient,
};
use valle_draw::{PathRef, Point, Rect, Rgba};

pub(crate) fn is_v1(face: &Face<'_>) -> bool {
    face.raw_face()
        .table(ttf_parser::Tag::from_bytes(b"COLR"))
        .is_some_and(|table| table.starts_with(&[0, 1]))
}

pub(crate) fn paint_glyph(
    recording: &mut ProgramRecording,
    face: &Face<'_>,
    glyph: GlyphId,
    foreground: Rgba,
    bounds: Rect,
) -> Result<(), &'static str> {
    let mut painter = Painter {
        face,
        recording,
        commands: Vec::new(),
        outline: None,
        layers: Vec::new(),
        transforms: vec![Affine::IDENTITY],
        bounds,
        error: None,
        paints: 0,
    };
    let color = RgbaColor::new(foreground.r, foreground.g, foreground.b, foreground.a);
    let result = face.paint_color_glyph(glyph, 0, color, &mut painter);
    if result.is_none() {
        return Err("invalid COLR glyph");
    }
    if let Some(error) = painter.error {
        return Err(error);
    }
    if painter.paints == 0 {
        return Err("COLR glyph produced no paint");
    }
    if !painter.layers.is_empty() {
        return Err("unbalanced COLR layers");
    }
    painter.recording.cmds.extend(painter.commands);
    Ok(())
}

struct Painter<'a, 'b> {
    face: &'b Face<'a>,
    recording: &'b mut ProgramRecording,
    commands: Vec<RecordCmd>,
    outline: Option<PathRef>,
    layers: Vec<(usize, colr::CompositeMode)>,
    transforms: Vec<Affine>,
    bounds: Rect,
    error: Option<&'static str>,
    paints: usize,
}

fn rgba(c: RgbaColor) -> Rgba {
    Rgba {
        r: c.red,
        g: c.green,
        b: c.blue,
        a: c.alpha,
    }
}
fn spread(e: colr::GradientExtend) -> SpreadMode {
    match e {
        colr::GradientExtend::Pad => SpreadMode::Pad,
        colr::GradientExtend::Repeat => SpreadMode::Repeat,
        colr::GradientExtend::Reflect => SpreadMode::Reflect,
    }
}

impl Painter<'_, '_> {
    fn stops(&mut self, iter: impl Iterator<Item = colr::ColorStop>) -> valle_draw::draw::Span {
        let mut stops: Vec<_> = iter
            .map(|s| (f64::from(s.stop_offset), rgba(s.color)))
            .collect();
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.recording.intern_stops(&stops)
    }
    fn rect(&mut self, rect: Rect, fill: Paint) {
        let path = self
            .recording
            .begin_path()
            .move_to(Point::new(rect.x, rect.y))
            .line_to(Point::new(rect.x + rect.width, rect.y))
            .line_to(Point::new(rect.x + rect.width, rect.y + rect.height))
            .line_to(Point::new(rect.x, rect.y + rect.height))
            .close()
            .finish();
        self.commands.push(RecordCmd::Path {
            path,
            fill_rule: FillRule::NonZero,
            fill: Some(fill),
            stroke: None,
        });
    }
}

impl<'a> colr::Painter<'a> for Painter<'a, '_> {
    fn outline_glyph(&mut self, glyph: GlyphId) {
        let mut outline = Outline::default();
        if self.face.outline_glyph(glyph, &mut outline).is_none() {
            self.error = Some("COLR layer has no outline");
            self.outline = None;
            return;
        }
        let mut path = self.recording.begin_path();
        for command in outline.0 {
            path = match command {
                Segment::Move(p) => path.move_to(p),
                Segment::Line(p) => path.line_to(p),
                Segment::Quad(a, b) => path.quad_to(a, b),
                Segment::Cubic(a, b, c) => path.cubic_to(a, b, c),
                Segment::Close => path.close(),
            };
        }
        self.outline = Some(path.finish());
    }
    fn paint(&mut self, paint: colr::Paint<'a>) {
        let paint = match paint {
            colr::Paint::Solid(c) => Paint::Solid(rgba(c)),
            colr::Paint::LinearGradient(g) => {
                // COLR's third point describes an iso-color line, not the gradient endpoint.
                let (vx, vy) = (f64::from(g.x2 - g.x0), f64::from(g.y2 - g.y0));
                let (dx, dy) = (f64::from(g.x1 - g.x0), f64::from(g.y1 - g.y0));
                let norm = vx * vx + vy * vy;
                let (ex, ey) = if norm > 0.0 {
                    let t = (dx * vx + dy * vy) / norm;
                    (dx - t * vx, dy - t * vy)
                } else {
                    (dx, dy)
                };
                if ex * ex + ey * ey == 0.0 {
                    self.error = Some("degenerate COLR linear gradient");
                    return;
                }
                let stops = self.stops(g.stops(0, self.face.variation_coordinates()));
                Paint::Linear(LinearGradient {
                    start: Point::new(g.x0.into(), g.y0.into()),
                    end: Point::new(f64::from(g.x0) + ex, f64::from(g.y0) + ey),
                    stops,
                    spread: spread(g.extend),
                    alpha: 1.0,
                })
            }
            colr::Paint::RadialGradient(g) => {
                let stops = self.stops(g.stops(0, self.face.variation_coordinates()));
                Paint::TwoCircle(TwoCircleGradient {
                    start: Point::new(g.x0.into(), g.y0.into()),
                    start_radius: g.r0.into(),
                    end: Point::new(g.x1.into(), g.y1.into()),
                    end_radius: g.r1.into(),
                    stops,
                    spread: spread(g.extend),
                    alpha: 1.0,
                })
            }
            colr::Paint::SweepGradient(_) => {
                self.error = Some("COLR sweep gradient is not supported");
                return;
            }
        };
        self.paints += 1;
        // COLRv1 fills the active clip; the glyph's finite clip box bounds that fill.
        let Some(inverse) = self.transforms.last().unwrap().inverse() else {
            self.error = Some("singular COLR transform");
            return;
        };
        let b = self.bounds;
        let points = [
            (b.x, b.y),
            (b.x + b.width, b.y),
            (b.x, b.y + b.height),
            (b.x + b.width, b.y + b.height),
        ]
        .map(|(x, y)| inverse.apply(Point::new(x, y)));
        let x = points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
        let y = points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let right = points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
        let bottom = points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
        self.rect(
            Rect {
                x,
                y,
                width: right - x,
                height: bottom - y,
            },
            paint,
        );
    }
    fn push_clip(&mut self) {
        if let Some(path) = self.outline {
            self.commands.push(RecordCmd::BeginClipPath {
                path,
                fill_rule: FillRule::NonZero,
            });
        } else {
            self.error = Some("COLR clip has no outline");
            self.commands.push(RecordCmd::BeginGroup);
        }
    }
    fn push_clip_box(&mut self, b: colr::ClipBox) {
        self.commands.push(RecordCmd::BeginClipRect {
            rect: Rect {
                x: b.x_min.into(),
                y: b.y_min.into(),
                width: f64::from(b.x_max - b.x_min),
                height: f64::from(b.y_max - b.y_min),
            },
        });
    }
    fn pop_clip(&mut self) {
        self.commands.push(RecordCmd::End);
    }
    fn push_transform(&mut self, t: Transform) {
        let transform = Affine([
            t.a.into(),
            t.b.into(),
            t.c.into(),
            t.d.into(),
            t.e.into(),
            t.f.into(),
        ]);
        self.transforms
            .push(transform.then(*self.transforms.last().unwrap()));
        self.commands.push(RecordCmd::BeginTransform { transform });
    }
    fn pop_transform(&mut self) {
        self.transforms.pop();
        self.commands.push(RecordCmd::End);
    }
    fn push_layer(&mut self, mode: colr::CompositeMode) {
        let start = self.commands.len();
        match mode {
            colr::CompositeMode::SourceOver => self.commands.push(RecordCmd::BeginSaveLayer {
                bounds: None,
                alpha: 1.0,
            }),
            colr::CompositeMode::SoftLight => self.commands.push(RecordCmd::BeginBlend {
                mode: BlendMode::SoftLight,
            }),
            colr::CompositeMode::SourceIn => {
                // ttf-parser emits an outer SourceOver group, its backdrop, then SourceIn.
                // Reuse a structured alpha mask instead of extending backend blend modes.
                if let Some(&(outer, colr::CompositeMode::SourceOver)) = self.layers.last() {
                    self.commands[outer] = RecordCmd::BeginAlphaMask;
                    self.commands.insert(outer + 1, RecordCmd::BeginGroup);
                    self.commands.push(RecordCmd::End);
                    self.commands.push(RecordCmd::BeginGroup);
                } else {
                    self.error = Some("invalid COLR source-in nesting");
                    self.commands.push(RecordCmd::BeginGroup);
                }
            }
            _ => {
                self.error = Some("unsupported COLR composite mode");
                self.commands.push(RecordCmd::BeginGroup);
            }
        }
        self.layers.push((start, mode));
    }
    fn pop_layer(&mut self) {
        self.layers.pop();
        self.commands.push(RecordCmd::End);
    }
}

#[derive(Default)]
struct Outline(Vec<Segment>);
enum Segment {
    Move(Point),
    Line(Point),
    Quad(Point, Point),
    Cubic(Point, Point, Point),
    Close,
}
impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push(Segment::Move(Point::new(x.into(), y.into())));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push(Segment::Line(Point::new(x.into(), y.into())));
    }
    fn quad_to(&mut self, x: f32, y: f32, x1: f32, y1: f32) {
        self.0.push(Segment::Quad(
            Point::new(x.into(), y.into()),
            Point::new(x1.into(), y1.into()),
        ));
    }
    fn curve_to(&mut self, x: f32, y: f32, x1: f32, y1: f32, x2: f32, y2: f32) {
        self.0.push(Segment::Cubic(
            Point::new(x.into(), y.into()),
            Point::new(x1.into(), y1.into()),
            Point::new(x2.into(), y2.into()),
        ));
    }
    fn close(&mut self) {
        self.0.push(Segment::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FONT: &[u8] = include_bytes!("../../../assets/fonts/noto/Noto-COLRv1.ttf");
    #[test]
    fn every_bundled_noto_color_glyph_lowers_to_a_valid_program() {
        let face = Face::parse(FONT, 0).unwrap();
        assert!(is_v1(&face));
        let b = face.global_bounding_box();
        let bounds = Rect {
            x: b.x_min.into(),
            y: b.y_min.into(),
            width: f64::from(b.x_max) - f64::from(b.x_min),
            height: f64::from(b.y_max) - f64::from(b.y_min),
        };
        let mut count = 0;
        for id in 0..face.number_of_glyphs() {
            let glyph = GlyphId(id);
            if !face.is_color_glyph(glyph) {
                continue;
            }
            let mut recording = ProgramRecording::new();
            paint_glyph(
                &mut recording,
                &face,
                glyph,
                Rgba {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                bounds,
            )
            .unwrap_or_else(|e| panic!("glyph {id}: {e}"));
            valle_draw::program::compile_recording(bounds, &recording)
                .unwrap_or_else(|e| panic!("glyph {id}: {e}"));
            count += 1;
        }
        assert!(count > 3000, "only tested {count} glyphs");
    }
}

pub(crate) fn emit_run(
    recording: &mut ProgramRecording,
    run: &takumi_core::layout::inline::PositionedInlineRun,
    layout: takumi_core::geometry::ComputedLayout,
    shadow: Option<(f64, f64, Rgba)>,
    placements: Option<&[Option<Affine>]>,
) -> Result<bool, &'static str> {
    use takumi_core::style::Affine as TAffine;
    let shaped = &run.glyph_run;
    let Ok(mut face) = Face::parse(shaped.font_data(), shaped.font_index) else {
        return Ok(false);
    };
    if !is_v1(&face)
        || !shaped
            .glyphs
            .iter()
            .any(|g| face.is_color_glyph(GlyphId(g.id as u16)))
    {
        return Ok(false);
    }
    for (tag, value) in &shaped.variations {
        face.set_variation(ttf_parser::Tag::from_bytes(tag), *value);
    }
    let scale = f64::from(shaped.font_size) / f64::from(face.units_per_em());
    let offset = run.glyph_offset(layout);
    let t = run.transform(TAffine::IDENTITY);
    recording.push(RecordCmd::BeginTransform {
        transform: Affine([
            t.a.into(),
            t.b.into(),
            t.c.into(),
            t.d.into(),
            t.x.into(),
            t.y.into(),
        ]),
    });
    recording.push(RecordCmd::BeginOpacity {
        alpha: f64::from(shaped.brush.opacity).clamp(0.0, 1.0),
    });
    let b = face.global_bounding_box();
    let bounds = Rect {
        x: b.x_min.into(),
        y: b.y_min.into(),
        width: f64::from(b.x_max) - f64::from(b.x_min),
        height: f64::from(b.y_max) - f64::from(b.y_min),
    };
    let c = shaped.brush.color;
    let foreground = Rgba::new(c.0[0], c.0[1], c.0[2], c.0[3]);
    for (at, glyph) in shaped.glyphs.iter().enumerate() {
        let id = GlyphId(glyph.id as u16);
        if !face.is_color_glyph(id) {
            if face.glyph_bounding_box(id).is_some() {
                return Err("mixed plain and COLRv1 glyph run");
            }
            continue;
        }
        if let Some(placements) = placements {
            let Some(Some(transform)) = placements.get(at) else {
                continue;
            };
            recording.push(RecordCmd::BeginTransform {
                transform: *transform,
            });
        }
        recording.push(RecordCmd::BeginTransform {
            transform: Affine([
                scale,
                0.0,
                0.0,
                -scale,
                f64::from(offset.x + glyph.x) + shadow.map_or(0.0, |s| s.0),
                f64::from(offset.y + glyph.y) + shadow.map_or(0.0, |s| s.1),
            ]),
        });
        if shadow.is_some() {
            recording.push(RecordCmd::BeginAlphaMask);
            recording.push(RecordCmd::BeginGroup);
        }
        paint_glyph(recording, &face, id, foreground, bounds)?;
        if let Some((_, _, color)) = shadow {
            recording.push(RecordCmd::End);
            recording.push(RecordCmd::BeginGroup);
            let path = recording
                .begin_path()
                .move_to(Point::new(bounds.x, bounds.y))
                .line_to(Point::new(bounds.x + bounds.width, bounds.y))
                .line_to(Point::new(
                    bounds.x + bounds.width,
                    bounds.y + bounds.height,
                ))
                .line_to(Point::new(bounds.x, bounds.y + bounds.height))
                .close()
                .finish();
            recording.push(RecordCmd::Path {
                path,
                fill_rule: FillRule::NonZero,
                fill: Some(Paint::Solid(color)),
                stroke: None,
            });
            recording.push(RecordCmd::End);
            recording.push(RecordCmd::End);
        }
        recording.push(RecordCmd::End);
        if placements.is_some() {
            recording.push(RecordCmd::End);
        }
    }
    recording.push(RecordCmd::End);
    recording.push(RecordCmd::End);
    Ok(true)
}
