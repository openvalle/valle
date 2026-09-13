use super::{ShaderDiagnostic, error};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Type {
    Float(u8),
    Matrix(u8),
    Int,
    Bool,
}

impl Type {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "float" | "half" => Self::Float(1),
            "float2" | "half2" => Self::Float(2),
            "float3" | "half3" => Self::Float(3),
            "float4" | "half4" => Self::Float(4),
            "float2x2" => Self::Matrix(2),
            "float3x3" => Self::Matrix(3),
            "float4x4" => Self::Matrix(4),
            "int" => Self::Int,
            "bool" => Self::Bool,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Float(1) => "float",
            Self::Float(2) => "float2",
            Self::Float(3) => "float3",
            Self::Float(4) => "float4",
            Self::Matrix(2) => "float2x2",
            Self::Matrix(3) => "float3x3",
            Self::Matrix(4) => "float4x4",
            Self::Int => "int",
            Self::Bool => "bool",
            _ => unreachable!(),
        }
    }

    pub fn width(self) -> usize {
        match self {
            Self::Float(n) => n as usize,
            Self::Matrix(n) => (n * n) as usize,
            _ => 1,
        }
    }

    pub fn accepts(self, other: Self) -> bool {
        self == other || (self == Self::Float(1) && other == Self::Int)
    }
}

#[derive(Debug, Clone)]
pub(super) struct Expr {
    pub pos: usize,
    pub kind: ExprKind,
}

#[derive(Debug, Clone)]
pub(super) enum ExprKind {
    Number(String),
    Name(String),
    Call(String, Vec<Expr>),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Select(Box<Expr>, Box<Expr>, Box<Expr>),
    Swizzle(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone)]
pub(super) struct Statement {
    pub pos: usize,
    pub kind: StatementKind,
}

#[derive(Debug, Clone)]
pub(super) enum StatementKind {
    Block(Vec<Statement>),
    Declare {
        constant: bool,
        ty: Type,
        name: String,
        value: Expr,
    },
    Assign {
        target: Expr,
        op: String,
        value: Expr,
    },
    Return(Expr),
    If {
        condition: Expr,
        yes: Box<Statement>,
        no: Option<Box<Statement>>,
    },
    For {
        name: String,
        start: Expr,
        comparison: String,
        end: Expr,
        step: Expr,
        subtract: bool,
        body: Box<Statement>,
    },
    Eval(Expr),
}

#[derive(Debug, Clone)]
pub(super) struct Function {
    pub pos: usize,
    pub name: String,
    pub result: Type,
    pub parameters: Vec<(Type, String)>,
    pub body: Statement,
}

#[derive(Clone)]
struct Token {
    pos: usize,
    text: String,
}

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
    depth: usize,
}

pub(super) fn parse(
    source: &str,
    max_tokens: usize,
    max_depth: usize,
) -> Result<Vec<Function>, ShaderDiagnostic> {
    let mut tokens = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut nesting = 0usize;
    while i < bytes.len() {
        let start = i;
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            i += 2;
            while i < bytes.len() && !bytes[i..].starts_with(b"*/") {
                i += 1;
            }
            if i == bytes.len() {
                return Err(error(start, "unterminated block comment"));
            }
            i += 2;
            continue;
        }
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
        } else if bytes[i].is_ascii_digit()
            || (bytes[i] == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
                i += 1;
                if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
                    i += 1;
                }
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let value = source[start..i]
                .parse::<f32>()
                .map_err(|_| error(start, "invalid numeric literal"))?;
            if !value.is_finite() {
                return Err(error(start, "numeric literal must be finite"));
            }
        } else if b"(){}[];,.?:+-*/%=<>!&|".contains(&bytes[i]) {
            if b"({[".contains(&bytes[i]) {
                nesting += 1;
                if nesting > max_depth {
                    return Err(error(start, "syntax nesting limit exceeded"));
                }
            } else if b")}]".contains(&bytes[i]) {
                nesting = nesting.saturating_sub(1);
            }
            i += 1;
            if i < bytes.len()
                && [
                    "++", "--", "+=", "-=", "*=", "/=", "<=", ">=", "==", "!=", "&&", "||",
                ]
                .contains(&&source[start..i + 1])
            {
                i += 1;
            }
        } else {
            return Err(error(i, "character is outside the shader language"));
        }
        tokens.push(Token {
            pos: start,
            text: source[start..i].into(),
        });
        if tokens.len() > max_tokens {
            return Err(error(start, "syntax token limit exceeded"));
        }
    }
    tokens.push(Token {
        pos: source.len(),
        text: String::new(),
    });
    let mut p = Parser {
        tokens,
        cursor: 0,
        depth: 0,
    };
    let mut functions = Vec::new();
    while !p.peek().is_empty() {
        let pos = p.pos();
        let result = p.ty()?;
        let name = p.ident()?;
        p.expect("(")?;
        let mut parameters = Vec::new();
        if !p.take(")") {
            loop {
                parameters.push((p.ty()?, p.ident()?));
                if p.take(")") {
                    break;
                }
                p.expect(",")?;
            }
        }
        if p.peek() != "{" {
            return Err(error(
                p.pos(),
                "function definition requires a block; mutable globals and prototypes are forbidden",
            ));
        }
        let body = p.statement()?;
        functions.push(Function {
            pos,
            name,
            result,
            parameters,
            body,
        });
    }
    Ok(functions)
}

impl Parser {
    fn peek(&self) -> &str {
        &self.tokens[self.cursor].text
    }
    fn pos(&self) -> usize {
        self.tokens[self.cursor].pos
    }
    fn take(&mut self, value: &str) -> bool {
        if self.peek() == value {
            self.cursor += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, value: &str) -> Result<(), ShaderDiagnostic> {
        if self.take(value) {
            Ok(())
        } else {
            Err(error(
                self.pos(),
                format!("expected `{value}`, found `{}`", self.peek()),
            ))
        }
    }
    fn ident(&mut self) -> Result<String, ShaderDiagnostic> {
        let text = self.peek().to_owned();
        if text
            .as_bytes()
            .first()
            .is_none_or(|b| !b.is_ascii_alphabetic() && *b != b'_')
        {
            return Err(error(self.pos(), "expected an identifier"));
        }
        self.cursor += 1;
        Ok(text)
    }
    fn ty(&mut self) -> Result<Type, ShaderDiagnostic> {
        let ty =
            Type::parse(self.peek()).ok_or_else(|| error(self.pos(), "expected a value type"))?;
        self.cursor += 1;
        Ok(ty)
    }
    fn statement(&mut self) -> Result<Statement, ShaderDiagnostic> {
        self.depth += 1;
        if self.depth > 48 {
            return Err(error(self.pos(), "statement nesting limit exceeded"));
        }
        let result = self.statement_inner();
        self.depth -= 1;
        result
    }
    fn statement_inner(&mut self) -> Result<Statement, ShaderDiagnostic> {
        let pos = self.pos();
        let kind = if self.take("{") {
            let mut statements = Vec::new();
            while !self.take("}") {
                statements.push(self.statement()?);
            }
            StatementKind::Block(statements)
        } else if self.take("return") {
            let value = self.expression(0)?;
            self.expect(";")?;
            StatementKind::Return(value)
        } else if self.take("if") {
            self.expect("(")?;
            let condition = self.expression(0)?;
            self.expect(")")?;
            let yes = Box::new(self.statement()?);
            let no = if self.take("else") {
                Some(Box::new(self.statement()?))
            } else {
                None
            };
            StatementKind::If { condition, yes, no }
        } else if self.take("for") {
            self.expect("(")?;
            self.expect("int")?;
            let name = self.ident()?;
            self.expect("=")?;
            let start = self.expression(0)?;
            self.expect(";")?;
            self.expect(&name)?;
            let comparison = self.peek().to_owned();
            if !["<", "<=", ">", ">="].contains(&comparison.as_str()) {
                return Err(error(self.pos(), "for requires a constant ordered bound"));
            }
            self.cursor += 1;
            let end = self.expression(0)?;
            self.expect(";")?;
            let (step, subtract) = if self.peek() == "++" || self.peek() == "--" {
                let subtract = self.take("--");
                if !subtract {
                    self.expect("++")?;
                }
                self.expect(&name)?;
                (
                    Expr {
                        pos,
                        kind: ExprKind::Number("1".into()),
                    },
                    subtract,
                )
            } else {
                self.expect(&name)?;
                if self.peek() == "++" || self.peek() == "--" {
                    let subtract = self.take("--");
                    if !subtract {
                        self.expect("++")?;
                    }
                    (
                        Expr {
                            pos,
                            kind: ExprKind::Number("1".into()),
                        },
                        subtract,
                    )
                } else {
                    let subtract = self.take("-=");
                    if !subtract {
                        self.expect("+=")?;
                    }
                    (self.expression(0)?, subtract)
                }
            };
            self.expect(")")?;
            let body = Box::new(self.statement()?);
            StatementKind::For {
                name,
                start,
                comparison,
                end,
                step,
                subtract,
                body,
            }
        } else if self.peek() == "const" || Type::parse(self.peek()).is_some() {
            let constant = self.take("const");
            let ty = self.ty()?;
            let name = self.ident()?;
            self.expect("=")?;
            let value = self.expression(0)?;
            self.expect(";")?;
            StatementKind::Declare {
                constant,
                ty,
                name,
                value,
            }
        } else {
            let target = self.expression(0)?;
            if ["=", "+=", "-=", "*=", "/="].contains(&self.peek()) {
                let op = self.peek().to_owned();
                self.cursor += 1;
                let value = self.expression(0)?;
                self.expect(";")?;
                StatementKind::Assign { target, op, value }
            } else {
                self.expect(";")?;
                StatementKind::Eval(target)
            }
        };
        Ok(Statement { pos, kind })
    }
    fn expression(&mut self, min: u8) -> Result<Expr, ShaderDiagnostic> {
        self.depth += 1;
        if self.depth > 48 {
            return Err(error(self.pos(), "expression nesting limit exceeded"));
        }
        let result = self.expression_inner(min);
        self.depth -= 1;
        result
    }
    fn expression_inner(&mut self, min: u8) -> Result<Expr, ShaderDiagnostic> {
        let pos = self.pos();
        let mut value = if ["+", "-", "!"].contains(&self.peek()) {
            let op = self.peek().to_owned();
            self.cursor += 1;
            Expr {
                pos,
                kind: ExprKind::Unary(op, Box::new(self.expression(8)?)),
            }
        } else if self.take("(") {
            let expr = self.expression(0)?;
            self.expect(")")?;
            expr
        } else if self
            .peek()
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
        {
            let number = self.peek().to_owned();
            self.cursor += 1;
            Expr {
                pos,
                kind: ExprKind::Number(number),
            }
        } else {
            let name = self.ident()?;
            if self.take("(") {
                let mut args = Vec::new();
                if !self.take(")") {
                    loop {
                        args.push(self.expression(0)?);
                        if self.take(")") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                Expr {
                    pos,
                    kind: ExprKind::Call(name, args),
                }
            } else {
                Expr {
                    pos,
                    kind: ExprKind::Name(name),
                }
            }
        };
        loop {
            if self.take(".") {
                let fields = self.ident()?;
                if self.peek() == "(" {
                    return Err(error(
                        pos,
                        "method calls are forbidden; textures are opaque and require sample helpers",
                    ));
                }
                value = Expr {
                    pos,
                    kind: ExprKind::Swizzle(Box::new(value), fields),
                };
                continue;
            }
            if self.take("[") {
                let index = self.expression(0)?;
                self.expect("]")?;
                value = Expr {
                    pos,
                    kind: ExprKind::Index(Box::new(value), Box::new(index)),
                };
                continue;
            }
            let precedence = match self.peek() {
                "||" => 1,
                "&&" => 2,
                "==" | "!=" => 3,
                "<" | "<=" | ">" | ">=" => 4,
                "+" | "-" => 5,
                "*" | "/" | "%" => 6,
                _ => 0,
            };
            if precedence == 0 || precedence < min {
                break;
            }
            let op = self.peek().to_owned();
            self.cursor += 1;
            value = Expr {
                pos,
                kind: ExprKind::Binary(
                    op,
                    Box::new(value),
                    Box::new(self.expression(precedence + 1)?),
                ),
            };
        }
        if min == 0 && self.take("?") {
            let yes = self.expression(0)?;
            self.expect(":")?;
            let no = self.expression(0)?;
            value = Expr {
                pos,
                kind: ExprKind::Select(Box::new(value), Box::new(yes), Box::new(no)),
            };
        }
        Ok(value)
    }
}
