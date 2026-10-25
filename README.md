<p align="center">
  <img src="web/apps/shared/brand/valle-mark.svg" width="80" alt="">
</p>

<h1 align="center">Valle</h1>

<p align="center">
  <strong>Your AI agent writes code. Valle turns it into video.</strong><br>
  Motion graphics in TSX, edits in JSON, rendered on your machine by one executable.
</p>

<p align="center">
  English · <a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <img src="docs/images/launch.webp" width="800" alt="The title Hello, Valle! rises letter by letter out of a blur over a gradient">
</p>

Coding agents build web pages in minutes. Valle lets them make the video too, with the
components, CSS and Tailwind they already know, and a renderer that shows them exactly what
they made.

```tsx
export const composition = { width: 1920, height: 1080, fps: 60, duration: 5 };

export default function Launch() {
  const { seconds } = useCurrentTime();
  return (
    <Scene className="grid size-full place-items-center bg-linear-to-br from-indigo-600 via-fuchsia-500 to-orange-400">
      <Text split="char" className="text-9xl font-black tracking-tight text-white" perUnit={(unit) => {
        const enter = interpolate(seconds, [unit.index * 0.04, 0.8 + unit.index * 0.04], [0, 1], { easing: "easeOutCubic" });
        const leave = interpolate(seconds, [3.1 + unit.index * 0.025, 3.8 + unit.index * 0.025], [0, 1], { easing: "easeInOutCubic" });
        const shown = clamp(enter - leave, 0, 1);
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

That is the whole scene: the animation at the top is this file, rendered by Valle.

## Why Valle

**It is frontend code.** Components, props, Flexbox, Grid, Tailwind classes and your own
modules. An agent writes a title sequence the way it writes a landing page.

**Agents see what they made.** An agent cannot watch a video, so Valle shows it the frames: a
storyboard of up to 12 frames in one image, a review that flags strobing motion and text that
is too brief, overlapping or out of frame, and errors that point to the line and usually name
what to write instead.

**Any frame, on demand.** Each frame is a pure function of its time and inputs. Frame 1,000
renders without playing the 999 before it, and the PNG you check is the frame the MP4 shows.

**It runs on your machine.** One executable checks, renders, previews in a browser Studio,
keeps a searchable media library and runs open models locally. No cloud service, no account.

## What you can make

| | |
| --- | --- |
| **Motion graphics** | Kinetic type with per-letter animation, springs, path drawing and morphing, masks, blur, glow, motion blur, transitions, LaTeX formulas, charts and particles |
| **3D and shaders** | GLB models and extruded geometry with PBR lighting, shadows, camera moves and depth of field; your own shaders |
| **Audio-driven motion** | Animation that follows the levels, bands, onsets and beats of a track |
| **Edits** | Footage, images, Motion clips, audio and captions on a Timeline; a Project keeps source snapshots |
| **Media processing** | Word-timed transcripts, voice cloning, speech cleanup, stem separation, matting, object tracking, inpainting, 4× upscaling and frame interpolation |
| **Delivery** | MP4 (H.264, or HEVC on macOS), transparent ProRes 4444 or lossless MOV, PNG frames; 4K from a 1080p layout; Metal rendering and hardware encoding on macOS |

## Get started

```sh
git clone https://github.com/openvalle/valle.git && cd valle
cargo xtask build --release
./dist/bin/valle motion studio examples/hello.motion.tsx
```

Studio prints a local URL to open in your browser. It refreshes whenever the file changes, so
you can watch an agent work. Building needs Rust, Bun and native build tools, and video export
needs FFmpeg; see [Development](docs/development.md).

Ten more scenes to start from are in [`examples`](examples), each in one file: kinetic type, a
logo sting, a dashboard, a product UI, a vertical story, a seamless loop, a route map,
confetti, 3D cards and a broadcast lower third.

## Hand it to your agent

Copy [`skills/valle`](skills/valle) into your agent's skills directory, such as
`~/.claude/skills/` for Claude Code. The skill tells the agent which command to run, how to
read the results and how to check its work before a long export. `valle docs` serves the full
guides offline, matching the installed version, and every command answers in `--json`.

## Documentation

| Guide | Covers |
| --- | --- |
| [Motion](docs/motion.md) | Scenes, animation, CSS and Tailwind, text, 3D and shaders |
| [Timeline](docs/timeline.md) | Footage, Motion clips, audio and captions cut together |
| [Project](docs/project.md) | Project folders, source snapshots, restore and rendering |
| [Assets](docs/assets.md) | The local media library |
| [Media](docs/media.md) | Local models for audio, images and video |
| [CLI](docs/cli.md) | Output contract, exit codes and runtime notes |
| [Development](docs/development.md) | Building, testing and packaging Valle |

Valle is in early development: expect rough edges and breaking changes.

Licensed under [Apache-2.0](LICENSE). See [third-party acknowledgements](THIRD_PARTY.md).
