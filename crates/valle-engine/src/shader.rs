//! Authored WGSL: `fn shade(input: ShaderInput) -> vec4f`, returning straight sRGB color.
//! The engine appends its fragment entry (coverage and premultiplication). The author's source
//! is never rewritten or translated. Naga validates it and supplies the native WGSL layout.
//!
//! Group 2 belongs to the author: one uniform struct and any supplied `texture_2d<f32>` images
//! and filtering samplers. Bindings may be sparse and in any order. Texture variable names
//! identify `content`, `backdrop`, or a named input; uniform struct fields match JSX `uniforms`.
//! Fields take f32/i32/u32 scalars, vectors, matrices or fixed arrays, flattened in memory order.
//! Unspecified values are zero. A JS boolean can fill a u32 flag; WGSL bool is not host-shareable.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use naga::{AddressSpace, ScalarKind, TypeInner};

use crate::template::{Error, V};

const INTERFACE: &str = include_str!("../shaders/authored.wgsl");

/// A field's scalar slots, including WGSL's vector, matrix and array padding.
#[derive(Clone, Debug)]
pub struct Uniform {
    pub name: Rc<str>,
    scalar: ScalarKind,
    offsets: Vec<u32>,
    color: bool,
}

#[derive(Debug)]
pub struct Image {
    pub name: Rc<str>,
    pub binding: u32,
}

/// The source sent directly to wgpu and the bindings its author declared.
#[derive(Debug)]
pub struct Program {
    pub key: u64,
    pub wgsl: String,
    pub uniforms: Vec<Uniform>,
    pub block: u32,
    pub uniform_binding: Option<u32>,
    pub images: Vec<Image>,
    pub samplers: Vec<u32>,
}

impl PartialEq for Program {
    fn eq(&self, other: &Program) -> bool {
        self.key == other.key
    }
}

impl Program {
    pub fn reads(&self, image: &str) -> bool {
        self.images.iter().any(|s| &*s.name == image)
    }

    /// Values by uniform field name, packed using the offsets of the WGSL struct. Matrices
    /// are column-major. Colors fill vec3f/vec4f fields, booleans fill scalar u32 flags.
    pub fn block(&self, values: &[(&str, Vec<&V>)]) -> Result<Vec<u8>, String> {
        let mut out = vec![0; self.block as usize];
        for (name, v) in values {
            let u = self
                .uniforms
                .iter()
                .find(|u| &*u.name == *name)
                .ok_or_else(|| format!("uniforms {name}: the code declares no uniform {name}"))?;
            let color;
            let values = if let [V::Str(s)] = &v[..] {
                if !u.color {
                    return Err(format!(
                        "uniforms {name}: a color needs a vec3f or vec4f field"
                    ));
                }
                color = crate::style::color::parse(s)?.map(V::Num);
                color[..u.offsets.len()].iter().collect::<Vec<_>>()
            } else {
                v.clone()
            };
            if values.len() != u.offsets.len() {
                return Err(format!(
                    "uniforms {name}: expected {} values, got {}",
                    u.offsets.len(),
                    values.len()
                ));
            }
            for (&offset, value) in u.offsets.iter().zip(values) {
                let bytes = match (u.scalar, value) {
                    (ScalarKind::Float, V::Num(n)) if (*n as f32).is_finite() => {
                        (*n as f32).to_ne_bytes()
                    }
                    (ScalarKind::Sint, V::Num(n))
                        if n.fract() == 0.0 && (i32::MIN as f64..=i32::MAX as f64).contains(n) =>
                    {
                        (*n as i32).to_ne_bytes()
                    }
                    (ScalarKind::Uint, V::Num(n))
                        if n.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(n) =>
                    {
                        (*n as u32).to_ne_bytes()
                    }
                    (ScalarKind::Uint, V::Bool(b)) if u.offsets.len() == 1 => {
                        u32::from(*b).to_ne_bytes()
                    }
                    _ => {
                        return Err(format!(
                            "uniforms {name}: value does not fit WGSL {:?}",
                            u.scalar
                        ));
                    }
                };
                out[offset as usize..offset as usize + 4].copy_from_slice(&bytes);
            }
        }
        Ok(out)
    }
}

type Programs = HashMap<(String, Vec<String>), Rc<Program>>;
thread_local! {
    static PROGRAMS: RefCell<Programs> = RefCell::new(HashMap::new());
}

/// Validate and reflect once per thread. Animated uniform values do not rebuild the program.
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

/// Numeric fields flatten to scalar slots. Padding follows the parsed WGSL layout rather
/// than a second implementation of struct alignment or annotations such as @align/@size.
fn slots(
    module: &naga::Module,
    ty: naga::Handle<naga::Type>,
    offset: u32,
) -> Result<(ScalarKind, Vec<u32>), Error> {
    let (scalar, offsets) = match module.types[ty].inner {
        TypeInner::Scalar(s) => (s, vec![offset]),
        TypeInner::Vector { size, scalar } => {
            (scalar, (0..size as u32).map(|i| offset + 4 * i).collect())
        }
        TypeInner::Matrix {
            columns,
            rows,
            scalar,
        } => {
            let stride = if rows == naga::VectorSize::Bi { 8 } else { 16 };
            (
                scalar,
                (0..columns as u32)
                    .flat_map(|c| (0..rows as u32).map(move |r| offset + c * stride + r * 4))
                    .collect(),
            )
        }
        TypeInner::Array {
            base,
            size: naga::ArraySize::Constant(n),
            stride,
        } => {
            let (kind, first) = slots(module, base, offset)?;
            let offsets = (0..n.get())
                .flat_map(|i| first.iter().map(move |o| o + i * stride))
                .collect();
            return Ok((kind, offsets));
        }
        _ => {
            return Err(
                "uniform fields take f32/i32/u32 scalars, vectors, matrices or fixed arrays".into(),
            );
        }
    };
    if scalar.width != 4
        || !matches!(
            scalar.kind,
            ScalarKind::Float | ScalarKind::Sint | ScalarKind::Uint
        )
    {
        return Err("uniform numbers are f32, i32 or u32; use u32 for boolean flags".into());
    }
    Ok((scalar.kind, offsets))
}

fn compile(code: &str, supplied: &[&str]) -> Result<Program, Error> {
    // Append, so parse errors in the author's code keep their original line numbers.
    let source = format!("{code}\n{INTERFACE}");
    let locate = |span: naga::Span, message: String| match span.to_range() {
        Some(r) if r.start < code.len() => {
            format!("line {}: {message}", span.location(&source).line_number)
        }
        _ => format!("{message} (shader interface: fn shade(input: ShaderInput) -> vec4f)"),
    };
    let module = naga::front::wgsl::parse_str(&source).map_err(|e| {
        let span = e.labels().next().map_or(naga::Span::UNDEFINED, |(s, _)| s);
        locate(span, e.message().to_string())
    })?;
    if module.entry_points.len() != 1 || module.entry_points[0].name != "valle_fragment" {
        return Err("provide fn shade(input: ShaderInput) -> vec4f, not a vertex, fragment or compute entry".into());
    }
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| {
        let span = e.spans().next().map_or(naga::Span::UNDEFINED, |(s, _)| *s);
        locate(span, e.to_string())
    })?;
    let mut uniforms = Vec::new();
    let mut uniform_binding = None;
    let mut block = 16;
    let mut images = Vec::new();
    let mut samplers = Vec::new();
    let mut bindings = HashSet::new();
    for (handle, var) in module.global_variables.iter() {
        let Some(binding) = &var.binding else {
            continue;
        };
        let name = var.name.as_deref().unwrap_or("");
        if name == "valle_host" {
            continue;
        }
        let bad = |s: String| locate(module.global_variables.get_span(handle), s);
        if binding.group != 2 {
            return Err(bad(format!(
                "{name}: authored resources use @group(2); groups 0 and 1 belong to the engine"
            )));
        }
        if !bindings.insert(binding.binding) {
            return Err(bad(format!("duplicate @binding({})", binding.binding)));
        }
        match (&module.types[var.ty].inner, var.space) {
            (TypeInner::Struct { members, span }, AddressSpace::Uniform) => {
                if uniform_binding.replace(binding.binding).is_some() {
                    return Err(bad("uniforms belong in one uniform struct".into()));
                }
                block = span.next_multiple_of(16);
                for member in members {
                    let (scalar, offsets) =
                        slots(&module, member.ty, member.offset).map_err(bad)?;
                    uniforms.push(Uniform {
                        name: member.name.as_deref().unwrap_or("").into(),
                        scalar,
                        offsets,
                        color: matches!(
                            module.types[member.ty].inner,
                            TypeInner::Vector {
                                size: naga::VectorSize::Tri | naga::VectorSize::Quad,
                                scalar: naga::Scalar {
                                    kind: ScalarKind::Float,
                                    width: 4
                                }
                            }
                        ),
                    });
                }
            }
            (
                TypeInner::Image {
                    dim: naga::ImageDimension::D2,
                    arrayed: false,
                    class:
                        naga::ImageClass::Sampled {
                            kind: ScalarKind::Float,
                            multi: false,
                        },
                },
                AddressSpace::Handle,
            ) => {
                if !supplied.contains(&name) {
                    return Err(bad(format!(
                        "texture {name}: no such input; available: {}",
                        supplied.join(", ")
                    )));
                }
                images.push(Image {
                    name: name.into(),
                    binding: binding.binding,
                });
            }
            (TypeInner::Sampler { comparison: false }, AddressSpace::Handle) => {
                samplers.push(binding.binding)
            }
            _ => {
                return Err(bad(format!(
                    "{name}: a resource is a uniform struct, texture_2d<f32> or sampler"
                )));
            }
        }
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    code.hash(&mut h);
    supplied.hash(&mut h);
    Ok(Program {
        key: h.finish(),
        wgsl: source,
        uniforms,
        block,
        uniform_binding,
        images,
        samplers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;

    #[test]
    fn wgsl_is_passed_through_with_its_bindings() {
        let code = indoc! {r#"
            struct Params { amount: f32, center: vec2f, tint: vec3f, steps: i32, flip: u32 }
            @group(2) @binding(7) var<uniform> params: Params;
            @group(2) @binding(4) var content: texture_2d<f32>;
            @group(2) @binding(9) var sampling: sampler;
            fn shade(input: ShaderInput) -> vec4f {
                let uv = select(input.uv, vec2f(input.uv.x, 1.0 - input.uv.y), params.flip != 0u);
                let c = textureSample(content, sampling, uv);
                let edge = fwidth(uv.x) + dpdx(uv.y) + dpdy(uv.x);
                return vec4f(c.rgb * params.tint + vec3f(edge), c.a * params.amount);
            }
        "#};
        let p = program(code, &["content"]).unwrap();
        assert!(p.wgsl.starts_with(code));
        assert_eq!(p.uniform_binding, Some(7));
        assert_eq!((&*p.images[0].name, p.images[0].binding), ("content", 4));
        assert_eq!(p.samplers, [9]);
        assert_eq!(
            p.uniforms.iter().map(|u| u.offsets[0]).collect::<Vec<_>>(),
            [0, 8, 16, 28, 32]
        );
        assert_eq!(p.block, 48);
        assert!(Rc::ptr_eq(&p, &program(code, &["content"]).unwrap()));
        let block = p
            .block(&[
                ("amount", vec![&V::Num(0.5)]),
                ("tint", vec![&V::Str("#ff008080".into())]),
                ("steps", vec![&V::Num(-2.0)]),
                ("flip", vec![&V::Bool(true)]),
            ])
            .unwrap();
        assert_eq!(f32::from_ne_bytes(block[0..4].try_into().unwrap()), 0.5);
        assert_eq!(f32::from_ne_bytes(block[16..20].try_into().unwrap()), 1.0);
        assert_eq!(i32::from_ne_bytes(block[28..32].try_into().unwrap()), -2);
        assert_eq!(u32::from_ne_bytes(block[32..36].try_into().unwrap()), 1);
        assert!(p.block(&[("steps", vec![&V::Num(0.5)])]).is_err());
        assert!(p.block(&[("flip", vec![&V::Num(-1.0)])]).is_err());
        assert!(p.block(&[("absent", vec![&V::Num(1.0)])]).is_err());
    }

    #[test]
    fn native_alignment_matrices_arrays_and_defaults() {
        let p = program(indoc! {r#"
            struct Params { @align(16) value: f32, @align(32) matrix: mat3x3f, colors: array<vec4f, 2> }
            @group(2) @binding(0) var<uniform> params: Params;
            fn shade(input: ShaderInput) -> vec4f { return params.colors[0]; }
        "#}, &[]).unwrap();
        assert_eq!(p.uniforms[1].offsets, [32, 36, 40, 48, 52, 56, 64, 68, 72]);
        assert_eq!(p.uniforms[2].offsets, [80, 84, 88, 92, 96, 100, 104, 108]);
        assert_eq!(p.block(&[]).unwrap(), vec![0; 128]);
        let values: Vec<V> = (1..=9).map(|n| V::Num(n as f64)).collect();
        let bytes = p.block(&[("matrix", values.iter().collect())]).unwrap();
        assert_eq!(f32::from_ne_bytes(bytes[64..68].try_into().unwrap()), 7.0);
        assert_eq!(&bytes[44..48], &[0; 4]);
    }

    #[test]
    fn errors_point_at_authored_lines_and_reject_unbound_resources() {
        let error = |code: &str| program(code, &["content"]).unwrap_err();
        let e = error("fn shade(input: ShaderInput) -> vec4f {\n return nope;\n}");
        assert!(e.starts_with("line 2:"), "{e}");
        let e = error(
            "@group(2) @binding(0) var missing: texture_2d<f32>;\nfn shade(input: ShaderInput) -> vec4f { return vec4f(1.0); }",
        );
        assert!(e.contains("no such input"), "{e}");
        let e = error(
            "@group(0) @binding(8) var content: texture_2d<f32>;\nfn shade(input: ShaderInput) -> vec4f { return vec4f(1.0); }",
        );
        assert!(e.contains("@group(2)"), "{e}");
        let e = error("fn other() -> vec4f { return vec4f(1.0); }");
        assert!(e.contains("shade"), "{e}");
        let e = error(
            "struct Params { enabled: bool }\n@group(2) @binding(0) var<uniform> params: Params;\nfn shade(input: ShaderInput) -> vec4f { return vec4f(1.0); }",
        );
        assert!(!e.is_empty());
    }
}
