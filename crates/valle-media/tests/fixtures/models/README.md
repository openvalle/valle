# Deterministic inference test fixtures

The ONNX files are tiny computation graphs with fixed outputs or simple arithmetic, not trained model weights. They exercise the real ONNX Runtime sessions, tensor validation, model adapters, media timing, and output publication. Tests assert exact pixels, alpha values, audio samples, or shot ranges. They do not measure model quality.

The ONNX and CoreML fixtures are stored directly in the repository. Rust tests load these files without a generation step. `affine.mlpackage` computes `2*x+1` for the native CoreML integration test.

ONNX integration tests require `ORT_DYLIB_PATH` and are explicitly marked ignored so they can be selected when the native runtime is available. Select `contract_graph` with `--include-ignored`; the unit tests under `tools::segment::contract_tests` also need this runtime. Test support creates an isolated version `1.0.1` with actual fixture SHA-256 hashes, then uses normal catalog and model-store verification. Published releases and production model hashes remain unchanged.

These assets are under the test directory and are not included in the Valle distribution. Qwen ASR and alignment tests separately use real published model weights.
