# Valle

[English](README.md) | 简体中文

面向 AI Agent 的视频创作与剪辑引擎。

**目前是早期开发版。** 我们先开放代码，许多功能、构建工具和文档仍未完善。项目还存在缺陷，后续迭代也可能带来不兼容的变更。

## 环境依赖

- 通过 [rustup](https://rustup.rs/) 安装 Rust。仓库的 `rust-toolchain.toml` 指定 **Rust 1.96.0** 和 `wasm32-unknown-unknown` 目标。
- [Bun](https://bun.com/docs/installation) **1.4.2**。
- **FFmpeg 开发头文件及动态库**：`libavcodec`、`libavformat`、`libavutil`、`libswscale`、`libswresample`。已在 macOS 使用 **FFmpeg 9.0.1** 本地验证，只有独立的 `ffmpeg` 可执行文件不够。
- 原生构建工具：C/C++ 编译器、LLVM/libclang、`pkg-config`、CMake、Python 3、Ninja、Git、curl、tar。参见 [rust-skia 构建要求](https://github.com/rust-skia/rust-skia#building)。

以下以 **macOS** 为例，其他平台的环境配置仍待验证。先安装上述 Rust 和 Bun，再通过 [Homebrew](https://brew.sh/) 安装原生依赖：

```sh
xcode-select --install # 尚未安装 Command Line Tools 时执行
brew install ffmpeg pkgconf llvm cmake ninja python
export PATH="$(brew --prefix llvm)/bin:$PATH"
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
export PKG_CONFIG_PATH="$(brew --prefix ffmpeg)/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
```

## 构建与运行

```sh
git clone https://github.com/openvalle/valle.git
cd valle
cargo xtask build
```

首次构建需要联网，耗时可能较长。Cargo 已配置为从源码构建 Skia，以提供 CLI 所需的 Skottie 支持。Rust `xtask` 会安装 Web 依赖、构建 Wasm 和 CLI，并将结果放入 `dist/`。使用 `cargo xtask build --release` 进行发布构建。运行 CLI 时也需要系统提供 FFmpeg 动态库。

仅开发 CLI 时，可以使用 `cargo build -p valle-cli`。Studio 还需要上述完整构建生成的 Web 资源。

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
