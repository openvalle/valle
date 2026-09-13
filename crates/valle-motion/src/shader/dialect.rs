use std::collections::{BTreeMap, BTreeSet};

use super::{DiagnosticCode, ShaderDiagnostic, ShaderManifest, UniformType};
mod syntax;
use syntax::{Expr, ExprKind, Function, Statement, StatementKind, Type};

/// Host limits apply to every admitted shader, independently of author metadata.
/// Costs count scalar work after loop/call expansion; branches use their maximum path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShaderLimits {
    pub source_bytes: usize,
    pub tokens: usize,
    pub syntax_depth: usize,
    pub functions: usize,
    pub call_depth: usize,
    pub loop_iterations: usize,
    pub operations_per_pixel: usize,
    pub samples_per_pixel: usize,
    pub calls_per_pixel: usize,
    pub frame_operations: u64,
    pub frame_samples: u64,
}

pub const SHADER_LIMITS: ShaderLimits = ShaderLimits {
    source_bytes: 32 * 1024,
    tokens: 8192,
    syntax_depth: 32,
    functions: 32,
    call_depth: 16,
    loop_iterations: 256,
    operations_per_pixel: 4096,
    samples_per_pixel: 64,
    calls_per_pixel: 256,
    frame_operations: super::MAX_LAYER_PIXELS * 1024,
    frame_samples: super::MAX_LAYER_PIXELS * 64,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialectReport {
    pub source_bytes: usize,
    pub samples_per_pixel: usize,
    pub operations_per_pixel: usize,
    pub calls_per_pixel: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct Cost {
    operations: usize,
    samples: usize,
    calls: usize,
    depth: usize,
}
impl Cost {
    fn work(operations: usize) -> Self {
        Self {
            operations,
            ..Self::default()
        }
    }
    fn add(self, rhs: Self) -> Self {
        Self {
            operations: self.operations.saturating_add(rhs.operations),
            samples: self.samples.saturating_add(rhs.samples),
            calls: self.calls.saturating_add(rhs.calls),
            depth: self.depth.max(rhs.depth),
        }
    }
    fn max(self, rhs: Self) -> Self {
        Self {
            operations: self.operations.max(rhs.operations),
            samples: self.samples.max(rhs.samples),
            calls: self.calls.max(rhs.calls),
            depth: self.depth.max(rhs.depth),
        }
    }
    fn times(self, count: usize) -> Self {
        Self {
            operations: self.operations.saturating_mul(count),
            samples: self.samples.saturating_mul(count),
            calls: self.calls.saturating_mul(count),
            depth: self.depth,
        }
    }
    fn validate(self, pos: usize) -> Result<(), ShaderDiagnostic> {
        if self.depth > SHADER_LIMITS.call_depth {
            return Err(error(pos, "function call depth limit exceeded"));
        }
        if self.operations > SHADER_LIMITS.operations_per_pixel
            || self.samples > SHADER_LIMITS.samples_per_pixel
            || self.calls > SHADER_LIMITS.calls_per_pixel
        {
            return Err(ShaderDiagnostic::new(
                DiagnosticCode::BudgetExceeded,
                pos.to_string(),
                format!(
                    "expanded shader requires {} scalar operations, {} samples, {} calls; host limits are {}, {}, {}",
                    self.operations,
                    self.samples,
                    self.calls,
                    SHADER_LIMITS.operations_per_pixel,
                    SHADER_LIMITS.samples_per_pixel,
                    SHADER_LIMITS.calls_per_pixel
                ),
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
struct Symbol {
    ty: Type,
    writable: bool,
    constant: Option<i64>,
    code: String,
}
type Scope = Vec<BTreeMap<String, Symbol>>;
fn lookup<'a>(scope: &'a Scope, name: &str) -> Option<&'a Symbol> {
    scope.iter().rev().find_map(|s| s.get(name))
}

struct Value {
    ty: Type,
    code: String,
    cost: Cost,
    constant: Option<i64>,
}
struct Body {
    code: String,
    cost: Cost,
    returns: bool,
}
#[derive(Clone)]
struct CheckedFunction {
    code: String,
    cost: Cost,
}

struct Checker<'a> {
    manifest: &'a ShaderManifest,
    functions: Vec<Function>,
    signatures: BTreeMap<String, usize>,
    checked: BTreeMap<usize, CheckedFunction>,
    visiting: BTreeSet<usize>,
    order: Vec<usize>,
    expression_depth: usize,
}

pub fn validate_dialect(
    manifest: &ShaderManifest,
    source: &str,
) -> Result<DialectReport, ShaderDiagnostic> {
    compile_dialect(manifest, source).map(|(report, _)| report)
}

pub fn lower_to_sksl(manifest: &ShaderManifest, source: &str) -> Result<String, ShaderDiagnostic> {
    compile_dialect(manifest, source).map(|(_, sksl)| sksl)
}

pub(super) fn compile_dialect(
    manifest: &ShaderManifest,
    source: &str,
) -> Result<(DialectReport, String), ShaderDiagnostic> {
    compile_inner(manifest, source).map_err(|mut e| {
        if let Ok(pos) = e.path.parse::<usize>() {
            let prefix = &source[..pos.min(source.len())];
            let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
            let column = prefix.len() - prefix.rfind('\n').map_or(0, |n| n + 1) + 1;
            e.path = format!("source:{line}:{column}");
        }
        e
    })
}

fn compile_inner(
    manifest: &ShaderManifest,
    source: &str,
) -> Result<(DialectReport, String), ShaderDiagnostic> {
    manifest.validate_shape()?;
    if source.len() > SHADER_LIMITS.source_bytes {
        return Err(ShaderDiagnostic::new(
            DiagnosticCode::BudgetExceeded,
            "source",
            "shader source byte limit exceeded",
        ));
    }
    if !source.is_ascii() {
        return Err(error(0, "source must be ASCII"));
    }
    let functions = syntax::parse(source, SHADER_LIMITS.tokens, SHADER_LIMITS.syntax_depth)?;
    if functions.len() > SHADER_LIMITS.functions {
        return Err(error(0, "function count limit exceeded"));
    }
    let mut signatures = BTreeMap::new();
    for (index, function) in functions.iter().enumerate() {
        if function.name != "valle_main" {
            validate_name(manifest, &function.name, function.pos)?;
        }
        if signatures.insert(function.name.clone(), index).is_some() {
            return Err(error(function.pos, "duplicate function name"));
        }
    }
    let entry = *signatures
        .get("valle_main")
        .ok_or_else(|| error(0, "source requires `float4 valle_main(float2 uv)`"))?;
    if functions[entry].result != Type::Float(4)
        || functions[entry]
            .parameters
            .iter()
            .map(|p| p.0)
            .collect::<Vec<_>>()
            != [Type::Float(2)]
    {
        return Err(error(
            functions[entry].pos,
            "entry requires one float2 parameter and a float4/half4 result",
        ));
    }
    let mut checker = Checker {
        manifest,
        functions,
        signatures,
        checked: BTreeMap::new(),
        visiting: BTreeSet::new(),
        order: Vec::new(),
        expression_depth: 0,
    };
    for index in 0..checker.functions.len() {
        checker.function(index)?;
    }
    let cost = checker.checked[&entry].cost;
    let mut code = prelude(manifest);
    for index in &checker.order {
        code.push_str(&checker.checked[index].code);
    }
    let alpha = if manifest.output.allow_transparent {
        "clamp(straight.a, 0.0, 1.0)"
    } else {
        "1.0"
    };
    code.push_str(&format!("float4 main(float2 xy) {{ float4 straight = valle_finite(valle_main(xy / resolution)); float a = {alpha}; return valle_finite(float4(valle_linear_srgb_to_working(straight.rgb) * a, a)); }}\n"));
    Ok((
        DialectReport {
            source_bytes: source.len(),
            samples_per_pixel: cost.samples,
            operations_per_pixel: cost.operations,
            calls_per_pixel: cost.calls,
        },
        code,
    ))
}

impl Checker<'_> {
    fn function(&mut self, index: usize) -> Result<CheckedFunction, ShaderDiagnostic> {
        if let Some(result) = self.checked.get(&index) {
            return Ok(result.clone());
        }
        let f = self.functions[index].clone();
        if !self.visiting.insert(index) {
            return Err(error(f.pos, "recursive call graph is forbidden"));
        }
        if self.visiting.len() > SHADER_LIMITS.call_depth {
            return Err(error(f.pos, "function call depth limit exceeded"));
        }
        let mut globals = BTreeMap::new();
        globals.insert(
            "resolution".into(),
            Symbol {
                ty: Type::Float(2),
                writable: false,
                constant: None,
                code: "resolution".into(),
            },
        );
        for u in &self.manifest.uniforms {
            let (ty, code) = match u.uniform_type {
                UniformType::Float => (Type::Float(1), u.name.clone()),
                UniformType::Float2 => (Type::Float(2), u.name.clone()),
                UniformType::Float3 => (Type::Float(3), u.name.clone()),
                UniformType::Float4 => (Type::Float(4), u.name.clone()),
                UniformType::Float2x2 => (Type::Matrix(2), u.name.clone()),
                UniformType::Float3x3 => (Type::Matrix(3), u.name.clone()),
                UniformType::Float4x4 => (Type::Matrix(4), u.name.clone()),
                UniformType::Color => (Type::Float(4), u.name.clone()),
                UniformType::Bool => (Type::Bool, format!("(valle_uniform_{} >= 0.5)", u.name)),
            };
            globals.insert(
                u.name.clone(),
                Symbol {
                    ty,
                    writable: false,
                    constant: None,
                    code,
                },
            );
        }
        let mut scope = vec![globals, BTreeMap::new()];
        let mut parameters = Vec::new();
        for (ty, name) in &f.parameters {
            self.declare(
                &mut scope,
                name,
                Symbol {
                    ty: *ty,
                    writable: true,
                    constant: None,
                    code: name.clone(),
                },
                f.pos,
            )?;
            parameters.push(format!("{} {name}", ty.name()));
        }
        let body = self.statement(&f.body, &mut scope, f.result)?;
        if !body.returns {
            return Err(error(f.pos, "every execution path must return a value"));
        }
        body.cost.validate(f.pos)?;
        let checked = CheckedFunction {
            code: format!(
                "{} {}({}) {}\n",
                f.result.name(),
                f.name,
                parameters.join(", "),
                body.code
            ),
            cost: body.cost,
        };
        self.visiting.remove(&index);
        self.checked.insert(index, checked.clone());
        self.order.push(index);
        Ok(checked)
    }

    fn declare(
        &self,
        scope: &mut Scope,
        name: &str,
        symbol: Symbol,
        pos: usize,
    ) -> Result<(), ShaderDiagnostic> {
        validate_name(self.manifest, name, pos)?;
        if self.signatures.contains_key(name) || scope.last().unwrap().contains_key(name) {
            return Err(error(
                pos,
                format!("duplicate or reserved declaration `{name}`"),
            ));
        }
        // Read-only loop counters cannot be shadowed, including by a nested loop.
        if lookup(scope, name).is_some_and(|s| !s.writable && s.code.starts_with("valle_loop_")) {
            return Err(error(pos, "loop control variable cannot be shadowed"));
        }
        scope.last_mut().unwrap().insert(name.into(), symbol);
        Ok(())
    }

    fn child_statement(
        &mut self,
        stmt: &Statement,
        scope: &mut Scope,
        result: Type,
    ) -> Result<Body, ShaderDiagnostic> {
        scope.push(BTreeMap::new());
        let body = self.statement(stmt, scope, result);
        scope.pop();
        body.map(|body| Body {
            code: format!("{{ {} }}", body.code),
            ..body
        })
    }

    fn statement(
        &mut self,
        stmt: &Statement,
        scope: &mut Scope,
        result: Type,
    ) -> Result<Body, ShaderDiagnostic> {
        let mut cost = Cost::default();
        let mut returns = false;
        let code = match &stmt.kind {
            StatementKind::Block(statements) => {
                scope.push(BTreeMap::new());
                let mut code = String::from("{\n");
                for statement in statements {
                    if returns {
                        return Err(error(statement.pos, "unreachable statement after return"));
                    }
                    let body = self.statement(statement, scope, result)?;
                    cost = cost.add(body.cost);
                    returns = body.returns;
                    code.push_str(&body.code);
                    code.push('\n');
                }
                scope.pop();
                code.push('}');
                code
            }
            StatementKind::Declare {
                constant,
                ty,
                name,
                value,
            } => {
                let value = self.expr(value, scope)?;
                expect_type(stmt.pos, *ty, value.ty)?;
                self.declare(
                    scope,
                    name,
                    Symbol {
                        ty: *ty,
                        writable: !constant,
                        constant: if *constant { value.constant } else { None },
                        code: name.clone(),
                    },
                    stmt.pos,
                )?;
                cost = value.cost.add(Cost::work(ty.width()));
                // Constants remain read-only in the language; SkSL need not treat a frame-uniform
                // expression as a compile-time constant. Integer loop bounds are folded below.
                format!("{} {name} = {};", ty.name(), value.code)
            }
            StatementKind::Assign { target, op, value } => {
                let root = assignment_root(target).ok_or_else(|| error(target.pos, "assignment requires a local variable, swizzle, or constant component index"))?;
                let symbol = lookup(scope, root)
                    .ok_or_else(|| error(target.pos, format!("unknown variable `{root}`")))?;
                if !symbol.writable {
                    return Err(error(
                        target.pos,
                        "cannot write a uniform, constant, or loop control variable",
                    ));
                }
                validate_write_components(target)?;
                let left = self.expr(target, scope)?;
                let right = self.expr(value, scope)?;
                let assigned = if op == "=" {
                    right
                } else {
                    binary(
                        &op[..1],
                        Value {
                            ty: left.ty,
                            code: left.code.clone(),
                            cost: Cost::default(),
                            constant: None,
                        },
                        right,
                        stmt.pos,
                    )?
                };
                expect_type(stmt.pos, left.ty, assigned.ty)?;
                cost = left
                    .cost
                    .add(assigned.cost)
                    .add(Cost::work(left.ty.width()));
                format!("{} = {};", left.code, assigned.code)
            }
            StatementKind::Return(value) => {
                let value = self.expr(value, scope)?;
                expect_type(stmt.pos, result, value.ty)?;
                cost = value.cost;
                returns = true;
                format!("return {};", value.code)
            }
            StatementKind::If { condition, yes, no } => {
                let condition = self.expr(condition, scope)?;
                expect_type(stmt.pos, Type::Bool, condition.ty)?;
                let yes = self.child_statement(yes, scope, result)?;
                let no = no
                    .as_ref()
                    .map(|n| self.child_statement(n, scope, result))
                    .transpose()?;
                returns = yes.returns && no.as_ref().is_some_and(|n| n.returns);
                cost = condition
                    .cost
                    .add(
                        yes.cost
                            .max(no.as_ref().map_or(Cost::default(), |n| n.cost)),
                    )
                    .add(Cost::work(1));
                format!(
                    "if ({}) {}{}",
                    condition.code,
                    yes.code,
                    no.map_or(String::new(), |n| format!(" else {}", n.code))
                )
            }
            StatementKind::For {
                name,
                start,
                comparison,
                end,
                step,
                subtract,
                body,
            } => {
                let start = self.constant_int(start, scope)?;
                let end = self.constant_int(end, scope)?;
                let step = self.constant_int(step, scope)? * if *subtract { -1 } else { 1 };
                if step == 0
                    || (comparison.starts_with('<') && step < 0)
                    || (comparison.starts_with('>') && step > 0)
                {
                    return Err(error(
                        stmt.pos,
                        "for step must progress toward its constant bound",
                    ));
                }
                let inside = |n| match comparison.as_str() {
                    "<" => n < end,
                    "<=" => n <= end,
                    ">" => n > end,
                    ">=" => n >= end,
                    _ => unreachable!(),
                };
                let mut n = start;
                let mut iterations = 0usize;
                while inside(n) {
                    iterations += 1;
                    if iterations > SHADER_LIMITS.loop_iterations {
                        return Err(ShaderDiagnostic::new(
                            DiagnosticCode::BudgetExceeded,
                            stmt.pos.to_string(),
                            "for iteration limit exceeded",
                        ));
                    }
                    n = n
                        .checked_add(step)
                        .filter(|n| i32::try_from(*n).is_ok())
                        .ok_or_else(|| error(stmt.pos, "for counter overflows int"))?;
                }
                scope.push(BTreeMap::new());
                let counter = format!("valle_loop_{}", stmt.pos);
                self.declare(
                    scope,
                    name,
                    Symbol {
                        ty: Type::Int,
                        writable: false,
                        constant: None,
                        code: counter.clone(),
                    },
                    stmt.pos,
                )?;
                let body = self.child_statement(body, scope, result)?;
                scope.pop();
                // A loop is not a guaranteed return in backend control-flow analysis.
                returns = false;
                cost = body
                    .cost
                    .add(Cost::work(2))
                    .times(iterations)
                    .add(Cost::work(2));
                format!(
                    "for (int {counter} = {start}; {counter} {comparison} {end}; {counter} += {step}) {}",
                    body.code
                )
            }
            StatementKind::Eval(expr) => {
                let value = self.expr(expr, scope)?;
                cost = value.cost;
                format!("{};", value.code)
            }
        };
        cost.validate(stmt.pos)?;
        Ok(Body {
            code,
            cost,
            returns,
        })
    }

    fn constant_int(&mut self, expr: &Expr, scope: &Scope) -> Result<i64, ShaderDiagnostic> {
        let value = self.expr(expr, scope)?;
        if value.ty != Type::Int {
            return Err(error(
                expr.pos,
                "for bounds and indices require constant int expressions",
            ));
        }
        value.constant.ok_or_else(|| {
            error(
                expr.pos,
                "for bounds and indices must be compile-time constants",
            )
        })
    }

    fn expr(&mut self, expr: &Expr, scope: &Scope) -> Result<Value, ShaderDiagnostic> {
        self.expression_depth += 1;
        if self.expression_depth > 128 {
            return Err(error(expr.pos, "expression tree depth limit exceeded"));
        }
        let result = self.expr_inner(expr, scope);
        self.expression_depth -= 1;
        result
    }

    fn expr_inner(&mut self, expr: &Expr, scope: &Scope) -> Result<Value, ShaderDiagnostic> {
        let value = match &expr.kind {
            ExprKind::Number(number) => {
                let integer = !number.contains(['.', 'e', 'E']);
                let constant = if integer {
                    Some(
                        number
                            .parse::<i32>()
                            .map_err(|_| error(expr.pos, "integer literal exceeds int range"))?
                            as i64,
                    )
                } else {
                    None
                };
                Value {
                    ty: if integer { Type::Int } else { Type::Float(1) },
                    code: number.clone(),
                    cost: Cost::default(),
                    constant,
                }
            }
            ExprKind::Name(name) if name == "true" || name == "false" => Value {
                ty: Type::Bool,
                code: name.clone(),
                cost: Cost::default(),
                constant: None,
            },
            ExprKind::Name(name) => {
                let symbol = lookup(scope, name).ok_or_else(|| {
                    error(
                        expr.pos,
                        format!(
                            "unknown value `{name}`; textures are opaque and require sample helpers"
                        ),
                    )
                })?;
                Value {
                    ty: symbol.ty,
                    code: symbol.code.clone(),
                    cost: Cost::default(),
                    constant: symbol.constant,
                }
            }
            ExprKind::Call(name, args) => {
                let args = args
                    .iter()
                    .map(|arg| self.expr(arg, scope))
                    .collect::<Result<Vec<_>, _>>()?;
                let cost = args.iter().fold(Cost::default(), |sum, v| sum.add(v.cost));
                let arg_code = args
                    .iter()
                    .map(|v| v.code.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                if let Some(&index) = self.signatures.get(name) {
                    if name == "valle_main" {
                        return Err(error(expr.pos, "entry cannot be called by author code"));
                    }
                    let f = &self.functions[index];
                    if f.parameters.len() != args.len() {
                        return Err(error(expr.pos, "function argument count mismatch"));
                    }
                    for ((ty, _), arg) in f.parameters.iter().zip(&args) {
                        expect_type(expr.pos, *ty, arg.ty)?;
                    }
                    let ty = f.result;
                    let function = self.function(index)?;
                    Value {
                        ty,
                        code: format!("{name}({arg_code})"),
                        cost: cost.add(Cost {
                            calls: function.cost.calls.saturating_add(1),
                            depth: function.cost.depth.saturating_add(1),
                            ..function.cost
                        }),
                        constant: None,
                    }
                } else if name == "sampleContent"
                    || self
                        .manifest
                        .inputs
                        .iter()
                        .any(|i| *name == format!("sample_{}", i.name))
                {
                    if args.len() != 1 || args[0].ty != Type::Float(2) {
                        return Err(error(expr.pos, "sampling requires one float2 UV"));
                    }
                    let input = self
                        .manifest
                        .inputs
                        .iter()
                        .find(|i| *name == format!("sample_{}", i.name));
                    let linear = input.is_some_and(|i| i.sampling == super::InputSampling::Linear);
                    let data = input.is_some_and(|i| i.kind == super::InputKind::Data);
                    Value {
                        ty: Type::Float(4),
                        code: format!("{name}({arg_code})"),
                        cost: cost.add(Cost {
                            samples: (if linear { 4 } else { 1 }) * (if data { 8 } else { 1 }),
                            operations: (if linear { 32 } else { 4 })
                                + (if data {
                                    80 * (if linear { 4 } else { 1 })
                                } else {
                                    0
                                }),
                            ..Cost::default()
                        }),
                        constant: None,
                    }
                } else {
                    builtin(name, &args, expr.pos, cost, &arg_code)?
                }
            }
            ExprKind::Unary(op, arg) => {
                let arg = self.expr(arg, scope)?;
                if (op == "!" && arg.ty != Type::Bool) || (op != "!" && arg.ty == Type::Bool) {
                    return Err(error(expr.pos, "unary operator has incompatible type"));
                }
                Value {
                    ty: arg.ty,
                    code: format!("({op}{})", arg.code),
                    cost: arg.cost.add(Cost::work(arg.ty.width())),
                    constant: arg.constant.and_then(|n| {
                        if op == "-" {
                            n.checked_neg().filter(|n| i32::try_from(*n).is_ok())
                        } else {
                            Some(n)
                        }
                    }),
                }
            }
            ExprKind::Binary(op, left, right) => binary(
                op,
                self.expr(left, scope)?,
                self.expr(right, scope)?,
                expr.pos,
            )?,
            ExprKind::Select(condition, yes, no) => {
                let condition = self.expr(condition, scope)?;
                let yes = self.expr(yes, scope)?;
                let no = self.expr(no, scope)?;
                expect_type(expr.pos, Type::Bool, condition.ty)?;
                let ty = common_type(yes.ty, no.ty).ok_or_else(|| {
                    error(expr.pos, "conditional branches have incompatible types")
                })?;
                Value {
                    ty,
                    code: format!("({} ? {} : {})", condition.code, yes.code, no.code),
                    cost: condition.cost.add(yes.cost.max(no.cost)).add(Cost::work(1)),
                    constant: None,
                }
            }
            ExprKind::Swizzle(base, fields) => {
                let base = self.expr(base, scope)?;
                let Type::Float(width) = base.ty else {
                    return Err(error(expr.pos, "swizzle requires a float vector"));
                };
                let alphabet = if fields.bytes().all(|b| b"xyzw".contains(&b)) {
                    b"xyzw"
                } else {
                    b"rgba"
                };
                if fields.is_empty()
                    || fields.len() > 4
                    || fields.bytes().any(|b| {
                        alphabet
                            .iter()
                            .position(|c| *c == b)
                            .is_none_or(|n| n >= width as usize)
                    })
                {
                    return Err(error(
                        expr.pos,
                        "swizzle components exceed vector width or mix naming sets",
                    ));
                }
                Value {
                    ty: Type::Float(fields.len() as u8),
                    code: format!("({}).{fields}", base.code),
                    cost: base.cost,
                    constant: None,
                }
            }
            ExprKind::Index(base, index) => {
                let base = self.expr(base, scope)?;
                let index = self.constant_int(index, scope)?;
                let (width, ty) = match base.ty {
                    Type::Float(n) => (n, Type::Float(1)),
                    Type::Matrix(n) => (n, Type::Float(n)),
                    _ => return Err(error(expr.pos, "indexing requires a vector or matrix")),
                };
                if index < 0 || index >= i64::from(width) {
                    return Err(error(expr.pos, "constant index is out of range"));
                }
                Value {
                    ty,
                    code: format!("({})[{index}]", base.code),
                    cost: base.cost,
                    constant: None,
                }
            }
        };
        value.cost.validate(expr.pos)?;
        Ok(value)
    }
}

fn expect_type(pos: usize, expected: Type, actual: Type) -> Result<(), ShaderDiagnostic> {
    if expected.accepts(actual) {
        Ok(())
    } else {
        Err(error(
            pos,
            format!("expected {}, got {}", expected.name(), actual.name()),
        ))
    }
}
fn common_type(a: Type, b: Type) -> Option<Type> {
    if a.accepts(b) {
        Some(a)
    } else if b.accepts(a) {
        Some(b)
    } else {
        None
    }
}
fn arithmetic_type(a: Type, b: Type) -> Option<Type> {
    if a == Type::Bool || b == Type::Bool {
        return None;
    }
    common_type(a, b).or_else(|| match (a, b) {
        (Type::Float(n), Type::Float(1) | Type::Int)
        | (Type::Float(1) | Type::Int, Type::Float(n)) => Some(Type::Float(n)),
        (Type::Matrix(n), Type::Float(1) | Type::Int)
        | (Type::Float(1) | Type::Int, Type::Matrix(n)) => Some(Type::Matrix(n)),
        _ => None,
    })
}
fn binary(op: &str, left: Value, right: Value, pos: usize) -> Result<Value, ShaderDiagnostic> {
    let ty = if matches!(op, "&&" | "||") {
        expect_type(pos, Type::Bool, left.ty)?;
        expect_type(pos, Type::Bool, right.ty)?;
        Type::Bool
    } else if matches!(op, "==" | "!=" | "<" | ">" | "<=" | ">=") {
        let common =
            common_type(left.ty, right.ty).ok_or_else(|| error(pos, "comparison types differ"))?;
        if common.width() != 1 || (common == Type::Bool && !matches!(op, "==" | "!=")) {
            return Err(error(pos, "comparison requires compatible scalars"));
        }
        Type::Bool
    } else if op == "*"
        && matches!((left.ty, right.ty), (Type::Matrix(n), Type::Float(m)) | (Type::Float(m), Type::Matrix(n)) if n == m)
    {
        match (left.ty, right.ty) {
            (Type::Float(n), _) | (_, Type::Float(n)) => Type::Float(n),
            _ => unreachable!(),
        }
    } else {
        arithmetic_type(left.ty, right.ty)
            .ok_or_else(|| error(pos, "arithmetic operand types differ"))?
    };
    if op == "%" && ty != Type::Int {
        return Err(error(
            pos,
            "% requires int operands; use mod for floating point",
        ));
    }
    if op == "/" && matches!(ty, Type::Matrix(_)) {
        return Err(error(
            pos,
            "matrix division is unsupported; multiply by a scalar reciprocal",
        ));
    }
    let code = if op == "/" || op == "%" {
        let function = if op == "%" {
            "valle_imod"
        } else if ty == Type::Int {
            "valle_idiv"
        } else {
            "valle_div"
        };
        format!(
            "{function}({}({}), {}({}))",
            ty.name(),
            left.code,
            ty.name(),
            right.code
        )
    } else {
        format!("({} {op} {})", left.code, right.code)
    };
    let constant = if ty == Type::Int {
        left.constant
            .zip(right.constant)
            .and_then(|(a, b)| match op {
                "+" => a.checked_add(b),
                "-" => a.checked_sub(b),
                "*" => a.checked_mul(b),
                "/" => {
                    if b == 0 {
                        Some(0)
                    } else {
                        a.checked_div(b)
                    }
                }
                "%" => {
                    if b == 0 {
                        Some(0)
                    } else {
                        a.checked_rem(b)
                    }
                }
                _ => None,
            })
            .filter(|n| i32::try_from(*n).is_ok())
    } else {
        None
    };
    let work = if op == "*"
        && (matches!(left.ty, Type::Matrix(_)) || matches!(right.ty, Type::Matrix(_)))
    {
        left.ty.width() * right.ty.width() * 2
    } else {
        ty.width() * if op == "/" { 3 } else { 1 }
    };
    Ok(Value {
        ty,
        code,
        cost: left.cost.add(right.cost).add(Cost::work(work)),
        constant,
    })
}

pub(super) fn builtin_names() -> &'static [&'static str] {
    &[
        "abs",
        "ceil",
        "clamp",
        "cos",
        "cross",
        "dot",
        "exp",
        "exp2",
        "floor",
        "fract",
        "length",
        "log",
        "log2",
        "max",
        "min",
        "mix",
        "mod",
        "normalize",
        "pow",
        "reflect",
        "saturate",
        "sin",
        "smoothstep",
        "sqrt",
        "step",
        "tan",
        "inversesqrt",
        "srgbToLinear",
        "linearToSrgb",
        "transpose",
    ]
}

fn builtin(
    name: &str,
    args: &[Value],
    pos: usize,
    cost: Cost,
    arg_code: &str,
) -> Result<Value, ShaderDiagnostic> {
    let types = args.iter().map(|a| a.ty).collect::<Vec<_>>();
    if let Some(ty) = Type::parse(name) {
        let valid = match ty {
            Type::Int | Type::Bool => args.len() == 1 && args[0].ty.width() == 1,
            Type::Float(n) => {
                !args.is_empty()
                    && args
                        .iter()
                        .all(|a| matches!(a.ty, Type::Float(_) | Type::Int))
                    && ((args.len() == 1 && args[0].ty.width() == 1)
                        || args.iter().map(|a| a.ty.width()).sum::<usize>() == n as usize)
            }
            Type::Matrix(n) => {
                (args.len() == 1
                    && (args[0].ty == Type::Float(1)
                        || args[0].ty == Type::Int
                        || args[0].ty == ty))
                    || (args
                        .iter()
                        .all(|a| matches!(a.ty, Type::Float(_) | Type::Int))
                        && args.iter().map(|a| a.ty.width()).sum::<usize>() == (n * n) as usize)
            }
        };
        if !valid {
            return Err(error(pos, "constructor arguments do not match target type"));
        }
        let constant = if ty == Type::Int && args.len() == 1 {
            args[0].constant
        } else {
            None
        };
        return Ok(Value {
            ty,
            code: format!("{}({arg_code})", ty.name()),
            cost: cost.add(Cost::work(ty.width())),
            constant,
        });
    }
    let invalid = || error(pos, format!("invalid arguments for builtin `{name}`"));
    if !builtin_names().contains(&name) {
        return Err(error(
            pos,
            format!("function `{name}` is outside the language builtin set"),
        ));
    }
    let float = |ty| matches!(ty, Type::Float(_));
    let ty = match name {
        "transpose" => match types.as_slice() {
            [ty @ Type::Matrix(_)] => *ty,
            _ => return Err(invalid()),
        },
        "cross" => {
            if types == [Type::Float(3), Type::Float(3)] {
                Type::Float(3)
            } else {
                return Err(invalid());
            }
        }
        "dot" | "reflect" => {
            if types.len() != 2 || !float(types[0]) || types[0] != types[1] {
                return Err(invalid());
            }
            if name == "dot" {
                Type::Float(1)
            } else {
                types[0]
            }
        }
        "length" => {
            if types.len() != 1 || !float(types[0]) {
                return Err(invalid());
            }
            Type::Float(1)
        }
        "srgbToLinear" | "linearToSrgb" => {
            if types != [Type::Float(3)] {
                return Err(invalid());
            }
            Type::Float(3)
        }
        "min" | "max" | "pow" | "mod" | "step" => {
            if types.len() != 2 {
                return Err(invalid());
            }
            let ty = arithmetic_type(types[0], types[1]).ok_or_else(invalid)?;
            if !float(ty) {
                return Err(invalid());
            }
            ty
        }
        "clamp" | "mix" | "smoothstep" => {
            if types.len() != 3 {
                return Err(invalid());
            }
            let ty = arithmetic_type(types[0], types[1])
                .and_then(|t| arithmetic_type(t, types[2]))
                .ok_or_else(invalid)?;
            if !float(ty) {
                return Err(invalid());
            }
            ty
        }
        _ => {
            if types.len() != 1 || !float(types[0]) {
                return Err(invalid());
            }
            types[0]
        }
    };
    // Convert scalar arguments explicitly when a component-wise builtin receives vectors.
    let arg_code = if matches!(
        name,
        "min" | "max" | "pow" | "mod" | "step" | "clamp" | "mix" | "smoothstep"
    ) {
        args.iter()
            .map(|a| format!("{}({})", ty.name(), a.code))
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        arg_code.to_owned()
    };
    let backend = match name {
        "normalize" => "valle_normalize",
        "sqrt" => "valle_sqrt",
        "inversesqrt" => "valle_inversesqrt",
        "pow" => "valle_pow",
        "mod" => "valle_mod",
        "log" => "valle_log",
        "log2" => "valle_log2",
        "smoothstep" => "valle_smoothstep",
        "srgbToLinear" => "valle_decode_srgb",
        "linearToSrgb" => "valle_encode_srgb",
        _ => name,
    };
    Ok(Value {
        ty,
        code: format!("{backend}({arg_code})"),
        cost: cost.add(Cost::work(
            args.iter().map(|a| a.ty.width()).sum::<usize>() * 8,
        )),
        constant: None,
    })
}

fn assignment_root(expr: &Expr) -> Option<&str> {
    match &expr.kind {
        ExprKind::Name(name) => Some(name),
        ExprKind::Swizzle(base, _) | ExprKind::Index(base, _) => assignment_root(base),
        _ => None,
    }
}
fn validate_write_components(expr: &Expr) -> Result<(), ShaderDiagnostic> {
    match &expr.kind {
        ExprKind::Swizzle(base, fields) => {
            if fields.bytes().collect::<BTreeSet<_>>().len() != fields.len() {
                return Err(error(
                    expr.pos,
                    "assignment swizzle cannot repeat components",
                ));
            }
            validate_write_components(base)
        }
        ExprKind::Index(base, _) => validate_write_components(base),
        _ => Ok(()),
    }
}
fn validate_name(
    manifest: &ShaderManifest,
    name: &str,
    pos: usize,
) -> Result<(), ShaderDiagnostic> {
    super::manifest::validate_identifier(name, "source")
        .map_err(|_| error(pos, format!("reserved or invalid name `{name}`")))?;
    if Type::parse(name).is_some()
        || builtin_names().contains(&name)
        || name == "content"
        || name == "resolution"
        || manifest.inputs.iter().any(|i| i.name == name)
        || manifest.uniforms.iter().any(|u| u.name == name)
    {
        return Err(error(pos, format!("reserved name `{name}`")));
    }
    Ok(())
}
fn error(pos: usize, message: impl Into<String>) -> ShaderDiagnostic {
    ShaderDiagnostic::new(DiagnosticCode::DialectViolation, pos.to_string(), message)
}

fn prelude(manifest: &ShaderManifest) -> String {
    let mut output = String::from("// generated by valle-shader\nuniform shader content;\n");
    for input in &manifest.inputs {
        output.push_str(&format!("uniform shader {};\n", input.name));
    }
    output.push_str("uniform float2 resolution;\n");
    for input in &manifest.inputs {
        output.push_str(&format!("uniform float2 valle_size_{};\n", input.name));
    }
    for u in &manifest.uniforms {
        let name = if u.uniform_type == UniformType::Bool {
            format!("valle_uniform_{}", u.name)
        } else {
            u.name.clone()
        };
        output.push_str(&format!("uniform {} {name};\n", u.uniform_type.sksl_name()));
    }
    output.push_str(include_str!("color.sksl"));
    output.push_str(&safe_math());
    output.push_str("float4 valle_unpremul(half4 c) { return c.a > 0.0 ? float4(valle_working_to_linear_srgb(c.rgb / c.a), c.a) : float4(0.0); }\n");
    output.push_str("float4 sampleContent(float2 uv) { return valle_unpremul(content.eval(valle_finite(uv * resolution))); }\n");
    for input in &manifest.inputs {
        let name = &input.name;
        if input.kind == super::InputKind::Data {
            let address = match input.wrap {
                super::InputWrap::Clamp => {
                    format!("p = clamp(p, float2(0.0), valle_size_{name} - 1.0);")
                }
                super::InputWrap::Repeat => format!("p = mod(p, valle_size_{name});"),
                super::InputWrap::Mirror => format!(
                    "p = mod(p, 2.0 * valle_size_{name}); p = min(p, 2.0 * valle_size_{name} - 1.0 - p);"
                ),
            };
            output.push_str(&format!(
                "float4 valle_texel_{name}(float2 p) {{ {address} p += 0.5; \
                 float2 row = float2(0.0, valle_size_{name}.y); float2 column = float2(valle_size_{name}.x, 0.0); \
                 float4 hi = float4({name}.eval(p).a, {name}.eval(p + row).a, {name}.eval(p + 2.0 * row).a, {name}.eval(p + 3.0 * row).a); \
                 p += column; \
                 float4 lo = float4({name}.eval(p).a, {name}.eval(p + row).a, {name}.eval(p + 2.0 * row).a, {name}.eval(p + 3.0 * row).a); \
                 return (floor(hi * 255.0 + 0.5) * 256.0 + floor(lo * 255.0 + 0.5)) / 65535.0; }}\n"
            ));
            let body = match input.sampling {
                super::InputSampling::Nearest => format!(
                    "return valle_texel_{name}(floor(valle_finite(uv * valle_size_{name})));"
                ),
                super::InputSampling::Linear => format!(
                    "float2 p = valle_finite(uv * valle_size_{name}) - 0.5; float2 lo = floor(p); float2 t = fract(p); \
                     return mix(mix(valle_texel_{name}(lo), valle_texel_{name}(lo + float2(1.0, 0.0)), t.x), \
                     mix(valle_texel_{name}(lo + float2(0.0, 1.0)), valle_texel_{name}(lo + 1.0), t.x), t.y);"
                ),
            };
            output.push_str(&format!("float4 sample_{name}(float2 uv) {{ {body} }}\n"));
            continue;
        }
        let body = match input.sampling {
            super::InputSampling::Nearest => {
                format!("return valle_unpremul({name}.eval(valle_finite(uv * valle_size_{name})));")
            }
            super::InputSampling::Linear => format!(
                "float2 p = valle_finite(uv * valle_size_{name}) - 0.5; float2 lo = floor(p); float2 t = fract(p); \
                 float4 a = float4({name}.eval(lo + float2(0.5, 0.5))); \
                 float4 b = float4({name}.eval(lo + float2(1.5, 0.5))); \
                 float4 c = float4({name}.eval(lo + float2(0.5, 1.5))); \
                 float4 d = float4({name}.eval(lo + float2(1.5, 1.5))); \
                 return valle_unpremul(half4(mix(mix(a, b, t.x), mix(c, d, t.x), t.y)));"
            ),
        };
        output.push_str(&format!("float4 sample_{name}(float2 uv) {{ {body} }}\n"));
    }
    output
}

fn safe_math() -> String {
    let mut code = String::from(
        "float valle_finite(float x) { return (x >= -3.402823466e+38 && x <= 3.402823466e+38) ? x : 0.0; }\n\
         int valle_idiv(int a, int b) { return b == 0 || (a == (-2147483647 - 1) && b == -1) ? 0 : a / b; }\n\
         int valle_imod(int a, int b) { return b == 0 || (a == (-2147483647 - 1) && b == -1) ? 0 : a - (a / b) * b; }\n\
         float valle_div(float a, float b) { return b == 0.0 ? 0.0 : a / b; }\n\
         float valle_sqrt(float x) { return x <= 0.0 ? 0.0 : sqrt(x); }\n\
         float valle_inversesqrt(float x) { return x <= 0.0 ? 0.0 : inversesqrt(x); }\n\
         float valle_pow(float a, float b) { return a < 0.0 || (a == 0.0 && b <= 0.0) ? 0.0 : pow(a, b); }\n\
         float valle_mod(float a, float b) { return b == 0.0 ? 0.0 : a - b * floor(a / b); }\n\
         float valle_log(float x) { return x <= 0.0 ? 0.0 : log(x); }\n\
         float valle_log2(float x) { return x <= 0.0 ? 0.0 : log2(x); }\n\
         float valle_smoothstep(float a, float b, float x) { float t = clamp(valle_div(x-a, b-a), 0.0, 1.0); return t*t*(3.0-2.0*t); }\n\
         float valle_normalize(float x) { return x == 0.0 ? 0.0 : sign(x); }\n",
    );
    for n in 2..=4 {
        let ty = format!("float{n}");
        for (name, arity) in [
            ("finite", 1),
            ("div", 2),
            ("sqrt", 1),
            ("inversesqrt", 1),
            ("pow", 2),
            ("mod", 2),
            ("log", 1),
            ("log2", 1),
            ("smoothstep", 3),
        ] {
            let args = ["a", "b", "c"][..arity]
                .iter()
                .map(|a| format!("{ty} {a}"))
                .collect::<Vec<_>>()
                .join(", ");
            let values = ['x', 'y', 'z', 'w'][..n]
                .iter()
                .map(|field| {
                    let values = ["a", "b", "c"][..arity]
                        .iter()
                        .map(|a| format!("{a}.{field}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("valle_{name}({values})")
                })
                .collect::<Vec<_>>()
                .join(", ");
            code.push_str(&format!(
                "{ty} valle_{name}({args}) {{ return {ty}({values}); }}\n"
            ));
        }
        code.push_str(&format!("{ty} valle_normalize({ty} x) {{ float m = max(abs(x.x), abs(x.y)); {} if (m == 0.0) return {ty}(0.0); {ty} u = x / m; return u / sqrt(dot(u,u)); }}\n", if n >= 3 { format!("m = max(m, abs(x.z)); {}", if n == 4 { "m = max(m, abs(x.w));" } else { "" }) } else { String::new() }));
    }
    code
}
