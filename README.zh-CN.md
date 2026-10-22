<p align="center">
  <img src="web/apps/shared/brand/valle-mark.svg" width="80" alt="">
</p>

<h1 align="center">Valle</h1>

<p align="center">
  <strong>你的 AI Agent 写代码，Valle 把它变成视频。</strong><br>
  动画用 TSX 写，剪辑用 JSON 写，一个可执行文件在你的电脑上完成渲染。
</p>

<p align="center">
  <a href="README.md">English</a> · 简体中文
</p>

<p align="center">
  <img src="docs/images/launch.webp" width="800" alt="标题 Hello, Valle! 在渐变背景上逐字从模糊中浮起">
</p>

编程 Agent 几分钟就能搭出一个网页。有了 Valle，它也能做视频：用的还是它熟悉的组件、CSS 和 Tailwind，渲染器还会把做出来的结果原原本本地展示给它看。

```tsx
export const composition = { width: 1920, height: 1080, fps: 30, duration: 3 };

export default function Launch() {
  const { seconds } = useCurrentTime();
  return (
    <Scene className="grid size-full place-items-center
      bg-linear-to-br from-indigo-600 via-fuchsia-500 to-orange-400">
      <Text split="char" className="text-9xl font-black tracking-tight text-white" perUnit={(unit) => {
        const shown = clamp(seconds * 2 - unit.index * 0.1, 0, 1);
        return { opacity: shown, blur: 16 * (1 - shown), translate: point(0, 40 * (1 - shown)) };
      }}>
        Hello, Valle!
      </Text>
    </Scene>
  );
}
```

```sh
valle motion render launch.motion.tsx -o launch.mp4
```

整个场景就这么多：页首的动画正是这个文件由 Valle 渲染出来的。

## 为什么选 Valle

**它就是前端代码。** 组件、props、Flexbox、Grid、Tailwind 类名，还能引用你自己的模块。Agent 写片头，就像写落地页一样。

**Agent 看得见自己做了什么。** Agent 没法看视频，Valle 就把帧给它看：一张图里最多 12 帧的故事板；一次审查，标出频闪的运动，以及停留太短、互相重叠或超出画面的文字；报错指向具体的行，通常还会给出该怎么写。

**任意一帧，随取随得。** 每一帧都是其时间和输入的纯函数。第 1000 帧不必先播放前面 999 帧就能渲染，你检查的 PNG 就是 MP4 里的那一帧。

**一切都在你的电脑上。** 一个可执行文件负责检查、渲染、在浏览器 Studio 里预览、管理可检索的素材库，并在本地运行开源模型。无需云服务，也无需账号。

## 能做什么

| | |
| --- | --- |
| **动态图形** | 逐字动画的动态文字、弹簧动画、路径描绘与变形、遮罩、模糊、辉光、运动模糊、转场、LaTeX 公式、图表和粒子 |
| **3D 与着色器** | GLB 模型和挤出几何体，配合 PBR 光照、阴影、镜头运动和景深；也可以写自己的着色器 |
| **音频驱动** | 让动画跟着音轨的响度、频段、起音和节拍走 |
| **剪辑** | 在 Timeline 上编排素材、图片、Motion 片段、音频和字幕；Project 保留每一次修订 |
| **媒体处理** | 带逐词时间戳的转写、声音克隆、语音降噪、人声伴奏分离、抠像、目标跟踪、画面修复、4 倍超分和插帧 |
| **交付** | MP4（H.264，macOS 上还有 HEVC）、透明 ProRes 4444 或无损 MOV、PNG 帧；1080p 的布局直接出 4K；macOS 上用 Metal 渲染并硬件编码 |

## 快速开始

```sh
git clone https://github.com/openvalle/valle.git && cd valle
cargo xtask build --release
./dist/bin/valle motion studio examples/hello.motion.tsx
```

Studio 会打印一个本地地址，用浏览器打开即可。文件一改它就刷新，你可以看着 Agent 干活。构建需要 Rust、Bun 和原生构建工具，导出视频需要 FFmpeg；详见[开发文档](docs/development.md)。

## 交给你的 Agent

把 [`skills/valle`](skills/valle) 复制到 Agent 的 skills 目录，例如 Claude Code 的 `~/.claude/skills/`。这个 skill 告诉 Agent 该跑哪条命令、怎么读结果，以及长时间导出前怎样检查自己的作品。`valle docs` 离线提供与所装版本一致的完整指南，每条命令都能用 `--json` 返回结果。

## 文档

| 指南 | 内容 |
| --- | --- |
| [Motion](docs/motion.md) | 场景、动画、CSS 与 Tailwind、文字、3D 和着色器 |
| [Timeline](docs/timeline.md) | 把素材、Motion 片段、音频和字幕剪到一起 |
| [Project](docs/project.md) | 修订、恢复和并发编辑 |
| [Assets](docs/assets.md) | 本地素材库 |
| [Media](docs/media.md) | 处理音频、图片和视频的本地模型 |
| [CLI](docs/cli.md) | 输出约定、退出码和运行时说明 |
| [开发](docs/development.md) | 构建、测试和打包 Valle |

Valle 仍处于早期开发阶段：会有不完善之处，也可能出现不兼容的变更。

采用 [Apache-2.0](LICENSE) 许可。另见[第三方致谢](THIRD_PARTY.md)。
