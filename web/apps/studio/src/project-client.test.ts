import { expect, test } from "bun:test";
import type { StudioBoot } from "./host.ts";
import { appendImportedAssets } from "./project-client.ts";

const boot = { project: { root: "/promo", name: "Promo", timeline: "timeline.json", entry: true },
  session: { kind: "timeline-file", input: "/promo/timeline.json", token: "token" } } as StudioBoot;
const empty = { canvas: { width: 32, height: 32, fps: 4 }, tracks: {} };

test("a partial import leaves the complete draft unchanged", () => {
  const before = structuredClone(empty);
  expect(() => appendImportedAssets(empty, [{ path: "media/good.png", kind: "image", status: "ok" },
    { path: "broken.motion.tsx", kind: "component", status: "error", error: { message: "local imports are unsupported" } }], boot, 0)).toThrow("Timeline was not changed");
  expect(empty).toEqual(before);
});

test("imports preserve aliases, avoid draft collisions, and create timed visual and audio clips", () => {
  const source = { ...empty, resources: { hero: "media/existing.png" } };
  const next = appendImportedAssets(source, [
    { path: "media/new.png", alias: "hero", kind: "image" },
    { path: "media/music.wav", alias: "music", kind: "audio", info: { duration: 12 } },
    { path: "components/card.motion.tsx", alias: "card", kind: "component" },
    { path: "media/existing.png", alias: "another", kind: "image" },
  ], boot, 2);
  expect(next.resources).toEqual({ hero: "media/existing.png", "hero-2": "media/new.png", music: "media/music.wav", card: "components/card.motion.tsx" });
  expect(next.tracks.visual?.[0]?.clips).toMatchObject([
    { kind: "image", src: "hero-2", start: 2, duration: 5 },
    { kind: "motion", component: "card", start: 7, duration: 3 },
    { kind: "image", src: "hero", start: 10, duration: 5 },
  ]);
  expect(next.tracks.audio?.[0]?.clips[0]).toEqual({ src: "music", start: 2, duration: 12 });
  expect(source.resources).toEqual({ hero: "media/existing.png" });
});

test("a secondary Timeline uses paths relative to its own directory", () => {
  const next = appendImportedAssets(empty, [{ path: "media/take.png", alias: "take", kind: "image" }],
    { ...boot, session: { ...boot.session, input: "/promo/edits/short.json" } }, 0);
  expect(next.resources?.take).toBe("../media/take.png");
});
