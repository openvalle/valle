//! Frozen, backend-neutral static SVG commands. No filesystem or network resolvers are used.
use serde::{Deserialize, Serialize};
use takumi_core::{
    geometry::PathCommand,
    resources::{
        font::{FontResource, Fonts},
        image::{ImageSource, SvgGradient, SvgOp, SvgPaint, SvgSpreadMethod},
    },
    style::Color,
};

pub const MAX_SVG_BYTES: usize = 4 * 1024 * 1024;
const MAX_COMMANDS: usize = 65_536;
const MAX_RASTER_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SvgDocument {
    pub width: f32,
    pub height: f32,
    pub ops: Vec<Op>,
}

impl SvgDocument {
    /// Raster subtrees are retained by pictures even after their source command list is dropped.
    pub fn raster_bytes(&self) -> u64 {
        fn paint(p: &Paint) -> u64 {
            if let Paint::Pattern { ops, .. } = p {
                ops_bytes(ops)
            } else {
                0
            }
        }
        fn ops_bytes(ops: &[Op]) -> u64 {
            ops.iter()
                .map(|op| match op {
                    Op::Raster { rgba, .. } => rgba.len() as u64,
                    Op::Mask { ops, .. } => ops_bytes(ops),
                    Op::Draw { fill, stroke, .. } => {
                        fill.as_ref().map_or(0, |f| paint(&f.paint))
                            + stroke.as_ref().map_or(0, |s| paint(&s.paint))
                    }
                    _ => 0,
                })
                .sum()
        }
        ops_bytes(&self.ops)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Op {
    Transform {
        matrix: [f32; 6],
    },
    Clip {
        path: String,
        evenodd: bool,
    },
    Blend {
        mode: String,
    },
    Opacity {
        opacity: f32,
    },
    Mask {
        ops: Vec<Op>,
        luminance: bool,
    },
    Pop,
    Draw {
        path: String,
        fill: Option<Box<Fill>>,
        stroke: Option<Box<Stroke>>,
    },
    Raster {
        rgba: Vec<u8>,
        width: u32,
        height: u32,
        rect: [f32; 4],
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fill {
    pub paint: Paint,
    pub opacity: f32,
    pub evenodd: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stroke {
    pub paint: Paint,
    pub opacity: f32,
    pub width: f32,
    pub miter: f32,
    pub cap: String,
    pub join: String,
    pub dash: Option<(Vec<f32>, f32)>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Paint {
    Color {
        rgb: [u8; 3],
    },
    Linear {
        start: [f32; 2],
        end: [f32; 2],
        gradient: Gradient,
    },
    Radial {
        center: [f32; 2],
        radius: f32,
        focal: [f32; 2],
        gradient: Gradient,
    },
    Pattern {
        ops: Vec<Op>,
        matrix: [f32; 6],
        width: f32,
        height: f32,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Gradient {
    pub matrix: [f32; 6],
    pub spread: String,
    pub stops: Vec<Stop>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stop {
    pub offset: f32,
    pub rgb: [u8; 3],
    pub opacity: f32,
}

/// XML is detected structurally, not by a filename extension; encoded bitmaps return false.
pub fn is_svg(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|s| {
        s.trim_start_matches('\u{feff}')
            .trim_start()
            .starts_with('<')
    })
}

/// Intrinsic layout size can be probed before the host has resolved project fonts.
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    if bytes.len() > MAX_SVG_BYTES {
        return Err("SVG exceeds 4 MiB source budget".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "SVG must be UTF-8")?;
    roxmltree::Document::parse(text).map_err(|e| format!("invalid SVG XML: {e}"))?;
    let ImageSource::Svg(svg) = ImageSource::from_bytes(bytes).map_err(|e| e.to_string())? else {
        return Err("expected SVG".into());
    };
    let (w, h) = svg.dimensions();
    if !w.is_finite()
        || !h.is_finite()
        || w <= 0.
        || h <= 0.
        || w > 16384.
        || h > 16384.
        || w * h > 16_777_216.
    {
        return Err("SVG exceeds intrinsic pixel budget".into());
    }
    Ok((w.ceil() as u32, h.ceil() as u32))
}

/// Font payloads are frozen project resources, never host system fonts. SVG text is converted
/// to outlines here so native and browser executors see exactly the same glyph geometry.
pub fn compile(bytes: &[u8], font_bytes: &[&[u8]]) -> Result<SvgDocument, String> {
    if bytes.len() > MAX_SVG_BYTES {
        return Err("SVG exceeds 4 MiB source budget".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "SVG must be UTF-8")?;
    let xml = roxmltree::Document::parse(text).map_err(|e| format!("invalid SVG XML: {e}"))?;
    if xml.root_element().tag_name().name() != "svg" {
        return Err("image XML root must be svg".into());
    }
    if xml.descendants().count() > MAX_COMMANDS {
        return Err("SVG exceeds node budget".into());
    }
    let mut has_text = false;
    for node in xml.descendants().filter(|n| n.is_element()) {
        if node.ancestors().take(66).count() > 64 {
            return Err("SVG nesting exceeds 64 levels".into());
        }
        match node.tag_name().name() {
            "script" | "foreignObject" | "animate" | "animateTransform" | "animateMotion"
            | "set" => {
                return Err(format!(
                    "static SVG does not support <{}>",
                    node.tag_name().name()
                ));
            }
            "text" => has_text = true,
            "style" => {
                let css = node.text().unwrap_or("");
                if css.contains("@import") || css.contains("@font-face") {
                    return Err("SVG CSS imports/fonts must be frozen project resources".into());
                }
            }
            _ => {}
        }
        for attr in node.attributes() {
            let value = attr.value().trim();
            if attr.name() == "href"
                && value.starts_with("data:")
                && !["data:image/png;", "data:image/jpeg;", "data:image/webp;"]
                    .iter()
                    .any(|prefix| value.starts_with(prefix))
            {
                return Err("SVG embedded images must be PNG, JPEG or WebP".into());
            }
            if attr.name() == "href" && !value.starts_with('#') && !value.starts_with("data:") {
                return Err(format!(
                    "SVG external reference '{value}' must be embedded before admission"
                ));
            }
            // URL paint servers may only reference a definition in this document.
            for after in value.split("url(").skip(1) {
                if !after
                    .trim_start()
                    .trim_start_matches(['\'', '"'])
                    .starts_with('#')
                {
                    return Err("SVG paint URLs must refer to local definitions".into());
                }
            }
        }
    }
    if has_text && font_bytes.is_empty() {
        return Err("SVG text requires frozen project font bytes (or outlined paths)".into());
    }
    if has_text {
        let faces: Vec<_> = font_bytes
            .iter()
            .flat_map(|bytes| {
                (0..ttf_parser::fonts_in_collection(bytes).unwrap_or(1))
                    .filter_map(|index| ttf_parser::Face::parse(bytes, index).ok())
            })
            .collect();
        for node in xml
            .descendants()
            .filter(|n| n.is_text() && n.ancestors().any(|a| a.tag_name().name() == "text"))
        {
            for ch in node.text().unwrap_or("").chars().filter(|ch| {
                !ch.is_whitespace()
                    && !ch.is_control()
                    && !matches!(ch, '\u{200c}' | '\u{200d}' | '\u{fe0e}' | '\u{fe0f}')
            }) {
                if !faces.iter().any(|face| face.glyph_index(ch).is_some()) {
                    return Err(format!("SVG frozen fonts do not cover U+{:04X}", ch as u32));
                }
            }
        }
    }
    let mut fonts = Fonts::default();
    for bytes in font_bytes {
        fonts
            .register(FontResource::new(*bytes))
            .map_err(|e| format!("SVG font: {e}"))?;
    }
    let ImageSource::Svg(svg) = ImageSource::from_bytes(bytes).map_err(|e| e.to_string())? else {
        return Err("expected SVG image".into());
    };
    let (width, height) = svg.dimensions();
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
        || width > 16384.0
        || height > 16384.0
    {
        return Err("SVG intrinsic dimensions must be in (0,16384]".into());
    }
    dimensions(bytes)?;
    let ops = svg.vector_ops(1.0, Color([0, 0, 0, 255]), Some(&fonts.snapshot()));
    let mut budget = Budget {
        commands: 0,
        rasters: 0,
    };
    Ok(SvgDocument {
        width,
        height,
        ops: convert_ops(ops, &mut budget)?,
    })
}

struct Budget {
    commands: usize,
    rasters: usize,
}
fn path(commands: Vec<PathCommand>, budget: &mut Budget) -> Result<String, String> {
    use std::fmt::Write;
    budget.commands += commands.len();
    if budget.commands > MAX_COMMANDS {
        return Err("SVG exceeds path/operation budget".into());
    }
    let mut s = String::new();
    for c in commands {
        match c {
            PathCommand::MoveTo(p) => write!(s, "M{} {}", p.x, p.y),
            PathCommand::LineTo(p) => write!(s, "L{} {}", p.x, p.y),
            PathCommand::QuadTo(a, b) => write!(s, "Q{} {} {} {}", a.x, a.y, b.x, b.y),
            PathCommand::CubicTo(a, b, c) => {
                write!(s, "C{} {} {} {} {} {}", a.x, a.y, b.x, b.y, c.x, c.y)
            }
            PathCommand::Close => write!(s, "Z"),
        }
        .unwrap();
    }
    Ok(s)
}
fn gradient(g: SvgGradient) -> Gradient {
    Gradient {
        matrix: g.transform,
        spread: match g.spread {
            SvgSpreadMethod::Pad => "clamp",
            SvgSpreadMethod::Reflect => "mirror",
            SvgSpreadMethod::Repeat => "repeat",
        }
        .into(),
        stops: g
            .stops
            .into_iter()
            .map(|s| Stop {
                offset: s.offset,
                rgb: s.color,
                opacity: s.opacity,
            })
            .collect(),
    }
}
fn paint(p: SvgPaint, b: &mut Budget) -> Result<Paint, String> {
    Ok(match p {
        SvgPaint::Color(rgb) => Paint::Color { rgb },
        SvgPaint::Linear {
            start,
            end,
            gradient: g,
        } => Paint::Linear {
            start: [start.x, start.y],
            end: [end.x, end.y],
            gradient: gradient(g),
        },
        SvgPaint::Radial {
            center,
            radius,
            focal,
            gradient: g,
        } => Paint::Radial {
            center: [center.x, center.y],
            radius,
            focal: [focal.x, focal.y],
            gradient: gradient(g),
        },
        SvgPaint::Pattern {
            ops,
            transform,
            width,
            height,
        } => Paint::Pattern {
            ops: convert_ops(ops, b)?,
            matrix: transform,
            width,
            height,
        },
    })
}
fn convert_ops(ops: Vec<SvgOp>, b: &mut Budget) -> Result<Vec<Op>, String> {
    b.commands += ops.len();
    if b.commands > MAX_COMMANDS {
        return Err("SVG exceeds operation budget".into());
    }
    ops.into_iter()
        .map(|op| {
            Ok(match op {
                SvgOp::PushTransform(matrix) => Op::Transform { matrix },
                SvgOp::PushClip { path: p, evenodd } => Op::Clip {
                    path: path(p, b)?,
                    evenodd,
                },
                SvgOp::PushBlend(mode) => Op::Blend {
                    mode: format!("{mode:?}"),
                },
                SvgOp::PushOpacity(opacity) => Op::Opacity { opacity },
                SvgOp::PushMask { ops, luminance } => Op::Mask {
                    ops: convert_ops(ops, b)?,
                    luminance,
                },
                SvgOp::Pop => Op::Pop,
                SvgOp::Draw {
                    path: p,
                    fill,
                    stroke,
                } => Op::Draw {
                    path: path(p, b)?,
                    fill: fill
                        .map(|f| {
                            Ok::<_, String>(Box::new(Fill {
                                paint: paint(f.paint, b)?,
                                opacity: f.opacity,
                                evenodd: f.evenodd,
                            }))
                        })
                        .transpose()?,
                    stroke: stroke
                        .map(|s| {
                            Ok::<_, String>(Box::new(Stroke {
                                paint: paint(s.paint, b)?,
                                opacity: s.opacity,
                                width: s.width,
                                miter: s.miter_limit,
                                cap: format!("{:?}", s.cap),
                                join: format!("{:?}", s.join),
                                dash: s.dash,
                            }))
                        })
                        .transpose()?,
                },
                SvgOp::Raster {
                    rgba,
                    width,
                    height,
                    rect: (x, y, w, h),
                } => {
                    b.rasters += rgba.len();
                    if b.rasters > MAX_RASTER_BYTES {
                        return Err("SVG exceeds raster fallback budget".into());
                    }
                    Op::Raster {
                        rgba,
                        width,
                        height,
                        rect: [x, y, w, h],
                    }
                }
            })
        })
        .collect()
}
