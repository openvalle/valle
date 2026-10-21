//! Conservative artifact-wide font selection. Text whose characters are bounded at packaging time
//! only needs the requested families plus faces that cover missing characters; unbounded text
//! retains coverage.
use std::collections::BTreeSet;
use valle_motion::{
    CaptionInput, ControlType, ControlsSchema, Expr, ExprId, InstanceColumnValues, InstanceGroup,
    MotionValue, NodeKind, SceneArtifact, StyleValue, TemplatePart, TextValue,
};

/// The characters a set of text values can produce, bounded to 1 MiB.
#[derive(Default)]
struct TextBound {
    text: String,
    ascii: bool,
}

impl TextBound {
    fn push(&mut self, value: &str) -> bool {
        if self.text.len().saturating_add(value.len()) > 1_048_576 {
            return false;
        }
        self.text.push_str(value);
        true
    }
    /// Non-string values print as ASCII tokens (`css_token`, number formatting).
    fn ascii(&mut self) -> bool {
        if !self.ascii {
            self.ascii = true;
            self.text.extend((0x20u8..0x7f).map(char::from));
        }
        true
    }
    fn value(&mut self, value: &MotionValue) -> bool {
        match value {
            MotionValue::Str(value) | MotionValue::Enum(value) => self.push(value),
            _ => self.ascii(),
        }
    }
}

/// Every character a text expression can produce: string constants and columns, template text,
/// both branches of a select, every value of a pick, interpolation stops and the choices of a prop
/// control. A free string prop has no bound at packaging time.
fn append_expr_text(
    controls: &ControlsSchema,
    exprs: &[Expr],
    group: Option<&InstanceGroup>,
    id: ExprId,
    bound: &mut TextBound,
    depth: usize,
) -> bool {
    if depth > 64 {
        return false;
    }
    let child = |id: &ExprId, bound: &mut TextBound| {
        append_expr_text(controls, exprs, group, *id, bound, depth + 1)
    };
    let Some(expr) = exprs.get(id.0 as usize) else {
        return false;
    };
    match expr {
        Expr::Const { value } => bound.value(value),
        Expr::InstanceField { column, .. } => {
            let Some(column) = group.and_then(|group| group.columns.get(*column as usize)) else {
                return false;
            };
            match &column.values {
                InstanceColumnValues::Strings(values) => {
                    values.iter().all(|value| bound.push(value))
                }
                InstanceColumnValues::Other(values) => {
                    values.iter().all(|value| bound.value(value))
                }
                InstanceColumnValues::Numbers(_)
                | InstanceColumnValues::Points(_)
                | InstanceColumnValues::Colors(_)
                | InstanceColumnValues::Rects(_)
                | InstanceColumnValues::Bools(_)
                | InstanceColumnValues::Formula { .. } => bound.ascii(),
            }
        }
        Expr::Template { parts } => parts.iter().all(|part| match part {
            TemplatePart::Text { value } => bound.push(value),
            TemplatePart::Expr { expr } => child(expr, bound),
        }),
        Expr::Select {
            when_true,
            when_false,
            ..
        } => child(when_true, bound) && child(when_false, bound),
        Expr::Pick { values, .. } => values.iter().all(|value| child(value, bound)),
        Expr::Interpolate { stops, .. } => stops.iter().all(|stop| bound.value(&stop.output)),
        Expr::Prop { name } => match controls.props.get(name).map(|prop| &prop.control) {
            Some(ControlType::Select { values }) => values.iter().all(|value| bound.push(value)),
            Some(ControlType::String) | None => false,
            Some(
                ControlType::Number { .. }
                | ControlType::Bool
                | ControlType::Color
                | ControlType::Length
                | ControlType::Angle
                | ControlType::Point
                | ControlType::Rect,
            ) => bound.ascii(),
        },
        // A caption's words are bound as it draws; its numbers print in ASCII.
        Expr::Caption { input } => match input {
            CaptionInput::Text | CaptionInput::RunText => false,
            _ => bound.ascii(),
        },
        Expr::InstanceIndex
        | Expr::InstanceCount
        | Expr::Context { .. }
        | Expr::RangeSelector { .. }
        | Expr::MakePoint { .. }
        | Expr::MakeRect { .. }
        | Expr::PathLine { .. }
        | Expr::PathTemplate { .. }
        | Expr::PathCubic { .. }
        | Expr::PathArc { .. }
        | Expr::PathArea { .. }
        | Expr::PathSector { .. }
        | Expr::PathAreaBand { .. }
        | Expr::PathOffset { .. }
        | Expr::PathResample { .. }
        | Expr::PathReverse { .. }
        | Expr::PathRoundCorners { .. }
        | Expr::PathZigzag { .. }
        | Expr::PathNoiseDisplace { .. }
        | Expr::PathPuckerBloat { .. }
        | Expr::PathTwist { .. }
        | Expr::PathSimplify { .. }
        | Expr::PathStrokeToPath { .. }
        | Expr::PathMorph { .. }
        | Expr::PathCompatibleMorph { .. }
        | Expr::PathCompatibleMorphSequence { .. }
        | Expr::PathPointAt { .. }
        | Expr::PathTangentAt { .. }
        | Expr::PathAngleAt { .. }
        | Expr::PathLength { .. }
        | Expr::PathTrajectory { .. }
        | Expr::GeometryField { .. }
        | Expr::ToLength2 { .. }
        | Expr::Add { .. }
        | Expr::Sub { .. }
        | Expr::Mul { .. }
        | Expr::Div { .. }
        | Expr::Neg { .. }
        | Expr::Compare { .. }
        | Expr::MathUnary { .. }
        | Expr::MathBinary { .. }
        | Expr::Noise1D { .. }
        | Expr::Noise2D { .. }
        | Expr::FormatNumber { .. }
        | Expr::SimulationSample { .. }
        | Expr::AudioSample { .. }
        | Expr::Spring { .. }
        | Expr::NodeBounds { .. }
        | Expr::Project3D { .. } => bound.ascii(),
    }
}

pub(super) fn selected_default_fonts(artifact: &SceneArtifact) -> Vec<std::sync::Arc<[u8]>> {
    let mut text = TextBound::default();
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
                TextValue::Static { value } => {
                    if !text.push(value) {
                        dynamic_text = true;
                    }
                }
                TextValue::Expr { expr } => {
                    let exprs =
                        group.map_or(artifact.exprs.as_slice(), |group| group.exprs.as_slice());
                    if !append_expr_text(&artifact.controls, exprs, group, *expr, &mut text, 0) {
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
    // Unbounded strings can introduce any script, including emoji and CJK, at a later frame.
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
    let text = text.text;
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
            "export default function T(){ const ctx = useTime();return <View className={ctx.localFrame<30?'font-serif!':'font-mono'}><Text>Hello</Text></View>}",
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
                "export default function T(){ const ctx = useTime();return <Text className={ctx.localFrame<30?'font-sans':'font-serif!'}>Hello</Text>}"
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
            "export default function T(){ const ctx = useTime();return <Text>{ctx.seconds > 0.5 ? '中文' : 'Hello'}</Text>}",
        );
        assert!(dynamic.len() < defaults.len());
        assert!(
            dynamic
                .iter()
                .any(|font| font.as_ref() == bundled_font("NotoSansCJKsc-Variable.otf"))
        );
        assert!(
            !dynamic
                .iter()
                .any(|font| font.as_ref() == bundled_font("Noto-COLRv1.ttf"))
        );
        let dynamic_weight = select(
            "export default function T(){ const ctx = useTime();return <Text style={{fontWeight:ctx.seconds > 0.5 ? 700 : 400}}>Hello</Text>}",
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
                    "export default function T(){{ const ctx = useTime();return <Text style={{{{{style}}}}}>Hello</Text>}}"
                )),
                dynamic_weight
            );
        }
    }

    /// Non-string values print as ASCII, so counters, formatted numbers and finite choices need
    /// only the requested families; a free string prop can carry any script at a later frame.
    #[test]
    fn bounded_dynamic_text_selects_only_the_requested_families() {
        let defaults = valle_motion::default_motion_fonts();
        assert_eq!(
            select(
                "export default function T(){ const ctx = useTime();return <Text style={{fontFamily:'monospace'}}>{`CH ${padNumber(ctx.localFrame,{width:2})} ${formatNumber(ctx.seconds,{decimals:1})}`}</Text>}"
            ),
            ["NotoSans-Variable.ttf", "NotoSansMono-Regular.ttf"]
                .map(|name| std::sync::Arc::<[u8]>::from(bundled_font(name)))
                .to_vec()
        );
        let choice = select(
            "export const controls={props:{mode:select({default:'晴',values:['晴','雨']}),count:number({default:3})}}; export default function T(props){ const ctx = useTime();return <Text>{`${props.mode} ${props.count}`}</Text>}",
        );
        assert!(choice.len() < defaults.len());
        assert!(
            choice
                .iter()
                .any(|font| font.as_ref() == bundled_font("NotoSansCJKsc-Variable.otf"))
        );
        let free = select(
            "export const controls={props:{title:string({default:'Hello'})}}; export default function T(props){ const ctx = useTime();return <Text>{props.title}</Text>}",
        );
        assert_eq!(free.len(), defaults.len());
    }
}
