#![cfg(feature = "motion")]

use valle_compiler::motion::compile_motion;
use valle_motion::prepare_scene;

fn admitted(property: &str, gradient: &str) -> bool {
    let gradient = serde_json::to_string(gradient).unwrap();
    let source = format!(
        r#"export default function Demo() {{ return <Scene><View style={{{{ width: 100, height: 100, {property}: {gradient} }}}} /></Scene>; }}"#,
    );
    let artifact = compile_motion(&source)
        .expect("gradient syntax compiles")
        .artifact;
    prepare_scene(&artifact).is_ok()
}

#[test]
fn legacy_and_explicit_srgb_gradients_remain_admitted() {
    for property in ["background", "backgroundImage"] {
        for gradient in [
            "linear-gradient(135deg, #020617 0%, #111d4a 52%, #4a154b 100%)",
            "linear-gradient(90deg in srgb, red, blue)",
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
            "linear-gradient(in oklab, red, blue)",
            "radial-gradient(in oklab, red, blue)",
            "conic-gradient(in oklab, red, blue)",
            "linear-gradient(in srgb-linear, red, blue)",
            "linear-gradient(oklab(60% 0.1 0.1), blue)",
            "url(https://example.com/image.png)",
            "linear-gradient(broken)",
        ] {
            assert!(!admitted(property, gradient), "{property}: {gradient}");
        }
    }
}

#[test]
fn mixed_layers_and_escaped_interpolation_keep_the_color_space_boundary() {
    assert!(admitted(
        "backgroundImage",
        "none, linear-gradient(red, blue), linear-gradient(in srgb, white, black)"
    ));
    for gradient in [
        "linear-gradient(red, blue), linear-gradient(in oklab, white, black)",
        "linear-gradient(in/**/oklab, red, blue)",
        r"linear-gradient(i\6e oklab, red, blue)",
        "linear-gradient(color(display-p3 1 0 0), blue)",
        "linear-gradient(rgb(from red r g b), blue)",
    ] {
        assert!(!admitted("backgroundImage", gradient), "{gradient}");
    }
}
