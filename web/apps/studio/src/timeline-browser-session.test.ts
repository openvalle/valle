import { expect, test } from "bun:test";
import type { Timeline } from "valle-engine";
import { mediaInputKey } from "./timeline-browser-session.ts";

const document: Timeline = {
  canvas: { width: 3840, height: 2160, fps: 60 },
  resources: {
    scene: "scene.motion.tsx",
    font: "font.otf",
    song: "song.wav",
    video: "hero.mov",
    unused: "unused.wav",
  },
  tracks: {
    visual: [
      {
        clips: [
          {
            kind: "motion",
            component: "scene",
            start: 0,
            duration: 2,
            resources: { brand: "font" },
          },
          { kind: "video", src: "video", start: 2, duration: 2, gain: 0 },
        ],
      },
    ],
    caption: [{ style: { font: "font" }, clips: [{ start: 0, duration: 1, text: "hello" }] }],
    audio: [{ clips: [{ src: "song", start: 0, duration: 4 }] }],
  },
};

test("editing captions, transforms, timing and volume reuses unchanged media facts", () => {
  const next = structuredClone(document);
  next.tracks.caption![0]!.clips[0]!.text = "edited";
  next.tracks.visual![0]!.clips[0]!.duration = 1;
  next.tracks.audio![0]!.clips[0]!.gain = 0.4;
  next.resources!.unused = "another-unused.wav";
  expect(mediaInputKey(next)).toBe(mediaInputKey(document));
});

test("source, type, font and video audio changes invalidate media facts", () => {
  for (const edit of [
    (value: Timeline) => {
      value.resources!.song = "changed.wav";
    },
    (value: Timeline) => {
      value.tracks.caption![0]!.style.font = "another";
    },
    (value: Timeline) => {
      const clip = value.tracks.visual![0]!.clips[1]!;
      if (clip.kind === "video") clip.gain = 1;
    },
    (value: Timeline) => {
      value.tracks.visual![0]!.clips[0] = { kind: "image", src: "scene", start: 0, duration: 2 };
    },
  ]) {
    const next = structuredClone(document);
    edit(next);
    expect(mediaInputKey(next)).not.toBe(mediaInputKey(document));
  }
  expect(
    mediaInputKey({ canvas: document.canvas, resources: document.resources, tracks: {} }),
  ).toBeNull();
});
