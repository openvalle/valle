//! Lower prepare-time `simulate()` tables to bounded numeric frame expressions.

use std::collections::{BTreeSet, VecDeque};
use std::sync::{Mutex, OnceLock};

use super::*;

const MAX_CACHE_ENTRIES: usize = 16;
const MAX_CACHE_BYTES: usize = 16 * 1024 * 1024;

struct CachedSimulation {
    source_digest: String,
    table_digest: String,
    table_json: String,
    hits: usize,
}

static SIMULATION_CACHE: OnceLock<Mutex<VecDeque<CachedSimulation>>> = OnceLock::new();

fn cache() -> &'static Mutex<VecDeque<CachedSimulation>> {
    SIMULATION_CACHE.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// A direct module constant with callbacks whose grammar proves they can only read their numeric
/// arguments and local constants. Calls, writes, captures, and dynamic property access take the
/// original prepare-time path, preserving authored behavior.
pub(super) fn simulation_cache_candidate(
    declaration: &oxc::ast::ast::VariableDeclaration<'_>,
) -> Option<Span> {
    if !declaration.kind.is_const() || declaration.declarations.len() != 1 {
        return None;
    }
    let declarator = &declaration.declarations[0];
    declarator.id.get_identifier_name()?;
    let initializer = declarator.init.as_ref()?;
    // TypeScript wrappers need the existing peeled declaration path; keep caching limited to a
    // source span that can be replaced without changing syntax or the variable binding.
    if !std::ptr::eq(peel_expr(initializer), initializer) || !pure_simulation_call(initializer) {
        return None;
    }
    Some(initializer.span())
}

pub(super) fn simulation_cache_key(initializer: &str) -> String {
    let mut bytes = Vec::with_capacity(STATIC_HELPERS.len() + initializer.len() + 1);
    bytes.extend_from_slice(STATIC_HELPERS.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(initializer.as_bytes());
    ContentDigest::of_bytes(&bytes).as_hex()
}

pub(super) fn cached_simulation(source_digest: &str) -> Option<String> {
    let mut entries = cache().lock().ok()?;
    let index = entries
        .iter()
        .position(|entry| entry.source_digest == source_digest)?;
    let mut entry = entries.remove(index)?;
    if ContentDigest::of_bytes(entry.table_json.as_bytes()).as_hex() != entry.table_digest {
        return None;
    }
    entry.hits += 1;
    let json = entry.table_json.clone();
    entries.push_back(entry);
    Some(json)
}

/// Evaluate a proven pure simulation in its own sandbox. Its result depends only on the
/// initializer source and the builtin implementation, so edits to unrelated module declarations
/// cannot invalidate or alter the table.
pub(super) fn prepare_and_cache_simulation(
    initializer: &str,
    source_digest: &str,
) -> Option<String> {
    let prelude = format!("{STATIC_HELPERS}\nconst __valleCachedSimulation = {initializer};");
    let sandbox = Sandbox::new(&prelude, None).ok()?;
    let value = sandbox
        .eval_json("__valleCachedSimulation", &[], "simulation-cache")
        .ok()?;
    let Ok(table) = serde_json::from_value::<SimulationTable>(value.clone()) else {
        return None;
    };
    if !table.is_valid() {
        return None;
    }
    let Ok(table_json) = serde_json::to_string(&value) else {
        return None;
    };
    if table_json.len() > MAX_CACHE_BYTES {
        return None;
    }
    let entry = CachedSimulation {
        source_digest: source_digest.to_owned(),
        table_digest: ContentDigest::of_bytes(table_json.as_bytes()).as_hex(),
        table_json: table_json.clone(),
        hits: 0,
    };
    let Ok(mut entries) = cache().lock() else {
        return Some(table_json);
    };
    if let Some(index) = entries
        .iter()
        .position(|cached| cached.source_digest == entry.source_digest)
    {
        entries.remove(index);
    }
    entries.push_back(entry);
    while entries.len() > MAX_CACHE_ENTRIES
        || entries
            .iter()
            .map(|entry| entry.table_json.len())
            .sum::<usize>()
            > MAX_CACHE_BYTES
    {
        entries.pop_front();
    }
    Some(table_json)
}

fn pure_simulation_call(expression: &Expression<'_>) -> bool {
    let Expression::CallExpression(call) = expression else {
        return false;
    };
    if !matches!(&call.callee, Expression::Identifier(name) if name.name == "simulate")
        || call.arguments.len() != 1
    {
        return false;
    }
    let Some(Expression::ObjectExpression(options)) = call.arguments[0].as_expression() else {
        return false;
    };
    let mut seen = BTreeSet::new();
    for option in &options.properties {
        let ObjectPropertyKind::ObjectProperty(property) = option else {
            return false;
        };
        if property.computed {
            return false;
        }
        let Some(name) = static_property_name(&property.key) else {
            return false;
        };
        if !seen.insert(name.clone()) {
            return false;
        }
        let valid = match name.as_str() {
            "dt" | "duration" => pure_numeric_expr(&property.value, &BTreeSet::new(), None, 0),
            "init" => pure_arrow(&property.value, 0),
            "step" => pure_arrow(&property.value, 3),
            _ => false,
        };
        if !valid {
            return false;
        }
    }
    seen == BTreeSet::from(["dt".into(), "duration".into(), "init".into(), "step".into()])
}

fn pure_arrow(expression: &Expression<'_>, parameter_count: usize) -> bool {
    let Expression::ArrowFunctionExpression(arrow) = peel_expr(expression) else {
        return false;
    };
    if arrow.params.rest.is_some() || arrow.params.items.len() != parameter_count {
        return false;
    }
    let mut locals = BTreeSet::new();
    let mut names = Vec::new();
    for parameter in &arrow.params.items {
        let BindingPattern::BindingIdentifier(identifier) = &parameter.pattern else {
            return false;
        };
        if !locals.insert(identifier.name.to_string()) {
            return false;
        }
        names.push(identifier.name.to_string());
    }
    let state = names
        .first()
        .filter(|_| parameter_count == 3)
        .map(String::as_str);
    if let Some(expression) = arrow.get_expression() {
        return pure_numeric_expr(expression, &locals, state, 0);
    }
    let Some(body) = arrow.body.as_function_body() else {
        return false;
    };
    for (index, statement) in body.statements.iter().enumerate() {
        match statement {
            Statement::VariableDeclaration(declaration) if declaration.kind.is_const() => {
                for declarator in &declaration.declarations {
                    let BindingPattern::BindingIdentifier(identifier) = &declarator.id else {
                        return false;
                    };
                    if !declarator
                        .init
                        .as_ref()
                        .is_some_and(|value| pure_numeric_expr(value, &locals, state, 0))
                        || !locals.insert(identifier.name.to_string())
                    {
                        return false;
                    }
                }
            }
            Statement::ReturnStatement(result) if index + 1 == body.statements.len() => {
                return result
                    .argument
                    .as_ref()
                    .is_some_and(|value| pure_numeric_expr(value, &locals, state, 0));
            }
            _ => return false,
        }
    }
    false
}

fn pure_numeric_expr(
    expression: &Expression<'_>,
    locals: &BTreeSet<String>,
    state: Option<&str>,
    depth: usize,
) -> bool {
    if depth > 64 {
        return false;
    }
    let next = depth + 1;
    match peel_expr(expression) {
        Expression::NumericLiteral(number) => number.value.is_finite(),
        Expression::Identifier(identifier) => locals.contains(identifier.name.as_str()),
        Expression::StaticMemberExpression(member) => {
            matches!((&member.object, state), (Expression::Identifier(object), Some(name)) if object.name == name)
        }
        Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::UnaryNegation => {
            pure_numeric_expr(&unary.argument, locals, state, next)
        }
        Expression::BinaryExpression(binary)
            if matches!(
                binary.operator,
                BinaryOperator::Addition
                    | BinaryOperator::Subtraction
                    | BinaryOperator::Multiplication
                    | BinaryOperator::Division
                    | BinaryOperator::Remainder
            ) =>
        {
            pure_numeric_expr(&binary.left, locals, state, next)
                && pure_numeric_expr(&binary.right, locals, state, next)
        }
        Expression::ObjectExpression(object) => object.properties.iter().all(|item| {
            let ObjectPropertyKind::ObjectProperty(property) = item else {
                return false;
            };
            !property.computed
                && static_property_name(&property.key).is_some()
                && pure_numeric_expr(&property.value, locals, state, next)
        }),
        _ => false,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SimulationTable {
    #[serde(rename = "__valleType")]
    marker: String,
    dt: f64,
    duration: f64,
    fields: BTreeMap<String, Vec<f64>>,
}

impl SimulationTable {
    fn is_valid(&self) -> bool {
        let steps = self.duration / self.dt;
        self.marker == "simulation"
            && self.dt.is_finite()
            && self.dt > 0.0
            && self.duration.is_finite()
            && self.duration > 0.0
            && steps.is_finite()
            && steps > 0.0
            && steps <= valle_motion::expr::MAX_SIMULATION_STEPS as f64
            && (1..=16).contains(&self.fields.len())
            && self.fields.len() * (steps.ceil() as usize + 1) <= 131_072
            && self.fields.iter().all(|(name, samples)| {
                simulation_field_name(name)
                    && samples.len() == steps.ceil() as usize + 1
                    && samples.iter().all(|sample| sample.is_finite())
                    && samples
                        .windows(2)
                        .all(|pair| (pair[1] - pair[0]).is_finite())
            })
    }
}

impl<'s> Compiler<'s> {
    pub(super) fn lower_simulation_field(
        &mut self,
        call: &oxc::ast::ast::CallExpression<'_>,
        receiver: &Expression<'_>,
        field: &str,
        span: Span,
    ) -> Option<ExprId> {
        let [argument] = call.arguments.as_slice() else {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span(),
                "simulation.at(t) takes exactly one time expression",
            );
            return None;
        };
        let Some(time_expression) = argument.as_expression() else {
            self.illegal(
                DiagCode::GrammarForbidden,
                argument.span(),
                "simulation.at(t) needs a time expression",
            );
            return None;
        };
        // Named immutable tables are resolved once per lexical value/theme, not once
        // per node. Do not cache arbitrary calls whose captured environment is unknown.
        let key = if let Expression::Identifier(identifier) = peel_expr(receiver) {
            Some(
                serde_json::to_string(&(
                    identifier.name.as_str(),
                    self.bindings.statics.get(identifier.name.as_str()),
                    &self.current_theme,
                ))
                .ok()?,
            )
        } else {
            None
        };
        let cached = (!self.shadowed_by_dynamic(receiver))
            .then(|| {
                key.as_ref()
                    .and_then(|key| self.simulation_tables.get(key))
                    .cloned()
            })
            .flatten();
        let table = if let Some(table) = cached {
            table
        } else {
            let Some(value) = self.eval_static(receiver) else {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    receiver.span(),
                    "simulation.at(t) requires a prepare-time simulate() table",
                );
                return None;
            };
            if value.get("__valleType").and_then(serde_json::Value::as_str) != Some("simulation") {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    receiver.span(),
                    "frame-time .at(t) requires a simulate() table",
                );
                return None;
            }
            let Ok(table) = serde_json::from_value::<SimulationTable>(value) else {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    receiver.span(),
                    "simulate() produced an invalid state table",
                );
                return None;
            };
            if !table.is_valid() {
                self.illegal(
                    DiagCode::BuiltinRejected,
                    receiver.span(),
                    "simulate() table exceeds its step budget or contains invalid samples",
                );
                return None;
            }
            let table = std::sync::Arc::new(table);
            if let Some(key) = key {
                self.simulation_tables.insert(key, table.clone());
            }
            table
        };
        let Some(samples) = table.fields.get(field) else {
            self.illegal(
                DiagCode::UnknownIdentifier,
                span,
                format!("simulation has no state field `{field}`"),
            );
            return None;
        };
        let time = self.lower_expr(time_expression)?;
        // Repeated .at(ctx.seconds).field reads share both the frozen sample vector
        // and its admission/evaluation work. TimeScope's transformed expression remains distinct.
        if let Some(index) = self.expr_arena.values.iter().position(|expr| {
            let Expr::SimulationSample {
                time: previous,
                dt,
                duration,
                samples: previous_samples,
            } = expr
            else {
                return false;
            };
            (*previous == time
                || self.expr_arena.values[previous.0 as usize]
                    == self.expr_arena.values[time.0 as usize])
                && *dt == table.dt
                && *duration == table.duration
                && previous_samples == samples
        }) {
            return Some(ExprId(index as u32));
        }
        Some(self.push(
            Expr::SimulationSample {
                time,
                dt: table.dt,
                duration: table.duration,
                samples: samples.clone(),
            },
            span,
        ))
    }
}

fn simulation_field_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$'))
        && bytes.all(|byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'$'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cached_hits(key: &str) -> Option<usize> {
        cache()
            .lock()
            .unwrap()
            .iter()
            .find(|entry| entry.source_digest == key)
            .map(|entry| entry.hits)
    }

    #[test]
    fn pure_module_simulation_survives_unrelated_studio_edits() {
        let expression = "simulate({ dt: 1 / 97, duration: 1, init: () => ({ x: 271 }), step: (state, t, dt) => ({ x: state.x + dt }) })";
        let source = format!(
            "export const composition = {{ width: 640, height: 360, fps: 30, duration: 1 }}; \
             const S = {expression}; export default function Demo(ctx) {{ return <Scene><View key=\"dot\" style={{{{ position: \"absolute\", left: S.at(ctx.seconds).x, top: 0, width: 10, height: 10 }}}} /></Scene>; }}"
        );
        let key = simulation_cache_key(expression);
        let first = compile_motion(&source).unwrap().artifact;
        let before = cached_hits(&key).expect("pure simulation was cached");
        let changed_style = source.replace("top: 0", "top: 1");
        let second = compile_motion(&changed_style).unwrap().artifact;
        assert_eq!(cached_hits(&key), Some(before + 1));
        let changed_static = source.replace("const S =", "const unrelated = 42; const S =");
        let third = compile_motion(&changed_static).unwrap().artifact;
        assert_eq!(cached_hits(&key), Some(before + 2));
        let samples = |artifact: &SceneArtifact| {
            artifact.exprs.iter().find_map(|expr| match expr {
                Expr::SimulationSample { samples, .. } => Some(samples.clone()),
                _ => None,
            })
        };
        assert_eq!(samples(&first), samples(&second));
        assert_eq!(samples(&first), samples(&third));

        let changed_step = source.replace("state.x + dt", "state.x + 2 * dt");
        let changed = compile_motion(&changed_step).unwrap().artifact;
        assert_ne!(samples(&first), samples(&changed));
    }

    #[test]
    fn effectful_callbacks_keep_running_after_unrelated_edits() {
        let source = r#"
            export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
            const counter = { value: 0 };
            const S = simulate({ dt: 1, duration: 2, init: () => ({ x: 0 }),
              step: (state, t, dt) => { counter.value += 1; return { x: state.x + 1 }; } });
            const COUNT = counter.value;
            export default function Demo(ctx) { return <Scene><View key="dot"
              style={{ position: "absolute", left: COUNT, top: 0, width: 10, height: 10 }} />
              </Scene>; }
        "#;
        for edited in [source.to_string(), source.replace("top: 0", "top: 1")] {
            let artifact = compile_motion(&edited).unwrap().artifact;
            let dot = artifact
                .nodes
                .iter()
                .find(|node| node.key == "dot")
                .unwrap();
            let left = dot
                .styles
                .iter()
                .find(|style| style.property == "left")
                .unwrap();
            assert!(matches!(
                &left.value,
                StyleValue::Static {
                    value: MotionValue::Number(2.0)
                }
            ));
        }
    }

    #[test]
    fn module_graph_recompilation_reuses_the_same_table() {
        let expression = "simulate({ dt: 1 / 113, duration: 1, init: () => ({ x: 913.125 }), step: (state, t, dt) => ({ x: state.x + dt }) })";
        let source = format!(
            "export const composition = {{ width: 640, height: 360, fps: 30, duration: 1 }}; \
             const S = {expression}; export default function Demo(ctx) {{ return <Scene><View key=\"dot\" style={{{{ position: \"absolute\", left: S.at(ctx.seconds).x, top: 0, width: 10, height: 10 }}}} /></Scene>; }}"
        );
        let graph = |source: String| {
            MotionModuleGraph::new(
                "scene.motion.tsx",
                BTreeMap::from([("scene.motion.tsx".to_string(), source)]),
            )
            .unwrap()
        };
        compile_motion_modules(&graph(source.clone())).unwrap();
        let key = simulation_cache_key(expression);
        let before = cached_hits(&key).expect("linked module simulation was cached");
        compile_motion_modules(&graph(source.replace("top: 0", "top: 1"))).unwrap();
        assert_eq!(cached_hits(&key), Some(before + 1));
    }
}
