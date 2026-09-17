//! The entry file's delivery contract: `export const composition = { ... }`.
//!
//! Canvas, optional default frame rate, and duration are fixed author inputs, so they are declared
//! once in the entry `.motion.tsx` and recorded in the artifact. Rendering paths read them from the
//! artifact instead of a command line; nothing substitutes a default here.

use super::*;
use oxc::ast::ast::{BinaryOperator, PropertyKind, UnaryOperator};
use valle_motion::Composition;
use valle_timeline::wire::timeline::{TimelineFrameRateWire, TimelineTimeWire};

/// Fields the contract admits, in the order diagnostics should mention them.
const FIELDS: &[&str] = &["width", "height", "fps", "duration"];

struct Problem {
    field: Option<&'static str>,
    message: String,
}

impl Problem {
    fn new(field: Option<&'static str>, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

impl Compiler<'_> {
    pub(super) fn compile_composition(&mut self, expression: &Expression<'_>) {
        let container = expression.span();
        let Some(value) = self.eval_static(expression) else {
            // `eval_static` already reported why the object is not a prepare-time constant.
            return;
        };
        match composition_from_value(&value) {
            Ok(composition) => self.composition = Some(composition),
            Err(problem) => {
                let span = match problem.field {
                    Some(field) => named_control_span(self.source, container, field),
                    None => container,
                };
                self.illegal(DiagCode::ModuleShape, span, problem.message);
            }
        }
    }
}

/// Read a contract whose values are literals or literal arithmetic, without running the sandbox.
///
/// The compiler needs the logical canvas *before* module-level constants are evaluated, because a
/// `measureText` call there must use the same viewport the scene renders at. Only the measurement
/// viewport is taken from this preview: the authoritative parse still runs in the sandbox and
/// reports every value error, so this never decides what the artifact records.
pub(super) fn literal_composition(program: &Program<'_>) -> Option<Composition> {
    for statement in &program.body {
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
            if declarator.id.get_identifier_name().as_deref() != Some("composition") {
                continue;
            }
            let value = literal_value(declarator.init.as_ref()?)?;
            return composition_from_value(&value).ok();
        }
    }
    None
}

/// Literal JSON for values the sandbox would evaluate identically without any binding.
fn literal_value(expression: &Expression<'_>) -> Option<serde_json::Value> {
    let expression = peel_expr(expression);
    match expression {
        Expression::NumericLiteral(literal) => json_number(literal.value),
        Expression::StringLiteral(literal) => {
            Some(serde_json::Value::String(literal.value.to_string()))
        }
        Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::UnaryNegation => {
            json_number(-literal_value(&unary.argument)?.as_f64()?)
        }
        Expression::BinaryExpression(binary) => {
            let left = literal_value(&binary.left)?.as_f64()?;
            let right = literal_value(&binary.right)?.as_f64()?;
            let value = match binary.operator {
                BinaryOperator::Addition => left + right,
                BinaryOperator::Subtraction => left - right,
                BinaryOperator::Multiplication => left * right,
                BinaryOperator::Division if right != 0.0 => left / right,
                _ => return None,
            };
            json_number(value)
        }
        Expression::ObjectExpression(object) => {
            let mut fields = serde_json::Map::new();
            for item in &object.properties {
                let ObjectPropertyKind::ObjectProperty(property) = item else {
                    return None;
                };
                if property.computed || property.method || property.kind != PropertyKind::Init {
                    return None;
                }
                let name = static_property_name(&property.key)?;
                fields.insert(name, literal_value(&property.value)?);
            }
            Some(serde_json::Value::Object(fields))
        }
        _ => None,
    }
}

/// JSON number for a literal, keeping integral values integer-typed so the field readers that ask
/// for a whole number see one.
fn json_number(value: f64) -> Option<serde_json::Value> {
    if !value.is_finite() {
        return None;
    }
    if value.fract() == 0.0 && value >= 0.0 && value <= u64::MAX as f64 {
        return Some(serde_json::Value::Number(serde_json::Number::from(
            value as u64,
        )));
    }
    serde_json::Number::from_f64(value).map(serde_json::Value::Number)
}

fn composition_from_value(value: &serde_json::Value) -> Result<Composition, Problem> {
    let Some(object) = value.as_object() else {
        return Err(Problem::new(
            None,
            format!(
                "composition must be an immutable object literal; for example {}",
                valle_motion::COMPOSITION_TEMPLATE
            ),
        ));
    };
    for key in object.keys() {
        if !FIELDS.contains(&key.as_str()) {
            return Err(Problem::new(
                None,
                format!(
                    "composition has no field `{key}`; the contract admits {}",
                    FIELDS.join(", ")
                ),
            ));
        }
    }
    for field in ["width", "height", "duration"] {
        if !object.contains_key(field) {
            return Err(Problem::new(
                Some(field),
                format!(
                    "composition needs `{field}`; for example {}",
                    valle_motion::COMPOSITION_TEMPLATE
                ),
            ));
        }
    }
    let width = dimension(object, "width")?;
    let height = dimension(object, "height")?;
    let duration = object["duration"].as_number().ok_or_else(|| {
        Problem::new(
            Some("duration"),
            "duration must be a positive number of seconds",
        )
    })?;
    let duration = TimelineTimeWire::new(duration.to_string())
        .map_err(|_| {
            Problem::new(
                Some("duration"),
                "duration must be a finite positive number of seconds",
            )
        })?
        .to_exact();
    if duration <= valle_timeline::time::ExactRational::ZERO {
        return Err(Problem::new(
            Some("duration"),
            "duration must be positive after microsecond normalization",
        ));
    }
    let fps = object
        .get("fps")
        .map(|value| {
            frame_rate(value)?
                .try_to_frame_rate()
                .map(|rate| rate.into_exact().to_string())
                .map_err(|_| {
                    Problem::new(
                        Some("fps"),
                        "fps must be a positive number or exact rational",
                    )
                })
        })
        .transpose()?;
    Ok(Composition {
        width,
        height,
        fps,
        duration: duration.to_string(),
    })
}

/// A pixel extent: whole, positive, and inside the artifact's `u32` storage.
fn dimension(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<u32, Problem> {
    let message = || {
        Problem::new(
            Some(field),
            format!("{field} must be a positive whole number of pixels"),
        )
    };
    let value = object[field].as_u64().ok_or_else(message)?;
    let value = u32::try_from(value).map_err(|_| message())?;
    if value == 0 {
        return Err(message());
    }
    Ok(value)
}

/// Accepts a JSON number (`30`), a decimal string (`"29.97"`), or an exact rational
/// (`"30000/1001"`), matching the Timeline frame-rate wire form.
fn frame_rate(value: &serde_json::Value) -> Result<TimelineFrameRateWire, Problem> {
    let invalid = || {
        Problem::new(
            Some("fps"),
            "fps must be a positive number such as 30, or an exact rational such as \"30000/1001\"",
        )
    };
    match value {
        serde_json::Value::String(token) if !token.contains('/') => {
            TimelineTimeWire::new(token.clone())
                .map(TimelineFrameRateWire::Decimal)
                .map_err(|_| invalid())
        }
        other => {
            serde_json::from_value::<TimelineFrameRateWire>(other.clone()).map_err(|_| invalid())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_contract_is_read_without_the_sandbox() {
        let allocator = Allocator::default();
        let source = r#"export const composition = { width: 320, height: 180, fps: 30, duration: 1 };
export default function A() { return <View key="a" />; }"#;
        let parsed = Parser::new(&allocator, source, SourceType::tsx()).parse();
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let composition = literal_composition(&parsed.program).expect("literal contract");
        assert_eq!(composition.viewport().tuple(), (320, 180));
        assert_eq!(composition.duration, "1/1");
    }

    #[test]
    fn arithmetic_and_rational_frame_rates_are_read() {
        let allocator = Allocator::default();
        let source = r#"export const composition = { width: 640 * 2, height: 720, fps: "30000/1001", duration: 2 + 1 };
export default function A() { return <View key="a" />; }"#;
        let parsed = Parser::new(&allocator, source, SourceType::tsx()).parse();
        let composition = literal_composition(&parsed.program).expect("literal contract");
        assert_eq!(composition.viewport().tuple(), (1280, 720));
        assert_eq!(composition.duration, "3/1");
        let fps = composition
            .frame_rate()
            .expect("frame rate")
            .expect("declared fps");
        assert_eq!((fps.numerator(), fps.denominator()), (30_000, 1_001));
    }

    #[test]
    fn a_contract_that_needs_the_sandbox_is_not_previewed() {
        let allocator = Allocator::default();
        let source = r#"const W = 320;
export const composition = { width: W, height: 180, fps: 30, duration: 1 };
export default function A() { return <View key="a" />; }"#;
        let parsed = Parser::new(&allocator, source, SourceType::tsx()).parse();
        assert!(literal_composition(&parsed.program).is_none());
    }
}
