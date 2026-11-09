//! Native WGSL transitions. The same shade(ShaderInput) function as ShaderLayer receives
//! two child images through getFromColor/getToColor. The uniform struct has a progress field;
//! built-in parameter defaults and ranges are explicit, never parsed out of shader comments.

pub const IMAGES: [&str; 2] = ["valle_from", "valle_to"];

/// Attach the two image bindings and sampling functions, without modifying authored WGSL.
pub fn wrap(code: &str) -> String {
    format!(
        "{code}\n{}",
        include_str!("../shaders/transitions/common.wgsl")
    )
}

pub struct Parameter {
    pub name: &'static str,
    pub default: f64,
    pub min: f64,
    pub max: f64,
}

pub struct Transition {
    pub code: &'static str,
    pub params: &'static [Parameter],
}

const fn param(name: &'static str, default: f64, min: f64, max: f64) -> Parameter {
    Parameter {
        name,
        default,
        min,
        max,
    }
}

pub fn kind(name: &str) -> Option<Transition> {
    Some(match name {
        "fade" => Transition {
            code: include_str!("../shaders/transitions/fade.wgsl"),
            params: const { &[] },
        },
        "wipeLeft" => Transition {
            code: include_str!("../shaders/transitions/wipeLeft.wgsl"),
            params: const { &[param("softness", 0.0, 0.0, 1.0)] },
        },
        "wipeRight" => Transition {
            code: include_str!("../shaders/transitions/wipeRight.wgsl"),
            params: const { &[param("softness", 0.0, 0.0, 1.0)] },
        },
        "circleOpen" => Transition {
            code: include_str!("../shaders/transitions/circleOpen.wgsl"),
            params: const {
                &[
                    param("centerX", 0.5, 0.0, 1.0),
                    param("centerY", 0.5, 0.0, 1.0),
                    param("softness", 0.5, 0.0, 100.0),
                ]
            },
        },
        "simpleZoom" => Transition {
            code: include_str!("../shaders/transitions/simpleZoom.wgsl"),
            params: const {
                &[
                    param("quickness", 0.8, 0.2, 1.0),
                    param("centerX", 0.5, 0.0, 1.0),
                    param("centerY", 0.5, 0.0, 1.0),
                ]
            },
        },
        "crossWarp" => Transition {
            code: include_str!("../shaders/transitions/crossWarp.wgsl"),
            params: const {
                &[
                    param("centerX", 0.5, 0.0, 1.0),
                    param("centerY", 0.5, 0.0, 1.0),
                ]
            },
        },
        "directionalWarp" => Transition {
            code: include_str!("../shaders/transitions/directionalWarp.wgsl"),
            params: const {
                &[
                    param("directionX", -1.0, -1.0, 1.0),
                    param("directionY", 1.0, -1.0, 1.0),
                    param("softness", 0.5, 0.001, 1.0),
                ]
            },
        },
        "linearBlur" => Transition {
            code: include_str!("../shaders/transitions/linearBlur.wgsl"),
            params: const { &[param("intensity", 0.1, 0.0, 2.0)] },
        },
        "dreamyZoom" => Transition {
            code: include_str!("../shaders/transitions/dreamyZoom.wgsl"),
            params: const {
                &[
                    param("strength", 0.5, 0.0, 4.0),
                    param("centerX", 0.5, 0.0, 1.0),
                    param("centerY", 0.5, 0.0, 1.0),
                ]
            },
        },
        "ripple" => Transition {
            code: include_str!("../shaders/transitions/ripple.wgsl"),
            params: const {
                &[
                    param("frequency", 100.0, 0.0, 256.0),
                    param("speed", 50.0, 0.0, 256.0),
                    param("amplitude", 0.0333333, 0.0, 1.0),
                ]
            },
        },
        "flyEye" => Transition {
            code: include_str!("../shaders/transitions/flyEye.wgsl"),
            params: const {
                &[
                    param("size", 0.04, 0.0, 1.0),
                    param("frequency", 50.0, 0.0, 256.0),
                    param("colorSeparation", 0.3, 0.0, 1.0),
                ]
            },
        },
        "multiplyBlend" => Transition {
            code: include_str!("../shaders/transitions/multiplyBlend.wgsl"),
            params: const { &[param("midpoint", 0.5, 0.001, 0.999)] },
        },
        "perlin" => Transition {
            code: include_str!("../shaders/transitions/perlin.wgsl"),
            params: const {
                &[
                    param("scale", 5.0, 0.001, 128.0),
                    param("smoothness", 0.1, 0.001, 1.0),
                ]
            },
        },
        _ => return None,
    })
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shader;

    #[test]
    fn every_kind_is_valid_wgsl_with_explicit_defaults() {
        for k in KINDS {
            let t = kind(k).unwrap();
            let p = shader::program(&wrap(t.code), &IMAGES).unwrap_or_else(|e| panic!("{k}: {e}"));
            assert!(p.uniforms.iter().any(|u| &*u.name == "progress"));
            for param in t.params {
                assert!(
                    p.uniforms.iter().any(|u| &*u.name == param.name),
                    "{k}: {}",
                    param.name
                );
                assert!((param.min..=param.max).contains(&param.default));
            }
        }
        assert!(kind("swirl").is_none());
    }

    #[test]
    fn errors_point_at_the_authored_lines() {
        let e = shader::program(
            &wrap("fn shade(input: ShaderInput) -> vec4f {\n return nope;\n}"),
            &IMAGES,
        )
        .unwrap_err();
        assert!(e.starts_with("line 2: "), "{e}");
    }
}
