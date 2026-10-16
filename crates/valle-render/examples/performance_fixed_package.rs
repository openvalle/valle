//! Benchmark adapter for admitted packages. It uses the production renderer unchanged.
use std::{path::Path, sync::Arc, time::Instant};

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use valle_engine::{
    fixed_package::{fixed_package_files, open_verified_fixed_package},
    render::FrameKey,
    resource::{ContentDigest, OutputBackground},
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
    let files = fixed_package_files(
        string("timelineJson")?,
        string("resourceManifestJson")?,
        string("verifiedBindingBundleJson")?,
        string("executionProfileJson")?,
    );
    let package = open_verified_fixed_package(string("fixedPackageManifestJson")?, &files)?;
    let mut catalog = NativeResourceCatalog::new();
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
    let renderer = NativeRenderer::new(
        NativeProject::from_render(package.engine_render(), Arc::new(catalog)),
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
    let memory = budget.memory_report();
    let result = json!({"delivery": {"backend": if string("backend")? == "native-metal" {"metal"} else {"raster"}, "budget": {
        "cpuThreads": budget.threads(), "memoryBudgetBytes": job["memoryBytes"].as_u64(),
        "managedPeakBytes": memory.peak_bytes, "managedPeakLiveBytes": memory.peak_live_bytes,
        "managedRemainingBytes": memory.used_bytes,
    }}, "frames": summary.frames, "width": summary.width, "height": summary.height});
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
