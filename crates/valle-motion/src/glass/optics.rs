//! The two shaders Glass draws with. Glass lowers to ordinary DrawProgram groups: a clip or mask
//! for its shape, a backdrop blur for its frost, the optics shader as a layer whose content is the
//! shape blurred into a height field and whose backdrop is the frosted backdrop, and the rim
//! shader as a layer over it, unclipped, whose content is the shape barely blurred, for the line
//! of light its edge catches, its glow inward and the dark edge just outside it. Both contents
//! are opaque inside the shape and gray by the presence of the Glass the shape belongs to, which
//! scales every effect. Both are written in the inline code of a ShaderLayer and prepared with an
//! artifact's inline shaders, so every backend draws them as it draws an author's shader.

use std::sync::OnceLock;

use crate::shader::{InlineCode, InputOptions, ShaderPackage, ShaderRole};

/// Uniforms: `sigma` is the height field's blur, `bevel` the width of the curved rim and
/// `refraction` the backdrop's greatest shift, at the rim, all in layer pixels; `light.xy` is a
/// unit vector toward the light in layer space and `light.z` how far the glint the rim shader
/// draws wraps round the rim.
pub const OPTICS_CODE: &str = r#"
uniform float sigma;
uniform float bevel;
uniform float refraction;
uniform float dispersion;
uniform float saturation;
uniform color tint;
uniform float3 light;
uniform float intensity;
uniform shader backdrop;

float height(float2 uv) {
  return sampleContent(uv).a;
}

// Premultiplied `top` drawn over premultiplied `under`. One expression: with statements in it,
// Skia's CPU pipeline returned the inner value of `over(a, over(b, c))`.
float4 over(float4 top, float4 under) {
  return top + under * (1.0 - top.a);
}

float4 valle_main(float2 uv) {
  float2 texel = 1.0 / resolution;
  // The blurred shape and its rise inward, from samples half a sigma to either side. A shader
  // layer's content is its children unclipped, so beyond the box the blur still holds.
  float2 reach = 0.5 * sigma * texel;
  float4 shape = sampleContent(uv);
  float here = shape.a;
  float presence = clamp(shape.r, 0.0, 1.0);
  float2 rise = float2(
      height(uv + float2(reach.x, 0.0)) - height(uv - float2(reach.x, 0.0)),
      height(uv + float2(0.0, reach.y)) - height(uv - float2(0.0, reach.y)));
  float steep = length(rise);
  float2 inward = steep > 0.0 ? rise / steep : float2(0.0);
  // Across a straight edge the blurred shape is Phi(depth / sigma): its inverse (Winitzki's
  // inverse error function) is the depth below the rim. Under the half level, as at a convex
  // corner, the depth is the rim's.
  float x = 2.0 * clamp(here, 0.5, 0.9985) - 1.0;
  float t = log(1.0 - x * x);
  float c = 4.3307467 + 0.5 * t;
  float z = 1.4142136 * sqrt(max(sqrt(max(c * c - 6.8027211 * t, 0.0)) - c, 0.0));
  float depth = sigma * z;
  // Where two rims meet, the rise is less than a straight edge's at that depth: the lens fades
  // there instead of turning over.
  float sure = clamp(steep / (0.3829249 * exp(-0.48 * z * z)), 0.0, 1.0);
  // A quarter-circle lens: the greatest shift at the rim, none from the bevel inward. A shift
  // beyond the bevel folds the rim's view over, as the edge of a thick lens does.
  float across = clamp(1.0 - depth / max(bevel, 0.0001), 0.0, 1.0);
  float lens = 1.0 - sqrt(max(1.0 - across * across, 0.0));
  float2 shift = inward * (refraction * presence * lens * sure) * texel;
  float4 behind = sample_backdrop(uv + shift);
  if (dispersion > 0.0) {
    float4 red = sample_backdrop(uv + shift * (1.0 - dispersion));
    float4 blue = sample_backdrop(uv + shift * (1.0 + dispersion));
    behind = float4(red.r, behind.g, blue.b, behind.a);
  }
  float luma = dot(behind.rgb, float3(0.2126, 0.7152, 0.0722));
  float3 seen = max(mix(float3(luma), behind.rgb, mix(1.0, saturation, presence)), 0.0);
  float cover = tint.a * presence;
  float4 glass = over(float4(tint.rgb * cover, cover), float4(seen * behind.a, behind.a));
  float3 face = glass.rgb / max(glass.a, 0.000001);
  // The raised rim shades the face behind it, its band shifted along the light.
  float facing = -dot(inward, light.xy);
  float lean = 0.3 * bevel * facing;
  float lead = lean > 0.0 ? smoothstep(0.0, 2.0 * lean, depth) : 1.0;
  float band = lead * (1.0 - smoothstep(0.0, max(0.8 * bevel, 0.0001), depth - lean));
  float shade = steep > 0.0 ? 0.055 * intensity * presence * band * mix(1.0, smoothstep(0.0, 1.0, 0.5 * facing + 0.5), 0.5) : 0.0;
  face = max(face - seen * behind.a * clamp(shade, 0.0, 1.0), 0.0);
  return glass.a > 0.0 ? float4(face, glass.a) : float4(0.0);
}
"#;

/// Uniforms: `sigma` is the blur of the rim's content, the shape, `glint` the width of the line
/// of light inside the silhouette, whose glow reaches four and a half times as far unless the
/// distance stops reading first, and `contour` the width of the dark edge outside it, all in layer
/// pixels; `light` is as the optics shader's.
pub const RIM_CODE: &str = r#"
uniform float sigma;
uniform float glint;
uniform float contour;
uniform float3 light;
uniform float intensity;
uniform shader backdrop;

float coverage(float2 uv) {
  return sampleContent(uv).a;
}

// The dark edge fades linearly outward over its width from the silhouette; this is that ramp's
// integral from the silhouette to `t`, so a pixel's share is the difference across its width.
float ramp(float t) {
  return clamp(t, 0.0, contour) - clamp(t, 0.0, contour) * clamp(t, 0.0, contour) / (2.0 * contour);
}

float4 valle_main(float2 uv) {
  float2 texel = 1.0 / resolution;
  // The barely blurred shape's rise inward, scaled as the optics shader's. Its direction is the
  // inward normal, corners included.
  float2 reach = 0.5 * sigma * texel;
  float4 shape = sampleContent(uv);
  float here = shape.a;
  float presence = clamp(shape.r, 0.0, 1.0);
  float2 rise = float2(
      coverage(uv + float2(reach.x, 0.0)) - coverage(uv - float2(reach.x, 0.0)),
      coverage(uv + float2(0.0, reach.y)) - coverage(uv - float2(0.0, reach.y)));
  float steep = length(rise);
  float2 inward = steep > 0.0 ? rise / steep : float2(0.0);
  // Signed distance inside the silhouette: across a straight edge the content is
  // Phi(distance / sigma), inverted as in the optics shader.
  float x = 2.0 * clamp(here, 0.0015, 0.9985) - 1.0;
  float t = log(1.0 - x * x);
  float c = 4.3307467 + 0.5 * t;
  float z = 1.4142136 * sqrt(max(sqrt(max(c * c - 6.8027211 * t, 0.0)) - c, 0.0));
  float inside = sigma * (x >= 0.0 ? z : -z);
  float facing = -dot(inward, light.xy);
  float along = 1.0 - abs(dot(inward, float2(-light.y, light.x)));
  float lobe = pow(max(along, 0.0), exp2(2.0 - 4.0 * light.z)) * (facing >= 0.0 ? 1.0 : 0.45);
  // The glint: a line from the silhouette inward, fading over its inner half, then its glow,
  // falling off quadratically over four and a half widths of the line and tapered to none at 2.9
  // sigmas, just short of where the distance stops reading; both over the glass's share of the
  // pixel.
  float cover = clamp(inside + 0.5, 0.0, 1.0);
  float line = (1.0 - smoothstep(0.5 * glint, glint, inside)) * cover;
  float fall = clamp(1.0 - inside / (4.5 * glint), 0.0, 1.0);
  float u = clamp(inside / (2.9 * sigma), 0.0, 1.0);
  float taper = 1.0 - u * u * u * u * u * u;
  float glow = fall * fall * taper * cover;
  float lit = steep > 0.0 ? intensity * presence * lobe : 0.0;
  float gain = lit * (10.0 * line + 12.0 * glow * (1.0 - line));
  // The dark edge just outside, box-filtered over the pixel, strongest where the rim turns
  // across the light and the glint fades.
  float edge = ramp(0.5 - inside) - ramp(-0.5 - inside);
  float dark = steep > 0.0 ? clamp(0.9 * intensity * presence * edge * mix(1.0, 1.0 - along, 0.77), 0.0, 1.0) : 0.0;
  // The glint brightens what lies under it in its own chroma, with a little white on the line
  // for dark backdrops; the dark edge dims it. One straight color over the backdrop adds both
  // changes.
  float4 under = sample_backdrop(uv);
  float white = 0.15 * lit * line;
  float3 change = under.rgb * (gain - dark) + white;
  float alpha = clamp(gain + dark + white, 0.0, 1.0);
  return alpha > 0.0 ? float4(max(under.rgb + change / alpha, 0.0), alpha) : float4(0.0);
}
"#;

/// The optics shader's package, compiled once.
pub fn optics_package() -> &'static ShaderPackage {
    static PACKAGE: OnceLock<ShaderPackage> = OnceLock::new();
    PACKAGE.get_or_init(|| {
        InlineCode::parse(OPTICS_CODE)
            .and_then(|code| code.package(ShaderRole::Layer, |_| InputOptions::backdrop(), [0; 4]))
            .unwrap_or_else(|error| panic!("the Glass optics shader compiles: {error}"))
    })
}

/// The rim shader's package, compiled once.
pub fn rim_package() -> &'static ShaderPackage {
    static PACKAGE: OnceLock<ShaderPackage> = OnceLock::new();
    PACKAGE.get_or_init(|| {
        InlineCode::parse(RIM_CODE)
            .and_then(|code| code.package(ShaderRole::Layer, |_| InputOptions::backdrop(), [0; 4]))
            .unwrap_or_else(|error| panic!("the Glass rim shader compiles: {error}"))
    })
}

/// The program a recording names the optics shader by.
pub(crate) fn optics_program() -> valle_draw::program::recording::ShaderProgram {
    program(optics_package())
}

/// The program a recording names the rim shader by.
pub(crate) fn rim_program() -> valle_draw::program::recording::ShaderProgram {
    program(rim_package())
}

fn program(package: &ShaderPackage) -> valle_draw::program::recording::ShaderProgram {
    valle_draw::program::recording::ShaderProgram {
        work_per_pixel: package.work_per_pixel(),
        padding: package.manifest.output.padding,
        uri: package.uri().to_string(),
        content_hash: valle_draw::requirements::DigestBytes::from_bytes(
            *package.content_hash.as_bytes(),
        ),
        abi_hash: valle_draw::requirements::DigestBytes::from_bytes(*package.abi_hash.as_bytes()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_optics_shader_compiles_within_the_budget_of_any_shader_layer() {
        let package = optics_package();
        assert_eq!(package.manifest.inputs.len(), 1);
        assert_eq!(package.manifest.inputs[0].name, "backdrop");
        let names = package
            .manifest
            .uniforms
            .iter()
            .map(|uniform| uniform.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "sigma",
                "bevel",
                "refraction",
                "dispersion",
                "saturation",
                "tint",
                "light",
                "intensity"
            ]
        );
        // Five height reads for the depth and the slope and at most three backdrop reads.
        assert!(
            package.work_per_pixel().samples <= 8,
            "{:?}",
            package.work_per_pixel()
        );
    }

    #[test]
    fn the_rim_shader_reads_its_content_and_one_backdrop_texel() {
        let package = rim_package();
        assert_eq!(package.manifest.inputs.len(), 1);
        assert_eq!(package.manifest.inputs[0].name, "backdrop");
        let names = package
            .manifest
            .uniforms
            .iter()
            .map(|uniform| uniform.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["sigma", "glint", "contour", "light", "intensity"]);
        // Five coverage reads for the distance and the slope, one backdrop read.
        assert!(
            package.work_per_pixel().samples <= 6,
            "{:?}",
            package.work_per_pixel()
        );
    }
}
