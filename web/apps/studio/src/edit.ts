// Document operations shared by Studio's inspector and media browser.
import type { Layout } from "./service.ts";

/** The value at a document path such as tracks.visual[0].items[2]. */
export function find(root: any, path: string): any {
  let here = root;
  for (const part of path.split(".")) {
    const [key, ...indices] = part.split("[");
    here = here?.[key];
    for (const index of indices) here = here?.[Number(index.slice(0, -1))];
  }
  return here;
}

/** Keep the moved clip selected, even when it has no authored id. */
export function selection(
  before: Layout | undefined,
  after: Layout,
  at: string | undefined,
  op: any,
): string | undefined {
  const clip = before?.tracks.flatMap((t) => t.clips).find((c) => c.at === at);
  if (!clip) return undefined;
  const own = op?.clip === clip.id || op?.clip === clip.at;
  if (own && op.op === "delete") return undefined;
  const renamed = own && op.op === "set" && op.field === "id";
  const clips = after.tracks.flatMap((t) => t.clips);
  if (renamed && op.value == null) return clips.find((c) => c.at === at)?.at;
  return clips.find((c) => c.id === (renamed ? op.value : clip.id))?.at;
}

/** A project-relative asset path, relative to the timeline's own folder. */
export function relative(entry: string, asset: string): string {
  const from = entry.split("/").slice(0, -1),
    to = asset.split("/");
  while (from.length && to.length && from[0] === to[0]) {
    from.shift();
    to.shift();
  }
  return "../".repeat(from.length) + to.join("/");
}

/** Append a media clip, creating its resource and track when needed. */
export function addMedia(
  text: string,
  entry: string,
  asset: string,
  kind: "video" | "audio" | "image",
  duration: number,
): { text: string; id: string } {
  const doc = JSON.parse(text);
  const resources = (doc.resources ??= {});
  const ids = new Set<string>(Object.keys(resources));
  for (const tracks of Object.values(doc.tracks) as any[][])
    for (const track of tracks) {
      for (const clip of track.items ?? track.clips ?? []) if (clip.id) ids.add(clip.id);
    }
  const stem =
    asset
      .split("/")
      .pop()!
      .replace(/\.[^.]+$/, "")
      .replace(/[^a-zA-Z0-9_.-]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 48) || kind;
  let id = stem;
  for (let n = 2; ids.has(id); n++) id = `${stem}-${n}`;
  const file = relative(entry, asset);
  const alias = Object.keys(resources).find((k) => resources[k] === file) ?? id;
  resources[alias] = file;
  const trackKind = kind === "audio" ? "audio" : "visual";
  const tracks = (doc.tracks[trackKind] ??= []);
  // Append to a sequence. Placed overlays keep their own timing and track.
  let track = tracks.find((t: any) => Array.isArray(t.items));
  if (!track) {
    track = { items: [] };
    tracks.push(track);
  }
  const fps =
    typeof doc.canvas.fps === "number"
      ? doc.canvas.fps
      : doc.canvas.fps
          .split("/")
          .map(Number)
          .reduce((a: number, b: number) => a / b);
  const frames = Math.floor(duration * fps + 1e-6);
  if (frames < 1) throw new Error("The media is shorter than one timeline frame.");
  track.items.push({
    id,
    ...(kind === "audio" ? {} : { kind }),
    name: asset.split("/").pop(),
    src: alias,
    duration: frames / fps,
  });
  return { text: JSON.stringify(doc, null, 2) + "\n", id };
}
