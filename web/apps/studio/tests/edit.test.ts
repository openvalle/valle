import { expect, test } from "bun:test";
import { addMedia, relative, selection } from "../src/edit.ts";
import type { Layout } from "../src/service.ts";

function layout(ids: string[], named = true): Layout {
  return {
    width: 320,
    height: 180,
    fps: 30,
    duration: ids.length,
    tracks: [
      {
        id: "v1",
        kind: "visual",
        at: "tracks.visual[0]",
        clips: ids.map((id, i) => ({
          id,
          named,
          at: `tracks.visual[0].items[${i}]`,
          name: null,
          start: i,
          duration: 1,
          enabled: true,
          anchor: null,
          kind: "video",
        })),
      },
    ],
  };
}
test("selection follows the moved clip rather than the item now at its old index", () => {
  expect(
    selection(layout(["a", "b", "c"]), layout(["c", "a", "b"]), "tracks.visual[0].items[2]", {
      op: "move",
      clip: "c",
      index: 0,
    }),
  ).toBe("tracks.visual[0].items[0]");
});
test("deleting an earlier sibling preserves the surviving unnamed clip's identity", () => {
  expect(
    selection(layout(["a", "b", "c"], false), layout(["b", "c"]), "tracks.visual[0].items[2]", {
      op: "delete",
      clip: "a",
    }),
  ).toBe("tracks.visual[0].items[1]");
});
test("deleted selection clears and an id edit follows its new id", () => {
  expect(
    selection(layout(["a"]), layout([]), "tracks.visual[0].items[0]", { op: "delete", clip: "a" }),
  ).toBeUndefined();
  expect(
    selection(layout(["a"]), layout(["new"]), "tracks.visual[0].items[0]", {
      op: "set",
      clip: "a",
      field: "id",
      value: "new",
    }),
  ).toBe("tracks.visual[0].items[0]");
  expect(
    selection(layout(["a", "title"]), layout([]), "tracks.visual[0].items[1]", {
      op: "delete",
      clip: "a",
    }),
  ).toBeUndefined();
  expect(
    selection(layout(["a"]), layout(["visual0.0"], false), "tracks.visual[0].items[0]", {
      op: "set",
      clip: "a",
      field: "id",
      value: null,
    }),
  ).toBe("tracks.visual[0].items[0]");
});
test("media paths are relative to the timeline, retaining unicode and spaces", () => {
  expect(relative("timelines/cuts/edit.timeline.json", "media/室内 tour.mp4")).toBe(
    "../../media/室内 tour.mp4",
  );
  expect(relative("timelines/edit.timeline.json", "timelines/media/a.png")).toBe("media/a.png");
});
test("first media in an empty project creates its track and binding without exceeding source duration", () => {
  const initial = JSON.stringify({ canvas: { width: 320, height: 180, fps: 30 }, tracks: {} });
  const added = addMedia(initial, "timelines/edit.timeline.json", "media/室内.mp4", "video", 4.13);
  const doc = JSON.parse(added.text),
    clip = doc.tracks.visual[0].items[0];
  expect(doc.resources[clip.src]).toBe("../media/室内.mp4");
  expect(clip.duration).toBe(123 / 30);
  expect(clip.kind).toBe("video");
  const again = JSON.parse(
    addMedia(added.text, "timelines/edit.timeline.json", "media/室内.mp4", "video", 4.13).text,
  );
  expect(again.tracks.visual[0].items[1].id).not.toBe(clip.id);
  expect(Object.keys(again.resources)).toHaveLength(1);
});
test("audio appends on its own track and still images get a finite duration", () => {
  const initial = JSON.stringify({
    canvas: { width: 320, height: 180, fps: "30000/1001" },
    tracks: { visual: [{ clips: [] }] },
  });
  const audio = JSON.parse(
    addMedia(initial, "edit.timeline.json", "media/music.wav", "audio", 1).text,
  );
  expect(audio.tracks.audio[0].items[0].kind).toBeUndefined();
  expect(audio.tracks.audio[0].items[0].duration).toBeLessThanOrEqual(1);
  const image = JSON.parse(
    addMedia(JSON.stringify(audio), "edit.timeline.json", "media/image.png", "image", 3).text,
  );
  expect(image.tracks.visual).toHaveLength(2);
  expect(image.tracks.visual[1].items[0].kind).toBe("image");
});
