# Valle

[English](README.md) | 简体中文

面向 AI Agent 的视频创作与剪辑引擎。

**目前是早期开发版。** 我们先开放代码，许多功能、构建工具和文档仍未完善。项目还存在缺陷，后续迭代也可能带来不兼容的变更。

## 环境依赖

- 通过 [rustup](https://rustup.rs/) 安装 Rust。仓库的 `rust-toolchain.toml` 指定 **Rust 1.96.0** 和 `wasm32-unknown-unknown` 目标。
- [Bun](https://bun.com/docs/installation) **1.4.2**。
- **FFmpeg 开发头文件及动态库**：`libavcodec`、`libavformat`、`libavutil`、`libswscale`、`libswresample`。已使用 **FFmpeg 8.1.1** 验证，只有独立的 `ffmpeg` 可执行文件不够。
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

在仓库根目录创建 `hello.motion.tsx`：

```tsx
export default function Hello(ctx) {
  const opacity = interpolate(ctx.hold.progress, [0, 0.6], [0, 1]);

  return (
    <Scene className="relative h-full w-full flex flex-col items-center justify-center"
      style={{ backgroundColor: "#102030" }}>
      <Text style={{ fontSize: 80, fontWeight: 700, color: "#ffffff", opacity }}>
        Hello, Valle!
      </Text>
      <Text style={{ marginTop: 20, fontSize: 28, color: "#94a3b8", opacity }}>
        Create with code. Bring it to life.
      </Text>
    </Scene>
  );
}
```

启动 Studio，打开终端打印的网址：

```sh
./dist/bin/valle motion studio hello.motion.tsx
```

也可以直接导出视频或单帧图片：

```sh
./dist/bin/valle motion render hello.motion.tsx --duration 3 --size 1280x720 -o hello.mp4
./dist/bin/valle motion render hello.motion.tsx --frame 30 --size 1280x720 -o hello.png
```

输出文件不能已存在。这个示例无需下载模型；AI 媒体工具需另行准备对应模型，以及适用的 ONNX Runtime。

采用 [Apache-2.0](LICENSE) 许可证。依赖与参考项目见[第三方说明](THIRD_PARTY.md)。
