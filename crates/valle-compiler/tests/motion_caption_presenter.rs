#![cfg(feature = "motion")]
use serde_json::{Value, json};
use valle_compiler::{
    motion::{PrepareDataBinding, compile_motion_with_full_env_and_data},
    timeline::prepare_caption_presenter_data,
};
use valle_motion::{AssetControl, AssetKind, ContentDigest, ResourceRef, SceneArtifact};
use valle_timeline::{MotionRole, OverlayHold, RationalTime, wire::timeline::*};

const ROLE: &str = "captionPresenter({intro:seconds(0.15),outro:seconds(0.15)})";
fn source(role: &str, controls: &str) -> String {
    format!(
        r#"export const composition={{width:240,height:80,fps:30,duration:1}};
export const role={role}; {controls}
export default function Words(props) {{ const ctx = useTime(); const caption = useCaption();return <Scene>{{caption.runs.map((run,index)=><View key={{index}} className="absolute" style={{{{left:index*30,width:24,height:12,top:ctx.seconds*10,backgroundColor:ctx.host.seconds>=run.start && ctx.host.seconds<run.end?"yellow":"white"}}}}/>)}}</Scene>;}}"#
    )
}
fn input() -> Value {
    json!({"text":"Build your story","runs":[{"text":"Build ","start":0,"end":0.4},{"text":"your story","start":0.4,"end":1.3,"fontSize":32,"color":"#facc15"}],"style":{"font":"asset://caption","fontSize":24,"color":"#ffffffff"},"region":[24,40,192,32],"align":"bottom-center"})
}
/// What a caption track compiles its template with.
fn layout() -> Value {
    let mut layout = input();
    let object = layout.as_object_mut().unwrap();
    object.remove("text");
    object.remove("runs");
    layout
}
fn bound_input() -> valle_motion::caption::BoundCaption {
    let data = serde_json::from_value(input()).unwrap();
    valle_motion::caption::BoundCaption::from_data(&data, 1.0).unwrap()
}
fn compile(
    src: &str,
    value: Value,
) -> Result<valle_compiler::motion::CompiledMotion, Vec<valle_compiler::motion::CompilerDiagnostic>>
{
    compile_motion_with_full_env_and_data(
        src,
        &[],
        None,
        None,
        Some(&PrepareDataBinding {
            source: "captions.json".into(),
            value,
        }),
    )
}
/// Compile with the host's caption font bound, as a template that draws text needs.
fn compile_with_font(
    src: &str,
    value: Value,
) -> Result<valle_compiler::motion::CompiledMotion, Vec<valle_compiler::motion::CompilerDiagnostic>>
{
    compile_motion_with_full_env_and_data(
        src,
        &[ResourceRef {
            control: "caption".into(),
            content_hash: ContentDigest::of_bytes(b"caption font"),
        }],
        None,
        None,
        Some(&PrepareDataBinding {
            source: "captions.json".into(),
            value,
        }),
    )
}
#[test]
fn caption_role_draws_a_row_per_run_and_reserves_the_host_font() {
    let compiled = compile(&source(ROLE, ""), layout()).unwrap();
    assert_eq!(
        compiled.artifact.role,
        MotionRole::CaptionPresenter {
            intro: RationalTime::new(3, 20).unwrap(),
            outro: RationalTime::new(3, 20).unwrap()
        }
    );
    // The template draws one row, which a caption repeats for each of its runs.
    assert_eq!(compiled.artifact.nodes.len(), 2);
    assert_eq!(compiled.artifact.caption_rows.len(), 1);
    assert!(compiled.artifact.reads_caption());
    let bound = compiled.artifact.bind_caption(&bound_input()).unwrap();
    assert_eq!(bound.nodes.len(), 3);
    assert!(bound.caption_rows.is_empty());
    // `--data` gives a whole caption; the template is prepared with its layout alone.
    assert_eq!(
        compile(&source(ROLE, ""), input()).unwrap().artifact,
        compiled.artifact
    );
    assert!(compiled.artifact.controls.data.is_empty());
    assert_eq!(
        compiled.artifact.controls.assets["caption"],
        AssetControl {
            kind: AssetKind::Font,
            required: true
        }
    );
    for controls in [
        "export const controls={data:{}};",
        "export const controls={data:{text:string()}};",
        "export const controls={assets:{caption:asset({kind:'font'})}};",
    ] {
        let error = compile(&source(ROLE, controls), input()).unwrap_err();
        assert!(
            error
                .iter()
                .any(|error| error.message.contains("captionPresenter"))
        );
    }
    // Export order cannot hide a collision with the reserved slot.
    let before = source(ROLE, "export const controls={data:{}};")
        .replace(&format!("export const role={ROLE};"), "")
        + &format!("export const role={ROLE};");
    assert!(compile(&before, input()).is_err());
    // Only a captionPresenter reads a caption, and no other node takes the key a row binds to.
    let mut clip = compiled.artifact.clone();
    clip.role = MotionRole::Clip;
    assert!(
        clip.validate()
            .unwrap_err()
            .iter()
            .any(|error| error.path == "/captionRows")
    );
    let mut taken = compiled.artifact.clone();
    let row = taken.node_children[taken.caption_rows[0].children.start as usize];
    let root = taken.root.0 as usize;
    taken.nodes[root].key = format!("{}#1", taken.nodes[row.0 as usize].key);
    assert!(
        taken
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.path == format!("/nodes/{root}/key"))
    );
    let mut forged: SceneArtifact = compiled.artifact;
    forged.controls.assets.remove("caption");
    assert!(
        forged
            .validate()
            .unwrap_err()
            .iter()
            .any(|error| error.path == "/controls/assets/caption")
    );
}
#[test]
fn caption_role_and_data_shapes_are_closed() {
    for role in [
        "captionPresenter({intro:0,outro:seconds(0)})",
        "captionPresenter({intro:seconds(0),outro:seconds(0),hold:'once'})",
        "captionPresenter({intro:seconds(0),outro:seconds(0),extra:true})",
        "captionPresenter({intro:seconds(-0.1),outro:seconds(0)})",
        "captionPresenter({intro:seconds(0.6),outro:seconds(0.5)})",
    ] {
        assert!(compile(&source(role, ""), input()).is_err(), "{role}");
    }
    assert!(
        compile_motion_with_full_env_and_data(&source(ROLE, ""), &[], None, None, None).is_err()
    );
    for (pointer, value) in [
        ("/unknown", json!(true)),
        ("/text", json!("different")),
        ("/style/font", json!("serif")),
        ("/style/fontSize", json!(0)),
        ("/region", json!([0, 0, 0, 20])),
        ("/runs/1/start", json!(0.3)),
        ("/runs/1/start", json!(0.4000004)),
        ("/runs/0/end", Value::Null),
        ("/runs/0/start", json!(-0.1)),
        ("/runs/1/color", json!("bad color")),
    ] {
        let mut data = input();
        if pointer == "/unknown" {
            data["unknown"] = value;
        } else {
            *data.pointer_mut(pointer).unwrap() = value;
        }
        let errors = compile(&source(ROLE, ""), data).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.source_path.as_deref() == Some("captions.json")),
            "{pointer}: {errors:?}"
        );
    }
}
fn author_clip() -> TimelineCaptionClipWire {
    serde_json::from_value(json!({"start":3.2,"duration":1.6,"runs":input()["runs"]})).unwrap()
}
fn style() -> TimelineCaptionStyleWire {
    serde_json::from_value(json!({"font":"bodyFont","fontSize":24,"color":"#ffffffff"})).unwrap()
}
#[test]
fn caption_preparation_inherits_layout_converts_pixels_and_validates_host_runs() {
    let clip = author_clip();
    let layout: TimelineCaptionLayoutWire =
        serde_json::from_value(json!({"region":[0.1,0.5,0.8,0.4],"align":"bottom-center"}))
            .unwrap();
    let data = prepare_caption_presenter_data(
        &style(),
        Some(&layout),
        &clip,
        [240, 80],
        "/tracks/caption/0/clips/0",
    )
    .unwrap();
    assert_eq!(
        data,
        serde_json::from_value::<valle_motion::caption::CaptionPresenterData>(input()).unwrap()
    );
    let mut shorter = clip.clone();
    shorter.duration = TimelineTimeWire::new("1.2").unwrap();
    assert!(
        prepare_caption_presenter_data(&style(), None, &shorter, [240, 80], "clip")
            .unwrap_err()
            .to_string()
            .contains("runs/1")
    );
    let mut missing_end = clip.clone();
    missing_end.runs.as_mut().unwrap()[0].end = None;
    assert!(
        prepare_caption_presenter_data(&style(), None, &missing_end, [240, 80], "clip").is_err()
    );
    for field in ["enter", "display", "exit", "presentation", "behavior"] {
        let mut value = serde_json::to_value(&clip).unwrap();
        value[field] = match field {
            "enter" | "exit" => json!({"preset":"fade"}),
            "display" => json!({"preset":"pulse"}),
            "presentation" => json!({}),
            _ => json!({"type":"karaoke","mode":"word"}),
        };
        // Use valid public values so rejection comes from presenter placement rather than decoding.
        let clip = serde_json::from_value::<TimelineCaptionClipWire>(value).unwrap();
        assert!(
            prepare_caption_presenter_data(&style(), None, &clip, [240, 80], "clip")
                .unwrap_err()
                .to_string()
                .contains(field)
        );
    }
}
#[test]
fn caption_template_identity_is_the_track_layout_not_the_caption() {
    let key =
        |clip: &TimelineCaptionClipWire, style: &TimelineCaptionStyleWire, canvas: [u32; 2]| {
            let author = json!({"canvas":{"width":canvas[0],"height":canvas[1],"fps":30},
          "resources":{"words":"words.motion.tsx","bodyFont":"body.ttf","otherFont":"other.ttf"},
          "tracks":{"caption":[{"presenter":{"component":"words"},"style":style,"clips":[clip]}]}});
            let timeline = valle_timeline::decode_timeline(&author.to_string()).unwrap();
            valle_compiler::motion_preparation_inputs(&timeline).unwrap()[0].key()
        };
    let clip = author_clip();
    let style = style();
    let original = key(&clip, &style, [240, 80]);
    // A track's captions share one compile: placement, words and timing are bound as each draws.
    let mut shifted = clip.clone();
    shifted.start = TimelineTimeWire::new("90").unwrap();
    shifted.duration = TimelineTimeWire::new("2.5").unwrap();
    assert_eq!(original, key(&shifted, &style, [240, 80]));
    let mut words = clip.clone();
    words.runs.as_mut().unwrap()[0].text = "Write ".into();
    assert_eq!(original, key(&words, &style, [240, 80]));
    let mut timed = clip.clone();
    timed.runs.as_mut().unwrap()[1].end = Some(TimelineTimeWire::new("1.4").unwrap());
    assert_eq!(original, key(&timed, &style, [240, 80]));
    // The track style, its font and the canvas the region is in pixels of are compiled in.
    let mut colored = style.clone();
    colored.color = Some("#ff0000".into());
    assert_ne!(original, key(&clip, &colored, [240, 80]));
    let mut font = style.clone();
    font.font = "otherFont".into();
    assert_ne!(original, key(&clip, &font, [240, 80]));
    assert_ne!(original, key(&clip, &style, [480, 80]));
    let one = compile(&source(ROLE, ""), input()).unwrap();
    let mut changed = input();
    changed["runs"][1]["end"] = json!(1.4);
    let two = compile(&source(ROLE, ""), changed).unwrap();
    assert_eq!(one.normalized_ast_digest, two.normalized_ast_digest);
    assert_eq!(one.artifact, two.artifact);
}

#[test]
fn caption_template_reads_the_caption_where_it_draws() {
    let presenter = |body: &str| {
        format!(
            "export const composition={{width:240,height:80,fps:30,duration:1}};\nexport const role={ROLE};\n{body}"
        )
    };
    let caption = bound_input();
    // One Text draws a Span per run; the layout is prepared, the words are bound.
    let spans = compile_with_font(
        &presenter(
            r#"export default function Words() { const { runs, style } = useCaption(); const size = style.fontSize * 2;
return <Scene><Text style={{fontFamily:style.font,fontSize:size}}>{runs.map((run) => <Span style={{color:run.color,fontSize:run.fontSize}}>{run.text}</Span>)}</Text></Scene>; }"#,
        ),
        layout(),
    )
    .unwrap();
    assert_eq!(spans.artifact.caption_rows.len(), 1);
    let bound = spans.artifact.bind_caption(&caption).unwrap();
    assert_eq!(bound.nodes.len(), spans.artifact.nodes.len() + 1);
    assert!(!bound.reads_caption());
    // A component prop, the whole text and the run count read the caption where they are drawn.
    let prop = compile_with_font(
        &presenter(
            r#"function Line(props) { return <Text style={{fontFamily:props.caption.style.font,fontSize:12,opacity:props.caption.runs.length>1?1:0.5}}>{props.caption.text}</Text>; }
export default function Words() { const caption = useCaption(); return <Scene><Line caption={caption}/></Scene>; }"#,
        ),
        layout(),
    )
    .unwrap();
    assert!(prop.artifact.reads_caption() && prop.artifact.caption_rows.is_empty());
    prop.artifact.bind_caption(&caption).unwrap();
    // A template that does not read its caption draws the same for every caption.
    let still = compile_with_font(
        &presenter(
            r##"export default function Cover() { const caption = useCaption(); return <Scene><View style={{width:caption.region[2],height:caption.style.fontSize,backgroundColor:"#0000ff"}}/></Scene>; }"##,
        ),
        layout(),
    )
    .unwrap();
    assert!(!still.artifact.reads_caption());

    for (body, message) in [
        (
            "export default function Words() { const data = useData(); return <Scene/>; }",
            "useCaption()",
        ),
        (
            "export default function Words() { const caption = useCaption(); const loud = caption.text.toUpperCase(); return <Scene><Text style={{fontFamily:caption.style.font,fontSize:12}}>{loud}</Text></Scene>; }",
            "textTransform",
        ),
        (
            "export default function Words() { const caption = useCaption(); return <Scene>{caption.runs.map((run) => <View key=\"row\">{caption.runs.map((other) => <View/>)}</View>)}</Scene>; }",
            "each run has one row",
        ),
        (
            "export default function Words() { const [first] = useCaption().runs; return <Scene/>; }",
            "cannot be destructured",
        ),
        (
            "export default function Words() { const caption = useCaption(); return <Scene><View style={{opacity:caption.runs[0].start}}/></Scene>; }",
            "runs.map",
        ),
        (
            "export default function Words() { const caption = useCaption(); return <Scene><View style={{opacity:caption.speaker}}/></Scene>; }",
            "not part of the caption",
        ),
    ] {
        let errors = compile_with_font(&presenter(body), layout()).unwrap_err();
        assert!(
            errors.iter().any(|error| error.message.contains(message)),
            "{body}: {errors:?}"
        );
    }
    let overlay = "export const composition={width:240,height:80,fps:30,duration:1};\nexport default function Title() { const caption = useCaption(); return <Scene/>; }";
    assert!(
        compile_motion_with_full_env_and_data(overlay, &[], None, None, None)
            .unwrap_err()
            .iter()
            .any(|error| error.message.contains("captionPresenter"))
    );
}

#[test]
fn documented_caption_presenter_draws_each_run() {
    let guide = include_str!("../../../docs/motion.md");
    let section = &guide[guide.find("### Caption presenter role").unwrap()..];
    let start = section.find("```tsx\n").unwrap() + "```tsx\n".len();
    let example = &section[start..start + section[start..].find("```").unwrap()];
    let compiled = compile_with_font(example, layout()).unwrap();
    assert_eq!(compiled.artifact.caption_rows.len(), 1);
    let bound = compiled.artifact.bind_caption(&bound_input()).unwrap();
    assert!(!bound.reads_caption());
    valle_motion::prepare_scene(&bound).unwrap();
}

#[test]
fn caption_timing_equals_once_overlay_at_exact_endpoints_and_fractional_frames() {
    let t = |n, d| RationalTime::new(n, d).unwrap();
    let role = MotionRole::CaptionPresenter {
        intro: t(3, 20),
        outro: t(3, 20),
    };
    let overlay = MotionRole::Overlay {
        intro: t(3, 20),
        outro: t(3, 20),
        hold: OverlayHold::Once,
    };
    for host in [t(3, 10), t(3, 2), t(4, 1)] {
        role.validate(t(1, 1), Some(host)).unwrap();
        for sample in [
            t(0, 1),
            t(3, 20),
            t(19 * 1001, 30000),
            host.checked_sub(t(1, 30)).unwrap(),
            host,
        ] {
            assert_eq!(
                role.template_time(sample, host, t(1, 1)).unwrap(),
                overlay.template_time(sample, host, t(1, 1)).unwrap()
            );
        }
    }
    assert!(role.validate(t(1, 1), Some(t(299999, 1000000))).is_err());
}

#[test]
fn native_caption_fixture_has_a_canonical_artifact_shared_with_wasm() {
    let compiled = compile(
        include_str!("fixtures/motion/caption/timed-word-bars.motion.tsx"),
        serde_json::from_str(include_str!(
            "fixtures/motion/caption/timed-word-bars.data.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let digest = valle_motion::ContentDigest::of_bytes(
        &valle_motion::canonical_bytes(&compiled.artifact).unwrap(),
    );
    assert_eq!(
        digest.to_string(),
        "sha256:ddda40286d58b1c34b678ea351d80bb347e3d37cf09577a04dd597cc881ae206"
    );
}

#[test]
fn caption_input_obeys_existing_prepare_budgets_and_cannot_occupy_a_visual_track() {
    let mut data = input();
    data["runs"] = json!(vec![
        json!({"text":"","start":0,"end":1,"fontSize":24,"color":"#ffffff"});
        9500
    ]);
    data["text"] = json!("");
    assert!(
        compile(&source(ROLE, ""), data)
            .unwrap_err()
            .iter()
            .any(|error| error.message.contains("value budget"))
    );
    let mut data = input();
    data["text"] = json!("a".repeat(1024 * 1024));
    data["runs"] = json!([{"text":data["text"]}]);
    assert!(
        compile(&source(ROLE, ""), data)
            .unwrap_err()
            .iter()
            .any(|error| error.message.contains("byte budget"))
    );
    let document = valle_timeline::decode_timeline(&json!({"canvas":{"width":240,"height":80,"fps":30},"resources":{"words":"words.motion.tsx"},"tracks":{"visual":[{"clips":[{"kind":"motion","component":"words","start":0,"duration":2}]}]}}).to_string()).unwrap();
    let metadata = std::collections::BTreeMap::from([(
        "words".into(),
        valle_timeline::MotionSourceMetadata {
            duration: RationalTime::new(1, 1).unwrap(),
            role: MotionRole::CaptionPresenter {
                intro: RationalTime::new(3, 20).unwrap(),
                outro: RationalTime::new(3, 20).unwrap(),
            },
        },
    )]);
    let error = valle_compiler::compile_timeline_with_motion_sources(document, &metadata)
        .unwrap_err()
        .to_string();
    assert!(error.contains("captionPresenter") && error.contains("/tracks/visual/0/clips/0"));
    assert!(
        error.contains("component `words`") && !error.contains("motion-"),
        "{error}"
    );
}

#[test]
fn caption_track_presenter_has_a_closed_wire_shape_and_uses_caption_items() {
    let author = json!({ "canvas": {"width":240,"height":80,"fps":30}, "resources":{"words":"words.motion.tsx","bodyFont":"body.ttf"},
      "tracks":{"caption":[{"presenter":{"component":"words"},"style":{"font":"bodyFont","fontSize":24},"clips":[{"start":3.2,"duration":1.6,"runs":input()["runs"]}]}]} });
    let decoded = valle_timeline::decode_timeline(&author.to_string()).unwrap();
    let prepared = valle_compiler::motion_preparation_inputs(&decoded).unwrap();
    assert_eq!(prepared.len(), 1);
    assert_eq!(prepared[0].clip_path, "/tracks/caption/0/clips/0");
    assert_eq!(prepared[0].resources["caption"], "bodyFont");
    // The template compiles with the layout; the words travel with the caption item.
    let fields = |data: &serde_json::Map<String, Value>| data.keys().cloned().collect::<Vec<_>>();
    assert_eq!(
        fields(prepared[0].data.as_object().unwrap()),
        ["align", "region", "style"]
    );
    let metadata = std::collections::BTreeMap::from([(
        "words".into(),
        valle_timeline::MotionSourceMetadata {
            duration: RationalTime::ONE,
            role: MotionRole::CaptionPresenter {
                intro: RationalTime::new(3, 20).unwrap(),
                outro: RationalTime::new(3, 20).unwrap(),
            },
        },
    )]);
    let canonical =
        valle_compiler::compile_timeline_with_motion_sources(decoded.clone(), &metadata)
            .unwrap()
            .to_wire();
    use valle_timeline::internal::wire::document::CaptionItemWire;
    let CaptionItemWire::Motion(clip) = &canonical.document.captions.tracks[0].items[1] else {
        panic!("presenter must belong to the caption band")
    };
    assert_eq!(
        fields(&clip.source.data.clone().into_iter().collect()),
        ["align", "region", "style"]
    );
    assert_eq!(
        clip.runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>(),
        "Build your story"
    );
    assert_eq!(clip.source.resources["caption"], "resource:bodyFont");
    let wrong_role = std::collections::BTreeMap::from([(
        "words".into(),
        valle_timeline::MotionSourceMetadata {
            duration: RationalTime::ONE,
            role: MotionRole::Clip,
        },
    )]);
    assert!(valle_compiler::compile_timeline_with_motion_sources(decoded, &wrong_role).is_err());
    for invalid in [
        json!({"component":"words","data":{}}),
        json!({"component":"words","resources":{"caption":"bodyFont"}}),
        json!({"component":"missing"}),
    ] {
        let mut author = author.clone();
        author["tracks"]["caption"][0]["presenter"] = invalid;
        assert!(valle_timeline::decode_timeline(&author.to_string()).is_err());
    }
}
