//! Motion authoring in the same Web Engine module used for playback.

use std::{collections::BTreeMap, sync::Arc};

use js_sys::{Array, Float32Array, Reflect, Uint8Array};
use serde::Deserialize;
use valle_compiler::motion::{
    AudioAnalysisEnv, AudioPcm, CompiledMotion, CompilerDiagnostic, MeasureEnv, MotionModuleGraph,
    PrepareDataBinding, compile_motion_modules_with_full_env_and_data_and_audio,
    compile_motion_with_full_env_and_data_and_audio,
};
use valle_motion::shader::{ShaderPackage, ShaderRegistry};
use valle_motion::{ContentDigest, ResourceRef};
use wasm_bindgen::{JsCast, JsError, JsValue, prelude::wasm_bindgen};

// rquickjs-sys requests an `env.__rquickjs_host_now_us` clock on wasm32. Motion preparation
// forbids Date/performance, and a fixed clock also keeps QuickJS initialization deterministic.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn __rquickjs_host_now_us() -> f64 {
    0.0
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompileOptions {
    #[serde(default)]
    resources: Vec<ResourceRef>,
    data: Option<PrepareDataBinding>,
    #[serde(default)]
    audio_sources: Vec<AudioSourceInput>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AudioSourceInput {
    control: String,
    content_hash: ContentDigest,
    sample_rate: u32,
}

thread_local! {
    // The Worker owns frozen typed arrays across source edits. Keep their Rust
    // allocation too, so a recompilation neither copies PCM nor repeats the FFT.
    static AUDIO_PCM: std::cell::RefCell<Vec<(Float32Array, Arc<AudioPcm>)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn cached_audio(source: &AudioSourceInput, pcm: Float32Array) -> Arc<AudioPcm> {
    AUDIO_PCM.with_borrow_mut(|cache| {
        if let Some((_, cached)) = cache.iter().find(|(array, cached)| {
            cached.content_hash == source.content_hash
                && cached.sample_rate == source.sample_rate
                && cached.samples.len() == pcm.length() as usize
                && js_sys::Object::is(array.as_ref(), pcm.as_ref())
        }) {
            return Arc::clone(cached);
        }
        let value = Arc::new(AudioPcm {
            content_hash: source.content_hash,
            sample_rate: source.sample_rate,
            samples: pcm.to_vec(),
        });
        const MAX_SAMPLES: usize = 128 * 1024 * 1024 / 4;
        while !cache.is_empty()
            && (cache.len() >= 16
                || cache.iter().map(|(_, p)| p.samples.len()).sum::<usize>() + value.samples.len()
                    > MAX_SAMPLES)
        {
            cache.remove(0);
        }
        if value.samples.len() <= MAX_SAMPLES {
            cache.push((pcm, Arc::clone(&value)));
        }
        value
    })
}

fn audio_env(sources: Vec<AudioSourceInput>, samples: &Array) -> Result<AudioAnalysisEnv, JsError> {
    if sources.len() != samples.length() as usize {
        return Err(JsError::new(
            "audio source metadata and PCM buffers must have equal length",
        ));
    }
    let mut env = AudioAnalysisEnv::default();
    for (index, source) in sources.into_iter().enumerate() {
        let pcm = samples
            .get(index as u32)
            .dyn_into::<Float32Array>()
            .map_err(|_| JsError::new("audio PCM must be a Float32Array"))?;
        if env
            .sources
            .insert(source.control.clone(), cached_audio(&source, pcm))
            .is_some()
        {
            return Err(JsError::new(&format!(
                "duplicate audio source `{}`",
                source.control
            )));
        }
    }
    Ok(env)
}

/// Compile a standalone Motion JSX source with explicit host resources.
#[wasm_bindgen]
pub fn compile_motion_jsx(
    source: &str,
    options_json: &str,
    fonts: &Array,
    font_aliases: &Array,
    shaders: &Array,
    audio_samples: &Array,
) -> Result<String, JsError> {
    let options = parse_options(options_json)?;
    let audio = audio_env(options.audio_sources, audio_samples)?;
    let measure = measure_env(fonts, font_aliases)?;
    let shaders = shader_registry(shaders)?;
    encode_result(compile_motion_with_full_env_and_data_and_audio(
        source,
        &options.resources,
        measure.as_ref(),
        Some(&shaders),
        options.data.as_ref(),
        Some(&audio),
    ))
}

/// Compile a complete, project-relative Motion module closure without filesystem access.
#[wasm_bindgen]
pub fn compile_motion_modules(
    entry: &str,
    modules_json: &str,
    options_json: &str,
    fonts: &Array,
    font_aliases: &Array,
    shaders: &Array,
    audio_samples: &Array,
) -> Result<String, JsError> {
    let options = parse_options(options_json)?;
    let audio = audio_env(options.audio_sources, audio_samples)?;
    let modules: BTreeMap<String, String> =
        serde_json::from_str(modules_json).map_err(|error| JsError::new(&error.to_string()))?;
    let graph = match MotionModuleGraph::new(entry, modules) {
        Ok(graph) => graph,
        Err(diagnostics) => return encode_result(Err(diagnostics)),
    };
    let measure = measure_env(fonts, font_aliases)?;
    let shaders = shader_registry(shaders)?;
    encode_result(compile_motion_modules_with_full_env_and_data_and_audio(
        &graph,
        &options.resources,
        measure.as_ref(),
        Some(&shaders),
        options.data.as_ref(),
        Some(&audio),
    ))
}

fn parse_options(json: &str) -> Result<CompileOptions, JsError> {
    serde_json::from_str(json).map_err(|error| JsError::new(&error.to_string()))
}

fn bytes(value: JsValue, label: &str) -> Result<Vec<u8>, JsError> {
    let bytes = value
        .dyn_into::<Uint8Array>()
        .map_err(|_| JsError::new(&format!("{label} must be a Uint8Array")))?;
    Ok(bytes.to_vec())
}

fn measure_env(fonts: &Array, aliases: &Array) -> Result<Option<MeasureEnv>, JsError> {
    let font_blobs = fonts
        .iter()
        .enumerate()
        .map(|(index, value)| bytes(value, &format!("fonts[{index}]")))
        .collect::<Result<Vec<_>, _>>()?;
    let aliases = aliases
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let pair = value
                .dyn_into::<Array>()
                .map_err(|_| JsError::new(&format!("fontAliases[{index}] must be a pair")))?;
            if pair.length() != 2 {
                return Err(JsError::new(&format!(
                    "fontAliases[{index}] must be a pair"
                )));
            }
            let name = pair
                .get(0)
                .as_string()
                .ok_or_else(|| JsError::new(&format!("fontAliases[{index}] needs a name")))?;
            Ok((name, bytes(pair.get(1), &format!("fontAliases[{index}]"))?))
        })
        .collect::<Result<Vec<_>, JsError>>()?;
    if font_blobs.is_empty() && aliases.is_empty() {
        return Ok(None);
    }
    MeasureEnv::new_unbound_with_aliases(&font_blobs, &aliases)
        .map(Some)
        .map_err(|error| JsError::new(&error.to_string()))
}

fn shader_registry(shaders: &Array) -> Result<ShaderRegistry, JsError> {
    let mut registry = ShaderRegistry::new();
    for (index, value) in shaders.iter().enumerate() {
        let frozen = Reflect::get(&value, &JsValue::from_str("frozenBytes"))
            .map_err(|_| JsError::new(&format!("shaders[{index}].frozenBytes is required")))?;
        let package =
            ShaderPackage::from_frozen(&bytes(frozen, &format!("shaders[{index}].frozenBytes"))?)
                .map_err(|error| JsError::new(&error.to_string()))?;
        let control = Reflect::get(&value, &JsValue::from_str("assetControl"))
            .map_err(|_| JsError::new(&format!("shaders[{index}].assetControl is invalid")))?;
        if control.is_undefined() {
            registry.register(package)
        } else {
            let name = control.as_string().ok_or_else(|| {
                JsError::new(&format!("shaders[{index}].assetControl must be a string"))
            })?;
            registry.register_asset(&name, package)
        }
        .map_err(|error| JsError::new(&error.to_string()))?;
    }
    Ok(registry)
}

fn encode_result(
    result: Result<CompiledMotion, Vec<CompilerDiagnostic>>,
) -> Result<String, JsError> {
    let value = match result {
        Ok(compiled) => {
            let canonical = valle_motion::canonical_bytes(&compiled.artifact)
                .map_err(|error| JsError::new(&error.to_string()))?;
            let artifact_digest = ContentDigest::of_bytes(&canonical);
            serde_json::json!({
                "status": "ok",
                "artifact": compiled.artifact,
                "artifactDigest": artifact_digest,
                "warnings": compiled.warnings,
                "sourceMap": compiled.source_map,
                "normalizedSource": compiled.normalized_source,
                "normalizedAstDigest": compiled.normalized_ast_digest,
                "preparedDataDigest": compiled.prepared_data_digest,
            })
        }
        Err(diagnostics) => serde_json::json!({
            "status": "error",
            "diagnostics": diagnostics,
        }),
    };
    serde_json::to_string(&value).map_err(|error| JsError::new(&error.to_string()))
}
