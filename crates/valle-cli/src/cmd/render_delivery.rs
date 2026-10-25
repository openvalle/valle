//! Output selection and compiler observations shared by all public rendering commands.

use crate::{RenderOutputArgs, RenderVideoCodec};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use valle_compiler::motion::CompilationTrace;
use valle_engine::{fixed_package::OpenedFixedPackage, render::FrameKey};
use valle_media::codec::{Mp4VideoCodec, TransparentVideoCodec};
use valle_render::host::{NativeRenderer, RenderSummary};
use valle_timeline::{FrameRate, wire::timeline::TimelineTimeWire};

pub(super) fn apply_frame_rate(
    timeline: valle_timeline::Timeline,
    tuning: &crate::RenderTuningArgs,
) -> Result<valle_timeline::Timeline> {
    let Some(fps) = &tuning.fps else {
        return Ok(timeline);
    };
    let mut document: Value = serde_json::from_slice(&valle_timeline::timeline_bytes(&timeline)?)?;
    let fps = parse_output_fps(fps)?;
    document["canvas"]["fps"] = json!(format!("{}/{}", fps.numerator(), fps.denominator()));
    valle_timeline::decode_timeline(&document.to_string()).map_err(Into::into)
}

pub(super) fn parse_output_fps(input: &str) -> Result<FrameRate> {
    use valle_timeline::time::ExactRational;
    let exact = if input.contains('/') {
        ExactRational::parse_canonical(input)?
    } else {
        TimelineTimeWire::new(input)?.to_exact()
    };
    FrameRate::from_exact(exact).map_err(Into::into)
}

pub(super) fn validate_tuning(
    output: &RenderOutputArgs,
    tuning: &crate::RenderTuningArgs,
) -> Result<()> {
    if tuning.hardware_encode
        && matches!(
            output.codec,
            Some(RenderVideoCodec::Qtrle | RenderVideoCodec::Prores4444)
        )
    {
        bail!("--hardware-encode requires .mp4 output");
    }
    if tuning.bitrate.is_some()
        && !tuning.hardware_encode
        && output.codec != Some(RenderVideoCodec::Hevc)
    {
        bail!("--bitrate requires --hardware-encode or --codec hevc");
    }
    if tuning.encode_threads.is_some() && output.codec == Some(RenderVideoCodec::Hevc) {
        bail!("--encode-threads only applies to software H.264");
    }
    Ok(())
}

pub(super) fn validate_output_size(
    tuning: &crate::RenderTuningArgs,
    canvas: (u32, u32),
) -> Result<()> {
    if let Some((width, height)) = tuning.output_size {
        if width > i32::MAX as u32 || height > i32::MAX as u32 {
            bail!("output dimensions exceed Skia's i32 domain");
        }
        if u64::from(width) * u64::from(canvas.1) != u64::from(height) * u64::from(canvas.0) {
            bail!(
                "--output-size must scale the canvas {}x{} proportionally",
                canvas.0,
                canvas.1
            );
        }
    }
    Ok(())
}

pub(super) struct DeliveryObservation {
    pub trace: CompilationTrace,
    pub started: Instant,
    pub performance: Option<(valle_render::host::RenderTrace, PathBuf)>,
}

impl DeliveryObservation {
    pub fn new() -> Result<Self> {
        let performance = std::env::var_os("VALLE_RENDER_TRACE").map(|path| {
            (
                valle_render::host::RenderTrace::new(50_000),
                PathBuf::from(path),
            )
        });
        if let Some((_, path)) = &performance {
            super::delivery_report::require_new_output(path)?;
        }
        let render_trace = performance.as_ref().map(|(trace, _)| trace.clone());
        Ok(Self {
            performance,
            started: Instant::now(),
            trace: CompilationTrace::new(move |event| {
                if let Some(trace) = &render_trace {
                    trace.record(
                        "compile",
                        Instant::now()
                            .checked_sub(event.elapsed)
                            .unwrap_or_else(Instant::now),
                        json!({"entry": event.entry}),
                    );
                }
                if crate::output::events() {
                    crate::events::emit(crate::events::EventKind::MotionCompilation {
                        entry: event.entry.clone(),
                        elapsed_ms: event.elapsed.as_secs_f64() * 1000.0,
                    });
                }
            }),
        })
    }
}

pub(super) struct DeliveryPlan {
    args: RenderOutputArgs,
    pattern: Option<PngPattern>,
    pub codec: Option<RenderVideoCodec>,
}

impl DeliveryPlan {
    pub fn new(args: RenderOutputArgs) -> Result<Self> {
        let mut pattern = None;
        let mut codec = None;
        if let Some(sheet) = &args.storyboard {
            require_extension(sheet, "png")?;
            super::delivery_report::require_new_output(sheet)?;
        }
        if let Some(output) = &args.output {
            if args.frame.is_some() {
                require_extension(output, "png")?;
                if output
                    .file_name()
                    .is_some_and(|name| name.as_encoded_bytes().contains(&b'%'))
                {
                    bail!(
                        "--frame requires one PNG file; use --frames with a PNG sequence pattern"
                    );
                }
                super::delivery_report::require_new_output(output)?;
            } else if output
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
            {
                pattern = Some(PngPattern::parse(output)?);
                if args.codec.is_some() {
                    bail!("--codec only applies to video outputs");
                }
            } else {
                if !args.frames.is_empty() || args.storyboard.is_some() {
                    bail!(
                        "--frames and --storyboard require a PNG output pattern when -o is present"
                    );
                }
                let extension = output
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(str::to_ascii_lowercase);
                codec = Some(match extension.as_deref() {
                    Some("mp4") => args.codec.unwrap_or(RenderVideoCodec::H264),
                    Some("mov") => args.codec.unwrap_or(RenderVideoCodec::Qtrle),
                    _ => bail!(
                        "output must be .mp4, .mov, a .png with --frame, or a PNG sequence pattern such as frames/%05d.png"
                    ),
                });
                if !matches!(
                    (extension.as_deref(), codec),
                    (
                        Some("mp4"),
                        Some(RenderVideoCodec::H264 | RenderVideoCodec::Hevc)
                    ) | (
                        Some("mov"),
                        Some(RenderVideoCodec::Qtrle | RenderVideoCodec::Prores4444)
                    )
                ) {
                    bail!(
                        "--codec must match the output extension: h264 and hevc use .mp4; qtrle and prores4444 use .mov"
                    );
                }
                super::delivery_report::require_new_output(output)?;
            }
        } else if args.storyboard.is_none() {
            bail!("provide -o or --storyboard");
        }
        if args.frame.is_some_and(|frame| frame < 0) {
            bail!("frame keys must be nonnegative");
        }
        if args.frames.iter().any(|frame| *frame < 0) {
            bail!("frame keys must be nonnegative");
        }
        Ok(Self {
            args,
            pattern,
            codec,
        })
    }

    pub fn output(&self) -> &Path {
        self.args
            .storyboard
            .as_deref()
            .or(self.args.output.as_deref())
            .expect("validated delivery")
    }

    pub fn validate_encoder(
        &self,
        tuning: &crate::RenderTuningArgs,
        transparent: bool,
    ) -> Result<()> {
        let mp4 = matches!(
            self.codec,
            Some(RenderVideoCodec::H264 | RenderVideoCodec::Hevc)
        );
        if !mp4
            && (tuning.hardware_encode
                || tuning.bitrate.is_some()
                || tuning.encode_threads.is_some())
        {
            bail!("encoder options require .mp4 output");
        }
        if mp4 && transparent {
            bail!(
                "H.264/HEVC cannot preserve a transparent canvas; use .mov with qtrle or prores4444, or set an opaque background"
            );
        }
        Ok(())
    }

    pub fn operation(&self) -> &'static str {
        if self.args.storyboard.is_some() {
            "storyboard"
        } else if self.args.frame.is_some() {
            "preview"
        } else {
            "export"
        }
    }

    fn frame_keys(&self, renderer: &NativeRenderer) -> Vec<FrameKey> {
        let total = renderer.project().compiled().canvas().frame_count();
        if let Some(frame) = self.args.frame {
            return vec![FrameKey::new(frame)];
        }
        if !self.args.frames.is_empty() {
            return self
                .args
                .frames
                .iter()
                .copied()
                .map(FrameKey::new)
                .collect();
        }
        if self.args.storyboard.is_some() {
            let cells = total.min(12);
            return (0..cells)
                .map(|i| {
                    FrameKey::new(if cells == 1 {
                        0
                    } else {
                        ((i128::from(i) * i128::from(total - 1)) / i128::from(cells - 1)) as i64
                    })
                })
                .collect();
        }
        (0..total).map(FrameKey::new).collect()
    }

    pub fn execute(&self, renderer: &NativeRenderer) -> Result<(RenderSummary, Value)> {
        if let Some(codec) = self.codec {
            let summary = match codec {
                RenderVideoCodec::H264 => renderer.export_mp4(self.output())?,
                RenderVideoCodec::Hevc => {
                    renderer.export_mp4_with_codec(self.output(), Mp4VideoCodec::Hevc)?
                }
                RenderVideoCodec::Qtrle => renderer.export_transparent_mov(self.output())?,
                RenderVideoCodec::Prores4444 => renderer.export_transparent_mov_with_codec(
                    self.output(),
                    TransparentVideoCodec::ProRes4444,
                )?,
            };
            let encoded_codec = if summary.audio_only {
                "aac"
            } else {
                match codec {
                    RenderVideoCodec::H264 => "h264",
                    RenderVideoCodec::Hevc => "hevc",
                    RenderVideoCodec::Qtrle => "qtrle",
                    RenderVideoCodec::Prores4444 => "prores4444",
                }
            };
            return Ok((summary, json!({"codec": encoded_codec})));
        }
        let frames = self.frame_keys(renderer);
        let outputs = if let Some(pattern) = &self.pattern {
            frames
                .iter()
                .map(|frame| pattern.at(frame.index()))
                .collect()
        } else if self.args.frame.is_some() {
            vec![self.output().to_owned()]
        } else {
            Vec::new()
        };
        let columns = u32::try_from(frames.len().min(4)).unwrap();
        let summary = renderer.render_png_frames(
            &frames,
            &outputs,
            self.args
                .storyboard
                .as_deref()
                .map(|sheet| (sheet, columns)),
        )?;
        let details = json!({
            "frameKeys": frames.iter().map(|frame| frame.index()).collect::<Vec<_>>(),
            "outputs": outputs, "storyboard": self.args.storyboard,
        });
        Ok((summary, details))
    }

    pub fn deliver_and_report(
        &self,
        renderer: &NativeRenderer,
        opened: &OpenedFixedPackage,
        observation: &DeliveryObservation,
        timing: Option<Value>,
    ) -> Result<()> {
        let started = Instant::now();
        let prepared = observation.started.elapsed();
        if let Some((trace, _)) = &observation.performance {
            trace.record(
                "package.prepare_total",
                observation.started,
                json!({"includesCompilation": true}),
            );
        }
        let (summary, details) = self.execute(renderer)?;
        let mut report = super::delivery_report::delivery_report(
            opened,
            self.operation(),
            self.output(),
            &summary,
            timing,
        )?;
        let metrics = observation.trace.metrics();
        report["compilations"] = json!(metrics.compilations);
        let delivery = report["delivery"].as_object_mut().expect("delivery report");
        delivery.extend(details.as_object().expect("delivery details").clone());
        let timing = delivery.entry("timing").or_insert_with(|| json!({}));
        timing["compileMs"] = json!(metrics.elapsed.as_secs_f64() * 1000.0);
        timing["prepareMs"] =
            json!(prepared.saturating_sub(metrics.elapsed).as_secs_f64() * 1000.0);
        timing["renderMs"] = json!(started.elapsed().as_secs_f64() * 1000.0);
        if let Some((trace, path)) = &observation.performance {
            trace.record("export.total", observation.started, json!({}));
            trace.write(path, json!({"backend": report["delivery"]["backend"], "status": "ok", "report": report}))
                .with_context(|| format!("writing render trace {}", path.display()))?;
        }
        crate::output::emit(report);
        Ok(())
    }
}

fn require_extension(output: &Path, extension: &str) -> Result<()> {
    if !output
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
    {
        bail!("output must have an .{extension} extension");
    }
    Ok(())
}

/// A deliberately narrow filename pattern, not a printf format string or a shell expression.
struct PngPattern {
    parent: PathBuf,
    prefix: String,
    suffix: String,
    width: usize,
}

impl PngPattern {
    fn parse(output: &Path) -> Result<Self> {
        let name = output
            .file_name()
            .and_then(|name| name.to_str())
            .context("PNG pattern needs a UTF-8 file name")?;
        let invalid = || {
            anyhow::anyhow!(
                "PNG sequence output requires exactly one %d or %0Nd field (1 <= N <= 20) in the filename, such as frames/%05d.png"
            )
        };
        let (prefix, field) = name.split_once('%').ok_or_else(invalid)?;
        let (padding, suffix) = field.split_once('d').ok_or_else(invalid)?;
        if suffix.contains('%') {
            return Err(invalid());
        }
        let width = if padding.is_empty() {
            0
        } else {
            let digits = padding
                .strip_prefix('0')
                .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
                .ok_or_else(invalid)?;
            let width = digits.parse::<usize>().map_err(|_| invalid())?;
            if !(1..=20).contains(&width) {
                return Err(invalid());
            }
            width
        };
        Ok(Self {
            parent: output.parent().unwrap_or(Path::new(".")).to_owned(),
            prefix: prefix.into(),
            suffix: suffix.into(),
            width,
        })
    }

    fn at(&self, frame: i64) -> PathBuf {
        self.parent.join(format!(
            "{}{:0width$}{}",
            self.prefix,
            frame,
            self.suffix,
            width = self.width
        ))
    }
}

pub(super) fn frame_costs(
    renderer: &valle_render::host::NativeRenderer,
    fps: FrameRate,
    (width, height): (u32, u32),
) -> Result<(serde_json::Value, usize)> {
    use std::cmp::Reverse;
    let seconds = |frame: valle_engine::render::FrameKey| {
        frame.index() as f64 * f64::from(fps.denominator()) / fps.numerator() as f64
    };
    let (mut frames, mut planned, mut failures) = (Vec::new(), Vec::new(), Vec::new());
    for (frame, cost) in renderer.plan_costs()? {
        match cost {
            Ok(cost) => {
                frames.push(
                    serde_json::json!({"frame":frame.index(), "pixels":cost.pixels,
                    "peakBytes":cost.peak_surface_bytes}),
                );
                planned.push((frame, cost));
            }
            Err(error) => {
                let mut failure = serde_json::json!({"frame":frame.index(),
                    "seconds":seconds(frame), "error":error.to_string()});
                if let Some(path) = error.path() {
                    failure["path"] = path.into();
                }
                failures.push(failure);
            }
        }
    }
    planned.sort_by_key(|(frame, cost)| (Reverse(cost.pixels), *frame));
    let hot = planned
        .iter_mut()
        .take(5)
        .map(|(frame, cost)| {
            let mut kinds = std::collections::BTreeMap::<&str, (usize, u64)>::new();
            for pass in &cost.passes {
                let kind = kinds.entry(pass.kind).or_default();
                kind.0 += 1;
                kind.1 += pass.device_roi.pixels();
            }
            let mut kinds = kinds.into_iter().collect::<Vec<_>>();
            kinds.sort_by_key(|(_, (_, pixels))| Reverse(*pixels));
            let kinds = kinds.into_iter().map(|(kind, (passes, pixels))| {
                serde_json::json!({"kind":kind, "passes":passes, "pixels":pixels})
            });
            cost.passes
                .sort_by_key(|pass| Reverse(pass.device_roi.pixels()));
            let passes = cost.passes.iter().take(5).map(|pass| {
                let roi = pass.device_roi;
                serde_json::json!({"path":pass.path, "kind":pass.kind,
                    "roi":[roi.x, roi.y, roi.width, roi.height], "pixels":roi.pixels()})
            });
            serde_json::json!({"frame":frame.index(), "seconds":seconds(*frame),
                "pixels":cost.pixels, "peakBytes":cost.peak_surface_bytes,
                "shaderOperations":cost.shader_work.operations,
                "kinds":kinds.collect::<Vec<_>>(), "passes":passes.collect::<Vec<_>>()})
        })
        .collect::<Vec<_>>();
    let count = failures.len();
    Ok((
        serde_json::json!({"width":width, "height":height, "frames":frames, "hotFrames":hot,
            "failures":failures}),
        count,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_patterns_have_one_bounded_integer_field() {
        for invalid in [
            "frame.png",
            "%s.png",
            "%5d.png",
            "%00d.png",
            "%021d.png",
            "%d-%d.png",
            "%0-2d.png",
        ] {
            assert!(PngPattern::parse(Path::new(invalid)).is_err(), "{invalid}");
        }
        assert_eq!(
            PngPattern::parse(Path::new("frames/key-%05d.png"))
                .unwrap()
                .at(30),
            Path::new("frames/key-00030.png")
        );
        assert_eq!(
            PngPattern::parse(Path::new("%d.png")).unwrap().at(123),
            Path::new("123.png")
        );
    }
}
