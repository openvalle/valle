//! CLI adapters for asset library commands. Parse arguments into `Verb`, execute the asset kernel,
//! and render reports. Native media, font, and Lottie probes are assembled here.

use std::path::Path;
use std::process::ExitCode;

use anyhow::Result;
use valle_project::assets::{
    AddMode, AssetKind, AssetsError, Ctx, Home, Probe, ProbeOutcome, Prober, Report, Verb, execute,
    execute_add_batch,
};

use crate::{AssetsAction, EntityCmd};

pub(crate) fn run(json: bool, events: bool, action: AssetsAction) -> Result<ExitCode> {
    if events {
        // Use NDJSON for progress events followed by the final report.
        crate::events::init_ndjson();
    }
    let home = match Home::resolve() {
        Ok(h) => h,
        Err(e) => return Ok(render_report(Report::from_error(e), json)),
    };
    let mut ctx = Ctx::new(home, Box::new(CliProber));
    // Render the same event as human-readable stderr or an NDJSON envelope.
    ctx.progress = Some(Box::new(|s: &str| {
        crate::events::emit(crate::events::EventKind::AnalyzeProgress {
            message: s.to_owned(),
        });
    }));
    ctx.analyzers = super::assets_analyzers::assemble();

    // Expand file arguments; the kernel aggregates per-file failures.
    if let AssetsAction::Add {
        paths,
        mode,
        kind,
        title,
        tags,
    } = action
    {
        let mode = match parse_mode(&mode) {
            Ok(m) => m,
            Err(e) => return Ok(render_report(Report::from_error(e), json)),
        };
        let kind = match kind.as_deref().map(AssetKind::parse).transpose() {
            Ok(k) => k,
            Err(e) => return Ok(render_report(Report::from_error(e), json)),
        };
        let paths: Vec<std::path::PathBuf> = paths.iter().map(Into::into).collect();
        if paths.is_empty() {
            return Ok(render_report(
                Report::from_error(
                    AssetsError::not_found("no asset provided")
                        .with_hint("provide at least one file path"),
                ),
                json,
            ));
        }
        let report = execute_add_batch(&ctx, &paths, mode, kind, title.as_deref(), &tags);
        let all_ok = report
            .data
            .as_ref()
            .and_then(|d| d.get("failed"))
            .and_then(|f| f.as_u64())
            .map(|f| f == 0)
            .unwrap_or(false);
        print_report(&report, json);
        return Ok(if all_ok {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }

    // Keep the optional VLM sidecar alive only for the analysis command.
    let _sidecar = match &action {
        AssetsAction::Analyze { with, .. } if with.iter().any(|w| w == "vlm") => {
            Sidecar::spawn_from_env()
        }
        _ => None,
    };

    let verb = match build(action) {
        Ok(v) => v,
        Err(e) => return Ok(render_report(Report::from_error(e), json)),
    };
    Ok(render_report(execute(&ctx, verb), json))
}

/// Analysis sidecar launched through `VALLE_VLM_SERVER_CMD` (`sh -c`) and terminated on drop. The
/// VLM client handles readiness retries.
pub(crate) struct Sidecar {
    child: std::process::Child,
}

impl Sidecar {
    pub(crate) fn spawn_from_env() -> Option<Sidecar> {
        let cmd = std::env::var("VALLE_VLM_SERVER_CMD").ok()?;
        eprintln!("starting sidecar: {cmd}");
        match std::process::Command::new("sh").arg("-c").arg(&cmd).spawn() {
            Ok(child) => Some(Sidecar { child }),
            Err(e) => {
                eprintln!("  warn: sidecar startup failed: {e}");
                None
            }
        }
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        eprintln!("sidecar stopped");
    }
}

/// Convert CLI arguments to a kernel command; batch imports are handled by `run`.
fn build(action: AssetsAction) -> std::result::Result<Verb, AssetsError> {
    Ok(match action {
        AssetsAction::Add { .. } => unreachable!("add is handled as a batch in run"),
        AssetsAction::Rm { hash, purge, force } => Verb::Rm { hash, purge, force },
        AssetsAction::List {
            kind,
            tag,
            stale,
            removed,
        } => Verb::List {
            kind: kind.as_deref().map(AssetKind::parse).transpose()?,
            tag,
            stale,
            removed,
        },
        AssetsAction::Edit {
            hash,
            title,
            subkind,
        } => Verb::Edit {
            hash,
            title,
            subkind,
        },
        AssetsAction::Tag { hash, add, rm } => Verb::Tag { hash, add, rm },
        AssetsAction::Annotate {
            hash,
            at,
            range,
            text,
            tags,
            entities,
            id,
            rm,
        } => {
            let range = match range {
                Some(v) if v.len() == 2 => Some([v[0], v[1]]),
                Some(_) => {
                    return Err(AssetsError::bad_query(
                        "--range requires two values: START END",
                    ));
                }
                None => None,
            };
            Verb::Annotate {
                hash,
                at,
                range,
                text,
                tags,
                entities,
                id,
                rm,
            }
        }
        AssetsAction::Entity { action } => Verb::Entity {
            op: match action {
                EntityCmd::Add {
                    name,
                    kind,
                    aliases,
                } => valle_project::assets::EntityOp::Add {
                    name,
                    kind,
                    aliases,
                },
                EntityCmd::List => valle_project::assets::EntityOp::List,
                EntityCmd::Edit { id, name, aliases } => {
                    valle_project::assets::EntityOp::Edit { id, name, aliases }
                }
            },
        },
        AssetsAction::Show { hash } => Verb::Show { hash },
        AssetsAction::Describe => Verb::Describe,
        AssetsAction::Analyze {
            hashes,
            all,
            kind,
            tag,
            with,
            budget,
            force,
        } => {
            if with.iter().any(|name| name == "asr") {
                return Err(AssetsError::bad_query(
                    "Assets no longer runs ASR models",
                )
                .with_hint(
                    "run `valle media transcribe -i <input> -o <output.words.json>`; existing asr@1 slots remain readable",
                ));
            }
            Verb::Analyze {
                hashes,
                all,
                kind: kind.as_deref().map(AssetKind::parse).transpose()?,
                tag,
                with,
                budget,
                force,
            }
        }
        AssetsAction::Search {
            query,
            kind,
            tag,
            entity,
            filter,
            limit,
        } => Verb::Search {
            query,
            kind: kind.as_deref().map(AssetKind::parse).transpose()?,
            tag,
            entity,
            filter,
            limit,
        },
        AssetsAction::Resolve { hash } => Verb::Resolve { hash },
        AssetsAction::Transcript { hash, level } => Verb::Transcript { hash, level },
        AssetsAction::Maintenance { action } => match action {
            crate::AssetsMaintenanceAction::Verify { deep } => Verb::Verify { deep },
            crate::AssetsMaintenanceAction::Gc => Verb::Gc,
            crate::AssetsMaintenanceAction::Reindex => Verb::Reindex,
            crate::AssetsMaintenanceAction::Sql { query } => Verb::Sql { query },
        },
    })
}

fn parse_mode(s: &str) -> std::result::Result<AddMode, AssetsError> {
    Ok(match s {
        "reflink" => AddMode::Reflink,
        "copy" => AddMode::Copy,
        "reference" => AddMode::Reference,
        other => {
            return Err(
                AssetsError::unsupported_media(format!("unknown mode '{other}'"))
                    .with_hint("reflink|copy|reference"),
            );
        }
    })
}

// Report rendering.

fn render_report(report: Report, json: bool) -> ExitCode {
    print_report(&report, json);
    if report.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn print_report(report: &Report, json: bool) {
    if crate::events::mode() == crate::events::Mode::Ndjson {
        // The final NDJSON event carries the report unchanged, including errors and warnings.
        crate::events::emit(crate::events::EventKind::Report(
            serde_json::to_value(report).unwrap_or_default(),
        ));
        return;
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(report).unwrap_or_default()
        );
        return;
    }
    if report.ok {
        human_ok(report);
    } else if let Some(e) = &report.error {
        eprintln!("error [{:?}]: {}", e.code, e.message);
        if let Some(h) = &e.hint {
            eprintln!("  hint: {h}");
        }
    }
    for w in &report.warnings {
        eprintln!("  warn: {w}");
    }
}

/// Choose a human-readable representation based on the report shape.
fn human_ok(report: &Report) {
    let data = report.data.clone().unwrap_or_default();

    // Batch import results.
    if let Some(items) = data.get("items").and_then(|v| v.as_array()) {
        for it in items {
            let path = it.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            match it
                .get("content_digest")
                .and_then(|value| value.as_str())
                .and_then(|wire| valle_project::ContentDigest::parse(wire).ok())
            {
                Some(digest) => {
                    let mark = if it.get("revived").and_then(|v| v.as_bool()).unwrap_or(false) {
                        "(revived)"
                    } else if it.get("existed").and_then(|v| v.as_bool()).unwrap_or(false) {
                        "(existing)"
                    } else {
                        ""
                    };
                    let kind = it.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                    println!("{}  {kind:9} {path} {mark}", &digest.as_hex()[..12]);
                }
                None => {
                    let msg = it
                        .pointer("/error/message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("failed");
                    println!("✗            {path}  {msg}");
                }
            }
        }
        return;
    }

    // Search results.
    if let Some(results) = data.get("results").and_then(|v| v.as_array()) {
        if results.is_empty() {
            println!("(no matches)");
            return;
        }
        for r in results {
            let h = r.get("asset").and_then(|v| v.as_str()).unwrap_or("?");
            let title = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let range = match r.get("range") {
                Some(serde_json::Value::Array(a)) if a.len() == 2 => format!(
                    " [{:.1}-{:.1}s]",
                    a[0].as_f64().unwrap_or(0.0),
                    a[1].as_f64().unwrap_or(0.0)
                ),
                Some(serde_json::Value::Array(a)) if a.len() == 1 => {
                    format!(" [@{:.1}s]", a[0].as_f64().unwrap_or(0.0))
                }
                _ => String::new(),
            };
            let ev = r
                .get("evidence")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .map(|e| {
                    format!(
                        "  {}:{}",
                        e.get("field").and_then(|v| v.as_str()).unwrap_or(""),
                        e.get("snippet").and_then(|v| v.as_str()).unwrap_or("")
                    )
                })
                .unwrap_or_default();
            println!("{}  {title}{range}{ev}", &h[..12.min(h.len())]);
        }
        return;
    }

    // list。
    if let Some(assets) = data.get("assets").and_then(|v| v.as_array()) {
        if assets.is_empty() {
            println!("(empty)");
            return;
        }
        for a in assets {
            let h = a.get("hash").and_then(|v| v.as_str()).unwrap_or("?");
            let kind = a.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
            let name = a
                .get("title")
                .and_then(|v| v.as_str())
                .or_else(|| a.get("original_name").and_then(|v| v.as_str()))
                .unwrap_or("");
            let dur = a
                .get("duration_ms")
                .and_then(|v| v.as_i64())
                .map(|ms| format!(" {:.1}s", ms as f64 / 1000.0))
                .unwrap_or_default();
            let tags = a
                .get("tags")
                .and_then(|v| v.as_array())
                .filter(|t| !t.is_empty())
                .map(|t| {
                    let s: Vec<&str> = t.iter().filter_map(|x| x.as_str()).collect();
                    format!("  [{}]", s.join(","))
                })
                .unwrap_or_default();
            let flag = if a.get("removed_at").and_then(|v| v.as_str()).is_some() {
                " (removed)"
            } else if a.get("stale").and_then(|v| v.as_bool()).unwrap_or(false) {
                " (stale)"
            } else {
                ""
            };
            println!(
                "{}  {kind:9} {name}{dur}{tags}{flag}",
                &h[..12.min(h.len())]
            );
        }
        return;
    }

    // Library summary.
    if data.get("by_kind").is_some() {
        println!(
            "assets {} (removed {} / stale {})  entities {}  annotations {}",
            data.get("assets").and_then(|v| v.as_i64()).unwrap_or(0),
            data.get("removed").and_then(|v| v.as_i64()).unwrap_or(0),
            data.get("stale").and_then(|v| v.as_i64()).unwrap_or(0),
            data.get("entities").and_then(|v| v.as_i64()).unwrap_or(0),
            data.get("annotations")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
        );
        if let Some(map) = data.get("by_kind").and_then(|v| v.as_object()) {
            let parts: Vec<String> = map.iter().map(|(k, n)| format!("{k} {n}")).collect();
            if !parts.is_empty() {
                println!("  distribution: {}", parts.join(" / "));
            }
        }
        if let Some(an) = data.get("analyzers").and_then(|v| v.as_array()) {
            for a in an {
                println!(
                    "  {}@... covers {} assets, cost {}ms / {} CNY cents",
                    a.get("analyzer").and_then(|v| v.as_str()).unwrap_or("?"),
                    a.get("assets").and_then(|v| v.as_i64()).unwrap_or(0),
                    a.get("cost_ms").and_then(|v| v.as_i64()).unwrap_or(0),
                    a.get("cost_fen").and_then(|v| v.as_i64()).unwrap_or(0),
                );
            }
        }
        return;
    }

    // verify。
    if let Some(issues) = data.get("issues").and_then(|v| v.as_array()) {
        println!(
            "checked {} assets, {} issues",
            data.get("checked").and_then(|v| v.as_i64()).unwrap_or(0),
            issues.len()
        );
        for i in issues {
            println!(
                "  [{}] {}",
                i.get("kind").and_then(|v| v.as_str()).unwrap_or("?"),
                i.get("detail").and_then(|v| v.as_str()).unwrap_or("")
            );
        }
        return;
    }

    // Print transcript sentences on separate lines and words continuously.
    if data.get("level").is_some() {
        if let Some(text) = data.get("text").and_then(|v| v.as_str()) {
            println!("{text}");
            return;
        }
    }

    // sql。
    if let Some(cols) = data.get("columns").and_then(|v| v.as_array()) {
        let names: Vec<&str> = cols.iter().filter_map(|c| c.as_str()).collect();
        println!("{}", names.join("\t"));
        if let Some(rows) = data.get("rows").and_then(|v| v.as_array()) {
            for r in rows.iter().filter_map(|r| r.as_array()) {
                let cells: Vec<String> = r.iter().map(cell_to_string).collect();
                println!("{}", cells.join("\t"));
            }
        }
        return;
    }

    // Use pretty JSON for other report shapes.
    println!(
        "{}",
        serde_json::to_string_pretty(&data).unwrap_or_default()
    );
}

fn cell_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "∅".to_owned(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

// Native probe adapters.

struct CliProber;

impl Prober for CliProber {
    fn probe(&self, path: &Path, kind: AssetKind) -> valle_project::assets::Result<ProbeOutcome> {
        match kind {
            AssetKind::Video | AssetKind::Audio | AssetKind::Image => probe_media(path, kind),
            AssetKind::Font => probe_font(path),
            AssetKind::Model3d => probe_model3d(path),
            AssetKind::Lottie => Ok(probe_lottie(path)),
            AssetKind::Component | AssetKind::Other => Ok(ProbeOutcome::default()),
        }
    }
}

fn probe_model3d(path: &Path) -> valle_project::assets::Result<ProbeOutcome> {
    let bytes = std::fs::read(path).map_err(|error| {
        AssetsError::unsupported_media(format!(
            "failed to read model3d ({}): {error}",
            path.display()
        ))
    })?;
    valle_motion::scene3d::admit_glb(&bytes).map_err(|error| {
        AssetsError::unsupported_media(format!(
            "model3d GLB admission failed ({}): {error}",
            path.display()
        ))
        .with_hint("only the supported Valle Scene3D glTF 2.0 GLB triangle subset is accepted")
    })?;
    Ok(ProbeOutcome::default())
}

/// Read duration, dimensions, and audio metadata through libav; reject registration if probing
/// fails.
fn probe_media(path: &Path, kind: AssetKind) -> valle_project::assets::Result<ProbeOutcome> {
    let av = valle_media::codec::probe_av(path).map_err(|e| {
        AssetsError::unsupported_media(format!("probe failed ({}): {e}", path.display())).with_hint(
            "the file may be corrupt; verify decoding or use --kind other to override detection",
        )
    })?;
    let mut p = Probe::default();
    match av.video {
        Some((w, h, dur)) => {
            p.width = Some(w);
            p.height = Some(h);
            p.duration_ms = Some((dur * 1000.0) as i64);
        }
        None => {
            if let Some(dur) = av.audio {
                p.duration_ms = Some((dur * 1000.0) as i64);
            } else if matches!(kind, AssetKind::Video | AssetKind::Audio) {
                return Err(AssetsError::unsupported_media(format!(
                    "no usable audio or video stream: {}",
                    path.display()
                )));
            }
        }
    }
    Ok(ProbeOutcome {
        probe: Some(p),
        warnings: vec![],
    })
}

/// Read font family, weight, style, and CJK coverage; reject malformed fonts.
fn probe_font(path: &Path) -> valle_project::assets::Result<ProbeOutcome> {
    let data =
        std::fs::read(path).map_err(|e| AssetsError::io(format!("failed to read font: {e}")))?;
    let face = ttf_parser::Face::parse(&data, 0).map_err(|e| {
        AssetsError::unsupported_media(format!("font parsing failed ({}): {e}", path.display()))
            .with_hint("the font may be corrupt; unpack WOFF2 first or use --kind other to override detection")
    })?;
    let family = face
        .names()
        .into_iter()
        .filter(|n| n.name_id == ttf_parser::name_id::FAMILY && n.is_unicode())
        .find_map(|n| n.to_string());
    let mut p = Probe::default();
    p.extra.insert("family".into(), serde_json::json!(family));
    p.extra.insert(
        "weight".into(),
        serde_json::json!(face.weight().to_number()),
    );
    p.extra
        .insert("italic".into(), serde_json::json!(face.is_italic()));
    p.extra
        .insert("glyphs".into(), serde_json::json!(face.number_of_glyphs()));
    // Use common Chinese characters as a coarse CJK coverage probe.
    let cjk = ['中', '文', '的', '一']
        .iter()
        .all(|c| face.glyph_index(*c).is_some());
    p.extra.insert("cjk".into(), serde_json::json!(cjk));
    Ok(ProbeOutcome {
        probe: Some(p),
        warnings: vec![],
    })
}

/// Read Lottie timing and dimensions from JSON on a best-effort basis.
fn probe_lottie(path: &Path) -> ProbeOutcome {
    let Ok(bytes) = std::fs::read(path) else {
        return ProbeOutcome::default();
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return ProbeOutcome::default();
    };
    let mut p = Probe::default();
    let fr = v.get("fr").and_then(|x| x.as_f64());
    let ip = v.get("ip").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let op = v.get("op").and_then(|x| x.as_f64());
    if let (Some(fr), Some(op)) = (fr, op) {
        if fr > 0.0 {
            p.fps = Some(fr);
            p.duration_ms = Some(((op - ip) / fr * 1000.0) as i64);
        }
    }
    p.width = v.get("w").and_then(|x| x.as_u64()).map(|x| x as u32);
    p.height = v.get("h").and_then(|x| x.as_u64()).map(|x| x as u32);
    ProbeOutcome {
        probe: Some(p),
        warnings: vec![],
    }
}
