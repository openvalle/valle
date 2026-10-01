#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::prepare_scene;

fn admitted(property: &str, gradient: &str) -> bool {
    let gradient = serde_json::to_string(gradient).unwrap();
    let source = format!(
        r#"export default function Demo() {{ return <Scene><View style={{{{ width: 100, height: 100, {property}: {gradient} }}}} /></Scene>; }}"#,
    );
    compile_motion(&source)
        .ok()
        .is_some_and(|compiled| prepare_scene(&compiled.artifact).is_ok())
}

#[test]
fn supported_interpolation_spaces_are_admitted_for_all_gradient_geometries() {
    for property in ["background", "backgroundImage"] {
        for gradient in [
            "linear-gradient(135deg, #020617 0%, #111d4a 52%, #4a154b 100%)",
            "linear-gradient(90deg in srgb, red, blue)",
            "linear-gradient(in oklab, red, blue)",
            "radial-gradient(in oklab, red, blue)",
            "conic-gradient(in oklab, red, blue)",
            "linear-gradient(in srgb-linear, red, blue)",
            "linear-gradient(in oklch longer hue, red, blue)",
            "radial-gradient(in oklch increasing hue, red, blue)",
            "conic-gradient(in oklch decreasing hue, red, blue)",
            "linear-gradient(oklab(60% 0.1 0.1), blue)",
            "linear-gradient(90deg/**/in/**/srgb, red, blue)",
            "LINEAR-GRADIENT(red, blue)",
            "radial-gradient(circle, red, blue)",
            "conic-gradient(from 45deg, red, blue)",
            "repeating-linear-gradient(0deg, transparent 0 79px, #ffffff12 79px 80px)",
        ] {
            assert!(admitted(property, gradient), "{property}: {gradient}");
        }
    }
    assert!(admitted(
        "backgroundImage",
        "linear-gradient(red, blue), radial-gradient(white, black)"
    ));
}

#[test]
fn other_interpolation_spaces_and_image_urls_remain_rejected() {
    for property in ["background", "backgroundImage"] {
        for gradient in [
            "linear-gradient(in lab, red, blue)",
            "radial-gradient(in lab, red, blue)",
            "conic-gradient(in lab, red, blue)",
            "linear-gradient(in display-p3, red, blue)",
            "url(https://example.com/image.png)",
            "linear-gradient(broken)",
        ] {
            assert!(!admitted(property, gradient), "{property}: {gradient}");
        }
    }
}

#[test]
fn mixed_layers_and_escaped_interpolation_keep_the_color_space_boundary() {
    // Takumi cannot parse `none` as one layer of an image list. Previously prepare
    // admitted this string, only for the first layout to fail. Reject it at compile.
    assert!(!admitted(
        "backgroundImage",
        "none, linear-gradient(red, blue), linear-gradient(in srgb, white, black)"
    ));
    for gradient in [
        "linear-gradient(red, blue), linear-gradient(in oklab, white, black)",
        "linear-gradient(in/**/oklab, red, blue)",
        "linear-gradient(color(display-p3 1 0 0), blue)",
        "linear-gradient(rgb(from red r g b), blue)",
    ] {
        assert!(admitted("backgroundImage", gradient), "{gradient}");
    }
    // The upstream gradient-header grammar does not decode an escaped `in` token.
    assert!(!admitted(
        "backgroundImage",
        r"linear-gradient(i\6e oklab, red, blue)"
    ));
}

#[test]
fn unsupported_interpolation_names_have_specific_diagnostics() {
    for space in ["lab", "lch", "display-p3", "hsl"] {
        let source = format!(
            r#"export default function E() {{ return <View style={{{{
            backgroundImage:"linear-gradient(in {space}, red, blue)"
        }}}}/>; }}"#
        );
        let errors = compile_motion(&source).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|d| d.message.contains("interpolation") && d.message.contains(space)),
            "{errors:?}"
        );
    }
}
