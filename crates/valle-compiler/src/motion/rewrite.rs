//! Narrow, source-preserving edits for directly authored Motion declarations.
//! OXC locates the current bytes; unsupported indirection is left to the code editor.

use oxc::allocator::Allocator;
use oxc::ast::ast::{
    Argument, Declaration, Expression, ObjectExpression, ObjectProperty, ObjectPropertyKind,
    Statement,
};
use oxc::parser::Parser;
use oxc::span::{GetSpan, SourceType, Span};
use serde::{Deserialize, Serialize};

use super::strip_parens;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionSourceEdit {
    pub source: String,
    pub target: MotionSourceEditTarget,
    pub value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum MotionSourceEditTarget {
    Composition { field: String },
    PropDefault { name: String },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionSourceEditResult {
    pub source: String,
    /// Byte offsets in the *previous* source. The caller never reuses them after an edit.
    pub replaced_span: [usize; 2],
}

pub fn rewrite_motion_source(input: MotionSourceEdit) -> Result<MotionSourceEditResult, String> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, &input.source, SourceType::tsx()).parse();
    if !parsed.diagnostics.is_empty() {
        return Err("fix source syntax before changing a declaration from the panel".into());
    }
    let source = &input.source;
    let replacement = serde_json::to_string(&input.value).map_err(|error| error.to_string())?;
    let span = match &input.target {
        MotionSourceEditTarget::Composition { field } => {
            if !matches!(
                field.as_str(),
                "width" | "height" | "fps" | "duration" | "durationInFrames" | "rootFontSize"
            ) {
                return Err(format!("composition field {field} is not editable"));
            }
            if !input.value.is_number() && !input.value.is_string() {
                return Err(format!(
                    "composition field {field} requires a number or frame-rate string"
                ));
            }
            let expression = exported_const(&parsed.program.body, "composition")?;
            let object = direct_object(expression, "composition")?;
            let property = unique_property(object, field)?
                .ok_or_else(|| format!("composition.{field} is not directly declared"))?;
            if !direct_scalar(&property.value) {
                return Err(format!(
                    "composition.{field} is computed; edit its source expression"
                ));
            }
            property.value.span()
        }
        MotionSourceEditTarget::PropDefault { name } => {
            let expression = exported_const(&parsed.program.body, "controls")?;
            let controls = match strip_parens(expression) {
                Expression::CallExpression(call) if matches!(&call.callee, Expression::Identifier(id) if id.name == "defineControls") =>
                {
                    let argument = call
                        .arguments
                        .first()
                        .and_then(Argument::as_expression)
                        .ok_or("defineControls needs a direct object argument")?;
                    direct_object(argument, "defineControls")?
                }
                expression => direct_object(expression, "controls")?,
            };
            let props = unique_property(controls, "props")?
                .ok_or("controls.props is not directly declared")?;
            let props = direct_object(&props.value, "controls.props")?;
            let prop = unique_property(props, name)?
                .ok_or_else(|| format!("controls.props.{name} is not directly declared"))?;
            let arguments = match strip_parens(&prop.value) {
                Expression::CallExpression(call) => call
                    .arguments
                    .first()
                    .and_then(Argument::as_expression)
                    .ok_or_else(|| {
                        format!("controls.props.{name} needs a direct options object")
                    })?,
                expression => expression,
            };
            let options = direct_object(arguments, &format!("controls.props.{name}"))?;
            if let Some(default) = unique_property(options, "default")? {
                if !direct_scalar(&default.value) {
                    return Err(format!(
                        "controls.props.{name}.default is computed; edit its source expression"
                    ));
                }
                default.value.span()
            } else {
                // Insertion after the closing brace is handled below; the marker is a zero-length span.
                let at = options.span.end - 1;
                let insertion = if options.properties.is_empty() {
                    format!("default: {replacement}")
                } else if has_trailing_comma(source, options, at)? {
                    format!(" default: {replacement}")
                } else {
                    format!(", default: {replacement}")
                };
                return replace(source, Span::new(at, at), &insertion);
            }
        }
    };
    replace(source, span, &replacement)
}

fn has_trailing_comma(
    source: &str,
    object: &ObjectExpression<'_>,
    close: u32,
) -> Result<bool, String> {
    let end = object
        .properties
        .last()
        .ok_or("object has no properties")?
        .span()
        .end;
    let mut tail = source
        .get(end as usize..close as usize)
        .ok_or("OXC property span is not a UTF-8 text boundary")?;
    loop {
        tail = tail.trim_start();
        if let Some(comment) = tail.strip_prefix("//") {
            tail = comment.split_once('\n').map_or("", |(_, rest)| rest);
        } else if let Some(comment) = tail.strip_prefix("/*") {
            tail = comment.split_once("*/").map_or("", |(_, rest)| rest);
        } else {
            return Ok(tail.starts_with(','));
        }
    }
}

fn exported_const<'a>(
    body: &'a oxc::allocator::Vec<'a, Statement<'a>>,
    name: &str,
) -> Result<&'a Expression<'a>, String> {
    let mut found = None;
    for statement in body {
        let Statement::ExportDeclaration(export) = statement else {
            continue;
        };
        let Declaration::VariableDeclaration(declaration) = &export.declaration else {
            continue;
        };
        if !declaration.kind.is_const() {
            continue;
        }
        for declarator in &declaration.declarations {
            if declarator.id.get_identifier_name().as_deref() != Some(name) {
                continue;
            }
            if found.is_some() {
                return Err(format!("{name} has multiple declarations"));
            }
            found = declarator.init.as_ref();
        }
    }
    found.ok_or_else(|| format!("export const {name} is not directly declared"))
}

fn direct_object<'a>(
    expression: &'a Expression<'a>,
    label: &str,
) -> Result<&'a ObjectExpression<'a>, String> {
    match strip_parens(expression) {
        Expression::ObjectExpression(object) => Ok(object),
        _ => Err(format!("{label} is computed; edit its source expression")),
    }
}

fn unique_property<'a>(
    object: &'a ObjectExpression<'a>,
    name: &str,
) -> Result<Option<&'a ObjectProperty<'a>>, String> {
    let mut found = None;
    for property in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = property else {
            return Err("object spread makes the declaration ambiguous; edit its source".into());
        };
        if property.computed || property.method {
            return Err("computed object keys cannot be changed from the panel".into());
        }
        if property.key.static_name().as_deref() != Some(name) {
            continue;
        }
        if found.is_some() {
            return Err(format!("duplicate {name} declaration"));
        }
        found = Some(property);
    }
    Ok(found.map(|property| &**property))
}

fn direct_scalar(expression: &Expression<'_>) -> bool {
    match strip_parens(expression) {
        Expression::NumericLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::BooleanLiteral(_) => true,
        Expression::UnaryExpression(unary) => {
            matches!(&unary.argument, Expression::NumericLiteral(_))
        }
        _ => false,
    }
}

fn replace(source: &str, span: Span, replacement: &str) -> Result<MotionSourceEditResult, String> {
    let start = span.start as usize;
    let end = span.end as usize;
    if !source.is_char_boundary(start) || !source.is_char_boundary(end) || start > end {
        return Err("OXC source span is not a UTF-8 text boundary".into());
    }
    let mut output = source.to_owned();
    output.replace_range(start..end, replacement);
    Ok(MotionSourceEditResult {
        source: output,
        replaced_span: [start, end],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_direct_fields_without_reformatting_unicode_or_comments() {
        let source = "// 标题 😀\nexport const composition = { width: 320, /* keep */ height: 180, fps: 30, duration: 1 };";
        let result = rewrite_motion_source(MotionSourceEdit {
            source: source.into(),
            target: MotionSourceEditTarget::Composition {
                field: "height".into(),
            },
            value: serde_json::json!(240),
        })
        .unwrap();
        assert_eq!(result.source, source.replace("height: 180", "height: 240"));
    }

    #[test]
    fn edits_or_inserts_direct_control_default() {
        let source =
            "export const controls = defineControls({props:{title:string({default:'旧标题'})}});";
        let result = rewrite_motion_source(MotionSourceEdit {
            source: source.into(),
            target: MotionSourceEditTarget::PropDefault {
                name: "title".into(),
            },
            value: serde_json::json!("新标题 😀"),
        })
        .unwrap();
        assert!(result.source.contains("default:\"新标题 😀\""));
        let source = "export const controls = defineControls({props:{size:number({min:1})}});";
        let result = rewrite_motion_source(MotionSourceEdit {
            source: source.into(),
            target: MotionSourceEditTarget::PropDefault {
                name: "size".into(),
            },
            value: serde_json::json!(12),
        })
        .unwrap();
        assert!(result.source.contains("min:1, default: 12"));
    }

    #[test]
    fn inserts_default_after_trailing_comma_without_breaking_source() {
        for (source, expected) in [
            (
                "export const controls={props:{size:number({min:1,})}};",
                "min:1, default: 12",
            ),
            (
                "export const controls={props:{size:number({min:1, /* keep */ })}};",
                "min:1, /* keep */  default: 12",
            ),
            (
                "export const controls={props:{size:number({min:1 /* keep, */ })}};",
                "min:1 /* keep, */ , default: 12",
            ),
        ] {
            let result = rewrite_motion_source(MotionSourceEdit {
                source: source.into(),
                target: MotionSourceEditTarget::PropDefault {
                    name: "size".into(),
                },
                value: serde_json::json!(12),
            })
            .unwrap();
            assert!(result.source.contains(expected), "{}", result.source);
            let allocator = Allocator::default();
            assert!(
                Parser::new(&allocator, &result.source, SourceType::tsx())
                    .parse()
                    .diagnostics
                    .is_empty()
            );
        }
    }

    #[test]
    fn rejects_indirection_and_spread() {
        let source = "const size=320; export const composition={width:size,height:180};";
        let error = rewrite_motion_source(MotionSourceEdit {
            source: source.into(),
            target: MotionSourceEditTarget::Composition {
                field: "width".into(),
            },
            value: serde_json::json!(400),
        })
        .unwrap_err();
        assert!(error.contains("computed"));
    }
}
