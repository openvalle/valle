//! RaTeX ProgramRecording → Valle GlyphRun/Path. Shared native/wasm path.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use ratex_types::color::Color as RatexColor;
use ratex_types::display_item::{DisplayItem, DisplayList as RatexList};
use ratex_types::path_command::PathCommand;
use valle_draw::Rgba;
use valle_draw::draw::{Cap, Join};
use valle_draw::geom::{Point, Rect};
use valle_draw::program::recording::{
    FillRule, FontFace, Glyph, Paint, ProgramRecording, RecordCmd, Stroke,
};

use super::admit::{AdmitError, AdmitPolicy};

use super::registry::FormulaFontRegistry;

/// Unfilled RaTeX paths use 1.5 **post-em** logical pixels, not 1.5em.
pub const UNFILLED_PATH_STROKE_PX: f64 = 1.5;

/// Number of times RaTeX parse/layout actually ran (cache misses).
pub static FORMULA_PREPARE_RUNS: AtomicU64 = AtomicU64::new(0);

type CacheKey = (String, bool, u64, u32);

fn fragment_cache() -> &'static Mutex<HashMap<CacheKey, FormulaFragment>> {
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, FormulaFragment>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug, Clone)]
pub struct FormulaFragment {
    pub list: ProgramRecording,
    pub width: f64,
    pub height: f64,
    pub depth: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct FormulaStyle {
    pub font_size: f64,
    pub color: Rgba,
}

impl Default for FormulaStyle {
    fn default() -> Self {
        Self {
            font_size: 32.0,
            color: Rgba::rgb(0, 0, 0),
        }
    }
}

pub fn emit_formula(
    source: &str,
    display: bool,
    style: FormulaStyle,
    registry: &FormulaFontRegistry,
    policy: &AdmitPolicy,
) -> Result<FormulaFragment, AdmitError> {
    if !style.font_size.is_finite() || style.font_size <= 0.0 {
        return Err(AdmitError::NonFinite {
            detail: "fontSize".into(),
        });
    }
    let key = (
        source.to_string(),
        display,
        style.font_size.to_bits(),
        u32::from_be_bytes([style.color.r, style.color.g, style.color.b, style.color.a]),
    );
    if let Ok(cache) = fragment_cache().lock() {
        if let Some(hit) = cache.get(&key) {
            return Ok(hit.clone());
        }
    }
    let ink = ratex_types::color::Color::new(
        style.color.r as f32 / 255.0,
        style.color.g as f32 / 255.0,
        style.color.b as f32 / 255.0,
        style.color.a as f32 / 255.0,
    );
    let ratex = super::prepare_ratex_list_with_color(source, display, policy, ink)?;
    FORMULA_PREPARE_RUNS.fetch_add(1, Ordering::Relaxed);
    let fragment = emit_ratex_list(&ratex, style.font_size, registry)?;
    if let Ok(mut cache) = fragment_cache().lock() {
        cache.insert(key, fragment.clone());
    }
    Ok(fragment)
}

pub fn emit_ratex_list(
    ratex: &RatexList,
    font_size: f64,
    registry: &FormulaFontRegistry,
) -> Result<FormulaFragment, AdmitError> {
    let mut list = ProgramRecording::new();
    let mut run: Vec<PendingGlyph> = Vec::new();

    for item in &ratex.items {
        match item {
            DisplayItem::GlyphPath {
                x,
                y,
                scale,
                font,
                char_code,
                color,
            } => {
                let size = finite(*scale, "glyph.scale")? * font_size;
                let paint = ratex_color_to_rgba(color)?;
                let gid = registry.glyph_id(font, *char_code)?;
                let face = registry.font_face(font, size)?;
                let glyph = PendingGlyph {
                    face,
                    paint,
                    glyph: Glyph {
                        id: gid,
                        x: finite(*x, "glyph.x")? * font_size,
                        y: finite(*y, "glyph.y")? * font_size,
                    },
                };
                if let Some(last) = run.last() {
                    if !last.compatible(&glyph) {
                        flush_run(&mut list, &mut run)?;
                    }
                }
                run.push(glyph);
            }
            other => {
                flush_run(&mut list, &mut run)?;
                emit_non_glyph(&mut list, other, font_size)?;
            }
        }
    }
    flush_run(&mut list, &mut run)?;
    list.validate().map_err(|err| AdmitError::NonFinite {
        detail: format!("display validate: {err}"),
    })?;
    Ok(FormulaFragment {
        list,
        width: finite(ratex.width, "width")? * font_size,
        height: finite(ratex.height, "height")? * font_size,
        depth: finite(ratex.depth, "depth")? * font_size,
    })
}

struct PendingGlyph {
    face: FontFace,
    paint: Rgba,
    glyph: Glyph,
}

impl PendingGlyph {
    fn compatible(&self, other: &Self) -> bool {
        self.face == other.face && self.paint == other.paint
    }
}

fn flush_run(list: &mut ProgramRecording, run: &mut Vec<PendingGlyph>) -> Result<(), AdmitError> {
    if run.is_empty() {
        return Ok(());
    }
    let face = run[0].face.clone();
    let paint = run[0].paint;
    let glyphs: Vec<Glyph> = run.drain(..).map(|item| item.glyph).collect();
    let font = list.intern_font(face);
    let span = list.intern_glyphs(&glyphs);
    list.push(RecordCmd::GlyphRun {
        font,
        glyphs: span,
        paint: Paint::Solid(paint),
        stroke: None,
        source: None,
    });
    Ok(())
}

fn emit_non_glyph(
    list: &mut ProgramRecording,
    item: &DisplayItem,
    font_size: f64,
) -> Result<(), AdmitError> {
    match item {
        DisplayItem::GlyphPath { .. } => unreachable!(),
        DisplayItem::Line {
            x,
            y,
            width,
            thickness,
            color,
            dashed,
        } => {
            let paint = ratex_color_to_rgba(color)?;
            let x = finite(*x, "line.x")? * font_size;
            let y = finite(*y, "line.y")? * font_size;
            let width = finite(*width, "line.width")? * font_size;
            let thickness = finite(*thickness, "line.thickness")? * font_size;
            if *dashed {
                emit_dashed_line(list, x, y, width, thickness, paint)?;
            } else {
                emit_rect(
                    list,
                    Rect {
                        x,
                        y: y - thickness / 2.0,
                        width,
                        height: thickness,
                    },
                    paint,
                    None,
                )?;
            }
        }
        DisplayItem::Rect {
            x,
            y,
            width,
            height,
            color,
        } => {
            let paint = ratex_color_to_rgba(color)?;
            emit_rect(
                list,
                Rect {
                    x: finite(*x, "rect.x")? * font_size,
                    y: finite(*y, "rect.y")? * font_size,
                    width: finite(*width, "rect.width")? * font_size,
                    height: finite(*height, "rect.height")? * font_size,
                },
                paint,
                None,
            )?;
        }
        DisplayItem::Path {
            x,
            y,
            commands,
            fill,
            color,
        } => {
            let paint = ratex_color_to_rgba(color)?;
            let origin_x = finite(*x, "path.x")? * font_size;
            let origin_y = finite(*y, "path.y")? * font_size;
            let mut builder = list.begin_path();
            for command in commands {
                builder = push_path_command(builder, command, origin_x, origin_y, font_size)?;
            }
            let path = builder.finish();
            if *fill {
                list.push(RecordCmd::Path {
                    path,
                    fill_rule: FillRule::NonZero,
                    fill: Some(Paint::Solid(paint)),
                    stroke: None,
                });
            } else {
                list.push(RecordCmd::Path {
                    path,
                    fill_rule: FillRule::NonZero,
                    fill: None,
                    stroke: Some(Stroke {
                        paint: Paint::Solid(paint),
                        width: UNFILLED_PATH_STROKE_PX,
                        dash: None,
                        dash_offset: 0.0,
                        cap: Cap::Round,
                        join: Join::Round,
                        miter_limit: 4.0,
                    }),
                });
            }
        }
    }
    Ok(())
}

fn push_path_command<'a>(
    builder: valle_draw::draw::PathBuilder<'a, ProgramRecording>,
    command: &PathCommand,
    origin_x: f64,
    origin_y: f64,
    font_size: f64,
) -> Result<valle_draw::draw::PathBuilder<'a, ProgramRecording>, AdmitError> {
    let map = |px: f64, py: f64| -> Result<Point, AdmitError> {
        Ok(Point::new(
            origin_x + finite(px, "path")? * font_size,
            origin_y + finite(py, "path")? * font_size,
        ))
    };
    Ok(match command {
        PathCommand::MoveTo { x, y } => builder.move_to(map(*x, *y)?),
        PathCommand::LineTo { x, y } => builder.line_to(map(*x, *y)?),
        PathCommand::QuadTo { x1, y1, x, y } => builder.quad_to(map(*x1, *y1)?, map(*x, *y)?),
        PathCommand::CubicTo {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        } => builder.cubic_to(map(*x1, *y1)?, map(*x2, *y2)?, map(*x, *y)?),
        PathCommand::Close => builder.close(),
    })
}

fn emit_dashed_line(
    list: &mut ProgramRecording,
    x: f64,
    y: f64,
    width: f64,
    thickness: f64,
    paint: Rgba,
) -> Result<(), AdmitError> {
    let dash = (3.0 * thickness).max(0.5);
    let gap = dash;
    let period = dash + gap;
    let top = y - thickness / 2.0;
    let mut cursor = x;
    let end = x + width;
    while cursor < end {
        let seg = dash.min(end - cursor);
        emit_rect(
            list,
            Rect {
                x: cursor,
                y: top,
                width: seg,
                height: thickness,
            },
            paint,
            None,
        )?;
        cursor += period;
    }
    Ok(())
}

fn emit_rect(
    list: &mut ProgramRecording,
    rect: Rect,
    paint: Rgba,
    stroke: Option<Stroke>,
) -> Result<(), AdmitError> {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
    {
        return Err(AdmitError::NonFinite {
            detail: "rect".into(),
        });
    }
    let path = list.begin_path().rect(rect).finish();
    list.push(RecordCmd::Path {
        path,
        fill_rule: FillRule::NonZero,
        fill: Some(Paint::Solid(paint)),
        stroke,
    });
    Ok(())
}

pub fn ratex_color_to_rgba(color: &RatexColor) -> Result<Rgba, AdmitError> {
    Ok(Rgba::new(
        quantize_channel(color.r, "r")?,
        quantize_channel(color.g, "g")?,
        quantize_channel(color.b, "b")?,
        quantize_channel(color.a, "a")?,
    ))
}

fn quantize_channel(value: f32, name: &str) -> Result<u8, AdmitError> {
    if !value.is_finite() {
        return Err(AdmitError::NonFinite {
            detail: format!("color.{name}"),
        });
    }
    let value = if value == 0.0 { 0.0 } else { value };
    let clamped = value.clamp(0.0, 1.0);
    Ok((clamped * 255.0 + 0.5).floor() as u8)
}

fn finite(value: f64, detail: &str) -> Result<f64, AdmitError> {
    if !value.is_finite() {
        return Err(AdmitError::NonFinite {
            detail: detail.into(),
        });
    }
    Ok(if value == 0.0 { 0.0 } else { value })
}
