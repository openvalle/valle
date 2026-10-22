//! The one shader Glass draws with. Glass lowers to ordinary DrawProgram groups: a clip or mask
//! for its shape, a backdrop blur for its frost, and this shader as a layer whose content is the
//! shape blurred into a height field and whose backdrop is the frosted backdrop. It is written in
//! the inline code of a ShaderLayer and prepared with an artifact's inline shaders, so every
//! backend draws it as it draws an author's shader.

use std::sync::OnceLock;

use crate::shader::{InlineCode, InputOptions, ShaderPackage, ShaderRole};

/// Uniforms: `sigma` is the height field's blur and `refraction` the backdrop's shift across a
/// straight rim, both in layer pixels; `light` is a unit vector toward the light in layer space.
pub const OPTICS_CODE: &str = r#"
uniform float sigma;
uniform float refraction;
uniform float dispersion;
uniform color tint;
uniform float3 light;
uniform float intensity;
uniform float presence;
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
  // The blurred shape's slope, scaled to 1 across a straight edge by 1 / (2 Phi(1/2) - 1). It
  // falls smoothly to 0 inside and on the midline of a narrow shape, so the surface has no
  // crease anywhere. The samples lie half a sigma to either side: a rounding step of the stored
  // height then shifts the backdrop about sigma / 2 times less than between neighbouring pixels.
  // A shader layer's content is its children unclipped, so beyond the box the blur still holds.
  float2 reach = 0.5 * sigma * texel;
  float2 slope = float2(
      height(uv + float2(reach.x, 0.0)) - height(uv - float2(reach.x, 0.0)),
      height(uv + float2(0.0, reach.y)) - height(uv - float2(0.0, reach.y)))
    * 2.6114780;
  // The surface rises inward, bending the backdrop in from the rim.
  float2 shift = slope * refraction * texel;
  float4 behind = sample_backdrop(uv + shift);
  if (dispersion > 0.0) {
    float4 red = sample_backdrop(uv + shift * (1.0 - dispersion));
    float4 blue = sample_backdrop(uv + shift * (1.0 + dispersion));
    behind = float4(red.r, behind.g, blue.b, behind.a);
  }
  float3 normal = normalize(float3(-1.5 * slope, 1.0));
  float specular = pow(max(dot(normal, normalize(light + float3(0.0, 0.0, 1.0))), 0.0), 40.0);
  float rim = pow(1.0 - normal.z, 2.0);
  float shine = clamp(intensity * (specular + 0.5 * rim), 0.0, 1.0);
  // White shine over the tint over what lies behind; nothing over nothing stays nothing.
  float4 glass = over(float4(shine),
    over(float4(tint.rgb * tint.a, tint.a), float4(behind.rgb * behind.a, behind.a)));
  return glass.a > 0.0 ? float4(glass.rgb / glass.a, glass.a * presence) : float4(0.0);
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

/// The program a recording names the optics shader by.
pub(crate) fn optics_program() -> valle_draw::program::recording::ShaderProgram {
    let package = optics_package();
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
                "refraction",
                "dispersion",
                "tint",
                "light",
                "intensity",
                "presence"
            ]
        );
        // Four height reads for the slope and at most three backdrop reads.
        assert!(
            package.work_per_pixel().samples <= 7,
            "{:?}",
            package.work_per_pixel()
        );
    }
}
