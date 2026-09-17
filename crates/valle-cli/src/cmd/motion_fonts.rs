//! Conservative artifact-wide font selection. Dynamic text retains coverage;
//! static text only needs the requested families plus faces that cover missing characters.
use std::collections::BTreeSet;
use valle_motion::{MotionValue, NodeKind, SceneArtifact, StyleValue, TextValue};

pub(super) fn selected_default_fonts(artifact: &SceneArtifact) -> Vec<Vec<u8>> {
    let mut text = String::new();
    let mut dynamic_text = false;
    let mut has_text = false;
    let mut families = String::from("sans-serif");
    let mut weights = BTreeSet::from([400_u16]);
    let mut all_weights = false;
    // A node may still name a font-related internal slot or a weight-looking token, so retain the
    // fixed bundled faces rather than selecting by an unexpanded class name. This never consults
    // installed system fonts.
    let mut unknown_family = false;
    for node in &artifact.nodes {
        if let NodeKind::Text { text: value, .. } = &node.kind {
            has_text = true;
            match value {
                TextValue::Static { value } => text.push_str(value),
                TextValue::Expr { .. } => dynamic_text = true,
            }
        }
        for style in &node.styles {
            if style.property.starts_with("--font-")
                || style.property.starts_with("--text-")
                    && style.property.ends_with("--font-weight")
            {
                unknown_family = true;
            }
            match (style.property.as_str(), &style.value) {
                (
                    "font-family",
                    StyleValue::Static {
                        value: MotionValue::Str(value) | MotionValue::Enum(value),
                    },
                ) => {
                    if value.contains("var(") || value.contains('\\') {
                        unknown_family = true;
                    }
                    families.push(',');
                    families.push_str(value.trim().trim_end_matches("!important").trim());
                }
                (
                    "font-weight",
                    StyleValue::Static {
                        value: MotionValue::Number(value),
                    },
                ) => {
                    weights.insert(*value as u16);
                }
                (
                    "font-weight",
                    StyleValue::Static {
                        value: MotionValue::Str(value) | MotionValue::Enum(value),
                    },
                ) => match value.as_str() {
                    "normal" => {
                        weights.insert(400);
                    }
                    "bold" => {
                        weights.insert(700);
                    }
                    value => match value.parse() {
                        Ok(value) => {
                            weights.insert(value);
                        }
                        Err(_) => all_weights = true,
                    },
                },
                ("font-weight", _) => all_weights = true,
                ("font-family" | "font", _) => unknown_family = true,
                _ => {}
            }
        }
        // Unknown/arbitrary font utilities retain the full palette. This avoids guessing
        // CSS semantics; ordinary weights and generic families cover the common case.
        for class in node
            .class_names
            .iter()
            .flat_map(|class| class.split_whitespace())
        {
            let authored_class = class;
            let class = class
                .rsplit(':')
                .next()
                .unwrap_or(class)
                .trim_end_matches('!');
            match class {
                "font-sans" => families.push_str(",sans-serif"),
                "font-serif" => families.push_str(",serif"),
                "font-mono" => families.push_str(",monospace"),
                "font-thin" => {
                    weights.insert(100);
                }
                "font-extralight" => {
                    weights.insert(200);
                }
                "font-light" => {
                    weights.insert(300);
                }
                "font-normal" => {
                    weights.insert(400);
                }
                "font-medium" => {
                    weights.insert(500);
                }
                "font-semibold" => {
                    weights.insert(600);
                }
                "font-bold" => {
                    weights.insert(700);
                }
                "font-extrabold" => {
                    weights.insert(800);
                }
                "font-black" => {
                    weights.insert(900);
                }
                _ if authored_class.contains("font") => unknown_family = true,
                _ => {}
            }
        }
    }
    if !has_text {
        return Vec::new();
    }
    let defaults = valle_motion::default_motion_fonts();
    // Unknown strings can introduce any script, including emoji and CJK, at a later frame.
    if dynamic_text || unknown_family {
        return defaults.iter().map(|font| font.to_vec()).collect();
    }
    let names: BTreeSet<_> = families
        .split(',')
        .map(|family| family.trim().trim_matches(['\'', '"']).to_lowercase())
        .collect();
    let mut selected = BTreeSet::new();
    for weight in weights {
        // The bundled sans faces are 400/500/600/700/800. CSS searches downward
        // below 400, 400..500 upward to 500 then downward, and upward above 500.
        let index = if weight <= 400 {
            0
        } else if weight <= 500 {
            1
        } else if weight <= 600 {
            2
        } else if weight <= 700 {
            3
        } else {
            4
        };
        selected.insert(index);
    }
    if all_weights {
        selected.extend(0..5);
    }
    for (index, bytes) in defaults.iter().enumerate() {
        let face = ttf_parser::Face::parse(bytes, 0).expect("bundled font");
        let named = face.names().into_iter().any(|name| {
            matches!(
                name.name_id,
                ttf_parser::name_id::FAMILY | ttf_parser::name_id::TYPOGRAPHIC_FAMILY
            ) && name
                .to_string()
                .is_some_and(|name| names.contains(&name.to_lowercase()))
        });
        if (named && index >= 5)
            || ((names.contains("serif") || names.contains("ui-serif")) && (5..=8).contains(&index))
            || ((names.contains("monospace") || names.contains("ui-monospace")) && index == 9)
            || (names.contains("emoji") && index == 14)
        {
            selected.insert(index);
        }
    }
    let emoji = ttf_parser::Face::parse(defaults[14], 0).expect("bundled emoji");
    if text.chars().any(|ch| {
        !ch.is_ascii()
            && emoji
                .glyph_index(ch)
                .is_some_and(|glyph| emoji.is_color_glyph(glyph))
    }) {
        selected.insert(14);
    }
    let faces: Vec<_> = defaults
        .iter()
        .map(|bytes| ttf_parser::Face::parse(bytes, 0).expect("bundled font"))
        .collect();
    let mut missing: BTreeSet<_> = text
        .chars()
        .filter(|ch| {
            !ch.is_control()
                && !selected
                    .iter()
                    .any(|index| faces[*index].glyph_index(*ch).is_some())
        })
        .collect();
    for (index, face) in faces.iter().enumerate() {
        if missing.iter().any(|ch| face.glyph_index(*ch).is_some()) {
            selected.insert(index);
            missing.retain(|ch| face.glyph_index(*ch).is_none());
        }
    }
    selected
        .into_iter()
        .map(|index| defaults[index].to_vec())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn select(source: &str) -> Vec<Vec<u8>> {
        selected_default_fonts(
            &valle_compiler::motion::compile_motion(source)
                .unwrap()
                .artifact,
        )
    }
    #[test]
    fn static_text_loads_only_needed_faces_and_script_fallbacks() {
        let defaults = valle_motion::default_motion_fonts();
        assert!(select("export default function T(){return <View/>}").is_empty());
        assert_eq!(
            select("export default function T(){return <Text>Hello</Text>}"),
            vec![defaults[0].to_vec()]
        );
        let cjk = select("export default function T(){return <Text>Hello 中文</Text>}");
        assert!(cjk.iter().any(|font| font == defaults[13]));
        assert!(!cjk.iter().any(|font| font == defaults[14]));
        let emoji = select("export default function T(){return <Text>Hello 👨‍👩‍👧‍👦 ❤️ 1️⃣</Text>}");
        assert!(emoji.iter().any(|font| font == defaults[14]));
        assert!(!emoji.iter().any(|font| font == defaults[13]));
        let bold = select(
            "export default function T(){return <View className=\"font-bold\"><Text>Hello</Text></View>}",
        );
        assert!(bold.iter().any(|font| font == defaults[3]));
        assert!(!bold.iter().any(|font| font == defaults[1]));
    }
    /// Every removed font-styling path must fail closed instead of silently keeping the palette:
    /// a variable family, a local `@theme` stylesheet and a custom property have no lowering.
    #[test]
    fn removed_font_variable_and_theme_paths_are_rejected() {
        for (source, needle) in [
            (
                "export default function T(){return <Text style={{fontFamily:'var(--brand, sans-serif)'}}>Hello</Text>}",
                "CSS variable references are not supported",
            ),
            (
                "export default function T(){return <View style={{'--font-sans':'monospace'}}><Text className='font-sans'>Hello</Text></View>}",
                "author CSS custom properties are not supported",
            ),
            (
                "export default function T(){return <Text className='font-sans md:font-serif!'>Hello</Text>}",
                "responsive and state variants are not supported",
            ),
        ] {
            let diagnostics = valle_compiler::motion::compile_motion(source)
                .expect_err("the removed path must not compile");
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(needle)),
                "{source}: {diagnostics:#?}"
            );
        }

        let graph = valle_compiler::motion::MotionModuleGraph::new(
            "scene.tsx",
            std::collections::BTreeMap::from([
                ("scene.tsx".into(), "import './brand.css'; export default function T(){return <Text className='font-sans'>Hello</Text>}".into()),
                ("brand.css".into(), "@theme {--font-sans:monospace;}".into()),
            ]),
        ).unwrap();
        let error = valle_compiler::motion::compile_motion_modules(&graph)
            .expect_err("a local stylesheet is a permanent boundary");
        let message = format!("{error:?}");
        assert!(message.contains("local CSS is not supported"), "{message}");
    }

    #[test]
    fn finite_font_utilities_package_all_candidate_families() {
        let defaults = valle_motion::default_motion_fonts();
        let fonts = select(
            "export default function T(ctx){return <View className={ctx.localFrame<30?'font-serif!':'font-mono'}><Text>Hello</Text></View>}",
        );
        assert!(
            defaults[5..=9]
                .iter()
                .all(|font| fonts.iter().any(|loaded| loaded == font))
        );
        assert!(!fonts.iter().any(|font| font == defaults[13]));
        let sans =
            select("export default function T(){return <Text className='font-sans'>Hello</Text>}");
        assert_eq!(sans, vec![defaults[0].to_vec()]);
    }

    /// Variants are gone, so every finite class choice must package all candidate families: the
    /// frame that selects an inactive branch still needs its face.
    #[test]
    fn finite_class_choices_package_inactive_candidates_too() {
        assert_eq!(
            select(
                "export default function T(ctx){return <Text className={ctx.localFrame<30?'font-sans':'font-serif!'}>Hello</Text>}"
            ),
            select(
                "export default function T(){return <Text className='font-sans font-serif!'>Hello</Text>}"
            )
        );
    }

    #[test]
    fn dynamic_text_and_weights_keep_later_frame_choices() {
        let defaults = valle_motion::default_motion_fonts();
        let dynamic = select(
            "export default function T(ctx){return <Text>{ctx.seconds > 0.5 ? '中文' : 'Hello'}</Text>}",
        );
        assert_eq!(dynamic.len(), defaults.len());
        let dynamic_weight = select(
            "export default function T(ctx){return <Text style={{fontWeight:ctx.seconds > 0.5 ? 700 : 400}}>Hello</Text>}",
        );
        assert!(
            defaults[..5]
                .iter()
                .all(|font| dynamic_weight.iter().any(|loaded| loaded == font))
        );
        assert!(!dynamic_weight.iter().any(|font| font == defaults[13]));
    }
}
