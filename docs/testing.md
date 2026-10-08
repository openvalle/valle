# Rust tests and coverage

## Continuous integration

Six manually dispatched `CI <OS> <architecture>` workflows independently build and verify macOS, Linux and Windows on arm64 and x86_64, run the ordinary Rust workspace tests for that target, and exercise real FFmpeg codec/muxer roundtrips. Each calls its matching `Package <OS> <architecture>` entry, backed by reusable OS build workflows. macOS x86_64 is cross-compiled on an arm64 `macos-15` runner and tested under Rosetta with FFmpeg 7/8/9 shared libraries cross-compiled from SHA-256-verified sources by `.github/scripts/install-ffmpeg-macos.sh`; Chrome runs natively on the arm64 host. macOS arm64 uses Homebrew. Linux and Windows use native runners and SHA-256-verified FFmpeg 9 shared-library bundles pinned in `.github/scripts/install-ffmpeg.ts`. Test runtimes are not included in the distributed packages.

The macOS arm64 job also runs all Web tests and generated-boundary checks. Formatting and packed Web SDK validation run automatically on pull requests and pushes to `main`. Native CI entries run only when dispatched, so a platform-specific fix can be verified without rebuilding the other five targets:

```sh
gh workflow run ci-macos-x86_64.yml --ref main
```

Hardware, real model-weight and opt-in runtime acceptance tests remain explicitly selected outside the ordinary suite as described below.

The full Rust workspace uses the default test profile with debug assertions enabled, reusing native dependencies built by the packaging-tool checks and avoiding thin-LTO linking of every test executable. CI disables debug symbols and incremental compilation to limit disk usage. Release-mode CLI runtime contracts and codec/muxer tests separately exercise the packaged build configuration.

macOS CI uses ad-hoc signing without Apple secrets. Manually dispatched macOS packaging requires Developer ID signing and accepted notarization for the selected architecture.

## Local tests and coverage

Use the toolchain pinned in `rust-toolchain.toml`: Rust 1.96.0. The native coverage run uses `cargo-llvm-cov` 0.9.1 and Rust's matching LLVM 22.1.2 tools; do not merge profiles from another Rust/LLVM version.

```sh
rustup component add llvm-tools-preview --toolchain 1.96.0
cargo install cargo-llvm-cov --version 0.9.1 --locked
```

Build the Web runtime and distribution notices before selecting the CLI's `embedded-runtime` feature. Set `VALLE_EMBED_WEB_DIR` to the built `web/dist` directory and `VALLE_EMBED_NOTICES` to the generated notices file. Native media tests need an installed FFmpeg library. The macOS coverage run uses the macOS 26.5 SDK for the pinned Skia build.

Use a dedicated coverage target directory to keep instrumentation separate from ordinary builds. Start with fresh profiles and matching workspace artifacts when validating changed source; retain the baseline JSON for comparison. Cached executables from older source versions can contribute obsolete coverage mappings. Clean workspace coverage artifacts before a new run, or generate reports from the current Cargo artifact list and all executed acceptance binaries.

```sh
export CARGO_LLVM_COV_TARGET_DIR="$PWD/target/llvm-cov-native-1.96.0"
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0

cargo +1.96.0 llvm-cov clean --workspace

cargo +1.96.0 llvm-cov --coverage-host-only --workspace \
  --features valle-cli/embedded-runtime --tests --locked \
  --no-report --no-fail-fast -- --test-threads=1
```

The ordinary run leaves explicitly ignored runtime/hardware tests unselected. Run the following acceptance targets separately on a machine with the required libraries and hardware:

```sh
cargo +1.96.0 llvm-cov --coverage-host-only --workspace \
  --features valle-cli/embedded-runtime --locked --no-report \
  --test contract_graph --test media_contract --test shared_gpu_roundtrip \
  --test motion_metal --test motion_host_time \
  -- --include-ignored --test-threads=1

cargo +1.96.0 llvm-cov --coverage-host-only --workspace \
  --features valle-cli/embedded-runtime --locked --no-report \
  --lib -- tools::segment::contract_tests --ignored --test-threads=1

cargo +1.96.0 llvm-cov --coverage-host-only --workspace \
  --features valle-cli/embedded-runtime --locked --no-report \
  --lib -- transport::shared_frame::contract_tests --ignored --test-threads=1
```

`ORT_DYLIB_PATH` must point to a compatible ONNX Runtime 1.28 library. The checked-in ONNX and CoreML test assets are documented in [the inference fixture README](../crates/valle-media/tests/fixtures/models/README.md). Rust tests load these assets directly. CoreML tests run the checked-in affine model through the native framework on macOS.

Qwen acceptance tests run only on macOS and require `VALLE_QWEN_ASR_DIR`, `VALLE_QWEN_ALIGNER_DIR`, and `VALLE_QWEN_FIXTURE_WAV`. The directories must contain the real pinned 1.0.0 model artifacts; the WAV is mono 16 kHz speech. These tests assert nonempty recognition and monotonic, bounded alignment timestamps. Linux and Windows instead verify that transcription is rejected before loading dependencies or changing output files. Other model adapters use deterministic graphs and exact pixel/sample/timing assertions.

Media backend implementations are compiled for FFmpeg 7, 8, and 9. Repeat codec unit/integration tests and `shared_gpu_roundtrip` with `VALLE_FFMPEG_DIR` set to each actual ABI installation; two aliases pointing to one library do not verify two ABIs. Keep ignored library tests explicitly selected by name: some are child-process roles that require parent-provided environment variables.

Crash recovery tests deliberately abort. Ordinary process-exit profile writing loses the aborted child counters. On Apple Silicon, the coverage verification relinks `assets_crash` and `revision_crash` with 16 KiB alignment for `__DATA,__llvm_prf_cnts`, `__llvm_prf_bits`, `__llvm_prf_data`, and the following `__data` section, then runs them with `%c` in `LLVM_PROFILE_FILE` to collect continuous profiles. Production crash behavior remains unchanged.

Generate reports using the same source scope as the baseline:

```sh
cargo +1.96.0 llvm-cov report \
  --ignore-filename-regex '(^|/)(tests|test_support)(/|\.rs$)|/[^/]+_tests?\.rs$' \
  --json --output-path target/coverage/rust.json --fail-under-file-lines 70

cargo +1.96.0 llvm-cov report \
  --ignore-filename-regex '(^|/)(tests|test_support)(/|\.rs$)|/[^/]+_tests?\.rs$' \
  --html --output-dir target/coverage/rust
```

The threshold applies to each reported source file's **line coverage**, rather than the workspace average. Standalone test files and test-support modules are excluded; existing inline unit tests remain in LLVM's source-file totals. Dependency and build-script sources are excluded by the workspace report. Stable doctest and branch coverage are not collected in this run. No production file is excluded to meet the threshold.

## Verified native run, 2026-10-06

The Apple Silicon run rebuilt the full workspace, then selected the native model/hardware tests, all three FFmpeg ABIs, actual package verification, and continuously profiled crash recovery tests. The baseline's 117 physical source files below 70% all exceeded 70% in the final report; their minimum was 70.0166%. Each ABI expansion was checked separately before grouping it under its physical source file.

| Metric | Before | After |
| --- | ---: | ---: |
| Line coverage | 74.1875% | 84.3017% |
| Function coverage | 73.6470% | 82.0427% |
| Region coverage | 74.2486% | 83.9634% |
| Original source files below 70% | 117 | 0 |

The full workspace run passed 2,418 tests across 207 executables. After explicitly selecting acceptance tests and deduplicating repeated ABI runs, 2,444 tests passed with zero failures. Twenty-one test roles remained ignored, including opt-in resource proofs and parent-invoked child roles; their skipped status does not mean all corresponding code was unexecuted. All 478 local links in the HTML index were valid.

Local artifacts from this run are `target/coverage/per-file-70/final.json`, `html/index.html`, `final-summary.json`, and `current-report-inputs.json`. The summary records every original low-coverage file and its final percentage. The input list records the current Cargo artifacts, all executed acceptance binaries, and fresh LLVM version-10 profiles; obsolete binaries were omitted to avoid counting older source mappings.
