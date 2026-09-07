use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, ensure};
use ort::ep;
use ort::session::builder::{BuilderResult, GraphOptimizationLevel, SessionBuilder};
use ort::session::{Session, SessionInputValue};
use ort::value::{PrimitiveTensorElementType, TensorRef};

use crate::models::runtime::{TensorInput, TensorOutput};

/// Borrowed input data accepted by [`OnnxSession::run_typed`].
#[derive(Debug, Clone, Copy)]
pub enum OnnxTensorData<'a> {
    F32(&'a [f32]),
    I32(&'a [i32]),
    U8(&'a [u8]),
}

/// A named ONNX input with an explicit element type.
///
/// The runtime validates shape products for every type and additionally rejects
/// NaN and infinite values in `f32` inputs before entering ONNX Runtime.
#[derive(Debug, Clone)]
pub struct OnnxTensorInput<'a> {
    pub name: std::borrow::Cow<'a, str>,
    pub shape: Vec<usize>,
    pub data: OnnxTensorData<'a>,
}

/// Owned `i64` tensor returned by an ONNX graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnnxI64TensorOutput {
    pub name: String,
    pub shape: Vec<usize>,
    pub data: Vec<i64>,
}

struct PrimitiveTensorOutput<T> {
    name: String,
    shape: Vec<usize>,
    data: Vec<T>,
}

impl<'a> OnnxTensorInput<'a> {
    pub fn borrowed_f32(name: &'a str, shape: impl Into<Vec<usize>>, data: &'a [f32]) -> Self {
        Self {
            name: std::borrow::Cow::Borrowed(name),
            shape: shape.into(),
            data: OnnxTensorData::F32(data),
        }
    }

    pub fn borrowed_i32(name: &'a str, shape: impl Into<Vec<usize>>, data: &'a [i32]) -> Self {
        Self {
            name: std::borrow::Cow::Borrowed(name),
            shape: shape.into(),
            data: OnnxTensorData::I32(data),
        }
    }

    pub fn borrowed_u8(name: &'a str, shape: impl Into<Vec<usize>>, data: &'a [u8]) -> Self {
        Self {
            name: std::borrow::Cow::Borrowed(name),
            shape: shape.into(),
            data: OnnxTensorData::U8(data),
        }
    }
}

/// ONNX Runtime session using its portable CPU execution provider.
pub struct OnnxSession {
    session: Session,
}

#[derive(Debug, Clone)]
pub struct OnnxSessionOptions {
    pub intra_threads: Option<usize>,
    pub memory_pattern: bool,
    pub prepacking: bool,
}

pub const REQUIRED_ORT_MAJOR: u32 = 1;
pub const REQUIRED_ORT_MINOR: u32 = 28;

/// Process-level ONNX Runtime identity used for diagnostics and provenance.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OnnxRuntimeDiagnostics {
    pub runtime_version: String,
    pub required_version: String,
    pub runtime_path: Option<PathBuf>,
    pub build_info: String,
}

static ONNX_RUNTIME: OnceLock<std::result::Result<OnnxRuntimeDiagnostics, String>> =
    OnceLock::new();

impl Default for OnnxSessionOptions {
    fn default() -> Self {
        Self {
            intra_threads: None,
            memory_pattern: true,
            prepacking: true,
        }
    }
}

impl OnnxSession {
    pub fn load(path: &Path) -> Result<Self> {
        Self::load_with_options(path, &OnnxSessionOptions::default())
    }

    pub fn load_with_options(path: &Path, options: &OnnxSessionOptions) -> Result<Self> {
        runtime_diagnostics()?;
        if let Some(intra_threads) = options.intra_threads {
            ensure!(intra_threads > 0, "ONNX intra_threads must be positive");
        }
        let mut builder = Session::builder()?;
        builder = recover_builder(builder.with_no_environment_execution_providers())?;
        builder = recover_builder(
            builder.with_execution_providers([ep::CPU::default()
                .with_arena_allocator(true)
                .build()
                .error_on_failure()]),
        )?;
        builder = recover_builder(builder.with_optimization_level(GraphOptimizationLevel::All))?;
        builder = recover_builder(builder.with_parallel_execution(false))?;
        builder = recover_builder(builder.with_inter_threads(1))?;
        builder = recover_builder(builder.with_memory_pattern(options.memory_pattern))?;
        builder = recover_builder(builder.with_prepacking(options.prepacking))?;
        if let Some(intra_threads) = options.intra_threads {
            builder = recover_builder(builder.with_intra_threads(intra_threads))?;
        }
        let session = builder
            .commit_from_file(path)
            .with_context(|| format!("failed to load ONNX model {}", path.display()))?;
        Ok(Self { session })
    }

    pub fn run_f32(&mut self, input: TensorInput<'_>, output_name: &str) -> Result<TensorOutput> {
        self.run_f32_many(std::slice::from_ref(&input), &[output_name])?
            .into_iter()
            .next()
            .context("single-output ONNX run returned no outputs")
    }

    /// Runs named f32 tensors and returns the requested named f32 outputs.
    ///
    /// Input data is borrowed for the duration of the call. Outputs are returned in exactly the
    /// same order as `output_names`, independent of their order in the ONNX graph.
    pub fn run_f32_many(
        &mut self,
        inputs: &[TensorInput<'_>],
        output_names: &[&str],
    ) -> Result<Vec<TensorOutput>> {
        let inputs = inputs
            .iter()
            .map(|input| OnnxTensorInput {
                name: std::borrow::Cow::Borrowed(input.name.as_ref()),
                shape: input.shape.clone(),
                data: OnnxTensorData::F32(input.data.as_ref()),
            })
            .collect::<Vec<_>>();
        self.run_typed(&inputs, output_names)
    }

    /// Runs named `f32`, `i32` and `u8` tensors and returns requested named `f32` outputs.
    ///
    /// Inputs are borrowed for the duration of the call. Output ordering follows
    /// `output_names`, independently of graph declaration order.
    pub fn run_typed(
        &mut self,
        inputs: &[OnnxTensorInput<'_>],
        output_names: &[&str],
    ) -> Result<Vec<TensorOutput>> {
        self.run_typed_primitive::<f32>(inputs, output_names)?
            .into_iter()
            .map(|output| {
                Ok(TensorOutput {
                    name: output.name,
                    shape: output.shape,
                    data: output.data,
                })
            })
            .collect()
    }

    /// Runs named typed inputs and returns the requested named `i64` outputs.
    ///
    /// This is intended for graphs whose semantic result is an index tensor,
    /// such as shot-range and transition-class predictions.
    pub fn run_typed_i64(
        &mut self,
        inputs: &[OnnxTensorInput<'_>],
        output_names: &[&str],
    ) -> Result<Vec<OnnxI64TensorOutput>> {
        self.run_typed_primitive::<i64>(inputs, output_names)?
            .into_iter()
            .map(|output| {
                Ok(OnnxI64TensorOutput {
                    name: output.name,
                    shape: output.shape,
                    data: output.data,
                })
            })
            .collect()
    }

    fn run_typed_primitive<T>(
        &mut self,
        inputs: &[OnnxTensorInput<'_>],
        output_names: &[&str],
    ) -> Result<Vec<PrimitiveTensorOutput<T>>>
    where
        T: PrimitiveTensorElementType + Clone,
    {
        validate_typed_request(inputs, output_names)?;
        let input_values = inputs
            .iter()
            .map(|input| {
                let value = match input.data {
                    OnnxTensorData::F32(data) => SessionInputValue::from(
                        TensorRef::from_array_view((input.shape.clone(), data))?,
                    ),
                    OnnxTensorData::I32(data) => SessionInputValue::from(
                        TensorRef::from_array_view((input.shape.clone(), data))?,
                    ),
                    OnnxTensorData::U8(data) => SessionInputValue::from(
                        TensorRef::from_array_view((input.shape.clone(), data))?,
                    ),
                };
                Ok((input.name.clone(), value))
            })
            .collect::<Result<Vec<_>>>()?;
        let outputs = self.session.run(input_values)?;

        output_names
            .iter()
            .map(|&output_name| {
                let output = outputs.get(output_name).ok_or_else(|| {
                    anyhow!("ONNX graph did not return requested output {output_name:?}")
                })?;
                let (shape, data) = output
                    .try_extract_tensor::<T>()
                    .with_context(|| format!("output {output_name:?} has the wrong tensor type"))?;
                let shape = shape
                    .iter()
                    .map(|&dimension| {
                        usize::try_from(dimension).with_context(|| {
                            format!("output {output_name:?} has dynamic dimension {dimension}")
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(PrimitiveTensorOutput {
                    name: output_name.into(),
                    shape,
                    data: data.to_vec(),
                })
            })
            .collect()
    }

    /// Returns all custom metadata entries embedded in the ONNX model.
    pub fn custom_metadata(&self) -> Result<BTreeMap<String, String>> {
        let metadata = self.session.metadata()?;
        metadata
            .custom_keys()?
            .into_iter()
            .map(|key| {
                let value = metadata
                    .custom(&key)
                    .with_context(|| format!("ONNX custom metadata key {key:?} has no value"))?;
                Ok((key, value))
            })
            .collect()
    }

    pub fn input_names(&self) -> Vec<String> {
        self.session
            .inputs()
            .iter()
            .map(|input| input.name().to_owned())
            .collect()
    }

    pub fn output_names(&self) -> Vec<String> {
        self.session
            .outputs()
            .iter()
            .map(|output| output.name().to_owned())
            .collect()
    }
}

/// Initialize ONNX Runtime exactly once, verify the loaded binary is 1.28.x, and report it.
pub fn runtime_diagnostics() -> Result<OnnxRuntimeDiagnostics> {
    match ONNX_RUNTIME
        .get_or_init(|| initialize_runtime_once().map_err(|error| format!("{error:#}")))
    {
        Ok(diagnostics) => Ok(diagnostics.clone()),
        Err(error) => Err(anyhow!(error.clone())),
    }
}

#[cfg(test)]
fn validate_request(inputs: &[TensorInput<'_>], output_names: &[&str]) -> Result<()> {
    let inputs = inputs
        .iter()
        .map(|input| OnnxTensorInput {
            name: std::borrow::Cow::Borrowed(input.name.as_ref()),
            shape: input.shape.clone(),
            data: OnnxTensorData::F32(input.data.as_ref()),
        })
        .collect::<Vec<_>>();
    validate_typed_request(&inputs, output_names)
}

fn validate_typed_request(inputs: &[OnnxTensorInput<'_>], output_names: &[&str]) -> Result<()> {
    ensure!(!inputs.is_empty(), "ONNX run requires at least one input");
    ensure!(
        !output_names.is_empty(),
        "ONNX run requires at least one output"
    );

    let mut input_names = BTreeSet::new();
    for input in inputs {
        ensure!(!input.name.is_empty(), "ONNX input name cannot be empty");
        ensure!(
            input_names.insert(input.name.as_ref()),
            "duplicate ONNX input name {:?}",
            input.name
        );
        let expected = input
            .shape
            .iter()
            .try_fold(1_usize, |count, dimension| count.checked_mul(*dimension));
        let expected = expected.with_context(|| {
            format!(
                "input {:?} shape {:?} is too large",
                input.name, input.shape
            )
        })?;
        let values = match input.data {
            OnnxTensorData::F32(data) => {
                ensure!(
                    data.iter().all(|value| value.is_finite()),
                    "input {:?} contains NaN or infinite f32 values",
                    input.name
                );
                data.len()
            }
            OnnxTensorData::I32(data) => data.len(),
            OnnxTensorData::U8(data) => data.len(),
        };
        ensure!(
            values == expected,
            "input {:?} has {} values, shape {:?} requires {expected}",
            input.name,
            values,
            input.shape
        );
    }

    let mut requested_outputs = BTreeSet::new();
    for output_name in output_names {
        ensure!(!output_name.is_empty(), "ONNX output name cannot be empty");
        ensure!(
            requested_outputs.insert(*output_name),
            "duplicate ONNX output name {output_name:?}"
        );
    }
    Ok(())
}

fn recover_builder(result: BuilderResult) -> Result<SessionBuilder> {
    result.map_err(|error| anyhow::Error::msg(error.to_string()))
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn initialize_runtime_once() -> Result<OnnxRuntimeDiagnostics> {
    let executable = std::env::current_exe().context("failed to locate the running executable")?;
    let path = select_runtime_path(
        std::env::var_os("ORT_DYLIB_PATH"),
        &executable,
        std::env::consts::OS,
    )?;
    let path = path
        .canonicalize()
        .with_context(|| format!("failed to resolve ONNX Runtime bundle {}", path.display()))?;
    let runtime_version = dynamic_runtime_version(&path)?;
    validate_runtime_version(&runtime_version)?;
    ort::init_from(&path)
        .map_err(|error| {
            let library = runtime_library_filename(std::env::consts::OS)
                .unwrap_or("the platform ONNX Runtime library");
            anyhow::anyhow!(
                "failed to load ONNX Runtime from {}: {error}; place {library} next to the executable or set ORT_DYLIB_PATH",
                path.display(),
            )
        })?
        .commit();
    Ok(OnnxRuntimeDiagnostics {
        runtime_version,
        required_version: required_runtime_version(),
        runtime_path: Some(path),
        build_info: ort::info().to_owned(),
    })
}

fn select_runtime_path(
    explicit: Option<std::ffi::OsString>,
    executable: &Path,
    target_os: &str,
) -> Result<PathBuf> {
    if let Some(explicit) = explicit.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(explicit));
    }
    adjacent_runtime_path(executable, target_os)
}

fn adjacent_runtime_path(executable: &Path, target_os: &str) -> Result<PathBuf> {
    let directory = executable
        .parent()
        .context("the running executable has no parent directory")?;
    Ok(directory.join(
        runtime_library_filename(target_os)
            .ok_or_else(|| anyhow!("ONNX Runtime is not packaged for {target_os}"))?,
    ))
}

fn runtime_library_filename(target_os: &str) -> Option<&'static str> {
    match target_os {
        "macos" => Some("libonnxruntime.dylib"),
        "linux" => Some("libonnxruntime.so"),
        "windows" => Some("onnxruntime.dll"),
        _ => None,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn dynamic_runtime_version(path: &Path) -> Result<String> {
    type GetApiBase = unsafe extern "system" fn() -> *const ort::sys::OrtApiBase;
    let library = unsafe { libloading::Library::new(path) }
        .with_context(|| format!("failed to inspect ONNX Runtime bundle {}", path.display()))?;
    let get_api_base: libloading::Symbol<'_, GetApiBase> = unsafe {
        library
            .get(b"OrtGetApiBase")
            .with_context(|| format!("{} does not export OrtGetApiBase", path.display()))?
    };
    let base = unsafe { get_api_base() };
    runtime_version_from_base(base)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn initialize_runtime_once() -> Result<OnnxRuntimeDiagnostics> {
    Err(anyhow!(
        "ONNX Runtime is not packaged for {}",
        std::env::consts::OS
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn runtime_version_from_base(base: *const ort::sys::OrtApiBase) -> Result<String> {
    ensure!(!base.is_null(), "ONNX Runtime returned a null API base");
    let version = unsafe { ((*base).GetVersionString)() };
    ensure!(
        !version.is_null(),
        "ONNX Runtime returned a null version string"
    );
    Ok(unsafe { CStr::from_ptr(version) }
        .to_str()
        .context("ONNX Runtime version is not UTF-8")?
        .to_owned())
}

fn required_runtime_version() -> String {
    format!("{REQUIRED_ORT_MAJOR}.{REQUIRED_ORT_MINOR}.x")
}

fn validate_runtime_version(version: &str) -> Result<()> {
    let core = version
        .split(['-', '+'])
        .next()
        .context("ONNX Runtime returned an empty version")?;
    let mut parts = core.split('.');
    let major = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .with_context(|| format!("invalid ONNX Runtime version {version:?}"))?;
    let minor = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .with_context(|| format!("invalid ONNX Runtime version {version:?}"))?;
    ensure!(
        major == REQUIRED_ORT_MAJOR && minor == REQUIRED_ORT_MINOR,
        "unsupported ONNX Runtime {version}; required {}",
        required_runtime_version()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{OnnxSession, runtime_diagnostics};
    use super::{
        OnnxSessionOptions, OnnxTensorInput, validate_request, validate_runtime_version,
        validate_typed_request,
    };
    use crate::models::runtime::TensorInput;

    const NAMED_IO_METADATA_MODEL: &[u8] = &[
        0x08, 0x07, 0x12, 0x19, 0x76, 0x61, 0x6c, 0x6c, 0x65, 0x2d, 0x6d, 0x6f, 0x64, 0x65, 0x6c,
        0x2d, 0x72, 0x75, 0x6e, 0x74, 0x69, 0x6d, 0x65, 0x2d, 0x74, 0x65, 0x73, 0x74, 0x73, 0x3a,
        0x9e, 0x01, 0x0a, 0x17, 0x0a, 0x04, 0x6c, 0x65, 0x66, 0x74, 0x0a, 0x05, 0x72, 0x69, 0x67,
        0x68, 0x74, 0x12, 0x03, 0x73, 0x75, 0x6d, 0x22, 0x03, 0x41, 0x64, 0x64, 0x0a, 0x1e, 0x0a,
        0x04, 0x6c, 0x65, 0x66, 0x74, 0x0a, 0x05, 0x72, 0x69, 0x67, 0x68, 0x74, 0x12, 0x0a, 0x64,
        0x69, 0x66, 0x66, 0x65, 0x72, 0x65, 0x6e, 0x63, 0x65, 0x22, 0x03, 0x53, 0x75, 0x62, 0x12,
        0x0d, 0x6e, 0x61, 0x6d, 0x65, 0x64, 0x2d, 0x69, 0x6f, 0x2d, 0x74, 0x65, 0x73, 0x74, 0x5a,
        0x12, 0x0a, 0x04, 0x6c, 0x65, 0x66, 0x74, 0x12, 0x0a, 0x0a, 0x08, 0x08, 0x01, 0x12, 0x04,
        0x0a, 0x02, 0x08, 0x02, 0x5a, 0x13, 0x0a, 0x05, 0x72, 0x69, 0x67, 0x68, 0x74, 0x12, 0x0a,
        0x0a, 0x08, 0x08, 0x01, 0x12, 0x04, 0x0a, 0x02, 0x08, 0x02, 0x62, 0x11, 0x0a, 0x03, 0x73,
        0x75, 0x6d, 0x12, 0x0a, 0x0a, 0x08, 0x08, 0x01, 0x12, 0x04, 0x0a, 0x02, 0x08, 0x02, 0x62,
        0x18, 0x0a, 0x0a, 0x64, 0x69, 0x66, 0x66, 0x65, 0x72, 0x65, 0x6e, 0x63, 0x65, 0x12, 0x0a,
        0x0a, 0x08, 0x08, 0x01, 0x12, 0x04, 0x0a, 0x02, 0x08, 0x02, 0x42, 0x04, 0x0a, 0x00, 0x10,
        0x0d, 0x72, 0x12, 0x0a, 0x07, 0x61, 0x64, 0x61, 0x70, 0x74, 0x65, 0x72, 0x12, 0x07, 0x64,
        0x70, 0x64, 0x66, 0x6e, 0x65, 0x74, 0x72, 0x14, 0x0a, 0x0b, 0x73, 0x61, 0x6d, 0x70, 0x6c,
        0x65, 0x5f, 0x72, 0x61, 0x74, 0x65, 0x12, 0x05, 0x34, 0x38, 0x30, 0x30, 0x30,
    ];

    #[test]
    fn cpu_defaults_enable_reusable_fixed_shape_optimizations() {
        let options = OnnxSessionOptions::default();
        assert_eq!(options.intra_threads, None);
        assert!(options.memory_pattern);
        assert!(options.prepacking);
    }

    #[test]
    fn runtime_version_gate_requires_exact_1_28_minor() {
        assert!(validate_runtime_version("1.28.0").is_ok());
        assert!(validate_runtime_version("1.28.1+vendor").is_ok());
        assert!(validate_runtime_version("1.27.9").is_err());
        assert!(validate_runtime_version("1.29.0").is_err());
        assert!(validate_runtime_version("2.28.0").is_err());
        assert!(validate_runtime_version("invalid").is_err());
    }

    #[test]
    #[ignore = "requires the pinned ORT_DYLIB_PATH bundle"]
    fn runtime_diagnostics_are_process_stable_and_report_1_28() -> anyhow::Result<()> {
        let first = runtime_diagnostics()?;
        let second = runtime_diagnostics()?;
        assert_eq!(first, second);
        assert!(first.runtime_version.starts_with("1.28."));
        assert_eq!(first.required_version, "1.28.x");
        assert!(!first.build_info.is_empty());
        Ok(())
    }

    #[test]
    fn named_request_validation_rejects_ambiguous_or_malformed_tensors() {
        let values = [1.0_f32, 2.0];
        let valid = TensorInput::borrowed("input", [2], &values);
        assert!(validate_request(std::slice::from_ref(&valid), &["output"]).is_ok());

        let wrong_shape = TensorInput::borrowed("input", [3], &values);
        assert!(
            validate_request(&[wrong_shape], &["output"])
                .unwrap_err()
                .to_string()
                .contains("requires 3")
        );
        assert!(
            validate_request(&[valid.clone(), valid], &["output"])
                .unwrap_err()
                .to_string()
                .contains("duplicate ONNX input")
        );
        assert!(
            validate_request(
                &[TensorInput::borrowed("input", [2], &values)],
                &["output", "output"]
            )
            .unwrap_err()
            .to_string()
            .contains("duplicate ONNX output")
        );
        assert!(
            validate_request(
                &[TensorInput::borrowed("input", [1], &[f32::NAN])],
                &["output"]
            )
            .unwrap_err()
            .to_string()
            .contains("NaN or infinite")
        );

        let labels = [1_i32, 0];
        let pixels = [255_u8, 0];
        let typed = [
            OnnxTensorInput::borrowed_f32("image", [1, 2], &values),
            OnnxTensorInput::borrowed_i32("labels", [2], &labels),
            OnnxTensorInput::borrowed_u8("pixels", [1, 2], &pixels),
        ];
        assert!(validate_typed_request(&typed, &["mask"]).is_ok());
        assert!(
            validate_typed_request(
                &[OnnxTensorInput::borrowed_i32("labels", [3], &labels)],
                &["mask"]
            )
            .unwrap_err()
            .to_string()
            .contains("requires 3")
        );
    }

    #[test]
    fn metadata_map_type_has_deterministic_key_order() {
        let metadata = BTreeMap::from([
            ("sample_rate".to_string(), "48000".to_string()),
            ("adapter".to_string(), "dpdfnet".to_string()),
        ]);
        assert_eq!(
            metadata.keys().map(String::as_str).collect::<Vec<_>>(),
            ["adapter", "sample_rate"]
        );
    }

    #[test]
    fn runtime_path_selection_prefers_override_then_platform_adjacent_library() -> anyhow::Result<()>
    {
        let executable = Path::new("/opt/valle/bin/valle");
        assert_eq!(
            super::select_runtime_path(
                Some("/runtime/override/libort".into()),
                executable,
                "linux",
            )?,
            PathBuf::from("/runtime/override/libort")
        );
        for (target_os, filename) in [
            ("macos", "libonnxruntime.dylib"),
            ("linux", "libonnxruntime.so"),
            ("windows", "onnxruntime.dll"),
        ] {
            assert_eq!(
                super::select_runtime_path(None, executable, target_os)?,
                Path::new("/opt/valle/bin").join(filename)
            );
        }
        assert!(super::select_runtime_path(None, executable, "freebsd").is_err());
        Ok(())
    }

    #[test]
    #[ignore = "requires the pinned ORT_DYLIB_PATH bundle"]
    fn named_multi_io_and_custom_metadata_round_trip() -> anyhow::Result<()> {
        let path = std::env::temp_dir().join(format!(
            "valle-model-runtime-named-io-{}.onnx",
            std::process::id()
        ));
        fs::write(&path, NAMED_IO_METADATA_MODEL)?;

        let result = (|| {
            let mut session = OnnxSession::load(&path)?;
            assert_eq!(
                session.custom_metadata()?,
                BTreeMap::from([
                    ("adapter".to_string(), "dpdfnet".to_string()),
                    ("sample_rate".to_string(), "48000".to_string()),
                ])
            );

            let left = [5.0_f32, 3.0];
            let right = [2.0_f32, 7.0];
            let inputs = [
                TensorInput::borrowed("left", [2], &left),
                TensorInput::borrowed("right", [2], &right),
            ];
            let outputs = session.run_f32_many(&inputs, &["difference", "sum"])?;
            assert_eq!(outputs.len(), 2);
            assert_eq!(outputs[0].name, "difference");
            assert_eq!(outputs[0].shape, [2]);
            assert_eq!(outputs[0].data, [3.0, -4.0]);
            assert_eq!(outputs[1].name, "sum");
            assert_eq!(outputs[1].shape, [2]);
            assert_eq!(outputs[1].data, [7.0, 10.0]);
            Ok(())
        })();

        let _ = fs::remove_file(path);
        result
    }
}
