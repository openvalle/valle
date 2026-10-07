use serde_json::json;
use valle_project::assets::{
    AddMode, AssetKind, Ctx, ErrorCode, Home, Verb,
    add::add,
    analysis::{AnalyzerOutput, write_slot},
    execute::execute,
    read,
};

#[test]
fn read_projections_preserve_transcripts_annotations_costs_and_filters() {
    let root = tempfile::tempdir().unwrap();
    let ctx = Ctx::bare(Home::at(root.path().join("home"))).with_actor("agent:read-contract");
    let input = root.path().join("speech.wav");
    std::fs::write(
        &input,
        b"stored audio identity; no decode is needed for read projections",
    )
    .unwrap();
    let hash = add(
        &ctx,
        &input,
        AddMode::Copy,
        None,
        Some("Greeting"),
        &["speech".into()],
    )
    .unwrap()
    .content_digest
    .as_hex();
    assert_eq!(
        read::transcript(&ctx, &hash, "paragraph").unwrap_err().code,
        ErrorCode::BadQuery
    );
    let missing = read::transcript(&ctx, &hash, "word").unwrap_err();
    assert_eq!(missing.code, ErrorCode::NotFound);
    assert!(missing.hint.unwrap().contains("existing asr@1"));
    write_slot(
        &ctx,
        &hash,
        "asr",
        1,
        &json!({}),
        &AnalyzerOutput {
            items: vec![
                json!({"text":"Hello ","start_ms":100,"end_ms":300}),
                json!({"text":"world.","start_ms":300,"end_ms":800}),
            ],
            cost_ms: 25,
            cost_fen: 3,
        },
    )
    .unwrap();
    let words = read::transcript(&ctx, &hash[..12], "word").unwrap();
    assert_eq!(words["text"], "Hello world.");
    assert_eq!(words["segments"][0]["start"], 0.1);
    let sentences = read::transcript(&ctx, &hash, "sentence").unwrap();
    assert_eq!(sentences["count"], 1);
    assert_eq!(sentences["text"], "Hello world.");
    let annotation = execute(
        &ctx,
        Verb::Annotate {
            hash: hash.clone(),
            at: Some(0.2),
            range: None,
            text: Some("Opening".into()),
            tags: vec!["intro".into()],
            entities: vec![],
            id: None,
            rm: None,
        },
    );
    assert!(annotation.ok);
    std::fs::write(
        ctx.home.annotations_dir(&hash).join("corrupt.json"),
        b"invalid json",
    )
    .unwrap();
    let card = read::show(&ctx, &hash).unwrap();
    assert_eq!(card["annotations"].as_array().unwrap().len(), 1);
    assert_eq!(card["analysis"][0]["items"], 2);
    assert_eq!(
        execute(
            &ctx,
            Verb::Edit {
                hash: hash.clone(),
                title: Some("Revised".into()),
                subkind: None
            }
        )
        .error
        .unwrap()
        .code,
        ErrorCode::Io
    );
    std::fs::remove_file(ctx.home.annotations_dir(&hash).join("corrupt.json")).unwrap();
    let listed = read::list(&ctx, Some(AssetKind::Audio), Some("speech"), false, false).unwrap();
    assert_eq!(listed["count"], 1);
    assert_eq!(
        read::list(&ctx, Some(AssetKind::Video), None, false, false).unwrap()["count"],
        0
    );
    assert_eq!(
        read::list(&ctx, None, None, true, false).unwrap()["count"],
        0
    );
    let stats = read::describe(&ctx).unwrap();
    assert_eq!(stats["assets"], 1);
    assert_eq!(stats["annotations"], 1);
    assert_eq!(stats["analyzers"][0]["cost_ms"], 25);
    assert_eq!(stats["analyzers"][0]["cost_fen"], 3);
    for value in [
        json!({"verb":"show","hash":hash}),
        json!({"verb":"describe"}),
        json!({"verb":"list","tag":"speech"}),
        json!({"verb":"transcript","hash":hash,"level":"sentence"}),
        json!({"verb":"resolve","hash":hash}),
        json!({"verb":"edit","hash":hash,"title":"Revised"}),
        json!({"verb":"tag","hash":hash,"add":["new"]}),
        json!({"verb":"entity","action":"add","name":"Speaker"}),
        json!({"verb":"entity","action":"list"}),
    ] {
        let report = execute(&ctx, serde_json::from_value(value).unwrap());
        assert!(report.ok, "{report:?}");
    }
    assert!(
        execute(
            &ctx,
            Verb::Rm {
                hash: hash.clone(),
                purge: false,
                force: false
            }
        )
        .ok
    );
    assert_eq!(
        read::list(&ctx, None, None, false, false).unwrap()["count"],
        0
    );
    assert_eq!(
        read::list(&ctx, None, Some("new"), false, true).unwrap()["count"],
        1
    );
    assert_eq!(read::describe(&ctx).unwrap()["removed"], 1);
}
