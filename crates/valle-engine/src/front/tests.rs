use std::collections::BTreeMap;

use super::*;
use crate::template::{Kind, Piece, Prop, Tag};

const HEAD: &str =
    "export const composition = { width: 320, height: 180, fps: 30, duration: 4 };\n";

/// Runs `f` on a thread with the stack the sandbox needs.
fn big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(32 << 20)
        .spawn(f)
        .expect("a thread")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// A front over `files`, the first of them the scene.
fn load(files: &[(&str, &str)]) -> Result<Front, Error> {
    let map: BTreeMap<String, String> = files
        .iter()
        .map(|(path, text)| (path.to_string(), text.to_string()))
        .collect();
    Front::load(files[0].0, &|path| map.get(path).cloned())
}

/// A front over one scene, `HEAD` then `body`.
fn scene(body: &str) -> Front {
    let source = format!("{HEAD}{body}");
    load(&[("scene.motion.tsx", source.as_str())]).expect("the scene loads")
}

fn at(f: &mut Front, seconds: &str) -> Frame {
    let t = Tick::parse_seconds(seconds).expect("a time");
    f.frame(t)
        .unwrap_or_else(|e| panic!("the frame at {seconds} s: {e}"))
}

/// The error loading `body` as a scene or tracing it at `seconds`.
fn error_at(body: &str, seconds: &str) -> String {
    let source = format!("{HEAD}{body}");
    match load(&[("scene.motion.tsx", source.as_str())]) {
        Err(e) => e,
        Ok(mut f) => match f.frame(Tick::parse_seconds(seconds).expect("a time")) {
            Err(e) => e,
            Ok(_) => panic!("no error from\n{body}"),
        },
    }
}

fn error(body: &str) -> String {
    error_at(body, "0")
}

fn value(frame: &Frame, prop: &Prop) -> V {
    match prop {
        Prop::Const(v) => v.clone(),
        Prop::Expr(i) => frame.values[*i as usize].clone(),
    }
}

fn text(v: V) -> String {
    match v {
        V::Str(s) => s.to_string(),
        other => format!("{other:?}"),
    }
}

/// The text of each Text node, its Spans' too, in node order.
fn texts(frame: &Frame) -> Vec<String> {
    fn of(frame: &Frame, i: usize, out: &mut String) {
        if let Kind::Text { content } | Kind::Span { content } = &frame.template.nodes[i].kind {
            for piece in content {
                match piece {
                    Piece::Text(t) => out.push_str(&text(value(frame, t))),
                    Piece::Span(s) => of(frame, *s as usize, out),
                }
            }
        }
    }
    let nodes = &frame.template.nodes;
    (0..nodes.len())
        .filter(|&i| matches!(nodes[i].kind, Kind::Text { .. }))
        .map(|i| {
            let mut s = String::new();
            of(frame, i, &mut s);
            s
        })
        .collect()
}

/// Node `node`'s style property `key`.
fn style(frame: &Frame, node: usize, key: &str) -> V {
    let (_, prop) = frame.template.nodes[node]
        .style
        .iter()
        .find(|(k, _)| &**k == key)
        .unwrap_or_else(|| panic!("no style {key}"));
    value(frame, prop)
}

#[test]
fn static_scenes_trace_once() {
    big_stack(|| {
        let mut f = scene(
            "export default function Scene() {
  return (
    <View className=\"flex  p-4\" style={{ backgroundColor: \"#123456\", opacity: 0.5 }}>
      <Text key=\"title\">Hello {\"world\"}</Text>
    </View>
  );
}
",
        );
        assert_eq!((f.composition.width, f.composition.height), (320, 180));
        let frame = at(&mut f, "0");
        let t = &frame.template;
        assert_eq!(t.roots, [0]);
        assert_eq!(t.nodes[0].kind, Kind::View);
        assert_eq!(t.nodes[0].classes, [Rc::from("flex"), Rc::from("p-4")]);
        assert_eq!(t.nodes[0].site.as_deref(), Some("scene.motion.tsx:4:5"));
        assert_eq!(t.nodes[0].children, [1]);
        assert_eq!(
            style(&frame, 0, "backgroundColor"),
            V::Str("#123456".into())
        );
        assert_eq!(style(&frame, 0, "opacity"), V::Num(0.5));
        assert_eq!(t.nodes[1].path.as_deref(), Some("title"));
        assert_eq!(texts(&frame), ["Hello world"]);
        assert!(t.guards.is_empty());
        for s in ["1", "2.5", "3.9"] {
            at(&mut f, s);
        }
        assert_eq!(f.traces(), 1);
    });
}

#[test]
fn a_mask_traces_its_source_as_a_root_after_it() {
    big_stack(|| {
        let mut f = scene(
            "export default function Scene() {
  return (
    <Mask key=\"m\" source={<Text key=\"s\">V</Text>} mode=\"luminance\" invert>
      <View />
    </Mask>
  );
}
",
        );
        let frame = at(&mut f, "0");
        let t = &frame.template;
        assert_eq!(t.roots, [0]);
        assert_eq!(
            t.nodes[0].kind,
            Kind::Mask {
                source: 1,
                luminance: true,
                invert: true
            }
        );
        assert_eq!(t.nodes[0].children, [3]);
        assert_eq!(
            (&t.nodes[1].kind, &t.nodes[1].children),
            (&Kind::View, &vec![2])
        );
        // Its source is keyed under it.
        assert_eq!(t.nodes[2].path.as_deref(), Some("m/s"));
        assert_eq!(texts(&frame), ["V"]);
    });
}

#[test]
fn svg_elements_trace_with_their_attributes() {
    big_stack(|| {
        let mut f = scene(
            "export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 10 10\" aria-hidden=\"true\">
      <title>Dot</title>
      <g fill=\"red\">
        <circle cx={5} cy={5} r={seconds} />
      </g>
    </svg>
  );
}
",
        );
        let frame = at(&mut f, "2");
        let t = &frame.template;
        let element = |i: usize| match &t.nodes[i].kind {
            Kind::Svg { tag, attributes } => {
                let values = attributes.iter().map(|(k, p)| (&**k, value(&frame, p)));
                (*tag, values.collect::<Vec<_>>())
            }
            kind => panic!("{kind:?}"),
        };
        // Inert attributes and <title> leave nothing.
        let view_box = ("viewBox", V::Str("0 0 10 10".into()));
        assert_eq!(element(0), (Tag::Svg, vec![view_box]));
        assert_eq!(t.nodes[0].children, [1]);
        assert_eq!(element(1), (Tag::G, vec![("fill", V::Str("red".into()))]));
        assert_eq!(t.nodes[1].children, [2]);
        let circle = [("cx", 5.0), ("cy", 5.0), ("r", 2.0)].map(|(k, v)| (k, V::Num(v)));
        assert_eq!(element(2), (Tag::Circle, circle.to_vec()));
        at(&mut f, "3");
        assert_eq!(f.traces(), 1);
    });
}

#[test]
fn spans_trace_into_their_text_with_time_next_to_them() {
    big_stack(|| {
        let mut f = scene(
            "const Strong = (props) => <Span style={{ fontWeight: 700 }}>{props.children}</Span>;
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <Text>
      Time <Span key=\"t\">{seconds} s <Strong>now</Strong></Span>!
    </Text>
  );
}
",
        );
        let frame = at(&mut f, "2");
        let t = &frame.template;
        let Kind::Text { content } = &t.nodes[0].kind else {
            panic!("{:?}", t.nodes[0])
        };
        assert_eq!(content[1], Piece::Span(1));
        assert!(matches!(content[..], [Piece::Text(_), _, Piece::Text(_)]));
        assert_eq!(t.nodes[0].children, [1]);
        let Kind::Span { content } = &t.nodes[1].kind else {
            panic!("{:?}", t.nodes[1])
        };
        assert!(matches!(
            content[..],
            [Piece::Text(Prop::Expr(_)), Piece::Span(2)]
        ));
        assert_eq!(t.nodes[1].path.as_deref(), Some("t"));
        assert!(matches!(t.nodes[2].kind, Kind::Span { .. }));
        assert_eq!(texts(&frame), ["Time 2 s now!"]);
        at(&mut f, "3");
        assert_eq!(f.traces(), 1);
    });
}

#[test]
fn time_flows_through_expressions() {
    big_stack(|| {
        let mut f = scene(
            "const sin = Math.sin;
export default function Scene() {
  const { seconds, progress } = useCurrentTime();
  const { width, duration } = useComposition();
  return (
    <View
      style={{
        opacity: interpolate(seconds, [0, 2], [0, 1]),
        width: width * progress,
        height: seconds ** 2,
        transform: `rotate(${seconds * 90}deg)`,
      }}
    >
      <Text>{`t=${seconds} of ${duration}`}</Text>
      <Text>{sin(seconds)}</Text>
    </View>
  );
}
",
        );
        let frame = at(&mut f, "1");
        assert_eq!(style(&frame, 0, "opacity"), V::Num(0.5));
        assert_eq!(style(&frame, 0, "width"), V::Num(80.0));
        assert_eq!(style(&frame, 0, "height"), V::Num(1.0));
        assert_eq!(
            style(&frame, 0, "transform"),
            V::Str("rotate(90deg)".into())
        );
        let sin = crate::js::number_to_string(libm::sin(1.0));
        assert_eq!(texts(&frame), ["t=1 of 4", sin.as_str()]);
        let frame = at(&mut f, "3");
        assert_eq!(style(&frame, 0, "opacity"), V::Num(1.0));
        assert_eq!(style(&frame, 0, "width"), V::Num(240.0));
        assert_eq!(style(&frame, 0, "height"), V::Num(9.0));
        assert_eq!(texts(&frame)[0], "t=3 of 4");
        assert_eq!(f.traces(), 1);
    });
}

#[test]
fn branches_become_variants() {
    big_stack(|| {
        let mut f = scene(
            "export default function Scene() {
  const { seconds } = useCurrentTime();
  if (seconds < 1) return <Text>early</Text>;
  return seconds < 2 ? (
    <Text>middle</Text>
  ) : (
    <View style={{ opacity: seconds < 3 ? 0.5 : 1 }}>
      <Text>late</Text>
    </View>
  );
}
",
        );
        assert_eq!(texts(&at(&mut f, "0.5")), ["early"]);
        assert_eq!(texts(&at(&mut f, "1.5")), ["middle"]);
        let late = at(&mut f, "2.5");
        assert_eq!(texts(&late), ["late"]);
        assert_eq!(style(&late, 0, "opacity"), V::Num(0.5));
        let kinds: Vec<&str> = late.template.guards.iter().map(|g| &*g.kind).collect();
        assert_eq!(kinds, ["if", "select"]);
        assert_eq!(style(&at(&mut f, "3.5"), 0, "opacity"), V::Num(1.0));
        assert_eq!(texts(&at(&mut f, "0.25")), ["early"]);
        assert_eq!(f.traces(), 3);
    });
}

#[test]
fn escapes_and_indices_retrace() {
    big_stack(|| {
        let mut f = scene(
            "const WORDS = [\"zero\", \"one\", \"two\", \"three\"];
export default function Scene() {
  const { seconds } = useCurrentTime();
  const label = seconds.toFixed(1);
  return (
    <View>
      <Text>{WORDS[Math.floor(seconds)]}</Text>
      <Text>{label}</Text>
    </View>
  );
}
",
        );
        let frame = at(&mut f, "0.5");
        assert_eq!(texts(&frame), ["zero", "0.5"]);
        let escapes: Vec<&str> = frame.template.escapes().map(|g| &*g.at).collect();
        assert_eq!(escapes, ["scene.motion.tsx:5"]);
        assert_eq!(texts(&at(&mut f, "0.6")), ["zero", "0.6"]);
        assert_eq!(texts(&at(&mut f, "1.5")), ["one", "1.5"]);
        assert_eq!(f.traces(), 3);
    });
}

#[test]
fn variants_are_kept_least_recently_used_first() {
    big_stack(|| {
        let mut f = scene(
            "export default function Scene() {
  const { seconds } = useCurrentTime();
  return <Text>{String(Math.round(seconds * 30))}</Text>;
}
",
        );
        for frame in 0..100 {
            let t = f.composition.fps.frame_start(frame).expect("a frame time");
            let shown = f.frame(t).expect("a frame");
            assert_eq!(texts(&shown), [frame.to_string()]);
        }
        assert_eq!((f.traces(), f.variants.len()), (100, VARIANTS));
        at(&mut f, "3");
        assert_eq!(f.traces(), 100);
        at(&mut f, "0");
        assert_eq!(f.traces(), 101);
    });
}

#[test]
fn modules_are_instantiated_once_per_trace() {
    big_stack(|| {
        let scene = format!(
            "{HEAD}import Title, {{ ACCENT, TOKEN }} from \"./lib/title\";
import {{ token }} from \"./sizes.ts\";
let calls = 0;
export default function Scene() {{
  calls = calls + 1;
  const {{ seconds }} = useCurrentTime();
  if (seconds >= 2) return <Text>{{`late ${{calls}}`}}</Text>;
  return (
    <View>
      <Title text={{seconds < 1 ? \"hi\" : \"bye\"}} />
      <Text>{{`${{ACCENT}} ${{calls}} ${{token === TOKEN ? \"shared\" : \"twice\"}}`}}</Text>
    </View>
  );
}}
"
        );
        let title = "import { SIZE, token } from \"../sizes\";
export const ACCENT = \"#f00\";
export const TOKEN = token;
export default function Title(props: { text: string }) {
  return <Text style={{ color: ACCENT, fontSize: SIZE }}>{props.text}</Text>;
}
";
        let sizes = "export const SIZE: number = 48;\nexport const token = {};\n";
        let mut f = load(&[
            ("scene.motion.tsx", scene.as_str()),
            ("lib/title.tsx", title),
            ("sizes.ts", sizes),
        ])
        .expect("the scene loads");
        let frame = at(&mut f, "0.5");
        assert_eq!(texts(&frame), ["hi", "#f00 1 shared"]);
        assert_eq!(style(&frame, 1, "fontSize"), V::Num(48.0));
        // A conditional over text is one variant.
        assert_eq!(texts(&at(&mut f, "1.5")), ["bye", "#f00 1 shared"]);
        // Module state starts afresh every trace.
        assert_eq!(texts(&at(&mut f, "2.5")), ["late 1"]);
        assert_eq!(f.traces(), 2);

        let e = load(&[
            ("scene.motion.tsx", "import { A } from \"./a\";\n"),
            ("a.ts", "import { B } from \"./b\";\nexport const A = 1;\n"),
            ("b.ts", "import { A } from \"./a\";\nexport const B = 2;\n"),
        ])
        .err()
        .expect("a cycle");
        assert!(e.starts_with("import cycle: a.ts → b.ts → a.ts"), "{e}");
        let e = load(&[("scene.motion.tsx", "import { A } from \"./missing\";\n")])
            .err()
            .expect("a missing module");
        assert!(e.contains("cannot import \"missing\""), "{e}");
    });
}

#[test]
fn the_sandbox_has_no_clock_or_code_made_at_run_time() {
    big_stack(|| {
        for (code, want) in [
            (
                "Date.now()",
                "Date is not available in scenes: animation time comes",
            ),
            ("performance.now()", "performance is not available"),
            ("new Promise(() => {})", "Promise is not available"),
            (
                "queueMicrotask(() => {})",
                "queueMicrotask is not available",
            ),
            ("setTimeout(() => {}, 1)", "setTimeout is not available"),
            ("new Proxy({}, {})", "Proxy is not available"),
            ("new WeakRef({})", "WeakRef is not available"),
            ("Intl.NumberFormat", "Intl is not available"),
            ("eval(\"1\")", "eval is not supported"),
            ("(0, eval)(\"1\")", "eval is not supported"),
            (
                "new Function(\"return 1\")",
                "new Function is not supported",
            ),
            (
                "(function () {}).constructor(\"return 1\")",
                "new Function is not supported",
            ),
            (
                "(function* () {}).constructor(\"yield 1\")",
                "new Function is not supported",
            ),
            ("Math.random()", "Math.random() is not supported"),
            ("useState(0)", "useState() is not supported"),
            ("__host.op(\"add\", [1, 2])", "__host"),
            ("__define([\"x\"], () => ({}))", "__define"),
        ] {
            let e = error(&format!(
                "export default function Scene() {{\n  {code};\n  return null;\n}}\n"
            ));
            assert!(e.starts_with("scene.motion.tsx:3: "), "{code}: {e}");
            assert!(e.contains(want), "{code}: {e}");
        }
    });
}

#[test]
fn built_ins_are_frozen() {
    big_stack(|| {
        for code in [
            "Array.prototype.extra = 1",
            "Object.prototype.toString = null",
            "Math.sin = Math.cos",
            "Error.prepareStackTrace = () => \"\"",
            "[].values().__proto__.extra = 1",
            "Object.getPrototypeOf(function* () {}).extra = 1",
            "useCurrentTime().seconds.v = 1",
            "globalThis.leak = 1",
        ] {
            let e = error(&format!(
                "export default function Scene() {{\n  {code};\n  return null;\n}}\n"
            ));
            assert!(
                e.starts_with("scene.motion.tsx:3: TypeError: "),
                "{code}: {e}"
            );
        }
    });
}

#[test]
fn limits_stop_runaway_scripts() {
    big_stack(|| {
        let body = |code: &str| {
            format!(
                "export default function Scene() {{\n  const {{ seconds }} = useCurrentTime();\n  \
                 {code}\n  return null;\n}}\n"
            )
        };
        // QuickJS polls every 10 000 calls and back jumps, so where the limit lands depends on
        // all that ran before; on the loop's back jump it gives the line of the call before.
        for code in ["while (true) {}", "try { while (true) {} } catch {}"] {
            let e = error(&body(code));
            let line = ["scene.motion.tsx:3: ", "scene.motion.tsx:4: "];
            assert!(line.iter().any(|l| e.starts_with(l)), "{e}");
            assert!(e.contains("step limit"), "{e}");
        }
        let e = error(&body(
            "const a = []; while (true) a.push(new Float64Array(1 << 20));",
        ));
        assert!(
            e.contains("scene.motion.tsx:4: InternalError: out of memory"),
            "{e}"
        );
        let e = error(&body("const f = (n) => f(n + 1) + 1; f(0);"));
        assert!(e.contains("Maximum call stack size exceeded"), "{e}");
        let e = error_at(&body("for (let i = 0; i < seconds * 20000; i++) {}"), "1");
        assert!(
            e.contains("decides on the time more than 10000 times"),
            "{e}"
        );
        let e = error_at(
            &body("let x = seconds; for (let i = 0; i < 250001; i++) x = x + 1;"),
            "1",
        );
        assert!(
            e.contains("more than 250000 values that depend on the time"),
            "{e}"
        );
        let e = error(
            "export default function Scene() {
  return <View>{Array.from({ length: 100001 }, (_, i) => <View key={i} />)}</View>;
}
",
        );
        assert!(e.starts_with("scene.motion.tsx:3:"), "{e}");
        assert!(e.contains("more than 100000 elements"), "{e}");
    });
}

#[test]
fn errors_point_at_the_source() {
    big_stack(|| {
        let element = |jsx: &str| {
            error(&format!(
                "export default function Scene() {{\n  return {jsx};\n}}\n"
            ))
        };
        for (jsx, want) in [
            ("<View foo={1} />", "<View> has no prop foo"),
            (
                "<View style={{ \"background-color\": \"red\" }} />",
                "write backgroundColor, not background-color",
            ),
            (
                "<View style={{ \"--accent\": 1 }} />",
                "CSS variables (--accent)",
            ),
            (
                "<View onClick={() => {}} />",
                "onClick: event handlers are not supported",
            ),
            (
                "<View style={{ opacity: [1] }} />",
                "style opacity must be a number or a string",
            ),
            ("<div />", "use <View> instead of <div>"),
            ("<Group />", "<Group> is removed: use <View>"),
            ("<Video />", "<Video> is not a Valle element"),
            (
                "<Image />",
                "<Image> src is a resource declared in controls.resources",
            ),
            (
                "<Image src=\"a.png\" />",
                "such as \"resource://logo\", not \"a.png\"",
            ),
            (
                "<Image src=\"resource://a\"><View /></Image>",
                "<Image> has no children",
            ),
            (
                "<View>hello</View>",
                "text must be inside <Text>: \"hello\"",
            ),
            ("<Text><View /></Text>", "<View> cannot be inside <Text>"),
            ("<Span>a</Span>", "<Span> must be inside <Text>"),
            (
                "<Text><Span><View /></Span></Text>",
                "<View> cannot be inside <Span>",
            ),
            ("<Text><span /></Text>", "use <Span> instead of <span>"),
            ("<Mask />", "<Mask> needs a source"),
            (
                "<Mask source={<View />} mode=\"luma\" />",
                "<Mask> mode is \"alpha\" or \"luminance\", not \"luma\"",
            ),
            (
                "<Mask source={<View />} invert={1} />",
                "<Mask> invert is true or false",
            ),
            ("<Mask source={\"red\"} />", "text must be inside <Text>"),
            ("<circle />", "<circle> must be inside an <svg>"),
            (
                "<svg><svg /></svg>",
                "an <svg> inside an <svg> is not supported yet",
            ),
            (
                "<svg><path vectorEffect=\"non-scaling-stroke\" /></svg>",
                "<path> vectorEffect is not supported yet",
            ),
            (
                "<svg><path fill-rule=\"evenodd\" /></svg>",
                "write fillRule, not fill-rule",
            ),
            (
                "<svg><path d={[1]} /></svg>",
                "<path> d must be a number or a string",
            ),
            (
                "<svg transform=\"scale(2)\" />",
                "<svg> transform is not supported yet",
            ),
            (
                "<svg><circle><rect /></circle></svg>",
                "<circle> holds no elements",
            ),
            (
                "<svg><stop /></svg>",
                "<stop> must be inside a <linearGradient> or <radialGradient>",
            ),
            (
                "<svg><linearGradient><rect /></linearGradient></svg>",
                "<rect> cannot be inside <linearGradient>: it holds <stop>s",
            ),
            (
                "<svg><clipPath><g /></clipPath></svg>",
                "<g> cannot be inside <clipPath>: it holds shapes",
            ),
            ("<mask />", "<mask> must be inside an <svg>"),
            ("<svg>hi</svg>", "text in an <svg> is not supported yet"),
            (
                "<svg><image href=\"data:image/png;base64,\" /></svg>",
                "<image> in an <svg> is not supported yet",
            ),
            ("<svg><View /></svg>", "<View> cannot be inside an <svg>"),
            (
                "<svg><foreignObject /></svg>",
                "<foreignObject> is not supported: put HTML-like content",
            ),
            ("<Text><svg /></Text>", "<svg> cannot be inside <Text>"),
        ] {
            let e = element(jsx);
            assert!(e.starts_with("scene.motion.tsx:3:"), "{jsx}: {e}");
            assert!(e.contains(": TypeError: "), "{jsx}: {e}");
            assert!(e.contains(want), "{jsx}: {e}");
        }
        let e = error("export default function Scene() {\n  throw new Error(\"boom\");\n}\n");
        assert!(
            e.starts_with("scene.motion.tsx:3: Error: boom\n    at Scene"),
            "{e}"
        );
        let e = error(
            "export default function Scene() {
  const { seconds } = useCurrentTime();
  return <View style={{ opacity: interpolate(seconds, [1, 0], [0, 1]) }} />;
}
",
        );
        assert!(
            e.contains("the input range must strictly increase, got [1, 0]"),
            "{e}"
        );
        let e = error("export default 5;\n");
        assert!(
            e.contains("default export must be its root component"),
            "{e}"
        );
        let e = load(&[("s.tsx", "export default function S() { return null; }\n")])
            .err()
            .expect("no composition");
        assert!(
            e.starts_with("s.tsx: a scene exports its composition"),
            "{e}"
        );
    });
}
