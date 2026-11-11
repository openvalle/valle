<p align="center">
  <img src="web/apps/studio/brand/valle-mark.svg" width="72" alt="">
</p>

<h1 align="center">Valle</h1>

<p align="center">
  <strong>The video engine for AI agents.</strong><br>
  Agents write video as code. Valle checks it, shows them the frames, and renders it,
  natively, on your machine.
</p>

<p align="center">
  English · <a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <img src="docs/images/reel.webp" width="880" alt="Five Valle scenes cut together: a shader aurora, a variable font, liquid letters, a seed spiral and a 3D carousel">
</p>

Your agent already writes React, CSS and JSON. With Valle, that is all it takes to make
video: motion graphics as TSX components, edits as a JSON timeline. No editor to drive, no
browser to screenshot, just files an agent can write, check and render.

```tsx
// title.motion.tsx
export const composition = { width: 1920, height: 1080, fps: 30, duration: 3 };

export default function Title() {
  const { seconds } = useCurrentTime();
  const shown = interpolate(seconds, [0, 0.8], [0, 1], { easing: "easeOutExpo" });
  return (
    <View className="grid size-full place-items-center bg-linear-to-br from-indigo-600 to-fuchsia-500">
      <Text className="text-9xl font-black text-white" style={{ opacity: shown }}>
        Hello, Valle
      </Text>
    </View>
  );
}
```

```json
// edit.timeline.json: the title, fading into footage
{
  "canvas": { "width": 1920, "height": 1080, "fps": 30 },
  "resources": { "title": "title.motion.tsx", "clip": "clip.mp4" },
  "tracks": {
    "visual": [{ "items": [
      { "id": "title", "kind": "motion", "component": "title", "duration": 2 },
      { "transition": "fade", "inOffset": 0.4, "outOffset": 0.4 },
      { "id": "shot", "kind": "video", "src": "clip", "trimStart": 0.5, "duration": 2.5 }
    ] }]
  }
}
```

```sh
valle timeline check edit.timeline.json                            # every frame, before drawing
valle timeline render edit.timeline.json --storyboard -o sheet.png # see it
valle timeline render edit.timeline.json -o edit.mp4               # render it
```

## Why Valle

**Video is code.** Scenes are components with Flexbox, Grid and Tailwind; edits are JSON with
tracks, transitions and keyframes. Diff them, review them, regenerate them.

**Agents see what they made.** An agent cannot watch a video, so Valle hands it frames: any
frame as a PNG, or a storyboard of many on one sheet. When something is wrong, `check` names
the line or field and what it takes.

**Native, not a browser.** Valle's own engine, in Rust, draws every frame on the GPU. No
headless Chrome, no screenshots: the same input gives the same pixels, on every machine.

**One file, your machine.** The CLI, the renderer and Studio, an editor for people to watch and
adjust the agent's work, in one executable for macOS, Linux and Windows. No cloud, no account.

## What you can make

Motion graphics, edits, captions and shaders. The reel above is a timeline cut from the five
scenes in [`examples/showcase`](examples/showcase), each one file.

## Get started

1. **Download** `valle` for your platform from
   [Releases](https://github.com/openvalle/valle/releases) and put it on your `PATH`. For video,
   install FFmpeg: `brew install ffmpeg` on macOS, other platforms in [Install](docs/install.md).
2. **Add the skill** to your agent:
   ```sh
   npx skills add openvalle/valle
   ```
3. **Ask for a video.** "Make a 10-second launch video for our app", "cut these clips together
   with a title and captions". The agent writes the files, checks them, looks at the frames and
   renders, and you can watch in Studio: `valle project studio`.

[Install](docs/install.md) · [Start](docs/start.md) · [Motion](docs/motion.md) ·
[Timeline](docs/timeline.md) · [Project](docs/project.md) · [CLI](docs/cli.md) ·
[Browser engine](web/packages/engine)

---

Valle 0.1 is early: expect rough edges and breaking changes. The previous version is archived
at [`archive/v0`](https://github.com/openvalle/valle/tree/archive/v0).
[Apache-2.0](LICENSE) · [Third-party software](THIRD_PARTY.md)
