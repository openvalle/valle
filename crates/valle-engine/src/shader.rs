//! Shaders authors write: GLSL ES 3.0 fragment shaders, as WebGL2 runs them, translated to WGSL
//! by naga, so that every backend and the browser run the same code.
//!
//! A shader declares what it reads at the top level: `uniform sampler2D <name>;` for the images
//! it is given by name, `uniform <type> <name>;` for values, of `float`, `int`, `bool` or `vec2`
//! to `vec4`, `in vec2 <uv>;` for where it is in its box, 0–1 with y up, and `out vec4 <color>;`
//! for the color it gives there, sRGB-encoded with straight alpha, as WebGL takes it. Images read
//! the same way: y up, straight alpha. `gl_FragCoord` is in the box's pixels, from its bottom left.
//!
//! The code is rewritten in place, line for line, so that naga's errors point at the author's
//! lines: the declarations become plain globals that a `main` of the engine's fills before it calls
//! the author's, renamed, and premultiplies the color it gives.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use crate::template::{Error, V};

/// A uniform's type: a vector's length for `float` and `vec2`–`vec4`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Type {
    Float(u32),
    Int,
    Bool,
}

impl Type {
    fn of(name: &str) -> Option<Type> {
        Some(match name {
            "float" => Type::Float(1),
            "vec2" => Type::Float(2),
            "vec3" => Type::Float(3),
            "vec4" => Type::Float(4),
            "int" => Type::Int,
            "bool" => Type::Bool,
            _ => return None,
        })
    }

    /// Its size and alignment in a uniform block (std140), bytes.
    fn layout(self) -> (u32, u32) {
        match self {
            Type::Float(1) | Type::Int | Type::Bool => (4, 4),
            Type::Float(2) => (8, 8),
            Type::Float(n) => (4 * n, 16),
        }
    }

    /// As the block holds it: a `bool` as an `int`.
    fn stored(self) -> &'static str {
        match self {
            Type::Float(1) => "float",
            Type::Float(2) => "vec2",
            Type::Float(3) => "vec3",
            Type::Float(_) => "vec4",
            Type::Int | Type::Bool => "int",
        }
    }
}

/// A uniform the code declares: its name, type and offset in the block, bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Uniform {
    pub name: Rc<str>,
    pub ty: Type,
    pub offset: u32,
}

/// Code translated: its WGSL fragment module, entry `main`, what it declares, and the block its
/// uniforms take, bytes, a multiple of 16.
#[derive(Debug)]
pub struct Program {
    /// Identity, for pipelines.
    pub key: u64,
    pub wgsl: String,
    pub uniforms: Vec<Uniform>,
    pub block: u32,
    /// The images it reads by name, in the order of their bindings.
    pub samplers: Vec<Rc<str>>,
}

impl PartialEq for Program {
    fn eq(&self, other: &Program) -> bool {
        self.key == other.key
    }
}

impl Program {
    pub fn reads(&self, image: &str) -> bool {
        self.samplers.iter().any(|s| &**s == image)
    }

    /// The uniform block for `values` by name, which name uniforms it declares: a number for a
    /// `float` or an `int` (a whole one), a boolean for a `bool`, numbers for a vector, or a color
    /// for a `vec3` or `vec4`, its sRGB-encoded channels with straight alpha. Those it is not given
    /// are 0, as in WebGL.
    pub fn block(&self, values: &[(&str, Vec<&V>)]) -> Result<Vec<u8>, String> {
        let mut out = vec![0u8; self.block as usize];
        for (name, v) in values {
            let Some(u) = self.uniforms.iter().find(|u| &*u.name == *name) else {
                return Err(format!(
                    "uniforms {name}: the code declares no uniform {name}"
                ));
            };
            let words: Vec<[u8; 4]> = match (u.ty, &v[..]) {
                (Type::Float(n), [V::Str(s)]) if n >= 3 => {
                    let c = crate::style::color::parse(s)
                        .map_err(|e| format!("uniforms {name}: {e}"))?;
                    c[..n as usize]
                        .iter()
                        .map(|&x| (x as f32).to_ne_bytes())
                        .collect()
                }
                (Type::Float(n), v) if v.len() == n as usize => (v.iter())
                    .map(|x| match x {
                        V::Num(x) => Ok((*x as f32).to_ne_bytes()),
                        _ => Err(format!("uniforms {name}: {} numbers", n)),
                    })
                    .collect::<Result<_, _>>()?,
                (Type::Int, [V::Num(x)]) if x.fract() == 0.0 && x.abs() < 2.0f64.powi(31) => {
                    vec![(*x as i32).to_ne_bytes()]
                }
                (Type::Bool, [V::Bool(b)]) => vec![i32::from(*b).to_ne_bytes()],
                (ty, _) => {
                    let wanted = match ty {
                        Type::Float(1) => "a number".into(),
                        Type::Float(n) if n >= 3 => format!("{n} numbers or a color"),
                        Type::Float(n) => format!("{n} numbers"),
                        Type::Int => "a whole number".into(),
                        Type::Bool => "true or false".into(),
                    };
                    return Err(format!(
                        "uniforms {name} is {wanted}, as its uniform is declared"
                    ));
                }
            };
            for (k, w) in words.iter().enumerate() {
                let at = u.offset as usize + 4 * k;
                out[at..at + 4].copy_from_slice(w);
            }
        }
        Ok(out)
    }
}

/// Programs by their code and the images they may read.
type Programs = HashMap<(String, Vec<String>), Rc<Program>>;

thread_local! {
    static PROGRAMS: RefCell<Programs> = RefCell::new(HashMap::new());
}

/// `code` translated, once a thread: a shader that may read the images `images` names.
pub fn program(code: &str, images: &[&str]) -> Result<Rc<Program>, Error> {
    let key = (
        code.to_string(),
        images.iter().map(|s| s.to_string()).collect(),
    );
    if let Some(p) = PROGRAMS.with(|p| p.borrow().get(&key).cloned()) {
        return Ok(p);
    }
    let p = Rc::new(compile(code, images)?);
    PROGRAMS.with(|c| c.borrow_mut().insert(key, p.clone()));
    Ok(p)
}

/// What a shader declares at the top level.
#[derive(Default)]
struct Declared {
    uniforms: Vec<(Rc<str>, Type)>,
    samplers: Vec<Rc<str>>,
    uv: Option<String>,
    color: Option<String>,
}

/// `code` with comments blanked, every byte where it was, newlines kept.
fn blank_comments(code: &str) -> String {
    let b = code.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        let end = if b[i..].starts_with(b"//") {
            code[i..].find('\n').map_or(b.len(), |n| i + n)
        } else if b[i..].starts_with(b"/*") {
            code[i + 2..].find("*/").map_or(b.len(), |n| i + n + 4)
        } else {
            i += 1;
            continue;
        };
        for c in &mut out[i..end] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
        i = end;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// The line of byte `at` in `code`, from 1.
fn line_of(code: &str, at: usize) -> usize {
    code[..at.min(code.len())].matches('\n').count() + 1
}

/// `range` of `code` replaced by `text`, keeping its newlines after it so that lines stay.
fn replace(
    edits: &mut Vec<(std::ops::Range<usize>, String)>,
    code: &str,
    range: std::ops::Range<usize>,
    text: String,
) {
    let lines = code[range.clone()].matches('\n').count();
    edits.push((range, text + &"\n".repeat(lines)));
}

/// The code rewritten line for line, and what it declares.
fn rewrite(code: &str, images: &[&str]) -> Result<(String, Declared), Error> {
    let clean = blank_comments(code);
    let b = clean.as_bytes();
    let mut d = Declared::default();
    let mut edits = Vec::new();
    let (mut depth, mut start, mut i) = (0usize, 0usize, 0usize);
    let at = |i: usize, message: String| format!("line {}: {message}", line_of(code, i));
    while i < b.len() {
        match b[i] {
            // A directive is a line of its own.
            b'#' if depth == 0 && clean[start..i].trim().is_empty() => {
                let end = clean[i..].find('\n').map_or(b.len(), |n| i + n);
                let directive = clean[i..end].trim();
                if directive.starts_with("#version") {
                    replace(&mut edits, code, i..end, String::new());
                } else if directive.starts_with("#extension") {
                    return Err(at(i, "extensions are not supported".into()));
                }
                i = end;
                start = end;
                continue;
            }
            b'{' => {
                if depth == 0 {
                    let head: Vec<&str> = clean[start..i]
                        .split(|c: char| c.is_whitespace() || c == '(' || c == ')')
                        .filter(|w| !w.is_empty())
                        .collect();
                    if head.first() == Some(&"uniform") {
                        return Err(at(
                            start,
                            "uniform blocks are not supported: declare each uniform on its own"
                                .into(),
                        ));
                    }
                    if head.len() >= 2 && head[0] == "void" && head[1] == "main" {
                        let m = start + clean[start..i].find("main").unwrap_or(0);
                        edits.push((m..m + 4, "valle_user_main".into()));
                    }
                }
                depth += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    start = i + 1;
                }
            }
            b';' if depth == 0 => {
                declaration(code, &clean, start..i, images, &mut d, &mut edits)?;
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if d.color.is_none() {
        return Err(
            "the code declares no color: declare `out vec4 color;` and set it in main".into(),
        );
    }
    edits.sort_by_key(|(r, _)| r.start);
    let mut out = String::with_capacity(code.len() + 256);
    let mut last = 0;
    for (r, text) in edits {
        out.push_str(&code[last..r.start]);
        out.push_str(&text);
        last = r.end;
    }
    out.push_str(&code[last..]);
    Ok((out, d))
}

/// A top-level declaration, `range` of `code` before its `;`: a uniform, the input or the output
/// becomes a plain global or nothing; a precision statement nothing.
fn declaration(
    code: &str,
    clean: &str,
    range: std::ops::Range<usize>,
    images: &[&str],
    d: &mut Declared,
    edits: &mut Vec<(std::ops::Range<usize>, String)>,
) -> Result<(), Error> {
    let text = &clean[range.clone()];
    let first = text.len() - text.trim_start().len();
    let line = line_of(code, range.start + first);
    let fail = |message: String| Err(format!("line {line}: {message}"));
    // `layout(…)` goes: the engine places what is declared.
    let text = match text.trim_start().strip_prefix("layout") {
        Some(rest) => match rest.find(')') {
            Some(close) => rest[close + 1..].to_string(),
            None => return fail("a layout qualifier without its `)`".into()),
        },
        None => text.to_string(),
    };
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|w| {
            !w.is_empty() && !matches!(*w, "highp" | "mediump" | "lowp" | "flat" | "smooth")
        })
        .collect();
    let Some(&qualifier) = words.first() else {
        return Ok(());
    };
    let whole = range.start..range.end + 1;
    match qualifier {
        "precision" => replace(edits, code, whole, String::new()),
        "uniform" => {
            let [_, ty, names @ ..] = &words[..] else {
                return fail("declare a uniform as `uniform <type> <name>;`".into());
            };
            if names.is_empty() {
                return fail("declare a uniform as `uniform <type> <name>;`".into());
            }
            if text.contains('[') || text.contains('=') {
                return fail(
                    "a uniform is one value, without an initializer: it comes from uniforms".into(),
                );
            }
            let mut globals = String::new();
            for &name in names {
                if *ty == "sampler2D" {
                    if !images.contains(&name) {
                        let list = images.join(", ");
                        return fail(format!(
                            "uniform sampler2D {name}: the images it may read are {list}"
                        ));
                    }
                    d.samplers.push(name.into());
                    continue;
                }
                let Some(t) = Type::of(ty) else {
                    return fail(format!(
                        "uniform {ty} {name}: a uniform is a float, int, bool, vec2, vec3, vec4 or \
                         sampler2D"
                    ));
                };
                d.uniforms.push((name.into(), t));
                globals.push_str(&format!("{ty} {name};"));
            }
            replace(edits, code, whole, globals);
        }
        "in" => {
            let ["in", "vec2", name] = words[..] else {
                return fail("the input is where it is in its box: `in vec2 uv;`".into());
            };
            if d.uv.replace(name.into()).is_some() {
                return fail("the code declares a second input".into());
            }
            replace(edits, code, whole, format!("vec2 {name};"));
        }
        "out" => {
            let ["out", "vec4", name] = words[..] else {
                return fail("the output is its color: `out vec4 color;`".into());
            };
            if d.color.replace(name.into()).is_some() {
                return fail("the code declares a second output".into());
            }
            replace(edits, code, whole, format!("vec4 {name};"));
        }
        "buffer" | "shared" | "attribute" | "varying" => {
            return fail(format!("{qualifier} declarations are not supported"));
        }
        _ => {}
    }
    Ok(())
}

/// The engine's part before the code: the host's block (set 1), and the code's uniforms and
/// images (set 2): its block at binding 0, each image a texture and a sampler after it.
fn prelude(d: &Declared) -> (String, Vec<Uniform>, u32) {
    let mut p = String::from("#version 450\n");
    p.push_str("layout(set = 1, binding = 0) uniform ValleHost { vec4 valle_h[4]; };\n");
    let mut uniforms = Vec::new();
    let mut size = 0u32;
    if !d.uniforms.is_empty() {
        p.push_str("layout(set = 2, binding = 0) uniform ValleUser {");
        for (name, ty) in &d.uniforms {
            let (bytes, align) = ty.layout();
            let offset = size.next_multiple_of(align);
            size = offset + bytes;
            p.push_str(&format!(" {} valle_u_{name};", ty.stored()));
            uniforms.push(Uniform {
                name: name.clone(),
                ty: *ty,
                offset,
            });
        }
        p.push_str(" };\n");
    }
    for (k, name) in d.samplers.iter().enumerate() {
        let (t, s) = (1 + 2 * k, 2 + 2 * k);
        p.push_str(&format!(
            "layout(set = 2, binding = {t}) uniform texture2D valle_t_{name};\n\
             layout(set = 2, binding = {s}) uniform sampler valle_s_{name};\n\
             #define {name} sampler2D(valle_t_{name}, valle_s_{name})\n"
        ));
    }
    p.push_str("vec4 valle_frag_coord;\n#define gl_FragCoord valle_frag_coord\n");
    (p, uniforms, size.next_multiple_of(16).max(16))
}

/// The engine's `main`: the box's antialiased edge, from its px (taken in uniform control flow),
/// the input and the uniforms set, the code's `main`, and its color premultiplied. The host's
/// block: the box's width and height in px, its outset, and its size in the pixels its images
/// have.
fn epilogue(d: &Declared) -> String {
    let mut m = String::from(
        "\nlayout(location = 0) in vec2 valle_local;\n\
         layout(location = 0) out vec4 valle_out;\n\
         void main() {\n\
         vec2 valle_size = valle_h[0].xy;\n\
         vec2 valle_fw = max(fwidth(valle_local), vec2(1e-6));\n\
         vec2 valle_half = 0.5 * valle_size + valle_h[0].z;\n\
         vec2 valle_d = abs(valle_local - 0.5 * valle_size);\n\
         vec2 valle_c = clamp((valle_half - valle_d) / valle_fw + 0.5, vec2(0.0), vec2(1.0));\n\
         vec2 valle_uv = vec2(valle_local.x, valle_size.y - valle_local.y) / valle_size;\n\
         valle_frag_coord = vec4(valle_uv * valle_h[1].xy, 0.5, 1.0);\n",
    );
    if let Some(uv) = &d.uv {
        m.push_str(&format!("{uv} = valle_uv;\n"));
    }
    for (name, ty) in &d.uniforms {
        match ty {
            Type::Bool => m.push_str(&format!("{name} = valle_u_{name} != 0;\n")),
            _ => m.push_str(&format!("{name} = valle_u_{name};\n")),
        }
    }
    let color = d.color.as_deref().unwrap_or("valle_out");
    m.push_str(&format!(
        "valle_user_main();\n\
         vec4 valle_s = clamp({color}, vec4(0.0), vec4(1.0));\n\
         valle_out = vec4(valle_s.rgb * valle_s.a, valle_s.a) * (valle_c.x * valle_c.y);\n\
         }}\n"
    ));
    m
}

fn compile(code: &str, images: &[&str]) -> Result<Program, Error> {
    let (body, declared) = rewrite(code, images)?;
    let (head, uniforms, block) = prelude(&declared);
    let skip = head.matches('\n').count();
    let source = format!("{head}{body}{}", epilogue(&declared));
    let lines = code.matches('\n').count() + 1;
    // An error on a line of the code, or else on the engine's.
    let locate = |span: naga::Span, message: String| {
        if !span.is_defined() {
            return message;
        }
        let line = span.location(&source).line_number as usize;
        match line.checked_sub(skip).filter(|&l| l >= 1 && l <= lines) {
            Some(l) => format!("line {l}: {message}"),
            None => format!("{message} (in what the engine adds around the code)"),
        }
    };
    let mut frontend = naga::front::glsl::Frontend::default();
    let options = naga::front::glsl::Options::from(naga::ShaderStage::Fragment);
    let module = frontend.parse(&options, &source).map_err(|e| {
        let first = e.errors.first();
        first.map_or_else(
            || "it does not parse".to_string(),
            |e| locate(e.meta, e.kind.to_string()),
        )
    })?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| {
        let message = e.as_inner().to_string();
        match e.spans().next() {
            Some((span, _)) => locate(*span, message),
            None => message,
        }
    })?;
    let wgsl =
        naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
            .map_err(|e| format!("engine bug: WGSL: {e}"))?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    code.hash(&mut h);
    images.hash(&mut h);
    Ok(Program {
        key: h.finish(),
        wgsl,
        uniforms,
        block,
        samplers: declared.samplers,
    })
}

#[cfg(test)]
mod tests;
