//! Context-aware ParseNode admission. Exhaustive over RaTeX `08cae05` variants.

use ratex_parser::parse_node::{ArrayTag, ParseNode, ProofBranch};
use ratex_types::display_item::{DisplayItem, DisplayList};

use super::fonts::{REJECTED_FALLBACK_FONT_NAMES, allowed_ratex_font_name};

/// Where a node sits after `ratex_parser::parse`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeContext {
    /// Top-level parse result or expression/group body.
    Expression,
    /// Argument that should have been consumed during parse (size, raw, url…).
    Intermediate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmitError {
    Parse {
        message: String,
    },
    IncludeGraphics,
    AutoNumber {
        env: String,
    },
    Leqno,
    CjkOrEmoji {
        detail: String,
    },
    LeftoverEmpty {
        variant: &'static str,
    },
    UnknownHtmlStyle {
        property: String,
    },
    Extension {
        capability: &'static str,
    },
    UnknownFont {
        name: String,
    },
    NonFinite {
        detail: String,
    },
    Budget {
        name: &'static str,
        actual: u64,
        limit: u64,
    },
}

impl std::fmt::Display for AdmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse { message } => write!(f, "parse error: {message}"),
            Self::IncludeGraphics => write!(f, "\\includegraphics is rejected; use <Image>"),
            Self::AutoNumber { env } => {
                write!(
                    f,
                    "auto-numbered environment `{env}` is not in motion-math-formula"
                )
            }
            Self::Leqno => write!(f, "leqno is not laid out; capability mismatch"),
            Self::CjkOrEmoji { detail } => write!(f, "CJK/emoji is rejected: {detail}"),
            Self::LeftoverEmpty { variant } => {
                write!(
                    f,
                    "leftover ParseNode `{variant}` would become an empty box"
                )
            }
            Self::UnknownHtmlStyle { property } => {
                write!(
                    f,
                    "\\htmlStyle property `{property}` is not in the v1 allowlist"
                )
            }
            Self::Extension { capability } => {
                write!(f, "requires optional capability `{capability}`")
            }
            Self::UnknownFont { name } => write!(f, "unknown or fallback formula font `{name}`"),
            Self::NonFinite { detail } => write!(f, "non-finite formula geometry: {detail}"),
            Self::Budget {
                name,
                actual,
                limit,
            } => write!(f, "budget `{name}` exceeded: {actual} > {limit}"),
        }
    }
}

impl std::error::Error for AdmitError {}

#[derive(Debug, Clone)]
pub struct AdmitPolicy {
    pub allow_mhchem: bool,
    pub allow_prooftree: bool,
    pub max_source_bytes: usize,
    pub max_nodes: usize,
}

impl Default for AdmitPolicy {
    fn default() -> Self {
        Self {
            allow_mhchem: false,
            allow_prooftree: false,
            max_source_bytes: 16 * 1024,
            max_nodes: 8_192,
        }
    }
}

const HTML_STYLE_ALLOW: &[&str] = &[
    "color",
    "font-size",
    "font-weight",
    "font-style",
    "background",
    "background-color",
    "text-decoration",
    "text-decoration-line",
];

const AUTONUM_ENVS: &[&str] = &[
    "align", "alignat", "equation", "gather", "eqnarray", "multline",
];

pub fn admit_source(source: &str, policy: &AdmitPolicy) -> Result<(), AdmitError> {
    if source.len() > policy.max_source_bytes {
        return Err(AdmitError::Budget {
            name: "source_bytes",
            actual: source.len() as u64,
            limit: policy.max_source_bytes as u64,
        });
    }
    if source.contains("\\includegraphics") {
        return Err(AdmitError::IncludeGraphics);
    }
    if source.contains("\\leqno") {
        return Err(AdmitError::Leqno);
    }
    for env in AUTONUM_ENVS {
        if has_unstarred_env(source, env) {
            return Err(AdmitError::AutoNumber {
                env: (*env).to_string(),
            });
        }
    }
    if !policy.allow_prooftree
        && (source.contains("\\begin{prooftree}") || source.contains("\\prooftree"))
    {
        return Err(AdmitError::Extension {
            capability: "motion-math-formula-prooftree",
        });
    }
    if !policy.allow_mhchem && (source.contains("\\ce{") || source.contains("\\pu{")) {
        return Err(AdmitError::Extension {
            capability: "motion-math-formula-mhchem",
        });
    }
    Ok(())
}

fn has_unstarred_env(source: &str, name: &str) -> bool {
    let needle = format!("\\begin{{{name}}}");
    source.contains(&needle)
}

pub fn admit_nodes(nodes: &[ParseNode], policy: &AdmitPolicy) -> Result<(), AdmitError> {
    let mut count = 0u64;
    for node in nodes {
        walk(node, NodeContext::Expression, policy, &mut count)?;
    }
    Ok(())
}

fn walk(
    node: &ParseNode,
    ctx: NodeContext,
    policy: &AdmitPolicy,
    count: &mut u64,
) -> Result<(), AdmitError> {
    *count += 1;
    if *count > policy.max_nodes as u64 {
        return Err(AdmitError::Budget {
            name: "parse_nodes",
            actual: *count,
            limit: policy.max_nodes as u64,
        });
    }

    match node {
        ParseNode::Atom { text, .. }
        | ParseNode::MathOrd { text, .. }
        | ParseNode::TextOrd { text, .. }
        | ParseNode::OpToken { text, .. }
        | ParseNode::SpacingNode { text, .. } => reject_cjk_or_emoji(text),

        ParseNode::AccentToken { .. } => leftover("AccentToken", ctx),
        ParseNode::ColorToken { .. } => leftover("ColorToken", ctx),
        ParseNode::Size { .. } => leftover("Size", ctx),
        ParseNode::LeftRightRight { .. } => leftover("LeftRightRight", ctx),
        ParseNode::Environment { .. } => leftover("Environment", ctx),
        ParseNode::Infix { .. } => leftover("Infix", ctx),
        ParseNode::Raw { .. } => leftover("Raw", ctx),
        ParseNode::Url { .. } => leftover("Url", ctx),
        ParseNode::CdLabel { .. } => leftover("CdLabel", ctx),
        ParseNode::CdLabelParent { .. } => leftover("CdLabelParent", ctx),
        // `\nonumber`/`\notag` are popped during parse (`environments.rs`). A
        // leftover here is never a legal parent-context case.
        ParseNode::NoNumber { .. } => leftover("NoNumber", ctx),

        ParseNode::IncludeGraphics { .. } => Err(AdmitError::IncludeGraphics),
        ParseNode::ProofTree { tree, .. } => {
            if !policy.allow_prooftree {
                return Err(AdmitError::Extension {
                    capability: "motion-math-formula-prooftree",
                });
            }
            walk_proof(tree, policy, count)
        }

        ParseNode::Internal { .. } => Ok(()),
        ParseNode::Cr { .. } => match ctx {
            NodeContext::Expression => Ok(()),
            NodeContext::Intermediate => leftover("Cr", ctx),
        },

        ParseNode::OrdGroup { body, .. }
        | ParseNode::OperatorName { body, .. }
        | ParseNode::Text { body, .. }
        | ParseNode::Color { body, .. }
        | ParseNode::Styling { body, .. }
        | ParseNode::Sizing { body, .. }
        | ParseNode::LeftRight { body, .. }
        | ParseNode::Phantom { body, .. }
        | ParseNode::MClass { body, .. }
        | ParseNode::HBox { body, .. }
        | ParseNode::Pmb { body, .. }
        | ParseNode::HtmlMathMl { html: body, .. } => {
            walk_all(body, NodeContext::Expression, policy, count)
        }

        ParseNode::SupSub { base, sup, sub, .. } => {
            if let Some(base) = base {
                walk(base, NodeContext::Expression, policy, count)?;
            }
            if let Some(sup) = sup {
                walk(sup, NodeContext::Expression, policy, count)?;
            }
            if let Some(sub) = sub {
                walk(sub, NodeContext::Expression, policy, count)?;
            }
            Ok(())
        }
        ParseNode::GenFrac { numer, denom, .. } => {
            walk(numer, NodeContext::Expression, policy, count)?;
            walk(denom, NodeContext::Expression, policy, count)
        }
        ParseNode::Sqrt { body, index, .. } => {
            walk(body, NodeContext::Expression, policy, count)?;
            if let Some(index) = index {
                walk(index, NodeContext::Expression, policy, count)?;
            }
            Ok(())
        }
        ParseNode::Accent { base, .. }
        | ParseNode::AccentUnder { base, .. }
        | ParseNode::HorizBrace { base, .. } => walk(base, NodeContext::Expression, policy, count),
        ParseNode::Op { body, .. } => {
            if let Some(body) = body {
                walk_all(body, NodeContext::Expression, policy, count)?;
            }
            Ok(())
        }
        ParseNode::Font { body, .. }
        | ParseNode::Overline { body, .. }
        | ParseNode::Underline { body, .. }
        | ParseNode::VPhantom { body, .. }
        | ParseNode::Smash { body, .. }
        | ParseNode::Enclose { body, .. }
        | ParseNode::Lap { body, .. }
        | ParseNode::RaiseBox { body, .. }
        | ParseNode::VCenter { body, .. } => walk(body, NodeContext::Expression, policy, count),
        ParseNode::DelimSizing { .. }
        | ParseNode::Middle { .. }
        | ParseNode::Rule { .. }
        | ParseNode::Kern { .. }
        | ParseNode::Verb { .. } => Ok(()),
        ParseNode::Array {
            body,
            tags,
            leqno,
            is_cd,
            ..
        } => {
            if leqno.unwrap_or(false) {
                return Err(AdmitError::Leqno);
            }
            let _ = is_cd;
            for row in body {
                walk_all(row, NodeContext::Expression, policy, count)?;
            }
            if let Some(tags) = tags {
                for tag in tags {
                    match tag {
                        ArrayTag::Auto(true) => {
                            return Err(AdmitError::AutoNumber {
                                env: "array-auto".into(),
                            });
                        }
                        ArrayTag::Auto(false) => {}
                        ArrayTag::Explicit(nodes) => {
                            walk_all(nodes, NodeContext::Expression, policy, count)?;
                        }
                    }
                }
            }
            Ok(())
        }
        ParseNode::Href { body, .. } => walk_all(body, NodeContext::Expression, policy, count),
        ParseNode::Html {
            attributes, body, ..
        } => {
            if let Some(style) = attributes.get("style") {
                admit_html_style(style)?;
            }
            for key in attributes.keys() {
                if key != "style" {
                    // class/id/data: keep body, ignore metadata — no extra properties.
                }
            }
            walk_all(body, NodeContext::Expression, policy, count)
        }
        ParseNode::MathChoice {
            display,
            text,
            script,
            scriptscript,
            ..
        } => {
            walk_all(display, NodeContext::Expression, policy, count)?;
            walk_all(text, NodeContext::Expression, policy, count)?;
            walk_all(script, NodeContext::Expression, policy, count)?;
            walk_all(scriptscript, NodeContext::Expression, policy, count)
        }
        ParseNode::XArrow { body, below, .. } => {
            walk(body, NodeContext::Expression, policy, count)?;
            if let Some(below) = below {
                walk(below, NodeContext::Expression, policy, count)?;
            }
            Ok(())
        }
        ParseNode::Tag { body, tag, .. } => {
            walk_all(body, NodeContext::Expression, policy, count)?;
            walk_all(tag, NodeContext::Expression, policy, count)
        }
        ParseNode::CdArrow {
            label_above,
            label_below,
            ..
        } => {
            if let Some(node) = label_above {
                walk(node, NodeContext::Expression, policy, count)?;
            }
            if let Some(node) = label_below {
                walk(node, NodeContext::Expression, policy, count)?;
            }
            Ok(())
        }
    }
}

fn walk_all(
    nodes: &[ParseNode],
    ctx: NodeContext,
    policy: &AdmitPolicy,
    count: &mut u64,
) -> Result<(), AdmitError> {
    for node in nodes {
        walk(node, ctx, policy, count)?;
    }
    Ok(())
}

fn walk_proof(
    branch: &ProofBranch,
    policy: &AdmitPolicy,
    count: &mut u64,
) -> Result<(), AdmitError> {
    walk_all(&branch.conclusion, NodeContext::Expression, policy, count)?;
    if let Some(label) = &branch.left_label {
        walk_all(label, NodeContext::Expression, policy, count)?;
    }
    if let Some(label) = &branch.right_label {
        walk_all(label, NodeContext::Expression, policy, count)?;
    }
    for premise in &branch.premises {
        walk_proof(premise, policy, count)?;
    }
    Ok(())
}

fn leftover(variant: &'static str, _ctx: NodeContext) -> Result<(), AdmitError> {
    Err(AdmitError::LeftoverEmpty { variant })
}

fn reject_cjk_or_emoji(text: &str) -> Result<(), AdmitError> {
    for ch in text.chars() {
        if is_cjk_or_emoji(ch) {
            return Err(AdmitError::CjkOrEmoji {
                detail: format!("U+{:04X}", ch as u32),
            });
        }
    }
    Ok(())
}

fn is_cjk_or_emoji(ch: char) -> bool {
    let cp = ch as u32;
    matches!(
        cp,
        0x3040..=0x30FF
            | 0x31F0..=0x31FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0xAC00..=0xD7AF
            | 0xFF01..=0xFF60
            | 0xFFE0..=0xFFEE
            | 0x1F000..=0x1FAFF
            | 0x2700..=0x27BF
            | 0x2600..=0x26FF
    )
}

fn admit_html_style(style: &str) -> Result<(), AdmitError> {
    for declaration in style.split(';') {
        let declaration = declaration.trim();
        if declaration.is_empty() {
            continue;
        }
        let Some((property, _)) = declaration.split_once(':') else {
            return Err(AdmitError::UnknownHtmlStyle {
                property: declaration.to_string(),
            });
        };
        let property = property.trim().to_ascii_lowercase();
        if !HTML_STYLE_ALLOW.contains(&property.as_str()) {
            return Err(AdmitError::UnknownHtmlStyle { property });
        }
    }
    Ok(())
}

pub fn admit_display_list(list: &DisplayList) -> Result<(), AdmitError> {
    check_finite(list.width, "width")?;
    check_finite(list.height, "height")?;
    check_finite(list.depth, "depth")?;
    for item in &list.items {
        match item {
            DisplayItem::GlyphPath {
                x,
                y,
                scale,
                font,
                color,
                ..
            } => {
                check_finite(*x, "glyph.x")?;
                check_finite(*y, "glyph.y")?;
                check_finite(*scale, "glyph.scale")?;
                check_color(color)?;
                if REJECTED_FALLBACK_FONT_NAMES.contains(&font.as_str()) {
                    return Err(AdmitError::CjkOrEmoji {
                        detail: font.clone(),
                    });
                }
                if !allowed_ratex_font_name(font) {
                    return Err(AdmitError::UnknownFont { name: font.clone() });
                }
            }
            DisplayItem::Line {
                x,
                y,
                width,
                thickness,
                color,
                ..
            } => {
                check_finite(*x, "line.x")?;
                check_finite(*y, "line.y")?;
                check_finite(*width, "line.width")?;
                check_finite(*thickness, "line.thickness")?;
                check_color(color)?;
            }
            DisplayItem::Rect {
                x,
                y,
                width,
                height,
                color,
            } => {
                check_finite(*x, "rect.x")?;
                check_finite(*y, "rect.y")?;
                check_finite(*width, "rect.width")?;
                check_finite(*height, "rect.height")?;
                check_color(color)?;
            }
            DisplayItem::Path {
                x,
                y,
                color,
                commands,
                ..
            } => {
                check_finite(*x, "path.x")?;
                check_finite(*y, "path.y")?;
                check_color(color)?;
                for command in commands {
                    admit_path_command(command)?;
                }
            }
        }
    }
    Ok(())
}

fn admit_path_command(command: &ratex_types::path_command::PathCommand) -> Result<(), AdmitError> {
    use ratex_types::path_command::PathCommand;
    match command {
        PathCommand::MoveTo { x, y } | PathCommand::LineTo { x, y } => {
            check_finite(*x, "path")?;
            check_finite(*y, "path")
        }
        PathCommand::QuadTo { x1, y1, x, y } => {
            check_finite(*x1, "path")?;
            check_finite(*y1, "path")?;
            check_finite(*x, "path")?;
            check_finite(*y, "path")
        }
        PathCommand::CubicTo {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        } => {
            check_finite(*x1, "path")?;
            check_finite(*y1, "path")?;
            check_finite(*x2, "path")?;
            check_finite(*y2, "path")?;
            check_finite(*x, "path")?;
            check_finite(*y, "path")
        }
        PathCommand::Close => Ok(()),
    }
}

fn check_color(color: &ratex_types::color::Color) -> Result<(), AdmitError> {
    for (name, value) in [
        ("r", color.r),
        ("g", color.g),
        ("b", color.b),
        ("a", color.a),
    ] {
        if !value.is_finite() {
            return Err(AdmitError::NonFinite {
                detail: format!("color.{name}"),
            });
        }
    }
    Ok(())
}

fn check_finite(value: f64, detail: &str) -> Result<(), AdmitError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(AdmitError::NonFinite {
            detail: detail.into(),
        })
    }
}
