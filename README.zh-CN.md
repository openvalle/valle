# Valle

[English](README.md) | 简体中文

面向 AI Agent 的视频创作与剪辑引擎。

**目前是早期开发版。** 我们先开放代码，许多功能、构建工具和文档仍未完善。项目还存在缺陷，后续迭代也可能带来不兼容的变更。

## 环境依赖

- 通过 [rustup](https://rustup.rs/) 安装 Rust。仓库的 `rust-toolchain.toml` 指定 **Rust 1.96.0** 和 `wasm32-unknown-unknown` 目标。
- [Bun](https://bun.com/docs/installation) **1.4.2**。
- 原生构建工具：C/C++ 编译器、LLVM/libclang、`pkg-config`、CMake、Meson、Make、Perl、Python 3、Ninja、Git、curl、tar（x86_64 还需要 NASM）。参见 [rust-skia 构建要求](https://github.com/rust-skia/rust-skia#building)。

以下以 **macOS** 为例，Linux 和 Windows 构建环境见对应的[打包工作流](.github/workflows)。先安装上述 Rust 和 Bun，再通过 [Homebrew](https://brew.sh/) 安装原生依赖：

```sh
xcode-select --install # 尚未安装 Command Line Tools 时执行
brew install pkgconf llvm cmake ninja meson python
export PATH="$(brew --prefix llvm)/bin:$PATH"
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
```

## 构建与运行

```sh
git clone https://github.com/openvalle/valle.git
cd valle
cargo xtask build
```

首次构建需要联网，耗时可能较长。Cargo 从源码构建 Skia，以提供 Skottie 支持。已发布的 `valle-ffmpeg` 和 `valle-ffmpeg-sys` crate 包含固定的 FFmpeg 7.0/8.0/9.0 公共头文件；构建需要 libclang，无需安装 FFmpeg 或额外检出其他仓库。

`cargo xtask build` 生成可执行文件 **`dist/bin/valle`**（Windows 为 `dist/bin/valle.exe`），内嵌 Studio Web 资源、CanvasKit full、字体和依赖声明。使用 `cargo xtask build --release` 进行优化构建。仅开发 CLI 时，`cargo build -p valle-cli` 会跳过这些内嵌资源；启动 Studio 时需传入 `--web-assets-dir /absolute/path/to/web/dist`。

FFmpeg 是可选的运行时依赖，不随 Valle 分发。媒体操作需要安装 FFmpeg 7.x、8.x 或 9.x **动态库**（macOS 可用 `brew install ffmpeg`）。Motion 检查、纯 Motion PNG 渲染及 Studio 启动无需 FFmpeg。运行 `./dist/bin/valle media capabilities` 检查安装情况；自定义路径和兼容规则见 [CLI 运行时说明](docs/cli.md#runtime-notes)。

## 打包

在需要分发的目标平台上构建：

```sh
# macOS arm64
cargo xtask package --min-macos 15.0
# Linux x86_64 或 Windows x86_64（MSVC）
cargo xtask package
```

命令在 FFmpeg 不可用的条件下验证启动、内嵌资源及 Motion/PNG/Studio，然后在 `target/package/` 生成可执行文件和 `SHA256SUMS`；macOS 可执行文件还会签名。GitHub Actions 负责生成分发压缩包及其 `.sha256` 校验文件，并检查解压后的文件完整性、程序启动和 macOS/Linux 执行权限。每个压缩包仅包含可执行文件和 `SHA256SUMS`。

| Actions 工作流 | 分发压缩包 | 包内可执行文件 |
| --- | --- | --- |
| Package macOS arm64 | `valle-vVERSION-darwin-arm64.tar.gz` | `valle-vVERSION-darwin-arm64` |
| Package Linux x86_64 | `valle-vVERSION-linux-x86_64.tar.gz` | `valle-vVERSION-linux-x86_64` |
| Package Windows x86_64 | `valle-vVERSION-windows-x86_64.zip` | `valle-vVERSION-windows-x86_64.exe` |

在 **Actions → 对应工作流 → Run workflow** 手动触发。工作流上传压缩包及校验文件，产物保留 30 天，构建和测试日志单独保存，不会自动发布 GitHub Release。macOS 保留 FFmpeg 7/8/9 软件测试，打包流程不再要求 GPU 硬件测试。

先解压 GitHub 下载的 artifact，再解压里面的分发包。`.tar.gz` 会保留 macOS/Linux 的执行权限，无需再运行 `chmod`。macOS 示例：

```sh
shasum -a 256 -c valle-vVERSION-darwin-arm64.tar.gz.sha256
tar -xzf valle-vVERSION-darwin-arm64.tar.gz
shasum -a 256 -c SHA256SUMS
./valle-vVERSION-darwin-arm64 --help
```

Linux 将命令换成 `sha256sum -c`，文件名换成 Linux 版本。Windows 解压内层 ZIP 后，在 PowerShell 运行 `.\valle-vVERSION-windows-x86_64.exe --help`；可用 `Get-FileHash -Algorithm SHA256` 对照校验文件。已安装构建工具时，三个平台均可用 `cargo xtask verify-package /absolute/path/to/EXECUTABLE` 重新验证运行时。

macOS 包要求 macOS 15+，采用 ad-hoc 签名，未经过 Apple 公证。Linux 包在 Ubuntu 24.04（glibc 2.39）上构建，依赖系统 C/C++ 库、Fontconfig、FreeType 和 OpenBLAS；Ubuntu 可安装 `libfontconfig1 libfreetype6 libopenblas0-pthread`。Windows 使用 MSVC 构建，可能需要 x64 Visual C++ Redistributable；ASR 使用自带计算实现，无需外部 BLAS。FFmpeg 和模型运行时仍为单独安装的可选依赖。

`valle licenses`（或 `valle --json licenses`）可查看内嵌声明和依赖对应版本的源码链接。源码单独提供；分发前需公开该构建对应的 Valle 源码提交。参见[第三方依赖声明](THIRD_PARTY.md#binary-distribution-notices)。

## 使用 CLI

| 命令 | 能力 |
| --- | --- |
| `motion` | 编写 JSX 动画、检查、Studio 预览、导出视频或单帧 |
| `timeline` | 验证并渲染时间线 JSON |
| `project` | 保存时间线版本、提交编辑、恢复历史、预览和渲染 |
| `assets` | 导入、整理、标注和检索本地素材 |
| `media` | 转写、抠像、降噪、分轨、镜头检测、分割、修复、超分和插帧 |
| `models` | 列出、安装和校验本地模型权重 |

[CLI 使用指南](docs/cli.md)覆盖六个命令域的工作流、模型与运行库依赖、JSON 结果、进度事件和退出码。每个命令都可以通过 `--help` 查看参数。

[Motion 编写手册](docs/motion.md)完整说明 JSX、组件、CSS、动画、资源绑定和可运行示例。[Timeline 编写手册](docs/timeline.md)说明片段时间、轨道、音频、字幕和 Motion 集成。

面向 AI Agent 的 [Valle skill](skills/valle/SKILL.md)用一个文件说明命令选择、输入编写、项目版本和结果处理。

直接运行仓库中的示例，无需下载模型：

```sh
./dist/bin/valle motion check examples/hello.motion.tsx
./dist/bin/valle motion render examples/hello.motion.tsx --duration 3 --size 640x360 -o hello.mp4 --events
./dist/bin/valle timeline check examples/timeline.json
./dist/bin/valle timeline render examples/timeline.json -o timeline.mp4 --events
./dist/bin/valle motion studio examples/hello.motion.tsx --size 640x360
```

输出文件不能已存在。`--json` 输出一个机器可读结果；`--events` 输出 NDJSON 进度及最终结果。Studio 输出就绪消息后持续运行，用 Ctrl-C 停止。

采用 [Apache-2.0](LICENSE) 许可证。依赖与参考项目见[第三方说明](THIRD_PARTY.md)。
