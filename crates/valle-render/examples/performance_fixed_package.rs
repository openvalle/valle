//! Benchmark adapter for admitted packages. It uses the production renderer unchanged.
use std::{path::Path, sync::Arc, time::Instant};

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use valle_engine::{
    fixed_package::{
        COMMON_PROFILE_KEY, canonical_fixed_execution_profile, fixed_package_files,
        open_verified_fixed_package,
    },
    frame::{RenderQuality, RenderSpec},
    render::FrameKey,
    resource::{ContentDigest, OutputBackground, OutputSpec},
};
use valle_media::codec::Mp4VideoCodec;
use valle_render::{
    executor::skia::SkiaBackendKind,
    host::{
        NativeProject, NativeRenderOptions, NativeRenderer, NativeResourceCatalog, RenderControl,
        RenderTrace,
    },
};

fn main() -> Result<()> {
    let started = Instant::now();
    let file = std::env::args().nth(1).context("expected job.json")?;
    let job: Value = serde_json::from_slice(&std::fs::read(file)?)?;
    let string =
        |key: &str| -> Result<&str> { job[key].as_str().with_context(|| format!("missing {key}")) };
    let number =
        |key: &str| -> Result<u64> { job[key].as_u64().with_context(|| format!("missing {key}")) };
    // The product host's config omits this reconstructible common member. The normal package
    // verifier below still checks its exact bytes against the manifest; another policy fails.
    let profile = job["executionProfileJson"]
        .as_str()
        .map(str::to_owned)
        .map(Ok)
        .unwrap_or_else(|| canonical_fixed_execution_profile(COMMON_PROFILE_KEY))
        .map_err(anyhow::Error::msg)?;
    let resource_bytes = job["resourceBytesPath"]
        .as_str()
        .map(std::fs::read)
        .transpose()?
        .unwrap_or_default();
    let files = fixed_package_files(
        string("timelineJson")?,
        string("resourceManifestJson")?,
        string("verifiedBindingBundleJson")?,
        &profile,
        &resource_bytes,
    );
    let package = open_verified_fixed_package(string("fixedPackageManifestJson")?, &files)?;
    let mut catalog = NativeResourceCatalog::new();
    // Frozen fonts travel in the verified binding bundle, not in the asset locator list.
    let render = package.engine_render();
    for resource in render.compiled().execution_resources() {
        if resource.kind() == valle_engine::render::CompiledExecutionResourceKind::FontBytes {
            catalog.insert_bytes(*resource.content_digest(), resource.bytes());
        }
    }
    for asset in job["localAssets"]
        .as_array()
        .context("missing localAssets")?
    {
        let file = asset.as_str().context("invalid asset path")?;
        catalog.insert_file(ContentDigest::of_bytes(&std::fs::read(file)?), file);
    }
    let backend = match string("backend")? {
        "native-cpu" => SkiaBackendKind::Raster,
        #[cfg(target_os = "macos")]
        "native-metal" => SkiaBackendKind::Metal,
        value => return Err(anyhow!("invalid backend {value}")),
    };
    let trace = std::env::var_os("VALLE_RENDER_TRACE").map(|file| (RenderTrace::new(50_000), file));
    if let Some((trace, _)) = &trace {
        trace.record("package.open", started, json!({}));
    }
    let control = trace
        .as_ref()
        .map_or_else(RenderControl::default, |(trace, _)| {
            RenderControl::default().with_trace(trace.clone())
        });
    let qa = string("mode")? == "qa";
    let budget = valle_render::host::NativeRenderBudget::new(
        usize::try_from(number("threads")?)?,
        job["memoryBytes"].as_u64(),
    )?;
    let project = NativeProject::from_render(render, Arc::new(catalog));
    let renderer = NativeRenderer::new(
        project.clone(),
        NativeRenderOptions {
            backend,
            output_size: Some((
                u32::try_from(number("width")?)?,
                u32::try_from(number("height")?)?,
            )),
            budget: budget.clone(),
            hardware_encode: true,
            bitrate: Some(usize::try_from(number("bitrate")?)?),
            background: if qa {
                OutputBackground::Transparent
            } else {
                OutputBackground::opaque_srgb([0, 0, 0])
            },
            control,
            ..NativeRenderOptions::default()
        },
    );
    let output = Path::new(string("output")?);
    let summary = if qa {
        let frames = job["frames"]
            .as_array()
            .context("missing frames")?
            .iter()
            .map(|f| f.as_i64().map(FrameKey::new).context("invalid frame"))
            .collect::<Result<Vec<_>>>()?;
        renderer.render_png_frames(
            &frames,
            &[],
            Some((output, u32::try_from(frames.len().min(4))?)),
        )?
    } else {
        let codec = match job["codec"].as_str().unwrap_or("h264") {
            "h264" => Mp4VideoCodec::H264,
            "hevc" => Mp4VideoCodec::Hevc,
            value => return Err(anyhow!("invalid codec {value}")),
        };
        renderer.export_mp4_with_codec(output, codec)?
    };
    let mut working_captures = Vec::new();
    if qa && job["workingF16"].as_bool().unwrap_or(false) {
        let directory = output.parent().context("capture output directory")?;
        let capture_budget =
            valle_engine::motion::memory::MemoryBudget::new(job["memoryBytes"].as_u64());
        let cpu = valle_engine::motion::cpu::CpuBudget::new_with_memory(
            usize::try_from(number("threads")?)?,
            Some(capture_budget.clone()),
        )
        .map_err(anyhow::Error::msg)?;
        let mut runner = project.frame_runner(backend)?;
        let spec = RenderSpec::new(
            u32::try_from(number("width")?)?,
            u32::try_from(number("height")?)?,
            RenderQuality::Final,
            OutputSpec::srgb_delivery(OutputBackground::Transparent)?,
        )?;
        for (sequence, frame) in job["frames"]
            .as_array()
            .context("missing frames")?
            .iter()
            .enumerate()
        {
            let frame = frame.as_i64().context("invalid frame")?;
            let capture = cpu.run(|| -> Result<_> {
                runner.capture_next_working_frame();
                runner.render_rgba8(FrameKey::new(frame), spec)?;
                runner
                    .take_working_capture()
                    .context("missing working F16 capture")
            })?;
            let file = format!("working-{sequence}-{frame}.f16");
            std::fs::write(directory.join(&file), &capture.rgba16f_le)?;
            working_captures.push(json!({"file":file,"frame":frame,"sequence":sequence,
                "width":capture.width,"height":capture.height,"bytes":capture.rgba16f_le.len(),
                "format":"linear-rec2020-premul-rgba16f-le"}));
        }
    }
    let memory = budget.memory_report();
    let result = json!({"delivery": {"backend": if string("backend")? == "native-metal" {"metal"} else {"raster"}, "budget": {
        "cpuThreads": budget.threads(), "memoryBudgetBytes": job["memoryBytes"].as_u64(),
        "managedPeakBytes": memory.peak_bytes, "managedPeakLiveBytes": memory.peak_live_bytes,
        "managedRemainingBytes": memory.used_bytes,
    }}, "frames": summary.frames, "width": summary.width, "height": summary.height, "workingCaptures":working_captures});
    if let Some((trace, file)) = trace {
        trace.record("export.total", started, json!({}));
        trace.write(
            Path::new(&file),
            json!({"status": "complete", "result": result}),
        )?;
    }
    println!("{result}");
    Ok(())
}
