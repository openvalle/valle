//! External analyzer transport: one JSON request on stdin and one response on stdout per batch.
//! Timeouts, nonzero exits, and invalid JSON become analyzer failures with stderr context. Requests
//! carry analyzer identity, asset details, parameters, and optional batch data; responses carry
//! items and cost.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::assets::analysis::{Analyzer, AnalyzerInput, AnalyzerOutput};
use crate::assets::kind::AssetKind;
use crate::assets::report::{AssetsError, Result};

/// External command analyzer with arguments, timeout, and supported kinds.
pub struct ExternalAnalyzer {
    pub name: &'static str,
    pub version: u32,
    pub kinds: Vec<AssetKind>,
    pub deps: &'static [&'static str],
    pub argv: Vec<String>,
    pub timeout: Duration,
    pub params: Value,
}

impl Analyzer for ExternalAnalyzer {
    fn name(&self) -> &'static str {
        self.name
    }
    fn version(&self) -> u32 {
        self.version
    }
    fn accepts(&self, kind: AssetKind) -> bool {
        self.kinds.contains(&kind)
    }
    fn dependencies(&self) -> &'static [&'static str] {
        self.deps
    }
    fn default_params(&self) -> Value {
        self.params.clone()
    }
    fn analyze(&self, input: &AnalyzerInput<'_>) -> Result<AnalyzerOutput> {
        let request = json!({
            "analyzer": self.name,
            "version": self.version,
            "hash": input.hash,
            "path": input.path.to_string_lossy(),
            "kind": input.kind.as_str(),
            "params": input.params,
        });
        run_external(&self.argv, &request, self.timeout)
    }
}

/// Run a child process, exchange JSON, and terminate it on timeout.
pub fn run_external(argv: &[String], request: &Value, timeout: Duration) -> Result<AnalyzerOutput> {
    let (prog, args) = argv
        .split_first()
        .ok_or_else(|| AssetsError::analyzer_failed("external analyzer argument list is empty"))?;
    let t0 = Instant::now();
    let mut child = Command::new(prog)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            AssetsError::analyzer_failed(format!("failed to start process {prog}: {e}"))
        })?;

    // Write one request and close stdin.
    {
        let mut stdin = child.stdin.take().expect("piped");
        let bytes = serde_json::to_vec(request)
            .map_err(|e| AssetsError::io(format!("failed to serialize request: {e}")))?;
        // An early child exit may produce EPIPE; collect stderr through the normal failure path.
        let _ = stdin.write_all(&bytes);
    }
    // Drain stdout and stderr concurrently to avoid pipe deadlocks while enforcing the timeout.
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let out_h = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_h = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AssetsError::analyzer_failed(format!(
                    "external analyzer timed out after {}s and was terminated",
                    timeout.as_secs_f64()
                ))
                .with_hint("allow enough timeout for large batches and local model startup"));
            }
            Err(e) => {
                return Err(AssetsError::analyzer_failed(format!(
                    "failed to wait for process: {e}"
                )));
            }
        }
    };
    let out = out_h.join().unwrap_or_default();
    let err_tail = String::from_utf8_lossy(&err_h.join().unwrap_or_default())
        .chars()
        .rev()
        .take(400)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();

    if !status.success() {
        return Err(AssetsError::analyzer_failed(format!(
            "external analyzer exited with {status}; stderr tail: {err_tail}"
        )));
    }
    let v: Value = serde_json::from_slice(&out).map_err(|e| {
        AssetsError::analyzer_failed(format!(
            "output is not valid JSON: {e}; stderr tail: {err_tail}"
        ))
        .with_hint("write one JSON object containing items and cost to stdout; send logs to stderr")
    })?;
    let items = v
        .get("items")
        .and_then(|x| x.as_array())
        .cloned()
        .ok_or_else(|| AssetsError::analyzer_failed("output is missing the items array"))?;
    let cost_ms = v
        .pointer("/cost/ms")
        .and_then(|x| x.as_i64())
        .unwrap_or_else(|| t0.elapsed().as_millis() as i64);
    let cost_fen = v.pointer("/cost/fen").and_then(|x| x.as_i64()).unwrap_or(0);
    Ok(AnalyzerOutput {
        items,
        cost_ms,
        cost_fen,
    })
}
