//! Statements in component, helper and callback bodies: `let` and assignment, `if`/`else`,
//! early `return`, `for`, `for...of`, `while`, and `push` onto a list of JSX.
//!
//! A body runs once, while the scene is prepared. A condition known then takes one branch, and a
//! loop unrolls while its condition is known then. A condition that depends on the frame lowers
//! both branches and joins them: what they leave in bindings and what they return becomes a
//! `Select` on the condition, and the JSX they return or push gets the condition as its
//! visibility. The body stays one fixed-topology function of the time.

use super::*;
use oxc::ast::ast::{
    AssignmentExpression, AssignmentOperator, AssignmentTarget, CallExpression, ForOfStatement,
    ForStatement, ForStatementInit, ForStatementLeft, IfStatement, ReturnStatement,
    SimpleAssignmentTarget, UpdateExpression, UpdateOperator, VariableDeclaration,
    VariableDeclarationKind,
};
use valle_motion::expr::ExprType;

/// Iterations one loop may unroll.
const MAX_LOOP_ITERATIONS: usize = 10_000;

const LOOP_CONDITION: &str = "a loop unrolls while the scene is prepared, so its condition \
     cannot depend on the frame; choose values with `cond ? a : b`, or build the list with \
     `.map`";

/// What a body returns: one value of the time, or JSX.
#[derive(Clone, Copy)]
enum BodyKind<'p> {
    Value,
    /// Nodes of the first `return` without a key of their own are keyed under `path`, and those
    /// of each later `return` under a path of their own.
    Nodes {
        path: &'p str,
    },
}

enum BodyValue {
    Value(ExprId),
    Nodes(Vec<PendingNode>),
}

/// The JSX that every path through a body returns.
pub(super) struct NodeBody {
    pub(super) nodes: Vec<PendingNode>,
    /// The argument of the body's `return`, when it has only one.
    pub(super) only_return: Option<Span>,
    /// Whether every `return` gives a literal fragment `<>…</>`.
    pub(super) fragment: bool,
}

/// What the paths through the statements lowered so far have returned.
enum Returned {
    /// None has.
    No,
    /// The paths where the condition holds have returned the value; the others go on.
    When(ExprId, BodyValue),
    /// Every path has.
    Always(BodyValue),
}

/// A `break` or `continue` that an unrolling loop takes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Jump {
    Break,
    Continue,
}

struct Outcome {
    returned: Returned,
    jump: Option<Jump>,
}

impl Outcome {
    /// Nothing returned, and the next statement runs.
    fn next() -> Self {
        Self {
            returned: Returned::No,
            jump: None,
        }
    }
}

/// The names one block declares, and the bindings they hide until it ends.
#[derive(Default)]
struct BlockScope<'s> {
    hidden: Vec<(String, NameBinding<'s>)>,
    lets: BTreeSet<String>,
    consts: BTreeSet<String>,
}

struct Body<'a, 's> {
    kind: BodyKind<'a>,
    /// The body's owner, as messages name it: "this component", "helper `f`".
    role: &'a str,
    /// Parameters, which the body may assign as it may its own `let` bindings.
    params: BTreeSet<String>,
    scopes: Vec<BlockScope<'s>>,
    /// For each enclosing loop, how many frame-dependent branches have been entered since it
    /// began. A `break` or `continue` may end an iteration only when there are none.
    loops: Vec<usize>,
    returns: usize,
    last_return: Option<Span>,
    fragment: bool,
}

impl<'a, 's> Body<'a, 's> {
    fn new(kind: BodyKind<'a>, role: &'a str, params: BTreeSet<String>) -> Self {
        Self {
            kind,
            role,
            params,
            scopes: vec![BlockScope::default()],
            loops: Vec::new(),
            returns: 0,
            last_return: None,
            fragment: true,
        }
    }

    /// Record `name` as declared in the current block.
    fn declared(&mut self, name: String, mutable: bool) {
        let scope = self.scopes.last_mut().expect("a body has a scope");
        if mutable {
            scope.consts.remove(&name);
            scope.lets.insert(name);
        } else {
            scope.lets.remove(&name);
            scope.consts.insert(name);
        }
    }
}

/// What an unrolled loop runs over.
enum Iterations<'s> {
    /// `for` and `while`: a condition before each iteration, and an update after it.
    Test {
        test: Option<&'s Expression<'s>>,
        update: Option<&'s Expression<'s>>,
    },
    /// `for...of` over a list known while the scene is prepared.
    Items {
        items: std::vec::IntoIter<serde_json::Value>,
        pattern: &'s BindingPattern<'s>,
        names: Vec<String>,
        mutable: bool,
    },
}

impl<'s> Compiler<'s> {
    /// Lower a body that returns JSX. `None` means that a diagnostic was reported or that the
    /// body has no `return`; callers word the latter.
    pub(super) fn lower_node_body(
        &mut self,
        statements: &'s [Statement<'s>],
        path: &str,
        role: &str,
        params: BTreeSet<String>,
    ) -> Option<NodeBody> {
        let mut body = Body::new(BodyKind::Nodes { path }, role, params);
        let BodyValue::Nodes(nodes) = self.lower_body(statements, &mut body)? else {
            unreachable!("a JSX body returns nodes");
        };
        Some(NodeBody {
            nodes,
            only_return: body.last_return.filter(|_| body.returns == 1),
            fragment: body.fragment,
        })
    }

    /// Lower a body that returns one value of the time.
    pub(super) fn lower_value_body(
        &mut self,
        statements: &'s [Statement<'s>],
        role: &str,
        params: BTreeSet<String>,
    ) -> Option<ExprId> {
        let mut body = Body::new(BodyKind::Value, role, params);
        let BodyValue::Value(value) = self.lower_body(statements, &mut body)? else {
            unreachable!("a value body returns one value");
        };
        Some(value)
    }

    fn lower_body(
        &mut self,
        statements: &'s [Statement<'s>],
        body: &mut Body<'_, 's>,
    ) -> Option<BodyValue> {
        match self.lower_statements(statements, body)?.returned {
            Returned::Always(value) => Some(value),
            Returned::No => None,
            Returned::When(..) => {
                let span = statements.last().map_or(Span::new(0, 0), GetSpan::span);
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!(
                        "{} does not return on every path; end it with a `return`",
                        body.role
                    ),
                );
                None
            }
        }
    }

    fn lower_statements(
        &mut self,
        statements: &'s [Statement<'s>],
        body: &mut Body<'_, 's>,
    ) -> Option<Outcome> {
        // A function declaration is bound from the top of its block, as in JavaScript.
        for statement in statements {
            if let Statement::FunctionDeclaration(function) = statement
                && let Some(id) = &function.id
            {
                let name = id.name.to_string();
                if RESERVED_INTRINSICS.contains(&name.as_str()) {
                    self.illegal(DiagCode::GrammarForbidden, id.span(), SHADOWED_INTRINSIC);
                    continue;
                }
                self.hide(&name, body);
                self.bindings.unbind(&name);
                self.bindings
                    .local_functions
                    .insert(name.clone(), AuthoredFn::Declared(function.as_ref()));
                body.declared(name, false);
            }
        }
        let mut returned = Returned::No;
        let mut failed = false;
        for statement in statements {
            // What follows a return on every path never runs.
            if matches!(returned, Returned::Always(_)) {
                break;
            }
            let Some(outcome) = self.lower_statement(statement, body) else {
                failed = true;
                continue;
            };
            returned = self.sequence(returned, outcome.returned, statement.span(), body)?;
            if outcome.jump.is_some() {
                return (!failed).then_some(Outcome {
                    returned,
                    jump: outcome.jump,
                });
            }
        }
        (!failed).then_some(Outcome {
            returned,
            jump: None,
        })
    }

    fn lower_statement(
        &mut self,
        statement: &'s Statement<'s>,
        body: &mut Body<'_, 's>,
    ) -> Option<Outcome> {
        match statement {
            Statement::VariableDeclaration(declaration) => {
                self.declare(declaration, body)?;
                Some(Outcome::next())
            }
            // Function declarations are bound before their block runs, and types do nothing.
            Statement::FunctionDeclaration(_)
            | Statement::EmptyStatement(_)
            | Statement::TSTypeAliasDeclaration(_)
            | Statement::TSInterfaceDeclaration(_) => Some(Outcome::next()),
            Statement::ReturnStatement(statement) => Some(Outcome {
                returned: Returned::Always(self.lower_return(statement, body)?),
                jump: None,
            }),
            Statement::IfStatement(statement) => self.lower_if(statement, body),
            Statement::BlockStatement(block) => self.lower_block(&block.body, body),
            Statement::ExpressionStatement(statement) => {
                self.lower_effect(&statement.expression, body)?;
                Some(Outcome::next())
            }
            Statement::ForStatement(statement) => self.lower_for(statement, body),
            Statement::ForOfStatement(statement) => self.lower_for_of(statement, body),
            Statement::WhileStatement(statement) => self.unroll(
                Iterations::Test {
                    test: Some(&statement.test),
                    update: None,
                },
                &statement.body,
                statement.span,
                body,
            ),
            Statement::BreakStatement(statement) => {
                self.lower_jump(Jump::Break, statement.label.is_some(), statement.span, body)
            }
            Statement::ContinueStatement(statement) => self.lower_jump(
                Jump::Continue,
                statement.label.is_some(),
                statement.span,
                body,
            ),
            other => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    other.span(),
                    statement_advice(other),
                );
                None
            }
        }
    }

    fn declare(
        &mut self,
        declaration: &'s VariableDeclaration<'s>,
        body: &mut Body<'_, 's>,
    ) -> Option<()> {
        let mutable = match declaration.kind {
            VariableDeclarationKind::Const => false,
            VariableDeclarationKind::Let => true,
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    declaration.span,
                    "declare locals with `const`, or with `let` when they are assigned again",
                );
                return None;
            }
        };
        for declarator in &declaration.declarations {
            let mut names = Vec::new();
            pattern_names(&declarator.id, &mut names);
            for name in &names {
                self.hide(name, body);
            }
            match &declarator.init {
                Some(initializer) => self.bind_local_pattern(&declarator.id, initializer),
                // `let x;` holds `undefined` until it is assigned.
                None if mutable => {
                    for name in &names {
                        self.bind_static(name.clone(), serde_json::Value::Null);
                    }
                }
                None => self.illegal(
                    DiagCode::GrammarForbidden,
                    declarator.span,
                    "a `const` needs an initializer; declare a value assigned later with `let`",
                ),
            }
            for name in names {
                body.declared(name, mutable);
            }
        }
        Some(())
    }

    fn lower_return(
        &mut self,
        statement: &'s ReturnStatement<'s>,
        body: &mut Body<'_, 's>,
    ) -> Option<BodyValue> {
        let Some(argument) = &statement.argument else {
            self.illegal(
                DiagCode::GrammarForbidden,
                statement.span,
                match body.kind {
                    BodyKind::Value => "`return` needs a value",
                    BodyKind::Nodes { .. } => "`return` needs a value; return `null` for no JSX",
                },
            );
            return None;
        };
        let index = body.returns;
        body.returns += 1;
        body.last_return = Some(argument.span());
        match body.kind {
            BodyKind::Value => self.lower_expr(argument).map(BodyValue::Value),
            BodyKind::Nodes { path } => {
                body.fragment &= matches!(peel_expr(argument), Expression::JSXFragment(_));
                let path = match index {
                    0 => path.to_owned(),
                    index => format!("{path}.return{index}"),
                };
                self.lower_node_expression(argument, &path)
                    .map(BodyValue::Nodes)
            }
        }
    }

    fn lower_if(
        &mut self,
        statement: &'s IfStatement<'s>,
        body: &mut Body<'_, 's>,
    ) -> Option<Outcome> {
        if let Some(value) = self.eval_static(&statement.test) {
            return match (js_truthy(&value), &statement.alternate) {
                (true, _) => self.lower_branch(&statement.consequent, body),
                (false, Some(alternate)) => self.lower_branch(alternate, body),
                (false, None) => Some(Outcome::next()),
            };
        }
        let condition = self.lower_condition(&statement.test)?;
        let before = self.bindings.snapshot_frame();
        // Both branches start from the JSX pushed before the `if`.
        let pushed = before
            .children
            .iter()
            .map(|(name, nodes)| (name.clone(), nodes.len()))
            .collect::<BTreeMap<_, _>>();
        for branches in &mut body.loops {
            *branches += 1;
        }
        let then = self.lower_branch(&statement.consequent, body);
        let after_then = self.bindings.snapshot_frame();
        self.bindings.restore_frame(before);
        let otherwise = match &statement.alternate {
            Some(alternate) => self.lower_branch(alternate, body),
            None => Some(Outcome::next()),
        };
        for branches in &mut body.loops {
            *branches -= 1;
        }
        let (then, otherwise) = (then?, otherwise?);
        match (&then.returned, &otherwise.returned) {
            // Only the paths that go on need what the branches bound.
            (Returned::Always(_), _) => {}
            (_, Returned::Always(_)) => self.bindings.restore_frame(after_then),
            _ => {
                let after_else = self.bindings.snapshot_frame();
                self.join_bindings(condition, &pushed, &after_then, &after_else, statement.span)?;
            }
        }
        let returned = self.join(
            condition,
            then.returned,
            otherwise.returned,
            statement.span,
            body,
        )?;
        Some(Outcome {
            returned,
            jump: None,
        })
    }

    /// A branch runs in a block of its own, braces or not.
    fn lower_branch(
        &mut self,
        statement: &'s Statement<'s>,
        body: &mut Body<'_, 's>,
    ) -> Option<Outcome> {
        match statement {
            Statement::BlockStatement(block) => self.lower_block(&block.body, body),
            statement => {
                body.scopes.push(BlockScope::default());
                let outcome = self.lower_statement(statement, body);
                self.end_scope(body);
                outcome
            }
        }
    }

    fn lower_block(
        &mut self,
        statements: &'s [Statement<'s>],
        body: &mut Body<'_, 's>,
    ) -> Option<Outcome> {
        body.scopes.push(BlockScope::default());
        let outcome = self.lower_statements(statements, body);
        self.end_scope(body);
        outcome
    }

    /// Leave a block: its names give way to what they hid.
    fn end_scope(&mut self, body: &mut Body<'_, 's>) {
        let scope = body.scopes.pop().expect("a block has a scope");
        for (name, binding) in scope.hidden.into_iter().rev() {
            self.bindings.put_name(&name, binding);
        }
    }

    /// Keep what a declaration in the current block hides, to bind again when the block ends.
    fn hide(&mut self, name: &str, body: &mut Body<'_, 's>) {
        let binding = self.bindings.name_binding(name);
        let scope = body.scopes.last_mut().expect("a body has a scope");
        if !scope.hidden.iter().any(|(hidden, _)| hidden == name) {
            scope.hidden.push((name.to_owned(), binding));
        }
    }

    /// A frame-dependent condition as a boolean. A number or a string is true as in JavaScript.
    fn lower_condition(&mut self, test: &'s Expression<'s>) -> Option<ExprId> {
        let id = self.lower_expr(test)?;
        let span = test.span();
        let falsy = match self.expr_arena.types[id.0 as usize] {
            Some(ExprType::Bool) => return Some(id),
            Some(ExprType::Number) => MotionValue::Number(0.0),
            Some(ExprType::String) => MotionValue::Str(String::new()),
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    "a frame-dependent condition must be a boolean, a number or a string",
                );
                return None;
            }
        };
        let rhs = self.push(Expr::Const { value: falsy }, span);
        Some(self.push(
            Expr::Compare {
                op: CompareOp::NotEq,
                lhs: id,
                rhs,
            },
            span,
        ))
    }

    /// Join what the two branches of a frame-dependent `if` returned.
    fn join(
        &mut self,
        condition: ExprId,
        then: Returned,
        otherwise: Returned,
        span: Span,
        body: &Body<'_, 's>,
    ) -> Option<Returned> {
        use Returned::{Always, No, When};
        Some(match (then, otherwise) {
            (No, No) => No,
            (Always(a), Always(b)) => Always(self.choose(condition, a, b, span, body)?),
            (Always(a), No) => When(condition, a),
            (No, Always(b)) => When(self.not_expr(condition, span), b),
            (When(d, a), No) => When(self.and_expr(condition, d, span), a),
            (No, When(e, b)) => {
                let not = self.not_expr(condition, span);
                When(self.and_expr(not, e, span), b)
            }
            (When(d, a), When(e, b)) => When(
                self.select_expr(condition, d, e, span),
                self.choose(condition, a, b, span, body)?,
            ),
            (Always(a), When(e, b)) => When(
                self.or_expr(condition, e, span),
                self.choose(condition, a, b, span, body)?,
            ),
            (When(d, a), Always(b)) => {
                let not = self.not_expr(condition, span);
                When(
                    self.or_expr(not, d, span),
                    self.choose(condition, a, b, span, body)?,
                )
            }
        })
    }

    /// Follow what earlier statements returned with what the next one returns on the paths that
    /// went on.
    fn sequence(
        &mut self,
        earlier: Returned,
        next: Returned,
        span: Span,
        body: &Body<'_, 's>,
    ) -> Option<Returned> {
        use Returned::{Always, No, When};
        Some(match (earlier, next) {
            (No, next) => next,
            (earlier @ Always(_), _) | (earlier @ When(..), No) => earlier,
            (When(c, a), Always(b)) => Always(self.choose(c, a, b, span, body)?),
            (When(c, a), When(d, b)) => {
                When(self.or_expr(c, d, span), self.choose(c, a, b, span, body)?)
            }
        })
    }

    /// The return that `condition` chooses: a value through `Select`, and JSX by showing the
    /// nodes of one return and hiding those of the other.
    fn choose(
        &mut self,
        condition: ExprId,
        when_true: BodyValue,
        when_false: BodyValue,
        span: Span,
        body: &Body<'_, 's>,
    ) -> Option<BodyValue> {
        match (when_true, when_false) {
            (BodyValue::Value(when_true), BodyValue::Value(when_false)) => {
                let (a, b) = (
                    self.expr_arena.types[when_true.0 as usize],
                    self.expr_arena.types[when_false.0 as usize],
                );
                if a != b {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        span,
                        format!(
                            "{} returns {} on one path and {} on another; return one kind of value",
                            body.role,
                            type_phrase(a),
                            type_phrase(b)
                        ),
                    );
                    return None;
                }
                Some(BodyValue::Value(
                    self.select_expr(condition, when_true, when_false, span),
                ))
            }
            (BodyValue::Nodes(mut when_true), BodyValue::Nodes(mut when_false)) => {
                self.apply_visibility(&mut when_true, condition);
                let not = self.not_expr(condition, span);
                self.apply_visibility(&mut when_false, not);
                when_true.extend(when_false);
                Some(BodyValue::Nodes(when_true))
            }
            _ => unreachable!("a body returns one kind of value"),
        }
    }

    fn select_expr(
        &mut self,
        condition: ExprId,
        when_true: ExprId,
        when_false: ExprId,
        span: Span,
    ) -> ExprId {
        self.push(
            Expr::Select {
                condition,
                when_true,
                when_false,
            },
            span,
        )
    }

    fn or_expr(&mut self, lhs: ExprId, rhs: ExprId, span: Span) -> ExprId {
        let true_value = self.push(
            Expr::Const {
                value: MotionValue::Bool(true),
            },
            span,
        );
        self.select_expr(lhs, true_value, rhs, span)
    }

    /// After a frame-dependent `if` whose branches both go on, bind each name they left
    /// different to what the condition chooses. The bindings of the `else` branch are in place,
    /// and `pushed` holds how much JSX each list held before the `if`.
    fn join_bindings(
        &mut self,
        condition: ExprId,
        pushed: &BTreeMap<String, usize>,
        then: &BindingFrame<'s>,
        otherwise: &BindingFrame<'s>,
        span: Span,
    ) -> Option<()> {
        let mut names = then.names();
        names.extend(otherwise.names());
        let mut failed = false;
        for name in names {
            let (a, b) = (then.binding(&name), otherwise.binding(&name));
            if a.same(&b) {
                continue;
            }
            let (left, right) = match (self.binding_expr(&a, span), self.binding_expr(&b, span)) {
                (Some(x), Some(y)) => {
                    let (x_type, y_type) = (
                        self.expr_arena.types[x.0 as usize],
                        self.expr_arena.types[y.0 as usize],
                    );
                    if x_type == y_type {
                        let chosen = self.select_expr(condition, x, y, span);
                        self.bind_dynamic(name, chosen);
                        continue;
                    }
                    (
                        type_phrase(x_type).to_owned(),
                        type_phrase(y_type).to_owned(),
                    )
                }
                _ => {
                    if let Some(chosen) = self.join_tuples(condition, &a, &b, span) {
                        self.bind_dynamic_tuple(name, chosen);
                        continue;
                    }
                    if let (Some(xs), Some(ys)) = (a.nodes(), b.nodes()) {
                        let before = pushed.get(&name).copied().unwrap_or(0);
                        if let Some(nodes) = self.join_nodes(condition, before, xs, ys, span) {
                            self.bind_children(name, nodes);
                            continue;
                        }
                    }
                    (a.phrase(), b.phrase())
                }
            };
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!(
                    "this frame-dependent `if` leaves `{name}` as {left} after one branch and \
                     {right} after the other; the frame can choose between values of one kind, \
                     lists of one length, and JSX pushed onto a list"
                ),
            );
            failed = true;
        }
        (!failed).then_some(())
    }

    /// A binding as one expression, when it holds one value.
    fn binding_expr(&mut self, binding: &NameBinding<'s>, span: Span) -> Option<ExprId> {
        if binding.local_function.is_some() {
            return None;
        }
        if let Some(id) = binding.scalar {
            return Some(id);
        }
        let value = motion_value_from_json(binding.statik.as_ref()?)?;
        Some(self.push(Expr::Const { value }, span))
    }

    /// A binding as a list of expressions.
    fn binding_tuple(&mut self, binding: &NameBinding<'s>, span: Span) -> Option<Vec<ExprId>> {
        if let Some(values) = &binding.tuple {
            return Some(values.clone());
        }
        let serde_json::Value::Array(items) = binding.statik.as_ref()? else {
            return None;
        };
        items
            .iter()
            .map(|item| {
                let value = motion_value_from_json(item)?;
                Some(self.push(Expr::Const { value }, span))
            })
            .collect()
    }

    /// Choose element by element between two lists of one length and of the same kinds.
    fn join_tuples(
        &mut self,
        condition: ExprId,
        a: &NameBinding<'s>,
        b: &NameBinding<'s>,
        span: Span,
    ) -> Option<Vec<ExprId>> {
        let (xs, ys) = (self.binding_tuple(a, span)?, self.binding_tuple(b, span)?);
        let types = &self.expr_arena.types;
        if xs.len() != ys.len()
            || xs
                .iter()
                .zip(&ys)
                .any(|(x, y)| types[x.0 as usize] != types[y.0 as usize])
        {
            return None;
        }
        Some(
            xs.into_iter()
                .zip(ys)
                .map(|(x, y)| self.select_expr(condition, x, y, span))
                .collect(),
        )
    }

    /// Two lists that each extend the `before` nodes the list held before the `if` keep both
    /// branches' pushes, each visible when its branch is taken.
    fn join_nodes(
        &mut self,
        condition: ExprId,
        before: usize,
        then: &[PendingNode],
        otherwise: &[PendingNode],
        span: Span,
    ) -> Option<Vec<PendingNode>> {
        if before > then.len().min(otherwise.len())
            || then[..before]
                .iter()
                .zip(&otherwise[..before])
                .any(|(a, b)| a.key != b.key)
        {
            return None;
        }
        let mut nodes = then[..before].to_vec();
        let mut pushed_then = then[before..].to_vec();
        self.apply_visibility(&mut pushed_then, condition);
        let mut pushed_else = otherwise[before..].to_vec();
        let not = self.not_expr(condition, span);
        self.apply_visibility(&mut pushed_else, not);
        nodes.extend(pushed_then);
        nodes.extend(pushed_else);
        Some(nodes)
    }

    fn lower_for(
        &mut self,
        statement: &'s ForStatement<'s>,
        body: &mut Body<'_, 's>,
    ) -> Option<Outcome> {
        // The loop's own declarations end with it.
        body.scopes.push(BlockScope::default());
        let initialized = match &statement.init {
            Some(ForStatementInit::VariableDeclaration(declaration)) => {
                self.declare(declaration, body)
            }
            Some(init) => match init.as_expression() {
                Some(expression) => self.lower_effect(expression, body),
                None => Some(()),
            },
            None => Some(()),
        };
        let outcome = initialized.and_then(|()| {
            self.unroll(
                Iterations::Test {
                    test: statement.test.as_ref(),
                    update: statement.update.as_ref(),
                },
                &statement.body,
                statement.span,
                body,
            )
        });
        self.end_scope(body);
        outcome
    }

    fn lower_for_of(
        &mut self,
        statement: &'s ForOfStatement<'s>,
        body: &mut Body<'_, 's>,
    ) -> Option<Outcome> {
        let Some(serde_json::Value::Array(items)) = self.eval_static(&statement.right) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                statement.right.span(),
                "for...of input must be a prepare-time static array; frame values cannot choose topology",
            );
            return None;
        };
        if items.len() > MAX_STATIC_MAP_ITEMS
            || self.expanded_list_items.saturating_add(items.len()) > MAX_TOTAL_EXPANDED_LIST_ITEMS
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                statement.right.span(),
                format!(
                    "prepare-time for...of expands {} items; the per-loop limit is {MAX_STATIC_MAP_ITEMS} and the module budget is {MAX_TOTAL_EXPANDED_LIST_ITEMS}",
                    items.len()
                ),
            );
            return None;
        }
        let ForStatementLeft::VariableDeclaration(declaration) = &statement.left else {
            self.illegal(
                DiagCode::GrammarForbidden,
                statement.left.span(),
                "for...of declares its item, as in `for (const item of items)`",
            );
            return None;
        };
        let mutable = match declaration.kind {
            VariableDeclarationKind::Const => false,
            VariableDeclarationKind::Let => true,
            _ => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    declaration.span,
                    "declare the item with `const`, as in `for (const item of items)`",
                );
                return None;
            }
        };
        let [declarator] = &declaration.declarations[..] else {
            self.illegal(
                DiagCode::GrammarForbidden,
                declaration.span,
                "for...of declares one item",
            );
            return None;
        };
        let mut names = Vec::new();
        pattern_names(&declarator.id, &mut names);
        self.expanded_list_items += items.len();
        self.unroll(
            Iterations::Items {
                items: items.into_iter(),
                pattern: &declarator.id,
                names,
                mutable,
            },
            &statement.body,
            statement.span,
            body,
        )
    }

    /// Unroll a loop while the scene is prepared.
    fn unroll(
        &mut self,
        iterations: Iterations<'s>,
        statement: &'s Statement<'s>,
        span: Span,
        body: &mut Body<'_, 's>,
    ) -> Option<Outcome> {
        body.loops.push(0);
        let outcome = self.unroll_iterations(iterations, statement, span, body);
        body.loops.pop();
        outcome
    }

    fn unroll_iterations(
        &mut self,
        mut iterations: Iterations<'s>,
        statement: &'s Statement<'s>,
        span: Span,
        body: &mut Body<'_, 's>,
    ) -> Option<Outcome> {
        let mut returned = Returned::No;
        for iteration in 0.. {
            let item = match &mut iterations {
                Iterations::Test {
                    test: Some(test), ..
                } => {
                    let test = *test;
                    let Some(value) = self.eval_static(test) else {
                        self.illegal(DiagCode::GrammarForbidden, test.span(), LOOP_CONDITION);
                        return None;
                    };
                    if !js_truthy(&value) {
                        break;
                    }
                    None
                }
                Iterations::Test { test: None, .. } => None,
                Iterations::Items { items, .. } => match items.next() {
                    Some(item) => Some(item),
                    None => break,
                },
            };
            if iteration == MAX_LOOP_ITERATIONS {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!(
                        "this loop runs more than {MAX_LOOP_ITERATIONS} times while the scene is \
                         prepared; build long lists with `.map`"
                    ),
                );
                return None;
            }
            body.scopes.push(BlockScope::default());
            if let (
                Some(item),
                Iterations::Items {
                    pattern,
                    names,
                    mutable,
                    ..
                },
            ) = (item, &iterations)
            {
                for name in names {
                    self.hide(name, body);
                }
                self.bind_static_pattern(pattern, item);
                for name in names {
                    body.declared(name.clone(), *mutable);
                }
            }
            let outcome = match statement {
                Statement::BlockStatement(block) => self.lower_statements(&block.body, body),
                statement => self.lower_statement(statement, body),
            };
            self.end_scope(body);
            let outcome = outcome?;
            returned = self.sequence(returned, outcome.returned, span, body)?;
            if matches!(returned, Returned::Always(_)) || outcome.jump == Some(Jump::Break) {
                break;
            }
            if let Iterations::Test {
                update: Some(update),
                ..
            } = &iterations
            {
                self.lower_effect(*update, body)?;
            }
        }
        Some(Outcome {
            returned,
            jump: None,
        })
    }

    fn lower_jump(
        &mut self,
        jump: Jump,
        labeled: bool,
        span: Span,
        body: &Body<'_, 's>,
    ) -> Option<Outcome> {
        let word = match jump {
            Jump::Break => "break",
            Jump::Continue => "continue",
        };
        let message = match body.loops.last() {
            _ if labeled => format!("a labeled `{word}` is not supported; restructure the loops"),
            Some(0) => {
                return Some(Outcome {
                    returned: Returned::No,
                    jump: Some(jump),
                });
            }
            Some(_) => format!(
                "`{word}` under a frame-dependent condition cannot end a loop that unrolls \
                 while the scene is prepared; choose values with the condition instead"
            ),
            None => format!("`{word}` must be inside a loop"),
        };
        self.illegal(DiagCode::GrammarForbidden, span, message);
        None
    }

    /// An expression statement: an assignment, or a `push` of JSX onto a list.
    fn lower_effect(
        &mut self,
        expression: &'s Expression<'s>,
        body: &mut Body<'_, 's>,
    ) -> Option<()> {
        match strip_parens(expression) {
            Expression::AssignmentExpression(assignment) => self.lower_assignment(assignment, body),
            Expression::UpdateExpression(update) => self.lower_update(update, body),
            Expression::SequenceExpression(sequence) => {
                for expression in &sequence.expressions {
                    self.lower_effect(expression, body)?;
                }
                Some(())
            }
            Expression::CallExpression(call)
                if matches!(body.kind, BodyKind::Nodes { .. })
                    && matches!(&call.callee, Expression::StaticMemberExpression(member)
                        if member.property.name == "push") =>
            {
                self.lower_push(call, body)
            }
            other => {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    other.span(),
                    "a body runs no effects, so this expression does nothing as a statement; \
                     bind it with `const`, assign it to a `let`, or return it",
                );
                None
            }
        }
    }

    /// `items.push(<View />)`: append JSX to a local list, which JSX children then read.
    fn lower_push(&mut self, call: &'s CallExpression<'s>, body: &Body<'_, 's>) -> Option<()> {
        let Expression::StaticMemberExpression(member) = &call.callee else {
            unreachable!("a push is a member call");
        };
        let Expression::Identifier(target) = &member.object else {
            self.illegal(
                DiagCode::GrammarForbidden,
                member.object.span(),
                "push JSX onto a local list, as in `items.push(<View />)`",
            );
            return None;
        };
        let [argument] = &call.arguments[..] else {
            self.illegal(
                DiagCode::GrammarForbidden,
                call.span,
                "`push` takes one JSX argument",
            );
            return None;
        };
        let Some(argument) = argument.as_expression() else {
            self.illegal(
                DiagCode::GrammarForbidden,
                argument.span(),
                "`push` takes one JSX argument, not a spread",
            );
            return None;
        };
        let name = target.name.to_string();
        let empty = matches!(
            self.bindings.statics.get(&name),
            Some(serde_json::Value::Array(items)) if items.is_empty()
        );
        if !empty && !self.bindings.children.contains_key(&name) {
            self.illegal(
                DiagCode::GrammarForbidden,
                target.span,
                format!(
                    "`{name}` must start as an empty list, `const {name} = []`, before JSX is \
                     pushed onto it"
                ),
            );
            return None;
        }
        let index = self.bindings.children.get(&name).map_or(0, Vec::len);
        // JSX pushed in a loop is a list item, as in a `.map`.
        let in_loop = usize::from(!body.loops.is_empty());
        self.list_depth += in_loop;
        let nodes = self.lower_node_expression(argument, &format!("{name}.{index}"));
        self.list_depth -= in_loop;
        let mut nodes = nodes?;
        self.bindings.statics.remove(&name);
        self.bindings
            .children
            .entry(name)
            .or_default()
            .append(&mut nodes);
        Some(())
    }

    fn lower_assignment(
        &mut self,
        assignment: &'s AssignmentExpression<'s>,
        body: &Body<'_, 's>,
    ) -> Option<()> {
        let AssignmentTarget::AssignmentTargetIdentifier(target) = &assignment.left else {
            self.illegal(
                DiagCode::GrammarForbidden,
                assignment.left.span(),
                "assign a whole `let` binding; an object or array cannot change in place, so \
                 build a new one",
            );
            return None;
        };
        let name = target.name.to_string();
        self.check_assignable(&name, target.span, body)?;
        if assignment.operator == AssignmentOperator::Assign {
            self.bind_local_value(name, &assignment.right);
            return Some(());
        }
        let operator = assignment.operator.to_binary_operator().filter(|operator| {
            matches!(
                operator,
                BinaryOperator::Addition
                    | BinaryOperator::Subtraction
                    | BinaryOperator::Multiplication
                    | BinaryOperator::Division
                    | BinaryOperator::Remainder
            )
        });
        let Some(operator) = operator else {
            let text = assignment.operator.as_str();
            self.illegal(
                DiagCode::GrammarForbidden,
                assignment.span,
                format!(
                    "`{text}` is not supported; write `{name} = {name} {} …`",
                    text.trim_end_matches('=')
                ),
            );
            return None;
        };
        self.lower_compound(name, operator, Some(&assignment.right), assignment.span)
    }

    fn lower_update(
        &mut self,
        update: &'s UpdateExpression<'s>,
        body: &Body<'_, 's>,
    ) -> Option<()> {
        let SimpleAssignmentTarget::AssignmentTargetIdentifier(target) = &update.argument else {
            self.illegal(
                DiagCode::GrammarForbidden,
                update.span,
                "only a `let` binding can be incremented or decremented",
            );
            return None;
        };
        let name = target.name.to_string();
        self.check_assignable(&name, target.span, body)?;
        let operator = match update.operator {
            UpdateOperator::Increment => BinaryOperator::Addition,
            UpdateOperator::Decrement => BinaryOperator::Subtraction,
        };
        self.lower_compound(name, operator, None, update.span)
    }

    /// `name op= right`, or `name op= 1` for `++` and `--`. JavaScript applies the operator while
    /// the scene is prepared when both sides are known then.
    fn lower_compound(
        &mut self,
        name: String,
        operator: BinaryOperator,
        right: Option<&'s Expression<'s>>,
        span: Span,
    ) -> Option<()> {
        if self.bindings.statics.contains_key(&name)
            && right.is_none_or(|right| !self.shadowed_by_dynamic(right))
        {
            let code = match right {
                Some(right) => format!("{name} {} ({})", operator.as_str(), self.js(right)),
                None => format!("(+{name}) {} 1", operator.as_str()),
            };
            match self
                .sandbox
                .eval_json(&code, &self.static_bindings(), "prepare")
            {
                Ok(value) => {
                    self.bind_static(name, value);
                    return Some(());
                }
                Err(diagnostic) if diagnostic.code == DiagCode::BuiltinRejected => {
                    self.push_diagnostic(from_motion_diagnostic(self.source, diagnostic, span));
                    return None;
                }
                // The right side reads what only lowering knows, such as a local helper.
                Err(_) => {}
            }
        }
        let current = self.bindings.name_binding(&name);
        let Some(lhs) = self.binding_expr(&current, span) else {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                format!(
                    "`{name}` holds {}, so `{}` cannot apply to it",
                    current.phrase(),
                    operator.as_str()
                ),
            );
            return None;
        };
        let rhs = match right {
            Some(right) => self.lower_expr(right)?,
            None => self.push(
                Expr::Const {
                    value: MotionValue::Number(1.0),
                },
                span,
            ),
        };
        let value = match operator {
            BinaryOperator::Addition => self.push(Expr::Add { lhs, rhs }, span),
            BinaryOperator::Subtraction => self.push(Expr::Sub { lhs, rhs }, span),
            BinaryOperator::Multiplication => self.push(Expr::Mul { lhs, rhs }, span),
            BinaryOperator::Division => self.push(Expr::Div { lhs, rhs }, span),
            BinaryOperator::Remainder => {
                self.lower_remainder(lhs, rhs, right.expect("`%=` has a right side"), span)?
            }
            _ => unreachable!("compound assignments are arithmetic"),
        };
        self.bind_dynamic(name, value);
        Some(())
    }

    fn check_assignable(&mut self, name: &str, span: Span, body: &Body<'_, 's>) -> Option<()> {
        for scope in body.scopes.iter().rev() {
            if scope.lets.contains(name) {
                return Some(());
            }
            if scope.consts.contains(name) {
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    format!("`{name}` is a `const`; declare it with `let` to assign it"),
                );
                return None;
            }
        }
        if body.params.contains(name) {
            return Some(());
        }
        self.illegal(
            DiagCode::GrammarForbidden,
            span,
            format!(
                "{} assigns only its own `let` bindings and parameters, and `{name}` is \
                 neither; declare `let {name} = …` here",
                body.role
            ),
        );
        None
    }
}

impl<'s> BindingFrame<'s> {
    /// Every name the frame binds.
    fn names(&self) -> BTreeSet<String> {
        self.local_functions
            .keys()
            .chain(self.scalars.keys())
            .chain(self.paints.keys())
            .chain(self.tuples.keys())
            .chain(self.instance_objects.keys())
            .chain(self.statics.keys())
            .chain(self.objects.keys())
            .chain(self.children.keys())
            .chain(self.times.keys())
            .chain(self.captions.keys())
            .cloned()
            .collect()
    }

    /// Everything the frame binds to `name`.
    fn binding(&self, name: &str) -> NameBinding<'s> {
        NameBinding {
            local_function: self.local_functions.get(name).copied(),
            scalar: self.scalars.get(name).copied(),
            paint: self.paints.get(name).cloned(),
            tuple: self.tuples.get(name).cloned(),
            instance_object: self.instance_objects.get(name).cloned(),
            statik: self.statics.get(name).cloned(),
            object: self.objects.get(name).cloned(),
            children: self.children.get(name).cloned(),
            prop_rest: None,
            time: self.times.get(name).cloned(),
            caption: self.captions.get(name).cloned(),
        }
    }
}

impl NameBinding<'_> {
    /// Whether two bindings hold the same values. Functions and objects are the same when they
    /// are the same syntax in the same scope, and JSX when it is the same nodes.
    fn same(&self, other: &Self) -> bool {
        self.scalar == other.scalar
            && self.statik == other.statik
            && self.tuple == other.tuple
            && self.time == other.time
            && self.caption == other.caption
            && self.paint == other.paint
            && self.instance_object == other.instance_object
            && self.prop_rest == other.prop_rest
            && self.local_function.map(|function| function.span())
                == other.local_function.map(|function| function.span())
            && match (&self.object, &other.object) {
                (Some(a), Some(b)) => a.same_object(b),
                (a, b) => a.is_none() && b.is_none(),
            }
            && match (&self.children, &other.children) {
                (Some(a), Some(b)) => {
                    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.key == b.key)
                }
                (a, b) => a.is_none() && b.is_none(),
            }
    }

    /// The JSX a list holds. An empty array holds none yet.
    fn nodes(&self) -> Option<&[PendingNode]> {
        match (&self.children, &self.statik) {
            (Some(nodes), _) => Some(nodes),
            (None, Some(serde_json::Value::Array(items))) if items.is_empty() => Some(&[]),
            _ => None,
        }
    }

    /// What the binding holds, as messages name it.
    fn phrase(&self) -> String {
        if self.children.is_some() {
            "JSX".into()
        } else if self.local_function.is_some() {
            "a function".into()
        } else if let Some(values) = &self.tuple {
            format!("a list of {}", values.len())
        } else if self.paint.is_some() {
            "a paint".into()
        } else if self.time.is_some() {
            "an object of the time".into()
        } else if self.caption.is_some() {
            "an object of the caption".into()
        } else if self.object.is_some() || self.instance_object.is_some() {
            "an object".into()
        } else {
            match &self.statik {
                Some(serde_json::Value::Array(items)) => format!("a list of {}", items.len()),
                Some(serde_json::Value::Object(_)) => "an object".into(),
                Some(serde_json::Value::Null) => "`undefined`".into(),
                Some(_) => "a value".into(),
                None if self.scalar.is_some() => "a value".into(),
                None => "nothing".into(),
            }
        }
    }
}

/// The names a component's destructured props bind, `{ title, index }`, which its body may
/// assign as parameters.
pub(super) fn props_names(params: &FormalParameters<'_>) -> BTreeSet<String> {
    let mut names = Vec::new();
    if let Some(first) = params.items.first()
        && let BindingPattern::ObjectPattern(_) = &first.pattern
    {
        pattern_names(&first.pattern, &mut names);
    }
    names.into_iter().collect()
}

/// The names the parameters of a helper or callback bind.
pub(super) fn param_names(params: &FormalParameters<'_>) -> BTreeSet<String> {
    let mut names = Vec::new();
    for parameter in &params.items {
        pattern_names(&parameter.pattern, &mut names);
    }
    names.into_iter().collect()
}

/// Every name a binding pattern binds.
fn pattern_names(pattern: &BindingPattern<'_>, out: &mut Vec<String>) {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => out.push(identifier.name.to_string()),
        BindingPattern::ObjectPattern(object) => {
            for property in &object.properties {
                pattern_names(&property.value, out);
            }
            if let Some(rest) = &object.rest {
                pattern_names(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(array) => {
            for element in array.elements.iter().flatten() {
                pattern_names(element, out);
            }
            if let Some(rest) = &array.rest {
                pattern_names(&rest.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(assignment) => pattern_names(&assignment.left, out),
    }
}

/// Whether JavaScript treats a value known while the scene is prepared as true.
fn js_truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        serde_json::Value::String(value) => !value.is_empty(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => true,
    }
}

fn type_phrase(ty: Option<ExprType>) -> &'static str {
    match ty {
        Some(ExprType::Number) => "a number",
        Some(ExprType::Bool) => "a boolean",
        Some(ExprType::String) => "a string",
        Some(ExprType::Enum) => "an enum keyword",
        Some(ExprType::Length) => "a length",
        Some(ExprType::Length2) => "a length pair",
        Some(ExprType::Angle) => "an angle",
        Some(ExprType::Color) => "a color",
        Some(ExprType::Point) => "a point",
        Some(ExprType::Vec2) => "a vector",
        Some(ExprType::Rect) => "a rect",
        Some(ExprType::PathData) => "path data",
        None => "an invalid value",
    }
}

/// What to write instead of a statement a body does not take.
fn statement_advice(statement: &Statement<'_>) -> &'static str {
    match statement {
        Statement::SwitchStatement(_) => {
            "`switch` is not supported; write `if`/`else if`, or look the value up in an object"
        }
        Statement::DoWhileStatement(_) => "`do...while` is not supported; write `while`",
        Statement::ForInStatement(_) => {
            "`for...in` is not supported; loop over `Object.keys(object)` with `for...of`"
        }
        Statement::TryStatement(_) | Statement::ThrowStatement(_) => {
            "a body cannot throw or catch; return a value on every path"
        }
        _ => {
            "a body takes `const` and `let`, assignment, `if`/`else`, `return`, `for`, \
             `for...of`, `while`, and `push` onto a list of JSX; it runs no effects and keeps \
             no state"
        }
    }
}
