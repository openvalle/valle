//! Conservative artifact-wide font selection. Dynamic text retains coverage;
//! static text only needs the requested families plus faces that cover missing characters.
use std::collections::BTreeSet;
use valle_motion::{
    Expr, ExprId, InstanceColumnValues, InstanceGroup, MotionValue, NodeKind, SceneArtifact,
    StyleValue, TemplatePart, TextValue,
};

/// Static template parts and string columns bound every possible character without shaping each row.
fn append_instance_text(
    group: &InstanceGroup,
    id: ExprId,
    text: &mut String,
    depth: usize,
) -> bool {
    fn append_bounded(text: &mut String, value: &str) -> bool {
        if text.len().saturating_add(value.len()) > 1_048_576 {
            return false;
        }
        text.push_str(value);
        true
    }
    if depth > 64 {
        return false;
    }
    match group.exprs.get(id.0 as usize) {
        Some(Expr::Const {
            value: MotionValue::Str(value),
        }) => append_bounded(text, value),
        Some(Expr::InstanceField { column, .. }) => {
            let Some(InstanceColumnValues::Strings(values)) = group
                .columns
                .get(*column as usize)
                .map(|column| &column.values)
            else {
                return false;
            };
            for value in values {
                if !append_bounded(text, value) {
                    return false;
                }
            }
            true
        }
        Some(Expr::Template { parts }) => parts.iter().all(|part| match part {
            TemplatePart::Text { value } => append_bounded(text, value),
            TemplatePart::Expr { expr } => append_instance_text(group, *expr, text, depth + 1),
        }),
        _ => false,
    }
}

pub(super) fn selected_default_fonts(artifact: &SceneArtifact) -> Vec<std::sync::Arc<[u8]>> {
    let mut text = String::new();
    let mut dynamic_text = false;
    let mut has_text = false;
    let mut families = String::from("sans-serif");
    // A node may still name a font-related internal slot or a weight-looking token, so retain the
    // fixed bundled faces rather than selecting by an unexpanded class name. This never consults
    // installed system fonts.
    let mut unknown_family = false;
    let mut nodes = artifact
        .nodes
        .iter()
        .map(|node| (node, None))
        .collect::<Vec<_>>();
    for group in &artifact.instance_groups {
        nodes.push((&group.template, Some(group)));
        let mut children = group.template_children.iter().collect::<Vec<_>>();
        while let Some(child) = children.pop() {
            nodes.push((&child.node, Some(group)));
            children.extend(&child.children);
        }
    }
    for (node, group) in nodes {
        if let NodeKind::Text { text: value, .. } = &node.kind {
            has_text = true;
            match value {
                TextValue::Static { value } => text.push_str(value),
                TextValue::Expr { expr } => {
                    if !group.is_some_and(|group| append_instance_text(group, *expr, &mut text, 0))
                    {
                        dynamic_text = true;
                    }
                }
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
                "font-thin" | "font-extralight" | "font-light" | "font-normal" | "font-medium"
                | "font-semibold" | "font-bold" | "font-extrabold" | "font-black" => {}
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
        return super::motion::shared_default_fonts().to_vec();
    }
    let names: BTreeSet<_> = families
        .split(',')
        .map(|family| family.trim().trim_matches(['\'', '"']).to_lowercase())
        .collect();
    let files = valle_motion::DEFAULT_MOTION_FONT_FILES;
    let font_index = |name| {
        files
            .iter()
            .position(|file| *file == name)
            .expect("bundled font")
    };
    // One variable sans face covers every static, fractional and animated weight.
    let mut selected = BTreeSet::from([font_index("NotoSans-Variable.ttf")]);
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
        let file = files[index];
        if named
            || ((names.contains("serif") || names.contains("ui-serif"))
                && file.starts_with("KaTeX_Main-"))
            || ((names.contains("monospace") || names.contains("ui-monospace"))
                && file == "NotoSansMono-Regular.ttf")
            || (names.contains("emoji") && file == "Noto-COLRv1.ttf")
        {
            selected.insert(index);
        }
    }
    let emoji_index = font_index("Noto-COLRv1.ttf");
    let emoji = ttf_parser::Face::parse(defaults[emoji_index], 0).expect("bundled emoji");
    if text.chars().any(|ch| {
        !ch.is_ascii()
            && emoji
                .glyph_index(ch)
                .is_some_and(|glyph| emoji.is_color_glyph(glyph))
    }) {
        selected.insert(emoji_index);
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
        .map(|index| super::motion::shared_default_fonts()[index].clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bundled_font(name: &str) -> &'static [u8] {
        valle_motion::DEFAULT_MOTION_FONT_FILES
            .iter()
            .zip(valle_motion::default_motion_fonts())
            .find(|(file, _)| **file == name)
            .unwrap()
            .1
    }
    fn select(source: &str) -> Vec<std::sync::Arc<[u8]>> {
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
            vec![std::sync::Arc::<[u8]>::from(defaults[0])]
        );
        let cjk = select("export default function T(){return <Text>Hello 中文</Text>}");
        assert!(
            cjk.iter()
                .any(|font| font.as_ref() == bundled_font("NotoSansCJKsc-Variable.otf"))
        );
        assert!(
            !cjk.iter()
                .any(|font| font.as_ref() == bundled_font("Noto-COLRv1.ttf"))
        );
        let emoji = select("export default function T(){return <Text>Hello 👨‍👩‍👧‍👦 ❤️ 1️⃣</Text>}");
        assert!(
            emoji
                .iter()
                .any(|font| font.as_ref() == bundled_font("Noto-COLRv1.ttf"))
        );
        assert!(
            !emoji
                .iter()
                .any(|font| font.as_ref() == bundled_font("NotoSansCJKsc-Variable.otf"))
        );
        let bold = select(
            "export default function T(){return <View className=\"font-bold\"><Text>Hello</Text></View>}",
        );
        assert_eq!(
            bold,
            vec![std::sync::Arc::<[u8]>::from(bundled_font(
                "NotoSans-Variable.ttf"
            ))]
        );
    }
    #[test]
    fn layout_instance_descendant_text_loads_default_fonts() {
        let fonts = select(include_str!(
            "../../../valle-compiler/tests/fixtures/motion/composition/repeated-cards.motion.tsx"
        ));
        assert_eq!(fonts.len(), 1);
        assert!(fonts[0].as_ref() == bundled_font("NotoSans-Variable.ttf"));
    }
    #[test]
    fn layout_instance_root_text_loads_default_fonts() {
        let fonts = select(include_str!(
            "../../../valle-compiler/tests/fixtures/motion/composition/text-root-instances.motion.tsx"
        ));
        assert_eq!(
            fonts,
            vec![std::sync::Arc::<[u8]>::from(bundled_font(
                "NotoSans-Variable.ttf"
            ))]
        );
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
            [
                "KaTeX_Main-Regular.ttf",
                "KaTeX_Main-Bold.ttf",
                "KaTeX_Main-Italic.ttf",
                "KaTeX_Main-BoldItalic.ttf",
                "NotoSansMono-Regular.ttf"
            ]
            .iter()
            .all(|name| fonts
                .iter()
                .any(|loaded| loaded.as_ref() == bundled_font(name)))
        );
        assert!(
            !fonts
                .iter()
                .any(|font| font.as_ref() == bundled_font("NotoSansCJKsc-Variable.otf"))
        );
        let sans =
            select("export default function T(){return <Text className='font-sans'>Hello</Text>}");
        assert_eq!(sans, vec![std::sync::Arc::<[u8]>::from(defaults[0])]);
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
        assert_eq!(
            dynamic_weight,
            vec![std::sync::Arc::<[u8]>::from(bundled_font(
                "NotoSans-Variable.ttf"
            ))]
        );
        for style in [
            "fontWeight:403.25",
            "fontVariationSettings:`\"wght\" ${100 + ctx.seconds * 800}, \"wdth\" 75`",
        ] {
            assert_eq!(
                select(&format!(
                    "export default function T(ctx){{return <Text style={{{{{style}}}}}>Hello</Text>}}"
                )),
                dynamic_weight
            );
        }
    }
}
