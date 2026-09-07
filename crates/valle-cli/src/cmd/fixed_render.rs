//! Native delivery for one complete, immutable Timeline fixed package.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Value, json};
use valle_engine::{
    fixed_package::{
        OpenedFixedPackage, VerifiedFixedPackage, fixed_package_files, verify_fixed_package,
    },
    render::FrameKey,
    resource::ContentDigest,
};
use valle_project::assets::{ActiveResourceLease, Home, PublishedPackagePin, ResourceRootStore};
use valle_render::{
    AssetCache, default_cache_dir,
    executor::skia::SkiaBackendKind,
    host::{
        NativeProject, NativeRenderOptions, NativeRenderer, NativeResourceCatalog, RenderSummary,
    },
};
use valle_timeline::internal::{
    ResourceManifest, decode_resource_manifest, wire::resource::ResourceEntryWire,
};

use crate::{FixedRenderAction, FixedRenderPackageArgs};

pub fn run(
    package: &FixedRenderPackageArgs,
    action: FixedRenderAction,
) -> Result<std::process::ExitCode> {
    let package_manifest_json = super::read(&package.package_manifest)?;
    let timeline_json = super::read(&package.canonical_timeline)?;
    let manifest_json = super::read(&package.resource_manifest)?;
    let bindings_json = super::read(&package.verified_binding_bundle)?;
    let execution_profile_json = super::read(&package.execution_profile)?;
    let files = fixed_package_files(
        &timeline_json,
        &manifest_json,
        &bindings_json,
        &execution_profile_json,
    );
    let verified =
        verify_fixed_package(&package_manifest_json, &files).map_err(|error| anyhow!(error))?;
    let opened = verified.open()?;

    if matches!(&action, FixedRenderAction::Open) {
        println!("{}", opened.receipt_json());
        return Ok(std::process::ExitCode::SUCCESS);
    }

    let home = Home::resolve()?;
    let store = ResourceRootStore::at(home.root());
    if matches!(&action, FixedRenderAction::Unpin) {
        store.remove_package_pin(*verified.package_digest())?;
        println!(
            "{}",
            json!({"status": "ok", "operation": "unpin", "packageDigest": verified.package_digest()})
        );
        return Ok(std::process::ExitCode::SUCCESS);
    }
    let manifest = decode_resource_manifest(manifest_json.as_bytes())?;
    let (catalog, lease) = load_resource_catalog(&manifest, &package.resources, &home, &store)?;
    // The lease is owned by this job through success, error and unwinding. On a
    // killed process the OS releases its lock and the next GC scan reclaims it.
    if matches!(&action, FixedRenderAction::Pin) {
        let pin = pin_verified_package(&store, &verified)?;
        lease.release()?;
        println!(
            "{}",
            json!({"status": "ok", "operation": "pin", "packageDigest": pin.package_digest()})
        );
        return Ok(std::process::ExitCode::SUCCESS);
    }
    let catalog = Arc::new(catalog);
    let project = NativeProject::from_render(opened.engine_render(), catalog);
    let renderer = NativeRenderer::new(
        project,
        NativeRenderOptions {
            backend: SkiaBackendKind::Raster,
            ..NativeRenderOptions::default()
        },
    );
    let (operation, output, summary) = match action {
        FixedRenderAction::Open | FixedRenderAction::Pin | FixedRenderAction::Unpin => {
            unreachable!("non-render actions returned before Native delivery")
        }
        FixedRenderAction::Preview { frame, output } => {
            require_new_output(&output)?;
            let summary = renderer.preview_frame_key(FrameKey::new(frame), &output)?;
            ("preview", output, summary)
        }
        FixedRenderAction::Export { output } => {
            require_new_output(&output)?;
            let summary = renderer.export_mp4(&output)?;
            ("export", output, summary)
        }
    };
    lease.release()?;
    print_delivery_report(&opened, operation, &output, &summary)?;
    Ok(std::process::ExitCode::SUCCESS)
}

fn load_resource_catalog(
    manifest: &ResourceManifest,
    specs: &[String],
    home: &Home,
    store: &ResourceRootStore,
) -> Result<(NativeResourceCatalog, ActiveResourceLease)> {
    let mut catalog = NativeResourceCatalog::new();
    let mut resources = Vec::<(ContentDigest, PathBuf)>::new();
    let mut bound_ids = BTreeSet::new();
    let mut remote_cache = None;
    for spec in specs {
        let (resource_id, locator) = spec
            .split_once('=')
            .filter(|(resource_id, locator)| !resource_id.is_empty() && !locator.is_empty())
            .ok_or_else(|| {
                anyhow!("invalid --resource `{spec}`; expected RESOURCE_ID=PATH_OR_URL")
            })?;
        if !bound_ids.insert(resource_id) {
            bail!("duplicate --resource binding for `{resource_id}`");
        }
        let entry = manifest.entries().get(resource_id).ok_or_else(|| {
            anyhow!("resource `{resource_id}` is absent from the pinned manifest")
        })?;
        let digest = *entry_digest(entry);
        let local_path = if is_http_locator(locator) {
            if remote_cache.is_none() {
                remote_cache = Some(AssetCache::new(default_cache_dir())?);
            }
            remote_cache
                .as_ref()
                .expect("remote cache was initialized")
                .fetch(locator, "bin")
                .with_context(|| format!("fetching `{resource_id}` from {locator}"))?
        } else {
            locator.into()
        };
        resources.push((digest, local_path));
    }
    // A pinned package can render offline after its original locators disappear.
    // Embedded-only facts need no file; missing external bytes still fail in fulfillment.
    let supplied = resources
        .iter()
        .map(|(digest, _)| *digest)
        .collect::<BTreeSet<_>>();
    for entry in manifest.entries().values() {
        let digest = *entry_digest(entry);
        let cached = home.object_path(&digest.as_hex(), None);
        if !supplied.contains(&digest) && cached.exists() {
            resources.push((digest, cached));
        }
    }
    let lease = store.import_and_lease(resources)?;
    for digest in lease.resource_digests() {
        catalog.insert_file(*digest, home.object_path(&digest.as_hex(), None))?;
    }
    Ok((catalog, lease))
}

/// The host is the only layer depending on both Engine and Project. Only an
/// Engine-verified package may reach the low-level persistent root writer.
fn pin_verified_package(
    store: &ResourceRootStore,
    package: &VerifiedFixedPackage,
) -> Result<PublishedPackagePin> {
    Ok(store
        .publish_package_pin_unchecked(std::str::from_utf8(package.canonical_manifest_bytes())?)?)
}

fn is_http_locator(locator: &str) -> bool {
    locator.starts_with("http://") || locator.starts_with("https://")
}

fn entry_digest(entry: &ResourceEntryWire) -> &ContentDigest {
    match entry {
        ResourceEntryWire::Video { digest, .. }
        | ResourceEntryWire::Audio { digest, .. }
        | ResourceEntryWire::Image { digest, .. }
        | ResourceEntryWire::Lottie { digest, .. }
        | ResourceEntryWire::Font { digest, .. }
        | ResourceEntryWire::MotionArtifact { digest, .. }
        | ResourceEntryWire::Shader { digest, .. } => digest,
    }
}

pub(super) fn require_new_output(output: &Path) -> Result<()> {
    if output.exists() {
        bail!("refusing to overwrite existing output {}", output.display());
    }
    Ok(())
}

pub(super) fn print_delivery_report(
    opened: &OpenedFixedPackage,
    operation: &str,
    output: &Path,
    summary: &RenderSummary,
) -> Result<()> {
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
    crate::output::emit(Value::Object(report));
    Ok(())
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

    #[test]
    fn only_explicit_http_locators_use_the_download_cache() {
        assert!(is_http_locator("http://example.test/a.mp4"));
        assert!(is_http_locator("https://example.test/a.mp4"));
        assert!(!is_http_locator("file:///tmp/a.mp4"));
        assert!(!is_http_locator("./https://local-file"));
    }
}
