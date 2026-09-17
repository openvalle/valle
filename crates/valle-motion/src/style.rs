//! Shared CSS admission for authored styles, loaded artifacts, and evaluated frames.
//!
//! A successful backend parse alone is insufficient: unknown properties and a few list
//! grammars in Takumi are intentionally forgiving. Motion must never silently drop them.

use std::str::FromStr;

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, Delimiter, ParseError, Parser, ParserInput,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, Token, parse_important,
};
use takumi_core::style::{
    Angle, Filter, FromCssStr, Length, Style, StyleDeclaration, ToCss, Transform, Transforms,
};

mod background;
pub(crate) use background::gradient_background_source;
mod diagnostic;
pub use diagnostic::{StyleIssue, StyleIssueKind};
mod property;
pub mod tokens;
pub(crate) use property::layout_probe_value;
pub use property::{
    GeometryReuse, PropertyAdmissionError, PropertyImpact, PropertyLowering, PropertySpec,
    ValueLimit, property_spec,
};

/// Author CSS variables are a permanent boundary: shared values are `const`s in the file.
pub(crate) fn custom_property_issue(name: &str, value: &str) -> StyleIssue {
    StyleIssue::new(
        StyleIssueKind::UnsupportedProperty,
        name,
        value,
        "author CSS custom properties are not supported",
    )
    .with_suggestion("a `const` at the top of the file, or a style object literal")
}

/// Whether Motion admits this CSS property (each value is checked separately).
pub fn supports_property(name: &str) -> bool {
    let spec = property_spec(name);
    spec.lowering == PropertyLowering::Css && spec.admit().is_ok()
}

/// Parse exactly one property value, preventing declaration injection through a string value.
pub fn parse_property(name: &str, value: &str) -> Result<Style, StyleIssue> {
    if name.starts_with("--") {
        return Err(custom_property_issue(name, value));
    }
    let spec = property_spec(name);
    spec.admit()
        .map_err(|error| StyleIssue::admission(error, value))?;
    if spec.lowering != PropertyLowering::Css {
        return Err(StyleIssue::new(
            StyleIssueKind::UnsupportedProperty,
            name,
            value,
            "requires Motion lowering rather than a CSS declaration",
        ));
    }
    spec.check_value(value)?;
    let source = format!("{name}: {value}");
    let (style, count) = parse_block(&source)
        .map_err(|reason| StyleIssue::new(StyleIssueKind::InvalidValue, name, value, reason))?;
    if count != 1 {
        return Err(StyleIssue::new(
            StyleIssueKind::InvalidValue,
            name,
            value,
            "expected one CSS value, not a declaration list",
        ));
    }
    Ok(style)
}

/// Parse a declaration list without ignoring unknown properties or invalid trailing tokens.
pub fn parse_declarations(source: &str) -> Result<Style, String> {
    parse_block(source).map(|(style, _)| style)
}

/// Detect decoded variable functions, including escapes and nested math/color functions.
/// Arbitrary utilities must wait for authored variable resolution instead of silently
/// accepting a reference which the backend would discard as an invalid computed value.
pub fn contains_variable(source: &str) -> bool {
    fn scan(input: &mut Parser<'_, '_>) -> bool {
        while let Ok(token) = input.next() {
            if matches!(token, Token::Function(name) if name.eq_ignore_ascii_case("var")) {
                return true;
            }
            if matches!(
                token,
                Token::Function(_)
                    | Token::ParenthesisBlock
                    | Token::SquareBracketBlock
                    | Token::CurlyBracketBlock
            ) && input
                .parse_nested_block(|input| Ok::<_, ParseError<'_, ()>>(scan(input)))
                .unwrap_or(true)
            {
                return true;
            }
        }
        false
    }
    scan(&mut Parser::new(&mut ParserInput::new(source)))
}

/// Internal bindings are consumed by Motion before CSS layout. Keep this boundary shared
/// with artifact admission so a directly loaded artifact follows the compiler's rules.
pub fn is_motion_property(name: &str) -> bool {
    property_spec(name).lowering == PropertyLowering::Motion
}

fn parse_block(source: &str) -> Result<(Style, usize), String> {
    let mut input = ParserInput::new(source);
    let mut input = Parser::new(&mut input);
    let mut parser = StrictDeclarations;
    let mut result = Style::default();
    let mut count = 0;
    for declaration in RuleBodyParser::new(&mut input, &mut parser) {
        let (style, important) = declaration
            .map_err(|(error, context)| format!("invalid CSS `{context}`: {error:?}"))?;
        for declaration in style.declarations.iter() {
            result.push(declaration.clone(), important);
        }
        count += 1;
    }
    Ok((result, count))
}

struct StrictDeclarations;

impl<'i> DeclarationParser<'i> for StrictDeclarations {
    type Declaration = (Style, bool);
    type Error = String;

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _: &cssparser::ParserState,
    ) -> Result<Self::Declaration, ParseError<'i, Self::Error>> {
        let name = if name.starts_with("--") {
            name.to_string()
        } else {
            name.to_ascii_lowercase()
        };
        let spec = property_spec(&name);
        spec.admit()
            .map_err(|error| input.new_custom_error(error.to_string()))?;
        if spec.lowering != PropertyLowering::Css {
            return Err(input.new_custom_error(format!("`{name}` is not a CSS declaration")));
        }
        let value = input.parse_until_before(Delimiter::Bang, |input| {
            let start = input.position();
            while input.next_including_whitespace_and_comments().is_ok() {}
            Ok(input.slice_from(start).trim().to_owned())
        })?;
        let important = input.try_parse(parse_important).is_ok();
        input.expect_exhausted()?;
        if name.starts_with("--") {
            return Err(input.new_custom_error(custom_property_issue(&name, &value).to_string()));
        }
        if !finite_css_numbers(&value) {
            return Err(input.new_custom_error(format!("`{name}` requires finite CSS numbers")));
        }
        spec.check_value(&value)
            .map_err(|error| input.new_custom_error(error.to_string()))?;
        if contains_variable(&value) {
            return Err(input.new_custom_error(format!(
                "`{name}` requires CSS variables to be resolved during preparation"
            )));
        }
        strict_grid_value(&name, &value).map_err(|reason| input.new_custom_error(reason))?;
        // Takumi represents automatic self alignment by its initial (Normal/None)
        // value. The public CSS grammar omits `auto`; keep that adapter here.
        let transform_state = (name == "scale").then(|| scale_presence(&value));
        let keyword = css_keyword(&value);
        let backend_value = if matches!(name.as_str(), "align-self" | "justify-self")
            && value == "auto"
            || matches!(name.as_str(), "translate" | "scale") && keyword.as_deref() == Some("none")
        {
            "initial"
        } else {
            &value
        };
        let mut style = if name == "transform" && keyword.is_none() {
            transform_style(&value).map_err(|reason| input.new_custom_error(reason))?
        } else {
            Style::from_str(&format!("{name}: {backend_value}"))
                .map_err(|reason| input.new_custom_error(reason.to_string()))?
        };
        if style.declarations.is_empty() {
            return Err(input.new_custom_error(format!("unsupported `{name}: {value}`")));
        }
        if name == "translate" {
            let mut translated = Style::default();
            for declaration in style.declarations.iter() {
                if let StyleDeclaration::Translate(pair) = declaration {
                    if matches!(pair.x, Length::Auto) || matches!(pair.y, Length::Auto) {
                        return Err(
                            input.new_custom_error("translate requires lengths or none".to_owned())
                        );
                    }
                    let mut pair = *pair;
                    if one_css_component(&value) {
                        // SpacePair copies its first value. CSS translate's missing Y is zero.
                        pair.y = Length::Px(0.0);
                    }
                    translated.push(StyleDeclaration::Translate(pair), false);
                } else {
                    translated.push(declaration.clone(), false);
                }
            }
            style = translated;
        }
        if matches!(name.as_str(), "filter" | "backdrop-filter") {
            if value.is_empty() || contains_variable(&value) {
                return Err(input.new_custom_error(format!(
                    "`{name}` requires a resolved filter list or none"
                )));
            }
            validate_filter_values(&style).map_err(|reason| input.new_custom_error(reason))?;
        }
        if let Some((marker, state)) = transform_state {
            let marker = if important {
                marker.replacen("--tw-", "--tw-important-", 1)
            } else {
                marker.into()
            };
            style.push(
                StyleDeclaration::CustomProperty(marker, state.into()),
                important,
            );
        }
        Ok((style, important))
    }
}

/// Keep an ordered 2D list. Adapt the optional second translate/skew argument, which
/// the backend grammar currently requires, without flattening into independent properties.
fn transform_style(source: &str) -> Result<Style, String> {
    fn argument<'i>(input: &mut Parser<'i, '_>) -> Result<String, ParseError<'i, String>> {
        input.parse_until_before(Delimiter::Comma, |input| {
            let start = input.position();
            while input.next().is_ok() {}
            Ok(input.slice_from(start).trim().to_owned())
        })
    }
    fn length(source: &str) -> Result<Length, String> {
        if !one_css_component(source) {
            return Err("each transform length must be one CSS value".into());
        }
        let mut tokens = ParserInput::new(source);
        let mut input = Parser::new(&mut tokens);
        if input
            .try_parse(Parser::expect_number)
            .is_ok_and(|value| value != 0.0)
        {
            return Err("transform lengths require a unit except zero".into());
        }
        let value = Length::from_css_str(source).map_err(|reason| format!("{reason:?}"))?;
        if matches!(value, Length::Auto) {
            return Err("transform lengths cannot be auto".into());
        }
        Ok(value)
    }
    let mut source = ParserInput::new(source);
    let mut input = Parser::new(&mut source);
    let mut transforms = Vec::new();
    let parsed = (|| -> Result<(), ParseError<'_, String>> {
        while !input.is_exhausted() {
            let name = input.expect_function()?.to_ascii_lowercase();
            let transform = input.parse_nested_block(|input| {
                if matches!(
                    name.as_str(),
                    "translate" | "translatex" | "translatey" | "skew"
                ) {
                    let first = argument(input)?;
                    let second = if input.is_exhausted() {
                        None
                    } else {
                        input.expect_comma()?;
                        Some(argument(input)?)
                    };
                    input.expect_exhausted()?;
                    if name == "skew" {
                        if !one_css_component(&first)
                            || second
                                .as_ref()
                                .is_some_and(|value| !one_css_component(value))
                        {
                            return Err(
                                input.new_custom_error("each skew angle must be one CSS value")
                            );
                        }
                        let x = Angle::from_css_str(&first)
                            .map_err(|e| input.new_custom_error(format!("{e:?}")))?;
                        let y = Angle::from_css_str(second.as_deref().unwrap_or("0deg"))
                            .map_err(|e| input.new_custom_error(format!("{e:?}")))?;
                        return Ok(Transform::Skew(x, y));
                    }
                    if second.is_some() && name != "translate" {
                        return Err(input.new_custom_error("axis translation takes one length"));
                    }
                    let mut x = length(&first).map_err(|e| input.new_custom_error(e))?;
                    let mut y = length(second.as_deref().unwrap_or("0px"))
                        .map_err(|e| input.new_custom_error(e))?;
                    if name == "translatey" {
                        std::mem::swap(&mut x, &mut y);
                    }
                    Ok(Transform::Translate(x, y))
                } else {
                    let start = input.position();
                    while input.next().is_ok() {}
                    let source = format!("{name}({})", input.slice_from(start));
                    Transform::from_css_str(&source)
                        .map_err(|e| input.new_custom_error(format!("{e:?}")))
                }
            })?;
            transforms.push(transform);
        }
        Ok(())
    })();
    parsed.map_err(|reason| format!("invalid 2D transform: {reason:?}"))?;
    if transforms.is_empty() {
        return Err("transform requires a nonempty list or none".into());
    }
    let mut style = Style::default();
    style.push(
        StyleDeclaration::Transform(Some(Transforms(transforms.into_boxed_slice()))),
        false,
    );
    Ok(style)
}

/// Takumi stores scale as a value pair, losing `none` versus identity.
/// Private, non-inherited markers follow the same cascade as those properties.
/// The layout adapter reads them after cascade, without reinterpreting class strings.
pub(crate) const SCALE_STATE: &str = "--tw-motion-scale-state";
pub(crate) const SCALE_IMPORTANT_STATE: &str = "--tw-important-motion-scale-state";

pub(crate) fn scale_presence(value: &str) -> (&'static str, &'static str) {
    let state = match css_keyword(value).as_deref() {
        Some("none" | "initial" | "unset") => "none",
        Some("inherit") => "inherit",
        _ => "present",
    };
    (SCALE_STATE, state)
}

fn css_keyword(value: &str) -> Option<String> {
    let mut source = ParserInput::new(value);
    let mut input = Parser::new(&mut source);
    input
        .try_parse(|input| {
            let keyword = input.expect_ident_cloned()?;
            input.expect_exhausted()?;
            Ok::<_, cssparser::BasicParseError<'_>>(keyword.to_ascii_lowercase())
        })
        .ok()
}

fn one_css_component(value: &str) -> bool {
    let mut source = ParserInput::new(value);
    let mut input = Parser::new(&mut source);
    input.next().is_ok() && input.next().is_err()
}

fn finite_css_numbers(source: &str) -> bool {
    fn scan(input: &mut Parser<'_, '_>) -> bool {
        while let Ok(token) = input.next() {
            match token {
                Token::Number { value, .. }
                | Token::Dimension { value, .. }
                | Token::Percentage {
                    unit_value: value, ..
                } if !value.is_finite() => return false,
                Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock => {
                    if !input
                        .parse_nested_block(|input| Ok::<_, ParseError<'_, ()>>(scan(input)))
                        .unwrap_or(false)
                    {
                        return false;
                    }
                }
                _ => {}
            }
        }
        true
    }
    scan(&mut Parser::new(&mut ParserInput::new(source)))
}

// The backend accepts percentages/auto for blur and negative numeric filter amounts.
// Check the typed values here so source, loaded artifacts and concrete frames agree.
fn validate_filter_values(style: &Style) -> Result<(), String> {
    for declaration in style.declarations.iter() {
        let (StyleDeclaration::Filter(filters) | StyleDeclaration::BackdropFilter(filters)) =
            declaration
        else {
            continue;
        };
        for filter in filters {
            match filter {
                Filter::Blur(length) => filter_length(*length, true)?,
                Filter::DropShadow(shadow) => {
                    filter_length(shadow.offset_x, false)?;
                    filter_length(shadow.offset_y, false)?;
                    filter_length(shadow.blur_radius, true)?;
                }
                Filter::Brightness(value)
                | Filter::Contrast(value)
                | Filter::Grayscale(value)
                | Filter::Saturate(value)
                | Filter::Invert(value)
                | Filter::Sepia(value)
                | Filter::Opacity(value) => {
                    if !value.0.is_finite() || value.0 < 0.0 {
                        return Err("filter amount must be finite and >= 0".into());
                    }
                }
                Filter::HueRotate(angle) if (**angle).is_finite() => {}
                _ => return Err("unsupported filter operation or non-finite angle".into()),
            }
        }
    }
    Ok(())
}

fn filter_length(length: Length, non_negative: bool) -> Result<(), String> {
    // Serialized parsed calc terms expose units without assuming a viewport/font size.
    // Mixed-unit calc sign is checked again against the real sizing context in emission.
    let source = length.to_css_string();
    let mut parser_input = ParserInput::new(&source);
    let mut input = Parser::new(&mut parser_input);
    fn check(input: &mut Parser<'_, '_>, non_negative: bool) -> Result<(), String> {
        while let Ok(token) = input.next() {
            match token {
                Token::Number { value, .. } | Token::Dimension { value, .. }
                    if value.is_finite() && (!non_negative || *value >= 0.0) => {}
                Token::Function(name) if name.eq_ignore_ascii_case("calc") => {
                    input
                        .parse_nested_block(|input| {
                            check(input, false).map_err(|error| input.new_custom_error(error))
                        })
                        .map_err(|_: ParseError<'_, String>| {
                            "invalid filter length math".to_owned()
                        })?;
                }
                Token::Delim('+' | '-' | '*' | '/') => {}
                _ => {
                    return Err("filter blur/offset requires a finite length; blur must be >= 0 and percentages are not allowed".into());
                }
            }
        }
        Ok(())
    }
    check(&mut input, non_negative)
}

impl<'i> AtRuleParser<'i> for StrictDeclarations {
    type Prelude = ();
    type AtRule = (Style, bool);
    type Error = String;
}

impl<'i> QualifiedRuleParser<'i> for StrictDeclarations {
    type Prelude = ();
    type QualifiedRule = (Style, bool);
    type Error = String;
}

impl<'i> RuleBodyItemParser<'i, (Style, bool), String> for StrictDeclarations {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}

// Takumi 0.23.1's outer grid-list parsers consume a failed element and then succeed.
// Isolate each complete CSS token, then require it to produce exactly one track component.
fn strict_grid_value(name: &str, value: &str) -> Result<(), String> {
    let template = matches!(name, "grid-template-columns" | "grid-template-rows");
    if !template && !matches!(name, "grid-auto-columns" | "grid-auto-rows") {
        return Ok(());
    }
    let mut source = ParserInput::new(value);
    let mut input = Parser::new(&mut source);
    if input
        .try_parse(|input| {
            let keyword = input.expect_ident()?;
            if matches!(
                keyword.to_ascii_lowercase().as_str(),
                "initial" | "inherit" | "unset" | "revert" | "revert-layer"
            ) || (template && keyword.eq_ignore_ascii_case("none"))
            {
                input.expect_exhausted().map_err(ParseError::<String>::from)
            } else {
                Err(input.new_error_for_next_token())
            }
        })
        .is_ok()
    {
        return Ok(());
    }
    let mut count = 0;
    while !input.is_exhausted() {
        let start = input.position();
        let token = input.next().map_err(|error| format!("{error:?}"))?;
        if matches!(token, Token::Function(_) | Token::SquareBracketBlock) {
            input
                .parse_nested_block(|input| {
                    while input.next().is_ok() {}
                    Ok::<_, ParseError<'_, String>>(())
                })
                .map_err(|error| format!("{error:?}"))?;
        }
        let component = input.slice_from(start);
        let parsed = Style::from_str(&format!("{name}: {component}")).is_ok_and(|style| {
            style
                .declarations
                .iter()
                .any(|declaration| match declaration {
                    StyleDeclaration::GridTemplateColumns(Some(parts))
                    | StyleDeclaration::GridTemplateRows(Some(parts)) => parts.len() == 1,
                    StyleDeclaration::GridAutoColumns(Some(parts))
                    | StyleDeclaration::GridAutoRows(Some(parts)) => parts.len() == 1,
                    _ => false,
                })
        });
        if !parsed {
            return Err(format!("unsupported or invalid `{name}: {value}`"));
        }
        count += 1;
    }
    if count == 0 {
        return Err(format!("`{name}` requires a nonempty track list"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_lists_consume_all_tokens() {
        for value in [
            "subgrid",
            "nonsense",
            "1fr nonsense",
            "repeat(2, 1fr nonsense)",
            "repeat(2, 1fr) nonsense",
            "[a 2] 1fr",
            "",
        ] {
            assert!(
                parse_property("grid-template-columns", value).is_err(),
                "{value}"
            );
        }
        for value in [
            "none",
            "initial",
            "1fr 2fr",
            "[start] 1fr [end]",
            "repeat(2, minmax(0px, 1fr))",
            "repeat(2, [a] 1fr [b])",
        ] {
            assert!(
                parse_property("grid-template-columns", value).is_ok(),
                "{value}"
            );
        }
        assert!(parse_property("grid-auto-rows", "10px nonsense").is_err());
        assert!(parse_property("grid-auto-rows", "10px 1fr").is_ok());
    }

    #[test]
    fn declarations_do_not_ignore_unknowns_or_allow_value_injection() {
        assert!(parse_declarations("width: 20px; made-up: 1").is_err());
        assert!(parse_property("width", "20px; height: 40px").is_err());
        assert!(parse_property("width", "20px rubbish").is_err());
        assert!(parse_declarations("width: calc(100% - 20px); color: red !important").is_ok());
    }
}
