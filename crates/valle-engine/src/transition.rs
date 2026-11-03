//! Transitions: shaders in gl-transitions' form, `vec4 transition(vec2 uv)` reading the two
//! images with `getFromColor(uv)` and `getToColor(uv)`, `progress` from 0 to 1 and the box's
//! `ratio`, its width over its height. The built-in ones keep gl-transitions' names; their
//! parameters' defaults are in comments after their uniforms, as gl-transitions writes them, and
//! their ranges here. Centers and directions are in the box, y down, as CSS has them.

/// The images a transition reads: what it goes from and what it goes to.
pub const IMAGES: [&str; 2] = ["valle_from", "valle_to"];

/// `code`, a transition, as a shader: the engine's part on the first line, the code's lines
/// after it as they are, so that errors point at them.
pub fn wrap(code: &str) -> String {
    format!(
        "uniform sampler2D valle_from; uniform sampler2D valle_to; uniform float progress; \
         float ratio; in vec2 valle_at; out vec4 valle_color; \
         vec4 getFromColor(vec2 p) {{ return texture(valle_from, p); }} \
         vec4 getToColor(vec2 p) {{ return texture(valle_to, p); }} {code}\n\
         void main() {{ ratio = valle_h[0].x / valle_h[0].y; \
         progress = clamp(progress, 0.0, 1.0); valle_color = transition(valle_at); }}\n"
    )
}

/// A parameter's range: its name and its least and greatest values.
pub type Range = (&'static str, f64, f64);

/// The built-in transition `kind`: its code and its parameters' ranges.
pub fn kind(kind: &str) -> Option<(&'static str, &'static [Range])> {
    const CENTER: [Range; 2] = [("centerX", 0.0, 1.0), ("centerY", 0.0, 1.0)];
    Some(match kind {
        "fade" => (FADE, &[]),
        "wipeLeft" => (WIPE_LEFT, &[("softness", 0.0, 1.0)]),
        "wipeRight" => (WIPE_RIGHT, &[("softness", 0.0, 1.0)]),
        "circleOpen" => (
            CIRCLE_OPEN,
            &[CENTER[0], CENTER[1], ("softness", 0.0, 100.0)],
        ),
        "simpleZoom" => (
            SIMPLE_ZOOM,
            &[("quickness", 0.2, 1.0), CENTER[0], CENTER[1]],
        ),
        "crossWarp" => (CROSS_WARP, &CENTER),
        "directionalWarp" => (
            DIRECTIONAL_WARP,
            &[
                ("directionX", -1.0, 1.0),
                ("directionY", -1.0, 1.0),
                ("softness", 0.001, 1.0),
            ],
        ),
        "linearBlur" => (LINEAR_BLUR, &[("intensity", 0.0, 2.0)]),
        "dreamyZoom" => (DREAMY_ZOOM, &[("strength", 0.0, 4.0), CENTER[0], CENTER[1]]),
        "ripple" => (
            RIPPLE,
            &[
                ("frequency", 0.0, 256.0),
                ("speed", 0.0, 256.0),
                ("amplitude", 0.0, 1.0),
            ],
        ),
        "flyEye" => (
            FLY_EYE,
            &[
                ("size", 0.0, 1.0),
                ("frequency", 0.0, 256.0),
                ("colorSeparation", 0.0, 1.0),
            ],
        ),
        "multiplyBlend" => (MULTIPLY_BLEND, &[("midpoint", 0.001, 0.999)]),
        "perlin" => (
            PERLIN,
            &[("scale", 0.001, 128.0), ("smoothness", 0.001, 1.0)],
        ),
        _ => return None,
    })
}

/// The built-in kinds' names.
pub const KINDS: [&str; 13] = [
    "fade",
    "wipeLeft",
    "wipeRight",
    "circleOpen",
    "simpleZoom",
    "crossWarp",
    "directionalWarp",
    "linearBlur",
    "dreamyZoom",
    "ripple",
    "flyEye",
    "multiplyBlend",
    "perlin",
];

const FADE: &str = "vec4 transition(vec2 uv) {
  return mix(getFromColor(uv), getToColor(uv), progress);
}";

// The new image comes in from the right edge, its edge `softness` of the width wide each way.
const WIPE_LEFT: &str = "uniform float softness; // = 0.0
vec4 transition(vec2 uv) {
  float edge = 1.0 + softness - progress * (1.0 + 2.0 * softness);
  float m = softness > 0.0 ? smoothstep(edge - softness, edge + softness, uv.x) : step(edge, uv.x);
  return mix(getFromColor(uv), getToColor(uv), m);
}";

const WIPE_RIGHT: &str = "uniform float softness; // = 0.0
vec4 transition(vec2 uv) {
  float edge = progress * (1.0 + 2.0 * softness) - softness;
  float m = softness > 0.0 ? smoothstep(edge + softness, edge - softness, uv.x) : step(uv.x, edge);
  return mix(getFromColor(uv), getToColor(uv), m);
}";

// The new image inside a circle that grows from the center past the farthest corner, its edge
// `softness` px wide each way, and at least half a pixel.
const CIRCLE_OPEN: &str = "uniform float centerX; // = 0.5
uniform float centerY; // = 0.5
uniform float softness; // = 0.5
vec4 transition(vec2 uv) {
  vec2 size = valle_h[0].xy;
  vec2 c = vec2(centerX, 1.0 - centerY) * size;
  vec2 far = max(c, size - c);
  float s = max(softness, 0.5);
  float r = progress * (length(far) + 2.0 * s) - s;
  float m = 1.0 - smoothstep(r - s, r + s, length(uv * size - c));
  return mix(getFromColor(uv), getToColor(uv), m);
}";

// The old image zooms into the center as the new one fades in.
const SIMPLE_ZOOM: &str = "uniform float quickness; // = 0.8
uniform float centerX; // = 0.5
uniform float centerY; // = 0.5
vec4 transition(vec2 uv) {
  vec2 c = vec2(centerX, 1.0 - centerY);
  float zoom = smoothstep(0.0, quickness, progress);
  vec4 from = getFromColor(c + (uv - c) * (1.0 - zoom));
  return mix(from, getToColor(uv), smoothstep(quickness - 0.2, 1.0, progress));
}";

// A sweep from left to right, each image squeezed toward the center on its side.
const CROSS_WARP: &str = "uniform float centerX; // = 0.5
uniform float centerY; // = 0.5
vec4 transition(vec2 uv) {
  vec2 c = vec2(centerX, 1.0 - centerY);
  float x = smoothstep(0.0, 1.0, progress * 2.0 + uv.x - 1.0);
  return mix(getFromColor((uv - c) * (1.0 - x) + c), getToColor((uv - c) * x + c), x);
}";

// A front that crosses the box along a direction, warping both images toward the center.
const DIRECTIONAL_WARP: &str = "uniform float directionX; // = -1.0
uniform float directionY; // = 1.0
uniform float softness; // = 0.5
vec4 transition(vec2 uv) {
  vec2 v = vec2(directionX, -directionY);
  v = length(v) > 0.0 ? normalize(v) : vec2(1.0, 0.0);
  v /= abs(v.x) + abs(v.y);
  float d = v.x * 0.5 + v.y * 0.5;
  float m = 1.0 - smoothstep(-softness, 0.0, v.x * uv.x + v.y * uv.y - (d - 0.5 + progress * (1.0 + softness)));
  return mix(getFromColor((uv - 0.5) * (1.0 - m) + 0.5), getToColor((uv - 0.5) * m + 0.5), m);
}";

// Both images blurred most halfway, as they cross-fade.
const LINEAR_BLUR: &str = "uniform float intensity; // = 0.1
vec4 transition(vec2 uv) {
  float spread = intensity * (0.5 - distance(0.5, progress));
  vec4 a = vec4(0.0);
  vec4 b = vec4(0.0);
  for (int i = 0; i < 6; i++) {
    for (int j = 0; j < 6; j++) {
      vec2 at = uv + spread * (vec2(float(i), float(j)) / 6.0 - 0.5);
      a += getFromColor(at);
      b += getToColor(at);
    }
  }
  return mix(a / 36.0, b / 36.0, progress);
}";

// Both images turn and grow toward the middle of the cross-fade, and back.
const DREAMY_ZOOM: &str = "uniform float strength; // = 0.5
uniform float centerX; // = 0.5
uniform float centerY; // = 0.5
vec4 transition(vec2 uv) {
  vec2 c = vec2(centerX, 1.0 - centerY);
  float k = sin(progress * 3.14159265) * strength;
  float a = radians(12.0) * k;
  vec2 p = mat2(cos(a), sin(a), -sin(a), cos(a)) * ((uv - c) * vec2(ratio, 1.0));
  p = p / vec2(ratio, 1.0) / (1.0 + 0.5 * k) + c;
  return mix(getFromColor(p), getToColor(p), smoothstep(0.0, 1.0, progress));
}";

// Rings that run out from the center over the old image as the new one fades in.
const RIPPLE: &str = "uniform float frequency; // = 100.0
uniform float speed; // = 50.0
uniform float amplitude; // = 0.0333333
vec4 transition(vec2 uv) {
  vec2 d = uv - 0.5;
  float r = length(d);
  vec2 offset = d * (sin(progress * r * frequency - progress * speed) + 0.5) * amplitude;
  return mix(getFromColor(uv + offset), getToColor(uv), smoothstep(0.2, 1.0, progress));
}";

// The images through a grid of lenses, the old one's channels apart.
const FLY_EYE: &str = "uniform float size; // = 0.04
uniform float frequency; // = 50.0
uniform float colorSeparation; // = 0.3
vec4 transition(vec2 uv) {
  float rest = 1.0 - progress;
  vec2 d = size * vec2(cos(frequency * uv.x), sin(frequency * uv.y));
  vec4 to = getToColor(uv + rest * d);
  vec4 g = getFromColor(uv + progress * d);
  float r = getFromColor(uv + progress * d * (1.0 - colorSeparation)).r;
  float b = getFromColor(uv + progress * d * (1.0 + colorSeparation)).b;
  return mix(vec4(r, g.g, b, g.a), to, progress);
}";

// The old image into both multiplied, at `midpoint`, then into the new one.
const MULTIPLY_BLEND: &str = "uniform float midpoint; // = 0.5
vec4 transition(vec2 uv) {
  vec4 a = getFromColor(uv);
  vec4 b = getToColor(uv);
  vec4 both = a * b;
  return progress < midpoint
    ? mix(a, both, progress / midpoint)
    : mix(both, b, (progress - midpoint) / (1.0 - midpoint));
}";

// The new image where value noise, of cells `scale` across the box, is under the progress. The
// noise hashes integers, so that every GPU has the same.
const PERLIN: &str = "uniform float scale; // = 5.0
uniform float smoothness; // = 0.1
float lattice(vec2 cell) {
  uvec2 q = uvec2(ivec2(cell) + 4096);
  uint h = q.x * 1664525u + q.y * 1013904223u;
  h ^= h >> 16u;
  h *= 2246822519u;
  h ^= h >> 13u;
  return float(h & 65535u) / 65535.0;
}
float noise(vec2 p) {
  vec2 i = floor(p);
  vec2 f = fract(p);
  vec2 u = f * f * (3.0 - 2.0 * f);
  float a = lattice(i);
  float b = lattice(i + vec2(1.0, 0.0));
  float c = lattice(i + vec2(0.0, 1.0));
  float d = lattice(i + vec2(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
vec4 transition(vec2 uv) {
  float n = noise(uv * scale);
  float p = mix(-smoothness, 1.0 + smoothness, progress);
  float m = 1.0 - smoothstep(p - smoothness, p + smoothness, n);
  return mix(getFromColor(uv), getToColor(uv), m);
}";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shader;

    #[test]
    fn every_kind_translates() {
        for k in KINDS {
            let (code, ranges) = kind(k).unwrap();
            let p = shader::program(&wrap(code), &IMAGES).unwrap_or_else(|e| panic!("{k}: {e}"));
            for (name, ..) in ranges {
                assert!(p.uniforms.iter().any(|u| &*u.name == *name), "{k}: {name}");
            }
        }
        assert!(kind("swirl").is_none());
    }

    #[test]
    fn errors_point_at_the_code_s_lines() {
        let e = shader::program(
            &wrap("vec4 transition(vec2 uv) {\n  return nope;\n}"),
            &IMAGES,
        )
        .unwrap_err();
        assert!(e.starts_with("line 2: "), "{e}");
    }
}
