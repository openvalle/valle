//! Shared delivery report and output checks for Timeline and project rendering.
use anyhow::{Result, bail};
use serde_json::{Map, Value, json};
use std::path::Path;
use valle_engine::fixed_package::OpenedFixedPackage;
use valle_render::{executor::skia::SkiaBackendKind, host::RenderSummary};

pub(super) fn require_new_output(output: &Path) -> Result<()> {
    if output.exists() {
        bail!("refusing to overwrite existing output {}", output.display());
    }
    Ok(())
}

pub(super) fn delivery_report(
    opened: &OpenedFixedPackage,
    operation: &str,
    output: &Path,
    summary: &RenderSummary,
    timing: Option<Value>,
) -> Result<Value> {
    if summary.render_id != opened.receipt().render_id() {
        bail!("Native delivery returned a different RenderId");
    }
    let receipt: Value = serde_json::from_str(opened.receipt_json())?;
    let mut report = receipt.as_object().cloned().unwrap_or_else(Map::new);
    report.insert("status".to_owned(), Value::String("ok".to_owned()));
    report.insert("operation".to_owned(), Value::String(operation.to_owned()));
    report.insert(
        "output".to_owned(),
        Value::String(output.display().to_string()),
    );
    report.insert(
        "delivery".to_owned(),
        json!({
            "renderId": summary.render_id.to_string(),
            "frames": summary.frames,
            "width": summary.width,
            "height": summary.height,
            "audioOnly": summary.audio_only,
        }),
    );
    if let Some(pipeline) = &summary.pipeline
        && let Some(budget) = pipeline.budget
    {
        report.get_mut("delivery").expect("delivery report")["budget"] = json!({
            "cpuThreads":budget.cpu_threads,"peakGrantedCpuSlots":budget.peak_cpu_slots,
            "memoryBudgetBytes":budget.memory_budget_bytes,"resourceProviders":budget.resource_providers,"executors":budget.executors,
            "configuredCapacityBytes":budget.configured_capacity_bytes,
            "managedPeakBytes":budget.memory.peak_bytes,"managedPeakLiveBytes":budget.memory.peak_live_bytes,"managedRemainingBytes":budget.memory.used_bytes,
            "managedPeakByKindBytes":budget.memory.peak_by_kind,
            "memoryScope":"shared frame reservations, resource/surface storage, conservative CPU filter/3D scratch and decoded media windows; excludes immutable inputs, bounded plan/topology metadata and driver/codec internals; not process RSS",
            "frameParallelism":pipeline.preparation_tasks,"maximumInFlightFrames":pipeline.maximum_in_flight_frames,
        });
    }
    if let Some(timing) = timing {
        report.get_mut("delivery").expect("delivery report")["timing"] = timing;
    }
    if let Some(profile) = summary
        .pipeline
        .as_ref()
        .and_then(|pipeline| pipeline.profile)
    {
        report.get_mut("delivery").expect("delivery report")["backend"] =
            json!(match profile.surface_backend {
                SkiaBackendKind::Raster => "raster",
                #[cfg(target_os = "macos")]
                SkiaBackendKind::Metal => "metal",
            });
    }
    Ok(Value::Object(report))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_overwrite_before_native_delivery() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("frame.png");
        std::fs::write(&output, b"keep").unwrap();
        let error = require_new_output(&output).unwrap_err().to_string();
        assert!(error.contains("refusing to overwrite"));
        assert_eq!(std::fs::read(output).unwrap(), b"keep");
    }
}
