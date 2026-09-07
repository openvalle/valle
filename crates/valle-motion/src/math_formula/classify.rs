//! Parse → admit → layout → display validation. Never uses `LayoutOptions::default()`.

use ratex_layout::{LayoutOptions, layout, to_display_list};
use ratex_parser::parse;
use ratex_types::color::Color;
use ratex_types::display_item::DisplayList;
use ratex_types::math_style::MathStyle;

use super::admit::{AdmitError, AdmitPolicy, admit_display_list, admit_nodes, admit_source};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    Accepted,
    ParseError,
    PolicyIncludeGraphics,
    PolicyAutoNumber,
    PolicyLeqno,
    PolicyCjkEmoji,
    PolicyExtension,
    LeftoverEmpty,
    NonFinite,
    UnknownFont,
    Budget,
}

#[derive(Debug, Clone)]
pub struct ClassifiedFormula {
    pub source: String,
    pub class: Classification,
    pub detail: String,
    pub width: f64,
    pub height: f64,
    pub depth: f64,
    pub item_count: usize,
}

pub fn classify_formula(source: &str, display: bool, policy: &AdmitPolicy) -> ClassifiedFormula {
    match classify_inner(source, display, policy, Color::BLACK) {
        Ok(list) => ClassifiedFormula {
            source: source.to_string(),
            class: Classification::Accepted,
            detail: String::new(),
            width: list.width,
            height: list.height,
            depth: list.depth,
            item_count: list.items.len(),
        },
        Err(err) => ClassifiedFormula {
            source: source.to_string(),
            class: class_of(&err),
            detail: err.to_string(),
            width: 0.0,
            height: 0.0,
            depth: 0.0,
            item_count: 0,
        },
    }
}

/// Shared prepare entry: parse, admit, layout, validate. `display == false` uses
/// `MathStyle::Text`, never `LayoutOptions::default()`.
pub fn prepare_ratex_list(
    source: &str,
    display: bool,
    policy: &AdmitPolicy,
) -> Result<DisplayList, AdmitError> {
    prepare_ratex_list_with_color(source, display, policy, Color::BLACK)
}

pub fn prepare_ratex_list_with_color(
    source: &str,
    display: bool,
    policy: &AdmitPolicy,
    color: Color,
) -> Result<DisplayList, AdmitError> {
    classify_inner(source, display, policy, color)
}

fn classify_inner(
    source: &str,
    display: bool,
    policy: &AdmitPolicy,
    color: Color,
) -> Result<DisplayList, AdmitError> {
    admit_source(source, policy)?;
    let nodes = parse(source).map_err(|err| AdmitError::Parse {
        message: err.to_string(),
    })?;
    admit_nodes(&nodes, policy)?;
    let options = layout_options(display, color);
    let boxed = layout(&nodes, &options);
    let list = to_display_list(&boxed);
    admit_display_list(&list)?;
    Ok(list)
}

fn layout_options(display: bool, color: Color) -> LayoutOptions {
    LayoutOptions {
        style: if display {
            MathStyle::Display
        } else {
            MathStyle::Text
        },
        color,
        align_relation_spacing: None,
        leftright_delim_height: None,
        inter_glyph_kern_em: 0.0,
        explicit_size_multiplier: 1.0,
    }
}

fn class_of(err: &AdmitError) -> Classification {
    match err {
        AdmitError::Parse { .. } => Classification::ParseError,
        AdmitError::IncludeGraphics => Classification::PolicyIncludeGraphics,
        AdmitError::AutoNumber { .. } => Classification::PolicyAutoNumber,
        AdmitError::Leqno => Classification::PolicyLeqno,
        AdmitError::CjkOrEmoji { .. } => Classification::PolicyCjkEmoji,
        AdmitError::LeftoverEmpty { .. } => Classification::LeftoverEmpty,
        AdmitError::Extension { .. } => Classification::PolicyExtension,
        AdmitError::UnknownFont { .. } => Classification::UnknownFont,
        AdmitError::Budget { .. } => Classification::Budget,
        AdmitError::UnknownHtmlStyle { .. } => Classification::PolicyExtension,
        AdmitError::NonFinite { .. } => Classification::NonFinite,
    }
}
