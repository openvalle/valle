//! Preserve paint color tokens alongside Takumi's authoritative CSS geometry and cascade.
use crate::{MotionValue, StyleBinding, StyleValue};
use cssparser::{Delimiter, Parser, ParserInput, Token};
use valle_draw::program::AuthorColor;

#[derive(Clone, Debug, Default)]
pub(crate) struct PreciseCssColors(Vec<(String, bool, Vec<Vec<AuthorColor>>)>);

impl PreciseCssColors {
    pub(crate) fn new(styles: &[StyleBinding], values: &[MotionValue]) -> Self {
        let mut declarations = Vec::new();
        for style in styles {
            if !matches!(
                style.property.as_str(),
                "border"
                    | "border-color"
                    | "border-top"
                    | "border-right"
                    | "border-bottom"
                    | "border-left"
                    | "border-top-color"
                    | "border-right-color"
                    | "border-bottom-color"
                    | "border-left-color"
                    | "outline"
                    | "outline-color"
                    | "box-shadow"
                    | "text-shadow"
                    | "background"
                    | "background-image"
            ) {
                continue;
            }
            let value = match &style.value {
                StyleValue::Static { value } => Some(value),
                StyleValue::Expr { expr } => values.get(expr.0 as usize),
            };
            let Some(value) = value else { continue };
            let source = crate::css_token(value);
            let source = source.trim();
            let important = source.ends_with("!important");
            let source = source.strip_suffix("!important").unwrap_or(source).trim();
            let layers = if let MotionValue::Color(color) = value {
                vec![vec![*color]]
            } else {
                color_layers(source)
            };
            declarations.push((style.property.clone(), important, layers));
        }
        Self(declarations)
    }

    pub(crate) fn layers(&self, property: &str) -> Option<&[Vec<AuthorColor>]> {
        self.0
            .iter()
            .enumerate()
            .filter(|(_, (name, _, _))| {
                name == property
                    || property
                        .strip_suffix("-color")
                        .is_some_and(|base| name == base)
                    || property.starts_with("border-")
                        && (name == "border" || name == "border-color")
                    || property == "background-image" && name == "background"
            })
            .max_by_key(|(index, (_, important, _))| (*important, *index))
            .map(|(_, (_, _, layers))| layers.as_slice())
    }

    pub(crate) fn color(&self, property: &str, fallback: valle_draw::Rgba) -> Option<AuthorColor> {
        let colors = self.layers(property)?.first()?;
        let index = if colors.len() > 1 && property.starts_with("border-") {
            let side = match property {
                "border-right-color" => 1,
                "border-bottom-color" => 2,
                "border-left-color" => 3,
                _ => 0,
            };
            match colors.len() {
                2 => side % 2,
                3 if side == 3 => 1,
                _ => side.min(colors.len() - 1),
            }
        } else {
            0
        };
        colors
            .get(index)
            .copied()
            .filter(|color| matches_byte(*color, fallback))
    }
}

pub(crate) fn matches_byte(color: AuthorColor, byte: valle_draw::Rgba) -> bool {
    color.to_srgb8() == byte
}

fn color_layers(source: &str) -> Vec<Vec<AuthorColor>> {
    fn scan(input: &mut Parser<'_, '_>) -> Vec<AuthorColor> {
        let mut colors = Vec::new();
        while !input.is_exhausted() {
            let start = input.position();
            let Ok(token) = input.next().cloned() else {
                break;
            };
            if matches!(token, Token::Function(_)) {
                let nested = input
                    .parse_nested_block(|nested| {
                        Ok::<_, cssparser::ParseError<'_, ()>>(scan(nested))
                    })
                    .unwrap_or_default();
                if let Some(color) = AuthorColor::parse(input.slice_from(start)) {
                    colors.push(color);
                } else {
                    colors.extend(nested);
                }
            } else if matches!(token, Token::Ident(_) | Token::Hash(_) | Token::IDHash(_))
                && let Some(color) = AuthorColor::parse(input.slice_from(start))
            {
                colors.push(color);
            }
        }
        colors
    }
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut layers = Vec::new();
    while !parser.is_exhausted() {
        let colors = parser
            .parse_until_before(Delimiter::Comma, |p| {
                Ok::<_, cssparser::ParseError<'_, ()>>(scan(p))
            })
            .unwrap_or_default();
        layers.push(colors);
        if parser.expect_comma().is_err() {
            break;
        }
    }
    layers
}
