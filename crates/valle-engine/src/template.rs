//! The frame template: what tracing one variant of a scene leaves for its frames. A tree of nodes
//! whose props are constants or expressions, one expression program in dependency order, and the
//! guards under which the variant holds. It lives in memory only; `{:?}` is a debugging aid, not a
//! format.

use std::rc::Rc;

use crate::js::MathFn;

pub type Error = String;

/// A value of the expression language: what a JavaScript number, boolean or string traces into.
#[derive(Clone, Debug, PartialEq)]
pub enum V {
    Num(f64),
    Bool(bool),
    Str(Rc<str>),
}

impl V {
    /// JavaScript `Number(v)` for numbers and booleans; NaN otherwise.
    pub fn num(&self) -> f64 {
        match self {
            V::Num(x) => *x,
            V::Bool(b) => f64::from(u8::from(*b)),
            V::Str(_) => f64::NAN,
        }
    }

    /// JavaScript truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            V::Num(x) => *x != 0.0 && !x.is_nan(),
            V::Bool(b) => *b,
            V::Str(s) => !s.is_empty(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bin {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

/// Literal text or an operand of a template.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text(Rc<str>),
    Expr(u32),
}

/// One expression. Operands are indices of earlier expressions in the same program.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Const(V),
    /// The local clock in seconds.
    Seconds,
    /// `seconds / duration`, within [0, 1].
    Progress,
    /// Numbers only; `+` on text traces as a template.
    Bin(Bin, u32, u32),
    /// Numbers only.
    Cmp(Cmp, u32, u32),
    Neg(u32),
    /// The second operand where the first is truthy, else the third.
    Select(u32, u32, u32),
    Math(MathFn, Vec<u32>),
    /// The parts joined as a template literal joins them.
    Template(Vec<Part>),
    Interp(Rc<Interp>),
}

/// `interpolate(input, inputs, outputs, options)` over numbers. Stops are operands, so traced
/// stops work; the inputs increase.
#[derive(Clone, Debug, PartialEq)]
pub struct Interp {
    pub input: u32,
    /// (input, output) operand pairs.
    pub stops: Vec<(u32, u32)>,
    /// Past the first stop and past the last: extend the end segment (true) or clamp (false).
    pub extend: [bool; 2],
}

/// Expressions in dependency order: every operand has a lower index.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Program {
    pub ops: Vec<Op>,
}

/// A node's prop: known at tracing, or an expression evaluated per frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Prop {
    Const(V),
    Expr(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    View,
    Text { text: Prop },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: Kind,
    /// The path of keys down to the node (`card/title`), when it has a key.
    pub path: Option<Rc<str>>,
    pub classes: Vec<Rc<str>>,
    /// Style properties as written, camelCase.
    pub style: Vec<(Rc<str>, Prop)>,
    pub children: Vec<u32>,
    /// Where its JSX is written, `path:line:col`.
    pub site: Option<Rc<str>>,
}

/// A structural decision tracing took on a traced value. The variant holds while every guard does.
#[derive(Clone, Debug, PartialEq)]
pub struct Guard {
    pub expr: u32,
    pub want: Want,
    /// The guard only applies while each of these expressions has the given truthiness: it was
    /// taken inside that side of a conditional.
    pub when: Vec<(u32, bool)>,
    /// What decided (`if`, `loop`, `select`, `index`, `escape`, …) and where, `path:line`.
    pub kind: Rc<str>,
    pub at: Rc<str>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Want {
    Truthy(bool),
    Eq(V),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Template {
    pub program: Program,
    pub nodes: Vec<Node>,
    /// What the scene's root component returned, in order; the canvas holds them.
    pub roots: Vec<u32>,
    pub guards: Vec<Guard>,
}

impl Template {
    /// Whether the variant holds for these expression values (the program's, at one frame).
    pub fn holds(&self, values: &[V]) -> bool {
        self.guards.iter().all(|g| {
            let applies = g
                .when
                .iter()
                .all(|&(c, side)| values[c as usize].truthy() == side);
            !applies
                || match &g.want {
                    Want::Truthy(t) => values[g.expr as usize].truthy() == *t,
                    Want::Eq(v) => same(&values[g.expr as usize], v),
                }
        })
    }

    /// Where a traced value reached code that cannot trace it, so the scene is traced again
    /// whenever that value changes.
    pub fn escapes(&self) -> impl Iterator<Item = &Guard> {
        self.guards.iter().filter(|g| &*g.kind == "escape")
    }
}

/// Equal as an exact-value guard compares: as `===`, except that NaN holds as NaN. (-0 and +0 are
/// one value here; JSON, which carries the traced value, does not tell them apart.)
fn same(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::Num(x), V::Num(y)) => x == y || (x.is_nan() && y.is_nan()),
        _ => a == b,
    }
}
