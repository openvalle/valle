#![cfg(feature = "motion")]
//! Closed module linking, symbol isolation, source maps and dependency invalidation.
use std::collections::BTreeMap;

use valle_compiler::motion::{MotionModuleGraph, compile_motion_modules};
use valle_motion::{MotionValue, StyleValue};

fn graph(entry: &str, modules: &[(&str, &str)]) -> MotionModuleGraph {
    MotionModuleGraph::new(
        entry,
        modules
            .iter()
            .map(|(path, source)| ((*path).to_owned(), (*source).to_owned()))
            .collect::<BTreeMap<_, _>>(),
    )
    .expect("valid graph")
}

const ENTRY: &str = r#"
import { Card as Panel } from "./components/card.motion";
import { accent } from "./theme.motion";

export const component = "ModuleDashboard";
export const controls = defineControls({});

export default function ModuleDashboard(ctx, props, signals) {
  return <Scene key="root">
    <Panel key="panel" label="Linked" color={accent} />
  </Scene>;
}
"#;

const CARD: &str = r#"
export function Card(ctx, props, signals) {
  return <View key="card" style={{ backgroundColor: props.color }}>
    <Text key="label">{props.label}</Text>
  </View>;
}
"#;

const THEME: &str = r##"export const accent = "#22d3ee";"##;

#[test]
fn relative_modules_aliases_and_source_map_compile_as_one_closed_program() {
    let compiled = compile_motion_modules(&graph(
        "dashboard.motion.tsx",
        &[
            ("dashboard.motion.tsx", ENTRY),
            ("components/card.motion.tsx", CARD),
            ("theme.motion.ts", THEME),
        ],
    ))
    .unwrap_or_else(|diagnostics| panic!("module graph must compile: {diagnostics:#?}"));

    assert_eq!(compiled.artifact.component, "ModuleDashboard");
    assert_eq!(compiled.source_map.version, 1);
    assert_eq!(compiled.source_map.entry, "dashboard.motion.tsx");
    assert_eq!(compiled.source_map.modules.len(), 3);
    assert_eq!(
        compiled
            .source_map
            .modules
            .iter()
            .map(|module| module.path.as_str())
            .collect::<Vec<_>>(),
        [
            "components/card.motion.tsx",
            "dashboard.motion.tsx",
            "theme.motion.ts"
        ]
    );
    let card = compiled
        .source_map
        .nodes
        .iter()
        .find(|mapping| mapping.key.ends_with("/card"))
        .expect("expanded imported component node is mapped");
    assert_eq!(card.source_path, "components/card.motion.tsx");
    assert_eq!(card.span.line, 3);
    assert_eq!(card.expansion_stack, ["Panel"]);
}

#[test]
fn module_closure_hash_changes_when_only_a_dependency_changes() {
    let before = compile_motion_modules(&graph(
        "dashboard.motion.tsx",
        &[
            ("dashboard.motion.tsx", ENTRY),
            ("components/card.motion.tsx", CARD),
            ("theme.motion.ts", THEME),
        ],
    ))
    .expect("before compiles");
    let after_theme = r##"export const accent = "#f472b6";"##;
    let after = compile_motion_modules(&graph(
        "dashboard.motion.tsx",
        &[
            ("dashboard.motion.tsx", ENTRY),
            ("components/card.motion.tsx", CARD),
            ("theme.motion.ts", after_theme),
        ],
    ))
    .expect("after compiles");

    assert_ne!(before.normalized_ast_digest, after.normalized_ast_digest);
    assert_ne!(
        before.source_map.closure_digest,
        after.source_map.closure_digest
    );
    assert_eq!(before.artifact.component, after.artifact.component);
}

#[test]
fn one_entry_closes_over_ten_local_modules_and_any_dependency_edit_invalidates_it() {
    let imports = (0..10)
        .map(|index| format!("import {{ v{index} }} from './values/v{index}';"))
        .collect::<Vec<_>>()
        .join("\n");
    let sum = (0..10)
        .map(|index| format!("v{index}"))
        .collect::<Vec<_>>()
        .join(" + ");
    let entry = format!(
        r#"
{imports}
export default function TenModuleEntry() {{
  return <View key="root" style={{{{ width: {sum}, height: 20 }}}} />;
}}
"#
    );
    let mut modules = BTreeMap::from([("entry.tsx".to_owned(), entry)]);
    for index in 0..10 {
        modules.insert(
            format!("values/v{index}.ts"),
            format!("export const v{index} = {};", index + 1),
        );
    }

    let before = compile_motion_modules(
        &MotionModuleGraph::new("entry.tsx", modules.clone()).expect("eleven-module graph"),
    )
    .expect("one entry can import ten local modules");
    assert_eq!(before.source_map.modules.len(), 11);

    modules.insert("values/v9.ts".into(), "export const v9 = 20;".into());
    let after = compile_motion_modules(
        &MotionModuleGraph::new("entry.tsx", modules).expect("edited eleven-module graph"),
    )
    .expect("edited ten-module closure compiles");
    assert_ne!(
        before.source_map.closure_digest, after.source_map.closure_digest,
        "editing any reachable dependency must invalidate the closed module graph"
    );
    assert_ne!(
        valle_motion::canonical_bytes(&before.artifact).unwrap(),
        valle_motion::canonical_bytes(&after.artifact).unwrap(),
        "the edited dependency must reach the prepared Artifact"
    );
}

#[test]
fn imported_theme_flows_through_a_provider_into_an_imported_component() {
    let entry = r#"
import { Dashboard } from "./dashboard.motion";
import { brandTheme } from "./theme.motion";
export default function App() {
  return <Scene key="root">
    <ThemeProvider value={brandTheme}><Dashboard key="dashboard" /></ThemeProvider>
  </Scene>;
}
"#;
    let dashboard = r#"
const panelColor = () => useTheme().colors.panel;
export function Dashboard() {
  const theme = useTheme();
  return <View key="panel" style={{ backgroundColor: panelColor(), padding: theme.spacing.card }}>
    <Text key="title" style={{ color: theme.colors.text }}>Revenue</Text>
  </View>;
}
"#;
    let theme = r##"
export const brandTheme = {
  colors: { panel: "#0f172a", text: "#f8fafc" },
  spacing: { card: 24 },
};
"##;
    let compiled = compile_motion_modules(&graph(
        "app.motion.tsx",
        &[
            ("app.motion.tsx", entry),
            ("dashboard.motion.tsx", dashboard),
            ("theme.motion.ts", theme),
        ],
    ))
    .expect("theme scope crosses the deterministic module graph");

    assert_eq!(
        compiled.artifact.nodes.len(),
        3,
        "ThemeProvider disappears before SceneArtifact"
    );
    let colors = compiled
        .artifact
        .nodes
        .iter()
        .flat_map(|node| &node.styles)
        .filter_map(|style| match &style.value {
            StyleValue::Static {
                value: MotionValue::Color(color),
            } => Some(*color),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(colors.contains(&valle_draw::Rgba::rgb(15, 23, 42)));
    assert!(colors.contains(&valle_draw::Rgba::rgb(248, 250, 252)));
    let panel = compiled
        .source_map
        .nodes
        .iter()
        .find(|node| node.key.ends_with("/panel"))
        .expect("imported themed component remains source-addressable");
    assert_eq!(panel.source_path, "dashboard.motion.tsx");
}

#[test]
fn same_top_level_names_in_separate_modules_are_isolated_by_symbol_identity() {
    let entry = r#"
import { Left } from "./left";
import { Right } from "./right";
export const component = "Twins";
export default function Twins(ctx) {
  return <Scene key="root"><Left key="left" /><Right key="right" /></Scene>;
}
"#;
    let left = r#"
const label = "LEFT";
export function Left(ctx) { return <Text key="text">{label}</Text>; }
"#;
    let right = r#"
const label = "RIGHT";
export function Right(ctx) { return <Text key="text">{label}</Text>; }
"#;
    let compiled = compile_motion_modules(&graph(
        "entry.motion.tsx",
        &[
            ("entry.motion.tsx", entry),
            ("left.tsx", left),
            ("right.tsx", right),
        ],
    ))
    .unwrap_or_else(|diagnostics| panic!("isolated module bindings compile: {diagnostics:#?}"));
    let texts = compiled
        .artifact
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            valle_motion::NodeKind::Text { text, .. } => match text {
                valle_motion::TextValue::Static { value, .. } => Some(value.as_str()),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(texts, ["LEFT", "RIGHT"]);
}

#[test]
fn canonical_module_ordinals_are_stable_and_isolate_every_module_path() {
    // Keep this pair: the former truncated path-derived namespace mapped both paths to one name.
    const LEFT_PATH: &str = "collision/module-797876.tsx";
    const RIGHT_PATH: &str = "collision/module-1730603.tsx";
    let entry = r#"
import { label as leftLabel } from "./collision/module-797876";
import { label as rightLabel } from "./collision/module-1730603";
export default function NamespaceCollision() {
  return <Scene key="root">
    <Text key="left">{leftLabel}</Text>
    <Text key="right">{rightLabel}</Text>
  </Scene>;
}
"#;
    let mut forward = BTreeMap::new();
    forward.insert("entry.tsx".to_owned(), entry.to_owned());
    forward.insert(LEFT_PATH.to_owned(), "export const label = 'LEFT';".into());
    forward.insert(
        RIGHT_PATH.to_owned(),
        "export const label = 'RIGHT';".into(),
    );
    let reverse = forward
        .iter()
        .rev()
        .map(|(path, source)| (path.clone(), source.clone()))
        .collect::<BTreeMap<_, _>>();

    let forward = compile_motion_modules(
        &MotionModuleGraph::new("entry.tsx", forward).expect("forward graph"),
    )
    .unwrap_or_else(|diagnostics| panic!("distinct paths compile: {diagnostics:#?}"));
    let reverse = compile_motion_modules(
        &MotionModuleGraph::new("entry.tsx", reverse).expect("reverse graph"),
    )
    .unwrap_or_else(|diagnostics| panic!("reverse graph compiles: {diagnostics:#?}"));

    assert_eq!(forward, reverse, "input insertion order must be irrelevant");
    let texts = forward
        .artifact
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            valle_motion::NodeKind::Text { text, .. } => match text {
                valle_motion::TextValue::Static { value, .. } => Some(value.as_str()),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(texts, ["LEFT", "RIGHT"]);
}

#[test]
fn unreachable_modules_do_not_change_linked_output_or_ordinals() {
    let entry = r#"
import { label } from "./visible";
const __valle_m1_label = "UNUSED_SENTINEL";
export default function ReachableOnly() {
  return <Text key="label">{label}</Text>;
}
"#;
    let mut reachable = BTreeMap::from([
        ("entry.tsx".to_owned(), entry.to_owned()),
        (
            "visible.ts".to_owned(),
            "export const label = 'VISIBLE';".to_owned(),
        ),
    ]);
    let baseline = compile_motion_modules(
        &MotionModuleGraph::new("entry.tsx", reachable.clone()).expect("reachable graph"),
    )
    .expect("reachable graph compiles");

    reachable.insert(
        "000-unreachable.ts".to_owned(),
        "export const label = 'UNREACHABLE';".to_owned(),
    );
    let with_unreachable = compile_motion_modules(
        &MotionModuleGraph::new("entry.tsx", reachable).expect("graph with unreachable module"),
    )
    .expect("unreachable module cannot reserve an ordinal");

    assert_eq!(baseline, with_unreachable);
    assert_eq!(
        with_unreachable
            .source_map
            .modules
            .iter()
            .map(|module| module.path.as_str())
            .collect::<Vec<_>>(),
        ["entry.tsx", "visible.ts"]
    );
}

#[test]
fn unicode_top_level_bindings_remain_distinct_inside_one_module() {
    let entry = r#"
import { Labels } from "./labels";
export default function UnicodeBindings() {
  return <Scene key="root"><Labels key="labels" /></Scene>;
}
"#;
    let labels = r#"
const café = "ACUTE";
const cafè = "GRAVE";
export function Labels() {
  return <View key="pair"><Text key="acute">{café}</Text><Text key="grave">{cafè}</Text></View>;
}
"#;
    let compiled = compile_motion_modules(&graph(
        "entry.tsx",
        &[("entry.tsx", entry), ("labels.tsx", labels)],
    ))
    .unwrap_or_else(|diagnostics| panic!("Unicode bindings compile: {diagnostics:#?}"));
    let texts = compiled
        .artifact
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            valle_motion::NodeKind::Text { text, .. } => match text {
                valle_motion::TextValue::Static { value, .. } => Some(value.as_str()),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(texts, ["ACUTE", "GRAVE"]);
}

#[test]
fn entry_bindings_cannot_capture_compiler_owned_module_names() {
    let entry = r#"
import { label } from "./dependency";
const __valle_m0_label = "ENTRY";
export default function ReservedCollision() {
  return <Text key="label">{label}</Text>;
}
"#;
    let diagnostics = compile_motion_modules(&graph(
        "entry.tsx",
        &[
            ("entry.tsx", entry),
            ("dependency.ts", "export const label = 'DEPENDENCY';"),
        ],
    ))
    .expect_err("entry bindings must not capture generated dependency bindings");

    assert_eq!(diagnostics[0].source_path.as_deref(), Some("entry.tsx"));
    assert!(
        diagnostics[0]
            .message
            .contains("collides with a compiler-owned linked module binding")
    );
}

#[test]
fn missing_exports_report_the_complete_import_chain_at_the_import_site() {
    let entry = r#"
import { Mid } from "./mid";
export const component = "Broken";
export default function Broken(ctx) { return <Mid />; }
"#;
    let mid = r#"
import { Missing } from "./leaf";
export function Mid(ctx) { return <Missing />; }
"#;
    let leaf = "export const present = 1;";
    let diagnostics = compile_motion_modules(&graph(
        "entry.tsx",
        &[("entry.tsx", entry), ("mid.tsx", mid), ("leaf.ts", leaf)],
    ))
    .expect_err("missing export must fail");
    assert_eq!(diagnostics[0].source_path.as_deref(), Some("mid.tsx"));
    assert!(diagnostics[0].message.contains("no value export `Missing`"));
    assert!(diagnostics[0].message.contains("entry.tsx -> mid.tsx"));
}

#[test]
fn cycles_and_root_escape_are_rejected_before_lowering() {
    let cycle = compile_motion_modules(&graph(
        "a.tsx",
        &[
            (
                "a.tsx",
                "import { b } from './b'; export default function A(ctx) { return <View />; }",
            ),
            ("b.ts", "import A from './a'; export const b = 1;"),
        ],
    ))
    .expect_err("cycle must fail");
    assert!(cycle[0].message.contains("a.tsx -> b.ts -> a.tsx"));

    let escaped = compile_motion_modules(&graph(
        "entry.tsx",
        &[(
            "entry.tsx",
            "import { x } from '../outside'; export default function Entry(ctx) { return <View />; }",
        )],
    ))
    .expect_err("root escape must fail");
    assert!(
        escaped[0]
            .message
            .contains("escapes the allowed project root")
    );

    let npm = compile_motion_modules(&graph(
        "entry.tsx",
        &[(
            "entry.tsx",
            "import { x } from 'some-package'; export default function Entry(ctx) { return <View />; }",
        )],
    ))
    .expect_err("npm resolution must fail");
    assert!(npm[0].message.contains("is not relative"));

    let dynamic = compile_motion_modules(&graph(
        "entry.tsx",
        &[
            (
                "entry.tsx",
                "const later = import('./other'); export default function Entry(ctx) { return <View />; }",
            ),
            ("other.ts", "export const value = 1;"),
        ],
    ))
    .expect_err("dynamic imports must fail");
    assert!(dynamic[0].message.contains("dynamic import() is forbidden"));
}

#[test]
fn reexports_resolve_without_emitting_runtime_module_objects() {
    let entry = r##"
import { Card } from "./public";
export const component = "Reexport";
export default function Reexport(ctx) { return <Scene key="root"><Card key="card" label="ok" color="#fff" /></Scene>; }
"##;
    let public = "export { Card } from './components/card.motion';";
    let compiled = compile_motion_modules(&graph(
        "entry.tsx",
        &[
            ("entry.tsx", entry),
            ("public.ts", public),
            ("components/card.motion.tsx", CARD),
        ],
    ))
    .unwrap_or_else(|diagnostics| panic!("re-export graph compiles: {diagnostics:#?}"));
    assert!(
        compiled
            .artifact
            .nodes
            .iter()
            .any(|node| node.key.ends_with("/card"))
    );
}

#[test]
fn erased_types_type_only_imports_as_const_and_satisfies_never_reach_quickjs() {
    let entry = r#"
import { TypedCard } from "./card";
import type { Theme } from "./types";
import { accent } from "./theme";
export const component: string = "Typed";
export default function Typed(ctx: unknown) {
  return <Scene key="root"><TypedCard key="card" color={accent} /></Scene>;
}
"#;
    let card = r#"
import type { Theme } from "./types";
export function TypedCard(ctx: unknown, props: { color: string }) {
  return <View key="box" style={{ backgroundColor: props.color }} />;
}
"#;
    let types = "export interface Theme { accent: string }";
    let theme = r##"
import type { Theme } from "./types";
export const accent: string = ("#22d3ee" as const) satisfies string;
"##;
    let compiled = compile_motion_modules(&graph(
        "entry.motion.tsx",
        &[
            ("entry.motion.tsx", entry),
            ("card.tsx", card),
            ("types.ts", types),
            ("theme.ts", theme),
        ],
    ))
    .unwrap_or_else(|diagnostics| panic!("erased TypeScript shell compiles: {diagnostics:#?}"));
    assert_eq!(compiled.source_map.modules.len(), 4);
    assert!(
        compiled
            .artifact
            .nodes
            .iter()
            .any(|node| node.key.ends_with("/box"))
    );
}
