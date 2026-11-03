use super::*;

const IMAGES: &[&str] = &["content"];

#[test]
fn a_webgl2_shader_translates() {
    let p = program(
        "#version 300 es
precision highp float;
uniform sampler2D content;
uniform float amount;
uniform vec2 center, size;
uniform vec3 tint;
uniform int steps;
uniform bool flip;
in vec2 uv;
out vec4 color;
float wave(float x) { return 0.5 + 0.5 * sin(x); }
void main() {
  vec4 c = texture(content, flip ? vec2(uv.x, 1.0 - uv.y) : uv);
  ivec2 at = ivec2(gl_FragCoord.xy) % textureSize(content, 0);
  uint h = uint(at.x * 73856093 ^ at.y * 19349663) >> 3u;
  float edge = fwidth(uv.x) + dFdx(uv.y) + dFdy(uv.x);
  vec4 t = texelFetch(content, at, 0);
  color = vec4(c.rgb * tint * wave(amount) + t.rgb * edge, c.a) * float(steps + int(h & 1u));
  color.xy += center + size;
}
",
        IMAGES,
    )
    .unwrap();
    assert_eq!(&*p.samplers, [Rc::from("content")]);
    let at: Vec<(&str, u32)> = (p.uniforms.iter()).map(|u| (&*u.name, u.offset)).collect();
    assert_eq!(
        at,
        [
            ("amount", 0),
            ("center", 8),
            ("size", 16),
            ("tint", 32),
            ("steps", 44),
            ("flip", 48)
        ]
    );
    assert_eq!(p.block, 64);
    assert!(p.wgsl.contains("fn main"), "{}", p.wgsl);
}

#[test]
fn values_fill_the_block() {
    let p = program(
        "uniform float a; uniform vec3 c; uniform int n; uniform bool b; out vec4 color;
void main() { color = vec4(c, a + float(n)) * float(b); }",
        IMAGES,
    )
    .unwrap();
    let (half, red, three, yes) = (
        V::Num(0.5),
        V::Str("#ff000080".into()),
        V::Num(3.0),
        V::Bool(true),
    );
    let block = p
        .block(&[
            ("a", vec![&half]),
            ("c", vec![&red]),
            ("n", vec![&three]),
            ("b", vec![&yes]),
        ])
        .unwrap();
    let f = |at: usize| f32::from_ne_bytes(block[at..at + 4].try_into().unwrap());
    let i = |at: usize| i32::from_ne_bytes(block[at..at + 4].try_into().unwrap());
    assert_eq!([f(0), f(16), f(20), f(24)], [0.5, 1.0, 0.0, 0.0]);
    assert_eq!([i(28), i(32)], [3, 1]);
    let wrong = |name, v: &V| p.block(&[(name, vec![v])]).unwrap_err();
    assert!(wrong("n", &half).contains("whole number"));
    assert!(wrong("b", &half).contains("true or false"));
    assert!(wrong("z", &half).contains("no uniform z"));
    assert!(p.block(&[("c", vec![&half, &half])]).is_err());
}

#[test]
fn errors_point_at_the_code_s_lines() {
    let error = |code: &str| program(code, IMAGES).unwrap_err();
    let e = error("out vec4 color;\nvoid main() {\n  color = vec4(nope);\n}\n");
    assert!(e.starts_with("line 3: "), "{e}");
    let e = error("out vec4 color;\nuniform mat3 m;\nvoid main() { color = vec4(1.0); }");
    assert!(e.starts_with("line 2: uniform mat3 m"), "{e}");
    let e = error("out vec4 color;\nuniform sampler2D other;\nvoid main() { color = vec4(1.0); }");
    assert!(e.starts_with("line 2: uniform sampler2D other"), "{e}");
    let e = error("void main() {}");
    assert!(e.contains("declares no color"), "{e}");
    let e = error("out vec4 color;\nuniform float a = 1.0;\nvoid main() { color = vec4(a); }");
    assert!(e.starts_with("line 2: "), "{e}");
    let e = error("out vec4 color;\nvoid main() {\n  color = vec4(1.0)\n}\n");
    assert!(
        e.starts_with("line 3: ") || e.starts_with("line 4: "),
        "{e}"
    );
}

#[test]
fn comments_and_blocks_are_not_declarations() {
    let p = program(
        "// uniform float hidden;
/* uniform vec2 also; */
struct S { float x; };
out vec4 color; // the color
void main() { S s = S(1.0); color = vec4(s.x); }",
        IMAGES,
    )
    .unwrap();
    assert!(p.uniforms.is_empty() && p.samplers.is_empty());
}

#[test]
fn defaults_come_from_comments() {
    let p = program(
        "uniform float a; // = 0.25
uniform vec2 c; // = vec2(0.5)
uniform vec3 d; // = vec3(1.0, 2.0, 3.0)
uniform int n; // = 3
uniform bool b; // = true
uniform float e, f; // = 9.0
uniform float g; // the default is 0
out vec4 color;
void main() { color = vec4(c, a + e + f + g, float(n) * float(b)) + vec4(d, 0.0); }",
        IMAGES,
    )
    .unwrap();
    let block = p.block(&[]).unwrap();
    let f = |at: usize| f32::from_ne_bytes(block[at..at + 4].try_into().unwrap());
    let i = |at: usize| i32::from_ne_bytes(block[at..at + 4].try_into().unwrap());
    let at = |name: &str| p.uniforms.iter().find(|u| &*u.name == name).unwrap().offset as usize;
    assert_eq!(f(at("a")), 0.25);
    assert_eq!([f(at("c")), f(at("c") + 4)], [0.5, 0.5]);
    assert_eq!([f(at("d")), f(at("d") + 8)], [1.0, 3.0]);
    assert_eq!([i(at("n")), i(at("b"))], [3, 1]);
    assert_eq!([f(at("e")), f(at("f")), f(at("g"))], [0.0, 0.0, 0.0]);
    let given = p.block(&[("a", vec![&V::Num(2.0)])]).unwrap();
    assert_eq!(f32::from_ne_bytes(given[0..4].try_into().unwrap()), 2.0);
}
