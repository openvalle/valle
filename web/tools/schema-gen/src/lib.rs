use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::{Config, TS};
use valle_timeline::internal::ContentDigest;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct MotionFrameRateWire {
    pub num: u32,
    pub den: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(type = "string")]
pub struct CodeWire(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[ts(tag = "kind", rename_all = "kebab-case")]
pub enum StudioSessionWire {
    Project {
        #[serde(rename = "projectId")]
        #[ts(rename = "projectId")]
        project_id: String,
        revision: u64,
        token: String,
    },
    TimelineFile {
        input: String,
    },
    MotionFile {
        input: String,
        generation: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
pub struct StudioCapabilitiesWire {
    pub save_timeline: bool,
    pub edit_project: bool,
    pub edit_motion_props: bool,
    pub write_motion_source: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
pub struct StudioRuntimeWire {
    pub asset_base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub proxy_base: Option<String>,
    pub asset_urls: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
pub struct StudioBootWire {
    pub protocol_version: u32,
    pub session: StudioSessionWire,
    pub capabilities: StudioCapabilitiesWire,
    pub runtime: StudioRuntimeWire,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
pub struct MotionTimingWire {
    pub enter_frames: u32,
    pub exit_frames: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[ts(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MotionCueWindowWire {
    SourceRange {
        start_frame: u32,
        end_frame: u32,
        enter_frames: u32,
        exit_frames: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
pub struct MotionViewportWire {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
pub struct MotionSourceSpanWire {
    pub start: u32,
    pub end: u32,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
pub struct MotionDiagnosticWire {
    pub class: String,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub span: Option<MotionSourceSpanWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
pub struct MotionModuleSourceInfoWire {
    pub path: String,
    #[ts(type = "ContentDigest")]
    pub source_digest: ContentDigest,
    #[ts(type = "ContentDigest")]
    pub normalized_ast_digest: ContentDigest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
pub struct MotionSourceMapWire {
    pub version: u32,
    pub component: String,
    pub entry: String,
    #[ts(type = "ContentDigest")]
    pub closure_digest: ContentDigest,
    pub modules: Vec<MotionModuleSourceInfoWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub controls_source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "Record<string, unknown>")]
    pub controls: Option<Value>,
    #[ts(type = "Array<Record<string, unknown>>")]
    pub nodes: Vec<Value>,
    #[ts(type = "Array<Record<string, unknown>>")]
    pub exprs: Vec<Value>,
    #[ts(type = "Array<Record<string, unknown>>")]
    pub objects: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
pub struct MotionAssetWire {
    pub name: String,
    pub kind: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
pub struct MotionResourceLocatorWire {
    pub id: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
pub struct MotionShaderWire {
    pub uri: String,
    pub manifest_bytes: Vec<u8>,
    pub source_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "lowercase", deny_unknown_fields)]
#[ts(tag = "status", rename_all = "lowercase")]
pub enum MotionContextWire {
    Ok {
        #[serde(rename = "protocolVersion")]
        #[ts(rename = "protocolVersion")]
        protocol_version: u32,
        generation: u64,
        input: String,
        #[serde(rename = "artifactDigest")]
        #[ts(rename = "artifactDigest", type = "ContentDigest")]
        artifact_digest: ContentDigest,
        #[ts(type = "Record<string, unknown>")]
        artifact: Value,
        #[serde(rename = "preparedData")]
        #[ts(rename = "preparedData", type = "Record<string, unknown>")]
        prepared_data: Value,
        #[serde(rename = "dataSource")]
        #[ts(rename = "dataSource")]
        data_source: Option<String>,
        timing: MotionTimingWire,
        #[serde(rename = "cueBindings")]
        #[ts(rename = "cueBindings")]
        cue_bindings: BTreeMap<String, MotionCueWindowWire>,
        #[serde(rename = "sourceMap")]
        #[ts(rename = "sourceMap")]
        source_map: MotionSourceMapWire,
        assets: Vec<MotionAssetWire>,
        #[serde(rename = "resourceLocators")]
        #[ts(rename = "resourceLocators")]
        resource_locators: Vec<MotionResourceLocatorWire>,
        shaders: Vec<MotionShaderWire>,
        #[serde(rename = "durationFrames")]
        #[ts(rename = "durationFrames")]
        duration_frames: u32,
        fps: MotionFrameRateWire,
        viewport: MotionViewportWire,
        diagnostics: Vec<MotionDiagnosticWire>,
        #[serde(rename = "runtimeBaseUrl")]
        #[ts(rename = "runtimeBaseUrl")]
        runtime_base_url: String,
        #[serde(rename = "runtimeAssets")]
        #[ts(rename = "runtimeAssets", type = "Record<string, unknown>")]
        runtime_assets: Value,
        #[serde(rename = "fixedPackageManifestJson")]
        #[ts(rename = "fixedPackageManifestJson")]
        fixed_package_manifest_json: String,
        #[serde(rename = "timelineJson")]
        #[ts(rename = "timelineJson")]
        timeline_json: String,
        #[ts(type = "TimelineDocument")]
        timeline: Value,
        #[serde(rename = "resourceManifestJson")]
        #[ts(rename = "resourceManifestJson")]
        resource_manifest_json: String,
        #[serde(rename = "resourceManifest")]
        #[ts(rename = "resourceManifest", type = "ResourceManifest")]
        resource_manifest: Value,
        #[serde(rename = "verifiedBindingBundleJson")]
        #[ts(rename = "verifiedBindingBundleJson")]
        verified_binding_bundle_json: String,
    },
    Error {
        #[serde(rename = "protocolVersion")]
        #[ts(rename = "protocolVersion")]
        protocol_version: u32,
        generation: u64,
        input: String,
        diagnostics: Vec<MotionDiagnosticWire>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(tag = "type", rename_all = "camelCase")]
pub enum StudioEventWire {
    Ready {
        generation: u64,
    },
    Seek {
        #[serde(rename = "timeS")]
        #[ts(rename = "timeS")]
        time_s: f64,
    },
    SelectClip {
        #[serde(rename = "clipId")]
        #[ts(rename = "clipId")]
        clip_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
pub struct StudioReportWire {
    pub schema_version: u32,
    pub status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<CodeWire>,
}

fn declaration<T: TS + ?Sized>() -> String {
    T::decl(&Config::default().with_large_int("number"))
}

fn studio_host_protocol_version() -> u32 {
    let source = include_str!("../../../runtime-protocol-version.txt");
    let value = source.strip_suffix('\n').unwrap_or(source);
    assert!(
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
        "runtime protocol version must be decimal digits with an optional newline"
    );
    value
        .parse()
        .expect("runtime protocol version must fit u32")
}

fn motion_source_map_version() -> u32 {
    // Read the authoritative compiler constant without linking the authoring toolchain.
    include_str!("../../../../crates/valle-compiler/src/motion/mod.rs")
        .lines()
        .find_map(|line| line.strip_prefix("pub const MOTION_SOURCE_MAP_VERSION: u32 = "))
        .and_then(|value| value.strip_suffix(';'))
        .expect("compiler must declare MOTION_SOURCE_MAP_VERSION")
        .parse()
        .expect("Motion source-map version must fit u32")
}

pub fn render_types(include_drift_probe: bool) -> String {
    let declarations = [
        declaration::<MotionFrameRateWire>(),
        declaration::<CodeWire>(),
        declaration::<StudioSessionWire>(),
        declaration::<StudioCapabilitiesWire>(),
        declaration::<StudioRuntimeWire>(),
        declaration::<StudioBootWire>(),
        declaration::<MotionTimingWire>(),
        declaration::<MotionCueWindowWire>(),
        declaration::<MotionViewportWire>(),
        declaration::<MotionSourceSpanWire>(),
        declaration::<MotionDiagnosticWire>(),
        declaration::<MotionModuleSourceInfoWire>(),
        declaration::<MotionSourceMapWire>(),
        declaration::<MotionAssetWire>(),
        declaration::<MotionResourceLocatorWire>(),
        declaration::<MotionShaderWire>(),
        declaration::<MotionContextWire>(),
        declaration::<StudioEventWire>(),
        declaration::<StudioReportWire>(),
    ];

    let mut out = format!(
        "// @generated by web/tools/schema-gen; do not edit.\n\
         // Non-Timeline host protocols are generated from their Rust DTOs.\n\
         // Canonical/storage types come from @valle/engine's private internal Timeline module.\n\n\
         import type {{ ResourceManifest, TimelineDocument }} from \"../internal-timeline.ts\";\n\n\
         export const STUDIO_HOST_PROTOCOL_VERSION = {} as const;\n\
         export const MOTION_SOURCE_MAP_VERSION = {} as const;\n\n\
         export type ContentDigest = string;\n\n",
        studio_host_protocol_version(),
        motion_source_map_version(),
    );
    for item in declarations {
        out.push_str("export ");
        out.push_str(
            &item
                .lines()
                .map(str::trim_end)
                .collect::<Vec<_>>()
                .join("\n"),
        );
        out.push_str("\n\n");
    }
    out.truncate(out.trim_end().len());
    if include_drift_probe {
        out.push_str("\n\nexport type RustDriftProbe = { changedField: number };");
    }
    out.push('\n');
    out
}
